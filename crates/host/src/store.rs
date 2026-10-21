use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{EraseError, PadStore, StoreError};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;

pub const ERASE_UNIT: u32 = 4096;

/// The pad region as a plain file. Erase overwrites with zeros and syncs.
pub struct FilePadStore {
    file: File,
    len: u32,
}

impl FilePadStore {
    pub fn create(path: &Path, bytes: &[u8]) -> io::Result<Self> {
        std::fs::write(path, bytes)?;
        Self::open(path)
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len() as u32;
        Ok(FilePadStore { file, len })
    }
}

impl PadStore for FilePadStore {
    fn len(&self) -> u32 {
        self.len
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        self.file
            .read_exact_at(buf, u64::from(off))
            .expect("pad read within bounds");
    }

    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError> {
        let end = off.checked_add(len).ok_or(EraseError::OutOfRange)?;
        if end > self.len {
            return Err(EraseError::OutOfRange);
        }
        let zeros = vec![0u8; len as usize];
        self.file
            .write_all_at(&zeros, u64::from(off))
            .map_err(|_| EraseError::VerifyFailed)?;
        self.file.sync_data().map_err(|_| EraseError::VerifyFailed)
    }

    fn erase_unit(&self) -> u32 {
        ERASE_UNIT
    }
}

/// The ledger region as a file, pre-filled with `0xFF` like blank NOR flash.
pub struct FileRecordStore {
    file: File,
    capacity: u32,
}

impl FileRecordStore {
    pub fn create(path: &Path, capacity: u32) -> io::Result<Self> {
        std::fs::write(path, vec![0xFFu8; capacity as usize])?;
        Self::open(path)
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let capacity = file.metadata()?.len() as u32;
        Ok(FileRecordStore { file, capacity })
    }
}

impl RecordStore for FileRecordStore {
    fn capacity(&self) -> u32 {
        self.capacity
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        self.file
            .read_exact_at(buf, u64::from(off))
            .expect("ledger read within bounds");
    }

    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        if off as usize + data.len() > self.capacity as usize {
            return Err(StoreError::OutOfRange);
        }
        self.file
            .write_all_at(data, u64::from(off))
            .map_err(|_| StoreError::WriteFailed)?;
        self.file.sync_data().map_err(|_| StoreError::WriteFailed)
    }
}
