//! The seat key and the circle root made in the chip, the chip record, and the circle record
//! that says where the key is.

mod common;

use cahoots_badge::chip::{ChipRecord, ChipSeat, PhoneSlot, SlotEntry, GENERATION};
use cahoots_badge::screens::making_key;
use cahoots_badge::{Badge, Board, Circle, Press, Step};
use cahoots_chip::{
    ChipError, ChipOps, Curve, KeyId, KeyKind, ObjectPolicy, SoftChip, TEST_OWNER_SECRET,
};
use cahoots_core::manifest::SealedManifest;
use cahoots_core::root::chat_key;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use common::{cable, FakeBoard, FakeLink};

/// Card 1 at the bottom, card 2 on top. `chips` says which cards have a chip.
fn pile_of_two(chips: (bool, bool)) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (lower, upper) = cable();
    let mut b1 = FakeBoard::seeded(0x9E37_79B9_7F4A_7C15);
    let mut b2 = FakeBoard::seeded(0xD1B5_4A32_D192_ED03);
    b1.links = Some((FakeLink::open(), lower));
    b2.links = Some((upper, FakeLink::open()));
    if chips.0 {
        b1 = b1.with_soft_chip([1; 32]);
    }
    if chips.1 {
        b2 = b2.with_soft_chip([2; 32]);
    }
    (Badge::new(b1), Badge::new(b2))
}

fn has_seat(b: &mut Badge<FakeBoard>) -> bool {
    b.board().screens.iter().any(|l| {
        l.iter().any(|x| x == "stack the cards") && l.iter().any(|x| x.starts_with("seat "))
    })
}

fn drive(
    b1: &mut Badge<FakeBoard>,
    b2: &mut Badge<FakeBoard>,
    mut done: impl FnMut(&mut Badge<FakeBoard>, &mut Badge<FakeBoard>, &Step, &Step) -> bool,
) {
    for _ in 0..10_000 {
        let s1 = b1.step();
        let s2 = b2.step();
        assert!(!matches!(s1, Step::Failed(_)), "card 1: {s1:?}");
        assert!(!matches!(s2, Step::Failed(_)), "card 2: {s2:?}");
        if done(b1, b2, &s1, &s2) {
            return;
        }
    }
    panic!("the badges did not get there");
}

/// Seat both cards, count the pile, and run to the keep prompt on both.
fn to_the_prompt(chips: (bool, bool)) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (mut b1, mut b2) = pile_of_two(chips);
    drive(&mut b1, &mut b2, |b1, b2, _, _| has_seat(b1) && has_seat(b2));
    b2.board().presses.push_back(Press::Both);
    let mut at = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        at.0 |= *s1 == Step::Prompt;
        at.1 |= *s2 == Step::Prompt;
        at.0 && at.1
    });
    (b1, b2)
}

/// Run to the prompt and keep the circle on both cards.
fn kept_pair(chips: (bool, bool)) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (mut b1, mut b2) = to_the_prompt(chips);
    b1.board().presses.push_back(Press::Both);
    b2.board().presses.push_back(Press::Both);
    let mut kept = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        kept.0 |= *s1 == Step::Kept;
        kept.1 |= *s2 == Step::Kept;
        kept.0 && kept.1
    });
    (b1, b2)
}

fn id(kind: KeyKind, slot: u8) -> KeyId {
    KeyId::new(kind, slot)
}

/// The board's chip, in the owner session.
fn chip(b: &mut Badge<FakeBoard>) -> &mut SoftChip {
    b.board().owner_chip()
}

/// The chip's objects that belong to circles: all but the owner object, the phone slot and
/// the inbox.
fn circle_objects(b: &mut Badge<FakeBoard>) -> Vec<KeyId> {
    let ids = chip(b).ids().unwrap();
    ids.into_iter().filter(|id| !id.is_badge_wide()).collect()
}

/// A software chip set up as a badge's first boot sets it up, in the owner session.
fn owned_chip(seed: [u8; 32]) -> SoftChip {
    let mut c = SoftChip::with_seed(seed);
    cahoots_badge::chip::first_boot(&mut c, &TEST_OWNER_SECRET, [0; 16]).unwrap();
    c
}

