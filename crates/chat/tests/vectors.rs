//! Chat vectors (docs/chat.md section 7), published to `crates/vectors/chat.json`.
//! Written if missing, compared if present. Delete and rerun after a deliberate change.

use cahoots_chat::{chain_init, epoch_key, open, parse, seal, step, EpochId, Header, MessageType};
use serde_json::{json, Value};

const CID: [u8; 16] = [
    0x63, 0x61, 0x68, 0x6f, 0x6f, 0x74, 0x73, 0x2d, 0x63, 0x68, 0x61, 0x74, 0x2d, 0x76, 0x65, 0x63,
];

#[allow(clippy::type_complexity)]
fn vectors() -> Value {
    let eid = EpochId {
        slice: 1,
        offset: 96,
    };
    let psk: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
    let ek = epoch_key(&CID, &eid, &psk);
    let mut senders = Vec::new();
    let mut messages = Vec::new();
    let plaintexts: [(u8, u32, &[u8]); 3] = [
        (0, 0, b"first from zero"),
        (2, 1, b"second from two, out of order"),
        (2, 0, b"first from two"),
    ];
    let mut chains: Vec<(u8, Vec<(Vec<u8>, Vec<u8>)>)> = Vec::new(); // (sender, [(msg_key, next_chain)])
    for s in [0u8, 2] {
        let c0 = chain_init(&ek, s);
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
            version: 2,
            circle_id: CID,
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
    json!({
        "description": "docs/chat.md v1. epoch_key = HKDF-SHA256(salt = \"cahoots/chat/epoch/v1\" || circle_id || slice || offset LE, ikm = psk, info = \"\"). chain_0[i] = HKDF-Expand(epoch_key, \"cahoots/chat/chain/v1\" || i); msg_key = Expand(chain, \"cahoots/chat/msg/v1\"); next = Expand(chain, \"cahoots/chat/next/v1\"). AEAD ChaCha20-Poly1305, nonce = sender || counter LE || 7 zero bytes, aad = header.",
        "circle_id": hex::encode(CID),
        "epoch_id": {"slice": eid.slice, "offset": eid.offset},
        "psk": hex::encode(psk),
        "epoch_key": hex::encode(*ek),
        "senders": senders,
        "messages": messages,
        "tampered": {"message": hex::encode(&tampered), "expected": "reject: tag"},
    })
}

#[test]
fn chat_vectors_are_stable() {
    let v = vectors();
    assert_eq!(v["epoch_key"].as_str().unwrap().len(), 64);
    // Pinned after first generation so an accidental derivation change is caught here too.
    assert_eq!(
        v["epoch_key"],
        "6e71df340bf3ed1ec3b269b39bca76043e752b62efdd94f88126fe628b569851"
    );
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
