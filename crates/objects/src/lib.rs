//! `cahoots-objects`: the four signed objects a seat emits (spec §6). `no_std` + `alloc`.
//!
//! Every object is a canonical CBOR map (RFC 8949 §4.2.1: definite lengths, shortest integers,
//! integer keys in ascending order) signed as a COSE_Sign1 with the payload embedded and the
//! object's domain string as external AAD. A signed object is a `Vec<u8>`.
//!
//! - [`grant::Grant`]: a seat lets a key spend on some purposes, and that key may delegate.
//! - [`approval::Approval`]: a seat's verdict on a request, with the lines the person saw.
//! - [`fact::Fact`]: a reading a seat took from a module, tied to a log position.
//! - [`receipt::Receipt`]: a record that a seat spent a range of the pad.
//!
//! Each has `encode`, `decode` (which refuses anything non-canonical), `sign`, and a `verify`
//! that checks the signature against the circle's manifest.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod approval;
pub mod assurance;
pub mod attestation;
pub mod fact;
pub mod grant;
#[cfg(feature = "std")]
mod json;
pub mod receipt;

pub use approval::Approval;
pub use fact::{fact_nonce, Fact};
pub use grant::{verify_chain, Grant, GrantError, Grantor};
pub use receipt::Receipt;

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose::{self, CoseError};
use cahoots_core::manifest::Manifest;
use cahoots_core::CircleId;
use core::convert::Infallible;
use minicbor::data::Type;
use minicbor::encode::Error as EncodeError;
use minicbor::{Decoder, Encoder};

/// The format version, key 0 of every object.
pub const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectError {
    /// The COSE_Sign1 does not parse or its signature does not verify.
    Cose(CoseError),
    /// Not the object: does not parse, a field missing or of the wrong type or length, trailing
    /// bytes, or a field outside its range (purposes out of order or repeated, assurance above
    /// [`assurance::PRESENT_PIN`], evidence on a fact that is not attested, a bad key).
    Malformed,
    /// Parses, but is not the canonical encoding of what it holds.
    NotCanonical,
    /// A version other than 1.
    Version,
    /// The kid is not the single byte of the seat the object names.
    WrongKid,
    /// The kid names a seat that is not in the manifest.
    NotInRoster(u8),
    /// The object belongs to another circle.
    Circle,
    /// A fact whose nonce is not `fact_nonce(circle_id, log)`.
    Nonce,
}

pub(crate) type Enc = Encoder<Vec<u8>>;
pub(crate) type EncResult = Result<(), EncodeError<Infallible>>;

/// Run a writer into a fresh buffer. Writing to a `Vec` cannot fail.
pub(crate) fn encode_with(f: impl FnOnce(&mut Enc) -> EncResult) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = f(&mut e);
    e.into_writer()
}

/// Verify a COSE_Sign1 signed by a seat of `manifest` under kid `[seat]`. Returns the seat and
/// the payload; the caller checks that the payload names that seat.
pub(crate) fn open_seat_signed(
    bytes: &[u8],
    manifest: &Manifest,
    domain: &[u8],
) -> Result<(u8, Vec<u8>), ObjectError> {
    let kid = cose::kid1(bytes).map_err(ObjectError::Cose)?;
    let [seat] = kid[..] else {
        return Err(ObjectError::WrongKid);
    };
    let key = manifest
        .seat_key(seat)
        .ok_or(ObjectError::NotInRoster(seat))?;
    let payload = cose::verify1(key, bytes, None, domain).map_err(ObjectError::Cose)?;
    Ok((seat, payload))
}

/// The checks every seat-signed object shares once its payload has decoded.
pub(crate) fn check_seat_and_circle(
    kid_seat: u8,
    seat: u8,
    circle_id: &CircleId,
    manifest: &Manifest,
) -> Result<(), ObjectError> {
    if kid_seat != seat {
        return Err(ObjectError::WrongKid);
    }
    if *circle_id != manifest.circle_id {
        return Err(ObjectError::Circle);
    }
    Ok(())
}

/// Reads a map whose keys are `0..n` in order, the only canonical shape our objects have.
pub(crate) struct Reader<'b> {
    d: Decoder<'b>,
    len: usize,
    n: u64,
    next: u64,
}

const M: ObjectError = ObjectError::Malformed;

impl<'b> Reader<'b> {
    /// Open the map and read key 0, the version.
    pub(crate) fn open(bytes: &'b [u8], n: u64) -> Result<Reader<'b>, ObjectError> {
        let mut d = Decoder::new(bytes);
        match d.map() {
            Ok(Some(len)) if len == n => {}
            Ok(None) => return Err(ObjectError::NotCanonical),
            _ => return Err(M),
        }
        let mut r = Reader {
            d,
            len: bytes.len(),
            n,
            next: 0,
        };
        r.key()?;
        if r.d.u64().map_err(|_| M)? != u64::from(VERSION) {
            return Err(ObjectError::Version);
        }
        Ok(r)
    }

    /// Read the next key. A known key out of place is not canonical; an unknown one is not ours.
    pub(crate) fn key(&mut self) -> Result<&mut Decoder<'b>, ObjectError> {
        let k = self.d.u64().map_err(|_| M)?;
        if k != self.next {
            return Err(if k < self.n {
                ObjectError::NotCanonical
            } else {
                M
            });
        }
        self.next += 1;
        Ok(&mut self.d)
    }

    pub(crate) fn u64(&mut self) -> Result<u64, ObjectError> {
        self.key()?.u64().map_err(|_| M)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, ObjectError> {
        self.u64()?.try_into().map_err(|_| M)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ObjectError> {
        self.u64()?.try_into().map_err(|_| M)
    }

    pub(crate) fn bool(&mut self) -> Result<bool, ObjectError> {
        self.key()?.bool().map_err(|_| M)
    }

    pub(crate) fn bytes(&mut self) -> Result<&'b [u8], ObjectError> {
        self.key()?.bytes().map_err(|_| M)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], ObjectError> {
        self.bytes()?.try_into().map_err(|_| M)
    }

    pub(crate) fn string(&mut self) -> Result<String, ObjectError> {
        self.key()?.str().map(String::from).map_err(|_| M)
    }

    /// A 32-byte string, or nil.
    pub(crate) fn opt_digest(&mut self) -> Result<Option<[u8; 32]>, ObjectError> {
        let d = self.key()?;
        match d.datatype().map_err(|_| M)? {
            Type::Null => {
                d.null().map_err(|_| M)?;
                Ok(None)
            }
            Type::Bytes => Ok(Some(d.bytes().map_err(|_| M)?.try_into().map_err(|_| M)?)),
            _ => Err(M),
        }
    }

    /// A definite array length; an indefinite one parses but is not canonical.
    pub(crate) fn array_len(d: &mut Decoder<'b>) -> Result<u64, ObjectError> {
        match d.array() {
            Ok(Some(n)) => Ok(n),
            Ok(None) => Err(ObjectError::NotCanonical),
            Err(_) => Err(M),
        }
    }

    /// Every key read and nothing after the map.
    pub(crate) fn finish(self) -> Result<(), ObjectError> {
        if self.next != self.n || self.d.position() != self.len {
            return Err(M);
        }
        Ok(())
    }
}

/// The last step of every `decode`: re-encoding must give back the input.
pub(crate) fn canonical<T>(value: T, encoded: Vec<u8>, bytes: &[u8]) -> Result<T, ObjectError> {
    if encoded != bytes {
        return Err(ObjectError::NotCanonical);
    }
    Ok(value)
}
