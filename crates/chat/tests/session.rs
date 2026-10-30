use cahoots_chat::{ChatError, ChatSession, EpochId, MAX_EPOCHS, MAX_SKIP, ROTATE_AFTER};

const ROOT: [u8; 32] = [0x42; 32];
const CID: [u8; 16] = [0x0c; 16];

/// `n` members of one circle, each already synced with an empty relay.
fn circle(n: u8) -> Vec<ChatSession> {
    (0..n)
        .map(|i| {
            let mut s = ChatSession::new(&ROOT, CID, n, i);
            s.mark_synced();
            s
        })
        .collect()
}

#[test]
fn three_members_round_trip() {
    let mut s = circle(3);
    let w = s[0].send(b"hi from 0").unwrap();
    for member in s.iter_mut().skip(1) {
        let (h, pt) = member.receive(&w).unwrap();
        assert_eq!(pt, b"hi from 0");
        assert_eq!(h.sender, 0);
        assert_eq!(h.counter, 0);
    }
    let w1 = s[1].send(b"hi from 1").unwrap();
    assert_eq!(s[0].receive(&w1).unwrap().1, b"hi from 1");
    assert_eq!(s[2].receive(&w1).unwrap().1, b"hi from 1");
    // Member 1 kept using member 0's epoch.
    let h = cahoots_chat::parse(&w1).unwrap().0;
    assert_eq!(h.epoch, cahoots_chat::parse(&w).unwrap().0.epoch);
}

#[test]
fn the_first_epoch_is_root_epoch_one_whoever_starts_it() {
    let mut s = circle(2);
    let w = s[1].send(b"x").unwrap();
    let h = cahoots_chat::parse(&w).unwrap().0;
    assert_eq!(h.epoch, EpochId::root(1));
    s[0].receive(&w).unwrap();
    assert_eq!(s[0].current_epoch(), Some(EpochId::root(1)));
}

#[test]
fn rotates_after_rotate_after_own_messages() {
    let mut s = circle(2);
    let first = cahoots_chat::parse(&s[0].send(b"m").unwrap())
        .unwrap()
        .0
        .epoch;
    let mut last = first;
    for _ in 1..ROTATE_AFTER {
        last = cahoots_chat::parse(&s[0].send(b"m").unwrap())
            .unwrap()
            .0
            .epoch;
    }
    assert_eq!(last, first);
    let w = s[0].send(b"rotated").unwrap();
    let h = cahoots_chat::parse(&w).unwrap().0;
    assert_ne!(h.epoch, first);
    assert_eq!(h.epoch, EpochId::root(2));
    assert_eq!(h.counter, 0);
    assert_eq!(s[1].receive(&w).unwrap().1, b"rotated");
}

#[test]
fn members_switch_to_newest_epoch_seen() {
    let mut s = circle(3);
    let w0 = s[0].send(b"a").unwrap();
    s[1].receive(&w0).unwrap();
    s[2].receive(&w0).unwrap();
    s[1].start_epoch();
    let w1 = s[1].send(b"b").unwrap();
    let e1 = cahoots_chat::parse(&w1).unwrap().0.epoch;
    assert_eq!(e1, EpochId::root(2));
    s[0].receive(&w1).unwrap();
    s[2].receive(&w1).unwrap();
    let w2 = s[2].send(b"c").unwrap();
    assert_eq!(cahoots_chat::parse(&w2).unwrap().0.epoch, e1);
    assert_eq!(s[0].receive(&w2).unwrap().1, b"c");
    assert_eq!(s[1].receive(&w2).unwrap().1, b"c");
}

#[test]
fn receiver_joining_mid_epoch_skips_ahead() {
    let mut s = circle(2);
    let _ = s[0].send(b"0").unwrap();
    let _ = s[0].send(b"1").unwrap();
    let w = s[0].send(b"2").unwrap();
    let (h, pt) = s[1].receive(&w).unwrap();
    assert_eq!(h.counter, 2);
    assert_eq!(pt, b"2");
}

#[test]
fn out_of_order_delivery_uses_skipped_keys() {
    let mut s = circle(2);
    let w0 = s[0].send(b"0").unwrap();
    let w1 = s[0].send(b"1").unwrap();
    let w2 = s[0].send(b"2").unwrap();
    assert_eq!(s[1].receive(&w2).unwrap().1, b"2");
    assert_eq!(s[1].receive(&w0).unwrap().1, b"0");
    assert_eq!(s[1].receive(&w1).unwrap().1, b"1");
    assert_eq!(s[1].receive(&w1).unwrap_err(), ChatError::Replay);
}

