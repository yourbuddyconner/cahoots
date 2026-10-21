use cahoots_core::pad::{PadStorage, StorageError};
use cahoots_hal::store::StorePad;
use cahoots_hal::{EraseError, PadStore};

struct Mem(Vec<u8>, bool);
impl PadStore for Mem {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&self, off: u32, buf: &mut [u8]) {
        let o = off as usize;
        buf.copy_from_slice(&self.0[o..o + buf.len()]);
    }
    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError> {
        let (s, e) = (off as usize, (off + len) as usize);
        if e > self.0.len() {
            return Err(EraseError::OutOfRange);
        }
        if self.1 {
            // Simulate a store that lies: it reports success without erasing.
            return Ok(());
        }
        self.0[s..e].fill(0);
        Ok(())
    }
    fn erase_unit(&self) -> u32 {
        4096
    }
}

#[test]
fn erase_is_verified_by_read_back() {
    let mut p = StorePad::new(Mem(vec![0xAB; 8192], false));
    assert_eq!(p.len(), 8192);
    p.erase(100, 50).unwrap();
    let mut buf = [1u8; 52];
    p.read(99, &mut buf);
    assert_eq!(buf[0], 0xAB);
    assert_eq!(&buf[1..51], &[0u8; 50]);
    assert_eq!(buf[51], 0xAB);
}

#[test]
fn lying_store_is_reported() {
    let mut p = StorePad::new(Mem(vec![0xAB; 8192], true));
    assert_eq!(p.erase(0, 10), Err(StorageError::VerifyFailed));
    assert_eq!(p.erase(8000, 1000), Err(StorageError::OutOfRange));
}
