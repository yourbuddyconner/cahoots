//! Card circles end to end: a phone pairs with a badge in memory (its software chip reached
//! through a fake NFC reader over the chip's APDU front end), keeps the badge's circle as a
//! card circle, and chats in it beside a member that holds the root, through a relay in
//! process.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_badge::chip::{self as badge_chip, ChipSeat};
use cahoots_badge::test_support::{cable, FakeBoard, FakeLink};
use cahoots_badge::{Badge, Press, Step};
use cahoots_chat::{bucket_pad, bucket_unpad, sign_bump, verify_bump, ChatSession, MessageType};
use cahoots_chip::{ChipOps, KeyId, KeyKind, SoftChip};
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_core::seat::SeatKey;
use cahoots_phone::nfc::Nfc;
use cahoots_phone::signer::SoftSigner;
use cahoots_phone::store::CircleRecord;
use cahoots_phone::{Http, Phone, PhoneError, Signer};
use cahoots_relay::{MemRelay, Relay};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const RELAY: &str = "https://relay.test";

/// A reader whose card is whatever chip a test puts in it, or none.
#[derive(Clone, Default)]
struct FakeNfc(Arc<Mutex<Option<SoftChip>>>);

impl Nfc for FakeNfc {
    fn begin(&self, _prompt: String) -> Result<(), PhoneError> {
        match *self.0.lock().unwrap() {
            Some(_) => Ok(()),
            None => Err(PhoneError::NoCard),
        }
    }
    fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        let mut card = self.0.lock().unwrap();
        Ok(card.as_mut().ok_or(PhoneError::NoCard)?.apdu(&apdu))
    }
    fn end(&self, _message: String, _failed: bool) -> Result<(), PhoneError> {
        Ok(())
    }
}

/// The relay's HTTP interface over a `MemRelay`, and a switch that takes it off the network.
#[derive(Clone)]
struct FakeHttp(Arc<Mutex<MemRelay>>, Arc<AtomicBool>);

impl FakeHttp {
    fn reach(&self) -> Result<(), PhoneError> {
        if self.1.load(Ordering::SeqCst) {
            return Err(PhoneError::Network { message: "offline".into() });
        }
        Ok(())
    }
}

fn circle_of(url: &str) -> [u8; 16] {
    let rest = url.strip_prefix(&format!("{RELAY}/v1/circles/")).expect("relay url");
    hex::decode(&rest[..32]).unwrap().try_into().unwrap()
}

impl Http for FakeHttp {
    fn get(&self, url: String) -> Result<Vec<u8>, PhoneError> {
        self.reach()?;
        let after = url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("after="))
            .map(|a| a.parse::<u64>().unwrap());
        let entries = self.0.lock().unwrap().poll(&circle_of(&url), after, 0).unwrap();
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(seq, m)| serde_json::json!({ "seq": seq, "message": URL_SAFE_NO_PAD.encode(m) }))
            .collect();
        Ok(serde_json::to_vec(&serde_json::json!({ "entries": entries })).unwrap())
    }
    fn post(&self, url: String, body: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        self.reach()?;
        let seq = self.0.lock().unwrap().post(&circle_of(&url), &body).unwrap();
        Ok(serde_json::to_vec(&serde_json::json!({ "seq": seq })).unwrap())
    }
    fn request(
        &self,
        _method: String,
        _url: String,
        _headers: Vec<cahoots_phone::HttpHeader>,
        _body: Option<Vec<u8>>,
    ) -> Result<cahoots_phone::HttpResponse, PhoneError> {
        Ok(cahoots_phone::HttpResponse { status: 404, body: Vec::new() })
    }
}

struct Device {
    _dir: TempDir,
    path: std::path::PathBuf,
    signer: SoftSigner,
    nfc: FakeNfc,
    http: FakeHttp,
    phone: Arc<Phone>,
}

