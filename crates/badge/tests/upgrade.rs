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
const REFUSED: &str = "the chip was touched or could not be set up. This chip cannot be used. Circles on this badge keep software keys.";
const NOT_NOW: &str = "the chip could not be set up. Try turning the badge off and on. Circles made now keep software keys.";

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

/// The chip's objects but the placeholders.
fn own_objects(b: &mut Badge<FakeBoard>) -> Vec<KeyId> {
    let c = b.board().owner_chip();
    let ids = c.ids().unwrap();
    ids.into_iter().filter(|id| !common::is_placeholder(c, *id)).collect()
}

/// The badge-wide objects a set-up chip holds, in id order.
fn badge_wide() -> Vec<KeyId> {
    vec![KeyId::new(KeyKind::Grant, 255), KeyId::SLOT, KeyId::PAIRING, KeyId::INBOX, KeyId::OWNER]
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
    assert_eq!(own_objects(&mut b), badge_wide());
    // And a placeholder at every circle slot's public id.
    assert_eq!(common::empty_public_ids(b.board().owner_chip()), Vec::<KeyId>::new());
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
    assert_eq!(own_objects(&mut b), badge_wide());
    assert_eq!(common::empty_public_ids(b.board().owner_chip()), Vec::<KeyId>::new());
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
    // Not this badge's: refused with nothing touched, and no restart is offered as a remedy.
    assert_eq!(not_now(&mut b), 0);
    assert_eq!(b.board().statuses().iter().filter(|s| *s == FOREIGN).count(), 1);
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

/// A chip this badge set up whose record says the phone slot, inbox and grant counter must be
/// made again (a conversation that emptied one was cut off), and whose slot a tap filled while
/// the pin rested low.
fn remake_board(planted_policy: ObjectPolicy) -> FakeBoard {
    let mut board = FakeBoard::new().with_soft_chip([6; 32]);
    let mut r = board.chip_record_load().unwrap().unwrap();
    r.generation = cahoots_badge::chip::REMAKE_GENERATION;
    board.chip_record_store(&r).unwrap();
    board.owner_chip().delete(KeyId::SLOT).unwrap();
    let c = board.chip.as_mut().unwrap();
    c.close_session().unwrap();
    c.put_aes_auth(KeyId::SLOT, &[0x42; 16], &planted_policy).unwrap();
    board
}

#[test]
fn a_slot_a_tap_filled_while_it_was_empty_is_deleted_and_made_again() {
    let mut b = Badge::new(remake_board(ObjectPolicy::any_session()));
    b.boot();
    pin_rests(&mut b);
    assert_eq!(refused(&mut b), 0);
    assert_eq!(not_now(&mut b), 0);
    assert!(b.board().has_chip());
    // The slot is the badge's again: a tap in no session cannot delete it.
    let c = b.board().chip.as_mut().unwrap();
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    let r = b.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.generation, cahoots_badge::chip::GENERATION);
    assert!(b
        .board()
        .log
        .iter()
        .any(|l| l == "chip: the phone slot, the inbox, the grant counter and the pairing object were made again"));
    // The issue counter was made again too, with the badge's policy.
    assert_eq!(b.board().owner_chip().counter_get(KeyId::new(KeyKind::Grant, 255)), Ok(0));
}

#[test]
fn a_slot_a_tap_filled_that_cannot_be_deleted_refuses_the_chip_for_good() {
    let mut b = Badge::new(remake_board(stuck()));
    b.boot();
    assert_eq!(refused(&mut b), 1);
    assert!(!b.board().has_chip());
    assert!(b.board().chip_record_load().unwrap().unwrap().is_refused());
    reboot(&mut b);
    assert_eq!(refused(&mut b), 2);
    assert!(!b.board().has_chip());
}

