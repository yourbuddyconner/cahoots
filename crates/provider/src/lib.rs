//! `cahoots-provider`: the pad provider API (handoff section 10.3).
//!
//! The provider is the only code that touches the spent-range ledger. Adapters never read
//! the pad directly. Every `psk` or `spend` call marks the range spent (durably), erases
//! it, and only then returns the bytes.
//!
//! Backends: [`MemProvider`] (tests, WASM), [`FileProvider`] (development, encrypted at
//! rest). Device-over-USB and TVC backends are later milestones.

mod circle;
mod file;
mod mem;
mod session;

pub use cahoots_core::CircleId;
pub use file::FileProvider;
pub use mem::MemProvider;
pub use session::{derive_session_key, PSK_SALT_DOMAIN, SESSION_KEY_INFO};
use zeroize::Zeroizing;

/// Locates a spent pad range so the peer can find the same bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PskHandle {
    pub circle_id: CircleId,
    pub slice: u8,
    pub offset: u32,
}

impl PskHandle {
    /// `circle_id || slice || offset (u32 LE)`: the `psk_id` for HPKE and the salt suffix
    /// for session-key derivation.
    pub fn to_bytes(&self) -> [u8; 21] {
        let mut out = [0u8; 21];
        out[..16].copy_from_slice(&self.circle_id);
        out[16] = self.slice;
        out[17..].copy_from_slice(&self.offset.to_le_bytes());
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != 21 {
            return None;
        }
        let mut circle_id = [0u8; 16];
        circle_id.copy_from_slice(&b[..16]);
        Some(PskHandle {
            circle_id,
            slice: b[16],
            offset: u32::from_le_bytes([b[17], b[18], b[19], b[20]]),
        })
    }
}

/// Bytes spent per `psk` call.
pub const PSK_LEN: u32 = 32;
/// `low_pad` fires when this fraction of the slice (in percent) or less remains.
pub const LOW_PAD_PERCENT: u64 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PadError {
    UnknownCircle,
    /// Not enough unspent bytes in the caller's slice.
    Exhausted,
    /// The handle's range overlaps an already-spent range, or names our own slice.
    AlreadySpent,
    /// The handle's slice or range lies outside the pad.
    OutOfRange,
    /// Parameters, slice index, or pad length do not agree.
    Config,
    Storage(String),
}

pub trait PadProvider {
    /// Spend 32 bytes from the caller's slice, erase them, and return them with a handle
    /// the peer uses to find the same bytes. The ledger update happens before return.
    fn psk(
        &mut self,
        circle: &CircleId,
        context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError>;
    /// Receiver side: look up the peer's range, refuse if already spent, then spend and
    /// erase it.
    fn psk_for(
        &mut self,
        handle: &PskHandle,
        context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError>;
    /// Raw spend for pure-pad messages (handoff 3.5).
    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError>;
    /// Receiver side of `spend`: the peer's `len`-byte range at `handle`.
    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError>;
    /// Unspent bytes left in the caller's slice. Zero for an unknown circle.
    fn remaining(&self, circle: &CircleId) -> u64;
    /// True when 10% or less of the caller's slice remains. The fix is a new ceremony.
    fn low_pad(&self, circle: &CircleId) -> bool;
}