fn open(path: &std::path::Path, signer: &SoftSigner, http: &FakeHttp, nfc: &FakeNfc) -> Arc<Phone> {
    Phone::open(
        path.to_string_lossy().into_owned(),
        Box::new(signer.clone()),
        Box::new(http.clone()),
        Box::new(nfc.clone()),
    )
    .unwrap()
}

fn device(relay: &Arc<Mutex<MemRelay>>) -> Device {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    let signer = SoftSigner::new();
    let nfc = FakeNfc::default();
    let http = FakeHttp(relay.clone(), Arc::new(AtomicBool::new(false)));
    let phone = open(&path, &signer, &http, &nfc);
    phone.set_default_relay(RELAY.into()).unwrap();
    Device { _dir: dir, path, signer, nfc, http, phone }
}

impl Device {
    /// The app is closed and opened again: everything is read from disk.
    fn restart(&mut self) {
        self.phone = open(&self.path, &self.signer, &self.http, &self.nfc);
    }
}

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

fn drive(b1: &mut Badge<FakeBoard>, b2: &mut Badge<FakeBoard>, mut done: impl FnMut(&Step, &Step) -> bool) {
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

/// Hold the phone to the badge: its chip goes to the reader for `f`, in no session as a tap
/// finds it, and comes back after.
fn tap<T>(b: &mut Badge<FakeBoard>, d: &Device, f: impl FnOnce(&Phone) -> T) -> T {
    let mut chip = b.board().chip.take().expect("a chip");
    chip.no_session_for_test();
    *d.nfc.0.lock().unwrap() = Some(chip);
    let r = f(&d.phone);
    b.board().chip = d.nfc.0.lock().unwrap().take();
    r
}

/// The whole pairing: the badge opens it, the phone taps, the words match, the owner presses
/// both buttons, the phone taps again. Returns the card circles kept.
fn pair(b: &mut Badge<FakeBoard>, d: &Device) -> Vec<String> {
    pair_then(b, d, |_| {})
}

/// [`pair`], with `accepted` run on the badge between the owner's press and the second tap.
fn pair_then(
    b: &mut Badge<FakeBoard>,
    d: &Device,
    accepted: impl FnOnce(&mut Badge<FakeBoard>),
) -> Vec<String> {
    b.pair_open().unwrap();
    let preview = tap(b, d, |p| p.pair_card_start("Conner's phone".into())).unwrap();
    let inbox = b.pair_read_inbox().unwrap().unwrap();
    let words: Vec<String> = b.pair_words(&inbox).unwrap().iter().map(|w| w.to_string()).collect();
    assert_eq!(preview.words, words);
    assert_eq!(preview.words.len(), 6);
    // Before the press only the count is readable, and the badge gives no name.
    assert_eq!(preview.circles, "1 circle");
    assert_eq!(preview.badge_name, "your badge");
    // Before the owner presses both buttons the slot does not know this phone.
    let early = tap(b, d, |p| p.pair_card_finish()).err().unwrap();
    assert!(matches!(early, PhoneError::NotPaired), "{early:?}");
    b.pair_accept(inbox).unwrap();
    accepted(b);
    tap(b, d, |p| p.pair_card_finish()).unwrap()
}

fn manifest(b: &Badge<FakeBoard>) -> Manifest {
    SealedManifest::decode(&b.circle().unwrap().manifest).unwrap().verify().unwrap()
}

/// A member holding the root: the chipless card's circle, chatting through the relay.
struct Member {
    session: ChatSession,
    seat: u8,
    key: SeatKey,
    manifest: Manifest,
    after: Option<u64>,
}

impl Member {
    fn of(b: &Badge<FakeBoard>) -> Member {
        let c = b.circle().unwrap();
        Member {
            session: ChatSession::new(&c.root.unwrap(), c.circle_id, c.params.n, c.seat),
            seat: c.seat,
            key: SeatKey::from_seed(&c.seat_key).unwrap(),
            manifest: manifest(b),
            after: None,
        }
    }

    /// Read the relay to the end, follow verified bumps, and return the texts others sent.
    fn read(&mut self, relay: &Arc<Mutex<MemRelay>>) -> Vec<(u8, String)> {
        let id = self.manifest.circle_id;
        let entries = relay.lock().unwrap().poll(&id, self.after, 0).unwrap();
        let mut out = Vec::new();
        for (seq, wire) in entries {
            self.after = Some(seq);
            let Ok((h, pt)) = self.session.receive(&wire) else {
                continue;
            };
            match h.msg_type {
                MessageType::Bump => {
                    if let Ok((from, to, _)) = verify_bump(&pt, &id, &self.manifest) {
                        self.session.apply_bump(from, to).unwrap();
                    }
                }
                MessageType::Text if h.sender != self.seat => {
                    out.push((h.sender, String::from_utf8(bucket_unpad(&pt).unwrap().to_vec()).unwrap()));
                }
                _ => {}
            }
        }
        self.session.mark_synced();
        out
    }

    fn say(&mut self, relay: &Arc<Mutex<MemRelay>>, text: &str) {
        self.read(relay);
        let wire = self.session.send(&bucket_pad(text.as_bytes()).unwrap()).unwrap();
        relay.lock().unwrap().post(&self.manifest.circle_id, &wire).unwrap();
    }

    /// Move the circle to generation `to` with a bump signed by this seat.
    fn bump(&mut self, relay: &Arc<Mutex<MemRelay>>, to: [u8; 32]) -> [u8; 32] {
        self.read(relay);
        let from = self.session.generation_id();
        let payload =
            sign_bump(&self.key, self.seat, &self.manifest.circle_id, &from, &to).unwrap();
        let wire = self.session.send_bump(to, &payload).unwrap();
        relay.lock().unwrap().post(&self.manifest.circle_id, &wire).unwrap();
        to
    }
}

/// Sign a contact card with the badge's seat key in its chip and put it in the circle's public
/// object, as the badge does once it has signed one.
fn badge_signs_a_card(b: &mut Badge<FakeBoard>) {
    let c = b.circle().unwrap().clone();
    let card = cahoots_objects::Card {
        circle_id: c.circle_id,
        seat: c.seat,
        name: "Ada's badge".into(),
        handles: Vec::new(),
        endpoints: Vec::new(),
        log: 0,
        face: Some([0x5F; 32]),
        device: Some(cahoots_objects::card::DEVICE_BADGE),
    };
    let chip = b.board().owner_chip();
    let signed = card
        .sign(ChipSeat::new(chip, KeyId::new(KeyKind::Seat, 0)), &[c.seat])
        .unwrap();
    let mut o = badge_chip::read_public(chip, 0).unwrap().unwrap();
    o.card = Some(signed);
    badge_chip::write_public(chip, 0, &o).unwrap();
    chip.close_session().unwrap();
}

#[test]
fn a_phone_pairs_chats_by_tap_follows_a_bump_and_forgets_the_card() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let badge_seat = b1.circle().unwrap().seat;
    let mut d = device(&relay);

    // The badge has signed a card: it is in the public object the second tap reads.
    let ids = pair_then(&mut b1, &d, badge_signs_a_card);
    assert_eq!(ids.len(), 1);
    let id = ids[0].clone();
    assert_eq!(id, hex::encode(b1.circle().unwrap().circle_id));

    // One card circle: the badge's seat, no root, no pad.
    let circles = d.phone.circles();
    assert_eq!(circles.len(), 1);
    assert_eq!(circles[0].kind, "card");
    assert_eq!(circles[0].my_seat, badge_seat);
    assert!(!circles[0].has_root);
    assert!(!circles[0].needs_tap);
    assert_eq!(circles[0].name, "fakes");
    let dir = d.path.join("circles").join(&id);
    let record = CircleRecord::load(&dir).unwrap();
    assert!(record.root.is_empty());
    assert!(record.pair_keys.iter().all(|k| k.is_empty()));
    assert!(record.card.is_some());
    assert!(!dir.join("pad.bin").exists());
    // The slot key is in the keychain, under the badge.
    let label = record.card.as_ref().unwrap().key_label.clone();
    assert!(label.starts_with("cahoots:card:"));
    assert_eq!(d.signer.get_secret(label.clone()).unwrap().map(|k| k.len()), Some(16));

    // The chat: a message the member posted reads, and this phone's reaches the member.
    member.say(&relay, "hello from the other card");
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text, "hello from the other card");
    assert_eq!(got[0].sender, member.seat);
    d.phone.send_chat(id.clone(), "hello from the phone".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(badge_seat, "hello from the phone".to_string())]);

    // The badge's card shows among the members.
    let members = d.phone.circle_members(id.clone()).unwrap();
    let badge = members.iter().find(|m| m.seat == badge_seat).unwrap();
    assert!(badge.mine);
    assert!(badge.has_card);
    assert_eq!(badge.name, "Ada's badge");
    assert_eq!(badge.device, Some(cahoots_objects::card::DEVICE_BADGE));
    assert_eq!(badge.face_seed, vec![0x5F; 32]);

    // A bump by the member moves the circle on. The phone holds no root: it waits for a tap.
    member.bump(&relay, [0x77; 32]);
    member.say(&relay, "after the reset");
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, "bump");
    assert!(d.phone.circles()[0].needs_tap);
    let e = d.phone.send_chat(id.clone(), "too soon".into()).err().unwrap();
    assert_eq!(e.to_string(), "Tap your card to catch up.");
    // Without the card nothing changes.
    assert!(matches!(d.phone.tap_for_keys(id.clone()), Err(PhoneError::NoCard)));
    assert!(d.phone.circles()[0].needs_tap);
    // Nor across a restart: the chat still waits for the key, and still says so.
    d.restart();
    assert!(d.phone.circles()[0].needs_tap);
    assert!(d.phone.poll(id.clone()).unwrap().is_empty());
    let e = d.phone.send_chat(id.clone(), "too soon".into()).err().unwrap();
    assert_eq!(e.to_string(), "Tap your card to catch up.");
    // The first key left the record once the chat state held it.
    let held = CircleRecord::load(&dir).unwrap();
    assert!(held.card.as_ref().unwrap().generation_key.is_none());

    tap(&mut b1, &d, |p| p.tap_for_keys(id.clone())).unwrap();
    assert!(!d.phone.circles()[0].needs_tap);
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text, "after the reset");
    d.phone.send_chat(id.clone(), "caught up".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(badge_seat, "caught up".to_string())]);
    let history: Vec<String> = d.phone.history(id.clone()).into_iter().map(|m| m.text).collect();
    assert!(history.contains(&"hello from the other card".to_string()));
    assert!(history.contains(&"caught up".to_string()));

    // With nothing to fetch there is no tap: no card in the reader, and no error.
    d.phone.tap_for_keys(id.clone()).unwrap();

    // The badge unpairs the phone: the next tap says so, and the card circles can be forgotten.
    b1.unpair().unwrap();
    member.bump(&relay, [0x78; 32]);
    d.phone.poll(id.clone()).unwrap();
    assert!(d.phone.circles()[0].needs_tap);
    let e = tap(&mut b1, &d, |p| p.tap_for_keys(id.clone())).err().unwrap();
    assert!(matches!(e, PhoneError::NotPaired));
    assert_eq!(e.to_string(), "This card no longer knows this phone.");
    d.phone.forget_card().unwrap();
    assert!(d.phone.circles().is_empty());
    assert!(!dir.exists());
    // The slot key is gone too.
    assert_eq!(d.signer.get_secret(label).unwrap(), None);
}

