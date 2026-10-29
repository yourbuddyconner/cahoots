//! Contact cards: a seat-signed profile, sealed for the circle under a key from its root.

mod common;

use cahoots_core::cose::CoseError;
use cahoots_objects::card::{self, Card, PURPOSE_CHAT, PURPOSE_SSH};
use cahoots_objects::ObjectError;
use common::*;

fn card() -> Card {
    Card {
        circle_id: CIRCLE,
        seat: 1,
        name: "Ada Lovelace".into(),
        handles: vec![
            ("email".into(), "ada@example.com".into()),
            ("github".into(), "ada".into()),
        ],
        endpoints: vec![
            (0, "203.0.113.7:51820".into()),
            (PURPOSE_SSH, "ada.example.com".into()),
            (PURPOSE_CHAT, "cahoots:chat:ada".into()),
        ],
        log: 42,
    }
}

#[test]
fn card_round_trip() {
    let c = card();
    let bytes = c.encode().unwrap();
    assert_eq!(Card::decode(&bytes).unwrap(), c);
    let signed = c.sign(&seat_key(1), &[1]).unwrap();
    assert_eq!(Card::verify(&signed, &manifest()).unwrap(), c);
}

#[test]
fn card_seal_and_open() {
    let signed = card().sign(&seat_key(1), &[1]).unwrap();
    let key = [0x5a; 32];
    let sealed = card::seal(&signed, &key).unwrap();
    assert_eq!(
        sealed.len(),
        12 + signed.len() + 16,
        "a nonce, the text, a tag"
    );
    assert_eq!(card::open(&sealed, &key).unwrap(), signed);
    // A fresh nonce every time.
    assert_ne!(card::seal(&signed, &key).unwrap(), sealed);
    // The wrong key, a flipped bit, a short input: refused, never a panic.
    assert_eq!(
        card::open(&sealed, &[0x5b; 32]),
        Err(ObjectError::Malformed)
    );
    let mut bad = sealed.clone();
    bad[20] ^= 1;
    assert_eq!(card::open(&bad, &key), Err(ObjectError::Malformed));
    for n in 0..30 {
        assert!(card::open(&sealed[..n], &key).is_err());
    }
}

#[test]
fn card_verify_rejects_wrong_seat() {
    let c = card();
    // Seat 2's key signing a card that names seat 1, under kid [2].
    let signed = c.sign(&seat_key(2), &[2]).unwrap();
    assert_eq!(
        Card::verify(&signed, &manifest()),
        Err(ObjectError::WrongKid)
    );
    // Seat 2's key under kid [1]: the signature is not seat 1's.
    let signed = c.sign(&seat_key(2), &[1]).unwrap();
    assert_eq!(
        Card::verify(&signed, &manifest()),
        Err(ObjectError::Cose(CoseError::Signature))
    );
    // A card of another circle.
    let mut other = card();
    other.circle_id = [8; 16];
    let signed = other.sign(&seat_key(1), &[1]).unwrap();
    assert_eq!(Card::verify(&signed, &manifest()), Err(ObjectError::Circle));
}

#[test]
fn card_decode_refuses_what_is_not_a_card() {
    let bytes = card().encode().unwrap();
    for n in 0..bytes.len() {
        assert!(Card::decode(&bytes[..n]).is_err());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Card::decode(&longer).is_err());
    // A purpose past the known ones, an empty name, too many handles.
    let mut c = card();
    c.endpoints.push((200, "x".into()));
    assert_eq!(
        Card::decode(&c.encode().unwrap()),
        Err(ObjectError::Malformed)
    );
    let mut c = card();
    c.name = String::new();
    assert_eq!(
        Card::decode(&c.encode().unwrap()),
        Err(ObjectError::Malformed)
    );
    let mut c = card();
    c.handles = (0..17).map(|i| ("x".into(), format!("{i}"))).collect();
    assert_eq!(
        Card::decode(&c.encode().unwrap()),
        Err(ObjectError::Malformed)
    );
    // Garbage never panics.
    for b in [&[][..], &[0xa0], &[0xbf, 0xff], &[0xff; 40]] {
        assert!(Card::decode(b).is_err());
    }
}

/// A card's CBOR built by hand, with no length cap: version 1, `circle_id`, `seat`, `name`,
/// `handles`, no endpoints, log 3.
fn raw_card(circle_id: &[u8; 16], seat: u8, name: &str, handles: &[(String, String)]) -> Vec<u8> {
    fn text(out: &mut Vec<u8>, s: &str) {
        match s.len() {
            n if n < 24 => out.push(0x60 + n as u8),
            n if n < 256 => out.extend_from_slice(&[0x78, n as u8]),
            n => out.extend_from_slice(&[0x79, (n >> 8) as u8, n as u8]),
        }
        out.extend_from_slice(s.as_bytes());
    }
    let mut out = vec![0xa7, 0x00, 0x01, 0x01, 0x50];
    out.extend_from_slice(circle_id);
    out.extend_from_slice(&[0x02, seat, 0x03]);
    text(&mut out, name);
    out.extend_from_slice(&[0x04, 0x80 + handles.len() as u8]);
    for (k, v) in handles {
        out.push(0x82);
        text(&mut out, k);
        text(&mut out, v);
    }
    out.extend_from_slice(&[0x05, 0x80, 0x06, 0x03]);
    out
}

/// Sixteen handles with values of `len` bytes, each within the per-field limits.
fn handles(len: usize) -> Vec<(String, String)> {
    (0..16)
        .map(|i| (format!("k{i}"), "v".repeat(len)))
        .collect()
}

#[test]
fn a_card_over_the_length_cap_neither_encodes_nor_decodes() {
    // The hand builder agrees with encode on a small card.
    let small = Card {
        circle_id: CIRCLE,
        seat: 1,
        name: "Ada".into(),
        handles: handles(10),
        endpoints: vec![],
        log: 3,
    };
    assert_eq!(
        small.encode().unwrap(),
        raw_card(&CIRCLE, 1, "Ada", &small.handles)
    );

    // Every field within its limit, the whole over MAX_CARD_LEN.
    let big = Card {
        handles: handles(256),
        ..small.clone()
    };
    let bytes = raw_card(&CIRCLE, 1, "Ada", &big.handles);
    assert!(bytes.len() > card::MAX_CARD_LEN);
    assert_eq!(big.encode(), Err(ObjectError::Malformed));
    assert_eq!(big.sign(&seat_key(1), &[1]), Err(ObjectError::Malformed));
    assert_eq!(Card::decode(&bytes), Err(ObjectError::Malformed));

    // The longest card that fits encodes and decodes.
    let mut len = 256;
    let fits = loop {
        let c = Card {
            handles: handles(len),
            ..small.clone()
        };
        if let Ok(b) = c.encode() {
            break (c, b);
        }
        len -= 1;
    };
    assert!(fits.1.len() <= card::MAX_CARD_LEN);
    assert_eq!(Card::decode(&fits.1).unwrap(), fits.0);
}
