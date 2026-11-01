//! Pairing a phone: the inbox, the four words, the phone slot, the hello objects, the public
//! object with its device grant, and the bump at unpair.

mod common;

use cahoots_badge::chip::{self, PhoneSlot, GENERATION, INBOX_LEN, REMAKE_GENERATION};
use cahoots_badge::pairing::{self, Inbox};
use cahoots_badge::public::PublicObject;
use cahoots_badge::{Badge, BadgeError, Board, Press, Step};
use cahoots_chat::{verify_bump, BumpOutcome, ChatSession};
use cahoots_chip::{ChipError, ChipOps, KeyId, KeyKind, SoftChip};
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_core::pairwise;
use cahoots_core::root::{chat_gen_key, gen0_id};
use cahoots_core::seat::SeatKey;
use cahoots_objects::grant::verify_device_grant;
use common::{cable, FakeBoard, FakeLink};
use zeroize::Zeroizing;

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
    mut done: impl FnMut(&Step, &Step) -> bool,
) {
    for _ in 0..10_000 {
        let s1 = b1.step();
        let s2 = b2.step();
        assert!(!matches!(s1, Step::Failed(_)), "card 1: {s1:?}");
        assert!(!matches!(s2, Step::Failed(_)), "card 2: {s2:?}");
        if done(&s1, &s2) {
            return;
        }
    }
    panic!("the badges did not get there");
}

/// Run a pile of two to the keep and keep the circle on both cards.
fn kept_pair(chips: (bool, bool)) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (mut b1, mut b2) = pile_of_two(chips);
    for _ in 0..10_000 {
        b1.step();
        b2.step();
        if has_seat(&mut b1) && has_seat(&mut b2) {
            break;
        }
    }
    b2.board().presses.push_back(Press::Both);
    let mut at = (false, false);
    drive(&mut b1, &mut b2, |s1, s2| {
        at.0 |= *s1 == Step::Prompt;
        at.1 |= *s2 == Step::Prompt;
        at.0 && at.1
    });
    b1.board().presses.push_back(Press::Both);
    b2.board().presses.push_back(Press::Both);
    let mut kept = (false, false);
    drive(&mut b1, &mut b2, |s1, s2| {
        kept.0 |= *s1 == Step::Kept;
        kept.1 |= *s2 == Step::Kept;
        kept.0 && kept.1
    });
    (b1, b2)
}

/// The board's chip in no session, as a tap finds it.
fn tap(b: &mut Badge<FakeBoard>) -> &mut SoftChip {
    let c = b.board().chip.as_mut().expect("a chip");
    c.no_session_for_test();
    c
}

fn manifest(b: &Badge<FakeBoard>) -> Manifest {
    let c = b.circle().unwrap();
    SealedManifest::decode(&c.manifest).unwrap().verify().unwrap()
}

/// The public object of circle slot 0, read in no session as a phone's tap reads it.
fn public(b: &mut Badge<FakeBoard>) -> PublicObject {
    let bytes = tap(b).read_binary(chip::public_id(0)).unwrap();
    PublicObject::decode(&bytes).unwrap()
}

const PHONE_KEY: [u8; 16] = [0xAB; 16];

fn phone_seat() -> SeatKey {
    SeatKey::from_seed(&[0x77; 32]).unwrap()
}

fn phone_inbox(name: &str, key: [u8; 16]) -> Inbox {
    Inbox {
        public: phone_seat().public().0,
        name: name.into(),
        key,
    }
}

/// A phone's first tap: the inbox written in no session.
fn write_inbox(b: &mut Badge<FakeBoard>, inbox: &Inbox) {
    tap(b).rewrite_binary(KeyId::INBOX, &inbox.encode()).unwrap();
}

/// The seat the other card holds.
fn other_seat(b: &Badge<FakeBoard>) -> u8 {
    1 - b.circle().unwrap().seat
}

