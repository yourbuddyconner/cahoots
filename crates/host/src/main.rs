//! `cahoots-host`: run simulated Cahoots badges on a laptop.
//!
//! ```text
//! cahoots-host ceremony --index 0 --n 4 --pad-len 262144 --name sf \
//!     --listen /tmp/c0.sock --next /tmp/c1.sock --state /tmp/dev0
//! cahoots-host send --state /tmp/dev0 --text "hello"
//! cahoots-host recv --state /tmp/dev3 --message <base64url>
//! cahoots-host status --state /tmp/dev0
//! cahoots-host pile --cards 3 --state /tmp/pile [--serial /dev/cu.usbmodem...] [--name sf] [--pad-len 16384]
//! cahoots-host badge --state /tmp/b0 --listen - --next /tmp/b1.sock --name sf --pad-len 16384 [--seed 1] [--chip soft]
//! cahoots-host agent (--serial <port> | --sim <socket>) --circle <name>... [--socket <path>] [--known-hosts <path>]
//! cahoots-host git-sign -Y sign -n git -f <key file> <file>
//! cahoots-host roster --ssh --circle <name> --state <dir>
//! cahoots-host attest (--serial <port> | --sim <socket>) --circle <name> [--json]
//! cahoots-host card set (--serial <port> | --sim <socket>) --circle <name> --name <text> [--handle kind=value]... [--endpoint purpose=value]... --out <file>
//! cahoots-host card fetch (--serial <port> | --sim <socket>) --circle <name> --state <dir> --in <file>... [--out <dir>]
//! cahoots-host card show --circle <name> --state <dir> <signed card file>
//! cahoots-host grant (--serial <port> | --sim <socket>) --circle <name> --to <key hex> --purpose <name>... --budget N --until N [--depth N] [--v2] [--out <file>]
//! ```
//!
//! `pile` runs a stack of cards on this machine, bottom first, over Unix sockets. With
//! `--serial`, the top card is a real board on that port and `--cards` counts it.
//!
//! `badge` runs one card through the badge crate on a [`HostBoard`]: boot, then a pile pass
//! while it has no circle, then the idle menu. `--listen` is the down link and `--next` the
//! up link; `-` means nothing is plugged in on that side. Press buttons by writing `L`, `R` or `B` (Both)
//! lines to `buttons.txt` in the state directory. `--chip soft` gives the card a software chip,
//! kept in `softchip.bin`, so its seat keys are made and used there.
//!
//! `agent` is an SSH agent whose keys are a badge's seat keys; the badge asks its holder for
//! every signature. `git-sign` is the program git runs to sign a commit (also built as
//! `cahoots-host-git-sign`, the one to name in `gpg.ssh.program`). `roster` prints a circle's
//! seat keys as `authorized_keys` lines. `attest` asks the badge's chip to attest a seat key.
//! `card` makes, opens and shows contact cards, and `grant` has the badge sign a grant; see
//! [`cahoots_host::card`].
//! `--sim` is the socket a simulated badge listens on (its `--listen`).

use base64::Engine;
use cahoots_badge::{Badge, Board};
use cahoots_core::message::MessageType;
use cahoots_core::params::CircleParams;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::Display;
use cahoots_hal::{Entropy, HealthError};
use cahoots_host::agent::Agent;
use cahoots_host::badge_client::BadgeClient;
use cahoots_host::board::HostBoard;
use cahoots_host::device::Device;
use cahoots_host::link::SocketLink;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    fn all(&self, key: &str) -> Vec<String> {
        self.0
            .windows(2)
            .filter(|w| w[0] == key)
            .map(|w| w[1].clone())
            .collect()
    }
    fn has(&self, flag: &str) -> bool {
        self.0.iter().any(|a| a == flag)
    }
    /// The badge named by `--serial <port>` or `--sim <socket>`.
    fn badge(&self) -> Result<BadgeClient, String> {
        match (self.get("--serial"), self.get("--sim")) {
            (Some(port), None) => BadgeClient::serial(port).map_err(|e| e.to_string()),
            (None, Some(sock)) => BadgeClient::sim(Path::new(sock)).map_err(|e| e.to_string()),
            _ => Err("name the badge with --serial <port> or --sim <socket>".into()),
        }
    }
    fn req(&self, key: &str) -> String {
        self.get(key).map(String::from).unwrap_or_else(|| {
            eprintln!("missing {key}");
            std::process::exit(2)
        })
    }
}

