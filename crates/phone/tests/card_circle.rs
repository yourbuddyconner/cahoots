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
use cahoots_phone::{Http, Phone, PhoneError, Signer, TapReport, WordsLeft};
use cahoots_relay::{MemRelay, Relay};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const RELAY: &str = "https://relay.test";

/// A reader whose card is whatever chip a test puts in it, or none, and how many commands
/// reach it before it moves away (`None`: it stays).
#[derive(Clone, Default)]
struct FakeNfc(Arc<Mutex<Option<SoftChip>>>, Arc<Mutex<Option<usize>>>);

impl FakeNfc {
    /// The card moves away at the `n`th command from now, counting from 1: that command and
    /// every one after it fail, until [`FakeNfc::keep_card`].
    fn lose_card_at(&self, n: usize) {
        *self.1.lock().unwrap() = Some(n - 1);
    }

    /// The card stays in the reader.
    fn keep_card(&self) {
        *self.1.lock().unwrap() = None;
    }
}

impl Nfc for FakeNfc {
    fn begin(&self, _prompt: String) -> Result<(), PhoneError> {
        match *self.0.lock().unwrap() {
            Some(_) => Ok(()),
            None => Err(PhoneError::NoCard),
        }
    }
    fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        match self.1.lock().unwrap().as_mut() {
            Some(0) => return Err(PhoneError::Nfc { message: "The card moved away.".into() }),
            Some(n) => *n -= 1,
            None => {}
        }
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

/// The seeds of the two cards [`kept_pair`] makes.
const SEEDS: (u64, u64) = (0x9E37_79B9_7F4A_7C15, 0xD1B5_4A32_D192_ED03);

/// Card 1 at the bottom, card 2 on top. `chips` says which cards have a chip.
fn pile_of_two(chips: (bool, bool)) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let mut b1 = FakeBoard::seeded(SEEDS.0);
    let mut b2 = FakeBoard::seeded(SEEDS.1);
    if chips.0 {
        b1 = b1.with_soft_chip([1; 32]);
    }
    if chips.1 {
        b2 = b2.with_soft_chip([2; 32]);
    }
    pile(b1, b2)
}

/// `b1` at the bottom of a pile, `b2` on top.
fn pile(mut b1: FakeBoard, mut b2: FakeBoard) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (lower, upper) = cable();
    b1.links = Some((FakeLink::open(), lower));
    b2.links = Some((upper, FakeLink::open()));
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
    keep(pile_of_two(chips))
}

/// A badge that is not [`kept_pair`]'s, with its circle kept beside a chipless card. Every
/// test chip is set up with the same badge nonce, and the nonce is how a phone tells badges
/// apart, so this one's chip is set up with another.
fn kept_pair_of_another_badge() -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let mut b1 = FakeBoard::seeded(0x0123_4567_89AB_CDEF);
    let mut chip = SoftChip::with_seed([3; 32]);
    let record = badge_chip::first_boot(&mut chip, &cahoots_chip::TEST_OWNER_SECRET, [0x6B; 16]).unwrap();
    chip.close_session().unwrap();
    b1.chip = Some(chip);
    b1.chip_record = Some(record.to_stored());
    keep(pile(b1, FakeBoard::seeded(0x0F1E_2D3C_4B5A_6978)))
}

/// Run the pile `(b1, b2)` to the keep and keep the circle on both cards.
fn keep((mut b1, mut b2): (Badge<FakeBoard>, Badge<FakeBoard>)) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
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
    pair_with(b, d, "1 circle", accepted).unwrap()
}

/// The whole pairing with a badge that holds `circles` ("1 circle", "2 circles"), with
/// `accepted` run on the badge between the owner's press and the second tap. Returns what
/// the second tap returns.
fn pair_with(
    b: &mut Badge<FakeBoard>,
    d: &Device,
    circles: &str,
    accepted: impl FnOnce(&mut Badge<FakeBoard>),
) -> Result<Vec<String>, PhoneError> {
    b.pair_open().unwrap();
    let preview = tap(b, d, |p| p.pair_card_start("Conner's phone".into())).unwrap();
    let inbox = b.pair_read_inbox().unwrap().unwrap();
    let words: Vec<String> = b.pair_words(&inbox).unwrap().iter().map(|w| w.to_string()).collect();
    assert_eq!(preview.words, words);
    assert_eq!(preview.words.len(), 6);
    // Before the press only the count is readable, and the badge gives no name.
    assert_eq!(preview.circles, circles);
    assert_eq!(preview.badge_name, "your badge");
    // Before the owner presses both buttons the slot does not know this phone.
    let early = tap(b, d, |p| p.pair_card_finish()).err().unwrap();
    assert!(matches!(early, PhoneError::NotPaired), "{early:?}");
    b.pair_accept(inbox).unwrap();
    accepted(b);
    tap(b, d, |p| p.pair_card_finish())
}

fn press(b: &mut Badge<FakeBoard>, gesture: Press) {
    b.board().presses.push_back(gesture);
    b.idle_step();
}

/// Right after a keep: the note names the circle `name`, and five seconds later home comes.
fn home_after_keep(b: &mut Badge<FakeBoard>, name: &str) {
    assert!(b.board().shows(&format!("saved {name}")), "{:?}", b.board().last());
    b.board().uptime += 5;
    b.idle_step();
    assert!(b.board().shows("closed to taps"), "{:?}", b.board().last());
}

