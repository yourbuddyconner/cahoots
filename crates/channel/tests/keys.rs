use cahoots_channel::keys::{chain_init, epoch_key, kid_of, step, EpochId, EPOCH_ID_LEN};
use cahoots_core::seat::SeatPublic;

#[test]
fn epoch_id_round_trips_and_keys_are_deterministic() {
    let id = EpochId {
        channel_id: [3; 16],
        epoch: 0x0102_0304,
    };
    let enc = id.encode();
    assert_eq!(enc.len(), EPOCH_ID_LEN);
    assert_eq!(&enc[..16], &[3; 16]);
    assert_eq!(&enc[16..], &[4, 3, 2, 1]);
    assert_eq!(EpochId::decode(&enc), Some(id));
    assert_eq!(EpochId::decode(&enc[..19]), None);
    assert_eq!(EpochId::decode(&[0; 21]), None);

    let root = [7u8; 32];
    let other_epoch = EpochId { epoch: 2, ..id };
    let other_channel = EpochId {
        channel_id: [4; 16],
        ..id
    };
    assert_eq!(*epoch_key(&id, &root), *epoch_key(&id, &root));
    assert_ne!(*epoch_key(&id, &root), *epoch_key(&other_epoch, &root));
    assert_ne!(*epoch_key(&id, &root), *epoch_key(&other_channel, &root));
    assert_ne!(*epoch_key(&id, &root), *epoch_key(&id, &[8; 32]));
}

#[test]
fn chains_differ_per_kid() {
    let ek = [5u8; 32];
    let a = kid_of(&SeatPublic([2; 33]));
    let b = kid_of(&SeatPublic([3; 33]));
    assert_ne!(a, b);
    assert_eq!(*chain_init(&ek, &a), *chain_init(&ek, &a));
    assert_ne!(*chain_init(&ek, &a), *chain_init(&ek, &b));
    assert_ne!(*chain_init(&ek, &a), *chain_init(&[6; 32], &a));
    let (m, n) = step(&chain_init(&ek, &a));
    assert_ne!(*m, *n);
}
