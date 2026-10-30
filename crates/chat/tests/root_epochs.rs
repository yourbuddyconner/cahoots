//! Chat keyed by root epochs (docs/chat.md section 1): the epoch key is
//! `chat_key(root, circle_id, n)`, and no pad is spent.

use cahoots_chat::{parse, ChatError, ChatSession, EpochId, ROOT_EPOCH_SLICE};
use cahoots_core::root::chat_key;
use cahoots_core::CircleId;

const ROOT: [u8; 32] = [0x5a; 32];
const CID: CircleId = [0xc1; 16];

/// `n` members of one circle, each already synced with an empty relay.
fn circle(n: u8) -> Vec<ChatSession> {
    (0..n).map(|i| synced(ChatSession::new(&ROOT, CID, n, i))).collect()
}

fn synced(mut s: ChatSession) -> ChatSession {
    s.mark_synced();
    s
}

fn epoch_of(wire: &[u8]) -> EpochId {
    parse(wire).unwrap().0.epoch
}

/// A wire message in root epoch `n` from `sender`, sealed with the real derivation.
fn sealed_in(root: &[u8; 32], n: u32, sender: u8, text: &[u8]) -> Vec<u8> {
    let mut s = synced(ChatSession::new(root, CID, 3, sender));
    while s.current_epoch() != Some(EpochId::root(n)) {
        s.start_epoch();
    }
    s.send(text).unwrap()
}

#[test]
fn epoch_ids_name_root_epochs() {
    let id = EpochId::root(7);
    assert_eq!(id.slice, ROOT_EPOCH_SLICE);
    assert_eq!(id.offset, 7);
    assert_eq!(id.number(), Some(7));
    assert_eq!(EpochId { slice: 3, offset: 7 }.number(), None);
    assert_eq!(id.encode(), [0xFF, 7, 0, 0, 0]);
}

#[test]
fn the_epoch_key_is_the_root_chat_key() {
    let ek = cahoots_chat::epoch_key(&ROOT, &CID, 9);
    assert_eq!(*ek, chat_key(&ROOT, &CID, 9));
}

#[test]
fn three_members_chat_and_rotate_on_root_epochs() {
    let mut s = circle(3);
    let w = s[0].send(b"one from A").unwrap();
    assert_eq!(epoch_of(&w), EpochId::root(1));
    assert_eq!(s[1].receive(&w).unwrap().1, b"one from A");
    assert_eq!(s[2].receive(&w).unwrap().1, b"one from A");

    assert_eq!(s[1].start_epoch(), EpochId::root(2));
    let wb = s[1].send(b"two from B").unwrap();
    assert_eq!(epoch_of(&wb), EpochId::root(2));

    // A sends in epoch 1 before it has seen B's rotation: two epochs are live.
    let wa = s[0].send(b"still one from A").unwrap();
    assert_eq!(epoch_of(&wa), EpochId::root(1));

    assert_eq!(s[0].receive(&wb).unwrap().1, b"two from B");
    assert_eq!(s[2].receive(&wb).unwrap().1, b"two from B");
    assert_eq!(s[1].receive(&wa).unwrap().1, b"still one from A");
    assert_eq!(s[2].receive(&wa).unwrap().1, b"still one from A");

    // Everyone now sends in the newest epoch they have seen.
    for m in s.iter() {
        assert_eq!(m.current_epoch(), Some(EpochId::root(2)));
    }
    let wc = s[2].send(b"from C").unwrap();
    assert_eq!(epoch_of(&wc), EpochId::root(2));
    assert_eq!(s[0].receive(&wc).unwrap().1, b"from C");
}

#[test]
fn catch_up_in_order() {
    let mut s = circle(3);
    let w1 = s[0].send(b"e1").unwrap();
    s[1].receive(&w1).unwrap();
    s[2].receive(&w1).unwrap();
    // C goes offline. A rotates twice.
    s[0].start_epoch();
    let w2 = s[0].send(b"e2").unwrap();
    s[1].receive(&w2).unwrap();
    s[0].start_epoch();
    let w3 = s[0].send(b"e3").unwrap();
    s[1].receive(&w3).unwrap();
    assert_eq!(epoch_of(&w3), EpochId::root(3));
    // C comes back and reads both in order.
    assert_eq!(s[2].receive(&w2).unwrap().1, b"e2");
    assert_eq!(s[2].receive(&w3).unwrap().1, b"e3");
    assert_eq!(s[2].current_epoch(), Some(EpochId::root(3)));
    // A new epoch from C is past everything it has seen.
    assert_eq!(s[2].start_epoch(), EpochId::root(4));
}

#[test]
fn a_message_from_another_root_is_refused() {
    let mut s = circle(3);
    let w = sealed_in(&[0x77; 32], 1, 0, b"other root");
    assert_eq!(s[1].receive(&w).unwrap_err(), ChatError::Tag);
    // Nothing was kept: a real epoch 1 message still reads, and a new epoch starts at 1.
    assert_eq!(s[1].current_epoch(), None);
    assert_eq!(s[1].start_epoch(), EpochId::root(1));
    let good = s[0].send(b"real").unwrap();
    assert_eq!(s[2].receive(&good).unwrap().1, b"real");
}