#[test]
fn a_missing_inbox_on_an_owned_chip_brings_all_three_back_new() {
    let mut board = FakeBoard::new().with_soft_chip([6; 32]);
    board.owner_chip().delete(KeyId::INBOX).unwrap();
    let mut b = Badge::new(board);
    b.boot();
    assert!(b.board().has_chip());
    assert!(b.board().owner_chip().exists(KeyId::INBOX).unwrap());
    assert!(b
        .board()
        .log
        .iter()
        .any(|l| l == "chip: the phone slot, the inbox, the grant counter and the pairing object were made again"));
}

#[test]
fn a_remade_grant_counter_starts_above_every_grant_on_the_chip() {
    let mut board = remake_board(ObjectPolicy::any_session());
    board
        .owner_chip()
        .put_counter(KeyId::new(KeyKind::Grant, 2), (3 << 32) + 1, &ObjectPolicy::owner_counter())
        .unwrap();
    let mut b = Badge::new(board);
    b.boot();
    // The next grant's base is 4 << 32: no base is given twice.
    assert_eq!(b.board().owner_chip().counter_get(KeyId::new(KeyKind::Grant, 255)), Ok(3));
}

/// The pairing object read in no session, as any tap reads it.
fn pairing_object(b: &mut Badge<FakeBoard>) -> cahoots_badge::pairing::PairingObject {
    let c = b.board().chip.as_mut().unwrap();
    c.no_session_for_test();
    let bytes = c.read_binary(KeyId::PAIRING).unwrap();
    cahoots_badge::pairing::PairingObject::decode(&bytes).unwrap()
}

#[test]
fn the_first_boot_makes_the_pairing_object_with_no_pairing_open() {
    let mut board = FakeBoard::new();
    board.chip = Some(SoftChip::with_seed([2; 32]));
    let mut b = Badge::new(board);
    b.boot();
    let p = pairing_object(&mut b);
    assert_eq!((p.nonce, p.badge_key, p.circles), ([0; 16], [0; 33], 0));
    // A tap cannot write it or delete it.
    let c = b.board().chip.as_mut().unwrap();
    assert_eq!(c.rewrite_binary(KeyId::PAIRING, &[1; 16]), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::PAIRING), Err(ChipError::NotAllowed));
}

#[test]
fn an_object_a_tap_planted_at_the_pairing_object_before_the_first_boot_refuses_the_chip() {
    let mut board = FakeBoard::new();
    let mut c = SoftChip::with_seed([2; 32]);
    c.write_binary(KeyId::PAIRING, &[0; 128], &stuck()).unwrap();
    board.chip = Some(c);
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(refused(&mut b), 1);
    assert!(!b.board().has_chip());
    assert!(b.board().chip_record_load().unwrap().unwrap().is_refused());
}

#[test]
fn an_object_a_tap_planted_at_a_circle_slots_public_id_before_the_first_boot_refuses_the_chip() {
    let last = cahoots_badge::chip::CIRCLE_SLOTS - 1;
    for planted in [KeyId::new(KeyKind::Public, 0), KeyId::new(KeyKind::Public, last)] {
        let mut board = FakeBoard::new();
        let mut c = SoftChip::with_seed([2; 32]);
        c.write_binary(planted, &[0; 16], &stuck()).unwrap();
        board.chip = Some(c);
        let mut b = Badge::new(board);
        b.boot();
        assert_eq!(refused(&mut b), 1, "{planted:?}");
        assert!(!b.board().has_chip());
        assert!(b.board().chip_record_load().unwrap().unwrap().is_refused());
        let log = b.board().log.join("\n");
        assert!(log.contains("a circle slot's public id"), "{log}");
    }
}

