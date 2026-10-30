//! Channel grants: issuing, chains and the evaluator.

mod common;

use cahoots_channel::grant::{
    evaluate, from_claims, issue_delegated, issue_root, sign_delegated, sign_root, verify_chain,
    ChannelGrant, EvalRequest, Refusal,
};
use cahoots_channel::{Action, ChannelError, ChannelId};
use cahoots_core::cose;
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_objects::cwt::{self, Claims, CHANNEL_TYPE};
use cahoots_objects::grant::{key_id, Grantor};
use cahoots_objects::GrantError;
use common::*;
use sha2::{Digest, Sha256};

const CHANNEL: ChannelId = [0xc4; 16];
const OTHER: ChannelId = [0x5e; 16];

type Revoked<'a> = &'a dyn Fn(&[u8; 16]) -> bool;

fn member(i: u64) -> SeatKey {
    SeatKey::from_seed(&fill(900 + i, 32).try_into().unwrap()).unwrap()
}

/// A root grant to `member(0)` from seat 1, signed.
fn root(actions: &[Action], budget: u32, not_after: u64, depth: u8) -> Vec<u8> {
    let claims = issue_root(
        &CIRCLE,
        &CHANNEL,
        1,
        &member(0).public(),
        actions,
        budget,
        not_after,
        depth,
        [1; 16],
    );
    sign_root(&claims, &seat_key(1), 1)
}

/// Claims for a delegation from the grantee of `parent_signed` to `member(1)`.
fn child_claims(parent_signed: &[u8], actions: &[Action], budget: u32, not_after: u64) -> Claims {
    let parent = verify_chain(&[parent_signed], &manifest(), &CHANNEL).unwrap();
    issue_delegated(
        parent_signed,
        &parent,
        &member(1).public(),
        actions,
        budget,
        not_after,
        [2; 16],
    )
}

fn delegate(parent_signed: &[u8], actions: &[Action], budget: u32, not_after: u64) -> Vec<u8> {
    sign_delegated(
        &child_claims(parent_signed, actions, budget, not_after),
        &member(0),
    )
}

fn request<'a>(
    action: Action,
    channel_id: &'a ChannelId,
    log: u64,
    posts_used: u32,
    revoked: Revoked<'a>,
    signer: &'a SeatPublic,
) -> EvalRequest<'a> {
    EvalRequest {
        action,
        channel_id,
        log,
        posts_used,
        revoked,
        signer,
    }
}

#[test]
fn root_grant_verifies_and_evaluates_for_read_and_post() {
    let claims = issue_root(
        &CIRCLE,
        &CHANNEL,
        1,
        &member(0).public(),
        &[Action::Read, Action::Post],
        3,
        100,
        0,
        [1; 16],
    );
    assert_eq!(claims.iss, cwt::issuer(&CIRCLE));
    assert_eq!(claims.sub, cwt::subject(&member(0).public()));
    assert_eq!(claims.cnf, member(0).public());
    assert_eq!(claims.cti, [1; 16]);
    assert_eq!(claims.grantor, Grantor::Seat(1));
    assert_eq!(claims.parent, None);
    assert_eq!(claims.log_not_after, Some(100));
    assert_eq!(
        (claims.aud.clone(), claims.exp, claims.base),
        (None, None, None)
    );
    let [d] = claims.authorization_details.as_slice() else {
        panic!("one detail");
    };
    assert_eq!(d.r#type, CHANNEL_TYPE);
    assert_eq!(d.channel.as_deref(), Some("c4".repeat(16).as_str()));
    assert_eq!(d.actions, Some(vec!["read".into(), "post".into()]));
    assert_eq!(d.budget, Some(3));

    let signed = sign_root(&claims, &seat_key(1), 1);
    assert_eq!(cose::kid1(&signed).unwrap(), vec![1]);
    let g = verify_chain(&[&signed], &manifest(), &CHANNEL).unwrap();
    assert_eq!(
        g,
        ChannelGrant {
            claims: claims.clone(),
            channel_id: CHANNEL,
            actions: vec![Action::Read, Action::Post],
            budget: 3,
            id: [1; 16],
            grantee: member(0).public(),
            not_after: 100,
            depth: 0,
        }
    );
    assert_eq!(from_claims(&claims).unwrap(), g);

    let key = member(0).public();
    let none = |_: &[u8; 16]| false;
    for action in [Action::Read, Action::Post] {
        assert_eq!(
            evaluate(&g, &[g.id], &request(action, &CHANNEL, 50, 2, &none, &key)),
            Ok(())
        );
    }
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Admit, &CHANNEL, 50, 0, &none, &key)
        ),
        Err(Refusal::Action)
    );
    // Budget 3 is spent at three posts; reads do not count against it.
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Post, &CHANNEL, 50, 3, &none, &key)
        ),
        Err(Refusal::Budget)
    );
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Read, &CHANNEL, 50, 3, &none, &key)
        ),
        Ok(())
    );

    // Refused roots: wrong channel, wrong seat key, seat not in the manifest, grantor not the
    // signing seat, another circle, no chain.
    assert_eq!(
        verify_chain(&[&signed], &manifest(), &OTHER),
        Err(ChannelError::WrongChannel)
    );
    let forged = sign_root(&claims, &seat_key(2), 1);
    assert!(matches!(
        verify_chain(&[&forged], &manifest(), &CHANNEL),
        Err(ChannelError::Cose(_))
    ));
    let off_roster = sign_root(&claims, &seat_key(1), 9);
    assert_eq!(
        verify_chain(&[&off_roster], &manifest(), &CHANNEL),
        Err(ChannelError::Grant(GrantError::NotInRoster(9)))
    );
    let wrong_seat = sign_root(&claims, &seat_key(2), 2);
    assert_eq!(
        verify_chain(&[&wrong_seat], &manifest(), &CHANNEL),
        Err(ChannelError::Grant(GrantError::WrongSigner))
    );
    assert_eq!(
        verify_chain(&[&signed], &manifest_for([8; 16]), &CHANNEL),
        Err(ChannelError::Grant(GrantError::Circle))
    );
    assert_eq!(
        verify_chain(&[], &manifest(), &CHANNEL),
        Err(ChannelError::Grant(GrantError::EmptyChain))
    );
}

