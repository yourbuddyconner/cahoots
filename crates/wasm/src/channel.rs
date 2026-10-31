//! Channel bindings (channels spec §4 to §9): [`ChannelOwner`] runs a channel from a circle's
//! seat, [`ChannelMember`] reads and posts in one, [`ChannelReceived`] is one message read.
//!
//! The logic is `cahoots_channel::owner` and `cahoots_channel::member`, shared with the phone;
//! these are thin wrappers that take and give hex, base64url and JSON as the app does.
//!
//! Byte results that hold several parts are canonical CBOR arrays of byte strings, each part
//! exactly the body the app posts; [`cbor_split`] turns one into base64url strings. Errors are
//! short stable strings, as in the rest of this crate.

use std::cell::RefCell;
use std::rc::Rc;

use cahoots_channel::control::Control;
use cahoots_channel::member::{
    self as core_member, bytes_array, open_manifest, read_bytes_array, JoinRequest,
};
use cahoots_channel::owner as core_owner;
use cahoots_channel::ChannelObject;
use cahoots_core::seat::SeatKey;
use wasm_bindgen::prelude::*;

use crate::{b64_decode, b64_encode, hex, Circle};

pub use cahoots_channel::member::JOIN_DOMAIN;
pub use cahoots_channel::owner::{GATE_DOMAIN, GATE_RETIRE_DOMAIN, REVOKE_DOMAIN};

