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
    let mut a = RingCeremony::new(p.clone(), 0, &mut Det(1), vec![mine]).unwrap();
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
    let mut a = RingCeremony::new(p0, 0, &mut Det(1), vec![it.next().unwrap()]).unwrap();
    let mut b = RingCeremony::new(p1, 1, &mut Det(2), vec![it.next().unwrap()]).unwrap();
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
    let err = match RingCeremony::new(p, 0, &mut Bad, vec![l]) {
        Err(e) => e,
        Ok(_) => panic!("expected health failure"),
    };
    assert_eq!(err, RingError::Health(HealthError::AdaptiveProportion));
}

#[test]
fn run_flushes_the_link_before_finishing() {
    use cahoots_hal::Display;
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
            // Like a real driver, move a little toward the wire on every call.
            let n = self.queued.len().min(100);
            let chunk: Vec<u8> = self.queued.drain(..n).collect();
            self.inner.send(&chunk);
            self.inner.recv(buf)
        }
        fn peer_present(&self) -> bool {
            true
        }
        fn flush(&mut self) -> bool {
            self.flushes += 1;
            // Hand over 100 bytes per call to force several rounds.
            let n = self.queued.len().min(100);
            let chunk: Vec<u8> = self.queued.drain(..n).collect();
            self.inner.send(&chunk);
            self.queued.is_empty()
        }
    }
    struct NoDisplay;
    impl Display for NoDisplay {
        fn show_words(&mut self, _: &[&str; 6]) {}
        fn show_qr(&mut self, _: &[u8]) {}
        fn show_status(&mut self, _: &str) {}
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
    let a = RingCeremony::new(p.clone(), 0, &mut Det(1), vec![lazy]).unwrap();
    let mut b = RingCeremony::new(p, 1, &mut Det(2), vec![other]).unwrap();
    // Drive b from a helper thread is not possible with Rc; interleave by hand instead:
    // a.run needs b to answer, so pump b inside a's idle callback.
    b.start();
    let (_result, links) = a
        .run(&mut NoDisplay, || {
            let _ = b.poll();
        })
        .unwrap();
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
            RingCeremony::new(params(), i as u8, &mut common::Det(3000 + i as u64), links).unwrap()
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
    let mut me = RingCeremony::new(p.clone(), 1, &mut Det(1), vec![mine0, mine1]).unwrap();
    me.start();
    // Neighbour 0 sends its params and commitment down link 0.
    let mut n0 = RingCeremony::new(p, 0, &mut Det(2), vec![peer0]).unwrap();
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
