//! The object endpoints. Bodies are raw bytes; answers are JSON with binary fields in base64url
//! without padding and ids in lowercase hex.
//!
//! Each handler's checks return the refusal itself as the error, so `?` ends the request with
//! its answer; the error is a `Response` by design, and lives only until it is returned.
#![allow(clippy::result_large_err)]

use super::store::{self, ChannelRow, GrantRow, PolicyRow, Put};
use crate::log::Log;
use crate::rules::MAX_BODY;
use crate::server::{err, parse_circle, State};
use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxState};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_channel::deposit::Deposit;
use cahoots_channel::{policy, ChannelError, ChannelObject, Policy};
use cahoots_core::cose::{self, CoseError};
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_objects::cwt::{Claims, Kind};
use cahoots_objects::ObjectError;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::MutexGuard;

/// The domain of a gate token statement.
pub const GATE_DOMAIN: &[u8] = b"cahoots/gate/v1";
/// The domain of a revocation statement.
pub const REVOKE_DOMAIN: &[u8] = b"cahoots/revoke/v1";

const PAGE_LIMIT: usize = 500;

type Answer = Result<Response, Response>;

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn created(v: Value) -> Response {
    (StatusCode::CREATED, Json(v)).into_response()
}

fn db(e: rusqlite::Error) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, "db", e)
}

fn malformed(detail: impl ToString) -> Response {
    err(StatusCode::BAD_REQUEST, "malformed", detail)
}

fn signature() -> Response {
    err(
        StatusCode::FORBIDDEN,
        "signature",
        "not signed by a seat of the circle's manifest",
    )
}

fn hex_id<const N: usize>(s: &str, code: &str) -> Result<[u8; N], Response> {
    hex::decode(s)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| {
            err(
                StatusCode::BAD_REQUEST,
                code,
                format!("expected {} hex chars", 2 * N),
            )
        })
}

fn channel_path(s: &str) -> Result<[u8; 16], Response> {
    hex_id(s, "channel_id")
}

fn channel_error(e: ChannelError) -> Response {
    match e {
        ChannelError::Cose(CoseError::Signature)
        | ChannelError::Signature
        | ChannelError::Object(ObjectError::WrongKid | ObjectError::NotInRoster(_)) => signature(),
        ChannelError::WrongChannel => err(
            StatusCode::BAD_REQUEST,
            "channel_id",
            "the channel id does not match",
        ),
        ChannelError::WrongCircle => err(
            StatusCode::BAD_REQUEST,
            "circle_id",
            "the object belongs to another circle",
        ),
        e => malformed(format!("{e:?}")),
    }
}

fn conflict(code: &str, detail: &str) -> Response {
    err(StatusCode::CONFLICT, code, detail)
}

/// Take the log for a write: refuse a body over the cap, and refuse once the database is full.
fn begin_write<'a>(st: &'a State, body: &[u8]) -> Result<MutexGuard<'a, Log>, Response> {
    if body.len() > MAX_BODY {
        return Err(err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!("bodies are at most {MAX_BODY} bytes"),
        ));
    }
    let log = st.log.lock().unwrap();
    if let Some(full) = st.full(&log) {
        return Err(full);
    }
    Ok(log)
}

/// The stored manifest for `circle`, decoded. It was verified when it was stored.
fn stored_manifest(conn: &Connection, circle: &[u8; 16]) -> Result<(Vec<u8>, Manifest), Response> {
    let bytes = store::manifest(conn, circle).map_err(db)?.ok_or_else(|| {
        err(
            StatusCode::NOT_FOUND,
            "manifest",
            "no manifest for this circle",
        )
    })?;
    let manifest = SealedManifest::decode(&bytes)
        .and_then(|s| Manifest::decode(&s.body))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "db", format!("{e:?}")))?;
    Ok((bytes, manifest))
}

fn stored_channel(conn: &Connection, channel: &[u8; 16]) -> Result<ChannelRow, Response> {
    store::channel(conn, channel)
        .map_err(db)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "channel", "no such channel"))
}

