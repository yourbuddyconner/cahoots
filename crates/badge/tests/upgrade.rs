//! The first boot of firmware that owns its chip: every object made before it named every
//! session, so the chip and the circles are wiped, the chip is set up anew, and the badge
//! says so once.

mod common;

use cahoots_badge::chip::ChipRecord;
use cahoots_badge::grants::GrantsRecord;
use cahoots_badge::{Badge, Board};
use cahoots_chip::{
    ChipError, ChipOps, Curve, KeyId, KeyKind, ObjectPolicy, Session, SoftChip, TEST_OWNER_SECRET,
};
use common::FakeBoard;

const SCREEN: &str = "updated for phones. Make a new circle.";
const REFUSED: &str = "this chip was touched before the badge owned it. It cannot be used. Circles on this badge keep software keys.";
const NOT_NOW: &str = "the chip could not be set up. Circles made now keep software keys.";

fn not_now(b: &mut Badge<FakeBoard>) -> usize {
    b.board().statuses().iter().filter(|s| *s == NOT_NOW).count()
}

/// The next boot: the board hands its chip out again until the badge says otherwise.
fn reboot(b: &mut Badge<FakeBoard>) {
    b.board().chip_refused = false;
    b.boot();
}

fn refused(b: &mut Badge<FakeBoard>) -> usize {
    b.board().statuses().iter().filter(|s| *s == REFUSED).count()
}

/// Every right for every session but delete, which nobody has: an object a tap made so that
/// it stays.
fn stuck() -> ObjectPolicy {
    ObjectPolicy {
        delete: Vec::new(),
        ..ObjectPolicy::any_session()
    }
}

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
    assert_eq!(
        ids,
        vec![KeyId::new(KeyKind::Grant, 255), KeyId::SLOT, KeyId::INBOX, KeyId::OWNER]
    );
    // The issue counter is new, at 0, for the owner alone.
    assert_eq!(b.board().owner_chip().counter_get(KeyId::new(KeyKind::Grant, 255)), Ok(0));

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
    b.boot();
    pin_rests(&mut b);
    assert_eq!(shown(&mut b), 1);
    assert!(!b.board().log.iter().any(|l| l.contains("upgrade: 0 chip objects")));
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
    assert_eq!(
        ids,
        vec![KeyId::new(KeyKind::Grant, 255), KeyId::SLOT, KeyId::INBOX, KeyId::OWNER]
    );
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

