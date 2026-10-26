//! The ceremony driver over touch links (handoff 3.4, "Transport on devices").
//!
//! Every device has one or more [`Link`]s, one per cable: two in a pile (down and up) or in a
//! closed ring (previous and next), one in a pair. Each device originates its own params,
//! commitment, and reveal frames on every link, and forwards every frame it sees for the first
//! time on every link except the one it arrived on, keyed by (kind, member index). Copies that
//! come back round, or arrive from both sides, are dropped by that dedupe, so the same driver
//! is correct on a line, a ring, or a pair. A device sends its reveal only once it holds all
//! `n` commitments. Hops deliver bytes in order, so a reveal never overtakes the commitments
//! that precede it; early reveals are buffered anyway. Frames of the pile layer (kinds 16 and
//! up, see [`crate::pile`]) are ignored.
//!
//! Frame payloads:
//!
//! | kind | payload |
//! |---|---|
//! | `Params` | `index u8 \| CircleParams::encode()` |
//! | `Commit` | `index u8 \| commitment (32 B) \| KEM public key (1216 B)` |
//! | `Reveal` | `index u8 \| sealed reveal (see `cahoots_core::reveal`)` |
//!
//! Reveals are sealed to every member, so the wire never carries a contribution in the clear.
//! | `Abort`  | `index u8 \| reason u8` |

