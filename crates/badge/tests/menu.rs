//! The idle menu: a ring of pages (the words, a key as a code, serving a laptop on the down
//! link), Left and Right to turn the page, Both to select.

mod common;

use cahoots_badge::{Badge, Press};
use cahoots_core::frame::{self, Frame};
use cahoots_custodian::wire::{
    ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE, MAX_REQUEST_LEN,
};
use cahoots_custodian::CircleId;
use cahoots_hal::{Entropy, Link};
use common::{cable, FakeBoard, FakeLink, FAKE_TIER};

/// A badge booted with a stored circle and in the menu, and the laptop end of its down link.
fn in_the_menu() -> (Badge<FakeBoard>, FakeLink, CircleId) {
    let (mut board, _, _) = FakeBoard::with_circle();
    let id = board.circle.as_ref().unwrap().circle_id;
    let (laptop, down) = cable();
    board.links = Some((down, FakeLink::open()));
    let mut badge = Badge::new(board);
    badge.boot();
    badge.idle_step();
    (badge, laptop, id)
}

fn press(badge: &mut Badge<FakeBoard>, gesture: Press) {
    badge.board().presses.push_back(gesture);
    badge.idle_step();
}

/// The last screen is menu page `page` of 3, titled `title`.
fn on_page(badge: &mut Badge<FakeBoard>, page: usize, title: &str) -> bool {
    let b = badge.board();
    b.shows(&format!("{page}/3")) && b.shows(title)
}

/// Right twice to the Serve page, then Both to start serving.
fn serving() -> (Badge<FakeBoard>, FakeLink, CircleId) {
    let (mut badge, laptop, id) = in_the_menu();
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Right);
    assert!(on_page(&mut badge, 3, "serve a laptop"));
    press(&mut badge, Press::Both);
    assert!(badge.board().shows("serving"));
    (badge, laptop, id)
}

/// Send a 32-byte key request with `seq` and step once, so the consent prompt is drawn.
fn ask_psk(badge: &mut Badge<FakeBoard>, laptop: &mut FakeLink, id: CircleId, seq: u32) {
    send(
        laptop,
        seq,
        &Request::Psk {
            circle: id,
            context: b"wg".to_vec(),
        },
    );
    badge.idle_step();
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

fn remaining(badge: &mut Badge<FakeBoard>, laptop: &mut FakeLink, id: CircleId) -> u64 {
    send(laptop, 99, &Request::Remaining { circle: id });
    badge.idle_step();
    match responses(laptop).as_slice() {
        [(99, Response::Remaining { bytes, .. })] => *bytes,
        other => panic!("not a remaining answer: {other:?}"),
    }
}

#[test]
fn the_menu_opens_on_the_words_page() {
    let (mut badge, _laptop, _) = in_the_menu();
    assert!(on_page(&mut badge, 1, "words"));
    let kept = badge.circle().unwrap().check_words().map(String::from);
    assert_eq!(badge.board().words().last(), Some(&kept));
}

#[test]
fn left_and_right_walk_the_page_ring() {
    let (mut badge, _laptop, _) = in_the_menu();
    press(&mut badge, Press::Right);
    assert!(on_page(&mut badge, 2, "key as code"));
    assert!(badge.board().shows("Both to spend 32 bytes"));
    press(&mut badge, Press::Right);
    assert!(on_page(&mut badge, 3, "serve a laptop"));
    press(&mut badge, Press::Right);
    assert!(on_page(&mut badge, 1, "words"));
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 3, "serve a laptop"));
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 2, "key as code"));
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 1, "words"));
    assert!(
        badge.board().codes().is_empty(),
        "turning pages spends nothing"
    );
}

#[test]
fn no_gesture_draws_nothing() {
    let (mut badge, _laptop, _) = in_the_menu();
    let drawn = badge.board().screens.len();
    for _ in 0..5 {
        badge.idle_step();
    }
    assert_eq!(badge.board().screens.len(), drawn);
}

