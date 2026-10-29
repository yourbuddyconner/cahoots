//! The ceremony driver over touch links (handoff 3.4, "Transport on devices").
//!
//! Every device has one or more [`Link`]s, one per cable: two in a pile (down and up) or in a
//! closed ring (previous and next), one in a pair. Each device originates its own params,
//! commitment, reveal, and attest frames on every link, and forwards every frame it sees for
//! the first time on every link except the one it arrived on: params and commitments keyed by
//! (kind, member index), reveals and attestations by the hash of their body, with at most
//! four bodies tried per seat and kind (see below). Copies that come back round, or arrive
//! from both sides, are dropped by that dedupe, so the same driver is correct on a line, a
//! ring, or a pair.
//!
//! A device sends its reveal only once it holds all `n` commitments, and its attestation of
//! the manifest only once every reveal is in. Hops deliver bytes in order, so a reveal never
//! overtakes the commitments that precede it; early reveals and attestations are buffered
//! anyway. The ceremony is complete when every seat has signed the manifest.
//!
//! Reveals and attestations are checked by the ceremony, and a member's slot is marked only
//! once its copy is accepted: a corrupted or forged copy that arrives first is dropped and
//! does not block the real one. Until then each distinct body is forwarded once (it is
//! remembered by its SHA-256), and at most four distinct bodies per member and kind, so
//! forwarding stays bounded. A frame that claims our own index is dropped.
//!
//! Frames of the pile layer (kinds 16 and up, see [`crate::pile`]) are ignored.
//!
//! Frame payloads:
//!
//! | kind | payload |
//! |---|---|
//! | `Params` | `index u8 \| CircleParams::encode()` |
//! | `Commit` | `index u8 \| commitment (32 B) \| KEM public key (1216 B) \| seat public key (33 B, SEC1 compressed) \| tier u8` |
//! | `Reveal` | `index u8 \| sealed reveal (see `cahoots_core::reveal`)` |
//! | `Abort`  | `index u8 \| reason u8` |
//! | `Attest` | `index u8 \| detached COSE_Sign1 over the manifest, at most 256 B (see `cahoots_core::manifest`)` |
//!
//! Reveals are sealed to every member, so the wire never carries a contribution in the clear.

use crate::{Entropy, HealthError, Link};
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::ceremony::{Ceremony, CeremonyError, CeremonyResult, Phase};
use cahoots_core::frame::{self, FrameError};
use cahoots_core::params::CircleParams;
use cahoots_core::reveal::{randomness_len, KEM_SEED_LEN, PUBLIC_KEY_LEN};
use cahoots_core::seat::{SeatKey, SeatPublic, SeatSigner};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Frame {
    Params = 1,
    Commit = 2,
    Reveal = 3,
    Abort = 4,
    /// A seat's signature over the manifest.
    Attest = 5,
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
/// A seat public key in a `Commit` payload: SEC1 compressed P-256.
const SEAT_PUBLIC_LEN: usize = 33;
/// `Commit` payload after the index: commitment, KEM key, seat key, tier.
const COMMIT_BODY_LEN: usize = 32 + PUBLIC_KEY_LEN + SEAT_PUBLIC_LEN + 1;
/// Largest `Attest` body. A COSE_Sign1 with a one-byte key id and a 64-byte signature is
/// under 120 bytes.
const MAX_ATTEST_BODY: usize = 256;
/// Distinct reveal or attest bodies forwarded per member before its copy is accepted. One is
/// normal; the rest leave room for corrupted or forged copies without letting them loop.
const MAX_COPIES: usize = 4;
/// Draws of the seat seed before the entropy source is taken to be broken. A uniform draw
/// is a valid P-256 scalar with probability above 1 - 2^-32.
const SEAT_SEED_TRIES: usize = 8;

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
    /// Reveals accepted by the ceremony, ours included.
    seen_reveal: [bool; MAX_MEMBERS],
    pending_reveals: Vec<(u8, Zeroizing<Vec<u8>>)>,
    /// Attestations accepted by the ceremony, ours included.
    seen_attest: [bool; MAX_MEMBERS],
    /// Attestations that arrived before our phase was `Attest`, or not yet applied.
    pending_attestations: Vec<(u8, Vec<u8>)>,
    /// (kind, member, SHA-256 of the body) of each reveal or attest body forwarded for a
    /// member whose copy is not yet accepted.
    copies: Vec<(u8, u8, [u8; 32])>,
    attested_self: bool,
    /// Randomness for sealing our reveal, drawn up front so no entropy source is held.
    reveal_rand: Zeroizing<Vec<u8>>,
    revealed_self: bool,
    aborted: Option<RingError>,
}

