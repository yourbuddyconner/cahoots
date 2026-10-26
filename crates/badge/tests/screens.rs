//! The screens the badge draws: their text and their frames.

use cahoots_badge::screens::{code, menu_page, words};
use cahoots_hal::Frame;

const W: [&str; 6] = ["abandon", "ability", "able", "about", "above", "absent"];

/// True if any pixel in `x0..x1` by `y0..y1` is black.
fn any_black(f: &Frame, x0: usize, x1: usize, y0: usize, y1: usize) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| f.get(x, y)))
}

#[test]
fn words_screen_lists_the_words_in_order() {
    let s = words("bench", "TRNG T1", &W);
    let tokens: Vec<&str> = s.lines.iter().flat_map(|l| l.split_whitespace()).collect();
    let at = tokens
        .iter()
        .position(|t| *t == W[0])
        .expect("the first word");
    assert_eq!(&tokens[at..at + 6], &W);
    assert!(s.lines[0].contains("bench") && s.lines[0].contains("TRNG T1"));
}

#[test]
fn menu_page_marks_its_neighbours() {
    let body = ["Both to spend 32 bytes"];
    let both = menu_page("bench", "TRNG T1", 2, 3, "key as code", &body, true, true);
    assert!(any_black(&both.frame, 2, 10, 95, 105), "left arrow");
    assert!(any_black(&both.frame, 190, 198, 95, 105), "right arrow");
    assert!(both.lines.iter().any(|l| l == "2/3"));
    let right = menu_page("bench", "TRNG T1", 1, 3, "key as code", &body, false, true);
    assert!(!any_black(&right.frame, 2, 10, 95, 105), "no left arrow");
    assert!(any_black(&right.frame, 190, 198, 95, 105), "right arrow");
}

#[test]
fn code_screen_shows_32_bytes_as_four_rows() {
    let s = code(&[0xAB; 32]);
    let rows: Vec<String> = s
        .lines
        .iter()
        .map(|l| l.split_whitespace().collect::<String>())
        .filter(|l| l.len() == 16 && l.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    assert_eq!(rows, vec!["ab".repeat(8); 4]);
}

#[test]
fn a_frame_is_deterministic() {
    let a = words("bench", "TRNG T1", &W);
    let b = words("bench", "TRNG T1", &W);
    assert_eq!(a.frame.bits, b.frame.bits);
    assert!(a.frame.bits.iter().any(|&b| b != 0));
}

#[test]
fn only_the_code_screen_is_secret() {
    assert!(code(&[0xAB; 32]).secret);
    assert!(!words("bench", "TRNG T1", &W).secret);
    assert!(!menu_page("bench", "TRNG T1", 2, 3, "key as code", &["x"], true, true).secret);
}