#[test]
fn a_slot_and_an_inbox_a_tap_made_before_the_first_boot_are_not_adopted() {
    let mut board = old_board();
    let c = board.chip.as_mut().unwrap();
    // A tap, in no session, made a phone slot with a key it knows and an inbox anyone reads.
    c.put_aes_auth(KeyId::SLOT, &[0x42; 16], &ObjectPolicy::any_session())
        .unwrap();
    c.write_binary(KeyId::INBOX, b"planted", &ObjectPolicy::any_session())
        .unwrap();
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(shown(&mut b), 1);
    assert_eq!(refused(&mut b), 0);
    // Both were deleted and made again by the badge, with its policies: a tap in no session
    // can neither delete the slot nor read the inbox now.
    let c = b.board().chip.as_mut().unwrap();
    assert_eq!(c.session(), Session::None);
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    assert_eq!(c.read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    let inbox = b.board().owner_chip().read_binary(KeyId::INBOX).unwrap();
    assert!(inbox.iter().all(|&x| x == 0), "a new, empty inbox");
    let log = b.board().log.join("\n");
    assert!(log.contains("upgrade: 6 chip objects made before the badge owned its chip deleted"), "{log}");
}

#[test]
fn an_object_a_tap_made_that_cannot_be_deleted_leaves_the_chip_unowned() {
    for planted in [KeyId::SLOT, KeyId::INBOX, KeyId::new(KeyKind::Grant, 255)] {
        let mut board = old_board();
        let c = board.chip.as_mut().unwrap();
        // The old issue counter goes first, so the tap's stuck one can sit at its id.
        c.delete(KeyId::new(KeyKind::Grant, 255)).unwrap();
        match planted {
            KeyId::SLOT => c.put_aes_auth(planted, &[0x42; 16], &stuck()).unwrap(),
            KeyId::INBOX => c.write_binary(planted, b"planted", &stuck()).unwrap(),
            _ => c
                .put_counter(planted, 0, &ObjectPolicy { sign: Vec::new(), attest: Vec::new(), ..stuck() })
                .unwrap(),
        }
        let mut b = Badge::new(board);
        b.boot();
        assert_eq!(refused(&mut b), 1, "{planted:?}");
        assert_eq!(shown(&mut b), 0);
        // The circles still go: their keys were reachable by any session.
        assert!(b.circles().is_empty());
        // No owner object, no record, and none of the objects a tap reaches made; the board
        // hands the chip out no more.
        let c = b.board().chip.as_mut().unwrap();
        assert!(!c.exists(KeyId::OWNER).unwrap(), "{planted:?}");
        for id in [KeyId::SLOT, KeyId::INBOX] {
            assert_eq!(c.exists(id).unwrap(), id == planted, "{planted:?} {id:?}");
        }
        assert!(b.board().chip_record_load().unwrap().unwrap().is_refused());
        assert!(!b.board().has_chip());
        assert!(b.board().chip_begin().is_none());
        assert!(!b.board().pin_high());
        let log = b.board().log.join("\n");
        assert!(log.contains("chip: not owned, never used"), "{log}");
    }
}

#[test]
fn an_owner_object_that_is_not_this_badges_leaves_the_chip_unowned() {
    let mut board = old_board();
    board
        .chip
        .as_mut()
        .unwrap()
        .put_user_id(KeyId::OWNER, &[0x42; 16], &ObjectPolicy::owner_object())
        .unwrap();
    let mut b = Badge::new(board);
    b.boot();
    // Not this badge's: refused this boot, with nothing touched, and tried again at the next.
    assert_eq!(not_now(&mut b), 1);
    assert!(!b.board().has_chip());
    assert_eq!(b.circles().len(), 1, "nothing wiped");
    let c = b.board().chip.as_mut().unwrap();
    assert!(!c.exists(KeyId::SLOT).unwrap());
    assert!(!c.exists(KeyId::INBOX).unwrap());
    // The old objects are left as they are: nothing was done in a chip the badge does not own.
    assert!(c.exists(KeyId::new(KeyKind::Seat, 0)).unwrap());
}

#[test]
fn a_current_record_on_a_chip_with_no_owner_object_is_set_up_again() {
    // The chip lost every object (a factory reset, or another chip): the record cannot be
    // trusted, so the badge starts over.
    let mut board = FakeBoard::new().with_soft_chip([6; 32]);
    board.chip = Some(SoftChip::with_seed([7; 32]));
    let mut b = Badge::new(board);
    b.boot();
    assert!(b.board().owner_chip().exists(KeyId::INBOX).unwrap());
    assert!(b.board().owner_chip().exists(KeyId::new(KeyKind::Grant, 255)).unwrap());
    assert_eq!(refused(&mut b), 0);
}

#[test]
fn a_refused_chip_stays_refused_and_software_circles_survive_the_next_boot() {
    let mut board = old_board();
    board
        .chip
        .as_mut()
        .unwrap()
        .put_aes_auth(KeyId::SLOT, &[0x42; 16], &stuck())
        .unwrap();
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(refused(&mut b), 1);
    let record = b.board().chip_record_load().unwrap().unwrap();
    assert!(record.is_refused());
    // A circle made while the chip is refused keeps software keys.
    let (soft, _, pad) = FakeBoard::with_circle();
    let circle = soft.circle.clone().unwrap();
    assert_eq!(circle.chip_slot, None);
    b.board().circle = Some(circle.clone());
    b.board().pad = pad;
    let log_before = b.board().log.len();
    reboot(&mut b);
    assert_eq!(refused(&mut b), 2, "said again");
    assert_eq!(b.circles(), &[circle]);
    assert!(!b.board().has_chip());
    let log = b.board().log[log_before..].join("\n");
    assert!(log.contains("chip: refused at an earlier boot"), "{log}");
    assert!(!log.contains("upgrade:"), "no second wipe: {log}");
}

#[test]
fn a_random_source_that_fails_leaves_everything_and_the_next_boot_tries_again() {
    let mut board = old_board();
    let before = board.chip_record.clone();
    board.entropy = cahoots_hal::health::Tested::new(
        common::Xorshift(0),
        cahoots_hal::health::HealthTests::for_min_entropy(6),
    );
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(not_now(&mut b), 1);
    assert_eq!(shown(&mut b), 0);
    // Nothing deleted, nothing made, the old record as it was, and the chip not handed out.
    assert!(!b.board().has_chip());
    assert!(!b.board().pin_high());
    assert_eq!(b.board().chip_record, before);
    assert_eq!(b.circles().len(), 1);
    let c = b.board().chip.as_mut().unwrap();
    assert!(c.exists(KeyId::new(KeyKind::Seat, 0)).unwrap());
    assert!(!c.exists(KeyId::OWNER).unwrap());
    assert!(!c.exists(KeyId::SLOT).unwrap());
    // The next boot, with a working source, upgrades.
    b.board().entropy = cahoots_hal::health::Tested::new(
        common::Xorshift(5),
        cahoots_hal::health::HealthTests::for_min_entropy(6),
    );
    reboot(&mut b);
    assert_eq!(shown(&mut b), 1);
    assert!(b.board().has_chip());
    assert!(b.board().chip_record_load().unwrap().unwrap().is_current());
}

#[test]
fn a_setup_that_fails_after_the_owner_object_is_refused_for_good() {
    let mut board = old_board();
    // The owner object is made, the phone slot is not.
    board.chip.as_mut().unwrap().fail_create_after_for_test(1);
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(refused(&mut b), 1);
    let c = b.board().chip.as_mut().unwrap();
    assert!(c.exists(KeyId::OWNER).unwrap());
    assert!(!c.exists(KeyId::SLOT).unwrap());
    assert!(b.board().chip_record_load().unwrap().unwrap().is_refused());
    assert!(!b.board().pin_high());
    reboot(&mut b);
    assert_eq!(refused(&mut b), 2);
    assert!(!b.board().has_chip());
    assert!(!b.board().chip.as_mut().unwrap().exists(KeyId::SLOT).unwrap());
}

#[test]
fn a_setup_that_fails_before_the_owner_object_is_tried_again() {
    let mut board = old_board();
    board.chip.as_mut().unwrap().fail_next_create_for_test();
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(not_now(&mut b), 1);
    assert!(!b.board().has_chip());
    assert!(!b.board().chip.as_mut().unwrap().exists(KeyId::OWNER).unwrap());
    assert_eq!(b.board().chip_record_load(), Ok(None));
    reboot(&mut b);
    assert!(b.board().has_chip());
    assert!(b.board().chip_record_load().unwrap().unwrap().is_current());
}
