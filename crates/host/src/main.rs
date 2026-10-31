//! `cahoots-host`: run simulated Cahoots badges on a laptop.
//!
//! ```text
//! cahoots-host ceremony --index 0 --n 4 --pad-len 262144 --name sf \
//!     --listen /tmp/c0.sock --next /tmp/c1.sock --state /tmp/dev0
//! cahoots-host send --state /tmp/dev0 --text "hello"
//! cahoots-host recv --state /tmp/dev3 --message <base64url>
//! cahoots-host status --state /tmp/dev0
//! cahoots-host pile --cards 3 --state /tmp/pile [--serial /dev/cu.usbmodem...] [--name sf] [--pad-len 16384]
//! cahoots-host bridge --serial <port> (--offer <text> | --offer-file <path>) [--badges N]
//! cahoots-host badge --state /tmp/b0 --listen - --next /tmp/b1.sock --name sf --pad-len 16384 [--seed 1] [--chip soft]
//! cahoots-host agent (--serial <port> | --sim <socket>) --circle <name>... [--socket <path>] [--known-hosts <path>]
//! cahoots-host git-sign -Y sign -n git -f <key file> <file>
//! cahoots-host sign --file <path> (--circle <name> | --key <pub file>) [--namespace file] [--out <path>]
//! cahoots-host roster --ssh --circle <name> --state <dir>
//! cahoots-host attest (--serial <port> | --sim <socket>) --circle <name> [--test-root] [--json]
//! cahoots-host card set (--serial <port> | --sim <socket>) --circle <name> --name <text> [--handle kind=value]... [--endpoint purpose=value]... --out <file>
//! cahoots-host card fetch (--serial <port> | --sim <socket>) --circle <name> --state <dir> --in <file>... [--out <dir>]
//! cahoots-host card show --circle <name> --state <dir> <signed card file>
//! cahoots-host grant (--serial <port> | --sim <socket>) --circle <name> --to <key hex> --purpose <name>... --budget N --until N [--depth N] [--v2] [--out <file>]
//! cahoots-host grant request (--serial <port> | --sim <socket>) --circle <name> --agent <text> --author <text> [--location <repo>] --budget N (--until YYYY-MM-DD | --standing)
//! cahoots-host grant list
//! cahoots-host grant show <file>
//! cahoots-host grant forget <file>
//! cahoots-host grant verify <file> --state <dir> --circle <name>
//! ```
//!
//! `pile` runs a stack of cards on this machine, bottom first, over Unix sockets. With
//! `--serial`, the top card is a real board on that port and `--cards` counts it.
//!
//! `bridge` puts a pile of cards on `--serial`, bottom card on the cable, into a ceremony
//! a phone hosts through the relay. `--offer` is the host's code (`--offer-file` reads it from
//! a file), and `--badges` the cards in the pile, by default the number the code was made for.
//! It prints a code for each card for the host's phone to scan. See [`cahoots_host::bridge`].
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
//! [`cahoots_host::card`]. `grant request`, `list`, `show`, `verify` and `forget` are grants for an
//! agent: the badge lets the agent's key sign git commits without a press; see
//! [`cahoots_host::grants`].
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

