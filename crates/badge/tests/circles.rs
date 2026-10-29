//! Many circles on one badge: a second ceremony makes a second record and a second chip slot,
//! the list of circles opens each one's menu, forgetting one keeps the others, Serve answers for
//! any circle held and no other, and nothing links two circles.

mod common;

use cahoots_badge::chip::ChipRecord;
use cahoots_badge::{Badge, Board, BoardError, Circle, Press, Step};
use cahoots_chip::{ChipOps, KeyId, KeyKind, SoftChip};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::{circle_id, CircleId};
use cahoots_core::frame::{self, Frame};
use cahoots_core::params::CircleParams;
use cahoots_core::root::chat_key;
use cahoots_custodian::wire::{
    ChipRequest, ChipResponse, ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE,
};
use cahoots_hal::Link;
use common::{cable, FakeBoard, FakeLink, FakePlace, FAKE_TIER};

fn id(kind: KeyKind, slot: u8) -> KeyId {
    KeyId::new(kind, slot)
}

fn has_seat(b: &mut Badge<FakeBoard>) -> bool {
    b.board().screens.iter().any(|l| {
        l.iter().any(|x| x == "stack the cards") && l.iter().any(|x| x.starts_with("seat "))
    })
}

fn press(badge: &mut Badge<FakeBoard>, gesture: Press) {
    badge.board().presses.push_back(gesture);
    badge.idle_step();
}

/// Card `b` at the bottom of a pile of two with a new top card seeded `seed`. Returns the top
/// card. `b` has the lower end of the cable as its up link the next time it asks for links.
fn pile_on(b: &mut Badge<FakeBoard>, seed: u64) -> Badge<FakeBoard> {
    let (lower, upper) = cable();
    b.board().links = Some((FakeLink::open(), lower));
    let mut top = FakeBoard::seeded(seed);
    top.links = Some((upper, FakeLink::open()));
    Badge::new(top)
}

/// Run a ceremony between `bottom` (stepped by `bottom_step`) and a new top card, and keep it
/// on both. Returns the top card.
fn ceremony(
    bottom: &mut Badge<FakeBoard>,
    seed: u64,
    mut bottom_step: impl FnMut(&mut Badge<FakeBoard>) -> Option<Step>,
) -> Badge<FakeBoard> {
    let mut top = pile_on(bottom, seed);
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
        let at_prompt = bottom.board().shows("keep this circle?");
        if at_prompt && !prompt.0 {
            prompt.0 = true;
            bottom.board().presses.push_back(Press::Both);
        }
        if s2 == Step::Prompt && !prompt.1 {
            prompt.1 = true;
            top.board().presses.push_back(Press::Both);
        }
        kept.0 |= s1 == Some(Step::Kept) || (prompt.0 && bottom.board().shows("saved. read the words aloud"));
        kept.1 |= s2 == Step::Kept;
        if kept.0 && kept.1 {
            return top;
        }
    }
    panic!("the ceremony did not finish: {:?}", bottom.board().last());
}

/// A badge (with a chip if `chip`) that kept two circles, the second started from its menu,
/// and the two top cards it made them with, which keep their roots in software.
fn two_circles(chip: bool) -> (Badge<FakeBoard>, Badge<FakeBoard>, Badge<FakeBoard>) {
    let mut board = FakeBoard::seeded(0x9E37_79B9_7F4A_7C15);
    board.name = "first".into();
    if chip {
        board.chip = Some(SoftChip::with_seed([1; 32]));
    }
    let mut b = Badge::new(board);
    let t1 = ceremony(&mut b, 0xD1B5_4A32_D192_ED03, |b| Some(b.step()));
    assert_eq!(b.circles().len(), 1);

    // The menu of the first circle; the Circles page; the list; "new circle".
    b.idle_step();
    assert!(b.board().shows("1/5"));
    for _ in 0..3 {
        press(&mut b, Press::Right);
    }
    assert!(b.board().shows("circles"));
    press(&mut b, Press::Both);
    assert!(b.board().shows("> first"), "{:?}", b.board().last());
    assert!(b.board().shows("  new circle"));
    press(&mut b, Press::Right);
    assert!(b.board().shows("> new circle"));
    b.board().name = "second".into();
    b.board().presses.push_back(Press::Both);
    let t2 = ceremony(&mut b, 0x2545_F491_4F6C_DD1D, |b| {
        b.idle_step();
        None
    });
    // The new circle's menu opens.
    b.idle_step();
    assert!(b.board().shows("second  tier 1"));
    (b, t1, t2)
}

