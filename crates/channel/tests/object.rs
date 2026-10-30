//! The channel object.

mod common;

use cahoots_channel::object::{channel_id_for, ChannelObject, DOMAIN};
use cahoots_channel::ChannelError;
use cahoots_core::cose;
use common::*;
use minicbor::Encoder;
use sha2::{Digest, Sha256};

fn object() -> ChannelObject {
    let nonce = [9; 16];
    ChannelObject {
        circle_id: CIRCLE,
        channel_id: channel_id_for(&CIRCLE, &nonce),
        name: "town square".into(),
        created_log: 0,
        policy_id: [3; 32],
        nonce,
    }
}

#[test]
fn channel_object_round_trips_and_verifies_under_seat_1() {
    let obj = object();
    assert_eq!(ChannelObject::decode(&obj.encode()).unwrap(), obj);
    let signed = obj.sign(&seat_key(1), 1);
    assert_eq!(cose::kid1(&signed).unwrap(), vec![1]);
    assert_eq!(
        ChannelObject::verify(&signed, &manifest()).unwrap(),
        (1, obj)
    );
}

#[test]
fn channel_id_is_the_documented_hash() {
    let nonce = [9; 16];
    let mut h = Sha256::new();
    h.update(b"cahoots/channel-id/v1");
    h.update(CIRCLE);
    h.update(nonce);
    let full: [u8; 32] = h.finalize().into();
    assert_eq!(channel_id_for(&CIRCLE, &nonce)[..], full[..16]);
}

#[test]
fn channel_object_refuses_wrong_id_name_length_and_other_circle() {
    let m = manifest();

    let mut wrong_id = object();
    wrong_id.channel_id[0] ^= 1;
    let signed = wrong_id.sign(&seat_key(1), 1);
    assert_eq!(
        ChannelObject::verify(&signed, &m),
        Err(ChannelError::WrongChannel)
    );

    let mut long = object();
    long.name = "x".repeat(65);
    let signed = long.sign(&seat_key(1), 1);
    assert_eq!(
        ChannelObject::verify(&signed, &m),
        Err(ChannelError::Malformed)
    );
    let mut empty = object();
    empty.name = String::new();
    assert_eq!(
        ChannelObject::decode(&empty.encode()),
        Err(ChannelError::Malformed)
    );
    let mut max = object();
    max.name = "x".repeat(64);
    assert_eq!(ChannelObject::decode(&max.encode()).unwrap(), max);

    let signed = object().sign(&seat_key(1), 1);
    assert_eq!(
        ChannelObject::verify(&signed, &manifest_for([8; 16])),
        Err(ChannelError::WrongCircle)
    );

    // Seat 2's key under seat 1's kid.
    let forged = object().sign(&seat_key(2), 1);
    assert!(matches!(
        ChannelObject::verify(&forged, &m),
        Err(ChannelError::Cose(_))
    ));
}

#[test]
fn channel_object_refuses_non_canonical_bytes() {
    let obj = object();
    let mut e = Encoder::new(Vec::new());
    e.begin_map().unwrap();
    e.u8(0).unwrap().u8(1).unwrap();
    e.u8(1).unwrap().bytes(&obj.circle_id).unwrap();
    e.u8(2).unwrap().bytes(&obj.channel_id).unwrap();
    e.u8(3).unwrap().str(&obj.name).unwrap();
    e.u8(4).unwrap().u64(obj.created_log).unwrap();
    e.u8(5).unwrap().bytes(&obj.policy_id).unwrap();
    e.u8(6).unwrap().bytes(&obj.nonce).unwrap();
    e.end().unwrap();
    let indefinite = e.into_writer();
    assert_eq!(
        ChannelObject::decode(&indefinite),
        Err(ChannelError::NotCanonical)
    );
    let signed = cose::sign1(&seat_key(1), &[1], &indefinite, DOMAIN, false);
    assert_eq!(
        ChannelObject::verify(&signed, &manifest()),
        Err(ChannelError::NotCanonical)
    );

    // created_log 0 written as a one-byte-argument integer instead of the shortest form.
    let bytes = obj.encode();
    let window = [0x04, 0x00, 0x05, 0x58, 0x20];
    let at = bytes.windows(5).position(|w| w == window).unwrap();
    let mut long_int = bytes.clone();
    long_int.splice(at + 1..at + 2, [0x18, 0x00]);
    assert_eq!(
        ChannelObject::decode(&long_int),
        Err(ChannelError::NotCanonical)
    );

    // Trailing bytes.
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        ChannelObject::decode(&trailing),
        Err(ChannelError::Malformed)
    );
}
