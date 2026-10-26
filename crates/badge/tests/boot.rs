mod common;

use cahoots_badge::{Badge, Board, Press};
use cahoots_core::checkwords::check_words;
use common::FakeBoard;

#[test]
fn boot_without_a_circle_shows_the_selftest_and_waits() {
    let mut badge = Badge::new(FakeBoard::new());
    badge.boot();
    let b = badge.board();
    let screen = &b.statuses()[0];
    assert!(screen.contains("trng: health tests pass"));
    assert!(screen.contains("fake: ok"));
    assert!(screen.contains("circle: none yet"));
    assert!(b.words().is_empty());
    assert_eq!(
        b.display.frames.len(),
        b.screens.len(),
        "a frame per screen"
    );
    assert!(badge.circle().is_none());
}

#[test]
fn boot_with_a_circle_shows_its_words() {
    let (board, params, pad) = FakeBoard::with_circle();
    let mut badge = Badge::new(board);
    badge.boot();
    assert!(badge.circle().is_some());
    let b = badge.board();
    assert!(b.statuses()[0].contains("circle: fakes seat 1 of 3"));
    assert_eq!(b.words().len(), 1);
    assert_eq!(b.words()[0], check_words(&params, &pad).map(String::from));
}

#[test]
fn a_held_at_boot_forgets_the_circle() {
    let (mut board, _, _) = FakeBoard::with_circle();
    board.a_down = true;
    let mut badge = Badge::new(board);
    badge.boot();
    assert!(badge.circle().is_none());
    assert!(badge.board().load_circle().is_none());
    assert!(badge.board().pad.iter().all(|&b| b == 0));
    assert!(badge.board().words().is_empty());
    assert!(badge
        .board()
        .statuses()
        .last()
        .unwrap()
        .contains("release A"));
    assert!(!badge.board().a_down());
}

#[test]
fn boot_shows_the_recorded_words_after_bytes_are_spent() {
    // Spending erases pad bytes. The words come from the record, so they do not change.
    let (mut board, params, pad) = FakeBoard::with_circle();
    board.pad[64..96].fill(0);
    let mut badge = Badge::new(board);
    badge.boot();
    assert_eq!(
        badge.board().words()[0],
        check_words(&params, &pad).map(String::from)
    );
}

#[test]
fn the_self_test_stays_for_two_and_a_half_seconds() {
    let mut badge = Badge::new(FakeBoard::new());
    badge.boot();
    assert!(badge.board().elapsed_ms >= 2500);
}

#[test]
fn both_ends_the_self_test_early_and_one_button_does_not() {
    let mut board = FakeBoard::new();
    board
        .presses
        .extend([Press::Left, Press::Right, Press::Both]);
    let mut badge = Badge::new(board);
    badge.boot();
    let b = badge.board();
    assert!(b.presses.is_empty(), "every gesture was read");
    assert!(
        b.elapsed_ms < 2500,
        "Both continued after {} ms",
        b.elapsed_ms
    );
    assert!(b.elapsed_ms > 0, "Left and Right did not");
}

#[test]
fn both_to_skip_the_self_test_does_not_forget_the_circle() {
    // A chord holds A down when Both ends the wait. That is not "A held at power-on".
    let (mut board, _, _) = FakeBoard::with_circle();
    board.a_down = true;
    board.presses.push_back(Press::Both);
    let mut badge = Badge::new(board);
    badge.boot();
    assert!(badge.circle().is_some());
    assert!(badge.board().load_circle().is_some());
    assert!(badge.board().pad.iter().any(|&b| b != 0));
}

#[test]
fn a_pressed_after_power_on_does_not_forget_the_circle() {
    let (mut board, _, _) = FakeBoard::with_circle();
    board.hold_a_at_ms = Some(100);
    let mut badge = Badge::new(board);
    badge.boot();
    assert!(badge.circle().is_some());
    assert!(badge.board().load_circle().is_some());
    assert!(badge.board().pad.iter().any(|&b| b != 0));
}