#[test]
fn keeping_a_circle_writes_hello_objects_and_a_public_object() {
    let (mut b1, mut b2) = kept_pair((true, true));
    let c1 = b1.circle().unwrap().clone();
    let (s1, s2) = (c1.seat, b2.circle().unwrap().seat);
    assert_ne!(s1, s2);

    // One hello object, for the other seat, on each card; none for the card's own seat.
    for (b, other) in [(&mut b1, s2), (&mut b2, s1)] {
        let c = b.board().owner_chip();
        assert!(c.exists(chip::hello_id(0, other)).unwrap());
        let mine = 1 - other;
        assert!(!c.exists(chip::hello_id(0, mine)).unwrap());
    }

    // Both cards' hello objects hold the same value: a slot session on each gets the same
    // words for the same speaker and counter, and they are the words the pair key gives.
    let mut data = pairwise::WORDS_DOMAIN.to_vec();
    data.push(s1);
    data.extend_from_slice(&7u32.to_le_bytes());
    data.push(1);
    let mut macs = Vec::new();
    for (b, other) in [(&mut b1, s2), (&mut b2, s1)] {
        let c = tap(b);
        // In no session a hello object MACs nothing.
        assert_eq!(c.hmac(chip::hello_id(0, other), &data), Err(ChipError::NotAllowed));
        c.open_slot_with_sm_for_test().unwrap();
        macs.push(c.hmac(chip::hello_id(0, other), &data).unwrap());
        c.no_session_for_test();
    }
    assert_eq!(macs[0], macs[1]);

    // The public object: everything a phone needs, no phone yet. Any session reads it, none
    // but the owner's writes it.
    let record = b1.board().chip_record_load().unwrap().unwrap();
    let p = public(&mut b1);
    assert_eq!(
        p,
        PublicObject {
            circle_id: c1.circle_id,
            relay: String::new(),
            manifest: c1.manifest.clone(),
            card: None,
            grant: None,
            generation: gen0_id(&c1.circle_id),
            pending_bump: None,
            badge_nonce: record.badge_nonce,
            phone_public: None,
        }
    );
    assert_eq!(
        tap(&mut b1).rewrite_binary(chip::public_id(0), &[0]),
        Err(ChipError::NotAllowed)
    );
    assert_eq!(tap(&mut b1).delete(chip::public_id(0)), Err(ChipError::NotAllowed));
    assert!(!b1.board().pin_high());
}

#[test]
fn a_hello_object_gives_the_pairwise_words() {
    let key = [0x3C; 32];
    let mut c = SoftChip::with_seed([4; 32]);
    let hello = chip::hello_value(&key);
    c.put_hmac_key(KeyId::new(KeyKind::Hello, 1), &hello, &cahoots_chip::ObjectPolicy::any_session())
        .unwrap();
    for (speaker, counter) in [(0u8, 0u32), (1, 5), (2, 70_000)] {
        let mut data = pairwise::WORDS_DOMAIN.to_vec();
        data.push(speaker);
        data.extend_from_slice(&counter.to_le_bytes());
        data.push(1);
        let t = c.hmac(KeyId::new(KeyKind::Hello, 1), &data).unwrap();
        let v = u32::from_le_bytes(t[..4].try_into().unwrap());
        let words = [
            cahoots_core::checkwords::WORDLIST[(v & 0x7ff) as usize],
            cahoots_core::checkwords::WORDLIST[((v >> 11) & 0x7ff) as usize],
        ];
        assert_eq!(words, pairwise::words(&key, speaker, counter));
    }
}