#[test]
fn a_pending_bump_in_the_public_object_is_forwarded_and_a_root_holder_moves() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let g0 = member.session.generation_id();

    // A phone paired and unpaired without ever posting: the badge left a bump for a device.
    let first = device(&relay);
    pair(&mut b1, &first);
    b1.unpair().unwrap();
    let o = badge_chip::read_public(b1.board().owner_chip(), 0).unwrap().unwrap();
    b1.board().chip.as_mut().unwrap().close_session().unwrap();
    assert_eq!(o.pending_bumps.len(), 1);
    let to = o.generation;
    assert_ne!(to, g0);
    // The member is still in the first generation: nobody posted the bump.
    member.say(&relay, "still in the first generation");
    assert_eq!(member.session.generation_id(), g0);

    // A second phone pairs and posts it, sealed in the generation it moves from.
    let second = device(&relay);
    let ids = pair(&mut b1, &second);
    assert_eq!(ids.len(), 1);
    member.read(&relay);
    assert_eq!(member.session.generation_id(), to);
    assert!(!second.phone.circles()[0].needs_tap);
    let record = CircleRecord::load(&second.path.join("circles").join(&ids[0])).unwrap();
    assert!(record.card.as_ref().unwrap().forward.is_empty());

    // Both chat in the new generation.
    let got = second.phone.poll(ids[0].clone()).unwrap();
    assert!(got.iter().all(|m| m.text != "still in the first generation"));
    second.phone.send_chat(ids[0].clone(), "in the new generation".into()).unwrap();
    let seat = b1.circle().unwrap().seat;
    assert_eq!(member.read(&relay), vec![(seat, "in the new generation".to_string())]);
    member.say(&relay, "me too");
    let got = second.phone.poll(ids[0].clone()).unwrap();
    assert_eq!(got.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["me too"]);
}

