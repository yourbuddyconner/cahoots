//! The owner and member cores together: deposits are signed and a member installs one only from
//! a key whose grant carries `govern`; `admit` grants only actions a rule gives evidence for,
//! and `permitted` only what a link proves; a retired link's statement waits for the relay.
#![cfg(feature = "std")]

mod common;

use cahoots_channel::deposit::{peek_signed, seal_for_key, seal_for_link};
use cahoots_channel::grant::{issue_root, sign_root};
use cahoots_channel::keys::EpochId;
use cahoots_channel::link::Link;
use cahoots_channel::member::{bytes_array, ChannelMember, JoinRequest};
use cahoots_channel::owner::ChannelOwner;
use cahoots_channel::Action;
use cahoots_core::cose;
use cahoots_core::manifest::{SealedManifest, MANIFEST_AAD};
use cahoots_core::seat::SeatKey;
use common::{fill, manifest, seat_key, CIRCLE};

const GROUP_CHAT: u8 = 2;
const MEMBERS_POST: u8 = 3;
const CLOSED: u8 = 4;

fn sealed() -> Vec<u8> {
    let body = manifest().encode();
    let attestations = (0..3u8)
        .map(|i| cose::sign1(&seat_key(i), &[i], &body, MANIFEST_AAD, true))
        .collect();
    SealedManifest { body, attestations }.encode()
}

/// An owner on seat 0 with a link and a first epoch, and the link.
fn owner(preset: u8) -> (ChannelOwner, String) {
    let seed: [u8; 32] = fill(350, 32).try_into().unwrap();
    let mut o = ChannelOwner::start_with_seed(&sealed(), 0, &seed, "Garden", preset, 1).unwrap();
    let link = o.link_url("relay.example").unwrap();
    o.rotate().unwrap();
    (o, link)
}

/// A key with a real root grant from seat 0 for `actions`, and the grant's chain.
fn granted(o: &ChannelOwner, actions: &[Action], i: u64) -> (SeatKey, Vec<u8>) {
    let key = SeatKey::from_seed(&fill(700 + i, 32).try_into().unwrap()).unwrap();
    let claims = issue_root(
        &CIRCLE,
        &o.channel_id(),
        0,
        &key.public(),
        actions,
        0,
        u64::MAX,
        0,
        [i as u8; 16],
    );
    let chain = bytes_array(&[&sign_root(&claims, &seat_key(0), 0)]);
    (key, chain)
}

fn next_epoch(o: &ChannelOwner) -> EpochId {
    EpochId {
        channel_id: o.channel_id(),
        epoch: o.member().current_epoch().unwrap().epoch + 1,
    }
}

#[test]
fn a_member_installs_only_deposits_signed_by_a_key_that_governs() {
    let (o, link) = owner(GROUP_CHAT);
    let manifest = o.manifest().to_vec();
    let secret = Link::parse(&link).unwrap().secret.unwrap();
    let mut m = ChannelMember::from_link(&link).unwrap();

    // A forged link deposit: sealed with the right secret, signed by a key whose grant does not
    // carry govern, shown with that grant's chain.
    let (poster, poster_chain) = granted(&o, &[Action::Read, Action::Post], 1);
    let forged = seal_for_link(&[0x66; 32], &secret, &next_epoch(&o)).sign(&poster);
    assert_eq!(
        m.install_from_link_deposit(&forged, &poster_chain, &manifest),
        Err("refused:action".into())
    );
    // The same, shown with the owner's chain: the kid is not the chain's key.
    assert_eq!(
        m.install_from_link_deposit(&forged, &o.self_grant(), &manifest),
        Err("signature".into())
    );
    // No chain at all, and the unsigned deposit.
    assert_eq!(
        m.install_from_link_deposit(&forged, &bytes_array(&[]), &manifest),
        Err("no-grant".into())
    );
    let (_, d) = peek_signed(&forged).unwrap();
    assert!(m
        .install_from_link_deposit(&d.encode(), &o.self_grant(), &manifest)
        .is_err());
    assert_eq!(m.current_epoch(), None);

    // The owner's deposit, signed by its member key under the self-grant, installs.
    let good = &o.deposits()[0];
    let (kid, _) = peek_signed(good).unwrap();
    assert_eq!(kid, o.member().my_kid());
    m.install_from_link_deposit(good, &o.self_grant(), &manifest)
        .unwrap();
    assert_eq!(m.current_epoch(), o.member().current_epoch());

    // A forged key deposit, sealed to the member's own X-Wing key.
    let request = m.join_request("Cee", "read").unwrap();
    let xwing_pk = JoinRequest::parse(&request).unwrap().xwing_pk;
    let forged_key = seal_for_key(
        &[0x67; 32],
        &next_epoch(&o),
        m.my_kid(),
        &xwing_pk,
        &mut rand::rng(),
    )
    .unwrap()
    .sign(&poster);
    assert_eq!(
        m.install_from_key_deposit(&forged_key, &poster_chain, &manifest),
        Err("refused:action".into())
    );
    assert_eq!(m.current_epoch().unwrap().epoch, 1);
}

