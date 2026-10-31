use cahoots_wasm::{
    b64_decode, b64_encode, bucket_pad, bucket_unpad, qr_svg, Circle, FountainDecoder,
    FountainEncoder, WasmCeremony,
};

fn contribution(i: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|k| ((k as u32).wrapping_mul(17 + i as u32) ^ i as u32) as u8)
        .collect()
}

fn seed(i: u8) -> Vec<u8> {
    vec![0x40 + i; 32]
}

fn seat(i: u8) -> Vec<u8> {
    vec![0x60 + i; 32]
}

/// Randomness for sealing: enough bytes for the ceremony, deterministic per member.
fn rand(c: &WasmCeremony, i: u8) -> Vec<u8> {
    contribution(100 + i, c.reveal_randomness_len())
}

#[test]
fn two_member_ceremony_through_bindings_agrees() {
    let l = 1024;
    let mut a = WasmCeremony::new(
        2,
        l as u32,
        "pair",
        false,
        0,
        &contribution(0, l),
        None,
        &seed(0),
        &seat(0),
        0,
    )
    .unwrap();
    let mut b = WasmCeremony::new(
        2,
        l as u32,
        "pair",
        false,
        1,
        &contribution(1, l),
        None,
        &seed(1),
        &seat(1),
        0,
    )
    .unwrap();
    assert_eq!(a.phase(), "commit");
    assert!(a.reveal(&[]).unwrap().is_none());
    let ca = a.commitment();
    let cb = b.commitment();
    assert_eq!(ca.len(), 32 + 1216 + 33 + 1);
    assert_ne!(ca[32 + 1216..32 + 1216 + 33], cb[32 + 1216..32 + 1216 + 33]);
    assert_eq!(ca[ca.len() - 1], 0, "a phone declares tier 0");
    assert_eq!(
        a.add_commitment(1, &cb[..cb.len() - 1]).unwrap_err(),
        "commitment-length"
    );
    a.add_commitment(1, &cb).unwrap();
    b.add_commitment(0, &ca).unwrap();
    assert_eq!(a.phase(), "reveal");
    assert_eq!(a.reveal(&[1, 2, 3]).unwrap_err(), "randomness-length");
    let ra = a.reveal(&rand(&a, 0)).unwrap().unwrap();
    let rb = b.reveal(&rand(&b, 1)).unwrap().unwrap();
    assert!(
        !ra.windows(32).any(|w| w == &contribution(0, l)[..32]),
        "contribution in the clear"
    );
    // A garbled copy is refused without changing anything.
    let mut garbled = rb.clone();
    let last = garbled.len() - 1;
    garbled[last] ^= 1;
    assert_eq!(a.add_reveal(1, &garbled).unwrap_err(), "corrupted:1");
    assert_eq!(a.phase(), "reveal");
    a.add_reveal(1, &rb).unwrap();
    b.add_reveal(0, &ra).unwrap();
    assert_eq!(a.phase(), "attest");
    assert_eq!(a.missing_attestations(), vec![1]);
    let (aa, ab) = (a.attestation().unwrap(), b.attestation().unwrap());
    let mut bad = ab.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(a.add_attestation(1, &bad).unwrap_err(), "bad-attestation:1");
    a.add_attestation(1, &ab).unwrap();
    b.add_attestation(0, &aa).unwrap();
    assert!(a.missing_attestations().is_empty());
    assert_eq!(a.phase(), "complete");
    let ra = a.finish("CSPRNG").unwrap();
    let rb = b.finish("TRNG").unwrap();
    assert_eq!(ra.words(), rb.words());
    assert_eq!(ra.words().split(' ').count(), 6);
    assert_eq!(ra.circle_id_hex(), rb.circle_id_hex());
    assert_eq!(ra.circle_id_hex().len(), 32);
    let mut ca = ra.into_circle();
    let mut cb = rb.into_circle();
    assert_eq!(ca.label(), "CSPRNG");
    assert_eq!(cb.label(), "TRNG");
    let wire = ca.send(0, b"hello").unwrap();
    let got = cb.receive(&wire).unwrap();
    assert_eq!(got.plaintext(), b"hello");
    assert_eq!(got.msg_type(), 0);
    assert_eq!(got.slice(), 0);
    assert_eq!(cb.receive(&wire).unwrap_err(), "reuse");
    let fresh = ca.send(0, b"fresh").unwrap();
    let mut bad = b64_decode(&fresh).unwrap();
    bad[30] ^= 1;
    assert_eq!(cb.receive(&b64_encode(&bad)).unwrap_err(), "tag");
}