use crate::{Entropy, HealthError, Link};
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::ceremony::{Ceremony, CeremonyError, CeremonyResult, Phase};
use cahoots_core::frame::{self, FrameError};
use cahoots_core::params::CircleParams;
use cahoots_core::reveal::{randomness_len, KEM_SEED_LEN, PUBLIC_KEY_LEN};
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
    links: Vec<L>,
    ceremony: Ceremony,
    params_bytes: Vec<u8>,
    n: u8,
    index: u8,
    seq: u32,
    /// Bytes received and not yet decoded, per link.
    rx: Vec<Vec<u8>>,
    seen_params: [bool; MAX_MEMBERS],
    seen_commit: [bool; MAX_MEMBERS],
    seen_reveal: [bool; MAX_MEMBERS],
    pending_reveals: Vec<(u8, Zeroizing<Vec<u8>>)>,
    /// Randomness for sealing our reveal, drawn up front so no entropy source is held.
    reveal_rand: Zeroizing<Vec<u8>>,
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
        links: Vec<L>,
    ) -> Result<Self, RingError> {
        Self::new_buffered(params, index, entropy, links, Vec::new())
    }

    /// Like [`new`](Self::new), with bytes that already arrived on each link before the
    /// ceremony took the links over (a neighbour that started first).
    pub fn new_buffered(
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        links: Vec<L>,
        mut buffered: Vec<Vec<u8>>,
    ) -> Result<Self, RingError> {
        buffered.resize(links.len(), Vec::new());
        let mut contribution = Zeroizing::new(vec![0u8; params.pad_len as usize]);
        entropy.fill(&mut contribution).map_err(RingError::Health)?;
        let mut kem_seed = Zeroizing::new([0u8; KEM_SEED_LEN]);
        entropy.fill(kem_seed.as_mut()).map_err(RingError::Health)?;
        let mut reveal_rand = Zeroizing::new(vec![0u8; randomness_len(params.n)]);
        entropy.fill(&mut reveal_rand).map_err(RingError::Health)?;
        let n = params.n;
        let params_bytes = params.encode();
        let ceremony = Ceremony::new(
            params,
            index,
            core::mem::take(&mut *contribution),
            *kem_seed,
        )?;
        Ok(RingCeremony {
            links,
            ceremony,
            params_bytes,
            n,
            index,
            seq: 0,
            rx: buffered,
            seen_params: [false; MAX_MEMBERS],
            seen_commit: [false; MAX_MEMBERS],
            seen_reveal: [false; MAX_MEMBERS],
            pending_reveals: Vec::new(),
            reveal_rand,
            revealed_self: false,
            aborted: None,
        })
    }

    pub fn links(&self) -> &[L] {
        &self.links
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

    /// Send on every link that has a peer.
    fn send(&mut self, kind: Frame, index: u8, body: &[u8]) {
        self.send_except(kind as u8, index, body, None);
    }

    /// Send on every link that has a peer, except `except`: the link a frame arrived on.
    fn send_except(&mut self, kind: u8, index: u8, body: &[u8], except: Option<usize>) {
        let mut payload = Vec::with_capacity(1 + body.len());
        payload.push(index);
        payload.extend_from_slice(body);
        let f = frame::Frame {
            kind,
            seq: self.seq,
            payload: &payload,
        };
        self.seq = self.seq.wrapping_add(1);
        let bytes = frame::encode(&f);
        for (k, link) in self.links.iter_mut().enumerate() {
            if Some(k) == except || !link.peer_present() {
                continue;
            }
            link.send(&bytes);
        }
    }

    /// Send our parameters and commitment.
    pub fn start(&mut self) {
        let me = usize::from(self.index);
        self.seen_params[me] = true;
        self.seen_commit[me] = true;
        let pb = self.params_bytes.clone();
        self.send(Frame::Params, self.index, &pb);
        let mut body = Vec::with_capacity(32 + PUBLIC_KEY_LEN);
        body.extend_from_slice(&self.ceremony.my_commitment());
        body.extend_from_slice(self.ceremony.my_public_key());
        self.send(Frame::Commit, self.index, &body);
    }

    fn abort(&mut self, e: RingError) -> RingError {
        if self.aborted.is_none() {
            self.aborted = Some(e);
            self.send(Frame::Abort, self.index, &[0]);
        }
        e
    }

    fn drain_links(&mut self) {
        let mut buf = [0u8; 256];
        for (k, link) in self.links.iter_mut().enumerate() {
            loop {
                let n = link.recv(&mut buf);
                if n == 0 {
                    break;
                }
                self.rx[k].extend_from_slice(&buf[..n]);
            }
        }
    }

    /// Pull bytes from the link, process complete frames, and advance. Call repeatedly.
    pub fn poll(&mut self) -> Result<Step, RingError> {
        if let Some(e) = self.aborted {
            return Err(e);
        }
        self.drain_links();
        for k in 0..self.links.len() {
            loop {
                let (kind, index, body, used) = match frame::decode(&self.rx[k]) {
                    Ok((f, used)) => {
                        if f.kind >= crate::pile::KIND_FIRST {
                            // The pile layer, or a laptop's keep-alive ping, talking during
                            // the ceremony. Not ours; a ping has no payload at all.
                            self.rx[k].drain(..used);
                            continue;
                        }
                        if f.payload.is_empty() {
                            return Err(self.abort(RingError::Protocol));
                        }
                        (f.kind, f.payload[0], f.payload[1..].to_vec(), used)
                    }
                    Err(FrameError::Incomplete) => break,
                    Err(FrameError::Crc) => {
                        // Resync: drop one byte and scan for the next frame.
                        self.rx[k].remove(0);
                        continue;
                    }
                };
                self.rx[k].drain(..used);
                self.handle(kind, index, body, k)?;
            }
        }
        self.maybe_reveal()?;
        Ok(if self.ceremony.phase() == Phase::Complete {
            Step::Complete
        } else {
            Step::Waiting
        })
    }

    fn handle(&mut self, kind: u8, index: u8, body: Vec<u8>, from: usize) -> Result<(), RingError> {
        if kind >= crate::pile::KIND_FIRST {
            // The pile layer talking after the ceremony began. Not ours.
            return Ok(());
        }
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
                    self.send_except(kind, index, &body, Some(from));
                }
            }
            k if k == Frame::Commit as u8 => {
                if body.len() != 32 + PUBLIC_KEY_LEN {
                    return Err(self.abort(RingError::Protocol));
                }
                let c: [u8; 32] = body[..32].try_into().expect("32 bytes");
                if let Err(e) = self.ceremony.add_commitment(index, c, &body[32..]) {
                    return Err(self.abort(RingError::Ceremony(e)));
                }
                if !self.seen_commit[i] {
                    self.seen_commit[i] = true;
                    self.send_except(kind, index, &body, Some(from));
                }
            }
            k if k == Frame::Reveal as u8 => {
                if !self.seen_reveal[i] {
                    self.seen_reveal[i] = true;
                    self.send_except(kind, index, &body, Some(from));
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
            let rand = core::mem::take(&mut self.reveal_rand);
            let mut pos = 0usize;
            let mut fill = |buf: &mut [u8]| {
                buf.copy_from_slice(&rand[pos..pos + buf.len()]);
                pos += buf.len();
            };
            let r = self
                .ceremony
                .my_reveal(&mut fill)
                .expect("in Reveal phase")
                .to_vec();
            self.send(Frame::Reveal, self.index, &r);
        }
        let pending = core::mem::take(&mut self.pending_reveals);
        for (idx, r) in pending {
            match self.ceremony.add_reveal(idx, &r) {
                Ok(()) => {}
                // A garbled frame on a wire is a CRC failure before it gets here; a sealed
                // reveal that still fails to open is not addressed to us. Drop it and keep going.
                Err(CeremonyError::Corrupted(_)) => {}
                Err(e) => return Err(self.abort(RingError::Ceremony(e))),
            }
        }
        Ok(())
    }

    /// Erase contributions and hand back the pad and the links.
    pub fn finish(self) -> Result<(CeremonyResult, Vec<L>), RingError> {
        let r = self.ceremony.finish()?;
        Ok((r, self.links))
    }

    /// Drive the ceremony to completion, reporting progress lines through `status` and
    /// calling `idle` between polls (sleep, yield, or feed a watchdog). The caller shows
    /// the check words from the result.
    pub fn run(
        mut self,
        mut status: impl FnMut(&str),
        mut idle: impl FnMut(),
    ) -> Result<(CeremonyResult, Vec<L>), RingError> {
        let mut last = (0u8, 0u8);
        status("ceremony: waiting for the others");
        self.start();
        loop {
            let step = match self.poll() {
                Ok(s) => s,
                Err(e) => {
                    status("ceremony: ABORTED");
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
                status(s.as_str());
            }
            if step == Step::Complete {
                break;
            }
            idle();
        }
        // Our neighbours may still need the frames we forwarded last. Keep reading while
        // we flush, so a neighbour that is also flushing toward us is never blocked.
        loop {
            let mut done = true;
            for link in self.links.iter_mut() {
                if !link.flush() {
                    done = false;
                }
            }
            if done {
                break;
            }
            self.drain_links();
            idle();
        }
        self.finish()
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
