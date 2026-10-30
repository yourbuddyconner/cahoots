//! The policy (channels spec §3): who may read, post, admit and govern a channel, and what each
//! needs.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16 |
//! | 2 | channel_id bstr 16 |
//! | 3 | rules: array of rule maps, in ascending action order, at most one per action |
//! | 4 | issuers: array of issuer maps |
//! | 5 | log uint, the channel log position at which the policy takes effect |
//! | 6 | parent bstr 32 or nil, the previous policy's id |
//! | 7 | nonce bstr 16 |
//!
//! A rule is `{0: action, 1: evidence, 2: per_evidence, 3: quorum, 4: assurance}`, an evidence
//! map `{0: kind, 1: ref, 2: scope}`, and an issuer map `{0: key, 1: endpoint, 2: measurement}`.
//!
//! Signed by a seat of the owner circle with kid `[seat]`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use cahoots_core::CircleId;
use cahoots_objects::assurance;
use minicbor::Decoder;
use sha2::{Digest, Sha256};

use crate::cbor::{array_len, canonical, encode_with, Enc, EncResult, Nested, Reader, VERSION};
use crate::{object, open_seat_signed, Action, ChannelError, ChannelId};

/// External AAD of a policy's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/policy/v1";

const M: ChannelError = ChannelError::Malformed;

/// Evidence kinds. Kinds 0 and 1 are built in this version; 2 to 4 are defined so the format
/// does not change when they arrive.
pub mod kind {
    /// `ref` is a link id, bstr 32.
    pub const LINK: u8 = 0;
    /// `ref` is one explicit key, bstr 33.
    pub const KEY: u8 = 1;
    /// `ref` is a manifest id, bstr 32.
    pub const CIRCLE: u8 = 2;
    /// `ref` is a verifier's registry root; `scope` is the nullifier scope.
    pub const DOCUMENT: u8 = 3;
    /// `ref` is an enclave measurement, bstr 32; `scope` names the issuer and the predicate.
    pub const ACCOUNT: u8 = 4;
}

/// What a key shows to earn a grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    /// One of [`kind`].
    pub kind: u8,
    pub r#ref: Vec<u8>,
    /// Empty for the kinds that do not use it (link, key, circle).
    pub scope: String,
}

/// One action's rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub action: Action,
    /// A key that satisfies any one of these may be granted the action. Empty: nobody but the
    /// owner's seats.
    pub evidence: Vec<Evidence>,
    /// How many member keys one piece of evidence may back, 0 for no limit.
    pub per_evidence: u32,
    /// How many seat approvals each use of the action needs; 0 means the grant alone suffices.
    pub quorum: u8,
    /// The minimum assurance of those approvals, one of the `cahoots_objects::assurance` levels.
    pub assurance: u8,
}

/// A key that may issue grants and deposits for the channel. The owner's seats always may and are
/// not listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issuer {
    pub key: SeatPublic,
    /// Where to send a join request.
    pub endpoint: String,
    /// The enclave measurement when the issuer is one.
    pub measurement: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub circle_id: CircleId,
    pub channel_id: ChannelId,
    /// In ascending action order, at most one per action. A missing action has no evidence.
    pub rules: Vec<Rule>,
    pub issuers: Vec<Issuer>,
    pub log: u64,
    pub parent: Option<[u8; 32]>,
    pub nonce: [u8; 16],
}

/// The policies the editor offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Read by link, post by the owner.
    Announcements,
    /// As announcements, with each post needing this many seat approvals.
    AnnouncementsQuorum(u8),
    /// Read and post by link.
    GroupChat,
    /// Read by link, post by key once the owner admits it.
    MembersPost,
    /// Read by key only.
    Closed,
}

/// The id of a policy: SHA-256 of its signed bytes.
pub fn id(signed: &[u8]) -> [u8; 32] {
    Sha256::digest(signed).into()
}