#[test]
fn extra_source_changes_commitment() {
    let l = 256;
    let a = WasmCeremony::new(
        2,
        l,
        "x",
        false,
        0,
        &contribution(0, 256),
        None,
        &seed(0),
        &seat(0),
        0,
    )
    .unwrap();
    let b = WasmCeremony::new(
        2,
        l,
        "x",
        false,
        0,
        &contribution(0, 256),
        Some(contribution(5, 256)),
        &seed(0),
        &seat(0),
        0,
    )
    .unwrap();
    assert_ne!(a.commitment()[..32], b.commitment()[..32]);
    assert_eq!(
        a.commitment()[32..],
        b.commitment()[32..],
        "same seed, same key"
    );
    assert!(WasmCeremony::new(
        2,
        l,
        "x",
        false,
        0,
        &contribution(0, 256),
        Some(contribution(5, 10)),
        &seed(0),
        &seat(0),
        0,
    )
    .is_err());
    assert_eq!(
        WasmCeremony::new(
            2,
            l,
            "x",
            false,
            0,
            &contribution(0, 256),
            None,
            &[1, 2],
            &seat(0),
            0
        )
        .err()
        .unwrap(),
        "kem-seed-length"
    );
    assert_eq!(
        WasmCeremony::new(
            2,
            l,
            "x",
            false,
            0,
            &contribution(0, 256),
            None,
            &seed(0),
            &[1],
            0
        )
        .err()
        .unwrap(),
        "seat-seed-length"
    );
    assert_eq!(
        WasmCeremony::new(
            2,
            l,
            "x",
            false,
            0,
            &contribution(0, 256),
            None,
            &seed(0),
            &[0; 32],
            0
        )
        .err()
        .unwrap(),
        "bad-seed"
    );
    assert_eq!(
        WasmCeremony::new(
            2,
            l,
            "x",
            false,
            0,
            &contribution(0, 256),
            None,
            &seed(0),
            &seat(0),
            5
        )
        .err()
        .unwrap(),
        "bad-tier"
    );
}

#[test]
fn circle_state_roundtrips_with_ledger() {
    let l = 512;
    let mut a = WasmCeremony::new(
        2,
        l,
        "s",
        true,
        0,
        &contribution(0, 512),
        None,
        &seed(0),
        &seat(0),
        0,
    )
    .unwrap();
    let mut b = WasmCeremony::new(
        2,
        l,
        "s",
        true,
        1,
        &contribution(1, 512),
        None,
        &seed(1),
        &seat(1),
        0,
    )
    .unwrap();
    let (ca, cb) = (a.commitment(), b.commitment());
    a.add_commitment(1, &cb).unwrap();
    b.add_commitment(0, &ca).unwrap();
    let (ra, rb) = (
        a.reveal(&rand(&a, 0)).unwrap().unwrap(),
        b.reveal(&rand(&b, 1)).unwrap().unwrap(),
    );
    a.add_reveal(1, &rb).unwrap();
    b.add_reveal(0, &ra).unwrap();
    let (aa, ab) = (a.attestation().unwrap(), b.attestation().unwrap());
    a.add_attestation(1, &ab).unwrap();
    b.add_attestation(0, &aa).unwrap();
    let mut ca = a.finish("TRNG").unwrap().into_circle();
    let mut cb = b.finish("TRNG").unwrap().into_circle();
    let w1 = ca.send(0, b"one").unwrap();
    cb.receive(&w1).unwrap();
    assert!(ca.has_seat_key() && cb.has_seat_key());
    assert_eq!(cb.tier(), 0);
    let id = cb.manifest_id_hex().unwrap();
    assert_eq!(id.len(), 64);
    assert_eq!(ca.manifest_id_hex().unwrap(), id);
    let saved = cb.export();
    assert_eq!(saved[0], 5);
    let mut restored = Circle::import(&saved).unwrap();
    assert!(restored.has_seat_key());
    assert!(restored.has_root());
    assert_eq!(restored.manifest_id_hex().unwrap(), id);
    assert_eq!(restored.export(), saved);
    // A version 3 state is the same without the version 4 and 5 trailers: it loads with no
    // seat key.
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
    assert!(!old.has_root());
    assert!(old.manifest_id_hex().is_none());
    assert_eq!(old.remaining(), cb.remaining());
    assert_eq!(old.export()[0], 5);
    assert!(!Circle::import(&old.export()).unwrap().has_seat_key());
    assert_eq!(restored.circle_id_hex(), cb.circle_id_hex());
    assert_eq!(restored.name(), "s");
    assert_eq!(restored.my_slice(), 1);
    assert_eq!(restored.label(), "TRNG");
    assert!(restored.agent_seat());
    assert_eq!(restored.remaining(), cb.remaining());
    assert_eq!(restored.receive(&w1).unwrap_err(), "reuse");
    let w2 = ca.send(0, b"two").unwrap();
    assert_eq!(restored.receive(&w2).unwrap().plaintext(), b"two");
    let w3 = restored.send(1, b"back").unwrap();
    assert_eq!(ca.receive(&w3).unwrap().plaintext(), b"back");
    assert!(Circle::import(&saved[..10]).is_err());
}

