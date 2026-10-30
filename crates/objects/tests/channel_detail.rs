//! The `cahoots:channel` grant detail.

mod common;

use cahoots_objects::cwt::{Claims, Detail, Kind, CHANNEL_TYPE};
use cahoots_objects::grant::Grantor;
use cahoots_objects::ObjectError;
use common::*;

const CHANNEL_HEX: &str = "00112233445566778899aabbccddeeff";

fn detail() -> Detail {
    Detail {
        r#type: CHANNEL_TYPE.into(),
        budget: Some(0),
        actions: Some(vec!["read".into(), "post".into()]),
        channel: Some(CHANNEL_HEX.into()),
        ..Default::default()
    }
}

fn claims_with(details: Vec<Detail>) -> Claims {
    let grantee = other_key(0).public();
    Claims {
        iss: cahoots_objects::cwt::issuer(&CIRCLE),
        sub: cahoots_objects::cwt::subject(&grantee),
        aud: None,
        exp: None,
        cnf: grantee,
        cti: [7; 16],
        authorization_details: details,
        log_not_after: Some(500),
        parent: None,
        depth: 1,
        grantor: Grantor::Seat(1),
        base: None,
    }
}

fn channel_claims() -> Claims {
    claims_with(vec![detail()])
}

fn malformed(c: &Claims) {
    assert_eq!(Claims::decode(&c.encode()), Err(ObjectError::Malformed));
}

#[test]
fn channel_detail_round_trips_and_reports_kind() {
    let c = channel_claims();
    let bytes = c.encode();
    assert_eq!(Claims::decode(&bytes).unwrap(), c);
    let (circle, kind) = c.kind().unwrap();
    assert_eq!(circle, CIRCLE);
    let mut channel = [0u8; 16];
    for (i, b) in channel.iter_mut().enumerate() {
        *b = (i as u8) * 0x11;
    }
    assert_eq!(
        kind,
        Kind::Channel {
            channel,
            actions: vec!["read".into(), "post".into()],
            budget: 0
        }
    );
}

#[test]
fn channel_detail_keys_are_canonical() {
    let bytes = channel_claims().encode();
    // Match each key with its CBOR text header, so the "cahoots:channel" type value is not a hit.
    let find = |key: &[u8]| {
        let mut needle = vec![0x60 + key.len() as u8];
        needle.extend_from_slice(key);
        bytes
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .unwrap()
    };
    let (t, b, a, ch) = (
        find(b"type"),
        find(b"budget"),
        find(b"actions"),
        find(b"channel"),
    );
    assert!(t < b && b < a && a < ch);
}

#[test]
fn channel_detail_projects_to_json() {
    let v = channel_claims().to_json();
    assert_eq!(
        v["authorization_details"][0]["channel"].as_str(),
        Some(CHANNEL_HEX)
    );
}

#[test]
fn channel_detail_refuses_bad_shapes() {
    let with = |f: &dyn Fn(&mut Detail)| {
        let mut d = detail();
        f(&mut d);
        claims_with(vec![d])
    };
    malformed(&with(&|d| d.actions = Some(vec![])));
    malformed(&with(&|d| {
        d.actions = Some(vec!["post".into(), "read".into()])
    }));
    malformed(&with(&|d| {
        d.actions = Some(vec!["read".into(), "read".into()])
    }));
    malformed(&with(&|d| d.actions = Some(vec!["delete".into()])));
    malformed(&with(&|d| d.actions = None));
    malformed(&with(&|d| d.channel = Some("00112233".into())));
    malformed(&with(&|d| d.channel = Some(CHANNEL_HEX.to_uppercase())));
    malformed(&with(&|d| d.channel = None));
    malformed(&with(&|d| d.budget = None));
    malformed(&with(&|d| d.author = Some("x".into())));
    malformed(&claims_with(vec![detail(), detail()]));
    let mut c = channel_claims();
    c.log_not_after = None;
    malformed(&c);
    let mut c = channel_claims();
    c.exp = Some(1);
    malformed(&c);
    let mut c = channel_claims();
    c.base = Some(1);
    malformed(&c);
    let mut c = channel_claims();
    c.aud = Some(cahoots_objects::cwt::audience(&CIRCLE, 2));
    malformed(&c);
}

#[test]
fn channel_field_is_refused_on_a_pad_detail() {
    let mut pad = Detail {
        r#type: cahoots_objects::cwt::PAD_TYPE.into(),
        budget: Some(100),
        ..Default::default()
    };
    assert!(Claims::decode(&claims_with(vec![pad.clone()]).encode()).is_ok());
    pad.channel = Some(CHANNEL_HEX.into());
    malformed(&claims_with(vec![pad]));
}
