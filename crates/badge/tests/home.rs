//! The badge's home page: the circles it holds, whether a phone can tap now, and the left
//! button that opens a two-minute window for a tap. The menu is one list that ends with
//! "back".

mod common;

use cahoots_badge::{Badge, Press, Step};
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

/// Run a pile of two to the keep, keep the circle on both cards, then take one idle step on
/// each, as the firmware's loop does right after a keep: each card rests where that puts it.
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
    step(&mut b1);
    step(&mut b2);
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
    assert!(b1.board().last().iter().any(|l| l.contains("tap") && l.contains("A+B  menu")));
}

#[test]
fn the_left_button_opens_a_two_minute_tap_window_that_closes_by_itself() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Left);
    assert!(b1.board().tap_open);
    assert!(b1.board().shows("open for a tap, 2 min"), "{:?}", b1.board().last());
    b1.board().uptime += 61;
    step(&mut b1);
    assert!(b1.board().shows("open for a tap, 1 min"));
    b1.board().uptime += 60;
    step(&mut b1);
    assert!(!b1.board().tap_open);
    assert!(b1.board().shows("closed to taps"));
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
