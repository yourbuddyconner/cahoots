use cahoots_core::ledger::{MemLedger, SpentLedger};
use cahoots_core::message::{MessageError, MessageType};
use cahoots_core::pad::{MemPadStorage, Pad, PadError, PadStorage};
use cahoots_core::params::CircleParams;

fn two_members() -> (Pad<MemPadStorage, MemLedger>, Pad<MemPadStorage, MemLedger>) {
    let params = CircleParams::new(2, 2048, "c", false).unwrap();
    let bytes: Vec<u8> = (0..2048u32)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 3)) as u8)
        .collect();
    let a = Pad::new(
        params.clone(),
        0,
        MemPadStorage::new(bytes.clone()),
        MemLedger::new(2),
    )
    .unwrap();
    let b = Pad::new(params, 1, MemPadStorage::new(bytes), MemLedger::new(2)).unwrap();
    (a, b)
}

#[test]
fn message_roundtrips_between_members() {
    let (mut a, mut b) = two_members();
    let wire = a.send(MessageType::Text, b"hello b").unwrap();
    let (hdr, pt) = b.receive(&wire).unwrap();
    assert_eq!(pt, b"hello b");
    assert_eq!(hdr.slice, 0);
    assert_eq!(hdr.offset, 0);
    assert_eq!(hdr.circle_id, a.circle_id());
    let reply = b.send(MessageType::Text, b"hi a").unwrap();
    assert_eq!(a.receive(&reply).unwrap().1, b"hi a");
}

#[test]
fn sender_erases_range_before_returning_message() {
    let (mut a, _) = two_members();
    let wire = a.send(MessageType::Text, b"abc").unwrap();
    assert_eq!(wire.len(), 25 + 3 + 16);
    let mut buf = [0u8; 35];
    a.storage().read(0, &mut buf);
    assert_eq!(buf, [0u8; 35], "key+stream erased");
    assert!(a.ledger().is_spent(0, 0, 35));
    assert_eq!(a.ledger().next_offset(0), 35);
}

#[test]
fn consecutive_sends_advance_offset() {
    let (mut a, mut b) = two_members();
    let w1 = a.send(MessageType::Text, b"one").unwrap();
    let w2 = a.send(MessageType::Text, b"two").unwrap();
    assert_eq!(b.receive(&w1).unwrap().1, b"one");
    assert_eq!(b.receive(&w2).unwrap().1, b"two");
    assert_eq!(w2[18..22], 35u32.to_le_bytes());
}

#[test]
fn receiver_flags_reuse_before_checking_tag() {
    let (mut a, mut b) = two_members();
    let wire = a.send(MessageType::Text, b"once").unwrap();
    b.receive(&wire).unwrap();
    assert_eq!(b.receive(&wire).unwrap_err(), PadError::Reuse);
    // A tampered replay is still reported as reuse, not as a tag failure.
    let mut t = wire.clone();
    t[30] ^= 1;
    assert_eq!(b.receive(&t).unwrap_err(), PadError::Reuse);
}

#[test]
fn receiver_does_not_spend_on_tag_failure() {
    let (mut a, mut b) = two_members();
    let mut wire = a.send(MessageType::Text, b"good").unwrap();
    wire[27] ^= 1;
    assert_eq!(
        b.receive(&wire).unwrap_err(),
        PadError::Message(MessageError::Tag)
    );
    assert!(!b.ledger().is_spent(0, 0, 36));
    let mut buf = [0u8; 36];
    b.storage().read(0, &mut buf);
    assert!(buf.iter().any(|&x| x != 0), "pad bytes untouched");
}

#[test]
fn receiver_erases_after_decrypt() {
    let (mut a, mut b) = two_members();
    let wire = a.send(MessageType::Text, b"gone").unwrap();
    b.receive(&wire).unwrap();
    let mut buf = [0u8; 36];
    b.storage().read(0, &mut buf);
    assert_eq!(buf, [0u8; 36]);
    assert!(b.ledger().is_spent(0, 0, 36));
}

#[test]
fn rejects_wrong_circle_and_exhausted_slice() {
    let (mut a, mut b) = two_members();
    let params = CircleParams::new(2, 2048, "other", false).unwrap();
    let mut c = Pad::new(
        params,
        0,
        MemPadStorage::new(vec![1u8; 2048]),
        MemLedger::new(2),
    )
    .unwrap();
    let wire = c.send(MessageType::Text, b"x").unwrap();
    assert_eq!(b.receive(&wire).unwrap_err(), PadError::WrongCircle);
    // slice is 1024 bytes; a 1000-byte message needs 1032
    assert_eq!(
        a.send(MessageType::Text, &[0u8; 1000]).unwrap_err(),
        PadError::Exhausted
    );
    assert_eq!(a.remaining(), 1024);
}

#[test]
fn receive_rejects_message_outside_slice_bounds() {
    let (mut a, mut b) = two_members();
    let mut wire = a.send(MessageType::Text, b"x").unwrap();
    wire[18..22].copy_from_slice(&2000u32.to_le_bytes());
    assert_eq!(b.receive(&wire).unwrap_err(), PadError::OutOfRange);
    wire[17] = 7;
    assert_eq!(b.receive(&wire).unwrap_err(), PadError::OutOfRange);
}

#[test]
fn padded_plaintext_hides_length() {
    use cahoots_core::pad::{bucket_pad, bucket_unpad};
    let p = bucket_pad(b"abc").unwrap();
    assert_eq!(p.len(), 32);
    assert_eq!(bucket_unpad(&p).unwrap(), b"abc");
    let p = bucket_pad(&[1u8; 40]).unwrap();
    assert_eq!(p.len(), 64);
    let p = bucket_pad(&[1u8; 200]).unwrap();
    assert_eq!(p.len(), 256);
    assert_eq!(bucket_pad(&[1u8; 255]).unwrap().len(), 256);
    assert!(
        bucket_pad(&[1u8; 256]).is_none(),
        "largest bucket holds 255 bytes plus the length byte"
    );
    assert_eq!(bucket_unpad(&bucket_pad(b"").unwrap()).unwrap(), b"");
}
