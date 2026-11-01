//! The first boot of firmware that owns its chip: every object made before it named every
//! session, so the chip and the circles are wiped, the chip is set up anew, and the badge
//! says so once.

mod common;

use cahoots_badge::chip::{ChipRecord, SlotEntry};
use cahoots_badge::grants::GrantsRecord;
use cahoots_badge::{Badge, Board};
use cahoots_chip::{
    ChipError, ChipOps, Curve, KeyId, KeyKind, ObjectPolicy, Session, SoftChip, TEST_OWNER_SECRET,
};
use common::FakeBoard;

const SCREEN: &str = "updated for phones. Make a new circle.";

/// A board as earlier firmware left it: a circle in chip slot 0, its seat key, root and epoch
/// counter and the grant issue counter made with every right for every session, and the chip
/// record in a `CAHC` sector.
fn old_board() -> FakeBoard {
    let (mut board, _, _) = FakeBoard::with_circle();
    let any = ObjectPolicy::any_session();
    let mut c = SoftChip::with_seed([8; 32]);
    c.generate(KeyId::new(KeyKind::Seat, 0), Curve::P256, &any).unwrap();
    c.put_hmac_key(KeyId::new(KeyKind::Root, 0), &[4; 32], &any).unwrap();
    // Earlier firmware made counters with no policy: every session may read, move or
    // delete them.
    let counter = ObjectPolicy {
        sign: Vec::new(),
        attest: Vec::new(),
        ..ObjectPolicy::any_session()
    };
    c.put_counter(KeyId::new(KeyKind::Epoch, 0), 0, &counter).unwrap();
    c.put_counter(KeyId::new(KeyKind::Grant, 255), 3, &counter).unwrap();
    board.chip = Some(c);
    let circle = board.circle.as_mut().unwrap();
    circle.chip_slot = Some(0);
    circle.seat_key = [0; 32];
    // `[[[0, circle id, true, true, true]]]` after `CAHC` and its length.
    let mut body = vec![0x81, 0x81, 0x85, 0x00, 0x50];
    body.extend_from_slice(&circle.circle_id);
    body.extend_from_slice(&[0xF5, 0xF5, 0xF5]);
    let mut sector = b"CAHC".to_vec();
    sector.extend_from_slice(&(body.len() as u16).to_le_bytes());
    sector.extend_from_slice(&body);
    board.chip_record = Some(sector);
    board.grants = Some(GrantsRecord::default().encode());
    board
}

fn shown(b: &mut Badge<FakeBoard>) -> usize {
    b.board().statuses().iter().filter(|s| *s == SCREEN).count()
}

/// The pin went high and low in turns and is low now.
fn pin_rests(b: &mut Badge<FakeBoard>) {
    let log = &b.board().pin_log;
    assert!(!log.is_empty(), "the chip was used");
    for (i, high) in log.iter().enumerate() {
        assert_eq!(*high, i % 2 == 0, "high only between begin and end: {log:?}");
    }
    assert!(!b.board().pin_high());
    assert_eq!(b.board().chip.as_ref().unwrap().session(), Session::None);
}

#[test]
fn the_first_boot_wipes_what_was_made_the_old_way_and_says_so_once() {
    let mut b = Badge::new(old_board());
    b.boot();
    pin_rests(&mut b);

    // The circle is gone, with its pad, and the chip holds only the badge's own objects.
    assert!(b.circles().is_empty());
    assert!(b.board().load_circles().is_empty());
    assert!(b.board().pad.iter().all(|&x| x == 0));
    let ids = b.board().owner_chip().ids().unwrap();
    assert_eq!(ids, vec![KeyId::SLOT, KeyId::INBOX, KeyId::OWNER]);

    // The record is CAHC2, of this generation, with no slot and a nonce.
    let stored = b.board().chip_record.clone().unwrap();
    assert!(stored.starts_with(b"CAHC2"));
    let record = ChipRecord::from_stored(&stored).unwrap().unwrap();
    assert!(record.is_current());
    assert!(record.slots.is_empty());
    assert_eq!(record.phone, None);
    assert_ne!(record.badge_nonce, [0; 16]);
    assert_eq!(b.board().grants_load(), Ok(Some(GrantsRecord::default())));

    // The badge said so, once.
    assert_eq!(shown(&mut b), 1);
    let log = b.board().log.join("\n");
    assert!(log.contains("upgrade: 4 chip objects made before the badge owned its chip deleted"), "{log}");

    // The badge owns the chip now: its secret opens the owner session, a tap's does not, and
    // a tap cannot take or remove the owner object or the phone slot.
    let c = b.board().chip.as_mut().unwrap();
    assert_eq!(c.open_owner(&[0; 16]), Err(ChipError::BadSession));
    assert_eq!(c.put_user_id(KeyId::OWNER, &[0; 16], &ObjectPolicy::any_session()), Err(ChipError::Exists));
    assert_eq!(c.put_aes_auth(KeyId::SLOT, &[0; 16], &ObjectPolicy::any_session()), Err(ChipError::Exists));
    assert_eq!(c.delete(KeyId::OWNER), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    assert!(c.open_owner(&TEST_OWNER_SECRET).is_ok());
    c.close_session().unwrap();

    // The next boot is an ordinary one: nothing wiped, nothing said.
    let c = b.board().owner_chip();
    c.generate(KeyId::new(KeyKind::Seat, 0), Curve::P256, &ObjectPolicy::owner_only())
        .unwrap();
    b.board().record_slot(SlotEntry {
        slot: 0,
        circle_id: [3; 16],
        has_seat: true,
        has_root: false,
        has_epoch: false,
    });
    b.boot();
    pin_rests(&mut b);
    assert_eq!(shown(&mut b), 1);
    assert!(b.board().owner_chip().exists(KeyId::new(KeyKind::Seat, 0)).unwrap());
    let again = b.board().chip_record_load().unwrap().unwrap();
    assert_eq!(again.badge_nonce, record.badge_nonce);
}

#[test]
fn a_new_badge_is_set_up_without_a_word() {
    let mut board = FakeBoard::new();
    board.chip = Some(SoftChip::with_seed([2; 32]));
    let mut b = Badge::new(board);
    b.boot();
    pin_rests(&mut b);
    assert_eq!(shown(&mut b), 0);
    let ids = b.board().owner_chip().ids().unwrap();
    assert_eq!(ids, vec![KeyId::SLOT, KeyId::INBOX, KeyId::OWNER]);
    assert!(b.board().chip_record_load().unwrap().unwrap().is_current());
}

#[test]
fn a_lost_badge_wide_object_is_made_again_at_boot() {
    let mut board = FakeBoard::new().with_soft_chip([6; 32]);
    let before = board.chip_record.clone();
    board.owner_chip().delete(KeyId::INBOX).unwrap();
    let mut b = Badge::new(board);
    b.boot();
    assert!(b.board().owner_chip().exists(KeyId::INBOX).unwrap());
    assert_eq!(b.board().chip_record, before, "no wipe, the same record");
    assert_eq!(shown(&mut b), 0);
}
