use cahoots_wasm::{
    b64_decode, b64_encode, chat_bucket_pad, chat_bucket_unpad, face_pixels, Circle, WasmCeremony,
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
fn as_version_4(v6: &[u8]) -> Vec<u8> {
    assert_eq!(v6[0], 6);
    let mut v4 = v6[..v6.len() - 33].to_vec();
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
    // The pad still works, and it saves as version 6 with no root.
    assert_eq!(old.receive(&pure).unwrap().plaintext(), b"pure pad");
    let again = Circle::import(&old.export()).unwrap();
    assert_eq!(old.export()[0], 6);
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

#[test]
fn a_bump_moves_the_other_member_and_a_forged_one_is_ignored() {
    let (mut a, mut b) = pair();
    let first = a.chat_send(b"before").unwrap();
    let got = b.chat_receive(&first).unwrap();
    assert_eq!(got.kind(), "text");
    assert!(!got.ignored());

    // A bump signed by a key that is not a seat: b ignores it and the chat goes on.
    let state = a.export();
    let root: [u8; 32] = state[state.len() - 32..].try_into().unwrap();
    let id: [u8; 16] = hex::decode(a.circle_id_hex()).unwrap().try_into().unwrap();
    let mut forger = cahoots_chat::ChatSession::new(&root, id, 2, 0);
    forger.mark_synced();
    forger.start_epoch();
    forger.start_epoch();
    let stranger = cahoots_core::seat::SeatKey::from_seed(&[0x55; 32]).unwrap();
    let fake_gen = [0xEE; 32];
    let payload = cahoots_chat::sign_bump(&stranger, 0, &id, &fake_gen).unwrap();
    let wire = b64_encode(&forger.send_bump(fake_gen, &payload).unwrap());
    let before = b.generation_id_hex();
    let ignored = b.chat_receive(&wire).unwrap();
    assert_eq!(ignored.kind(), "bump");
    assert!(ignored.ignored());
    assert_eq!(b.generation_id_hex(), before);
    let more = a.chat_send(b"still here").unwrap();
    assert_eq!(b.chat_receive(&more).unwrap().text(), b"still here");

    // A real bump moves b, and a later text in the new generation opens.
    let wire = a.bump_generation().unwrap();
    // Until the relay accepts it the bump stays with the circle, across a save and a load.
    assert_eq!(a.pending_bump().as_deref(), Some(wire.as_str()));
    let mut saved = Circle::import(&a.export()).unwrap();
    assert_eq!(saved.pending_bump().as_deref(), Some(wire.as_str()));
    saved.clear_pending_bump();
    assert!(saved.pending_bump().is_none());
    assert!(Circle::import(&saved.export()).unwrap().pending_bump().is_none());
    a.clear_pending_bump();
    assert_ne!(a.generation_id_hex(), before);
    let moved = b.chat_receive(&wire).unwrap();
    assert_eq!(moved.kind(), "bump");
    assert!(!moved.ignored());
    assert_eq!(b.generation_id_hex(), a.generation_id_hex());
    let after = a.chat_send(b"after the reset").unwrap();
    let got = b.chat_receive(&after).unwrap();
    assert_eq!(got.kind(), "text");
    assert_eq!(got.text(), b"after the reset");
    let reply = b.chat_send(b"heard").unwrap();
    assert_eq!(a.chat_receive(&reply).unwrap().text(), b"heard");

    // The new generation survives a save and a load.
    let back = Circle::import(&b.export()).unwrap();
    assert_eq!(back.generation_id_hex(), a.generation_id_hex());
}

#[test]
fn a_version_5_state_still_imports() {
    let (mut a, _) = pair();
    a.chat_send(b"x").unwrap();
    let mut state = a.export();
    state[0] = 5;
    let back = Circle::import(&state).unwrap();
    assert!(back.has_root());
    assert_eq!(back.export()[0], 6);
}

#[test]
fn a_second_reset_waits_for_the_first_to_be_posted() {
    let (mut a, _) = pair();
    let first = a.bump_generation().unwrap();
    let id = a.generation_id_hex();
    assert_eq!(a.bump_generation().unwrap_err(), "reset-pending");
    assert_eq!(a.generation_id_hex(), id);
    assert_eq!(a.pending_bump().as_deref(), Some(first.as_str()));
    a.clear_pending_bump();
    assert!(a.bump_generation().is_ok());
}

#[test]
fn an_old_bump_read_again_moves_nothing() {
    let (mut a, mut b) = pair();
    let w0 = a.bump_generation().unwrap();
    a.clear_pending_bump();
    assert!(!b.chat_receive(&w0).unwrap().ignored());
    let current = b.generation_id_hex();
    // A later bump moves b on, then the first wire again.
    let w1 = a.bump_generation().unwrap();
    a.clear_pending_bump();
    b.chat_receive(&w1).unwrap();
    let newest = b.generation_id_hex();
    assert_ne!(newest, current);
    // The older bump's wire again is sealed in a generation b has dropped: refused, nothing moves.
    assert_eq!(b.chat_receive(&w0).unwrap_err(), "needs-generation");
    assert_eq!(b.generation_id_hex(), newest);
}

#[test]
fn a_card_travels_in_the_chat_with_a_name_a_face_and_a_device() {
    let (mut a, mut b) = pair();
    let face = [0x5a; 32];
    let wire = a.publish_card("  Ada Lovelace ", &face, 2).unwrap();
    // Padded as a text is: the length of the name does not show on the wire.
    let short = a.publish_card("A", &face, 2).unwrap();
    assert_eq!(b64_decode(&wire).unwrap().len(), b64_decode(&short).unwrap().len());
    let got = b.chat_receive(&wire).unwrap();
    assert_eq!(got.kind(), "card");
    assert!(!got.ignored());
    assert_eq!(got.sender(), 0);
    assert_eq!(got.card_name(), "Ada Lovelace");
    assert_eq!(got.card_face(), face.to_vec());
    assert_eq!(got.card_device(), Some(2));
    // Our own copy carries nothing to keep.
    assert!(a.chat_receive(&wire).unwrap().ignored());

    // A seat posting another seat's genuine card is refused.
    let signed = got.text();
    let again = b.send_signed_card(&signed).unwrap();
    let refused = a.chat_receive(&again).unwrap();
    assert_eq!(refused.kind(), "card");
    assert!(refused.ignored());
    assert_eq!(refused.card_name(), "");

    // A card is not text, and a text has no card.
    let t = a.chat_send(b"hi").unwrap();
    let text = b.chat_receive(&t).unwrap();
    assert_eq!(text.card_name(), "");
    assert!(text.card_device().is_none());
}

#[test]
fn a_card_name_is_1_to_64_characters_and_the_face_32_bytes() {
    let (mut a, _) = pair();
    assert_eq!(a.publish_card("   ", &[1; 32], 2).unwrap_err(), "name");
    assert_eq!(a.publish_card(&"x".repeat(65), &[1; 32], 2).unwrap_err(), "name");
    assert!(a.publish_card(&"\u{1F600}".repeat(64), &[1; 32], 2).is_ok());
    assert_eq!(a.publish_card("Ada", &[1; 31], 2).unwrap_err(), "face");
}

#[test]
fn faces_are_576_pixels_and_each_seat_has_an_interim_face() {
    for seed in [[0u8; 32], [0xff; 32], [7; 32]] {
        let px = face_pixels(&seed);
        assert_eq!(px.len(), 576);
        assert!(px.iter().all(|&p| p <= 1));
        assert!(px.contains(&1));
    }
    let (a, b) = pair();
    let (a0, a1) = (a.interim_face_seed(0).unwrap(), a.interim_face_seed(1).unwrap());
    assert_eq!(a0.len(), 32);
    assert_ne!(a0, a1);
    assert_eq!(b.interim_face_seed(0).unwrap(), a0);
    assert!(a.interim_face_seed(2).is_none());
}
