//! A channel run from one seat of its owner circle (channels spec §4 to §9): the signed channel
//! object and policy, the link secret, the epochs it issues, the members it admitted, and its
//! own member. Shared by the browser's wasm bindings and the phone. Needs `std`.
//!
//! What a seat signs (the policy, the channel object, grants, gate and revocation statements,
//! approvals) is signed by the `SeatSigner` the caller passes: the browser's seat key from its
//! seed, or the phone's Secure Enclave. Each signing call first checks that the signer is the
//! manifest's key for this seat (`seat`). The owner may keep the seat key's seed itself
//! ([`ChannelOwner::start_with_seed`]), which [`ChannelOwner::seat_key`] then gives back;
//! started with a signer, it keeps none.
//!
//! Errors are short stable strings, as in [`crate::member`].

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use cahoots_core::cose;
use cahoots_core::seat::{SeatKey, SeatSigner};
use cahoots_core::CircleId;
use cahoots_objects::assurance;
use minicbor::Encoder;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::control::Control;
use crate::deposit::{self, Deposit};
use crate::grant;
use crate::keys::{epoch_key, EpochId, Key};
use crate::link::{Link, LinkSecret};
use crate::member::{
    bytes_array, check_name, code, has_link, open_manifest, opt_bytes, parse_actions, random,
    random_key, read_policy, s, ChannelMember, JoinRequest, Rng10, SecretBuf, StateReader, STATE,
    STATE_VERSION,
};
use crate::object::channel_id_for;
use crate::policy::{self, kind};
use crate::quorum::{approval_for, PROFILE_POST};
use crate::{Action, ChannelId, ChannelObject, Policy, Preset};

/// Domain of the statement that registers a gate token with the relay.
pub const GATE_DOMAIN: &[u8] = b"cahoots/gate/v1";
/// Domain of the statement that removes a gate token from the relay.
pub const GATE_RETIRE_DOMAIN: &[u8] = b"cahoots/gate-retire/v1";
/// Domain of a revocation statement.
pub const REVOKE_DOMAIN: &[u8] = b"cahoots/revoke/v1";

