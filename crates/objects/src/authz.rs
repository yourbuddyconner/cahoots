//! The sign grant: a seat lets an agent's key sign commits, logins or files as a named author,
//! a fixed number of times. This module holds the grant, an evaluator that answers one
//! question (may this key do this, here, now) and the agent's proof that it holds the key.
//!
//! The evaluator takes its inputs in the shape of an AuthZEN access request: a subject, an
//! action, a resource and a context. The verifier supplies the context (the author named in
//! the commit, the time, how many signatures are used, whether the grant is revoked).

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use cahoots_core::CircleId;
use minicbor::{Decoder, Encoder};

use crate::cwt::{self, Claims, Detail, Kind, SIGN_TYPE};
use crate::grant::{key_id, Grantor, DOMAIN};
use crate::ObjectError;

/// External AAD of an agent's proof of possession.
pub const PROOF_DOMAIN: &str = "cahoots/proof/v1";

/// What a sign grant can allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    GitCommit,
    SshLogin,
    File,
}

impl Action {
    /// The name written in the grant.
    pub fn name(self) -> &'static str {
        match self {
            Action::GitCommit => "git-commit",
            Action::SshLogin => "ssh-login",
            Action::File => "file",
        }
    }

    /// The action a grant's name stands for, if any.
    pub fn parse(s: &str) -> Option<Action> {
        [Action::GitCommit, Action::SshLogin, Action::File]
            .into_iter()
            .find(|a| a.name() == s)
    }
}

/// A seat's grant to an agent's key: sign these actions, as this author, at these places, up to
/// a budget of signatures, until `exp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignGrant {
    pub circle_id: CircleId,
    /// The seat that issues and signs the grant.
    pub seat: u8,
    /// The agent's key.
    pub grantee: SeatPublic,
    pub cti: [u8; 16],
    pub actions: Vec<Action>,
    /// Who commits are signed as, exactly.
    pub author: String,
    /// Where the grant applies. Empty means anywhere.
    pub locations: Vec<String>,
    /// A name for the agent, for people to read.
    pub agent: String,
    /// How many signatures the grant allows.
    pub budget: u32,
    /// Unix time at which the grant ends, if it does.
    pub exp: Option<u64>,
    /// Where the grant's use counter starts on the badge that issued it: uses are the counter
    /// less this. 0 on a badge that counts without a chip.
    pub base: u64,
}

impl SignGrant {
    /// The grant as CWT claims: issued by `seat`, depth 0, no parent.
    pub fn claims(&self) -> Claims {
        Claims {
            iss: cwt::issuer(&self.circle_id),
            sub: cwt::subject(&self.grantee),
            aud: Some(cwt::audience(&self.circle_id, self.seat)),
            exp: self.exp,
            cnf: self.grantee,
            cti: self.cti,
            authorization_details: alloc::vec![Detail {
                r#type: String::from(SIGN_TYPE),
                budget: Some(self.budget),
                actions: Some(
                    self.actions
                        .iter()
                        .map(|a| String::from(a.name()))
                        .collect()
                ),
                author: Some(self.author.clone()),
                locations: Some(self.locations.clone()),
                agent: Some(self.agent.clone()),
                channel: None,
            }],
            log_not_after: None,
            parent: None,
            depth: 0,
            grantor: Grantor::Seat(self.seat),
            base: Some(self.base),
        }
    }

    /// The sign grant that `c` holds. `Malformed` if `c` is not one, or is not issued by the
    /// seat it names.
    pub fn from_claims(c: &Claims) -> Result<SignGrant, ObjectError> {
        let m = ObjectError::Malformed;
        let (circle_id, Kind::Sign(d)) = c.kind()? else {
            return Err(m);
        };
        let (_, seat) = cwt::seat_of(c.aud.as_deref().ok_or(m)?).ok_or(m)?;
        if c.grantor != Grantor::Seat(seat) {
            return Err(m);
        }
        let actions = d
            .actions
            .iter()
            .map(|a| Action::parse(a).ok_or(m))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SignGrant {
            circle_id,
            seat,
            grantee: c.cnf,
            cti: c.cti,
            actions,
            author: d.author,
            locations: d.locations,
            agent: d.agent,
            budget: d.budget,
            exp: c.exp,
            base: c.base.ok_or(m)?,
        })
    }

    /// Sign the grant as `seat`: a COSE_Sign1 under kid `[seat]` and the grant domain.
    pub fn sign<S: SeatSigner>(&self, key: S, seat: u8) -> Vec<u8> {
        cose::sign1(key, &[seat], &self.claims().encode(), DOMAIN, false)
    }

    /// Check a signed sign grant against the key of `seat` in circle `circle_id`.
    pub fn verify(
        signed: &[u8],
        seat_key: &SeatPublic,
        circle_id: &CircleId,
        seat: u8,
    ) -> Result<SignGrant, ObjectError> {
        let kid = cose::kid1(signed).map_err(ObjectError::Cose)?;
        if kid[..] != [seat] {
            return Err(ObjectError::WrongKid);
        }
        let payload = cose::verify1(seat_key, signed, None, DOMAIN).map_err(ObjectError::Cose)?;
        let grant = SignGrant::from_claims(&Claims::decode(&payload)?)?;
        if grant.circle_id != *circle_id {
            return Err(ObjectError::Circle);
        }
        if grant.seat != seat {
            return Err(ObjectError::WrongKid);
        }
        Ok(grant)
    }
}