/// Verify a COSE_Sign1 by a seat of `manifest` under kid `[seat]` and external AAD `domain`.
fn seat_signed(bytes: &[u8], manifest: &Manifest, domain: &[u8]) -> Result<Vec<u8>, Response> {
    let kid = cose::kid1(bytes).map_err(|e| malformed(format!("{e:?}")))?;
    let [seat] = kid[..] else {
        return Err(signature());
    };
    let key = manifest.seat_key(seat).ok_or_else(signature)?;
    cose::verify1(key, bytes, None, domain).map_err(|e| match e {
        CoseError::Signature => signature(),
        e => malformed(format!("{e:?}")),
    })
}

fn cbor_bytes_array(items: &[&[u8]]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    // Writing to a Vec cannot fail.
    let _ = e.array(items.len() as u64);
    for i in items {
        let _ = e.bytes(i);
    }
    e.into_writer()
}

/// Read a definite array of byte strings at the decoder's position.
fn read_bytes_array<'b>(d: &mut minicbor::Decoder<'b>) -> Option<Vec<&'b [u8]>> {
    let n = d.array().ok()??;
    (0..n).map(|_| d.bytes().ok()).collect()
}

/// A whole body that is the canonical CBOR array of byte strings.
fn bytes_array(body: &[u8]) -> Result<Vec<&[u8]>, Response> {
    let mut d = minicbor::Decoder::new(body);
    let items = read_bytes_array(&mut d).ok_or_else(|| malformed("expected an array of bstr"))?;
    if cbor_bytes_array(&items) != body {
        return Err(malformed("not the canonical encoding"));
    }
    Ok(items)
}

/// A canonical map `{0: bstr, 1: bstr, 2: tstr?}`: the payload of a gate or revocation statement.
fn statement(payload: &[u8], with_reason: bool) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut d = minicbor::Decoder::new(payload);
    let n = if with_reason { 3 } else { 2 };
    if d.map().ok()? != Some(n) || d.u8().ok()? != 0 {
        return None;
    }
    let channel = d.bytes().ok()?.to_vec();
    if d.u8().ok()? != 1 {
        return None;
    }
    let second = d.bytes().ok()?.to_vec();
    let mut e = minicbor::Encoder::new(Vec::new());
    let _ = e
        .map(n)
        .and_then(|e| e.u8(0))
        .and_then(|e| e.bytes(&channel));
    let _ = e.u8(1).and_then(|e| e.bytes(&second));
    if with_reason {
        if d.u8().ok()? != 2 {
            return None;
        }
        let reason = d.str().ok()?;
        let _ = e.u8(2).and_then(|e| e.str(reason));
    }
    (e.into_writer() == payload).then_some((channel, second))
}

fn finish(r: Answer) -> Response {
    r.unwrap_or_else(|e| e)
}

// Manifests.

pub async fn put_manifest(
    AxState(st): AxState<State>,
    Path(circle): Path<String>,
    body: Bytes,
) -> Response {
    finish(put_manifest_inner(&st, &circle, &body))
}

fn put_manifest_inner(st: &State, circle: &str, body: &[u8]) -> Answer {
    let circle = parse_circle(circle).map_err(|r| *r)?;
    let manifest = SealedManifest::decode(body)
        .and_then(|s| s.verify())
        .map_err(|e| malformed(format!("{e:?}")))?;
    if manifest.circle_id != circle {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "circle_id",
            "the manifest is for another circle",
        ));
    }
    let mut log = begin_write(st, body)?;
    match store::put_manifest(log.conn(), &circle, body).map_err(db)? {
        Put::Exists => Err(conflict(
            "exists",
            "this circle already has another manifest",
        )),
        _ => Ok(created(json!({ "circle_id": hex::encode(circle) }))),
    }
}

pub async fn get_manifest(AxState(st): AxState<State>, Path(circle): Path<String>) -> Response {
    finish((|| {
        let circle = parse_circle(&circle).map_err(|r| *r)?;
        let mut log = st.log.lock().unwrap();
        let (bytes, _) = stored_manifest(log.conn(), &circle)?;
        Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
    })())
}

// Channel objects.

