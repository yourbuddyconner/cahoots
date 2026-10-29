//! Approvals (spec §6.3): a seat's verdict on a request, carrying the lines the badge rendered
//! so the evidence says what the person saw.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16 |
//! | 2 | seat uint |
//! | 3 | profile tstr, the operation's profile name |
//! | 4 | fields: array of `[label tstr, value tstr]` pairs, in the order shown |
//! | 5 | request_digest bstr 32 |
//! | 6 | policy_id bstr |
//! | 7 | verdict bool |
//! | 8 | assurance uint, at most [`assurance::PRESENT_PIN`] |
//! | 9 | log uint |
//! | 10 | nonce bstr 16 |
//!
//! Signed by the seat with kid `[seat]`.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatSigner;
use cahoots_core::CircleId;

use crate::{
    assurance, canonical, check_seat_and_circle, encode_with, open_seat_signed, Enc, EncResult,
    ObjectError, Reader, VERSION,
};

/// External AAD of an approval's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/approval/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub circle_id: CircleId,
    pub seat: u8,
    pub profile: String,
    /// `(label, value)` lines in the order the badge showed them.
    pub fields: Vec<(String, String)>,
    pub request_digest: [u8; 32],
    pub policy_id: Vec<u8>,
    pub verdict: bool,
    /// One of the [`assurance`] levels.
    pub assurance: u8,
    pub log: u64,
    pub nonce: [u8; 16],
}

impl Approval {
    /// The canonical CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(11)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.u8(self.seat)?;
        e.u8(3)?.str(&self.profile)?;
        e.u8(4)?.array(self.fields.len() as u64)?;
        for (label, value) in &self.fields {
            e.array(2)?.str(label)?.str(value)?;
        }
        e.u8(5)?.bytes(&self.request_digest)?;
        e.u8(6)?.bytes(&self.policy_id)?;
        e.u8(7)?.bool(self.verdict)?;
        e.u8(8)?.u8(self.assurance)?;
        e.u8(9)?.u64(self.log)?;
        e.u8(10)?.bytes(&self.nonce)?;
        Ok(())
    }

    /// Parse a canonical payload.
    pub fn decode(bytes: &[u8]) -> Result<Approval, ObjectError> {
        let m = ObjectError::Malformed;
        let mut r = Reader::open(bytes, 11)?;
        let circle_id = r.array()?;
        let seat = r.u8()?;
        let profile = r.string()?;
        let fields = {
            let d = r.key()?;
            let n = Reader::array_len(d)?;
            let mut list = Vec::new();
            for _ in 0..n {
                if Reader::array_len(d)? != 2 {
                    return Err(m);
                }
                let label = d.str().map_err(|_| m)?.into();
                let value = d.str().map_err(|_| m)?.into();
                list.push((label, value));
            }
            list
        };
        let approval = Approval {
            circle_id,
            seat,
            profile,
            fields,
            request_digest: r.array()?,
            policy_id: r.bytes()?.to_vec(),
            verdict: r.bool()?,
            assurance: r.u8()?,
            log: r.u64()?,
            nonce: r.array()?,
        };
        r.finish()?;
        if approval.assurance > assurance::PRESENT_PIN {
            return Err(m);
        }
        let encoded = approval.encode();
        canonical(approval, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded; `kid` is `[seat]`.
    pub fn sign<S: SeatSigner>(&self, key: S, kid: &[u8]) -> Vec<u8> {
        cose::sign1(key, kid, &self.encode(), DOMAIN, false)
    }

    /// Verify a signed approval against the manifest: the kid is the approving seat, that seat's
    /// key signed it, the payload is canonical, and the circle is the manifest's.
    pub fn verify(bytes: &[u8], manifest: &Manifest) -> Result<Approval, ObjectError> {
        let (seat, payload) = open_seat_signed(bytes, manifest, DOMAIN)?;
        let approval = Approval::decode(&payload)?;
        check_seat_and_circle(seat, approval.seat, &approval.circle_id, manifest)?;
        Ok(approval)
    }
}