type CoreMember = core_member::ChannelMember;
type CoreOwner = core_owner::ChannelOwner;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn s(code: &str) -> String {
    code.to_string()
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

/// Each byte string of a CBOR array, as base64url without padding: for the parts of
/// `admit`, `revoke`, `deposits` and `self_grant`'s chain. Error: `malformed`.
#[wasm_bindgen]
pub fn cbor_split(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(read_bytes_array(bytes)?
        .iter()
        .map(|b| b64_encode(b))
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Member
// ---------------------------------------------------------------------------------------------

/// Which member a handle reaches: one of its own, or the member inside an owner.
enum Handle {
    Own(Rc<RefCell<CoreMember>>),
    Owner(Rc<RefCell<CoreOwner>>),
}

/// A member of a channel: its member key, its X-Wing key for key deposits, the epoch keys it
/// holds, and its grant.
///
/// The owner's own member ([`ChannelOwner::member`]) is a handle onto the session inside the
/// owner: what one sends moves the other's counters, and saving the owner saves it.
#[wasm_bindgen]
pub struct ChannelMember {
    inner: Handle,
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
        self.with(|m| {
            f.debug_struct("ChannelMember")
                .field("channel_id", &hex(&m.channel_id()))
                .finish_non_exhaustive()
        })
    }
}

impl ChannelMember {
    fn own(member: CoreMember) -> ChannelMember {
        ChannelMember {
            inner: Handle::Own(Rc::new(RefCell::new(member))),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&CoreMember) -> T) -> T {
        match &self.inner {
            Handle::Own(m) => f(&m.borrow()),
            Handle::Owner(o) => f(o.borrow().member()),
        }
    }

    fn with_mut<T>(&self, f: impl FnOnce(&mut CoreMember) -> T) -> T {
        match &self.inner {
            Handle::Own(m) => f(&mut m.borrow_mut()),
            Handle::Owner(o) => f(o.borrow_mut().member_mut()),
        }
    }
}

#[wasm_bindgen]
impl ChannelMember {
    /// A new member from an invite link (the full URL or its `#/ch/...` fragment), with a fresh
    /// member key and X-Wing key. Error: `link`.
    pub fn from_link(url: &str) -> Result<ChannelMember, String> {
        CoreMember::from_link(url).map(ChannelMember::own)
    }

    pub fn channel_id_hex(&self) -> String {
        self.with(|m| hex(&m.channel_id()))
    }

    /// The relay host from the link (`host` or `host:port`).
    pub fn relay_host(&self) -> String {
        self.with(|m| m.relay_host().to_string())
    }

    /// The gate token for the `Cahoots-Gate` header, hex; `""` when the link had no secret.
    pub fn gate_token_hex(&self) -> String {
        self.with(|m| m.gate_token().map(|t| hex(&t)).unwrap_or_default())
    }

    /// Take a newer link to the same channel (after the owner retired the old one): its relay
    /// and secret replace the ones held; the member key, grant and epochs stay.
    /// Errors: `link`, `wrong-channel`.
    pub fn set_link(&mut self, url: &str) -> Result<(), String> {
        self.with_mut(|m| m.set_link(url))
    }

    /// Whether this member's own grant carries `post`: the composer is on only then. `false`
    /// without a grant, and for a grant to read only.
    pub fn may_post(&self) -> bool {
        self.with(|m| m.grant_allows(cahoots_channel::Action::Post))
    }

    /// Whether the held policy lets someone who is not an owner seat ask to post: its post rule
    /// takes link or key evidence. `false` without a policy, or when only the owner posts.
    pub fn may_ask_to_post(&self) -> bool {
        self.with(CoreMember::may_ask_to_post)
    }

    /// The link's id (`SHA-256("cahoots/link/id/v1" || secret)`), hex: the `to` of its link
    /// deposits; `""` when the link had no secret.
    pub fn link_id_hex(&self) -> String {
        self.with(|m| m.link_id().map(|id| hex(&id)).unwrap_or_default())
    }

    /// The member key's id (`SHA-256` of the compressed key), hex: the `Cahoots-Kid` header and
    /// the `kid` of this member's messages.
    pub fn my_kid_hex(&self) -> String {
        self.with(|m| hex(&m.my_kid()))
    }

    /// The member key, SEC1 compressed, hex.
    pub fn member_key_hex(&self) -> String {
        self.with(|m| hex(&m.member_key().0))
    }

    /// The epoch this member sends in, hex of the 20-byte epoch id; `None` before a deposit.
    pub fn current_epoch_hex(&self) -> Option<String> {
        self.with(|m| m.current_epoch().map(|id| hex(&id.encode())))
    }

    /// Open a signed link deposit with the link's secret and install its epoch, once its
    /// signer checks out. `signer_chain` is the grant chain the relay lists for the deposit's
    /// signer (`deposit_meta`'s `kid`), `manifest` the owner circle's encoded
    /// `SealedManifest`: the chain must verify, carry `govern`, and name the key that signed.
    /// An epoch already held is left as it is. Errors: `manifest`, `malformed`, `no-grant`,
    /// `grant:<reason>`, `no-admit`, `refused:<reason>`, `signature`, `no-link`,
    /// `wrong-channel`, `tag`, `reuse`.
    pub fn install_from_link_deposit(
        &mut self,
        deposit: &[u8],
        signer_chain: &[u8],
        manifest: &[u8],
    ) -> Result<(), String> {
        self.with_mut(|m| m.install_from_link_deposit(deposit, signer_chain, manifest))
    }

    /// Open a signed key deposit with this member's X-Wing secret key and install its epoch,
    /// once its signer checks out, as for `install_from_link_deposit`. An epoch already held
    /// is left as it is. Errors: as `install_from_link_deposit` but `no-link` and `tag`;
    /// `not-mine` (sealed to another key id), `hpke`.
    pub fn install_from_key_deposit(
        &mut self,
        deposit: &[u8],
        signer_chain: &[u8],
        manifest: &[u8],
    ) -> Result<(), String> {
        self.with_mut(|m| m.install_from_key_deposit(deposit, signer_chain, manifest))
    }

    /// A join request for the owner: the COSE_Sign1, kid this member's key id, external AAD
    /// `cahoots/join/v1`, over the canonical map
    /// `{0: channel_id, 1: member key bstr 33, 2: xwing_pk, 3: actions "read,post", 4: name,
    ///   5: gate token bstr 32, or empty without a link secret}`.
    /// Errors: `name` (empty or over 64 bytes), `actions`.
    pub fn join_request(&self, name: &str, actions: &str) -> Result<Vec<u8>, String> {
        self.with(|m| m.join_request(name, actions))
    }

    /// Keep this member's grant chain (a CBOR array of signed grants, root first, as `admit`
    /// returned it). `receive` uses it for this member's own posts. Error: `malformed`.
    pub fn set_grant(&mut self, chain: &[u8]) -> Result<(), String> {
        self.with_mut(|m| m.set_grant(chain))
    }

    /// Keep the channel's current signed policy, for counting announcement approvals.
    /// Errors: `malformed`, `wrong-channel`.
    pub fn set_policy(&mut self, signed: &[u8]) -> Result<(), String> {
        self.with_mut(|m| m.set_policy(signed))
    }

    /// The `Cahoots-Proof` header for a gated read: `proof_sign(member key, SHA-256(target),
    /// nonce)` as base64url without padding. `target` is `METHOD " " PATH "?" QUERY` exactly as
    /// in the request line, the `?` always written (`GET /v1/channels/<id>/head?`); `nonce_hex`
    /// is the relay's `GET /v1/nonce` answer. Error: `nonce`.
    pub fn proof_for(&self, target: &str, nonce_hex: &str) -> Result<String, String> {
        let nonce: [u8; 16] = unhex_n(nonce_hex, "nonce")?;
        Ok(b64_encode(&self.with(|m| m.proof_for(target, &nonce))))
    }

    /// Encrypt and sign a text post in the current epoch; base64url wire bytes. The text is
    /// sent as given: pad it with `chat_bucket_pad` first. Error: `unknown-epoch` before any
    /// deposit is installed.
    pub fn send(&mut self, text: &[u8]) -> Result<String, String> {
        self.with_mut(|m| m.send(text)).map(|w| b64_encode(&w))
    }

    /// Seal an announcement in the current epoch and keep it unsigned, for seats to approve.
    /// Returns the body (header and ciphertext). The counter is spent now, so the body is the
    /// one the relay will see: get its digest with `announcement_digest`, collect seat
    /// approvals of it (`ChannelOwner::approve_post`), then `send_proposed`. A new proposal
    /// replaces one not yet sent. The text is sent as given: pad it first.
    /// Error: `unknown-epoch`.
    pub fn propose_announcement(&mut self, text: &[u8]) -> Result<Vec<u8>, String> {
        self.with_mut(|m| m.propose_announcement(text))
    }

    /// `SHA-256` of the pending announcement's body, hex: the `request_digest` its approvals
    /// sign. Error: `no-proposal`.
    pub fn announcement_digest(&self) -> Result<String, String> {
        self.with(|m| m.announcement_digest()).map(|d| hex(&d))
    }

    /// Sign the pending announcement with `approvals` (a CBOR array of signed approvals) and
    /// return its base64url wire; the proposal is then gone. Errors: `no-proposal`,
    /// `malformed`.
    pub fn send_proposed(&mut self, approvals: &[u8]) -> Result<String, String> {
        self.with_mut(|m| m.send_proposed(approvals))
            .map(|w| b64_encode(&w))
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
        let r = self.with_mut(|m| m.receive(&wire, signer_chain, manifest))?;
        Ok(ChannelReceived {
            control_json: r.control.as_ref().map(control_json).unwrap_or_default(),
            text: r.text,
            kid: r.kid,
            counter: r.counter,
            msg_type: r.msg_type as u8,
            epoch: r.epoch.encode(),
            announcement_ok: r.announcement_ok,
        })
    }

    /// The whole member as canonical CBOR, secrets included (the member key's seed, the X-Wing
    /// secret key, the link secret, every epoch key): store it under the app's own protection.
    ///
    /// `{0: 1, 1: relay tstr, 2: channel_id, 3: link secret or nil, 4: member key seed,
    ///   5: xwing_sk, 6: xwing_pk, 7: session state, 8: [grant...], 9: policy or nil,
    ///   10: [revoked grant id...], 11: pending announcement body or nil}`
    pub fn export(&self) -> Vec<u8> {
        self.with(|m| m.export().to_vec())
    }

    /// Read back what `export` wrote. Error: `state`.
    pub fn import(state: &[u8]) -> Result<ChannelMember, String> {
        CoreMember::import(state).map(ChannelMember::own)
    }
}

/// A signed deposit's id (the SHA-256 of the unsigned deposit, as a rotation names it) and
/// its signer's key id, read without checking the signature:
/// `{"id":"<hex 32>","kid":"<hex 32>"}`. Fetch the signer's chain by `kid`; the install checks
/// it. Errors: `malformed`, `signature`, `wrong-channel`, `not-canonical`.
#[wasm_bindgen]
pub fn deposit_meta(signed: &[u8]) -> Result<String, String> {
    let (kid, d) = cahoots_channel::deposit::peek_signed(signed).map_err(core_member::code)?;
    Ok(format!(
        "{{\"id\":\"{}\",\"kid\":\"{}\"}}",
        hex(&d.id()),
        hex(&kid)
    ))
}

/// What a join request asks for, for the owner's Requests screen, after its signature checks:
/// `{"channel_id":"<hex>","kid":"<hex 32>","name":"...","actions":"read,post"}`.
/// Errors: `malformed`, `signature`.
#[wasm_bindgen]
pub fn join_request_summary(request: &[u8]) -> Result<String, String> {
    let r = JoinRequest::parse(request)?;
    Ok(format!(
        "{{\"channel_id\":\"{}\",\"kid\":\"{}\",\"name\":{},\"actions\":{}}}",
        hex(&r.channel),
        hex(&r.kid),
        json_str(&r.name),
        json_str(&r.wanted)
    ))
}

// ---------------------------------------------------------------------------------------------
// Owner
// ---------------------------------------------------------------------------------------------

/// A channel run from one seat of its owner circle: the signed channel object and policy, the
/// link secret, the epochs it issues, the members it admitted, and its own member. It keeps
/// the seat key's seed and signs with it.
#[wasm_bindgen]
pub struct ChannelOwner {
    inner: Rc<RefCell<CoreOwner>>,
}

impl std::fmt::Debug for ChannelOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let o = self.inner.borrow();
        f.debug_struct("ChannelOwner")
            .field("channel_id", &hex(&o.channel_id()))
            .field("seat", &o.seat())
            .field("members", &o.members().len())
            .finish_non_exhaustive()
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
        let owner = CoreOwner::start_with_seed(
            &self.manifest,
            self.my_slice(),
            seed,
            name,
            preset,
            seats_hint,
        )?;
        Ok(ChannelOwner {
            inner: Rc::new(RefCell::new(owner)),
        })
    }

    /// One approval of a proposed announcement by this device's seat, for a channel this
    /// circle owns but this device does not run (another seat's device proposed it): as
    /// `ChannelOwner::approve_post`, under `policy`, the channel's current signed policy,
    /// which must be signed by a seat of this circle. Errors: `no-seat-key`, `manifest`,
    /// `seat`, `malformed`, `signature`, `wrong-circle`, `digest`.
    pub fn approve_channel_post(
        &self,
        policy: &[u8],
        digest_hex: &str,
        channel_name: &str,
        first_line: &str,
        log: u64,
    ) -> Result<Vec<u8>, String> {
        let seed = self.seat_key.as_ref().ok_or_else(|| s("no-seat-key"))?;
        open_manifest(&self.manifest)?;
        let key = SeatKey::from_seed(seed).map_err(|_| s("seat"))?;
        // A digest that is not hex reaches the core as no bytes, refused there as `digest`
        // after the policy is checked.
        let digest = unhex(digest_hex).unwrap_or_default();
        core_owner::approve_channel_post(
            &self.manifest,
            self.my_slice(),
            &key,
            policy,
            &digest,
            channel_name,
            first_line,
            log,
        )
    }
}