#[test]
fn the_inbox_round_trips_and_refuses_what_is_not_an_inbox() {
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);
    let mut bytes = inbox.encode();
    assert_eq!(Inbox::decode(&bytes), Some(inbox.clone()));
    // What follows the map is not read: an earlier, longer write.
    bytes.extend_from_slice(&[0xFF; 20]);
    assert_eq!(Inbox::decode(&bytes), Some(inbox.clone()));
    assert!(format!("{inbox:?}").find("171").is_none(), "Debug leaves the key out");
    assert_eq!(Inbox::decode(&[0; INBOX_LEN]), None);
    let with = |public: [u8; 33], name: &str| Inbox { public, name: name.into(), key: PHONE_KEY };
    for bad in [
        with(inbox.public, ""),
        with(inbox.public, &"x".repeat(65)),
        with(
            {
                let mut p = [0xFF; 33];
                p[0] = 2;
                p
            },
            "phone",
        ),
    ] {
        assert_eq!(Inbox::decode(&bad.encode()), None);
    }
    assert!(Inbox::decode(&with(inbox.public, &"x".repeat(64)).encode()).is_some());
}

#[test]
fn the_words_are_four_from_the_check_word_list() {
    let w = pairing::words(&phone_seat().public().0, &[1; 16]);
    assert_eq!(w, pairing::words(&phone_seat().public().0, &[1; 16]));
    assert_ne!(w, pairing::words(&phone_seat().public().0, &[2; 16]));
    assert!(w.iter().all(|x| cahoots_core::checkwords::WORDLIST.contains(x)));
}

#[test]
fn a_phone_pairs_and_its_slot_session_reaches_only_what_a_phone_may() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);

    // A tap writes the inbox and cannot read it back.
    write_inbox(&mut b1, &inbox);
    assert_eq!(tap(&mut b1).read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    assert_eq!(b1.pair_read_inbox(), Some(inbox.clone()));
    assert!(!b1.board().pin_high());

    b1.pair_accept(inbox.clone()).unwrap();
    assert!(!b1.board().pin_high());
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.generation, GENERATION);
    assert_eq!(
        record.phone,
        Some(PhoneSlot { name: "Conner's phone".into(), public: inbox.public })
    );
    // The inbox is empty again.
    assert_eq!(b1.board().owner_chip().read_binary(KeyId::INBOX).unwrap(), vec![0; INBOX_LEN]);
    assert_eq!(b1.pair_read_inbox(), None);

    // The public object holds a device grant for the phone's key, signed by the seat.
    let p = public(&mut b1);
    assert_eq!(p.phone_public, Some(inbox.public));
    assert_eq!(p.generation, gen0_id(&c1.circle_id));
    let (seat, grantee, actions) =
        verify_device_grant(p.grant.as_ref().unwrap(), &manifest(&b1)).unwrap();
    assert_eq!(seat, c1.seat);
    assert_eq!(grantee, phone_seat().public());
    assert_eq!(actions, ["read", "post", "hello"]);
    // The phone draws the same four words from what it wrote and what it read.
    assert_eq!(
        pairing::words(&inbox.public, &p.badge_nonce),
        pairing::words(&inbox.public, &record.badge_nonce)
    );

    // The slot opens with the phone's key and no other.
    let c = tap(&mut b1);
    assert_eq!(c.open_slot_with_key_for_test(&[0xAC; 16]), Err(ChipError::BadSession));
    c.open_slot_with_key_for_test(&PHONE_KEY).unwrap();

    let other = other_seat(&b1);
    let c = b1.board().chip.as_mut().unwrap();
    // In the slot's session: the generation key from the root, and the hello object.
    let mut label = b"chat/gen".to_vec();
    label.extend_from_slice(&c1.circle_id);
    label.extend_from_slice(&gen0_id(&c1.circle_id));
    let key = c.hmac(KeyId::new(KeyKind::Root, 0), &label).unwrap();
    assert_eq!(key, *chat_gen_key(&root, &c1.circle_id, &gen0_id(&c1.circle_id)));
    assert!(c.hmac(chip::hello_id(0, other), b"cahoots/words/v1").is_ok());
    let c = b1.board().chip.as_mut().unwrap();
    assert!(c.read_binary(chip::public_id(0)).is_ok());

    // And nothing else: no signature with the seat key, no inbox, no owner object, no slot,
    // no public object written.
    assert_eq!(c.sign_p256(KeyId::new(KeyKind::Seat, 0), &[1; 32]), Err(ChipError::NotAllowed));
    assert_eq!(c.public(KeyId::new(KeyKind::Seat, 0)), Err(ChipError::NotAllowed));
    assert_eq!(c.read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::OWNER), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::new(KeyKind::Root, 0)), Err(ChipError::NotAllowed));
    assert_eq!(c.rewrite_binary(chip::public_id(0), &[0]), Err(ChipError::NotAllowed));
    assert_eq!(c.counter_next(KeyId::new(KeyKind::Epoch, 0)), Err(ChipError::NotAllowed));

    // One phone per badge.
    let second = phone_inbox("another phone", [0xCD; 16]);
    write_inbox(&mut b1, &second);
    assert_eq!(b1.pair_accept(second), Err(BadgeError::AlreadyPaired));
    b1.board().chip.as_mut().unwrap().open_slot_with_key_for_test(&PHONE_KEY).unwrap();
}