/// The laptop end of `b`'s down link, with `b` on its Serve page and serving.
fn serve(b: &mut Badge<FakeBoard>) -> FakeLink {
    let (laptop, down) = cable();
    b.board().links = Some((down, FakeLink::open()));
    // Back to the list and into the first circle's menu, which takes the links.
    let mut n = 0;
    while !b.board().shows("4/5") {
        press(b, Press::Right);
        n += 1;
        assert!(n < 20, "{:?}", b.board().last());
    }
    press(b, Press::Both);
    let first = b.circles()[0].params.name.clone();
    let mut n = 0;
    while !b.board().shows(&format!("> {first}")) {
        press(b, Press::Right);
        n += 1;
        assert!(n < 20, "{:?}", b.board().last());
    }
    press(b, Press::Both);
    press(b, Press::Right);
    press(b, Press::Right);
    assert!(b.board().shows("serve a laptop"));
    press(b, Press::Both);
    assert!(b.board().shows("serving"));
    laptop
}

fn ask(b: &mut Badge<FakeBoard>, laptop: &mut FakeLink, seq: u32, req: &Request) -> Response {
    let payload = req.encode();
    laptop.send(&frame::encode(&Frame {
        kind: KIND_REQUEST,
        seq,
        payload: &payload,
    }));
    b.idle_step();
    let mut bytes = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = laptop.recv(&mut buf);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let (f, _) = frame::decode(&bytes).expect("an answer, not a prompt");
    assert_eq!((f.kind, f.seq), (KIND_RESPONSE, seq));
    Response::decode(f.payload).unwrap()
}

fn msg(label: &[u8], circle: &CircleId) -> Vec<u8> {
    let mut m = label.to_vec();
    m.extend_from_slice(circle);
    m
}

fn chat(circle: &CircleId) -> Vec<u8> {
    let mut m = msg(b"chat", circle);
    m.extend_from_slice(&0u64.to_be_bytes());
    m
}

#[test]
fn two_ceremonies_two_records() {
    let (mut b, t1, t2) = two_circles(true);
    let circles = b.circles().to_vec();
    assert_eq!(circles.len(), 2);
    assert_eq!(b.board().load_circles(), circles);
    let (a, c) = (&circles[0], &circles[1]);
    assert_eq!(a.params.name, "first");
    assert_eq!(c.params.name, "second");
    assert_eq!(a.circle_id, t1.circle().unwrap().circle_id);
    assert_eq!(c.circle_id, t2.circle().unwrap().circle_id);
    assert_ne!(a.circle_id, c.circle_id);
    assert_eq!((a.chip_slot, c.chip_slot), (Some(0), Some(1)));

    // Slots 0 and 1, each with a seat key, a root and a counter; different keys and roots.
    let chip = b.board().chip.as_mut().unwrap();
    assert_eq!(chip.ids().unwrap().len(), 6);
    let k0 = chip.public(id(KeyKind::Seat, 0)).unwrap();
    let k1 = chip.public(id(KeyKind::Seat, 1)).unwrap();
    assert_ne!(k0, k1);
    let r0 = chip.hmac(id(KeyKind::Root, 0), &chat(&a.circle_id)).unwrap();
    let r1 = chip.hmac(id(KeyKind::Root, 1), &chat(&c.circle_id)).unwrap();
    assert_ne!(r0, r1);
    // Each slot's root is its own circle's: the one its partner keeps in software.
    assert_eq!(r0, chat_key(&t1.circle().unwrap().root.unwrap(), &a.circle_id, 0));
    assert_eq!(r1, chat_key(&t2.circle().unwrap().root.unwrap(), &c.circle_id, 0));

    let record = b.board().chip_record_load().unwrap().unwrap();
    let slots: Vec<(u8, CircleId)> = record.slots.iter().map(|e| (e.slot, e.circle_id)).collect();
    assert_eq!(slots, vec![(0, a.circle_id), (1, c.circle_id)]);

    // Each circle has its own pad: the first place's and the second's.
    assert_eq!(b.board().pad.len(), 192);
    assert_eq!(b.board().more[0].pad.len(), 192);
    let board = b.board();
    assert_ne!(board.pad, board.more[0].pad);

    // Identify lists both, and says the badge has a chip.
    let mut laptop = serve(&mut b);
    match ask(&mut b, &mut laptop, 1, &Request::Identify) {
        Response::Identity { circles, chip, tier, .. } => {
            assert!(chip);
            assert_eq!(tier, FAKE_TIER);
            let ids: Vec<CircleId> = circles.iter().map(|i| i.circle_id).collect();
            assert_eq!(ids, vec![a.circle_id, c.circle_id]);
            assert_eq!(circles[1].name, "second");
        }
        other => panic!("not an identity: {other:?}"),
    }
    // Describe answers for each circle with its own key.
    for (circle, key) in [(a.circle_id, &k0), (c.circle_id, &k1)] {
        let r = ask(&mut b, &mut laptop, 2, &Request::Chip { circle, op: ChipRequest::Describe });
        match r {
            Response::Chip {
                result: ChipResponse::Description { public, in_chip, .. },
            } => {
                assert!(in_chip);
                assert_eq!(&public, key);
            }
            other => panic!("not a description: {other:?}"),
        }
    }
    // Remaining answers for each circle from its own pad.
    for circle in [a.circle_id, c.circle_id] {
        match ask(&mut b, &mut laptop, 3, &Request::Remaining { circle }) {
            Response::Remaining { bytes, .. } => assert_eq!(bytes, 96),
            other => panic!("{other:?}"),
        }
    }

    // A reboot lists both, and the list comes first.
    b.boot();
    assert_eq!(b.circles(), &circles[..]);
    assert_eq!(b.board().chip.as_mut().unwrap().ids().unwrap().len(), 6);
    b.idle_step();
    assert!(b.board().shows("> first"));
    assert!(b.board().shows("  second"));
}

