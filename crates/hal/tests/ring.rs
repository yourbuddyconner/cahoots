//! The ceremony driver in-process: a pair over one cable, a ring with a cable to each
//! neighbour, and the rules a device follows on its links.

use cahoots_core::checkwords::check_words;
use cahoots_core::params::CircleParams;
use cahoots_hal::ring::{Frame, RingCeremony, RingError, Step};
use cahoots_hal::{Entropy, HealthError, Link};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

type Queue = Rc<RefCell<VecDeque<u8>>>;

/// tx pushes into the next device's queue, rx pops from our own.
struct QLink {
    tx: Queue,
    rx: Queue,
}
impl Link for QLink {
    fn send(&mut self, frame: &[u8]) {
        self.tx.borrow_mut().extend(frame);
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        let mut q = self.rx.borrow_mut();
        let n = buf.len().min(q.len());
        for b in buf.iter_mut().take(n) {
            *b = q.pop_front().unwrap();
        }
        n
    }
    fn peer_present(&self) -> bool {
        true
    }
}

struct Det(u64);
impl Entropy for Det {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        for b in buf.iter_mut() {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
        Ok(())
    }
}

fn ring(n: usize) -> Vec<QLink> {
    let queues: Vec<Queue> = (0..n)
        .map(|_| Rc::new(RefCell::new(VecDeque::new())))
        .collect();
    (0..n)
        .map(|i| QLink {
            tx: queues[(i + 1) % n].clone(),
            rx: queues[i].clone(),
        })
        .collect()
}

fn params() -> CircleParams {
    CircleParams::new(4, 1024, "ring-test", false).unwrap()
}

#[test]
fn reveal_is_withheld_until_all_commitments_held() {
    let mut links = ring(2);
    let other = links.pop().unwrap();
    let mine = links.pop().unwrap();
    let p = CircleParams::new(2, 256, "two", false).unwrap();
    let mut a = RingCeremony::new(p.clone(), 0, &mut Det(1), vec![mine], 0).unwrap();
    let _b_link = other;
    a.start();
    assert_eq!(a.poll().unwrap(), Step::Waiting);
    // Only a's own params and commit are on the wire. No reveal.
    let bytes: Vec<u8> = a.links()[0].tx.borrow().iter().copied().collect();
    let mut off = 0;
    let mut kinds = vec![];
    while off < bytes.len() {
        let (f, used) = cahoots_core::frame::decode(&bytes[off..]).unwrap();
        kinds.push(f.kind);
        off += used;
    }
    assert_eq!(kinds, vec![Frame::Params as u8, Frame::Commit as u8]);
}