/// Run a ceremony between `bottom` (stepped by `bottom_step`) and a new chipless top card
/// seeded `seed`, and keep it on both. Returns the top card. As the badge's own tests do.
fn ceremony(
    bottom: &mut Badge<FakeBoard>,
    seed: u64,
    mut bottom_step: impl FnMut(&mut Badge<FakeBoard>) -> Option<Step>,
) -> Badge<FakeBoard> {
    let (lower, upper) = cable();
    bottom.board().links = Some((FakeLink::open(), lower));
    let mut board = FakeBoard::seeded(seed);
    board.links = Some((upper, FakeLink::open()));
    let mut top = Badge::new(board);
    let mut asked = false;
    let mut kept = (false, false);
    let mut prompt = (false, false);
    for _ in 0..20_000 {
        let s1 = bottom_step(bottom);
        let s2 = top.step();
        assert!(!matches!(s1, Some(Step::Failed(_))), "bottom: {s1:?}");
        assert!(!matches!(s2, Step::Failed(_)), "top: {s2:?}");
        if !asked && has_seat(bottom) && has_seat(&mut top) {
            asked = true;
            top.board().presses.push_back(Press::Both);
        }
        if bottom.board().shows("keep this circle?") && !prompt.0 {
            prompt.0 = true;
            bottom.board().presses.push_back(Press::Both);
        }
        if s2 == Step::Prompt && !prompt.1 {
            prompt.1 = true;
            top.board().presses.push_back(Press::Both);
        }
        let saved = bottom.board().last().iter().any(|l| l.starts_with("saved "));
        kept.0 |= s1 == Some(Step::Kept) || (prompt.0 && saved);
        kept.1 |= s2 == Step::Kept;
        if kept.0 && kept.1 {
            return top;
        }
    }
    panic!("the ceremony did not finish: {:?}", bottom.board().last());
}

/// A badge with a chip that kept two circles, "first" and "second", and the two chipless top
/// cards it made them with, which hold their roots.
fn badge_with_two_circles() -> (Badge<FakeBoard>, Badge<FakeBoard>, Badge<FakeBoard>) {
    let mut board = FakeBoard::seeded(SEEDS.0).with_soft_chip([1; 32]);
    board.name = "first".into();
    let mut b = Badge::new(board);
    let t1 = ceremony(&mut b, SEEDS.1, |b| Some(b.step()));
    home_after_keep(&mut b, "first");
    // Home, the list, then "new circle".
    press(&mut b, Press::Both);
    press(&mut b, Press::Right);
    assert!(b.board().shows("> new circle"), "{:?}", b.board().last());
    b.board().name = "second".into();
    b.board().presses.push_back(Press::Both);
    let t2 = ceremony(&mut b, 0x2545_F491_4F6C_DD1D, |b| {
        b.idle_step();
        None
    });
    home_after_keep(&mut b, "second");
    assert_eq!(b.circles().len(), 2);
    (b, t1, t2)
}

/// Forget the circle `name` on the badge, from home: the list, the circle, "forget this
/// circle", its question, and both buttons.
fn forget_on_badge(b: &mut Badge<FakeBoard>, name: &str) {
    assert!(b.board().shows("closed to taps"), "{:?}", b.board().last());
    press(b, Press::Both);
    for _ in 0..4 {
        if b.board().shows(&format!("> {name}")) {
            break;
        }
        press(b, Press::Left);
    }
    assert!(b.board().shows(&format!("> {name}")), "{:?}", b.board().last());
    press(b, Press::Both);
    for _ in 0..8 {
        if b.board().shows("> forget this circle") {
            break;
        }
        press(b, Press::Left);
    }
    assert!(b.board().shows("> forget this circle"), "{:?}", b.board().last());
    press(b, Press::Both);
    press(b, Press::Both);
    assert!(b.board().shows("forget this circle?"), "{:?}", b.board().last());
    press(b, Press::Both);
    assert!(b.board().log.iter().any(|l| l.starts_with(&format!("forgotten: {name} "))));
}

/// The circle id of the one circle `b` holds, in hex.
fn id_of(b: &Badge<FakeBoard>) -> String {
    hex::encode(b.circle().unwrap().circle_id)
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
    assert_eq!(e.to_string(), "Tap the card.");
    // Without the card nothing changes.
    assert!(matches!(d.phone.tap_card(), Err(PhoneError::NoCard)));
    assert!(d.phone.circles()[0].needs_tap);
    // Nor across a restart: the chat still waits for the key, and still says so.
    d.restart();
    assert!(d.phone.circles()[0].needs_tap);
    assert!(d.phone.poll(id.clone()).unwrap().is_empty());
    let e = d.phone.send_chat(id.clone(), "too soon".into()).err().unwrap();
    assert_eq!(e.to_string(), "Tap the card.");
    // The first key left the record once the chat state held it.
    let held = CircleRecord::load(&dir).unwrap();
    assert!(held.card.as_ref().unwrap().generation_key.is_none());

    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 0, kept: 0 });
    assert!(!d.phone.circles()[0].needs_tap);
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text, "after the reset");
    d.phone.send_chat(id.clone(), "caught up".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(badge_seat, "caught up".to_string())]);
    let history: Vec<String> = d.phone.history(id.clone()).into_iter().map(|m| m.text).collect();
    assert!(history.contains(&"hello from the other card".to_string()));
    assert!(history.contains(&"caught up".to_string()));

    // With nothing waiting a tap fetches nothing, and says so.
    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 0, bumps: 0, words: 0, kept: 0 });

    // The badge unpairs the phone: the next tap says so, and the card circles can be forgotten.
    b1.unpair().unwrap();
    member.bump(&relay, [0x78; 32]);
    d.phone.poll(id.clone()).unwrap();
    assert!(d.phone.circles()[0].needs_tap);
    let e = tap(&mut b1, &d, |p| p.tap_card()).err().unwrap();
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

