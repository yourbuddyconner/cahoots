//! Pairing a phone: the inbox, the four words, the phone slot, the hello objects, the public
//! object with its device grant, and the bump at unpair.

mod common;

use cahoots_badge::chip::{self, PhoneSlot, GENERATION, INBOX_LEN, REMAKE_GENERATION};
use cahoots_chip::ObjectPolicy;
use cahoots_badge::pairing::{self, Inbox, PairingKey, PairingObject};
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

/// The pairing object, as a phone's tap reads it.
fn tapped(b: &mut Badge<FakeBoard>) -> PairingObject {
    let bytes = tap(b).read_binary(KeyId::PAIRING).unwrap();
    PairingObject::decode(&bytes).unwrap()
}

/// The nonce of the open pairing, as a phone's tap reads it from the pairing object.
fn tapped_nonce(b: &mut Badge<FakeBoard>) -> [u8; 16] {
    tapped(b).nonce
}

/// The phone's ephemeral scalar for the seal.
const PHONE_EPHEMERAL: [u8; 32] = [0x31; 32];

/// An inbox from the honest phone, its AES key sealed to `badge_key`.
fn sealed(nonce: &[u8; 16], badge_key: &[u8; 33], name: &str, key: [u8; 16]) -> Inbox {
    Inbox::sealed(&phone_seat(), nonce, badge_key, name, &key, &PHONE_EPHEMERAL).unwrap()
}

/// What a tap that knows the honest phone's request makes of it with its own AES key: sealed
/// to the badge's key (anyone can), carrying the honest phone's public key and signature.
fn swapped_key(honest: &Inbox, nonce: &[u8; 16], badge_key: &[u8; 33], key: [u8; 16]) -> Inbox {
    let other = SeatKey::from_seed(&[0x99; 32]).unwrap();
    let mut s = Inbox::sealed(&other, nonce, badge_key, &honest.name, &key, &[0x32; 32]).unwrap();
    s.public = honest.public;
    s.sig = honest.sig;
    s
}

/// A phone's first tap: it reads the nonce and the badge's key and writes a signed, sealed
/// request in the inbox, in no session.
fn phone_taps(b: &mut Badge<FakeBoard>, name: &str, key: [u8; 16]) -> Inbox {
    let p = tapped(b);
    let inbox = sealed(&p.nonce, &p.badge_key, name, key);
    write_inbox(b, &inbox);
    inbox
}

fn write_inbox(b: &mut Badge<FakeBoard>, inbox: &Inbox) {
    tap(b).rewrite_binary(KeyId::INBOX, &inbox.encode()).unwrap();
}

