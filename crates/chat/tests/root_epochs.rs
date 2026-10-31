//! Epochs (docs/chat.md section 1). New messages go in epochs of a generation, keyed from the
//! generation key, and no pad is spent. Old root epochs, keyed by `chat_key(root, circle_id,
//! n)`, are never sent in again and still open.

use cahoots_chat::{
    chain_init, epoch_key, generation_tag, parse, seal, step, ChatError, ChatSession, EpochId,
    Header, MessageType, ROOT_EPOCH_SLICE,
};
use cahoots_core::root::{chat_key, gen0_id};
use cahoots_core::CircleId;

const ROOT: [u8; 32] = [0x5a; 32];
const CID: CircleId = [0xc1; 16];

/// `n` members of one circle, each already synced with an empty relay.
fn circle(n: u8) -> Vec<ChatSession> {
    (0..n)
        .map(|i| synced(ChatSession::new(&ROOT, CID, n, i)))
        .collect()
}

fn synced(mut s: ChatSession) -> ChatSession {
    s.mark_synced();
    s
}

/// Epoch `n` of generation 0, where every circle starts.
fn ep(n: u16) -> EpochId {
    EpochId::generation(generation_tag(&gen0_id(&CID)), n)
}

fn epoch_of(wire: &[u8]) -> EpochId {
    parse(wire).unwrap().0.epoch
}

