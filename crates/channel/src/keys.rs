//! The key schedule: an epoch id names a channel and an epoch number; the epoch key comes from the
//! epoch's root secret; each sender's chain starts from the epoch key and the sender's key id.

use crate::ChannelId;
use alloc::vec::Vec;
use cahoots_core::seat::SeatPublic;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

pub use cahoots_chat::step;

pub const EPOCH_ID_LEN: usize = 20;

const EPOCH_DOMAIN: &[u8] = b"cahoots/channel/epoch/v1";
const CHAIN_DOMAIN: &[u8] = b"cahoots/channel/chain/v1";

pub type Key = Zeroizing<[u8; 32]>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EpochId {
    pub channel_id: ChannelId,
    pub epoch: u32,
}

impl EpochId {
    /// The channel id, then the epoch as a little-endian `u32`.
    pub fn encode(&self) -> [u8; EPOCH_ID_LEN] {
        let mut out = [0u8; EPOCH_ID_LEN];
        out[..16].copy_from_slice(&self.channel_id);
        out[16..].copy_from_slice(&self.epoch.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<EpochId> {
        let bytes: &[u8; EPOCH_ID_LEN] = bytes.try_into().ok()?;
        Some(EpochId {
            channel_id: bytes[..16].try_into().ok()?,
            epoch: u32::from_le_bytes(bytes[16..].try_into().ok()?),
        })
    }
}

/// `HKDF(salt = domain || epoch id, ikm = root, info = "")`.
pub fn epoch_key(id: &EpochId, root: &[u8; 32]) -> Key {
    let mut salt = Vec::with_capacity(EPOCH_DOMAIN.len() + EPOCH_ID_LEN);
    salt.extend_from_slice(EPOCH_DOMAIN);
    salt.extend_from_slice(&id.encode());
    let hk = Hkdf::<Sha256>::new(Some(&salt), root);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"", out.as_mut())
        .expect("32 bytes is a valid HKDF length");
    out
}

/// `chain_0[kid] = HKDF-Expand(epoch_key, domain || kid)`.
pub fn chain_init(epoch_key: &[u8; 32], kid: &[u8; 32]) -> Key {
    let mut info = Vec::with_capacity(CHAIN_DOMAIN.len() + 32);
    info.extend_from_slice(CHAIN_DOMAIN);
    info.extend_from_slice(kid);
    let hk = Hkdf::<Sha256>::from_prk(epoch_key).expect("32 bytes is a valid PRK");
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(&info, out.as_mut())
        .expect("32 bytes is a valid HKDF length");
    out
}

/// A sender's key id: `SHA-256` of the public key.
pub fn kid_of(key: &SeatPublic) -> [u8; 32] {
    cahoots_objects::grant::key_id(key)
}