/// The words `speaker` says at hello `n` between card `b2` and the seat `badge_seat`, worked out
/// by `b2` from its own hello object for that seat.
fn other_card_words(b2: &mut Badge<FakeBoard>, badge_seat: u8, speaker: u8, n: u32) -> String {
    let mut data = cahoots_core::pairwise::WORDS_DOMAIN.to_vec();
    data.push(speaker);
    data.extend_from_slice(&n.to_le_bytes());
    data.push(1);
    let chip = b2.board().owner_chip();
    let mac = chip.hmac(badge_chip::hello_id(0, badge_seat), &data).unwrap();
    chip.close_session().unwrap();
    let v = u32::from_le_bytes(mac[..4].try_into().unwrap());
    let w = cahoots_core::checkwords::WORDLIST;
    format!("{} {}", w[(v & 0x7ff) as usize], w[((v >> 11) & 0x7ff) as usize])
}

#[test]
fn hello_words_from_the_pocket_equal_the_other_members() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, mut b2) = kept_pair((true, true));
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let mine = b1.circle().unwrap().seat;
    let other = b2.circle().unwrap().seat;

    // No card in the reader: the words come from what the pairing tap kept.
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!(h.number, 0);
    assert_eq!(h.mine, other_card_words(&mut b2, mine, mine, 0));
    assert_eq!(h.theirs, other_card_words(&mut b2, mine, other, 0));
    assert_ne!(h.mine, h.theirs);

    let h = d.phone.next_hello(id.clone(), other).unwrap();
    assert_eq!(h.number, 1);
    assert_eq!(h.mine, other_card_words(&mut b2, mine, mine, 1));
    assert_eq!(h.theirs, other_card_words(&mut b2, mine, other, 1));

    // Not with this seat itself.
    assert!(matches!(d.phone.hello_words(id.clone(), mine), Err(PhoneError::NotFound { .. })));
    assert!(matches!(d.phone.next_hello(id.clone(), mine), Err(PhoneError::NotFound { .. })));
}

#[test]
fn the_second_tap_fills_a_pocket_of_eight_hellos_per_seat_and_no_tap_follows() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, mut b2) = kept_pair((true, true));
    let mut d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let mine = b1.circle().unwrap().seat;
    let other = b2.circle().unwrap().seat;
    let dir = d.path.join("circles").join(&id);

    // The pairing tap kept eight pairs for the other seat, numbers 0 to 7.
    let record = CircleRecord::load(&dir).unwrap();
    let pocket = record.card.as_ref().unwrap().hello_pocket.clone().unwrap();
    assert_eq!(pocket.len(), 8);
    assert!(pocket.iter().all(|p| p.other == other && p.mine.len() == 32 && p.theirs.len() == 32));
    let mut numbers: Vec<u32> = pocket.iter().map(|p| p.number).collect();
    numbers.sort();
    assert_eq!(numbers, (0..8).collect::<Vec<u32>>());

    // The card is gone from the reader: a tap would fail, and none is needed.
    assert!(d.nfc.0.lock().unwrap().is_none());
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (0, Some(8)));
    assert_eq!(h.mine, other_card_words(&mut b2, mine, mine, 0));
    assert_eq!(h.theirs, other_card_words(&mut b2, mine, other, 0));
    for n in 1..8u32 {
        let h = d.phone.next_hello(id.clone(), other).unwrap();
        assert_eq!((h.number, h.card_left), (n, Some(8 - n)));
        assert_eq!(h.mine, other_card_words(&mut b2, mine, mine, n));
        assert_eq!(h.theirs, other_card_words(&mut b2, mine, other, n));
        assert_eq!(d.phone.hello_words(id.clone(), other).unwrap(), h);
        if n == 4 {
            // The pocket and the number are on disk.
            d.restart();
            assert_eq!(d.phone.hello_words(id.clone(), other).unwrap(), h);
        }
    }

    // The ninth asks for a tap, and the number stays at 7.
    let e = d.phone.next_hello(id.clone(), other).err().unwrap();
    assert!(matches!(e, PhoneError::NeedsTap), "{e:?}");
    assert_eq!(e.to_string(), "Tap the card first.");
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (7, Some(1)));
    assert_eq!(CircleRecord::load(&dir).unwrap().pair_counters[usize::from(other)], 7);
    d.restart();
    assert_eq!(d.phone.hello_words(id.clone(), other).unwrap().number, 7);
}