/// `cahoots-host bridge`: carry a pile on a serial port into a phone's ceremony.
fn run_bridge(args: &Args) -> Result<(), String> {
    use cahoots_host::bridge::Bridge;
    let text = match (args.get("--offer"), args.get("--offer-file")) {
        (Some(t), None) => t.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?,
        _ => return Err("give the host's code with --offer <text> or --offer-file <path>".into()),
    };
    let offer = cahoots_core::channel::Channel::from_offer(&text)
        .map_err(|e| match e {
            cahoots_core::channel::ChannelError::OlderOffer | cahoots_core::channel::ChannelError::NewerOffer => {
                "the host's code is from another version of the app. Update both and start again".to_string()
            }
            _ => "the host's code could not be read. Copy it again from the host's phone".to_string(),
        })?;
    let badges = match args.get("--badges") {
        Some(b) => b.parse().map_err(|_| "--badges: a number of cards")?,
        None => offer.badges,
    };
    let port = args.get("--serial").ok_or("name the bottom card's port with --serial <port>")?;
    let mut poster = cahoots_relay::HttpRelay::new(&offer.relay);
    let mut poller = cahoots_relay::HttpRelay::new(&offer.relay);
    let mut bridge = Bridge::new(offer, badges).map_err(|e| e.to_string())?;
    let mut link = cahoots_host::link::SerialLink::open(port, "pile").map_err(|e| format!("{port}: {e}"))?;
    println!("Waiting for the cards. Each card's code appears here for the host's phone to scan.");
    bridge
        .run(&mut link, &mut poster, &mut poller, &mut |seat, code| {
            println!();
            match qrcode::QrCode::new(code.as_bytes()) {
                Ok(qr) => println!(
                    "{}",
                    qr.render::<qrcode::render::unicode::Dense1x2>()
                        .quiet_zone(true)
                        .build()
                ),
                Err(e) => eprintln!("the code for seat {} could not be drawn: {e}", u16::from(seat) + 1),
            }
            println!("Card in seat {}. Scan this code with the host's phone.", u16::from(seat) + 1);
            println!("{code}");
        })
        .map_err(|e| e.to_string())?;
    println!("every seat has signed. The badges are keeping their circles.");
    Ok(())
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!(
            "usage: cahoots-host <ceremony|pile|bridge|badge|logs|send|recv|status|agent|git-sign|roster|attest|card|grant> ..."
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
        "bridge" => run_bridge(&args),
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
        "sign" => {
            let file = PathBuf::from(args.req("--file"));
            let namespace = args
                .get("--namespace")
                .unwrap_or(cahoots_host::sign::FILE_NAMESPACE)
                .to_string();
            let socket = std::env::var_os("CAHOOTS_AGENT_SOCK")
                .or_else(|| std::env::var_os("SSH_AUTH_SOCK"))
                .map(PathBuf::from)
                .ok_or_else(|| String::from("no agent: set SSH_AUTH_SOCK to the socket `cahoots-host agent` printed"));
            socket
                .and_then(|socket| {
                    let key = match (args.get("--circle"), args.get("--key")) {
                        (Some(c), _) => cahoots_host::sign::key_for_circle(&socket, c)?,
                        (None, Some(k)) => cahoots_host::gitsign::read_key(Path::new(k))?,
                        (None, None) => return Err(String::from("say which key: --circle <name> or --key <pub file>")),
                    };
                    let out = args.get("--out").map(PathBuf::from);
                    cahoots_host::sign::sign_file(&socket, &key, &namespace, &file, out.as_deref())
                })
                .map(|s| {
                    println!("{}", s.sig_path.display());
                    eprintln!(
                        "signed {} in namespace {namespace}{}. To check it, put\n  you@example.com namespaces=\"{namespace}\" {}\nin an allowed_signers file and run\n  ssh-keygen -Y verify -f allowed_signers -I you@example.com -n {namespace} -s {} < {}",
                        file.display(),
                        if s.shown { "" } else { " (too big to show the badge; it signed the hash)" },
                        s.key_line,
                        s.sig_path.display(),
                        file.display()
                    );
                })
        }
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
            // A simulated badge's chip is the software chip, so its test root is the one that
            // can verify it; `--test-root` asks for the same on a board.
            let test_root = args.has("--sim") || args.has("--test-root");
            args.badge()
                .and_then(|mut b| cahoots_host::attest::attest(&mut b, &circle, test_root))
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
        "grant" => match args.0.first().map(String::as_str) {
            Some("request" | "list" | "show" | "verify" | "forget") => run_agent_grant(&args),
            _ => run_grant(&args),
        },
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
    eprintln!("agent: the badge asks once, on its screen, whether to allow this laptop until it is unplugged");
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

/// `grant request`, `grant list`, `grant show`, `grant verify` and `grant forget`: grants for
/// an agent.
fn run_agent_grant(args: &Args) -> Result<(), String> {
    use cahoots_host::{agentkey, grants};
    let home = agentkey::dir();
    // The file named last, for `show`, `verify` and `forget`.
    let file = || -> Result<PathBuf, String> {
        args.0
            .get(1)
            .filter(|f| !f.starts_with("--"))
            .map(PathBuf::from)
            .ok_or_else(|| "name the grant file after the command".into())
    };
    let read = |path: &Path| std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    match args.0.first().map(String::as_str) {
        Some("request") => {
            let circle = args.req("--circle");
            let budget: u32 = args
                .req("--budget")
                .parse()
                .map_err(|_| "--budget: a number")?;
            let standing = args.has("--standing");
            let until = match (args.get("--until"), standing) {
                (Some(d), false) => Some(
                    grants::parse_date(d)
                        .ok_or("--until: a date as YYYY-MM-DD, from 1970 on")?,
                ),
                (None, true) => None,
                _ => return Err("say when the grant ends: --until YYYY-MM-DD, or --standing for until revoked".into()),
            };
            let key = agentkey::load_or_create(&home)?;
            let mut badge = args.badge()?;
            eprintln!("the badge asks its holder to allow the grant");
            let stored = grants::request(
                &mut badge,
                &circle,
                &args.req("--agent"),
                &args.req("--author"),
                args.get("--location"),
                budget,
                until,
                standing,
                &key,
            )?;
            let path = grants::save(&home, &stored)?;
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&grants::show(&stored.bytes)?).unwrap_or_default()
            );
            println!("{}", path.display());
            Ok(())
        }
        Some("list") => {
            let all = grants::load_all(&home);
            if all.is_empty() {
                eprintln!("no grants in {}", home.join("grants").display());
            }
            for s in all {
                let budget = grants::show(&s.bytes)
                    .ok()
                    .and_then(|j| j["authorization_details"][0]["budget"].as_u64());
                let left = match (s.remaining, budget) {
                    (Some(n), Some(b)) => format!("{n} left of {b}"),
                    (Some(n), None) => format!("{n} left"),
                    (None, Some(b)) => format!("? left of {b}"),
                    (None, None) => "? left".into(),
                };
                println!("{}  {}  {}  {left}", s.path(&home).display(), s.circle, s.agent);
            }
            Ok(())
        }
        Some("show") => {
            let shown = grants::show(&read(&file()?)?)?;
            println!("{}", serde_json::to_string_pretty(&shown).unwrap_or_default());
            Ok(())
        }
        Some("verify") => {
            let bytes = read(&file()?)?;
            let circle = args.req("--circle");
            let manifest = cahoots_host::roster::manifest(Path::new(&args.req("--state")), &circle)?;
            let seat = grants::seat_of(&bytes)?;
            grants::verify(&bytes, &manifest, seat)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&grants::show(&bytes)?).unwrap_or_default()
            );
            eprintln!("the grant verifies: signed by seat {seat} of {circle}, under the key in the circle's manifest");
            Ok(())
        }
        Some("forget") => {
            let gone = grants::forget(&file()?)?;
            eprintln!("forgot {} and the .json beside it", gone.display());
            Ok(())
        }
        _ => Err("grant request, grant list, grant show, grant verify or grant forget".into()),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
