//! `cahoots-provider`: the pad provider API (handoff section 10.3).
//!
//! Status: placeholder for S1. The trait shape below mirrors the handoff so adapters can
//! compile against it. Backends (file, phone, device, TVC) are not implemented yet.

use cahoots_core::CircleId;
use zeroize::Zeroizing;

/// Locates a spent pad range so the peer can find the same bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PskHandle {
    pub circle_id: CircleId,
    pub slice: u8,
    pub offset: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PadError {
    UnknownCircle,
    Exhausted,
    AlreadySpent,
    Storage,
}

pub trait PadProvider {
    /// Spend 32 bytes from the caller's slice, erase them, and return them with a handle.
    fn psk(
        &mut self,
        circle: &CircleId,
        context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError>;
    /// Receiver side: look up the peer's range, refuse if spent, then spend and erase it.
    fn psk_for(
        &mut self,
        handle: &PskHandle,
        context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError>;
    /// Raw spend for pure-pad messages.
    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError>;
    fn remaining(&self, circle: &CircleId) -> u64;
}
