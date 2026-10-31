//! HTTP surface over the log. Long-polling is a `tokio::sync::Notify` that fires on every
//! append; pollers re-check the log and return as soon as something new exists.

use crate::limits::{self, Limiter, Limits};
use crate::log::{Entry, EntryRecord, Log, LogError};
use crate::rules::{RuleError, Submission, MAX_BODY};
use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxState};
use axum::http::StatusCode;
#[cfg(feature = "feed-page")]
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cahoots_core::circle_id::CircleId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

pub const MAX_WAIT_MS: u64 = 30_000;
const PAGE_LIMIT: usize = 500;

#[derive(Clone)]
pub struct State {
    pub(crate) log: Arc<Mutex<Log>>,
    pub(crate) notify: Arc<Notify>,
    pub(crate) limiter: Limiter,
    /// Refuse posts once the database is this large. `None` for no cap.
    max_log_bytes: Option<u64>,
    /// Nonces for read proofs and each key's post count.
    #[cfg(feature = "gate")]
    pub(crate) gate: Arc<crate::gate::Gate>,
}

impl State {
    pub fn open(path: &std::path::Path) -> Result<Self, LogError> {
        Self::open_with(path, Limits::default(), None)
    }

    pub fn open_with(
        path: &std::path::Path,
        limits: Limits,
        max_log_bytes: Option<u64>,
    ) -> Result<Self, LogError> {
        let log = Log::open(path)?;
        Ok(State {
            #[cfg(feature = "gate")]
            gate: Arc::new(crate::gate::Gate::from_log(&log)),
            log: Arc::new(Mutex::new(log)),
            notify: Arc::new(Notify::new()),
            limiter: Limiter::new(limits),
            max_log_bytes,
        })
    }

    pub fn limits(&self) -> Limits {
        self.limiter.limits()
    }

    /// `507 {"error":"full"}` once the database has reached its size cap.
    pub(crate) fn full(&self, log: &Log) -> Option<Response> {
        self.max_log_bytes
            .is_some_and(|cap| log.bytes() >= cap)
            .then(|| {
                err(
                    StatusCode::INSUFFICIENT_STORAGE,
                    "full",
                    "this relay's log has reached its size cap; the operator must grow it",
                )
            })
    }
}

pub fn app(state: State) -> Router {
    let limiter = state.limiter.clone();
    let router = Router::new();
    #[cfg(feature = "feed-page")]
    let router = router.route("/", get(index));
    #[cfg(feature = "objects")]
    let router = crate::objects::routes(router);
    #[cfg(feature = "channels")]
    let router = crate::channels::routes(router);
    #[cfg(feature = "gate")]
    let router = crate::gate::routes(router);
    router
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/info", get(info))
        .route("/v1/head", get(global_head))
        .route("/v1/circles", get(circles))
        .route(
            "/v1/circles/:circle/messages",
            post(post_message).get(get_messages),
        )
        .route("/v1/circles/:circle/head", get(circle_head))
        .route("/v1/feed", get(feed))
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(limiter, limits::limit))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY + 1))
        // The feed is public by design: ciphertext only. Any origin may read and post.
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(cors_methods())
                .allow_headers(cors_headers()),
        )
}

/// The methods browsers may use: the object endpoints add PUT and DELETE.
fn cors_methods() -> Vec<http::Method> {
    #[allow(unused_mut)]
    let mut methods = vec![http::Method::GET, http::Method::POST, http::Method::OPTIONS];
    #[cfg(feature = "objects")]
    methods.extend([http::Method::PUT, http::Method::DELETE]);
    methods
}

