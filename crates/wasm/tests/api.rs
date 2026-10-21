use cahoots_wasm::{
    b64_decode, b64_encode, bucket_pad, bucket_unpad, qr_svg, Circle, FountainDecoder,
    FountainEncoder, WasmCeremony,
};

fn contribution(i: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|k| ((k as u32).wrapping_mul(17 + i as u32) ^ i as u32) as u8)
        .collect()
}

#[test]
fn two_member_ceremony_through_bindings_agrees() {
    let l = 1024;
    let mut a =
        WasmCeremony::new(2, l as u32, "pair", false, 0, &contribution(0, l), None).unwrap();
    let mut b =
        WasmCeremony::new(2, l as u32, "pair", false, 1, &contribution(1, l), None).unwrap();
    assert_eq!(a.phase(), "commit");
    assert!(a.reveal().is_none());
    let ca = a.commitment();
    let cb = b.commitment();
    assert_eq!(ca.len(), 32);
    a.add_commitment(1, &cb).unwrap();
    b.add_commitment(0, &ca).unwrap();
    assert_eq!(a.phase(), "reveal");
    let ra = a.reveal().unwrap();
    let rb = b.reveal().unwrap();
    a.add_reveal(1, &rb).unwrap();
    b.add_reveal(0, &ra).unwrap();
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
    let a = WasmCeremony::new(2, l, "x", false, 0, &contribution(0, 256), None).unwrap();
    let b = WasmCeremony::new(
        2,
        l,
        "x",
        false,
        0,
        &contribution(0, 256),
        Some(contribution(5, 256)),
    )
    .unwrap();
    assert_ne!(a.commitment(), b.commitment());
    assert!(WasmCeremony::new(
        2,
        l,
        "x",
        false,
        0,
        &contribution(0, 256),
        Some(contribution(5, 10))
    )
    .is_err());
}

#[test]
fn circle_state_roundtrips_with_ledger() {
    let l = 512;
    let mut a = WasmCeremony::new(2, l, "s", true, 0, &contribution(0, 512), None).unwrap();
    let mut b = WasmCeremony::new(2, l, "s", true, 1, &contribution(1, 512), None).unwrap();
    let (ca, cb) = (a.commitment(), b.commitment());
    a.add_commitment(1, &cb).unwrap();
    b.add_commitment(0, &ca).unwrap();
    let (ra, rb) = (a.reveal().unwrap(), b.reveal().unwrap());
    a.add_reveal(1, &rb).unwrap();
    b.add_reveal(0, &ra).unwrap();
    let mut ca = a.finish("TRNG").unwrap().into_circle();
    let mut cb = b.finish("TRNG").unwrap().into_circle();
    let w1 = ca.send(0, b"one").unwrap();
    cb.receive(&w1).unwrap();
    let saved = cb.export();
    let mut restored = Circle::import(&saved).unwrap();
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
    let mut a = WasmCeremony::new(2, 1024, "late", false, 0, &contrib(0), None).unwrap();
    let b = WasmCeremony::new(2, 1024, "late", false, 1, &contrib(1), None).unwrap();
    // A never saw B's commitment code. It derives it from B's reveal, then applies the reveal.
    let derived = a.commitment_for(1, &contrib(1));
    assert_eq!(derived, b.commitment());
    a.add_commitment(1, &derived).unwrap();
    assert_eq!(a.phase(), "reveal");
    a.add_reveal(1, &contrib(1)).unwrap();
    assert_eq!(a.phase(), "complete");
}