impl ChannelOwner {
    /// The seat key from the seed the owner keeps.
    fn key(&self) -> Result<SeatKey, String> {
        self.inner
            .borrow()
            .seat_key()
            .ok_or_else(|| s("no-seat-key"))
    }
}

#[wasm_bindgen]
impl ChannelOwner {
    pub fn channel_id_hex(&self) -> String {
        hex(&self.inner.borrow().channel_id())
    }

    /// The signed channel object, the body of `POST /v1/channels`.
    pub fn channel_object(&self) -> Vec<u8> {
        self.inner.borrow().channel_object().to_vec()
    }

    /// The current signed policy.
    pub fn policy(&self) -> Vec<u8> {
        self.inner.borrow().policy().to_vec()
    }

    /// `[policy, []]`: the body of `POST …/policy` for the first policy, which needs no
    /// approvals.
    pub fn policy_body(&self) -> Vec<u8> {
        self.inner.borrow().policy_body()
    }

    /// SHA-256 of the current signed policy, hex.
    pub fn policy_id_hex(&self) -> String {
        hex(&self.inner.borrow().policy_id())
    }

    /// The owner circle's encoded `SealedManifest`: the body of `PUT` manifest, and what
    /// members pass to `receive` and `verify_channel_object`.
    pub fn manifest(&self) -> Vec<u8> {
        self.inner.borrow().manifest().to_vec()
    }