pub async fn post_channel(AxState(st): AxState<State>, body: Bytes) -> Response {
    finish(post_channel_inner(&st, &body))
}

fn post_channel_inner(st: &State, body: &[u8]) -> Answer {
    let payload = cose::payload1(body).map_err(|e| malformed(format!("{e:?}")))?;
    let claimed = ChannelObject::decode(&payload).map_err(channel_error)?;
    let mut log = begin_write(st, body)?;
    let conn = log.conn();
    let (_, manifest) = stored_manifest(conn, &claimed.circle_id)?;
    let (_, object) = ChannelObject::verify(body, &manifest).map_err(channel_error)?;
    let row = ChannelRow {
        circle: object.circle_id,
        body: body.to_vec(),
        created_log: object.created_log,
    };
    match store::put_channel(conn, &object.channel_id, &row).map_err(db)? {
        Put::Stale => Err(conflict(
            "stale",
            "a channel object at the same or a later log position is stored",
        )),
        _ => Ok(created(
            json!({ "channel_id": hex::encode(object.channel_id) }),
        )),
    }
}

pub async fn get_channel(AxState(st): AxState<State>, Path(channel): Path<String>) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let mut log = st.log.lock().unwrap();
        let conn = log.conn();
        let row = stored_channel(conn, &channel)?;
        let (manifest, _) = stored_manifest(conn, &row.circle)?;
        let policy = store::current_policy(conn, &channel).map_err(db)?;
        Ok(Json(json!({
            "channel": b64(&row.body),
            "policy": policy.map(|p| b64(&p.body)),
            "manifest": b64(&manifest),
        }))
        .into_response())
    })())
}

// Policies.

pub async fn post_policy(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    finish(post_policy_inner(&st, &channel, &body))
}

fn post_policy_inner(st: &State, channel: &str, body: &[u8]) -> Answer {
    let channel = channel_path(channel)?;
    let shape = || malformed("expected [signed_policy, [approval, ...]]");
    let mut d = minicbor::Decoder::new(body);
    if d.array().ok().flatten() != Some(2) {
        return Err(shape());
    }
    let signed = d.bytes().map_err(|_| shape())?;
    let approvals_at = d.position();
    let approvals = read_bytes_array(&mut d).ok_or_else(shape)?;
    let approvals_bytes = &body[approvals_at..];
    if cbor_bytes_array(&approvals) != approvals_bytes {
        return Err(malformed("not the canonical encoding"));
    }
    let payload = cose::payload1(signed).map_err(|e| malformed(format!("{e:?}")))?;
    let claimed = Policy::decode(&payload).map_err(channel_error)?;
    if claimed.channel_id != channel {
        return Err(channel_error(ChannelError::WrongChannel));
    }
    let mut log = begin_write(st, body)?;
    let conn = log.conn();
    let (_, manifest) = stored_manifest(conn, &claimed.circle_id)?;
    let (_, verified) = Policy::verify(signed, &manifest).map_err(channel_error)?;
    let row = PolicyRow {
        id: policy::id(signed),
        body: signed.to_vec(),
        log: verified.log,
        approvals: approvals_bytes.to_vec(),
    };
    match store::put_policy(conn, &channel, &row).map_err(db)? {
        Put::Stale => Err(conflict(
            "stale",
            "a policy at the same or a later log position is current",
        )),
        _ => Ok(created(json!({ "policy_id": hex::encode(row.id) }))),
    }
}

// Grants.

pub async fn post_grant(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    finish(post_grant_inner(&st, &channel, &body))
}