/// A wire message in epoch `n` of generation 0 from `sender`, sealed with the real derivation.
fn sealed_in(root: &[u8; 32], n: u16, sender: u8, text: &[u8]) -> Vec<u8> {
    let mut s = synced(ChatSession::new(root, CID, 3, sender));
    while s.current_epoch() != Some(ep(n)) {
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
    assert_eq!(
        EpochId {
            slice: 3,
            offset: 7
        }
        .number(),
        None
    );
    assert_eq!(id.encode(), [0xFF, 7, 0, 0, 0]);
}

#[test]
fn the_epoch_key_is_the_root_chat_key() {
    let ek = cahoots_chat::epoch_key(&ROOT, &CID, 9);
    assert_eq!(*ek, chat_key(&ROOT, &CID, 9));
}

#[test]
fn generation_epoch_ids_carry_the_tag_then_n() {
    let id = EpochId::generation(0xBBAA, 0x0201);
    assert_eq!(id.slice, 0xFE);
    assert_eq!(id.encode(), [0xFE, 0xAA, 0xBB, 0x01, 0x02]);
    assert_eq!(EpochId::decode(&id.encode()), id);
    assert_eq!(id.generation_parts(), Some((0xBBAA, 0x0201)));
    assert_eq!(id.number(), None);
    assert_eq!(EpochId::root(7).generation_parts(), None);
}

/// A message in old root epoch `n`, sealed the way the build before generations sealed it.
fn root_epoch_message(n: u32, sender: u8, counter: u32, text: &[u8]) -> Vec<u8> {
    let mut chain = chain_init(&epoch_key(&ROOT, &CID, n), sender);
    for _ in 0..counter {
        chain = step(&chain).1;
    }
    let header = Header {
        version: 2,
        circle_id: CID,
        epoch: EpochId::root(n),
        sender,
        counter,
        len: text.len() as u32,
        msg_type: MessageType::Text,
    };
    seal(&header, &step(&chain).0, text)
}

#[test]
fn old_root_epoch_messages_still_open_and_are_never_sent() {
    let mut s = circle(3);
    let old = [
        root_epoch_message(1, 0, 0, b"one"),
        root_epoch_message(1, 0, 1, b"two"),
        root_epoch_message(4, 2, 0, b"later"),
    ];
    for w in &old {
        assert_eq!(parse(w).unwrap().0.version, 2);
        assert_eq!(w[0], 2);
    }
    assert_eq!(s[1].receive(&old[1]).unwrap().1, b"two");
    assert_eq!(s[1].receive(&old[0]).unwrap().1, b"one");
    assert_eq!(s[1].receive(&old[2]).unwrap().1, b"later");
    assert_eq!(s[1].receive(&old[0]).unwrap_err(), ChatError::Replay);
    // Reading old root epochs does not move where this member sends.
    assert_eq!(s[1].current_epoch(), None);
    let w = s[1].send(b"new").unwrap();
    assert_eq!(epoch_of(&w), ep(1));
    assert_eq!(parse(&w).unwrap().0.version, 3);
    assert_eq!(s[0].receive(&w).unwrap().1, b"new");
}

#[test]
fn three_members_chat_and_rotate_on_generation_epochs() {
    let mut s = circle(3);
    let w = s[0].send(b"one from A").unwrap();
    assert_eq!(epoch_of(&w), ep(1));
    assert_eq!(s[1].receive(&w).unwrap().1, b"one from A");
    assert_eq!(s[2].receive(&w).unwrap().1, b"one from A");

    assert_eq!(s[1].start_epoch(), ep(2));
    let wb = s[1].send(b"two from B").unwrap();
    assert_eq!(epoch_of(&wb), ep(2));

    // A sends in epoch 1 before it has seen B's rotation: two epochs are live.
    let wa = s[0].send(b"still one from A").unwrap();
    assert_eq!(epoch_of(&wa), ep(1));

    assert_eq!(s[0].receive(&wb).unwrap().1, b"two from B");
    assert_eq!(s[2].receive(&wb).unwrap().1, b"two from B");
    assert_eq!(s[1].receive(&wa).unwrap().1, b"still one from A");
    assert_eq!(s[2].receive(&wa).unwrap().1, b"still one from A");

    // Everyone now sends in the newest epoch they have seen.
    for m in s.iter() {
        assert_eq!(m.current_epoch(), Some(ep(2)));
    }
    let wc = s[2].send(b"from C").unwrap();
    assert_eq!(epoch_of(&wc), ep(2));
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
    assert_eq!(epoch_of(&w3), ep(3));
    // C comes back and reads both in order.
    assert_eq!(s[2].receive(&w2).unwrap().1, b"e2");
    assert_eq!(s[2].receive(&w3).unwrap().1, b"e3");
    assert_eq!(s[2].current_epoch(), Some(ep(3)));
    // A new epoch from C is past everything it has seen.
    assert_eq!(s[2].start_epoch(), ep(4));
}

#[test]
fn a_message_from_another_root_is_refused() {
    let mut s = circle(3);
    let w = sealed_in(&[0x77; 32], 1, 0, b"other root");
    assert_eq!(s[1].receive(&w).unwrap_err(), ChatError::Tag);
    // Nothing was kept: a real epoch 1 message still reads, and a new epoch starts at 1.
    assert_eq!(s[1].current_epoch(), None);
    assert_eq!(s[1].start_epoch(), ep(1));
    let good = s[0].send(b"real").unwrap();
    assert_eq!(s[2].receive(&good).unwrap().1, b"real");
}

#[test]
fn an_epoch_far_ahead_with_a_bad_tag_installs_nothing() {
    let mut s = circle(3);
    let w1 = s[0].send(b"e1").unwrap();
    s[1].receive(&w1).unwrap();
    // Epoch 60000 in the header: the header is authenticated, so the tag fails.
    let mut far = w1.clone();
    far[20..22].copy_from_slice(&60_000u16.to_le_bytes());
    assert_eq!(s[1].receive(&far).unwrap_err(), ChatError::Tag);
    assert_eq!(s[1].current_epoch(), Some(ep(1)));
    assert_eq!(s[1].epochs(), 1);
    // A new epoch from this member still follows the highest it has read.
    assert_eq!(s[1].start_epoch(), ep(2));
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
    assert_eq!(epoch_of(&latest), ep(501));
    assert_eq!(s[2].receive(&latest).unwrap().1, b"e501");
    assert_eq!(s[2].current_epoch(), Some(ep(501)));
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
    assert_eq!(back.current_epoch(), Some(ep(2)));
    assert_eq!(back.receive(&w1).unwrap().1, b"b");
    assert_eq!(back.receive(&w0).unwrap_err(), ChatError::Replay);
    let w2 = back.send(b"c").unwrap();
    assert_eq!(epoch_of(&w2), ep(2));
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
    assert_eq!(epoch_of(&next), ep(2));
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