#[test]
fn ceremony_with_soft_chip_makes_seat_in_chip() {
    let (mut b1, mut b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let c2 = b2.circle().unwrap().clone();
    assert_eq!(c1.circle_id, c2.circle_id);

    // The chip's pin rose for each conversation and is down again after the keep, with the
    // session closed: high only between `chip_begin` and `chip_end`.
    let pins = b1.board().pin_log.clone();
    assert!(pins.len() >= 4, "{pins:?}");
    assert!(pins.iter().enumerate().all(|(i, &high)| high == (i % 2 == 0)), "{pins:?}");
    assert!(!b1.board().pin_high());
    assert_eq!(b1.board().chip.as_ref().unwrap().session(), cahoots_chip::Session::None);

    // The seat key, the root and the epoch counter are in the chip at slot 0.
    for kind in [KeyKind::Seat, KeyKind::Root, KeyKind::Epoch] {
        assert!(chip(&mut b1).exists(id(kind, 0)).unwrap(), "{kind:?}");
    }
    assert_eq!(circle_objects(&mut b1).len(), 3);

    // The seat key signs for the owner alone; the root MACs for the owner and for the phone
    // slot under secure messaging, and for nothing in no session.
    let digest = [1u8; 32];
    let c = chip(&mut b1);
    assert!(c.sign_p256(id(KeyKind::Seat, 0), &digest).is_ok());
    assert!(c.hmac(id(KeyKind::Root, 0), b"chat/gen").is_ok());
    c.open_slot_with_sm_for_test().unwrap();
    assert_eq!(c.sign_p256(id(KeyKind::Seat, 0), &digest), Err(ChipError::NotAllowed));
    assert!(c.hmac(id(KeyKind::Root, 0), b"chat/gen").is_ok());
    c.open_slot_without_sm_for_test().unwrap();
    assert_eq!(c.hmac(id(KeyKind::Root, 0), b"chat/gen"), Err(ChipError::NotAllowed));
    c.no_session_for_test();
    assert_eq!(c.sign_p256(id(KeyKind::Seat, 0), &digest), Err(ChipError::NotAllowed));
    assert_eq!(c.hmac(id(KeyKind::Root, 0), b"chat/gen"), Err(ChipError::NotAllowed));
    assert!(b1.board().log.iter().any(|l| l
        == "chip: slot 0 kept, seat key for the owner alone, root for the owner and the phone slot"));

    // The manifest names the chip's key for this seat.
    let manifest = SealedManifest::decode(&c1.manifest).unwrap().verify().unwrap();
    let public = chip(&mut b1).public(id(KeyKind::Seat, 0)).unwrap();
    let public = SeatPublic::parse(&public).unwrap();
    assert_eq!(manifest.seat_key(c1.seat), Some(&public));

    // The record says the key is in the chip and holds no root and no key bytes.
    assert_eq!(c1.chip_slot, Some(0));
    assert_eq!(c1.root, None);
    assert_eq!(c1.seat_key, [0; 32]);
    assert_eq!(b1.board().load_circle().as_ref(), Some(&c1));

    // The chip's root is the one the software card keeps: the same chat key from both.
    let root = c2.root.expect("the software card keeps the root");
    let mut msg = b"chat".to_vec();
    msg.extend_from_slice(&c1.circle_id);
    msg.extend_from_slice(&0u64.to_be_bytes());
    let from_chip = chip(&mut b1).hmac(id(KeyKind::Root, 0), &msg).unwrap();
    assert_eq!(from_chip, chat_key(&root, &c1.circle_id, 0));
    // The epoch counter starts at 0.
    assert_eq!(chip(&mut b1).counter_next(id(KeyKind::Epoch, 0)).unwrap(), 1);

    // The chip record knows the slot and what was made in it, and is stored as CAHC2.
    assert!(b1.board().chip_record.as_ref().unwrap().starts_with(b"CAHC2"));
    let record = b1.board().chip_record_load().unwrap().expect("a chip record");
    assert_eq!(record.generation, GENERATION);
    assert_eq!(
        record.slots,
        vec![SlotEntry {
            slot: 0,
            circle_id: c1.circle_id,
            has_seat: true,
            has_root: true,
            has_epoch: true,
        }]
    );

    // Each card showed where its key was made.
    let shown = |b: &mut Badge<FakeBoard>, place: &str| {
        b.board().screens.iter().any(|l| {
            l.iter().any(|x| x == "this circle's key") && l.iter().any(|x| x == place)
        })
    };
    assert!(shown(&mut b1, "in the chip"));
    assert!(shown(&mut b2, "in software"));
}

#[test]
fn software_board_keeps_root_in_record() {
    let (mut b1, mut b2) = kept_pair((false, false));
    let c1 = b1.circle().unwrap().clone();
    let c2 = b2.circle().unwrap().clone();
    assert_eq!(c1.chip_slot, None);
    assert_eq!(c2.chip_slot, None);
    assert!(c1.root.is_some());
    assert_eq!(c1.root, c2.root, "every card holds the same root");
    assert_ne!(c1.seat_key, [0; 32]);
    assert_eq!(b1.board().load_circle().as_ref(), Some(&c1));
    assert!(b1.board().chip_record.is_none());
    assert!(b2.board().chip_record.is_none());
}

#[test]
fn discarding_deletes_the_key_made_in_the_chip() {
    let (mut b1, mut b2) = to_the_prompt((true, false));
    assert!(chip(&mut b1).exists(id(KeyKind::Seat, 0)).unwrap());
    b1.board().presses.extend([Press::Left, Press::Both]);
    b2.board().presses.push_back(Press::Both);
    let mut out = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        out.0 |= *s1 == Step::Discarded;
        out.1 |= *s2 == Step::Kept;
        out.0 && out.1
    });
    assert!(circle_objects(&mut b1).is_empty());
    assert!(b1
        .board()
        .chip_record_load()
        .unwrap()
        .is_none_or(|r| r.slots.is_empty()));
}

