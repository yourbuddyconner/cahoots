//! Generates `vectors/*.json` from `cahoots-core`. Every client (WASM, firmware, the [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) app)
//! must reproduce these. Run `cargo run -p cahoots-vectors` to regenerate and
//! `cargo run -p cahoots-vectors -- --check` to verify the committed files are current.

use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::circle_id;
use cahoots_core::commit::commit;
use cahoots_core::control::{Control, Proto, Purpose};
use cahoots_core::frame;
use cahoots_core::ledger::{MemLedger, SpentLedger};
use cahoots_core::message::{self, MessageType};
use cahoots_core::pad::{bucket_pad, MemPadStorage, Pad, PadError, PadStorage};
use cahoots_core::pairwise;
use cahoots_core::params::CircleParams;
use cahoots_core::reveal::{self, randomness_len};
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

/// Placeholder seat seed and tier for every member until Task 4 gives each member its own
/// seat seed and a tier.
const SEAT_SEED: [u8; 32] = [0x11; 32];
const TIER: u8 = 0;

fn ceremony_vectors() -> Value {
    let params = CircleParams::new(3, 192, "vectors", false).unwrap();
    let contributions: Vec<Vec<u8>> = (0..3).map(|i| fill(100 + i, 192)).collect();
    let seeds: Vec<[u8; 32]> = (0..3)
        .map(|i| fill(300 + i, 32).try_into().unwrap())
        .collect();
    let mut devs: Vec<Ceremony> = contributions
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Ceremony::new(
                params.clone(),
                i as u8,
                c.clone(),
                seeds[i],
                SEAT_SEED,
                TIER,
            )
            .unwrap()
        })
        .collect();
    let seat_pk = devs[0].my_seat_public();
    let commits: Vec<([u8; 32], [u8; 1216])> = devs
        .iter()
        .map(|d| (d.my_commitment(), *d.my_public_key()))
        .collect();
    for d in devs.iter_mut() {
        for (j, (c, pk)) in commits.iter().enumerate() {
            d.add_commitment(j as u8, *c, pk, &seat_pk.0, TIER).unwrap();
        }
    }
    // Sealing randomness: a deterministic stream per member, drawn in the order seal() asks.
    let rand: Vec<Vec<u8>> = (0..3).map(|i| fill(400 + i, randomness_len(3))).collect();
    let reveals: Vec<Vec<u8>> = devs
        .iter_mut()
        .enumerate()
        .map(|(i, d)| {
            let mut pos = 0usize;
            let mut f = |buf: &mut [u8]| {
                buf.copy_from_slice(&rand[i][pos..pos + buf.len()]);
                pos += buf.len();
            };
            d.my_reveal(&mut f).unwrap().to_vec()
        })
        .collect();
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
        assert_eq!(d.phase(), Phase::Attest);
    }
    let attestations: Vec<Vec<u8>> = devs.iter().map(|d| d.my_attestation().unwrap()).collect();
    for d in devs.iter_mut() {
        for (j, a) in attestations.iter().enumerate() {
            d.add_attestation(j as u8, a).unwrap();
        }
        assert_eq!(d.phase(), Phase::Complete);
    }
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap()).collect();
    let result = &results[2];
    let mut expected = vec![0u8; 192];
    for c in &contributions {
        for (e, b) in expected.iter_mut().zip(c) {
            *e ^= b;
        }
    }
    assert_eq!(result.pad.as_slice(), expected.as_slice());

    // Mismatch abort: member 1 seals bytes that differ from its commitment, with its real keys.
    let mut victim = Ceremony::new(
        params.clone(),
        0,
        contributions[0].clone(),
        seeds[0],
        SEAT_SEED,
        TIER,
    )
    .unwrap();
    for (j, (c, pk)) in commits.iter().enumerate() {
        victim
            .add_commitment(j as u8, *c, pk, &seat_pk.0, TIER)
            .unwrap();
    }
    let mut bad = contributions[1].clone();
    bad[0] ^= 0x01;
    let recipients: Vec<(u8, &[u8])> = vec![(0, &commits[0].1), (2, &commits[2].1)];
    let mut pos = 0usize;
    let mut f = |buf: &mut [u8]| {
        buf.copy_from_slice(&rand[1][pos..pos + buf.len()]);
        pos += buf.len();
    };
    let bad_reveal = reveal::seal(
        &params,
        1,
        &commits[1].1,
        &seat_pk,
        TIER,
        &recipients,
        &bad,
        &mut f,
    )
    .unwrap()
    .blob;
    let err = victim.add_reveal(1, &bad_reveal).unwrap_err();
    assert_eq!(err, CeremonyError::CommitMismatch(1));
    assert_eq!(victim.phase(), Phase::Aborted);

    // Garbled in transit: one flipped byte opens as nothing and changes nothing.
    let mut victim = Ceremony::new(
        params.clone(),
        0,
        contributions[0].clone(),
        seeds[0],
        SEAT_SEED,
        TIER,
    )
    .unwrap();
    for (j, (c, pk)) in commits.iter().enumerate() {
        victim
            .add_commitment(j as u8, *c, pk, &seat_pk.0, TIER)
            .unwrap();
    }
    let mut garbled = reveals[1].clone();
    let last = garbled.len() - 1;
    garbled[last] ^= 0x01;
    assert_eq!(
        victim.add_reveal(1, &garbled).unwrap_err(),
        CeremonyError::Corrupted(1)
    );
    assert_eq!(victim.phase(), Phase::Reveal);

    json!({
        "description": "Commit-then-reveal ceremony for n=3, L=192. commit_i = SHA-256(\"cahoots/commit/v2\" || params || i || pk_i || seat_pk_i || tier_i || R_i), where pk_i is the X-Wing encapsulation key derived from kem_seed_i, seat_pk_i is the 33-byte SEC1 compressed P-256 public key of the seat seed (here 0x11 repeated 32 times for every member) and tier_i is one byte (here 0). Sealed reveals are version 2: version || from || pk_from || seat_pk || tier || k || entries || content. Reveals are sealed to every other member (crates/core/src/reveal.rs): content key K_i, per recipient X-Wing encapsulation with the 64 bytes of randomness given, wrap = ChaCha20-Poly1305(HKDF-SHA256(salt \"cahoots/reveal-wrap/v1\", ikm shared, info params||from||to), nonce 0, aad from||to, K_i), content = ChaCha20-Poly1305(K_i, nonce 0, aad \"cahoots/reveal/v1\"||params||from||pk_from||seat_pk||tier, R_i). pad = XOR of all R_i.",
        "params": {
            "n": params.n, "pad_len": params.pad_len, "name": params.name, "agent_seat": params.agent_seat,
            "encoded": hex::encode(params.encode()),
        },
        "contributions": contributions.iter().map(hex::encode).collect::<Vec<_>>(),
        "kem_seeds": seeds.iter().map(hex::encode).collect::<Vec<_>>(),
        "public_keys": commits.iter().map(|(_, pk)| hex::encode(pk)).collect::<Vec<_>>(),
        "commitments": commits.iter().map(|(c, _)| hex::encode(c)).collect::<Vec<_>>(),
        "seal_randomness": rand.iter().map(hex::encode).collect::<Vec<_>>(),
        "sealed_reveals": reveals.iter().map(hex::encode).collect::<Vec<_>>(),
        "pad": hex::encode(&*result.pad),
        "check_words": result.check_words,
        "circle_id": hex::encode(result.circle_id),
        "pair_keys": {
            "description": "pair_key(a, b) = HKDF-SHA256(salt \"cahoots/pair/v1\", ikm = ss(lo->hi) || ss(hi->lo), info = params || lo || hi), 32 bytes, where lo < hi are the two member indices and ss(x->y) is the X-Wing shared secret that x encapsulated to y in its sealed reveal (crates/core/src/pairwise.rs). keys[i][j] is member i's key with member j; null at j = i.",
            "keys": results.iter().map(|r| r.pair_keys.iter().map(|k| k.as_ref().map(|k| hex::encode(k.as_ref()))).collect::<Vec<_>>()).collect::<Vec<_>>(),
        },
        "hello_words": {
            "description": "words(pair_key, speaker, counter): v = LE32(HKDF-SHA256(no salt, ikm = pair_key, info = \"cahoots/words/v1\" || speaker || LE32(counter))[0..4]); the words are WORDLIST[v & 0x7ff] and WORDLIST[(v >> 11) & 0x7ff]. For the pair (0, 1).",
            "at": (0..3u32).map(|c| {
                let k = results[0].pair_keys[1].as_ref().unwrap();
                json!({ "counter": c, "member_0_says": pairwise::words(k, 0, c), "member_1_says": pairwise::words(k, 1, c) })
            }).collect::<Vec<_>>(),
        },
        "mismatch_abort": {
            "description": "Member 0, holding all commitments, receives this reveal for member 1. It opens, but the contribution does not match the commitment. It MUST abort.",
            "member": 1,
            "bad_contribution": hex::encode(&bad),
            "sealed_reveal": hex::encode(&bad_reveal),
            "expected": "abort: commit mismatch for member 1",
        },
        "garbled": {
            "description": "Member 1's real sealed reveal with its last byte flipped. Member 0 MUST refuse it as corrupted and change nothing.",
            "sealed_reveal": hex::encode(&garbled),
            "expected": "corrupted; phase stays reveal",
        },
        "single_commit": {
            "description": "Standalone commitment check: commit(params, index, public_key, seat_public_key, tier, contribution).",
            "index": 0,
            "public_key": hex::encode(commits[0].1),
            "seat_public_key": hex::encode(seat_pk.0),
            "tier": TIER,
            "contribution": hex::encode(&contributions[0]),
            "commitment": hex::encode(commit(&params, 0, &commits[0].1, &seat_pk, TIER, &contributions[0])),
        }
    })
}