#[test]
fn unpairing_ends_the_taps_and_leaves_a_bump_a_device_can_post() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);
    write_inbox(&mut b1, &inbox);
    b1.pair_accept(inbox).unwrap();

    b1.unpair().unwrap();
    assert!(!b1.board().pin_high());
    // The old key no longer opens the slot, and the placeholder is the owner's alone.
    let c = tap(&mut b1);
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone.clone()), (GENERATION, None));

    // The public object: no phone, no grant, a new generation and the bump to it, signed by
    // the seat from generation 0.
    let p = public(&mut b1);
    assert_eq!((p.phone_public, p.grant.clone()), (None, None));
    let bump = p.pending_bump.clone().unwrap();
    let (from, to, seat) = verify_bump(&bump, &c1.circle_id, &manifest(&b1)).unwrap();
    assert_eq!((from, seat), (gen0_id(&c1.circle_id), c1.seat));
    assert_eq!(p.generation, to);
    assert_eq!(record.entry(0).unwrap().chat_generation, Some(to));
    assert_eq!(b1.unpair(), Err(BadgeError::NotPaired));

    // Another device of the seat holding generation 0's key posts the bump; a member with the
    // root follows it to the generation the badge drew.
    let g0 = gen0_id(&c1.circle_id);
    let mut device = ChatSession::from_generation(
        g0,
        chat_gen_key(&root, &c1.circle_id, &g0),
        c1.circle_id,
        2,
        c1.seat,
    );
    device.mark_synced();
    let wire = device.send_bump(to, &bump).unwrap();
    assert_eq!(device.pending_generation(), Some(to));
    let mut member = ChatSession::new(&root, c1.circle_id, 2, other_seat(&b1));
    let (_, got) = member.receive(&wire).unwrap();
    assert_eq!(got, bump);
    assert_eq!(member.apply_bump(from, to), Ok(BumpOutcome::Moved { dropped_own: false }));
    assert_eq!(member.generation_id(), to);

    // A second pairing works, with a new key, and starts in the new generation; the bump stays
    // for it to post.
    let again = phone_inbox("new phone", [0xCD; 16]);
    write_inbox(&mut b1, &again);
    b1.pair_accept(again.clone()).unwrap();
    let p = public(&mut b1);
    assert_eq!(p.phone_public, Some(again.public));
    assert_eq!(p.generation, to);
    assert_eq!(p.pending_bump, Some(bump));
    assert!(verify_device_grant(p.grant.as_ref().unwrap(), &manifest(&b1)).is_ok());
    let c = tap(&mut b1);
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
    c.open_slot_with_key_for_test(&[0xCD; 16]).unwrap();
    let mut label = b"chat/gen".to_vec();
    label.extend_from_slice(&c1.circle_id);
    label.extend_from_slice(&to);
    assert_eq!(
        Zeroizing::new(c.hmac(KeyId::new(KeyKind::Root, 0), &label).unwrap()),
        chat_gen_key(&root, &c1.circle_id, &to)
    );

    // The next unpair bumps from the generation the last one moved to.
    b1.unpair().unwrap();
    let p = public(&mut b1);
    let (from2, to2, _) = verify_bump(&p.pending_bump.unwrap(), &c1.circle_id, &manifest(&b1)).unwrap();
    assert_eq!(from2, to);
    assert_ne!(to2, to);
}