#[test]
fn fountain_roundtrip_with_dropped_packets() {
    let data: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let enc = FountainEncoder::new(&data, 500);
    assert!(enc.source_packets() >= 40);
    let mut dec = FountainDecoder::new();
    assert_eq!(dec.progress(), 0.0);
    let mut i = 0u32;
    while !dec.done() {
        if i % 3 != 0 {
            dec.add(&enc.packet(i)).unwrap();
        }
        i += 1;
        assert!(i < 10_000, "never completed");
    }
    assert_eq!(dec.result().unwrap(), data);
    assert!(dec.progress() >= 1.0);
    assert!(dec.add(&[1, 2, 3]).is_err());
}

#[test]
fn fountain_packets_carry_a_header_the_decoder_recognises() {
    let enc = FountainEncoder::new(b"short", 100);
    let p = enc.packet(0);
    assert_eq!(&p[..4], b"CHQ1");
    let mut dec = FountainDecoder::new();
    dec.add(&p).unwrap();
    assert!(dec.done());
    assert_eq!(dec.result().unwrap(), b"short");
    let other = FountainEncoder::new(b"other data here", 100);
    assert!(
        dec.add(&other.packet(0)).is_err(),
        "mixed transfers are rejected"
    );
}

#[test]
fn qr_svg_and_bucket_and_base64url() {
    let svg = qr_svg(b"hello").unwrap();
    assert!(svg.contains("<svg"));
    let p = bucket_pad(b"abc").unwrap();
    assert_eq!(p.len(), 32);
    assert_eq!(bucket_unpad(&p).unwrap(), b"abc");
    assert!(bucket_pad(&[0u8; 300]).is_none());
    let e = b64_encode(&[0xfb, 0xff]);
    assert_eq!(e, "-_8");
    assert_eq!(b64_decode(&e).unwrap(), vec![0xfb, 0xff]);
    assert!(b64_decode("***").is_err());
}

#[test]
fn late_reveal_can_stand_in_for_its_commitment() {
    let contrib = |i: u8| -> Vec<u8> {
        (0..1024u32)
            .map(|k| (k.wrapping_mul(7 + i as u32)) as u8)
            .collect()
    };
    let mut a = WasmCeremony::new(
        2,
        1024,
        "late",
        false,
        0,
        &contrib(0),
        None,
        &seed(0),
        &seat(0),
        0,
    )
    .unwrap();
    let mut b = WasmCeremony::new(
        2,
        1024,
        "late",
        false,
        1,
        &contrib(1),
        None,
        &seed(1),
        &seat(1),
        0,
    )
    .unwrap();
    // B scanned A's commitment and is already sealing. A never saw B's commitment code:
    // B's sealed reveal stands in for it, and A has every reveal in one step.
    b.add_commitment(0, &a.commitment()).unwrap();
    let rb = b.reveal(&rand(&b, 1)).unwrap().unwrap();
    assert_eq!(a.phase(), "commit");
    a.add_reveal(1, &rb).unwrap();
    assert_eq!(a.phase(), "attest");
    // A's own reveal still goes to B, sealed to B's key, which A learned from the late reveal.
    let ra = a.reveal(&rand(&a, 0)).unwrap().unwrap();
    b.add_reveal(0, &ra).unwrap();
    assert_eq!(b.phase(), "attest");
    let (aa, ab) = (a.attestation().unwrap(), b.attestation().unwrap());
    a.add_attestation(1, &ab).unwrap();
    b.add_attestation(0, &aa).unwrap();
    assert_eq!(b.phase(), "complete");
    assert_eq!(
        a.finish("CSPRNG").unwrap().words(),
        b.finish("CSPRNG").unwrap().words()
    );
}

