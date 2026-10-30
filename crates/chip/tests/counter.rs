use cahoots_chip::{ChipError, ChipOps, KeyId, KeyKind, SoftChip};

#[test]
fn counter_get_reads_without_incrementing() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Grant, 3);
    chip.put_counter(id, 0).unwrap();
    assert_eq!(chip.counter_get(id).unwrap(), 0);
    assert_eq!(chip.counter_next(id).unwrap(), 1);
    assert_eq!(chip.counter_get(id).unwrap(), 1);
    assert_eq!(chip.counter_get(KeyId::new(KeyKind::Grant, 4)), Err(ChipError::NoSuchKey));
    assert!(chip.ids().unwrap().contains(&id));
}
