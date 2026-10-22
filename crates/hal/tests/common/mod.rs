//! Shared test gear: deterministic entropy, in-process links with a presence flag, lockstep driving.
#![allow(dead_code)]

use cahoots_hal::ring::{RingCeremony, Step};
use cahoots_hal::{Entropy, HealthError, Link};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

pub type Queue = Rc<RefCell<VecDeque<u8>>>;

pub struct Det(pub u64);
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

pub fn contribution(seed: u64, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    Det(seed).fill(&mut v).unwrap();
    v
}

/// One end of a cable. `tx` is read by the other end, `rx` is written by it. `present` is the
/// detect line: true while something is plugged into this end.
pub struct End {
    pub tx: Queue,
    pub rx: Queue,
    pub present: Rc<Cell<bool>>,
}
impl Link for End {
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
        self.present.get()
    }
}

/// A cable: two ends that talk to each other and share one detect line.
pub fn cable() -> (End, End) {
    let a = Rc::new(RefCell::new(VecDeque::new()));
    let b = Rc::new(RefCell::new(VecDeque::new()));
    let present = Rc::new(Cell::new(true));
    (
        End {
            tx: a.clone(),
            rx: b.clone(),
            present: present.clone(),
        },
        End {
            tx: b,
            rx: a,
            present,
        },
    )
}

/// A connector with nothing plugged in.
pub fn open_end() -> End {
    End {
        tx: Rc::new(RefCell::new(VecDeque::new())),
        rx: Rc::new(RefCell::new(VecDeque::new())),
        present: Rc::new(Cell::new(false)),
    }
}

/// Frame kinds on an end's tx queue, in order.
pub fn kinds_on(q: &Queue) -> Vec<u8> {
    let bytes: Vec<u8> = q.borrow().iter().copied().collect();
    let mut off = 0;
    let mut kinds = vec![];
    while off < bytes.len() {
        let (f, used) = cahoots_core::frame::decode(&bytes[off..]).unwrap();
        kinds.push(f.kind);
        off += used;
    }
    kinds
}

/// Poll every device in turn until all report Complete. Panics if it takes too long.
pub fn drive<L: Link>(devs: &mut [RingCeremony<L>]) {
    for d in devs.iter_mut() {
        d.start();
    }
    let mut rounds = 0;
    loop {
        let mut all_done = true;
        for d in devs.iter_mut() {
            if d.poll().unwrap() != Step::Complete {
                all_done = false;
            }
        }
        if all_done {
            break;
        }
        rounds += 1;
        assert!(rounds < 2000, "did not converge");
    }
}