/// The request headers browsers may send: the gate adds a reader's credentials.
fn cors_headers() -> Vec<http::HeaderName> {
    #[allow(unused_mut)]
    let mut headers = vec![http::header::CONTENT_TYPE];
    #[cfg(feature = "gate")]
    headers.extend(
        [
            crate::gate::GATE_HEADER,
            crate::gate::PROOF_HEADER,
            crate::gate::KID_HEADER,
        ]
        .map(http::HeaderName::from_static),
    );
    headers
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn err(status: StatusCode, code: &str, detail: impl ToString) -> Response {
    (
        status,
        Json(json!({ "error": code, "detail": detail.to_string() })),
    )
        .into_response()
}

pub(crate) fn parse_circle(hex_id: &str) -> Result<CircleId, Box<Response>> {
    hex::decode(hex_id)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| {
            Box::new(err(
                StatusCode::BAD_REQUEST,
                "circle_id",
                "expected 32 hex chars",
            ))
        })
}

async fn post_message(
    AxState(st): AxState<State>,
    Path(circle): Path<String>,
    body: Bytes,
) -> Response {
    let circle = match parse_circle(&circle) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let result = {
        // One lock across the channel check and the append, so a channel stored in between
        // cannot gain a circle entry.
        let mut log = st.log.lock().unwrap();
        if let Some(refused) = channel_not_circle(&mut log, &circle) {
            return refused;
        }
        let sub = match Submission::parse(&circle, &body) {
            Ok(s) => s,
            Err(RuleError::TooLarge) => {
                return err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    RuleError::TooLarge,
                )
            }
            Err(e) => return err(StatusCode::BAD_REQUEST, "malformed", e),
        };
        if let Some(full) = st.full(&log) {
            return full;
        }
        log.append(sub, now_ms())
    };
    st.appended(result)
}

/// With `channels`, an id the `channels` table holds is a channel, and the circle routes
/// refuse it with `404 channel`, so its stream is reached only through the channel routes,
/// their rate bucket and their gate.
#[cfg(feature = "channels")]
fn channel_not_circle(log: &mut Log, id: &CircleId) -> Option<Response> {
    match crate::objects::store::channel(log.conn(), id) {
        Ok(None) => None,
        Ok(Some(_)) => Some(err(
            StatusCode::NOT_FOUND,
            "channel",
            "this id is a channel; use /v1/channels/...",
        )),
        Err(e) => Some(err(StatusCode::INTERNAL_SERVER_ERROR, "db", e)),
    }
}

#[cfg(not(feature = "channels"))]
fn channel_not_circle(_log: &mut Log, _id: &CircleId) -> Option<Response> {
    None
}

impl State {
    /// Answer an append: wake the pollers and `201 {"seq","hash"}`, or the refusal.
    pub(crate) fn appended(&self, result: Result<Entry, LogError>) -> Response {
        match result {
            Ok(entry) => {
                self.notify.notify_waiters();
                (
                    StatusCode::CREATED,
                    Json(json!({ "seq": entry.seq, "hash": hex::encode(entry.hash) })),
                )
                    .into_response()
            }
            Err(e @ LogError::Reuse { prior_seq, .. }) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "reuse", "prior_seq": prior_seq, "detail": e.to_string() })),
            )
                .into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "log", e),
        }
    }
}

#[derive(Deserialize)]
pub struct PollQuery {
    pub(crate) after: Option<u64>,
    pub(crate) wait: Option<u64>,
}

async fn get_messages(
    AxState(st): AxState<State>,
    Path(circle): Path<String>,
    Query(q): Query<PollQuery>,
) -> Response {
    let circle = match parse_circle(&circle) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Some(refused) = channel_not_circle(&mut st.log.lock().unwrap(), &circle) {
        return refused;
    }
    poll_stream(&st, &circle, &q, |e| EntryRecord::from(e)).await
}

