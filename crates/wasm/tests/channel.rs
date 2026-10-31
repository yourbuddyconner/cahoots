//! Native tests of the channel bindings: an owner on a circle, members who join by link.

use cahoots_core::cose;
use cahoots_core::seat::SeatPublic;
use cahoots_wasm::channel::{policy_summary, verify_channel_object, ChannelMember, ChannelOwner};
use cahoots_wasm::{b64_decode, b64_encode, Circle, WasmCeremony};
use minicbor::Decoder;
use sha2::{Digest, Sha256};

fn contribution(i: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|k| ((k as u32).wrapping_mul(29 + i as u32) ^ (i as u32 * 7)) as u8)
        .collect()
}

/// A two-seat circle; seat 0 is returned.
fn circle() -> Circle {
    circles().0
}

/// Both seats of a two-seat circle.
fn circles() -> (Circle, Circle) {
    let l = 4096;
    let mut a = WasmCeremony::new(
        2,
        l,
        "chan",
        false,
        0,
        &contribution(0, l as usize),
        None,
        &[7u8; 32],
        &[0x21; 32],
        0,
    )
    .unwrap();
    let mut b = WasmCeremony::new(
        2,
        l,
        "chan",
        false,
        1,
        &contribution(1, l as usize),
        None,
        &[9u8; 32],
        &[0x22; 32],
        0,
    )
    .unwrap();
    let (ca, cb) = (a.commitment(), b.commitment());
    a.add_commitment(1, &cb).unwrap();
    b.add_commitment(0, &ca).unwrap();
    let na = a.reveal_randomness_len();
    let nb = b.reveal_randomness_len();
    let ra = a.reveal(&contribution(50, na)).unwrap().unwrap();
    let rb = b.reveal(&contribution(51, nb)).unwrap().unwrap();
    a.add_reveal(1, &rb).unwrap();
    b.add_reveal(0, &ra).unwrap();
    let (aa, ab) = (a.attestation().unwrap(), b.attestation().unwrap());
    a.add_attestation(1, &ab).unwrap();
    b.add_attestation(0, &aa).unwrap();
    (
        a.finish("CSPRNG").unwrap().into_circle(),
        b.finish("CSPRNG").unwrap().into_circle(),
    )
}

/// Split a CBOR array of byte strings.
fn split(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut d = Decoder::new(bytes);
    let n = d.array().unwrap().unwrap();
    let out = (0..n).map(|_| d.bytes().unwrap().to_vec()).collect();
    assert_eq!(d.position(), bytes.len());
    out
}

const GROUP_CHAT: u8 = 2;

/// An owner of a group chat with a link and a first epoch, and the link.
fn group_chat() -> (ChannelOwner, String) {
    let mut owner = circle().channel_owner("Garden", GROUP_CHAT, 1).unwrap();
    let link = owner.link_url("relay.example").unwrap();
    owner.rotate().unwrap();
    (owner, link)
}

/// A member from the link holding the current link deposit.
fn joined(owner: &ChannelOwner, link: &str) -> ChannelMember {
    let mut m = ChannelMember::from_link(link).unwrap();
    for d in split(&owner.deposits()) {
        m.install_from_link_deposit(&d, &owner.self_grant(), &owner.manifest())
            .unwrap();
    }
    m
}