/// A circle the badge kept before it made hello objects has none: a placeholder, or nothing,
/// stands at its hello ids. The second tap keeps it with its key, marks that seat, and the
/// seat gets no words from the card: a hello with it says so plainly and asks for no tap, and
/// the Card screen does not count it as running low.
#[test]
fn a_circle_without_hello_objects_is_kept_with_the_seat_marked() {
    for placeholder in [true, false] {
        let relay = Arc::new(Mutex::new(MemRelay::default()));
        let (mut b1, b2) = kept_pair((true, true));
        let other = b2.circle().unwrap().seat;
        let slot = b1.circle().unwrap().chip_slot.unwrap();
        let hello = badge_chip::hello_id(slot, other);
        let chip = b1.board().owner_chip();
        chip.delete(hello).unwrap();
        if placeholder {
            badge_chip::make_placeholder_at(chip, hello).unwrap();
        }
        chip.close_session().unwrap();
        let d = device(&relay);
        let ids = pair(&mut b1, &d);
        assert_eq!(ids.len(), 1, "placeholder {placeholder}");
        let dir = d.path.join("circles").join(&ids[0]);
        let record = CircleRecord::load(&dir).unwrap();
        let card = record.card.as_ref().unwrap();
        assert!(card.generation_key.is_some());
        assert!(card.hello_pocket.as_ref().is_some_and(|p| p.is_empty()));
        assert_eq!(card.no_hello, Some(vec![other]));
        assert!(d.phone.card_status().unwrap().words_low.is_empty());
        for e in [
            d.phone.hello_words(ids[0].clone(), other).err().unwrap(),
            d.phone.next_hello(ids[0].clone(), other).err().unwrap(),
        ] {
            assert!(matches!(e, PhoneError::NotFound { .. }), "{e:?}");
            assert_eq!(e.to_string(), "This seat has no hello words from the card.");
        }
    }
}

/// A later tap that gets a marked seat's words takes the mark off, and the words come.
#[test]
fn a_tap_that_gets_a_marked_seats_words_takes_the_mark_off() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, true));
    let other = b2.circle().unwrap().seat;
    let mut d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let dir = d.path.join("circles").join(&id);

    // As if an earlier tap had found no hello object for the seat.
    let mut record = CircleRecord::load(&dir).unwrap();
    let card = record.card.as_mut().unwrap();
    card.no_hello = Some(vec![other]);
    card.hello_pocket = Some(Vec::new());
    record.save(&dir).unwrap();
    d.restart();
    assert!(matches!(d.phone.hello_words(id.clone(), other), Err(PhoneError::NotFound { .. })));

    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report.words, 8);
    let record = CircleRecord::load(&dir).unwrap();
    assert_eq!(record.card.as_ref().unwrap().no_hello, Some(Vec::new()));
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (0, Some(8)));
}

#[test]
fn a_reset_does_not_empty_the_pocket() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let other = member.seat;
    let before = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((before.number, before.card_left), (0, Some(8)));

    // A root holder resets the chat: the card circle waits for the next generation's key.
    member.bump(&relay, [0x77; 32]);
    d.phone.poll(id.clone()).unwrap();
    assert!(d.phone.circles()[0].needs_tap);

    // Hello words do not hang on the chat's keys: they answer without a tap.
    assert!(d.nfc.0.lock().unwrap().is_none());
    assert_eq!(d.phone.hello_words(id.clone(), other).unwrap(), before);
    let h = d.phone.next_hello(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (1, Some(7)));
    assert_ne!(h.mine, before.mine);
}

#[test]
fn pairing_again_keeps_the_hello_numbers_and_fills_from_there() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let other = b2.circle().unwrap().seat;
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    for _ in 0..3 {
        d.phone.next_hello(id.clone(), other).unwrap();
    }
    let at_3 = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((at_3.number, at_3.card_left), (3, Some(5)));

    // The badge unpairs the phone and pairs it again: the circle it holds keeps its numbers,
    // and the tap fills eight from there.
    b1.unpair().unwrap();
    assert_eq!(pair(&mut b1, &d), vec![id.clone()]);
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (3, Some(8)));
    assert_eq!((h.mine, h.theirs), (at_3.mine, at_3.theirs));
    let record = CircleRecord::load(&d.path.join("circles").join(&id)).unwrap();
    let mut numbers: Vec<u32> =
        record.card.as_ref().unwrap().hello_pocket.as_ref().unwrap().iter().map(|p| p.number).collect();
    numbers.sort();
    assert_eq!(numbers, (3..11).collect::<Vec<u32>>());
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
        tap(&mut b1, &d, |p| p.tap_card()).unwrap();
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