#[test]
fn params_mismatch_aborts() {
    let links = ring(2);
    let mut it = links.into_iter();
    let p0 = CircleParams::new(2, 256, "x", false).unwrap();
    let p1 = CircleParams::new(2, 256, "y", false).unwrap();
    let mut a = RingCeremony::new(p0, 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut b = RingCeremony::new(p1, 1, &mut Det(2), vec![it.next().unwrap()], 0).unwrap();
    a.start();
    b.start();
    assert_eq!(b.poll().unwrap_err(), RingError::ParamsMismatch);
    assert_eq!(a.poll().unwrap_err(), RingError::ParamsMismatch);
}

#[test]
fn entropy_failure_aborts_before_anything_is_sent() {
    struct Bad;
    impl Entropy for Bad {
        fn fill(&mut self, _: &mut [u8]) -> Result<(), HealthError> {
            Err(HealthError::AdaptiveProportion)
        }
    }
    let links = ring(2);
    let l = links.into_iter().next().unwrap();
    let p = CircleParams::new(2, 256, "x", false).unwrap();
    let err = match RingCeremony::new(p, 0, &mut Bad, vec![l], 0) {
        Err(e) => e,
        Ok(_) => panic!("expected health failure"),
    };
    assert_eq!(err, RingError::Health(HealthError::AdaptiveProportion));
}

#[test]
fn run_flushes_the_link_before_finishing() {
    /// A link whose writes sit in a queue until flushed.
    struct Lazy {
        inner: QLink,
        queued: Vec<u8>,
        flushes: usize,
    }
    impl Link for Lazy {
        fn send(&mut self, frame: &[u8]) {
            self.queued.extend_from_slice(frame);
        }
        fn recv(&mut self, buf: &mut [u8]) -> usize {
            // Like a real driver, move a little toward the wire on every call. A few bytes
            // only, so that our attestation is still queued when the ceremony completes.
            let n = self.queued.len().min(8);
            let chunk: Vec<u8> = self.queued.drain(..n).collect();
            self.inner.send(&chunk);
            self.inner.recv(buf)
        }
        fn peer_present(&self) -> bool {
            true
        }
        fn flush(&mut self) -> bool {
            self.flushes += 1;
            // Hand over 16 bytes per call to force several rounds.
            let n = self.queued.len().min(16);
            let chunk: Vec<u8> = self.queued.drain(..n).collect();
            self.inner.send(&chunk);
            self.queued.is_empty()
        }
    }
    let links = ring(2);
    let mut it = links.into_iter();
    let p = CircleParams::new(2, 256, "two", false).unwrap();
    let lazy = Lazy {
        inner: it.next().unwrap(),
        queued: vec![],
        flushes: 0,
    };
    let other = it.next().unwrap();
    let a = RingCeremony::new(p.clone(), 0, &mut Det(1), vec![lazy], 0).unwrap();
    let mut b = RingCeremony::new(p, 1, &mut Det(2), vec![other], 0).unwrap();
    // Drive b from a helper thread is not possible with Rc; interleave by hand instead:
    // a.run needs b to answer, so pump b inside a's idle callback.
    b.start();
    let mut lines: Vec<String> = Vec::new();
    let (_result, links) = a
        .run(
            |s| lines.push(s.to_string()),
            || {
                let _ = b.poll();
            },
        )
        .unwrap();
    assert_eq!(lines[0], "ceremony: waiting for the others");
    assert_eq!(
        lines.last().unwrap(),
        "commits 2/2  reveals 2/2  attestations 2/2"
    );
    let link = &links[0];
    assert!(
        link.queued.is_empty(),
        "everything was flushed before finish"
    );
    assert!(link.flushes > 1);
    assert_eq!(b.poll().unwrap(), Step::Complete);
}

mod common;

/// Every device has a cable to each neighbour, so frames go round both ways and the dedupe
/// drops the second copy. This is how the host simulator and a closed ring of badges are wired.
#[test]
fn a_ring_with_a_cable_to_each_neighbour_completes() {
    use common::{cable, drive, End};
    let n = 4usize;
    let mut cables: Vec<Option<(End, End)>> = (0..n).map(|_| Some(cable())).collect();
    let mut prev: Vec<Option<End>> = (0..n).map(|_| None).collect();
    let mut next: Vec<Option<End>> = (0..n).map(|_| None).collect();
    for i in 0..n {
        let (a, b) = cables[i].take().unwrap();
        next[i] = Some(a);
        prev[(i + 1) % n] = Some(b);
    }
    let mut devs: Vec<RingCeremony<End>> = (0..n)
        .map(|i| {
            let links = vec![prev[i].take().unwrap(), next[i].take().unwrap()];
            RingCeremony::new(
                params(),
                i as u8,
                &mut common::Det(3000 + i as u64),
                links,
                0,
            )
            .unwrap()
        })
        .collect();
    drive(&mut devs);
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap().0).collect();
    let mut expected = vec![0u8; 1024];
    for i in 0..n as u64 {
        for (e, c) in expected
            .iter_mut()
            .zip(common::contribution(3000 + i, 1024))
        {
            *e ^= c;
        }
    }
    let words = check_words(&params(), &expected);
    for r in &results {
        assert_eq!(r.pad.as_slice(), expected.as_slice());
        assert_eq!(r.check_words, words);
    }
}

/// A frame is forwarded on every link except the one it arrived on.
#[test]
fn a_frame_is_not_echoed_on_the_link_it_arrived_on() {
    use common::{cable, kinds_on};
    let (peer0, mine0) = cable();
    let (_peer1, mine1) = cable();
    let p = CircleParams::new(3, 256, "line", false).unwrap();
    let mut me = RingCeremony::new(p.clone(), 1, &mut Det(1), vec![mine0, mine1], 0).unwrap();
    me.start();
    // Neighbour 0 sends its params and commitment down link 0.
    let mut n0 = RingCeremony::new(p, 0, &mut Det(2), vec![peer0], 0).unwrap();
    n0.start();
    assert_eq!(me.poll().unwrap(), Step::Waiting);
    let own = [Frame::Params as u8, Frame::Commit as u8];
    assert_eq!(
        kinds_on(&me.links()[0].tx),
        own.to_vec(),
        "link 0 carries only our own frames"
    );
    assert_eq!(
        kinds_on(&me.links()[1].tx),
        [own.as_slice(), own.as_slice()].concat(),
        "link 1 carries ours and the forwarded copy"
    );
}

