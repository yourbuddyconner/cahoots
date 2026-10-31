//! The `gate` feature: who may read a channel, who may post to it, and which policies and grant
//! chains the relay stores. The handlers call these checks; nothing here holds or derives a
//! key. The relay sees only gate token hashes, signed grants, signed statements and the
//! signatures on messages, never an epoch key, a link secret or a seal key.
//!
//! **Reads.** `GET …/messages`, `…/head`, `…/deposits`, `…/grants` and `…/requests` on a
//! channel need one of:
//!
//! - `Cahoots-Gate: <hex 32>`, a gate token whose SHA-256 the channel's owner stored with
//!   `POST …/gates`. Not enough for `…/requests`.
//! - `Cahoots-Proof: <base64url COSE_Sign1>` and `Cahoots-Kid: <hex 32>`: a
//!   `cahoots_objects::authz::proof_sign` proof by the member key whose id is the kid, over
//!   `data_hash = SHA-256(METHOD " " PATH "?" QUERY)` (the request line as sent, `?` written even
//!   when the query is empty, e.g. `GET /v1/channels/<id>/messages?after=4`) and a nonce from
//!   `GET /v1/nonce`, unused and at most 300 seconds old. One of the key's stored chains for the
//!   channel must verify and allow `read` at the channel head; `…/requests` needs `admit` too.
//!
//! Otherwise `401 {"error":"gate"}`. An unknown channel is `404 {"error":"channel"}`.
//!
//! **Posts.** One of the stored chains for the message's `kid` must verify, the message's
//! signature must verify under its leaf's key, and the leaf must allow `post` (text and
//! announcements) or `govern` (control; an admin seat's own grant carries all four actions) at
//! the channel head, under its budget, with no link revoked. A message whose
//! `(kid, epoch_id, counter)` the stream already holds is `409 {"error":"replay"}`.
//! An announcement's approvals must meet the current policy's post rule. Refusals are
//! `403 {"error":"grant","reason": "missing"|"chain"|"signature"|"revoked"|"expired"|"budget"|
//! "action"|"quorum"}`.
#![allow(clippy::result_large_err)]

use crate::log::Log;
use crate::objects::handlers::{bytes_array, channel_error, stored_channel, stored_manifest};
use crate::objects::store;
use crate::server::{err, State};
use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_channel::grant::{chain_ids, evaluate, verify_chain, ChannelGrant, EvalRequest};
use cahoots_channel::message::{self, MessageType};
use cahoots_channel::quorum::{check_quorum, PROFILE_GOVERN, PROFILE_POST};
use cahoots_channel::{policy, Action, ChannelError, Policy, Refusal, Rule};
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a nonce from `GET /v1/nonce` stays usable.
pub const NONCE_TTL: Duration = Duration::from_secs(300);

/// The request headers that carry a reader's credentials.
pub const GATE_HEADER: &str = "cahoots-gate";
pub const PROOF_HEADER: &str = "cahoots-proof";
pub const KID_HEADER: &str = "cahoots-kid";

type Checked = Result<(), Response>;

/// A channel id and a member key id.
type ChannelKid = ([u8; 16], [u8; 32]);

/// What makes a message unique in a channel's stream: its channel, `kid`, epoch id and counter.
type MessageKey = ([u8; 16], [u8; 32], [u8; 20], u32);

/// The most nonces held at once; `GET /v1/nonce` answers 429 beyond it.
pub const MAX_NONCES: usize = 10_000;

/// The gate's memory: unused nonces, each key's post count per channel, and the messages each
/// channel's stream holds.
pub struct Gate {
    nonces: Mutex<HashMap<[u8; 16], Instant>>,
    posts: Mutex<HashMap<ChannelKid, u32>>,
    seen: Mutex<HashSet<MessageKey>>,
}

fn message_key(channel: &[u8; 16], h: &message::Header) -> MessageKey {
    (*channel, h.kid, h.epoch_id.encode(), h.counter)
}

impl Gate {
    /// Count every version 4 entry in the log by its stream and header kid, and note its
    /// `(kid, epoch_id, counter)`.
    pub(crate) fn from_log(log: &Log) -> Gate {
        let mut posts = HashMap::new();
        let mut seen = HashSet::new();
        for e in log.entries() {
            if let Ok(h) = message::Header::parse(&e.message) {
                *posts.entry((e.circle_id, h.kid)).or_insert(0u32) += 1;
                seen.insert(message_key(&e.circle_id, &h));
            }
        }
        Gate {
            nonces: Mutex::new(HashMap::new()),
            posts: Mutex::new(posts),
            seen: Mutex::new(seen),
        }
    }

