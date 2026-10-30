mod common;

use cahoots_channel::keys::{epoch_key, kid_of, EpochId, Key};
use cahoots_channel::message::{split, MessageType};
use cahoots_channel::session::{ChannelSession, MAX_EPOCHS};
use cahoots_channel::ChannelError;
use cahoots_chat::MAX_SKIP;
use common::seat_key;

const CHANNEL: [u8; 16] = [3; 16];

fn epoch(n: u32) -> EpochId {
    EpochId {
        channel_id: CHANNEL,
        epoch: n,
    }
}

fn key(n: u32) -> Key {
    epoch_key(&epoch(n), &[n as u8 ^ 0x5a; 32])
}

/// A member on seat key `i`, holding epoch 1.
fn member(i: u8) -> ChannelSession {
    let mut s = ChannelSession::new(CHANNEL, seat_key(i));
    s.install_epoch(epoch(1), key(1)).unwrap();
    s
}

fn text(s: &mut ChannelSession, body: &[u8]) -> Vec<u8> {
    s.send(MessageType::Text, body, &[]).unwrap()
}

#[test]
fn two_members_on_one_epoch_exchange_text_both_ways() {
    let mut alice = member(0);
    let mut bob = member(1);
    assert_eq!(alice.my_kid(), kid_of(&seat_key(0).public()));
    assert_eq!(alice.my_public(), seat_key(0).public());
    assert_eq!(alice.current_epoch(), Some(epoch(1)));

    let w = text(&mut alice, b"hello bob");
    let (h, pt, approvals) = bob.receive(&w, &alice.my_public()).unwrap();
    assert_eq!(pt, b"hello bob");
    assert!(approvals.is_empty());
    assert_eq!(h.kid, alice.my_kid());
    assert_eq!(h.epoch_id, epoch(1));
    assert_eq!(h.counter, 0);
    assert_eq!(h.len, 9);
    assert_eq!(h.msg_type, MessageType::Text);

    let w = text(&mut bob, b"hello alice");
    assert_eq!(
        alice.receive(&w, &bob.my_public()).unwrap().1,
        b"hello alice"
    );
    let w = text(&mut alice, b"again");
    let (h, pt, _) = bob.receive(&w, &alice.my_public()).unwrap();
    assert_eq!((h.counter, &pt[..]), (1, &b"again"[..]));
    assert_eq!(alice.sent_in_current(), 2);

    // Approvals ride along and come back out.
    let approvals = vec![vec![1, 2, 3], vec![]];
    let w = alice
        .send(MessageType::Announcement, b"news", &approvals)
        .unwrap();
    let (h, _, got) = bob.receive(&w, &alice.my_public()).unwrap();
    assert_eq!(h.msg_type, MessageType::Announcement);
    assert_eq!(got, approvals);

    // A replayed message is refused.
    assert_eq!(
        bob.receive(&w, &alice.my_public()),
        Err(ChannelError::Reuse)
    );
}

#[test]
fn my_own_message_echoed_by_the_relay_opens() {
    let mut alice = member(0);
    let w0 = text(&mut alice, b"one");
    let w1 = text(&mut alice, b"two");
    let me = alice.my_public();
    assert_eq!(alice.receive(&w0, &me).unwrap().1, b"one");
    assert_eq!(alice.receive(&w1, &me).unwrap().1, b"two");
    // Sending goes on from where it was, not from the receive side.
    let w2 = text(&mut alice, b"three");
    assert_eq!(split(&w2).unwrap().2.counter, 2);
}

#[test]
fn a_skipped_message_still_opens_later_ones_within_max_skip() {
    let mut alice = member(0);
    let mut bob = member(1);
    let a = alice.my_public();
    let wires: Vec<Vec<u8>> = (0..4u8).map(|i| text(&mut alice, &[i; 5])).collect();
    // Message 1 is lost for now; 0, 2 and 3 open.
    assert_eq!(bob.receive(&wires[0], &a).unwrap().1, [0; 5]);
    assert_eq!(bob.receive(&wires[2], &a).unwrap().1, [2; 5]);
    assert_eq!(bob.receive(&wires[3], &a).unwrap().1, [3; 5]);
    // It arrives late and still opens, once.
    assert_eq!(bob.receive(&wires[1], &a).unwrap().1, [1; 5]);
    assert_eq!(bob.receive(&wires[1], &a), Err(ChannelError::Reuse));

    // A gap of exactly MAX_SKIP opens; one more is too far ahead.
    let mut carol = member(2);
    let mut dave = member(1);
    let c = carol.my_public();
    let wires: Vec<Vec<u8>> = (0..MAX_SKIP + 2).map(|_| text(&mut carol, b"x")).collect();
    let before = dave.export_state();
    assert_eq!(
        dave.receive(&wires[MAX_SKIP as usize + 1], &c),
        Err(ChannelError::TooFarAhead)
    );
    assert_eq!(*dave.export_state(), *before);
    assert!(dave.receive(&wires[MAX_SKIP as usize], &c).is_ok());
    assert!(dave.receive(&wires[0], &c).is_ok());
}

