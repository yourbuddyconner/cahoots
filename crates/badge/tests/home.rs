//! The badge's home page: the circles it holds, whether a phone can tap now, and the left
//! button that opens a two-minute window for a tap. The menu is one list that ends with
//! "back".

mod common;

use cahoots_badge::{Badge, Press, Step};
use cahoots_core::frame::{self, Frame};
use cahoots_custodian::wire::{ChipRequest, Request, Response, KIND_REQUEST, KIND_RESPONSE};
use cahoots_hal::Link;
use cahoots_objects::card::Card;
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

/// Run a pile of two to the keep and keep the circle on both cards. Each card shows the keep's
/// note, "saved" and the circle's name, and five seconds later rests on home.
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
    for b in [&mut b1, &mut b2] {
        let saved = format!("saved {}", circle_name(b));
        assert!(b.board().shows(&saved), "{:?}", b.board().last());
        b.board().uptime += 5;
        step(b);
    }
    (b1, b2)
}

/// One gesture, then one idle step.
fn press(b: &mut Badge<FakeBoard>, p: Press) {
    b.board().presses.push_back(p);
    b.idle_step();
}

/// One idle step with no gesture.
fn step(b: &mut Badge<FakeBoard>) {
    b.idle_step();
}

/// The name of the badge's circle.
fn circle_name(b: &Badge<FakeBoard>) -> String {
    b.circle().unwrap().params.name.clone()
}

fn send(laptop: &mut FakeLink, seq: u32, req: &Request) {
    let payload = req.encode();
    laptop.send(&frame::encode(&Frame {
        kind: KIND_REQUEST,
        seq,
        payload: &payload,
    }));
}

/// Everything waiting on the laptop end, as `(seq, response)` frames.
fn responses(laptop: &mut FakeLink) -> Vec<(u32, Response)> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = laptop.recv(&mut buf);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let (f, used) = frame::decode(&bytes[at..]).expect("a whole frame");
        assert_eq!(f.kind, KIND_RESPONSE);
        out.push((f.seq, Response::decode(f.payload).expect("a response")));
        at += used;
    }
    out
}

/// Plug a laptop into the down connector of a badge resting on home, and return the
/// laptop's end. The badge takes the link when home is entered again: the list, then "back".
fn plug_laptop(b: &mut Badge<FakeBoard>) -> FakeLink {
    let (laptop, down) = cable();
    b.board().links = Some((down, FakeLink::open()));
    press(b, Press::Both);
    to_entry(b, "back");
    press(b, Press::Both);
    assert!(b.board().shows("closed to taps"), "{:?}", b.board().last());
    laptop
}

/// The laptop's first request asks about the laptop, and Both allows it until unplugged.
fn allow(b: &mut Badge<FakeBoard>, laptop: &mut FakeLink) {
    send(laptop, 900, &Request::Identify);
    step(b);
    assert!(b.board().shows("a laptop"), "{:?}", b.board().last());
    press(b, Press::Both);
    let got = responses(laptop);
    assert!(matches!(got.as_slice(), [(900, Response::Identity { .. })]), "{got:?}");
}

/// Press Right until `entry` is the marked entry of the list on the screen.
fn to_entry(b: &mut Badge<FakeBoard>, entry: &str) {
    let marked = format!("> {entry}");
    for _ in 0..16 {
        if b.board().shows(&marked) {
            return;
        }
        press(b, Press::Right);
    }
    panic!("no {entry} in {:?}", b.board().last());
}

#[test]
fn the_badge_rests_on_a_home_page_that_lists_its_circles() {
    let (mut b1, _b2) = kept_pair((true, false));
    let name = circle_name(&b1);
    // Boot ends on home, whatever the circle count.
    assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
    assert!(b1.board().shows(&name));
    // Closed, the left button's label says it opens the window for a tap.
    assert!(b1.board().shows("tap  A+B  menu  menu"), "{:?}", b1.board().last());
}

#[test]
fn the_left_button_opens_a_two_minute_tap_window_that_closes_by_itself() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Left);
    assert!(b1.board().tap_open);
    assert!(b1.board().shows("open for a tap, 2 min"), "{:?}", b1.board().last());
    // Open, the left button's label says it closes the window.
    assert!(b1.board().shows("close  A+B  menu  menu"), "{:?}", b1.board().last());
    b1.board().uptime += 61;
    step(&mut b1);
    assert!(b1.board().shows("open for a tap, 1 min"));
    b1.board().uptime += 60;
    step(&mut b1);
    assert!(!b1.board().tap_open);
    assert!(b1.board().shows("closed to taps"));
    assert!(b1.board().shows("tap  A+B  menu  menu"), "{:?}", b1.board().last());
    // A second press while open closes it at once.
    press(&mut b1, Press::Left);
    assert!(b1.board().tap_open);
    press(&mut b1, Press::Left);
    assert!(!b1.board().tap_open);
}

#[test]
fn a_conversation_of_the_badges_own_ends_the_window_and_home_says_so() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Left);
    // The board's chip_begin restores the power, as a laptop's request would.
    b1.board().tap_open = false;
    step(&mut b1);
    assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
}

