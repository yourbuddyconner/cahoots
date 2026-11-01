//! The software chip's APDU front end refuses what the applet refuses.

use cahoots_chip::soft::APPLET_ID;
use cahoots_chip::{ChipOps, KeyId, KeyKind, ObjectPolicy, SoftChip, TEST_OWNER_SECRET};

fn sw(answer: &[u8]) -> u16 {
    u16::from_be_bytes([answer[answer.len() - 2], answer[answer.len() - 1]])
}

fn select() -> Vec<u8> {
    let mut a = vec![0x00, 0xA4, 0x04, 0x00, 0x10];
    a.extend_from_slice(&APPLET_ID);
    a.push(0);
    a
}

fn create_session(id: KeyId) -> Vec<u8> {
    let mut a = vec![0x80, 0x04, 0x00, 0x1B, 0x06, 0x41, 0x04];
    a.extend_from_slice(&id.0.to_be_bytes());
    a.push(0);
    a
}

fn in_session(sid: &[u8], inner: &[u8]) -> Vec<u8> {
    let mut d = vec![0x10, 0x08];
    d.extend_from_slice(sid);
    d.extend_from_slice(&[0x41, inner.len() as u8]);
    d.extend_from_slice(inner);
    let mut a = vec![0x80, 0x05, 0x00, 0x00, d.len() as u8];
    a.extend_from_slice(&d);
    a.push(0);
    a
}

fn chip() -> SoftChip {
    let mut c = SoftChip::with_seed([3; 32]);
    c.put_user_id(KeyId::OWNER, &TEST_OWNER_SECRET, &ObjectPolicy::owner_object()).unwrap();
    c.put_aes_auth(KeyId::SLOT, &[1; 16], &ObjectPolicy::placeholder_slot()).unwrap();
    c.put_hmac_key(KeyId::new(KeyKind::Root, 0), &[2; 32], &ObjectPolicy::owner_and_slot_mac()).unwrap();
    c
}

#[test]
fn nothing_answers_before_the_applet_is_selected() {
    let mut c = chip();
    assert_eq!(sw(&c.apdu(&create_session(KeyId::SLOT))), 0x6E00);
    let mut wrong = select();
    wrong[6] ^= 1;
    assert_eq!(sw(&c.apdu(&wrong)), 0x6A82);
    let a = c.apdu(&select());
    assert_eq!(sw(&a), 0x9000);
    assert_eq!(&a[..3], &[7, 2, 0]);
}

#[test]
fn a_session_opens_only_against_an_aes_key_and_never_while_the_badge_holds_the_chip() {
    let mut c = chip();
    c.apdu(&select());
    // The applet itself opens a session against the owner's user id and refuses at
    // VerifySessionUserId without the secret; the software chip, which does not model user-id
    // sessions over APDUs, refuses at once.
    assert_eq!(sw(&c.apdu(&create_session(KeyId::OWNER))), 0x6985);
    // An id that holds nothing: 6985, as the applet answers (bench), not 6A82.
    assert_eq!(sw(&c.apdu(&create_session(KeyId::new(KeyKind::Root, 9)))), 0x6985);
    c.open_owner(&TEST_OWNER_SECRET).unwrap();
    assert_eq!(sw(&c.apdu(&create_session(KeyId::SLOT))), 0x6985);
    c.close_session().unwrap();
    assert_eq!(sw(&c.apdu(&create_session(KeyId::SLOT))), 0x9000);
}

#[test]
fn a_host_without_the_key_is_refused_and_the_session_ends() {
    let mut c = chip();
    c.apdu(&select());
    let a = c.apdu(&create_session(KeyId::SLOT));
    // `41 82 00 08` and the id: the applet writes this length in its two-byte form.
    assert_eq!(&a[..4], &[0x41, 0x82, 0x00, 0x08]);
    let sid = a[4..12].to_vec();
    // Out of order: EXTERNAL AUTHENTICATE before INITIALIZE UPDATE.
    let mut auth = vec![0x84, 0x82, 0x33, 0x00, 0x10];
    auth.extend_from_slice(&[0; 16]);
    assert_eq!(sw(&c.apdu(&in_session(&sid, &auth))), 0x6985);
    let init = [0x80, 0x50, 0x00, 0x00, 0x08, 1, 2, 3, 4, 5, 6, 7, 8, 0x00];
    let a = c.apdu(&in_session(&sid, &init));
    assert_eq!(sw(&a), 0x9000);
    assert_eq!(a.len(), 29 + 2);
    assert_eq!(sw(&c.apdu(&in_session(&sid, &auth))), 0x6982);
    // The session is gone.
    assert_eq!(sw(&c.apdu(&in_session(&sid, &init))), 0x6985);
}

#[test]
fn a_command_outside_a_session_runs_in_no_session() {
    let mut c = chip();
    c.apdu(&select());
    let mut mac = vec![0x80, 0x03, 0x0D, 0x45, 0x0C, 0x41, 0x04];
    mac.extend_from_slice(&KeyId::new(KeyKind::Root, 0).0.to_be_bytes());
    mac.extend_from_slice(&[0x42, 0x01, 0x19, 0x43, 0x01, 0x78, 0x00]);
    assert_eq!(sw(&c.apdu(&mac)), 0x6986);
}
