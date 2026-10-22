//! `cahoots-mesh`: a Meshtastic channel keyed from a Cahoots circle.
//!
//! One member runs `issue`; everyone else runs `accept`. The handle travels over the Numbers
//! Station as a `psk` control message, or by paste. Each side prints the channel URL for the
//! Meshtastic app and the CLI command for a node on a cable.

use cahoots_core::control::EVERYONE;
use cahoots_meshtastic::{accept, issue, issue_on_feed, Channel, PURPOSE};
use cahoots_provider::FileProvider;
use cahoots_relay::{HttpRelay, Relay};
use cahoots_wireguard::{sync, FeedItem, FeedState, PskToken};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "cahoots-mesh",
    about = "Meshtastic channel keys from a Cahoots circle pad"
)]
struct Cli {
    /// Provider directory (from a ceremony export or `cahoots-chat local-ceremony`).
    #[arg(long, env = "CAHOOTS_DIR")]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Spend 32 pad bytes for a new channel key and tell the circle. Rotation is the same command again.
    Issue {
        #[arg(long)]
        circle: String,
        /// Channel name shown in Meshtastic. Defaults to the circle's name.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        relay: Option<String>,
        /// Meshtastic channel index for the printed command.
        #[arg(long, default_value_t = 0)]
        index: u8,
        /// Print only the base64 key.
        #[arg(long)]
        raw: bool,
    },
    /// Accept the circle's channel key: a pasted token, or the newest one on the feed.
    Accept {
        #[arg(long)]
        circle: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        handle: Option<String>,
        #[arg(long)]
        relay: Option<String>,
        #[arg(long, default_value_t = 0)]
        index: u8,
        #[arg(long)]
        raw: bool,
    },
}

fn parse_circle(hex_id: &str) -> Result<[u8; 16], String> {
    let bytes = hex::decode(hex_id).map_err(|e| format!("circle id: {e}"))?;
    <[u8; 16]>::try_from(bytes).map_err(|_| "circle id must be 16 bytes".to_string())
}

fn state_path(dir: &Path, circle: &[u8; 16]) -> PathBuf {
    dir.join(hex::encode(circle)).join("mesh.json")
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

fn show(ch: &Channel, index: u8, raw: bool) {
    if raw {
        println!("{}", *ch.key_base64());
        return;
    }
    println!("channel: {}", ch.name);
    println!("key:     {}", *ch.key_base64());
    println!("url:     {}", *ch.url());
    println!("cli:     {}", *ch.cli_command(index));
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Issue {
            circle,
            name,
            relay,
            index,
            raw,
        } => {
            let id = parse_circle(&circle)?;
            let mut pad = FileProvider::open_pad(&cli.dir, &id).map_err(|e| format!("{e:?}"))?;
            let name = name.unwrap_or_else(|| pad.params().name.clone());
            let ch = match relay {
                Some(url) => {
                    let mut relay = HttpRelay::new(&url);
                    let mut state = load_state(&cli.dir, &id);
                    sync(&mut pad, &mut relay, &mut state).map_err(|e| e.to_string())?;
                    let (ch, wire) = issue_on_feed(&mut pad, &name).map_err(|e| e.to_string())?;
                    let seq = relay.post(&id, &wire).map_err(|e| e.to_string())?;
                    save_state(&cli.dir, &id, &state)?;
                    eprintln!("channel key posted to the feed (seq {seq})");
                    ch
                }
                None => {
                    let ch = issue(&mut pad, &id, &name).map_err(|e| e.to_string())?;
                    eprintln!("handle for the circle: {}", ch.token.encode());
                    ch
                }
            };
            show(&ch, index, raw);
        }
        Cmd::Accept {
            circle,
            name,
            handle,
            relay,
            index,
            raw,
        } => {
            let id = parse_circle(&circle)?;
            let mut pad = FileProvider::open_pad(&cli.dir, &id).map_err(|e| format!("{e:?}"))?;
            let name = name.unwrap_or_else(|| pad.params().name.clone());
            let token = match (handle, relay) {
                (Some(h), _) => PskToken::parse(&h).map_err(|e| e.to_string())?,
                (None, Some(url)) => {
                    let mut state = load_state(&cli.dir, &id);
                    let items = sync(&mut pad, &mut HttpRelay::new(&url), &mut state)
                        .map_err(|e| e.to_string())?;
                    save_state(&cli.dir, &id, &state)?;
                    // The newest channel key wins: everyone rotates to the last one issued.
                    items
                        .into_iter()
                        .filter_map(|i| match i {
                            FeedItem::Psk(t) if t.purpose == PURPOSE && t.to == EVERYONE => Some(t),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .pop()
                        .ok_or("no channel key on the feed yet")?
                }
                (None, None) => return Err("give --handle <token> or --relay <url>".into()),
            };
            let ch = accept(&mut pad, &token, &name).map_err(|e| e.to_string())?;
            eprintln!(
                "accepted the channel key issued by seat {}",
                token.from_seat() + 1
            );
            show(&ch, index, raw);
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("cahoots-mesh: {e}");
        std::process::exit(1);
    }
}
