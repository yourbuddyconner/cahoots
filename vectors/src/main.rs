//! Generates `vectors/*.json` from `cahoots-core`. Every client (WASM, firmware, TVC app)
//! must reproduce these. Run `cargo run -p cahoots-vectors` to regenerate and
//! `cargo run -p cahoots-vectors -- --check` to verify the committed files are current.

use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::circle_id;
use cahoots_core::commit::commit;
use cahoots_core::frame;
use cahoots_core::ledger::{MemLedger, SpentLedger};
use cahoots_core::message::{self, MessageType};
use cahoots_core::pad::{bucket_pad, MemPadStorage, Pad, PadError, PadStorage};
use cahoots_core::params::CircleParams;
use serde_json::{json, Value};
use std::path::Path;

/// Deterministic filler. These are test vectors, not randomness.
fn fill(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 56) as u8
        })
        .collect()
}

fn ceremony_vectors() -> Value {
    let params = CircleParams::new(3, 192, "vectors", false).unwrap();
    let contributions: Vec<Vec<u8>> = (0..3).map(|i| fill(100 + i, 192)).collect();
    let mut devs: Vec<Ceremony> = contributions
        .iter()
        .enumerate()
        .map(|(i, c)| Ceremony::new(params.clone(), i as u8, c.clone()).unwrap())
        .collect();
    let commits: Vec<[u8; 32]> = devs.iter().map(|d| d.my_commitment()).collect();
    for d in devs.iter_mut() {
        for (j, c) in commits.iter().enumerate() {
            d.add_commitment(j as u8, *c).unwrap();
        }
        for (j, r) in contributions.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
        assert_eq!(d.phase(), Phase::Complete);
    }
    let result = devs.pop().unwrap().finish().unwrap();
    let mut expected = vec![0u8; 192];
    for c in &contributions {
        for (e, b) in expected.iter_mut().zip(c) {
            *e ^= b;
        }
    }
    assert_eq!(result.pad.as_slice(), expected.as_slice());

    // Mismatch abort: member 1 reveals bytes that differ from its commitment.
    let mut victim = Ceremony::new(params.clone(), 0, contributions[0].clone()).unwrap();
    for (j, c) in commits.iter().enumerate() {
        victim.add_commitment(j as u8, *c).unwrap();
    }
    let mut bad = contributions[1].clone();
    bad[0] ^= 0x01;
    let err = victim.add_reveal(1, &bad).unwrap_err();
    assert_eq!(err, CeremonyError::CommitMismatch(1));
    assert_eq!(victim.phase(), Phase::Aborted);

    json!({
        "description": "Commit-then-reveal ceremony for n=3, L=192. commit_i = SHA-256(\"cahoots/commit/v1\" || params || i || R_i). pad = XOR of all R_i.",
        "params": {
            "n": params.n, "pad_len": params.pad_len, "name": params.name, "agent_seat": params.agent_seat,
            "encoded": hex::encode(params.encode()),
        },
        "contributions": contributions.iter().map(hex::encode).collect::<Vec<_>>(),
        "commitments": commits.iter().map(hex::encode).collect::<Vec<_>>(),
        "pad": hex::encode(&*result.pad),
        "check_words": result.check_words,
        "circle_id": hex::encode(result.circle_id),
        "mismatch_abort": {
            "description": "A device holding all commitments receives this reveal for member 1. It MUST abort.",
            "member": 1,
            "bad_reveal": hex::encode(&bad),
            "expected": "abort: commit mismatch for member 1",
        },
        "single_commit": {
            "description": "Standalone commitment check: commit(params, index, contribution).",
            "index": 0,
            "contribution": hex::encode(&contributions[0]),
            "commitment": hex::encode(commit(&params, 0, &contributions[0])),
        }
    })
}

fn checkword_vectors() -> Value {
    let params = CircleParams::new(2, 96, "words", true).unwrap();
    let cases: Vec<Value> = (0..4u64)
        .map(|s| {
            let pad = fill(200 + s, 96);
            json!({
                "pad": hex::encode(&pad),
                "check_words": check_words(&params, &pad),
                "circle_id": hex::encode(circle_id(&pad)),
            })
        })
        .collect();
    json!({
        "description": "check = SHA-256(\"cahoots/check/v1\" || params || pad); first 66 bits as six 11-bit big-endian BIP-39 English indices. circle_id = SHA-256(\"cahoots/circle-id/v1\" || pad)[..16].",
        "params_encoded": hex::encode(params.encode()),
        "cases": cases,
    })
}