#[test]
fn delegation_needs_admit_and_narrows() {
    let m = manifest();
    let admin = root(&[Action::Read, Action::Post, Action::Admit], 5, 100, 2);

    // A narrower child verifies, and names its parent and signer.
    let child = delegate(&admin, &[Action::Read, Action::Post], 5, 90);
    assert_eq!(
        cose::kid1(&child).unwrap(),
        key_id(&member(0).public()).to_vec()
    );
    let leaf = verify_chain(&[&admin, &child], &m, &CHANNEL).unwrap();
    assert_eq!(leaf.grantee, member(1).public());
    assert_eq!(leaf.depth, 1);
    assert_eq!(leaf.id, [2; 16]);
    assert_eq!(leaf.not_after, 90);
    assert_eq!(leaf.claims.grantor, Grantor::Key(member(0).public()));
    let parent_hash: [u8; 32] = Sha256::digest(&admin).into();
    assert_eq!(leaf.claims.parent, Some(parent_hash));

    // A parent without Admit cannot delegate, whatever its depth.
    let reader = root(&[Action::Read, Action::Post], 5, 100, 2);
    let child = delegate(&reader, &[Action::Read], 5, 90);
    assert_eq!(
        verify_chain(&[&reader, &child], &m, &CHANNEL),
        Err(ChannelError::NoAdmit)
    );

    // An action the parent does not have.
    let child = delegate(&admin, &[Action::Read, Action::Govern], 5, 90);
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::ExtraPurpose))
    );

    // A larger budget; 0 is no limit, so it is larger than any nonzero budget.
    for budget in [6, 0] {
        let child = delegate(&admin, &[Action::Post], budget, 90);
        assert_eq!(
            verify_chain(&[&admin, &child], &m, &CHANNEL),
            Err(ChannelError::Grant(GrantError::LargerBudget))
        );
    }
    let unlimited = root(&[Action::Post, Action::Admit], 0, 100, 1);
    for budget in [5, 0] {
        let child = delegate(&unlimited, &[Action::Post], budget, 90);
        assert_eq!(
            verify_chain(&[&unlimited, &child], &m, &CHANNEL)
                .unwrap()
                .budget,
            budget
        );
    }

    // A later not_after.
    let child = delegate(&admin, &[Action::Read], 5, 101);
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::LaterNotAfter))
    );

    // A depth that is not one less, and a parent of depth 0.
    let mut claims = child_claims(&admin, &[Action::Read], 5, 90);
    claims.depth = 2;
    let child = sign_delegated(&claims, &member(0));
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::Depth))
    );
    let flat = root(&[Action::Read, Action::Admit], 5, 100, 0);
    let child = delegate(&flat, &[Action::Read], 5, 90);
    assert_eq!(
        verify_chain(&[&flat, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::Depth))
    );

    // Another channel in the child.
    let mut claims = child_claims(&admin, &[Action::Read], 5, 90);
    claims.authorization_details[0].channel = Some("5e".repeat(16));
    let child = sign_delegated(&claims, &member(0));
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::WrongChannel)
    );

    // Signed by a key other than the parent's grantee; a parent hash that does not match; a
    // grantor that is not the parent's grantee; another circle.
    let claims = child_claims(&admin, &[Action::Read], 5, 90);
    let child = sign_delegated(&claims, &member(2));
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::WrongKid))
    );
    let mut claims = child_claims(&admin, &[Action::Read], 5, 90);
    claims.parent = Some([0; 32]);
    let child = sign_delegated(&claims, &member(0));
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::BadParent))
    );
    let mut claims = child_claims(&admin, &[Action::Read], 5, 90);
    claims.grantor = Grantor::Key(member(2).public());
    let child = sign_delegated(&claims, &member(0));
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::WrongSigner))
    );
    let mut claims = child_claims(&admin, &[Action::Read], 5, 90);
    claims.iss = cwt::issuer(&[8; 16]);
    let child = sign_delegated(&claims, &member(0));
    assert_eq!(
        verify_chain(&[&admin, &child], &m, &CHANNEL),
        Err(ChannelError::Grant(GrantError::Circle))
    );
}