/// The pocket on disk, as (other seat, number, mine, theirs), sorted.
fn pocket_on_disk(dir: &std::path::Path) -> Vec<(u8, u32, Vec<u8>, Vec<u8>)> {
    let record = CircleRecord::load(dir).unwrap();
    let mut out: Vec<_> = record
        .card
        .as_ref()
        .unwrap()
        .hello_pocket
        .iter()
        .flatten()
        .map(|p| (p.other, p.number, p.mine.to_vec(), p.theirs.to_vec()))
        .collect();
    out.sort();
    out
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn one_tap_fetches_the_waiting_key_posts_the_bumps_and_tops_up_the_pocket() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let mut d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let mine = b1.circle().unwrap().seat;
    let other = member.seat;

    // Nothing waits after the pairing tap.
    let s = d.phone.card_status().unwrap();
    assert!(s.paired);
    assert_eq!(s.badge, "your badge");
    assert_eq!(s.circles, 1);
    assert_eq!((s.keys_wanted, s.bumps_to_post), (0, 0));
    assert!(s.words_low.is_empty(), "{:?}", s.words_low);
    assert_eq!(s.last_tap, None);

    // Five hellos with the other seat leave three in the pocket.
    for _ in 0..5 {
        d.phone.next_hello(id.clone(), other).unwrap();
    }
    assert_eq!(d.phone.hello_words(id.clone(), other).unwrap().card_left, Some(3));

    // A root holder resets the chat: the card circle waits for the next generation's key.
    member.bump(&relay, [0x77; 32]);
    member.say(&relay, "after the reset");
    d.phone.poll(id.clone()).unwrap();
    assert!(d.phone.circles()[0].needs_tap);

    // The status says what a tap would do.
    let s = d.phone.card_status().unwrap();
    assert!(s.paired);
    assert_eq!(s.badge, "your badge");
    assert_eq!(s.circles, 1);
    assert_eq!(s.keys_wanted, 1);
    assert_eq!(s.bumps_to_post, 0);
    assert_eq!(s.words_low, vec![WordsLeft { circle: id.clone(), seat: other, left: 3 }]);
    assert_eq!(s.last_tap, None);

    // One tap: the key, and five more pairs, numbers 8 to 12.
    let started = now_secs();
    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 5, kept: 0 });
    assert!(!d.phone.circles()[0].needs_tap);
    let s = d.phone.card_status().unwrap();
    assert_eq!(s.keys_wanted, 0);
    assert!(s.words_low.is_empty(), "{:?}", s.words_low);
    let last = s.last_tap.expect("a last tap");
    assert!(last >= started && last <= now_secs(), "{last}");

    // The pocket holds numbers 5 to 12, and the words are the badge's own for them.
    let dir = d.path.join("circles").join(&id);
    let numbers: Vec<u32> = pocket_on_disk(&dir).iter().map(|p| p.1).collect();
    assert_eq!(numbers, (5..13).collect::<Vec<u32>>());
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (5, Some(8)));
    assert!(d.nfc.0.lock().unwrap().is_none());
    for n in 6..13u32 {
        let h = d.phone.next_hello(id.clone(), other).unwrap();
        assert_eq!((h.number, h.card_left), (n, Some(13 - n)));
        // The badge's hello object for the other seat, asked by the badge itself.
        assert_eq!(h.mine, other_card_words(&mut b1, other, mine, n));
        assert_eq!(h.theirs, other_card_words(&mut b1, other, other, n));
    }

    // The chat goes on in the new generation.
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["after the reset"]);
    d.phone.send_chat(id.clone(), "caught up".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(mine, "caught up".to_string())]);

    // The last tap is on disk.
    d.restart();
    assert_eq!(d.phone.card_status().unwrap().last_tap, Some(last));
}

#[test]
fn a_tap_that_loses_the_card_keeps_nothing_half_done() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let mut d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let other = member.seat;
    let dir = d.path.join("circles").join(&id);
    for _ in 0..5 {
        d.phone.next_hello(id.clone(), other).unwrap();
    }
    member.bump(&relay, [0x77; 32]);
    d.phone.poll(id.clone()).unwrap();
    assert!(d.phone.circles()[0].needs_tap);

    let pocket = pocket_on_disk(&dir);
    let words = d.phone.hello_words(id.clone(), other).unwrap();
    let status = d.phone.card_status().unwrap();
    let kept_nothing = |d: &Device| {
        assert!(d.phone.circles()[0].needs_tap);
        assert_eq!(pocket_on_disk(&dir), pocket);
        assert_eq!(d.phone.hello_words(id.clone(), other).unwrap(), words);
        assert_eq!(d.phone.card_status().unwrap(), status);
        let e = d.phone.send_chat(id.clone(), "too soon".into()).err().unwrap();
        assert!(matches!(e, PhoneError::Chat { .. }), "{e:?}");
    };

    // The card moves away at the third command.
    d.nfc.lose_card_at(3);
    let e = tap(&mut b1, &d, |p| p.tap_card()).err().unwrap();
    assert_eq!(e.to_string(), "The card moved away.");
    kept_nothing(&d);

    // And at every later command, up to the one that closes the session. Once, in the middle
    // of the session, the app is closed and opened again.
    let mut at = 4;
    let report = loop {
        d.nfc.lose_card_at(at);
        match tap(&mut b1, &d, |p| p.tap_card()) {
            Ok(r) => break r,
            Err(e) => {
                assert_eq!(e.to_string(), "The card moved away.", "at command {at}");
                kept_nothing(&d);
                if at == 15 {
                    d.restart();
                    kept_nothing(&d);
                }
            }
        }
        at += 1;
        assert!(at < 100, "the tap never got through");
    };
    d.nfc.keep_card();
    // The reads, the key, ten hello MACs and the close: the sweep went past the restart.
    assert!(at > 15, "{at}");
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 5, kept: 0 });
    assert!(!d.phone.circles()[0].needs_tap);
    assert_eq!(d.phone.hello_words(id.clone(), other).unwrap().card_left, Some(8));
}

#[test]
fn a_card_circle_from_before_the_pocket_fills_at_the_next_tap() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let other = b2.circle().unwrap().seat;
    let mut d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);

    // The record as a build from before the pocket wrote it.
    let dir = d.path.join("circles").join(&id);
    let mut record = CircleRecord::load(&dir).unwrap();
    record.card.as_mut().unwrap().hello_pocket = None;
    record.save(&dir).unwrap();
    d.restart();
    assert!(matches!(d.phone.hello_words(id.clone(), other), Err(PhoneError::NeedsTap)));
    let s = d.phone.card_status().unwrap();
    assert_eq!(s.keys_wanted, 0);
    assert_eq!(s.words_low, vec![WordsLeft { circle: id.clone(), seat: other, left: 0 }]);

    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 0, bumps: 0, words: 8, kept: 0 });
    let h = d.phone.hello_words(id.clone(), other).unwrap();
    assert_eq!((h.number, h.card_left), (0, Some(8)));
    assert!(d.phone.card_status().unwrap().words_low.is_empty());
    d.restart();
    assert_eq!(d.phone.hello_words(id.clone(), other).unwrap(), h);
}

