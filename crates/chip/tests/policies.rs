//! The software chip enforces policies and sessions the way the real chip does: what a tap in
//! no session, or a phone in the slot's session, can and cannot do.

use cahoots_chip::{
    ChipError, ChipOps, Curve, KeyId, KeyKind, ObjectPolicy, SoftChip, TEST_OWNER_SECRET,
};
use sha2::{Digest, Sha256};

const SEAT: KeyId = KeyId::new(KeyKind::Seat, 0);
const ROOT: KeyId = KeyId::new(KeyKind::Root, 0);
const PUBLIC: KeyId = KeyId::new(KeyKind::Public, 0);

/// A chip set up as a badge sets it up: the owner object, the phone slot holding a key, the
/// inbox; in the owner session.
fn badge_chip() -> SoftChip {
    let mut c = SoftChip::with_seed([5; 32]);
    c.put_user_id(KeyId::OWNER, &TEST_OWNER_SECRET, &ObjectPolicy::owner_object())
        .unwrap();
    c.open_owner(&TEST_OWNER_SECRET).unwrap();
    c.put_aes_auth(KeyId::SLOT, &[3; 16], &ObjectPolicy::placeholder_slot())
        .unwrap();
    c.write_binary(KeyId::INBOX, &[0; 64], &ObjectPolicy::inbox()).unwrap();
    c
}

fn digest() -> [u8; 32] {
    Sha256::digest(b"a commit").into()
}

#[test]
fn a_seat_key_signs_for_the_owner_alone() {
    let mut c = badge_chip();
    c.generate(SEAT, Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    assert!(c.sign_p256(SEAT, &digest()).is_ok());
    assert!(c.public(SEAT).is_ok());

    c.open_slot_with_sm_for_test().unwrap();
    assert_eq!(c.sign_p256(SEAT, &digest()), Err(ChipError::NotAllowed));
    assert_eq!(c.public(SEAT), Err(ChipError::NotAllowed));

    c.no_session_for_test();
    assert_eq!(c.sign_p256(SEAT, &digest()), Err(ChipError::NotAllowed));
    assert_eq!(c.attest(SEAT, &[0; 16]), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(SEAT), Err(ChipError::NotAllowed));
}

#[test]
fn a_root_macs_for_the_owner_and_for_the_slot_under_secure_messaging() {
    let mut c = badge_chip();
    c.put_hmac_key(ROOT, &[9; 32], &ObjectPolicy::owner_and_slot_mac())
        .unwrap();
    let mac = c.hmac(ROOT, b"chat/gen").unwrap();

    c.open_slot_with_sm_for_test().unwrap();
    assert_eq!(c.hmac(ROOT, b"chat/gen"), Ok(mac));
    assert_eq!(c.delete(ROOT), Err(ChipError::NotAllowed), "the slot cannot delete it");

    c.open_slot_without_sm_for_test().unwrap();
    assert_eq!(c.hmac(ROOT, b"chat/gen"), Err(ChipError::NotAllowed));

    c.no_session_for_test();
    assert_eq!(c.hmac(ROOT, b"chat/gen"), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(ROOT), Err(ChipError::NotAllowed));
}

#[test]
fn the_public_object_reads_in_no_session_and_refuses_a_write_there() {
    let mut c = badge_chip();
    c.write_binary(PUBLIC, b"circle", &ObjectPolicy::public_object())
        .unwrap();
    c.no_session_for_test();
    assert_eq!(c.read_binary(PUBLIC).unwrap(), b"circle");
    assert_eq!(c.rewrite_binary(PUBLIC, b"forged"), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(PUBLIC), Err(ChipError::NotAllowed));
    c.open_owner(&TEST_OWNER_SECRET).unwrap();
    c.rewrite_binary(PUBLIC, b"CIRCLE").unwrap();
    assert_eq!(c.read_binary(PUBLIC).unwrap(), b"CIRCLE");
}

#[test]
fn the_inbox_takes_a_write_in_no_session_and_refuses_a_read_there() {
    let mut c = badge_chip();
    c.no_session_for_test();
    c.rewrite_binary(KeyId::INBOX, b"a phone's key").unwrap();
    assert_eq!(c.read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::INBOX), Err(ChipError::NotAllowed));
    c.open_owner(&TEST_OWNER_SECRET).unwrap();
    assert!(c.read_binary(KeyId::INBOX).unwrap().starts_with(b"a phone's key"));
}

