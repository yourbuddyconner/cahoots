//! Channels on the phone: the browser app's flows (`web/app/channels.js`) step for step, over
//! the owner and member cores of `cahoots-channel`.
//!
//! A channel lives in `channels/<id>/`: the core's exported state (`owner.cbor` or
//! `member.cbor`, secrets), the app's record (`meta.cbor`) and the history (`messages.cbor`).
//! Each call loads them, works, and writes them back with `store::write_atomic`; a call that
//! fails before writing leaves the files as they were. The state is written before anything
//! it made leaves the phone, and before the record that moves the relay cursor.
//!
//! The relay is reached through `Http::request`, which hands back every status. A gated read
//! carries `Cahoots-Gate: <hex>` (the link's token) or `Cahoots-Proof` and `Cahoots-Kid`, a
//! proof by the member key over `METHOD " " PATH "?" QUERY` and a nonce from `GET /v1/nonce`.
//!
//! What the owner's seat signs goes through the `Signer`, under the circle's seat key label: a
//! refused Face ID (`PhoneError::Locked`) stops the call before anything is written.

use crate::signer::Seat;
use crate::store::{self, ChannelMessageRecord, ChannelMessages, ChannelMeta};
use crate::{
    ChannelMemberInfo, ChannelMessage, ChannelPreview, ChannelRequest, ChannelRole, ChannelSummary,
    HttpHeader, HttpResponse, Phone, PhoneError,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_channel::control::Control;
use cahoots_channel::deposit::peek_signed;
use cahoots_channel::keys::EpochId;
use cahoots_channel::link::Link;
use cahoots_channel::member::{
    bytes_array, open_manifest, policy_summary, read_policy, ChannelMember, JoinRequest,
};
use cahoots_channel::message::MessageType;
use cahoots_channel::owner::{majority, ChannelOwner};
use cahoots_channel::{Action, ChannelObject};
use cahoots_chat::{bucket_pad, bucket_unpad};
use cahoots_core::seat::SeatPublic;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

const DEAD: &str = "This link no longer works. Ask for a new one.";
const WAITING: &str = "Waiting for the owner to let you post.";
const READ_ONLY: &str = "You can read this channel. Only the owner posts.";
const CLOSED: &str = "Only people the owner lets in can read this channel.";
const NO_LINK: &str = "This channel has no link. Only people you let in can read.";
const NEEDS_SEATS: &str = "Announcements with approvals need more than one owner device to approve, which this version cannot collect yet.";
const OTHER_RELAY: &str = "This link names a different relay than the one this channel uses.";
const OWNER_ONLY: &str = "Only the channel's owner can do that.";
const REMOVED: &str = "removed by the owner";
const NOTE_REVOKED: &str = "The owner removed a member and will start new keys.";

/// The plain words for a core's error code, as `channels.js` shows them.
fn friendly(code: &str) -> String {
    match code {
        "needs-approvals" => "This circle has more than one member; changing the policy needs their approval, which this version cannot collect yet.".into(),
        "relay" => "That relay address cannot go in a link.".into(),
        "name" => "Give a name of up to 64 bytes.".into(),
        "policy" => "This channel lets only the owner post.".into(),
        "evidence" => "This request came with an old link, so it cannot be allowed.".into(),
        "signature" => "This does not carry a valid signature.".into(),
        "unknown-epoch" => "This message uses keys this device does not hold.".into(),
        "link" => "That link does not read.".into(),
        "no-link" => NO_LINK.into(),
        "seat" => "This phone's seat key is not the one the circle's member list names.".into(),
        "manifest" => "The circle's member list does not check out.".into(),
        "state" => "This channel's record on this phone does not read.".into(),
        c if c.starts_with("refused:revoked") => "Sent by a member who was removed.".into(),
        c if c.starts_with("refused:") || c.starts_with("grant:") => {
            "Sent by someone without a grant to post.".into()
        }
        other => format!("The channel could not do that ({other})."),
    }
}

fn ch_err(code: String) -> PhoneError {
    PhoneError::Channel {
        message: friendly(&code),
    }
}

fn channel(message: &str) -> PhoneError {
    PhoneError::Channel {
        message: message.into(),
    }
}

fn network(message: impl Into<String>) -> PhoneError {
    PhoneError::Network {
        message: message.into(),
    }
}

fn unreadable() -> PhoneError {
    network("The relay's answer does not read.")
}

/// A relay refusal, kept apart so the flows can read its status, or any other error.
enum Fail {
    Relay {
        status: u16,
        code: String,
        reason: String,
    },
    Phone(PhoneError),
}

impl From<PhoneError> for Fail {
    fn from(e: PhoneError) -> Self {
        Fail::Phone(e)
    }
}

impl From<Fail> for PhoneError {
    fn from(f: Fail) -> Self {
        match f {
            Fail::Phone(e) => e,
            Fail::Relay {
                status,
                code,
                reason,
            } => network(match status {
                401 => "The relay refused the read.".to_string(),
                403 => format!(
                    "The relay refused it ({}).",
                    if reason.is_empty() { &code } else { &reason }
                ),
                404 => "The relay does not hold this channel.".into(),
                429 => "The relay is busy. Try again in a moment.".into(),
                s if code.is_empty() => format!("The relay answered {s}."),
                s => format!("The relay answered {s} ({code})."),
            }),
        }
    }
}

type R<T> = Result<T, Fail>;

fn is_status(f: &Fail, want: u16) -> bool {
    matches!(f, Fail::Relay { status, .. } if *status == want)
}

/// How a gated read proves it may read.
#[derive(Clone, Copy)]
enum Auth {
    None,
    /// The link's gate token.
    Gate([u8; 32]),
    /// A proof by this channel's member key.
    Proof,
}

fn short(kid: &str) -> String {
    kid.chars().take(8).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn json(body: &[u8]) -> R<serde_json::Value> {
    serde_json::from_slice(body).map_err(|_| Fail::Phone(unreadable()))
}

fn b64(text: &str) -> R<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| Fail::Phone(unreadable()))
}

/// The base64url strings of `v[key]`, decoded.
fn b64_list(v: &serde_json::Value, key: &str) -> R<Vec<Vec<u8>>> {
    v[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| b64(x.as_str().unwrap_or("")))
        .collect()
}

/// The relay's address from a link's host: plain http for this machine, https for the rest.
fn relay_base(host: &str) -> String {
    let name = host
        .strip_prefix('[')
        .and_then(|h| h.split(']').next().map(|n| format!("[{n}]")))
        .unwrap_or_else(|| host.split(':').next().unwrap_or("").to_string());
    let local = name == "localhost"
        || name == "[::1]"
        || (name.starts_with("127.")
            && name.split('.').count() == 4
            && name.split('.').all(|p| p.parse::<u8>().is_ok()));
    format!("{}://{host}", if local { "http" } else { "https" })
}

/// `host` or `host:port` of a relay address.
fn host_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// The policy's read rule takes no link evidence (or there is none): no link reads, only
/// readers the owner lets in (the closed preset admits them by hand).
fn read_is_closed(policy: &[u8]) -> bool {
    read_policy(policy).is_ok_and(|p| {
        p.rule(Action::Read).is_none_or(|r| {
            !r.evidence
                .iter()
                .any(|e| e.kind == cahoots_channel::policy::kind::LINK)
        })
    })
}

