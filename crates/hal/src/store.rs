//! Adapts a raw [`PadStore`] to `cahoots_core::pad::PadStorage`, verifying every erase.

use crate::{EraseError, PadStore};
use cahoots_core::pad::{PadStorage, StorageError};

pub struct StorePad<S: PadStore> {
    store: S,
}

impl<S: PadStore> StorePad<S> {
    pub fn new(store: S) -> Self {
        StorePad { store }
    }
    pub fn inner(&self) -> &S {
        &self.store
    }
    pub fn into_inner(self) -> S {
        self.store
    }
}

impl<S: PadStore> PadStorage for StorePad<S> {
    fn len(&self) -> u32 {
        self.store.len()
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        self.store.read(off, buf)
    }

    /// Erase, then read back every byte and require zero.
    fn erase(&mut self, off: u32, len: u32) -> Result<(), StorageError> {
        let end = off.checked_add(len).ok_or(StorageError::OutOfRange)?;
        if end > self.store.len() {
            return Err(StorageError::OutOfRange);
        }
        self.store.erase(off, len).map_err(|e| match e {
            EraseError::OutOfRange => StorageError::OutOfRange,
            EraseError::Unaligned | EraseError::VerifyFailed => StorageError::VerifyFailed,
        })?;
        let mut chunk = [0u8; 64];
        let mut pos = off;
        while pos < end {
            let n = ((end - pos) as usize).min(chunk.len());
            self.store.read(pos, &mut chunk[..n]);
            if chunk[..n].iter().any(|&b| b != 0) {
                return Err(StorageError::VerifyFailed);
            }
            pos += n as u32;
        }
        Ok(())
    }
}
