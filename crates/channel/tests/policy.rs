//! The policy.

mod common;

use cahoots_channel::object::channel_id_for;
use cahoots_channel::policy::{
    id, verify_governed, Evidence, Issuer, Policy, Preset, Rule, DOMAIN,
};
use cahoots_channel::{Action, ChannelError};
use cahoots_core::cose;
use cahoots_core::seat::SeatKey;
use common::*;
use sha2::{Digest, Sha256};

const LINK: [u8; 32] = [0x11; 32];

fn channel() -> [u8; 16] {
    channel_id_for(&CIRCLE, &[9; 16])
}

fn evidence(kind: u8, r: &[u8], scope: &str) -> Evidence {
    Evidence {
        kind,
        r#ref: r.to_vec(),
        scope: scope.into(),
    }
}

fn rule(action: Action, evidence: Vec<Evidence>) -> Rule {
    Rule {
        action,
        evidence,
        per_evidence: 0,
        quorum: 0,
        assurance: 2,
    }
}

fn full_policy() -> Policy {
    let member = SeatKey::from_seed(&[0x40; 32]).unwrap().public();
    let issuer = SeatKey::from_seed(&[0x41; 32]).unwrap().public();
    Policy {
        circle_id: CIRCLE,
        channel_id: channel(),
        rules: vec![
            Rule {
                per_evidence: 50,
                ..rule(
                    Action::Read,
                    vec![
                        evidence(0, &LINK, ""),
                        evidence(1, &member.0, ""),
                        evidence(2, &[0x22; 32], ""),
                        evidence(3, &[0x33; 20], "passport:2026"),
                        evidence(4, &[0x44; 32], "github:org:example"),
                    ],
                )
            },
            Rule {
                quorum: 2,
                assurance: 3,
                ..rule(Action::Post, vec![evidence(0, &LINK, "")])
            },
            rule(Action::Govern, vec![]),
        ],
        issuers: vec![
            Issuer {
                key: issuer,
                endpoint: "https://issuer.example/join".into(),
                measurement: Some([0x55; 32]),
            },
            Issuer {
                key: member,
                endpoint: String::new(),
                measurement: None,
            },
        ],
        log: 7,
        parent: Some([0x66; 32]),
        nonce: [0x77; 16],
    }
}

#[test]
fn action_names_parse_and_numbers() {
    for (a, name, n) in [
        (Action::Read, "read", 0),
        (Action::Post, "post", 1),
        (Action::Admit, "admit", 2),
        (Action::Govern, "govern", 3),
    ] {
        assert_eq!(a.name(), name);
        assert_eq!(Action::parse(name), Some(a));
        assert_eq!(Action::from_u8(n), Some(a));
        assert_eq!(a as u8, n);
    }
    assert_eq!(Action::parse("write"), None);
    assert_eq!(Action::from_u8(4), None);
}

#[test]
fn policy_round_trips_with_every_evidence_kind_and_an_issuer() {
    let p = full_policy();
    assert_eq!(Policy::decode(&p.encode()).unwrap(), p);
    let signed = p.sign(&seat_key(0), 0);
    assert_eq!(
        Policy::verify(&signed, &manifest()).unwrap(),
        (0, p.clone())
    );
    assert_eq!(p.rule(Action::Post).unwrap().quorum, 2);
    assert!(p.rule(Action::Admit).is_none());
    assert!(p.rule(Action::Govern).unwrap().evidence.is_empty());

    let mut root = p.clone();
    root.parent = None;
    root.issuers.clear();
    assert_eq!(Policy::decode(&root.encode()).unwrap(), root);

    assert_eq!(
        Policy::verify(&signed, &manifest_for([8; 16])),
        Err(ChannelError::WrongCircle)
    );
}

#[test]
fn presets_have_the_documented_rules() {
    let make = |preset, seats| Policy::preset(preset, CIRCLE, channel(), LINK, seats, 4, [5; 16]);
    // Link evidence names no link: the owner's current one, whose gate token the relay holds.
    let link = vec![evidence(0, &[], "")];
    let all = [Action::Read, Action::Post, Action::Admit, Action::Govern];

    for preset in [
        Preset::Announcements,
        Preset::AnnouncementsQuorum(2),
        Preset::GroupChat,
        Preset::MembersPost,
        Preset::Closed,
    ] {
        let p = make(preset, 3);
        assert_eq!(p.circle_id, CIRCLE);
        assert_eq!(p.channel_id, channel());
        assert_eq!(p.log, 4);
        assert_eq!(p.nonce, [5; 16]);
        assert_eq!(p.parent, None);
        assert!(p.issuers.is_empty());
        assert_eq!(p.rules.iter().map(|r| r.action).collect::<Vec<_>>(), all);
        for r in &p.rules {
            assert_eq!(r.assurance, 2);
            assert_eq!(r.per_evidence, 0);
        }
        assert!(p.rule(Action::Admit).unwrap().evidence.is_empty());
        assert!(p.rule(Action::Govern).unwrap().evidence.is_empty());
        assert_eq!(p.rule(Action::Govern).unwrap().quorum, 2);
        assert_eq!(Policy::decode(&p.encode()).unwrap(), p);
    }

    let ann = make(Preset::Announcements, 3);
    assert_eq!(ann.rule(Action::Read).unwrap().evidence, link);
    assert!(ann.rule(Action::Post).unwrap().evidence.is_empty());
    assert_eq!(ann.rule(Action::Post).unwrap().quorum, 0);

    let annq = make(Preset::AnnouncementsQuorum(2), 3);
    assert_eq!(annq.rule(Action::Read).unwrap().evidence, link);
    assert!(annq.rule(Action::Post).unwrap().evidence.is_empty());
    assert_eq!(annq.rule(Action::Post).unwrap().quorum, 2);

    let chat = make(Preset::GroupChat, 3);
    assert_eq!(chat.rule(Action::Read).unwrap().evidence, link);
    assert_eq!(chat.rule(Action::Post).unwrap().evidence, link);
    assert_eq!(chat.rule(Action::Post).unwrap().quorum, 0);

    // Members post: one key evidence with no key named, any key the owner admits by hand.
    let members = make(Preset::MembersPost, 3);
    assert_eq!(members.rule(Action::Read).unwrap().evidence, link);
    assert_eq!(
        members.rule(Action::Post).unwrap().evidence,
        vec![evidence(1, &[], "")]
    );
    assert_eq!(members.rule(Action::Post).unwrap().quorum, 0);
    assert_ne!(
        members.rule(Action::Post),
        ann.rule(Action::Post),
        "announcements and members post differ"
    );

    let closed = make(Preset::Closed, 3);
    assert!(closed.rules.iter().all(|r| r.evidence.is_empty()));

    for (seats, quorum) in [(1, 1), (2, 2), (3, 2), (4, 3), (5, 3)] {
        let p = make(Preset::GroupChat, seats);
        assert_eq!(
            p.rule(Action::Govern).unwrap().quorum,
            quorum,
            "{seats} seats"
        );
    }
}