/// A relay address or a link's host, as compared: the host lowercased and the port kept;
/// scheme, path and trailing slash dropped.
fn relay_key(relay: &str) -> String {
    let with_scheme = if relay.contains("://") {
        relay.to_string()
    } else {
        relay_base(relay)
    };
    host_of(&with_scheme).to_lowercase()
}

/// Whether a stored relay address and a link's host name the same relay.
fn same_relay(stored: &str, link_host: &str) -> bool {
    let (a, b) = (relay_key(stored), relay_key(link_host));
    !a.is_empty() && a == b
}

fn is_grant_error(code: &str) -> bool {
    [
        "grant:",
        "refused:",
        "signature",
        "no-admit",
        "wrong-channel",
        "no-grant",
    ]
    .iter()
    .any(|p| code.starts_with(p))
}

fn to_message(m: &ChannelMessageRecord) -> ChannelMessage {
    ChannelMessage {
        id: m.id.clone(),
        sender: m.sender.clone(),
        mine: m.mine,
        text: m.text.clone(),
        time: m.time,
        announcement: m.kind == 1,
        approved: m.kind != 1 || m.ok,
        note: m.kind == 2,
        refused: m.kind == 3,
        seq: m.seq,
    }
}

fn check_id(id: &str) -> Result<(), PhoneError> {
    if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(crate::not_found("channel"))
    }
}

/// A channel as the relay holds it, checked.
struct Fetched {
    obj: ChannelObject,
    policy: Vec<u8>,
    manifest: Vec<u8>,
    line: String,
}

/// A channel loaded from its directory.
struct Ch {
    dir: PathBuf,
    id: String,
    meta: ChannelMeta,
    owner: Option<ChannelOwner>,
    member: Option<ChannelMember>,
    /// Grant chains read from the relay this call, by sender key id.
    chains: HashMap<[u8; 32], Vec<Vec<u8>>>,
}

impl Ch {
    fn load(dir: PathBuf, id: &str) -> Result<Ch, PhoneError> {
        let meta = ChannelMeta::load(&dir)?;
        let bad = |_| PhoneError::Storage {
            message: "This channel's record on this phone does not read.".into(),
        };
        let (owner, member) = if meta.owner {
            let bytes =
                zeroize::Zeroizing::new(std::fs::read(dir.join(store::OWNER)).map_err(store::io)?);
            (Some(ChannelOwner::import(&bytes).map_err(bad)?), None)
        } else {
            let bytes =
                zeroize::Zeroizing::new(std::fs::read(dir.join(store::MEMBER)).map_err(store::io)?);
            (None, Some(ChannelMember::import(&bytes).map_err(bad)?))
        };
        Ok(Ch {
            dir,
            id: id.to_string(),
            meta,
            owner,
            member,
            chains: HashMap::new(),
        })
    }

    fn member(&self) -> &ChannelMember {
        match &self.owner {
            Some(o) => o.member(),
            None => self
                .member
                .as_ref()
                .expect("a channel has an owner or a member"),
        }
    }

    fn member_mut(&mut self) -> &mut ChannelMember {
        match &mut self.owner {
            Some(o) => o.member_mut(),
            None => self
                .member
                .as_mut()
                .expect("a channel has an owner or a member"),
        }
    }

    fn owner(&self) -> Result<&ChannelOwner, PhoneError> {
        self.owner.as_ref().ok_or_else(|| channel(OWNER_ONLY))
    }

    fn owner_mut(&mut self) -> Result<&mut ChannelOwner, PhoneError> {
        self.owner.as_mut().ok_or_else(|| channel(OWNER_ONLY))
    }

    fn is_owner(&self) -> bool {
        self.owner.is_some()
    }

    /// The core's state, then the record.
    fn persist(&self) -> Result<(), PhoneError> {
        self.save_state()?;
        self.meta.save(&self.dir)
    }

    fn save_state(&self) -> Result<(), PhoneError> {
        match &self.owner {
            Some(o) => store::write_atomic(&self.dir.join(store::OWNER), &o.export()),
            None => store::write_atomic(&self.dir.join(store::MEMBER), &self.member().export()),
        }
    }

    fn can_post(&self) -> bool {
        !self.meta.dead && (self.is_owner() || self.meta.grant_chain.is_some())
    }

    fn read_auth(&self) -> Auth {
        if self.is_owner() {
            return Auth::Proof;
        }
        match self.member().gate_token() {
            Some(t) if !self.meta.use_proof => Auth::Gate(t),
            _ if self.meta.grant_chain.is_some() => Auth::Proof,
            _ => Auth::None,
        }
    }

    fn host(&self) -> String {
        host_of(&self.meta.relay)
    }

    fn summary(&self, root: &std::path::Path) -> ChannelSummary {
        let owner = self.is_owner();
        let circle = self.meta.circle.clone();
        ChannelSummary {
            id: self.id.clone(),
            name: self.meta.name.clone(),
            role: if owner {
                ChannelRole::Owner
            } else {
                ChannelRole::Member
            },
            policy_line: self.meta.policy_line.clone(),
            relay: self.host(),
            dead: self.meta.dead,
            can_post: self.can_post(),
            waiting: !owner && self.meta.grant_chain.is_none() && self.meta.requested,
            unread: self.meta.unread,
            may_ask_to_post: self.member().may_ask_to_post(),
            closed: self.closed(),
            circle: (owner || store::circle_ids(root).contains(&circle)).then_some(circle),
        }
    }

    /// Only people the owner lets in read: a member with no link secret and no grant, or a
    /// policy whose read rule takes no evidence.
    fn closed(&self) -> bool {
        let shut_out = !self.is_owner()
            && self.member().gate_token().is_none()
            && self.meta.grant_chain.is_none();
        shut_out || read_is_closed(&self.meta.policy)
    }

    fn name_for(&self, kid: &[u8; 32]) -> String {
        let k = hex::encode(kid);
        if let Some(o) = &self.owner {
            return o
                .members()
                .iter()
                .find(|m| m.kid == *kid)
                .map_or_else(|| short(&k), |m| m.name.clone());
        }
        if self.meta.owner_kids.contains(&k) {
            "owner".into()
        } else {
            short(&k)
        }
    }

    fn actions_of(&self, grant_id: &str) -> String {
        self.meta
            .actions
            .iter()
            .find(|(g, _)| g == grant_id)
            .map_or_else(|| "read".into(), |(_, a)| a.clone())
    }
}

/// The `Signer` for the owner's seat, and the check after each core call that signed.
fn signed<T>(seat: &mut Seat<'_>, r: Result<T, String>) -> Result<T, PhoneError> {
    if let Some(e) = seat.error.take() {
        return Err(e);
    }
    r.map_err(ch_err)
}

impl Phone {
    fn channel_dir(&self, id: &str) -> PathBuf {
        store::channels_dir(&self.dir).join(id)
    }

    fn open_channel(&self, id: &str) -> Result<Ch, PhoneError> {
        check_id(id)?;
        let dir = self.channel_dir(id);
        if !dir.join(store::META).is_file() {
            return Err(crate::not_found("channel"));
        }
        Ch::load(dir, id)
    }