    /// The invite link on `relay_host` (`host` or `host:port`). Makes the link secret on the
    /// first call when there is none (a closed channel, or after `retire_link`), and keeps it;
    /// a policy names a new secret only after the next `set_policy`. Error: `relay`.
    pub fn link_url(&mut self, relay_host: &str) -> Result<String, String> {
        self.inner.borrow_mut().link_url(relay_host)
    }

    /// The seat-signed `cahoots/gate/v1` statement `{0: channel_id, 1: gate_token}` for the
    /// current link: the body of `POST …/gates`. Error: `no-link`.
    pub fn gate_registration(&self) -> Result<Vec<u8>, String> {
        let key = self.key()?;
        self.inner.borrow().gate_registration(&key)
    }

    /// SHA-256 of the current link's gate token, hex: the path of `DELETE …/gates/{hash}`.
    /// Read it before `retire_link`. `None` without a link.
    pub fn gate_token_hash_hex(&self) -> Option<String> {
        self.inner.borrow().gate_token_hash().map(|h| hex(&h))
    }

    /// The seat-signed `cahoots/gate-retire/v1` statement for the current link (the body of
    /// `DELETE …/gates/{hash}`), and forget its secret at once: no later admission can use it
    /// and later rotations make no link deposit. The statement stays in `pending_retire_json`
    /// until `retire_confirmed`. Error: `no-link`.
    pub fn retire_link(&mut self) -> Result<Vec<u8>, String> {
        let key = self.key()?;
        self.inner.borrow_mut().retire_link(&key)
    }

