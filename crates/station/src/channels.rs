//! The `channels` feature: version 3 messages, each appended to its channel's stream. `seq` is
//! global, as for circles, and the hash chain runs through every entry; a channel's head is its
//! stream's last entry. Posts take from the channel's own rate bucket as well as the client's.

use crate::limits::rate_limited;
use crate::log::{Entry, StreamSummary};
use crate::objects::store;
use crate::rules::{RuleError, Submission};
use crate::server::{err, head_json, now_ms, poll_stream, PollQuery, State};
use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;

/// The channel message routes.
pub fn routes(router: Router<State>) -> Router<State> {
    router
        .route("/v1/channels", get(list))
        .route(
            "/v1/channels/:channel/messages",
            get(get_messages).post(post_message),
        )
        .route("/v1/channels/:channel/head", get(channel_head))
}

/// A channel entry on the wire. Bytes are base64url, ids and hashes hex.
#[derive(Serialize)]
struct ChannelEntryRecord {
    seq: u64,
    received_at: u64,
    channel_id: String,
    message: String,
    hash: String,
}

impl From<&Entry> for ChannelEntryRecord {
    fn from(e: &Entry) -> Self {
        ChannelEntryRecord {
            seq: e.seq,
            received_at: e.received_at,
            channel_id: hex::encode(e.circle_id),
            message: URL_SAFE_NO_PAD.encode(&e.message),
            hash: hex::encode(e.hash),
        }
    }
}

fn parse_channel(hex_id: &str) -> Result<[u8; 16], Box<Response>> {
    hex::decode(hex_id)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| {
            Box::new(err(
                StatusCode::BAD_REQUEST,
                "channel_id",
                "expected 32 hex chars",
            ))
        })
}

/// `POST /v1/channels/{channel}/messages`: the body must be a version 3
/// message for the channel and the channel must be held; then the size cap, with `gate` the
/// grant checks ([`crate::gate::check_post`]), the channel's rate bucket, and the append.
async fn post_message(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    let channel = match parse_channel(&channel) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let sub = match Submission::parse_channel(&channel, &body) {
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
        match store::channel(log.conn(), &channel) {
            Ok(Some(_)) => {}
            Ok(None) => return err(StatusCode::NOT_FOUND, "channel", "no such channel"),
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
        }
        if let Some(full) = st.full(&log) {
            return full;
        }
        #[cfg(feature = "gate")]
        if let Err(refused) = crate::gate::check_post(&st, &mut log, &channel, &body) {
            return refused;
        }
        // Only a post that would be stored spends the channel's tokens, so junk, a full relay
        // and a refused grant cannot use them up.
        if !st.limiter.take_channel_post(&channel) {
            return rate_limited("too many posts to this channel; try again in a few seconds");
        }
        let result = log.append(sub, now_ms());
        // Counted under the same lock as the append, so the budget and replay checks above
        // see it.
        #[cfg(feature = "gate")]
        if result.is_ok() {
            if let Ok(h) = cahoots_channel::message::Header::parse(&body) {
                st.gate.counted(&channel, &h);
            }
        }
        result
    };
    st.appended(result)
}

/// `GET /v1/channels/{channel}/messages?after&wait`: as for circles, over the channel's stream.
async fn get_messages(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    Query(q): Query<PollQuery>,
    #[cfg(feature = "gate")] creds: crate::gate::Credentials,
) -> Response {
    let channel = match parse_channel(&channel) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    #[cfg(feature = "gate")]
    {
        let mut log = st.log.lock().unwrap();
        if let Err(refused) =
            crate::gate::check_read(&st, &mut log, &channel, &creds, crate::gate::Need::Read)
        {
            return refused;
        }
    }
    poll_stream(&st, &channel, &q, |e| ChannelEntryRecord::from(e)).await
}

async fn channel_head(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    #[cfg(feature = "gate")] creds: crate::gate::Credentials,
) -> Response {
    let channel = match parse_channel(&channel) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    #[allow(unused_mut)]
    let mut log = st.log.lock().unwrap();
    #[cfg(feature = "gate")]
    if let Err(refused) =
        crate::gate::check_read(&st, &mut log, &channel, &creds, crate::gate::Need::Read)
    {
        return refused;
    }
    head_json(log.stream_head(&channel))
}

/// `GET /v1/channels`: every channel the relay holds, with its entry count and last `seq`
/// (`0` and `null` before its first message). The most recently written first.
async fn list(AxState(st): AxState<State>) -> Response {
    let mut log = st.log.lock().unwrap();
    let ids = match store::channel_ids(log.conn()) {
        Ok(ids) => ids,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    };
    let streams: HashMap<[u8; 16], StreamSummary> =
        log.streams().into_iter().map(|s| (s.stream, s)).collect();
    let mut rows: Vec<(Option<u64>, serde_json::Value)> = ids
        .iter()
        .map(|id| {
            let s = streams.get(id);
            let last_seq = s.map(|s| s.last_seq);
            let row = json!({
                "channel_id": hex::encode(id),
                "count": s.map_or(0, |s| s.count),
                "last_seq": last_seq,
            });
            (last_seq, row)
        })
        .collect();
    // `None` sorts below every `Some`, so channels without messages come last.
    rows.sort_by_key(|(last, _)| std::cmp::Reverse(*last));
    let channels: Vec<_> = rows.into_iter().map(|(_, row)| row).collect();
    Json(json!({ "channels": channels })).into_response()
}
