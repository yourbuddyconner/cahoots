//! Channel grants (spec §6): a version 2 grant with one `cahoots:channel` detail that lets a key
//! read, post, admit or govern in one channel, up to a post budget and a position in the
//! channel's own log.
//!
//! The chain rules are those of [`cahoots_objects::grant::verify_chain`]: a root grant is
//! signed by a seat under kid `[seat]`, a delegated one by the parent's grantee under kid
//! [`key_id`] of it, and every link narrows. A link may delegate only if it carries
//! [`Action::Admit`]. [`evaluate`] then decides one request against the leaf.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use cahoots_core::CircleId;
use cahoots_objects::cwt::{self, Claims, Detail, Kind, CHANNEL_TYPE};
use cahoots_objects::grant::{key_id, Grantor, DOMAIN};
use cahoots_objects::GrantError;
use sha2::{Digest, Sha256};

use crate::{Action, ChannelError, ChannelId};

/// Why a channel grant does not allow a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A grant in the chain has been revoked.
    Revoked,
    /// The grant is for another channel.
    WrongChannel,
    /// The grant does not carry the action.
    Action,
    /// The channel head is at or past the grant's `not_after`.
    Expired,
    /// The grant's post budget is spent.
    Budget,
    /// The signer is not the grantee.
    Key,
}

/// A channel grant, read from its claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelGrant {
    pub claims: Claims,
    pub channel_id: ChannelId,
    /// Each action once, in number order.
    pub actions: Vec<Action>,
    /// Posts the grant allows; 0 is no limit.
    pub budget: u32,
    /// The `cti`: the grant's id for the relay and for revocation.
    pub id: [u8; 16],
    /// The key that may use the grant (`cnf`), and sign a delegated one.
    pub grantee: SeatPublic,
    /// The channel log position at which the grant stops holding.
    pub not_after: u64,
    /// Delegations still allowed below this grant.
    pub depth: u8,
}

/// The channel grant `c` holds. `Object(e)` if the claims are not a well-formed grant (see
/// [`Claims::kind`]), `Malformed` if they are a grant of another kind.
pub fn from_claims(c: &Claims) -> Result<ChannelGrant, ChannelError> {
    let (_, kind) = c.kind().map_err(ChannelError::Object)?;
    let Kind::Channel {
        channel,
        actions,
        budget,
    } = kind
    else {
        return Err(ChannelError::Malformed);
    };
    let actions = actions
        .iter()
        .map(|a| Action::parse(a))
        .collect::<Option<Vec<_>>>()
        .ok_or(ChannelError::Malformed)?;
    Ok(ChannelGrant {
        claims: c.clone(),
        channel_id: channel,
        actions,
        budget,
        id: c.cti,
        grantee: c.cnf,
        not_after: c.log_not_after.ok_or(ChannelError::Malformed)?,
        depth: c.depth,
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| [DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 15)]])
        .map(char::from)
        .collect()
}

/// The one detail. Actions are written once each, in number order, as [`Claims::kind`] needs.
fn detail(channel_id: &ChannelId, actions: &[Action], budget: u32) -> Detail {
    let mut actions = actions.to_vec();
    actions.sort();
    actions.dedup();
    Detail {
        r#type: String::from(CHANNEL_TYPE),
        budget: Some(budget),
        actions: Some(actions.iter().map(|a| String::from(a.name())).collect()),
        channel: Some(hex(channel_id)),
        ..Detail::default()
    }
}

/// The claims of a root grant from seat `grantor_seat` of the circle to `grantee`.
#[allow(clippy::too_many_arguments)]
pub fn issue_root(
    circle_id: &CircleId,
    channel_id: &ChannelId,
    grantor_seat: u8,
    grantee: &SeatPublic,
    actions: &[Action],
    budget: u32,
    not_after: u64,
    depth: u8,
    nonce: [u8; 16],
) -> Claims {
    Claims {
        iss: cwt::issuer(circle_id),
        sub: cwt::subject(grantee),
        aud: None,
        exp: None,
        cnf: *grantee,
        cti: nonce,
        authorization_details: alloc::vec![detail(channel_id, actions, budget)],
        log_not_after: Some(not_after),
        parent: None,
        depth,
        grantor: Grantor::Seat(grantor_seat),
        base: None,
    }
}

