//! A channel member as an app runs one (channels spec §4 to §9): its member key, its X-Wing key
//! for key deposits, the epoch keys it holds, its grant, and the policy it last read. Shared by
//! the browser's wasm bindings and the phone. Needs `std` for HPKE and the system's randomness.
//!
//! Errors are short stable strings (`malformed`, `refused:revoked`, ...), the codes the apps
//! show in plain words. [`code`] turns a [`ChannelError`] into one.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::convert::Infallible;

use cahoots_core::cose;
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_hpke::Suite;
use cahoots_objects::authz::proof_sign;
use minicbor::{Decoder, Encoder};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::control::Control;
use crate::deposit::{self, Deposit};
use crate::grant::{self, EvalRequest};
use crate::keys::{kid_of, EpochId, Key};
use crate::link::{Link, LinkSecret};
use crate::message::{self, MessageType};
use crate::policy::{kind, Rule};
use crate::quorum::{check_quorum, PROFILE_POST};
use crate::session::ChannelSession;
use crate::{Action, ChannelError, ChannelId, Policy, Refusal};

/// Domain of a join request's COSE_Sign1.
pub const JOIN_DOMAIN: &[u8] = b"cahoots/join/v1";

/// The longest channel or member name, in bytes.
pub const NAME_MAX: usize = 64;
/// Format version of [`ChannelMember::export`] and `ChannelOwner::export`.
pub(crate) const STATE_VERSION: u8 = 1;

// ---------------------------------------------------------------------------------------------
// Helpers shared with the owner
// ---------------------------------------------------------------------------------------------

/// The system's generator through `getrandom`, behind the `rand_core` 0.10 traits that
/// `cahoots-hpke` takes.
pub(crate) struct Rng10;

impl rand_core::TryRng for Rng10 {
    type Error = Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(u32::from_le_bytes(random()))
    }
    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok(u64::from_le_bytes(random()))
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        fill(dst);
        Ok(())
    }
}

impl rand_core::TryCryptoRng for Rng10 {}

fn fill(dst: &mut [u8]) {
    getrandom::getrandom(dst).expect("the system's random number generator");
}

pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    fill(&mut out);
    out
}

pub(crate) fn random_key() -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    fill(out.as_mut());
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

pub(crate) fn s(code: &str) -> String {
    code.to_string()
}

/// The stable string for a channel error.
pub fn code(e: ChannelError) -> String {
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

/// `refused:<reason>`.
pub fn refusal(r: Refusal) -> String {
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

/// The canonical CBOR array of byte strings.
pub fn bytes_array(items: &[&[u8]]) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = e.array(items.len() as u64);
    for item in items {
        let _ = e.bytes(item);
    }
    e.into_writer()
}

/// The byte strings of a canonical CBOR array. Error: `malformed`.
pub fn read_bytes_array(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
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

/// The owner circle's manifest from its encoded `SealedManifest`, every seat's signature
/// checked. Error: `manifest`.
pub fn open_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    SealedManifest::decode(bytes)
        .and_then(|m| m.verify())
        .map_err(|_| s("manifest"))
}

/// The policy a signed policy holds, without checking who signed it.
/// Errors: `malformed`, `not-canonical`.
pub fn read_policy(signed: &[u8]) -> Result<Policy, String> {
    let payload = cose::payload1(signed).map_err(|_| s("malformed"))?;
    Policy::decode(&payload).map_err(code)
}

/// `"read,post"` as actions, each once in number order. Error: `actions`.
pub(crate) fn parse_actions(text: &str) -> Result<Vec<Action>, String> {
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

pub(crate) fn actions_text(actions: &[Action]) -> String {
    actions
        .iter()
        .map(|a| a.name())
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > NAME_MAX {
        return Err(s("name"));
    }
    Ok(())
}

pub(crate) fn has_link(rule: &Rule) -> bool {
    rule.evidence.iter().any(|e| e.kind == kind::LINK)
}

/// Reads a state map whose keys are `0..n` in order, key 0 the state version.
pub(crate) struct StateReader<'b> {
    d: Decoder<'b>,
    len: usize,
    next: u64,
}

pub(crate) const STATE: &str = "state";

impl<'b> StateReader<'b> {
    pub(crate) fn open(bytes: &'b [u8], n: u64) -> Result<Self, String> {
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

    pub(crate) fn key(&mut self) -> Result<&mut Decoder<'b>, String> {
        if self.d.u64().ok() != Some(self.next) {
            return Err(s(STATE));
        }
        self.next += 1;
        Ok(&mut self.d)
    }

    pub(crate) fn bytes(&mut self) -> Result<&'b [u8], String> {
        self.key()?.bytes().map_err(|_| s(STATE))
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.bytes()?.try_into().map_err(|_| s(STATE))
    }

    pub(crate) fn opt_bytes(&mut self) -> Result<Option<&'b [u8]>, String> {
        let d = self.key()?;
        if d.datatype().map_err(|_| s(STATE))? == minicbor::data::Type::Null {
            d.null().map_err(|_| s(STATE))?;
            return Ok(None);
        }
        d.bytes().map(Some).map_err(|_| s(STATE))
    }

    pub(crate) fn bytes_list(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let d = self.key()?;
        let n = d.array().ok().flatten().ok_or_else(|| s(STATE))?;
        (0..n)
            .map(|_| d.bytes().map(<[u8]>::to_vec).map_err(|_| s(STATE)))
            .collect()
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        if self.d.position() != self.len {
            return Err(s(STATE));
        }
        Ok(())
    }
}