#[test]
fn policy_refuses_rules_out_of_action_order_or_repeated_and_unknown_kinds() {
    let base = full_policy();
    let refused = |p: &Policy| Policy::decode(&p.encode());

    let mut out_of_order = base.clone();
    out_of_order.rules.swap(0, 1);
    assert_eq!(refused(&out_of_order), Err(ChannelError::Malformed));

    let mut repeated = base.clone();
    repeated.rules.insert(1, rule(Action::Read, vec![]));
    assert_eq!(refused(&repeated), Err(ChannelError::Malformed));

    let mut unknown_kind = base.clone();
    unknown_kind.rules[0]
        .evidence
        .push(evidence(5, &[0; 32], ""));
    assert_eq!(refused(&unknown_kind), Err(ChannelError::Malformed));

    for kind in [0u8, 1, 2] {
        let len = if kind == 1 { 33 } else { 32 };
        let mut scoped = base.clone();
        scoped.rules[0].evidence = vec![evidence(kind, &vec![2; len], "scope")];
        assert_eq!(
            refused(&scoped),
            Err(ChannelError::Malformed),
            "kind {kind}"
        );
    }

    // A key evidence names one key (33 bytes) or none (empty); any other length is refused.
    for len in [1usize, 32, 34] {
        let mut odd_key = base.clone();
        odd_key.rules[0].evidence = vec![evidence(1, &vec![2; len], "")];
        assert_eq!(refused(&odd_key), Err(ChannelError::Malformed), "{len}");
    }
    let mut any_key = base.clone();
    any_key.rules[0].evidence = vec![evidence(1, &[], "")];
    assert_eq!(Policy::decode(&any_key.encode()), Ok(any_key));

    // A link evidence names one link id (32 bytes) or none (empty, the current link).
    for len in [1usize, 31, 33] {
        let mut odd_link = base.clone();
        odd_link.rules[0].evidence = vec![evidence(0, &vec![0; len], "")];
        assert_eq!(refused(&odd_link), Err(ChannelError::Malformed), "{len}");
    }
    let mut current_link = base.clone();
    current_link.rules[0].evidence = vec![evidence(0, &[], "")];
    assert_eq!(Policy::decode(&current_link.encode()), Ok(current_link));

    let mut high_assurance = base.clone();
    high_assurance.rules[1].assurance = 4;
    assert_eq!(refused(&high_assurance), Err(ChannelError::Malformed));

    // An issuer key that is not a point: patch the first issuer key's prefix byte in the bytes.
    let bytes = base.encode();
    let key = base.issuers[0].key.0;
    let at = bytes.windows(33).position(|w| w == key).unwrap();
    let mut bad_key = bytes.clone();
    bad_key[at] = 0x04;
    assert_eq!(Policy::decode(&bad_key), Err(ChannelError::Malformed));
}

#[test]
fn policy_id_is_sha256_of_signed_bytes() {
    let signed = full_policy().sign(&seat_key(2), 2);
    let want: [u8; 32] = Sha256::digest(&signed).into();
    assert_eq!(id(&signed), want);
    assert_ne!(id(&signed), id(&full_policy().sign(&seat_key(1), 1)));
}

#[test]
fn verify_governed_takes_the_signing_seat_as_its_own_approval() {
    let m = manifest();
    let p = Policy::preset(Preset::GroupChat, CIRCLE, channel(), LINK, 1, 0, [1; 16]);
    let signed = p.sign(&seat_key(0), 0);
    let govern = p.rule(Action::Govern).unwrap();
    assert_eq!(govern.quorum, 1);
    assert_eq!(
        verify_governed(&signed, DOMAIN, &[], &m, govern, "channel-govern").unwrap(),
        p.encode()
    );

    let two = Rule {
        quorum: 2,
        ..govern.clone()
    };
    assert!(matches!(
        verify_governed(&signed, DOMAIN, &[], &m, &two, "channel-govern"),
        Err(ChannelError::Quorum { need: 2, .. })
    ));

    // Not signed by a seat: a member key under seat 0's kid.
    let stranger = SeatKey::from_seed(&[0x40; 32]).unwrap();
    let forged = cose::sign1(&stranger, &[0], &p.encode(), DOMAIN, false);
    assert!(verify_governed(&forged, DOMAIN, &[], &m, govern, "channel-govern").is_err());
}