#[test]
fn face_id_comes_before_the_reader_opens() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    member.bump(&relay, [0x77; 32]);
    d.phone.poll(id.clone()).unwrap();

    // Face ID is refused. The card would move away at its first command, so a reader opened
    // before the key was read would say that instead.
    d.signer.set_locked(true);
    d.nfc.lose_card_at(1);
    let e = tap(&mut b1, &d, |p| p.tap_card()).err().unwrap();
    assert!(matches!(e, PhoneError::Locked), "{e:?}");
    assert!(d.phone.circles()[0].needs_tap);
    assert_eq!(d.phone.card_status().unwrap().last_tap, None);

    d.signer.set_locked(false);
    d.nfc.keep_card();
    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 0, kept: 0 });
    assert!(!d.phone.circles()[0].needs_tap);
}

#[test]
fn one_tap_serves_every_card_circle() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b, t1, t2) = badge_with_two_circles();
    let (mut m1, mut m2) = (Member::of(&t1), Member::of(&t2));
    let d = device(&relay);
    let mut ids = pair_with(&mut b, &d, "2 circles", |_| {}).unwrap();
    ids.sort();
    let mut want = vec![id_of(&t1), id_of(&t2)];
    want.sort();
    assert_eq!(ids, want);
    let (first, second) = (id_of(&t1), id_of(&t2));

    // Both circles reset, and five hellos are used in the first.
    m1.bump(&relay, [0x71; 32]);
    m2.bump(&relay, [0x72; 32]);
    m1.say(&relay, "first after the reset");
    m2.say(&relay, "second after the reset");
    for id in [&first, &second] {
        d.phone.poll(id.clone()).unwrap();
    }
    assert!(d.phone.circles().iter().all(|c| c.needs_tap));
    for _ in 0..5 {
        d.phone.next_hello(first.clone(), m1.seat).unwrap();
    }
    let s = d.phone.card_status().unwrap();
    assert_eq!((s.circles, s.keys_wanted, s.bumps_to_post), (2, 2, 0));
    assert_eq!(s.words_low, vec![WordsLeft { circle: first.clone(), seat: m1.seat, left: 3 }]);

    // One tap: both keys, and the first circle's pocket topped up.
    let report = tap(&mut b, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 2, bumps: 0, words: 5, kept: 0 });
    assert!(d.phone.circles().iter().all(|c| !c.needs_tap));
    let s = d.phone.card_status().unwrap();
    assert_eq!((s.circles, s.keys_wanted), (2, 0));
    assert!(s.words_low.is_empty(), "{:?}", s.words_low);

    // Both chats go on.
    for (id, m, text) in [(&first, &mut m1, "first"), (&second, &mut m2, "second")] {
        let got = d.phone.poll(id.clone()).unwrap();
        let after = format!("{text} after the reset");
        assert_eq!(got.iter().map(|g| g.text.as_str()).collect::<Vec<_>>(), [after.as_str()]);
        d.phone.send_chat(id.clone(), format!("caught up in {text}")).unwrap();
        let mine = 1 - m.seat;
        assert_eq!(m.read(&relay), vec![(mine, format!("caught up in {text}"))]);
    }
}

#[test]
fn a_circle_the_badge_forgot_does_not_block_the_tap_for_the_other() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b, t1, t2) = badge_with_two_circles();
    let mut m2 = Member::of(&t2);
    let d = device(&relay);
    pair_with(&mut b, &d, "2 circles", |_| {}).unwrap();
    let (first, second) = (id_of(&t1), id_of(&t2));

    // The second circle resets, then the badge forgets the first.
    m2.bump(&relay, [0x72; 32]);
    d.phone.poll(second.clone()).unwrap();
    // The badge is turned off and on, and rests on home.
    b.boot();
    b.idle_step();
    forget_on_badge(&mut b, "first");
    assert_eq!(b.circles().len(), 1);

    // The phone still holds both. The tap passes over the one the badge forgot.
    assert_eq!(d.phone.card_status().unwrap().circles, 2);
    let report = tap(&mut b, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 0, kept: 0 });
    let needs = |id: &str| d.phone.circles().into_iter().find(|c| c.id == id).unwrap().needs_tap;
    assert!(!needs(&second));
    assert!(!needs(&first));
    m2.say(&relay, "after the reset");
    let got = d.phone.poll(second.clone()).unwrap();
    assert_eq!(got.iter().map(|g| g.text.as_str()).collect::<Vec<_>>(), ["after the reset"]);

    // A slot that holds another circle now is passed over the same way.
    m2.bump(&relay, [0x73; 32]);
    d.phone.poll(second.clone()).unwrap();
    assert!(needs(&second));
    let dir = d.path.join("circles").join(&second);
    let slot = CircleRecord::load(&dir).unwrap().card.as_ref().unwrap().circle_slot;
    let chip = b.board().owner_chip();
    let mut o = badge_chip::read_public(chip, slot).unwrap().unwrap();
    o.circle_id = [0xEE; 16];
    badge_chip::write_public(chip, slot, &o).unwrap();
    chip.close_session().unwrap();
    let report = tap(&mut b, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 0, bumps: 0, words: 0, kept: 0 });
    assert!(needs(&second));
}