fn control_vectors() -> Value {
    let peer = Control::Peer {
        proto: Proto::WireGuard,
        public_key: fill(500, 32),
        endpoint: "203.0.113.7:51820".into(),
        allowed_ips: "10.8.200.2/32".into(),
    };
    let psk = Control::Psk {
        purpose: Purpose::WireGuard,
        peer_a: 0,
        peer_b: 1,
        offset: 64,
    };
    let rotate = Control::Rotate {
        purpose: Purpose::WireGuard,
        peer_a: 0,
        peer_b: 1,
        not_after: 1_761_000_000,
    };
    let transfer = Control::SliceTransfer { member: 3 };
    let enc = |c: &Control| hex::encode(c.encode().unwrap());
    for c in [&peer, &psk, &rotate, &transfer] {
        assert_eq!(&Control::decode(&c.encode().unwrap()).unwrap(), c);
    }
    json!({
        "description": "Control message plaintexts (docs/protocol.md section 4.3): one subtype byte then the fields, little-endian. A pre-shared key for purpose p between seats a and b is derived with context p || a || b.",
        "peer": { "proto": "wireguard", "public_key": hex::encode(fill(500, 32)), "endpoint": "203.0.113.7:51820", "allowed_ips": "10.8.200.2/32", "encoded": enc(&peer) },
        "psk": { "purpose": "wireguard", "peer_a": 0, "peer_b": 1, "offset": 64, "encoded": enc(&psk), "context": hex::encode(Purpose::WireGuard.context(0, 1)) },
        "rotate": { "purpose": "wireguard", "peer_a": 0, "peer_b": 1, "not_after": 1_761_000_000u64, "encoded": enc(&rotate) },
        "slice_transfer": { "member": 3, "encoded": enc(&transfer) },
        "malformed": [
            { "bytes": "", "expected": "length" },
            { "bytes": "09", "expected": "unknown subtype 9" },
            { "bytes": "0100000100000000ff", "expected": "length: trailing byte" },
            { "bytes": "0107000100000000", "expected": "value: unknown purpose" }
        ]
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
        ("control.json", control_vectors()),
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