    /// A fresh nonce, or `None` when [`MAX_NONCES`] live nonces are held.
    fn new_nonce(&self) -> Option<[u8; 16]> {
        let mut n = [0u8; 16];
        getrandom::getrandom(&mut n).expect("the OS random source is available");
        let mut nonces = self.nonces.lock().unwrap();
        nonces.retain(|_, at| at.elapsed() < NONCE_TTL);
        if nonces.len() >= MAX_NONCES {
            return None;
        }
        nonces.insert(n, Instant::now());
        Some(n)
    }

    fn holds(&self, channel: &[u8; 16], h: &message::Header) -> bool {
        self.seen.lock().unwrap().contains(&message_key(channel, h))
    }

    /// Spend a nonce: `true` when the relay gave it out under [`NONCE_TTL`] ago and it was unused.
    fn take_nonce(&self, n: &[u8; 16]) -> bool {
        self.nonces
            .lock()
            .unwrap()
            .remove(n)
            .is_some_and(|at| at.elapsed() < NONCE_TTL)
    }

    fn posts_used(&self, channel: &[u8; 16], kid: &[u8; 32]) -> u32 {
        let posts = self.posts.lock().unwrap();
        posts.get(&(*channel, *kid)).copied().unwrap_or(0)
    }

    /// Count one stored post and note it. Call it with the log lock held, after the append.
    pub(crate) fn counted(&self, channel: &[u8; 16], h: &message::Header) {
        *self
            .posts
            .lock()
            .unwrap()
            .entry((*channel, h.kid))
            .or_insert(0) += 1;
        self.seen.lock().unwrap().insert(message_key(channel, h));
    }
}

/// `GET /v1/nonce`.
pub fn routes(router: Router<State>) -> Router<State> {
    router.route("/v1/nonce", get(nonce))
}

async fn nonce(axum::extract::State(st): axum::extract::State<State>) -> Response {
    match st.gate.new_nonce() {
        Some(n) => Json(json!({ "nonce": hex::encode(n) })).into_response(),
        None => crate::limits::rate_limited("too many open nonces; try again in a few seconds"),
    }
}

/// What a read request presents: its headers, and the request line a proof signs.
pub struct Credentials {
    gate: Option<String>,
    proof: Option<String>,
    kid: Option<String>,
    /// `METHOD " " PATH "?" QUERY`.
    signed: String,
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for Credentials {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        let header = |name: &str| {
            parts
                .headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        Ok(Credentials {
            gate: header(GATE_HEADER),
            proof: header(PROOF_HEADER),
            kid: header(KID_HEADER),
            signed: format!(
                "{} {}?{}",
                parts.method,
                parts.uri.path(),
                parts.uri.query().unwrap_or("")
            ),
        })
    }
}

/// What a read needs beyond the channel's read rule.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Read,
    /// The join queue: a proof by a key that may read and admit.
    Admit,
}

fn gate_refused() -> Response {
    err(
        StatusCode::UNAUTHORIZED,
        "gate",
        "this channel needs a Cahoots-Gate token or a Cahoots-Proof by a key with a read grant",
    )
}

/// `403 {"error":"grant","reason":…}`.
fn refused(reason: &str, detail: impl ToString) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "grant", "reason": reason, "detail": detail.to_string() })),
    )
        .into_response()
}

fn refusal_reason(r: Refusal) -> &'static str {
    match r {
        Refusal::Revoked => "revoked",
        Refusal::Expired => "expired",
        Refusal::Budget => "budget",
        Refusal::Action => "action",
        Refusal::Key => "signature",
        Refusal::WrongChannel => "chain",
    }
}

fn hex_n<const N: usize>(s: &str) -> Option<[u8; N]> {
    hex::decode(s).ok()?.try_into().ok()
}

/// The channel head as a grant reads it: the global `seq` of the channel's last entry, 0 when
/// it has none.
fn head(log: &Log, channel: &[u8; 16]) -> u64 {
    log.stream_head(channel).seq.unwrap_or(0)
}