#[test]
fn hello_words_from_a_tap_equal_the_other_members() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, mut b2) = kept_pair((true, true));
    let d = device(&relay);
    let ids = pair(&mut b1, &d);
    let id = ids[0].clone();
    let mine = b1.circle().unwrap().seat;
    let other = b2.circle().unwrap().seat;

    // The other card's words, from its own hello object for this badge's seat.
    let theirs_at = |b2: &mut Badge<FakeBoard>, speaker: u8, n: u32| {
        let mut data = cahoots_core::pairwise::WORDS_DOMAIN.to_vec();
        data.push(speaker);
        data.extend_from_slice(&n.to_le_bytes());
        data.push(1);
        let chip = b2.board().owner_chip();
        let mac = chip.hmac(badge_chip::hello_id(0, mine), &data).unwrap();
        chip.close_session().unwrap();
        let v = u32::from_le_bytes(mac[..4].try_into().unwrap());
        let w = cahoots_core::checkwords::WORDLIST;
        format!("{} {}", w[(v & 0x7ff) as usize], w[((v >> 11) & 0x7ff) as usize])
    };

    let h = tap(&mut b1, &d, |p| p.hello_words(id.clone(), other)).unwrap();
    assert_eq!(h.number, 0);
    assert_eq!(h.mine, theirs_at(&mut b2, mine, 0));
    assert_eq!(h.theirs, theirs_at(&mut b2, other, 0));
    assert_ne!(h.mine, h.theirs);

    let h = tap(&mut b1, &d, |p| p.next_hello(id.clone(), other)).unwrap();
    assert_eq!(h.number, 1);
    assert_eq!(h.mine, theirs_at(&mut b2, mine, 1));
    assert_eq!(h.theirs, theirs_at(&mut b2, other, 1));

    // Not with this seat itself, and nothing without the card.
    assert!(matches!(d.phone.hello_words(id.clone(), mine), Err(PhoneError::NotFound { .. })));
    assert!(matches!(d.phone.hello_words(id.clone(), other), Err(PhoneError::NoCard)));
    // A failed tap does not move the number.
    assert!(d.phone.next_hello(id.clone(), other).is_err());
    let h = tap(&mut b1, &d, |p| p.hello_words(id.clone(), other)).unwrap();
    assert_eq!(h.number, 1);
}

