//! The seat key and the circle root made in the chip, the chip record, and the circle record
//! that says where the key is.

mod common;

use cahoots_badge::chip::{
    ChipRecord, ChipSeat, PhoneSlot, SlotEntry, CIRCLE_SLOTS, GENERATION, PLACEHOLDERS,
};
use cahoots_badge::screens::making_key;
use cahoots_badge::{Badge, Board, Circle, Press, Step};
use cahoots_chip::{
    ChipError, ChipOps, Curve, KeyId, KeyKind, ObjectPolicy, SoftChip, TEST_OWNER_SECRET,
};
use cahoots_core::manifest::SealedManifest;
use cahoots_core::root::chat_key;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use common::{cable, empty_public_ids, every_public_id, is_placeholder, FakeBoard, FakeLink};

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

/// The objects a kept circle of two has in the chip: the seat key, the root, the epoch
/// counter, one hello object and the public object.
const KEPT_OBJECTS: usize = 5;

fn id(kind: KeyKind, slot: u8) -> KeyId {
    KeyId::new(kind, slot)
}

/// The board's chip, in the owner session.
fn chip(b: &mut Badge<FakeBoard>) -> &mut SoftChip {
    b.board().owner_chip()
}

/// The chip's objects that belong to circles: all but the owner object, the phone slot, the
/// inbox, the grant issue counter and the placeholders.
fn circle_objects(b: &mut Badge<FakeBoard>) -> Vec<KeyId> {
    let c = chip(b);
    let ids = c.ids().unwrap();
    ids.into_iter()
        .filter(|id| id.circle_slot().is_some() && !is_placeholder(c, *id))
        .collect()
}

/// No circle slot's public id holds nothing: each holds a circle's public object or the badge's
/// placeholder.
fn no_empty_public_id(b: &mut Badge<FakeBoard>) {
    assert_eq!(empty_public_ids(chip(b)), Vec::<KeyId>::new());
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

    // The seat key, the root and the epoch counter are in the chip at slot 0, with the hello
    // object for the other seat and the public object.
    for kind in [KeyKind::Seat, KeyKind::Root, KeyKind::Epoch, KeyKind::Public] {
        assert!(chip(&mut b1).exists(id(kind, 0)).unwrap(), "{kind:?}");
    }
    assert!(chip(&mut b1).exists(id(KeyKind::Hello, 1 - c1.seat)).unwrap());
    assert_eq!(circle_objects(&mut b1).len(), 5);

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
            chat_generation: None,
            public_written: Some(true),
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
    // The slot's public id holds its placeholder again, and the record says so.
    no_empty_public_id(&mut b1);
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
    assert!(b1
        .board()
        .chip_record_load()
        .unwrap()
        .is_none_or(|r| r.slots.is_empty()));
}