#[test]
fn owner_creates_member_joins_by_link_and_they_talk() {
    let (mut owner, link) = group_chat();
    let manifest = owner.manifest();
    let json = verify_channel_object(&owner.channel_object(), &manifest).unwrap();
    assert!(json.contains(&format!("\"channel_id\":\"{}\"", owner.channel_id_hex())));
    assert!(json.contains("\"name\":\"Garden\""));
    assert!(json.contains(&format!("\"policy_id\":\"{}\"", owner.policy_id_hex())));
    assert_eq!(
        hex::encode(Sha256::digest(owner.policy())),
        owner.policy_id_hex()
    );

    let mut member = ChannelMember::from_link(&link).unwrap();
    assert_eq!(member.channel_id_hex(), owner.channel_id_hex());
    assert_eq!(member.relay_host(), "relay.example");
    assert_eq!(member.gate_token_hex().len(), 64);
    assert!(member.current_epoch_hex().is_none());
    let deposits = split(&owner.deposits());
    assert_eq!(deposits.len(), 1, "one link deposit and no members yet");
    member
        .install_from_link_deposit(&deposits[0], &owner.self_grant(), &owner.manifest())
        .unwrap();
    assert_eq!(
        member.current_epoch_hex(),
        owner.member().current_epoch_hex()
    );

    // The member reads the owner's rotation control under the self-grant.
    let rotation = owner.rotate().unwrap();
    for d in split(&owner.deposits()) {
        member
            .install_from_link_deposit(&d, &owner.self_grant(), &owner.manifest())
            .unwrap();
    }
    let got = member
        .receive(&b64_encode(&rotation), &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(got.msg_type(), 1);
    assert!(got.control_json().starts_with(&format!(
        "{{\"kind\":\"rotation\",\"epoch\":\"{}",
        owner.channel_id_hex()
    )));

    // Join, admit, post, read.
    let request = member.join_request("Ann", "read,post").unwrap();
    let parts = split(&owner.admit(&request, "read,post").unwrap());
    assert_eq!(parts.len(), 3);
    let chain = parts[0].clone();
    member
        .install_from_key_deposit(&parts[1], &owner.self_grant(), &owner.manifest())
        .unwrap();
    let join = owner
        .member()
        .receive(&b64_encode(&parts[2]), &[], &manifest)
        .unwrap();
    assert!(join.control_json().starts_with("{\"kind\":\"join\""));
    assert!(join.control_json().contains("\"name\":\"Ann\""));
    let join_seen = member
        .receive(&b64_encode(&parts[2]), &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(join_seen.control_json(), join.control_json());

    member.set_grant(&chain).unwrap();
    let wire = member.send(b"hello").unwrap();
    let got = owner.member().receive(&wire, &chain, &manifest).unwrap();
    assert_eq!(got.text(), b"hello");
    assert_eq!(got.msg_type(), 0);
    assert_eq!(got.kid_hex(), member.my_kid_hex());
    assert_eq!(got.counter(), 0);
    assert_eq!(got.control_json(), "");
    // The member reads its own post with the grant it holds.
    assert_eq!(
        member.receive(&wire, &[], &manifest).unwrap().text(),
        b"hello"
    );

    let wire = owner.member().send(b"hi back").unwrap();
    let got = member
        .receive(&wire, &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(got.text(), b"hi back");
    assert_eq!(got.kid_hex(), owner.member().my_kid_hex());
}

#[test]
fn a_read_only_members_post_is_refused_on_receive() {
    let (mut owner, link) = group_chat();
    let manifest = owner.manifest();
    let mut member = joined(&owner, &link);
    let request = member.join_request("Bo", "read").unwrap();
    let chain = split(&owner.admit(&request, "read").unwrap())[0].clone();
    member.set_grant(&chain).unwrap();
    let wire = member.send(b"may I").unwrap();
    assert_eq!(
        owner
            .member()
            .receive(&wire, &chain, &manifest)
            .unwrap_err(),
        "refused:action"
    );
    // A grant for more than was asked is refused at admission.
    assert_eq!(owner.admit(&request, "read,post").unwrap_err(), "actions");
    // A chain for another key does not cover the post.
    assert_eq!(
        owner
            .member()
            .receive(&wire, &owner.self_grant(), &manifest)
            .unwrap_err(),
        "signature"
    );
}

#[test]
fn admission_checks_the_gate_token_and_the_signature() {
    let (mut owner, link) = group_chat();
    // A member who came by a link without its secret has no gate token to show.
    let bare = link.rsplit_once('/').unwrap().0;
    let stranger = ChannelMember::from_link(bare).unwrap();
    assert_eq!(stranger.gate_token_hex(), "");
    let request = stranger.join_request("Cy", "read,post").unwrap();
    assert_eq!(owner.admit(&request, "read,post").unwrap_err(), "evidence");
    // A changed request does not verify.
    let member = joined(&owner, &link);
    let mut request = member.join_request("Di", "read,post").unwrap();
    let last = request.len() - 1;
    request[last] ^= 1;
    assert_eq!(owner.admit(&request, "read,post").unwrap_err(), "signature");
    assert_eq!(member.join_request("", "read").unwrap_err(), "name");
    assert_eq!(
        member.join_request("Di", "read,fly").unwrap_err(),
        "actions"
    );
}

#[test]
fn a_rotation_after_revoke_leaves_the_revoked_member_out() {
    let (mut owner, link) = group_chat();
    let manifest = owner.manifest();
    let mut a = joined(&owner, &link);
    let mut b = joined(&owner, &link);
    let ra = a.join_request("A", "read,post").unwrap();
    let pa = split(&owner.admit(&ra, "read,post").unwrap());
    let rb = b.join_request("B", "read,post").unwrap();
    owner.admit(&rb, "read,post").unwrap();
    let a_join = b
        .receive(&b64_encode(&pa[2]), &owner.self_grant(), &manifest)
        .unwrap()
        .control_json();
    let a_grant = a_join
        .split("\"grant_id\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(a_grant.len(), 32);
    assert!(owner.members_json().contains(&a_grant));

    let parts = split(&owner.revoke(&a_grant, "left the garden").unwrap());
    assert_eq!(parts.len(), 2);
    // The relay's shape: [grant_id, seat-signed {0: channel, 1: grant, 2: reason}].
    let statement = split(&parts[0]);
    assert_eq!(hex::encode(&statement[0]), a_grant);
    let payload = cose::payload1(&statement[1]).unwrap();
    assert_eq!(cose::kid1(&statement[1]).unwrap(), vec![0]);
    assert!(payload.windows(15).any(|w| w == b"left the garden"));
    let seen = b
        .receive(&b64_encode(&parts[1]), &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(
        seen.control_json(),
        format!(
            "{{\"kind\":\"revocation\",\"grant_id\":\"{a_grant}\",\"reason\":\"left the garden\"}}"
        )
    );
    assert!(!owner.members_json().contains(&a_grant));

    // Retire the link, then rotate: only B's key deposit is made.
    let retire = owner.retire_link().unwrap();
    assert_eq!(cose::kid1(&retire).unwrap(), vec![0]);
    let rotation = owner.rotate().unwrap();
    let deposits = split(&owner.deposits());
    assert_eq!(deposits.len(), 1);
    let before = a.current_epoch_hex();
    for d in &deposits {
        assert!(a
            .install_from_key_deposit(d, &owner.self_grant(), &owner.manifest())
            .is_err());
        assert!(a
            .install_from_link_deposit(d, &owner.self_grant(), &owner.manifest())
            .is_err());
    }
    b.install_from_key_deposit(&deposits[0], &owner.self_grant(), &owner.manifest())
        .unwrap();
    assert_eq!(b.current_epoch_hex(), owner.member().current_epoch_hex());
    assert_ne!(a.current_epoch_hex(), b.current_epoch_hex());
    assert_eq!(a.current_epoch_hex(), before);
    // The rotation control goes out under the old epoch: both read it, and it names only the
    // new epoch and B's deposit.
    let epoch_before = before.clone().unwrap();
    let for_b = b
        .receive(&b64_encode(&rotation), &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(for_b.epoch_hex(), epoch_before);
    assert!(for_b.control_json().starts_with("{\"kind\":\"rotation\""));
    let for_a = a
        .receive(&b64_encode(&rotation), &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(for_a.control_json(), for_b.control_json());
    assert_eq!(a.current_epoch_hex(), before);
    // The next text is in the new epoch: B reads it, A cannot.
    let next = owner.member().send(b"after the rotation").unwrap();
    assert_eq!(
        b.receive(&next, &owner.self_grant(), &manifest)
            .unwrap()
            .text(),
        b"after the rotation"
    );
    assert_eq!(
        a.receive(&next, &owner.self_grant(), &manifest)
            .unwrap_err(),
        "unknown-epoch"
    );
    // B's revocation list now holds A's grant: A's posts are refused.
    a.set_grant(&pa[0]).unwrap();
    let old = a.send(b"still here").unwrap();
    assert_eq!(
        b.receive(&old, &pa[0], &manifest).unwrap_err(),
        "refused:revoked"
    );
}

#[test]
fn proof_for_verifies_under_the_member_key() {
    let (owner, link) = group_chat();
    let member = joined(&owner, &link);
    let target = format!("GET /v1/channels/{}/head?", member.channel_id_hex());
    let nonce = [0x5a; 16];
    let proof = b64_decode(&member.proof_for(&target, &hex::encode(nonce)).unwrap()).unwrap();
    let key = SeatPublic::parse(&hex::decode(member.member_key_hex()).unwrap()).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(key.0)),
        member.my_kid_hex(),
        "the kid is the key's SHA-256"
    );
    let hash: [u8; 32] = Sha256::digest(target.as_bytes()).into();
    assert!(cahoots_objects::authz::proof_verify(
        &proof, &key, &hash, &nonce
    ));
    let other: [u8; 32] = Sha256::digest(b"GET /v1/nonce?").into();
    assert!(!cahoots_objects::authz::proof_verify(
        &proof, &key, &other, &nonce
    ));
    assert_eq!(member.proof_for(&target, "00").unwrap_err(), "nonce");
}

#[test]
fn the_gate_registration_names_the_link_token() {
    let (owner, link) = group_chat();
    let member = ChannelMember::from_link(&link).unwrap();
    let statement = owner.gate_registration().unwrap();
    assert_eq!(cose::kid1(&statement).unwrap(), vec![0]);
    let payload = cose::payload1(&statement).unwrap();
    let mut d = Decoder::new(&payload);
    assert_eq!(d.map().unwrap(), Some(2));
    assert_eq!(d.u8().unwrap(), 0);
    assert_eq!(hex::encode(d.bytes().unwrap()), owner.channel_id_hex());
    assert_eq!(d.u8().unwrap(), 1);
    assert_eq!(hex::encode(d.bytes().unwrap()), member.gate_token_hex());
    assert_eq!(
        owner.gate_token_hash_hex().unwrap(),
        hex::encode(Sha256::digest(
            hex::decode(member.gate_token_hex()).unwrap()
        ))
    );
}

#[test]
fn self_grant_is_one_root_link_with_every_action() {
    let (owner, _) = group_chat();
    let chain = split(&owner.self_grant());
    assert_eq!(chain.len(), 1);
    let payload = cose::payload1(&chain[0]).unwrap();
    let claims = cahoots_objects::cwt::Claims::decode(&payload).unwrap();
    assert_eq!(claims.depth, 2);
    assert_eq!(claims.log_not_after, Some(u64::MAX));
    let detail = &claims.authorization_details[0];
    assert_eq!(detail.budget, Some(0));
    assert_eq!(
        detail.actions.as_deref().unwrap(),
        ["read", "post", "admit", "govern"]
    );
}

#[test]
fn policy_summary_for_each_preset() {
    let c = circle();
    let summary = |preset: u8| {
        let owner = c.channel_owner("P", preset, 1).unwrap();
        policy_summary(&owner.policy()).unwrap()
    };
    assert_eq!(
        summary(0),
        "anyone with the link can read; only the owner can post"
    );
    assert_eq!(
        summary(1),
        "anyone with the link can read; the owner can post, with 1 approval"
    );
    assert_eq!(summary(2), "anyone with the link can read and post");
    assert_eq!(
        summary(3),
        "anyone with the link can read; members the owner lets in can post"
    );
    assert_eq!(
        summary(4),
        "members the owner lets in can read; only the owner can post"
    );
    assert_eq!(c.channel_owner("P", 5, 1).unwrap_err(), "preset");
    assert_eq!(c.channel_owner("", 2, 1).unwrap_err(), "name");
}

fn split_policy_body(body: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut d = Decoder::new(body);
    assert_eq!(d.array().unwrap(), Some(2));
    let signed = d.bytes().unwrap().to_vec();
    let n = d.array().unwrap().unwrap();
    let approvals = (0..n).map(|_| d.bytes().unwrap().to_vec()).collect();
    assert_eq!(d.position(), body.len());
    (signed, approvals)
}

#[test]
fn set_policy_returns_the_post_body_and_a_matching_control() {
    let (mut owner, link) = group_chat();
    let manifest = owner.manifest();
    let mut member = joined(&owner, &link);
    let first = owner.policy_id_hex();
    let initial = split_policy_body(&owner.policy_body());
    assert_eq!(initial.0, owner.policy());
    assert!(initial.1.is_empty());

    let body = owner.set_policy(1, 2).unwrap();
    let (signed, approvals) = split_policy_body(&body);
    assert!(approvals.is_empty());
    assert_eq!(signed, owner.policy());
    assert_ne!(owner.policy_id_hex(), first);
    // The channel object is reissued: it names the new policy at the policy's log.
    let object = owner.channel_object();
    let json = verify_channel_object(&object, &manifest).unwrap();
    assert!(json.contains(&format!("\"policy_id\":\"{}\"", owner.policy_id_hex())));
    assert!(json.contains(&format!("\"channel_id\":\"{}\"", owner.channel_id_hex())));
    assert!(json.contains("\"name\":\"Garden\""));
    let claims = cahoots_channel::ChannelObject::decode(&cose::payload1(&object).unwrap()).unwrap();
    let policy =
        cahoots_channel::Policy::decode(&cose::payload1(&owner.policy()).unwrap()).unwrap();
    assert_eq!(claims.created_log, policy.log);
    assert_eq!(policy.log, 1);
    assert_eq!(
        policy_summary(&signed).unwrap(),
        "anyone with the link can read; the owner can post, with 2 approvals"
    );
    let control = owner.policy_change_control().unwrap();
    let got = member
        .receive(&b64_encode(&control), &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(
        got.control_json(),
        format!(
            "{{\"kind\":\"policy\",\"policy_id\":\"{}\"}}",
            owner.policy_id_hex()
        )
    );

    // A policy whose govern rule needs two seats cannot be replaced alone in this version.
    let mut two = circle().channel_owner("Two", GROUP_CHAT, 2).unwrap();
    assert_eq!(two.set_policy(2, 0).unwrap_err(), "needs-approvals");
}

#[test]
fn announcements_are_ok_only_with_a_policy() {
    let (owner, link) = group_chat();
    let manifest = owner.manifest();
    let mut member = joined(&owner, &link);
    // A text post is never an announcement.
    let wire = owner.member().send(b"plain").unwrap();
    let got = member
        .receive(&wire, &owner.self_grant(), &manifest)
        .unwrap();
    assert!(!got.announcement_ok());
    member.set_policy(&owner.policy()).unwrap();
    assert!(member.set_policy(b"not a policy").is_err());
}

#[test]
fn owner_and_member_state_survive_export_and_import() {
    let (mut owner, link) = group_chat();
    let manifest = owner.manifest();
    let mut member = joined(&owner, &link);
    let request = member.join_request("Ev", "read,post").unwrap();
    let chain = split(&owner.admit(&request, "read,post").unwrap())[0].clone();
    member.set_grant(&chain).unwrap();
    let first = owner.member().current_epoch_hex().unwrap();

    let saved = owner.export();
    let mut owner = ChannelOwner::import(&saved).unwrap();
    assert_eq!(owner.export(), saved);
    let mut member = ChannelMember::import(&member.export()).unwrap();
    assert_eq!(member.channel_id_hex(), owner.channel_id_hex());

    let wire = member.send(b"after restore").unwrap();
    assert_eq!(
        owner
            .member()
            .receive(&wire, &chain, &manifest)
            .unwrap()
            .text(),
        b"after restore"
    );
    // The epoch counter came back: the next epoch is one past the first.
    owner.rotate().unwrap();
    let second = owner.member().current_epoch_hex().unwrap();
    assert_eq!(&second[..32], &first[..32]);
    let n = |h: &str| u32::from_le_bytes(hex::decode(&h[32..]).unwrap().try_into().unwrap());
    assert_eq!(n(&second), n(&first) + 1);
    // The admitted member is still tracked, so it gets a key deposit beside the link deposit.
    let deposits = split(&owner.deposits());
    assert_eq!(deposits.len(), 2);
    assert!(deposits.iter().any(|d| member
        .install_from_key_deposit(d, &owner.self_grant(), &owner.manifest())
        .is_ok()));
    assert_eq!(member.current_epoch_hex().unwrap(), second);

    assert!(ChannelOwner::import(&saved[..saved.len() - 1]).is_err());
    assert!(ChannelMember::import(b"\xa0").is_err());
}

#[test]
fn a_circle_without_a_seat_key_cannot_own_a_channel() {
    let saved = circle().export();
    // The version 3 layout: the version 4 and 5 trailers taken off.
    let v4 = &saved[..saved.len() - 33];
    let mlen = (0..v4.len() - 38)
        .find(|&m| {
            let at = v4.len() - m - 4;
            u32::from_le_bytes(v4[at..at + 4].try_into().unwrap()) as usize == m
        })
        .unwrap();
    let mut v3 = v4[..v4.len() - (33 + 1 + 4 + mlen)].to_vec();
    v3[0] = 3;
    let old = Circle::import(&v3).unwrap();
    assert!(!old.has_seat_key());
    assert_eq!(old.channel_owner("X", 2, 1).unwrap_err(), "no-seat-key");
}

#[test]
fn an_announcements_channel_admits_readers_but_no_posters() {
    for (preset, post_ok) in [(0u8, false), (1, false), (3, true)] {
        let mut owner = circle().channel_owner("News", preset, 1).unwrap();
        let link = owner.link_url("relay.example").unwrap();
        owner.rotate().unwrap();
        let member = joined(&owner, &link);
        let request = member.join_request("Fi", "read,post").unwrap();
        let got = owner.admit(&request, "read,post");
        if post_ok {
            assert_eq!(split(&got.unwrap()).len(), 3, "preset {preset}");
        } else {
            assert_eq!(got.unwrap_err(), "policy", "preset {preset}");
        }
        assert_eq!(split(&owner.admit(&request, "read").unwrap()).len(), 3);
    }
}

/// A CBOR array of byte strings.
fn array(items: &[&[u8]]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.array(items.len() as u64).unwrap();
    for item in items {
        e.bytes(item).unwrap();
    }
    e.into_writer()
}

#[test]
fn an_announcement_is_ok_with_the_quorum_of_seat_approvals() {
    let (a, b) = circles();
    // Announcements with approvals, sized by two seats: each announcement needs two.
    let mut owner = a.channel_owner("News", 1, 2).unwrap();
    let other_seat = b.channel_owner("Other", 1, 2).unwrap();
    let manifest = owner.manifest();
    assert_eq!(
        policy_summary(&owner.policy()).unwrap(),
        "anyone with the link can read; the owner can post, with 2 approvals"
    );
    let link = owner.link_url("relay.example").unwrap();
    owner.rotate().unwrap();
    let mut member = joined(&owner, &link);
    member.set_policy(&owner.policy()).unwrap();
    let mut no_policy = joined(&owner, &link);
    let mut poster = owner.member();
    assert_eq!(poster.announcement_digest().unwrap_err(), "no-proposal");
    assert_eq!(
        poster.send_proposed(&array(&[])).unwrap_err(),
        "no-proposal"
    );

    let mut announce = |text: &[u8], seats: &[&ChannelOwner]| {
        let body = poster.propose_announcement(text).unwrap();
        let digest = poster.announcement_digest().unwrap();
        assert_eq!(digest, hex::encode(Sha256::digest(&body)));
        let approvals: Vec<Vec<u8>> = seats
            .iter()
            .map(|o| o.approve_post(&digest, "News", "first line", 0).unwrap())
            .collect();
        let refs: Vec<&[u8]> = approvals.iter().map(Vec::as_slice).collect();
        let wire = poster.send_proposed(&array(&refs)).unwrap();
        assert_eq!(poster.announcement_digest().unwrap_err(), "no-proposal");
        wire
    };

    let two = announce(b"market on sunday", &[&owner, &other_seat]);
    let got = member
        .receive(&two, &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(got.msg_type(), 2);
    assert_eq!(got.text(), b"market on sunday");
    assert!(got.announcement_ok());
    // A member without the policy does not count approvals.
    let got = no_policy
        .receive(&two, &owner.self_grant(), &manifest)
        .unwrap();
    assert!(!got.announcement_ok());

    let one = announce(b"one seat only", &[&owner]);
    let got = member
        .receive(&one, &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(got.text(), b"one seat only");
    assert!(!got.announcement_ok());

    let same_seat_twice = announce(b"same seat twice", &[&owner, &owner]);
    let got = member
        .receive(&same_seat_twice, &owner.self_grant(), &manifest)
        .unwrap();
    assert!(!got.announcement_ok());

    assert_eq!(
        owner.approve_post("00", "News", "x", 0).unwrap_err(),
        "digest"
    );
}

#[test]
fn another_seat_approves_through_its_circle() {
    let (a, b) = circles();
    let mut owner = a.channel_owner("News", 1, 2).unwrap();
    let manifest = owner.manifest();
    let link = owner.link_url("relay.example").unwrap();
    owner.rotate().unwrap();
    let mut member = joined(&owner, &link);
    member.set_policy(&owner.policy()).unwrap();
    let mut poster = owner.member();
    poster.propose_announcement(b"from two seats").unwrap();
    let digest = poster.announcement_digest().unwrap();
    let mine = owner
        .approve_post(&digest, "News", "from two seats", 0)
        .unwrap();
    let theirs = b
        .approve_channel_post(&owner.policy(), &digest, "News", "from two seats", 0)
        .unwrap();
    let wire = poster.send_proposed(&array(&[&mine, &theirs])).unwrap();
    let got = member
        .receive(&wire, &owner.self_grant(), &manifest)
        .unwrap();
    assert!(got.announcement_ok());
    assert_eq!(
        b.approve_channel_post(b"junk", &digest, "News", "x", 0)
            .unwrap_err(),
        "signature"
    );
    assert_eq!(
        b.approve_channel_post(&owner.policy(), "00", "News", "x", 0)
            .unwrap_err(),
        "digest"
    );
}

#[test]
fn the_link_id_and_a_request_summary_for_the_app() {
    let (owner, link) = group_chat();
    let m = ChannelMember::from_link(&link).unwrap();
    let secret = b64_decode(link.rsplit('/').next().unwrap()).unwrap();
    let mut h = Sha256::new();
    h.update(b"cahoots/link/id/v1");
    h.update(&secret);
    assert_eq!(m.link_id_hex(), hex::encode(h.finalize()));
    // The link deposit is addressed to that id.
    let first = split(&owner.deposits()).remove(0);
    assert!(hex::encode(&first).contains(&m.link_id_hex()));
    let req = m.join_request("Rosa", "read,post").unwrap();
    let json = cahoots_wasm::channel::join_request_summary(&req).unwrap();
    assert_eq!(
        json,
        format!(
            "{{\"channel_id\":\"{}\",\"kid\":\"{}\",\"name\":\"Rosa\",\"actions\":\"read,post\"}}",
            owner.channel_id_hex(),
            m.my_kid_hex()
        )
    );
    let mut bad = req.clone();
    let n = bad.len();
    bad[n - 1] ^= 1;
    assert_eq!(
        cahoots_wasm::channel::join_request_summary(&bad).unwrap_err(),
        "signature"
    );
}

#[test]
fn a_new_link_after_retire_admits_under_the_same_policy() {
    let (mut owner, old_link) = group_chat();
    let policy_before = owner.policy();
    let old = ChannelMember::from_link(&old_link).unwrap();
    owner.retire_link().unwrap();
    let new_link = owner.link_url("relay.example").unwrap();
    assert_ne!(new_link, old_link);
    assert_eq!(owner.policy(), policy_before, "no policy change");
    let probe = ChannelMember::from_link(&new_link).unwrap();
    assert_ne!(probe.gate_token_hex(), old.gate_token_hex());
    // The gate statement names the new token.
    let payload = cose::payload1(&owner.gate_registration().unwrap()).unwrap();
    let mut d = Decoder::new(&payload);
    d.map().unwrap();
    d.u8().unwrap();
    d.bytes().unwrap();
    d.u8().unwrap();
    assert_eq!(hex::encode(d.bytes().unwrap()), probe.gate_token_hex());
    owner.rotate().unwrap();
    let fresh = joined(&owner, &new_link);
    assert!(fresh.current_epoch_hex().is_some());
    // The new link's token is evidence; the old one's is not.
    let req = fresh.join_request("Cy", "read,post").unwrap();
    assert_eq!(split(&owner.admit(&req, "read,post").unwrap()).len(), 3);
    let stale = old.join_request("Old", "read,post").unwrap();
    assert_eq!(owner.admit(&stale, "read,post").unwrap_err(), "evidence");
    // A member holding the old link takes the new one and keeps its key.
    let mut moved = ChannelMember::from_link(&old_link).unwrap();
    let kid = moved.my_kid_hex();
    moved.set_link(&new_link).unwrap();
    assert_eq!(moved.my_kid_hex(), kid);
    assert_eq!(moved.gate_token_hex(), fresh.gate_token_hex());
    let other = circle().channel_owner("Else", GROUP_CHAT, 1).unwrap();
    let elsewhere = format!(
        "https://cahoots.network/app/#/ch/relay.example/{}/{}",
        other.channel_id_hex(),
        new_link.rsplit('/').next().unwrap()
    );
    assert_eq!(moved.set_link(&elsewhere).unwrap_err(), "wrong-channel");
    assert_eq!(moved.set_link("nonsense").unwrap_err(), "link");
}

#[test]
fn who_may_ask_to_post_follows_the_post_rule() {
    let c = circle();
    for (preset, may) in [(0u8, false), (1, false), (2, true), (3, true), (4, false)] {
        let mut owner = c.channel_owner("P", preset, 1).unwrap();
        let link = owner.link_url("relay.example").unwrap();
        let mut m = ChannelMember::from_link(&link).unwrap();
        assert!(!m.may_ask_to_post(), "no policy held yet");
        m.set_policy(&owner.policy()).unwrap();
        assert_eq!(m.may_ask_to_post(), may, "preset {preset}");
    }
}

/// A join request made by hand for `key`, with `token` as its evidence: what
/// `ChannelMember::join_request` makes, for a key the test holds.
fn request_for(
    key: &cahoots_core::seat::SeatKey,
    channel: &[u8],
    xwing_pk: &[u8],
    token: &[u8],
) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.map(6).unwrap();
    e.u8(0).unwrap().bytes(channel).unwrap();
    e.u8(1).unwrap().bytes(&key.public().0).unwrap();
    e.u8(2).unwrap().bytes(xwing_pk).unwrap();
    e.u8(3).unwrap().str("read,post").unwrap();
    e.u8(4).unwrap().str("Mallory").unwrap();
    e.u8(5).unwrap().bytes(token).unwrap();
    let kid: [u8; 32] = Sha256::digest(key.public().0).into();
    cose::sign1(
        key,
        &kid,
        &e.into_writer(),
        cahoots_wasm::channel::JOIN_DOMAIN,
        false,
    )
}

#[test]
fn deposits_signed_by_a_key_without_govern_are_refused() {
    use cahoots_channel::deposit::peek_signed;
    let (mut owner, link) = group_chat();
    let manifest = owner.manifest();
    let mut m = ChannelMember::from_link(&link).unwrap();
    let real = m.join_request("Cee", "read,post").unwrap();
    let xwing_pk = cahoots_channel::member::JoinRequest::parse(&real)
        .unwrap()
        .xwing_pk;

    // Mallory joins by the link and is let in to post: a real grant, without govern.
    let mallory = cahoots_core::seat::SeatKey::from_seed(&[0x4d; 32]).unwrap();
    let channel = hex::decode(owner.channel_id_hex()).unwrap();
    let token = hex::decode(m.gate_token_hex()).unwrap();
    let req = request_for(&mallory, &channel, &xwing_pk, &token);
    let mallory_chain = split(&owner.admit(&req, "read,post").unwrap())[0].clone();

    // A link deposit sealed with the right secret, and a key deposit sealed to the member, each
    // signed again by Mallory.
    let (_, link_dep) = peek_signed(&split(&owner.deposits())[0]).unwrap();
    let kid = m.my_kid_hex();
    let (_, key_dep) = peek_signed(&owner.deposit_for_key(&kid, &xwing_pk).unwrap()).unwrap();
    let forged_link = link_dep.sign(&mallory);
    let forged_key = key_dep.sign(&mallory);
    assert_eq!(
        m.install_from_link_deposit(&forged_link, &mallory_chain, &manifest)
            .unwrap_err(),
        "refused:action"
    );
    assert_eq!(
        m.install_from_key_deposit(&forged_key, &mallory_chain, &manifest)
            .unwrap_err(),
        "refused:action"
    );
    // Shown with the owner's chain instead, the kid is not the chain's key.
    assert_eq!(
        m.install_from_link_deposit(&forged_link, &owner.self_grant(), &manifest)
            .unwrap_err(),
        "signature"
    );
    assert!(m.current_epoch_hex().is_none());

    // deposit_meta names the signer to fetch the chain by, and the id a rotation names.
    let good = split(&owner.deposits())[0].clone();
    let meta = cahoots_wasm::channel::deposit_meta(&good).unwrap();
    assert_eq!(
        meta,
        format!(
            "{{\"id\":\"{}\",\"kid\":\"{}\"}}",
            hex::encode(link_dep.id()),
            owner.member().my_kid_hex()
        )
    );
    m.install_from_link_deposit(&good, &owner.self_grant(), &manifest)
        .unwrap();
    assert_eq!(m.current_epoch_hex(), owner.member().current_epoch_hex());
}

#[test]
fn permitted_is_what_the_link_proves_and_the_owner_picks_the_rest() {
    let (mut owner, link) = group_chat();
    let m = ChannelMember::from_link(&link).unwrap();
    let req = m.join_request("Cee", "read,post,admit,govern").unwrap();
    assert_eq!(owner.permitted(&req).unwrap(), "read,post");
    assert_eq!(owner.admit(&req, "govern").unwrap_err(), "policy");
    assert_eq!(owner.admit(&req, "read,admit").unwrap_err(), "policy");
    assert_eq!(split(&owner.admit(&req, "read,post").unwrap()).len(), 3);

    // Members post: the link grants read, post is by hand; closed: nothing by link.
    for (preset, want) in [(3u8, "read"), (4, "")] {
        let mut o = circle().channel_owner("Inner", preset, 1).unwrap();
        let link = o.link_url("relay.example").unwrap();
        o.rotate().unwrap();
        let r = ChannelMember::from_link(&link)
            .unwrap()
            .join_request("Dee", "read,post")
            .unwrap();
        assert_eq!(o.permitted(&r).unwrap(), want, "preset {preset}");
    }
    // Closed: the owner lets a reader in by hand.
    let mut o = circle().channel_owner("Inner", 4, 1).unwrap();
    let link = o.link_url("relay.example").unwrap();
    o.rotate().unwrap();
    let r = ChannelMember::from_link(&link)
        .unwrap()
        .join_request("Dee", "read,post")
        .unwrap();
    assert_eq!(o.admit(&r, "read,post").unwrap_err(), "policy");
    assert_eq!(split(&o.admit(&r, "read").unwrap()).len(), 3);
}

#[test]
fn a_retired_link_waits_for_the_relay_to_confirm() {
    let (mut owner, _) = group_chat();
    let hash = owner.gate_token_hash_hex().unwrap();
    let statement = owner.retire_link().unwrap();
    assert!(owner.gate_token_hash_hex().is_none());
    let pending = format!(
        "[{{\"hash\":\"{hash}\",\"statement\":\"{}\"}}]",
        b64_encode(&statement)
    );
    assert_eq!(owner.pending_retire_json(), pending);
    let mut back = ChannelOwner::import(&owner.export()).unwrap();
    assert_eq!(back.pending_retire_json(), pending);
    back.retire_confirmed(&hash).unwrap();
    assert_eq!(back.pending_retire_json(), "[]");
    assert_eq!(back.retire_confirmed("zz").unwrap_err(), "hash");
}

#[test]
fn may_post_follows_the_grants_actions() {
    let (mut owner, link) = group_chat();
    let mut m = ChannelMember::from_link(&link).unwrap();
    assert!(!m.may_post());
    let req = m.join_request("Cee", "read,post").unwrap();
    let read = split(&owner.admit(&req, "read").unwrap())[0].clone();
    m.set_grant(&read).unwrap();
    assert!(!m.may_post());
    let post = split(&owner.admit(&req, "read,post").unwrap())[0].clone();
    m.set_grant(&post).unwrap();
    assert!(m.may_post());
}
