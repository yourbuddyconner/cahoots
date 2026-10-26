//! The idle menu: the words, a key as a code, and serving a laptop on the down link.

mod common;

use cahoots_badge::Badge;
use cahoots_core::frame::{self, Frame};
use cahoots_custodian::wire::{
    ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE, MAX_REQUEST_LEN,
};
use cahoots_custodian::CircleId;
use cahoots_hal::{Entropy, Link};
use common::{cable, Button, FakeBoard, FakeLink, FAKE_TIER};

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

fn press(badge: &mut Badge<FakeBoard>, button: Button) {
    badge.board().buttons.push_back(button);
    badge.idle_step();
}

/// A then B: from the words item to serve, and serving.
fn serving() -> (Badge<FakeBoard>, FakeLink, CircleId) {
    let (mut badge, laptop, id) = in_the_menu();
    press(&mut badge, Button::A);
    press(&mut badge, Button::A);
    assert_eq!(badge.board().statuses().last().unwrap(), "serve . B");
    press(&mut badge, Button::B);
    (badge, laptop, id)
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
fn key_as_code_spends_32_and_shows_a_qr() {
    let (mut badge, _laptop, _) = in_the_menu();
    press(&mut badge, Button::A);
    assert_eq!(badge.board().statuses().last().unwrap(), "key as code . B");
    press(&mut badge, Button::B);
    assert_eq!(badge.board().codes().len(), 1);
    assert_eq!(badge.board().codes()[0].len(), 32);
    assert_eq!(
        badge.board().statuses().last().unwrap(),
        "shown. A for menu"
    );
    press(&mut badge, Button::B);
    let qr = &badge.board().codes();
    assert_eq!(qr.len(), 2);
    assert_ne!(qr[0], qr[1]);
}

#[test]
fn words_shows_the_six_words() {
    let (mut badge, _laptop, _) = in_the_menu();
    let at_boot = badge.board().words().clone();
    assert_eq!(at_boot.len(), 1);
    assert_eq!(badge.board().statuses().last().unwrap(), "words");
    press(&mut badge, Button::B);
    assert_eq!(badge.board().words().len(), 2);
    assert_eq!(badge.board().words()[1], at_boot[0]);
}

#[test]
fn words_are_the_same_after_a_spend() {
    let (mut badge, _laptop, _) = in_the_menu();
    press(&mut badge, Button::B);
    let before = badge.board().words().last().unwrap().clone();
    press(&mut badge, Button::A);
    press(&mut badge, Button::B);
    assert_eq!(badge.board().codes().len(), 1);
    press(&mut badge, Button::A);
    press(&mut badge, Button::A);
    assert_eq!(badge.board().statuses().last().unwrap(), "words");
    press(&mut badge, Button::B);
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
}

#[test]
fn serve_psk_waits_for_b() {
    let (mut badge, mut laptop, id) = serving();
    let before = remaining(&mut badge, &mut laptop, id);
    send(
        &mut laptop,
        3,
        &Request::Psk {
            circle: id,
            context: b"wg".to_vec(),
        },
    );
    for _ in 0..5 {
        badge.idle_step();
    }
    assert!(responses(&mut laptop).is_empty());
    assert_eq!(
        badge.board().statuses().last().unwrap(),
        "spend 32 for fakes? B yes . A no"
    );
    press(&mut badge, Button::B);
    let got = responses(&mut laptop);
    assert!(
        matches!(got.as_slice(), [(3, Response::Psk { handle, .. })] if handle.circle_id == id),
        "{got:?}"
    );
    assert_eq!(remaining(&mut badge, &mut laptop, id), before - 32);
}

#[test]
fn serve_psk_refused_with_a() {
    let (mut badge, mut laptop, id) = serving();
    let before = remaining(&mut badge, &mut laptop, id);
    send(
        &mut laptop,
        4,
        &Request::Psk {
            circle: id,
            context: Vec::new(),
        },
    );
    badge.idle_step();
    press(&mut badge, Button::A);
    assert_eq!(
        responses(&mut laptop),
        vec![(
            4,
            Response::Error {
                code: ErrorCode::Denied
            }
        )]
    );
    assert_eq!(remaining(&mut badge, &mut laptop, id), before);
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
    assert_eq!(
        badge.board().statuses().last().unwrap(),
        "spend 12 for fakes? B yes . A no"
    );
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
fn a_press_before_the_prompt_is_not_consent() {
    let (mut badge, mut laptop, id) = serving();
    let before = remaining(&mut badge, &mut laptop, id);
    // B tapped while the prompt's refresh blocks: queued before the badge reads the request.
    badge.board().buttons.push_back(Button::B);
    send(
        &mut laptop,
        8,
        &Request::Psk {
            circle: id,
            context: b"wg".to_vec(),
        },
    );
    badge.idle_step();
    assert!(responses(&mut laptop).is_empty());
    assert!(badge.board().buttons.is_empty(), "the early press is spent");
    assert_eq!(
        badge.board().statuses().last().unwrap(),
        "spend 32 for fakes? B yes . A no"
    );
    press(&mut badge, Button::B);
    let got = responses(&mut laptop);
    assert!(
        matches!(got.as_slice(), [(8, Response::Psk { .. })]),
        "{got:?}"
    );
    assert_eq!(remaining(&mut badge, &mut laptop, id), before - 32);
}