/// Open a pairing, let the phone tap, read the request and accept it.
fn pair(b: &mut Badge<FakeBoard>, name: &str, key: [u8; 16]) -> Inbox {
    b.pair_open().unwrap();
    let inbox = phone_taps(b, name, key);
    let read = b.pair_read_inbox().unwrap().unwrap();
    assert_eq!(read, inbox);
    b.pair_accept(read).unwrap();
    inbox
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
    // words for the same speaker and counter.
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
            pending_bumps: Vec::new(),
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
    c.put_hmac_key(KeyId::new(KeyKind::Hello, 1), &hello, &ObjectPolicy::any_session())
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
fn the_inbox_round_trips_checks_its_signature_and_refuses_what_is_not_an_inbox() {
    let nonce = [6; 16];
    let badge = PairingKey::from_scalar(&[0x21; 32]).unwrap();
    let inbox = sealed(&nonce, &badge.public(), "Conner's phone", PHONE_KEY);
    assert!(inbox.verify(&nonce, &badge.public()));
    assert!(!inbox.verify(&[7; 16], &badge.public()));
    assert_eq!(*inbox.open(&nonce, &badge).unwrap(), PHONE_KEY);
    // The AES key is not in the inbox in the clear.
    let mut bytes = inbox.encode();
    assert!(!bytes.windows(16).any(|w| w == PHONE_KEY));
    assert_eq!(Inbox::decode(&bytes), Some(inbox.clone()));
    // What follows the map is not read: an earlier, longer write.
    bytes.extend_from_slice(&[0xFF; 20]);
    assert_eq!(Inbox::decode(&bytes), Some(inbox.clone()));
    assert_eq!(Inbox::decode(&[0; INBOX_LEN]), None);
    // The signature covers the sealed key and the name.
    let mut other_seal = inbox.clone();
    other_seal.sealed[40] ^= 1;
    assert!(!other_seal.verify(&nonce, &badge.public()));
    assert_eq!(other_seal.open(&nonce, &badge), None);
    let mut other_name = inbox.clone();
    other_name.name = "a stranger".into();
    assert!(!other_name.verify(&nonce, &badge.public()));
    let with = |public: [u8; 33], name: &str| Inbox {
        public,
        name: name.into(),
        sealed: inbox.sealed,
        sig: inbox.sig,
    };
    let mut bad_point = [0xFF; 33];
    bad_point[0] = 2;
    for bad in [
        with(inbox.public, ""),
        with(inbox.public, &"x".repeat(65)),
        with(bad_point, "phone"),
    ] {
        assert_eq!(Inbox::decode(&bad.encode()), None);
    }
    let longest = sealed(&nonce, &badge.public(), &"x".repeat(64), PHONE_KEY);
    assert!(longest.encode().len() <= INBOX_LEN);
    assert!(Inbox::decode(&longest.encode()).is_some());
}

#[test]
fn an_inbox_sealed_to_another_badge_key_does_not_open() {
    let nonce = [6; 16];
    let badge = PairingKey::from_scalar(&[0x21; 32]).unwrap();
    let other = PairingKey::from_scalar(&[0x22; 32]).unwrap();
    let inbox = sealed(&nonce, &other.public(), "Conner's phone", PHONE_KEY);
    assert!(!inbox.verify(&nonce, &badge.public()));
    assert_eq!(inbox.open(&nonce, &badge), None);
    // Signed for this badge's key but sealed to the other: the seal does not open.
    let mut resigned = sealed(&nonce, &badge.public(), "Conner's phone", PHONE_KEY);
    resigned.sealed = inbox.sealed;
    resigned.sig = phone_seat().sign(&pairing::signed_bytes(&nonce, &badge.public(), &resigned.sealed, &resigned.name));
    assert!(resigned.verify(&nonce, &badge.public()));
    assert_eq!(resigned.open(&nonce, &badge), None);
}

#[test]
fn the_words_cover_the_nonce_the_badge_key_the_phone_key_and_the_aes_key() {
    let public = phone_seat().public().0;
    let badge = PairingKey::from_scalar(&[0x21; 32]).unwrap().public();
    let other = PairingKey::from_scalar(&[0x22; 32]).unwrap().public();
    let w = pairing::words(&[1; 16], &badge, &public, &PHONE_KEY);
    assert_eq!(w, pairing::words(&[1; 16], &badge, &public, &PHONE_KEY));
    assert_ne!(w, pairing::words(&[2; 16], &badge, &public, &PHONE_KEY));
    assert_ne!(w, pairing::words(&[1; 16], &other, &public, &PHONE_KEY));
    assert_ne!(w, pairing::words(&[1; 16], &badge, &public, &[0xAC; 16]));
    assert!(w.iter().all(|x| cahoots_core::checkwords::WORDLIST.contains(x)));
}

#[test]
fn a_phone_pairs_and_its_slot_session_reaches_only_what_a_phone_may() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();

    // No pairing is open: the pairing object does not exist yet, and nothing is read.
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotOpen));
    let nonce = b1.pair_open().unwrap();
    assert!(!b1.board().pin_high());
    assert_eq!(tapped_nonce(&mut b1), nonce);
    // A tap reads the pairing object and cannot write it.
    assert_eq!(tap(&mut b1).rewrite_binary(KeyId::PAIRING, &[0]), Err(ChipError::NotAllowed));
    assert_eq!(b1.pair_read_inbox(), Ok(None));

    // A tap writes the inbox and cannot read it back.
    let inbox = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    assert_eq!(tap(&mut b1).read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    assert_eq!(b1.pair_read_inbox(), Ok(Some(inbox.clone())));
    assert!(!b1.board().pin_high());

    b1.pair_accept(inbox.clone()).unwrap();
    assert!(!b1.board().pin_high());
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.generation, GENERATION);
    assert_eq!(
        record.phone,
        Some(PhoneSlot { name: "Conner's phone".into(), public: inbox.public })
    );
    // The inbox is empty again and the pairing is closed.
    assert_eq!(b1.board().owner_chip().read_binary(KeyId::INBOX).unwrap(), vec![0; INBOX_LEN]);
    assert_eq!(tapped_nonce(&mut b1), [0; 16]);
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotOpen));

    // The public object holds a device grant for the phone's key, signed by the seat.
    let p = public(&mut b1);
    assert_eq!(p.phone_public, Some(inbox.public));
    assert_eq!(p.generation, gen0_id(&c1.circle_id));
    let (seat, grantee, actions) =
        verify_device_grant(p.grant.as_ref().unwrap(), &manifest(&b1)).unwrap();
    assert_eq!(seat, c1.seat);
    assert_eq!(grantee, phone_seat().public());
    assert_eq!(actions, ["read", "post", "hello"]);

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
    assert!(c.read_binary(chip::public_id(0)).is_ok());

    // And nothing else: no signature with the seat key, no inbox, no owner object, no slot,
    // no public or pairing object written.
    assert_eq!(c.sign_p256(KeyId::new(KeyKind::Seat, 0), &[1; 32]), Err(ChipError::NotAllowed));
    assert_eq!(c.public(KeyId::new(KeyKind::Seat, 0)), Err(ChipError::NotAllowed));
    assert_eq!(c.read_binary(KeyId::INBOX), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::OWNER), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    assert_eq!(c.delete(KeyId::new(KeyKind::Root, 0)), Err(ChipError::NotAllowed));
    assert_eq!(c.rewrite_binary(chip::public_id(0), &[0]), Err(ChipError::NotAllowed));
    assert_eq!(c.rewrite_binary(KeyId::PAIRING, &[0]), Err(ChipError::NotAllowed));
    assert_eq!(c.counter_next(KeyId::new(KeyKind::Epoch, 0)), Err(ChipError::NotAllowed));

    // One phone per badge.
    assert_eq!(b1.pair_open(), Err(BadgeError::AlreadyPaired));
    b1.board().chip.as_mut().unwrap().open_slot_with_key_for_test(&PHONE_KEY).unwrap();
}

