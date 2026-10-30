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
    let (mut a, mut b) = (
        a.finish("CSPRNG").unwrap().into_circle(),
        b.finish("CSPRNG").unwrap().into_circle(),
    );
    a.chat_mark_synced();
    b.chat_mark_synced();
    (a, b)
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
    assert_eq!(got.epoch(), "1");
    // A receiver adopts the newest epoch it has seen, so b replies in a's epoch.
    assert_eq!(b.current_epoch().unwrap(), "1");
    let r = b.chat_send(b"hi a").unwrap();
    let got = a.chat_receive(&r).unwrap();
    assert_eq!(got.text(), b"hi a");
    assert_eq!(got.sender(), 1);
    assert_eq!(got.epoch(), "1");
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
fn rotate_epoch_spends_nothing_and_peers_follow() {
    let (mut a, mut b) = pair();
    assert!(a.has_root() && b.has_root());
    let w1 = a.chat_send(b"one").unwrap();
    b.chat_receive(&w1).unwrap();
    let before = a.remaining();
    let id = a.rotate_epoch().unwrap();
    assert_eq!(id, "2");
    assert_eq!(a.remaining(), before);
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
    assert!(!r.chat_synced());
    r.chat_mark_synced();
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

/// A version 4 state: the version 5 layout without the root trailer, with pad-epoch chat
/// state in the chat slot.
fn as_version_4(v5: &[u8]) -> Vec<u8> {
    assert_eq!(v5[0], 5);
    let mut v4 = v5[..v5.len() - 33].to_vec();
    v4[0] = 4;
    v4
}

#[test]
fn a_version_4_state_opens_without_chat() {
    let (mut a, mut b) = pair();
    let w = a.chat_send(b"one").unwrap();
    b.chat_receive(&w).unwrap();
    let pure = a.send(0, b"pure pad").unwrap();
    let mut old = Circle::import(&as_version_4(&b.export())).unwrap();
    assert!(!old.has_root());
    assert!(old.has_seat_key());
    assert_eq!(old.current_epoch(), None);
    assert_eq!(old.chat_send(b"x").unwrap_err(), "no-root");
    assert_eq!(old.chat_receive(&w).unwrap_err(), "no-root");
    assert_eq!(old.rotate_epoch().unwrap_err(), "no-root");
    // The pad still works, and it saves as version 5 with no root.
    assert_eq!(old.receive(&pure).unwrap().plaintext(), b"pure pad");
    let again = Circle::import(&old.export()).unwrap();
    assert_eq!(old.export()[0], 5);
    assert!(!again.has_root());
}

#[test]
fn own_messages_restore_the_sending_position() {
    let (mut a, mut b) = pair();
    let w0 = a.chat_send(b"zero").unwrap();
    let old = a.export();
    let w1 = a.chat_send(b"one").unwrap();
    for w in [&w0, &w1] {
        b.chat_receive(w).unwrap();
    }
    let mut back = Circle::import(&old).unwrap();
    assert_eq!(back.chat_send(b"x").unwrap_err(), "not-synced");
    for w in [&w0, &w1] {
        let own = back.chat_receive(w).unwrap();
        assert_eq!(own.sender(), 0);
        assert!(own.text().is_empty());
    }
    back.chat_mark_synced();
    let w2 = back.chat_send(b"two").unwrap();
    let got = b.chat_receive(&w2).unwrap();
    assert_eq!(got.counter(), 2);
    assert_eq!(got.text(), b"two");
}