fn post_grant_inner(st: &State, channel: &str, body: &[u8]) -> Answer {
    let channel = channel_path(channel)?;
    let links = bytes_array(body)?;
    let leaf = links
        .last()
        .ok_or_else(|| malformed("an empty grant chain"))?;
    let claims = cose::payload1(leaf)
        .map_err(|e| format!("{e:?}"))
        .and_then(|p| Claims::decode(&p).map_err(|e| format!("{e:?}")))
        .map_err(malformed)?;
    match claims.kind().map_err(|e| malformed(format!("{e:?}")))? {
        (_, Kind::Channel { channel: c, .. }) if c == channel => {}
        (_, Kind::Channel { .. }) => return Err(channel_error(ChannelError::WrongChannel)),
        _ => return Err(malformed("not a channel grant")),
    }
    let row = GrantRow {
        cti: claims.cti,
        channel,
        kid: cahoots_objects::grant::key_id(&claims.cnf),
        chain: body.to_vec(),
        not_after: claims.log_not_after,
    };
    let mut log = begin_write(st, body)?;
    match store::put_grant(log.conn(), &row).map_err(db)? {
        Put::Exists => Err(conflict("exists", "another chain is stored under this cti")),
        _ => Ok(created(json!({ "grant_id": hex::encode(row.cti) }))),
    }
}

#[derive(Deserialize)]
pub struct KidQuery {
    kid: Option<String>,
}

pub async fn get_grants(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    Query(q): Query<KidQuery>,
) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let kid: [u8; 32] = hex_id(q.kid.as_deref().unwrap_or(""), "malformed")?;
        let mut log = st.log.lock().unwrap();
        let rows = store::grants_for_kid(log.conn(), &channel, &kid).map_err(db)?;
        let grants: Vec<String> = rows.iter().map(|r| b64(&r.chain)).collect();
        Ok(Json(json!({ "grants": grants })).into_response())
    })())
}

pub async fn get_grant(
    AxState(st): AxState<State>,
    Path((channel, cti)): Path<(String, String)>,
) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let cti: [u8; 16] = hex_id(&cti, "grant_id")?;
        let mut log = st.log.lock().unwrap();
        let row = store::grant(log.conn(), &channel, &cti)
            .map_err(db)?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "grant", "no such grant"))?;
        Ok(Json(json!({ "grant": b64(&row.chain) })).into_response())
    })())
}

// Revocations.

pub async fn post_revocation(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    finish(post_revocation_inner(&st, &channel, &body))
}

fn post_revocation_inner(st: &State, channel: &str, body: &[u8]) -> Answer {
    let channel = channel_path(channel)?;
    let [grant_id, signed] = bytes_array(body)?[..] else {
        return Err(malformed("expected [grant_id, signed_statement]"));
    };
    let grant_id: [u8; 16] = grant_id
        .try_into()
        .map_err(|_| malformed("a grant id is 16 bytes"))?;
    let mut log = begin_write(st, body)?;
    let seq = log.head().seq;
    let conn = log.conn();
    let row = stored_channel(conn, &channel)?;
    let (_, manifest) = stored_manifest(conn, &row.circle)?;
    let payload = seat_signed(signed, &manifest, REVOKE_DOMAIN)?;
    let (c, g) = statement(&payload, true)
        .ok_or_else(|| malformed("expected {0: channel_id, 1: grant_id, 2: reason}"))?;
    if c != channel {
        return Err(channel_error(ChannelError::WrongChannel));
    }
    if g != grant_id {
        return Err(malformed("the statement names another grant"));
    }
    store::put_revocation(conn, &channel, &grant_id, seq, body).map_err(db)?;
    Ok(created(json!({ "grant_id": hex::encode(grant_id) })))
}

pub async fn get_revocations(AxState(st): AxState<State>, Path(channel): Path<String>) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let mut log = st.log.lock().unwrap();
        let list: Vec<Value> = store::revocations(log.conn(), &channel)
            .map_err(db)?
            .into_iter()
            .map(|(id, seq)| json!({ "grant_id": hex::encode(id), "seq": seq }))
            .collect();
        Ok(Json(json!({ "revocations": list })).into_response())
    })())
}

// Deposits.

pub async fn post_deposit(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    finish(post_deposit_inner(&st, &channel, &body))
}

fn post_deposit_inner(st: &State, channel: &str, body: &[u8]) -> Answer {
    let channel = channel_path(channel)?;
    let d = Deposit::decode(body).map_err(channel_error)?;
    if d.channel_id != channel {
        return Err(channel_error(ChannelError::WrongChannel));
    }
    let id = d.id();
    let mut log = begin_write(st, body)?;
    store::put_deposit(log.conn(), &id, &channel, &d.epoch_id.encode(), &d.to, body).map_err(db)?;
    Ok(created(json!({ "deposit_id": hex::encode(id) })))
}

