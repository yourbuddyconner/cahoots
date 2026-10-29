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
    let bytes = c.encode();
    assert_eq!(Card::decode(&bytes).unwrap(), c);
    let signed = c.sign(&seat_key(1), &[1]);
    assert_eq!(Card::verify(&signed, &manifest()).unwrap(), c);
}

#[test]
fn card_seal_and_open() {
    let signed = card().sign(&seat_key(1), &[1]);
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
    let signed = c.sign(&seat_key(2), &[2]);
    assert_eq!(
        Card::verify(&signed, &manifest()),
        Err(ObjectError::WrongKid)
    );
    // Seat 2's key under kid [1]: the signature is not seat 1's.
    let signed = c.sign(&seat_key(2), &[1]);
    assert_eq!(
        Card::verify(&signed, &manifest()),
        Err(ObjectError::Cose(CoseError::Signature))
    );
    // A card of another circle.
    let mut other = card();
    other.circle_id = [8; 16];
    let signed = other.sign(&seat_key(1), &[1]);
    assert_eq!(Card::verify(&signed, &manifest()), Err(ObjectError::Circle));
}

#[test]
fn card_decode_refuses_what_is_not_a_card() {
    let bytes = card().encode();
    for n in 0..bytes.len() {
        assert!(Card::decode(&bytes[..n]).is_err());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Card::decode(&longer).is_err());
    // A purpose past the known ones, an empty name, too many handles.
    let mut c = card();
    c.endpoints.push((200, "x".into()));
    assert_eq!(Card::decode(&c.encode()), Err(ObjectError::Malformed));
    let mut c = card();
    c.name = String::new();
    assert_eq!(Card::decode(&c.encode()), Err(ObjectError::Malformed));
    let mut c = card();
    c.handles = (0..17).map(|i| ("x".into(), format!("{i}"))).collect();
    assert_eq!(Card::decode(&c.encode()), Err(ObjectError::Malformed));
    // Garbage never panics.
    for b in [&[][..], &[0xa0], &[0xbf, 0xff], &[0xff; 40]] {
        assert!(Card::decode(b).is_err());
    }
}
