//! Channel bindings (channels spec §4 to §9): [`ChannelOwner`] runs a channel from a circle's
//! seat, [`ChannelMember`] reads and posts in one, [`ChannelReceived`] is one message read.
//!
//! Byte results that hold several parts are canonical CBOR arrays of byte strings, each part
//! exactly the body the app posts; [`cbor_split`] turns one into base64url strings. Errors are
//! short stable strings, as in the rest of this crate.

use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;

use cahoots_channel::control::Control;
use cahoots_channel::deposit::{self, Deposit};
use cahoots_channel::grant::{self, EvalRequest};
use cahoots_channel::keys::{epoch_key, kid_of, EpochId, Key};
use cahoots_channel::link::{Link, LinkSecret};
use cahoots_channel::message::{self, MessageType};
use cahoots_channel::object::channel_id_for;
use cahoots_channel::policy::{self, kind, Rule};
use cahoots_channel::quorum::{approval_for, check_quorum, PROFILE_POST};
use cahoots_channel::session::ChannelSession;
use cahoots_channel::{Action, ChannelError, ChannelId, ChannelObject, Policy, Preset, Refusal};
use cahoots_core::cose;
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_core::CircleId;
use cahoots_hpke::Suite;
use cahoots_objects::assurance;
use cahoots_objects::authz::proof_sign;
use minicbor::{Decoder, Encoder};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use wasm_bindgen::prelude::*;
use zeroize::{Zeroize, Zeroizing};

use crate::{b64_decode, b64_encode, hex, Circle};

/// Domain of a join request's COSE_Sign1.
pub const JOIN_DOMAIN: &[u8] = b"cahoots/join/v1";
/// Domain of the statement that registers a gate token with the relay.
pub const GATE_DOMAIN: &[u8] = b"cahoots/gate/v1";
/// Domain of the statement that removes a gate token from the relay.
pub const GATE_RETIRE_DOMAIN: &[u8] = b"cahoots/gate-retire/v1";
/// Domain of a revocation statement.
pub const REVOKE_DOMAIN: &[u8] = b"cahoots/revoke/v1";

/// The longest channel or member name, in bytes.
const NAME_MAX: usize = 64;
/// Format version of [`ChannelOwner::export`] and [`ChannelMember::export`].
const STATE_VERSION: u8 = 1;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// The browser's generator through `getrandom`, behind the `rand_core` 0.10 traits that
/// `cahoots-hpke` takes.
struct Rng10;

impl rand_core_10::TryRng for Rng10 {
    type Error = Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(OsRng.next_u32())
    }
    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok(OsRng.next_u64())
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        OsRng.fill_bytes(dst);
        Ok(())
    }
}

impl rand_core_10::TryCryptoRng for Rng10 {}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}

fn random_key() -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(out.as_mut());
    out
}

/// A fresh member key: its seed and the key.
fn new_member_key() -> (Zeroizing<[u8; 32]>, SeatKey) {
    loop {
        let seed = random_key();
        if let Ok(key) = SeatKey::from_seed(&seed) {
            return (seed, key);
        }
    }
}

fn s(code: &str) -> String {
    code.to_string()
}

fn err(e: ChannelError) -> String {
    match e {
        ChannelError::Object(o) => format!("object:{o:?}").to_lowercase(),
        ChannelError::Cose(_) | ChannelError::Signature => s("signature"),
        ChannelError::Grant(g) => format!("grant:{g:?}").to_lowercase(),
        ChannelError::Malformed => s("malformed"),
        ChannelError::NotCanonical => s("not-canonical"),
        ChannelError::WrongChannel => s("wrong-channel"),
        ChannelError::WrongCircle => s("wrong-circle"),
        ChannelError::Tag => s("tag"),
        ChannelError::NoGrant => s("no-grant"),
        ChannelError::NoAdmit => s("no-admit"),
        ChannelError::Refused(r) => refusal(r),
        ChannelError::Quorum { .. } => s("quorum"),
        ChannelError::UnknownEpoch => s("unknown-epoch"),
        ChannelError::Reuse => s("reuse"),
        ChannelError::TooFarAhead => s("too-far-ahead"),
        ChannelError::Hpke => s("hpke"),
    }
}

fn refusal(r: Refusal) -> String {
    let name = match r {
        Refusal::Revoked => "revoked",
        Refusal::WrongChannel => "wrong-channel",
        Refusal::Action => "action",
        Refusal::Expired => "expired",
        Refusal::Budget => "budget",
        Refusal::Key => "key",
    };
    format!("refused:{name}")
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let b = text.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    b.chunks(2)
        .map(|p| Some(nib(p[0])? << 4 | nib(p[1])?))
        .collect()
}

fn unhex_n<const N: usize>(text: &str, code: &str) -> Result<[u8; N], String> {
    unhex(text)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| s(code))
}

/// A JSON string literal.
fn json_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The canonical CBOR array of byte strings.
fn bytes_array(items: &[&[u8]]) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = e.array(items.len() as u64);
    for item in items {
        let _ = e.bytes(item);
    }
    e.into_writer()
}

fn read_bytes_array(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let bad = || s("malformed");
    let mut d = Decoder::new(bytes);
    let n = d.array().map_err(|_| bad())?.ok_or_else(bad)?;
    let mut items = Vec::new();
    for _ in 0..n {
        items.push(d.bytes().map_err(|_| bad())?.to_vec());
    }
    let refs: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
    if d.position() != bytes.len() || bytes_array(&refs) != bytes {
        return Err(bad());
    }
    Ok(items)
}

/// Each byte string of a CBOR array, as base64url without padding: for the parts of
/// `admit`, `revoke`, `deposits` and `self_grant`'s chain. Error: `malformed`.
#[wasm_bindgen]
pub fn cbor_split(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(read_bytes_array(bytes)?
        .iter()
        .map(|b| b64_encode(b))
        .collect())
}

/// The owner circle's manifest from its encoded `SealedManifest`, every seat's signature
/// checked. Error: `manifest`.
fn open_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    SealedManifest::decode(bytes)
        .and_then(|m| m.verify())
        .map_err(|_| s("manifest"))
}

/// The policy a signed policy holds, without checking who signed it.
fn read_policy(signed: &[u8]) -> Result<Policy, String> {
    let payload = cose::payload1(signed).map_err(|_| s("malformed"))?;
    Policy::decode(&payload).map_err(err)
}

/// `"read,post"` as actions, each once in number order. Error: `actions`.
fn parse_actions(text: &str) -> Result<Vec<Action>, String> {
    let mut actions = text
        .split(',')
        .map(|a| Action::parse(a.trim()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| s("actions"))?;
    actions.sort();
    actions.dedup();
    if actions.is_empty() {
        return Err(s("actions"));
    }
    Ok(actions)
}

fn actions_text(actions: &[Action]) -> String {
    actions
        .iter()
        .map(|a| a.name())
        .collect::<Vec<_>>()
        .join(",")
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > NAME_MAX {
        return Err(s("name"));
    }
    Ok(())
}

/// The editor's presets by number: 0 announcements, 1 announcements with `quorum` approvals,
/// 2 group chat, 3 members post, 4 closed. Errors: `preset`, `quorum` (0 for preset 1).
fn preset_of(n: u8, quorum: u8) -> Result<Preset, String> {
    Ok(match n {
        0 => Preset::Announcements,
        1 if quorum == 0 => return Err(s("quorum")),
        1 => Preset::AnnouncementsQuorum(quorum),
        2 => Preset::GroupChat,
        3 => Preset::MembersPost,
        4 => Preset::Closed,
        _ => return Err(s("preset")),
    })
}

fn has_link(rule: &Rule) -> bool {
    rule.evidence.iter().any(|e| e.kind == kind::LINK)
}

/// A seat-signed statement `{0: channel_id, 1: second}` or, with a reason, `{..., 2: reason}`.
fn statement(
    key: &SeatKey,
    seat: u8,
    domain: &[u8],
    channel: &ChannelId,
    second: &[u8],
    reason: Option<&str>,
) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = e.map(if reason.is_some() { 3 } else { 2 });
    let _ = e.u8(0).and_then(|e| e.bytes(channel));
    let _ = e.u8(1).and_then(|e| e.bytes(second));
    if let Some(r) = reason {
        let _ = e.u8(2).and_then(|e| e.str(r));
    }
    cose::sign1(key, &[seat], &e.into_writer(), domain, false)
}