/// A buffer that zeroizes what it held each time it grows, so an export leaves no copy of its
/// secrets behind.
pub(crate) struct SecretBuf(pub(crate) Zeroizing<Vec<u8>>);

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

pub(crate) fn opt_bytes<W: minicbor::encode::Write>(
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
// The member
// ---------------------------------------------------------------------------------------------

/// A member of a channel: its member key, its X-Wing key for key deposits, the epoch keys it
/// holds, and its grant.
pub struct ChannelMember {
    /// The relay host from the link; empty for an owner's member until `link_url`.
    pub(crate) relay: String,
    pub(crate) channel_id: ChannelId,
    pub(crate) secret: Option<LinkSecret>,
    /// The member key's seed. The session holds the same key.
    seed: Zeroizing<[u8; 32]>,
    xwing_sk: Zeroizing<Vec<u8>>,
    xwing_pk: Vec<u8>,
    session: ChannelSession,
    /// This member's own grant chain, root first, each link's signed bytes.
    pub(crate) grant: Vec<Vec<u8>>,
    /// The channel's current signed policy, as the app last handed it.
    pub(crate) policy: Option<Vec<u8>>,
    /// Grant ids revoked by control messages this member has read.
    pub(crate) revoked: Vec<[u8; 16]>,
    /// An announcement sealed by `propose_announcement` and not yet signed: its body
    /// (header and ciphertext), whose counter is already spent.
    pending: Option<Vec<u8>>,
}

impl Drop for ChannelMember {
    fn drop(&mut self) {
        if let Some(secret) = self.secret.as_mut() {
            secret.0.zeroize();
        }
    }
}

impl core::fmt::Debug for ChannelMember {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelMember")
            .field("channel_id", &self.channel_id)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// One message read from a channel.
#[derive(Debug)]
pub struct ChannelReceived {
    /// The plaintext: the sender's padded text for a post or announcement, the control map's
    /// bytes for a control message.
    pub text: Vec<u8>,
    /// The sender's key id.
    pub kid: [u8; 32],
    pub counter: u32,
    pub msg_type: MessageType,
    /// The epoch the message was sent in.
    pub epoch: EpochId,
    /// What a control message says; `None` for any other type.
    pub control: Option<Control>,
    /// For an announcement: its approvals reach the post rule's quorum under the policy this
    /// member holds. False without a policy, and for every other type.
    pub announcement_ok: bool,
}

impl ChannelMember {
    pub(crate) fn new(
        channel_id: ChannelId,
        relay: String,
        secret: Option<LinkSecret>,
    ) -> ChannelMember {
        let (seed, key) = new_member_key();
        let (xwing_sk, xwing_pk) = cahoots_hpke::gen_keypair(Suite::XWing, &mut Rng10);
        ChannelMember {
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

    pub(crate) fn install(&mut self, id: EpochId, key: Key) -> Result<(), String> {
        if self.session.has_epoch(&id) {
            return Ok(());
        }
        self.session.install_epoch(id, key).map_err(code)
    }

    pub(crate) fn send_control(&mut self, control: &Control) -> Result<Vec<u8>, String> {
        self.session
            .send(MessageType::Control, &control.encode(), &[])
            .map_err(code)
    }

    pub(crate) fn public(&self) -> SeatPublic {
        self.session.my_public()
    }

    /// A new member from an invite link (the full URL or its `#/ch/...` fragment), with a fresh
    /// member key and X-Wing key. Error: `link`.
    pub fn from_link(url: &str) -> Result<ChannelMember, String> {
        let link = Link::parse(url).ok_or_else(|| s("link"))?;
        Ok(ChannelMember::new(
            link.channel_id,
            link.relay.clone(),
            link.secret.clone(),
        ))
    }

    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// The relay host from the link (`host` or `host:port`).
    pub fn relay_host(&self) -> &str {
        &self.relay
    }

    /// The gate token for the `Cahoots-Gate` header; `None` when the link had no secret.
    pub fn gate_token(&self) -> Option<[u8; 32]> {
        self.secret
            .as_ref()
            .map(|sec| sec.gate_token(&self.channel_id))
    }

    /// Take a newer link to the same channel (after the owner retired the old one): its relay
    /// and secret replace the ones held; the member key, grant and epochs stay.
    /// Errors: `link`, `wrong-channel`.
    pub fn set_link(&mut self, url: &str) -> Result<(), String> {
        let link = Link::parse(url).ok_or_else(|| s("link"))?;
        if link.channel_id != self.channel_id {
            return Err(s("wrong-channel"));
        }
        if let Some(mut old) = self.secret.take() {
            old.0.zeroize();
        }
        self.secret = link.secret.clone();
        self.relay = link.relay.clone();
        Ok(())
    }

    /// Whether the held policy lets someone who is not an owner seat ask to post: its post rule
    /// takes link or key evidence. `false` without a policy, or when only the owner posts.
    pub fn may_ask_to_post(&self) -> bool {
        self.policy
            .as_deref()
            .and_then(|signed| read_policy(signed).ok())
            .is_some_and(|p| {
                p.rule(Action::Post).is_some_and(|r| {
                    r.evidence
                        .iter()
                        .any(|e| e.kind == kind::LINK || e.kind == kind::KEY)
                })
            })
    }

    /// The link's id (`SHA-256("cahoots/link/id/v1" || secret)`): the `to` of its link
    /// deposits; `None` when the link had no secret.
    pub fn link_id(&self) -> Option<[u8; 32]> {
        self.secret.as_ref().map(LinkSecret::id)
    }

    /// The member key's id (`SHA-256` of the compressed key): the `Cahoots-Kid` header and
    /// the `kid` of this member's messages.
    pub fn my_kid(&self) -> [u8; 32] {
        self.session.my_kid()
    }

    /// The member key, SEC1 compressed.
    pub fn member_key(&self) -> SeatPublic {
        self.session.my_public()
    }

    /// The epoch this member sends in; `None` before a deposit.
    pub fn current_epoch(&self) -> Option<EpochId> {
        self.session.current_epoch()
    }

    /// Open a link deposit with the link's secret and install its epoch. An epoch already held
    /// is left as it is. Errors: `no-link`, `wrong-channel`, `tag`, `malformed`, `reuse`.
    pub fn install_from_link_deposit(&mut self, deposit: &[u8]) -> Result<(), String> {
        let d = Deposit::decode(deposit).map_err(code)?;
        if d.channel_id != self.channel_id {
            return Err(s("wrong-channel"));
        }
        let secret = self.secret.as_ref().ok_or_else(|| s("no-link"))?;
        let key = deposit::open_with_link(&d, secret).map_err(code)?;
        self.install(d.epoch_id, key)
    }

    /// Open a key deposit with this member's X-Wing secret key and install its epoch. An epoch
    /// already held is left as it is. Errors: `not-mine` (sealed to another key id),
    /// `wrong-channel`, `hpke`, `malformed`, `reuse`.
    pub fn install_from_key_deposit(&mut self, deposit: &[u8]) -> Result<(), String> {
        let d = Deposit::decode(deposit).map_err(code)?;
        if d.channel_id != self.channel_id {
            return Err(s("wrong-channel"));
        }
        if d.kind == deposit::KIND_KEY && d.to != self.session.my_kid() {
            return Err(s("not-mine"));
        }
        let key = deposit::open_with_key(&d, &self.xwing_sk).map_err(code)?;
        self.install(d.epoch_id, key)
    }

    /// A join request for the owner: the COSE_Sign1, kid this member's key id, external AAD
    /// `cahoots/join/v1`, over the canonical map
    /// `{0: channel_id, 1: member key bstr 33, 2: xwing_pk, 3: actions "read,post", 4: name,
    ///   5: gate token bstr 32, or empty without a link secret}`.
    /// Errors: `name` (empty or over 64 bytes), `actions`.
    pub fn join_request(&self, name: &str, actions: &str) -> Result<Vec<u8>, String> {
        check_name(name)?;
        let actions = actions_text(&parse_actions(actions)?);
        let token = self
            .secret
            .as_ref()
            .map(|sec| sec.gate_token(&self.channel_id).to_vec())
            .unwrap_or_default();
        let key = self.key();
        let mut e = Encoder::new(Vec::new());
        let _ = (|| -> Result<(), minicbor::encode::Error<Infallible>> {
            e.map(6)?;
            e.u8(0)?.bytes(&self.channel_id)?;
            e.u8(1)?.bytes(&key.public().0)?;
            e.u8(2)?.bytes(&self.xwing_pk)?;
            e.u8(3)?.str(&actions)?;
            e.u8(4)?.str(name)?;
            e.u8(5)?.bytes(&token)?;
            Ok(())
        })();
        let kid = self.session.my_kid();
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
        self.grant = read_bytes_array(chain)?;
        Ok(())
    }

    /// This member's grant chain, root first; empty before `set_grant`.
    pub fn grant(&self) -> &[Vec<u8>] {
        &self.grant
    }

    /// Keep the channel's current signed policy, for counting announcement approvals.
    /// Errors: `malformed`, `wrong-channel`.
    pub fn set_policy(&mut self, signed: &[u8]) -> Result<(), String> {
        let p = read_policy(signed)?;
        if p.channel_id != self.channel_id {
            return Err(s("wrong-channel"));
        }
        self.policy = Some(signed.to_vec());
        Ok(())
    }

    /// The signed policy last kept with `set_policy`.
    pub fn policy(&self) -> Option<&[u8]> {
        self.policy.as_deref()
    }

    /// The `Cahoots-Proof` header's bytes for a gated read: `proof_sign(member key,
    /// SHA-256(target), nonce)`. `target` is `METHOD " " PATH "?" QUERY` exactly as in the
    /// request line, the `?` always written (`GET /v1/channels/<id>/head?`); `nonce` is the
    /// relay's `GET /v1/nonce` answer.
    pub fn proof_for(&self, target: &str, nonce: &[u8; 16]) -> Vec<u8> {
        let hash: [u8; 32] = Sha256::digest(target.as_bytes()).into();
        proof_sign(&self.key(), &hash, nonce)
    }

    /// Encrypt and sign a text post in the current epoch; the wire bytes. The text is sent as
    /// given: pad it with `cahoots_chat::bucket_pad` first. Error: `unknown-epoch` before any
    /// deposit is installed.
    pub fn send(&mut self, text: &[u8]) -> Result<Vec<u8>, String> {
        self.session
            .send(MessageType::Text, text, &[])
            .map_err(code)
    }

    /// Seal an announcement in the current epoch and keep it unsigned, for seats to approve.
    /// Returns the body (header and ciphertext). The counter is spent now, so the body is the
    /// one the relay will see: get its digest with `announcement_digest`, collect seat
    /// approvals of it (`ChannelOwner::approve_post`), then `send_proposed`. A new proposal
    /// replaces one not yet sent. The text is sent as given: pad it first.
    /// Error: `unknown-epoch`.
    pub fn propose_announcement(&mut self, text: &[u8]) -> Result<Vec<u8>, String> {
        // The session seals and signs in one step; the signature over no approvals is
        // dropped, and `send_proposed` signs the same body again with the approvals.
        let wire = self
            .session
            .send(MessageType::Announcement, text, &[])
            .map_err(code)?;
        let (body, _, _) = message::split(&wire).map_err(code)?;
        let body = body.to_vec();
        self.pending = Some(body.clone());
        Ok(body)
    }

    /// `SHA-256` of the pending announcement's body: the `request_digest` its approvals sign.
    /// Error: `no-proposal`.
    pub fn announcement_digest(&self) -> Result<[u8; 32], String> {
        let body = self.pending.as_ref().ok_or_else(|| s("no-proposal"))?;
        Ok(message::body_digest(body))
    }

    /// Sign the pending announcement with `approvals` (a CBOR array of signed approvals) and
    /// return its wire bytes; the proposal is then gone. Errors: `no-proposal`, `malformed`.
    pub fn send_proposed(&mut self, approvals: &[u8]) -> Result<Vec<u8>, String> {
        let approvals = read_bytes_array(approvals)?;
        let refs: Vec<&[u8]> = approvals.iter().map(Vec::as_slice).collect();
        let body = self.pending.take().ok_or_else(|| s("no-proposal"))?;
        let mut aad = Vec::with_capacity(message::DOMAIN.len() + body.len());
        aad.extend_from_slice(message::DOMAIN);
        aad.extend_from_slice(&body);
        let kid = self.session.my_kid();
        let signature = cose::sign1(&self.key(), &kid, &bytes_array(&refs), &aad, false);
        let mut wire = body;
        wire.extend_from_slice(&signature);
        Ok(wire)
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
    /// decode (`malformed`) still uses up its counter: only a granted sender can make one.
    /// Errors: `manifest`, `no-grant`, `malformed`, `signature`, `grant:<reason>`, `no-admit`,
    /// `wrong-channel`, `refused:<reason>` (`revoked`, `action`, ...), `unknown-epoch`,
    /// `reuse`, `too-far-ahead`, `tag`.
    pub fn receive(
        &mut self,
        wire: &[u8],
        signer_chain: &[u8],
        manifest: &[u8],
    ) -> Result<ChannelReceived, String> {
        let manifest = open_manifest(manifest)?;
        let chain = if signer_chain.is_empty() {
            self.grant.clone()
        } else {
            read_bytes_array(signer_chain)?
        };
        if chain.is_empty() {
            return Err(s("no-grant"));
        }
        let refs: Vec<&[u8]> = chain.iter().map(Vec::as_slice).collect();
        let channel_id = self.channel_id;
        let leaf = grant::verify_chain(&refs, &manifest, &channel_id).map_err(code)?;
        let ids = grant::chain_ids(&refs).map_err(code)?;
        let (body, _, header) = message::split(wire).map_err(code)?;
        if header.kid != kid_of(&leaf.grantee) {
            return Err(s("signature"));
        }
        let digest = message::body_digest(body);
        let action = match header.msg_type {
            MessageType::Control => Action::Govern,
            MessageType::Text | MessageType::Announcement => Action::Post,
        };
        let revoked = self.revoked.clone();
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
        let (header, text, approvals) = self.session.receive(wire, &leaf.grantee).map_err(code)?;
        let announcement_ok = header.msg_type == MessageType::Announcement
            && self.policy.as_ref().is_some_and(|signed| {
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
        let control = if header.msg_type == MessageType::Control {
            let control = Control::decode(&text).map_err(code)?;
            if let Control::Revocation { grant_id, .. } = &control {
                if !self.revoked.contains(grant_id) {
                    self.revoked.push(*grant_id);
                }
            }
            Some(control)
        } else {
            None
        };
        Ok(ChannelReceived {
            text,
            kid: header.kid,
            counter: header.counter,
            msg_type: header.msg_type,
            epoch: header.epoch_id,
            control,
            announcement_ok,
        })
    }

    /// The whole member as canonical CBOR, secrets included (the member key's seed, the X-Wing
    /// secret key, the link secret, every epoch key): store it under the app's own protection.
    ///
    /// `{0: 1, 1: relay tstr, 2: channel_id, 3: link secret or nil, 4: member key seed,
    ///   5: xwing_sk, 6: xwing_pk, 7: session state, 8: [grant...], 9: policy or nil,
    ///   10: [revoked grant id...], 11: pending announcement body or nil}`
    pub fn export(&self) -> Zeroizing<Vec<u8>> {
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

    /// Read back what `export` wrote. Error: `state`.
    pub fn import(bytes: &[u8]) -> Result<ChannelMember, String> {
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
        let state = ChannelMember {
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

// ---------------------------------------------------------------------------------------------
// Join requests
// ---------------------------------------------------------------------------------------------

/// A join request, parsed and its signature checked.
#[derive(Debug, Clone)]
pub struct JoinRequest {
    /// The channel id the request names, as sent (not checked to be 16 bytes).
    pub channel: Vec<u8>,
    pub member_key: SeatPublic,
    pub xwing_pk: Vec<u8>,
    /// The actions asked for, as sent: `"read,post"`.
    pub wanted: String,
    pub name: String,
    /// The gate token shown, or empty.
    pub token: Vec<u8>,
    /// The member key's id.
    pub kid: [u8; 32],
}

impl JoinRequest {
    /// Errors: `malformed`, `signature`.
    pub fn parse(request: &[u8]) -> Result<JoinRequest, String> {
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
        let channel = d.bytes().map_err(|_| bad())?.to_vec();
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
        Ok(JoinRequest {
            channel,
            member_key,
            xwing_pk,
            wanted,
            name,
            token,
            kid,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The policy in one line
// ---------------------------------------------------------------------------------------------

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

/// The policy in one line, as the apps show it. Per preset:
///
/// - announcements: `"anyone with the link can read; only the owner can post"`
/// - with approvals: `"anyone with the link can read; the owner can post, with 2 approvals"`
/// - group chat: `"anyone with the link can read and post"`
/// - members post: `"anyone with the link can read; members the owner lets in can post"`
/// - closed: `"members the owner lets in can read; only the owner can post"`
///
/// Reads the signed policy without checking who signed it. Errors: `malformed`,
/// `not-canonical`.
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