/// The editor's presets by number: 0 announcements, 1 announcements with `quorum` approvals,
/// 2 group chat, 3 members post, 4 closed. Errors: `preset`, `quorum` (0 for preset 1).
pub fn preset_of(n: u8, quorum: u8) -> Result<Preset, String> {
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

/// The majority of `seats`: the approvals preset 1 asks for.
pub fn majority(seats: u8) -> u8 {
    if seats <= 1 {
        1
    } else {
        seats / 2 + 1
    }
}

/// A seat-signed statement `{0: channel_id, 1: second}` or, with a reason, `{..., 2: reason}`.
fn statement<S: SeatSigner>(
    key: S,
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

/// A member the owner admitted and seals each epoch to.
#[derive(Debug, Clone)]
pub struct Admitted {
    pub grant_id: [u8; 16],
    pub kid: [u8; 32],
    pub xwing_pk: Vec<u8>,
    pub name: String,
}

/// What `admit` makes: post the chain to `…/grants`, the deposit to `…/deposits`, the control
/// to `…/messages`.
#[derive(Debug, Clone)]
pub struct Admission {
    pub grant_id: [u8; 16],
    pub kid: [u8; 32],
    /// The grant chain, a CBOR array of one signed root grant.
    pub chain: Vec<u8>,
    /// The key deposit of the current epoch for the new member.
    pub deposit: Vec<u8>,
    /// The join control message, wire bytes.
    pub control: Vec<u8>,
}

/// What `revoke` makes: post the statement to `…/revocations`, the control to `…/messages`.
#[derive(Debug, Clone)]
pub struct Revocation {
    /// `[grant_id, COSE_Sign1 by the seat under cahoots/revoke/v1 over
    /// {0: channel_id, 1: grant_id, 2: reason}]`.
    pub statement: Vec<u8>,
    /// The revocation control message, wire bytes.
    pub control: Vec<u8>,
}

/// One approval of a proposed announcement by a seat of the owner circle, for a channel whose
/// owner runs on another device: as [`ChannelOwner::approve_post`], under `policy`, the
/// channel's current signed policy, which must be signed by a seat of `manifest`'s circle.
/// `digest` is the announcement's 32-byte digest. Errors: `manifest`, `seat`, `malformed`,
/// `signature`, `wrong-circle`, `digest`.
#[allow(clippy::too_many_arguments)]
pub fn approve_channel_post<S: SeatSigner>(
    manifest: &[u8],
    seat: u8,
    mut signer: S,
    policy: &[u8],
    digest: &[u8],
    channel_name: &str,
    first_line: &str,
    log: u64,
) -> Result<Vec<u8>, String> {
    let manifest = open_manifest(manifest)?;
    if manifest.seat_key(seat) != Some(&signer.public()) {
        return Err(s("seat"));
    }
    Policy::verify(policy, &manifest).map_err(code)?;
    let digest: [u8; 32] = digest.try_into().map_err(|_| s("digest"))?;
    Ok(approval_for(
        signer,
        seat,
        &manifest.circle_id,
        PROFILE_POST,
        vec![
            (s("channel"), channel_name.to_string()),
            (s("text"), first_line.to_string()),
        ],
        digest,
        &policy::id(policy),
        log,
        assurance::PRESENT,
        random::<16>(),
    ))
}

/// A channel run from one seat of its owner circle.
pub struct ChannelOwner {
    circle_id: CircleId,
    channel_id: ChannelId,
    seat: u8,
    /// The seat key's seed, when this owner keeps it (the browser). `None` on a device whose
    /// seat key never leaves its key store.
    seat_seed: Option<Zeroizing<[u8; 32]>>,
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

impl core::fmt::Debug for ChannelOwner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelOwner")
            .field("channel_id", &self.channel_id)
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

impl ChannelOwner {
    /// Start a channel owned by the circle of `manifest` (its encoded `SealedManifest`),
    /// signed by `signer`, the key of `seat`.
    ///
    /// `preset`: 0 announcements, 1 announcements with approvals (a majority of `seats_hint`),
    /// 2 group chat, 3 members post, 4 closed. `seats_hint` is the number of seats the
    /// `govern` quorum is sized by (1 for a quorum of 1); 0 means every seat of the circle.
    /// Every preset but closed makes the link secret at once, since the policy names it.
    ///
    /// The owner's own member key gets a root grant for all four actions (`self_grant`).
    /// No epoch exists until `rotate`. Errors: `manifest`, `seat`, `name`, `preset`.
    pub fn start<S: SeatSigner>(
        manifest: &[u8],
        seat: u8,
        signer: S,
        name: &str,
        preset: u8,
        seats_hint: u8,
    ) -> Result<ChannelOwner, String> {
        Self::make(manifest, seat, signer, None, name, preset, seats_hint)
    }

    /// `start` with the seat key's seed, which the owner then keeps and exports.
    /// Errors: `manifest`, `seat`, `name`, `preset`.
    pub fn start_with_seed(
        manifest: &[u8],
        seat: u8,
        seed: &[u8; 32],
        name: &str,
        preset: u8,
        seats_hint: u8,
    ) -> Result<ChannelOwner, String> {
        open_manifest(manifest)?;
        let key = SeatKey::from_seed(seed).map_err(|_| s("seat"))?;
        Self::make(
            manifest,
            seat,
            &key,
            Some(Zeroizing::new(*seed)),
            name,
            preset,
            seats_hint,
        )
    }

    fn make<S: SeatSigner>(
        manifest_bytes: &[u8],
        seat: u8,
        mut signer: S,
        seat_seed: Option<Zeroizing<[u8; 32]>>,
        name: &str,
        preset: u8,
        seats_hint: u8,
    ) -> Result<ChannelOwner, String> {
        let manifest = open_manifest(manifest_bytes)?;
        if manifest.seat_key(seat) != Some(&signer.public()) {
            return Err(s("seat"));
        }
        check_name(name)?;
        let seats = if seats_hint == 0 {
            manifest.params.n
        } else {
            seats_hint
        };
        let preset = preset_of(preset, majority(seats))?;
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
        .sign(&mut signer, seat);
        let object = ChannelObject {
            circle_id,
            channel_id,
            name: name.to_string(),
            created_log: 0,
            policy_id: policy::id(&policy),
            nonce,
        }
        .sign(&mut signer, seat);
        let mut member = ChannelMember::new(channel_id, String::new(), secret.clone());
        let claims = grant::issue_root(
            &circle_id,
            &channel_id,
            seat,
            &member.public(),
            &[Action::Read, Action::Post, Action::Admit, Action::Govern],
            0,
            u64::MAX,
            2,
            random::<16>(),
        );
        let self_grant = grant::sign_root(&claims, &mut signer, seat);
        member.grant = vec![self_grant.clone()];
        member.policy = Some(policy.clone());
        Ok(ChannelOwner {
            circle_id,
            channel_id,
            seat,
            seat_seed,
            manifest: manifest_bytes.to_vec(),
            seats,
            object,
            policy,
            secret,
            epoch: 0,
            epoch_key: None,
            deposits: Vec::new(),
            members: Vec::new(),
            self_grant,
            member,
        })
    }

    /// The seat key from the seed this owner keeps; `None` when it keeps none.
    pub fn seat_key(&self) -> Option<SeatKey> {
        self.seat_seed
            .as_ref()
            .map(|seed| SeatKey::from_seed(seed).expect("the seed was checked when it was loaded"))
    }

    /// Error `seat` unless `signer` is the manifest's key for this owner's seat.
    fn check_signer<S: SeatSigner>(&self, signer: &mut S) -> Result<(), String> {
        let manifest = open_manifest(&self.manifest)?;
        if manifest.seat_key(self.seat) != Some(&signer.public()) {
            return Err(s("seat"));
        }
        Ok(())
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
            self.member.secret = Some(secret.clone());
            self.secret = Some(secret);
        }
        self.secret.as_ref().expect("made above")
    }

    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub fn circle_id(&self) -> CircleId {
        self.circle_id
    }

    /// The seat this owner signs as.
    pub fn seat(&self) -> u8 {
        self.seat
    }

    /// The signed channel object, the body of `POST /v1/channels`.
    pub fn channel_object(&self) -> &[u8] {
        &self.object
    }

    /// The current signed policy.
    pub fn policy(&self) -> &[u8] {
        &self.policy
    }

    /// `[policy, []]`: the body of `POST …/policy` for the first policy, which needs no
    /// approvals.
    pub fn policy_body(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        let _ = e.array(2).and_then(|e| e.bytes(&self.policy));
        let _ = e.array(0);
        e.into_writer()
    }

    /// SHA-256 of the current signed policy.
    pub fn policy_id(&self) -> [u8; 32] {
        policy::id(&self.policy)
    }

    /// The owner circle's encoded `SealedManifest`: the body of `PUT` manifest, and what
    /// members pass to `receive`.
    pub fn manifest(&self) -> &[u8] {
        &self.manifest
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
        self.member.relay = relay_host.to_string();
        Ok(url)
    }

    /// The seat-signed `cahoots/gate/v1` statement `{0: channel_id, 1: gate_token}` for the
    /// current link: the body of `POST …/gates`. Errors: `no-link`, `seat`.
    pub fn gate_registration<S: SeatSigner>(&self, mut signer: S) -> Result<Vec<u8>, String> {
        let token = self.link_secret()?.gate_token(&self.channel_id);
        self.check_signer(&mut signer)?;
        Ok(statement(
            signer,
            self.seat,
            GATE_DOMAIN,
            &self.channel_id,
            &token,
            None,
        ))
    }

    /// SHA-256 of the current link's gate token: the path of `DELETE …/gates/{hash}`. Read it
    /// before `retire_link`. `None` without a link.
    pub fn gate_token_hash(&self) -> Option<[u8; 32]> {
        let token = self.secret.as_ref()?.gate_token(&self.channel_id);
        Some(Sha256::digest(token).into())
    }

    /// The seat-signed `cahoots/gate-retire/v1` statement for the current link (the body of
    /// `DELETE …/gates/{hash}`), and forget its secret: later rotations make no link deposit.
    /// Errors: `no-link`, `seat`.
    pub fn retire_link<S: SeatSigner>(&mut self, mut signer: S) -> Result<Vec<u8>, String> {
        let token = self.link_secret()?.gate_token(&self.channel_id);
        self.check_signer(&mut signer)?;
        let signed = statement(
            signer,
            self.seat,
            GATE_RETIRE_DOMAIN,
            &self.channel_id,
            &token,
            None,
        );
        if let Some(mut secret) = self.secret.take() {
            secret.0.zeroize();
        }
        if let Some(mut secret) = self.member.secret.take() {
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
                deposit::seal_for_key(&key, &id, m.kid, &m.xwing_pk, &mut Rng10).map_err(code)?,
            );
        }
        let control = Control::Rotation {
            epoch_id: id,
            deposits: deposits.iter().map(Deposit::id).collect(),
        };
        let wire = if self.epoch_key.is_some() {
            let wire = self.member.send_control(&control)?;
            self.member.install(id, Zeroizing::new(*key))?;
            wire
        } else {
            self.member.install(id, Zeroizing::new(*key))?;
            self.member.send_control(&control)?
        };
        self.epoch = next;
        self.epoch_key = Some(key);
        self.deposits = deposits.iter().map(Deposit::encode).collect();
        Ok(wire)
    }

    /// The deposits made for the current epoch, each the body of one `POST …/deposits`: one
    /// for the live link, then one per admitted member.
    pub fn deposits(&self) -> &[Vec<u8>] {
        &self.deposits
    }

    /// A key deposit of the current epoch for one more key (`kid` its signing key's id),
    /// outside the admitted list: it is not repeated at the next rotation.
    /// Errors: `unknown-epoch`, `hpke`.
    pub fn deposit_for_key(&self, kid: &[u8; 32], xwing_pk: &[u8]) -> Result<Vec<u8>, String> {
        let (id, key) = self.current()?;
        let d = deposit::seal_for_key(key, &id, *kid, xwing_pk, &mut Rng10).map_err(code)?;
        Ok(d.encode())
    }

    /// Admit a join request (as `ChannelMember::join_request` makes it), granting `actions`
    /// ("read,post"), which must be among those requested. For each granted action whose rule
    /// names a link, the request's gate token must be the live link's.
    ///
    /// Issues a root grant (depth 0, no budget, `not_after = u64::MAX`), seals the current
    /// epoch to the member's X-Wing key, keeps the member for later rotations, and returns
    /// the chain, the deposit and the join control.
    /// A request for `post` is refused with `policy` when the post rule has no evidence (only
    /// the owner's seats post, as in an announcements channel).
    /// Errors: `malformed`, `signature`, `wrong-channel`, `actions`, `name`, `policy`,
    /// `evidence`, `unknown-epoch`, `hpke`, `seat`.
    pub fn admit<S: SeatSigner>(
        &mut self,
        request: &[u8],
        actions: &str,
        mut signer: S,
    ) -> Result<Admission, String> {
        let JoinRequest {
            channel,
            member_key,
            xwing_pk,
            wanted,
            name,
            token,
            kid,
        } = JoinRequest::parse(request)?;
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
                    // An empty ref is the owner's current link; a named one must be it.
                    .any(|e| {
                        e.kind == kind::LINK && (e.r#ref.is_empty() || e.r#ref[..] == sec.id())
                    })
                    && token[..] == sec.gate_token(&self.channel_id)
            });
            if !shown {
                return Err(s("evidence"));
            }
        }
        let (id, epoch_key) = self.current()?;
        let deposit =
            deposit::seal_for_key(epoch_key, &id, kid, &xwing_pk, &mut Rng10).map_err(code)?;
        self.check_signer(&mut signer)?;
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
        let signed = grant::sign_root(&claims, signer, self.seat);
        let wire = self.member.send_control(&Control::Join {
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
        Ok(Admission {
            grant_id,
            kid,
            chain: bytes_array(&[&signed]),
            deposit: deposit.encode(),
            control: wire,
        })
    }

    /// The admitted members, in the order they were admitted.
    pub fn members(&self) -> &[Admitted] {
        &self.members
    }

    /// Revoke a grant: the revocation statement and the revocation control message. The
    /// member is no longer sealed to; `rotate` (and `retire_link` if the member holds the
    /// link) then shuts it out. Errors: `unknown-epoch`, `seat`.
    pub fn revoke<S: SeatSigner>(
        &mut self,
        grant_id: &[u8; 16],
        reason: &str,
        mut signer: S,
    ) -> Result<Revocation, String> {
        self.check_signer(&mut signer)?;
        let signed = statement(
            signer,
            self.seat,
            REVOKE_DOMAIN,
            &self.channel_id,
            grant_id,
            Some(reason),
        );
        let body = bytes_array(&[grant_id, &signed]);
        let wire = self.member.send_control(&Control::Revocation {
            grant_id: *grant_id,
            reason: reason.to_string(),
        })?;
        self.members.retain(|m| m.grant_id != *grant_id);
        if !self.member.revoked.contains(grant_id) {
            self.member.revoked.push(*grant_id);
        }
        Ok(Revocation {
            statement: body,
            control: wire,
        })
    }

    /// Replace the policy with a preset (numbers as for `start`; `quorum` is the approvals
    /// each announcement needs, read for preset 1 only). Makes a link secret if the preset
    /// admits by link and there is none. Returns `[signed policy, [approvals]]`, the body of
    /// `POST …/policy`.
    ///
    /// It also reissues the channel object: a new signed object with `policy_id` the new
    /// policy's id and `created_log` the new policy's `log` (same name and nonce, so the same
    /// channel id), which `channel_object()` then returns. Post the policy, then the channel
    /// object, then `policy_change_control()`.
    ///
    /// In this version the approvals are always empty: when the current policy's `govern`
    /// rule needs more than one seat, this refuses with `needs-approvals`.
    /// Errors: `preset`, `quorum`, `needs-approvals`, `malformed`, `seat`.
    pub fn set_policy<S: SeatSigner>(
        &mut self,
        preset: u8,
        quorum: u8,
        mut signer: S,
    ) -> Result<Vec<u8>, String> {
        let preset = preset_of(preset, quorum)?;
        let current = read_policy(&self.policy)?;
        if current.rule(Action::Govern).is_some_and(|r| r.quorum > 1) {
            return Err(s("needs-approvals"));
        }
        self.check_signer(&mut signer)?;
        let payload = cose::payload1(&self.object).map_err(|_| s("malformed"))?;
        let mut object = ChannelObject::decode(&payload).map_err(code)?;
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
        let signed = next.sign(&mut signer, self.seat);
        object.policy_id = policy::id(&signed);
        object.created_log = next.log;
        self.object = object.sign(&mut signer, self.seat);
        self.policy = signed.clone();
        self.member.policy = Some(signed.clone());
        let mut e = Encoder::new(Vec::new());
        let _ = e.array(2).and_then(|e| e.bytes(&signed));
        let _ = e.array(0);
        Ok(e.into_writer())
    }

    /// This seat's approval of an announcement, the signed bytes to put in the array given to
    /// `ChannelMember::send_proposed`: `profile` `channel-post`, `request_digest` the
    /// proposal's `announcement_digest` (`digest`, 32 bytes), fields `[["channel",
    /// channel_name], ["text", first_line]]`, the current policy's id, `log` the channel
    /// position the proposer named, assurance present (2), a fresh nonce.
    /// Errors: `digest`, `seat`.
    pub fn approve_post<S: SeatSigner>(
        &self,
        digest: &[u8],
        channel_name: &str,
        first_line: &str,
        log: u64,
        mut signer: S,
    ) -> Result<Vec<u8>, String> {
        let digest: [u8; 32] = digest.try_into().map_err(|_| s("digest"))?;
        self.check_signer(&mut signer)?;
        Ok(approval_for(
            signer,
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
        self.member
            .send_control(&Control::PolicyChange { policy_id })
    }

    /// The owner's own member, under the self-grant. It sends and reads like any member;
    /// exporting the owner exports it.
    pub fn member(&self) -> &ChannelMember {
        &self.member
    }

    pub fn member_mut(&mut self) -> &mut ChannelMember {
        &mut self.member
    }

    /// The whole owner as canonical CBOR, secrets included (the seat key's seed when kept,
    /// the link secret, the current epoch key, the owner member's keys): store it under the
    /// app's own protection.
    ///
    /// `{0: 1, 1: seat, 2: seat seed or nil, 3: manifest, 4: seats, 5: channel object,
    ///   6: policy, 7: link secret or nil, 8: epoch number, 9: epoch key or nil,
    ///   10: [deposit...], 11: [[grant_id, kid, xwing_pk, name tstr]...], 12: self grant,
    ///   13: member state}`
    pub fn export(&self) -> Zeroizing<Vec<u8>> {
        let member = self.member.export();
        let mut e = Encoder::new(SecretBuf(Zeroizing::new(Vec::with_capacity(
            member.len() + self.manifest.len() + 8192,
        ))));
        let _ = (|| -> Result<(), minicbor::encode::Error<Infallible>> {
            let e = &mut e;
            e.map(14)?.u8(0)?.u8(STATE_VERSION)?;
            e.u8(1)?.u8(self.seat)?;
            e.u8(2)?;
            opt_bytes(e, self.seat_seed.as_ref().map(|s| &s[..]))?;
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
    /// a kept seat seed against the manifest's seat key. Error: `state`.
    pub fn import(state: &[u8]) -> Result<ChannelOwner, String> {
        let bad = || s(STATE);
        let mut r = StateReader::open(state, 14)?;
        let seat = r.key()?.u8().map_err(|_| bad())?;
        let seat_seed = match r.opt_bytes()? {
            Some(b) => Some(Zeroizing::new(<[u8; 32]>::try_from(b).map_err(|_| bad())?)),
            None => None,
        };
        let manifest_bytes = r.bytes()?.to_vec();
        let manifest = open_manifest(&manifest_bytes).map_err(|_| bad())?;
        if let Some(seed) = &seat_seed {
            let key = SeatKey::from_seed(seed).map_err(|_| bad())?;
            if manifest.seat_key(seat) != Some(&key.public()) {
                return Err(bad());
            }
        } else if manifest.seat_key(seat).is_none() {
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
        let member = ChannelMember::import(r.bytes()?)?;
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
            member,
        };
        if *owner.export() != *state {
            return Err(bad());
        }
        Ok(owner)
    }
}
