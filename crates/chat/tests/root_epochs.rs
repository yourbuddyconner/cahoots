//! Chat keyed by root epochs (docs/chat.md section 1): the epoch key is
//! `chat_key(root, circle_id, n)`, and no pad is spent.

use cahoots_chat::{parse, ChatError, ChatSession, EpochId, MAX_AHEAD, ROOT_EPOCH_SLICE};
use cahoots_core::root::chat_key;
use cahoots_core::CircleId;

const ROOT: [u8; 32] = [0x5a; 32];
const CID: CircleId = [0xc1; 16];

fn circle(n: u8) -> Vec<ChatSession> {
    (0..n).map(|i| ChatSession::new(&ROOT, CID, n, i)).collect()
}

fn epoch_of(wire: &[u8]) -> EpochId {
    parse(wire).unwrap().0.epoch
}

/// A wire message in root epoch `n` from `sender`, sealed with the real derivation.
fn sealed_in(root: &[u8; 32], n: u32, sender: u8, text: &[u8]) -> Vec<u8> {
    let mut s = ChatSession::new(root, CID, 3, sender);
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
fn an_epoch_far_ahead_is_refused() {
    let mut s = circle(3);
    let w1 = s[0].send(b"e1").unwrap();
    s[1].receive(&w1).unwrap();
    let too_far = sealed_in(&ROOT, 1 + MAX_AHEAD + 1, 0, b"far");
    assert_eq!(
        s[1].receive(&too_far).unwrap_err(),
        ChatError::EpochTooFarAhead
    );
    assert_eq!(s[1].current_epoch(), Some(EpochId::root(1)));
    assert_eq!(s[1].epochs(), 1);
    let at_edge = sealed_in(&ROOT, 1 + MAX_AHEAD, 0, b"edge");
    assert_eq!(s[1].receive(&at_edge).unwrap().1, b"edge");
    assert_eq!(s[1].current_epoch(), Some(EpochId::root(1 + MAX_AHEAD)));
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