impl<L: Link> RingCeremony<L> {
    /// Draw the contribution, the KEM seed, and the seat seed from `entropy` (already
    /// health-tested) and prepare the commitment. `tier` is this device's declared tier, 0
    /// to 4; another is `RingError::Ceremony(CeremonyError::BadTier)`. Nothing is sent until
    /// [`start`](Self::start).
    pub fn new(
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        links: Vec<L>,
        tier: u8,
    ) -> Result<Self, RingError> {
        Self::new_buffered(params, index, entropy, links, Vec::new(), tier)
    }

    /// Like [`new`](Self::new), with bytes that already arrived on each link before the
    /// ceremony took the links over (a neighbour that started first).
    pub fn new_buffered(
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        links: Vec<L>,
        buffered: Vec<Vec<u8>>,
        tier: u8,
    ) -> Result<Self, RingError> {
        Self::build(params, index, entropy, links, buffered, None, tier)
    }

    /// Like [`new_buffered`](Self::new_buffered), for a seat key held outside (a chip): no
    /// seat seed is drawn, and the commitment binds `seat_public`. The manifest is signed only
    /// through [`poll_signing`](Self::poll_signing): poll with the signer for `seat_public`, or
    /// the ceremony waits at its attestation forever.
    pub fn with_seat_public(
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        links: Vec<L>,
        buffered: Vec<Vec<u8>>,
        seat_public: SeatPublic,
        tier: u8,
    ) -> Result<Self, RingError> {
        Self::build(params, index, entropy, links, buffered, Some(seat_public), tier)
    }