#[derive(Deserialize)]
pub struct DepositQuery {
    to: Option<String>,
    epoch: Option<String>,
}

pub async fn get_deposits(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    Query(q): Query<DepositQuery>,
) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let to: Option<[u8; 32]> = q.to.as_deref().map(|s| hex_id(s, "to")).transpose()?;
        let epoch: Option<[u8; 20]> = q.epoch.as_deref().map(|s| hex_id(s, "epoch")).transpose()?;
        let mut log = st.log.lock().unwrap();
        let list: Vec<String> = store::deposits(log.conn(), &channel, to.as_ref(), epoch.as_ref())
            .map_err(db)?
            .iter()
            .map(|b| b64(b))
            .collect();
        Ok(Json(json!({ "deposits": list })).into_response())
    })())
}

// Join requests.

pub async fn post_request(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    finish(post_request_inner(&st, &channel, &body))
}

fn post_request_inner(st: &State, channel: &str, body: &[u8]) -> Answer {
    let channel = channel_path(channel)?;
    if body.is_empty() {
        return Err(malformed("an empty request"));
    }
    let id: [u8; 32] = Sha256::digest(body).into();
    let mut log = begin_write(st, body)?;
    let seq = store::put_request(log.conn(), &channel, &id, body).map_err(db)?;
    Ok(created(
        json!({ "request_id": hex::encode(id), "seq": seq }),
    ))
}

#[derive(Deserialize)]
pub struct AfterQuery {
    after: Option<u64>,
}

pub async fn get_requests(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    Query(q): Query<AfterQuery>,
) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let mut log = st.log.lock().unwrap();
        let list: Vec<Value> = store::requests_after(log.conn(), &channel, q.after, PAGE_LIMIT)
            .map_err(db)?
            .into_iter()
            .map(|r| json!({ "id": hex::encode(r.id), "body": b64(&r.body), "seq": r.seq }))
            .collect();
        Ok(Json(json!({ "requests": list })).into_response())
    })())
}

// Gate tokens.

/// Check a gate statement for `channel` against the channel's manifest; returns the token's hash.
fn gate_token_hash(
    conn: &Connection,
    channel: &[u8; 16],
    body: &[u8],
) -> Result<[u8; 32], Response> {
    let row = stored_channel(conn, channel)?;
    let (_, manifest) = stored_manifest(conn, &row.circle)?;
    let payload = seat_signed(body, &manifest, GATE_DOMAIN)?;
    let (c, token) = statement(&payload, false)
        .ok_or_else(|| malformed("expected {0: channel_id, 1: token}"))?;
    if c != channel {
        return Err(channel_error(ChannelError::WrongChannel));
    }
    if token.len() != 32 {
        return Err(malformed("a gate token is 32 bytes"));
    }
    Ok(Sha256::digest(&token).into())
}

pub async fn post_gate(
    AxState(st): AxState<State>,
    Path(channel): Path<String>,
    body: Bytes,
) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let mut log = begin_write(&st, &body)?;
        let conn = log.conn();
        let hash = gate_token_hash(conn, &channel, &body)?;
        store::put_gate(conn, &channel, &hash).map_err(db)?;
        Ok(created(json!({ "token_hash": hex::encode(hash) })))
    })())
}

/// Removing a token never grows the database much and takes access away, so it is not refused
/// when the database is full.
pub async fn delete_gate(
    AxState(st): AxState<State>,
    Path((channel, token_hash)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    finish((|| {
        let channel = channel_path(&channel)?;
        let path_hash: [u8; 32] = hex_id(&token_hash, "token_hash")?;
        let mut log = st.log.lock().unwrap();
        let conn = log.conn();
        let hash = gate_token_hash(conn, &channel, &body)?;
        if hash != path_hash {
            return Err(malformed("the path names another token than the statement"));
        }
        store::delete_gate(conn, &channel, &hash).map_err(db)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })())
}