#[test]
fn a_request_with_the_honest_key_and_another_aes_key_is_refused() {
    let (mut b1, _b2) = kept_pair((true, false));
    b1.pair_open().unwrap();
    let honest = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    // A second tap keeps the honest public key, name and signature, and seals its own key.
    let p = tapped(&mut b1);
    let swapped = swapped_key(&honest, &p.nonce, &p.badge_key, [0x66; 16]);
    write_inbox(&mut b1, &swapped);
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotForThisBadge));
    // The inbox was emptied, and nothing was paired.
    assert_eq!(b1.board().owner_chip().read_binary(KeyId::INBOX).unwrap(), vec![0; INBOX_LEN]);
    assert_eq!(b1.pair_read_inbox(), Ok(None));
    // Accepting it anyway is refused too.
    assert_eq!(b1.pair_accept(swapped), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.paired_phone(), None);
    assert_eq!(tap(&mut b1).open_slot_with_key_for_test(&[0x66; 16]), Err(ChipError::BadSession));
}

#[test]
fn a_request_with_a_wrong_signature_is_refused() {
    let (mut b1, _b2) = kept_pair((true, false));
    b1.pair_open().unwrap();
    let mut forged = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    forged.sig[10] ^= 1;
    write_inbox(&mut b1, &forged);
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.pair_accept(forged), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.paired_phone(), None);
}

#[test]
fn a_request_sealed_to_another_pairing_key_is_refused() {
    let (mut b1, _b2) = kept_pair((true, false));
    b1.pair_open().unwrap();
    let nonce = tapped_nonce(&mut b1);
    let stranger = PairingKey::from_scalar(&[0x23; 32]).unwrap();
    let inbox = sealed(&nonce, &stranger.public(), "Conner's phone", PHONE_KEY);
    write_inbox(&mut b1, &inbox);
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.pair_accept(inbox), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.paired_phone(), None);
}

