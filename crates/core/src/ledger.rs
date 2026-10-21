//! Spent-range ledger (handoff 3.6). The ledger MUST be durable before the matching pad
//! bytes are erased, so a crash between the two steps never leaves a range reusable.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerError {
    BadSlice,
    /// The backing store could not persist the record.
    Storage,
}

pub trait SpentLedger {
    /// Record `[off, off+len)` of `slice` as spent. MUST be durable on return.
    fn mark_spent(&mut self, slice: u8, off: u32, len: u32) -> Result<(), LedgerError>;
    /// True if any byte of `[off, off+len)` in `slice` is already spent.
    fn is_spent(&self, slice: u8, off: u32, len: u32) -> bool;
    /// One past the highest spent offset in `slice`: the next send offset.
    fn next_offset(&self, slice: u8) -> u32;
}

/// In-memory ledger for tests, WASM, and as the cache layer for persistent backends.
#[derive(Clone, Debug, Default)]
pub struct MemLedger {
    ranges: Vec<Vec<(u32, u32)>>,
}

impl MemLedger {
    pub fn new(slices: u8) -> Self {
        MemLedger {
            ranges: (0..slices).map(|_| Vec::new()).collect(),
        }
    }

    pub fn ranges(&self, slice: u8) -> &[(u32, u32)] {
        &self.ranges[usize::from(slice)]
    }
}

impl SpentLedger for MemLedger {
    fn mark_spent(&mut self, slice: u8, off: u32, len: u32) -> Result<(), LedgerError> {
        let r = self
            .ranges
            .get_mut(usize::from(slice))
            .ok_or(LedgerError::BadSlice)?;
        r.push((off, off.saturating_add(len)));
        Ok(())
    }

    fn is_spent(&self, slice: u8, off: u32, len: u32) -> bool {
        let end = off.saturating_add(len);
        self.ranges
            .get(usize::from(slice))
            .map(|r| r.iter().any(|&(s, e)| off < e && s < end))
            .unwrap_or(true)
    }

    fn next_offset(&self, slice: u8) -> u32 {
        self.ranges
            .get(usize::from(slice))
            .and_then(|r| r.iter().map(|&(_, e)| e).max())
            .unwrap_or(0)
    }
}