/// Two cables: one from the previous device (it connects to us) and one to the next (we
/// connect to it). Frames flood both ways, so this works as a closed ring or an open line.
fn connect_ring(listen: &str, next: &str) -> std::io::Result<(SocketLink, SocketLink)> {
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
    let (prev, _) = listener.accept()?;
    Ok((SocketLink::new(prev)?, SocketLink::new(tx)?))
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!(
            "usage: cahoots-host <ceremony|pile|badge|logs|send|recv|status|agent|git-sign|roster|attest|card|grant> ..."
        );
        std::process::exit(2);
    };
    if cmd == "git-sign" {
        std::process::exit(cahoots_host::gitsign::main(&argv[1..]));
    }
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
            let (prev, next) = connect_ring(&args.req("--listen"), &args.req("--next"))
                .unwrap_or_else(|e| {
                    eprintln!("link: {e}");
                    std::process::exit(1)
                });
            let links = vec![prev, next];
            let tests = HealthTests::for_min_entropy(7);
            let r = match args.get("--seed") {
                Some(s) => {
                    let mut e = Tested::new(Seeded(s.parse().expect("--seed")), tests);
                    Device::ceremony(&state, params, index, &mut e, links)
                }
                None => {
                    let mut e = Tested::new(OsEntropy, tests);
                    Device::ceremony(&state, params, index, &mut e, links)
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
        "pile" => {
            let cards: usize = args.req("--cards").parse().expect("--cards");
            let pad_len: u32 = args
                .get("--pad-len")
                .map(|s| s.parse().expect("--pad-len"))
                .unwrap_or(16 * 1024);
            let name = args.get("--name").unwrap_or("cahoot").to_string();
            let state = PathBuf::from(args.req("--state"));
            let seed = args.get("--seed").map(|s| s.parse().expect("--seed"));
            cahoots_host::pile::run_pile(cards, args.get("--serial"), &name, pad_len, &state, seed)
                .map(|devices| {
                    for (i, d) in devices.iter().enumerate() {
                        println!(
                            "card{i} seat {i} circle_id {} words {}",
                            hex(&d.circle_id()),
                            d.check_words().join(" ")
                        );
                    }
                })
                .map_err(|e| e.to_string())
        }
        "badge" => {
            let state = PathBuf::from(args.req("--state"));
            let pad_len: u32 = args.req("--pad-len").parse().expect("--pad-len");
            let seed = args.get("--seed").map(|s| s.parse().expect("--seed"));
            let board = HostBoard::new(
                &state,
                &args.req("--listen"),
                &args.req("--next"),
                &args.req("--name"),
                pad_len,
                seed,
            );
            let board = match args.get("--chip") {
                None => board,
                Some("soft") => board.with_soft_chip(),
                Some(other) => {
                    eprintln!(
                        "--chip {other}: the simulator has only a software chip, --chip soft"
                    );
                    std::process::exit(2)
                }
            };
            let mut badge = Badge::new(board);
            badge.boot();
            loop {
                if badge.circle().is_none() {
                    if let Err(e) = badge.run_once() {
                        badge.board().log(&e);
                        let screen = cahoots_badge::screens::status(&[&e]);
                        badge.board().note_screen(&screen.lines, screen.secret);
                        badge.board().display().draw(&screen.frame);
                        badge.board().delay_ms(1500);
                    }
                } else {
                    badge.idle_step();
                }
            }
        }
        "logs" => {
            // Print a board's log lines and nothing else: no pings, so the board does not
            // take the laptop for a card below it, which matters when it is the bottom of
            // a real pile.
            let path = args.req("--serial");
            let label = args.get("--label").unwrap_or("board").to_string();
            cahoots_host::link::tail_logs(&path, &label).map_err(|e| e.to_string())
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
        "agent" => run_agent(&args),
        "roster" => {
            if !args.has("--ssh") {
                eprintln!("roster prints authorized_keys lines: add --ssh");
                std::process::exit(2);
            }
            let state = PathBuf::from(args.req("--state"));
            cahoots_host::roster::manifest(&state, &args.req("--circle"))
                .and_then(|m| cahoots_host::roster::ssh_lines(&m))
                .map(|lines| {
                    for l in lines {
                        println!("{l}");
                    }
                })
        }
        "attest" => {
            let circle = args.req("--circle");
            args.badge()
                .and_then(|mut b| cahoots_host::attest::attest(&mut b, &circle))
                .and_then(|o| {
                    if args.has("--json") {
                        println!("{}", o.json);
                    } else {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&o.json).unwrap_or_default()
                        );
                        for l in &o.lines {
                            println!("{l}");
                        }
                    }
                    if o.verified {
                        Ok(())
                    } else {
                        Err("the attestation did not verify".into())
                    }
                })
        }
        "card" => run_card(&args),
        "grant" => run_grant(&args),
        _ => Err(format!("unknown command {cmd}")),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// `agent`: listen on the socket, print the line that points ssh at it, and answer until
/// killed.
fn run_agent(args: &Args) -> Result<(), String> {
    let circles = args.all("--circle");
    if circles.is_empty() {
        return Err("name at least one circle with --circle".into());
    }
    let socket = args
        .get("--socket")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cahoots-agent.sock"));
    let known_hosts = match args.get("--known-hosts") {
        Some(p) => PathBuf::from(p),
        None => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".ssh/known_hosts"))
            .ok_or("no HOME: name the known_hosts file with --known-hosts")?,
    };
    let badge = args.badge()?;
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", socket.display()))?;
    }
    println!("SSH_AUTH_SOCK={}; export SSH_AUTH_SOCK;", socket.display());
    use std::io::Write;
    let _ = std::io::stdout().flush();
    eprintln!("agent: the badge must be on its Serve page, serving, to answer");
    Arc::new(Agent::new(badge, circles, known_hosts))
        .serve(listener)
        .map_err(|e| e.to_string())
}