#[test]
fn a_phone_keeps_the_circles_of_one_badge() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, _) = kept_pair((true, false));
    let (mut c1, _) = kept_pair_of_another_badge();
    let d = device(&relay);
    let ids = pair(&mut b1, &d);
    let label = |d: &Device, id: &str| {
        let r = CircleRecord::load(&d.path.join("circles").join(id)).unwrap();
        r.card.as_ref().unwrap().key_label.clone()
    };
    let first_label = label(&d, &ids[0]);

    // Another badge: the second tap refuses it, and keeps nothing of it.
    let e = pair_with(&mut c1, &d, "1 circle", |_| {}).err().unwrap();
    assert!(matches!(e, PhoneError::OtherBadge), "{e:?}");
    assert_eq!(e.to_string(), "Forget this card first.");
    assert_eq!(d.phone.circles().into_iter().map(|c| c.id).collect::<Vec<_>>(), ids);
    let nonce = badge_chip::read_public(c1.board().owner_chip(), 0).unwrap().unwrap().badge_nonce;
    c1.board().chip.as_mut().unwrap().close_session().unwrap();
    let other_label = format!("cahoots:card:{}", hex::encode(nonce));
    assert_ne!(other_label, first_label);
    assert_eq!(d.signer.get_secret(other_label.clone()).unwrap(), None);
    assert!(d.signer.get_secret(first_label).unwrap().is_some());
    // The first badge still taps.
    tap(&mut b1, &d, |p| p.tap_card()).unwrap();

    // The same badge again is no other badge.
    b1.unpair().unwrap();
    assert_eq!(pair(&mut b1, &d), ids);

    // Once the first is forgotten, the other pairs. It took this phone at the refused tap,
    // so its owner unpairs it there first.
    d.phone.forget_card().unwrap();
    assert!(c1.pair_open().is_err());
    c1.unpair().unwrap();
    let other = pair(&mut c1, &d);
    assert_eq!(other, vec![id_of(&c1)]);
    assert_eq!(label(&d, &other[0]), other_label);
}

#[test]
fn a_tap_posts_the_bumps_the_badge_left_and_counts_what_reached_the_relay() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    // A phone paired and unpaired without ever posting: the badge left a bump for a device.
    let first = device(&relay);
    pair(&mut b1, &first);
    b1.unpair().unwrap();
    let o = badge_chip::read_public(b1.board().owner_chip(), 0).unwrap().unwrap();
    b1.board().chip.as_mut().unwrap().close_session().unwrap();
    assert_eq!(o.pending_bumps.len(), 1);

    // A second phone pairs while the relay is out of reach, and the app is closed: the bump
    // waits, and so does the key it moves to.
    let mut second = device(&relay);
    second.http.1.store(true, Ordering::SeqCst);
    let id = pair(&mut b1, &second).remove(0);
    second.restart();
    let s = second.phone.card_status().unwrap();
    assert_eq!((s.keys_wanted, s.bumps_to_post), (1, 1));

    // A tap with the relay still out of reach fetches the key, and the bump still waits.
    let report = tap(&mut b1, &second, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 0, kept: 0 });
    assert_eq!(second.phone.card_status().unwrap().bumps_to_post, 1);
    assert!(relay.lock().unwrap().poll(&o.circle_id, None, 0).unwrap().is_empty());

    // Back online, the next tap posts it.
    second.http.1.store(false, Ordering::SeqCst);
    let report = tap(&mut b1, &second, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 1, words: 0, kept: 0 });
    let s = second.phone.card_status().unwrap();
    assert_eq!((s.keys_wanted, s.bumps_to_post), (0, 0));
    member.read(&relay);
    assert_eq!(member.session.generation_id(), o.generation);
    assert!(!second.phone.circles()[0].needs_tap);
    second.phone.send_chat(id.clone(), "after the bump".into()).unwrap();
    let seat = b1.circle().unwrap().seat;
    assert_eq!(member.read(&relay), vec![(seat, "after the bump".to_string())]);
}