#[test]
fn a_badge_without_a_chip_keeps_two_circles_in_software() {
    let (mut b, t1, t2) = two_circles(false);
    let circles = b.circles().to_vec();
    assert_eq!(circles.len(), 2);
    assert_eq!(circles[0].root, t1.circle().unwrap().root);
    assert_eq!(circles[1].root, t2.circle().unwrap().root);
    assert_ne!(circles[0].root, circles[1].root);
    assert_ne!(circles[0].seat_key, circles[1].seat_key);
    let mut laptop = serve(&mut b);
    match ask(&mut b, &mut laptop, 1, &Request::Identify) {
        Response::Identity { circles, chip, .. } => {
            assert!(!chip);
            assert_eq!(circles.len(), 2);
        }
        other => panic!("not an identity: {other:?}"),
    }
}

/// A board with two circles already stored: "fakes" in the first place, "other" in the second.
fn two_stored() -> FakeBoard {
    let (mut board, _, _) = FakeBoard::with_circle();
    let params = CircleParams::new(2, 128, "other", false).unwrap();
    let pad: Vec<u8> = (0..128u32).map(|i| (i * 13 + 1) as u8).collect();
    board.more[0] = FakePlace {
        circle: Some(Circle {
            params: params.clone(),
            seat: 0,
            circle_id: circle_id(&pad),
            words: cahoots_badge::word_indices(&check_words(&params, &pad)).unwrap(),
            seat_key: [8; 32],
            chip_slot: None,
            root: Some([4; 32]),
            tier: FAKE_TIER,
            manifest: Vec::new(),
        }),
        pad,
        records: vec![0xFF; 4096],
    };
    board
}

