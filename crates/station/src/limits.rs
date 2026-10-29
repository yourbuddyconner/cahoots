//! What a public relay needs before it faces the internet: a per-client rate limit and a cap
//! on the log file. Both are plain and in-process, so a single station needs no proxy to be
//! safe to expose. The limit is a token bucket per client address: a client gets `per_minute`
//! requests of each kind in a burst and then that many per minute.

use axum::extract::{ConnectInfo, Request, State as AxState};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Rate limits for one station. `None` for a kind means that kind is not limited.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Accepted posts per client per minute.
    pub posts_per_minute: Option<u32>,
    /// Accepted reads (every GET under `/v1`) per client per minute.
    pub reads_per_minute: Option<u32>,
    /// Take the client address from `Fly-Client-IP` or the first `X-Forwarded-For` entry.
    /// Only behind a proxy that sets them; otherwise any client can pick its own bucket.
    pub trust_forwarded: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            posts_per_minute: Some(120),
            reads_per_minute: Some(600),
            trust_forwarded: false,
        }
    }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Clone)]
pub struct Limiter {
    limits: Limits,
    posts: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    reads: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
}

const PRUNE_ABOVE: usize = 10_000;

impl Limiter {
    pub fn new(limits: Limits) -> Self {
        Limiter {
            limits,
            posts: Arc::new(Mutex::new(HashMap::new())),
            reads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Spend one token for `ip` from a bucket of `per_minute`. `true` when the request may go on.
    fn take(buckets: &Mutex<HashMap<IpAddr, Bucket>>, ip: IpAddr, per_minute: u32) -> bool {
        let cap = per_minute as f64;
        let now = Instant::now();
        let mut map = buckets.lock().unwrap();
        if map.len() > PRUNE_ABOVE {
            map.retain(|_, b| now.duration_since(b.last).as_secs() < 120);
        }
        let b = map.entry(ip).or_insert(Bucket {
            tokens: cap,
            last: now,
        });
        let refill = now.duration_since(b.last).as_secs_f64() * cap / 60.0;
        b.tokens = (b.tokens + refill).min(cap);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn client_ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> IpAddr {
        if self.limits.trust_forwarded {
            let forwarded = headers
                .get("fly-client-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .or_else(|| {
                    headers
                        .get("x-forwarded-for")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.split(',').next())
                        .map(str::trim)
                });
            if let Some(ip) = forwarded.and_then(|s| s.parse().ok()) {
                return ip;
            }
        }
        // Without connection info (a test that serves the router directly) every client
        // shares one bucket; that is strict rather than lax.
        peer.map(|p| p.ip())
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    }
}

/// Middleware: refuse with `429 {"error":"rate"}` when the client is over its limit.
pub async fn limit(AxState(limiter): AxState<Limiter>, req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let ip = limiter.client_ip(req.headers(), peer);
    let under_v1 = req.uri().path().starts_with("/v1/");
    let allowed = match (req.method(), under_v1) {
        (&Method::POST, true) => limiter
            .limits
            .posts_per_minute
            .is_none_or(|n| Limiter::take(&limiter.posts, ip, n)),
        (&Method::GET, true) => limiter
            .limits
            .reads_per_minute
            .is_none_or(|n| Limiter::take(&limiter.reads, ip, n)),
        _ => true,
    };
    if !allowed {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "10")],
            Json(json!({ "error": "rate", "detail": "too many requests from this client; try again in a few seconds" })),
        )
            .into_response();
    }
    next.run(req).await
}
