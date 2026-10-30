//! Grants as CBOR Web Tokens (version 2) beside version 1: the same meaning, the same chain
//! rules, and links of either version in one chain.

mod common;

use cahoots_objects::cwt::{self, Claims, Detail};
use cahoots_objects::grant::{key_id, verify_chain, Grant, GrantError, Grantor};
use cahoots_objects::ObjectError;
use common::*;
use sha2::{Digest, Sha256};

fn root() -> Grant {
    Grant {
        circle_id: CIRCLE,
        grantor: Grantor::Seat(1),
        grantee: other_key(0).public(),
        purposes: vec![0, 2, 5],
        budget: 4096,
        not_after: 1000,
        depth: 2,
        parent: None,
        nonce: [0x11; 16],
    }
}

fn child(parent_signed: &[u8]) -> Grant {
    Grant {
        circle_id: CIRCLE,
        grantor: Grantor::Key(other_key(0).public()),
        grantee: other_key(1).public(),
        purposes: vec![2, 5],
        budget: 1024,
        not_after: 900,
        depth: 1,
        parent: Some(Sha256::digest(parent_signed).into()),
        nonce: [0x22; 16],
    }
}

fn sign_child_v1(g: &Grant) -> Vec<u8> {
    g.sign(&other_key(0), &key_id(&other_key(0).public()))
}

fn sign_child_v2(g: &Grant) -> Vec<u8> {
    g.sign_v2(&other_key(0), &key_id(&other_key(0).public()))
}

#[test]
fn grant_v2_round_trip_equals_v1_meaning() {
    for g in [root(), child(&[1, 2, 3])] {
        let claims = g.to_v2();
        let bytes = claims.encode();
        assert_eq!(Claims::decode(&bytes).unwrap(), claims);
        assert_eq!(Grant::from_v2(&claims).unwrap(), g);
        let (back, version) = Grant::decode_any(&bytes).unwrap();
        assert_eq!((back, version), (g.clone(), 2));
        assert_eq!(Grant::decode_any(&g.encode()).unwrap(), (g.clone(), 1));
    }
    let claims = root().to_v2();
    assert_eq!(claims.iss, format!("cahoots:circle:{}", "07".repeat(16)));
    assert_eq!(
        claims.sub,
        format!(
            "cahoots:key:{}",
            hex::encode(key_id(&other_key(0).public()))
        )
    );
    assert_eq!(claims.cnf, other_key(0).public());
    assert_eq!(claims.cti, [0x11; 16]);
    assert_eq!(claims.log_not_after, Some(1000));
    assert_eq!(
        claims.authorization_details,
        vec![
            Detail {
                r#type: "cahoots:wireguard".into(),
                budget: None,
                ..Default::default()
            },
            Detail {
                r#type: "cahoots:mls".into(),
                budget: None,
                ..Default::default()
            },
            Detail {
                r#type: "cahoots:other".into(),
                budget: None,
                ..Default::default()
            },
            Detail {
                r#type: "cahoots:pad".into(),
                budget: Some(4096),
                ..Default::default()
            },
        ]
    );
    // The same grant signed either way verifies to the same leaf.
    let v1 = root().sign(&seat_key(1), &[1]);
    let v2 = root().sign_v2(&seat_key(1), &[1]);
    assert_ne!(v1, v2);
    assert_eq!(verify_chain(&[&v1], &manifest()).unwrap(), root());
    assert_eq!(verify_chain(&[&v2], &manifest()).unwrap(), root());
}

#[test]
fn the_payload_uses_the_registered_claim_keys() {
    let bytes = root().to_v2().encode();
    let mut d = minicbor::Decoder::new(&bytes);
    let n = d.map().unwrap().unwrap();
    let mut keys = Vec::new();
    for _ in 0..n {
        keys.push(i64::try_from(d.int().unwrap()).unwrap());
        d.skip().unwrap();
    }
    assert_eq!(
        keys,
        [
            1,
            2,
            7,
            8,
            cwt::LOG_NOT_AFTER,
            cwt::AUTHORIZATION_DETAILS,
            cwt::DEPTH,
            cwt::GRANTOR
        ]
    );
    assert_eq!(
        [cwt::LOG_NOT_AFTER, cwt::PARENT, cwt::AUTHORIZATION_DETAILS],
        [-65537, -65538, -65539]
    );
    // cnf is {1: COSE_Key} with an EC2 P-256 key.
    let mut d = minicbor::Decoder::new(&bytes);
    d.map().unwrap();
    for _ in 0..3 {
        d.skip().unwrap();
        d.skip().unwrap();
    }
    assert_eq!(d.u8().unwrap(), 8);
    assert_eq!(d.map().unwrap(), Some(1));
    assert_eq!(d.u8().unwrap(), 1);
    assert_eq!(d.map().unwrap(), Some(4));
    assert_eq!((d.u8().unwrap(), d.u8().unwrap()), (1, 2), "kty EC2");
    assert_eq!((d.i8().unwrap(), d.u8().unwrap()), (-1, 1), "crv P-256");
}