/// A recorded inbox: the same bytes written again after the page reopens, under a new nonce
/// and a new badge key.
#[test]
fn a_request_signed_for_an_earlier_pairing_is_refused() {
    let (mut b1, _b2) = kept_pair((true, false));
    b1.pair_open().unwrap();
    let old = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    b1.pair_close();
    assert_eq!(tapped_nonce(&mut b1), [0; 16]);
    // Opening again empties the inbox and draws a new nonce.
    b1.pair_open().unwrap();
    assert_eq!(b1.pair_read_inbox(), Ok(None));
    write_inbox(&mut b1, &old);
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.pair_accept(old), Err(BadgeError::NotForThisBadge));
    assert_eq!(b1.paired_phone(), None);
}

#[test]
fn an_inbox_from_earlier_firmware_is_made_again_at_its_new_size() {
    let (mut b1, _b2) = kept_pair((true, false));
    let c = b1.board().owner_chip();
    c.delete(KeyId::INBOX).unwrap();
    c.write_binary(KeyId::INBOX, &[0; 128], &ObjectPolicy::inbox()).unwrap();
    b1.pair_open().unwrap();
    assert_eq!(b1.board().owner_chip().read_binary(KeyId::INBOX).unwrap().len(), INBOX_LEN);
    assert_eq!(b1.board().chip_record_load().unwrap().unwrap().generation, GENERATION);
    let inbox = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    b1.pair_accept(inbox).unwrap();
}

#[test]
fn unpairing_ends_the_taps_and_leaves_a_bump_a_device_can_post() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    pair(&mut b1, "Conner's phone", PHONE_KEY);

    b1.unpair().unwrap();
    assert!(!b1.board().pin_high());
    // The old key no longer opens the slot, and the placeholder is the owner's alone.
    let c = tap(&mut b1);
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone.clone()), (GENERATION, None));
    assert_eq!(record.pending_unpair, None);

    // The public object: no phone, no grant, a new generation and the bump to it, signed by
    // the seat from generation 0.
    let p = public(&mut b1);
    assert_eq!((p.phone_public, p.grant.clone()), (None, None));
    let [bump] = p.pending_bumps.as_slice() else { panic!("{:?}", p.pending_bumps) };
    let (from, to, seat) = verify_bump(bump, &c1.circle_id, &manifest(&b1)).unwrap();
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
    let wire = device.send_bump(to, bump).unwrap();
    assert_eq!(device.pending_generation(), Some(to));
    let mut member = ChatSession::new(&root, c1.circle_id, 2, other_seat(&b1));
    let (_, got) = member.receive(&wire).unwrap();
    assert_eq!(&got, bump);
    assert_eq!(member.apply_bump(from, to), Ok(BumpOutcome::Moved { dropped_own: false }));
    assert_eq!(member.generation_id(), to);

    // A second pairing works, with a new key, and starts in the new generation; the bump stays
    // for it to post.
    let again = pair(&mut b1, "new phone", [0xCD; 16]);
    let p = public(&mut b1);
    assert_eq!(p.phone_public, Some(again.public));
    assert_eq!(p.generation, to);
    assert_eq!(p.pending_bumps.as_slice(), std::slice::from_ref(bump));
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
}

/// A device of the seat holding generation `from`'s key posts `bump`; `member` reads it and
/// applies it.
fn post_and_apply(
    member: &mut ChatSession,
    root: &[u8; 32],
    c: &cahoots_badge::Circle,
    bump: &[u8],
) -> BumpOutcome {
    let from: [u8; 32] = bump[..32].try_into().unwrap();
    let to: [u8; 32] = bump[32..64].try_into().unwrap();
    let mut device = ChatSession::from_generation(
        from,
        chat_gen_key(root, &c.circle_id, &from),
        c.circle_id,
        2,
        c.seat,
    );
    device.mark_synced();
    let wire = device.send_bump(to, bump).unwrap();
    match member.receive(&wire) {
        Ok(_) => member.apply_bump(from, to).unwrap(),
        // Sealed in a generation the member has left and no longer holds.
        Err(_) => BumpOutcome::Stale,
    }
}