#[test]
fn a_circle_the_badge_keeps_after_pairing_comes_to_the_phone_at_the_next_tap() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let mut board = FakeBoard::seeded(SEEDS.0).with_soft_chip([1; 32]);
    board.name = "first".into();
    let mut b = Badge::new(board);
    let t1 = ceremony(&mut b, SEEDS.1, |b| Some(b.step()));
    home_after_keep(&mut b, "first");
    let d = device(&relay);
    let ids = pair_with(&mut b, &d, "1 circle", |_| {}).unwrap();
    assert_eq!(ids, vec![id_of(&t1)]);

    // The badge keeps a second circle with the phone paired.
    press(&mut b, Press::Both);
    press(&mut b, Press::Right);
    assert!(b.board().shows("> new circle"), "{:?}", b.board().last());
    b.board().name = "second".into();
    b.board().presses.push_back(Press::Both);
    let t2 = ceremony(&mut b, 0x2545_F491_4F6C_DD1D, |b| {
        b.idle_step();
        None
    });
    home_after_keep(&mut b, "second");
    let second = id_of(&t2);
    assert_eq!(d.phone.circles().len(), 1);

    // The next tap brings it, with its chat key and a pocket of hello words.
    let mut m2 = Member::of(&t2);
    let report = tap(&mut b, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 1, bumps: 0, words: 8, kept: 1 });
    let held = d.phone.circles();
    assert_eq!(held.len(), 2);
    let c = held.iter().find(|c| c.id == second).expect("the new circle");
    assert_eq!(c.kind, "card");
    assert_eq!(c.name, "second");
    assert!(!c.needs_tap);
    let h = d.phone.hello_words(second.clone(), m2.seat).unwrap();
    assert_eq!((h.number, h.card_left), (0, Some(8)));
    assert_eq!(d.phone.card_status().unwrap().circles, 2);

    // Its chat reads and posts.
    m2.say(&relay, "hello from the second circle");
    let got = d.phone.poll(second.clone()).unwrap();
    let texts: Vec<&str> = got.iter().map(|g| g.text.as_str()).collect();
    assert_eq!(texts, ["hello from the second circle"]);
    d.phone.send_chat(second.clone(), "the phone is here".into()).unwrap();
    let mine = b.circles().iter().find(|c| hex::encode(c.circle_id) == second).unwrap().seat;
    assert_eq!(m2.read(&relay), vec![(mine, "the phone is here".to_string())]);

    // The next tap does not bring it again.
    let report = tap(&mut b, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 0, bumps: 0, words: 0, kept: 0 });
    assert_eq!(d.phone.circles().len(), 2);
}

#[test]
fn a_card_circle_removed_from_the_phone_comes_back_at_the_next_tap() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b, t1, _t2) = badge_with_two_circles();
    let d = device(&relay);
    pair_with(&mut b, &d, "2 circles", |_| {}).unwrap();
    let first = id_of(&t1);

    // The person removes one card circle in Settings. The other keeps the card's key here.
    d.phone.delete_circle(first.clone()).unwrap();
    assert_eq!(d.phone.circles().len(), 1);

    // The badge still vouches for it, so the next tap keeps it again.
    let report = tap(&mut b, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report.kept, 1);
    let ids: Vec<String> = d.phone.circles().into_iter().map(|c| c.id).collect();
    assert!(ids.contains(&first), "{ids:?}");
    assert_eq!(ids.len(), 2);
}

#[test]
fn pairing_after_the_badge_joined_its_oldest_bumps_needs_no_tap() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let mine = b1.circle().unwrap().seat;

    // Five phones pair and are unpaired in turn. Each pairing posts the bumps the unpairs
    // before it left, so the relay holds the whole chain, one bump per generation. At the
    // fifth unpair the badge is over its cap of four and joins its two oldest bumps into one,
    // which skips a generation the relay still names.
    for _ in 0..5 {
        let d = device(&relay);
        pair(&mut b1, &d);
        b1.unpair().unwrap();
    }
    let o = badge_chip::read_public(b1.board().owner_chip(), 0).unwrap().unwrap();
    b1.board().chip.as_mut().unwrap().close_session().unwrap();
    assert_eq!(o.pending_bumps.len(), 4);
    member.say(&relay, "before the sixth phone");

    // The sixth phone pairs. Its two taps bring every key its chat meets on the relay, the
    // skipped generation's among them, so it waits for nothing.
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    assert!(!d.phone.circles()[0].needs_tap, "the chat waits for a key right after pairing");
    assert_eq!(d.phone.card_status().unwrap().keys_wanted, 0);

    // It posted the last bump, and both chat in the badge's generation.
    member.read(&relay);
    assert_eq!(member.session.generation_id(), o.generation);
    d.phone.send_chat(id.clone(), "paired once".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(mine, "paired once".to_string())]);
    member.say(&relay, "welcome");
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(got.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["welcome"]);
}

#[test]
fn one_tap_brings_every_key_a_chain_of_resets_needs() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let (mut b1, b2) = kept_pair((true, false));
    let mut member = Member::of(&b2);
    let d = device(&relay);
    let id = pair(&mut b1, &d).remove(0);
    let mine = b1.circle().unwrap().seat;

    // Two resets by a root holder while the phone was away. The second is sealed in the
    // generation the first moves to, so the phone learns of it only once it holds that key.
    member.bump(&relay, [0x71; 32]);
    member.bump(&relay, [0x72; 32]);
    member.say(&relay, "after two resets");
    d.phone.poll(id.clone()).unwrap();
    assert!(d.phone.circles()[0].needs_tap);
    assert_eq!(d.phone.card_status().unwrap().keys_wanted, 1);

    // One tap brings both keys.
    let report = tap(&mut b1, &d, |p| p.tap_card()).unwrap();
    assert_eq!(report, TapReport { keys: 2, bumps: 0, words: 0, kept: 0 });
    assert!(!d.phone.circles()[0].needs_tap);
    assert_eq!(d.phone.card_status().unwrap().keys_wanted, 0);
    // The second reset's line and the text behind it come with the next poll.
    let got = d.phone.poll(id.clone()).unwrap();
    assert_eq!(
        got.iter().map(|m| (m.kind.as_str(), m.text.as_str())).collect::<Vec<_>>(),
        [("bump", "Keys were reset by seat 2."), ("text", "after two resets")]
    );
    d.phone.send_chat(id.clone(), "caught up".into()).unwrap();
    assert_eq!(member.read(&relay), vec![(mine, "caught up".to_string())]);
}
