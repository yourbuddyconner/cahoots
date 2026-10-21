//! One circle's pad, slice, and ledger, with the spend ordering from handoff 3.5/3.6.

use crate::{CircleId, PadError, PskHandle, LOW_PAD_PERCENT};
use cahoots_core::ledger::SpentLedger;
use cahoots_core::pad::PadStorage;
use cahoots_core::params::CircleParams;
use zeroize::Zeroizing;

pub(crate) struct Circle<S: PadStorage, L: SpentLedger> {
    pub(crate) params: CircleParams,
    pub(crate) my_slice: u8,
    pub(crate) circle_id: CircleId,
    pub(crate) storage: S,
    pub(crate) ledger: L,
}

impl<S: PadStorage, L: SpentLedger> Circle<S, L> {
    pub(crate) fn new(
        params: CircleParams,
        my_slice: u8,
        circle_id: CircleId,
        storage: S,
        ledger: L,
    ) -> Result<Self, PadError> {
        if storage.len() != params.pad_len || my_slice >= params.slices() {
            return Err(PadError::Config);
        }
        Ok(Circle {
            params,
            my_slice,
            circle_id,
            storage,
            ledger,
        })
    }

    pub(crate) fn remaining(&self) -> u32 {
        self.params
            .slice_len()
            .saturating_sub(self.ledger.next_offset(self.my_slice))
    }

    pub(crate) fn low_pad(&self) -> bool {
        u64::from(self.remaining()) * 100 <= u64::from(self.params.slice_len()) * LOW_PAD_PERCENT
    }

    fn absolute(&self, slice: u8, off: u32, len: u32) -> Result<u32, PadError> {
        if slice >= self.params.slices() {
            return Err(PadError::OutOfRange);
        }
        let end = off.checked_add(len).ok_or(PadError::OutOfRange)?;
        if end > self.params.slice_len() {
            return Err(PadError::OutOfRange);
        }
        Ok(self.params.slice_start(slice) + off)
    }

    /// Read, mark spent (durable), erase, verify, return.
    fn spend_at(&mut self, slice: u8, off: u32, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        let abs = self.absolute(slice, off, len)?;
        let mut buf = Zeroizing::new(vec![0u8; len as usize]);
        self.storage.read(abs, &mut buf);
        self.ledger
            .mark_spent(slice, off, len)
            .map_err(|e| PadError::Storage(format!("ledger: {e:?}")))?;
        self.storage
            .erase(abs, len)
            .map_err(|e| PadError::Storage(format!("erase: {e:?}")))?;
        Ok(buf)
    }

    /// Spend `len` bytes from our own slice at the next unused offset.
    pub(crate) fn spend_own(
        &mut self,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        if len > self.remaining() {
            return Err(PadError::Exhausted);
        }
        let off = self.ledger.next_offset(self.my_slice);
        let bytes = self.spend_at(self.my_slice, off, len)?;
        Ok((
            PskHandle {
                circle_id: self.circle_id,
                slice: self.my_slice,
                offset: off,
            },
            bytes,
        ))
    }

    /// Spend a peer's range named by a handle. Our own slice is never spent this way:
    /// a handle for it means a replay or a misrouted message.
    pub(crate) fn spend_peer(
        &mut self,
        handle: &PskHandle,
        len: u32,
    ) -> Result<Zeroizing<Vec<u8>>, PadError> {
        if handle.slice == self.my_slice {
            return Err(PadError::AlreadySpent);
        }
        self.absolute(handle.slice, handle.offset, len)?;
        if self.ledger.is_spent(handle.slice, handle.offset, len) {
            return Err(PadError::AlreadySpent);
        }
        self.spend_at(handle.slice, handle.offset, len)
    }
}