/// Each bump in `list` verifies, and each moves from where the one before moved to.
fn chain(b: &Badge<FakeBoard>, list: &[Vec<u8>]) -> Vec<([u8; 32], [u8; 32])> {
    let c = b.circle().unwrap();
    let ends: Vec<_> = list
        .iter()
        .map(|x| {
            let (f, t, _) = verify_bump(x, &c.circle_id, &manifest(b)).unwrap();
            (f, t)
        })
        .collect();
    for w in ends.windows(2) {
        assert_eq!(w[0].1, w[1].0);
    }
    ends
}

#[test]
fn an_unpair_after_a_phone_that_never_tapped_appends_and_a_member_follows_both() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    let g0 = gen0_id(&c1.circle_id);
    pair(&mut b1, "first phone", PHONE_KEY);
    b1.unpair().unwrap();
    pair(&mut b1, "second phone", [0xCD; 16]);
    b1.unpair().unwrap();

    // The second phone may have posted W to X, so the list is [W to X, X to Y].
    let p = public(&mut b1);
    let ends = chain(&b1, &p.pending_bumps);
    assert_eq!(ends.len(), 2);
    assert_eq!(ends[0].0, g0);
    assert_eq!(p.generation, ends[1].1);
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.entry(0).unwrap().chat_generation, Some(ends[1].1));
    assert!(!record.paired_since_bump(0));

    // A member still in W follows both, in order.
    let mut member = ChatSession::new(&root, c1.circle_id, 2, other_seat(&b1));
    for bump in &p.pending_bumps {
        assert_eq!(post_and_apply(&mut member, &root, &c1, bump), BumpOutcome::Moved { dropped_own: false });
    }
    assert_eq!(member.generation_id(), p.generation);
}

#[test]
fn an_unpair_after_a_phone_that_posted_the_first_bump_moves_members_in_x() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    pair(&mut b1, "first phone", PHONE_KEY);
    b1.unpair().unwrap();
    let first = public(&mut b1).pending_bumps;
    // The second phone taps and posts W to X: the member moves to X.
    pair(&mut b1, "second phone", [0xCD; 16]);
    let mut member = ChatSession::new(&root, c1.circle_id, 2, other_seat(&b1));
    assert_eq!(post_and_apply(&mut member, &root, &c1, &first[0]), BumpOutcome::Moved { dropped_own: false });
    let x = member.generation_id();
    b1.unpair().unwrap();

    // The list is the same as without the tap. The first bump, posted again, moves nothing;
    // the second moves the member from X.
    let p = public(&mut b1);
    let ends = chain(&b1, &p.pending_bumps);
    assert_eq!(ends.len(), 2);
    assert_eq!(p.pending_bumps[0], first[0]);
    assert_eq!(ends[1].0, x);
    assert_eq!(post_and_apply(&mut member, &root, &c1, &p.pending_bumps[0]), BumpOutcome::Stale);
    assert_eq!(member.generation_id(), x);
    assert_eq!(
        post_and_apply(&mut member, &root, &c1, &p.pending_bumps[1]),
        BumpOutcome::Moved { dropped_own: false }
    );
    assert_eq!(member.generation_id(), p.generation);
}

#[test]
fn a_bump_no_phone_could_have_posted_is_replaced_from_where_it_started() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    let g0 = gen0_id(&c1.circle_id);
    pair(&mut b1, "first phone", PHONE_KEY);
    b1.unpair().unwrap();
    pair(&mut b1, "second phone", [0xCD; 16]);
    // As if the second phone had been paired before the bump was made.
    let mut record = b1.board().chip_record_load().unwrap().unwrap();
    assert!(record.paired_since_bump(0));
    record.set_paired_since_bump(0, false);
    b1.board().chip_record_store(&record).unwrap();
    b1.unpair().unwrap();
    let p = public(&mut b1);
    let ends = chain(&b1, &p.pending_bumps);
    assert_eq!(ends, vec![(g0, p.generation)]);
    let mut member = ChatSession::new(&root, c1.circle_id, 2, other_seat(&b1));
    assert_eq!(
        post_and_apply(&mut member, &root, &c1, &p.pending_bumps[0]),
        BumpOutcome::Moved { dropped_own: false }
    );
    assert_eq!(member.generation_id(), p.generation);
}