impl Policy {
    /// A preset's policy. Every action has a rule; evidence is the link where the preset admits
    /// by link; `govern` needs a majority of `seats` (1 when there is one seat); assurance is
    /// present (2) throughout.
    pub fn preset(
        preset: Preset,
        circle_id: CircleId,
        channel_id: ChannelId,
        link_id: [u8; 32],
        seats: u8,
        log: u64,
        nonce: [u8; 16],
    ) -> Policy {
        let link = || {
            vec![Evidence {
                kind: kind::LINK,
                r#ref: link_id.to_vec(),
                scope: String::new(),
            }]
        };
        let rule = |action, evidence, quorum| Rule {
            action,
            evidence,
            per_evidence: 0,
            quorum,
            assurance: assurance::PRESENT,
        };
        let (read, post, post_quorum) = match preset {
            Preset::Announcements => (link(), Vec::new(), 0),
            Preset::AnnouncementsQuorum(k) => (link(), Vec::new(), k),
            Preset::GroupChat => (link(), link(), 0),
            Preset::MembersPost => (link(), Vec::new(), 0),
            Preset::Closed => (Vec::new(), Vec::new(), 0),
        };
        let govern_quorum = if seats == 1 { 1 } else { seats / 2 + 1 };
        Policy {
            circle_id,
            channel_id,
            rules: vec![
                rule(Action::Read, read, 0),
                rule(Action::Post, post, post_quorum),
                rule(Action::Admit, Vec::new(), 0),
                rule(Action::Govern, Vec::new(), govern_quorum),
            ],
            issuers: Vec::new(),
            log,
            parent: None,
            nonce,
        }
    }

    /// The rule for `action`, or `None` when the policy has none (nobody but the owner's seats).
    pub fn rule(&self, action: Action) -> Option<&Rule> {
        self.rules.iter().find(|r| r.action == action)
    }

    /// The canonical CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(8)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.bytes(&self.channel_id)?;
        e.u8(3)?.array(self.rules.len() as u64)?;
        for rule in &self.rules {
            e.map(5)?;
            e.u8(0)?.u8(rule.action as u8)?;
            e.u8(1)?.array(rule.evidence.len() as u64)?;
            for ev in &rule.evidence {
                e.map(3)?;
                e.u8(0)?.u8(ev.kind)?;
                e.u8(1)?.bytes(&ev.r#ref)?;
                e.u8(2)?.str(&ev.scope)?;
            }
            e.u8(2)?.u32(rule.per_evidence)?;
            e.u8(3)?.u8(rule.quorum)?;
            e.u8(4)?.u8(rule.assurance)?;
        }
        e.u8(4)?.array(self.issuers.len() as u64)?;
        for issuer in &self.issuers {
            e.map(3)?;
            e.u8(0)?.bytes(&issuer.key.0)?;
            e.u8(1)?.str(&issuer.endpoint)?;
            e.u8(2)?;
            match &issuer.measurement {
                Some(m) => e.bytes(m)?,
                None => e.null()?,
            };
        }
        e.u8(5)?.u64(self.log)?;
        e.u8(6)?;
        match &self.parent {
            Some(p) => e.bytes(p)?,
            None => e.null()?,
        };
        e.u8(7)?.bytes(&self.nonce)?;
        Ok(())
    }

