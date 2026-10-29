//! Grants (spec §6.2): a seat lets a key spend pad on some purposes, up to a budget and a log
//! position, and lets that key delegate a narrower grant `depth` more times.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16 |
//! | 2 | grantor: uint seat, or bstr 33 key for a delegated grant |
//! | 3 | grantee key bstr 33 |
//! | 4 | purposes: array of uint, ascending, no repeats |
//! | 5 | budget uint, bytes |
//! | 6 | not_after uint, log position |
//! | 7 | depth uint, delegations still allowed below this grant |
//! | 8 | parent: bstr 32, SHA-256 of the parent's signed bytes, or nil for a root grant |
//! | 9 | nonce bstr 16 |
//!
//! A root grant is signed by a seat key with kid `[seat]`. A delegated grant is signed by the
//! parent's grantee key with kid = SHA-256 of that key's 33 bytes.
//!
//! Version 2 of the payload is a CBOR Web Token with the same meaning ([`crate::cwt`]). Both
//! versions are signed the same way under the same external AAD, and a chain may mix them:
//! a payload whose first key is 0 is version 1, any other is read as a CWT.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose::{self, CoseError};
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use cahoots_core::CircleId;
use minicbor::data::Type;
use sha2::{Digest, Sha256};

use crate::cwt::{self, Claims, Detail};
use crate::{canonical, encode_with, Enc, EncResult, ObjectError, Reader, VERSION};

/// External AAD of a grant's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/grant/v1";

/// Who signed a grant: a seat of the manifest for a root grant, or the parent's grantee key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grantor {
    Seat(u8),
    Key(SeatPublic),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub circle_id: CircleId,
    pub grantor: Grantor,
    /// The key that may use the grant, and sign a delegated one.
    pub grantee: SeatPublic,
    /// [`cahoots_core::control::Purpose`] bytes, ascending, no repeats.
    pub purposes: Vec<u8>,
    /// Bytes of pad the grant may spend.
    pub budget: u32,
    /// The last log position at which the grant holds.
    pub not_after: u64,
    /// Delegations still allowed below this grant.
    pub depth: u8,
    /// SHA-256 of the parent's signed bytes; `None` for a root grant.
    pub parent: Option<[u8; 32]>,
    pub nonce: [u8; 16],
}

/// Why a grant chain was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantError {
    /// No grants at all.
    EmptyChain,
    /// A link's COSE_Sign1 does not parse or does not verify.
    Cose(CoseError),
    /// A link's payload is not a canonical grant.
    Decode(ObjectError),
    /// The root's kid names a seat that is not in the manifest.
    NotInRoster(u8),
    /// The kid is not `[seat]` for a root, or SHA-256 of the previous grantee for a delegation.
    WrongKid,
    /// The grantor field does not name the signer: a root whose grantor is not the kid's seat,
    /// or a delegation whose grantor is not the previous grantee.
    WrongSigner,
    /// A root with a parent, or a delegation whose parent is not the hash of the previous link.
    BadParent,
    /// A purpose the previous link does not have.
    ExtraPurpose,
    /// A budget larger than the previous link's.
    LargerBudget,
    /// A `not_after` later than the previous link's.
    LaterNotAfter,
    /// The previous link allowed no delegation, or this depth is not one less.
    Depth,
    /// A link in a circle other than the manifest's.
    Circle,
}

