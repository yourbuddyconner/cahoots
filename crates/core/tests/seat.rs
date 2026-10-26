//! Seat keys: P-256 ECDSA, deterministic signatures, compressed public keys.

use cahoots_core::seat::{SeatError, SeatKey, SeatPublic};

const SEED: [u8; 32] = [7u8; 32];

#[test]
fn a_seed_round_trips_through_to_bytes() {
    let key = SeatKey::from_seed(&SEED).unwrap();
    let bytes = key.to_bytes();
    assert_eq!(*bytes, SEED);
    let again = SeatKey::from_seed(&bytes).unwrap();
    assert_eq!(again.public(), key.public());
}

#[test]
fn zero_seed_is_rejected() {
    assert!(matches!(
        SeatKey::from_seed(&[0u8; 32]),
        Err(SeatError::BadSeed)
    ));
    // At or above the curve order is rejected too.
    assert!(matches!(
        SeatKey::from_seed(&[0xffu8; 32]),
        Err(SeatError::BadSeed)
    ));
}

#[test]
fn sign_and_verify() {
    let key = SeatKey::from_seed(&SEED).unwrap();
    let sig = key.sign(b"hello circle");
    assert!(key.public().verify(b"hello circle", &sig));
    assert!(!key.public().verify(b"hello circlf", &sig));
}

#[test]
fn a_flipped_bit_fails_verification() {
    let key = SeatKey::from_seed(&SEED).unwrap();
    let msg = b"a message";
    let sig = key.sign(msg);
    for i in [0usize, 31, 32, 63] {
        let mut bad = sig;
        bad[i] ^= 0x01;
        assert!(!key.public().verify(msg, &bad), "flip at byte {i}");
    }
    let other = SeatKey::from_seed(&[9u8; 32]).unwrap();
    assert!(!other.public().verify(msg, &sig));
}

#[test]
fn public_key_is_33_bytes_compressed_and_parses() {
    let key = SeatKey::from_seed(&SEED).unwrap();
    let public = key.public();
    assert_eq!(public.0.len(), 33);
    assert!(public.0[0] == 0x02 || public.0[0] == 0x03);
    assert_eq!(SeatPublic::parse(&public.0).unwrap(), public);
    assert!(matches!(
        SeatPublic::parse(&public.0[..32]),
        Err(SeatError::BadPublic)
    ));
    let mut bad = public.0;
    bad[0] = 0x04;
    assert!(SeatPublic::parse(&bad).is_err());
    assert!(SeatPublic::parse(&[0u8; 33]).is_err());
}

#[test]
fn signatures_are_deterministic() {
    let key = SeatKey::from_seed(&SEED).unwrap();
    assert_eq!(key.sign(b"same"), key.sign(b"same"));
    assert_ne!(key.sign(b"same"), key.sign(b"different"));
}
