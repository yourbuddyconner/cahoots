use cahoots_core::ledger::SpentLedger;
use cahoots_hal::ledger::{PersistentLedger, RecordStore, RECORD_LEN};
use cahoots_hal::StoreError;

#[derive(Clone)]
struct MemRecords(Vec<u8>);
impl RecordStore for MemRecords {
    fn capacity(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&self, off: u32, buf: &mut [u8]) {
        let o = off as usize;
        buf.copy_from_slice(&self.0[o..o + buf.len()]);
    }
    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        let o = off as usize;
        if o + data.len() > self.0.len() {
            return Err(StoreError::OutOfRange);
        }
        self.0[o..o + data.len()].copy_from_slice(data);
        Ok(())
    }
}

#[test]
fn records_are_written_before_cache_and_survive_reopen() {
    let store = MemRecords(vec![0xFF; RECORD_LEN * 8]);
    let mut l = PersistentLedger::open(store, 2).unwrap();
    assert_eq!(l.next_offset(0), 0);
    l.mark_spent(0, 0, 37).unwrap();
    l.mark_spent(1, 10, 5).unwrap();
    let store = l.into_store();
    // Reopen from the raw bytes only.
    let l2 = PersistentLedger::open(store, 2).unwrap();
    assert!(l2.is_spent(0, 0, 37));
    assert!(l2.is_spent(1, 12, 1));
    assert!(!l2.is_spent(1, 0, 10));
    assert_eq!(l2.next_offset(0), 37);
    assert_eq!(l2.next_offset(1), 15);
}

#[test]
fn full_store_refuses_new_records() {
    let store = MemRecords(vec![0xFF; RECORD_LEN * 2]);
    let mut l = PersistentLedger::open(store, 1).unwrap();
    l.mark_spent(0, 0, 1).unwrap();
    l.mark_spent(0, 1, 1).unwrap();
    assert!(l.mark_spent(0, 2, 1).is_err());
    assert!(!l.is_spent(0, 2, 1), "refused record is not cached either");
}

#[test]
fn corrupt_record_ends_the_log() {
    let store = MemRecords(vec![0xFF; RECORD_LEN * 4]);
    let mut l = PersistentLedger::open(store, 1).unwrap();
    l.mark_spent(0, 0, 8).unwrap();
    l.mark_spent(0, 8, 8).unwrap();
    let mut store = l.into_store();
    store.0[RECORD_LEN + 3] ^= 0x01; // damage the second record
    let l2 = PersistentLedger::open(store, 1).unwrap();
    assert!(l2.is_spent(0, 0, 8));
    assert!(!l2.is_spent(0, 8, 8));
    // A torn second record is reported so the caller can decide what to do.
    assert_eq!(l2.torn_records(), 1);
}

#[test]
fn zero_filled_media_is_also_empty() {
    let store = MemRecords(vec![0x00; RECORD_LEN * 4]);
    let l = PersistentLedger::open(store, 1).unwrap();
    assert_eq!(l.next_offset(0), 0);
}