    fn build(
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        links: Vec<L>,
        mut buffered: Vec<Vec<u8>>,
        seat_public: Option<SeatPublic>,
        tier: u8,
    ) -> Result<Self, RingError> {
        buffered.resize(links.len(), Vec::new());
        let mut contribution = Zeroizing::new(vec![0u8; params.pad_len as usize]);
        entropy.fill(&mut contribution).map_err(RingError::Health)?;
        let mut kem_seed = Zeroizing::new([0u8; KEM_SEED_LEN]);
        entropy.fill(kem_seed.as_mut()).map_err(RingError::Health)?;
        // The seat seed follows the KEM seed; a seed that is not a valid scalar is redrawn, a
        // few times only: a source that keeps failing is stuck.
        let mut seat_seed = Zeroizing::new([0u8; 32]);
        if seat_public.is_none() {
            let mut valid = false;
            for _ in 0..SEAT_SEED_TRIES {
                entropy
                    .fill(seat_seed.as_mut())
                    .map_err(RingError::Health)?;
                if SeatKey::from_seed(&seat_seed).is_ok() {
                    valid = true;
                    break;
                }
            }
            if !valid {
                return Err(RingError::Health(HealthError::RepetitionCount));
            }
        }
        let mut reveal_rand = Zeroizing::new(vec![0u8; randomness_len(params.n)]);
        entropy.fill(&mut reveal_rand).map_err(RingError::Health)?;
        let n = params.n;
        let params_bytes = params.encode();
        let contribution = core::mem::take(&mut *contribution);
        let ceremony = match seat_public {
            None => Ceremony::new(params, index, contribution, *kem_seed, *seat_seed, tier)?,
            Some(p) => {
                Ceremony::with_seat_public(params, index, contribution, *kem_seed, p, tier)?
            }
        };
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
            seen_attest: [false; MAX_MEMBERS],
            pending_attestations: Vec::new(),
            copies: Vec::new(),
            attested_self: false,
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
    pub fn attestations_held(&self) -> u8 {
        self.seen_attest.iter().filter(|&&s| s).count() as u8
    }

    /// Send on every link that has a peer.
    fn send(&mut self, kind: Frame, index: u8, body: &[u8]) {
        self.send_except(kind as u8, index, body, None);
    }

    /// Send on every link that has a peer, except `except`: the link a frame arrived on.
    fn send_except(&mut self, kind: u8, index: u8, body: &[u8], except: Option<usize>) {
        // Nothing to send to (the top card of a pile forwarding away from its only
        // neighbour): build no frame, which on a badge is a reveal-sized allocation.
        let any = self
            .links
            .iter()
            .enumerate()
            .any(|(k, l)| Some(k) != except && l.peer_present());
        if !any {
            return;
        }
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
        let mut body = Vec::with_capacity(COMMIT_BODY_LEN);
        body.extend_from_slice(&self.ceremony.my_commitment());
        body.extend_from_slice(self.ceremony.my_public_key());
        body.extend_from_slice(&self.ceremony.my_seat_public().0);
        body.push(self.ceremony.tier_of(self.index).expect("own tier is set"));
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
        self.poll_inner(None)
    }

    /// Like [`poll`](Self::poll), for a ceremony built with
    /// [`with_seat_public`](Self::with_seat_public): once every reveal is in, the manifest is
    /// signed with `signer`. A signature that does not verify under the committed seat key
    /// aborts the ceremony with `BadAttestation`. For a ceremony that holds its own key the
    /// signer is never asked.
    pub fn poll_signing(&mut self, signer: &mut dyn SeatSigner) -> Result<Step, RingError> {
        self.poll_inner(Some(signer))
    }

    fn poll_inner(&mut self, signer: Option<&mut dyn SeatSigner>) -> Result<Step, RingError> {
        if let Some(e) = self.aborted {
            return Err(e);
        }
        self.drain_links();
        for k in 0..self.links.len() {
            loop {
                let (kind, used) = match frame::decode(&self.rx[k]) {
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
                        (f.kind, used)
                    }
                    Err(FrameError::Incomplete) => break,
                    Err(FrameError::Crc) => {
                        // Resync: drop one byte and scan for the next frame.
                        self.rx[k].remove(0);
                        continue;
                    }
                };
                // Take the frame out of the receive buffer before handling it, so a 20 KiB
                // reveal is held once, not twice, on a board with a small heap.
                let bytes: Vec<u8> = self.rx[k].drain(..used).collect();
                let (f, _) = frame::decode(&bytes).expect("decoded a moment ago");
                let (index, body) = (f.payload[0], &f.payload[1..]);
                self.handle(kind, index, body, k)?;
            }
            // A receive buffer that grew to hold reveals keeps its capacity otherwise.
            if self.rx[k].capacity() > 4096 && self.rx[k].len() <= 1024 {
                self.rx[k].shrink_to(4096);
            }
        }
        self.maybe_reveal()?;
        self.maybe_attest(signer)?;
        Ok(if self.ceremony.phase() == Phase::Complete {
            Step::Complete
        } else {
            Step::Waiting
        })
    }