#[test]
fn the_menu_is_one_list_that_ends_with_back_to_home() {
    let (mut b1, _b2) = kept_pair((true, false));
    let name = circle_name(&b1);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows(&format!("> {name}")), "{:?}", b1.board().last());
    assert!(b1.board().shows("  new circle"));
    assert!(b1.board().shows("  pair a phone"));
    assert!(b1.board().shows("  back"));
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("closed to taps"));
}

#[test]
fn home_is_drawn_again_only_when_what_it_says_changes() {
    let (mut b1, _b2) = kept_pair((true, false));
    let at_rest = b1.board().screens.len();
    // At rest, time passing draws nothing.
    for _ in 0..3 {
        b1.board().uptime += 30;
        step(&mut b1);
    }
    assert_eq!(b1.board().screens.len(), at_rest);
    // The window opens: one drawing.
    press(&mut b1, Press::Left);
    assert_eq!(b1.board().screens.len(), at_rest + 1);
    // Seventy seconds left: still two minutes, nothing drawn.
    for _ in 0..5 {
        b1.board().uptime += 10;
        step(&mut b1);
    }
    assert_eq!(b1.board().screens.len(), at_rest + 1);
    // One minute left: one drawing.
    b1.board().uptime += 10;
    step(&mut b1);
    assert_eq!(b1.board().screens.len(), at_rest + 2);
    assert!(b1.board().shows("open for a tap, 1 min"), "{:?}", b1.board().last());
    for _ in 0..5 {
        b1.board().uptime += 10;
        step(&mut b1);
    }
    assert_eq!(b1.board().screens.len(), at_rest + 2);
    // The window closes: one drawing, then nothing more.
    b1.board().uptime += 10;
    step(&mut b1);
    assert_eq!(b1.board().screens.len(), at_rest + 3);
    assert!(b1.board().shows("closed to taps"));
    for _ in 0..3 {
        b1.board().uptime += 60;
        step(&mut b1);
    }
    assert_eq!(b1.board().screens.len(), at_rest + 3);
}

#[test]
fn a_board_that_cannot_cut_the_chips_power_opens_no_window() {
    let (mut b1, _b2) = kept_pair((true, false));
    // A held chip keeps its power: the board's tap_begin does nothing.
    b1.board().chip_held = true;
    let at_rest = b1.board().screens.len();
    press(&mut b1, Press::Left);
    assert!(!b1.board().tap_open);
    assert_eq!(b1.board().screens.len(), at_rest);
    assert!(b1.board().shows("closed to taps"));
}

#[test]
fn a_signature_allowed_while_the_window_is_open_draws_home_once_and_closed() {
    let (mut b1, _b2) = kept_pair((true, false));
    let mut laptop = plug_laptop(&mut b1);
    allow(&mut b1, &mut laptop);
    let (circle, seat) = {
        let c = b1.circle().unwrap();
        (c.circle_id, c.seat)
    };
    press(&mut b1, Press::Left);
    assert!(b1.board().tap_open);
    // A card to sign: asking reads nothing from the chip, so the window is still open while
    // the question is on the screen.
    let card = Card {
        circle_id: circle,
        seat,
        name: "Ada Lovelace".into(),
        handles: vec![("email".into(), "ada@example.com".into())],
        endpoints: Vec::new(),
        log: 1,
        face: None,
        device: None,
    };
    let op = ChipRequest::Card {
        card: card.encode().unwrap(),
    };
    send(&mut laptop, 7, &Request::Chip { circle, op });
    step(&mut b1);
    assert!(b1.board().shows("sign your card"), "{:?}", b1.board().last());
    assert!(b1.board().tap_open);
    let before = b1.board().screens.len();
    // Both: the chip signs, which gives it its power back and so ends the window.
    press(&mut b1, Press::Both);
    let got = responses(&mut laptop);
    assert!(matches!(got.as_slice(), [(7, Response::Chip { .. })]), "{got:?}");
    assert!(!b1.board().tap_open);
    // Home is drawn once, and says the window is closed.
    assert_eq!(b1.board().screens.len(), before + 1);
    assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
    step(&mut b1);
    step(&mut b1);
    assert_eq!(b1.board().screens.len(), before + 1);
}

#[test]
fn a_press_in_the_step_a_laptop_first_asks_is_not_its_answer() {
    let (mut b1, _b2) = kept_pair((true, false));
    let mut laptop = plug_laptop(&mut b1);
    // The laptop's first request and a Both come in the same step: the question is drawn,
    // and the press is not taken as the answer, nor as home's.
    send(&mut laptop, 900, &Request::Identify);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("a laptop"), "{:?}", b1.board().last());
    assert!(responses(&mut laptop).is_empty());
    press(&mut b1, Press::Both);
    let got = responses(&mut laptop);
    assert!(matches!(got.as_slice(), [(900, Response::Identity { .. })]), "{got:?}");
}