#[test]
fn the_bump_list_stays_at_four_with_the_oldest_two_joined() {
    let (mut b1, b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let root = b2.circle().unwrap().root.unwrap();
    let g0 = gen0_id(&c1.circle_id);
    let mut seen = Vec::new();
    for i in 0..5u8 {
        pair(&mut b1, "a phone", [i; 16]);
        b1.unpair().unwrap();
        seen.push(public(&mut b1).generation);
    }
    let p = public(&mut b1);
    assert_eq!(p.pending_bumps.len(), 4);
    let ends = chain(&b1, &p.pending_bumps);
    // The oldest two, g0 to the first and the first to the second, became one.
    assert_eq!(ends[0], (g0, seen[1]));
    assert_eq!(ends[1..].iter().map(|e| e.1).collect::<Vec<_>>(), seen[2..].to_vec());
    // A member in g0 follows all four.
    let mut member = ChatSession::new(&root, c1.circle_id, 2, other_seat(&b1));
    for bump in &p.pending_bumps {
        assert_eq!(post_and_apply(&mut member, &root, &c1, bump), BumpOutcome::Moved { dropped_own: false });
    }
    assert_eq!(member.generation_id(), p.generation);
    // A phone paired afterwards still finds four.
    for i in 0..6u8 {
        pair(&mut b1, "another phone", [0x40 + i; 16]);
        b1.unpair().unwrap();
    }
    assert_eq!(public(&mut b1).pending_bumps.len(), 4);
}

#[test]
fn an_unpair_left_unfinished_keeps_a_phone_paired_since_and_blocks_a_pairing_until_done() {
    let (mut b1, _b2) = kept_pair((true, false));
    let phone = pair(&mut b1, "Conner's phone", PHONE_KEY);
    // A bump left in the record from an earlier unpair is written at boot without taking the
    // paired phone's grant or key off the public object.
    let mut record = b1.board().chip_record_load().unwrap().unwrap();
    record.pending_unpair =
        Some(vec![chip::PendingBump { slot: 0, to: [9; 32], bumps: vec![vec![1, 2, 3].into()] }]);
    b1.board().chip_record_store(&record).unwrap();
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    let p = public(&mut b1);
    assert_eq!(p.phone_public, Some(phone.public));
    assert!(verify_device_grant(p.grant.as_ref().unwrap(), &manifest(&b1)).is_ok());
    assert_eq!((p.pending_bumps, p.generation), (vec![vec![1, 2, 3]], [9; 32]));
    assert_eq!(b1.board().chip_record_load().unwrap().unwrap().pending_unpair, None);

    // An unpair whose public object cannot be written leaves its bump in the record, and no
    // phone pairs until it is written.
    let c = b1.board().owner_chip();
    c.delete(chip::public_id(0)).unwrap();
    c.write_binary(chip::public_id(0), &[0], &ObjectPolicy::public_object()).unwrap();
    assert_eq!(b1.unpair(), Err(BadgeError::NotAllUpdated));
    assert!(b1.board().chip_record_load().unwrap().unwrap().pending_unpair.is_some());
    b1.pair_open().unwrap();
    let inbox = phone_taps(&mut b1, "new phone", [0xCD; 16]);
    assert_eq!(b1.pair_accept(inbox), Err(BadgeError::UnpairUnfinished));
    assert_eq!(b1.paired_phone(), None);
    assert_eq!(tap(&mut b1).open_slot_with_key_for_test(&[0xCD; 16]), Err(ChipError::BadSession));
}

#[test]
fn a_boot_closes_a_pairing_left_open() {
    let (mut b1, _b2) = kept_pair((true, false));
    b1.pair_open().unwrap();
    assert_ne!(tapped_nonce(&mut b1), [0; 16]);
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    assert_eq!(tapped_nonce(&mut b1), [0; 16]);
    assert_eq!(b1.pair_read_inbox(), Err(BadgeError::NotOpen));
}

#[test]
fn a_pairing_whose_slot_key_is_not_made_leaves_a_placeholder() {
    let (mut b1, _b2) = kept_pair((true, false));
    let before = public(&mut b1);
    b1.pair_open().unwrap();
    let inbox = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    // The slot is deleted, then making it with the phone's key fails; the placeholder is made.
    b1.board().chip.as_mut().unwrap().fail_next_create_for_test();
    assert_eq!(b1.pair_accept(inbox), Err(BadgeError::Chip(ChipError::Status(0x6A80))));
    assert!(!b1.board().pin_high());
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone), (GENERATION, None));
    assert_eq!(public(&mut b1), before);
    let c = tap(&mut b1);
    assert!(c.exists(KeyId::SLOT).unwrap());
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
}