#[test]
fn an_object_a_tap_planted_at_a_circle_slots_public_id_that_can_be_deleted_is_not_adopted() {
    let mut board = old_board();
    let planted = KeyId::new(KeyKind::Public, 2);
    board
        .chip
        .as_mut()
        .unwrap()
        .write_binary(planted, b"planted", &ObjectPolicy::any_session())
        .unwrap();
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(refused(&mut b), 0);
    assert_eq!(shown(&mut b), 1);
    // The badge's placeholder stands there now: a tap in no session cannot use or delete it.
    let c = b.board().chip.as_mut().unwrap();
    assert!(common::is_placeholder(c, planted));
    c.no_session_for_test();
    assert_eq!(c.read_binary(planted), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(planted), Err(ChipError::NotAllowed));
    assert_eq!(common::empty_public_ids(c), Vec::<KeyId>::new());
}

#[test]
fn a_first_boot_with_no_room_for_placeholders_owns_the_chip_and_fills_them_when_it_can() {
    let mut board = FakeBoard::new();
    let mut c = SoftChip::with_seed([2; 32]);
    // The first placeholder meets a full chip, after the badge's own objects.
    c.fail_next_create_with(0x6A84);
    c.fail_create_after_for_test(5);
    board.chip = Some(c);
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(refused(&mut b) + not_now(&mut b), 0);
    assert!(b.board().has_chip());
    let r = b.board().chip_record_load().unwrap().unwrap();
    assert!(r.is_current());
    assert_eq!(r.placeholders, Some(cahoots_badge::chip::PLACEHOLDERS));
    // The same boot fills them, the chip having room again.
    assert_eq!(common::empty_public_ids(b.board().owner_chip()), Vec::<KeyId>::new());
    let log = b.board().log.join("\n");
    assert!(log.contains("chip: 16 placeholders made at the circle slots' public ids"), "{log}");
}

#[test]
fn a_first_boot_whose_placeholders_fail_is_owned_and_the_same_boot_clears_and_fills_them() {
    let mut board = FakeBoard::new();
    let mut c = SoftChip::with_seed([2; 32]);
    // The first placeholder fails with 6A80, after the badge's own objects.
    c.fail_create_after_for_test(5);
    board.chip = Some(c);
    let mut b = Badge::new(board);
    b.boot();
    assert_eq!(refused(&mut b) + not_now(&mut b), 0);
    assert!(b.board().has_chip());
    let r = b.board().chip_record_load().unwrap().unwrap();
    assert!(r.is_current());
    assert_eq!(r.placeholders, Some(cahoots_badge::chip::PLACEHOLDERS));
    assert_eq!(common::empty_public_ids(b.board().owner_chip()), Vec::<KeyId>::new());
    let log = b.board().log.join("\n");
    assert!(log.contains("chip: 16 placeholders made at the circle slots' public ids"), "{log}");
}

/// A chip whose record is current, with the pairing object missing: the boot flags the record
/// before it deletes anything. A power cut after the deletes, before the makes, then leaves
/// every antenna id empty. A tap fills them all. The next boot sees the flag and makes them
/// again rather than adopt what the tap made.
#[test]
fn a_remake_cut_off_after_its_deletes_is_done_again_and_adopts_nothing() {
    let mut board = FakeBoard::new().with_soft_chip([6; 32]);
    board.owner_chip().delete(KeyId::PAIRING).unwrap();
    board.chip.as_mut().unwrap().close_session().unwrap();
    board.chip.as_mut().unwrap().cut_after_delete_for_test(KeyId::new(KeyKind::Grant, 255));
    let mut b = Badge::new(board);
    let cut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| b.boot()));
    assert!(cut.is_err(), "the power was cut");
    let r = b.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.generation, cahoots_badge::chip::REMAKE_GENERATION);
    // The pin rests low after the cut, and a tap fills every antenna id.
    b.board().power_cycle();
    let c = b.board().chip.as_mut().unwrap();
    for id in cahoots_badge::chip::ANTENNA_IDS {
        assert!(!c.exists(id).unwrap(), "{id:?} was deleted");
    }
    c.put_aes_auth(KeyId::SLOT, &[0x42; 16], &ObjectPolicy::any_session()).unwrap();
    c.write_binary(KeyId::INBOX, &[0; 256], &ObjectPolicy::any_session()).unwrap();
    c.put_counter(KeyId::new(KeyKind::Grant, 255), 0, &ObjectPolicy::owner_counter()).unwrap();
    c.write_binary(KeyId::PAIRING, &[0; 128], &ObjectPolicy::any_session()).unwrap();
    b.boot();
    assert_eq!(refused(&mut b), 0);
    assert!(b.board().has_chip());
    let c = b.board().chip.as_mut().unwrap();
    c.no_session_for_test();
    assert_eq!(c.open_slot_with_key_for_test(&[0x42; 16]), Err(ChipError::BadSession));
    c.no_session_for_test();
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    assert_eq!(c.read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::PAIRING), Err(ChipError::NotAllowed));
    assert_eq!(pairing_object(&mut b).nonce, [0; 16]);
    let r = b.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.generation, cahoots_badge::chip::GENERATION);
}

