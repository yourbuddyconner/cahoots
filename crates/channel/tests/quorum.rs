//! Quorum approvals and governed acts.

mod common;

use cahoots_channel::policy::{verify_governed, Policy, Preset, DOMAIN};
use cahoots_channel::quorum::{approval_for, check_quorum, PROFILE_GOVERN, PROFILE_POST};
use cahoots_channel::{Action, ChannelError};
use cahoots_core::seat::SeatKey;
use cahoots_objects::{assurance, Approval};
use common::*;
use sha2::{Digest, Sha256};

const DIGEST: [u8; 32] = [0x5a; 32];
const POLICY_ID: [u8; 32] = [0x77; 32];

fn approve(seat: u8, profile: &str, digest: [u8; 32], level: u8, nonce: u8) -> Vec<u8> {
    approval_for(
        seat_key(seat),
        seat,
        &CIRCLE,
        profile,
        vec![("post".into(), "hello".into())],
        digest,
        &POLICY_ID,
        3,
        level,
        [nonce; 16],
    )
}

#[test]
fn approval_for_builds_a_verifying_yes() {
    let m = manifest();
    let bytes = approve(1, PROFILE_POST, DIGEST, assurance::PRESENT, 1);
    let a = Approval::verify(&bytes, &m).unwrap();
    assert!(a.verdict);
    assert_eq!(a.seat, 1);
    assert_eq!(a.policy_id, POLICY_ID.to_vec());
    assert_eq!(a.request_digest, DIGEST);
    assert_eq!(a.profile, PROFILE_POST);
}

#[test]
fn quorum_counts_distinct_seats_at_or_above_assurance() {
    let m = manifest();
    let p = assurance::PRESENT;
    let two_by_one = [
        approve(1, PROFILE_POST, DIGEST, p, 1),
        approve(1, PROFILE_POST, DIGEST, p, 2),
    ];
    assert_eq!(
        check_quorum(&two_by_one, &m, &DIGEST, PROFILE_POST, 2, p),
        Err(ChannelError::Quorum { have: 1, need: 2 })
    );
    let two_seats = [
        approve(1, PROFILE_POST, DIGEST, p, 1),
        approve(2, PROFILE_POST, DIGEST, assurance::PRESENT_PIN, 2),
    ];
    assert_eq!(
        check_quorum(&two_seats, &m, &DIGEST, PROFILE_POST, 2, p),
        Ok(2)
    );
    // Assurance 1 under a rule of 2 does not count.
    let weak = [
        approve(1, PROFILE_POST, DIGEST, p, 1),
        approve(2, PROFILE_POST, DIGEST, 1, 2),
    ];
    assert_eq!(
        check_quorum(&weak, &m, &DIGEST, PROFILE_POST, 2, p),
        Err(ChannelError::Quorum { have: 1, need: 2 })
    );
    // Quorum 0 reads nothing.
    assert_eq!(
        check_quorum(&[b"junk".to_vec()], &m, &DIGEST, PROFILE_POST, 0, p),
        Ok(0)
    );
}

#[test]
fn quorum_refuses_wrong_profile_digest_or_verdict_false() {
    let m = manifest();
    let p = assurance::PRESENT;
    let good = approve(1, PROFILE_POST, DIGEST, p, 1);
    let wrong_profile = approve(2, PROFILE_GOVERN, DIGEST, p, 2);
    let wrong_digest = approve(2, PROFILE_POST, [0; 32], p, 3);
    let yes = Approval::verify(&approve(2, PROFILE_POST, DIGEST, p, 4), &m).unwrap();
    let no = Approval {
        verdict: false,
        ..yes
    }
    .sign(seat_key(2), &[2]);
    for other in [wrong_profile, wrong_digest, no] {
        assert_eq!(
            check_quorum(&[good.clone(), other], &m, &DIGEST, PROFILE_POST, 2, p),
            Err(ChannelError::Quorum { have: 1, need: 2 })
        );
    }
}

#[test]
fn quorum_fails_whole_on_a_bad_or_foreign_approval() {
    let m = manifest();
    let p = assurance::PRESENT;
    let good = approve(1, PROFILE_POST, DIGEST, p, 1);
    // Signed by a key that is not seat 2's.
    let stranger = SeatKey::from_seed(&[0x40; 32]).unwrap();
    let a = Approval::verify(&approve(2, PROFILE_POST, DIGEST, p, 2), &m).unwrap();
    let forged = a.sign(&stranger, &[2]);
    assert!(matches!(
        check_quorum(&[good.clone(), forged], &m, &DIGEST, PROFILE_POST, 1, p),
        Err(ChannelError::Object(_))
    ));
    // Another circle's approval.
    let foreign = approval_for(
        seat_key(2),
        2,
        &[8; 16],
        PROFILE_POST,
        vec![],
        DIGEST,
        &POLICY_ID,
        3,
        p,
        [9; 16],
    );
    assert!(matches!(
        check_quorum(&[good, foreign], &m, &DIGEST, PROFILE_POST, 1, p),
        Err(ChannelError::Object(_))
    ));
}

fn policy(seats: u8) -> Policy {
    let channel = cahoots_channel::object::channel_id_for(&CIRCLE, &[9; 16]);
    Policy::preset(
        Preset::GroupChat,
        CIRCLE,
        channel,
        [0x11; 32],
        seats,
        0,
        [1; 16],
    )
}

#[test]
fn govern_quorum_2_refuses_a_policy_with_one_approval() {
    let m = manifest();
    let p = policy(3);
    let signed = p.sign(&seat_key(0), 0);
    let digest: [u8; 32] = Sha256::digest(&signed).into();
    let govern = p.rule(Action::Govern).unwrap();
    assert_eq!(govern.quorum, 2);
    let one = vec![approve(1, PROFILE_GOVERN, digest, assurance::PRESENT, 1)];
    assert_eq!(
        verify_governed(&signed, DOMAIN, &one, &m, govern, PROFILE_GOVERN),
        Err(ChannelError::Quorum { have: 1, need: 2 })
    );
    let mut two = one.clone();
    two.push(approve(2, PROFILE_GOVERN, digest, assurance::PRESENT, 2));
    assert_eq!(
        verify_governed(&signed, DOMAIN, &two, &m, govern, PROFILE_GOVERN).unwrap(),
        p.encode()
    );
    // Approvals of another request do not carry over.
    let moved = vec![
        approve(1, PROFILE_GOVERN, [0; 32], assurance::PRESENT, 3),
        approve(2, PROFILE_GOVERN, [0; 32], assurance::PRESENT, 4),
    ];
    assert!(verify_governed(&signed, DOMAIN, &moved, &m, govern, PROFILE_GOVERN).is_err());
}

#[test]
fn govern_quorum_1_accepts_the_signer_alone() {
    let m = manifest();
    let p = policy(1);
    let signed = p.sign(&seat_key(0), 0);
    let govern = p.rule(Action::Govern).unwrap();
    assert_eq!(govern.quorum, 1);
    assert_eq!(
        verify_governed(&signed, DOMAIN, &[], &m, govern, PROFILE_GOVERN).unwrap(),
        p.encode()
    );
    // The domain is explicit: another domain does not open it.
    let other = cahoots_channel::object::DOMAIN;
    assert!(verify_governed(&signed, other, &[], &m, govern, PROFILE_GOVERN).is_err());
}