#[test]
fn chooser_navigation() {
    let mut b = Badge::new(two_stored());
    b.boot();
    // Two circles: no words at boot; the list is the first page.
    assert!(b.board().words().is_empty());
    b.idle_step();
    assert_eq!(
        b.board().last(),
        ["circles", "> fakes", "  other", "  new circle", "prev  A+B  open  next"]
    );
    press(&mut b, Press::Right);
    assert!(b.board().shows("> other"));
    press(&mut b, Press::Right);
    assert!(b.board().shows("> new circle"));
    press(&mut b, Press::Right);
    assert!(b.board().shows("> fakes"), "Right wraps to the first");
    press(&mut b, Press::Left);
    assert!(b.board().shows("> new circle"), "Left wraps to the last");
    press(&mut b, Press::Left);
    assert!(b.board().shows("> other"));
    // Both opens that circle's menu, on its words page.
    press(&mut b, Press::Both);
    assert!(b.board().shows("other  tier 1"));
    assert!(b.board().shows("1/5"));
    assert_eq!(b.circle().unwrap().params.name, "other");
    // Its Circles page goes back to the list, with it marked.
    press(&mut b, Press::Left);
    press(&mut b, Press::Left);
    assert!(b.board().shows("circles"));
    press(&mut b, Press::Both);
    assert!(b.board().shows("> other"));
    press(&mut b, Press::Left);
    press(&mut b, Press::Both);
    assert!(b.board().shows("fakes  tier 1"));
}

#[test]
fn one_circle_opens_its_menu_without_the_list() {
    let (board, _, _) = FakeBoard::with_circle();
    let mut b = Badge::new(board);
    b.boot();
    b.idle_step();
    assert!(b.board().shows("1/5"));
    assert!(!b.board().screens.iter().any(|l| l.iter().any(|x| x == "circles")));
}

#[test]
fn a_full_board_offers_no_new_circle() {
    let mut board = two_stored();
    board.more.truncate(1);
    let mut b = Badge::new(board);
    b.boot();
    b.idle_step();
    assert_eq!(b.board().last(), ["circles", "> fakes", "  other", "prev  A+B  open  next"]);
}

#[test]
fn left_or_right_in_the_pile_goes_back_to_the_list() {
    let mut b = Badge::new(two_stored());
    b.boot();
    b.idle_step();
    press(&mut b, Press::Left);
    assert!(b.board().shows("> new circle"));
    press(&mut b, Press::Both);
    b.idle_step();
    assert!(b.board().shows("stack the cards"));
    assert!(b.board().shows("back  A+B on the top card  start  back"));
    press(&mut b, Press::Right);
    assert!(b.board().shows("> fakes"), "{:?}", b.board().last());
    assert_eq!(b.circles().len(), 2);
}

#[test]
fn forget_one_keeps_the_other() {
    let (mut b, _t1, _t2) = two_circles(true);
    let kept = b.circles()[1].clone();
    let first_pad = b.board().pad.clone();
    let second_pad = b.board().more[0].pad.clone();
    // The second circle's menu is open: back to the list, open the first.
    press(&mut b, Press::Left);
    press(&mut b, Press::Left);
    press(&mut b, Press::Both);
    press(&mut b, Press::Left);
    press(&mut b, Press::Both);
    assert_eq!(b.circle().unwrap().params.name, "first");
    // Left from the words page is Forget. A single press on the question goes back.
    press(&mut b, Press::Left);
    assert!(b.board().shows("forget this circle"));
    press(&mut b, Press::Both);
    assert!(b.board().shows("forget this circle?"));
    assert!(b.board().shows("first"));
    assert!(b.board().shows("back  A+B  forget  back"));
    press(&mut b, Press::Right);
    assert!(b.board().shows("5/5"));
    assert_eq!(b.circles().len(), 2);
    press(&mut b, Press::Both);
    press(&mut b, Press::Both);
    assert!(b.board().log.iter().any(|l| l == "forgotten: first seat 0"));

    // The other circle stays: its record, its pad, its slot and its chip record entry.
    assert_eq!(b.circles(), std::slice::from_ref(&kept));
    assert_eq!(b.board().load_circles(), vec![kept.clone()]);
    assert!(b.board().pad.iter().all(|&x| x == 0), "the first pad is erased");
    assert_ne!(first_pad, b.board().pad);
    assert!(b.board().records.iter().all(|&x| x == 0xFF), "its ledger too");
    assert_eq!(b.board().more[0].pad, second_pad);
    let chip = b.board().chip.as_mut().unwrap();
    let mut ids = chip.ids().unwrap();
    ids.sort_by_key(|k| k.0);
    let mut want = vec![id(KeyKind::Seat, 1), id(KeyKind::Root, 1), id(KeyKind::Epoch, 1)];
    want.sort_by_key(|k| k.0);
    assert_eq!(ids, want);
    let record = b.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.slots.len(), 1);
    assert_eq!((record.slots[0].slot, record.slots[0].circle_id), (1, kept.circle_id));

    // One circle is left: its menu opens without the list, and a reboot keeps it.
    b.idle_step();
    assert!(b.board().shows("second  tier 1"));
    b.boot();
    assert_eq!(b.circles(), [kept]);
    assert_eq!(b.board().chip.as_mut().unwrap().ids().unwrap().len(), 3);
}