#[test]
fn a_pairing_that_leaves_the_slot_empty_holds_the_chip_until_the_next_boot() {
    let (mut b1, _b2) = kept_pair((true, false));
    let before = public(&mut b1);
    b1.pair_open().unwrap();
    let inbox = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    // The phone's key and both placeholders fail: the slot's id is empty.
    b1.board().chip.as_mut().unwrap().fail_next_creates_for_test(3);
    assert_eq!(b1.pair_accept(inbox), Err(BadgeError::ChipHeld));
    // The pin stays high, so no tap reaches the chip, and the badge uses it no more. The
    // pairing object says no pairing is open.
    assert!(b1.board().pin_high());
    assert_eq!(tapped_nonce(&mut b1), [0; 16]);
    b1.idle_step();
    assert!(b1.board().pin_high());
    // The list says why the chip is gone, after the restart note.
    for _ in 0..3 {
        press(&mut b1, Press::Right);
    }
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("chip needs a restart"), "{:?}", b1.board().last());
    assert!(!b1.board().last().iter().any(|l| l.ends_with("pair a phone")));
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.generation, REMAKE_GENERATION);
    assert!(!b1.board().owner_chip().exists(KeyId::SLOT).unwrap());
    assert_eq!(public(&mut b1), before);

    // Turned off and on, the badge makes the slot again as a placeholder.
    b1.board().power_cycle();
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    assert!(!b1.board().pin_high());
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone), (GENERATION, None));
    let c = tap(&mut b1);
    assert!(c.exists(KeyId::SLOT).unwrap());
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
    assert_eq!(c.delete(KeyId::SLOT), Err(ChipError::NotAllowed));
}

