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
use cahoots_core::manifest::{Manifest, Seat};
use cahoots_core::message::{self, MessageType};
use cahoots_core::pad::{bucket_pad, MemPadStorage, Pad, PadError, PadStorage};
use cahoots_core::pairwise;
use cahoots_core::params::CircleParams;
use cahoots_core::reveal::{self, randomness_len};
use cahoots_core::seat::SeatKey;
use cahoots_objects::grant::{self, verify_chain, Grant, Grantor};
use cahoots_objects::card::{self, Card, PURPOSE_CHAT, PURPOSE_SSH};
use cahoots_objects::{assurance, fact_nonce, Approval, Fact, Receipt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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

/// The tier each member declares; the middle one differs so a swap shows.
const TIERS: [u8; 3] = [0, 1, 0];

/// What the ceremony vectors' run gives the circle root vectors: the circle id, every seat's
/// reveal content key in seat order, and the root every member derived.
struct RootCase {
    circle_id: [u8; 16],
    content_keys: Vec<[u8; 32]>,
    root: [u8; 32],
}

fn ceremony_vectors() -> (Value, RootCase) {
    let params = CircleParams::new(3, 192, "vectors", false).unwrap();
    let contributions: Vec<Vec<u8>> = (0..3).map(|i| fill(100 + i, 192)).collect();
    let seeds: Vec<[u8; 32]> = (0..3)
        .map(|i| fill(300 + i, 32).try_into().unwrap())
        .collect();
    let seat_seeds: Vec<[u8; 32]> = (0..3)
        .map(|i| fill(350 + i, 32).try_into().unwrap())
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
                seat_seeds[i],
                TIERS[i],
            )
            .unwrap()
        })
        .collect();
    let seat_pks: Vec<_> = devs.iter().map(|d| d.my_seat_public()).collect();
    let commits: Vec<([u8; 32], [u8; 1216])> = devs
        .iter()
        .map(|d| (d.my_commitment(), *d.my_public_key()))
        .collect();
    for d in devs.iter_mut() {
        for (j, (c, pk)) in commits.iter().enumerate() {
            d.add_commitment(j as u8, *c, pk, &seat_pks[j].0, TIERS[j])
                .unwrap();
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
    let manifest = devs[0].manifest().unwrap().to_vec();
    // The circle root: every member holds the same content keys and derives the same root.
    let root_case = RootCase {
        circle_id: devs[0].circle_id().unwrap(),
        content_keys: devs[0].reveal_secrets().unwrap().to_vec(),
        root: *devs[0].circle_root().unwrap(),
    };
    for d in &devs {
        assert_eq!(*d.reveal_secrets().unwrap(), root_case.content_keys);
        assert_eq!(*d.circle_root().unwrap(), root_case.root);
    }
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap()).collect();
    for r in &results {
        assert_eq!(r.root.as_deref(), Some(&root_case.root));
    }
    let result = &results[2];
    let mut expected = vec![0u8; 192];
    for c in &contributions {
        for (e, b) in expected.iter_mut().zip(c) {
            *e ^= b;
        }
    }
    assert_eq!(result.pad.as_slice(), expected.as_slice());
    let sealed = &result.manifest;
    assert_eq!(sealed.body, manifest);
    let parsed = sealed.verify().unwrap();
    for r in &results {
        assert_eq!(r.manifest, *sealed);
    }

    // Mismatch abort: member 1 seals bytes that differ from its commitment, with its real keys.
    let mut victim = Ceremony::new(
        params.clone(),
        0,
        contributions[0].clone(),
        seeds[0],
        seat_seeds[0],
        TIERS[0],
    )
    .unwrap();
    for (j, (c, pk)) in commits.iter().enumerate() {
        victim
            .add_commitment(j as u8, *c, pk, &seat_pks[j].0, TIERS[j])
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
        &seat_pks[1],
        TIERS[1],
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
        seat_seeds[0],
        TIERS[0],
    )
    .unwrap();
    for (j, (c, pk)) in commits.iter().enumerate() {
        victim
            .add_commitment(j as u8, *c, pk, &seat_pks[j].0, TIERS[j])
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

    let value = json!({
        "description": "Commit-then-reveal ceremony for n=3, L=192. commit_i = SHA-256(\"cahoots/commit/v2\" || params || i || pk_i || seat_pk_i || tier_i || R_i), where pk_i is the X-Wing encapsulation key derived from kem_seed_i, seat_pk_i is the 33-byte SEC1 compressed P-256 public key of the seat seed (from seat_seeds[i]) and tier_i is one byte (tiers[i]: 0, 1, 0). Sealed reveals are version 2: version || from || pk_from || seat_pk || tier || k || entries || content. Reveals are sealed to every other member (crates/core/src/reveal.rs): content key K_i, per recipient X-Wing encapsulation with the 64 bytes of randomness given, wrap = ChaCha20-Poly1305(HKDF-SHA256(salt \"cahoots/reveal-wrap/v1\", ikm shared, info params||from||to), nonce 0, aad from||to, K_i), content = ChaCha20-Poly1305(K_i, nonce 0, aad \"cahoots/reveal/v1\"||params||from||pk_from||seat_pk||tier, R_i). pad = XOR of all R_i. After the reveals every member builds the same canonical manifest body (docs/superpowers/specs/2026-10-26-seats-manifest-objects.md sections 4 and 5), signs it with its seat key as a detached COSE_Sign1 (attestations[i]), and collects every attestation. sealed_manifest is SealedManifest::encode(); manifest_id is Manifest::id of the body.",
        "params": {
            "n": params.n, "pad_len": params.pad_len, "name": params.name, "agent_seat": params.agent_seat,
            "encoded": hex::encode(params.encode()),
        },
        "contributions": contributions.iter().map(hex::encode).collect::<Vec<_>>(),
        "seat_seeds": seat_seeds.iter().map(hex::encode).collect::<Vec<_>>(),
        "seat_publics": seat_pks.iter().map(|p| hex::encode(p.0)).collect::<Vec<_>>(),
        "tiers": TIERS,
        "kem_seeds": seeds.iter().map(hex::encode).collect::<Vec<_>>(),
        "public_keys": commits.iter().map(|(_, pk)| hex::encode(pk)).collect::<Vec<_>>(),
        "commitments": commits.iter().map(|(c, _)| hex::encode(c)).collect::<Vec<_>>(),
        "seal_randomness": rand.iter().map(hex::encode).collect::<Vec<_>>(),
        "sealed_reveals": reveals.iter().map(hex::encode).collect::<Vec<_>>(),
        "attestations": attestations.iter().map(hex::encode).collect::<Vec<_>>(),
        "manifest": hex::encode(&manifest),
        "manifest_id": hex::encode(parsed.id()),
        "sealed_manifest": hex::encode(sealed.encode()),
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
            "seat_public_key": hex::encode(seat_pks[0].0),
            "tier": TIERS[0],
            "contribution": hex::encode(&contributions[0]),
            "commitment": hex::encode(commit(&params, 0, &commits[0].1, &seat_pks[0], TIERS[0], &contributions[0])),
        }
    });
    (value, root_case)
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

/// Inputs of a grant, every field, so a client can rebuild the payload.
fn grant_inputs(g: &Grant) -> Value {
    json!({
        "circle_id": hex::encode(g.circle_id),
        "grantor": match g.grantor {
            Grantor::Seat(s) => json!(s),
            Grantor::Key(k) => json!(hex::encode(k.0)),
        },
        "grantee": hex::encode(g.grantee.0),
        "purposes": g.purposes,
        "budget": g.budget,
        "not_after": g.not_after,
        "depth": g.depth,
        "parent": g.parent.map(hex::encode),
        "nonce": hex::encode(g.nonce),
    })
}

fn signed_object(inputs: Value, payload: Vec<u8>, signed: &[u8], projection: Value) -> Value {
    json!({
        "inputs": inputs,
        "payload": hex::encode(payload),
        "signed": hex::encode(signed),
        "json": projection,
    })
}

fn object_vectors() -> Value {
    // A small three-seat circle, built here and given as hex.
    let params = CircleParams::new(3, 192, "objects", false).unwrap();
    let seat_seeds: Vec<[u8; 32]> = (0..3)
        .map(|i| fill(500 + i, 32).try_into().unwrap())
        .collect();
    let seat_keys: Vec<SeatKey> = seat_seeds
        .iter()
        .map(|s| SeatKey::from_seed(s).unwrap())
        .collect();
    let circle: [u8; 16] = fill(510, 16).try_into().unwrap();
    let manifest = Manifest {
        circle_id: circle,
        seats: seat_keys
            .iter()
            .enumerate()
            .map(|(i, k)| Seat {
                seat: i as u8,
                key: k.public(),
                tier: TIERS[i],
                evidence: Vec::new(),
            })
            .collect(),
        floor: Manifest::default_floor(&params),
        params,
        genesis: 0,
    };
    // Every object is signed by seat 1.
    let seat = 1u8;
    let seat_key = &seat_keys[usize::from(seat)];
    let delegate_seed: [u8; 32] = fill(520, 32).try_into().unwrap();
    let delegate = SeatKey::from_seed(&delegate_seed).unwrap();
    let leaf_seed: [u8; 32] = fill(521, 32).try_into().unwrap();
    let leaf = SeatKey::from_seed(&leaf_seed).unwrap();

    let root = Grant {
        circle_id: circle,
        grantor: Grantor::Seat(seat),
        grantee: delegate.public(),
        purposes: vec![
            Purpose::WireGuard as u8,
            Purpose::Tls as u8,
            Purpose::Mls as u8,
        ],
        budget: 4096,
        not_after: 1000,
        depth: 1,
        parent: None,
        nonce: fill(530, 16).try_into().unwrap(),
    };
    let root_signed = root.sign(seat_key, &[seat]);
    let delegated = Grant {
        circle_id: circle,
        grantor: Grantor::Key(delegate.public()),
        grantee: leaf.public(),
        purposes: vec![Purpose::Tls as u8],
        budget: 1024,
        not_after: 900,
        depth: 0,
        parent: Some(Sha256::digest(&root_signed).into()),
        nonce: fill(531, 16).try_into().unwrap(),
    };
    let delegated_signed = delegated.sign(&delegate, &grant::key_id(&delegate.public()));
    assert_eq!(
        verify_chain(&[&root_signed, &delegated_signed], &manifest).unwrap(),
        delegated
    );

    let approval = Approval {
        circle_id: circle,
        seat,
        profile: "payment".into(),
        fields: vec![
            ("Payee".into(), "Acme Ltd".into()),
            ("Amount".into(), "120.00 EUR".into()),
        ],
        request_digest: fill(540, 32).try_into().unwrap(),
        policy_id: b"policy-7".to_vec(),
        verdict: true,
        assurance: assurance::PRESENT,
        log: 42,
        nonce: fill(541, 16).try_into().unwrap(),
    };
    let approval_signed = approval.sign(seat_key, &[seat]);
    assert_eq!(
        Approval::verify(&approval_signed, &manifest).unwrap(),
        approval
    );

    let fact = Fact {
        circle_id: circle,
        seat,
        module: "sht4x/temperature".into(),
        reading: vec![0x09, 0xc4],
        counter: 7,
        nonce: fact_nonce(&circle, 43),
        log: 43,
        attested: true,
        evidence: fill(550, 24),
    };
    let fact_signed = fact.sign(seat_key, &[seat]);
    assert_eq!(Fact::verify(&fact_signed, &manifest).unwrap(), fact);

    let receipt = Receipt {
        circle_id: circle,
        seat,
        slice: 2,
        offset: 64,
        len: 32,
        purpose: Purpose::Tls as u8,
        grant: Some(Sha256::digest(&delegated_signed).into()),
        log: 44,
    };
    let receipt_signed = receipt.sign(seat_key, &[seat]);
    assert_eq!(
        Receipt::verify(&receipt_signed, &manifest).unwrap(),
        receipt
    );

    // Version 2 grants: the same root as a CWT, and a delegation below it, also version 2.
    let root_v2_signed = root.sign_v2(seat_key, &[seat]);
    let delegated_v2 = Grant {
        parent: Some(Sha256::digest(&root_v2_signed).into()),
        ..delegated.clone()
    };
    let delegated_v2_signed = delegated_v2.sign_v2(&delegate, &grant::key_id(&delegate.public()));
    assert_eq!(
        verify_chain(&[&root_v2_signed, &delegated_v2_signed], &manifest).unwrap(),
        delegated_v2
    );

    // A contact card, and the card sealed under the card seal key of a given root.
    let contact = Card {
        circle_id: circle,
        seat,
        name: "Ada Lovelace".into(),
        handles: vec![
            ("email".into(), "ada@example.com".into()),
            ("github".into(), "ada".into()),
        ],
        endpoints: vec![
            (Purpose::WireGuard as u8, "203.0.113.7:51820".into()),
            (PURPOSE_SSH, "ada.example.com".into()),
            (PURPOSE_CHAT, "cahoots:chat:ada".into()),
        ],
        log: 45,
    };
    let card_signed = contact.sign(seat_key, &[seat]).unwrap();
    assert_eq!(Card::verify(&card_signed, &manifest).unwrap(), contact);
    let circle_root: [u8; 32] = fill(560, 32).try_into().unwrap();
    let seal_key = cahoots_core::root::seal_key(&circle_root, cahoots_core::root::SEAL_CARD, &circle);
    let seal_nonce: [u8; 12] = fill(561, 12).try_into().unwrap();
    let sealed = card::seal_with_nonce(&card_signed, &seal_key, seal_nonce);
    assert_eq!(card::open(&sealed, &seal_key).unwrap(), card_signed);
    let mut card_vector = signed_object(json!({
        "circle_id": hex::encode(contact.circle_id),
        "seat": contact.seat,
        "name": contact.name,
        "handles": contact.handles.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "endpoints": contact.endpoints.iter().map(|(p, v)| json!([p, v])).collect::<Vec<_>>(),
        "log": contact.log,
    }), contact.encode().unwrap(), &card_signed, contact.to_json());
    card_vector["seal"] = json!({
        "root": hex::encode(circle_root),
        "key": hex::encode(seal_key),
        "nonce": hex::encode(seal_nonce),
        "sealed": hex::encode(&sealed),
    });

    json!({
        "description_v2": "Contact cards and version 2 grants (docs/superpowers/specs/2026-10-28-chip-apps-design.md sections 6 and 8). grant_v2_root is root_grant's inputs as a CBOR Web Token claims set (crates/objects/src/cwt.rs, docs/protocol.md section 8), signed by seat 1 under kid [1] with the same external AAD cahoots/grant/v1; grant_v2_delegated is delegated_grant's inputs with parent SHA-256 of grant_v2_root's signed bytes, as a CWT signed by the delegate. card is a contact card signed by seat 1 under kid [1] with external AAD cahoots/card/v1; card.seal gives a circle root, the card seal key HMAC-SHA256(root, \"seal/card\" || circle_id), a nonce, and the sealed card nonce || AES-256-GCM(key, nonce, aad cahoots/card/v1, signed card).",
        "grant_v2_root": signed_object(grant_inputs(&root), root.encode_v2(), &root_v2_signed, root.to_v2().to_json()),
        "grant_v2_delegated": signed_object(grant_inputs(&delegated_v2), delegated_v2.encode_v2(), &delegated_v2_signed, delegated_v2.to_v2().to_json()),
        "card": card_vector,
        "description": "The four signed objects (docs/superpowers/specs/2026-10-26-seats-manifest-objects.md section 6). Each payload is a canonical CBOR map with integer keys 0.. ascending, key 0 the version 1; each signed form is an untagged COSE_Sign1 [protected {1: -7, 4: kid}, {}, payload, signature] with ES256 (RFC 6979 deterministic, so the signatures reproduce) and external AAD the object's domain: cahoots/grant/v1, cahoots/approval/v1, cahoots/fact/v1, cahoots/receipt/v1. The manifest is a three-seat circle whose seat keys come from seat_seeds; every object is signed by seat 1 under kid [1], except the delegated grant, signed by the root grant's grantee (delegate_seed) under kid SHA-256 of its 33-byte public key. The delegated grant's parent is SHA-256 of the root's signed bytes; the receipt's grant is SHA-256 of the delegated grant's signed bytes. The fact's nonce is SHA-256(\"cahoots/fact-nonce/v1\" || circle_id || log as 8 bytes LE). json is the informative JSON projection.",
        "manifest": hex::encode(manifest.encode()),
        "circle_id": hex::encode(circle),
        "seat_seeds": seat_seeds.iter().map(hex::encode).collect::<Vec<_>>(),
        "seat_publics": seat_keys.iter().map(|k| hex::encode(k.public().0)).collect::<Vec<_>>(),
        "seat": seat,
        "delegate_seed": hex::encode(delegate_seed),
        "delegate_public": hex::encode(delegate.public().0),
        "leaf_seed": hex::encode(leaf_seed),
        "leaf_public": hex::encode(leaf.public().0),
        "root_grant": signed_object(grant_inputs(&root), root.encode(), &root_signed, root.to_json()),
        "delegated_grant": signed_object(grant_inputs(&delegated), delegated.encode(), &delegated_signed, delegated.to_json()),
        "approval": signed_object(json!({
            "circle_id": hex::encode(approval.circle_id),
            "seat": approval.seat,
            "profile": approval.profile,
            "fields": approval.fields.iter().map(|(l, v)| json!([l, v])).collect::<Vec<_>>(),
            "request_digest": hex::encode(approval.request_digest),
            "policy_id": hex::encode(&approval.policy_id),
            "verdict": approval.verdict,
            "assurance": approval.assurance,
            "log": approval.log,
            "nonce": hex::encode(approval.nonce),
        }), approval.encode(), &approval_signed, approval.to_json()),
        "fact": signed_object(json!({
            "circle_id": hex::encode(fact.circle_id),
            "seat": fact.seat,
            "module": fact.module,
            "reading": hex::encode(&fact.reading),
            "counter": fact.counter,
            "nonce": hex::encode(fact.nonce),
            "log": fact.log,
            "attested": fact.attested,
            "evidence": hex::encode(&fact.evidence),
        }), fact.encode(), &fact_signed, fact.to_json()),
        "receipt": signed_object(json!({
            "circle_id": hex::encode(receipt.circle_id),
            "seat": receipt.seat,
            "slice": receipt.slice,
            "offset": receipt.offset,
            "len": receipt.len,
            "purpose": receipt.purpose,
            "grant": receipt.grant.map(hex::encode),
            "log": receipt.log,
        }), receipt.encode(), &receipt_signed, receipt.to_json()),
    })
}

/// The circle root from the ceremony vectors' content keys, the chat key per epoch and the seal
/// keys named by a label.
fn root_vectors(case: &RootCase) -> Value {
    use cahoots_core::root;
    let derived = root::derive(&case.content_keys, &case.circle_id);
    assert_eq!(derived, case.root);
    let chat: Vec<Value> = [0u64, 1, 7, 1 << 32]
        .iter()
        .map(|&epoch| {
            json!({
                "epoch": epoch,
                "key": hex::encode(root::chat_key(&case.root, &case.circle_id, epoch)),
            })
        })
        .collect();
    let seal: Vec<Value> = [root::SEAL_CARD, b"seal/notes".as_slice()]
        .iter()
        .map(|label| {
            json!({
                "label": std::str::from_utf8(label).unwrap(),
                "key": hex::encode(root::seal_key(&case.root, label, &case.circle_id)),
            })
        })
        .collect();
    json!({
        "description": "The circle root and the keys derived from it (crates/core/src/root.rs, docs/superpowers/specs/2026-10-28-chip-apps-design.md section 2). content_keys are the reveal content keys of the ceremony in ceremony.json, in seat order, and circle_id is that ceremony's circle id. root = HKDF-SHA256(ikm = content_keys concatenated in seat order, salt = circle_id, info = \"cahoots/root/v1\"), 32 bytes. A chat key is HMAC-SHA256(root, \"chat\" || circle_id || epoch as 8 bytes big-endian). A seal key is HMAC-SHA256(root, label || circle_id); the card seal key is the one labelled seal/card.",
        "source": "ceremony.json",
        "circle_id": hex::encode(case.circle_id),
        "content_keys": case.content_keys.iter().map(hex::encode).collect::<Vec<_>>(),
        "root": hex::encode(case.root),
        "chat_keys": chat,
        "seal_keys": seal,
    })
}

/// Append `b` as an SSH string.
fn ssh_string(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// The userauth request OpenSSH asks an agent to sign.
fn userauth(session_id: &[u8], user: &str, method: &str, key: &[u8], host_key: Option<&[u8]>) -> Vec<u8> {
    let mut v = Vec::new();
    ssh_string(&mut v, session_id);
    v.push(50);
    ssh_string(&mut v, user.as_bytes());
    ssh_string(&mut v, b"ssh-connection");
    ssh_string(&mut v, method.as_bytes());
    v.push(1);
    ssh_string(&mut v, b"ecdsa-sha2-nistp256");
    ssh_string(&mut v, key);
    if let Some(h) = host_key {
        ssh_string(&mut v, h);
    }
    v
}

/// Requests to sign for ssh and git, with the badge's screen for each and the signature it
/// returns, from a software seat key.
fn ssh_vectors() -> Value {
    use cahoots_badge::apps::BadgeApps;
    use cahoots_badge::{ssh, Circle};
    use cahoots_custodian::contract::ChipApps;
    use cahoots_custodian::wire::{ChipRequest, SshHost};

    let seat_seed: [u8; 32] = fill(600, 32).try_into().unwrap();
    let seat = SeatKey::from_seed(&seat_seed).unwrap();
    let circle_id: [u8; 16] = fill(610, 16).try_into().unwrap();
    let circle = Circle {
        params: CircleParams::new(2, 192, "bench", false).unwrap(),
        seat: 0,
        circle_id,
        words: [0; 6],
        seat_key: seat_seed,
        chip_slot: None,
        root: None,
        tier: 0,
        manifest: Vec::new(),
    };
    let host_seed: [u8; 32] = fill(620, 32).try_into().unwrap();
    let host_key = SeatKey::from_seed(&host_seed).unwrap();
    let host_blob = ssh::public_key_blob(&host_key.public());
    let session_id = fill(630, 32);
    let binding = SshHost {
        host_key: host_blob.clone(),
        session_id: session_id.clone(),
        host_sig: ssh::encode_signature(&host_key.sign(&session_id)),
        name: "git.example.com".into(),
        user: "git".into(),
    };
    let seat_blob = ssh::public_key_blob(&seat.public());

    let commit = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
\n\
Fix the parser for long names\n\
\n\
Names longer than 64 bytes were cut.\n";
    let mut sshsig = b"SSHSIG".to_vec();
    ssh_string(&mut sshsig, b"git");
    ssh_string(&mut sshsig, b"");
    ssh_string(&mut sshsig, b"sha512");
    ssh_string(&mut sshsig, &sha2::Sha512::digest(commit.as_bytes()));

    let cases: Vec<(&str, Vec<u8>, Option<SshHost>, Option<Vec<u8>>)> = vec![
        (
            "login bound to a P-256 host key: the host is verified and still the laptop's name",
            userauth(&session_id, "git", "publickey", &seat_blob, None),
            Some(binding.clone()),
            None,
        ),
        (
            "login with the server's key in the request (publickey-hostbound-v00@openssh.com)",
            userauth(&session_id, "git", "publickey-hostbound-v00@openssh.com", &seat_blob, Some(&host_blob)),
            Some(binding.clone()),
            None,
        ),
        (
            "login with no binding: host not named",
            userauth(&session_id, "root", "publickey", &seat_blob, None),
            None,
            None,
        ),
        (
            "login bound to another session: host not named",
            userauth(&fill(631, 32), "git", "publickey", &seat_blob, None),
            Some(binding.clone()),
            None,
        ),
        (
            "git commit: an SSHSIG envelope over SHA-512 of the commit text, which the laptop sends as content",
            sshsig,
            None,
            Some(commit.as_bytes().to_vec()),
        ),
    ];
    let host_json = |h: &SshHost| {
        json!({
            "host_key": hex::encode(&h.host_key),
            "session_id": hex::encode(&h.session_id),
            "host_sig": hex::encode(&h.host_sig),
            "name": h.name,
            "user": h.user,
        })
    };
    let circles = [circle];
    let out: Vec<Value> = cases
        .into_iter()
        .map(|(what, data, host, content)| {
            let op = ChipRequest::SshSign {
                data: data.clone(),
                host: host.clone(),
                content: content.clone(),
            };
            let mut apps = BadgeApps::new(&circles, None, [0u8; 16]);
            let screen = apps.prompt(&circle_id, &op).unwrap().unwrap();
            let signature = apps
                .ssh_sign(&circle_id, &data, host.as_ref(), content.as_deref())
                .unwrap();
            json!({
                "case": what,
                "data": hex::encode(&data),
                "host": host.as_ref().map(host_json),
                "content": content.as_ref().map(hex::encode),
                "screen": screen.lines,
                "signature": hex::encode(&signature),
            })
        })
        .collect();
    json!({
        "description": "Requests to sign for ssh and git (docs/superpowers/specs/2026-10-28-chip-apps-design.md section 5, docs/protocol.md). A badge with one circle, bench, seat 0, whose seat key comes from seat_seed, receives a Chip SshSign request for each case: data is the exact bytes OpenSSH asks an agent to sign, host the session binding the laptop learned from session-bind@openssh.com (host_key and host_sig are SSH wire blobs; the host key comes from host_seed) and content the commit text for git. screen is the text of the consent screen the badge shows, line by line, footer last; a host name is always marked (laptop). signature is the SSH signature blob the badge returns: ecdsa-sha2-nistp256 with r and s as mpints, ECDSA P-256 over SHA-256(data), RFC 6979 deterministic.",
        "seat_seed": hex::encode(seat_seed),
        "seat_public": hex::encode(seat.public().0),
        "seat_blob": hex::encode(&seat_blob),
        "circle_id": hex::encode(circle_id),
        "circle": "bench",
        "seat": 0,
        "host_seed": hex::encode(host_seed),
        "cases": out,
    })
}

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (ceremony, root_case) = ceremony_vectors();
    let files = [
        ("ceremony.json", ceremony),
        ("root.json", root_vectors(&root_case)),
        ("ssh.json", ssh_vectors()),
        ("control.json", control_vectors()),
        ("checkwords.json", checkword_vectors()),
        ("messages.json", message_vectors()),
        ("spend.json", spend_vectors()),
        ("frames.json", frame_vectors()),
        ("objects.json", object_vectors()),
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
