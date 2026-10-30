//! The channel object (channels spec §2): names a channel and points at its current policy.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16, the owner circle |
//! | 2 | channel_id bstr 16 |
//! | 3 | name tstr, 1 to 64 bytes |
//! | 4 | created_log uint, the channel log position at which this object takes effect |
//! | 5 | policy_id bstr 32, SHA-256 of the current policy's signed bytes |
//! | 6 | nonce bstr 16 |
//!
//! Signed by a seat of the owner circle with kid `[seat]`. A channel object is replaced, never
//! edited: a later one with the same `channel_id` and a later `created_log` supersedes it.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatSigner;
use cahoots_core::CircleId;
use sha2::{Digest, Sha256};

use crate::cbor::{canonical, encode_with, Enc, EncResult, Reader, VERSION};
use crate::{open_seat_signed, ChannelError, ChannelId};

/// External AAD of a channel object's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/channel/v1";

/// The longest name, in bytes.
const NAME_MAX: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelObject {
    pub circle_id: CircleId,
    pub channel_id: ChannelId,
    pub name: String,
    pub created_log: u64,
    pub policy_id: [u8; 32],
    pub nonce: [u8; 16],
}

/// The first 16 bytes of `SHA-256("cahoots/channel-id/v1" || circle_id || nonce)`.
pub fn channel_id_for(circle_id: &CircleId, nonce: &[u8; 16]) -> ChannelId {
    let mut h = Sha256::new();
    h.update(b"cahoots/channel-id/v1");
    h.update(circle_id);
    h.update(nonce);
    let digest = h.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

impl ChannelObject {
    /// The canonical CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(7)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.bytes(&self.channel_id)?;
        e.u8(3)?.str(&self.name)?;
        e.u8(4)?.u64(self.created_log)?;
        e.u8(5)?.bytes(&self.policy_id)?;
        e.u8(6)?.bytes(&self.nonce)?;
        Ok(())
    }

    /// Parse a canonical payload. Refuses a name that is empty or longer than 64 bytes.
    pub fn decode(bytes: &[u8]) -> Result<ChannelObject, ChannelError> {
        let mut r = Reader::open(bytes, 7)?;
        let object = ChannelObject {
            circle_id: r.array()?,
            channel_id: r.array()?,
            name: r.string()?,
            created_log: r.u64()?,
            policy_id: r.array()?,
            nonce: r.array()?,
        };
        r.finish()?;
        if object.name.is_empty() || object.name.len() > NAME_MAX {
            return Err(ChannelError::Malformed);
        }
        let encoded = object.encode();
        canonical(object, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded; the kid is `[seat]`.
    pub fn sign<S: SeatSigner>(&self, key: S, seat: u8) -> Vec<u8> {
        cose::sign1(key, &[seat], &self.encode(), DOMAIN, false)
    }

    /// Verify a signed channel object against the owner circle's manifest: the kid names a seat,
    /// that seat's key signed it, the payload is canonical, the circle is the manifest's, and the
    /// channel id is derived from the circle and the nonce. Returns the signing seat.
    pub fn verify(bytes: &[u8], manifest: &Manifest) -> Result<(u8, ChannelObject), ChannelError> {
        let (seat, payload) = open_seat_signed(bytes, manifest, DOMAIN)?;
        let object = ChannelObject::decode(&payload)?;
        if object.circle_id != manifest.circle_id {
            return Err(ChannelError::WrongCircle);
        }
        if object.channel_id != channel_id_for(&object.circle_id, &object.nonce) {
            return Err(ChannelError::WrongChannel);
        }
        Ok((seat, object))
    }
}