#[test]
fn admit_grants_only_what_a_rule_gives_evidence_for() {
    let (mut o, link) = owner(GROUP_CHAT);
    let m = ChannelMember::from_link(&link).unwrap();
    let request = m.join_request("Cee", "read,post,admit,govern").unwrap();
    // Group chat: the link proves read and post; admit and govern are the owner's seats only.
    assert_eq!(
        o.permitted(&request).unwrap(),
        vec![Action::Read, Action::Post]
    );
    let seat = SeatKey::from_seed(&fill(350, 32).try_into().unwrap()).unwrap();
    for actions in ["govern", "admit", "read,post,admit", "read,govern"] {
        assert_eq!(
            o.admit(&request, actions, &seat).map(|_| ()),
            Err("policy".into()),
            "{actions}"
        );
    }
    assert!(o.members().is_empty());
    o.admit(&request, "read,post", &seat).unwrap();
    assert_eq!(o.members().len(), 1);

    // A request with an old link proves nothing.
    let stale = ChannelMember::from_link(&link.replace(
        &link.rsplit('/').next().unwrap().to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ))
    .unwrap()
    .join_request("Dee", "read,post")
    .unwrap();
    assert!(o.permitted(&stale).unwrap().is_empty());
    assert_eq!(
        o.admit(&stale, "read", &seat).map(|_| ()),
        Err("evidence".into())
    );
}

#[test]
fn permitted_is_the_link_granted_subset() {
    // Members post: the link proves read; post is by hand, so it is left out.
    let (o, link) = owner(MEMBERS_POST);
    let m = ChannelMember::from_link(&link).unwrap();
    for wanted in ["read,post", "read,post,admit,govern"] {
        let request = m.join_request("Cee", wanted).unwrap();
        assert_eq!(
            o.permitted(&request).unwrap(),
            vec![Action::Read],
            "{wanted}"
        );
    }
    // Closed: nothing is granted by link.
    let (o, link) = owner(CLOSED);
    let m = ChannelMember::from_link(&link).unwrap();
    for wanted in ["read", "read,post", "read,post,admit,govern"] {
        let request = m.join_request("Cee", wanted).unwrap();
        assert!(o.permitted(&request).unwrap().is_empty(), "{wanted}");
    }
    // Members post: the owner may still let someone post by hand.
    let (mut o, link) = owner(MEMBERS_POST);
    let seat = SeatKey::from_seed(&fill(350, 32).try_into().unwrap()).unwrap();
    let request = ChannelMember::from_link(&link)
        .unwrap()
        .join_request("Cee", "read,post,govern")
        .unwrap();
    assert_eq!(
        o.admit(&request, "read,post,govern", &seat).map(|_| ()),
        Err("policy".into())
    );
    o.admit(&request, "read,post", &seat).unwrap();
}

#[test]
fn the_closed_preset_admits_readers_by_hand() {
    let (mut o, link) = owner(CLOSED);
    let seat = SeatKey::from_seed(&fill(350, 32).try_into().unwrap()).unwrap();
    let m = ChannelMember::from_link(&link).unwrap();
    let request = m.join_request("Cee", "read,post").unwrap();
    assert!(o.permitted(&request).unwrap().is_empty());
    // Post, admit and govern stay with the owner's seats.
    assert_eq!(
        o.admit(&request, "read,post", &seat).map(|_| ()),
        Err("policy".into())
    );
    let a = o.admit(&request, "read", &seat).unwrap();
    assert_eq!(o.members().len(), 1);
    // The member opens the key deposit it was given, from the owner's member key.
    let mut m = m;
    m.install_from_key_deposit(&a.deposit, &o.self_grant(), o.manifest())
        .unwrap();
    assert_eq!(m.current_epoch(), o.member().current_epoch());
    // And a request without a link at all is let in by hand too.
    let bare = link.rsplit_once('/').unwrap().0;
    let request = ChannelMember::from_link(bare)
        .unwrap()
        .join_request("Dee", "read")
        .unwrap();
    o.admit(&request, "read", &seat).unwrap();
}

#[test]
fn a_retired_links_statement_waits_for_the_relay() {
    let (mut o, _) = owner(GROUP_CHAT);
    let seat = SeatKey::from_seed(&fill(350, 32).try_into().unwrap()).unwrap();
    let hash = o.gate_token_hash().unwrap();
    let statement = o.retire_link(&seat).unwrap();
    // The secret is gone at once; the statement is kept, through an export.
    assert_eq!(o.gate_token_hash(), None);
    assert_eq!(o.pending_retire(), &[(hash, statement.clone())]);
    let mut back = ChannelOwner::import(&o.export()).unwrap();
    assert_eq!(back.pending_retire(), &[(hash, statement)]);
    back.retire_confirmed(&[0; 32]);
    assert_eq!(back.pending_retire().len(), 1);
    back.retire_confirmed(&hash);
    assert!(back.pending_retire().is_empty());
    assert!(ChannelOwner::import(&back.export())
        .unwrap()
        .pending_retire()
        .is_empty());
}