#[test]
fn forget_deletes_slot_objects() {
    let (mut b1, _b2) = kept_pair((true, false));
    assert_eq!(circle_objects(&mut b1).len(), KEPT_OBJECTS);
    let board = b1.board();
    board.a_down = true;
    board.release_at_ms = board.elapsed_ms + 3000;
    b1.boot();
    assert!(b1.circle().is_none());
    assert!(b1.board().load_circle().is_none());
    assert!(circle_objects(&mut b1).is_empty());
    // The badge's own objects stay: it still owns its chip. Every circle slot's public id holds
    // its placeholder again.
    let c = chip(&mut b1);
    let ids: Vec<KeyId> = c.ids().unwrap().into_iter().filter(|i| !is_placeholder(c, *i)).collect();
    assert_eq!(
        ids,
        vec![id(KeyKind::Grant, 255), KeyId::SLOT, KeyId::PAIRING, KeyId::INBOX, KeyId::OWNER]
    );
    no_empty_public_id(&mut b1);
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
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
            chat_generation: None,
            public_written: None,
        }],
        ..ChipRecord::default()
    };
    // An object of a kind this firmware does not know is no circle's, and stays.
    let unknown = KeyId(0x4C41_0AF0);
    c.put_user_id(unknown, &[1; 16], &ObjectPolicy::owner_object()).unwrap();
    let lines = record.reconcile(&mut c);
    assert!(c.exists(unknown).unwrap());
    // The badge-wide objects belong to no circle slot and stay, and so do the placeholders.
    assert_eq!(empty_public_ids(&mut c), Vec::<KeyId>::new());
    assert_eq!(
        c.ids().unwrap().into_iter().filter(|i| !is_placeholder(&c, *i)).collect::<Vec<_>>(),
        vec![
            id(KeyKind::Seat, 0),
            id(KeyKind::Grant, 255),
            KeyId::SLOT,
            KeyId::PAIRING,
            KeyId::INBOX,
            KeyId::OWNER,
            unknown
        ]
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
                chat_generation: None,
                public_written: None,
            },
            SlotEntry {
                slot: 3,
                circle_id: [2; 16],
                has_seat: true,
                has_root: false,
                has_epoch: false,
                chat_generation: None,
                public_written: None,
            },
        ],
        phone: Some(PhoneSlot {
            name: "a phone".into(),
            public: [2; 33],
        }),
        badge_nonce: [9; 16],
        generation: GENERATION,
        stuck: Some(vec![3, 7]),
        pending_unpair: Some(vec![cahoots_badge::chip::PendingBump {
            slot: 3,
            to: [4; 32],
            bumps: vec![vec![1, 2, 3].into()],
        }]),
        paired_since_bump: Some(vec![3]),
        public_policy: Some(cahoots_badge::chip::PUBLIC_POLICY),
        placeholders: Some(PLACEHOLDERS),
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
    // A stuck list that is empty or out of order is not canonical.
    for bad in [Some(vec![]), Some(vec![7, 3]), Some(vec![3, 3])] {
        let mut r2 = r.clone();
        r2.stuck = bad;
        assert!(ChipRecord::decode(&r2.encode()).is_err());
    }
    // A public policy is the current one or none, which means the first.
    for bad in [Some(1), Some(3)] {
        let mut r2 = r.clone();
        r2.public_policy = bad;
        assert!(ChipRecord::decode(&r2.encode()).is_err());
    }
    let mut r2 = r.clone();
    r2.public_policy = None;
    assert_eq!(ChipRecord::decode(&r2.encode()).unwrap().public_policy(), 1);
    // Placeholders are made under this firmware or not known of.
    for bad in [Some(0), Some(2)] {
        let mut r2 = r.clone();
        r2.placeholders = bad;
        assert!(ChipRecord::decode(&r2.encode()).is_err());
    }
    let mut r2 = r.clone();
    r2.placeholders = None;
    assert_eq!(ChipRecord::decode(&r2.encode()), Ok(r2.clone()));
    // A record with no stuck slots encodes as before, four fields.
    let mut r2 = r.clone();
    r2.pending_unpair = Some(vec![]);
    assert!(ChipRecord::decode(&r2.encode()).is_err(), "an empty unpair is not canonical");
    let mut plain = r.clone();
    plain.stuck = None;
    plain.pending_unpair = None;
    plain.paired_since_bump = None;
    plain.public_policy = None;
    plain.placeholders = None;
    assert_eq!(plain.encode()[0], 0x84);
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
    assert_eq!(circle_objects(&mut b1).len(), KEPT_OBJECTS, "the kept circle's objects stay");
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
    assert_eq!(circle_objects(&mut b1).len(), KEPT_OBJECTS);
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

#[test]
fn a_wipe_deletes_every_object_it_can_and_says_what_stayed() {
    let (mut b1, _b2) = kept_pair((true, false));
    // An object nobody may delete, as a tap could make one, listed before the circle's root
    // and counter: a wipe that stopped at it would leave them.
    let stuck = id(KeyKind::Seat, 7);
    let policy = ObjectPolicy {
        delete: Vec::new(),
        ..ObjectPolicy::any_session()
    };
    chip(&mut b1).generate(stuck, Curve::P256, &policy).unwrap();
    let board = b1.board();
    board.a_down = true;
    board.release_at_ms = board.elapsed_ms + 3000;
    b1.boot();
    assert!(b1.circle().is_none());
    // The seat key, root and counter went; the stuck object stayed, and the badge said so.
    assert_eq!(circle_objects(&mut b1), vec![stuck]);
    no_empty_public_id(&mut b1);
    let log = b1.board().log.join("\n");
    assert!(log.contains("chip: object 4c410107 could not be deleted: NotAllowed"), "{log}");
    assert!(b1
        .board()
        .statuses()
        .iter()
        .any(|s| s == "circle forgotten\n1 chip object could not be deleted\nrelease A"));
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert!(record.slots.is_empty());
}

