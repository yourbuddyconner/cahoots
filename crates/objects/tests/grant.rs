mod common;

use cahoots_core::cose;
use cahoots_objects::grant::{verify_chain, Grant, GrantError, Grantor};
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

fn signed_root() -> Vec<u8> {
    root().sign(&seat_key(1), &[1])
}

/// A child of `parent_signed`, delegated by `other_key(0)` to `other_key(1)`.
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

fn kid_of(k: &cahoots_core::seat::SeatPublic) -> Vec<u8> {
    Sha256::digest(k.0).to_vec()
}

fn sign_child(g: &Grant) -> Vec<u8> {
    g.sign(&other_key(0), &kid_of(&other_key(0).public()))
}

#[test]
fn a_root_grant_signed_by_a_seat_verifies_against_the_manifest() {
    let signed = signed_root();
    assert_eq!(verify_chain(&[&signed], &manifest()).unwrap(), root());
}

#[test]
fn a_delegated_grant_narrows_and_verifies() {
    let r = signed_root();
    let c = sign_child(&child(&r));
    assert_eq!(verify_chain(&[&r, &c], &manifest()).unwrap(), child(&r));
}

fn refused(mutate: impl Fn(&mut Grant)) -> GrantError {
    let r = signed_root();
    let mut g = child(&r);
    mutate(&mut g);
    let c = sign_child(&g);
    verify_chain(&[&r, &c], &manifest()).unwrap_err()
}

#[test]
fn a_child_with_an_extra_purpose_is_refused() {
    assert_eq!(
        refused(|g| g.purposes = vec![1, 2, 5]),
        GrantError::ExtraPurpose
    );
}

#[test]
fn a_child_with_a_larger_budget_is_refused() {
    assert_eq!(refused(|g| g.budget = 4097), GrantError::LargerBudget);
}

#[test]
fn a_child_with_a_later_not_after_is_refused() {
    assert_eq!(refused(|g| g.not_after = 1001), GrantError::LaterNotAfter);
}

#[test]
fn a_child_with_the_wrong_depth_is_refused() {
    assert_eq!(refused(|g| g.depth = 2), GrantError::Depth);
    assert_eq!(refused(|g| g.depth = 0), GrantError::Depth);
}

#[test]
fn a_child_with_a_wrong_parent_is_refused() {
    assert_eq!(refused(|g| g.parent = Some([0; 32])), GrantError::BadParent);
    assert_eq!(refused(|g| g.parent = None), GrantError::BadParent);
}

#[test]
fn a_child_of_a_depth_zero_grant_is_refused() {
    let mut r = root();
    r.depth = 0;
    let r = r.sign(&seat_key(1), &[1]);
    let mut g = child(&r);
    g.depth = 0;
    let c = sign_child(&g);
    assert_eq!(
        verify_chain(&[&r, &c], &manifest()).unwrap_err(),
        GrantError::Depth
    );
}

#[test]
fn a_child_signed_by_someone_other_than_the_previous_grantee_is_refused() {
    let r = signed_root();
    let g = child(&r);
    // Signed by the wrong key under the right kid.
    let c = g.sign(&other_key(2), &kid_of(&other_key(0).public()));
    assert!(matches!(
        verify_chain(&[&r, &c], &manifest()).unwrap_err(),
        GrantError::Cose(_)
    ));
    // The wrong kid.
    let c = g.sign(&other_key(0), &[1]);
    assert_eq!(
        verify_chain(&[&r, &c], &manifest()).unwrap_err(),
        GrantError::WrongKid
    );
    // The grantor field names another key.
    let mut g2 = child(&r);
    g2.grantor = Grantor::Key(other_key(2).public());
    assert_eq!(
        verify_chain(&[&r, &sign_child(&g2)], &manifest()).unwrap_err(),
        GrantError::WrongSigner
    );
}

#[test]
fn a_child_in_another_circle_is_refused() {
    assert_eq!(refused(|g| g.circle_id = [8; 16]), GrantError::Circle);
}

#[test]
fn a_root_grant_from_a_key_not_in_the_roster_is_refused() {
    // A seat index past the roster.
    let mut g = root();
    g.grantor = Grantor::Seat(5);
    let signed = g.sign(&other_key(3), &[5]);
    assert_eq!(
        verify_chain(&[&signed], &manifest()).unwrap_err(),
        GrantError::NotInRoster(5)
    );
    // An outside key claiming seat 1.
    let signed = root().sign(&other_key(3), &[1]);
    assert!(matches!(
        verify_chain(&[&signed], &manifest()).unwrap_err(),
        GrantError::Cose(cose::CoseError::Signature)
    ));
    // A grantor that is a key, presented as a root.
    let mut g = root();
    g.grantor = Grantor::Key(other_key(3).public());
    let signed = g.sign(&seat_key(1), &[1]);
    assert_eq!(
        verify_chain(&[&signed], &manifest()).unwrap_err(),
        GrantError::WrongSigner
    );
    // A root that names a parent.
    let mut g = root();
    g.parent = Some([1; 32]);
    let signed = g.sign(&seat_key(1), &[1]);
    assert_eq!(
        verify_chain(&[&signed], &manifest()).unwrap_err(),
        GrantError::BadParent
    );
}

#[test]
fn an_empty_chain_is_refused() {
    assert_eq!(
        verify_chain(&[], &manifest()).unwrap_err(),
        GrantError::EmptyChain
    );
}

#[test]
fn a_root_in_another_circle_is_refused() {
    let mut g = root();
    g.circle_id = [9; 16];
    let signed = g.sign(&seat_key(1), &[1]);
    assert_eq!(
        verify_chain(&[&signed], &manifest()).unwrap_err(),
        GrantError::Circle
    );
}

#[test]
fn purposes_must_be_ascending_without_repeats() {
    for purposes in [vec![2, 1], vec![2, 2]] {
        let mut g = root();
        g.purposes = purposes;
        assert_eq!(
            Grant::decode(&g.encode()).unwrap_err(),
            ObjectError::Malformed
        );
    }
}

#[test]
fn encode_is_canonical() {
    let bytes = root().encode();
    assert_eq!(Grant::decode(&bytes).unwrap(), root());
    // Key 0 first, ascending integer keys, definite lengths: a 10-entry map.
    assert_eq!(bytes[0], 0xaa);
    assert_eq!(&bytes[1..3], &[0x00, 0x01]);
    // A non-shortest integer (budget 4096 as a 4-byte uint) is refused.
    let short = [0x05, 0x19, 0x10, 0x00];
    let long = [0x05, 0x1a, 0x00, 0x00, 0x10, 0x00];
    let at = bytes.windows(4).position(|w| w == short).unwrap();
    let mut bad = bytes[..at].to_vec();
    bad.extend_from_slice(&long);
    bad.extend_from_slice(&bytes[at + 4..]);
    assert_eq!(Grant::decode(&bad).unwrap_err(), ObjectError::NotCanonical);
    // A version other than 1.
    let mut v2 = bytes.clone();
    v2[2] = 0x02;
    assert_eq!(Grant::decode(&v2).unwrap_err(), ObjectError::Version);
    // Trailing bytes.
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        Grant::decode(&trailing).unwrap_err(),
        ObjectError::Malformed
    );
}

#[test]
fn a_delegated_grantor_encodes_as_a_key() {
    let r = signed_root();
    let g = child(&r);
    assert_eq!(Grant::decode(&g.encode()).unwrap(), g);
}