fn message_vectors() -> Value {
    let cid: [u8; 16] = fill(300, 16).try_into().unwrap();
    let mut cases = Vec::new();
    for (i, (ty, pt)) in [
        (MessageType::Text, b"hello circle".to_vec()),
        (MessageType::Approve, fill(301, 32)),
        (MessageType::Vouch, Vec::new()),
        (MessageType::Control, bucket_pad(b"padded").unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        let range = fill(310 + i as u64, 32 + pt.len());
        let slice = i as u8;
        let offset = (i as u32) * 1000;
        let wire = message::seal(&cid, slice, offset, ty, &pt, &range).unwrap();
        cases.push(json!({
            "msg_type": ty as u8,
            "slice": slice,
            "offset": offset,
            "plaintext": hex::encode(&pt),
            "pad_range": hex::encode(&range),
            "message": hex::encode(&wire),
        }));
    }
    // Tag failure: flip one ciphertext byte.
    let pt = b"integrity".to_vec();
    let range = fill(320, 32 + pt.len());
    let good = message::seal(&cid, 0, 7, MessageType::Text, &pt, &range).unwrap();
    let mut flipped = good.clone();
    flipped[message::HEADER_LEN] ^= 0x80;
    let (h, ct, tag) = message::parse(&flipped).unwrap();
    assert_eq!(
        message::open(&h, ct, tag, &range),
        Err(message::MessageError::Tag)
    );
    json!({
        "description": "message = header(25) || ct || Poly1305(key, header || ct). key = range[0..32], stream = range[32..]. All integers little-endian.",
        "circle_id": hex::encode(cid),
        "cases": cases,
        "tag_failure": {
            "pad_range": hex::encode(&range),
            "message": hex::encode(&good),
            "tampered": hex::encode(&flipped),
            "expected": "reject: tag mismatch; nothing is marked spent",
        },
    })
}

fn spend_vectors() -> Value {
    let params = CircleParams::new(2, 256, "spend", false).unwrap();
    let pad_bytes = fill(400, 256);
    let mut a = Pad::new(
        params.clone(),
        0,
        MemPadStorage::new(pad_bytes.clone()),
        MemLedger::new(2),
    )
    .unwrap();
    let mut b = Pad::new(
        params.clone(),
        1,
        MemPadStorage::new(pad_bytes.clone()),
        MemLedger::new(2),
    )
    .unwrap();
    let m1 = a.send(MessageType::Text, b"first").unwrap();
    let m2 = a.send(MessageType::Text, b"second").unwrap();
    // After sending, the sender's range is erased and ledgered.
    let mut erased = [0u8; 37];
    a.storage().read(0, &mut erased);
    assert_eq!(erased, [0u8; 37]);
    assert!(a.ledger().is_spent(0, 0, 37));
    let (_, p1) = b.receive(&m1).unwrap();
    assert_eq!(p1, b"first");
    let replay = b.receive(&m1).unwrap_err();
    assert_eq!(replay, PadError::Reuse);
    let (_, p2) = b.receive(&m2).unwrap();
    assert_eq!(p2, b"second");
    json!({
        "description": "Spend ordering. Sender: read range, mark spent (durable), erase, then seal. Receiver: reject overlap with ledger, verify tag, decrypt, mark spent, erase.",
        "params_encoded": hex::encode(params.encode()),
        "pad": hex::encode(&pad_bytes),
        "circle_id": hex::encode(a.circle_id()),
        "slice_len": params.slice_len(),
        "sends_from_slice_0": [
            {"plaintext": hex::encode(b"first"), "offset": 0, "spend_len": 37, "message": hex::encode(&m1)},
            {"plaintext": hex::encode(b"second"), "offset": 37, "spend_len": 38, "message": hex::encode(&m2)},
        ],
        "sender_state_after": {
            "slice_0_bytes_0_to_75": "all zero",
            "ledger_slice_0": a.ledger().ranges(0).iter().map(|&(s, e)| json!([s, e])).collect::<Vec<_>>(),
        },
        "receiver_replay_of_first": "reject: reuse (checked before the tag)",
        "receiver_ledger_slice_0_after": b.ledger().ranges(0).iter().map(|&(s, e)| json!([s, e])).collect::<Vec<_>>(),
    })
}

fn frame_vectors() -> Value {
    let cases: Vec<Value> = [(1u8, 0u32, b"".to_vec()), (2, 7, fill(500, 40))]
        .into_iter()
        .map(|(kind, seq, payload)| {
            let f = frame::Frame { kind, seq, payload: &payload };
            json!({"type": kind, "seq": seq, "payload": hex::encode(&payload), "frame": hex::encode(frame::encode(&f))})
        })
        .collect();
    json!({
        "description": "Touch-link frame: type (u8) | seq (u32 LE) | len (u32 LE) | payload | CRC-32 (IEEE 802.3, LE) over everything before it.",
        "cases": cases,
    })
}

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = [
        ("ceremony.json", ceremony_vectors()),
        ("checkwords.json", checkword_vectors()),
        ("messages.json", message_vectors()),
        ("spend.json", spend_vectors()),
        ("frames.json", frame_vectors()),
    ];
    let mut stale = Vec::new();
    for (name, value) in files {
        let path = dir.join(name);
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        if check {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(&text) {
                stale.push(name);
            }
        } else {
            std::fs::write(&path, text).unwrap();
            println!("wrote {}", path.display());
        }
    }
    if !stale.is_empty() {
        eprintln!("stale vectors: {stale:?}. Run `cargo run -p cahoots-vectors`.");
        std::process::exit(1);
    }
}