#[test]
fn forget_deletes_slot_objects() {
    let (mut b1, _b2) = kept_pair((true, false));
    assert_eq!(circle_objects(&mut b1).len(), 3);
    let board = b1.board();
    board.a_down = true;
    board.release_at_ms = board.elapsed_ms + 3000;
    b1.boot();
    assert!(b1.circle().is_none());
    assert!(b1.board().load_circle().is_none());
    assert!(circle_objects(&mut b1).is_empty());
    // The badge's own objects stay: it still owns its chip.
    let ids = chip(&mut b1).ids().unwrap();
    assert_eq!(ids, vec![KeyId::SLOT, KeyId::INBOX, KeyId::OWNER]);
    assert!(b1
        .board()
        .chip_record_load()
        .unwrap()
        .is_none_or(|r| r.slots.is_empty()));
}

#[test]
fn reconcile_deletes_unknown_slot_and_logs() {
    let mut c = owned_chip([3; 32]);
    c.generate(id(KeyKind::Seat, 0), Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    c.generate(id(KeyKind::Seat, 2), Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    c.put_hmac_key(id(KeyKind::Root, 2), &[9; 32], &ObjectPolicy::owner_and_slot_mac()).unwrap();
    let mut record = ChipRecord {
        slots: vec![SlotEntry {
            slot: 0,
            circle_id: [5; 16],
            has_seat: true,
            has_root: false,
            has_epoch: false,
        }],
        ..ChipRecord::default()
    };
    // An object of a kind this firmware does not know is no circle's, and stays.
    let unknown = KeyId(0x4C41_0AF0);
    c.put_user_id(unknown, &[1; 16], &ObjectPolicy::owner_object()).unwrap();
    let lines = record.reconcile(&mut c);
    assert!(c.exists(unknown).unwrap());
    // The badge-wide objects belong to no circle slot and stay.
    assert_eq!(
        c.ids().unwrap(),
        vec![id(KeyKind::Seat, 0), KeyId::SLOT, KeyId::INBOX, KeyId::OWNER, unknown]
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|l| l.contains("slot 2")), "{lines:?}");
    assert_eq!(record.slots.len(), 1);
}

#[test]
fn boot_deletes_chip_objects_the_record_does_not_know() {
    let mut board = FakeBoard::new().with_soft_chip([4; 32]);
    let c = board.owner_chip();
    c.generate(id(KeyKind::Seat, 2), Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    c.put_counter(id(KeyKind::Epoch, 2), 0, &ObjectPolicy::owner_counter())
        .unwrap();
    let mut badge = Badge::new(board);
    badge.boot();
    assert!(circle_objects(&mut badge).is_empty());
    let log = &badge.board().log;
    assert_eq!(log.iter().filter(|l| l.contains("slot 2")).count(), 2, "{log:?}");
}

#[test]
fn making_key_screen_lines() {
    let s = making_key(true);
    assert_eq!(
        s.lines,
        vec!["making a key", "this circle's key", "in the chip", "one moment"]
    );
    assert!(!s.secret);
    let s = making_key(false);
    assert_eq!(s.lines[2], "in software");
}

#[test]
fn chip_seat_signs_with_the_chip_key() {
    let mut c = owned_chip([6; 32]);
    let seat = id(KeyKind::Seat, 1);
    c.generate(seat, Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    let mut s = ChipSeat::new(&mut c, seat);
    let public = s.public();
    let sig = s.sign(b"a message");
    assert!(public.verify(b"a message", &sig));
    assert_eq!(s.error, None);
}

#[test]
fn chip_seat_returns_zeros_when_the_chip_fails() {
    let mut c = SoftChip::with_seed([6; 32]);
    let mut s = ChipSeat::new(&mut c, id(KeyKind::Seat, 1));
    assert_eq!(s.sign(b"a message"), [0; 64]);
    assert!(s.error.is_some());
}

#[test]
fn chip_record_round_trips_and_refuses_other_encodings() {
    let r = ChipRecord {
        slots: vec![
            SlotEntry {
                slot: 0,
                circle_id: [1; 16],
                has_seat: true,
                has_root: true,
                has_epoch: true,
            },
            SlotEntry {
                slot: 3,
                circle_id: [2; 16],
                has_seat: true,
                has_root: false,
                has_epoch: false,
            },
        ],
        phone: Some(PhoneSlot {
            name: "a phone".into(),
            public: [2; 33],
        }),
        badge_nonce: [9; 16],
        generation: GENERATION,
    };
    let bytes = r.encode();
    assert_eq!(ChipRecord::decode(&bytes), Ok(r.clone()));
    // Stored, it is CAHC2 and the CBOR.
    let stored = r.to_stored();
    assert_eq!(&stored[..5], b"CAHC2");
    assert_eq!(ChipRecord::from_stored(&stored), Ok(Some(r.clone())));
    // The record of earlier firmware reads as no record: in a CAHC sector, or bare.
    // `[[[0, h'0101..01', true, true, true]]]`: one slot entry, as earlier firmware wrote it.
    let mut old = vec![0x81, 0x81, 0x85, 0x00, 0x50];
    old.extend_from_slice(&[1; 16]);
    old.extend_from_slice(&[0xF5, 0xF5, 0xF5]);
    assert_eq!(ChipRecord::from_stored(&old), Ok(None));
    let mut sector = b"CAHC".to_vec();
    sector.extend_from_slice(&old);
    assert_eq!(ChipRecord::from_stored(&sector), Ok(None));
    assert!(ChipRecord::from_stored(&[0xFF; 8]).is_err());
    let mut cut = stored.clone();
    cut.pop();
    assert!(ChipRecord::from_stored(&cut).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(ChipRecord::decode(&trailing).is_err());
    assert!(ChipRecord::decode(&bytes[..bytes.len() - 1]).is_err());
    assert!(ChipRecord::decode(&[0xFF; 8]).is_err());
}

fn sample_circle(chip_slot: Option<u8>, root: Option<[u8; 32]>) -> Circle {
    let (board, _, _) = FakeBoard::with_circle();
    let mut c = board.circle.clone().unwrap();
    c.chip_slot = chip_slot;
    c.root = root;
    c.manifest = vec![0xA5; 40];
    c
}

#[test]
fn circle_record_round_trips_as_cah3() {
    for (slot, root) in [(None, None), (Some(4), None), (None, Some([8; 32]))] {
        let c = sample_circle(slot, root);
        let bytes = c.encode_record().unwrap();
        assert_eq!(&bytes[..4], b"CAH3");
        assert_eq!(Circle::decode_record(&bytes).as_ref(), Some(&c));
        let mut cut = bytes.to_vec();
        cut.pop();
        assert_eq!(Circle::decode_record(&cut), None);
    }
}

#[test]
fn a_cah2_record_loads_with_no_slot_and_no_root() {
    let c = sample_circle(None, None);
    let cah3 = c.encode_record().unwrap();
    // CAH2 is CAH3 without the trailing flags, slot and root: 34 bytes shorter.
    let body_len = u16::from_le_bytes([cah3[4], cah3[5]]) as usize - 34;
    let mut cah2 = b"CAH2".to_vec();
    cah2.extend_from_slice(&(body_len as u16).to_le_bytes());
    cah2.extend_from_slice(&cah3[6..6 + body_len]);
    assert_eq!(Circle::decode_record(&cah2), Some(c));
    let mut cah1 = cah2.clone();
    cah1[3] = b'1';
    assert_eq!(Circle::decode_record(&cah1), None);
}

#[test]
fn a_chip_that_cannot_sign_fails_the_pass_and_leaves_no_key() {
    let (mut b1, mut b2) = pile_of_two((true, false));
    drive(&mut b1, &mut b2, |b1, b2, _, _| has_seat(b1) && has_seat(b2));
    b2.board().presses.push_back(Press::Both);
    drive(&mut b1, &mut b2, |b1, _, s1, _| {
        *s1 == Step::InCeremony && circle_objects(b1).len() == 1
    });
    // The key goes before the manifest is signed: the chip's signature is all zeros.
    chip(&mut b1).delete(id(KeyKind::Seat, 0)).unwrap();
    chip(&mut b1).generate(id(KeyKind::Seat, 7), Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    let mut failed = None;
    for _ in 0..10_000 {
        if let Step::Failed(e) = b1.step() {
            failed = Some(e);
            break;
        }
        let _ = b2.step();
    }
    let failed = failed.expect("the pass fails, it does not wait");
    assert!(failed.contains("BadAttestation"), "{failed}");
    assert!(b1.board().log.iter().any(|l| l.contains("signing failed")));
    // Slot 0 is cleaned up; slot 7 was never this pass's.
    assert_eq!(circle_objects(&mut b1), vec![id(KeyKind::Seat, 7)]);
}

#[test]
fn an_unreadable_chip_record_deletes_nothing_at_boot() {
    let (mut b1, _b2) = kept_pair((true, false));
    b1.board().chip_record = Some(vec![0xFF; 5]);
    b1.boot();
    assert_eq!(circle_objects(&mut b1).len(), 3, "the kept circle's objects stay");
    assert!(b1.circle().is_some());
    assert!(b1.board().log.iter().any(|l| l.contains("chip record: unreadable")));
    assert_eq!(b1.board().chip_record, Some(vec![0xFF; 5]), "not written over");
}

#[test]
fn boot_keeps_the_slot_the_circle_names_even_if_the_record_forgot_it() {
    let (mut b1, _b2) = kept_pair((true, false));
    let empty = ChipRecord {
        generation: GENERATION,
        ..ChipRecord::default()
    };
    b1.board().chip_record = Some(empty.to_stored());
    b1.boot();
    assert_eq!(circle_objects(&mut b1).len(), 3);
    assert!(b1.board().log.iter().any(|l| l.contains("a circle names it")));
}

#[test]
fn an_unreadable_chip_record_makes_no_key() {
    let (mut b1, mut b2) = pile_of_two((true, false));
    b1.board().chip_record = Some(vec![0xFF; 5]);
    drive(&mut b1, &mut b2, |b1, b2, _, _| has_seat(b1) && has_seat(b2));
    b2.board().presses.push_back(Press::Both);
    let mut failed = None;
    for _ in 0..10_000 {
        if let Step::Failed(e) = b1.step() {
            failed = Some(e);
            break;
        }
        let _ = b2.step();
    }
    assert!(failed.expect("the pass fails").contains("chip record: unreadable"));
    assert!(circle_objects(&mut b1).is_empty());
    assert_eq!(b1.board().chip_record, Some(vec![0xFF; 5]));
}