/// The claims of a grant delegated by `parent`'s grantee, whose signed bytes are
/// `parent_signed`. Its depth is one less than the parent's; a parent of depth 0 gives claims
/// of depth 0 that [`verify_chain`] refuses.
pub fn issue_delegated(
    parent_signed: &[u8],
    parent: &ChannelGrant,
    grantee: &SeatPublic,
    actions: &[Action],
    budget: u32,
    not_after: u64,
    nonce: [u8; 16],
) -> Claims {
    Claims {
        iss: parent.claims.iss.clone(),
        sub: cwt::subject(grantee),
        aud: None,
        exp: None,
        cnf: *grantee,
        cti: nonce,
        authorization_details: alloc::vec![detail(&parent.channel_id, actions, budget)],
        log_not_after: Some(not_after),
        parent: Some(Sha256::digest(parent_signed).into()),
        depth: parent.depth.saturating_sub(1),
        grantor: Grantor::Key(parent.grantee),
        base: None,
    }
}

/// Sign a root grant as seat `seat`: COSE_Sign1 under [`DOMAIN`], kid `[seat]`.
pub fn sign_root<S: SeatSigner>(claims: &Claims, key: S, seat: u8) -> Vec<u8> {
    cose::sign1(key, &[seat], &claims.encode(), DOMAIN, false)
}

/// Sign a delegated grant with the parent's grantee key: kid [`key_id`] of that key.
pub fn sign_delegated<S: SeatSigner>(claims: &Claims, mut key: S) -> Vec<u8> {
    let kid = key_id(&key.public());
    cose::sign1(key, &kid, &claims.encode(), DOMAIN, false)
}

/// Verify a chain of signed channel grants, root first, against the manifest, and return the
/// leaf. Every link must name `channel_id`.
///
/// The root is signed by a seat of the manifest under kid `[seat]`, names that seat as its
/// grantor, has no parent, and its `iss` is the manifest's circle. Each later link is signed by
/// the previous grantee under kid [`key_id`] of it, names it as grantor, carries as `parent` the
/// SHA-256 of the previous link's exact signed bytes, has the same `iss`, and narrows: its
/// actions a subset, its budget no greater (0 is no limit, greater than any other), its
/// `not_after` no greater, its depth exactly one less than a nonzero previous depth. A link may
/// have a child only if it carries [`Action::Admit`] ([`ChannelError::NoAdmit`]).
pub fn verify_chain(
    chain: &[&[u8]],
    manifest: &Manifest,
    channel_id: &ChannelId,
) -> Result<ChannelGrant, ChannelError> {
    let g = ChannelError::Grant;
    let (root_bytes, rest) = chain.split_first().ok_or(g(GrantError::EmptyChain))?;
    let kid = cose::kid1(root_bytes).map_err(ChannelError::Cose)?;
    let [seat] = kid[..] else {
        return Err(g(GrantError::WrongKid));
    };
    let key = manifest
        .seat_key(seat)
        .ok_or(g(GrantError::NotInRoster(seat)))?;
    let mut prev = open(root_bytes, key)?;
    if prev.claims.grantor != Grantor::Seat(seat) {
        return Err(g(GrantError::WrongSigner));
    }
    if prev.claims.parent.is_some() {
        return Err(g(GrantError::BadParent));
    }
    if cwt::circle_of(&prev.claims.iss) != Some(manifest.circle_id) {
        return Err(g(GrantError::Circle));
    }
    if prev.channel_id != *channel_id {
        return Err(ChannelError::WrongChannel);
    }
    let mut prev_bytes: &[u8] = root_bytes;
    for bytes in rest {
        let signer = prev.grantee;
        if cose::kid1(bytes).map_err(ChannelError::Cose)? != key_id(&signer) {
            return Err(g(GrantError::WrongKid));
        }
        let link = open(bytes, &signer)?;
        if link.claims.grantor != Grantor::Key(signer) {
            return Err(g(GrantError::WrongSigner));
        }
        let parent: [u8; 32] = Sha256::digest(prev_bytes).into();
        if link.claims.parent != Some(parent) {
            return Err(g(GrantError::BadParent));
        }
        if link.claims.iss != prev.claims.iss {
            return Err(g(GrantError::Circle));
        }
        if link.channel_id != *channel_id {
            return Err(ChannelError::WrongChannel);
        }
        if !prev.actions.contains(&Action::Admit) {
            return Err(ChannelError::NoAdmit);
        }
        if !link.actions.iter().all(|a| prev.actions.contains(a)) {
            return Err(g(GrantError::ExtraPurpose));
        }
        if prev.budget != 0 && (link.budget == 0 || link.budget > prev.budget) {
            return Err(g(GrantError::LargerBudget));
        }
        if link.not_after > prev.not_after {
            return Err(g(GrantError::LaterNotAfter));
        }
        if prev.depth == 0 || link.depth != prev.depth - 1 {
            return Err(g(GrantError::Depth));
        }
        prev = link;
        prev_bytes = bytes;
    }
    Ok(prev)
}

