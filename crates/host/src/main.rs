//! `cahoots-host`: run simulated Cahoots badges on a laptop.
//!
//! ```text
//! cahoots-host ceremony --index 0 --n 4 --pad-len 262144 --name sf \
//!     --listen /tmp/c0.sock --next /tmp/c1.sock --state /tmp/dev0
//! cahoots-host send --state /tmp/dev0 --text "hello"
//! cahoots-host recv --state /tmp/dev3 --message <base64url>
//! cahoots-host status --state /tmp/dev0
//! ```

use base64::Engine;
use cahoots_core::message::MessageType;
use cahoots_core::params::CircleParams;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::{Entropy, HealthError};
use cahoots_host::device::Device;
use cahoots_host::link::SocketLink;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The host's randomness: the OS CSPRNG. A simulated badge is a `CSPRNG` source, not a
/// hardware TRNG. Real targets replace this with their TRNG.
struct OsEntropy;
impl Entropy for OsEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        getrandom::fill(buf).map_err(|_| HealthError::SourceUnavailable)
    }
}

/// Deterministic xorshift* source for reproducible demos (`--seed`). Never use for a real circle.
struct Seeded(u64);
impl Entropy for Seeded {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        for b in buf.iter_mut() {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
        Ok(())
    }
}

struct Args(Vec<String>);
impl Args {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .position(|a| a == key)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }
    fn req(&self, key: &str) -> String {
        self.get(key).map(String::from).unwrap_or_else(|| {
            eprintln!("missing {key}");
            std::process::exit(2)
        })
    }
}

fn connect_ring(listen: &str, next: &str) -> std::io::Result<SocketLink> {
    let _ = std::fs::remove_file(listen);
    let listener = UnixListener::bind(listen)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let tx = loop {
        match UnixStream::connect(next) {
            Ok(s) => break s,
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    };
    let (rx, _) = listener.accept()?;
    SocketLink::new(tx, rx)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!("usage: cahoots-host <ceremony|send|recv|status> ...");
        std::process::exit(2);
    };
    let args = Args(argv[1..].to_vec());
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let result: Result<(), String> = match cmd.as_str() {
        "ceremony" => {
            let index: u8 = args.req("--index").parse().expect("--index");
            let n: u8 = args.req("--n").parse().expect("--n");
            let pad_len: u32 = args
                .get("--pad-len")
                .map(|s| s.parse().expect("--pad-len"))
                .unwrap_or(256 * 1024);
            let name = args.get("--name").unwrap_or("cahoot").to_string();
            let agent = args.get("--agent-seat").is_some();
            let state = PathBuf::from(args.req("--state"));
            let params = CircleParams::new(n, pad_len, &name, agent)
                .map_err(|e| format!("params: {e:?}"))
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(2)
                });
            let link =
                connect_ring(&args.req("--listen"), &args.req("--next")).unwrap_or_else(|e| {
                    eprintln!("link: {e}");
                    std::process::exit(1)
                });
            let tests = HealthTests::for_min_entropy(7);
            let r = match args.get("--seed") {
                Some(s) => {
                    let mut e = Tested::new(Seeded(s.parse().expect("--seed")), tests);
                    Device::ceremony(&state, params, index, &mut e, link)
                }
                None => {
                    let mut e = Tested::new(OsEntropy, tests);
                    Device::ceremony(&state, params, index, &mut e, link)
                }
            };
            r.map(|d| {
                println!("circle_id {}", hex(&d.circle_id()));
                println!("check_words {}", d.check_words().join(" "));
                println!("state {}", d.dir().display());
            })
            .map_err(|e| e.to_string())
        }
        "send" => {
            let state = PathBuf::from(args.req("--state"));
            let text = args.req("--text");
            Device::open(&state)
                .and_then(|mut d| d.send(MessageType::Text, text.as_bytes()))
                .map(|wire| println!("{}", b64.encode(wire)))
                .map_err(|e| e.to_string())
        }
        "recv" => {
            let state = PathBuf::from(args.req("--state"));
            let wire = b64
                .decode(args.req("--message"))
                .map_err(|e| format!("base64: {e}"));
            wire.and_then(|w| {
                Device::open(&state)
                    .and_then(|mut d| d.receive(&w))
                    .map(|(h, pt)| {
                        println!(
                            "slice {} offset {} type {:?}: {}",
                            h.slice,
                            h.offset,
                            h.msg_type,
                            String::from_utf8_lossy(&pt)
                        )
                    })
                    .map_err(|e| e.to_string())
            })
        }
        "status" => {
            let state = PathBuf::from(args.req("--state"));
            Device::open(&state)
                .map(|d| {
                    println!("circle_id {}", hex(&d.circle_id()));
                    println!("name {}", d.params().name);
                    println!("check_words {}", d.check_words().join(" "));
                    println!("remaining {} of {}", d.remaining(), d.params().slice_len());
                    for s in 0..d.params().slices() {
                        println!("slice {s} spent {:?}", d.spent_ranges(s));
                    }
                })
                .map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown command {cmd}")),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