    fn seat_for(&self, meta: &ChannelMeta) -> Result<Seat<'_>, PhoneError> {
        let public = SeatPublic::parse(&meta.seat_public).map_err(|_| PhoneError::Storage {
            message: "This channel's record on this phone does not read.".into(),
        })?;
        Ok(Seat {
            signer: self.signer.as_ref(),
            label: meta.seat_label.clone(),
            public,
            error: None,
        })
    }

    // ------------------------------------------------------------------------- the relay

    fn send_raw(
        &self,
        base: &str,
        method: &str,
        path: &str,
        headers: Vec<HttpHeader>,
        body: Option<Vec<u8>>,
    ) -> R<Vec<u8>> {
        let url = format!("{}{path}", base.trim_end_matches('/'));
        let HttpResponse { status, body } = self.http.request(method.into(), url, headers, body)?;
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        Err(Fail::Relay {
            status,
            code: v["error"].as_str().unwrap_or("").into(),
            reason: v["reason"].as_str().unwrap_or("").into(),
        })
    }

    /// One relay call for a channel, with the read credential `auth`.
    fn call(
        &self,
        ch: &Ch,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        auth: Auth,
    ) -> R<Vec<u8>> {
        let base = ch.meta.relay.clone();
        let mut headers = Vec::new();
        let header = |name: &str, value: String| HttpHeader {
            name: name.into(),
            value,
        };
        if body.is_some() {
            headers.push(header("Content-Type", "application/octet-stream".into()));
        }
        match auth {
            Auth::None => {}
            Auth::Gate(token) => headers.push(header("Cahoots-Gate", hex::encode(token))),
            Auth::Proof => {
                let v = json(&self.send_raw(&base, "GET", "/v1/nonce", Vec::new(), None)?)?;
                let nonce: [u8; 16] = hex::decode(v["nonce"].as_str().unwrap_or(""))
                    .ok()
                    .and_then(|n| n.try_into().ok())
                    .ok_or_else(|| Fail::Phone(unreadable()))?;
                let (p, q) = path.split_once('?').unwrap_or((path, ""));
                let proof = ch.member().proof_for(&format!("{method} {p}?{q}"), &nonce);
                headers.push(header("Cahoots-Proof", URL_SAFE_NO_PAD.encode(proof)));
                headers.push(header("Cahoots-Kid", hex::encode(ch.member().my_kid())));
            }
        }
        self.send_raw(&base, method, path, headers, body)
    }

    fn post(&self, ch: &Ch, path: &str, body: Vec<u8>) -> R<Vec<u8>> {
        self.call(ch, "POST", path, Some(body), Auth::None)
    }

    /// A message post; a 409 replay means the relay already holds it.
    fn post_message(&self, ch: &Ch, wire: Vec<u8>) -> R<Option<u64>> {
        match self.post(ch, &format!("/v1/channels/{}/messages", ch.id), wire) {
            Ok(b) => Ok(json(&b)?["seq"].as_u64()),
            Err(Fail::Relay {
                status: 409, code, ..
            }) if code == "replay" => Ok(None),
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------------------- owner actions

    /// A new epoch: deposits for the live link and every admitted member, then the rotation
    /// control.
    fn start_new_keys(&self, ch: &mut Ch) -> R<()> {
        let wire = ch.owner_mut()?.rotate().map_err(ch_err)?;
        ch.persist()?;
        for d in ch.owner()?.deposits().to_vec() {
            self.post(ch, &format!("/v1/channels/{}/deposits", ch.id), d)?;
        }
        self.post_message(ch, wire)?;
        Ok(())
    }

    /// Register the current link's gate token with the relay, if it is not registered yet.
    fn register_gate(&self, ch: &mut Ch) -> R<bool> {
        let Some(hash) = ch.owner()?.gate_token_hash().map(hex::encode) else {
            return Ok(false);
        };
        if ch.meta.gate_hash.as_deref() == Some(hash.as_str()) {
            return Ok(false);
        }
        let mut seat = self.seat_for(&ch.meta)?;
        let r = ch.owner()?.gate_registration(&mut seat);
        let body = signed(&mut seat, r)?;
        self.post(ch, &format!("/v1/channels/{}/gates", ch.id), body)?;
        ch.meta.gate_hash = Some(hash);
        ch.persist()?;
        Ok(true)
    }

    /// Retire the live link: the relay refuses its token, and later epochs have no deposit
    /// for it. The secret is forgotten at once and the signed statement kept with the owner's
    /// state until the relay confirms; a `DELETE` that fails here is sent again on each poll.
    fn retire(&self, ch: &mut Ch) -> R<bool> {
        if ch.owner()?.gate_token_hash().is_none() {
            return Ok(false);
        }
        let mut seat = self.seat_for(&ch.meta)?;
        let r = ch.owner_mut()?.retire_link(&mut seat);
        signed(&mut seat, r)?;
        ch.meta.gate_hash = None;
        ch.persist()?;
        self.flush_retires(ch)?;
        Ok(true)
    }

    /// Send every retire statement the relay has not confirmed, oldest first. A 204, or a 404
    /// (the relay no longer holds the token), clears it; anything else stops and keeps the
    /// rest for the next try.
    fn flush_retires(&self, ch: &mut Ch) -> R<()> {
        let pending = ch.owner()?.pending_retire().to_vec();
        for (hash, statement) in pending {
            let path = format!("/v1/channels/{}/gates/{}", ch.id, hex::encode(hash));
            match self.call(ch, "DELETE", &path, Some(statement), Auth::None) {
                Ok(_) => {}
                Err(e) if is_status(&e, 404) => {}
                Err(e) => return Err(e),
            }
            ch.owner_mut()?.retire_confirmed(&hash);
            ch.save_state()?;
        }
        Ok(())
    }

    /// A new link (the policy's link evidence is the current link, so no policy change), and
    /// its gate token registered.
    fn issue_link(&self, ch: &mut Ch) -> R<String> {
        let host = ch.host();
        let url = ch.owner_mut()?.link_url(&host).map_err(ch_err)?;
        ch.persist()?;
        self.register_gate(ch)?;
        Ok(url)
    }

    /// The live link, or `None` without one.
    fn live_link(&self, ch: &mut Ch) -> Result<Option<String>, PhoneError> {
        if ch.owner()?.gate_token_hash().is_none() {
            return Ok(None);
        }
        let host = ch.host();
        ch.owner_mut()?.link_url(&host).map(Some).map_err(ch_err)
    }

    /// The join requests not yet allowed or refused, with their bytes. A request that does
    /// not check out is set aside as handled.
    fn fetch_requests(&self, ch: &mut Ch) -> R<Vec<(String, Vec<u8>, JoinRequest)>> {
        let path = format!("/v1/channels/{}/requests", ch.id);
        let v = json(&self.call(ch, "GET", &path, None, Auth::Proof)?)?;
        let channel_id = ch.member().channel_id();
        let mut out = Vec::new();
        let mut set_aside = false;
        for r in v["requests"].as_array().into_iter().flatten() {
            let rid = r["id"].as_str().unwrap_or("").to_string();
            if rid.is_empty() || ch.meta.handled.contains(&rid) {
                continue;
            }
            let body = b64(r["body"].as_str().unwrap_or(""))?;
            match JoinRequest::parse(&body) {
                Ok(j) if j.channel == channel_id => out.push((rid, body, j)),
                _ => {
                    ch.meta.handled.push(rid);
                    set_aside = true;
                }
            }
        }
        if set_aside {
            ch.meta.save(&ch.dir)?;
        }
        Ok(out)
    }

    /// Admit one join request with `actions`; a `post` the policy does not allow falls back
    /// to read. Returns what was given.
    fn admit_request(&self, ch: &mut Ch, bytes: &[u8], actions: &str) -> R<String> {
        let mut seat = self.seat_for(&ch.meta)?;
        let mut given = actions.to_string();
        let first = ch.owner_mut()?.admit(bytes, &given, &mut seat);
        if let Some(e) = seat.error.take() {
            return Err(e.into());
        }
        let a = match first {
            Ok(a) => a,
            Err(code) if code == "policy" && given.contains("post") => {
                given = "read".into();
                let again = ch.owner_mut()?.admit(bytes, &given, &mut seat);
                signed(&mut seat, again)?
            }
            Err(code) => return Err(ch_err(code).into()),
        };
        ch.meta
            .actions
            .push((hex::encode(a.grant_id), given.clone()));
        ch.persist()?;
        self.post(ch, &format!("/v1/channels/{}/grants", ch.id), a.chain)?;
        self.post(ch, &format!("/v1/channels/{}/deposits", ch.id), a.deposit)?;
        self.post_message(ch, a.control)?;
        Ok(given)
    }

    // ------------------------------------------------------------------------- reading

    fn chains_for(&self, ch: &mut Ch, kid: &[u8; 32], fresh: bool) -> R<Vec<Vec<u8>>> {
        if !fresh {
            if let Some(c) = ch.chains.get(kid) {
                return Ok(c.clone());
            }
        }
        let path = format!("/v1/channels/{}/grants?kid={}", ch.id, hex::encode(kid));
        let v = json(&self.call(ch, "GET", &path, None, ch.read_auth())?)?;
        let list = b64_list(&v, "grants")?;
        ch.chains.insert(*kid, list.clone());
        Ok(list)
    }

    /// The deposits sealed to `to` for `epoch`. A refused link token (the link was just
    /// retired) falls back to a proof by a member with a grant.
    fn deposits(&self, ch: &mut Ch, to: &[u8; 32], epoch: &[u8; 20]) -> R<Vec<Vec<u8>>> {
        let path = format!(
            "/v1/channels/{}/deposits?to={}&epoch={}",
            ch.id,
            hex::encode(to),
            hex::encode(epoch)
        );
        let auth = ch.read_auth();
        if matches!(auth, Auth::None) {
            return Ok(Vec::new());
        }
        match self.call(ch, "GET", &path, None, auth) {
            Ok(b) => b64_list(&json(&b)?, "deposits"),
            Err(e) if is_status(&e, 401) => {
                if !matches!(auth, Auth::Gate(_)) || ch.meta.grant_chain.is_none() {
                    return Ok(Vec::new());
                }
                ch.meta.use_proof = true;
                match self.call(ch, "GET", &path, None, Auth::Proof) {
                    Ok(b) => b64_list(&json(&b)?, "deposits"),
                    Err(e) if is_status(&e, 401) => Ok(Vec::new()),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Fetch and open this member's deposit for an epoch: the link's first, then one sealed
    /// to its key. Each deposit is signed; it is installed only once its signer's grant chain
    /// (read by the deposit's kid) verifies against the owner circle and carries `govern`. One
    /// that does not is passed over for the next. `ids`, the deposits a rotation control names,
    /// limits the deposits tried to those when the control is at hand.
    fn fetch_epoch(&self, ch: &mut Ch, epoch: &[u8; 20], ids: Option<&[[u8; 32]]>) -> R<bool> {
        let mut places = Vec::new();
        if let Some(link_id) = ch.member().link_id() {
            places.push((link_id, true));
        }
        places.push((ch.member().my_kid(), false));
        for (to, by_link) in places {
            for d in self.deposits(ch, &to, epoch)? {
                if self.install_deposit(ch, &d, by_link, ids)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Install one signed deposit if its signer checks out; `Ok(false)` when it does not.
    fn install_deposit(
        &self,
        ch: &mut Ch,
        signed: &[u8],
        by_link: bool,
        ids: Option<&[[u8; 32]]>,
    ) -> R<bool> {
        let Ok((kid, d)) = peek_signed(signed) else {
            return Ok(false);
        };
        if ids.is_some_and(|ids| !ids.contains(&d.id())) {
            return Ok(false);
        }
        let manifest = ch.meta.manifest.to_vec();
        for fresh in [false, true] {
            for chain in self.chains_for(ch, &kid, fresh)? {
                let m = ch.member_mut();
                let installed = if by_link {
                    m.install_from_link_deposit(signed, &chain, &manifest)
                } else {
                    m.install_from_key_deposit(signed, &chain, &manifest)
                };
                if installed.is_ok() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Take the policy the relay now holds, once it checks out: the channel object is verified
    /// against the owner circle's member list this phone keeps, names this channel, and names
    /// the policy by its SHA-256. Keep the one held if anything does not.
    fn refresh_policy(&self, ch: &mut Ch) {
        let path = format!("/v1/channels/{}", ch.id);
        let Ok(v) = self
            .call(ch, "GET", &path, None, Auth::None)
            .and_then(|b| json(&b))
        else {
            return;
        };
        let (Some(Ok(policy)), Some(Ok(object))) = (
            v["policy"].as_str().map(b64),
            v["channel"].as_str().map(b64),
        ) else {
            return;
        };
        let Ok(manifest) = open_manifest(&ch.meta.manifest) else {
            return;
        };
        let Ok((_, obj)) = ChannelObject::verify(&object, &manifest) else {
            return;
        };
        if obj.channel_id != ch.member().channel_id()
            || <[u8; 32]>::from(Sha256::digest(&policy)) != obj.policy_id
        {
            return;
        }
        if ch.member_mut().set_policy(&policy).is_ok() {
            if let Ok(line) = policy_summary(&policy) {
                ch.meta.policy_line = line;
            }
            ch.meta.policy = policy;
        }
    }

    /// Read one entry of the stream: a line to keep, or `None`.
    fn take(
        &self,
        ch: &mut Ch,
        seq: u64,
        msg: &[u8],
        time: u64,
    ) -> R<Option<ChannelMessageRecord>> {
        if msg.first() != Some(&4) || msg.len() < 78 {
            return Ok(None);
        }
        let kid: [u8; 32] = msg[37..69].try_into().expect("32 bytes");
        let epoch: [u8; 20] = msg[17..37].try_into().expect("20 bytes");
        let number = EpochId::decode(&epoch).map_or(0, |e| e.epoch);
        ch.meta.max_epoch = ch.meta.max_epoch.max(number);
        if kid == ch.member().my_kid() {
            return Ok(None); // our own: kept when it was sent
        }
        let owner = ch.is_owner();
        let manifest = ch.meta.manifest.to_vec();
        let mut got = None;
        let mut error = String::from("no-grant");
        for fresh in [false, true] {
            for chain in self.chains_for(ch, &kid, fresh)? {
                match ch.member_mut().receive(msg, &chain, &manifest) {
                    Ok(r) => {
                        got = Some(r);
                        break;
                    }
                    Err(e) => {
                        error = e;
                        if error == "unknown-epoch"
                            && !owner
                            && self.fetch_epoch(ch, &epoch, None)?
                        {
                            match ch.member_mut().receive(msg, &chain, &manifest) {
                                Ok(r) => {
                                    got = Some(r);
                                    break;
                                }
                                Err(e) => error = e,
                            }
                        }
                        if !is_grant_error(&error) {
                            break;
                        }
                    }
                }
            }
            if got.is_some() || !is_grant_error(&error) {
                break;
            }
        }
        let k = hex::encode(kid);
        let line = |sender: String, text: String, kind: u8, ok: bool| ChannelMessageRecord {
            id: format!("relay-{seq}"),
            kid: k.clone(),
            sender,
            mine: false,
            text,
            kind,
            ok,
            seq: Some(seq),
            time,
        };
        let Some(r) = got else {
            // Keys this phone never held, or a message read before (a crash between the
            // state and the cursor): passed over.
            if error == "unknown-epoch" || error == "reuse" {
                return Ok(None);
            }
            return Ok(Some(line(short(&k), friendly(&error), 3, false)));
        };
        match r.msg_type {
            MessageType::Control => {
                if !ch.meta.owner_kids.contains(&k) {
                    ch.meta.owner_kids.push(k.clone());
                }
                match r.control {
                    Some(Control::Rotation { epoch_id, deposits }) if !owner => {
                        // The control is read now; a failed fetch is tried again after the
                        // sync, never by reading the control again.
                        match self.fetch_epoch(ch, &epoch_id.encode(), Some(&deposits)) {
                            Ok(true) => {}
                            Ok(false) => ch.meta.behind = true,
                            Err(_) => ch.meta.retry_epoch = Some(epoch_id.encode().to_vec().into()),
                        }
                        Ok(None)
                    }
                    Some(Control::PolicyChange { .. }) => {
                        self.refresh_policy(ch);
                        let text = format!("The policy changed: {}.", ch.meta.policy_line);
                        Ok(Some(line(String::new(), text, 2, true)))
                    }
                    Some(Control::Revocation { .. }) => {
                        Ok(Some(line(String::new(), NOTE_REVOKED.into(), 2, true)))
                    }
                    _ => Ok(None),
                }
            }
            t => {
                let text =
                    String::from_utf8_lossy(bucket_unpad(&r.text).unwrap_or(&r.text)).into_owned();
                let announcement = t == MessageType::Announcement;
                let ok = !announcement || r.announcement_ok;
                Ok(Some(line(
                    ch.name_for(&kid),
                    text,
                    u8::from(announcement),
                    ok,
                )))
            }
        }
    }

    /// One pass over what is new: read to the end, then, for a member, check the keys are
    /// current (or the link is dead) and look for a grant asked for.
    fn poll_channel(&self, ch: &mut Ch) -> R<Vec<ChannelMessage>> {
        let mut out = Vec::new();
        if ch.meta.dead {
            return Ok(out);
        }
        // A link retired here whose removal the relay never confirmed: send it again. A
        // relay that cannot be reached now is tried on the next poll.
        if ch.is_owner() {
            let _ = self.flush_retires(ch);
        }
        let mut history = ChannelMessages::load(&ch.dir)?;
        loop {
            let auth = ch.read_auth();
            if matches!(auth, Auth::None) {
                return Ok(out); // no link and no grant: nothing to read
            }
            let q = match ch.meta.seq {
                Some(s) => format!("after={s}&wait=0"),
                None => "wait=0".into(),
            };
            let path = format!("/v1/channels/{}/messages?{q}", ch.id);
            let body = match self.call(ch, "GET", &path, None, auth) {
                Ok(b) => b,
                Err(e) if is_status(&e, 401) && !ch.is_owner() => {
                    if matches!(auth, Auth::Gate(_))
                        && ch.meta.grant_chain.is_some()
                        && !ch.meta.use_proof
                    {
                        ch.meta.use_proof = true;
                        continue;
                    }
                    ch.meta.dead = true;
                    ch.persist()?;
                    return Ok(out);
                }
                Err(e) => return Err(e),
            };
            let v = json(&body)?;
            let mut entries = Vec::new();
            for e in v["entries"].as_array().into_iter().flatten() {
                let seq = e["seq"].as_u64().ok_or_else(|| Fail::Phone(unreadable()))?;
                let msg = b64(e["message"].as_str().unwrap_or(""))?;
                entries.push((seq, msg, e["received_at"].as_u64().unwrap_or(0)));
            }
            let mut failed = None;
            for (seq, msg, time) in &entries {
                match self.take(ch, *seq, msg, *time) {
                    Ok(line) => {
                        ch.meta.seq = Some(*seq);
                        if let Some(m) = line {
                            if m.kind <= 1 {
                                ch.meta.unread = ch.meta.unread.saturating_add(1);
                            }
                            out.push(to_message(&m));
                            history.messages.push(m);
                        }
                    }
                    // A fetch inside failed before the entry was read: it is read again next
                    // time.
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            if !entries.is_empty() {
                // The state, the lines it read, then the cursor. Read again after a crash, an
                // entry is passed over as a replay, and its line is already kept.
                ch.save_state()?;
                history.save(&ch.dir)?;
                ch.meta.save(&ch.dir)?;
            }
            if let Some(e) = failed {
                return Err(e);
            }
            if entries.is_empty() {
                break;
            }
        }
        if !ch.is_owner() {
            if let Some(epoch) = ch.meta.retry_epoch.clone() {
                let epoch: [u8; 20] = epoch[..].try_into().unwrap_or([0; 20]);
                match self.fetch_epoch(ch, &epoch, None) {
                    Ok(found) => {
                        ch.meta.behind |= !found;
                        ch.meta.retry_epoch = None;
                    }
                    Err(e) => {
                        ch.persist()?;
                        return Err(e);
                    }
                }
            }
            // A link with no deposit at the channel's current keys reads nothing new.
            let current = ch.member().current_epoch().map_or(0, |e| e.epoch);
            if ch.meta.behind || current == 0 || ch.meta.max_epoch > current {
                ch.meta.dead = true;
                ch.persist()?;
                return Ok(out);
            }
            self.look_for_grant(ch);
        }
        ch.persist()?;
        Ok(out)
    }

    /// After asking to post: take the grant once the owner has issued it.
    fn look_for_grant(&self, ch: &mut Ch) {
        if ch.meta.grant_chain.is_some() || !ch.meta.requested {
            return;
        }
        let auth = match ch.read_auth() {
            Auth::None => Auth::Proof,
            a => a,
        };
        let path = format!(
            "/v1/channels/{}/grants?kid={}",
            ch.id,
            hex::encode(ch.member().my_kid())
        );
        let Ok(list) = self
            .call(ch, "GET", &path, None, auth)
            .and_then(|b| b64_list(&json(&b)?, "grants"))
        else {
            return;
        };
        if let Some(chain) = list.last() {
            if ch.member_mut().set_grant(chain).is_ok() {
                ch.meta.grant_chain = Some(chain.clone().into());
            }
        }
    }

    fn keep_line(&self, ch: &Ch, line: ChannelMessageRecord) -> Result<(), PhoneError> {
        let mut history = ChannelMessages::load(&ch.dir)?;
        history.messages.push(line);
        history.save(&ch.dir)
    }

    fn mine(
        &self,
        ch: &Ch,
        text: &str,
        kind: u8,
        seq: Option<u64>,
    ) -> Result<ChannelMessageRecord, PhoneError> {
        Ok(ChannelMessageRecord {
            id: crate::chat::random_id()?,
            kid: ch.meta.kid.clone(),
            sender: "you".into(),
            mine: true,
            text: text.into(),
            kind,
            ok: true,
            seq,
            time: now_ms(),
        })
    }

    /// The channel a link names, as the relay holds it: the object checked against the owner
    /// circle's member list, its policy the one the object names. Nothing is stored.
    fn fetch_channel(&self, link: &Link) -> R<Fetched> {
        let base = relay_base(&link.relay);
        let id = hex::encode(link.channel_id);
        let v = json(&self.send_raw(
            &base,
            "GET",
            &format!("/v1/channels/{id}"),
            Vec::new(),
            None,
        )?)?;
        let policy = match v["policy"].as_str() {
            Some(p) => b64(p)?,
            None => return Err(channel("The relay holds no policy for this channel.").into()),
        };
        let manifest = b64(v["manifest"].as_str().unwrap_or(""))?;
        let object = b64(v["channel"].as_str().unwrap_or(""))?;
        let m = open_manifest(&manifest).map_err(ch_err)?;
        let (_, obj) = ChannelObject::verify(&object, &m)
            .map_err(|e| ch_err(cahoots_channel::member::code(e)))?;
        if obj.channel_id != link.channel_id {
            return Err(channel("The relay answered with another channel.").into());
        }
        if <[u8; 32]>::from(Sha256::digest(&policy)) != obj.policy_id {
            return Err(channel("The relay's copy of this channel does not check out.").into());
        }
        let line = policy_summary(&policy).map_err(ch_err)?;
        Ok(Fetched {
            obj,
            policy,
            manifest,
            line,
        })
    }

    fn join(&self, url: &str) -> R<Ch> {
        let link = Link::parse(url).ok_or_else(|| PhoneError::BadCode {
            message: "That link does not read.".into(),
        })?;
        let id = hex::encode(link.channel_id);
        let dir = self.channel_dir(&id);
        if dir.join(store::META).is_file() {
            let mut ch = Ch::load(dir, &id)?;
            // An owner's link always goes to its own channel.
            if ch.is_owner() {
                return Ok(ch);
            }
            // A link to the same channel on another relay is refused: the channel's keys,
            // cursor and grant belong to the relay it was read on.
            if !same_relay(&ch.meta.relay, &link.relay) {
                return Err(channel(OTHER_RELAY).into());
            }
            if link.secret.is_none() {
                return Ok(ch);
            }
            // A newer link for a channel this phone already reads: take its secret, keep the
            // key and grant.
            let newer = link.secret.as_ref().map(|s| s.gate_token(&link.channel_id))
                != ch.member().gate_token();
            if newer || ch.meta.dead {
                ch.member_mut().set_link(url).map_err(ch_err)?;
                ch.meta.relay = relay_base(&link.relay);
                ch.meta.dead = false;
                ch.meta.use_proof = false;
                ch.meta.behind = false;
                ch.meta.max_epoch = 0;
                ch.meta.retry_epoch = None;
                ch.persist()?;
            }
            return Ok(ch);
        }
        let base = relay_base(&link.relay);
        let Fetched {
            obj,
            policy,
            manifest,
            line,
        } = self.fetch_channel(&link)?;
        if link.secret.is_none() {
            return Err(channel(CLOSED).into());
        }
        let mut member = ChannelMember::from_link(url).map_err(ch_err)?;
        member.set_policy(&policy).map_err(ch_err)?;
        let meta = ChannelMeta {
            owner: false,
            circle: hex::encode(obj.circle_id),
            relay: base,
            name: obj.name.clone(),
            policy,
            policy_line: line,
            manifest,
            kid: hex::encode(member.my_kid()),
            ..ChannelMeta::default()
        };
        std::fs::create_dir_all(&dir).map_err(store::io)?;
        let ch = Ch {
            dir,
            id,
            meta,
            owner: None,
            member: Some(member),
            chains: HashMap::new(),
        };
        ch.persist()?;
        Ok(ch)
    }
}

#[uniffi::export]
impl Phone {
    /// Start a channel owned by `circle`, signed by this phone's seat: `preset` 0
    /// announcements, 1 announcements with approvals (a majority of the circle), 2 group chat,
    /// 3 members post, 4 closed. The circle's relay must carry channels and gates. The
    /// channel, its policy, the owner's grant, the link's gate and the first keys go up in
    /// that order; the seat key signs four or five times, each through the `Signer`.
    pub fn channel_start(
        &self,
        circle: String,
        name: String,
        preset: u8,
    ) -> Result<ChannelSummary, PhoneError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(channel("Give the channel a name."));
        }
        let (manifest, seat, label, public, relay, seats) = self.with_circle(&circle, |c, _| {
            Ok((
                c.record.manifest.to_vec(),
                c.record.seat,
                c.record.seat_label.clone(),
                c.record.seat_public.to_vec(),
                c.settings.relay.clone(),
                c.params.n,
            ))
        })?;
        if preset == 1 && majority(seats) > 1 {
            return Err(channel(NEEDS_SEATS));
        }
        let _guard = self.lock();
        let host = host_of(&relay);
        let base = relay.clone();
        let info = self
            .send_raw(&base, "GET", "/v1/info", Vec::new(), None)
            .and_then(|b| json(&b))?;
        let features: Vec<&str> = info["features"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str())
            .collect();
        if !features.contains(&"channels") || !features.contains(&"gate") {
            return Err(channel(&format!(
                "{host} does not carry channels yet. Point this circle at a relay that does, from its relay address."
            )));
        }
        let mut meta = ChannelMeta {
            owner: true,
            circle: circle.clone(),
            relay: base,
            name: name.clone(),
            preset: Some(preset),
            quorum: if preset == 1 { majority(seats) } else { 0 },
            seat_label: label,
            seat_public: public,
            ..ChannelMeta::default()
        };
        let mut seat_key = self.seat_for(&meta)?;
        let started = ChannelOwner::start(&manifest, seat, &mut seat_key, &name, preset, 0);
        let mut owner = signed(&mut seat_key, started)?;
        let closed = preset == 4;
        if !closed {
            owner.link_url(&host).map_err(ch_err)?;
        }
        let id = hex::encode(owner.channel_id());
        meta.policy = owner.policy().to_vec();
        meta.policy_line = policy_summary(owner.policy()).map_err(ch_err)?;
        meta.grant_chain = Some(owner.self_grant().into());
        meta.manifest = owner.manifest().to_vec();
        meta.kid = hex::encode(owner.member().my_kid());
        let dir = self.channel_dir(&id);
        std::fs::create_dir_all(&dir).map_err(store::io)?;
        let mut ch = Ch {
            dir,
            id: id.clone(),
            meta,
            owner: Some(owner),
            member: None,
            chains: HashMap::new(),
        };
        ch.persist()?;
        let run = |ch: &mut Ch| -> R<()> {
            let o = ch.owner()?;
            let (object, policy_body, grant) =
                (o.channel_object().to_vec(), o.policy_body(), o.self_grant());
            self.call(
                ch,
                "PUT",
                &format!("/v1/circles/{circle}/manifest"),
                Some(ch.meta.manifest.to_vec()),
                Auth::None,
            )?;
            self.post(ch, "/v1/channels", object)?;
            self.post(ch, &format!("/v1/channels/{id}/policy"), policy_body)?;
            self.post(ch, &format!("/v1/channels/{id}/grants"), grant)?;
            if !closed {
                self.register_gate(ch)?;
            }
            self.start_new_keys(ch)
        };
        run(&mut ch)?;
        Ok(ch.summary(&self.dir))
    }

    /// Every channel on this phone, in id order.
    pub fn channels(&self) -> Vec<ChannelSummary> {
        let _guard = self.lock();
        store::channel_ids(&self.dir)
            .into_iter()
            .filter_map(|id| self.open_channel(&id).ok())
            .map(|ch| ch.summary(&self.dir))
            .collect()
    }

    pub fn channel(&self, id: String) -> Result<ChannelSummary, PhoneError> {
        let _guard = self.lock();
        Ok(self.open_channel(&id)?.summary(&self.dir))
    }

    /// The owner's invite link: `https://cahoots.network/app/#/ch/<host>/<id>/<secret>`.
    /// Anyone it reaches can read until it is retired. A channel whose link was lost on the
    /// way (a retire that did not finish) gets a new one, and new keys. A closed channel has
    /// none: `PhoneError::Channel`.
    pub fn channel_link(&self, id: String) -> Result<String, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        if let Some(url) = self.live_link(&mut ch)? {
            return Ok(url);
        }
        if ch.meta.preset == Some(4) {
            return Err(channel(NO_LINK));
        }
        let url = self.issue_link(&mut ch)?;
        self.start_new_keys(&mut ch)?;
        Ok(url)
    }

    /// What a channel link opens, before joining: read from the relay the link names, its
    /// signatures checked against the owner circle's member list. Nothing is kept. A link that
    /// does not read is `PhoneError::BadCode`.
    pub fn channel_preview(&self, url: String) -> Result<ChannelPreview, PhoneError> {
        let link = Link::parse(&url).ok_or_else(|| PhoneError::BadCode {
            message: "That link does not read.".into(),
        })?;
        let f = self.fetch_channel(&link)?;
        Ok(ChannelPreview {
            name: f.obj.name,
            policy_line: f.line,
            relay: link.relay.clone(),
            channel_id: hex::encode(link.channel_id),
            has_secret: link.secret.is_some(),
        })
    }

    /// Open a channel link: read the channel from the relay the link names, check its
    /// signatures against the owner circle's member list, keep it, and read what is there.
    /// A newer link to a channel already kept takes the place of the old one, keeping the key
    /// and grant; a link to it on another relay is refused and changes nothing. A link
    /// without its secret (a closed channel) is refused with the words to show; one that does
    /// not read is `PhoneError::BadCode`.
    pub fn channel_join(&self, url: String) -> Result<ChannelSummary, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.join(&url)?;
        // The first read; a relay that cannot be reached now is read on the next poll.
        let _ = self.poll_channel(&mut ch);
        let ch = self.open_channel(&ch.id)?;
        Ok(ch.summary(&self.dir))
    }

    /// Read everything new: messages, keys, policy changes, and the grant once the owner
    /// gives it. Returns the lines kept, oldest first. A member whose link stopped working
    /// comes back `dead`; a dead channel reads nothing.
    pub fn channel_poll(&self, id: String) -> Result<Vec<ChannelMessage>, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        Ok(self.poll_channel(&mut ch)?)
    }

    /// Every line kept, oldest first. Clears the unread count.
    pub fn channel_history(&self, id: String) -> Result<Vec<ChannelMessage>, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        let history = ChannelMessages::load(&ch.dir)?;
        if ch.meta.unread != 0 {
            ch.meta.unread = 0;
            ch.meta.save(&ch.dir)?;
        }
        Ok(history.messages.iter().map(to_message).collect())
    }

    /// Post `text`. The owner posts in every channel; a member once the owner let it in. In
    /// an announcements channel with approvals the owner's own approval goes with it; one
    /// that needs more approvals than that is refused.
    pub fn channel_send(&self, id: String, text: String) -> Result<(), PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        if text.trim().is_empty() {
            return Ok(());
        }
        if !ch.can_post() {
            return Err(channel(if ch.meta.dead {
                DEAD
            } else if ch.meta.requested {
                WAITING
            } else {
                READ_ONLY
            }));
        }
        let pt = bucket_pad(text.as_bytes()).ok_or_else(|| channel("That message is too long."))?;
        let proposing = ch.is_owner() && ch.meta.preset == Some(1);
        let wire = if proposing {
            let need = ch.meta.quorum.max(1);
            if need > 1 {
                return Err(channel(&format!(
                    "This channel needs {need} approvals to post, which this app cannot collect yet."
                )));
            }
            ch.member_mut().propose_announcement(&pt).map_err(ch_err)?;
            let digest = ch.member().announcement_digest().map_err(ch_err)?;
            let first_line: String = text
                .split('\n')
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            let log = ch.meta.seq.unwrap_or(0);
            let mut seat = self.seat_for(&ch.meta)?;
            let name = ch.meta.name.clone();
            let r = ch
                .owner()?
                .approve_post(&digest, &name, &first_line, log, &mut seat);
            let mine = signed(&mut seat, r)?;
            ch.member_mut()
                .send_proposed(&bytes_array(&[&mine]))
                .map_err(ch_err)?
        } else {
            ch.member_mut().send(&pt).map_err(ch_err)?
        };
        // The chain step is recorded before the ciphertext leaves.
        ch.persist()?;
        let seq = self.post_message(&ch, wire)?;
        let line = self.mine(&ch, &text, u8::from(proposing), seq)?;
        self.keep_line(&ch, line)
    }

    /// Ask the owner to let this member post, under `name` (shown to the owner, and to
    /// members beside its posts). The grant is looked for on each poll; until then the
    /// channel is `waiting`.
    pub fn channel_ask_to_post(&self, id: String, name: String) -> Result<(), PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        if ch.is_owner() || ch.meta.grant_chain.is_some() {
            return Ok(());
        }
        if ch.meta.dead {
            return Err(channel(DEAD));
        }
        if !ch.member().may_ask_to_post() {
            return Err(channel("This channel lets only the owner post."));
        }
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(channel("Give a name, so the owner knows who is asking."));
        }
        let request = ch
            .member()
            .join_request(&name, "read,post")
            .map_err(ch_err)?;
        self.post(&ch, &format!("/v1/channels/{id}/requests"), request)?;
        ch.meta.requested = true;
        ch.meta.wants_post = true;
        ch.meta.my_name = name;
        ch.persist()
    }

    /// The owner's waiting join requests: who asks, and for what. A request that does not
    /// check out is set aside.
    pub fn channel_requests(&self, id: String) -> Result<Vec<ChannelRequest>, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        ch.owner()?;
        Ok(self
            .fetch_requests(&mut ch)?
            .into_iter()
            .map(|(rid, _, j)| ChannelRequest {
                id: rid,
                name: j.name,
                actions: j.wanted,
            })
            .collect())
    }

    /// What a request may be let in for without the owner choosing, as "read,post": the
    /// actions it asks for that the live link proves (`ChannelOwner::permitted`). `""` when
    /// nothing is (a closed channel, members post, an old link, or a request for an action
    /// the owner gives by hand): auto-allow leaves it for the owner.
    pub fn channel_permitted(&self, id: String, request_id: String) -> Result<String, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        ch.owner()?;
        let requests = self.fetch_requests(&mut ch)?;
        let (_, bytes, _) = requests
            .into_iter()
            .find(|(rid, _, _)| *rid == request_id)
            .ok_or_else(|| crate::not_found("request"))?;
        let actions = ch.owner()?.permitted(&bytes).map_err(ch_err)?;
        Ok(actions
            .iter()
            .map(|a| a.name())
            .collect::<Vec<_>>()
            .join(","))
    }

    /// Let a request in with `actions`, "read,post" or "read", which the owner chose on the
    /// screen (or `channel_permitted` gave): a grant from this phone's seat, the current keys
    /// sealed to the member, and a note on the channel. A `post` the policy does not allow is
    /// given as read only; `admit` and `govern` are never given (the policy refuses them).
    pub fn channel_allow(
        &self,
        id: String,
        request_id: String,
        actions: String,
    ) -> Result<(), PhoneError> {
        if actions != "read" && actions != "read,post" {
            return Err(channel("A request is let in to read, or to read and post."));
        }
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        ch.owner()?;
        let requests = self.fetch_requests(&mut ch)?;
        let (_, bytes, _) = requests
            .into_iter()
            .find(|(rid, _, _)| *rid == request_id)
            .ok_or_else(|| crate::not_found("request"))?;
        self.admit_request(&mut ch, &bytes, &actions)?;
        ch.meta.handled.push(request_id);
        ch.meta.save(&ch.dir)
    }

    /// Set a request aside without letting it in.
    pub fn channel_refuse(&self, id: String, request_id: String) -> Result<(), PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        ch.owner()?;
        if !ch.meta.handled.contains(&request_id) {
            ch.meta.handled.push(request_id);
        }
        ch.meta.save(&ch.dir)
    }

    /// The people the owner let in, in the order they were let in.
    pub fn channel_members(&self, id: String) -> Result<Vec<ChannelMemberInfo>, PhoneError> {
        let _guard = self.lock();
        let ch = self.open_channel(&id)?;
        Ok(ch
            .owner()?
            .members()
            .iter()
            .map(|m| {
                let grant_id = hex::encode(m.grant_id);
                ChannelMemberInfo {
                    actions: ch.actions_of(&grant_id),
                    grant_id,
                    name: m.name.clone(),
                }
            })
            .collect())
    }

    /// Remove a member: a revocation from the seat and a note on the channel, then, when the
    /// channel has a link (the member holds it), the link retired and a new one made, and new
    /// keys for everyone else. Returns the new link, which everyone who should keep reading
    /// needs; `""` for a channel without a link.
    pub fn channel_revoke(&self, id: String, grant_id: String) -> Result<String, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        let gid: [u8; 16] = hex::decode(&grant_id)
            .ok()
            .and_then(|g| g.try_into().ok())
            .ok_or_else(|| crate::not_found("member"))?;
        if !ch.owner()?.members().iter().any(|m| m.grant_id == gid) {
            return Err(crate::not_found("member"));
        }
        let holds_link = ch.owner()?.gate_token_hash().is_some();
        let mut seat = self.seat_for(&ch.meta)?;
        let r = ch.owner_mut()?.revoke(&gid, REMOVED, &mut seat);
        let rev = signed(&mut seat, r)?;
        ch.persist()?;
        self.post(
            &ch,
            &format!("/v1/channels/{id}/revocations"),
            rev.statement,
        )?;
        self.post_message(&ch, rev.control)?;
        // Retire, a new link and its gate, then keys whose deposits include the new link's.
        let mut url = String::new();
        if holds_link {
            self.retire(&mut ch)?;
            url = self.issue_link(&mut ch)?;
        }
        self.start_new_keys(&mut ch)?;
        Ok(url)
    }

    /// Retire the link: the relay refuses it and the next keys carry no deposit for it, so
    /// everyone reading by it stops. People the owner let in keep reading. Makes a new link
    /// and new keys, and returns the link.
    pub fn channel_retire_link(&self, id: String) -> Result<String, PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        if !self.retire(&mut ch)? {
            return Err(channel(NO_LINK));
        }
        let url = self.issue_link(&mut ch)?;
        self.start_new_keys(&mut ch)?;
        Ok(url)
    }

    /// Start new keys: sealed to the link and to everyone let in.
    pub fn channel_new_keys(&self, id: String) -> Result<(), PhoneError> {
        let _guard = self.lock();
        let mut ch = self.open_channel(&id)?;
        Ok(self.start_new_keys(&mut ch)?)
    }

    /// Delete the channel's records from this phone, its keys with them. Nothing is sent.
    pub fn channel_delete(&self, id: String) -> Result<(), PhoneError> {
        let _guard = self.lock();
        check_id(&id)?;
        let dir = self.channel_dir(&id);
        if !dir.exists() {
            return Err(crate::not_found("channel"));
        }
        std::fs::remove_dir_all(dir).map_err(store::io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relays_compare_by_host_and_port() {
        assert!(same_relay("https://relay.example", "relay.example"));
        assert!(same_relay("https://Relay.Example/", "relay.EXAMPLE"));
        assert!(same_relay(
            "https://relay.example/some/path",
            "relay.example:443"
        ));
        assert!(same_relay("http://127.0.0.1:8080/", "127.0.0.1:8080"));
        assert!(!same_relay("http://127.0.0.1:8080", "127.0.0.1:8081"));
        assert!(!same_relay("https://relay.example", "other.example"));
        assert!(!same_relay("", ""));
    }

    #[test]
    fn this_machine_is_plain_http() {
        assert_eq!(relay_base("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(relay_base("localhost"), "http://localhost");
        assert_eq!(relay_base("[::1]:9"), "http://[::1]:9");
        assert_eq!(
            relay_base("relay.cahoots.network"),
            "https://relay.cahoots.network"
        );
        assert_eq!(relay_base("127.example.com"), "https://127.example.com");
        assert_eq!(host_of("http://127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(
            host_of("https://relay.cahoots.network"),
            "relay.cahoots.network"
        );
    }
}