const FOREIGN: &str = "this chip belongs to another badge or an older build. It cannot be used.";
/// The owner secret of a build before 2026-11-01, as the board keeps it beside the current one.
const LEGACY: [u8; 16] = *b"older-owner-key!";

/// A board whose chip was set up by a build that used `secret` as the owner secret.
fn owned_under(secret: &[u8; 16]) -> FakeBoard {
    let mut board = FakeBoard::new();
    let mut c = SoftChip::with_seed([7; 32]);
    let record = cahoots_badge::chip::first_boot(&mut c, secret, [0x5A; 16]).unwrap();
    c.close_session().unwrap();
    board.chip = Some(c);
    board.chip_record = Some(record.to_stored());
    board
}

#[test]
fn a_chip_owned_under_the_older_secret_opens_with_it_and_is_used() {
    let mut board = owned_under(&LEGACY);
    board.legacy_secret = Some(LEGACY);
    let mut b = Badge::new(board);
    b.boot();
    assert!(b.board().has_chip());
    assert_eq!(refused(&mut b) + not_now(&mut b), 0);
    assert_eq!(b.board().statuses().iter().filter(|s| *s == FOREIGN).count(), 0);
    let older = |b: &mut Badge<FakeBoard>| {
        b.board().log.iter().filter(|l| *l == "se051: owner session open with the older secret").count()
    };
    assert_eq!(older(&mut b), 1, "once per boot");
    // Nothing was made again or wiped: the chip is used as it is, under the older secret.
    assert!(!b.board().log.iter().any(|l| l.contains("made again") || l.starts_with("upgrade:")));
    assert_eq!(b.board().owner_secret(), Some(LEGACY));
    b.pair_open().unwrap();
    b.pair_close();
    pin_rests(&mut b);
    assert_eq!(older(&mut b), 1);
}

#[test]
fn a_chip_owned_under_neither_secret_is_not_used_and_says_whose_it_is() {
    let mut board = owned_under(&[0x33; 16]);
    board.legacy_secret = Some(LEGACY);
    let mut b = Badge::new(board);
    b.boot();
    assert!(!b.board().has_chip());
    assert_eq!(b.board().statuses().iter().filter(|s| *s == FOREIGN).count(), 1);
    assert_eq!(not_now(&mut b), 0);
    // Nothing is stored: the build that owns it can still use it.
    assert!(!b.board().chip_record_load().unwrap().unwrap().is_refused());
}

/// A fresh badge, with no chip record, never adopts an owner object made under the older
/// secret: a tap that knew it could have planted one.
#[test]
fn a_chip_owned_under_the_older_secret_with_no_record_is_not_this_badges() {
    let mut board = owned_under(&LEGACY);
    board.chip_record = None;
    board.legacy_secret = Some(LEGACY);
    let mut b = Badge::new(board);
    b.boot();
    assert!(!b.board().has_chip());
    assert_eq!(b.board().statuses().iter().filter(|s| *s == FOREIGN).count(), 1);
    assert!(!b.board().log.iter().any(|l| l == "se051: owner session open with the older secret"));
}