#[test]
fn chain_mixes_v1_and_v2() {
    let m = manifest();
    // A version 1 root and a version 2 child.
    let r1 = root().sign(&seat_key(1), &[1]);
    let c2 = sign_child_v2(&child(&r1));
    assert_eq!(verify_chain(&[&r1, &c2], &m).unwrap(), child(&r1));
    // A version 2 root and a version 1 child.
    let r2 = root().sign_v2(&seat_key(1), &[1]);
    let c1 = sign_child_v1(&child(&r2));
    assert_eq!(verify_chain(&[&r2, &c1], &m).unwrap(), child(&r2));
    // The chain rules hold across versions: a wider child is refused.
    let mut wide = child(&r2);
    wide.budget = 8192;
    assert_eq!(
        verify_chain(&[&r2, &sign_child_v2(&wide)], &m),
        Err(GrantError::LargerBudget)
    );
}

#[test]
fn cnf_mismatch_fails_chain() {
    let m = manifest();
    // A v2 root whose cnf names other_key(2); the child is signed by other_key(0), naming it
    // as grantor. The chain names other_key(0) as the root's grantee nowhere the root agrees.
    let mut r = root();
    r.grantee = other_key(2).public();
    let r2 = r.sign_v2(&seat_key(1), &[1]);
    let c = sign_child_v2(&child(&r2));
    assert_eq!(verify_chain(&[&r2, &c], &m), Err(GrantError::WrongKid));
    // The same child under the kid the root names: the signature is not cnf's key.
    let c = child(&r2).sign_v2(&other_key(0), &key_id(&other_key(2).public()));
    assert_eq!(
        verify_chain(&[&r2, &c], &m),
        Err(GrantError::Cose(cahoots_core::cose::CoseError::Signature))
    );
    // A root whose sub names other_key(0) while cnf is other_key(2): not a grant at all.
    let mut claims = root().to_v2();
    claims.cnf = other_key(2).public();
    let payload = claims.encode();
    assert_eq!(Claims::decode(&payload), Err(ObjectError::Malformed));
    let signed = cahoots_core::cose::sign1(
        &seat_key(1),
        &[1],
        &payload,
        cahoots_objects::grant::DOMAIN,
        false,
    );
    assert_eq!(
        verify_chain(&[&signed], &m),
        Err(GrantError::Decode(ObjectError::Malformed))
    );
}

#[test]
fn claims_decode_refuses_what_is_not_canonical() {
    let bytes = root().to_v2().encode();
    for n in 0..bytes.len() {
        assert!(Claims::decode(&bytes[..n]).is_err());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Claims::decode(&longer).is_err());
    // An indefinite-length map.
    let mut indef = bytes.clone();
    indef[0] = 0xbf;
    indef.push(0xff);
    assert_eq!(Claims::decode(&indef), Err(ObjectError::NotCanonical));
    // A known purpose spelled as a number is not canonical.
    let mut c = root().to_v2();
    c.authorization_details[0].r#type = "cahoots:purpose-0".into();
    assert_eq!(Claims::decode(&c.encode()), Err(ObjectError::NotCanonical));
    // An unknown type, no pad detail, a pad detail without a budget: not a grant.
    let mut c = root().to_v2();
    c.authorization_details[0].r#type = "payment_initiation".into();
    assert!(Claims::decode(&c.encode()).is_err());
    let mut c = root().to_v2();
    c.authorization_details.pop();
    assert!(Claims::decode(&c.encode()).is_err());
    let mut c = root().to_v2();
    c.authorization_details.last_mut().unwrap().budget = None;
    assert!(Claims::decode(&c.encode()).is_err());
    // An issuer that is not a circle.
    let mut c = root().to_v2();
    c.iss = "https://example.com".into();
    assert!(Claims::decode(&c.encode()).is_err());
    // Garbage never panics.
    for b in [
        &[][..],
        &[0xa0],
        &[0xbf, 0xff],
        &[0xff; 40],
        &[0xa1, 0x3a, 0, 1, 0, 0, 0],
    ] {
        assert!(Claims::decode(b).is_err());
        assert!(Grant::decode_any(b).is_err());
    }
}
