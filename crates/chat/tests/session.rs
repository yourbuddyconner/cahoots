use cahoots_chat::{ChatError, ChatSession, MAX_EPOCHS, MAX_SKIP, ROTATE_AFTER};
use cahoots_core::ledger::{MemLedger, SpentLedger};
use cahoots_core::pad::{MemPadStorage, Pad};
use cahoots_core::params::CircleParams;

type Session = ChatSession<MemPadStorage, MemLedger>;

fn circle(n: u8, len: u32) -> Vec<Session> {
    let params = CircleParams::new(n, len, "chat", false).unwrap();
    let bytes: Vec<u8> = (0..len)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8 ^ (i as u8))
        .collect();
    (0..n)
        .map(|i| {
            let pad = Pad::new(
                params.clone(),
                i,
                MemPadStorage::new(bytes.clone()),
                MemLedger::new(n),
            )
            .unwrap();
            ChatSession::new(pad, i)
        })
        .collect()
}

#[test]
fn three_members_round_trip() {
    let mut s = circle(3, 4096);
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
fn starting_an_epoch_spends_32_bytes_from_own_slice_and_receivers_spend_the_same() {
    let mut s = circle(2, 2048);
    let w = s[1].send(b"x").unwrap();
    let h = cahoots_chat::parse(&w).unwrap().0;
    assert_eq!(h.epoch.slice, 1);
    assert_eq!(h.epoch.offset, 0);
    assert!(s[1].pad().ledger().is_spent(1, 0, 32));
    assert!(!s[0].pad().ledger().is_spent(1, 0, 32));
    s[0].receive(&w).unwrap();
    assert!(s[0].pad().ledger().is_spent(1, 0, 32));
    let mut buf = [0u8; 32];
    cahoots_core::pad::PadStorage::read(s[0].pad().storage(), 1024, &mut buf);
    assert_eq!(buf, [0u8; 32], "receiver erased the epoch range");
}

#[test]
fn rotates_after_rotate_after_own_messages() {
    let mut s = circle(2, 1 << 16);
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
    assert_eq!(h.epoch.offset, 32);
    assert_eq!(h.counter, 0);
    assert_eq!(s[1].receive(&w).unwrap().1, b"rotated");
}

#[test]
fn members_switch_to_newest_epoch_seen() {
    let mut s = circle(3, 4096);
    let w0 = s[0].send(b"a").unwrap();
    s[1].receive(&w0).unwrap();
    s[2].receive(&w0).unwrap();
    s[1].start_epoch().unwrap();
    let w1 = s[1].send(b"b").unwrap();
    let e1 = cahoots_chat::parse(&w1).unwrap().0.epoch;
    assert_eq!(e1.slice, 1);
    s[0].receive(&w1).unwrap();
    s[2].receive(&w1).unwrap();
    let w2 = s[2].send(b"c").unwrap();
    assert_eq!(cahoots_chat::parse(&w2).unwrap().0.epoch, e1);
    assert_eq!(s[0].receive(&w2).unwrap().1, b"c");
    assert_eq!(s[1].receive(&w2).unwrap().1, b"c");
}

#[test]
fn receiver_joining_mid_epoch_skips_ahead() {
    let mut s = circle(2, 4096);
    let _ = s[0].send(b"0").unwrap();
    let _ = s[0].send(b"1").unwrap();
    let w = s[0].send(b"2").unwrap();
    let (h, pt) = s[1].receive(&w).unwrap();
    assert_eq!(h.counter, 2);
    assert_eq!(pt, b"2");
}

#[test]
fn out_of_order_delivery_uses_skipped_keys() {
    let mut s = circle(2, 4096);
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
    let mut s = circle(2, 1 << 16);
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
    let mut s = circle(2, 4096);
    let w = s[0].send(b"intact").unwrap();
    let mut bad = w.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(s[1].receive(&bad).unwrap_err(), ChatError::Tag);
    assert_eq!(s[1].receive(&w).unwrap().1, b"intact");
}

#[test]
fn tamper_on_unknown_epoch_still_spends_the_epoch_range() {
    let mut s = circle(2, 4096);
    let w = s[0].send(b"x").unwrap();
    let mut bad = w.clone();
    bad[40] ^= 1;
    assert_eq!(s[1].receive(&bad).unwrap_err(), ChatError::Tag);
    // The epoch was spent on first sight (the spec says so), and the good copy still works.
    assert!(s[1].pad().ledger().is_spent(0, 0, 32));
    assert_eq!(s[1].receive(&w).unwrap().1, b"x");
}

#[test]
fn replay_is_rejected() {
    let mut s = circle(2, 4096);
    let w = s[0].send(b"once").unwrap();
    s[1].receive(&w).unwrap();
    assert_eq!(s[1].receive(&w).unwrap_err(), ChatError::Replay);
}

#[test]
fn epoch_pointing_at_spent_bytes_is_reuse() {
    let mut s = circle(2, 4096);
    // Member 1 spends slice-1 bytes 0..32 on a pure-pad message it never sends anywhere.
    let _ = s[1].pad_mut().spend_next(32).unwrap();
    // Member 0 sees a forged chat header claiming epoch (1, 0).
    let mut forged = circle(2, 4096);
    let w = forged[1].send(b"forged").unwrap();
    assert_eq!(
        s[0].receive(&w).unwrap().1,
        b"forged",
        "member 0 had not spent it"
    );
    assert_eq!(s[1].receive(&w).unwrap_err(), ChatError::Reuse);
}

#[test]
fn wrong_circle_and_bad_sender() {
    let mut s = circle(2, 4096);
    let mut other = circle(2, 4096);
    let mut o = other.remove(0);
    let w = o.send(b"elsewhere").unwrap();
    // Different pad bytes, so a different circle id.
    let params = CircleParams::new(2, 4096, "other", false).unwrap();
    let bytes: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(7919) >> 5) as u8)
        .collect();
    let pad = Pad::new(params, 0, MemPadStorage::new(bytes), MemLedger::new(2)).unwrap();
    let mut x: Session = ChatSession::new(pad, 0);
    assert_eq!(x.receive(&w).unwrap_err(), ChatError::WrongCircle);
    let mut bad = w.clone();
    bad[22] = 7;
    assert_eq!(s[1].receive(&bad).unwrap_err(), ChatError::BadSender);
    drop(o);
}