#[test]
fn evaluate_refuses_in_order() {
    let signed = root(&[Action::Read, Action::Post], 2, 10, 0);
    let g = verify_chain(&[&signed], &manifest(), &CHANNEL).unwrap();
    let key = member(0).public();
    let stranger = member(5).public();
    let none = |_: &[u8; 16]| false;
    let all = |_: &[u8; 16]| true;
    let ids = [[9; 16], g.id];
    let only_parent = |id: &[u8; 16]| *id == [9; 16];

    // Each check refuses only once those before it pass, though every later one would fail too.
    let cases: [(Revoked, &SeatPublic, &ChannelId, Action, u64, Refusal); 6] = [
        (
            &all,
            &stranger,
            &OTHER,
            Action::Govern,
            10,
            Refusal::Revoked,
        ),
        (
            &only_parent,
            &stranger,
            &OTHER,
            Action::Govern,
            10,
            Refusal::Revoked,
        ),
        (&none, &stranger, &OTHER, Action::Govern, 10, Refusal::Key),
        (
            &none,
            &key,
            &OTHER,
            Action::Govern,
            10,
            Refusal::WrongChannel,
        ),
        (&none, &key, &CHANNEL, Action::Govern, 10, Refusal::Action),
        (&none, &key, &CHANNEL, Action::Post, 10, Refusal::Expired),
    ];
    for (revoked, signer, channel, action, log, want) in cases {
        assert_eq!(
            evaluate(&g, &ids, &request(action, channel, log, 2, revoked, signer)),
            Err(want)
        );
    }
    assert_eq!(
        evaluate(
            &g,
            &ids,
            &request(Action::Post, &CHANNEL, 9, 2, &none, &key)
        ),
        Err(Refusal::Budget)
    );
    assert_eq!(
        evaluate(
            &g,
            &ids,
            &request(Action::Post, &CHANNEL, 9, 1, &none, &key)
        ),
        Ok(())
    );
}

#[test]
fn expired_at_the_channel_head() {
    let signed = root(&[Action::Read], 0, 10, 0);
    let g = verify_chain(&[&signed], &manifest(), &CHANNEL).unwrap();
    let key = member(0).public();
    let none = |_: &[u8; 16]| false;
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Read, &CHANNEL, 10, 0, &none, &key)
        ),
        Err(Refusal::Expired)
    );
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Read, &CHANNEL, 9, 0, &none, &key)
        ),
        Ok(())
    );
}

#[test]
fn read_only_grant_cannot_post() {
    let signed = root(&[Action::Read], 0, 100, 0);
    let g = verify_chain(&[&signed], &manifest(), &CHANNEL).unwrap();
    let key = member(0).public();
    let none = |_: &[u8; 16]| false;
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Read, &CHANNEL, 0, 0, &none, &key)
        ),
        Ok(())
    );
    // Budget 0 is no limit on posts, but it does not allow posting.
    assert_eq!(
        evaluate(
            &g,
            &[g.id],
            &request(Action::Post, &CHANNEL, 0, 0, &none, &key)
        ),
        Err(Refusal::Action)
    );
}