    /// Retired links the relay has not confirmed removing, oldest first:
    /// `[{"hash":"<hex 32>","statement":"<base64url>"}]`. Send each as
    /// `DELETE …/gates/{hash}` with the statement as the body; on 204 (or 404, already gone)
    /// call `retire_confirmed(hash)`.
    pub fn pending_retire_json(&self) -> String {
        let o = self.inner.borrow();
        let items: Vec<String> = o
            .pending_retire()
            .iter()
            .map(|(h, st)| {
                format!(
                    "{{\"hash\":\"{}\",\"statement\":\"{}\"}}",
                    hex(h),
                    b64_encode(st)
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }

    /// The relay removed the gate token whose hash is `hash_hex`: stop sending its retire.
    /// Error: `hash`.
    pub fn retire_confirmed(&mut self, hash_hex: &str) -> Result<(), String> {
        let hash: [u8; 32] = unhex_n(hash_hex, "hash")?;
        self.inner.borrow_mut().retire_confirmed(&hash);
        Ok(())
    }

    /// The owner member's grant chain: a CBOR array of one signed root grant for read, post,
    /// admit and govern, depth 2, no budget, `not_after = u64::MAX`. The body of
    /// `POST …/grants`.
    pub fn self_grant(&self) -> Vec<u8> {
        self.inner.borrow().self_grant()
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
        self.inner.borrow_mut().rotate()
    }

    /// The deposits made for the current epoch, a CBOR array of signed deposits, each the
    /// body of one `POST …/deposits`: one for the live link, then one per admitted member.
    pub fn deposits(&self) -> Vec<u8> {
        let o = self.inner.borrow();
        let refs: Vec<&[u8]> = o.deposits().iter().map(Vec::as_slice).collect();
        bytes_array(&refs)
    }

    /// A key deposit of the current epoch for one more key (`kid_hex` its signing key's id),
    /// outside the admitted list: it is not repeated at the next rotation.
    /// Errors: `kid`, `unknown-epoch`, `hpke`.
    pub fn deposit_for_key(&self, kid_hex: &str, xwing_pk: &[u8]) -> Result<Vec<u8>, String> {
        let kid: [u8; 32] = unhex_n(kid_hex, "kid")?;
        self.inner.borrow().deposit_for_key(&kid, xwing_pk)
    }

    /// The actions a join request may be granted automatically, as `"read,post"`: those asked
    /// for whose rule takes link evidence the request's gate token proves against the live
    /// link. `""` when nothing is (closed, members post, an old link): auto-allow skips it.
    /// Errors: `malformed`, `signature`, `wrong-channel`, `actions`.
    pub fn permitted(&self, request: &[u8]) -> Result<String, String> {
        let actions = self.inner.borrow().permitted(request)?;
        Ok(actions
            .iter()
            .map(|a| a.name())
            .collect::<Vec<_>>()
            .join(","))
    }

    /// Admit a join request (as `ChannelMember::join_request` makes it), granting `actions`
    /// ("read,post"), which must be among those requested. Each granted action needs a rule
    /// with evidence (else `policy`: `admit` and `govern` always, `post` in an announcements
    /// channel), and the request must show one of it: the live link's gate token for link
    /// evidence; key evidence is the owner choosing (else `evidence`).
    ///
    /// Issues a root grant (depth 0, no budget, `not_after = u64::MAX`), seals the current
    /// epoch to the member's X-Wing key (a signed deposit), keeps the member for later
    /// rotations, and returns the CBOR array `[grant chain, key deposit, join control wire]`:
    /// post the chain to `…/grants`, the deposit to `…/deposits`, the control to `…/messages`.
    /// Errors: `malformed`, `signature`, `wrong-channel`, `actions`, `name`, `policy`,
    /// `evidence`, `unknown-epoch`, `hpke`.
    pub fn admit(&mut self, request: &[u8], actions: &str) -> Result<Vec<u8>, String> {
        let key = self.key()?;
        let a = self.inner.borrow_mut().admit(request, actions, &key)?;
        Ok(bytes_array(&[&a.chain, &a.deposit, &a.control]))
    }

    /// The admitted members, as JSON: `[{"grant_id":"<hex 16>","kid":"<hex 32>","name":"..."}]`.
    pub fn members_json(&self) -> String {
        let o = self.inner.borrow();
        let items: Vec<String> = o
            .members()
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
        let key = self.key()?;
        let r = self.inner.borrow_mut().revoke(&grant_id, reason, &key)?;
        Ok(bytes_array(&[&r.statement, &r.control]))
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
        let key = self.key()?;
        self.inner.borrow_mut().set_policy(preset, quorum, &key)
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
        let key = self.key()?;
        self.inner
            .borrow()
            .approve_post(&digest, channel_name, first_line, log, &key)
    }

    /// The policy change control message for the current policy (wire bytes).
    /// Error: `unknown-epoch`.
    pub fn policy_change_control(&mut self) -> Result<Vec<u8>, String> {
        self.inner.borrow_mut().policy_change_control()
    }

    /// The owner's own member: a handle onto the session inside this owner, under the
    /// self-grant. It sends and reads like any member; saving the owner saves it.
    pub fn member(&self) -> ChannelMember {
        ChannelMember {
            inner: Handle::Owner(Rc::clone(&self.inner)),
        }
    }

    /// The whole owner as canonical CBOR, secrets included (the seat key's seed, the link
    /// secret, the current epoch key, the owner member's keys): store it under the app's own
    /// protection.
    ///
    /// `{0: 1, 1: seat, 2: seat seed, 3: manifest, 4: seats, 5: channel object, 6: policy,
    ///   7: link secret or nil, 8: epoch number, 9: epoch key or nil, 10: [deposit...],
    ///   11: [[grant_id, kid, xwing_pk, name tstr]...], 12: self grant, 13: member state,
    ///   14: [[token hash, retire statement]...]}`
    pub fn export(&self) -> Vec<u8> {
        self.inner.borrow().export().to_vec()
    }

    /// Read back what `export` wrote. The channel object is verified against the manifest and
    /// the seat seed against the manifest's seat key. Error: `state`.
    pub fn import(state: &[u8]) -> Result<ChannelOwner, String> {
        let owner = CoreOwner::import(state)?;
        // The browser's owner always keeps its seat seed.
        if owner.seat_key().is_none() {
            return Err(s("state"));
        }
        Ok(ChannelOwner {
            inner: Rc::new(RefCell::new(owner)),
        })
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
    let (_, o) = ChannelObject::verify(bytes, &manifest).map_err(core_member::code)?;
    Ok(format!(
        "{{\"circle_id\":\"{}\",\"channel_id\":\"{}\",\"name\":{},\"policy_id\":\"{}\"}}",
        hex(&o.circle_id),
        hex(&o.channel_id),
        json_str(&o.name),
        hex(&o.policy_id)
    ))
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
    core_member::policy_summary(bytes)
}
