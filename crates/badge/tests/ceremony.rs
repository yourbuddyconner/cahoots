//! Two fake cards in a pile run the ceremony through `Badge::step`, on one thread.

mod common;

use cahoots_badge::{Badge, Board, Step};
use common::{cable, Button, FakeBoard, FakeLink};

/// Card 1 at the bottom, card 2 on top: card 1's up is card 2's down.
fn pile_of_two() -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (lower, upper) = cable();
    let mut b1 = FakeBoard::seeded(0x9E37_79B9_7F4A_7C15);
    let mut b2 = FakeBoard::seeded(0xD1B5_4A32_D192_ED03);
    b1.links = Some((FakeLink::open(), lower));
    b2.links = Some((upper, FakeLink::open()));
    (Badge::new(b1), Badge::new(b2))
}

fn has_seat(b: &mut Badge<FakeBoard>) -> bool {
    b.board()
        .statuses()
        .iter()
        .any(|s| s.starts_with("seat ") && s.ends_with("B when the pile is whole"))
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

/// Seat both cards, press B on top, and run to the keep prompt on both.
fn to_the_prompt() -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (mut b1, mut b2) = pile_of_two();
    drive(&mut b1, &mut b2, |b1, b2, _, _| {
        has_seat(b1) && has_seat(b2)
    });
    b2.board().buttons.push_back(Button::B);
    // Each card stops at the prompt; keep stepping the other until it gets there too.
    let mut at = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        at.0 |= *s1 == Step::Prompt;
        at.1 |= *s2 == Step::Prompt;
        at.0 && at.1
    });
    assert_eq!(
        b1.board().statuses().last().unwrap(),
        "B keep this circle . A discard"
    );
    (b1, b2)
}

#[test]
fn two_fake_cards_complete_a_ceremony_and_keep_it() {
    let (mut b1, mut b2) = to_the_prompt();
    b1.board().buttons.push_back(Button::B);
    b2.board().buttons.push_back(Button::B);
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
fn a_at_the_prompt_discards() {
    let (mut b1, mut b2) = to_the_prompt();
    b1.board().buttons.push_back(Button::A);
    b2.board().buttons.push_back(Button::B);
    let mut out = (false, false);
    drive(&mut b1, &mut b2, |_, _, s1, s2| {
        out.0 |= *s1 == Step::Discarded;
        out.1 |= *s2 == Step::Kept;
        out.0 && out.1
    });
    assert!(b1.circle().is_none());
    assert!(b1.board().load_circle().is_none());
    assert!(b1.board().pad.is_empty());
    assert_eq!(b1.board().statuses().last().unwrap(), "discarded");
    assert!(b2.circle().is_some());
}

#[test]
fn run_once_returns_after_the_prompt() {
    // Both cards wait at the prompt, so each one's run_once can finish its pass alone.
    let (mut b1, mut b2) = to_the_prompt();
    b1.board().buttons.push_back(Button::A);
    assert_eq!(b1.run_once(), Ok(()));
    assert_eq!(b1.board().statuses().last().unwrap(), "discarded");
    b2.board().buttons.push_back(Button::B);
    assert_eq!(b2.run_once(), Ok(()));
    assert!(b2.circle().is_some());
}
