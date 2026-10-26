//! Two fake cards in a pile run the ceremony through `Badge::step`, on one thread.

mod common;

use cahoots_badge::{Badge, Board, Press, Step};
use common::{cable, FakeBoard, FakeLink};

/// Card 1 at the bottom, card 2 on top: card 1's up is card 2's down.
fn pile_of_two() -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (lower, upper) = cable();
    let mut b1 = FakeBoard::seeded(0x9E37_79B9_7F4A_7C15);
    let mut b2 = FakeBoard::seeded(0xD1B5_4A32_D192_ED03);
    b1.links = Some((FakeLink::open(), lower));
    b2.links = Some((upper, FakeLink::open()));
    (Badge::new(b1), Badge::new(b2))
}

/// The pile screen with a seat on it has been drawn.
fn has_seat(b: &mut Badge<FakeBoard>) -> bool {
    b.board().screens.iter().any(|l| {
        l.iter().any(|x| x == "stack the cards") && l.iter().any(|x| x.starts_with("seat "))
    })
}

/// Step both badges in turn until `done` holds, or panic. Returns the last steps.
fn drive(
    b1: &mut Badge<FakeBoard>,
    b2: &mut Badge<FakeBoard>,
    mut done: impl FnMut(&mut Badge<FakeBoard>, &mut Badge<FakeBoard>, &Step, &Step) -> bool,
) -> (Step, Step) {
    for _ in 0..10_000 {
        let s1 = b1.step();
        let s2 = b2.step();
        assert!(!matches!(s1, Step::Failed(_)), "card 1: {s1:?}");
        assert!(!matches!(s2, Step::Failed(_)), "card 2: {s2:?}");
        if done(b1, b2, &s1, &s2) {
            return (s1, s2);
        }
    }
    panic!("the badges did not get there");
}

/// Seat both cards, press Both on top, and run to the keep prompt on both.
fn to_the_prompt() -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    to_the_prompt_with(|_, _| {})
}

/// As [`to_the_prompt`], calling `then` once both cards are in the ceremony.
fn to_the_prompt_with(
    mut then: impl FnMut(&mut Badge<FakeBoard>, &mut Badge<FakeBoard>),
) -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (mut b1, mut b2) = pile_of_two();
    drive(&mut b1, &mut b2, |b1, b2, _, _| {
        has_seat(b1) && has_seat(b2)
    });
    b2.board().presses.push_back(Press::Both);
    let mut on = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        on.0 |= *s1 == Step::InCeremony;
        on.1 |= *s2 == Step::InCeremony;
        on.0 && on.1
    });
    then(&mut b1, &mut b2);
    // Each card stops at the prompt; keep stepping the other until it gets there too.
    let mut at = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        at.0 |= *s1 == Step::Prompt;
        at.1 |= *s2 == Step::Prompt;
        at.0 && at.1
    });
    assert!(b1.board().shows("keep this circle?"));
    assert!(b2.board().shows("keep this circle?"));
    (b1, b2)
}

#[test]
fn two_fake_cards_complete_a_ceremony_and_keep_it() {
    let (mut b1, mut b2) = to_the_prompt();
    b1.board().presses.push_back(Press::Both);
    b2.board().presses.push_back(Press::Both);
    let mut kept = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        kept.0 |= *s1 == Step::Kept;
        kept.1 |= *s2 == Step::Kept;
        kept.0 && kept.1
    });
    assert_eq!(
        b1.circle().unwrap().circle_id,
        b2.circle().unwrap().circle_id
    );
    assert_eq!(b1.board().words().last(), b2.board().words().last());
    assert!(b1.board().words().last().is_some());
    let kept = b1.circle().unwrap().check_words().map(String::from);
    assert_eq!(b1.board().words().last(), Some(&kept));
    assert_eq!(b1.circle().unwrap().seat, 0);
    assert_eq!(b2.circle().unwrap().seat, 1);
    assert_eq!(b1.circle().unwrap().params.n, 2);
    assert_eq!(b1.board().load_circle().as_ref(), b1.circle());
    assert_eq!(b1.board().pad.len(), 192);
    assert_eq!(b1.board().pad, b2.board().pad);
    assert_eq!(
        b1.board().statuses().last().unwrap(),
        "saved. read the words aloud"
    );
}

#[test]
fn both_keeps_the_circle_and_a_single_press_discards() {
    for single in [Press::Left, Press::Right] {
        let (mut b1, mut b2) = to_the_prompt();
        b1.board().presses.push_back(single);
        b2.board().presses.push_back(Press::Both);
        let mut out = (false, false);
        drive(&mut b1, &mut b2, |_, _, s1, s2| {
            out.0 |= *s1 == Step::Discarded;
            out.1 |= *s2 == Step::Kept;
            out.0 && out.1
        });
        assert!(b1.circle().is_none(), "{single:?}");
        assert!(b1.board().load_circle().is_none());
        assert!(b1.board().pad.is_empty());
        assert_eq!(b1.board().statuses().last().unwrap(), "discarded");
        assert!(b2.circle().is_some());
        assert_eq!(b2.board().load_circle().as_ref(), b2.circle());
    }
}

#[test]
fn a_gesture_during_the_ceremony_is_not_an_answer() {
    let (mut b1, _b2) = to_the_prompt_with(|b1, _| b1.board().presses.push_back(Press::Both));
    for _ in 0..10 {
        assert_eq!(b1.step(), Step::Prompt);
    }
    assert!(b1.circle().is_none());
}

#[test]
fn run_once_returns_after_the_prompt() {
    // Both cards wait at the prompt, so each one's run_once can finish its pass alone.
    let (mut b1, mut b2) = to_the_prompt();
    b1.board().presses.push_back(Press::Left);
    assert_eq!(b1.run_once(), Ok(()));
    assert_eq!(b1.board().statuses().last().unwrap(), "discarded");
    b2.board().presses.push_back(Press::Both);
    assert_eq!(b2.run_once(), Ok(()));
    assert!(b2.circle().is_some());
}