/// Who asks: the id of the key, see [`key_id`].
pub struct Subject {
    pub key_id: [u8; 32],
}

/// What is asked about: a circle, and where in it.
pub struct Resource {
    pub circle_id: CircleId,
    /// A repository or host, as the person wrote it.
    pub location: Option<String>,
}

/// What the verifier knows about the moment.
pub struct Context {
    /// Who the commit says it is by.
    pub author: String,
    /// Unix time, if the verifier has one.
    pub time: Option<u64>,
    /// Signatures already made under the grant.
    pub used: u64,
    pub revoked: bool,
}

/// One question for [`evaluate`].
pub struct EvalRequest {
    pub subject: Subject,
    pub action: Action,
    pub resource: Resource,
    pub context: Context,
}

/// Why [`evaluate`] answered as it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Allowed,
    WrongKey,
    WrongCircle,
    WrongAction,
    WrongAuthor,
    WrongLocation,
    Expired,
    Spent,
    Revoked,
}

/// The answer: yes or no, the reason, and the signatures left after this one.
pub struct Decision {
    pub decision: bool,
    pub reason: Reason,
    pub remaining: u32,
}

/// May the key in `req` do this under `grant`? Checks in a fixed order and stops at the first
/// failure: revoked, key, circle, action, author, location, expiry, budget. A grant with an
/// `exp` and a request with no time is allowed: a verifier without a clock cannot judge expiry,
/// and the badge supplies its own time.
pub fn evaluate(grant: &SignGrant, req: &EvalRequest) -> Decision {
    let no = |reason| Decision {
        decision: false,
        reason,
        remaining: 0,
    };
    if req.context.revoked {
        return no(Reason::Revoked);
    }
    if req.subject.key_id != key_id(&grant.grantee) {
        return no(Reason::WrongKey);
    }
    if req.resource.circle_id != grant.circle_id {
        return no(Reason::WrongCircle);
    }
    if !grant.actions.contains(&req.action) {
        return no(Reason::WrongAction);
    }
    if req.context.author != grant.author {
        return no(Reason::WrongAuthor);
    }
    if !grant.locations.is_empty() {
        let here = req.resource.location.as_deref();
        if !here.is_some_and(|l| grant.locations.iter().any(|g| location_matches(g, l))) {
            return no(Reason::WrongLocation);
        }
    }
    if let (Some(exp), Some(time)) = (grant.exp, req.context.time) {
        if time >= exp {
            return no(Reason::Expired);
        }
    }
    if req.context.used >= u64::from(grant.budget) {
        return no(Reason::Spent);
    }
    Decision {
        decision: true,
        reason: Reason::Allowed,
        // `used < budget`, so `used` fits in a u32.
        remaining: grant.budget - req.context.used as u32 - 1,
    }
}

/// The host and path of a location, written one way: no scheme, no `git@`, `:` as `/`, no
/// trailing `.git` or `/`, and the host in lower case.
fn normalise(loc: &str) -> String {
    let s = loc.split_once("://").map_or(loc, |(_, rest)| rest);
    let s = s.strip_prefix("git@").unwrap_or(s);
    let s: String = s.chars().map(|c| if c == ':' { '/' } else { c }).collect();
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    let s = s.trim_end_matches('/');
    match s.split_once('/') {
        Some((host, path)) => {
            let mut out = host.to_ascii_lowercase();
            out.push('/');
            out.push_str(path);
            out
        }
        None => s.to_ascii_lowercase(),
    }
}

/// Do two ways of writing a repository or host name the same place? The path is case-sensitive,
/// the host is not.
pub fn location_matches(granted: &str, given: &str) -> bool {
    normalise(granted) == normalise(given)
}

/// The agent's proof that it holds its key right now: a signature over the hash of what it
/// wants signed and a fresh nonce from the verifier. A COSE_Sign1 under [`PROOF_DOMAIN`] with
/// the key's [`key_id`] as kid, whose payload is the CBOR array `[data_hash, nonce]`.
pub fn proof_sign<S: SeatSigner>(mut key: S, data_hash: &[u8; 32], nonce: &[u8; 16]) -> Vec<u8> {
    let kid = key_id(&key.public());
    let mut e = Encoder::new(Vec::new());
    let _ = e
        .array(2)
        .and_then(|e| e.bytes(data_hash))
        .and_then(|e| e.bytes(nonce));
    cose::sign1(key, &kid, &e.into_writer(), PROOF_DOMAIN.as_bytes(), false)
}

/// Check a proof made by `cnf` over this hash and nonce.
pub fn proof_verify(
    proof: &[u8],
    cnf: &SeatPublic,
    data_hash: &[u8; 32],
    nonce: &[u8; 16],
) -> bool {
    let Ok(kid) = cose::kid1(proof) else {
        return false;
    };
    if kid[..] != key_id(cnf) {
        return false;
    }
    let Ok(payload) = cose::verify1(cnf, proof, None, PROOF_DOMAIN.as_bytes()) else {
        return false;
    };
    let mut d = Decoder::new(&payload);
    matches!(d.array(), Ok(Some(2)))
        && d.bytes().is_ok_and(|b| b == data_hash)
        && d.bytes().is_ok_and(|b| b == nonce)
        && d.position() == payload.len()
}