#[test]
fn the_laptops_allowance_lasts_through_the_list_until_it_is_unplugged() {
    let (mut b1, _b2) = kept_pair((true, false));
    let mut laptop = plug_laptop(&mut b1);
    allow(&mut b1, &mut laptop);
    // Home, the list, the circle's actions: a request is answered with nothing asked.
    press(&mut b1, Press::Both);
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("> hello words"), "{:?}", b1.board().last());
    send(&mut laptop, 901, &Request::Identify);
    step(&mut b1);
    let got = responses(&mut laptop);
    assert!(matches!(got.as_slice(), [(901, Response::Identity { .. })]), "{got:?}");
    // Its "back", the list, "back": home answers with nothing asked too.
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
    send(&mut laptop, 902, &Request::Identify);
    step(&mut b1);
    let got = responses(&mut laptop);
    assert!(matches!(got.as_slice(), [(902, Response::Identity { .. })]), "{got:?}");
    assert!(b1.board().shows("closed to taps"));

    // Unplugged while the list is on the screen: the answer goes. Plugged in again, the next
    // request asks.
    press(&mut b1, Press::Both);
    laptop.present.set(false);
    step(&mut b1);
    assert!(b1.board().log.iter().any(|l| l == "down link: nothing plugged in"));
    laptop.present.set(true);
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    send(&mut laptop, 903, &Request::Identify);
    step(&mut b1);
    assert!(b1.board().shows("a laptop"), "{:?}", b1.board().last());
    assert!(responses(&mut laptop).is_empty());
}

#[test]
fn a_pass_after_the_last_circle_is_forgotten_asks_about_the_laptop_again() {
    // Unplugged while the pass runs and plugged in again, or plugged in all along: either
    // way, the first request after the keep asks.
    for unplugged in [true, false] {
        let (mut b1, _b2) = kept_pair((true, false));
        let mut laptop = plug_laptop(&mut b1);
        allow(&mut b1, &mut laptop);
        // Forget the only circle: the list, its actions, "forget this circle", Both, Both.
        press(&mut b1, Press::Both);
        press(&mut b1, Press::Both);
        to_entry(&mut b1, "forget this circle");
        press(&mut b1, Press::Both);
        assert!(b1.board().shows("forget this circle"), "{:?}", b1.board().last());
        press(&mut b1, Press::Both);
        press(&mut b1, Press::Both);
        assert!(b1.circles().is_empty());
        if unplugged {
            laptop.present.set(false);
        }
        // With no circle left the firmware runs a pass: a new pile, b1 at the bottom.
        let (lower, upper) = cable();
        b1.board().links = Some((FakeLink::open(), lower));
        let mut top = FakeBoard::seeded(0x2545_F491_4F6C_DD1D);
        top.links = Some((upper, FakeLink::open()));
        let mut b3 = Badge::new(top);
        for _ in 0..10_000 {
            b1.step();
            b3.step();
            if has_seat(&mut b1) && has_seat(&mut b3) {
                break;
            }
        }
        b3.board().presses.push_back(Press::Both);
        let mut at = (false, false);
        drive(&mut b1, &mut b3, |s1, s2| {
            at.0 |= *s1 == Step::Prompt;
            at.1 |= *s2 == Step::Prompt;
            at.0 && at.1
        });
        b1.board().presses.push_back(Press::Both);
        b3.board().presses.push_back(Press::Both);
        let mut kept = (false, false);
        drive(&mut b1, &mut b3, |s1, s2| {
            kept.0 |= *s1 == Step::Kept;
            kept.1 |= *s2 == Step::Kept;
            kept.0 && kept.1
        });
        laptop.present.set(true);
        let saved = format!("saved {}", circle_name(&b1));
        assert!(b1.board().shows(&saved), "{:?}", b1.board().last());
        b1.board().uptime += 5;
        step(&mut b1);
        assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
        send(&mut laptop, 950, &Request::Identify);
        step(&mut b1);
        assert!(b1.board().shows("a laptop"), "unplugged {unplugged}: {:?}", b1.board().last());
        assert!(responses(&mut laptop).is_empty(), "unplugged {unplugged}");
    }
}

#[test]
fn a_circle_is_one_page_of_actions_two_levels_down_and_back_comes_home() {
    let (mut b1, _b2) = kept_pair((true, false));
    let name = circle_name(&b1);
    press(&mut b1, Press::Both);               // the menu
    press(&mut b1, Press::Both);               // the first circle
    // A list: each action is a row, marked or not.
    for a in ["hello words", "key as code", "grants", "forget this circle", "back"] {
        let row = |l: &String| l.trim_start_matches(['>', ' ']) == a;
        assert!(b1.board().last().iter().any(row), "{a} in {:?}", b1.board().last());
    }
    press(&mut b1, Press::Both);               // hello words
    assert!(b1.board().shows("hello words"));
    assert!(!b1.board().last().iter().any(|l| l.contains("/5")));
    press(&mut b1, Press::Left);               // back to the actions, not another page
    assert!(b1.board().shows("> hello words"));
    // "back" on the actions is the list, with the circle marked. "back" there is home.
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    assert!(b1.board().shows(&format!("> {name}")), "{:?}", b1.board().last());
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("closed to taps"));
}