/// `card set`, `card fetch` and `card show`.
fn run_card(args: &Args) -> Result<(), String> {
    use cahoots_host::card;
    let circle = args.req("--circle");
    match args.0.first().map(String::as_str) {
        Some("set") => {
            let mut badge = args.badge()?;
            let info = card::circle(&mut badge, &circle)?;
            let log = match args.get("--log") {
                Some(n) => n.parse().map_err(|_| "--log: a number")?,
                None => 0,
            };
            let c = card::build(
                &info,
                &args.req("--name"),
                &args.all("--handle"),
                &args.all("--endpoint"),
                log,
            )?;
            let out = PathBuf::from(args.req("--out"));
            card::set(&mut badge, &info, &c, &out)?;
            println!("{}", serde_json::to_string_pretty(&c.to_json()).unwrap_or_default());
            println!("sealed card written to {}", out.display());
            Ok(())
        }
        Some("fetch") => {
            let manifest = cahoots_host::roster::manifest(Path::new(&args.req("--state")), &circle)?;
            let files: Vec<PathBuf> = args.all("--in").into_iter().map(PathBuf::from).collect();
            if files.is_empty() {
                return Err("name the sealed cards with --in <file>".into());
            }
            let mut badge = args.badge()?;
            let info = card::circle(&mut badge, &circle)?;
            let key = card::seal_key(&mut badge, &info)?;
            let cards = card::open_all(&files, &key, &manifest)?;
            for (signed, c) in &cards {
                println!("{}", serde_json::to_string_pretty(&c.to_json()).unwrap_or_default());
                if let Some(dir) = args.get("--out") {
                    let path = Path::new(dir).join(format!("{}.card", c.seat));
                    std::fs::write(&path, signed).map_err(|e| format!("{}: {e}", path.display()))?;
                }
            }
            Ok(())
        }
        Some("show") => {
            let manifest = cahoots_host::roster::manifest(Path::new(&args.req("--state")), &circle)?;
            let file = args
                .0
                .last()
                .filter(|f| !f.starts_with("--") && args.0.len() > 1)
                .ok_or("name the signed card file last")?;
            let signed = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let c = card::read_signed(&signed, &manifest)?;
            println!("{}", serde_json::to_string_pretty(&c.to_json()).unwrap_or_default());
            Ok(())
        }
        _ => Err("card set, card fetch or card show".into()),
    }
}

/// `grant`: a root grant from this seat, signed by the badge.
fn run_grant(args: &Args) -> Result<(), String> {
    use cahoots_host::card;
    let circle = args.req("--circle");
    let number = |key: &str| -> Result<u64, String> {
        args.req(key).parse().map_err(|_| format!("{key}: a number"))
    };
    let budget = u32::try_from(number("--budget")?).map_err(|_| "--budget: too large")?;
    let until = number("--until")?;
    let depth = match args.get("--depth") {
        Some(d) => d.parse().map_err(|_| "--depth: 0 to 255")?,
        None => 0,
    };
    let mut badge = args.badge()?;
    let info = card::circle(&mut badge, &circle)?;
    let g = card::build_grant(&info, &args.req("--to"), &args.all("--purpose"), budget, until, depth)?;
    let v2 = args.has("--v2");
    let signed = card::sign_grant(&mut badge, &info, &g, v2)?;
    match args.get("--out") {
        Some(out) => std::fs::write(out, &signed).map_err(|e| format!("{out}: {e}"))?,
        None => println!("{}", hex(&signed)),
    }
    let projection = if v2 { g.to_v2().to_json() } else { g.to_json() };
    eprintln!("{}", serde_json::to_string_pretty(&projection).unwrap_or_default());
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
