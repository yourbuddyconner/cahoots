//! The custodian trait and the types it speaks, plus the impl for a core `Pad`.

use crate::wire::{ErrorCode, GrantRequestFields, SshHost, UnderGrantFields};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_chip::Attested;
use cahoots_core::ledger::SpentLedger;
use cahoots_core::pad::{Pad, PadStorage};
use cahoots_core::CircleId;
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

/// What a custodian says about one circle it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CircleInfo {
    pub circle_id: CircleId,
    /// The caller's seat (slice) in the circle.
    pub seat: u8,
    /// Number of members.
    pub n: u8,
    pub name: String,
    /// How hard the holder is to extract bytes from. 0 means no claim.
    pub tier: u8,
}

pub trait Custodian {
    /// The circles this custodian holds.
    fn circles(&self) -> Vec<CircleInfo>;
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

/// Copy a 32-byte spend into a fixed-size key. The input is zeroized on drop.
pub fn to_psk(bytes: Zeroizing<Vec<u8>>) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&bytes);
    out
}

/// A core `Pad` is a custodian too: handles name its own slice, and `psk_for` spends the
/// peer's range. This lets one object both derive pre-shared keys and seal messages.
impl<S: PadStorage, L: SpentLedger> Custodian for Pad<S, L> {
    fn circles(&self) -> Vec<CircleInfo> {
        vec![CircleInfo {
            circle_id: self.circle_id(),
            seat: self.my_slice(),
            n: self.params().n,
            name: self.params().name.clone(),
            tier: 0,
        }]
    }

    fn psk(
        &mut self,
        circle: &CircleId,
        _context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError> {
        if *circle != self.circle_id() {
            return Err(PadError::UnknownCircle);
        }
        let (offset, bytes) = self.spend_next(PSK_LEN).map_err(core_err)?;
        let handle = PskHandle {
            circle_id: *circle,
            slice: self.my_slice(),
            offset,
        };
        Ok((handle, to_psk(bytes)))
    }

    fn psk_for(
        &mut self,
        handle: &PskHandle,
        _context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError> {
        if handle.circle_id != self.circle_id() {
            return Err(PadError::UnknownCircle);
        }
        if handle.slice == self.my_slice() {
            return Err(PadError::AlreadySpent);
        }
        let bytes = self
            .spend_range(handle.slice, handle.offset, PSK_LEN)
            .map_err(core_err)?;
        Ok(to_psk(bytes))
    }

    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        if *circle != self.circle_id() {
            return Err(PadError::UnknownCircle);
        }
        let (offset, bytes) = self.spend_next(len).map_err(core_err)?;
        Ok((
            PskHandle {
                circle_id: *circle,
                slice: self.my_slice(),
                offset,
            },
            bytes,
        ))
    }

    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        if handle.circle_id != self.circle_id() {
            return Err(PadError::UnknownCircle);
        }
        self.spend_range(handle.slice, handle.offset, len)
            .map_err(core_err)
    }

    fn remaining(&self, circle: &CircleId) -> u64 {
        if *circle != self.circle_id() {
            return 0;
        }
        u64::from(Pad::remaining(self))
    }

    fn low_pad(&self, circle: &CircleId) -> bool {
        if *circle != self.circle_id() {
            return true;
        }
        u64::from(Pad::remaining(self)) * 100
            <= u64::from(self.params().slice_len()) * LOW_PAD_PERCENT
    }
}

fn core_err(e: cahoots_core::pad::PadError) -> PadError {
    use cahoots_core::pad::PadError as E;
    match e {
        E::WrongCircle => PadError::UnknownCircle,
        E::OutOfRange => PadError::OutOfRange,
        E::Reuse => PadError::AlreadySpent,
        E::Exhausted => PadError::Exhausted,
        E::Config => PadError::Config,
        other => PadError::Storage(format!("{other:?}")),
    }
}

/// What a badge says about a seat key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipInfo {
    pub public: Vec<u8>,
    pub curve: u8,
    pub in_chip: bool,
    pub can_attest: bool,
}

/// What a badge does with the keys of its seats. Errors are wire error codes: `NoChip`,
/// `ChipError`, `Unparseable`, `UnknownCircle`, `Denied`. `serve` has asked for consent
/// before it calls any signing method.
pub trait ChipApps {
    fn describe(&mut self, circle: &CircleId) -> Result<ChipInfo, ErrorCode>;
    fn attest(&mut self, circle: &CircleId, freshness: &[u8; 16]) -> Result<Attested, ErrorCode>;
    fn ssh_sign(
        &mut self,
        circle: &CircleId,
        data: &[u8],
        host: Option<&SshHost>,
        content: Option<&[u8]>,
    ) -> Result<Vec<u8>, ErrorCode>;
    fn card_sign(&mut self, circle: &CircleId, card: &[u8]) -> Result<Vec<u8>, ErrorCode>;
    fn grant_sign(&mut self, circle: &CircleId, grant: &[u8]) -> Result<Vec<u8>, ErrorCode>;
    fn seal_key(&mut self, circle: &CircleId, label: &str) -> Result<[u8; 32], ErrorCode>;
    /// The nonce the next proof under a grant must be made over: the same until a proof
    /// under it is accepted.
    fn nonce(&mut self, circle: &CircleId) -> Result<[u8; 16], ErrorCode>;
    /// Signs a grant for an agent and returns it with the counter slot it was given.
    fn grant_request(
        &mut self,
        circle: &CircleId,
        req: &GrantRequestFields,
    ) -> Result<(Vec<u8>, u8), ErrorCode>;
    /// Signs for ssh under a grant and returns the signature with the uses left.
    fn ssh_sign_under_grant(
        &mut self,
        circle: &CircleId,
        req: &UnderGrantFields,
    ) -> Result<(Vec<u8>, u32), ErrorCode>;
    /// Whether the device has a chip: `Identify` says so. A device without one keeps this.
    fn has_chip(&self) -> bool {
        false
    }
}

/// A bare pad has no chip: every operation says so.
impl<S: PadStorage, L: SpentLedger> ChipApps for Pad<S, L> {
    fn describe(&mut self, _: &CircleId) -> Result<ChipInfo, ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn attest(&mut self, _: &CircleId, _: &[u8; 16]) -> Result<Attested, ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn ssh_sign(
        &mut self,
        _: &CircleId,
        _: &[u8],
        _: Option<&SshHost>,
        _: Option<&[u8]>,
    ) -> Result<Vec<u8>, ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn card_sign(&mut self, _: &CircleId, _: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn grant_sign(&mut self, _: &CircleId, _: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn seal_key(&mut self, _: &CircleId, _: &str) -> Result<[u8; 32], ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn nonce(&mut self, _: &CircleId) -> Result<[u8; 16], ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn grant_request(
        &mut self,
        _: &CircleId,
        _: &GrantRequestFields,
    ) -> Result<(Vec<u8>, u8), ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn ssh_sign_under_grant(
        &mut self,
        _: &CircleId,
        _: &UnderGrantFields,
    ) -> Result<(Vec<u8>, u32), ErrorCode> {
        Err(ErrorCode::NoChip)
    }
}
