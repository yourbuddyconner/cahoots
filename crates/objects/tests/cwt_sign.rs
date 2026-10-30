//! The two kinds of grant claims: a pad grant as before, and a sign grant.

mod common;

use cahoots_objects::cwt::{audience, seat_of, Claims, Detail, Kind, SignDetail, SIGN_TYPE};
use cahoots_objects::ObjectError;
use common::*;

fn sign_detail() -> Detail {
    Detail {
        r#type: SIGN_TYPE.into(),
        actions: Some(vec!["git-commit".into()]),
        author: Some("Conner Swann <me@connerswann.me>".into()),
        locations: Some(vec!["github.com/yourbuddyconner/cahoots".into()]),
        agent: Some("claude on connerswann-mbp".into()),
        budget: Some(50),
        channel: None,
    }
}

fn sign_claims() -> Claims {
    let grantee = other_key(0).public();
    Claims {
        iss: cahoots_objects::cwt::issuer(&CIRCLE),
        sub: cahoots_objects::cwt::subject(&grantee),
        aud: Some(audience(&CIRCLE, 2)),
        exp: Some(1_764_374_400),
        cnf: grantee,
        cti: [0x33; 16],
        authorization_details: vec![sign_detail()],
        log_not_after: None,
        parent: None,
        depth: 0,
        grantor: cahoots_objects::grant::Grantor::Seat(1),
        base: Some(1 << 32),
    }
}

fn malformed(c: &Claims) {
    assert_eq!(Claims::decode(&c.encode()), Err(ObjectError::Malformed));
}

#[test]
fn a_sign_grant_round_trips() {
    let c = sign_claims();
    assert_eq!(Claims::decode(&c.encode()), Ok(c.clone()));
    let (circle, kind) = c.kind().unwrap();
    assert_eq!(circle, CIRCLE);
    assert_eq!(
        kind,
        Kind::Sign(SignDetail {
            actions: vec!["git-commit".into()],
            author: "Conner Swann <me@connerswann.me>".into(),
            locations: vec!["github.com/yourbuddyconner/cahoots".into()],
            agent: "claude on connerswann-mbp".into(),
            budget: 50,
        })
    );
    assert_eq!(seat_of(&audience(&CIRCLE, 2)), Some((CIRCLE, 2)));
}

#[test]
fn a_pad_grant_encodes_as_before() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../vectors/objects.json")).unwrap();
    let bytes = hex::decode(v["grant_v2_root"]["payload"].as_str().unwrap()).unwrap();
    let c = Claims::decode(&bytes).unwrap();
    assert_eq!(c.encode(), bytes);
    assert!(matches!(c.kind().unwrap().1, Kind::Pad { .. }));
}

#[test]
fn a_sign_grant_with_a_log_claim_is_malformed() {
    let mut c = sign_claims();
    c.log_not_after = Some(10);
    malformed(&c);
}

#[test]
fn a_pad_grant_without_a_log_claim_is_malformed() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../vectors/objects.json")).unwrap();
    let bytes = hex::decode(v["grant_v2_root"]["payload"].as_str().unwrap()).unwrap();
    let mut c = Claims::decode(&bytes).unwrap();
    c.log_not_after = None;
    malformed(&c);
}

#[test]
fn a_mixed_grant_is_malformed() {
    let mut c = sign_claims();
    c.authorization_details.push(Detail {
        r#type: "cahoots:pad".into(),
        budget: Some(4096),
        ..Default::default()
    });
    malformed(&c);
}

#[test]
fn a_sign_grant_with_two_details_is_malformed() {
    let mut c = sign_claims();
    c.authorization_details.push(sign_detail());
    malformed(&c);
}

#[test]
fn an_unknown_detail_field_is_malformed() {
    // {"foo": 1, "type": ...} inside a sign grant's details: hand-encode the claims.
    let good = sign_claims().encode();
    let mut e = minicbor::Encoder::new(Vec::new());
    e.map(2).unwrap().str("foo").unwrap().u8(1).unwrap();
    e.str("type").unwrap().str(SIGN_TYPE).unwrap();
    let foo = e.into_writer();
    // Swap the whole details array (key -65539) for one with the unknown field.
    let mut d = minicbor::Decoder::new(&good);
    let n = d.map().unwrap().unwrap();
    let mut out = minicbor::Encoder::new(Vec::new());
    out.map(n).unwrap();
    for _ in 0..n {
        let start = d.position();
        let key = i64::try_from(d.int().unwrap()).unwrap();
        let key_end = d.position();
        d.skip().unwrap();
        let end = d.position();
        if key == cahoots_objects::cwt::AUTHORIZATION_DETAILS {
            out.writer_mut().extend_from_slice(&good[start..key_end]);
            out.array(1).unwrap();
            out.writer_mut().extend_from_slice(&foo);
        } else {
            out.writer_mut().extend_from_slice(&good[start..end]);
        }
    }
    assert_eq!(
        Claims::decode(&out.into_writer()),
        Err(ObjectError::Malformed)
    );
}

#[test]
fn a_sign_grant_without_budget_is_malformed() {
    let mut c = sign_claims();
    c.authorization_details[0].budget = None;
    malformed(&c);
}

#[test]
fn an_aud_that_is_not_a_seat_is_malformed() {
    let mut c = sign_claims();
    c.aud = Some("cahoots:circle:nothing".into());
    malformed(&c);
    let mut c = sign_claims();
    c.aud = Some(audience(&[9; 16], 2));
    malformed(&c);
}

#[test]
fn a_sign_grant_without_a_base_is_malformed() {
    let mut c = sign_claims();
    c.base = None;
    malformed(&c);
}

#[test]
fn a_pad_grant_with_a_base_is_malformed() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../vectors/objects.json")).unwrap();
    let bytes = hex::decode(v["grant_v2_root"]["payload"].as_str().unwrap()).unwrap();
    let mut c = Claims::decode(&bytes).unwrap();
    c.base = Some(1 << 32);
    malformed(&c);
}

#[test]
fn the_base_is_the_last_claim_and_reads_back() {
    let c = sign_claims();
    let bytes = c.encode();
    // -65542 then 1 << 32: 3a 00010005 1b 0000000100000000.
    assert!(bytes.ends_with(&[0x3a, 0, 1, 0, 5, 0x1b, 0, 0, 0, 1, 0, 0, 0, 0]));
    assert_eq!(Claims::decode(&bytes).unwrap().base, Some(1 << 32));
}

#[test]
fn a_sign_detail_with_a_channel_field_is_refused() {
    let mut c = sign_claims();
    c.authorization_details[0].channel = Some("00112233445566778899aabbccddeeff".into());
    malformed(&c);
}