/// A laptop standing in for the rest of a pile keeps sending `Ping` frames, which carry no
/// payload, and the pile layer may talk too. The ring ignores those kinds instead of
/// treating an empty payload as a protocol violation.
#[test]
fn pile_layer_and_ping_frames_during_the_ceremony_are_ignored() {
    let links = ring(2);
    let mut it = links.into_iter();
    let l0 = it.next().unwrap();
    let l1 = it.next().unwrap();
    let p = CircleParams::new(2, 256, "two", false).unwrap();
    let mut a = RingCeremony::new(p.clone(), 0, &mut Det(1), vec![l0], 0).unwrap();
    let mut b = RingCeremony::new(p, 1, &mut Det(2), vec![l1], 0).unwrap();
    a.start();
    b.start();
    let ping = cahoots_core::frame::encode(&cahoots_core::frame::Frame {
        kind: cahoots_hal::pile::KIND_PING,
        seq: 1,
        payload: &[],
    });
    let hello = cahoots_core::frame::encode(&cahoots_core::frame::Frame {
        kind: cahoots_hal::pile::KIND_HELLO,
        seq: 2,
        payload: &[],
    });
    for _ in 0..50 {
        // Pings and pile frames land on a's inbox between every poll.
        a.links()[0].rx.borrow_mut().extend(ping.iter().copied());
        a.links()[0].rx.borrow_mut().extend(hello.iter().copied());
        let sa = a.poll().expect("a ignores pile-layer frames");
        let sb = b.poll().expect("b runs");
        if sa == Step::Complete && sb == Step::Complete {
            break;
        }
    }
    let (ra, _) = a.finish().unwrap();
    let (rb, _) = b.finish().unwrap();
    assert_eq!(ra.circle_id, rb.circle_id);
}

// ---- The attest phase ----

fn two() -> CircleParams {
    CircleParams::new(2, 256, "two", false).unwrap()
}

/// Frames waiting in a queue, as (kind, payload); the queue is emptied.
fn take_frames(q: &Queue) -> Vec<(u8, Vec<u8>)> {
    let bytes: Vec<u8> = q.borrow_mut().drain(..).collect();
    let mut off = 0;
    let mut out = vec![];
    while off < bytes.len() {
        let (f, used) = cahoots_core::frame::decode(&bytes[off..]).unwrap();
        out.push((f.kind, f.payload.to_vec()));
        off += used;
    }
    out
}

fn put_frame(q: &Queue, kind: u8, payload: &[u8]) {
    let bytes = cahoots_core::frame::encode(&cahoots_core::frame::Frame {
        kind,
        seq: 0,
        payload,
    });
    q.borrow_mut().extend(bytes);
}

/// A copy of `payload` with its last byte flipped: for a sealed reveal, the AEAD tag; for a
/// COSE_Sign1, the signature.
fn flipped(payload: &[u8]) -> Vec<u8> {
    let mut p = payload.to_vec();
    *p.last_mut().unwrap() ^= 1;
    p
}

#[test]
fn three_devices_complete_through_attestation_and_the_manifests_agree() {
    use common::{cable, drive, End};
    let n = 3usize;
    let mut prev: Vec<Option<End>> = (0..n).map(|_| None).collect();
    let mut next: Vec<Option<End>> = (0..n).map(|_| None).collect();
    for i in 0..n {
        let (a, b) = cable();
        next[i] = Some(a);
        prev[(i + 1) % n] = Some(b);
    }
    let p = CircleParams::new(3, 512, "three", false).unwrap();
    let mut devs: Vec<RingCeremony<End>> = (0..n)
        .map(|i| {
            let links = vec![prev[i].take().unwrap(), next[i].take().unwrap()];
            let tier = i as u8;
            RingCeremony::new(p.clone(), i as u8, &mut Det(4000 + i as u64), links, tier).unwrap()
        })
        .collect();
    drive(&mut devs);
    for d in &devs {
        assert_eq!(d.attestations_held(), 3);
    }
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap().0).collect();
    let first = &results[0].manifest;
    assert_eq!(first.attestations.len(), 3);
    let m = first.verify().expect("every seat signed the manifest");
    let tiers: Vec<u8> = m.seats.iter().map(|s| s.tier).collect();
    assert_eq!(tiers, vec![0, 1, 2]);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(
            r.manifest, *first,
            "device {i} holds the same sealed manifest"
        );
        assert_eq!(r.tier, i as u8);
        assert_eq!(r.circle_id, results[0].circle_id);
    }
}