#[test]
fn too_far_ahead_is_rejected_without_state_change() {
    let mut s = circle(2);
    let w0 = s[0].send(b"0").unwrap();
    s[1].receive(&w0).unwrap();
    let mut wires = Vec::new();
    for _ in 0..=MAX_SKIP + 1 {
        wires.push(s[0].send(b"n").unwrap());
    }
    let far = wires.last().unwrap();
    assert_eq!(s[1].receive(far).unwrap_err(), ChatError::TooFarAhead);
    // The next in-order message still decrypts, so the failed receive changed nothing.
    assert_eq!(s[1].receive(&wires[0]).unwrap().1, b"n");
}

#[test]
fn tamper_fails_tag_and_leaves_state_intact() {
    let mut s = circle(2);
    let w = s[0].send(b"intact").unwrap();
    let mut bad = w.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(s[1].receive(&bad).unwrap_err(), ChatError::Tag);
    assert_eq!(s[1].receive(&w).unwrap().1, b"intact");
}

#[test]
fn tamper_on_unknown_epoch_keeps_nothing() {
    let mut s = circle(2);
    let w = s[0].send(b"x").unwrap();
    let mut bad = w.clone();
    bad[40] ^= 1;
    assert_eq!(s[1].receive(&bad).unwrap_err(), ChatError::Tag);
    assert_eq!(s[1].epochs(), 0);
    assert_eq!(s[1].current_epoch(), None);
    assert_eq!(s[1].receive(&w).unwrap().1, b"x");
}

#[test]
fn replay_is_rejected() {
    let mut s = circle(2);
    let w = s[0].send(b"once").unwrap();
    s[1].receive(&w).unwrap();
    assert_eq!(s[1].receive(&w).unwrap_err(), ChatError::Replay);
}

#[test]
fn wrong_circle_and_bad_sender() {
    let mut s = circle(2);
    let mut other = circle(2);
    let mut o = other.remove(0);
    let w = o.send(b"elsewhere").unwrap();
    let mut x = ChatSession::new(&ROOT, [0x0d; 16], 2, 0);
    assert_eq!(x.receive(&w).unwrap_err(), ChatError::WrongCircle);
    let mut bad = w.clone();
    bad[22] = 7;
    assert_eq!(s[1].receive(&bad).unwrap_err(), ChatError::BadSender);
    drop(o);
}

#[test]
fn evicts_oldest_epoch_beyond_max_epochs() {
    let mut s = circle(2);
    let w_first = s[0].send(b"first epoch, delivered").unwrap();
    let w_late = s[0].send(b"first epoch, delivered late").unwrap();
    s[1].receive(&w_first).unwrap();
    for _ in 0..MAX_EPOCHS {
        s[0].start_epoch();
        let w = s[0].send(b"e").unwrap();
        s[1].receive(&w).unwrap();
    }
    assert_eq!(s[1].epochs(), MAX_EPOCHS);
    // The first epoch's state is gone, and it is not derived again from the root, which
    // would let its messages be replayed.
    assert_eq!(s[1].receive(&w_late).unwrap_err(), ChatError::UnknownEpoch);
}

#[test]
fn state_export_import_roundtrip() {
    let mut s = circle(2);
    let w0 = s[0].send(b"a").unwrap();
    s[1].receive(&w0).unwrap();
    let w1 = s[0].send(b"b").unwrap();
    let w2 = s[0].send(b"c").unwrap();
    s[1].receive(&w2).unwrap();
    let state = s[1].export_state();
    let mut restored = ChatSession::import_state(&ROOT, CID, 2, 1, &state).unwrap();
    restored.mark_synced();
    assert_eq!(restored.receive(&w1).unwrap().1, b"b");
    assert_eq!(restored.receive(&w2).unwrap_err(), ChatError::Replay);
    let w3 = restored.send(b"d").unwrap();
    assert_eq!(s[0].receive(&w3).unwrap().1, b"d");
    assert!(ChatSession::import_state(&ROOT, CID, 2, 1, &state[..state.len() / 2]).is_err());
}

#[test]
fn a_session_from_another_root_reads_nothing() {
    let mut s = circle(2);
    let mut other = ChatSession::new(&[0x43; 32], CID, 2, 1);
    let w = s[0].send(b"x").unwrap();
    assert_eq!(other.receive(&w).unwrap_err(), ChatError::Tag);
}