impl Grant {
    /// The canonical CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(10)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?;
        match &self.grantor {
            Grantor::Seat(s) => e.u8(*s)?,
            Grantor::Key(k) => e.bytes(&k.0)?,
        };
        e.u8(3)?.bytes(&self.grantee.0)?;
        e.u8(4)?.array(self.purposes.len() as u64)?;
        for p in &self.purposes {
            e.u8(*p)?;
        }
        e.u8(5)?.u32(self.budget)?;
        e.u8(6)?.u64(self.not_after)?;
        e.u8(7)?.u8(self.depth)?;
        e.u8(8)?;
        match &self.parent {
            Some(h) => e.bytes(h)?,
            None => e.null()?,
        };
        e.u8(9)?.bytes(&self.nonce)?;
        Ok(())
    }

    /// Parse a canonical payload.
    pub fn decode(bytes: &[u8]) -> Result<Grant, ObjectError> {
        let m = ObjectError::Malformed;
        let mut r = Reader::open(bytes, 10)?;
        let circle_id = r.array()?;
        let grantor = {
            let d = r.key()?;
            match d.datatype().map_err(|_| m)? {
                Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
                    Grantor::Seat(d.u64().map_err(|_| m)?.try_into().map_err(|_| m)?)
                }
                Type::Bytes => {
                    Grantor::Key(SeatPublic::parse(d.bytes().map_err(|_| m)?).map_err(|_| m)?)
                }
                _ => return Err(m),
            }
        };
        let grantee = SeatPublic::parse(r.bytes()?).map_err(|_| m)?;
        let purposes = {
            let d = r.key()?;
            let n = Reader::array_len(d)?;
            let mut list = Vec::new();
            for _ in 0..n {
                list.push(d.u64().map_err(|_| m)?.try_into().map_err(|_| m)?);
            }
            list
        };
        if !purposes.windows(2).all(|w: &[u8]| w[0] < w[1]) {
            return Err(m);
        }
        let grant = Grant {
            circle_id,
            grantor,
            grantee,
            purposes,
            budget: r.u32()?,
            not_after: r.u64()?,
            depth: r.u8()?,
            parent: r.opt_digest()?,
            nonce: r.array()?,
        };
        r.finish()?;
        let encoded = grant.encode();
        canonical(grant, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded. `kid` is `[seat]` for a root grant and
    /// [`key_id`] of the signer for a delegated one.
    pub fn sign<S: SeatSigner>(&self, key: S, kid: &[u8]) -> Vec<u8> {
        cose::sign1(key, kid, &self.encode(), DOMAIN, false)
    }

    /// The same grant as version 2 claims.
    pub fn to_v2(&self) -> Claims {
        let mut details: Vec<Detail> = self
            .purposes
            .iter()
            .map(|p| Detail {
                r#type: cwt::purpose_type(*p),
                budget: None,
            })
            .collect();
        details.push(Detail {
            r#type: String::from(cwt::PAD_TYPE),
            budget: Some(self.budget),
        });
        Claims {
            iss: cwt::issuer(&self.circle_id),
            sub: cwt::subject(&self.grantee),
            cnf: self.grantee,
            cti: self.nonce,
            authorization_details: details,
            log_not_after: self.not_after,
            parent: self.parent,
            depth: self.depth,
            grantor: self.grantor,
        }
    }

    /// The grant version 2 claims hold. `Malformed` if they are not a grant: see
    /// [`Claims::decode`].
    pub fn from_v2(claims: &Claims) -> Result<Grant, ObjectError> {
        let (circle_id, purposes, budget) = claims.meaning()?;
        Ok(Grant {
            circle_id,
            grantor: claims.grantor,
            grantee: claims.cnf,
            purposes,
            budget,
            not_after: claims.log_not_after,
            depth: claims.depth,
            parent: claims.parent,
            nonce: claims.cti,
        })
    }

    /// The version 2 payload: [`Grant::to_v2`] encoded.
    pub fn encode_v2(&self) -> Vec<u8> {
        self.to_v2().encode()
    }

    /// Sign the version 2 payload, as [`Grant::sign`] does version 1.
    pub fn sign_v2<S: SeatSigner>(&self, key: S, kid: &[u8]) -> Vec<u8> {
        cose::sign1(key, kid, &self.encode_v2(), DOMAIN, false)
    }

    /// Parse a payload of either version: the grant and its version, 1 or 2.
    pub fn decode_any(bytes: &[u8]) -> Result<(Grant, u8), ObjectError> {
        // Version 1 is a map whose first key is 0; a CWT has no key 0.
        if bytes.len() >= 2 && bytes[0] & 0xe0 == 0xa0 && bytes[1] == 0x00 {
            return Grant::decode(bytes).map(|g| (g, 1));
        }
        let claims = Claims::decode(bytes)?;
        Grant::from_v2(&claims).map(|g| (g, 2))
    }
}

/// The kid of a delegated grant: SHA-256 of the signing key's 33 bytes.
pub fn key_id(key: &SeatPublic) -> [u8; 32] {
    Sha256::digest(key.0).into()
}

/// Verify a chain of signed grants, root first, against the manifest, and return the leaf.
///
/// The root is signed by a seat of the manifest under kid `[seat]`, names that seat as its
/// grantor and has no parent. Each later link is signed by the previous grantee under kid
/// [`key_id`] of it, names it as grantor, carries as `parent` the SHA-256 of the previous
/// link's exact signed bytes, and narrows: its purposes a subset, its budget and `not_after` no
/// greater, its depth exactly one less than a nonzero previous depth. Every link is in the
/// manifest's circle.
pub fn verify_chain(chain: &[&[u8]], manifest: &Manifest) -> Result<Grant, GrantError> {
    let (root_bytes, rest) = chain.split_first().ok_or(GrantError::EmptyChain)?;
    let kid = cose::kid1(root_bytes).map_err(GrantError::Cose)?;
    let [seat] = kid[..] else {
        return Err(GrantError::WrongKid);
    };
    let key = manifest
        .seat_key(seat)
        .ok_or(GrantError::NotInRoster(seat))?;
    let mut prev = open(root_bytes, key)?;
    if prev.grantor != Grantor::Seat(seat) {
        return Err(GrantError::WrongSigner);
    }
    if prev.parent.is_some() {
        return Err(GrantError::BadParent);
    }
    if prev.circle_id != manifest.circle_id {
        return Err(GrantError::Circle);
    }
    let mut prev_bytes: &[u8] = root_bytes;
    for bytes in rest {
        let signer = prev.grantee;
        if cose::kid1(bytes).map_err(GrantError::Cose)? != key_id(&signer) {
            return Err(GrantError::WrongKid);
        }
        let g = open(bytes, &signer)?;
        if g.grantor != Grantor::Key(signer) {
            return Err(GrantError::WrongSigner);
        }
        let parent: [u8; 32] = Sha256::digest(prev_bytes).into();
        if g.parent != Some(parent) {
            return Err(GrantError::BadParent);
        }
        if g.circle_id != prev.circle_id {
            return Err(GrantError::Circle);
        }
        if !g.purposes.iter().all(|p| prev.purposes.contains(p)) {
            return Err(GrantError::ExtraPurpose);
        }
        if g.budget > prev.budget {
            return Err(GrantError::LargerBudget);
        }
        if g.not_after > prev.not_after {
            return Err(GrantError::LaterNotAfter);
        }
        if prev.depth == 0 || g.depth != prev.depth - 1 {
            return Err(GrantError::Depth);
        }
        prev = g;
        prev_bytes = bytes;
    }
    Ok(prev)
}

fn open(bytes: &[u8], key: &SeatPublic) -> Result<Grant, GrantError> {
    let payload = cose::verify1(key, bytes, None, DOMAIN).map_err(GrantError::Cose)?;
    Grant::decode_any(&payload)
        .map(|(g, _)| g)
        .map_err(GrantError::Decode)
}
