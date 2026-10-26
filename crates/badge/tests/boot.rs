mod common;

use cahoots_badge::{Badge, Board};
use cahoots_core::checkwords::check_words;
use common::FakeBoard;

#[test]
fn boot_without_a_circle_shows_the_selftest_and_waits() {
    let mut badge = Badge::new(FakeBoard::new());
    badge.boot();
    let b = badge.board();
    let screen = &b.display.status[0];
    assert!(screen.contains("trng: health tests pass"));
    assert!(screen.contains("fake: ok"));
    assert!(screen.contains("circle: none yet"));
    assert!(b.display.words.is_empty());
    assert!(badge.circle().is_none());
}

#[test]
fn boot_with_a_circle_shows_its_words() {
    let (board, params, pad) = FakeBoard::with_circle();
    let mut badge = Badge::new(board);
    badge.boot();
    assert!(badge.circle().is_some());
    let b = badge.board();
    assert!(b.display.status[0].contains("circle: fakes seat 1 of 3"));
    assert_eq!(b.display.words.len(), 1);
    assert_eq!(
        b.display.words[0],
        check_words(&params, &pad).map(String::from)
    );
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
    assert!(badge.board().display.words.is_empty());
    assert!(badge
        .board()
        .display
        .status
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
        badge.board().display.words[0],
        check_words(&params, &pad).map(String::from)
    );
}
