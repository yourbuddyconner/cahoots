//! A spent-range ledger with power-fail safety (handoff 3.6).
//!
//! Records are appended to a raw region before the matching pad bytes are erased. On
//! open, the region is replayed into an in-memory cache. Unwritten media reads as either
//! all `0xFF` (NOR flash) or all `0x00`; both end the log. A record that is neither blank
//! nor valid is a torn write and also ends the log; it is counted in [`PersistentLedger::torn_records`].
//!
//! Record layout, 16 bytes: `magic 0xCA | slice u8 | off u32 LE | len u32 LE | crc32 LE | 0 | 0`.
//! The CRC covers the first 10 bytes.

use crate::StoreError;
use cahoots_core::ledger::{LedgerError, MemLedger, SpentLedger};
use crc::{Crc, CRC_32_ISO_HDLC};

pub const RECORD_LEN: usize = 16;
const MAGIC: u8 = 0xCA;
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

/// A raw, append-friendly storage region for ledger records.
pub trait RecordStore {
    fn capacity(&self) -> u32;
    fn read(&self, off: u32, buf: &mut [u8]);
    /// MUST be durable on return.
    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError>;
}

fn encode(slice: u8, off: u32, len: u32) -> [u8; RECORD_LEN] {
    let mut r = [0u8; RECORD_LEN];
    r[0] = MAGIC;
    r[1] = slice;
    r[2..6].copy_from_slice(&off.to_le_bytes());
    r[6..10].copy_from_slice(&len.to_le_bytes());
    let crc = CRC32.checksum(&r[..10]);
    r[10..14].copy_from_slice(&crc.to_le_bytes());
    r
}

fn decode(r: &[u8; RECORD_LEN]) -> Option<(u8, u32, u32)> {
    if r[0] != MAGIC {
        return None;
    }
    let crc = u32::from_le_bytes([r[10], r[11], r[12], r[13]]);
    if CRC32.checksum(&r[..10]) != crc {
        return None;
    }
    Some((
        r[1],
        u32::from_le_bytes([r[2], r[3], r[4], r[5]]),
        u32::from_le_bytes([r[6], r[7], r[8], r[9]]),
    ))
}

pub struct PersistentLedger<S: RecordStore> {
    store: S,
    cache: MemLedger,
    slices: u8,
    next: u32,
    torn: u32,
}

impl<S: RecordStore> PersistentLedger<S> {
    /// Replay the region into a cache.
    pub fn open(store: S, slices: u8) -> Result<Self, LedgerError> {
        let mut cache = MemLedger::new(slices);
        let mut off = 0u32;
        let mut torn = 0;
        let mut rec = [0u8; RECORD_LEN];
        while off + RECORD_LEN as u32 <= store.capacity() {
            store.read(off, &mut rec);
            match decode(&rec) {
                Some((slice, o, len)) => {
                    cache.mark_spent(slice, o, len)?;
                    off += RECORD_LEN as u32;
                }
                None => {
                    let blank = rec.iter().all(|&b| b == 0xFF) || rec.iter().all(|&b| b == 0x00);
                    if !blank {
                        torn = 1;
                    }
                    break;
                }
            }
        }
        Ok(PersistentLedger {
            store,
            cache,
            slices,
            next: off,
            torn,
        })
    }

    pub fn torn_records(&self) -> u32 {
        self.torn
    }
    pub fn into_store(self) -> S {
        self.store
    }
    pub fn cache(&self) -> &MemLedger {
        &self.cache
    }
}

impl<S: RecordStore> SpentLedger for PersistentLedger<S> {
    fn mark_spent(&mut self, slice: u8, off: u32, len: u32) -> Result<(), LedgerError> {
        if slice >= self.slices {
            return Err(LedgerError::BadSlice);
        }
        if self.next + RECORD_LEN as u32 > self.store.capacity() {
            return Err(LedgerError::Storage);
        }
        self.store
            .write(self.next, &encode(slice, off, len))
            .map_err(|_| LedgerError::Storage)?;
        self.next += RECORD_LEN as u32;
        self.cache.mark_spent(slice, off, len)
    }

    fn is_spent(&self, slice: u8, off: u32, len: u32) -> bool {
        self.cache.is_spent(slice, off, len)
    }

    fn next_offset(&self, slice: u8) -> u32 {
        self.cache.next_offset(slice)
    }
}