/// Reads a state map whose keys are `0..n` in order, key 0 the state version.
struct StateReader<'b> {
    d: Decoder<'b>,
    len: usize,
    next: u64,
}

const STATE: &str = "state";

impl<'b> StateReader<'b> {
    fn open(bytes: &'b [u8], n: u64) -> Result<Self, String> {
        let mut d = Decoder::new(bytes);
        if d.map().ok().flatten() != Some(n) {
            return Err(s(STATE));
        }
        let mut r = StateReader {
            d,
            len: bytes.len(),
            next: 0,
        };
        if r.key()?.u8().ok() != Some(STATE_VERSION) {
            return Err(s(STATE));
        }
        Ok(r)
    }

    fn key(&mut self) -> Result<&mut Decoder<'b>, String> {
        if self.d.u64().ok() != Some(self.next) {
            return Err(s(STATE));
        }
        self.next += 1;
        Ok(&mut self.d)
    }

    fn bytes(&mut self) -> Result<&'b [u8], String> {
        self.key()?.bytes().map_err(|_| s(STATE))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.bytes()?.try_into().map_err(|_| s(STATE))
    }

    fn opt_bytes(&mut self) -> Result<Option<&'b [u8]>, String> {
        let d = self.key()?;
        if d.datatype().map_err(|_| s(STATE))? == minicbor::data::Type::Null {
            d.null().map_err(|_| s(STATE))?;
            return Ok(None);
        }
        d.bytes().map(Some).map_err(|_| s(STATE))
    }

    fn bytes_list(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let d = self.key()?;
        let n = d.array().ok().flatten().ok_or_else(|| s(STATE))?;
        (0..n)
            .map(|_| d.bytes().map(<[u8]>::to_vec).map_err(|_| s(STATE)))
            .collect()
    }

    fn finish(self) -> Result<(), String> {
        if self.d.position() != self.len {
            return Err(s(STATE));
        }
        Ok(())
    }
}

/// A buffer that zeroizes what it held each time it grows, so an export leaves no copy of its
/// secrets behind.
struct SecretBuf(Zeroizing<Vec<u8>>);

impl minicbor::encode::Write for SecretBuf {
    type Error = Infallible;

    fn write_all(&mut self, buf: &[u8]) -> Result<(), Infallible> {
        let need = self.0.len() + buf.len();
        if need > self.0.capacity() {
            let mut grown =
                Zeroizing::new(Vec::with_capacity(need.max(2 * self.0.capacity()).max(256)));
            grown.extend_from_slice(&self.0);
            self.0 = grown;
        }
        self.0.extend_from_slice(buf);
        Ok(())
    }
}

