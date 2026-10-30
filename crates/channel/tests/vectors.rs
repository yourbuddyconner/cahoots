//! Reproduces `crates/vectors/channel.json`: every recorded byte string is decoded, verified
//! against the manifest rebuilt from the recorded seat seeds, and encoded again to the same hex.

use cahoots_channel::control::Control;
use cahoots_channel::deposit::{self, Deposit, KIND_KEY, KIND_LINK};
use cahoots_channel::grant::{self, verify_chain};
use cahoots_channel::keys::{chain_init, epoch_key, kid_of, EpochId};
use cahoots_channel::link::{Link, LinkSecret};
use cahoots_channel::message::{self, body_digest};
use cahoots_channel::object::ChannelObject;
use cahoots_channel::policy::{self, Policy, Preset};
use cahoots_channel::quorum::{check_quorum, PROFILE_POST};
use cahoots_channel::session::ChannelSession;
use cahoots_channel::Action;
use cahoots_core::manifest::{Manifest, Seat};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use serde_json::Value;
use std::collections::HashMap;

fn load() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../vectors/channel.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn bytes(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn arr<const N: usize>(v: &Value) -> [u8; N] {
    bytes(v).try_into().unwrap()
}

fn key(seed: &Value) -> SeatKey {
    SeatKey::from_seed(&arr(seed)).unwrap()
}

/// The three-seat circle of the objects vectors, from the recorded seat seeds.
fn manifest(v: &Value) -> (Manifest, Vec<SeatKey>) {
    let params = CircleParams::new(3, 192, "objects", false).unwrap();
    let keys: Vec<SeatKey> = v["seat_seeds"]
        .as_array()
        .unwrap()
        .iter()
        .map(key)
        .collect();
    let tiers = [0u8, 1, 0];
    let manifest = Manifest {
        circle_id: arr(&v["circle_id"]),
        seats: keys
            .iter()
            .enumerate()
            .map(|(i, k)| Seat {
                seat: i as u8,
                key: k.public(),
                tier: tiers[i],
                evidence: Vec::new(),
            })
            .collect(),
        floor: Manifest::default_floor(&params),
        params,
        genesis: 0,
    };
    (manifest, keys)
}

#[test]
fn channel_object_reproduces() {
    let v = load();
    let (manifest, keys) = manifest(&v);
    let c = &v["channel_object"];
    let signed = bytes(&c["signed"]);
    let (seat, object) = ChannelObject::verify(&signed, &manifest).unwrap();
    assert_eq!(seat, v["seat"].as_u64().unwrap() as u8);
    assert_eq!(object.name, c["name"].as_str().unwrap());
    assert_eq!(object.channel_id, arr::<16>(&c["channel_id"]));
    assert_eq!(object.policy_id, arr::<32>(&c["policy_id"]));
    assert_eq!(object.encode(), bytes(&c["payload"]));
    assert_eq!(object.sign(&keys[usize::from(seat)], seat), signed);
}

#[test]
fn policies_reproduce() {
    let v = load();
    let (manifest, keys) = manifest(&v);
    let p = &v["policy_presets"];
    let circle: [u8; 16] = arr(&v["circle_id"]);
    let channel_id: [u8; 16] = arr(&v["channel_object"]["channel_id"]);
    let presets = [
        ("announcements", Preset::Announcements),
        ("announcements_quorum_2", Preset::AnnouncementsQuorum(2)),
        ("group_chat", Preset::GroupChat),
        ("members_post", Preset::MembersPost),
        ("closed", Preset::Closed),
    ];
    let recorded = p["presets"].as_array().unwrap();
    assert_eq!(recorded.len(), presets.len());
    let mut by_name = HashMap::new();
    for (case, (name, preset)) in recorded.iter().zip(presets) {
        assert_eq!(case["preset"].as_str().unwrap(), name);
        let policy = Policy::preset(
            preset,
            circle,
            channel_id,
            arr(&p["link_id"]),
            p["seats"].as_u64().unwrap() as u8,
            p["log"].as_u64().unwrap(),
            arr(&p["nonce"]),
        );
        let payload = bytes(&case["payload"]);
        assert_eq!(Policy::decode(&payload).unwrap(), policy);
        assert_eq!(policy.encode(), payload);
        by_name.insert(name, policy);
    }

    let s = &v["policy_signed"];
    let signed = bytes(&s["signed"]);
    let (seat, policy) = Policy::verify(&signed, &manifest).unwrap();
    assert_eq!(policy, by_name[s["preset"].as_str().unwrap()]);
    assert_eq!(policy.encode(), bytes(&s["payload"]));
    assert_eq!(policy.sign(&keys[usize::from(seat)], seat), signed);
    assert_eq!(policy::id(&signed), arr::<32>(&s["id"]));
    assert_eq!(
        v["channel_object"]["policy_id"].as_str(),
        s["id"].as_str(),
        "the channel object names the signed policy"
    );
}

#[test]
fn link_reproduces() {
    let v = load();
    let l = &v["link"];
    let secret = LinkSecret(arr(&l["secret"]));
    let channel_id: [u8; 16] = arr(&l["channel_id"]);
    assert_eq!(secret.id(), arr::<32>(&l["id"]));
    assert_eq!(*secret.seal_key(&channel_id), arr::<32>(&l["seal_key"]));
    assert_eq!(secret.gate_token(&channel_id), arr::<32>(&l["gate_token"]));
    let link = Link {
        relay: l["relay"].as_str().unwrap().to_string(),
        channel_id,
        secret: Some(secret),
    };
    let url = l["url"].as_str().unwrap();
    assert_eq!(link.to_url(), url);
    assert_eq!(Link::parse(url).unwrap(), link);
}

#[test]
fn epoch_reproduces() {
    let v = load();
    let e = &v["epoch"];
    let id = EpochId::decode(&bytes(&e["epoch_id"])).unwrap();
    assert_eq!(id.encode().to_vec(), bytes(&e["epoch_id"]));
    let ekey = epoch_key(&id, &arr(&e["root"]));
    assert_eq!(*ekey, arr::<32>(&e["epoch_key"]));
    let seeds = [
        &v["message_text"]["member_seed"],
        &v["message_control_rotation"]["member_seed"],
    ];
    let chains = e["chains"].as_array().unwrap();
    assert_eq!(chains.len(), seeds.len());
    for (chain, seed) in chains.iter().zip(seeds) {
        let kid = kid_of(&key(seed).public());
        assert_eq!(kid, arr::<32>(&chain["kid"]));
        assert_eq!(*chain_init(&ekey, &kid), arr::<32>(&chain["chain"]));
    }
}

#[test]
fn deposits_reproduce() {
    let v = load();
    let ekey: [u8; 32] = arr(&v["epoch"]["epoch_key"]);

    let d = &v["deposit_link"];
    let dep = Deposit::decode(&bytes(&d["deposit"])).unwrap();
    assert_eq!(dep.kind, KIND_LINK);
    assert_eq!(dep.channel_id, arr::<16>(&d["channel_id"]));
    assert_eq!(dep.epoch_id.encode().to_vec(), bytes(&d["epoch_id"]));
    assert_eq!(dep.to, arr::<32>(&d["to"]));
    assert_eq!(dep.sealed, bytes(&d["sealed"]));
    assert_eq!(dep.id(), arr::<32>(&d["id"]));
    let secret = LinkSecret(arr(&v["link"]["secret"]));
    assert_eq!(*deposit::open_with_link(&dep, &secret).unwrap(), ekey);
    // The link deposit is deterministic: sealing again gives the same bytes.
    assert_eq!(deposit::seal_for_link(&ekey, &secret, &dep.epoch_id), dep);

    // The key deposit is randomized, so it is checked by opening it.
    let k = &v["deposit_key"];
    let d = &k["deposit"];
    let dep = Deposit::decode(&bytes(&d["deposit"])).unwrap();
    assert_eq!(dep.kind, KIND_KEY);
    assert_eq!(dep.epoch_id.encode().to_vec(), bytes(&d["epoch_id"]));
    assert_eq!(dep.sealed, bytes(&d["sealed"]));
    assert_eq!(dep.encode(), bytes(&d["deposit"]));
    assert_eq!(dep.id(), arr::<32>(&d["id"]));
    assert_eq!(dep.to, kid_of(&key(&k["member_seed"]).public()));
    assert_eq!(dep.to, arr::<32>(&d["to"]));
    let opened = deposit::open_with_key(&dep, &bytes(&k["recipient_secret_key"])).unwrap();
    assert_eq!(*opened, ekey);
    assert_eq!(*opened, arr::<32>(&k["epoch_key"]));
}

#[test]
fn grant_chain_reproduces() {
    let v = load();
    let (manifest, keys) = manifest(&v);
    let g = &v["grant_chain"];
    let channel_id: [u8; 16] = arr(&v["channel_object"]["channel_id"]);
    let root = bytes(&g["root"]);
    let delegated = bytes(&g["delegated"]);
    let leaf = verify_chain(&[&root, &delegated], &manifest, &channel_id).unwrap();
    let root_grant = verify_chain(&[&root], &manifest, &channel_id).unwrap();
    assert_eq!(root_grant.id, arr::<16>(&g["root_id"]));
    assert_eq!(leaf.id, arr::<16>(&g["delegated_id"]));
    assert_eq!(
        root_grant.actions,
        [Action::Read, Action::Post, Action::Admit]
    );
    assert_eq!(
        (root_grant.budget, root_grant.not_after, root_grant.depth),
        (0, 1000, 1)
    );
    assert_eq!(leaf.actions, [Action::Read, Action::Post]);
    assert_eq!((leaf.budget, leaf.not_after, leaf.depth), (10, 500, 0));

    let seat = v["seat"].as_u64().unwrap() as u8;
    let delegate = key(&g["delegate_seed"]);
    assert_eq!(root_grant.grantee, delegate.public());
    assert_eq!(leaf.grantee, key(&g["leaf_seed"]).public());
    assert_eq!(
        grant::sign_root(&root_grant.claims, &keys[usize::from(seat)], seat),
        root
    );
    assert_eq!(grant::sign_delegated(&leaf.claims, &delegate), delegated);
}

/// Opens every message case in order, one receiving session per sender.
#[test]
fn messages_reproduce() {
    let v = load();
    let (manifest, _) = manifest(&v);
    let channel_id: [u8; 16] = arr(&v["channel_object"]["channel_id"]);
    let mut sessions: HashMap<Vec<u8>, ChannelSession> = HashMap::new();
    let mut senders: HashMap<Vec<u8>, ChannelSession> = HashMap::new();
    for name in [
        "message_text",
        "message_control_rotation",
        "message_announcement",
        "revocation",
    ] {
        let m = &v[name];
        let sender = key(&m["member_seed"]);
        let signer = sender.public();
        assert_eq!(signer.0.to_vec(), bytes(&m["member_public"]));
        let wire = bytes(&m["wire"]);

        let (header, _, approvals) = message::verify_signature(&wire, &signer).unwrap();
        assert_eq!(header.kid, arr::<32>(&m["kid"]));
        assert_eq!(header.msg_type as u64, m["msg_type"].as_u64().unwrap());
        assert_eq!(u64::from(header.counter), m["counter"].as_u64().unwrap());
        assert_eq!(header.epoch_id.encode().to_vec(), bytes(&m["epoch_id"]));
        let recorded: Vec<Vec<u8>> = m["approvals"]
            .as_array()
            .unwrap()
            .iter()
            .map(bytes)
            .collect();
        assert_eq!(approvals, recorded);

        let session = sessions.entry(bytes(&m["member_seed"])).or_insert_with(|| {
            let mut s = ChannelSession::new(channel_id, key(&m["member_seed"]));
            let id = EpochId::decode(&bytes(&m["epoch_id"])).unwrap();
            let ekey = epoch_key(&id, &arr(&v["epoch"]["root"]));
            assert_eq!(*ekey, arr::<32>(&m["epoch_key"]));
            s.install_epoch(id, ekey).unwrap();
            s
        });
        let (header, plaintext, _) = session.receive(&wire, &signer).unwrap();
        assert_eq!(plaintext, bytes(&m["plaintext"]));

        // Sending the same plaintext as the same sender gives the same wire, byte for byte.
        let sender_session = senders.entry(bytes(&m["member_seed"])).or_insert_with(|| {
            let mut s = ChannelSession::new(channel_id, sender);
            let id = EpochId::decode(&bytes(&m["epoch_id"])).unwrap();
            s.install_epoch(id, epoch_key(&id, &arr(&v["epoch"]["root"])))
                .unwrap();
            s
        });
        assert_eq!(
            sender_session
                .send(header.msg_type, &plaintext, &approvals)
                .unwrap(),
            wire
        );

        match name {
            "message_control_rotation" => {
                let Control::Rotation { epoch_id, deposits } = Control::decode(&plaintext).unwrap()
                else {
                    panic!("not a rotation");
                };
                assert_eq!(epoch_id.epoch, 2);
                assert_eq!(
                    deposits,
                    [
                        arr::<32>(&v["deposit_link"]["id"]),
                        arr::<32>(&v["deposit_key"]["deposit"]["id"]),
                    ]
                );
                assert_eq!(Control::decode(&plaintext).unwrap().encode(), plaintext);
            }
            "message_announcement" => {
                let digest = body_digest(message::split(&wire).unwrap().0);
                assert_eq!(digest, arr::<32>(&m["body_digest"]));
                assert_eq!(m["profile"].as_str().unwrap(), PROFILE_POST);
                let quorum = m["quorum"].as_u64().unwrap() as u8;
                let assurance = m["assurance"].as_u64().unwrap() as u8;
                assert_eq!(
                    check_quorum(
                        &approvals,
                        &manifest,
                        &digest,
                        PROFILE_POST,
                        quorum,
                        assurance
                    )
                    .unwrap() as u64,
                    m["quorum_count"].as_u64().unwrap()
                );
                assert_eq!(m["policy_id"].as_str(), v["policy_signed"]["id"].as_str());
            }
            "revocation" => {
                let Control::Revocation { grant_id, reason } = Control::decode(&plaintext).unwrap()
                else {
                    panic!("not a revocation");
                };
                assert_eq!(grant_id, arr::<16>(&m["grant_id"]));
                assert_eq!(grant_id, arr::<16>(&v["grant_chain"]["delegated_id"]));
                assert_eq!(reason, m["reason"].as_str().unwrap());
                assert_eq!(Control::Revocation { grant_id, reason }.encode(), plaintext);
            }
            _ => {}
        }
    }
}