#[test]
fn an_attest_frame_before_our_phase_is_attest_is_buffered_and_applied() {
    let mut it = ring(2).into_iter();
    let mut a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut b = RingCeremony::new(two(), 1, &mut Det(2), vec![it.next().unwrap()], 0).unwrap();
    a.start();
    b.start();
    assert_eq!(b.poll().unwrap(), Step::Waiting); // b holds both commitments and reveals
    assert_eq!(a.poll().unwrap(), Step::Waiting); // a reveals and attests
                                                  // a's reveal and attestation wait in b's inbox; b reads the attestation while its own
                                                  // phase is still Reveal.
    assert_eq!(b.phase(), cahoots_core::ceremony::Phase::Reveal);
    let kinds = common::kinds_on(&b.links()[0].rx);
    assert_eq!(kinds, vec![Frame::Reveal as u8, Frame::Attest as u8]);
    assert_eq!(b.poll().unwrap(), Step::Complete);
    assert_eq!(b.attestations_held(), 2);
    assert_eq!(a.poll().unwrap(), Step::Complete);
}

/// Two devices up to the point where b's attestation sits in a's inbox, unread.
fn two_up_to_bs_attestation() -> (RingCeremony<QLink>, RingCeremony<QLink>) {
    let mut it = ring(2).into_iter();
    let mut a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut b = RingCeremony::new(two(), 1, &mut Det(2), vec![it.next().unwrap()], 0).unwrap();
    a.start();
    b.start();
    b.poll().unwrap();
    a.poll().unwrap();
    assert_eq!(b.poll().unwrap(), Step::Complete);
    assert_eq!(a.phase(), cahoots_core::ceremony::Phase::Attest);
    (a, b)
}

#[test]
fn an_attest_frame_with_a_bad_signature_does_not_abort_the_ring() {
    let (mut a, _b) = two_up_to_bs_attestation();
    let inbox = a.links()[0].rx.clone();
    let frames = take_frames(&inbox);
    let (kind, real) = frames.last().unwrap().clone();
    assert_eq!(kind, Frame::Attest as u8);
    put_frame(&inbox, kind, &flipped(&real));
    assert_eq!(
        a.poll().unwrap(),
        Step::Waiting,
        "a bad signature is dropped"
    );
    assert_eq!(a.attestations_held(), 1);
    // The real one still counts after the forged one was refused.
    put_frame(&inbox, kind, &real);
    assert_eq!(a.poll().unwrap(), Step::Complete);
}

#[test]
fn run_reports_attestations() {
    let mut it = ring(2).into_iter();
    let a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut b = RingCeremony::new(two(), 1, &mut Det(2), vec![it.next().unwrap()], 0).unwrap();
    b.start();
    let mut lines: Vec<String> = Vec::new();
    let (result, _) = a
        .run(
            |s| lines.push(s.to_string()),
            || {
                let _ = b.poll();
            },
        )
        .unwrap();
    assert_eq!(
        lines.last().unwrap(),
        "commits 2/2  reveals 2/2  attestations 2/2"
    );
    assert!(
        lines[1..].iter().all(|l| l.contains("  attestations ")),
        "every progress line counts attestations: {lines:?}"
    );
    assert_eq!(result.manifest.attestations.len(), 2);
}

#[test]
fn a_forged_attestation_arriving_first_does_not_block_the_real_one() {
    let (mut a, _b) = two_up_to_bs_attestation();
    let inbox = a.links()[0].rx.clone();
    let (kind, real) = take_frames(&inbox).pop().unwrap();
    // Both in one read: the forged copy first.
    put_frame(&inbox, kind, &flipped(&real));
    put_frame(&inbox, kind, &real);
    assert_eq!(a.poll().unwrap(), Step::Complete);
}

#[test]
fn a_corrupted_reveal_arriving_first_does_not_block_the_real_one() {
    let mut it = ring(2).into_iter();
    let mut a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut b = RingCeremony::new(two(), 1, &mut Det(2), vec![it.next().unwrap()], 0).unwrap();
    a.start();
    b.start();
    b.poll().unwrap();
    let inbox = a.links()[0].rx.clone();
    for (kind, payload) in take_frames(&inbox) {
        if kind == Frame::Reveal as u8 {
            put_frame(&inbox, kind, &flipped(&payload));
        }
        put_frame(&inbox, kind, &payload);
    }
    for _ in 0..10 {
        let sa = a.poll().expect("a corrupted reveal does not abort");
        let sb = b.poll().unwrap();
        if sa == Step::Complete && sb == Step::Complete {
            return;
        }
    }
    panic!("a did not complete: the corrupted reveal blocked the real one");
}