#[test]
fn a_slot_holding_an_object_that_cannot_be_deleted_is_passed_over() {
    let (mut b1, mut b2) = pile_of_two((true, false));
    let policy = ObjectPolicy {
        delete: Vec::new(),
        ..ObjectPolicy::any_session()
    };
    b1.board()
        .owner_chip()
        .put_hmac_key(id(KeyKind::Hello, 3), &[3; 32], &policy)
        .unwrap();
    drive(&mut b1, &mut b2, |b1, b2, _, _| has_seat(b1) && has_seat(b2));
    b2.board().presses.push_back(Press::Both);
    drive(&mut b1, &mut b2, |b1, _, s1, _| {
        *s1 == Step::InCeremony && b1.board().log.iter().any(|l| l.starts_with("seat key made"))
    });
    assert!(b1.board().log.iter().any(|l| l == "seat key made in the chip, slot 1"));
    // Slot 0's public id, emptied as the badge tried the slot, holds its placeholder again, and
    // so does slot 1's until the keep.
    no_empty_public_id(&mut b1);
    let c = chip(&mut b1);
    assert!(is_placeholder(c, id(KeyKind::Public, 0)));
    assert!(is_placeholder(c, id(KeyKind::Public, 1)));
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
}

#[test]
fn boot_empties_a_slot_a_wipe_marked_stuck_and_leaves_one_merely_unnamed() {
    // A wipe left slot 3 in the record, marked stuck, because an object there would not go
    // then. Slot 5 is in the record too, but its circle's record only failed to load.
    let mut board = FakeBoard::new().with_soft_chip([4; 32]);
    for slot in [3, 5] {
        board
            .owner_chip()
            .generate(id(KeyKind::Seat, slot), Curve::P256, &ObjectPolicy::owner_only())
            .unwrap();
        board.record_slot(SlotEntry {
            slot,
            circle_id: [slot; 16],
            has_seat: true,
            has_root: false,
            has_epoch: false,
            chat_generation: None,
            public_written: None,
        });
    }
    let mut r = board.chip_record_load().unwrap().unwrap();
    r.set_stuck(3, true);
    board.chip_record_store(&r).unwrap();
    let mut badge = Badge::new(board);
    badge.boot();
    // Slot 3 is emptied and let go; slot 5's key stays, and so does its entry.
    assert_eq!(circle_objects(&mut badge), vec![id(KeyKind::Seat, 5)]);
    let r = badge.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.slots.iter().map(|e| e.slot).collect::<Vec<_>>(), vec![5]);
    assert_eq!(r.stuck, None);
    let log = &badge.board().log;
    assert!(log.iter().any(|l| l == "chip: slot 3 names no circle, emptied and let go"));
    assert!(log
        .iter()
        .any(|l| l == "chip: slot 5 names no circle that loaded, left as it is"));
}

#[test]
fn a_wipe_marks_the_slot_it_could_not_empty_as_stuck() {
    let (mut b1, _b2) = kept_pair((true, false));
    let policy = ObjectPolicy {
        delete: Vec::new(),
        ..ObjectPolicy::any_session()
    };
    // A hello object for a seat a circle of two does not have.
    chip(&mut b1).put_hmac_key(id(KeyKind::Hello, 5), &[3; 32], &policy).unwrap();
    let board = b1.board();
    board.a_down = true;
    board.release_at_ms = board.elapsed_ms + 3000;
    b1.boot();
    // The record vouches for no slot, the circle's included, and marks slot 0 stuck so the
    // next boot tries it again.
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert!(r.slots.is_empty());
    assert!(r.is_stuck(0));
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
    no_empty_public_id(&mut b1);
}

