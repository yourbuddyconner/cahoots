//! `cahoots-wg`: issue and accept WireGuard pre-shared keys from a circle pad held in a
//! `cahoots-provider` file directory.
//!
//! ```text
//! # Peer A
//! cahoots-wg --dir ~/.cahoots issue --circle <hex> --peer <B's pubkey>
//!   -> prints the handle token for B and a [Peer] section for A's config
//! # Peer B
//! cahoots-wg --dir ~/.cahoots accept --handle cahoots-wg:... --peer <A's pubkey>
//!   -> prints a [Peer] section for B's config
//! # Either, to install live:
//! cahoots-wg --dir ~/.cahoots accept --handle ... --raw | wg set wg0 peer <pk> preshared-key /dev/stdin
//! ```

use cahoots_provider::{FileProvider, PadProvider};
use cahoots_wireguard::{accept, issue, parse_handle_token, render_peer, rotations_remaining};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cahoots-wg",
    about = "WireGuard pre-shared keys from a Cahoots circle pad"
)]
struct Cli {
    /// Provider directory (created by the ceremony tooling).
    #[arg(long, env = "CAHOOTS_DIR")]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List circles in the provider directory with their remaining rotation budget.
    Circles,
    /// Spend 32 pad bytes and print a handle token for the peer.
    Issue {
        /// Circle ID, hex.
        #[arg(long)]
        circle: String,
        /// Peer's WireGuard public key. Renders a [Peer] section when given.
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        allowed_ips: Vec<String>,
        #[arg(long)]
        endpoint: Option<String>,
        /// Print only the base64 PSK, for piping into `wg set`.
        #[arg(long)]
        raw: bool,
    },
    /// Accept a handle token from the peer and derive the same PSK.
    Accept {
        #[arg(long)]
        handle: String,
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        allowed_ips: Vec<String>,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        raw: bool,
    },
}

fn parse_circle(hex_id: &str) -> Result<[u8; 16], String> {
    let bytes = hex::decode(hex_id).map_err(|e| format!("circle id: {e}"))?;
    <[u8; 16]>::try_from(bytes).map_err(|_| "circle id must be 16 bytes".to_string())
}

fn print_result(
    psk: &str,
    peer: Option<&str>,
    allowed_ips: &[String],
    endpoint: Option<&str>,
    raw: bool,
) {
    if raw || peer.is_none() {
        println!("{psk}");
        return;
    }
    let ips: Vec<&str> = allowed_ips.iter().map(String::as_str).collect();
    print!(
        "{}",
        render_peer(peer.unwrap_or_default(), psk, &ips, endpoint, Some(25))
    );
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let mut provider = FileProvider::open(&cli.dir).map_err(|e| format!("{e:?}"))?;
    match cli.cmd {
        Cmd::Circles => {
            for id in provider.circles() {
                let name = provider
                    .params(&id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                println!(
                    "{}  {name}  rotations left: {}{}",
                    hex::encode(id),
                    rotations_remaining(&provider, &id),
                    if provider.low_pad(&id) {
                        "  (LOW PAD: hold a new ceremony)"
                    } else {
                        ""
                    }
                );
            }
        }
        Cmd::Issue {
            circle,
            peer,
            allowed_ips,
            endpoint,
            raw,
        } => {
            let id = parse_circle(&circle)?;
            let issued = issue(&mut provider, &id).map_err(|e| format!("{e:?}"))?;
            eprintln!("handle for peer: {}", issued.handle_token());
            print_result(
                &issued.preshared_key,
                peer.as_deref(),
                &allowed_ips,
                endpoint.as_deref(),
                raw,
            );
            if provider.low_pad(&id) {
                eprintln!("warning: 10% or less of this slice remains. Hold a new ceremony.");
            }
        }
        Cmd::Accept {
            handle,
            peer,
            allowed_ips,
            endpoint,
            raw,
        } => {
            let h = parse_handle_token(&handle).map_err(|e| format!("{e:?}"))?;
            let psk = accept(&mut provider, &h).map_err(|e| format!("{e:?}"))?;
            print_result(
                &psk,
                peer.as_deref(),
                &allowed_ips,
                endpoint.as_deref(),
                raw,
            );
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