#[test]
fn no_session_cannot_take_the_slot_or_the_owner_id() {
    let mut c = badge_chip();
    c.no_session_for_test();
    assert_eq!(
        c.put_aes_auth(KeyId::SLOT, &[6; 16], &ObjectPolicy::any_session()),
        Err(ChipError::Exists)
    );
    assert_eq!(
        c.put_user_id(KeyId::OWNER, &[6; 16], &ObjectPolicy::any_session()),
        Err(ChipError::Exists)
    );
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::OWNER), Err(ChipError::NotAllowed));
    // The tap's key does not open the slot's session as the owner's either.
    assert_eq!(c.open_owner(&[6; 16]), Err(ChipError::BadSession));
}

#[test]
fn the_owner_object_is_never_deleted_and_the_placeholder_only_by_the_owner() {
    let mut c = badge_chip();
    assert_eq!(c.delete(KeyId::OWNER), Err(ChipError::NotAllowed));
    c.open_slot_with_sm_for_test().unwrap();
    assert_eq!(c.delete(KeyId::OWNER), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    c.open_owner(&TEST_OWNER_SECRET).unwrap();
    c.delete(KeyId::SLOT).unwrap();
    assert!(!c.exists(KeyId::SLOT).unwrap());
}

#[test]
fn open_owner_with_the_wrong_secret_fails() {
    let mut c = badge_chip();
    c.close_session().unwrap();
    let mut wrong = TEST_OWNER_SECRET;
    wrong[0] ^= 1;
    assert_eq!(c.open_owner(&wrong), Err(ChipError::BadSession));
    c.generate(SEAT, Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    assert_eq!(c.sign_p256(SEAT, &digest()), Err(ChipError::NotAllowed));
    assert_eq!(
        SoftChip::with_seed([1; 32]).open_owner(&TEST_OWNER_SECRET),
        Err(ChipError::NoSuchKey),
        "no owner object yet"
    );
}

#[test]
fn a_session_the_policy_names_gets_nothing_from_the_no_session_entry() {
    // As the applet does it on the bench: the owner is named (for delete), so the no-session
    // entry's sign is not the owner's; a session the policy does not name gets that entry.
    let mut c = badge_chip();
    let p = ObjectPolicy {
        sign: vec![cahoots_chip::Identity::None],
        delete: vec![cahoots_chip::Identity::Owner],
        ..ObjectPolicy::placeholder_slot()
    };
    c.put_hmac_key(ROOT, &[9; 32], &p).unwrap();
    assert_eq!(c.hmac(ROOT, b"x"), Err(ChipError::NotAllowed));
    c.open_slot_with_sm_for_test().unwrap();
    assert!(c.hmac(ROOT, b"x").is_ok(), "the slot is not named: the no-session entry");
    c.no_session_for_test();
    assert!(c.hmac(ROOT, b"x").is_ok());
}

#[test]
fn a_counter_takes_no_sign_or_attest_rights() {
    // As the applet answers on the bench: 6A80.
    let mut c = badge_chip();
    let id = KeyId::new(KeyKind::Epoch, 0);
    assert_eq!(
        c.put_counter(id, 0, &ObjectPolicy::owner_only()),
        Err(ChipError::Status(0x6A80))
    );
    c.put_counter(id, 0, &ObjectPolicy::owner_counter()).unwrap();
    assert_eq!(c.counter_next(id), Ok(1));
    c.no_session_for_test();
    assert_eq!(c.counter_next(id), Err(ChipError::NotAllowed));
    assert_eq!(c.counter_get(id), Err(ChipError::NotAllowed));
}

#[test]
fn creating_never_replaces() {
    let mut c = badge_chip();
    c.generate(SEAT, Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    assert_eq!(
        c.generate(SEAT, Curve::P256, &ObjectPolicy::owner_only()),
        Err(ChipError::Exists)
    );
    assert_eq!(
        c.put_counter(SEAT, 0, &ObjectPolicy::owner_counter()),
        Err(ChipError::Exists)
    );
}

#[test]
fn the_owner_secret_follows_from_the_board_id() {
    let a = cahoots_chip::owner_secret(&[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(a, cahoots_chip::owner_secret(&[1, 2, 3, 4, 5, 6, 7, 8]));
    assert_ne!(a, cahoots_chip::owner_secret(&[1, 2, 3, 4, 5, 6, 7, 9]));
    // HKDF-SHA256 with salt "cahoots/owner/v1" and no info, the first 16 bytes.
    use hkdf::Hkdf;
    let mut want = [0u8; 16];
    Hkdf::<Sha256>::new(Some(b"cahoots/owner/v1"), &[1, 2, 3, 4, 5, 6, 7, 8])
        .expand(&[], &mut want)
        .unwrap();
    assert_eq!(a, want);
}
