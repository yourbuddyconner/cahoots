use cahoots_station::server::{app, State};
use clap::Parser;
use std::path::PathBuf;

/// The Numbers Station relay.
#[derive(Parser)]
#[command(name = "cahoots-station", version)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    /// Append-only JSONL log file. Replayed and chain-verified on start.
    #[arg(long, default_value = "./station.jsonl")]
    log: PathBuf,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let state = match State::open(&args.log) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("refusing to start: {e}");
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(&args.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {}: {e}", args.listen);
            std::process::exit(1);
        }
    };
    eprintln!(
        "numbers station listening on http://{} (log: {})",
        args.listen,
        args.log.display()
    );
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    if let Err(e) = axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
