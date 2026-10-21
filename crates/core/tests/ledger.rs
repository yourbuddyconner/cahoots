use cahoots_core::ledger::{MemLedger, SpentLedger};

#[test]
fn ranges_start_unspent_and_become_spent() {
    let mut l = MemLedger::new(3);
    assert!(!l.is_spent(0, 0, 10));
    l.mark_spent(0, 0, 10).unwrap();
    assert!(l.is_spent(0, 0, 10));
    assert!(l.is_spent(0, 9, 1));
    assert!(l.is_spent(0, 5, 20), "any overlap counts");
    assert!(!l.is_spent(0, 10, 5));
    assert!(!l.is_spent(1, 0, 10), "other slice untouched");
}

#[test]
fn next_offset_is_high_water_mark() {
    let mut l = MemLedger::new(2);
    assert_eq!(l.next_offset(1), 0);
    l.mark_spent(1, 0, 40).unwrap();
    l.mark_spent(1, 100, 4).unwrap();
    assert_eq!(l.next_offset(1), 104);
    assert_eq!(l.next_offset(0), 0);
}
