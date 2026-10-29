//! HTTP surface over the log. Long-polling is a `tokio::sync::Notify` that fires on every
//! append; pollers re-check the log and return as soon as something new exists.

use crate::limits::{self, Limiter, Limits};
use crate::log::{EntryRecord, Log, LogError};
use crate::rules::{RuleError, Submission, MAX_BODY};
use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxState};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cahoots_core::circle_id::CircleId;
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

pub const MAX_WAIT_MS: u64 = 30_000;
const PAGE_LIMIT: usize = 500;

#[derive(Clone)]
pub struct State {
    log: Arc<Mutex<Log>>,
    notify: Arc<Notify>,
    limiter: Limiter,
    /// Refuse posts once the log file is this large. `None` for no cap.
    max_log_bytes: Option<u64>,
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
        Ok(State {
            log: Arc::new(Mutex::new(Log::open(path)?)),
            notify: Arc::new(Notify::new()),
            limiter: Limiter::new(limits),
            max_log_bytes,
        })
    }

    pub fn limits(&self) -> Limits {
        self.limiter.limits()
    }
}

pub fn app(state: State) -> Router {
    let limiter = state.limiter.clone();
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(|| async { "ok" }))
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
                .allow_methods([http::Method::GET, http::Method::POST, http::Method::OPTIONS])
                .allow_headers([http::header::CONTENT_TYPE]),
        )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn err(status: StatusCode, code: &str, detail: impl ToString) -> Response {
    (
        status,
        Json(json!({ "error": code, "detail": detail.to_string() })),
    )
        .into_response()
}

fn parse_circle(hex_id: &str) -> Result<CircleId, Box<Response>> {
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
    let result = {
        let mut log = st.log.lock().unwrap();
        if st.max_log_bytes.is_some_and(|cap| log.bytes() >= cap) {
            return err(
                StatusCode::INSUFFICIENT_STORAGE,
                "full",
                "this relay's log has reached its size cap; the operator must grow it",
            );
        }
        log.append(sub, now_ms())
    };
    match result {
        Ok(entry) => {
            st.notify.notify_waiters();
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

#[derive(Deserialize)]
pub struct PollQuery {
    after: Option<u64>,
    wait: Option<u64>,
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
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(q.wait.unwrap_or(0).min(MAX_WAIT_MS));
    loop {
        // Register interest before checking, so an append between check and wait is not lost.
        let notified = st.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let (entries, head): (Vec<EntryRecord>, _) = {
            let log = st.log.lock().unwrap();
            let e = log
                .entries_after(&circle, q.after)
                .into_iter()
                .take(PAGE_LIMIT)
                .map(EntryRecord::from)
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

async fn feed(AxState(st): AxState<State>, Query(q): Query<PollQuery>) -> Response {
    let log = st.log.lock().unwrap();
    let entries: Vec<EntryRecord> = log
        .all_after(q.after, PAGE_LIMIT)
        .into_iter()
        .map(EntryRecord::from)
        .collect();
    Json(json!({ "entries": entries, "head_seq": log.head().seq })).into_response()
}

fn head_json(h: crate::log::Head) -> Response {
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
    let h = st.log.lock().unwrap().circle_head(&circle);
    head_json(h)
}

async fn circles(AxState(st): AxState<State>) -> Response {
    let c = st.log.lock().unwrap().circles();
    Json(json!({ "circles": c })).into_response()
}

async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}
