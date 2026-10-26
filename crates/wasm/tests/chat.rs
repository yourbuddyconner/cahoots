use cahoots_wasm::{
    b64_decode, b64_encode, chat_bucket_pad, chat_bucket_unpad, Circle, WasmCeremony,
};

fn contribution(i: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|k| ((k as u32).wrapping_mul(29 + i as u32) ^ (i as u32 * 7)) as u8)
        .collect()
}

fn pair() -> (Circle, Circle) {
    let l = 4096;
    let mut a = WasmCeremony::new(
        2,
        l,
        "chat",
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
        "chat",
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
    let rand = |i: u8, n: usize| contribution(50 + i, n);
    let na = a.reveal_randomness_len();
    let nb = b.reveal_randomness_len();
    let (ra, rb) = (
        a.reveal(&rand(0, na)).unwrap().unwrap(),
        b.reveal(&rand(1, nb)).unwrap().unwrap(),
    );
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

#[test]
fn chat_round_trips_both_ways_and_reports_epoch() {
    let (mut a, mut b) = pair();
    assert!(a.current_epoch().is_none());
    let w = a.chat_send(b"hello b").unwrap();
    let got = b.chat_receive(&w).unwrap();
    assert_eq!(got.text(), b"hello b");
    assert_eq!(got.sender(), 0);
    assert_eq!(got.counter(), 0);
    assert_eq!(got.epoch(), a.current_epoch().unwrap());
    assert_eq!(got.epoch(), "0:0");
    // A receiver adopts the newest epoch it has seen, so b replies in a's epoch.
    assert_eq!(b.current_epoch().unwrap(), "0:0");
    let r = b.chat_send(b"hi a").unwrap();
    let got = a.chat_receive(&r).unwrap();
    assert_eq!(got.text(), b"hi a");
    assert_eq!(got.sender(), 1);
    assert_eq!(got.epoch(), "0:0");
}

#[test]
fn chat_errors_are_stable_strings() {
    let (mut a, mut b) = pair();
    let w = a.chat_send(b"once").unwrap();
    let mut bad = b64_decode(&w).unwrap();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(b.chat_receive(&b64_encode(&bad)).unwrap_err(), "tag");
    assert_eq!(b.chat_receive(&w).unwrap().text(), b"once");
    assert_eq!(b.chat_receive(&w).unwrap_err(), "replay");
    let mut v = b64_decode(&w).unwrap();
    v[0] = 1;
    assert_eq!(b.chat_receive(&b64_encode(&v)).unwrap_err(), "version");
    assert_eq!(b.chat_receive("!!").unwrap_err(), "base64url");
}

#[test]
fn rotate_epoch_spends_and_peers_follow() {
    let (mut a, mut b) = pair();
    let w1 = a.chat_send(b"one").unwrap();
    b.chat_receive(&w1).unwrap();
    let before = a.remaining();
    let id = a.rotate_epoch().unwrap();
    assert_eq!(a.remaining(), before - 32);
    assert_eq!(id, a.current_epoch().unwrap());
    let w2 = a.chat_send(b"two").unwrap();
    let got = b.chat_receive(&w2).unwrap();
    assert_eq!(got.epoch(), id);
    assert_eq!(got.text(), b"two");
    assert!(a.pure_pad_still_works());
}

#[test]
fn state_export_carries_chat_and_relay_seq() {
    let (mut a, mut b) = pair();
    let w1 = a.chat_send(b"one").unwrap();
    b.chat_receive(&w1).unwrap();
    b.set_relay_seq(Some(7));
    let saved = b.export();
    let mut r = Circle::import(&saved).unwrap();
    assert_eq!(r.relay_seq(), Some(7));
    assert_eq!(r.current_epoch(), b.current_epoch());
    assert_eq!(r.chat_receive(&w1).unwrap_err(), "replay");
    let w2 = a.chat_send(b"two").unwrap();
    assert_eq!(r.chat_receive(&w2).unwrap().text(), b"two");
    let back = r.chat_send(b"three").unwrap();
    assert_eq!(a.chat_receive(&back).unwrap().text(), b"three");
}

#[test]
fn chat_buckets() {
    let p = chat_bucket_pad(b"abc").unwrap();
    assert_eq!(p.len(), 64);
    assert_eq!(chat_bucket_unpad(&p).unwrap(), b"abc");
}
