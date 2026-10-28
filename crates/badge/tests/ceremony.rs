//! Two fake cards in a pile run the ceremony through `Badge::step`, on one thread.

mod common;

use cahoots_badge::{Badge, Board, Press, Step};
use cahoots_core::manifest::SealedManifest;
use cahoots_core::seat::SeatKey;
use common::{cable, FakeBoard, FakeLink, FAKE_TIER};

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
fn both_keeps_the_circle_and_a_single_press_then_both_discards() {
    for single in [Press::Left, Press::Right] {
        let (mut b1, mut b2) = to_the_prompt();
        b1.board().presses.extend([single, Press::Both]);
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
fn a_single_press_asks_again_and_another_goes_back() {
    let (mut b1, _b2) = to_the_prompt();
    let drawn = b1.board().screens.len();
    b1.board().presses.push_back(Press::Left);
    assert_eq!(b1.step(), Step::Prompt);
    assert_eq!(b1.board().screens.len(), drawn + 1, "the discard question is drawn");
    assert!(b1.board().screens.last().unwrap().join(" ").contains("discard this circle?"));
    b1.board().presses.push_back(Press::Right);
    assert_eq!(b1.step(), Step::Prompt);
    assert!(b1.board().screens.last().unwrap().join(" ").contains("keep this circle?"));
    for _ in 0..5 {
        assert_eq!(b1.step(), Step::Prompt);
    }
    assert!(b1.circle().is_none());
    assert!(!b1.board().log.iter().any(|l| l == "discarded"));
}

#[test]
fn left_or_right_in_the_pile_does_nothing() {
    let (mut b1, mut b2) = pile_of_two();
    drive(&mut b1, &mut b2, |b1, b2, _, _| {
        has_seat(b1) && has_seat(b2)
    });
    let drawn = b2.board().screens.len();
    b2.board().presses.extend([Press::Left, Press::Right]);
    for _ in 0..50 {
        assert_eq!(b1.step(), Step::InPile);
        assert_eq!(b2.step(), Step::InPile);
    }
    assert!(b2.board().presses.is_empty(), "both gestures were read");
    assert_eq!(b2.board().screens.len(), drawn, "nothing drawn");
    assert!(!b2.board().log.iter().any(|l| l.contains("counting")));
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
    b1.board().presses.extend([Press::Left, Press::Both]);
    assert_eq!(b1.run_once(), Ok(()));
    assert_eq!(b1.board().statuses().last().unwrap(), "discarded");
    b2.board().presses.push_back(Press::Both);
    assert_eq!(b2.run_once(), Ok(()));
    assert!(b2.circle().is_some());
}

/// Run two cards to the prompt and keep the circle on both.
fn kept_pair() -> (Badge<FakeBoard>, Badge<FakeBoard>) {
    let (mut b1, mut b2) = to_the_prompt();
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

#[test]
fn keeping_the_circle_stores_the_seat_key_and_a_manifest_that_verifies() {
    let (mut b1, mut b2) = kept_pair();
    let mut ids = Vec::new();
    for b in [&mut b1, &mut b2] {
        let c = b.circle().unwrap().clone();
        assert_eq!(b.board().load_circle().as_ref(), Some(&c));
        let manifest = SealedManifest::decode(&c.manifest)
            .unwrap()
            .verify()
            .unwrap();
        assert_eq!(manifest.circle_id, c.circle_id);
        assert_eq!(manifest.params, c.params);
        let key = SeatKey::from_seed(&c.seat_key).unwrap();
        assert_eq!(manifest.seat_key(c.seat), Some(&key.public()));
        // The seat key never reaches the log or a noted screen.
        let seed: String = c.seat_key.iter().map(|x| format!("{x:02x}")).collect();
        let board = b.board();
        assert!(!board.log.iter().any(|l| l.contains(&seed)));
        assert!(!board.screens.iter().flatten().any(|l| l.contains(&seed)));
        ids.push(manifest.id());
    }
    assert_eq!(ids[0], ids[1]);
    assert_ne!(b1.circle().unwrap().seat_key, b2.circle().unwrap().seat_key);
}

#[test]
fn the_manifest_lists_this_badge_at_its_tier() {
    let (b1, b2) = kept_pair();
    for b in [&b1, &b2] {
        let c = b.circle().unwrap();
        assert_eq!(c.tier, FAKE_TIER);
        let manifest = SealedManifest::decode(&c.manifest)
            .unwrap()
            .verify()
            .unwrap();
        assert_eq!(manifest.seats.len(), 2);
        let seat = &manifest.seats[usize::from(c.seat)];
        assert_eq!(seat.seat, c.seat);
        assert_eq!(seat.tier, FAKE_TIER);
    }
}

#[test]
fn the_ceremony_screen_counts_attestations() {
    let (mut b1, _b2) = kept_pair();
    assert!(b1
        .board()
        .screens
        .iter()
        .any(|l| l.iter().any(|x| x == "attestations 2/2")));
}