#[test]
fn past_max_skip_skipped_keys_the_earliest_are_forgotten() {
    let mut carol = member(2);
    let mut dave = member(1);
    let c = carol.my_public();
    let m = MAX_SKIP as usize;
    let wires: Vec<Vec<u8>> = (0..m + 4).map(|_| text(&mut carol, b"x")).collect();
    // Open 0, then m + 1 (skipping 1..=m, a full cache), then m + 3 (skipping m + 2 too).
    dave.receive(&wires[0], &c).unwrap();
    dave.receive(&wires[m + 1], &c).unwrap();
    dave.receive(&wires[m + 3], &c).unwrap();
    // The cache keeps MAX_SKIP keys: the earliest skipped (counter 1) is gone, the rest open.
    assert_eq!(dave.receive(&wires[1], &c), Err(ChannelError::Reuse));
    assert!(dave.receive(&wires[2], &c).is_ok());
    assert!(dave.receive(&wires[m], &c).is_ok());
    assert!(dave.receive(&wires[m + 2], &c).is_ok());
}

#[test]
fn two_readers_decrypt_the_same_message_independently() {
    let mut alice = member(0);
    let mut bob = member(1);
    let mut carol = member(2);
    let a = alice.my_public();
    let w0 = text(&mut alice, b"first");
    let w1 = text(&mut alice, b"second");
    // Bob reads both in order; Carol reads only the second, then the first.
    assert_eq!(bob.receive(&w0, &a).unwrap().1, b"first");
    assert_eq!(carol.receive(&w1, &a).unwrap().1, b"second");
    assert_eq!(bob.receive(&w1, &a).unwrap().1, b"second");
    assert_eq!(carol.receive(&w0, &a).unwrap().1, b"first");
}

#[test]
fn unknown_epoch_is_refused_and_state_unchanged() {
    let mut alice = member(0);
    let mut bob = ChannelSession::new(CHANNEL, seat_key(1));
    assert_eq!(
        bob.send(MessageType::Text, b"x", &[]),
        Err(ChannelError::UnknownEpoch)
    );
    let a = alice.my_public();
    let w = text(&mut alice, b"hi");
    let before = bob.export_state();
    assert_eq!(bob.receive(&w, &a), Err(ChannelError::UnknownEpoch));
    assert_eq!(*bob.export_state(), *before);

    // The wrong key for the epoch: the tag fails and nothing moves.
    bob.install_epoch(epoch(1), key(2)).unwrap();
    let before = bob.export_state();
    assert_eq!(bob.receive(&w, &a), Err(ChannelError::Tag));
    assert_eq!(*bob.export_state(), *before);

    // A message from another channel is refused.
    let mut other = ChannelSession::new([4; 16], seat_key(0));
    other
        .install_epoch(
            EpochId {
                channel_id: [4; 16],
                epoch: 1,
            },
            key(1),
        )
        .unwrap();
    let w = text(&mut other, b"elsewhere");
    assert_eq!(bob.receive(&w, &a), Err(ChannelError::WrongChannel));
}

#[test]
fn epochs_are_installed_once_and_at_most_four_are_kept() {
    let mut s = member(0);
    assert_eq!(s.install_epoch(epoch(1), key(9)), Err(ChannelError::Reuse));
    assert_eq!(
        s.install_epoch(
            EpochId {
                channel_id: [4; 16],
                epoch: 2
            },
            key(2)
        ),
        Err(ChannelError::WrongChannel)
    );
    text(&mut s, b"x");
    for n in 2..=MAX_EPOCHS as u32 + 1 {
        s.install_epoch(epoch(n), key(n)).unwrap();
        assert_eq!(s.current_epoch(), Some(epoch(n)));
        assert_eq!(s.sent_in_current(), 0);
    }
    assert!(!s.has_epoch(&epoch(1)));
    for n in 2..=MAX_EPOCHS as u32 + 1 {
        assert!(s.has_epoch(&epoch(n)));
    }
}

#[test]
fn an_older_epoch_installed_late_is_for_reading_only() {
    let mut alice = member(0);
    alice.install_epoch(epoch(3), key(3)).unwrap();
    let a = alice.my_public();
    let old = {
        let mut a1 = member(0);
        text(&mut a1, b"from epoch one")
    };
    let new = text(&mut alice, b"from epoch three");

    // Bob gets epoch 3 first, then a catch-up deposit for epoch 1.
    let mut bob = ChannelSession::new(CHANNEL, seat_key(1));
    bob.install_epoch(epoch(3), key(3)).unwrap();
    text(&mut bob, b"sent in three");
    bob.install_epoch(epoch(1), key(1)).unwrap();
    assert_eq!(bob.current_epoch(), Some(epoch(3)));
    assert_eq!(bob.sent_in_current(), 1);
    assert_eq!(bob.receive(&old, &a).unwrap().1, b"from epoch one");
    assert_eq!(bob.receive(&new, &a).unwrap().1, b"from epoch three");
    // Bob still sends under epoch 3.
    let w = text(&mut bob, b"still three");
    assert_eq!(split(&w).unwrap().2.epoch_id, epoch(3));
}

