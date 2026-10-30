//! `cahoots-channel`: channels, a stream published under a circle that anyone holding a grant may
//! read or post to. `no_std` + `alloc`.
//!
//! - [`object::ChannelObject`]: names the channel and its current policy, signed by a seat.
//! - [`link`]: the invite link and the keys derived from its secret.
//! - [`keys`]: epoch ids, epoch keys and chain starts.
//! - [`policy::Policy`]: who may read, post, admit and govern, and what each needs.
//! - [`deposit`]: an epoch key sealed to a link or to a member's X-Wing key.
//! - [`message`]: the version 3 message, encrypted under a sender chain and signed by the sender.
//! - [`grant`]: channel grants, their chains, and the evaluator that decides a request.
//! - [`control`]: what a control message says: rotation, policy change, revocation, join, leave.
//!
//! Every object is a canonical CBOR map (definite lengths, shortest integers, integer keys in
//! ascending order) signed as a COSE_Sign1 with the payload embedded, kid `[seat]`, and the
//! object's domain string as external AAD.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod cbor;
pub mod control;
pub mod deposit;
pub mod grant;
pub mod keys;
pub mod link;
pub mod message;
pub mod object;
pub mod policy;

pub use grant::Refusal;
pub use object::ChannelObject;
pub use policy::{Evidence, Issuer, Policy, Preset, Rule};

use alloc::vec::Vec;
use cahoots_core::cose::{self, CoseError};
use cahoots_core::manifest::Manifest;
use cahoots_objects::{GrantError, ObjectError};

/// A channel's id: the first 16 bytes of `SHA-256("cahoots/channel-id/v1" || circle_id || nonce)`.
pub type ChannelId = [u8; 16];

/// What a rule governs. The number is the rule's `action` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    Read = 0,
    Post = 1,
    Admit = 2,
    /// Change the policy or the channel object, rotate, revoke.
    Govern = 3,
}

impl Action {
    /// Every action, in number order.
    const ALL: [Action; 4] = [Action::Read, Action::Post, Action::Admit, Action::Govern];

    pub fn name(self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Post => "post",
            Action::Admit => "admit",
            Action::Govern => "govern",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == s)
    }

    pub fn from_u8(n: u8) -> Option<Action> {
        Action::ALL.get(usize::from(n)).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// A signed object from `cahoots-objects` was refused, or a kid does not name a seat of the
    /// manifest ([`ObjectError::WrongKid`], [`ObjectError::NotInRoster`]).
    Object(ObjectError),
    /// A COSE_Sign1 does not parse or its signature does not verify.
    Cose(CoseError),
    /// A grant chain was refused.
    Grant(GrantError),
    /// Not the object: does not parse, a field missing, of the wrong type or length, out of range
    /// or out of order, an unknown version, or trailing bytes.
    Malformed,
    /// Parses, but is not the canonical encoding of what it holds.
    NotCanonical,
    /// A channel id that is not `channel_id_for(circle_id, nonce)`, or names another channel.
    WrongChannel,
    /// The object belongs to another circle than the manifest's.
    WrongCircle,
    /// A signature that does not verify.
    Signature,
    /// A ChaCha20-Poly1305 tag that does not verify: the wrong key, or changed bytes.
    Tag,
    /// No grant for the key.
    NoGrant,
    /// A grant chain in which a link has a child but does not carry `admit`.
    NoAdmit,
    /// A grant that does not allow the request.
    Refused(Refusal),
    /// Fewer approvals than the rule needs.
    Quorum { have: usize, need: u8 },
    /// No key for the epoch.
    UnknownEpoch,
    /// A message counter already used.
    Reuse,
    /// A message counter too far ahead of the chain.
    TooFarAhead,
    /// HPKE sealing or opening failed.
    Hpke,
}

/// Verify a COSE_Sign1 signed by a seat of `manifest` under kid `[seat]` and external AAD
/// `domain`. Returns the seat and the payload.
pub(crate) fn open_seat_signed(
    bytes: &[u8],
    manifest: &Manifest,
    domain: &[u8],
) -> Result<(u8, Vec<u8>), ChannelError> {
    let kid = cose::kid1(bytes).map_err(ChannelError::Cose)?;
    let [seat] = kid[..] else {
        return Err(ChannelError::Object(ObjectError::WrongKid));
    };
    let key = manifest
        .seat_key(seat)
        .ok_or(ChannelError::Object(ObjectError::NotInRoster(seat)))?;
    let payload = cose::verify1(key, bytes, None, domain).map_err(ChannelError::Cose)?;
    Ok((seat, payload))
}