#[test]
fn a_held_at_boot_forgets_every_circle() {
    let (mut b, _t1, _t2) = two_circles(true);
    let board = b.board();
    board.a_down = true;
    board.release_at_ms = board.elapsed_ms + 3000;
    b.boot();
    assert!(b.circles().is_empty());
    assert!(b.circle().is_none());
    assert!(b.board().load_circles().is_empty());
    assert!(b.board().pad.iter().all(|&x| x == 0));
    assert!(b.board().more[0].pad.iter().all(|&x| x == 0));
    assert!(b.board().chip.as_mut().unwrap().ids().unwrap().is_empty());
    assert_eq!(b.board().chip_record_load(), Ok(Some(ChipRecord::default())));
    assert!(b.board().shows("circles forgotten"));
}

/// Every 32-byte value one circle is made of: each 32-byte window of its record that holds key
/// material (24 or more distinct bytes, as 32 random bytes have about 30; the record's framing,
/// the absent fields and the manifest's CBOR structure have far fewer, and are the same in
/// every record), its seat public key, and what its root gives: the chat key and the card seal
/// key.
fn values(b: &mut Badge<FakeBoard>, c: &Circle) -> Vec<Vec<u8>> {
    let record = c.encode_record().unwrap();
    let distinct = |w: &[u8]| {
        let mut seen = [false; 256];
        w.iter().for_each(|&x| seen[usize::from(x)] = true);
        seen.iter().filter(|&&s| s).count()
    };
    let mut out: Vec<Vec<u8>> = record
        .windows(32)
        .filter(|w| distinct(w) >= 24)
        .map(<[u8]>::to_vec)
        .collect();
    assert!(out.len() > 100, "the record holds keys and signatures");
    match (c.chip_slot, b.board().chip.as_mut()) {
        (Some(slot), Some(chip)) => {
            out.push(chip.public(id(KeyKind::Seat, slot)).unwrap()[1..].to_vec());
            out.push(chip.hmac(id(KeyKind::Root, slot), &chat(&c.circle_id)).unwrap().to_vec());
            out.push(chip.hmac(id(KeyKind::Root, slot), &msg(b"seal/card", &c.circle_id)).unwrap().to_vec());
        }
        _ => {
            let root = c.root.unwrap();
            out.push(chat_key(&root, &c.circle_id, 0).to_vec());
            out.push(c.seat_key.to_vec());
        }
    }
    out
}

#[test]
fn nothing_links_two_circles() {
    for chip in [true, false] {
        let (mut b, _t1, _t2) = two_circles(chip);
        let circles = b.circles().to_vec();
        let a = values(&mut b, &circles[0]);
        let c = values(&mut b, &circles[1]);
        let c_bytes: Vec<u8> = c.concat();
        let c_record = circles[1].encode_record().unwrap();
        for v in &a {
            assert!(
                !c.contains(v) && !c_bytes.windows(32).any(|w| w == &v[..]),
                "chip {chip}: a value of the first circle is in the second's: {v:02x?}"
            );
            assert!(!c_record.windows(32).any(|w| w == &v[..]));
        }
        // Nothing smaller links them either: circle ids, and chip slots.
        assert!(!c_record.windows(16).any(|w| w == circles[0].circle_id));
        if chip {
            assert_ne!(circles[0].chip_slot, circles[1].chip_slot);
        }
    }
}

#[test]
fn unknown_circle_chip_request() {
    let (mut b, _t1, _t2) = two_circles(true);
    let mut laptop = serve(&mut b);
    let stranger: CircleId = [9; 16];
    for op in [
        ChipRequest::Describe,
        ChipRequest::Attest { freshness: [1; 16] },
        ChipRequest::SshSign {
            data: vec![0; 40],
            host: None,
            content: None,
        },
    ] {
        let r = ask(&mut b, &mut laptop, 5, &Request::Chip { circle: stranger, op });
        assert_eq!(r, Response::Error { code: ErrorCode::UnknownCircle });
    }
    // A spend for a circle the badge does not hold is refused without asking.
    let r = ask(&mut b, &mut laptop, 6, &Request::Psk { circle: stranger, context: b"wg".to_vec() });
    assert_eq!(r, Response::Error { code: ErrorCode::UnknownCircle });
    assert!(b.board().shows("serving"));
}