/// The stored chains for `kid` in `channel`, each as its links, root first.
fn chains(conn: &Connection, channel: &[u8; 16], kid: &[u8; 32]) -> Result<Vec<Vec<u8>>, Response> {
    store::grants_for_kid(conn, channel, kid)
        .map(|rows| rows.into_iter().map(|r| r.chain).collect())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "db", e))
}

/// Verify a stored chain and read its ids. `Err` is the refusal reason.
fn open_chain(
    chain: &[u8],
    manifest: &Manifest,
    channel: &[u8; 16],
) -> Result<(ChannelGrant, Vec<[u8; 16]>), &'static str> {
    let links = bytes_array(chain).map_err(|_| "chain")?;
    let leaf = verify_chain(&links, manifest, channel).map_err(|_| "chain")?;
    let ids = chain_ids(&links).map_err(|_| "chain")?;
    Ok((leaf, ids))
}

/// Check a read of `channel`. Takes the nonce of a proof whether or not the proof holds.
pub(crate) fn check_read(
    st: &State,
    log: &mut Log,
    channel: &[u8; 16],
    creds: &Credentials,
    need: Need,
) -> Checked {
    let row = stored_channel(log.conn(), channel)?;
    if need == Need::Read {
        if let Some(token) = creds.gate.as_deref().and_then(hex_n::<32>) {
            let hash: [u8; 32] = Sha256::digest(token).into();
            let held = store::has_gate(log.conn(), channel, &hash)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "db", e))?;
            if held {
                return Ok(());
            }
        }
    }
    let (Some(proof), Some(kid)) = (creds.proof.as_deref(), creds.kid.as_deref()) else {
        return Err(gate_refused());
    };
    let (Ok(proof), Some(kid)) = (URL_SAFE_NO_PAD.decode(proof), hex_n::<32>(kid)) else {
        return Err(gate_refused());
    };
    let nonce = proof_nonce(&proof).ok_or_else(gate_refused)?;
    if !st.gate.take_nonce(&nonce) {
        return Err(gate_refused());
    }
    let data_hash: [u8; 32] = Sha256::digest(creds.signed.as_bytes()).into();
    let at = head(log, channel);
    let conn = log.conn();
    let (_, manifest) = stored_manifest(conn, &row.circle)?;
    let revoked = |id: &[u8; 16]| store::revoked(conn, channel, id).unwrap_or(true);
    let actions: &[Action] = match need {
        Need::Read => &[Action::Read],
        Need::Admit => &[Action::Read, Action::Admit],
    };
    for chain in chains(conn, channel, &kid)? {
        let Ok((leaf, ids)) = open_chain(&chain, &manifest, channel) else {
            continue;
        };
        let allowed = actions.iter().all(|&action| {
            let req = EvalRequest {
                action,
                channel_id: channel,
                log: at,
                posts_used: 0,
                revoked: &revoked,
                signer: &leaf.grantee,
            };
            evaluate(&leaf, &ids, &req).is_ok()
        });
        if allowed
            && cahoots_objects::authz::proof_verify(&proof, &leaf.grantee, &data_hash, &nonce)
        {
            return Ok(());
        }
    }
    Err(gate_refused())
}

/// The nonce a proof's payload `[data_hash, nonce]` names, read before the signature is checked.
fn proof_nonce(proof: &[u8]) -> Option<[u8; 16]> {
    let payload = cose::payload1(proof).ok()?;
    let mut d = minicbor::Decoder::new(&payload);
    if d.array().ok()? != Some(2) {
        return None;
    }
    d.bytes().ok()?;
    d.bytes().ok()?.try_into().ok()
}