#[test]
fn an_unpair_cut_off_before_the_public_objects_is_finished_at_the_next_boot() {
    let (mut b1, _b2) = kept_pair((true, false));
    let c1 = b1.circle().unwrap().clone();
    let inbox = pair(&mut b1, "Conner's phone", PHONE_KEY);
    // The placeholder is never made: the chip is held with the bumps stored and no public
    // object written, as a power cut there leaves it.
    b1.board().chip.as_mut().unwrap().fail_next_creates_for_test(2);
    assert_eq!(b1.unpair(), Err(BadgeError::ChipHeld));
    assert!(b1.board().pin_high());
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.generation, REMAKE_GENERATION);
    let [pending] = record.pending_unpair.clone().unwrap().try_into().unwrap();
    assert_eq!(public(&mut b1).phone_public, Some(inbox.public));
    assert!(!b1.board().owner_chip().exists(KeyId::SLOT).unwrap());

    b1.board().power_cycle();
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    assert!(b1.board().log.iter().any(|l| l == "pairing: finishing an unpair that was cut off"));
    let record = b1.board().chip_record_load().unwrap().unwrap();
    assert_eq!((record.generation, record.phone.clone()), (GENERATION, None));
    assert_eq!(record.pending_unpair, None);
    assert_eq!(record.entry(0).unwrap().chat_generation, Some(pending.to));
    let p = public(&mut b1);
    assert_eq!((p.phone_public, p.grant.clone()), (None, None));
    let payload = pending.bumps[0].to_vec();
    assert_eq!(p.pending_bumps, vec![payload.clone()]);
    assert_eq!(p.generation, pending.to);
    let (from, to, _) = verify_bump(&payload, &c1.circle_id, &manifest(&b1)).unwrap();
    assert_eq!((from, to), (gen0_id(&c1.circle_id), pending.to));
    let c = tap(&mut b1);
    assert_eq!(c.open_slot_with_key_for_test(&PHONE_KEY), Err(ChipError::BadSession));
    // A second boot changes nothing more: the bump is there once.
    b1.board().presses.push_back(Press::Both);
    b1.boot();
    assert_eq!(public(&mut b1).pending_bumps, vec![payload]);
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

    // The list: the circle, "new circle", "pair a phone".
    to_the_list_at(&mut b1, 2);
    assert!(b1.board().shows("> pair a phone"), "{:?}", b1.board().last());

    // Opening the page opens a pairing: the nonce is on the chip for a tap, and the pin is
    // low so the phone can tap.
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("hold the phone"), "{:?}", b1.board().last());
    assert!(!b1.board().pin_high());
    assert_ne!(tapped_nonce(&mut b1), [0; 16]);
    // Both before a tap: nothing there yet.
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("no phone has tapped yet"), "{:?}", b1.board().last());
    // A request with another AES key is refused and emptied.
    let p = tapped(&mut b1);
    let honest = sealed(&p.nonce, &p.badge_key, "Conner's phone", PHONE_KEY);
    let swapped = swapped_key(&honest, &p.nonce, &p.badge_key, [0x66; 16]);
    write_inbox(&mut b1, &swapped);
    press(&mut b1, Press::Both);
    assert!(
        b1.board().shows("that phone's request is not for this badge. Try again."),
        "{:?}",
        b1.board().last()
    );

    // The phone taps; Both shows the name and the words the phone shows.
    let inbox = phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    let p = tapped(&mut b1);
    let words = pairing::words(&p.nonce, &p.badge_key, &inbox.public, &PHONE_KEY);
    assert_eq!(b1.pair_words(&inbox), Some(words));
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("pair with Conner's phone?"), "{:?}", b1.board().last());
    for w in words {
        assert!(b1.board().shows(w), "{w} in {:?}", b1.board().last());
    }
    assert!(!b1.board().pin_high());
    // Left declines, empties the inbox and closes the pairing.
    press(&mut b1, Press::Left);
    assert!(b1.board().shows("not paired"));
    assert_eq!(tapped_nonce(&mut b1), [0; 16]);
    press(&mut b1, Press::Right);

    // Open, tap, Both, then Both again, pairs.
    press(&mut b1, Press::Both);
    phone_taps(&mut b1, "Conner's phone", PHONE_KEY);
    press(&mut b1, Press::Both);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("paired with Conner's phone"), "{:?}", b1.board().last());
    assert_eq!(b1.paired_phone().as_deref(), Some("Conner's phone"));
    assert_eq!(tapped_nonce(&mut b1), [0; 16]);
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
fn a_phone_pairs_with_no_circles_and_a_circle_kept_later_carries_a_grant_for_it() {
    let (mut b1, mut b2) = pile_of_two((true, false));
    // No circle yet: the phone pairs for the circles to come.
    let inbox = pair(&mut b1, "Conner's phone", PHONE_KEY);
    assert_eq!(b1.paired_phone().as_deref(), Some("Conner's phone"));
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
        pending_bumps: vec![vec![5, 6], vec![7]],
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

#[test]
fn the_pairing_object_round_trips_and_is_no_circles() {
    let o = PairingObject { nonce: [9; 16], badge_key: [2; 33], name: String::new() };
    let bytes = o.padded().unwrap();
    assert_eq!(bytes.len(), pairing::PAIRING_LEN);
    assert_eq!(PairingObject::decode(&bytes), Some(o));
    assert_eq!(KeyId::PAIRING.circle_slot(), None);
    assert!(KeyId::PAIRING.is_badge_wide());
}