/// A board with one record: only `load_circle`, `save_circle` and `forget`, through the
/// defaults that adapt it.
struct Single(FakeBoard);

impl Board for Single {
    type Entropy = <FakeBoard as Board>::Entropy;
    type Pad = <FakeBoard as Board>::Pad;
    type Records = <FakeBoard as Board>::Records;
    type Display = <FakeBoard as Board>::Display;
    type Link = <FakeBoard as Board>::Link;
    fn entropy(&mut self) -> &mut Self::Entropy {
        self.0.entropy()
    }
    fn display(&mut self) -> &mut Self::Display {
        self.0.display()
    }
    fn press(&mut self) -> Press {
        self.0.press()
    }
    fn a_down(&self) -> bool {
        self.0.a_down()
    }
    fn delay_ms(&mut self, ms: u32) {
        self.0.delay_ms(ms)
    }
    fn links(&mut self) -> (FakeLink, FakeLink) {
        self.0.links()
    }
    fn load_circle(&self) -> Option<Circle> {
        self.0.circle.clone()
    }
    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError> {
        self.0.circle = Some(c.clone());
        Ok(())
    }
    fn forget(&mut self, circle: &CircleId, pad_len: u32) -> Result<(), BoardError> {
        self.0.forget(circle, pad_len)?;
        self.0.circle = None;
        Ok(())
    }
    fn write_pad(&mut self, circle: &CircleId, pad: &[u8]) -> Result<Self::Pad, BoardError> {
        self.0.write_pad(circle, pad)
    }
    fn open_pad(&mut self, circle: &CircleId, len: u32) -> Result<Self::Pad, BoardError> {
        self.0.open_pad(circle, len)
    }
    fn records(&mut self, circle: &CircleId, fresh: bool) -> Result<Self::Records, BoardError> {
        self.0.records(circle, fresh)
    }
    fn proposal(&self) -> (String, u32) {
        self.0.proposal()
    }
    fn tier(&self) -> u8 {
        self.0.tier()
    }
    fn selftest(&mut self) -> Vec<String> {
        self.0.selftest()
    }
    fn log(&mut self, line: &str) {
        self.0.log(line)
    }
    fn note_screen(&mut self, lines: &[String], secret: bool) {
        self.0.note_screen(lines, secret)
    }
}

#[test]
fn a_board_with_one_record_holds_one_circle() {
    let (board, _, _) = FakeBoard::with_circle();
    let mut b = Badge::new(Single(board));
    b.boot();
    assert_eq!(b.circles().len(), 1);
    assert_eq!(b.board().max_circles(), 1);
    b.idle_step();
    // Circles page: the list has the one circle and no "new circle".
    for _ in 0..3 {
        b.board().0.presses.push_back(Press::Right);
        b.idle_step();
    }
    b.board().0.presses.push_back(Press::Both);
    b.idle_step();
    assert_eq!(b.board().0.last(), ["circles", "> fakes", "prev  A+B  open  next"]);
    // Forget it from its menu: the record goes through the adapter.
    for p in [Press::Both, Press::Left, Press::Both, Press::Both] {
        b.board().0.presses.push_back(p);
        b.idle_step();
    }
    assert!(b.circles().is_empty());
    assert_eq!(b.board().load_circles(), Vec::<Circle>::new());
    assert!(b.board().0.pad.iter().all(|&x| x == 0));
}