/// `{"entries", "head_seq"}` for `stream` after `q.after`, waiting up to `q.wait` ms (at most
/// [`MAX_WAIT_MS`]) for the first one. `record` gives each entry its wire form.
pub(crate) async fn poll_stream<R: Serialize>(
    st: &State,
    stream: &[u8; 16],
    q: &PollQuery,
    record: fn(&Entry) -> R,
) -> Response {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(q.wait.unwrap_or(0).min(MAX_WAIT_MS));
    loop {
        // Register interest before checking, so an append between check and wait is not lost.
        let notified = st.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let (entries, head): (Vec<R>, _) = {
            let log = st.log.lock().unwrap();
            let e = log
                .entries_after_stream(stream, q.after)
                .into_iter()
                .take(PAGE_LIMIT)
                .map(record)
                .collect();
            (e, log.head().seq)
        };
        if !entries.is_empty() || tokio::time::Instant::now() >= deadline {
            return Json(json!({ "entries": entries, "head_seq": head })).into_response();
        }
        tokio::select! {
            _ = notified => {}
            _ = tokio::time::sleep_until(deadline) => {}
        }
    }
}

/// Every circle entry after `after`. With `channels`, channel entries are left out: the feed
/// and its page are for circles.
async fn feed(AxState(st): AxState<State>, Query(q): Query<PollQuery>) -> Response {
    #[allow(unused_mut)]
    let mut log = st.log.lock().unwrap();
    #[cfg(feature = "channels")]
    let channels = match crate::objects::store::channel_ids(log.conn()) {
        Ok(ids) => ids,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    };
    #[cfg(not(feature = "channels"))]
    let channels: Vec<[u8; 16]> = Vec::new();
    let entries: Vec<EntryRecord> = log
        .all_after(q.after, usize::MAX)
        .into_iter()
        .filter(|e| !channels.contains(&e.circle_id))
        .take(PAGE_LIMIT)
        .map(EntryRecord::from)
        .collect();
    Json(json!({ "entries": entries, "head_seq": log.head().seq })).into_response()
}

pub(crate) fn head_json(h: crate::log::Head) -> Response {
    Json(json!({ "seq": h.seq, "hash": hex::encode(h.hash) })).into_response()
}

async fn global_head(AxState(st): AxState<State>) -> Response {
    let h = st.log.lock().unwrap().head();
    head_json(h)
}

async fn circle_head(AxState(st): AxState<State>, Path(circle): Path<String>) -> Response {
    let circle = match parse_circle(&circle) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let mut log = st.log.lock().unwrap();
    if let Some(refused) = channel_not_circle(&mut log, &circle) {
        return refused;
    }
    head_json(log.circle_head(&circle))
}

async fn circles(AxState(st): AxState<State>) -> Response {
    #[allow(unused_mut)]
    let mut log = st.log.lock().unwrap();
    #[allow(unused_mut)]
    let mut c = log.circles();
    // A channel's stream is not a circle: leave out the ids the channels table holds.
    #[cfg(feature = "channels")]
    match crate::objects::store::channel_ids(log.conn()) {
        Ok(channels) => c.retain(|s| !channels.contains(&s.circle_id)),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
    Json(json!({ "circles": c })).into_response()
}

/// The features compiled in that have code, in a fixed order. `issuer` and `anchor` are
/// declared but do nothing in this version, so they are never reported: an app must not rely
/// on them.
pub fn features() -> Vec<&'static str> {
    [
        ("feed-page", cfg!(feature = "feed-page")),
        ("objects", cfg!(feature = "objects")),
        ("channels", cfg!(feature = "channels")),
        ("gate", cfg!(feature = "gate")),
    ]
    .into_iter()
    .filter_map(|(name, on)| on.then_some(name))
    .collect()
}

async fn info(AxState(st): AxState<State>) -> Response {
    let limits = st.limits();
    #[allow(unused_mut)]
    let mut shown = json!({
        "max_body": MAX_BODY,
        "posts_per_minute": limits.posts_per_minute,
        "reads_per_minute": limits.reads_per_minute,
    });
    #[cfg(feature = "channels")]
    {
        shown["channel_posts_per_minute"] = json!(limits.channel_posts_per_minute);
    }
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "features": features(),
        "limits": shown,
    }))
    .into_response()
}

#[cfg(feature = "feed-page")]
async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}