#[test]
fn an_epoch_far_ahead_with_a_bad_tag_installs_nothing() {
    let mut s = circle(3);
    let w1 = s[0].send(b"e1").unwrap();
    s[1].receive(&w1).unwrap();
    // Epoch 100000 in the header: the header is authenticated, so the tag fails.
    let mut far = w1.clone();
    far[18..22].copy_from_slice(&100_000u32.to_le_bytes());
    assert_eq!(s[1].receive(&far).unwrap_err(), ChatError::Tag);
    assert_eq!(s[1].current_epoch(), Some(EpochId::root(1)));
    assert_eq!(s[1].epochs(), 1);
    // A new epoch from this member still follows the highest it has read.
    assert_eq!(s[1].start_epoch(), EpochId::root(2));
}

#[test]
fn a_member_far_behind_catches_up() {
    let mut s = circle(3);
    let w1 = s[0].send(b"e1").unwrap();
    s[2].receive(&w1).unwrap();
    // Seat 0 moves on 500 epochs while seat 2 is away.
    for _ in 0..500 {
        s[0].start_epoch();
    }
    let latest = s[0].send(b"e501").unwrap();
    assert_eq!(epoch_of(&latest), EpochId::root(501));
    assert_eq!(s[2].receive(&latest).unwrap().1, b"e501");
    assert_eq!(s[2].current_epoch(), Some(EpochId::root(501)));
}

#[test]
fn a_pad_epoch_id_is_refused() {
    let mut s = circle(3);
    let mut w = s[0].send(b"x").unwrap();
    w[17] = 3; // the epoch id's slice byte
    assert_eq!(s[1].receive(&w).unwrap_err(), ChatError::PadEpoch);
    assert_eq!(s[1].epochs(), 0);
}

#[test]
fn export_and_import_keep_the_place() {
    let mut s = circle(3);
    let w0 = s[0].send(b"a").unwrap();
    let w1 = s[0].send(b"b").unwrap();
    s[1].receive(&w0).unwrap();
    s[1].start_epoch();
    let state = s[1].export_state();
    let mut back = ChatSession::import_state(&ROOT, CID, 3, 1, &state).unwrap();
    back.mark_synced();
    assert_eq!(back.current_epoch(), Some(EpochId::root(2)));
    assert_eq!(back.receive(&w1).unwrap().1, b"b");
    assert_eq!(back.receive(&w0).unwrap_err(), ChatError::Replay);
    let w2 = back.send(b"c").unwrap();
    assert_eq!(epoch_of(&w2), EpochId::root(2));
    assert_eq!(s[0].receive(&w2).unwrap().1, b"c");
    // State for another seat, or cut short, does not import.
    assert!(ChatSession::import_state(&ROOT, CID, 3, 2, &state).is_err());
    assert!(ChatSession::import_state(&ROOT, CID, 3, 1, &state[..state.len() / 2]).is_err());
}

#[test]
fn sending_before_sync_is_refused() {
    let mut fresh = ChatSession::new(&ROOT, CID, 3, 0);
    assert!(!fresh.synced());
    assert_eq!(fresh.send(b"x").unwrap_err(), ChatError::NotSynced);
    fresh.mark_synced();
    assert!(fresh.send(b"x").is_ok());
    // An imported session starts unsynced too.
    let mut back = ChatSession::import_state(&ROOT, CID, 3, 0, &fresh.export_state()).unwrap();
    assert!(!back.synced());
    assert_eq!(back.send(b"y").unwrap_err(), ChatError::NotSynced);
}

#[test]
fn a_restored_session_does_not_reuse_a_counter() {
    let mut s = circle(3);
    let w0 = s[0].send(b"zero").unwrap();
    let saved = s[0].export_state();
    let w1 = s[0].send(b"one").unwrap();
    let w2 = s[0].send(b"two").unwrap();
    // Seat 0 comes back from the older state and reads the relay, its own messages included.
    let mut back = ChatSession::import_state(&ROOT, CID, 3, 0, &saved).unwrap();
    for w in [&w0, &w1, &w2] {
        let (h, pt) = back.receive(w).unwrap();
        assert_eq!(h.sender, 0);
        assert!(pt.is_empty(), "our own messages are not decrypted");
    }
    back.mark_synced();
    let w3 = back.send(b"three").unwrap();
    assert_eq!(parse(&w3).unwrap().0.counter, 3);
    assert_eq!(s[1].receive(&w3).unwrap().1, b"three");
}

#[test]
fn a_session_that_lost_everything_finds_its_place_on_the_relay() {
    let mut s = circle(3);
    s[0].send(b"a").unwrap();
    s[0].start_epoch();
    let w = s[0].send(b"b").unwrap();
    let mut fresh = ChatSession::new(&ROOT, CID, 3, 0);
    fresh.receive(&w).unwrap();
    fresh.mark_synced();
    let next = fresh.send(b"c").unwrap();
    assert_eq!(epoch_of(&next), EpochId::root(2));
    assert_eq!(parse(&next).unwrap().0.counter, 1);
}

#[test]
fn a_forged_own_message_moves_nothing() {
    let mut s = circle(3);
    let mut w = s[0].send(b"a").unwrap();
    let w_counter_10 = {
        let mut h = parse(&w).unwrap().0;
        h.counter = 10;
        let mut v = h.encode().to_vec();
        v.extend_from_slice(&w[cahoots_chat::HEADER_LEN..]);
        v
    };
    assert_eq!(s[0].receive(&w_counter_10).unwrap_err(), ChatError::Tag);
    w = s[0].send(b"b").unwrap();
    assert_eq!(parse(&w).unwrap().0.counter, 1);
}