#[test]
fn a_pairing_cut_off_while_the_slot_is_empty_leaves_it_to_the_next_boot() {
    let (mut b1, _b2) = kept_pair((true, false));
    let before = public(&mut b1);
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);
    write_inbox(&mut b1, &inbox);
    // The slot is deleted, then making it with the phone's key fails.
    b1.board().chip.as_mut().unwrap().fail_next_create_for_test();
    assert_eq!(b1.pair_accept(inbox), Err(BadgeError::Chip(ChipError::Status(0x6A80))));
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.generation, REMAKE_GENERATION);
    assert_eq!(record.phone, None);
    assert!(!b1.board().owner_chip().exists(KeyId::SLOT).unwrap());
    // The public objects are put back as they were.
    assert_eq!(public(&mut b1), before);

    // The next boot makes the slot again as a placeholder.
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone), (GENERATION, None));
    let c = tap(&mut b1);
    assert!(c.exists(KeyId::SLOT).unwrap());
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
}

#[test]
fn an_unpairing_cut_off_while_the_slot_is_empty_still_ends_the_taps() {
    let (mut b1, _b2) = kept_pair((true, false));
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);
    write_inbox(&mut b1, &inbox);
    b1.pair_accept(inbox).unwrap();
    b1.board().chip.as_mut().unwrap().fail_next_create_for_test();
    assert!(b1.unpair().is_err());
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.generation, REMAKE_GENERATION);
    let c = tap(&mut b1);
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::NoSuchKey));
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone), (GENERATION, None));
    assert_eq!(tap(&mut b1).open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
}

/// One gesture, then one idle step.
fn press(b: &mut Badge<FakeBoard>, p: Press) {
    b.board().presses.push_back(p);
    b.idle_step();
}

/// From a just-kept circle's menu to the list of circles, entry `index` marked.
fn to_the_list_at(b: &mut Badge<FakeBoard>, index: usize) {
    b.idle_step();
    for _ in 0..3 {
        press(b, Press::Right);
    }
    press(b, Press::Both);
    assert!(b.board().last().iter().any(|l| l == "circles"), "{:?}", b.board().last());
    for _ in 0..index {
        press(b, Press::Right);
    }
}

