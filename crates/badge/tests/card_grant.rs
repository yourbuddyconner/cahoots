//! Contact cards, grants and seal keys on the badge: the screens, the checks before anyone is
//! asked, and the Serve path end to end with the seat key and root in the chip or in software.

mod common;

use cahoots_badge::chip::{ChipRecord, SlotEntry};
use cahoots_badge::screens;
use cahoots_badge::{Badge, Press};
use cahoots_chip::{ChipOps, Curve, KeyId, KeyKind, SoftChip};
use cahoots_core::cose;
use cahoots_core::frame::{self, Frame};
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_custodian::wire::{
    ChipRequest, ChipResponse, ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE,
};
use cahoots_custodian::CircleId;
use cahoots_hal::Link;
use cahoots_objects::card::{self, Card, PURPOSE_SSH};
use cahoots_objects::grant::{self, key_id, Grant, Grantor};
use common::{cable, FakeBoard, FakeLink};

const ROOT: [u8; 32] = [0x3c; 32];

// ---- the screens ----

#[test]
fn card_screen_lines() {
    let s = screens::card_sign("Ada Lovelace", 2, 3, "fakes");
    assert_eq!(
        s.lines,
        [
            "sign your card",
            "Ada Lovelace",
            "2 handles, 3 endpoints",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    let s = screens::card_sign("Ada", 1, 1, "fakes");
    assert_eq!(s.lines[2], "1 handle, 1 endpoint");
    let s = screens::card_sign("Ada", 0, 0, "fakes");
    assert_eq!(s.lines[2], "no handles, no endpoints");
}

#[test]
fn seal_key_screen_lines() {
    let s = screens::seal_key("seal/card", "fakes");
    assert_eq!(
        s.lines,
        [
            "release a key",
            "key for seal/card",
            "a laptop asks for it",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
}

#[test]
fn grant_screen_lines() {
    let s = screens::grant(
        "1301c691",
        &["wireguard", "tls", "mls"],
        4096,
        1000,
        true,
        "fakes",
    );
    assert_eq!(
        s.lines,
        [
            "grant",
            "to 1301c691 (laptop)",
            "for wireguard, tls, mls",
            "4096 bytes of pad",
            "until log entry 1000 (laptop)",
            "may delegate",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    let s = screens::grant(
        "1301c691",
        &["wireguard", "tls", "mls", "wifi", "other"],
        1,
        7,
        false,
        "fakes",
    );
    assert_eq!(s.lines[2], "for wireguard, tls, mls +2");
    assert_eq!(s.lines[3], "1 byte of pad");
    assert_eq!(s.lines[5], "no delegation");
    let s = screens::grant("1301c691", &[], 0, 7, false, "fakes");
    assert_eq!(s.lines[2], "for nothing");
}

// ---- the Serve path ----

fn send(laptop: &mut FakeLink, seq: u32, req: &Request) {
    let payload = req.encode();
    laptop.send(&frame::encode(&Frame {
        kind: KIND_REQUEST,
        seq,
        payload: &payload,
    }));
}

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
        let (f, used) = frame::decode_max(&bytes[at..], 1 << 16).expect("a whole frame");
        assert_eq!(f.kind, KIND_RESPONSE);
        out.push((f.seq, Response::decode(f.payload).expect("a response")));
        at += used;
    }
    out
}

fn press(badge: &mut Badge<FakeBoard>, gesture: Press) {
    badge.board().presses.push_back(gesture);
    badge.idle_step();
}

/// The lines of the screen before the last: the prompt, when the last is the page again.
fn prompt(badge: &mut Badge<FakeBoard>) -> Vec<String> {
    let screens = &badge.board().screens;
    screens[screens.len() - 2].clone()
}

/// A badge serving its down link whose circle's root is `ROOT`: with `chip`, the seat key and
/// root are in a SoftChip in slot 0; without, both are in the circle record.
fn serving(chip: bool) -> (Badge<FakeBoard>, FakeLink, CircleId, SeatPublic) {
    let (mut board, _, _) = FakeBoard::with_circle();
    let id = board.circle.as_ref().unwrap().circle_id;
    let public = if chip {
        let mut c = SoftChip::with_seed([9; 32]);
        let p = c
            .generate(KeyId::new(KeyKind::Seat, 0), Curve::P256)
            .unwrap();
        c.put_hmac_key(KeyId::new(KeyKind::Root, 0), &ROOT).unwrap();
        let mut record = ChipRecord::default();
        record.insert(SlotEntry {
            slot: 0,
            circle_id: id,
            has_seat: true,
            has_root: true,
            has_epoch: false,
        });
        board.chip_record = Some(record.encode());
        board.chip = Some(c);
        let circle = board.circle.as_mut().unwrap();
        circle.chip_slot = Some(0);
        circle.seat_key = [0; 32];
        SeatPublic::parse(&p).unwrap()
    } else {
        board.circle.as_mut().unwrap().root = Some(ROOT);
        SeatKey::from_seed(&[7; 32]).unwrap().public()
    };
    let (mut laptop, down) = cable();
    board.links = Some((down, FakeLink::open()));
    let mut badge = Badge::new(board);
    badge.boot();
    badge.idle_step();
    send(&mut laptop, 900, &Request::Identify);
    badge.idle_step();
    assert!(badge.board().shows("a laptop"), "{:?}", badge.board().last());
    press(&mut badge, Press::Both);
    let got = responses(&mut laptop);
    assert!(matches!(got.as_slice(), [(900, Response::Identity { .. })]), "{got:?}");
    (badge, laptop, id, public)
}

/// Send `op`, step so the prompt is drawn, answer with `gesture`, and return the response.
fn ask(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    op: ChipRequest,
    gesture: Press,
) -> Response {
    send(laptop, 7, &Request::Chip { circle: id, op });
    badge.idle_step();
    assert!(responses(laptop).is_empty(), "answered before asking");
    press(badge, gesture);
    match responses(laptop).as_slice() {
        [(7, r)] => r.clone(),
        other => panic!("not one answer: {other:?}"),
    }
}

/// Send `op` and step once: the answer must come back without a prompt.
fn ask_now(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    op: ChipRequest,
) -> Response {
    let before = badge.board().screens.len();
    send(laptop, 8, &Request::Chip { circle: id, op });
    badge.idle_step();
    assert_eq!(badge.board().screens.len(), before, "a screen was drawn");
    match responses(laptop).as_slice() {
        [(8, r)] => r.clone(),
        other => panic!("not one answer: {other:?}"),
    }
}

fn signature_of(r: Response) -> Vec<u8> {
    match r {
        Response::Chip {
            result: ChipResponse::Signature { signature },
        } => signature,
        other => panic!("not a signature: {other:?}"),
    }
}

fn refused(code: ErrorCode) -> Response {
    Response::Error { code }
}

fn my_card(id: CircleId) -> Card {
    Card {
        circle_id: id,
        seat: 1,
        name: "Ada Lovelace".into(),
        handles: vec![
            ("email".into(), "ada@example.com".into()),
            ("github".into(), "ada".into()),
        ],
        endpoints: vec![(PURPOSE_SSH, "ada.example.com".into())],
        log: 3,
    }
}

fn root_grant(id: CircleId) -> Grant {
    Grant {
        circle_id: id,
        grantor: Grantor::Seat(1),
        grantee: SeatKey::from_seed(&[0x44; 32]).unwrap().public(),
        purposes: vec![0, 1, 2, 3],
        budget: 4096,
        not_after: 1000,
        depth: 1,
        parent: None,
        nonce: [0x11; 16],
    }
}

#[test]
fn serve_card_and_grant_sign() {
    for chip in [true, false] {
        let (mut badge, mut laptop, id, public) = serving(chip);

        // A card: signed by the seat key, under kid [seat], and kept for the menu.
        let c = my_card(id);
        let r = ask(
            &mut badge,
            &mut laptop,
            id,
            ChipRequest::Card {
                card: c.encode().unwrap(),
            },
            Press::Both,
        );
        assert_eq!(
            prompt(&mut badge),
            [
                "sign your card",
                "Ada Lovelace",
                "2 handles, 1 endpoint",
                "fakes",
                "refuse  A+B  allow  refuse"
            ]
        );
        let signed = signature_of(r);
        assert_eq!(cose::kid1(&signed).unwrap(), [1]);
        let payload = cose::verify1(&public, &signed, None, card::DOMAIN).unwrap();
        assert_eq!(Card::decode(&payload).unwrap(), c);
        assert_eq!(badge.signed_card(&id), Some(signed.as_slice()));

        // The card seal key: HMAC-SHA256(root, "seal/card" || circle_id).
        let r = ask(
            &mut badge,
            &mut laptop,
            id,
            ChipRequest::SealKey {
                label: "seal/card".into(),
            },
            Press::Both,
        );
        assert_eq!(
            prompt(&mut badge),
            [
                "release a key",
                "key for seal/card",
                "a laptop asks for it",
                "fakes",
                "refuse  A+B  allow  refuse"
            ]
        );
        let key = cahoots_core::root::seal_key(&ROOT, b"seal/card", &id);
        assert_eq!(
            r,
            Response::Chip {
                result: ChipResponse::Key { key }
            }
        );
        // The card seals and opens under it.
        let sealed = card::seal(&signed, &key).unwrap();
        assert_eq!(card::open(&sealed, &key).unwrap(), signed);

        // A version 2 grant.
        let g = root_grant(id);
        let r = ask(
            &mut badge,
            &mut laptop,
            id,
            ChipRequest::GrantSign {
                grant: g.encode_v2(),
            },
            Press::Both,
        );
        let short = &hex(&key_id(&g.grantee))[..8];
        assert_eq!(
            prompt(&mut badge),
            [
                "grant".to_string(),
                format!("to {short} (laptop)"),
                "for wireguard, tls, mls +1".into(),
                "4096 bytes of pad".into(),
                "until log entry 1000 (laptop)".into(),
                "may delegate".into(),
                "fakes".into(),
                "refuse  A+B  allow  refuse".into(),
            ]
        );
        let signed = signature_of(r);
        assert_eq!(cose::kid1(&signed).unwrap(), [1]);
        let payload = cose::verify1(&public, &signed, None, grant::DOMAIN).unwrap();
        assert_eq!(payload, g.encode_v2());
        assert_eq!(Grant::decode_any(&payload).unwrap(), (g.clone(), 2));

        // A version 1 grant is signed too, as the same grant.
        let r = ask(
            &mut badge,
            &mut laptop,
            id,
            ChipRequest::GrantSign { grant: g.encode() },
            Press::Both,
        );
        let payload = cose::verify1(&public, &signature_of(r), None, grant::DOMAIN).unwrap();
        assert_eq!(payload, g.encode());

        // A single press refuses each.
        for op in [
            ChipRequest::Card {
                card: c.encode().unwrap(),
            },
            ChipRequest::GrantSign {
                grant: g.encode_v2(),
            },
            ChipRequest::SealKey {
                label: "seal/card".into(),
            },
        ] {
            for gesture in [Press::Left, Press::Right] {
                let r = ask(&mut badge, &mut laptop, id, op.clone(), gesture);
                assert_eq!(r, refused(ErrorCode::Denied));
            }
        }
    }
}

#[test]
fn what_the_badge_cannot_describe_is_refused_without_asking() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let mut ops = vec![
        ChipRequest::Card { card: vec![1] },
        ChipRequest::GrantSign { grant: vec![1] },
    ];
    // A card for another circle or another seat.
    let mut other = my_card(id);
    other.circle_id = [0xee; 16];
    ops.push(ChipRequest::Card {
        card: other.encode().unwrap(),
    });
    let mut other = my_card(id);
    other.seat = 2;
    ops.push(ChipRequest::Card {
        card: other.encode().unwrap(),
    });
    // A grant for another circle, from another seat, or delegated: the badge signs root
    // grants of its own seat only.
    let mut g = root_grant(id);
    g.circle_id = [0xee; 16];
    ops.push(ChipRequest::GrantSign {
        grant: g.encode_v2(),
    });
    let mut g = root_grant(id);
    g.grantor = Grantor::Seat(2);
    ops.push(ChipRequest::GrantSign {
        grant: g.encode_v2(),
    });
    let mut g = root_grant(id);
    g.grantor = Grantor::Key(SeatKey::from_seed(&[0x45; 32]).unwrap().public());
    g.parent = Some([1; 32]);
    ops.push(ChipRequest::GrantSign {
        grant: g.encode_v2(),
    });
    // Labels the badge cannot name on its screen.
    for label in ["", "seal/\ncard", "seal/card\u{e9}", &"x".repeat(33)] {
        ops.push(ChipRequest::SealKey {
            label: label.into(),
        });
    }
    for op in ops {
        let r = ask_now(&mut badge, &mut laptop, id, op.clone());
        assert_eq!(r, refused(ErrorCode::Unparseable), "{op:?}");
    }
    // A circle the badge does not hold.
    for op in [
        ChipRequest::Card {
            card: my_card(id).encode().unwrap(),
        },
        ChipRequest::SealKey {
            label: "seal/card".into(),
        },
    ] {
        let r = ask_now(&mut badge, &mut laptop, [0xee; 16], op);
        assert_eq!(r, refused(ErrorCode::UnknownCircle));
    }
    assert_eq!(badge.signed_card(&id), None);
}

#[test]
fn a_circle_without_a_root_has_no_seal_key() {
    // A software circle kept before roots were: no root in its record.
    let (mut badge, mut laptop, id, _) = serving(false);
    let mut circle = badge.circles()[0].clone();
    circle.root = None;
    let apps_circles = [circle];
    let mut apps = cahoots_badge::apps::BadgeApps::new(&apps_circles, None);
    use cahoots_custodian::ChipApps;
    assert_eq!(apps.seal_key(&id, "seal/card"), Err(ErrorCode::NoChip));
    // And on a chip badge whose root is missing from the chip: a chip error, before asking.
    let (mut badge2, mut laptop2, id2, _) = serving(true);
    badge2
        .board()
        .chip
        .as_mut()
        .unwrap()
        .delete(KeyId::new(KeyKind::Root, 0))
        .unwrap();
    let r = ask_now(
        &mut badge2,
        &mut laptop2,
        id2,
        ChipRequest::SealKey {
            label: "seal/card".into(),
        },
    );
    assert_eq!(r, refused(ErrorCode::ChipError));
    let _ = (&mut badge, &mut laptop);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A request longer than 4 KiB that is not an ssh signature is refused before the holder is
/// asked, and so is a card over the card length cap that still fits in a request.
#[test]
fn a_long_request_is_refused_before_asking() {
    use cahoots_custodian::wire::MAX_SMALL_REQUEST_LEN;
    let (mut badge, mut laptop, id, _) = serving(true);
    for op in [
        ChipRequest::Card {
            card: vec![1; MAX_SMALL_REQUEST_LEN],
        },
        ChipRequest::GrantSign {
            grant: vec![1; MAX_SMALL_REQUEST_LEN],
        },
        ChipRequest::SealKey {
            label: "x".repeat(MAX_SMALL_REQUEST_LEN),
        },
    ] {
        let r = ask_now(&mut badge, &mut laptop, id, op.clone());
        assert_eq!(r, refused(ErrorCode::Malformed), "{op:?}");
    }

    // A card of about 4050 bytes: the request carrying it is under 4 KiB and every field is
    // within its limit, but the card is over MAX_CARD_LEN, so it is not a card the badge signs.
    let raw = |n: u8| {
        let mut card = vec![0xa7, 0x00, 0x01, 0x01, 0x50];
        card.extend_from_slice(&id);
        card.extend_from_slice(&[0x02, 0x01, 0x03, 0x63]);
        card.extend_from_slice(b"Ada");
        card.extend_from_slice(&[0x04, 0x80 + n]);
        for i in 0..n {
            card.extend_from_slice(&[0x82, 0x62, b'k', b'a' + i, 0x78, 245]);
            card.extend(std::iter::repeat_n(b'v', 245));
        }
        card.extend_from_slice(&[0x05, 0x80, 0x06, 0x03]);
        card
    };
    // With one handle fewer it is a card, so the form is right and only the length is wrong.
    assert_eq!(Card::decode(&raw(15)).unwrap().handles.len(), 15);
    let card = raw(16);
    assert!(card.len() > card::MAX_CARD_LEN, "{}", card.len());
    let req = Request::Chip {
        circle: id,
        op: ChipRequest::Card { card: card.clone() },
    };
    assert!(req.encode().len() <= MAX_SMALL_REQUEST_LEN);
    let r = ask_now(&mut badge, &mut laptop, id, ChipRequest::Card { card });
    assert_eq!(r, refused(ErrorCode::Unparseable));
    assert_eq!(badge.signed_card(&id), None);
}