/// Check a post of `wire` to `channel`, whose existence the caller has checked.
pub(crate) fn check_post(st: &State, log: &mut Log, channel: &[u8; 16], wire: &[u8]) -> Checked {
    let (body, _, header) = message::split(wire)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "malformed", format!("{e:?}")))?;
    // A message the stream already holds, sent again: refused before it can spend a budget.
    if st.gate.holds(channel, &header) {
        return Err(err(
            StatusCode::CONFLICT,
            "replay",
            "this channel already holds a message with this kid, epoch and counter",
        ));
    }
    let at = head(log, channel);
    let posts_used = st.gate.posts_used(channel, &header.kid);
    let conn = log.conn();
    let stored = chains(conn, channel, &header.kid)?;
    if stored.is_empty() {
        return Err(refused("missing", "no grant is stored for this kid"));
    }
    let row = stored_channel(conn, channel)?;
    let (_, manifest) = stored_manifest(conn, &row.circle)?;
    let post_rule = match header.msg_type {
        MessageType::Announcement => current_rule(conn, channel, Action::Post)?,
        _ => None,
    };
    let revoked = |id: &[u8; 16]| store::revoked(conn, channel, id).unwrap_or(true);
    let mut last = "chain";
    for chain in &stored {
        let one = || -> Result<(), &'static str> {
            let (leaf, ids) = open_chain(chain, &manifest, channel)?;
            let (_, _, approvals) =
                message::verify_signature(wire, &leaf.grantee).map_err(|_| "signature")?;
            let eval = |action| {
                let req = EvalRequest {
                    action,
                    channel_id: channel,
                    log: at,
                    posts_used,
                    revoked: &revoked,
                    signer: &leaf.grantee,
                };
                evaluate(&leaf, &ids, &req).map_err(refusal_reason)
            };
            match header.msg_type {
                MessageType::Text => eval(Action::Post),
                MessageType::Control => eval(Action::Govern),
                MessageType::Announcement => {
                    eval(Action::Post)?;
                    let (quorum, assurance) = post_rule
                        .as_ref()
                        .map_or((0, 0), |r| (r.quorum, r.assurance));
                    check_quorum(
                        &approvals,
                        &manifest,
                        &message::body_digest(body),
                        PROFILE_POST,
                        quorum,
                        assurance,
                    )
                    .map(|_| ())
                    .map_err(|_| "quorum")
                }
            }
        };
        match one() {
            Ok(()) => return Ok(()),
            Err(reason) => last = reason,
        }
    }
    Err(refused(
        last,
        "no stored grant for this kid allows the post",
    ))
}

/// The current policy's rule for `action`, `None` when there is no policy or no rule.
fn current_rule(
    conn: &Connection,
    channel: &[u8; 16],
    action: Action,
) -> Result<Option<Rule>, Response> {
    let Some(row) = store::current_policy(conn, channel)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "db", e))?
    else {
        return Ok(None);
    };
    // Stored policies were verified when they were stored.
    let policy = cose::payload1(&row.body)
        .ok()
        .and_then(|p| Policy::decode(&p).ok())
        .ok_or_else(|| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "db",
                "a stored policy does not decode",
            )
        })?;
    Ok(policy.rule(action).cloned())
}

/// Check a new policy for `channel` against the current one's `govern` rule. The first policy
/// of a channel needs only the seat signature the caller has verified; so does a current policy
/// with no `govern` rule (only the owner's seats govern).
pub(crate) fn check_policy(
    conn: &Connection,
    channel: &[u8; 16],
    signed: &[u8],
    approvals: &[&[u8]],
    manifest: &Manifest,
) -> Checked {
    if store::current_policy(conn, channel)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "db", e))?
        .is_none()
    {
        return Ok(());
    }
    let rule = current_rule(conn, channel, Action::Govern)?.unwrap_or(Rule {
        action: Action::Govern,
        evidence: Vec::new(),
        per_evidence: 0,
        quorum: 0,
        assurance: 0,
    });
    let approvals: Vec<Vec<u8>> = approvals.iter().map(|a| a.to_vec()).collect();
    policy::verify_governed(
        signed,
        policy::DOMAIN,
        &approvals,
        manifest,
        &rule,
        PROFILE_GOVERN,
    )
    .map(|_| ())
    .map_err(|e| match e {
        ChannelError::Quorum { have, need } => {
            refused("quorum", format!("{have} of {need} seat approvals"))
        }
        ChannelError::Object(_) if rule.quorum > 1 => {
            refused("quorum", "an approval does not verify")
        }
        e => channel_error(e),
    })
}

/// Check a grant chain for `channel` against the owner circle's manifest.
pub(crate) fn check_grant(conn: &Connection, channel: &[u8; 16], links: &[&[u8]]) -> Checked {
    let row = stored_channel(conn, channel)?;
    let (_, manifest) = stored_manifest(conn, &row.circle)?;
    verify_chain(links, &manifest, channel)
        .map(|_| ())
        .map_err(|e| refused("chain", format!("{e:?}")))
}