#[test]
fn pairing_from_the_list_shows_the_name_and_the_four_words() {
    let (mut b1, _b2) = kept_pair((true, false));
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);
    let nonce = b1.board().chip_record_load().unwrap().unwrap().badge_nonce;
    let words = pairing::words(&inbox.public, &nonce);

    // The list: the circle, "new circle", "pair a phone".
    to_the_list_at(&mut b1, 2);
    assert!(b1.board().shows("> pair a phone"), "{:?}", b1.board().last());

    // No phone has written the inbox yet.
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("no phone has asked to pair"));
    press(&mut b1, Press::Left);

    // Left declines and empties the inbox.
    write_inbox(&mut b1, &inbox);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("pair with Conner's phone?"), "{:?}", b1.board().last());
    for w in words {
        assert!(b1.board().shows(w), "{w} in {:?}", b1.board().last());
    }
    assert!(!b1.board().pin_high());
    press(&mut b1, Press::Left);
    assert!(b1.board().shows("not paired"));
    assert_eq!(b1.pair_read_inbox(), None);
    press(&mut b1, Press::Right);

    // Both, then Both again, pairs.
    write_inbox(&mut b1, &inbox);
    press(&mut b1, Press::Both);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("paired with Conner's phone"), "{:?}", b1.board().last());
    assert_eq!(b1.paired_phone().as_deref(), Some("Conner's phone"));
    assert!(!b1.board().pin_high());

    // Back on the list, "pair a phone" says to unpair first, and "phone" is there.
    press(&mut b1, Press::Right);
    assert!(b1.board().shows("> pair a phone"));
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("unpair the current phone first"));
    press(&mut b1, Press::Right);
    press(&mut b1, Press::Right);
    assert!(b1.board().shows("> phone"), "{:?}", b1.board().last());
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("Conner's phone"), "{:?}", b1.board().last());
    // Left goes back without unpairing.
    press(&mut b1, Press::Left);
    assert_eq!(b1.paired_phone().as_deref(), Some("Conner's phone"));
    press(&mut b1, Press::Both);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("phone unpaired"), "{:?}", b1.board().last());
    assert_eq!(b1.paired_phone(), None);
    assert_eq!(tap(&mut b1).open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));

    // With the phone gone the list has no "phone" entry again, and the mark is on its last.
    press(&mut b1, Press::Right);
    assert!(b1.board().shows("> pair a phone"), "{:?}", b1.board().last());
    assert!(!b1.board().last().iter().any(|l| l.trim_start_matches(['>', ' ']) == "phone"));
}

#[test]
fn a_circle_kept_while_a_phone_is_paired_carries_a_grant_for_it() {
    let (mut b1, mut b2) = pile_of_two((true, false));
    // Pair before any circle: the record, the slot and the inbox exist from the first boot.
    let inbox = phone_inbox("Conner's phone", PHONE_KEY);
    write_inbox(&mut b1, &inbox);
    b1.pair_accept(inbox.clone()).unwrap();
    for _ in 0..10_000 {
        b1.step();
        b2.step();
        if has_seat(&mut b1) && has_seat(&mut b2) {
            break;
        }
    }
    b2.board().presses.push_back(Press::Both);
    let mut at = (false, false);
    drive(&mut b1, &mut b2, |s1, s2| {
        at.0 |= *s1 == Step::Prompt;
        at.1 |= *s2 == Step::Prompt;
        at.0 && at.1
    });
    b1.board().presses.push_back(Press::Both);
    b2.board().presses.push_back(Press::Both);
    let mut kept = (false, false);
    drive(&mut b1, &mut b2, |s1, s2| {
        kept.0 |= *s1 == Step::Kept;
        kept.1 |= *s2 == Step::Kept;
        kept.0 && kept.1
    });
    let p = public(&mut b1);
    assert_eq!(p.phone_public, Some(inbox.public));
    let (_, grantee, _) = verify_device_grant(p.grant.as_ref().unwrap(), &manifest(&b1)).unwrap();
    assert_eq!(grantee, phone_seat().public());
}

#[test]
fn a_public_object_refuses_bytes_that_are_not_one() {
    let o = PublicObject {
        circle_id: [1; 16],
        relay: String::new(),
        manifest: vec![1, 2, 3],
        card: Some(vec![4]),
        grant: None,
        generation: [2; 32],
        pending_bump: Some(vec![5, 6]),
        badge_nonce: [3; 16],
        phone_public: Some([2; 33]),
    };
    let padded = o.padded(400).unwrap();
    assert_eq!(PublicObject::decode(&padded), Ok(o.clone()));
    assert_eq!(PublicObject::decode(&o.encode()), Ok(o.clone()));
    let mut junk = padded.clone();
    junk[399] = 1;
    assert!(PublicObject::decode(&junk).is_err());
    assert!(PublicObject::decode(&[0; 64]).is_err());
    assert_eq!(o.padded(10), None);
    assert!(o.new_len().unwrap() > o.encode().len());
}