#[test]
fn a_fifth_epoch_drops_the_lowest_number_and_it_never_comes_back() {
    let mut s = ChannelSession::new(CHANNEL, seat_key(0));
    // Installed out of order: 5, 2, 9, 4.
    for n in [5, 2, 9, 4] {
        s.install_epoch(epoch(n), key(n)).unwrap();
    }
    assert_eq!(s.current_epoch(), Some(epoch(9)));
    // A fifth, lower than the current, drops 2 (the lowest), not 9.
    s.install_epoch(epoch(7), key(7)).unwrap();
    assert!(!s.has_epoch(&epoch(2)));
    for n in [4, 5, 7, 9] {
        assert!(s.has_epoch(&epoch(n)));
    }
    assert_eq!(s.current_epoch(), Some(epoch(9)));

    // The dropped number, or anything below it, is refused and changes nothing.
    let before = s.export_state();
    assert_eq!(s.install_epoch(epoch(2), key(2)), Err(ChannelError::Reuse));
    assert_eq!(s.install_epoch(epoch(1), key(1)), Err(ChannelError::Reuse));
    // With the window full, an epoch below every one held would be dropped at once: refused.
    assert_eq!(s.install_epoch(epoch(3), key(3)), Err(ChannelError::Reuse));
    assert_eq!(*s.export_state(), *before);

    // The refusal survives export and import.
    let mut back = ChannelSession::import_state(&s.export_state()).unwrap();
    assert_eq!(
        back.install_epoch(epoch(2), key(2)),
        Err(ChannelError::Reuse)
    );
    assert_eq!(back.current_epoch(), Some(epoch(9)));

    // A higher one still goes in, drops 4 and becomes current.
    back.install_epoch(epoch(10), key(10)).unwrap();
    assert!(!back.has_epoch(&epoch(4)));
    assert_eq!(back.current_epoch(), Some(epoch(10)));
    assert_eq!(
        back.install_epoch(epoch(3), key(3)),
        Err(ChannelError::Reuse)
    );
}

#[test]
fn export_import_keeps_chains_and_counters() {
    let mut alice = member(0);
    let mut bob = member(1);
    let a = alice.my_public();
    let wires: Vec<Vec<u8>> = (0..4u8).map(|i| text(&mut alice, &[i])).collect();
    bob.receive(&wires[0], &a).unwrap();
    bob.receive(&wires[2], &a).unwrap(); // leaves 1 skipped
    text(&mut bob, b"mine");
    bob.install_epoch(epoch(2), key(2)).unwrap();
    text(&mut bob, b"in two");
    text(&mut bob, b"in two again");

    let state = bob.export_state();
    let mut back = ChannelSession::import_state(&state).unwrap();
    assert_eq!(*back.export_state(), *state);
    assert_eq!(back.my_kid(), bob.my_kid());
    assert_eq!(back.current_epoch(), Some(epoch(2)));
    assert_eq!(back.sent_in_current(), 2);
    assert!(back.has_epoch(&epoch(1)));
    // The skipped key and the chain position both survive.
    assert_eq!(back.receive(&wires[1], &a).unwrap().1, [1]);
    assert_eq!(back.receive(&wires[3], &a).unwrap().1, [3]);
    assert_eq!(back.receive(&wires[2], &a), Err(ChannelError::Reuse));
    // Sending carries on at counter 2 in epoch 2.
    let w = text(&mut back, b"after");
    let h = split(&w).unwrap().2;
    assert_eq!((h.epoch_id, h.counter), (epoch(2), 2));

    // Changed, short or trailing bytes are refused.
    let mut longer = state.to_vec();
    longer.push(0);
    assert!(ChannelSession::import_state(&longer).is_err());
    assert!(ChannelSession::import_state(&state[..state.len() - 1]).is_err());
}

#[test]
fn wrong_signer_is_refused_before_any_chain_moves() {
    let mut alice = member(0);
    let mut bob = member(1);
    let w = text(&mut alice, b"signed by alice");
    let before = bob.export_state();
    assert_eq!(
        bob.receive(&w, &seat_key(2).public()),
        Err(ChannelError::Signature)
    );
    assert_eq!(*bob.export_state(), *before);
    // A flipped ciphertext byte breaks the signature too.
    let mut bad = w.clone();
    let body_len = split(&w).unwrap().0.len();
    bad[body_len - 1] ^= 1;
    assert_eq!(
        bob.receive(&bad, &alice.my_public()),
        Err(ChannelError::Signature)
    );
    assert_eq!(*bob.export_state(), *before);
    assert_eq!(
        bob.receive(&w, &alice.my_public()).unwrap().1,
        b"signed by alice"
    );
}
