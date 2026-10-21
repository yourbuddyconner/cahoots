use cahoots_yk::health::chi_square_bytes;
use cahoots_yk::pcsc_card::{readers, PcscCard};
use cahoots_yk::OpenPgp;
use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "cahoots-yk",
    about = "YubiKey randomness over OpenPGP GET CHALLENGE"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List readers and the card's maximum GET CHALLENGE length.
    Info,
    /// Read N random bytes from the card and write them raw.
    Random {
        #[arg(long)]
        bytes: usize,
        /// Output file. Without it, bytes go to stdout, which must not be a terminal.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Measure throughput for N bytes and print a byte-histogram chi-square.
    Bench {
        #[arg(long, default_value_t = 262_144)]
        bytes: usize,
    },
}

fn open() -> Result<OpenPgp<PcscCard>, Box<dyn std::error::Error>> {
    let card = PcscCard::connect_first()?;
    eprintln!("reader: {}", card.reader());
    Ok(OpenPgp::open(card)?)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().cmd {
        Cmd::Info => {
            let list = readers()?;
            if list.is_empty() {
                println!("no readers");
                return Ok(());
            }
            for r in &list {
                println!("reader: {r}");
            }
            match PcscCard::connect_first() {
                Ok(card) => {
                    let pgp = OpenPgp::open(card)?;
                    println!(
                        "openpgp: selected, max challenge {} bytes",
                        pgp.max_challenge()
                    );
                }
                Err(e) => println!("card: {e}"),
            }
        }
        Cmd::Random { bytes, out } => {
            if out.is_none() && std::io::stdout().is_terminal() {
                return Err(
                    "refusing to write raw bytes to a terminal; use --out or a pipe".into(),
                );
            }
            let mut pgp = open()?;
            let mut buf = vec![0u8; bytes];
            pgp.random(&mut buf)?;
            match out {
                Some(p) => std::fs::write(&p, &buf)?,
                None => {
                    let mut o = std::io::stdout().lock();
                    o.write_all(&buf)?;
                    o.flush()?;
                }
            }
            eprintln!("wrote {bytes} bytes");
        }
        Cmd::Bench { bytes } => {
            let mut pgp = open()?;
            let mut buf = vec![0u8; bytes];
            let t = Instant::now();
            pgp.random(&mut buf)?;
            let secs = t.elapsed().as_secs_f64();
            println!(
                "{bytes} bytes in {secs:.2} s = {:.1} KiB/s (chunk {} bytes)",
                bytes as f64 / 1024.0 / secs,
                pgp.max_challenge()
            );
            println!(
                "byte histogram chi-square (255 dof): {:.1}",
                chi_square_bytes(&buf)
            );
        }
    }
    Ok(())
}