    /// Parse a canonical payload. Refuses rules out of action order or repeated, an unknown
    /// evidence kind, a `ref` of the wrong length for its kind, a scope on a kind that does not
    /// use it, an assurance above present-with-PIN, and an issuer key that is not a compressed
    /// P-256 point.
    pub fn decode(bytes: &[u8]) -> Result<Policy, ChannelError> {
        let mut r = Reader::open(bytes, 8)?;
        let circle_id = r.array()?;
        let channel_id = r.array()?;
        let rules = {
            let d = r.key()?;
            let n = array_len(d)?;
            let mut rules: Vec<Rule> = Vec::new();
            for _ in 0..n {
                let rule = decode_rule(d)?;
                if rules.last().is_some_and(|last| last.action >= rule.action) {
                    return Err(M);
                }
                rules.push(rule);
            }
            rules
        };
        let issuers = {
            let d = r.key()?;
            let n = array_len(d)?;
            let mut issuers = Vec::new();
            for _ in 0..n {
                issuers.push(decode_issuer(d)?);
            }
            issuers
        };
        let policy = Policy {
            circle_id,
            channel_id,
            rules,
            issuers,
            log: r.u64()?,
            parent: r.opt_digest()?,
            nonce: r.array()?,
        };
        r.finish()?;
        let encoded = policy.encode();
        canonical(policy, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded; the kid is `[seat]`.
    pub fn sign<S: SeatSigner>(&self, key: S, seat: u8) -> Vec<u8> {
        cose::sign1(key, &[seat], &self.encode(), DOMAIN, false)
    }

    /// Verify a signed policy against the owner circle's manifest: the kid names a seat, that
    /// seat's key signed it, the payload is canonical, and the circle is the manifest's. Returns
    /// the signing seat.
    pub fn verify(bytes: &[u8], manifest: &Manifest) -> Result<(u8, Policy), ChannelError> {
        let (seat, payload) = open_seat_signed(bytes, manifest, DOMAIN)?;
        let policy = Policy::decode(&payload)?;
        if policy.circle_id != manifest.circle_id {
            return Err(ChannelError::WrongCircle);
        }
        Ok((seat, policy))
    }
}

fn decode_rule(d: &mut Decoder<'_>) -> Result<Rule, ChannelError> {
    let mut m = Nested::open(d, 5)?;
    let action = Action::from_u8(m.u8()?).ok_or(M)?;
    let evidence = {
        let d = m.key()?;
        let n = array_len(d)?;
        let mut list = Vec::new();
        for _ in 0..n {
            list.push(decode_evidence(d)?);
        }
        list
    };
    let rule = Rule {
        action,
        evidence,
        per_evidence: m.u32()?,
        quorum: m.u8()?,
        assurance: m.u8()?,
    };
    m.finish()?;
    if rule.assurance > assurance::PRESENT_PIN {
        return Err(M);
    }
    Ok(rule)
}

fn decode_evidence(d: &mut Decoder<'_>) -> Result<Evidence, ChannelError> {
    let mut m = Nested::open(d, 3)?;
    let ev = Evidence {
        kind: m.u8()?,
        r#ref: m.bytes()?.to_vec(),
        scope: m.string()?,
    };
    m.finish()?;
    let ref_ok = match ev.kind {
        kind::LINK | kind::CIRCLE | kind::ACCOUNT => ev.r#ref.len() == 32,
        kind::KEY => ev.r#ref.len() == 33,
        kind::DOCUMENT => true,
        _ => return Err(M),
    };
    let scoped = matches!(ev.kind, kind::DOCUMENT | kind::ACCOUNT);
    if !ref_ok || (!scoped && !ev.scope.is_empty()) {
        return Err(M);
    }
    Ok(ev)
}

fn decode_issuer(d: &mut Decoder<'_>) -> Result<Issuer, ChannelError> {
    let mut m = Nested::open(d, 3)?;
    let issuer = Issuer {
        key: SeatPublic::parse(m.bytes()?).map_err(|_| M)?,
        endpoint: m.string()?,
        measurement: m.opt_digest()?,
    };
    m.finish()?;
    Ok(issuer)
}

/// Verify a governed act (a policy or a channel object, signed under its own domain) against the
/// rule that governs it: the signer is a seat of `manifest`, and the approvals for
/// `SHA-256(signed)` under `profile` meet `rule.quorum`. Returns the payload.
///
/// The signing seat counts as one approval. Approval objects are not yet checked: a rule whose
/// quorum is above 1 is refused.
pub fn verify_governed(
    signed: &[u8],
    approvals: &[Vec<u8>],
    manifest: &Manifest,
    rule: &Rule,
    profile: &str,
) -> Result<Vec<u8>, ChannelError> {
    let (_seat, payload) = open_seat_signed(signed, manifest, DOMAIN)
        .or_else(|_| open_seat_signed(signed, manifest, object::DOMAIN))?;
    let _ = (approvals, profile);
    let have = 1;
    if usize::from(rule.quorum) > have {
        return Err(ChannelError::Quorum {
            have,
            need: rule.quorum,
        });
    }
    Ok(payload)
}