#[test]
fn hello_words_are_pairwise_and_move_with_the_counter() {
    let l = 512;
    let mut devs: Vec<WasmCeremony> = (0..3u8)
        .map(|i| {
            WasmCeremony::new(
                3,
                l,
                "hi",
                false,
                i,
                &contribution(i, 512),
                None,
                &seed(i),
                &seat(i),
                0,
            )
            .unwrap()
        })
        .collect();
    let commits: Vec<_> = devs.iter().map(|d| d.commitment()).collect();
    for (i, d) in devs.iter_mut().enumerate() {
        for (j, c) in commits.iter().enumerate() {
            if i != j {
                d.add_commitment(j as u8, c).unwrap();
            }
        }
    }
    let reveals: Vec<_> = devs
        .iter_mut()
        .enumerate()
        .map(|(i, d)| d.reveal(&rand(d, i as u8)).unwrap().unwrap())
        .collect();
    for (i, d) in devs.iter_mut().enumerate() {
        for (j, r) in reveals.iter().enumerate() {
            if i != j {
                d.add_reveal(j as u8, r).unwrap();
            }
        }
    }
    let atts: Vec<_> = devs.iter().map(|d| d.attestation().unwrap()).collect();
    for (i, d) in devs.iter_mut().enumerate() {
        for (j, a) in atts.iter().enumerate() {
            if i != j {
                d.add_attestation(j as u8, a).unwrap();
            }
        }
    }
    let mut circles: Vec<Circle> = devs
        .into_iter()
        .map(|d| d.finish("CSPRNG").unwrap().into_circle())
        .collect();
    assert!(!circles[0].has_pair_key(0), "no key with yourself");
    assert!(circles[0].has_pair_key(1));
    assert!(circles[0].has_pair_key(2));
    let a = circles[0].hello_words(1).unwrap();
    let b = circles[1].hello_words(0).unwrap();
    assert_eq!(a.counter(), 0);
    assert_eq!(a.mine(), b.theirs(), "what 0 says is what 1 expects");
    assert_eq!(a.theirs(), b.mine());
    assert_ne!(a.mine(), a.theirs());
    assert_eq!(a.mine().split(' ').count(), 2);
    let c = circles[2].hello_words(0).unwrap();
    assert_ne!(
        c.theirs(),
        a.mine(),
        "a third member expects different words from 0"
    );
    // Next on both sides: same new words, different from before.
    let a2 = circles[0].next_hello(1).unwrap();
    let b2 = circles[1].next_hello(0).unwrap();
    assert_eq!(a2.counter(), 1);
    assert_eq!(circles[0].pair_counter(1), 1);
    assert_eq!(a2.mine(), b2.theirs());
    assert_ne!(a2.mine(), a.mine());
    // Out of step: set the number by hand and they agree again.
    circles[1].set_pair_counter(0, 5);
    circles[0].set_pair_counter(1, 5);
    assert_eq!(
        circles[0].hello_words(1).unwrap().mine(),
        circles[1].hello_words(0).unwrap().theirs()
    );
    // Keys and counters survive export/import.
    let restored = Circle::import(&circles[0].export()).unwrap();
    assert_eq!(restored.pair_counter(1), 5);
    assert_eq!(
        restored.hello_words(1).unwrap().mine(),
        circles[0].hello_words(1).unwrap().mine()
    );
    assert!(!restored.has_pair_key(0));
    assert_eq!(circles[0].hello_words(0).unwrap_err(), "no-pair-key");
    assert_eq!(circles[0].hello_words(9).unwrap_err(), "no-pair-key");
}