#[test]
fn both_on_the_key_page_spends_32_bytes_and_shows_the_code() {
    let (mut badge, mut laptop, id) = serving();
    let before = remaining(&mut badge, &mut laptop, id);
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 2, "key as code"));
    press(&mut badge, Press::Both);
    assert_eq!(badge.board().codes().len(), 1);
    assert_eq!(badge.board().codes()[0].len(), 32);
    assert!(badge.board().shows("code"));
    // Any gesture on the code screen goes back to the Key page, not to a neighbour.
    press(&mut badge, Press::Right);
    assert!(on_page(&mut badge, 2, "key as code"));
    press(&mut badge, Press::Both);
    press(&mut badge, Press::Both);
    assert!(on_page(&mut badge, 2, "key as code"));
    let codes = badge.board().codes();
    assert_eq!(codes.len(), 2);
    assert_ne!(codes[0], codes[1]);
    // The fake circle's slice is 64 bytes: a third key is refused, and Left goes back.
    press(&mut badge, Press::Both);
    assert!(badge.board().last()[0].starts_with("key: "));
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 2, "key as code"));
    assert_eq!(badge.board().codes().len(), 2);
    // Serving stopped when the page turned, so ask on the Serve page.
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Both);
    assert_eq!(remaining(&mut badge, &mut laptop, id), before - 2 * 32);
}

#[test]
fn both_on_the_words_page_shows_the_six_words_again() {
    let (mut badge, _laptop, _) = in_the_menu();
    let first = badge.board().words().last().unwrap().clone();
    let n = badge.board().words().len();
    press(&mut badge, Press::Both);
    assert!(on_page(&mut badge, 1, "words"));
    assert_eq!(badge.board().words().len(), n + 1);
    assert_eq!(badge.board().words().last().unwrap(), &first);
}

#[test]
fn words_are_the_same_after_a_spend() {
    let (mut badge, _laptop, _) = in_the_menu();
    let before = badge.board().words().last().unwrap().clone();
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Both);
    assert_eq!(badge.board().codes().len(), 1);
    press(&mut badge, Press::Both);
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 1, "words"));
    assert_eq!(badge.board().words().last().unwrap(), &before);
}

#[test]
fn serve_answers_identify_without_a_prompt() {
    let (mut badge, mut laptop, id) = serving();
    send(&mut laptop, 7, &Request::Identify);
    badge.idle_step();
    let got = responses(&mut laptop);
    assert_eq!(got.len(), 1);
    match &got[0] {
        (7, Response::Identity { circles, tier, .. }) => {
            assert_eq!(*tier, FAKE_TIER);
            assert_eq!(circles.len(), 1);
            assert_eq!(circles[0].tier, FAKE_TIER);
            assert_eq!(circles[0].circle_id, id);
            assert_eq!(circles[0].name, "fakes");
            assert_eq!(circles[0].seat, 1);
        }
        other => panic!("not an identity: {other:?}"),
    }
    assert!(on_page(&mut badge, 3, "serve a laptop"));
}

#[test]
fn the_serve_page_shows_listening_once_a_laptop_is_present() {
    let (mut badge, laptop, _) = in_the_menu();
    laptop.present.set(false);
    press(&mut badge, Press::Left);
    assert!(on_page(&mut badge, 3, "serve a laptop"));
    assert!(!badge.board().shows("a laptop is listening"));
    laptop.present.set(true);
    badge.idle_step();
    assert!(on_page(&mut badge, 3, "serve a laptop"));
    assert!(badge.board().shows("a laptop is listening"));
    laptop.present.set(false);
    badge.idle_step();
    assert!(!badge.board().shows("a laptop is listening"));
}

#[test]
fn serve_psk_waits_for_both() {
    let (mut badge, mut laptop, id) = serving();
    let before = remaining(&mut badge, &mut laptop, id);
    ask_psk(&mut badge, &mut laptop, id, 3);
    for _ in 0..5 {
        badge.idle_step();
    }
    assert!(responses(&mut laptop).is_empty());
    assert!(badge.board().shows("spend 32 bytes"));
    assert!(badge.board().shows("for fakes?"));
    press(&mut badge, Press::Both);
    let got = responses(&mut laptop);
    assert!(
        matches!(got.as_slice(), [(3, Response::Psk { handle, .. })] if handle.circle_id == id),
        "{got:?}"
    );
    assert!(on_page(&mut badge, 3, "serve a laptop"));
    assert_eq!(remaining(&mut badge, &mut laptop, id), before - 32);
}

#[test]
fn a_single_press_refuses_consent_and_both_allows() {
    for single in [Press::Left, Press::Right] {
        let (mut badge, mut laptop, id) = serving();
        let before = remaining(&mut badge, &mut laptop, id);
        ask_psk(&mut badge, &mut laptop, id, 4);
        press(&mut badge, single);
        assert_eq!(
            responses(&mut laptop),
            vec![(
                4,
                Response::Error {
                    code: ErrorCode::Denied
                }
            )],
            "{single:?}"
        );
        assert!(on_page(&mut badge, 3, "serve a laptop"), "{single:?}");
        assert_eq!(remaining(&mut badge, &mut laptop, id), before);

        // Still serving: the next request is asked and Both allows it.
        ask_psk(&mut badge, &mut laptop, id, 5);
        press(&mut badge, Press::Both);
        let got = responses(&mut laptop);
        assert!(
            matches!(got.as_slice(), [(5, Response::Psk { .. })]),
            "{got:?}"
        );
        assert_eq!(remaining(&mut badge, &mut laptop, id), before - 32);
    }
}

