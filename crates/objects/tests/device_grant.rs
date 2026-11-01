//! The device grant: a seat lets a paired phone read, post and say hello in one circle.

mod common;

use cahoots_objects::cwt::{Claims, Detail, Kind, DEVICE_ACTIONS, DEVICE_TYPE};
use cahoots_objects::grant::{
    issue_device_grant, sign_device_grant, verify_chain, verify_device_grant, GrantError,
    Grantor, DOMAIN,
};
use cahoots_objects::ObjectError;
use common::*;

const NONCE: [u8; 16] = [9; 16];

fn device_claims() -> Claims {
    issue_device_grant(&CIRCLE, 1, other_key(0).public(), NONCE)
}

fn all_actions() -> Vec<String> {
    DEVICE_ACTIONS.iter().map(|a| String::from(*a)).collect()
}

fn malformed(c: &Claims) {
    assert_eq!(Claims::decode(&c.encode()), Err(ObjectError::Malformed), "{c:?}");
}

#[test]
fn a_device_grant_round_trips_and_reads_as_a_device_kind() {
    let c = device_claims();
    assert_eq!(c.grantor, Grantor::Seat(1));
    assert_eq!(c.cnf, other_key(0).public());
    assert_eq!(c.cti, NONCE);
    assert_eq!((c.aud.as_ref(), c.exp, c.log_not_after, c.base, c.parent), (None, None, None, None, None));
    assert_eq!(c.depth, 0);
    assert_eq!(
        c.authorization_details,
        vec![Detail {
            r#type: DEVICE_TYPE.into(),
            actions: Some(all_actions()),
            ..Default::default()
        }]
    );
    let bytes = c.encode();
    assert_eq!(Claims::decode(&bytes).unwrap(), c);
    assert_eq!(
        c.kind().unwrap(),
        (CIRCLE, Kind::Device { circle: CIRCLE, actions: all_actions() })
    );
    // The detail is `{"type": "cahoots:device", "actions": [...]}`, shorter key first.
    let detail = [
        &[0xa2, 0x64][..],
        b"type",
        &[0x6e],
        b"cahoots:device",
        &[0x67],
        b"actions",
        &[0x83, 0x64],
        b"read",
        &[0x64],
        b"post",
        &[0x65],
        b"hello",
    ]
    .concat();
    assert!(bytes.windows(detail.len()).any(|w| w == detail.as_slice()));
    // Encoding is deterministic.
    assert_eq!(device_claims().encode(), bytes);
}

#[test]
fn a_device_grant_with_fewer_actions_in_order_is_a_device_kind() {
    let mut c = device_claims();
    c.authorization_details[0].actions = Some(vec!["read".into(), "hello".into()]);
    assert_eq!(Claims::decode(&c.encode()).unwrap(), c);
}

#[test]
fn a_device_grant_refuses_unknown_repeated_or_reordered_actions() {
    for actions in [
        vec!["read", "post", "manage"],
        vec!["read", "admit"],
        vec!["post", "read"],
        vec!["read", "read"],
        vec![],
    ] {
        let mut c = device_claims();
        c.authorization_details[0].actions = Some(actions.into_iter().map(String::from).collect());
        malformed(&c);
    }
}

#[test]
fn a_device_grant_refuses_any_other_claim_or_field() {
    let mut c = device_claims();
    c.log_not_after = Some(5);
    malformed(&c);
    let mut c = device_claims();
    c.aud = Some(cahoots_objects::cwt::audience(&CIRCLE, 1));
    malformed(&c);
    let mut c = device_claims();
    c.exp = Some(1);
    malformed(&c);
    let mut c = device_claims();
    c.base = Some(1);
    malformed(&c);
    let mut c = device_claims();
    c.depth = 1;
    malformed(&c);
    let mut c = device_claims();
    c.parent = Some([1; 32]);
    malformed(&c);
    let mut c = device_claims();
    c.grantor = Grantor::Key(other_key(1).public());
    malformed(&c);
    let mut c = device_claims();
    c.authorization_details[0].budget = Some(0);
    malformed(&c);
    let mut c = device_claims();
    c.authorization_details[0].channel = Some("00112233445566778899aabbccddeeff".into());
    malformed(&c);
    let mut c = device_claims();
    c.authorization_details[0].actions = None;
    malformed(&c);
    let mut c = device_claims();
    let extra = c.authorization_details[0].clone();
    c.authorization_details.push(extra);
    malformed(&c);
}

#[test]
fn a_signed_device_grant_verifies_against_the_manifest() {
    let signed = sign_device_grant(&device_claims(), &seat_key(1), 1);
    let (seat, grantee, actions) = verify_device_grant(&signed, &manifest()).unwrap();
    assert_eq!(seat, 1);
    assert_eq!(grantee, other_key(0).public());
    assert_eq!(actions, all_actions());
}

#[test]
fn a_device_grant_is_refused_when_the_signer_or_circle_is_wrong() {
    // Signed by another seat's key under kid 1.
    let signed = sign_device_grant(&device_claims(), &seat_key(2), 1);
    assert!(matches!(verify_device_grant(&signed, &manifest()), Err(GrantError::Cose(_))));
    // Signed by seat 2 under its own kid, but naming seat 1 as the grantor.
    let signed = sign_device_grant(&device_claims(), &seat_key(2), 2);
    assert_eq!(verify_device_grant(&signed, &manifest()), Err(GrantError::WrongSigner));
    // A seat the manifest does not have.
    let c = issue_device_grant(&CIRCLE, 5, other_key(0).public(), NONCE);
    let signed = sign_device_grant(&c, &seat_key(1), 5);
    assert_eq!(verify_device_grant(&signed, &manifest()), Err(GrantError::NotInRoster(5)));
    // Another circle.
    let c = issue_device_grant(&[8; 16], 1, other_key(0).public(), NONCE);
    let signed = sign_device_grant(&c, &seat_key(1), 1);
    assert_eq!(verify_device_grant(&signed, &manifest()), Err(GrantError::Circle));
    // A pad grant is not a device grant.
    let pad = cahoots_objects::grant::Grant {
        circle_id: CIRCLE,
        grantor: Grantor::Seat(1),
        grantee: other_key(0).public(),
        purposes: vec![0],
        budget: 32,
        not_after: 10,
        depth: 0,
        parent: None,
        nonce: NONCE,
    };
    let signed = pad.sign_v2(&seat_key(1), &[1]);
    assert_eq!(
        verify_device_grant(&signed, &manifest()),
        Err(GrantError::Decode(ObjectError::Malformed))
    );
}

#[test]
fn a_device_grant_is_not_a_pad_grant_in_a_chain() {
    let signed = sign_device_grant(&device_claims(), &seat_key(1), 1);
    assert_eq!(
        verify_chain(&[&signed], &manifest()),
        Err(GrantError::Decode(ObjectError::Malformed))
    );
    // Signed under the grant domain like every grant.
    let payload = cahoots_core::cose::verify1(&seat_key(1).public(), &signed, None, DOMAIN).unwrap();
    assert_eq!(payload, device_claims().encode());
}