#[test]
fn a_tap_without_an_open_pairing_says_so_and_keeps_nothing() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, _b2) = kept_pair((true, false));
    let d = device(&relay);
    let e = tap(&mut b1, &d, |p| p.pair_card_start("Conner's phone".into())).err().unwrap();
    assert!(e.to_string().contains("pair a phone"), "{e}");
    let e = tap(&mut b1, &d, |p| p.pair_card_finish()).err().unwrap();
    assert!(matches!(e, PhoneError::NotFound { .. }));
    assert!(d.phone.circles().is_empty());
    // A name the badge cannot hold.
    let e = d.phone.pair_card_start("x".repeat(65)).err().unwrap();
    assert_eq!(e.to_string(), "That name is too long for the badge. Try a shorter one.");
}

#[test]
fn an_entry_in_a_generation_the_card_circle_dropped_is_passed_over() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let seat = b1.circle().unwrap().seat;

    // A device of the member's seat that never moved from the first generation.
    let c = b2.circle().unwrap();
    let mut stale = ChatSession::new(&c.root.unwrap(), c.circle_id, c.params.n, c.seat);
    stale.mark_synced();

    // Two bumps, each caught up with a tap: the phone drops the first generation.
    for to in [[0x81; 32], [0x82; 32]] {
        member.bump(&relay, to);
        d.phone.poll(id.clone()).unwrap();
        assert!(d.phone.circles()[0].needs_tap);
        tap(&mut b1, &d, |p| p.tap_for_keys(id.clone())).unwrap();
        d.phone.poll(id.clone()).unwrap();
        assert!(!d.phone.circles()[0].needs_tap);
    }

    // A late entry in the first generation does not stop the poll, and no tap is asked for.
    let late = stale.send(&bucket_pad(b"late").unwrap()).unwrap();
    relay.lock().unwrap().post(&c.circle_id, &late).unwrap();
    member.say(&relay, "after the late one");
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["after the late one"]);
    assert!(!d.phone.circles()[0].needs_tap);
    d.phone.send_chat(id.clone(), "still chatting".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(seat, "still chatting".to_string())]);
}

