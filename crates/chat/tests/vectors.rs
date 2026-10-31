//! Chat vectors (docs/chat.md section 8), published to `crates/vectors/chat.json`.
//! Written if missing, compared if present. Delete and rerun after a deliberate change.
//!
//! The root and circle id are those of `crates/vectors/root.json`. The generation case is
//! epoch 7 of generation 0, whose id and key root.json publishes. The old root-epoch case is
//! root epoch 7, one of root.json's chat key epochs, so its epoch key is the chat key there.
//! The bump is signed by seat 1 of the ceremony in `crates/vectors/ceremony.json`, the same
//! circle.

use cahoots_chat::{
    chain_init, epoch_key, epoch_key_in, generation_tag, open, parse, seal, sign_bump, step,
    verify_bump, EpochId, Header, Key, MessageType, CHAT_ROOT_VERSION, CHAT_VERSION,
};
use cahoots_core::manifest::Manifest;
use cahoots_core::root::{chat_gen_key, gen0_id};
use cahoots_core::seat::SeatKey;
use serde_json::{json, Value};

const EPOCH: u32 = 7;
/// The id of the generation the bump vector moves to.
const BUMP_ID: [u8; 32] = [0xb1; 32];

fn vectors_file(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../vectors")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn root_vectors() -> Value {
    vectors_file("root.json")
}