/// The `cti` of every link of a chain, root first, read without checking signatures: call it
/// on a chain [`verify_chain`] accepted, and pass the ids to [`evaluate`] so that a revoked
/// parent ends its children.
pub fn chain_ids(chain: &[&[u8]]) -> Result<Vec<[u8; 16]>, ChannelError> {
    chain
        .iter()
        .map(|link| {
            let payload = cose::payload1(link).map_err(ChannelError::Cose)?;
            Ok(Claims::decode(&payload).map_err(ChannelError::Object)?.cti)
        })
        .collect()
}

fn open(bytes: &[u8], key: &SeatPublic) -> Result<ChannelGrant, ChannelError> {
    let payload = cose::verify1(key, bytes, None, DOMAIN).map_err(ChannelError::Cose)?;
    let claims = Claims::decode(&payload).map_err(ChannelError::Object)?;
    from_claims(&claims)
}

/// One request to check against a grant.
pub struct EvalRequest<'a> {
    pub action: Action,
    pub channel_id: &'a ChannelId,
    /// The channel head.
    pub log: u64,
    /// Posts already made under the grant.
    pub posts_used: u32,
    /// Whether a grant id has been revoked.
    pub revoked: &'a dyn Fn(&[u8; 16]) -> bool,
    /// The key that signed the request.
    pub signer: &'a SeatPublic,
}

/// Whether `grant`, the leaf of a chain whose grant ids are `chain_ids`, allows `req`. The
/// checks run in this order and the first that fails is the refusal: revoked (any id in the
/// chain), key, channel, action, expired, budget (posts only).
pub fn evaluate(
    grant: &ChannelGrant,
    chain_ids: &[[u8; 16]],
    req: &EvalRequest,
) -> Result<(), Refusal> {
    if chain_ids.iter().any(|id| (req.revoked)(id)) {
        return Err(Refusal::Revoked);
    }
    if *req.signer != grant.grantee {
        return Err(Refusal::Key);
    }
    if *req.channel_id != grant.channel_id {
        return Err(Refusal::WrongChannel);
    }
    if !grant.actions.contains(&req.action) {
        return Err(Refusal::Action);
    }
    if req.log >= grant.not_after {
        return Err(Refusal::Expired);
    }
    if req.action == Action::Post && grant.budget != 0 && req.posts_used >= grant.budget {
        return Err(Refusal::Budget);
    }
    Ok(())
}