#[test]
fn first_boot_puts_a_placeholder_at_every_circle_slots_public_id() {
    let mut c = SoftChip::with_seed([3; 32]);
    let record = cahoots_badge::chip::first_boot(&mut c, &TEST_OWNER_SECRET, [0; 16]).unwrap();
    assert_eq!(record.placeholders, Some(PLACEHOLDERS));
    let ids = every_public_id();
    assert_eq!(ids.len(), CIRCLE_SLOTS as usize);
    for &at in &ids {
        assert!(is_placeholder(&c, at), "{at:?}");
    }
    // The known limit: the hello ids get none, for want of the chip's memory.
    assert!(!c.exists(id(KeyKind::Hello, 0)).unwrap());
    // A stranger's tap, in no session, and the phone slot's session may not write, read, use
    // or delete any of them, and nobody can make an object of their own at those ids.
    c.no_session_for_test();
    for slot_session in [false, true] {
        if slot_session {
            c.open_slot_with_sm_for_test().unwrap();
        }
        for &at in &ids {
            assert_eq!(c.delete(at), Err(ChipError::NotAllowed), "{at:?}");
            assert_eq!(c.rewrite_binary(at, &[1]), Err(ChipError::NotAllowed), "{at:?}");
            assert_eq!(c.read_binary(at), Err(ChipError::NotAllowed), "{at:?}");
            assert_eq!(c.hmac(at, b"x"), Err(ChipError::NotAllowed), "{at:?}");
            assert_eq!(
                c.write_binary(at, &[1], &ObjectPolicy::any_session()),
                Err(ChipError::Exists),
                "{at:?}"
            );
        }
    }
}

#[test]
fn a_keep_takes_the_placeholder_at_its_public_id_and_leaves_the_rest() {
    let (mut b1, _b2) = kept_pair((true, false));
    let public = id(KeyKind::Public, 0);
    let c = chip(&mut b1);
    // The circle's public object stands where slot 0's placeholder was, made by the badge
    // under its own policy, and reads back.
    assert_eq!(c.policy_for_test(public), Some(ObjectPolicy::public_object()));
    let bytes = c.read_binary(public).unwrap();
    assert!(cahoots_badge::public::PublicObject::decode(&bytes).is_ok());
    // Every other slot's public id still holds a placeholder.
    for at in every_public_id() {
        assert_eq!(is_placeholder(c, at), at != public, "{at:?}");
    }
    assert_eq!(circle_objects(&mut b1).len(), KEPT_OBJECTS);
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
}

#[test]
fn a_chip_record_of_earlier_firmware_gets_its_placeholders_once() {
    let (mut b1, _b2) = kept_pair((true, false));
    // As firmware before placeholders left the chip: none at any public id, and a record that
    // does not know of them. A tap made an object at a free slot's public id meanwhile.
    let c = chip(&mut b1);
    for at in every_public_id() {
        if is_placeholder(c, at) {
            c.delete(at).unwrap();
        }
    }
    c.no_session_for_test();
    c.write_binary(id(KeyKind::Public, 3), b"planted", &ObjectPolicy::any_session())
        .unwrap();
    let mut r = b1.board().chip_record_load().unwrap().unwrap();
    r.placeholders = None;
    b1.board().chip_record_store(&r).unwrap();

    let from = b1.board().log.len();
    b1.boot();
    // The circle keeps its objects and its slot. Every other public id holds the badge's
    // placeholder: the tap's object was deleted and one made in its place, not adopted.
    assert_eq!(b1.circle().unwrap().chip_slot, Some(0));
    assert_eq!(circle_objects(&mut b1).len(), KEPT_OBJECTS);
    let c = chip(&mut b1);
    for at in every_public_id() {
        assert_eq!(is_placeholder(c, at), at != id(KeyKind::Public, 0), "{at:?}");
    }
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
    let made = |b: &mut Badge<FakeBoard>| -> Vec<String> {
        b.board().log[from..].iter().filter(|l| l.contains("made at the circle slots' public ids")).cloned().collect()
    };
    assert_eq!(made(&mut b1), ["chip: 15 placeholders made at the circle slots' public ids"]);
    // Once: the next boot makes none.
    b1.boot();
    assert_eq!(made(&mut b1).len(), 1);
    no_empty_public_id(&mut b1);
}

#[test]
fn a_first_boot_with_no_room_for_placeholders_still_owns_the_chip() {
    let mut c = SoftChip::with_seed([3; 32]);
    // The owner object, the phone slot, the inbox, the issue counter and the pairing object are
    // made. The first placeholder meets a full chip.
    c.fail_next_create_with(0x6A84);
    c.fail_create_after_for_test(5);
    let record = cahoots_badge::chip::first_boot(&mut c, &TEST_OWNER_SECRET, [0; 16]).unwrap();
    // Nothing was left at the public ids to adopt, so the record says the placeholders are the
    // badge's: a later boot fills the empty ids when the chip has room.
    assert_eq!(record.placeholders, Some(PLACEHOLDERS));
    for id in [KeyId::OWNER, KeyId::SLOT, KeyId::INBOX, KeyId::PAIRING] {
        assert!(c.exists(id).unwrap(), "{id:?}");
    }
    assert_eq!(empty_public_ids(&mut c).len(), CIRCLE_SLOTS as usize);
}

