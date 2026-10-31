use cahoots_station::limits::Limits;
use cahoots_station::server::{app, State};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The Numbers Station relay.
#[derive(Parser)]
#[command(name = "cahoots-station", version)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    /// SQLite database holding the log. Replayed and chain-verified on start.
    #[arg(long, default_value = "./station.db")]
    db: PathBuf,
    /// Posts accepted per client per minute. 0 for no limit.
    #[arg(long, default_value_t = 120)]
    posts_per_minute: u32,
    /// Reads under /v1 accepted per client per minute. 0 for no limit.
    #[arg(long, default_value_t = 600)]
    reads_per_minute: u32,
    /// Take the client address from Fly-Client-IP or X-Forwarded-For. Only behind a proxy
    /// that sets them.
    #[arg(long)]
    trust_forwarded: bool,
    /// Refuse posts once the database (with its write-ahead log) reaches this many bytes.
    /// 0 for no cap.
    #[arg(long, default_value_t = 0)]
    max_log_bytes: u64,
    /// Message posts accepted per channel per minute, whoever sends them. 0 for no limit.
    #[cfg(feature = "channels")]
    #[arg(long, default_value_t = 300)]
    channel_posts_per_minute: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let limits = Limits {
        posts_per_minute: (args.posts_per_minute > 0).then_some(args.posts_per_minute),
        reads_per_minute: (args.reads_per_minute > 0).then_some(args.reads_per_minute),
        trust_forwarded: args.trust_forwarded,
        #[cfg(feature = "channels")]
        channel_posts_per_minute: (args.channel_posts_per_minute > 0)
            .then_some(args.channel_posts_per_minute),
        #[cfg(not(feature = "channels"))]
        channel_posts_per_minute: None,
    };
    let state = match State::open_with(
        &args.db,
        limits,
        (args.max_log_bytes > 0).then_some(args.max_log_bytes),
    ) {
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
        "numbers station listening on http://{} (db: {})",
        args.listen,
        args.db.display()
    );
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let service = app(state).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service)
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
