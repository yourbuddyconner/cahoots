//! `cahoots-wg`: WireGuard pre-shared keys from a Cahoots circle.
//!
//! Two ways to move a key handle to the peer: paste the token, or let the Numbers Station
//! carry it as a `psk` control message next to the `peer` announcements. Either way the
//! relay sees sealed bytes only, and the pad is what makes them trustworthy.

use cahoots_core::pad::Pad;
use cahoots_provider::{FileLedger, FileProvider, FileStorage, PadProvider};
use cahoots_relay::{HttpRelay, Relay};
use cahoots_wireguard::{
    accept, issue, issue_on_feed, overlay_address, overlay_subnet, render_peer,
    rotations_remaining, sync, wg_set_command, FeedItem, FeedState, PskToken,
};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(
    name = "cahoots-wg",
    about = "WireGuard pre-shared keys from a Cahoots circle pad"
)]
struct Cli {
    /// Provider directory (from a ceremony export or `cahoots-chat local-ceremony`).
    #[arg(long, env = "CAHOOTS_DIR")]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args)]
struct Install {
    /// Peer's WireGuard public key. Taken from the feed when known.
    #[arg(long)]
    peer: Option<String>,
    #[arg(long)]
    allowed_ips: Vec<String>,
    #[arg(long)]
    endpoint: Option<String>,
    /// Print only the base64 PSK.
    #[arg(long)]
    raw: bool,
    /// Run `wg set` on this interface instead of printing. Needs root.
    #[arg(long)]
    apply: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List circles in the provider directory with their remaining rotation budget.
    Circles,
    /// This host's address and the circle's subnet, derived from the circle ID.
    Address {
        #[arg(long)]
        circle: String,
    },
    /// Post this host's WireGuard public key, endpoint, and routes to the feed.
    Announce {
        #[arg(long)]
        circle: String,
        #[arg(long)]
        relay: String,
        #[arg(long)]
        public_key: String,
        #[arg(long)]
        endpoint: Option<String>,
        /// Defaults to this host's overlay address as a /32.
        #[arg(long)]
        allowed_ips: Option<String>,
    },
    /// Read the feed and list the peers this host knows.
    Peers {
        #[arg(long)]
        circle: String,
        #[arg(long)]
        relay: String,
    },
    /// Spend 32 pad bytes for the pair (this host, --to) and print the key. With --relay the
    /// handle goes to the feed; otherwise it is printed for the peer to paste.
    Issue {
        #[arg(long)]
        circle: String,
        /// The peer's seat number, as shown in the app (1-based).
        #[arg(long)]
        to: u8,
        #[arg(long)]
        relay: Option<String>,
        #[command(flatten)]
        install: Install,
    },
    /// Accept a handle: a pasted token, or every token on the feed addressed to this host.
    Accept {
        #[arg(long)]
        circle: String,
        #[arg(long)]
        handle: Option<String>,
        #[arg(long)]
        relay: Option<String>,
        #[command(flatten)]
        install: Install,
    },
}

type FilePad = Pad<FileStorage, FileLedger>;

fn parse_circle(hex_id: &str) -> Result<[u8; 16], String> {
    let bytes = hex::decode(hex_id).map_err(|e| format!("circle id: {e}"))?;
    <[u8; 16]>::try_from(bytes).map_err(|_| "circle id must be 16 bytes".to_string())
}

fn state_path(dir: &Path, circle: &[u8; 16]) -> PathBuf {
    dir.join(hex::encode(circle)).join("wg.json")
}
fn load_state(dir: &Path, circle: &[u8; 16]) -> FeedState {
    std::fs::read(state_path(dir, circle))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save_state(dir: &Path, circle: &[u8; 16], state: &FeedState) -> Result<(), String> {
    let path = state_path(dir, circle);
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn open(dir: &Path, circle: &[u8; 16]) -> Result<FilePad, String> {
    FileProvider::open_pad(dir, circle).map_err(|e| format!("{e:?}"))
}

/// Print the key, render a `[Peer]`, or run `wg set`, depending on what the user asked for
/// and what the feed knows about the peer.
fn install(psk: &str, seat: u8, opts: &Install, state: &FeedState) -> Result<(), String> {
    let known = state.peers.get(&seat);
    let peer = opts
        .peer
        .clone()
        .or_else(|| known.map(|p| p.public_key.clone()));
    let endpoint = opts
        .endpoint
        .clone()
        .or_else(|| known.and_then(|p| p.endpoint.clone()));
    let allowed: Vec<String> = if opts.allowed_ips.is_empty() {
        known
            .map(|p| vec![p.allowed_ips.clone()])
            .unwrap_or_default()
    } else {
        opts.allowed_ips.clone()
    };
    if let Some(iface) = &opts.apply {
        let peer = peer
            .ok_or("--apply needs the peer's public key: --peer, or an announcement on the feed")?;
        let allowed_arg = (!allowed.is_empty()).then(|| allowed.join(","));
        let args = wg_set_command(iface, &peer, endpoint.as_deref(), allowed_arg.as_deref());
        let mut child = Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("running wg: {e}"))?;
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(format!("{psk}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("wg set exited with {status}"));
        }
        eprintln!("installed on {iface} for peer {}", &peer[..8]);
        return Ok(());
    }
    if opts.raw || peer.is_none() {
        println!("{psk}");
        return Ok(());
    }
    let ips: Vec<&str> = allowed.iter().map(String::as_str).collect();
    print!(
        "{}",
        render_peer(
            peer.as_deref().unwrap_or_default(),
            psk,
            &ips,
            endpoint.as_deref(),
            Some(25)
        )
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Circles => {
            let provider = FileProvider::open(&cli.dir).map_err(|e| format!("{e:?}"))?;
            for id in provider.circles() {
                let name = provider
                    .params(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let seat = provider.my_slice(&id).map(|s| s + 1).unwrap_or(0);
                println!(
                    "{}  {name}  seat {seat}  address {}  rotations left: {}{}",
                    hex::encode(id),
                    overlay_address(&id, seat.saturating_sub(1)),
                    rotations_remaining(&provider, &id),
                    if provider.low_pad(&id) {
                        "  (LOW PAD: hold a new ceremony)"
                    } else {
                        ""
                    }
                );
            }
        }
        Cmd::Address { circle } => {
            let id = parse_circle(&circle)?;
            let pad = open(&cli.dir, &id)?;
            println!(
                "{}/24  subnet {}",
                overlay_address(&id, pad.my_slice()),
                overlay_subnet(&id)
            );
        }
        Cmd::Announce {
            circle,
            relay,
            public_key,
            endpoint,
            allowed_ips,
        } => {
            let id = parse_circle(&circle)?;
            let mut pad = open(&cli.dir, &id)?;
            let allowed = allowed_ips
                .unwrap_or_else(|| format!("{}/32", overlay_address(&id, pad.my_slice())));
            let wire =
                cahoots_wireguard::announce(&mut pad, &public_key, endpoint.as_deref(), &allowed)
                    .map_err(|e| e.to_string())?;
            let seq = HttpRelay::new(&relay)
                .post(&id, &wire)
                .map_err(|e| e.to_string())?;
            eprintln!(
                "announced seat {} as {allowed} (feed seq {seq})",
                pad.my_slice() + 1
            );
        }
        Cmd::Peers { circle, relay } => {
            let id = parse_circle(&circle)?;
            let mut pad = open(&cli.dir, &id)?;
            let mut state = load_state(&cli.dir, &id);
            sync(&mut pad, &mut HttpRelay::new(&relay), &mut state).map_err(|e| e.to_string())?;
            save_state(&cli.dir, &id, &state)?;
            for p in state.peers.values() {
                println!(
                    "seat {}  {}  {}  {}",
                    p.seat + 1,
                    p.public_key,
                    p.endpoint.as_deref().unwrap_or("-"),
                    p.allowed_ips
                );
            }
            if state.peers.is_empty() {
                eprintln!("no announcements yet");
            }
        }
        Cmd::Issue {
            circle,
            to,
            relay,
            install: opts,
        } => {
            let id = parse_circle(&circle)?;
            let mut pad = open(&cli.dir, &id)?;
            let mut state = load_state(&cli.dir, &id);
            let to_seat = to.checked_sub(1).ok_or("--to is 1-based")?;
            let issued = match relay {
                Some(url) => {
                    let mut relay = HttpRelay::new(&url);
                    sync(&mut pad, &mut relay, &mut state).map_err(|e| e.to_string())?;
                    let (issued, wire) =
                        issue_on_feed(&mut pad, to_seat).map_err(|e| e.to_string())?;
                    let seq = relay.post(&id, &wire).map_err(|e| e.to_string())?;
                    eprintln!("handle posted to the feed (seq {seq})");
                    issued
                }
                None => {
                    let issued = issue(&mut pad, &id, to_seat).map_err(|e| e.to_string())?;
                    eprintln!("handle for seat {to}: {}", issued.token.encode());
                    issued
                }
            };
            save_state(&cli.dir, &id, &state)?;
            install(&issued.preshared_key, to_seat, &opts, &state)?;
            if PadProvider::low_pad(&pad, &id) {
                eprintln!("warning: 10% or less of this slice remains. Hold a new ceremony.");
            }
        }
        Cmd::Accept {
            circle,
            handle,
            relay,
            install: opts,
        } => {
            let id = parse_circle(&circle)?;
            let mut pad = open(&cli.dir, &id)?;
            let mut state = load_state(&cli.dir, &id);
            let me = pad.my_slice();
            let tokens: Vec<PskToken> = match (handle, relay) {
                (Some(h), _) => vec![PskToken::parse(&h).map_err(|e| e.to_string())?],
                (None, Some(url)) => {
                    let items = sync(&mut pad, &mut HttpRelay::new(&url), &mut state)
                        .map_err(|e| e.to_string())?;
                    save_state(&cli.dir, &id, &state)?;
                    items
                        .into_iter()
                        .filter_map(|i| match i {
                            FeedItem::Psk(t)
                                if t.purpose == cahoots_wireguard::PURPOSE
                                    && t.to == me
                                    && !state.accepted.contains(&t.encode()) =>
                            {
                                Some(t)
                            }
                            _ => None,
                        })
                        .collect()
                }
                (None, None) => return Err("give --handle <token> or --relay <url>".into()),
            };
            if tokens.is_empty() {
                eprintln!("nothing to accept");
            }
            for t in tokens {
                if t.to != me {
                    return Err(cahoots_wireguard::WgError::NotForMe { to: t.to }.to_string());
                }
                let psk = accept(&mut pad, &t).map_err(|e| e.to_string())?;
                state.accepted.push(t.encode());
                save_state(&cli.dir, &id, &state)?;
                eprintln!("accepted a key from seat {}", t.from_seat() + 1);
                install(&psk, t.from_seat(), &opts, &state)?;
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("cahoots-wg: {e}");
        std::process::exit(1);
    }
}