    fn handle(&mut self, kind: u8, index: u8, body: &[u8], from: usize) -> Result<(), RingError> {
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
                if body != &self.params_bytes[..] {
                    return Err(self.abort(RingError::ParamsMismatch));
                }
                if !self.seen_params[i] {
                    self.seen_params[i] = true;
                    self.send_except(kind, index, body, Some(from));
                }
            }
            k if k == Frame::Commit as u8 => {
                if body.len() != COMMIT_BODY_LEN {
                    return Err(self.abort(RingError::Protocol));
                }
                let c: [u8; 32] = body[..32].try_into().expect("32 bytes");
                let seat_at = 32 + PUBLIC_KEY_LEN;
                let (kem_pk, seat_pk, tier) = (
                    &body[32..seat_at],
                    &body[seat_at..seat_at + SEAT_PUBLIC_LEN],
                    body[COMMIT_BODY_LEN - 1],
                );
                if let Err(e) = self
                    .ceremony
                    .add_commitment(index, c, kem_pk, seat_pk, tier)
                {
                    return Err(self.abort(RingError::Ceremony(e)));
                }
                if !self.seen_commit[i] {
                    self.seen_commit[i] = true;
                    self.send_except(kind, index, body, Some(from));
                }
            }
            k if k == Frame::Reveal as u8 => {
                // Ours is marked when we send it; a copy from a peer is never needed.
                if index != self.index
                    && !self.seen_reveal[i]
                    && self.first_sight(kind, index, body, from)
                {
                    self.pending_reveals.push((index, Zeroizing::new(body.to_vec())));
                }
            }
            k if k == Frame::Attest as u8 => {
                if body.len() > MAX_ATTEST_BODY {
                    return Err(self.abort(RingError::Protocol));
                }
                if index != self.index
                    && !self.seen_attest[i]
                    && self.first_sight(kind, index, body, from)
                {
                    self.pending_attestations.push((index, body.to_vec()));
                }
            }
            k if k == Frame::Abort as u8 => {
                return Err(self.abort(RingError::PeerAborted(index)));
            }
            _ => return Err(self.abort(RingError::Protocol)),
        }
        Ok(())
    }

    /// Forward a reveal or attest body for `index` the first time we see it. False, and
    /// nothing sent, when this body came by before (a copy coming back round, or from the
    /// other side) or `index` already used up its [`MAX_COPIES`].
    fn first_sight(&mut self, kind: u8, index: u8, body: &[u8], from: usize) -> bool {
        let digest: [u8; 32] = Sha256::digest(body).into();
        let mut held = 0;
        for &(k, i, d) in &self.copies {
            if (k, i) == (kind, index) {
                if d == digest {
                    return false;
                }
                held += 1;
            }
        }
        if held >= MAX_COPIES {
            return false;
        }
        self.copies.push((kind, index, digest));
        self.send_except(kind, index, body, Some(from));
        true
    }

    /// `index`'s copy of `kind` was accepted: later copies are dropped by the seen flag.
    fn forget_copies(&mut self, kind: Frame, index: u8) {
        self.copies
            .retain(|&(k, i, _)| (k, i) != (kind as u8, index));
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
            if self.seen_reveal[usize::from(idx)] {
                continue;
            }
            match self.ceremony.add_reveal(idx, &r) {
                Ok(()) => {
                    self.seen_reveal[usize::from(idx)] = true;
                    self.forget_copies(Frame::Reveal, idx);
                }
                // A garbled frame on a wire is a CRC failure before it gets here; a sealed
                // reveal that still fails to open is not addressed to us. Drop it and keep going.
                Err(CeremonyError::Corrupted(_)) => {}
                Err(e) => return Err(self.abort(RingError::Ceremony(e))),
            }
        }
        Ok(())
    }

    /// Once every reveal is in, send our attestation of the manifest and apply any buffered
    /// attestations.
    fn maybe_attest(&mut self, signer: Option<&mut dyn SeatSigner>) -> Result<(), RingError> {
        if !matches!(self.ceremony.phase(), Phase::Attest | Phase::Complete) {
            return Ok(());
        }
        if !self.attested_self {
            if self.ceremony.my_attestation().is_none() {
                // The key is held outside: sign now, if the caller passed the signer.
                if let Some(signer) = signer {
                    if let Err(e) = self.ceremony.sign_manifest(signer) {
                        return Err(self.abort(RingError::Ceremony(e)));
                    }
                }
            }
            // Without a signature yet, wait; others' attestations are applied below.
            if let Some(a) = self.ceremony.my_attestation() {
                self.attested_self = true;
                self.seen_attest[usize::from(self.index)] = true;
                self.send(Frame::Attest, self.index, &a);
            }
        }
        let pending = core::mem::take(&mut self.pending_attestations);
        for (idx, a) in pending {
            if self.seen_attest[usize::from(idx)] {
                continue;
            }
            match self.ceremony.add_attestation(idx, &a) {
                Ok(()) => {
                    self.seen_attest[usize::from(idx)] = true;
                    self.forget_copies(Frame::Attest, idx);
                }
                // Like a reveal that fails to open: drop it and keep going.
                Err(CeremonyError::BadAttestation(_)) => {}
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
        let mut last = (0u8, 0u8, 0u8);
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
            let now = (
                self.commitments_held(),
                self.reveals_held(),
                self.attestations_held(),
            );
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
                s.push("  attestations ");
                s.push_num(now.2);
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
    buf: [u8; 64],
    len: usize,
}

impl StatusLine {
    fn new() -> Self {
        StatusLine {
            buf: [0; 64],
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