#[test]
fn an_attest_frame_naming_our_own_seat_is_dropped() {
    use common::{cable, kinds_on};
    let (peer0, mine0) = cable();
    let (_peer1, mine1) = cable();
    let p = CircleParams::new(3, 256, "line", false).unwrap();
    let mut me = RingCeremony::new(p, 1, &mut Det(1), vec![mine0, mine1], 0).unwrap();
    me.start();
    // A peer hands us a frame that claims to be our attestation, before we made one.
    peer0.tx.borrow_mut().clear();
    let bytes = cahoots_core::frame::encode(&cahoots_core::frame::Frame {
        kind: Frame::Attest as u8,
        seq: 0,
        payload: &[1, 0xd2, 0x84, 0x40],
    });
    peer0.tx.borrow_mut().extend(bytes);
    assert_eq!(me.poll().unwrap(), Step::Waiting);
    assert_eq!(me.attestations_held(), 0);
    assert!(
        !kinds_on(&me.links()[1].tx).contains(&(Frame::Attest as u8)),
        "not forwarded"
    );
}

#[test]
fn an_attest_frame_naming_our_own_seat_does_not_reach_the_ceremony() {
    // Were it handed to the ceremony once we attest, it would conflict with ours and abort.
    let mut it = ring(2).into_iter();
    let mut a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut b = RingCeremony::new(two(), 1, &mut Det(2), vec![it.next().unwrap()], 0).unwrap();
    put_frame(&a.links()[0].rx.clone(), Frame::Attest as u8, &[0, 1, 2, 3]);
    a.start();
    b.start();
    for _ in 0..10 {
        let sa = a.poll().expect("our index from a peer is dropped");
        let sb = b.poll().unwrap();
        if sa == Step::Complete && sb == Step::Complete {
            return;
        }
    }
    panic!("did not complete");
}

#[test]
fn an_attest_body_over_256_bytes_is_a_protocol_error() {
    let mut it = ring(2).into_iter();
    let mut a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut payload = vec![1u8];
    payload.extend_from_slice(&[0u8; 257]);
    put_frame(&a.links()[0].rx.clone(), Frame::Attest as u8, &payload);
    a.start();
    assert_eq!(a.poll().unwrap_err(), RingError::Protocol);
}

#[test]
fn an_attest_body_of_256_bytes_is_not_refused_for_its_length() {
    let mut it = ring(2).into_iter();
    let mut a = RingCeremony::new(two(), 0, &mut Det(1), vec![it.next().unwrap()], 0).unwrap();
    let mut payload = vec![1u8];
    payload.extend_from_slice(&[0u8; 256]);
    put_frame(&a.links()[0].rx.clone(), Frame::Attest as u8, &payload);
    a.start();
    assert_eq!(a.poll().unwrap(), Step::Waiting);
}

#[test]
fn a_tier_above_four_is_refused() {
    let l = ring(2).into_iter().next().unwrap();
    let err = match RingCeremony::new(two(), 0, &mut Det(1), vec![l], 5) {
        Err(e) => e,
        Ok(_) => panic!("expected BadTier"),
    };
    assert_eq!(
        err,
        RingError::Ceremony(cahoots_core::ceremony::CeremonyError::BadTier)
    );
}

#[test]
fn a_source_that_never_yields_a_valid_seat_seed_fails_after_eight_tries() {
    /// All zeros: a zero scalar is not a valid seat key. Panics if asked too often.
    struct Zeros(usize);
    impl Entropy for Zeros {
        fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
            self.0 += 1;
            assert!(self.0 < 100, "the seat seed redraw is not bounded");
            buf.fill(0);
            Ok(())
        }
    }
    let l = ring(2).into_iter().next().unwrap();
    let mut z = Zeros(0);
    let err = match RingCeremony::new(two(), 0, &mut z, vec![l], 0) {
        Err(e) => e,
        Ok(_) => panic!("expected a health failure"),
    };
    assert!(matches!(err, RingError::Health(_)), "{err:?}");
    // Contribution, KEM seed, then eight seat seeds.
    assert_eq!(z.0, 2 + 8);
}