#[test]
fn a_record_of_earlier_firmware_whose_fill_fails_keeps_saying_so() {
    let (mut b1, _b2) = kept_pair((true, false));
    let c = chip(&mut b1);
    for at in every_public_id() {
        if is_placeholder(c, at) {
            c.delete(at).unwrap();
        }
    }
    let mut r = b1.board().chip_record_load().unwrap().unwrap();
    r.placeholders = None;
    b1.board().chip_record_store(&r).unwrap();
    // The first placeholder fails for a reason that is not memory.
    b1.board().chip.as_mut().unwrap().fail_next_create_for_test();
    b1.boot();
    assert!(b1.board().has_chip());
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, None);
    assert!(b1.board().log.iter().any(|l| l.starts_with("chip: placeholders not made")));
    // The next boot clears and fills them, and says so in the record.
    b1.boot();
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.placeholders, Some(PLACEHOLDERS));
    no_empty_public_id(&mut b1);
}

#[test]
fn a_first_boot_whose_placeholders_fail_otherwise_owns_the_chip_and_says_they_are_not_made() {
    let mut c = SoftChip::with_seed([3; 32]);
    // The first placeholder fails with 6A80, after the badge's own objects.
    c.fail_create_after_for_test(5);
    let record = cahoots_badge::chip::first_boot(&mut c, &TEST_OWNER_SECRET, [0; 16]).unwrap();
    assert_eq!(record.placeholders, None);
    for id in [KeyId::OWNER, KeyId::SLOT, KeyId::INBOX, KeyId::PAIRING] {
        assert!(c.exists(id).unwrap(), "{id:?}");
    }
}

#[test]
fn a_circle_kept_in_a_slot_marked_stuck_reaches_the_pairing_object() {
    // A forget left slot 0 stuck with no entry, for an object that would not go then, and
    // went later.
    let (mut b1, mut b2) = pile_of_two((true, false));
    let mut r = b1.board().chip_record_load().unwrap().unwrap();
    r.set_stuck(0, true);
    b1.board().chip_record_store(&r).unwrap();
    drive(&mut b1, &mut b2, |b1, b2, _, _| has_seat(b1) && has_seat(b2));
    b2.board().presses.push_back(Press::Both);
    let mut at = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        at.0 |= *s1 == Step::Prompt;
        at.1 |= *s2 == Step::Prompt;
        at.0 && at.1
    });
    b1.board().presses.push_back(Press::Both);
    b2.board().presses.push_back(Press::Both);
    let mut kept = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        kept.0 |= *s1 == Step::Kept;
        kept.1 |= *s2 == Step::Kept;
        kept.0 && kept.1
    });
    assert_eq!(b1.circle().unwrap().chip_slot, Some(0));
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert!(!r.is_stuck(0));
    assert_eq!(r.circle_mask(), 1);
    // The pairing object a phone reads names the new circle's slot.
    let c = b1.board().chip.as_mut().unwrap();
    c.no_session_for_test();
    let pairing = cahoots_badge::pairing::PairingObject::decode(&c.read_binary(KeyId::PAIRING).unwrap()).unwrap();
    assert_eq!(pairing.circles, 1);
}

#[test]
fn a_wipe_whose_chip_record_cannot_be_written_deletes_nothing_and_says_what_stayed() {
    let (mut b1, _b2) = kept_pair((true, false));
    let board = b1.board();
    board.a_down = true;
    board.release_at_ms = board.elapsed_ms + 3000;
    board.fail_chip_record_store = true;
    b1.boot();
    // The circle's five objects stay, and the record still vouches for them, as the pairing
    // object does: nothing a phone reads is emptied.
    assert_eq!(circle_objects(&mut b1).len(), KEPT_OBJECTS);
    assert!(b1
        .board()
        .statuses()
        .iter()
        .any(|s| s == "circle forgotten\n5 chip objects could not be deleted\nrelease A"));
    let r = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(r.circle_mask(), 1);
    assert!(chip(&mut b1).exists(id(KeyKind::Public, 0)).unwrap());
}
