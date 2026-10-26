//! Facts (spec §6.4): a reading a seat took from a module, tied to a log position by its nonce.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16 |
//! | 2 | seat uint |
//! | 3 | module tstr, the catalogue entry that explains the reading |
//! | 4 | reading bstr |
//! | 5 | counter uint |
//! | 6 | nonce bstr 32, [`fact_nonce`] of the circle and the log position |
//! | 7 | log uint |
//! | 8 | attested bool |
//! | 9 | evidence bstr, the chip's attested-read response, empty when `attested` is false |
//!
//! Signed by the seat with kid `[seat]`.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatKey;
use cahoots_core::CircleId;
use sha2::{Digest, Sha256};

use crate::{
    canonical, check_seat_and_circle, encode_with, open_seat_signed, Enc, EncResult, ObjectError,
    Reader, VERSION,
};

/// External AAD of a fact's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/fact/v1";
/// Domain of [`fact_nonce`].
pub const NONCE_DOMAIN: &[u8] = b"cahoots/fact-nonce/v1";

/// `SHA-256("cahoots/fact-nonce/v1" || circle_id || log as 8 bytes LE)`: the nonce a fact taken
/// at `log` must carry, so a reading cannot be moved to another position or circle.
pub fn fact_nonce(circle_id: &CircleId, log: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(NONCE_DOMAIN);
    h.update(circle_id);
    h.update(log.to_le_bytes());
    h.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub circle_id: CircleId,
    pub seat: u8,
    pub module: String,
    pub reading: Vec<u8>,
    pub counter: u64,
    pub nonce: [u8; 32],
    pub log: u64,
    pub attested: bool,
    /// Empty unless `attested`.
    pub evidence: Vec<u8>,
}

impl Fact {
    /// The canonical CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(10)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.u8(self.seat)?;
        e.u8(3)?.str(&self.module)?;
        e.u8(4)?.bytes(&self.reading)?;
        e.u8(5)?.u64(self.counter)?;
        e.u8(6)?.bytes(&self.nonce)?;
        e.u8(7)?.u64(self.log)?;
        e.u8(8)?.bool(self.attested)?;
        e.u8(9)?.bytes(&self.evidence)?;
        Ok(())
    }

    /// Parse a canonical payload.
    pub fn decode(bytes: &[u8]) -> Result<Fact, ObjectError> {
        let mut r = Reader::open(bytes, 10)?;
        let fact = Fact {
            circle_id: r.array()?,
            seat: r.u8()?,
            module: r.string()?,
            reading: r.bytes()?.to_vec(),
            counter: r.u64()?,
            nonce: r.array()?,
            log: r.u64()?,
            attested: r.bool()?,
            evidence: r.bytes()?.to_vec(),
        };
        r.finish()?;
        if !fact.attested && !fact.evidence.is_empty() {
            return Err(ObjectError::Malformed);
        }
        let encoded = fact.encode();
        canonical(fact, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded; `kid` is `[seat]`.
    pub fn sign(&self, key: &SeatKey, kid: &[u8]) -> Vec<u8> {
        cose::sign1(key, kid, &self.encode(), DOMAIN, false)
    }

    /// Verify a signed fact against the manifest: as an approval, and the nonce must be
    /// [`fact_nonce`] of the circle and the log position.
    pub fn verify(bytes: &[u8], manifest: &Manifest) -> Result<Fact, ObjectError> {
        let (seat, payload) = open_seat_signed(bytes, manifest, DOMAIN)?;
        let fact = Fact::decode(&payload)?;
        check_seat_and_circle(seat, fact.seat, &fact.circle_id, manifest)?;
        if fact.nonce != fact_nonce(&fact.circle_id, fact.log) {
            return Err(ObjectError::Nonce);
        }
        Ok(fact)
    }
}