#[test]
fn evicts_oldest_epoch_beyond_max_epochs() {
    let mut s = circle(2, 1 << 16);
    let w_first = s[0].send(b"first epoch, delivered").unwrap();
    let w_late = s[0].send(b"first epoch, delivered late").unwrap();
    s[1].receive(&w_first).unwrap();
    for _ in 0..MAX_EPOCHS {
        s[0].start_epoch().unwrap();
        let w = s[0].send(b"e").unwrap();
        s[1].receive(&w).unwrap();
    }
    assert_eq!(s[1].epochs(), MAX_EPOCHS);
    // The first epoch's state is gone and its range is already spent, so a late message
    // from it reads as reuse rather than as a fresh epoch.
    assert_eq!(s[1].receive(&w_late).unwrap_err(), ChatError::Reuse);
}

#[test]
fn state_export_import_roundtrip() {
    let mut s = circle(2, 4096);
    let w0 = s[0].send(b"a").unwrap();
    s[1].receive(&w0).unwrap();
    let w1 = s[0].send(b"b").unwrap();
    let w2 = s[0].send(b"c").unwrap();
    s[1].receive(&w2).unwrap();
    let state = s[1].export_state();
    let pad = s.remove(1).into_pad();
    let mut restored: Session = ChatSession::import_state(pad, 1, &state).unwrap();
    assert_eq!(restored.receive(&w1).unwrap().1, b"b");
    assert_eq!(restored.receive(&w2).unwrap_err(), ChatError::Replay);
    let w3 = restored.send(b"d").unwrap();
    assert_eq!(s[0].receive(&w3).unwrap().1, b"d");
    assert!(ChatSession::<MemPadStorage, MemLedger>::import_state(
        restored.into_pad(),
        1,
        &state[..state.len() / 2]
    )
    .is_err());
}

#[test]
fn exhausted_slice_cannot_start_epoch() {
    let mut s = circle(2, 128);
    s[0].start_epoch().unwrap();
    s[0].start_epoch().unwrap();
    assert_eq!(
        s[0].start_epoch().unwrap_err(),
        ChatError::Pad(cahoots_core::pad::PadError::Exhausted)
    );
}
