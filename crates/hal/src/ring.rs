//! The ring ceremony driver (handoff 3.4, "Transport on devices").
//!
//! Every device has one [`Link`]: it transmits to the next device and receives from the
//! previous one. Each device originates its own params, commitment, and reveal frames and
//! forwards every frame it sees for the first time, keyed by (kind, member index). Frames
//! that come all the way round are dropped by that dedupe. A device sends its reveal only
//! once it holds all `n` commitments. Hops deliver bytes in order, so a reveal never
//! overtakes the commitments that precede it; early reveals are buffered anyway.
//!
//! Frame payloads:
//!
//! | kind | payload |
//! |---|---|
//! | `Params` | `index u8 \| CircleParams::encode()` |
//! | `Commit` | `index u8 \| commitment (32 B)` |
//! | `Reveal` | `index u8 \| R_index (L bytes)` |
//! | `Abort`  | `index u8 \| reason u8` |

use crate::{Display, Entropy, HealthError, Link};
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::ceremony::{Ceremony, CeremonyError, CeremonyResult, Phase};
use cahoots_core::frame::{self, FrameError};
use cahoots_core::params::CircleParams;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Frame {
    Params = 1,
    Commit = 2,
    Reveal = 3,
    Abort = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingError {
    Health(HealthError),
    Ceremony(CeremonyError),
    /// A peer's parameters differ from ours.
    ParamsMismatch,
    /// A frame had an impossible index or length.
    Protocol,
    /// A peer aborted; the payload carries its index.
    PeerAborted(u8),
}

impl From<CeremonyError> for RingError {
    fn from(e: CeremonyError) -> Self {
        RingError::Ceremony(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Waiting,
    Complete,
}

const MAX_MEMBERS: usize = 8;

pub struct RingCeremony<L: Link> {
    link: L,
    ceremony: Ceremony,
    params_bytes: Vec<u8>,
    n: u8,
    index: u8,
    seq: u32,
    rx: Vec<u8>,
    seen_params: [bool; MAX_MEMBERS],
    seen_commit: [bool; MAX_MEMBERS],
    seen_reveal: [bool; MAX_MEMBERS],
    pending_reveals: Vec<(u8, Zeroizing<Vec<u8>>)>,
    revealed_self: bool,
    aborted: Option<RingError>,
}

impl<L: Link> RingCeremony<L> {
    /// Draw the contribution from `entropy` (already health-tested) and prepare the
    /// commitment. Nothing is sent until [`start`](Self::start).
    pub fn new(
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        link: L,
    ) -> Result<Self, RingError> {
        let mut contribution = Zeroizing::new(vec![0u8; params.pad_len as usize]);
        entropy.fill(&mut contribution).map_err(RingError::Health)?;
        let n = params.n;
        let params_bytes = params.encode();
        let ceremony = Ceremony::new(params, index, core::mem::take(&mut *contribution))?;
        Ok(RingCeremony {
            link,
            ceremony,
            params_bytes,
            n,
            index,
            seq: 0,
            rx: Vec::new(),
            seen_params: [false; MAX_MEMBERS],
            seen_commit: [false; MAX_MEMBERS],
            seen_reveal: [false; MAX_MEMBERS],
            pending_reveals: Vec::new(),
            revealed_self: false,
            aborted: None,
        })
    }

    pub fn link(&self) -> &L {
        &self.link
    }
    pub fn phase(&self) -> Phase {
        self.ceremony.phase()
    }
    pub fn commitments_held(&self) -> u8 {
        self.seen_commit.iter().filter(|&&s| s).count() as u8
    }
    pub fn reveals_held(&self) -> u8 {
        self.seen_reveal.iter().filter(|&&s| s).count() as u8
    }

    fn send(&mut self, kind: Frame, index: u8, body: &[u8]) {
        let mut payload = Vec::with_capacity(1 + body.len());
        payload.push(index);
        payload.extend_from_slice(body);
        let f = frame::Frame {
            kind: kind as u8,
            seq: self.seq,
            payload: &payload,
        };
        self.seq = self.seq.wrapping_add(1);
        let bytes = frame::encode(&f);
        self.link.send(&bytes);
    }

    /// Send our parameters and commitment.
    pub fn start(&mut self) {
        let me = usize::from(self.index);
        self.seen_params[me] = true;
        self.seen_commit[me] = true;
        let pb = self.params_bytes.clone();
        self.send(Frame::Params, self.index, &pb);
        let c = self.ceremony.my_commitment();
        self.send(Frame::Commit, self.index, &c);
    }

    fn abort(&mut self, e: RingError) -> RingError {
        if self.aborted.is_none() {
            self.aborted = Some(e);
            self.send(Frame::Abort, self.index, &[0]);
        }
        e
    }

    fn drain_link(&mut self) {
        let mut buf = [0u8; 256];
        loop {
            let n = self.link.recv(&mut buf);
            if n == 0 {
                break;
            }
            self.rx.extend_from_slice(&buf[..n]);
        }
    }

    /// Pull bytes from the link, process complete frames, and advance. Call repeatedly.
    pub fn poll(&mut self) -> Result<Step, RingError> {
        if let Some(e) = self.aborted {
            return Err(e);
        }
        self.drain_link();
        loop {
            let (kind, index, body, used) = match frame::decode(&self.rx) {
                Ok((f, used)) => {
                    if f.payload.is_empty() {
                        return Err(self.abort(RingError::Protocol));
                    }
                    (f.kind, f.payload[0], f.payload[1..].to_vec(), used)
                }
                Err(FrameError::Incomplete) => break,
                Err(FrameError::Crc) => {
                    // Resync: drop one byte and scan for the next frame.
                    self.rx.remove(0);
                    continue;
                }
            };
            self.rx.drain(..used);
            self.handle(kind, index, body)?;
        }
        self.maybe_reveal()?;
        Ok(if self.ceremony.phase() == Phase::Complete {
            Step::Complete
        } else {
            Step::Waiting
        })
    }

    fn handle(&mut self, kind: u8, index: u8, body: Vec<u8>) -> Result<(), RingError> {
        if index >= self.n {
            return Err(self.abort(RingError::Protocol));
        }
        let i = usize::from(index);
        match kind {
            k if k == Frame::Params as u8 => {
                if body != self.params_bytes {
                    return Err(self.abort(RingError::ParamsMismatch));
                }
                if !self.seen_params[i] {
                    self.seen_params[i] = true;
                    self.send(Frame::Params, index, &body);
                }
            }
            k if k == Frame::Commit as u8 => {
                let c: [u8; 32] = body
                    .as_slice()
                    .try_into()
                    .map_err(|_| RingError::Protocol)?;
                if let Err(e) = self.ceremony.add_commitment(index, c) {
                    return Err(self.abort(RingError::Ceremony(e)));
                }
                if !self.seen_commit[i] {
                    self.seen_commit[i] = true;
                    self.send(Frame::Commit, index, &body);
                }
            }
            k if k == Frame::Reveal as u8 => {
                if !self.seen_reveal[i] {
                    self.seen_reveal[i] = true;
                    self.send(Frame::Reveal, index, &body);
                    self.pending_reveals.push((index, Zeroizing::new(body)));
                }
            }
            k if k == Frame::Abort as u8 => {
                return Err(self.abort(RingError::PeerAborted(index)));
            }
            _ => return Err(self.abort(RingError::Protocol)),
        }
        Ok(())
    }

    /// Once all commitments are held, reveal ourselves and apply any buffered reveals.
    fn maybe_reveal(&mut self) -> Result<(), RingError> {
        if self.ceremony.phase() != Phase::Reveal {
            return Ok(());
        }
        if !self.revealed_self {
            self.revealed_self = true;
            self.seen_reveal[usize::from(self.index)] = true;
            let r = self.ceremony.my_reveal().expect("in Reveal phase").to_vec();
            self.send(Frame::Reveal, self.index, &r);
            self.ceremony.add_reveal(self.index, &r)?;
        }
        let pending = core::mem::take(&mut self.pending_reveals);
        for (idx, r) in pending {
            if let Err(e) = self.ceremony.add_reveal(idx, &r) {
                return Err(self.abort(RingError::Ceremony(e)));
            }
        }
        Ok(())
    }

    /// Erase contributions and hand back the pad and the link.
    pub fn finish(self) -> Result<(CeremonyResult, L), RingError> {
        let r = self.ceremony.finish()?;
        Ok((r, self.link))
    }

    /// Drive the ceremony to completion, reporting progress on `display` and calling
    /// `idle` between polls (sleep, yield, or feed a watchdog).
    pub fn run<D: Display>(
        mut self,
        display: &mut D,
        mut idle: impl FnMut(),
    ) -> Result<(CeremonyResult, L), RingError> {
        let mut last = (0u8, 0u8);
        display.show_status("ceremony: waiting for ring");
        self.start();
        loop {
            let step = match self.poll() {
                Ok(s) => s,
                Err(e) => {
                    display.show_status("ceremony: ABORTED");
                    return Err(e);
                }
            };
            let now = (self.commitments_held(), self.reveals_held());
            if now != last {
                last = now;
                let mut s = StatusLine::new();
                s.push("commits ");
                s.push_num(now.0);
                s.push("/");
                s.push_num(self.n);
                s.push("  reveals ");
                s.push_num(now.1);
                s.push("/");
                s.push_num(self.n);
                display.show_status(s.as_str());
            }
            if step == Step::Complete {
                break;
            }
            idle();
        }
        // Our neighbours may still need the frames we forwarded last. Keep reading while
        // we flush, so a neighbour that is also flushing toward us is never blocked.
        while !self.link.flush() {
            self.drain_link();
            idle();
        }
        let (result, link) = self.finish()?;
        display.show_words(&result.check_words);
        Ok((result, link))
    }
}

/// Tiny fixed-capacity string builder so status lines need no formatting machinery.
struct StatusLine {
    buf: [u8; 48],
    len: usize,
}

impl StatusLine {
    fn new() -> Self {
        StatusLine {
            buf: [0; 48],
            len: 0,
        }
    }
    fn push(&mut self, s: &str) {
        for &b in s.as_bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
    }
    fn push_num(&mut self, n: u8) {
        if n >= 10 {
            self.push_byte(b'0' + n / 10);
        }
        self.push_byte(b'0' + n % 10);
    }
    fn push_byte(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}