#[test]
fn a_relay_out_of_reach_at_the_second_tap_leaves_the_bumps_for_the_next_poll() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let first = device(&relay);
    pair(&mut b1, &first);
    b1.unpair().unwrap();
    let o = badge_chip::read_public(b1.board().owner_chip(), 0).unwrap().unwrap();
    b1.board().chip.as_mut().unwrap().close_session().unwrap();

    // The second phone pairs while the relay is out of reach: the circle is kept, and the
    // bump waits.
    let second = device(&relay);
    second.http.1.store(true, Ordering::SeqCst);
    let id = pair(&mut b1, &second).remove(0);
    let dir = second.path.join("circles").join(&id);
    let record = CircleRecord::load(&dir).unwrap();
    assert_eq!(record.card.as_ref().unwrap().forward.len(), 1);
    assert!(relay.lock().unwrap().poll(&o.circle_id, None, 0).unwrap().is_empty());

    // Back online, the next poll posts it, with the key the second tap fetched ahead.
    second.http.1.store(false, Ordering::SeqCst);
    second.phone.poll(id.clone()).unwrap();
    let record = CircleRecord::load(&dir).unwrap();
    assert!(record.card.as_ref().unwrap().forward.is_empty());
    assert!(!second.phone.circles()[0].needs_tap);
    member.read(&relay);
    assert_eq!(member.session.generation_id(), o.generation);
    second.phone.send_chat(id.clone(), "after the outage".into()).unwrap();
    let seat = b1.circle().unwrap().seat;
    assert_eq!(member.read(&relay), vec![(seat, "after the outage".to_string())]);
}