fn opt_bytes<W: minicbor::encode::Write>(
    e: &mut Encoder<W>,
    v: Option<&[u8]>,
) -> Result<(), minicbor::encode::Error<W::Error>> {
    match v {
        Some(b) => e.bytes(b)?,
        None => e.null()?,
    };
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Member
// ---------------------------------------------------------------------------------------------

struct MemberState {
    /// The relay host from the link; empty for an owner's member until `link_url`.
    relay: String,
    channel_id: ChannelId,
    secret: Option<LinkSecret>,
    /// The member key's seed. The session holds the same key.
    seed: Zeroizing<[u8; 32]>,
    xwing_sk: Zeroizing<Vec<u8>>,
    xwing_pk: Vec<u8>,
    session: ChannelSession,
    /// This member's own grant chain, root first, each link's signed bytes.
    grant: Vec<Vec<u8>>,
    /// The channel's current signed policy, as the app last handed it.
    policy: Option<Vec<u8>>,
    /// Grant ids revoked by control messages this member has read.
    revoked: Vec<[u8; 16]>,
    /// An announcement sealed by `propose_announcement` and not yet signed: its body
    /// (header and ciphertext), whose counter is already spent.
    pending: Option<Vec<u8>>,
}

impl Drop for MemberState {
    fn drop(&mut self) {
        if let Some(secret) = self.secret.as_mut() {
            secret.0.zeroize();
        }
    }
}

impl MemberState {
    fn new(channel_id: ChannelId, relay: String, secret: Option<LinkSecret>) -> MemberState {
        let (seed, key) = new_member_key();
        let (xwing_sk, xwing_pk) = cahoots_hpke::gen_keypair(Suite::XWing, &mut Rng10);
        MemberState {
            relay,
            channel_id,
            secret,
            seed,
            xwing_sk,
            xwing_pk,
            session: ChannelSession::new(channel_id, key),
            grant: Vec::new(),
            policy: None,
            revoked: Vec::new(),
            pending: None,
        }
    }

    fn key(&self) -> SeatKey {
        SeatKey::from_seed(&self.seed).expect("the seed was checked when it was made or loaded")
    }

    fn install(&mut self, id: EpochId, key: Key) -> Result<(), String> {
        if self.session.has_epoch(&id) {
            return Ok(());
        }
        self.session.install_epoch(id, key).map_err(err)
    }

    fn send_control(&mut self, control: &Control) -> Result<Vec<u8>, String> {
        self.session
            .send(MessageType::Control, &control.encode(), &[])
            .map_err(err)
    }

    fn export(&self) -> Zeroizing<Vec<u8>> {
        let session = self.session.export_state();
        let mut e = Encoder::new(SecretBuf(Zeroizing::new(Vec::with_capacity(
            session.len() + self.xwing_sk.len() + self.xwing_pk.len() + 4096,
        ))));
        let _ = (|| -> Result<(), minicbor::encode::Error<Infallible>> {
            let e = &mut e;
            e.map(12)?.u8(0)?.u8(STATE_VERSION)?;
            e.u8(1)?.str(&self.relay)?;
            e.u8(2)?.bytes(&self.channel_id)?;
            e.u8(3)?;
            opt_bytes(e, self.secret.as_ref().map(|s| &s.0[..]))?;
            e.u8(4)?.bytes(self.seed.as_ref())?;
            e.u8(5)?.bytes(&self.xwing_sk)?;
            e.u8(6)?.bytes(&self.xwing_pk)?;
            e.u8(7)?.bytes(&session)?;
            e.u8(8)?.array(self.grant.len() as u64)?;
            for g in &self.grant {
                e.bytes(g)?;
            }
            e.u8(9)?;
            opt_bytes(e, self.policy.as_deref())?;
            e.u8(10)?.array(self.revoked.len() as u64)?;
            for r in &self.revoked {
                e.bytes(r)?;
            }
            e.u8(11)?;
            opt_bytes(e, self.pending.as_deref())?;
            Ok(())
        })();
        e.into_writer().0
    }

    fn import(bytes: &[u8]) -> Result<MemberState, String> {
        let mut r = StateReader::open(bytes, 12)?;
        let relay = r.key()?.str().map_err(|_| s(STATE))?.to_string();
        let channel_id: ChannelId = r.array()?;
        let secret = match r.opt_bytes()? {
            Some(b) => Some(LinkSecret(b.try_into().map_err(|_| s(STATE))?)),
            None => None,
        };
        let seed = Zeroizing::new(r.array::<32>()?);
        let key = SeatKey::from_seed(&seed).map_err(|_| s(STATE))?;
        let xwing_sk = Zeroizing::new(r.bytes()?.to_vec());
        let xwing_pk = r.bytes()?.to_vec();
        let session = ChannelSession::import_state(r.bytes()?).map_err(|_| s(STATE))?;
        if session.channel_id() != channel_id || session.my_public() != key.public() {
            return Err(s(STATE));
        }
        let grant = r.bytes_list()?;
        let policy = r.opt_bytes()?.map(<[u8]>::to_vec);
        let revoked = r
            .bytes_list()?
            .into_iter()
            .map(|b| b.try_into().map_err(|_| s(STATE)))
            .collect::<Result<Vec<[u8; 16]>, String>>()?;
        let pending = r.opt_bytes()?.map(<[u8]>::to_vec);
        r.finish()?;
        let state = MemberState {
            relay,
            channel_id,
            secret,
            seed,
            xwing_sk,
            xwing_pk,
            session,
            grant,
            policy,
            revoked,
            pending,
        };
        if *state.export() != *bytes {
            return Err(s(STATE));
        }
        Ok(state)
    }
}

/// A member of a channel: its member key, its X-Wing key for key deposits, the epoch keys it
/// holds, and its grant.
///
/// The owner's own member ([`ChannelOwner::member`]) is a handle onto the session inside the
/// owner: what one sends moves the other's counters, and saving the owner saves it.
#[wasm_bindgen]
pub struct ChannelMember {
    inner: Rc<RefCell<MemberState>>,
}

/// One message read from a channel.
#[wasm_bindgen]
#[derive(Debug)]
pub struct ChannelReceived {
    text: Vec<u8>,
    kid: [u8; 32],
    counter: u32,
    msg_type: u8,
    epoch: [u8; 20],
    control_json: String,
    announcement_ok: bool,
}

#[wasm_bindgen]
impl ChannelReceived {
    /// The plaintext: the sender's padded text for a post or announcement, the control map's
    /// bytes for a control message.
    pub fn text(&self) -> Vec<u8> {
        self.text.clone()
    }
    /// The sender's key id, hex.
    pub fn kid_hex(&self) -> String {
        hex(&self.kid)
    }
    pub fn counter(&self) -> u32 {
        self.counter
    }
    /// 0 text, 1 control, 2 announcement.
    pub fn msg_type(&self) -> u8 {
        self.msg_type
    }
    /// The epoch id the message was sent in, hex of 20 bytes.
    pub fn epoch_hex(&self) -> String {
        hex(&self.epoch)
    }
    /// What a control message says, as JSON; `""` for any other type.
    ///
    /// - `{"kind":"rotation","epoch":"<hex 20 bytes>","deposits":["<hex 32>",...]}`
    /// - `{"kind":"policy","policy_id":"<hex 32>"}`
    /// - `{"kind":"revocation","grant_id":"<hex 16>","reason":"..."}`
    /// - `{"kind":"join","grant_id":"<hex 16>","xwing_pk":"<base64url>","name":"..."}`
    /// - `{"kind":"leave"}`
    pub fn control_json(&self) -> String {
        self.control_json.clone()
    }
    /// For an announcement: its approvals reach the post rule's quorum under the policy this
    /// member holds. False without a policy, and for every other type.
    pub fn announcement_ok(&self) -> bool {
        self.announcement_ok
    }
}

fn control_json(c: &Control) -> String {
    match c {
        Control::Rotation { epoch_id, deposits } => format!(
            "{{\"kind\":\"rotation\",\"epoch\":\"{}\",\"deposits\":[{}]}}",
            hex(&epoch_id.encode()),
            deposits
                .iter()
                .map(|d| format!("\"{}\"", hex(d)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Control::PolicyChange { policy_id } => format!(
            "{{\"kind\":\"policy\",\"policy_id\":\"{}\"}}",
            hex(policy_id)
        ),
        Control::Revocation { grant_id, reason } => format!(
            "{{\"kind\":\"revocation\",\"grant_id\":\"{}\",\"reason\":{}}}",
            hex(grant_id),
            json_str(reason)
        ),
        Control::Join {
            grant_id,
            xwing_pk,
            name,
        } => format!(
            "{{\"kind\":\"join\",\"grant_id\":\"{}\",\"xwing_pk\":\"{}\",\"name\":{}}}",
            hex(grant_id),
            b64_encode(xwing_pk),
            json_str(name)
        ),
        Control::Leave => s("{\"kind\":\"leave\"}"),
    }
}

impl std::fmt::Debug for ChannelMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.inner.borrow();
        f.debug_struct("ChannelMember")
            .field("channel_id", &hex(&st.channel_id))
            .field("session", &st.session)
            .finish_non_exhaustive()
    }
}

impl ChannelMember {
    fn from_state(state: MemberState) -> ChannelMember {
        ChannelMember {
            inner: Rc::new(RefCell::new(state)),
        }
    }
}

#[wasm_bindgen]
impl ChannelMember {
    /// A new member from an invite link (the full URL or its `#/ch/...` fragment), with a fresh
    /// member key and X-Wing key. Error: `link`.
    pub fn from_link(url: &str) -> Result<ChannelMember, String> {
        let link = Link::parse(url).ok_or_else(|| s("link"))?;
        Ok(ChannelMember::from_state(MemberState::new(
            link.channel_id,
            link.relay.clone(),
            link.secret.clone(),
        )))
    }

    pub fn channel_id_hex(&self) -> String {
        hex(&self.inner.borrow().channel_id)
    }

    /// The relay host from the link (`host` or `host:port`).
    pub fn relay_host(&self) -> String {
        self.inner.borrow().relay.clone()
    }

    /// The gate token for the `Cahoots-Gate` header, hex; `""` when the link had no secret.
    pub fn gate_token_hex(&self) -> String {
        let st = self.inner.borrow();
        st.secret
            .as_ref()
            .map(|sec| hex(&sec.gate_token(&st.channel_id)))
            .unwrap_or_default()
    }

    /// The member key's id (`SHA-256` of the compressed key), hex: the `Cahoots-Kid` header and
    /// the `kid` of this member's messages.
    pub fn my_kid_hex(&self) -> String {
        hex(&self.inner.borrow().session.my_kid())
    }

    /// The member key, SEC1 compressed, hex.
    pub fn member_key_hex(&self) -> String {
        hex(&self.inner.borrow().session.my_public().0)
    }

    /// The epoch this member sends in, hex of the 20-byte epoch id; `None` before a deposit.
    pub fn current_epoch_hex(&self) -> Option<String> {
        let st = self.inner.borrow();
        st.session.current_epoch().map(|id| hex(&id.encode()))
    }

    /// Open a link deposit with the link's secret and install its epoch. An epoch already held
    /// is left as it is. Errors: `no-link`, `wrong-channel`, `tag`, `malformed`, `reuse`.
    pub fn install_from_link_deposit(&mut self, deposit: &[u8]) -> Result<(), String> {
        let mut st = self.inner.borrow_mut();
        let d = Deposit::decode(deposit).map_err(err)?;
        if d.channel_id != st.channel_id {
            return Err(s("wrong-channel"));
        }
        let secret = st.secret.as_ref().ok_or_else(|| s("no-link"))?;
        let key = deposit::open_with_link(&d, secret).map_err(err)?;
        st.install(d.epoch_id, key)
    }

    /// Open a key deposit with this member's X-Wing secret key and install its epoch. An epoch
    /// already held is left as it is. Errors: `not-mine` (sealed to another key id),
    /// `wrong-channel`, `hpke`, `malformed`, `reuse`.
    pub fn install_from_key_deposit(&mut self, deposit: &[u8]) -> Result<(), String> {
        let mut st = self.inner.borrow_mut();
        let d = Deposit::decode(deposit).map_err(err)?;
        if d.channel_id != st.channel_id {
            return Err(s("wrong-channel"));
        }
        if d.kind == deposit::KIND_KEY && d.to != st.session.my_kid() {
            return Err(s("not-mine"));
        }
        let key = deposit::open_with_key(&d, &st.xwing_sk).map_err(err)?;
        st.install(d.epoch_id, key)
    }

    /// A join request for the owner: the COSE_Sign1, kid this member's key id, external AAD
    /// `cahoots/join/v1`, over the canonical map
    /// `{0: channel_id, 1: member key bstr 33, 2: xwing_pk, 3: actions "read,post", 4: name,
    ///   5: gate token bstr 32, or empty without a link secret}`.
    /// Errors: `name` (empty or over 64 bytes), `actions`.
    pub fn join_request(&self, name: &str, actions: &str) -> Result<Vec<u8>, String> {
        check_name(name)?;
        let actions = actions_text(&parse_actions(actions)?);
        let st = self.inner.borrow();
        let token = st
            .secret
            .as_ref()
            .map(|sec| sec.gate_token(&st.channel_id).to_vec())
            .unwrap_or_default();
        let key = st.key();
        let mut e = Encoder::new(Vec::new());
        let _ = (|| -> Result<(), minicbor::encode::Error<Infallible>> {
            e.map(6)?;
            e.u8(0)?.bytes(&st.channel_id)?;
            e.u8(1)?.bytes(&key.public().0)?;
            e.u8(2)?.bytes(&st.xwing_pk)?;
            e.u8(3)?.str(&actions)?;
            e.u8(4)?.str(name)?;
            e.u8(5)?.bytes(&token)?;
            Ok(())
        })();
        let kid = st.session.my_kid();
        Ok(cose::sign1(
            &key,
            &kid,
            &e.into_writer(),
            JOIN_DOMAIN,
            false,
        ))
    }

    /// Keep this member's grant chain (a CBOR array of signed grants, root first, as `admit`
    /// returned it). `receive` uses it for this member's own posts. Error: `malformed`.
    pub fn set_grant(&mut self, chain: &[u8]) -> Result<(), String> {
        self.inner.borrow_mut().grant = read_bytes_array(chain)?;
        Ok(())
    }

    /// Keep the channel's current signed policy, for counting announcement approvals.
    /// Errors: `malformed`, `wrong-channel`.
    pub fn set_policy(&mut self, signed: &[u8]) -> Result<(), String> {
        let p = read_policy(signed)?;
        let mut st = self.inner.borrow_mut();
        if p.channel_id != st.channel_id {
            return Err(s("wrong-channel"));
        }
        st.policy = Some(signed.to_vec());
        Ok(())
    }

    /// The `Cahoots-Proof` header for a gated read: `proof_sign(member key, SHA-256(target),
    /// nonce)` as base64url without padding. `target` is `METHOD " " PATH "?" QUERY` exactly as
    /// in the request line, the `?` always written (`GET /v1/channels/<id>/head?`); `nonce_hex`
    /// is the relay's `GET /v1/nonce` answer. Error: `nonce`.
    pub fn proof_for(&self, target: &str, nonce_hex: &str) -> Result<String, String> {
        let nonce: [u8; 16] = unhex_n(nonce_hex, "nonce")?;
        let hash: [u8; 32] = Sha256::digest(target.as_bytes()).into();
        let key = self.inner.borrow().key();
        Ok(b64_encode(&proof_sign(&key, &hash, &nonce)))
    }

    /// Encrypt and sign a text post in the current epoch; base64url wire bytes. The text is
    /// sent as given: pad it with `chat_bucket_pad` first. Error: `unknown-epoch` before any
    /// deposit is installed.
    pub fn send(&mut self, text: &[u8]) -> Result<String, String> {
        let wire = self
            .inner
            .borrow_mut()
            .session
            .send(MessageType::Text, text, &[])
            .map_err(err)?;
        Ok(b64_encode(&wire))
    }

    /// Seal an announcement in the current epoch and keep it unsigned, for seats to approve.
    /// Returns the body (header and ciphertext). The counter is spent now, so the body is the
    /// one the relay will see: get its digest with `announcement_digest`, collect seat
    /// approvals of it (`ChannelOwner::approve_post`), then `send_proposed`. A new proposal
    /// replaces one not yet sent. The text is sent as given: pad it first.
    /// Error: `unknown-epoch`.
    pub fn propose_announcement(&mut self, text: &[u8]) -> Result<Vec<u8>, String> {
        let mut st = self.inner.borrow_mut();
        // The session seals and signs in one step; the signature over no approvals is
        // dropped, and `send_proposed` signs the same body again with the approvals.
        let wire = st
            .session
            .send(MessageType::Announcement, text, &[])
            .map_err(err)?;
        let (body, _, _) = message::split(&wire).map_err(err)?;
        let body = body.to_vec();
        st.pending = Some(body.clone());
        Ok(body)
    }

    /// `SHA-256` of the pending announcement's body, hex: the `request_digest` its approvals
    /// sign. Error: `no-proposal`.
    pub fn announcement_digest(&self) -> Result<String, String> {
        let st = self.inner.borrow();
        let body = st.pending.as_ref().ok_or_else(|| s("no-proposal"))?;
        Ok(hex(&message::body_digest(body)))
    }

    /// Sign the pending announcement with `approvals` (a CBOR array of signed approvals) and
    /// return its base64url wire; the proposal is then gone. Errors: `no-proposal`,
    /// `malformed`.
    pub fn send_proposed(&mut self, approvals: &[u8]) -> Result<String, String> {
        let approvals = read_bytes_array(approvals)?;
        let refs: Vec<&[u8]> = approvals.iter().map(Vec::as_slice).collect();
        let mut st = self.inner.borrow_mut();
        let body = st.pending.take().ok_or_else(|| s("no-proposal"))?;
        let mut aad = Vec::with_capacity(message::DOMAIN.len() + body.len());
        aad.extend_from_slice(message::DOMAIN);
        aad.extend_from_slice(&body);
        let kid = st.session.my_kid();
        let signature = cose::sign1(&st.key(), &kid, &bytes_array(&refs), &aad, false);
        let mut wire = body;
        wire.extend_from_slice(&signature);
        Ok(b64_encode(&wire))
    }

    /// Verify and open one message.
    ///
    /// `signer_chain` is the sender's grant chain (a CBOR array of signed grants, root first,
    /// as the relay lists them by kid); empty bytes mean this member's own chain from
    /// `set_grant`. `manifest` is the owner circle's encoded `SealedManifest`. The chain is
    /// verified against it, the message's kid must be the chain's grantee, and the grant must
    /// carry `post` for a text or announcement and `govern` for a control message and not be
    /// revoked by a control message this member has read.
    ///
    /// Readers do not check expiry or post budgets: the evaluator runs with `log = 0` and
    /// `posts_used = 0`, and the relay enforces both.
    ///
    /// A revocation control message is remembered: grants it names, and their children, are
    /// refused from then on. A control message is decoded only after it is opened, and the
    /// session commits its receiving chain on opening, so a control whose plaintext does not
    /// decode (`malformed`) still uses up its counter: only a granted sender can make one. Errors: `base64url`, `manifest`, `no-grant`, `malformed`,
    /// `signature`, `grant:<reason>`, `no-admit`, `wrong-channel`, `refused:<reason>`
    /// (`revoked`, `action`, ...), `unknown-epoch`, `reuse`, `too-far-ahead`, `tag`.
    pub fn receive(
        &mut self,
        wire_b64: &str,
        signer_chain: &[u8],
        manifest: &[u8],
    ) -> Result<ChannelReceived, String> {
        let wire = b64_decode(wire_b64)?;
        let manifest = open_manifest(manifest)?;
        let mut st = self.inner.borrow_mut();
        let chain = if signer_chain.is_empty() {
            st.grant.clone()
        } else {
            read_bytes_array(signer_chain)?
        };
        if chain.is_empty() {
            return Err(s("no-grant"));
        }
        let refs: Vec<&[u8]> = chain.iter().map(Vec::as_slice).collect();
        let channel_id = st.channel_id;
        let leaf = grant::verify_chain(&refs, &manifest, &channel_id).map_err(err)?;
        let ids = grant::chain_ids(&refs).map_err(err)?;
        let (body, _, header) = message::split(&wire).map_err(err)?;
        if header.kid != kid_of(&leaf.grantee) {
            return Err(s("signature"));
        }
        let digest = message::body_digest(body);
        let action = match header.msg_type {
            MessageType::Control => Action::Govern,
            MessageType::Text | MessageType::Announcement => Action::Post,
        };
        let revoked = st.revoked.clone();
        let is_revoked = |id: &[u8; 16]| revoked.contains(id);
        grant::evaluate(
            &leaf,
            &ids,
            &EvalRequest {
                action,
                channel_id: &channel_id,
                log: 0,
                posts_used: 0,
                revoked: &is_revoked,
                signer: &leaf.grantee,
            },
        )
        .map_err(refusal)?;
        let (header, text, approvals) = st.session.receive(&wire, &leaf.grantee).map_err(err)?;
        let announcement_ok = header.msg_type == MessageType::Announcement
            && st.policy.as_ref().is_some_and(|signed| {
                let Ok((_, p)) = Policy::verify(signed, &manifest) else {
                    return false;
                };
                let (quorum, assurance) = p
                    .rule(Action::Post)
                    .map_or((0, 0), |r| (r.quorum, r.assurance));
                check_quorum(
                    &approvals,
                    &manifest,
                    &digest,
                    PROFILE_POST,
                    quorum,
                    assurance,
                )
                .is_ok()
            });
        let control_json = if header.msg_type == MessageType::Control {
            let control = Control::decode(&text).map_err(err)?;
            if let Control::Revocation { grant_id, .. } = &control {
                if !st.revoked.contains(grant_id) {
                    st.revoked.push(*grant_id);
                }
            }
            control_json(&control)
        } else {
            String::new()
        };
        Ok(ChannelReceived {
            text,
            kid: header.kid,
            counter: header.counter,
            msg_type: header.msg_type as u8,
            epoch: header.epoch_id.encode(),
            control_json,
            announcement_ok,
        })
    }

    /// The whole member as canonical CBOR, secrets included (the member key's seed, the X-Wing
    /// secret key, the link secret, every epoch key): store it under the app's own protection.
    ///
    /// `{0: 1, 1: relay tstr, 2: channel_id, 3: link secret or nil, 4: member key seed,
    ///   5: xwing_sk, 6: xwing_pk, 7: session state, 8: [grant...], 9: policy or nil,
    ///   10: [revoked grant id...], 11: pending announcement body or nil}`
    pub fn export(&self) -> Vec<u8> {
        self.inner.borrow().export().to_vec()
    }

    /// Read back what `export` wrote. Error: `state`.
    pub fn import(state: &[u8]) -> Result<ChannelMember, String> {
        MemberState::import(state).map(ChannelMember::from_state)
    }
}

// ---------------------------------------------------------------------------------------------
// Owner
// ---------------------------------------------------------------------------------------------

/// A member the owner admitted and seals each epoch to.
struct Admitted {
    grant_id: [u8; 16],
    kid: [u8; 32],
    xwing_pk: Vec<u8>,
    name: String,
}

/// A channel run from one seat of its owner circle: the signed channel object and policy, the
/// link secret, the epochs it issues, the members it admitted, and its own member.
#[wasm_bindgen]
pub struct ChannelOwner {
    circle_id: CircleId,
    channel_id: ChannelId,
    seat: u8,
    seat_seed: Zeroizing<[u8; 32]>,
    /// The owner circle's encoded `SealedManifest`.
    manifest: Vec<u8>,
    /// The seat count the presets size the `govern` quorum by.
    seats: u8,
    object: Vec<u8>,
    policy: Vec<u8>,
    secret: Option<LinkSecret>,
    /// The last epoch number issued; 0 before the first rotation. Only this device issues.
    epoch: u32,
    epoch_key: Option<Key>,
    /// The deposits made for the current epoch, encoded.
    deposits: Vec<Vec<u8>>,
    members: Vec<Admitted>,
    self_grant: Vec<u8>,
    member: ChannelMember,
}

impl std::fmt::Debug for ChannelOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelOwner")
            .field("channel_id", &hex(&self.channel_id))
            .field("seat", &self.seat)
            .field("epoch", &self.epoch)
            .field("members", &self.members.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ChannelOwner {
    fn drop(&mut self) {
        if let Some(secret) = self.secret.as_mut() {
            secret.0.zeroize();
        }
    }
}

#[wasm_bindgen]
impl Circle {
    /// Start a channel owned by this circle, signed by this device's seat.
    ///
    /// `preset`: 0 announcements, 1 announcements with approvals (a majority of `seats_hint`),
    /// 2 group chat, 3 members post, 4 closed. `seats_hint` is the number of seats the
    /// `govern` quorum is sized by (1 for a quorum of 1); 0 means every seat of the circle.
    /// Every preset but closed makes the link secret at once, since the policy names it.
    ///
    /// The owner's own member key gets a root grant for all four actions (`self_grant`).
    /// No epoch exists until `rotate`. Errors: `no-seat-key`, `manifest`, `seat`, `name`,
    /// `preset`.
    pub fn channel_owner(
        &self,
        name: &str,
        preset: u8,
        seats_hint: u8,
    ) -> Result<ChannelOwner, String> {
        let seed = self.seat_key.as_ref().ok_or_else(|| s("no-seat-key"))?;
        let manifest = open_manifest(&self.manifest)?;
        let seat = self.my_slice();
        let key = SeatKey::from_seed(seed).map_err(|_| s("seat"))?;
        if manifest.seat_key(seat) != Some(&key.public()) {
            return Err(s("seat"));
        }
        check_name(name)?;
        let seats = if seats_hint == 0 {
            manifest.params.n
        } else {
            seats_hint
        };
        let majority = if seats <= 1 { 1 } else { seats / 2 + 1 };
        let preset = preset_of(preset, majority)?;
        let nonce = random::<16>();
        let circle_id = manifest.circle_id;
        let channel_id = channel_id_for(&circle_id, &nonce);
        let secret = (preset != Preset::Closed).then(|| LinkSecret(*random_key()));
        let link_id = secret.as_ref().map(LinkSecret::id).unwrap_or_default();
        let policy = Policy::preset(
            preset,
            circle_id,
            channel_id,
            link_id,
            seats,
            0,
            random::<16>(),
        )
        .sign(&key, seat);
        let object = ChannelObject {
            circle_id,
            channel_id,
            name: name.to_string(),
            created_log: 0,
            policy_id: policy::id(&policy),
            nonce,
        }
        .sign(&key, seat);
        let mut member = MemberState::new(channel_id, String::new(), secret.clone());
        let claims = grant::issue_root(
            &circle_id,
            &channel_id,
            seat,
            &member.session.my_public(),
            &[Action::Read, Action::Post, Action::Admit, Action::Govern],
            0,
            u64::MAX,
            2,
            random::<16>(),
        );
        let self_grant = grant::sign_root(&claims, &key, seat);
        member.grant = vec![self_grant.clone()];
        member.policy = Some(policy.clone());
        Ok(ChannelOwner {
            circle_id,
            channel_id,
            seat,
            seat_seed: Zeroizing::new(**seed),
            manifest: self.manifest.clone(),
            seats,
            object,
            policy,
            secret,
            epoch: 0,
            epoch_key: None,
            deposits: Vec::new(),
            members: Vec::new(),
            self_grant,
            member: ChannelMember::from_state(member),
        })
    }
}

impl ChannelOwner {
    fn seat_key(&self) -> SeatKey {
        SeatKey::from_seed(&self.seat_seed).expect("the seed was checked when it was loaded")
    }

    fn current(&self) -> Result<(EpochId, &Key), String> {
        let key = self.epoch_key.as_ref().ok_or_else(|| s("unknown-epoch"))?;
        Ok((
            EpochId {
                channel_id: self.channel_id,
                epoch: self.epoch,
            },
            key,
        ))
    }

    fn link_secret(&self) -> Result<&LinkSecret, String> {
        self.secret.as_ref().ok_or_else(|| s("no-link"))
    }

    /// Make the link secret if there is none, and give the owner's member the same.
    fn ensure_link(&mut self) -> &LinkSecret {
        if self.secret.is_none() {
            let secret = LinkSecret(*random_key());
            self.member.inner.borrow_mut().secret = Some(secret.clone());
            self.secret = Some(secret);
        }
        self.secret.as_ref().expect("made above")
    }

    fn send_control(&mut self, control: &Control) -> Result<Vec<u8>, String> {
        self.member.inner.borrow_mut().send_control(control)
    }
}

#[wasm_bindgen]
impl ChannelOwner {
    pub fn channel_id_hex(&self) -> String {
        hex(&self.channel_id)
    }

    /// The signed channel object, the body of `POST /v1/channels`.
    pub fn channel_object(&self) -> Vec<u8> {
        self.object.clone()
    }

    /// The current signed policy.
    pub fn policy(&self) -> Vec<u8> {
        self.policy.clone()
    }

    /// `[policy, []]`: the body of `POST …/policy` for the first policy, which needs no
    /// approvals.
    pub fn policy_body(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        let _ = e.array(2).and_then(|e| e.bytes(&self.policy));
        let _ = e.array(0);
        e.into_writer()
    }

    /// SHA-256 of the current signed policy, hex.
    pub fn policy_id_hex(&self) -> String {
        hex(&policy::id(&self.policy))
    }

    /// The owner circle's encoded `SealedManifest`: the body of `PUT` manifest, and what
    /// members pass to `receive` and `verify_channel_object`.
    pub fn manifest(&self) -> Vec<u8> {
        self.manifest.clone()
    }

    /// The invite link on `relay_host` (`host` or `host:port`). Makes the link secret on the
    /// first call when there is none (a closed channel, or after `retire_link`), and keeps it;
    /// a policy names a new secret only after the next `set_policy`. Error: `relay`.
    pub fn link_url(&mut self, relay_host: &str) -> Result<String, String> {
        let secret = self.ensure_link().clone();
        let url = Link {
            relay: relay_host.to_string(),
            channel_id: self.channel_id,
            secret: Some(secret),
        }
        .to_url();
        if Link::parse(&url).is_none() {
            return Err(s("relay"));
        }
        self.member.inner.borrow_mut().relay = relay_host.to_string();
        Ok(url)
    }

    /// The seat-signed `cahoots/gate/v1` statement `{0: channel_id, 1: gate_token}` for the
    /// current link: the body of `POST …/gates`. Error: `no-link`.
    pub fn gate_registration(&self) -> Result<Vec<u8>, String> {
        let token = self.link_secret()?.gate_token(&self.channel_id);
        Ok(statement(
            &self.seat_key(),
            self.seat,
            GATE_DOMAIN,
            &self.channel_id,
            &token,
            None,
        ))
    }

    /// SHA-256 of the current link's gate token, hex: the path of `DELETE …/gates/{hash}`.
    /// Read it before `retire_link`. `None` without a link.
    pub fn gate_token_hash_hex(&self) -> Option<String> {
        let token = self.secret.as_ref()?.gate_token(&self.channel_id);
        Some(hex(&Sha256::digest(token)))
    }

    /// The seat-signed `cahoots/gate-retire/v1` statement for the current link (the body of
    /// `DELETE …/gates/{hash}`), and forget its secret: later rotations make no link deposit.
    /// Error: `no-link`.
    pub fn retire_link(&mut self) -> Result<Vec<u8>, String> {
        let token = self.link_secret()?.gate_token(&self.channel_id);
        let signed = statement(
            &self.seat_key(),
            self.seat,
            GATE_RETIRE_DOMAIN,
            &self.channel_id,
            &token,
            None,
        );
        if let Some(mut secret) = self.secret.take() {
            secret.0.zeroize();
        }
        if let Some(mut secret) = self.member.inner.borrow_mut().secret.take() {
            secret.0.zeroize();
        }
        Ok(signed)
    }

    /// The owner member's grant chain: a CBOR array of one signed root grant for read, post,
    /// admit and govern, depth 2, no budget, `not_after = u64::MAX`. The body of
    /// `POST …/grants`.
    pub fn self_grant(&self) -> Vec<u8> {
        bytes_array(&[&self.self_grant])
    }

    /// Start the next epoch: draw a root, derive its key, seal it to the live link and to every
    /// admitted member, install it in the owner's member, and return the rotation control
    /// message (wire bytes). Post `deposits()` first, then this.
    ///
    /// The control is sealed under the previous epoch, so every member who held it, a member
    /// being shut out included, reads which deposits to fetch; the control carries only the
    /// epoch id and deposit ids. The new epoch is installed after it is sent. The first
    /// rotation has no previous epoch and seals the control under the new one.
    /// Errors: `hpke` (a member's X-Wing key is bad), `reuse`.
    pub fn rotate(&mut self) -> Result<Vec<u8>, String> {
        let next = self.epoch.checked_add(1).ok_or_else(|| s("reuse"))?;
        let id = EpochId {
            channel_id: self.channel_id,
            epoch: next,
        };
        let key = epoch_key(&id, &random_key());
        let mut deposits = Vec::new();
        if let Some(secret) = &self.secret {
            deposits.push(deposit::seal_for_link(&key, secret, &id));
        }
        for m in &self.members {
            deposits.push(
                deposit::seal_for_key(&key, &id, m.kid, &m.xwing_pk, &mut Rng10).map_err(err)?,
            );
        }
        let control = Control::Rotation {
            epoch_id: id,
            deposits: deposits.iter().map(Deposit::id).collect(),
        };
        let wire = {
            let mut member = self.member.inner.borrow_mut();
            if self.epoch_key.is_some() {
                let wire = member.send_control(&control)?;
                member.install(id, Zeroizing::new(*key))?;
                wire
            } else {
                member.install(id, Zeroizing::new(*key))?;
                member.send_control(&control)?
            }
        };
        self.epoch = next;
        self.epoch_key = Some(key);
        self.deposits = deposits.iter().map(Deposit::encode).collect();
        Ok(wire)
    }

    /// The deposits made for the current epoch, a CBOR array of encoded deposits, each the
    /// body of one `POST …/deposits`: one for the live link, then one per admitted member.
    pub fn deposits(&self) -> Vec<u8> {
        let refs: Vec<&[u8]> = self.deposits.iter().map(Vec::as_slice).collect();
        bytes_array(&refs)
    }

    /// A key deposit of the current epoch for one more key (`kid_hex` its signing key's id),
    /// outside the admitted list: it is not repeated at the next rotation.
    /// Errors: `kid`, `unknown-epoch`, `hpke`.
    pub fn deposit_for_key(&self, kid_hex: &str, xwing_pk: &[u8]) -> Result<Vec<u8>, String> {
        let kid: [u8; 32] = unhex_n(kid_hex, "kid")?;
        let (id, key) = self.current()?;
        let d = deposit::seal_for_key(key, &id, kid, xwing_pk, &mut Rng10).map_err(err)?;
        Ok(d.encode())
    }

    /// Admit a join request (as `ChannelMember::join_request` makes it), granting `actions`
    /// ("read,post"), which must be among those requested. For each granted action whose rule
    /// names a link, the request's gate token must be the live link's.
    ///
    /// Issues a root grant (depth 0, no budget, `not_after = u64::MAX`), seals the current
    /// epoch to the member's X-Wing key, keeps the member for later rotations, and returns the
    /// CBOR array `[grant chain, key deposit, join control wire]`: post the chain to
    /// `…/grants`, the deposit to `…/deposits`, the control to `…/messages`.
    /// A request for `post` is refused with `policy` when the post rule has no evidence (only
    /// the owner's seats post, as in an announcements channel).
    /// Errors: `malformed`, `signature`, `wrong-channel`, `actions`, `name`, `policy`,
    /// `evidence`, `unknown-epoch`, `hpke`.
    pub fn admit(&mut self, request: &[u8], actions: &str) -> Result<Vec<u8>, String> {
        let bad = || s("malformed");
        let payload = cose::payload1(request).map_err(|_| bad())?;
        let mut d = Decoder::new(&payload);
        let key = |d: &mut Decoder<'_>, k: u8| -> Result<(), String> {
            (d.u8().ok() == Some(k)).then_some(()).ok_or_else(bad)
        };
        if d.map().ok().flatten() != Some(6) {
            return Err(bad());
        }
        key(&mut d, 0)?;
        let channel = d.bytes().map_err(|_| bad())?;
        key(&mut d, 1)?;
        let member_key = SeatPublic::parse(d.bytes().map_err(|_| bad())?).map_err(|_| bad())?;
        key(&mut d, 2)?;
        let xwing_pk = d.bytes().map_err(|_| bad())?.to_vec();
        key(&mut d, 3)?;
        let wanted = d.str().map_err(|_| bad())?.to_string();
        key(&mut d, 4)?;
        let name = d.str().map_err(|_| bad())?.to_string();
        key(&mut d, 5)?;
        let token = d.bytes().map_err(|_| bad())?.to_vec();
        if d.position() != payload.len() {
            return Err(bad());
        }
        let kid = kid_of(&member_key);
        if cose::kid1(request).map_err(|_| bad())? != kid {
            return Err(s("signature"));
        }
        cose::verify1(&member_key, request, None, JOIN_DOMAIN).map_err(|_| s("signature"))?;
        if channel != self.channel_id {
            return Err(s("wrong-channel"));
        }
        check_name(&name)?;
        let wanted = parse_actions(&wanted)?;
        let granted = parse_actions(actions)?;
        if !granted.iter().all(|a| wanted.contains(a)) {
            return Err(s("actions"));
        }
        let policy = read_policy(&self.policy)?;
        // A post rule with no evidence is only the owner's seats: nobody is admitted to post.
        if granted.contains(&Action::Post)
            && policy
                .rule(Action::Post)
                .is_none_or(|r| r.evidence.is_empty())
        {
            return Err(s("policy"));
        }
        for action in &granted {
            let Some(rule) = policy.rule(*action) else {
                continue;
            };
            if !has_link(rule) {
                continue;
            }
            let shown = self.secret.as_ref().is_some_and(|sec| {
                rule.evidence
                    .iter()
                    .any(|e| e.kind == kind::LINK && e.r#ref[..] == sec.id())
                    && token[..] == sec.gate_token(&self.channel_id)
            });
            if !shown {
                return Err(s("evidence"));
            }
        }
        let (id, epoch_key) = self.current()?;
        let deposit =
            deposit::seal_for_key(epoch_key, &id, kid, &xwing_pk, &mut Rng10).map_err(err)?;
        let grant_id = random::<16>();
        let claims = grant::issue_root(
            &self.circle_id,
            &self.channel_id,
            self.seat,
            &member_key,
            &granted,
            0,
            u64::MAX,
            0,
            grant_id,
        );
        let signed = grant::sign_root(&claims, &self.seat_key(), self.seat);
        let wire = self.send_control(&Control::Join {
            grant_id,
            xwing_pk: xwing_pk.clone(),
            name: name.clone(),
        })?;
        self.members.retain(|m| m.kid != kid);
        self.members.push(Admitted {
            grant_id,
            kid,
            xwing_pk,
            name,
        });
        let chain = bytes_array(&[&signed]);
        Ok(bytes_array(&[&chain, &deposit.encode(), &wire]))
    }

    /// The admitted members, as JSON: `[{"grant_id":"<hex 16>","kid":"<hex 32>","name":"..."}]`.
    pub fn members_json(&self) -> String {
        let items: Vec<String> = self
            .members
            .iter()
            .map(|m| {
                format!(
                    "{{\"grant_id\":\"{}\",\"kid\":\"{}\",\"name\":{}}}",
                    hex(&m.grant_id),
                    hex(&m.kid),
                    json_str(&m.name)
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }

    /// Revoke a grant: returns the CBOR array `[revocation, control wire]`. The revocation is
    /// the body of `POST …/revocations`, itself `[grant_id, COSE_Sign1 by the seat under
    /// cahoots/revoke/v1 over {0: channel_id, 1: grant_id, 2: reason}]`; the control is a
    /// revocation message for the channel. The member is no longer sealed to; `rotate` (and
    /// `retire_link` if the member holds the link) then shuts it out.
    /// Errors: `grant-id`, `unknown-epoch`.
    pub fn revoke(&mut self, grant_id_hex: &str, reason: &str) -> Result<Vec<u8>, String> {
        let grant_id: [u8; 16] = unhex_n(grant_id_hex, "grant-id")?;
        let signed = statement(
            &self.seat_key(),
            self.seat,
            REVOKE_DOMAIN,
            &self.channel_id,
            &grant_id,
            Some(reason),
        );
        let body = bytes_array(&[&grant_id, &signed]);
        let wire = self.send_control(&Control::Revocation {
            grant_id,
            reason: reason.to_string(),
        })?;
        self.members.retain(|m| m.grant_id != grant_id);
        let mut st = self.member.inner.borrow_mut();
        if !st.revoked.contains(&grant_id) {
            st.revoked.push(grant_id);
        }
        Ok(bytes_array(&[&body, &wire]))
    }

    /// Replace the policy with a preset (numbers as for `Circle::channel_owner`; `quorum` is the
    /// approvals each announcement needs, read for preset 1 only). Makes a link secret if the
    /// preset admits by link and there is none. Returns `[signed policy, [approvals]]`, the
    /// body of `POST …/policy`.
    ///
    /// It also reissues the channel object: a new signed object with `policy_id` the new
    /// policy's id and `created_log` the new policy's `log` (same name and nonce, so the same
    /// channel id), which `channel_object()` then returns. Post the policy, then the channel
    /// object, then `policy_change_control()`.
    ///
    /// In this version the approvals are always empty: when the current policy's `govern`
    /// rule needs more than one seat, this refuses with `needs-approvals`.
    /// Errors: `preset`, `quorum`, `needs-approvals`, `malformed`.
    pub fn set_policy(&mut self, preset: u8, quorum: u8) -> Result<Vec<u8>, String> {
        let preset = preset_of(preset, quorum)?;
        let current = read_policy(&self.policy)?;
        if current.rule(Action::Govern).is_some_and(|r| r.quorum > 1) {
            return Err(s("needs-approvals"));
        }
        let link_id = if preset == Preset::Closed {
            self.secret.as_ref().map(LinkSecret::id).unwrap_or_default()
        } else {
            self.ensure_link().id()
        };
        let mut next = Policy::preset(
            preset,
            self.circle_id,
            self.channel_id,
            link_id,
            self.seats,
            current.log + 1,
            random::<16>(),
        );
        next.parent = Some(policy::id(&self.policy));
        let signed = next.sign(&self.seat_key(), self.seat);
        let payload = cose::payload1(&self.object).map_err(|_| s("malformed"))?;
        let mut object = ChannelObject::decode(&payload).map_err(err)?;
        object.policy_id = policy::id(&signed);
        object.created_log = next.log;
        self.object = object.sign(&self.seat_key(), self.seat);
        self.policy = signed.clone();
        self.member.inner.borrow_mut().policy = Some(signed.clone());
        let mut e = Encoder::new(Vec::new());
        let _ = e.array(2).and_then(|e| e.bytes(&signed));
        let _ = e.array(0);
        Ok(e.into_writer())
    }

    /// This seat's approval of an announcement, the signed bytes to put in the array given to
    /// `ChannelMember::send_proposed`: `profile` `channel-post`, `request_digest` the
    /// proposal's `announcement_digest`, fields `[["channel", channel_name], ["text",
    /// first_line]]`, the current policy's id, `log` the channel position the proposer named,
    /// assurance present (2), a fresh nonce. Error: `digest`.
    pub fn approve_post(
        &self,
        digest_hex: &str,
        channel_name: &str,
        first_line: &str,
        log: u64,
    ) -> Result<Vec<u8>, String> {
        let digest: [u8; 32] = unhex_n(digest_hex, "digest")?;
        Ok(approval_for(
            &self.seat_key(),
            self.seat,
            &self.circle_id,
            PROFILE_POST,
            vec![
                (s("channel"), channel_name.to_string()),
                (s("text"), first_line.to_string()),
            ],
            digest,
            &policy::id(&self.policy),
            log,
            assurance::PRESENT,
            random::<16>(),
        ))
    }

    /// The policy change control message for the current policy (wire bytes).
    /// Error: `unknown-epoch`.
    pub fn policy_change_control(&mut self) -> Result<Vec<u8>, String> {
        let policy_id = policy::id(&self.policy);
        self.send_control(&Control::PolicyChange { policy_id })
    }

    /// The owner's own member: a handle onto the session inside this owner, under the
    /// self-grant. It sends and reads like any member; saving the owner saves it.
    pub fn member(&self) -> ChannelMember {
        ChannelMember {
            inner: Rc::clone(&self.member.inner),
        }
    }

    /// The whole owner as canonical CBOR, secrets included (the seat key's seed, the link
    /// secret, the current epoch key, the owner member's keys): store it under the app's own
    /// protection.
    ///
    /// `{0: 1, 1: seat, 2: seat seed, 3: manifest, 4: seats, 5: channel object, 6: policy,
    ///   7: link secret or nil, 8: epoch number, 9: epoch key or nil, 10: [deposit...],
    ///   11: [[grant_id, kid, xwing_pk, name tstr]...], 12: self grant, 13: member state}`
    pub fn export(&self) -> Vec<u8> {
        self.export_secret().to_vec()
    }

    /// `export`, in a buffer zeroized on drop.
    fn export_secret(&self) -> Zeroizing<Vec<u8>> {
        let member = self.member.inner.borrow().export();
        let mut e = Encoder::new(SecretBuf(Zeroizing::new(Vec::with_capacity(
            member.len() + self.manifest.len() + 8192,
        ))));
        let _ = (|| -> Result<(), minicbor::encode::Error<Infallible>> {
            let e = &mut e;
            e.map(14)?.u8(0)?.u8(STATE_VERSION)?;
            e.u8(1)?.u8(self.seat)?;
            e.u8(2)?.bytes(self.seat_seed.as_ref())?;
            e.u8(3)?.bytes(&self.manifest)?;
            e.u8(4)?.u8(self.seats)?;
            e.u8(5)?.bytes(&self.object)?;
            e.u8(6)?.bytes(&self.policy)?;
            e.u8(7)?;
            opt_bytes(e, self.secret.as_ref().map(|s| &s.0[..]))?;
            e.u8(8)?.u32(self.epoch)?;
            e.u8(9)?;
            opt_bytes(e, self.epoch_key.as_ref().map(|k| &k[..]))?;
            e.u8(10)?.array(self.deposits.len() as u64)?;
            for d in &self.deposits {
                e.bytes(d)?;
            }
            e.u8(11)?.array(self.members.len() as u64)?;
            for m in &self.members {
                e.array(4)?
                    .bytes(&m.grant_id)?
                    .bytes(&m.kid)?
                    .bytes(&m.xwing_pk)?
                    .str(&m.name)?;
            }
            e.u8(12)?.bytes(&self.self_grant)?;
            e.u8(13)?.bytes(&member)?;
            Ok(())
        })();
        e.into_writer().0
    }

    /// Read back what `export` wrote. The channel object is verified against the manifest and
    /// the seat seed against the manifest's seat key. Error: `state`.
    pub fn import(state: &[u8]) -> Result<ChannelOwner, String> {
        let bad = || s(STATE);
        let mut r = StateReader::open(state, 14)?;
        let seat = r.key()?.u8().map_err(|_| bad())?;
        let seat_seed = Zeroizing::new(r.array::<32>()?);
        let manifest_bytes = r.bytes()?.to_vec();
        let manifest = open_manifest(&manifest_bytes).map_err(|_| bad())?;
        let key = SeatKey::from_seed(&seat_seed).map_err(|_| bad())?;
        if manifest.seat_key(seat) != Some(&key.public()) {
            return Err(bad());
        }
        let seats = r.key()?.u8().map_err(|_| bad())?;
        let object = r.bytes()?.to_vec();
        let (_, obj) = ChannelObject::verify(&object, &manifest).map_err(|_| bad())?;
        let policy = r.bytes()?.to_vec();
        let secret = match r.opt_bytes()? {
            Some(b) => Some(LinkSecret(b.try_into().map_err(|_| bad())?)),
            None => None,
        };
        let epoch = r.key()?.u32().map_err(|_| bad())?;
        let epoch_key = match r.opt_bytes()? {
            Some(b) => Some(Zeroizing::new(<[u8; 32]>::try_from(b).map_err(|_| bad())?)),
            None => None,
        };
        let deposits = r.bytes_list()?;
        let members = {
            let d = r.key()?;
            let n = d.array().ok().flatten().ok_or_else(bad)?;
            let mut list = Vec::new();
            for _ in 0..n {
                if d.array().ok().flatten() != Some(4) {
                    return Err(bad());
                }
                let grant_id = d
                    .bytes()
                    .map_err(|_| bad())?
                    .try_into()
                    .map_err(|_| bad())?;
                let kid = d
                    .bytes()
                    .map_err(|_| bad())?
                    .try_into()
                    .map_err(|_| bad())?;
                let xwing_pk = d.bytes().map_err(|_| bad())?.to_vec();
                let name = d.str().map_err(|_| bad())?.to_string();
                list.push(Admitted {
                    grant_id,
                    kid,
                    xwing_pk,
                    name,
                });
            }
            list
        };
        let self_grant = r.bytes()?.to_vec();
        let member = MemberState::import(r.bytes()?)?;
        r.finish()?;
        if member.channel_id != obj.channel_id {
            return Err(bad());
        }
        let owner = ChannelOwner {
            circle_id: obj.circle_id,
            channel_id: obj.channel_id,
            seat,
            seat_seed,
            manifest: manifest_bytes,
            seats,
            object,
            policy,
            secret,
            epoch,
            epoch_key,
            deposits,
            members,
            self_grant,
            member: ChannelMember::from_state(member),
        };
        if *owner.export_secret() != *state {
            return Err(bad());
        }
        Ok(owner)
    }
}

// ---------------------------------------------------------------------------------------------
// Reading channel objects and policies
// ---------------------------------------------------------------------------------------------

/// Verify a signed channel object against the owner circle's encoded `SealedManifest`, and
/// return `{"circle_id":"<hex>","channel_id":"<hex>","name":"...","policy_id":"<hex>"}`.
/// Errors: `manifest`, `signature`, `malformed`, `not-canonical`, `wrong-circle`,
/// `wrong-channel`, `object:<reason>`.
#[wasm_bindgen]
pub fn verify_channel_object(bytes: &[u8], manifest: &[u8]) -> Result<String, String> {
    let manifest = open_manifest(manifest)?;
    let (_, o) = ChannelObject::verify(bytes, &manifest).map_err(err)?;
    Ok(format!(
        "{{\"circle_id\":\"{}\",\"channel_id\":\"{}\",\"name\":{},\"policy_id\":\"{}\"}}",
        hex(&o.circle_id),
        hex(&o.channel_id),
        json_str(&o.name),
        hex(&o.policy_id)
    ))
}

/// Who may read under a rule, for the summary: link evidence admits anyone with the link;
/// otherwise readers get key deposits from the owner.
fn reader(rule: Option<&Rule>) -> &'static str {
    match rule {
        Some(r) if has_link(r) => "anyone with the link",
        _ => "members the owner lets in",
    }
}

/// Who may post under a rule without a quorum: no rule or no evidence is only the owner's
/// seats (spec §3); link evidence is anyone with the link; other evidence (a key, or any key
/// the owner admits by hand) is the members the owner lets in.
fn poster(rule: Option<&Rule>) -> &'static str {
    match rule {
        None => "only the owner",
        Some(r) if r.evidence.is_empty() => "only the owner",
        Some(r) if has_link(r) => "anyone with the link",
        Some(_) => "members the owner lets in",
    }
}

/// The policy in one line, as the app shows it. Per preset:
///
/// - announcements: `"anyone with the link can read; only the owner can post"`
/// - with approvals: `"anyone with the link can read; the owner can post, with 2 approvals"`
/// - group chat: `"anyone with the link can read and post"`
/// - members post: `"anyone with the link can read; members the owner lets in can post"`
/// - closed: `"members the owner lets in can read; only the owner can post"`
///
/// Reads the signed policy without checking who signed it. Errors: `malformed`,
/// `not-canonical`.
#[wasm_bindgen]
pub fn policy_summary(bytes: &[u8]) -> Result<String, String> {
    let p = read_policy(bytes)?;
    let read = reader(p.rule(Action::Read));
    let post_rule = p.rule(Action::Post);
    let quorum = post_rule.filter(|r| !has_link(r)).map_or(0, |r| r.quorum);
    if quorum > 0 {
        let plural = if quorum == 1 { "" } else { "s" };
        return Ok(format!(
            "{read} can read; the owner can post, with {quorum} approval{plural}"
        ));
    }
    let post = poster(post_rule);
    Ok(if read == post {
        format!("{read} can read and post")
    } else {
        format!("{read} can read; {post} can post")
    })
}
