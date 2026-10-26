//! Receipts (spec §6.5): a record that a seat spent a range of the pad on a purpose. This is the
//! clear form; sealing receipts to the circle comes later.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16 |
//! | 2 | seat uint |
//! | 3 | slice uint |
//! | 4 | offset uint |
//! | 5 | len uint |
//! | 6 | purpose uint |
//! | 7 | grant: bstr 32, SHA-256 of the signed grant it spent under, or nil |
//! | 8 | log uint |
//!
//! Signed by the seat with kid `[seat]`.

use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatKey;
use cahoots_core::CircleId;

use crate::{
    canonical, check_seat_and_circle, encode_with, open_seat_signed, Enc, EncResult, ObjectError,
    Reader, VERSION,
};

/// External AAD of a receipt's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/receipt/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub circle_id: CircleId,
    pub seat: u8,
    pub slice: u32,
    pub offset: u32,
    pub len: u32,
    /// A [`cahoots_core::control::Purpose`] byte.
    pub purpose: u8,
    pub grant: Option<[u8; 32]>,
    pub log: u64,
}

impl Receipt {
    /// The canonical CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(9)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.u8(self.seat)?;
        e.u8(3)?.u32(self.slice)?;
        e.u8(4)?.u32(self.offset)?;
        e.u8(5)?.u32(self.len)?;
        e.u8(6)?.u8(self.purpose)?;
        e.u8(7)?;
        match &self.grant {
            Some(h) => e.bytes(h)?,
            None => e.null()?,
        };
        e.u8(8)?.u64(self.log)?;
        Ok(())
    }

    /// Parse a canonical payload.
    pub fn decode(bytes: &[u8]) -> Result<Receipt, ObjectError> {
        let mut r = Reader::open(bytes, 9)?;
        let receipt = Receipt {
            circle_id: r.array()?,
            seat: r.u8()?,
            slice: r.u32()?,
            offset: r.u32()?,
            len: r.u32()?,
            purpose: r.u8()?,
            grant: r.opt_digest()?,
            log: r.u64()?,
        };
        r.finish()?;
        let encoded = receipt.encode();
        canonical(receipt, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded; `kid` is `[seat]`.
    pub fn sign(&self, key: &SeatKey, kid: &[u8]) -> Vec<u8> {
        cose::sign1(key, kid, &self.encode(), DOMAIN, false)
    }

    /// Verify a signed receipt against the manifest, as an approval.
    pub fn verify(bytes: &[u8], manifest: &Manifest) -> Result<Receipt, ObjectError> {
        let (seat, payload) = open_seat_signed(bytes, manifest, DOMAIN)?;
        let receipt = Receipt::decode(&payload)?;
        check_seat_and_circle(seat, receipt.seat, &receipt.circle_id, manifest)?;
        Ok(receipt)
    }
}