#[test]
fn a_failed_forget_never_leaves_a_record_over_a_blank_pad() {
    let mut board = two_stored();
    board.fail_ledger_erase = true;
    let other = board.more[0].circle.clone().unwrap();
    let gone = board.circle.clone().unwrap().circle_id;
    let mut b = Badge::new(board);
    b.boot();
    b.idle_step();
    // Open "fakes", then Left to Forget, Both to ask, Both to forget: the ledger erase fails.
    press(&mut b, Press::Both);
    press(&mut b, Press::Left);
    press(&mut b, Press::Both);
    press(&mut b, Press::Both);
    assert!(b.board().log.iter().any(|l| l.starts_with("forget: ")), "{:?}", b.board().log);
    // The record went first: the circle is not held, and only the other one is.
    assert_eq!(b.circles(), std::slice::from_ref(&other));
    assert_eq!(b.board().load_circles(), vec![other.clone()]);
    assert!(b.board().pad.iter().all(|&x| x == 0));

    // Serving: nothing is answered for the forgotten circle; the other still is.
    b.board().fail_ledger_erase = false;
    let (mut laptop, down) = cable();
    b.board().links = Some((down, FakeLink::open()));
    b.idle_step();
    assert!(b.board().shows("other  tier 1"));
    press(&mut b, Press::Right);
    press(&mut b, Press::Right);
    press(&mut b, Press::Both);
    assert!(b.board().shows("serving"));
    for req in [
        Request::Psk { circle: gone, context: b"wg".to_vec() },
        Request::Spend { circle: gone, len: 32 },
    ] {
        let r = ask(&mut b, &mut laptop, 7, &req);
        assert_eq!(r, Response::Error { code: ErrorCode::UnknownCircle });
    }
    match ask(&mut b, &mut laptop, 8, &Request::Remaining { circle: gone }) {
        Response::Remaining { bytes, .. } => assert_eq!(bytes, 0),
        other => panic!("{other:?}"),
    }
    match ask(&mut b, &mut laptop, 9, &Request::Identify) {
        Response::Identity { circles, .. } => {
            assert_eq!(circles.len(), 1);
            assert_eq!(circles[0].circle_id, other.circle_id);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_held_at_boot_forgets_every_record_even_when_an_erase_fails() {
    let mut board = two_stored();
    board.fail_ledger_erase = true;
    board.a_down = true;
    let mut b = Badge::new(board);
    b.boot();
    assert!(b.circles().is_empty());
    assert!(b.board().load_circles().is_empty());
    assert!(b.board().log.iter().any(|l| l == "wipe failed"));
}

#[test]
fn boot_deletes_a_stray_slot_and_the_next_circle_gets_a_clean_one() {
    let (mut b, _t1, _t2) = two_circles(true);
    let chip = b.board().chip.as_mut().unwrap();
    let stray = chip.generate(id(KeyKind::Seat, 2), cahoots_chip::Curve::P256).unwrap();
    chip.put_hmac_key(id(KeyKind::Root, 2), &[7; 32]).unwrap();
    assert_eq!(chip.ids().unwrap().len(), 8);
    b.boot();
    // Slot 2 is deleted and logged; slots 0 and 1 stay with their circles.
    let chip = b.board().chip.as_mut().unwrap();
    let mut slots: Vec<u8> = chip.ids().unwrap().iter().map(|k| k.0 as u8).collect();
    slots.sort();
    assert_eq!(slots, vec![0, 0, 0, 1, 1, 1]);
    let log = &b.board().log;
    assert_eq!(
        log.iter().filter(|l| l.contains("slot 2 is not in the chip record, deleted")).count(),
        2,
        "{log:?}"
    );
    assert_eq!(b.circles().len(), 2);

    // A third circle takes slot 2, made new: not the stray key, and no stray root.
    b.idle_step();
    press(&mut b, Press::Left);
    assert!(b.board().shows("> new circle"));
    b.board().name = "third".into();
    b.board().presses.push_back(Press::Both);
    let t3 = ceremony(&mut b, 0x5851_F42D_4C95_7F2D, |b| {
        b.idle_step();
        None
    });
    let third = b.circles().iter().find(|c| c.params.name == "third").unwrap().clone();
    assert_eq!(third.chip_slot, Some(2));
    let chip = b.board().chip.as_mut().unwrap();
    assert_ne!(chip.public(id(KeyKind::Seat, 2)).unwrap(), stray);
    let from_chip = chip.hmac(id(KeyKind::Root, 2), &chat(&third.circle_id)).unwrap();
    assert_eq!(from_chip, chat_key(&t3.circle().unwrap().root.unwrap(), &third.circle_id, 0));
    let record = b.board().chip_record_load().unwrap().unwrap();
    assert_eq!(record.slots.len(), 3);
}
