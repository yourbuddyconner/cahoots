//! The sign grant, its evaluator and the agent's proof of possession.

mod common;

use cahoots_core::seat::SeatKey;
use cahoots_objects::authz::*;
use cahoots_objects::cwt::Claims;
use cahoots_objects::grant::key_id;
use cahoots_objects::ObjectError;
use common::*;

fn seat_key() -> SeatKey {
    SeatKey::from_seed(&[7; 32]).unwrap()
}

fn grant() -> SignGrant {
    SignGrant {
        circle_id: CIRCLE,
        seat: 1,
        grantee: other_key(0).public(),
        cti: [0x33; 16],
        actions: vec![Action::GitCommit],
        author: "Conner Swann <me@connerswann.me>".into(),
        locations: vec!["github.com/a/b".into()],
        agent: "claude on connerswann-mbp".into(),
        budget: 2,
        exp: Some(1000),
        base: 1 << 32,
    }
}

fn request() -> EvalRequest {
    EvalRequest {
        subject: Subject {
            key_id: key_id(&other_key(0).public()),
        },
        action: Action::GitCommit,
        resource: Resource {
            circle_id: CIRCLE,
            location: Some("https://github.com/a/b.git".into()),
        },
        context: Context {
            author: "Conner Swann <me@connerswann.me>".into(),
            time: Some(500),
            used: 0,
            revoked: false,
        },
    }
}

fn reason(r: &EvalRequest) -> Reason {
    evaluate(&grant(), r).reason
}

#[test]
fn claims_round_trip() {
    let g = grant();
    let c = g.claims();
    assert_eq!(SignGrant::from_claims(&c), Ok(g.clone()));
    assert_eq!(Claims::decode(&c.encode()), Ok(c.clone()));
    let mut g2 = g;
    g2.exp = None;
    g2.locations = vec![];
    assert_eq!(SignGrant::from_claims(&g2.claims()), Ok(g2));
}

#[test]
fn sign_and_verify() {
    let key = seat_key();
    let g = grant();
    let signed = g.sign(&key, 1);
    assert_eq!(SignGrant::verify(&signed, &key.public(), &CIRCLE, 1), Ok(g));
    let other = SeatKey::from_seed(&[8; 32]).unwrap();
    assert!(matches!(
        SignGrant::verify(&signed, &other.public(), &CIRCLE, 1),
        Err(ObjectError::Cose(_))
    ));
    assert_eq!(
        SignGrant::verify(&signed, &key.public(), &[9; 16], 1),
        Err(ObjectError::Circle)
    );
    assert_eq!(
        SignGrant::verify(&signed, &key.public(), &CIRCLE, 2),
        Err(ObjectError::WrongKid)
    );
}

#[test]
fn evaluate_allows_and_counts_down() {
    let mut r = request();
    let d = evaluate(&grant(), &r);
    assert_eq!(
        (d.decision, d.reason, d.remaining),
        (true, Reason::Allowed, 1)
    );
    r.context.used = 1;
    let d = evaluate(&grant(), &r);
    assert_eq!(
        (d.decision, d.reason, d.remaining),
        (true, Reason::Allowed, 0)
    );
    r.context.used = 2;
    let d = evaluate(&grant(), &r);
    assert_eq!(
        (d.decision, d.reason, d.remaining),
        (false, Reason::Spent, 0)
    );
}

#[test]
fn revoked() {
    let mut r = request();
    r.context.revoked = true;
    assert_eq!(reason(&r), Reason::Revoked);
}

#[test]
fn wrong_key() {
    let mut r = request();
    r.subject.key_id = key_id(&other_key(1).public());
    assert_eq!(reason(&r), Reason::WrongKey);
}

#[test]
fn wrong_circle() {
    let mut r = request();
    r.resource.circle_id = [9; 16];
    assert_eq!(reason(&r), Reason::WrongCircle);
}

#[test]
fn wrong_action() {
    let mut r = request();
    r.action = Action::SshLogin;
    assert_eq!(reason(&r), Reason::WrongAction);
}

#[test]
fn wrong_author() {
    let mut r = request();
    r.context.author = "Someone Else <x@example.com>".into();
    assert_eq!(reason(&r), Reason::WrongAuthor);
}

#[test]
fn wrong_location() {
    let mut r = request();
    r.resource.location = Some("github.com/a/c".into());
    assert_eq!(reason(&r), Reason::WrongLocation);
    r.resource.location = None;
    assert_eq!(reason(&r), Reason::WrongLocation);
    let mut open = grant();
    open.locations = vec![];
    assert_eq!(evaluate(&open, &r).reason, Reason::Allowed);
}

#[test]
fn expired() {
    let mut r = request();
    r.context.time = Some(1000);
    assert_eq!(reason(&r), Reason::Expired);
    r.context.time = Some(999);
    assert_eq!(reason(&r), Reason::Allowed);
    r.context.time = None;
    assert_eq!(reason(&r), Reason::Allowed);
}

#[test]
fn location_matches_normalises() {
    for given in [
        "https://github.com/a/b.git",
        "git@github.com:a/b.git",
        "github.com/a/b",
        "https://GitHub.com/a/b/",
    ] {
        assert!(location_matches("github.com/a/b", given), "{given}");
    }
    assert!(!location_matches("github.com/a/b", "github.com/a/c"));
    assert!(!location_matches("github.com/a/b", "github.com/A/b"));
}

#[test]
fn action_names() {
    for a in [Action::GitCommit, Action::SshLogin, Action::File] {
        assert_eq!(Action::parse(a.name()), Some(a));
    }
    assert_eq!(Action::parse("nope"), None);
}

#[test]
fn proof_round_trip_and_replay() {
    let key = other_key(0);
    let (hash, nonce) = ([1u8; 32], [2u8; 16]);
    let proof = proof_sign(&key, &hash, &nonce);
    assert!(proof_verify(&proof, &key.public(), &hash, &nonce));
    assert!(!proof_verify(&proof, &key.public(), &hash, &[3; 16]));
    assert!(!proof_verify(&proof, &key.public(), &[4; 32], &nonce));
    assert!(!proof_verify(&proof, &other_key(1).public(), &hash, &nonce));
    assert!(!proof_verify(b"junk", &key.public(), &hash, &nonce));
}