fn hex_array<const N: usize>(v: &Value) -> [u8; N] {
    hex::decode(v.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

/// Chain and message keys for senders 0 and 2 in one epoch, three messages, and a tampered one.
#[allow(clippy::type_complexity)]
fn message_set(cid: [u8; 16], eid: EpochId, ek: &Key, version: u8) -> (Value, Value, Value) {
    let ek: &[u8; 32] = ek;
    let mut senders = Vec::new();
    let mut messages = Vec::new();
    let plaintexts: [(u8, u32, &[u8]); 3] = [
        (0, 0, b"first from zero"),
        (2, 1, b"second from two, out of order"),
        (2, 0, b"first from two"),
    ];
    let mut chains: Vec<(u8, Vec<(Vec<u8>, Vec<u8>)>)> = Vec::new(); // (sender, [(msg_key, next_chain)])
    for s in [0u8, 2] {
        let c0 = chain_init(ek, s);
        let (mk0, c1) = step(&c0);
        let (mk1, c2) = step(&c1);
        senders.push(json!({
            "sender": s,
            "chain_0": hex::encode(*c0),
            "msg_key_0": hex::encode(*mk0),
            "chain_1": hex::encode(*c1),
            "msg_key_1": hex::encode(*mk1),
            "chain_2": hex::encode(*c2),
        }));
        chains.push((
            s,
            vec![(mk0.to_vec(), c1.to_vec()), (mk1.to_vec(), c2.to_vec())],
        ));
    }
    for (sender, counter, pt) in plaintexts {
        let mk: [u8; 32] = chains.iter().find(|(s, _)| *s == sender).unwrap().1[counter as usize]
            .0
            .clone()
            .try_into()
            .unwrap();
        let h = Header {
            version,
            circle_id: cid,
            epoch: eid,
            sender,
            counter,
            len: pt.len() as u32,
            msg_type: MessageType::Text,
        };
        let wire = seal(&h, &mk, pt);
        let (ph, body) = parse(&wire).unwrap();
        assert_eq!(open(&ph, &mk, body).unwrap(), pt);
        messages.push(json!({
            "sender": sender, "counter": counter,
            "plaintext": hex::encode(pt),
            "message": hex::encode(&wire),
        }));
    }
    let good = hex::decode(messages[0]["message"].as_str().unwrap()).unwrap();
    let mut tampered = good.clone();
    tampered[40] ^= 0x01;
    let mk0: [u8; 32] = chains[0].1[0].0.clone().try_into().unwrap();
    let (ph, body) = parse(&tampered).unwrap();
    assert!(open(&ph, &mk0, body).is_err());
    (
        json!(senders),
        json!(messages),
        json!({"message": hex::encode(&tampered), "expected": "reject: tag"}),
    )
}

fn vectors() -> Value {
    let rv = root_vectors();
    let root: [u8; 32] = hex_array(&rv["root"]);
    let cid: [u8; 16] = hex_array(&rv["circle_id"]);

    // Generation 0, epoch 7: the form every new message takes.
    let gen0 = gen0_id(&cid);
    let gen_key = chat_gen_key(&root, &cid, &gen0);
    let tag = generation_tag(&gen0);
    let gen_eid = EpochId::generation(tag, EPOCH as u16);
    let gen_ek = epoch_key_in(&gen_key, EPOCH as u16);
    let (g_senders, g_messages, g_tampered) = message_set(cid, gen_eid, &gen_ek, CHAT_VERSION);

    // Root epoch 7: the old form, which still opens.
    let eid = EpochId::root(EPOCH);
    let ek = epoch_key(&root, &cid, EPOCH);
    let (senders, messages, tampered) = message_set(cid, eid, &ek, CHAT_ROOT_VERSION);

    // A bump to BUMP_ID signed by seat 1 of the ceremony, and the same payload with the id
    // changed, which must fail.
    let cv = vectors_file("ceremony.json");
    assert_eq!(cv["circle_id"], rv["circle_id"]);
    let manifest =
        Manifest::decode(&hex::decode(cv["manifest"].as_str().unwrap()).unwrap()).unwrap();
    let seed: [u8; 32] = hex_array(&cv["seat_seeds"][1]);
    let seat_key = SeatKey::from_seed(&seed).unwrap();
    let payload = sign_bump(&seat_key, 1, &cid, &BUMP_ID).unwrap();
    assert_eq!(
        verify_bump(&payload, &cid, &manifest).unwrap(),
        (BUMP_ID, 1)
    );
    let mut bad_payload = payload.clone();
    bad_payload[0] ^= 1;
    assert!(verify_bump(&bad_payload, &cid, &manifest).is_err());
    let mut signed = b"cahoots/chat/bump/v1".to_vec();
    signed.extend_from_slice(&cid);
    signed.extend_from_slice(&BUMP_ID);
    let bump_key = chat_gen_key(&root, &cid, &BUMP_ID);

    json!({
        "description": "docs/chat.md section 1, generations. generation_id of generation 0 = SHA-256(\"cahoots/chat/gen0\" || circle_id). generation_key = HMAC-SHA256(root, \"chat/gen\" || circle_id || generation_id), as root.json publishes. epoch_key = HMAC-SHA256(generation_key, \"chat/epoch\" || n as u32 BE). epoch_id = 0xFE || tag || n as u16 LE, where tag is the first two bytes of SHA-256(generation_id). Byte 0 of every message is 2, the version the relay checks; a client reads version 3 from the 0xFE slice. Below the epoch key, chains, keys and the AEAD are as in the root-epoch case.",
        "generation": {
            "generation_id": hex::encode(gen0),
            "generation_key": hex::encode(gen_key),
            "tag": hex::encode(tag.to_le_bytes()),
            "epoch": EPOCH,
            "epoch_id": hex::encode(gen_eid.encode()),
            "epoch_key": hex::encode(*gen_ek),
            "senders": g_senders,
            "messages": g_messages,
            "tampered": g_tampered,
        },
        "bump": {
            "description": "A bump's payload is generation_id || COSE_Sign1, a detached ES256 signature (RFC 6979, so it reproduces) with kid [seat] and empty external AAD over signed = \"cahoots/chat/bump/v1\" || circle_id || generation_id. The seat key is seat_seeds[1] of ceremony.json, and the manifest is that ceremony's. generation_key is the key every root holder derives for it. tampered has one bit of the id flipped and MUST fail.",
            "seat": 1,
            "generation_id": hex::encode(BUMP_ID),
            "generation_key": hex::encode(bump_key),
            "signed": hex::encode(&signed),
            "payload": hex::encode(&payload),
            "tampered": {"payload": hex::encode(&bad_payload), "expected": "reject: signature"},
        },
        "root_epochs": {
        "description": "The old form, which no new message takes and which still opens. docs/chat.md v2, root epochs. epoch_id = 0xFF || n as u32 LE. epoch_key = HMAC-SHA256(root, \"chat\" || circle_id || n as u64 BE), the chat key of root.json, whose root and circle_id these are. chain_0[i] = HKDF-Expand(epoch_key, \"cahoots/chat/chain/v1\" || i); msg_key = Expand(chain, \"cahoots/chat/msg/v1\"); next = Expand(chain, \"cahoots/chat/next/v1\"). AEAD ChaCha20-Poly1305, nonce = sender || counter LE || 7 zero bytes, aad = header.",
        "root": hex::encode(root),
        "circle_id": hex::encode(cid),
        "epoch": EPOCH,
        "epoch_id": hex::encode(eid.encode()),
        "epoch_key": hex::encode(*ek),
        "senders": senders,
        "messages": messages,
        "tampered": tampered,
        },
    })
}

#[test]
fn chat_vectors_are_stable() {
    let v = vectors();
    // The generation key is the one root.json publishes for generation 0.
    let rv = root_vectors();
    assert_eq!(v["generation"]["generation_id"], rv["gen0_id"]);
    let published_gen = rv["generation_keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["generation_id"] == rv["gen0_id"])
        .unwrap()["key"]
        .clone();
    assert_eq!(v["generation"]["generation_key"], published_gen);
    let v = &v["root_epochs"];
    // The epoch key is the chat key root.json publishes for the same epoch.
    let published = root_vectors()["chat_keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["epoch"] == EPOCH)
        .unwrap()["key"]
        .clone();
    assert_eq!(v["epoch_key"], published);
}

#[test]
fn chat_vectors_file_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vectors/chat.json");
    let text = serde_json::to_string_pretty(&vectors()).unwrap() + "\n";
    match std::fs::read_to_string(&path) {
        Ok(existing) => assert_eq!(
            existing, text,
            "vectors/chat.json is stale; delete and rerun"
        ),
        Err(_) => std::fs::write(&path, text).unwrap(),
    }
}