#[test]
fn spend_prompt_names_the_length() {
    let (mut badge, mut laptop, id) = serving();
    send(
        &mut laptop,
        5,
        &Request::Spend {
            circle: id,
            len: 12,
        },
    );
    badge.idle_step();
    assert!(badge.board().shows("spend 12 bytes"));
}

#[test]
fn a_request_outside_serve_is_busy() {
    let (mut badge, mut laptop, _) = in_the_menu();
    send(&mut laptop, 1, &Request::Identify);
    badge.idle_step();
    assert_eq!(
        responses(&mut laptop),
        vec![(
            1,
            Response::Error {
                code: ErrorCode::Busy
            }
        )]
    );
}

#[test]
fn turning_off_the_serve_page_stops_serving() {
    let (mut badge, mut laptop, _) = serving();
    press(&mut badge, Press::Right);
    assert!(on_page(&mut badge, 1, "words"));
    press(&mut badge, Press::Left);
    send(&mut laptop, 2, &Request::Identify);
    badge.idle_step();
    assert_eq!(
        responses(&mut laptop),
        vec![(
            2,
            Response::Error {
                code: ErrorCode::Busy
            }
        )]
    );
}

#[test]
fn serve_resyncs_after_garbage_on_the_down_link() {
    let (mut badge, mut laptop, id) = serving();
    let mut junk = vec![0u8; 2048];
    common::Xorshift(0x1234_5678_9ABC_DEF1)
        .fill(&mut junk)
        .unwrap();
    laptop.send(&junk);
    badge.idle_step();
    send(&mut laptop, 11, &Request::Identify);
    badge.idle_step();
    match responses(&mut laptop).as_slice() {
        [(11, Response::Identity { circles, .. })] => assert_eq!(circles[0].circle_id, id),
        other => panic!("not an identity: {other:?}"),
    }
}

#[test]
fn serve_resyncs_past_a_header_that_is_never_filled() {
    let (mut badge, mut laptop, id) = serving();
    // A header whose length is never filled: the request behind it must still be read.
    let mut fake = vec![KIND_REQUEST, 0, 0, 0, 0];
    fake.extend_from_slice(&3000u32.to_le_bytes());
    laptop.send(&fake);
    send(&mut laptop, 12, &Request::Identify);
    badge.idle_step();
    match responses(&mut laptop).as_slice() {
        [(12, Response::Identity { circles, .. })] => assert_eq!(circles[0].circle_id, id),
        other => panic!("not an identity: {other:?}"),
    }
}

#[test]
fn serve_drops_a_header_longer_than_any_request() {
    let (mut badge, mut laptop, id) = serving();
    let mut fake = vec![KIND_REQUEST, 0, 0, 0, 0];
    fake.extend_from_slice(&(MAX_REQUEST_LEN as u32 + 1).to_le_bytes());
    laptop.send(&fake);
    badge.idle_step();
    send(&mut laptop, 13, &Request::Identify);
    badge.idle_step();
    match responses(&mut laptop).as_slice() {
        [(13, Response::Identity { circles, .. })] => assert_eq!(circles[0].circle_id, id),
        other => panic!("not an identity: {other:?}"),
    }
}

#[test]
fn a_gesture_before_the_prompt_is_drawn_is_not_an_answer() {
    for early in [Press::Both, Press::Left] {
        let (mut badge, mut laptop, id) = serving();
        let before = remaining(&mut badge, &mut laptop, id);
        // A gesture made while the prompt's refresh blocks: queued before the badge reads
        // the request.
        badge.board().presses.push_back(early);
        ask_psk(&mut badge, &mut laptop, id, 8);
        assert!(responses(&mut laptop).is_empty(), "{early:?}");
        assert!(
            badge.board().presses.is_empty(),
            "the early gesture is spent"
        );
        assert!(badge.board().shows("spend 32 bytes"));
        press(&mut badge, Press::Both);
        let got = responses(&mut laptop);
        assert!(
            matches!(got.as_slice(), [(8, Response::Psk { .. })]),
            "{got:?}"
        );
        assert_eq!(remaining(&mut badge, &mut laptop, id), before - 32);
    }
}
