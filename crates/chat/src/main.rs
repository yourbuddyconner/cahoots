//! `cahoots-chat`: the chat demo. See `crates/chat/README.md`.

use cahoots_chat::cli::ceremony::local_ceremony;
use cahoots_chat::cli::client::{Client, Event};
use cahoots_chat::cli::relay::HttpRelay;
use clap::{Parser, Subcommand};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "cahoots-chat",
    about = "End-to-end encrypted chat demo, keyed by the circle root"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a ceremony for N members in this process and write one circle directory each.
    LocalCeremony {
        #[arg(long, default_value_t = 3)]
        members: u8,
        #[arg(long, default_value_t = 65536)]
        pad_len: u32,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "demo")]
        name: String,
    },
    /// Chat from one member directory through a relay.
    Run {
        #[arg(long)]
        circle: PathBuf,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        relay: String,
        #[arg(long, default_value = "anon")]
        name: String,
        /// Start a new epoch after this many of our own messages.
        #[arg(long, default_value_t = 1000)]
        rotate_after: u32,
    },
    /// Post a message with one flipped ciphertext byte so readers can show the rejection.
    TamperTest {
        #[arg(long)]
        circle: PathBuf,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        relay: String,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn print_event(ev: &Event) {
    match ev {
        Event::Own { .. } => {}
        Event::Received { sender, text, .. } => println!("[#{sender}] {text}"),
        Event::Rejected { seq, error } => println!("[rejected seq {seq}: {error:?}]"),
        Event::Unreadable { seq, sender } => println!("[seq {seq} from #{sender}: not text]"),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().cmd {
        Cmd::LocalCeremony {
            members,
            pad_len,
            out,
            name,
        } => {
            let r = local_ceremony(members, pad_len, &name, &out).map_err(|e| e.to_string())?;
            println!("circle id   {}", hex::encode(r.circle_id));
            println!("check words {}", r.check_words.join(" "));
            println!("label       CSPRNG (development ceremony, no hardware TRNG)");
            for d in r.member_dirs {
                println!("member dir  {}", d.display());
            }
        }
        Cmd::TamperTest { circle, relay } => {
            let mut c =
                Client::open(&circle, HttpRelay::new(&relay), 1000).map_err(|e| e.to_string())?;
            c.sync().map_err(|e| e.to_string())?;
            let seq = c
                .post_tampered("this message was tampered with")
                .map_err(|e| e.to_string())?;
            println!("posted tampered message as seq {seq}; readers must reject it");
        }
        Cmd::Run {
            circle,
            relay,
            name,
            rotate_after,
        } => {
            let mut client = Client::open(&circle, HttpRelay::new(&relay), rotate_after)
                .map_err(|e| e.to_string())?;
            // Read the relay to the end, our own messages included, before the first send.
            let backlog = client.sync().map_err(|e| e.to_string())?;
            println!("end-to-end encrypted, keyed by the circle root");
            println!(
                "circle   {}  label {}",
                hex::encode(client.circle_id()),
                client.label()
            );
            println!(
                "member   #{}  pad remaining {} bytes",
                client.my_index(),
                client.remaining()
            );
            match client.current_epoch() {
                Some(e) => println!("epoch    {}", e.offset),
                None => println!("epoch    none yet (first send starts one)"),
            }
            for ev in &backlog {
                print_event(ev);
            }
            println!("type a line and press enter; ctrl-d to quit");
            let client = Arc::new(Mutex::new(client));
            let poller = client.clone();
            std::thread::spawn(move || loop {
                let r = poller.lock().unwrap().poll(0);
                match r {
                    Ok(events) => {
                        for ev in &events {
                            print_event(ev);
                        }
                        let _ = std::io::stdout().flush();
                    }
                    Err(e) => eprintln!("[poll error: {e}]"),
                }
                std::thread::sleep(Duration::from_millis(500));
            });
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let text = format!("{name}: {line}");
                let mut c = client.lock().unwrap();
                if let Err(e) = c.send_line(&text) {
                    println!("[send failed: {e}]");
                }
            }
        }
    }
    Ok(())
}
