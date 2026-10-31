//! The bridge: a laptop at the bottom of a pile that carries frames between the pile and a
//! ceremony a phone hosts through the relay.
//!
//! The bridge holds no seat, is not counted, makes no key and never opens a reveal. It names
//! the circle to the pile with the host's parameters (a bridged `Seat` saying it holds seat
//! 0, so the first card is seat 1) and starts the count with the host's size. Each card's
//! ring frame that comes down the cable goes to the relay as that seat's channel message,
//! and each channel message from a seat that is not a card goes up the cable as a ring
//! frame. Each card's commitment makes a join code for the host's phone to scan, so the
//! host takes the cards as it takes a phone that joins.
//!
//! The bridge checks what the cards cannot: seat 0's commitment on the relay must be the one
//! the offer names, every card must propose the offer's parameters, and the pile must have
//! the offer's number of cards. A seat 0 commitment that is not the host's is a stray, as it
//! is for a joiner: it is dropped, and the bridge waits for the real one. A ring frame from a
//! seat above the cards means the pile is bigger (nothing the bridge sends up comes back
//! down), and no parameters from the top card's seat within [`PILE_DEADLINE`] means it is
//! smaller. An abort from a card ends the run.
//!
//! From the relay, only the first commitment for a seat goes up the cable. Every new reveal
//! and signature goes up, so one forged copy cannot keep the real one from the cards: the
//! ring keeps a few copies per seat and drops the ones that do not open or check.
//!
//! The bridge cannot check a signature, so it never stops on one. Once everyone has signed
//! it says so and keeps carrying until the pile is unplugged.
//!
//! [`Bridge::run`] keeps the relay off the cable's loop. One thread posts, one long-polls, and
//! the loop that reads the cable never waits on either, so a board that takes two seconds
//! without bytes for an unplugged laptop never sees a slow relay that way.

use cahoots_core::channel::{commitment_hash, join_code, parts, Mailbox, Offer, Round};
use cahoots_core::frame::{self, FrameError};
use cahoots_hal::pile::{bridged_seat_frame, count_frame, KIND_FIRST, KIND_HELLO};
use cahoots_hal::ring::Frame;
use cahoots_hal::Link;
use cahoots_relay::{Relay, RelayError};
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The bridge's mailbox seat. No real seat has it, so the mailbox hands up every seat's
/// messages.
const NO_SEAT: u8 = u8::MAX;
/// Relay failures in a row that [`Bridge::run`] waits out before it gives up.
const RELAY_RETRIES: u32 = 30;
/// How long the relay holds a read open when it has nothing new.
pub const POLL_WAIT_MS: u64 = 2000;
/// The least time between two reads that came back empty, for a relay that does not wait.
const EMPTY_POLL_EVERY: Duration = Duration::from_millis(500);
/// Between passes of the cable's loop.
const LINK_EVERY: Duration = Duration::from_millis(20);
/// How long after the pile starts the top card's parameters may take to come down. A pile
/// with fewer cards than the offer never sends them.
pub const PILE_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub enum BridgeError {
    /// The pile has at least `pile` cards, or no more than `pile`, and the host's offer counts
    /// `offer`.
    Count { pile: u8, offer: u8 },
    /// The card in `seat` stopped the ceremony.
    Aborted { seat: u8, reason: u8 },
    /// The card in `seat` proposed parameters that differ from the host's.
    Params { seat: u8 },
    /// The cable to the pile was unplugged.
    Link,
    Relay(RelayError),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Count { pile, offer } => write!(
                f,
                "the pile has at least {pile} cards and the host said {offer}. Restack and start again."
            ),
            BridgeError::Aborted { seat, reason } => write!(
                f,
                "card {} left the ceremony (reason {reason}).",
                u16::from(*seat) + 1
            ),
            BridgeError::Params { seat } => write!(
                f,
                "the card in seat {} has a different circle from the host. Stop and start again",
                u16::from(*seat) + 1
            ),
            BridgeError::Link => write!(f, "the pile was unplugged"),
            BridgeError::Relay(e) => write!(f, "relay: {e:?}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<RelayError> for BridgeError {
    fn from(e: RelayError) -> Self {
        BridgeError::Relay(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeStep {
    Working,
    /// Every card's signature is queued for the relay, and at least one signature for every
    /// other seat has gone down the cable. The bridge cannot check a signature, so a later
    /// copy may still be the real one: [`Bridge::run`] keeps carrying until the pile is gone.
    Done,
}

/// The cable's side of the bridge. It makes no relay calls: [`deliver`](Self::deliver)
/// hands it what the relay read, and [`take_posts`](Self::take_posts) what to post.
pub struct Bridge {
    offer: Offer,
    params_bytes: Vec<u8>,
    mailbox: Mailbox,
    /// Relay entries read and not yet handled. They wait until the pile has started.
    inbox: VecDeque<Vec<u8>>,
    /// Station messages to post, in order.
    outbox: Vec<Vec<u8>>,
    /// Bytes from the cable not yet decoded.
    rx: Vec<u8>,
    seq: u32,
    was_present: bool,
    /// A card's ring frame came down: the pile is in the ceremony. Until then nothing but
    /// pile frames goes up, so a `Seat` sent again for a late `Hello` is never stuck behind
    /// a ring frame the card's pile layer will not read past.
    started: bool,
    /// When the pile started, to give the top card [`PILE_DEADLINE`].
    started_at: Option<Instant>,
    /// How long the top card's parameters may take: [`PILE_DEADLINE`], or shorter in tests.
    pile_deadline: Duration,
    /// The top card's parameters came down: the pile is as tall as the offer says.
    pile_whole: bool,
    /// The highest card seat a frame came down from.
    highest_card: u8,
    /// (kind, index, SHA-256 of the body) of each card frame taken from the cable.
    seen: HashSet<(u8, u8, [u8; 32])>,
    /// (round, seat, SHA-256 of the payload) of each relay reveal and signature sent up.
    sent_up: HashSet<(u8, u8, [u8; 32])>,
    /// Seat 0 commitments on the relay that are not the host's, already reported.
    strays: HashSet<[u8; 32]>,
    /// Per seat: its signature has gone where it was going (a card's to the outbox, a
    /// phone's up the cable).
    attested: Vec<bool>,
}

impl Bridge {
    /// A bridge for the host's `offer`, under a pile of `badges_in_pile` cards. The pile must
    /// have the number of cards the offer was made for.
    pub fn new(offer: Offer, badges_in_pile: u8) -> Result<Bridge, BridgeError> {
        if badges_in_pile != offer.badges {
            return Err(BridgeError::Count {
                pile: badges_in_pile,
                offer: offer.badges,
            });
        }
        let params_bytes = offer.params.encode();
        let mailbox = Mailbox::new(offer.channel.clone(), NO_SEAT);
        let n = usize::from(offer.params.n);
        Ok(Bridge {
            offer,
            params_bytes,
            mailbox,
            inbox: VecDeque::new(),
            outbox: Vec::new(),
            rx: Vec::new(),
            seq: 0,
            was_present: false,
            started: false,
            started_at: None,
            pile_deadline: PILE_DEADLINE,
            pile_whole: false,
            highest_card: 0,
            seen: HashSet::new(),
            sent_up: HashSet::new(),
            strays: HashSet::new(),
            attested: vec![false; n],
        })
    }

    /// Give the top card `deadline` in place of [`PILE_DEADLINE`], for tests.
    pub fn with_pile_deadline(mut self, deadline: Duration) -> Bridge {
        self.pile_deadline = deadline;
        self
    }

    pub fn offer(&self) -> &Offer {
        &self.offer
    }

    /// Entries the relay returned for the offer's channel, in the relay's order.
    pub fn deliver(&mut self, entries: Vec<(u64, Vec<u8>)>) {
        self.inbox.extend(entries.into_iter().map(|(_, wire)| wire));
    }

    /// The station messages to post now, in order. Each is taken once.
    pub fn take_posts(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outbox)
    }

    /// Tell the card on the cable where it is and how many seats there are.
    fn greet(&self, link: &mut dyn Link) {
        link.send(&bridged_seat_frame(0, &self.offer.params));
        link.send(&count_frame(self.offer.params.n));
    }

    fn send_ring(&mut self, link: &mut dyn Link, kind: u8, index: u8, body: &[u8]) {
        let mut payload = Vec::with_capacity(1 + body.len());
        payload.push(index);
        payload.extend_from_slice(body);
        let bytes = frame::encode(&frame::Frame {
            kind,
            seq: self.seq,
            payload: &payload,
        });
        self.seq = self.seq.wrapping_add(1);
        link.send(&bytes);
    }

    /// The pile is in the ceremony: give it the host's parameters for every seat that is
    /// not a card, as each of those seats would on a cable.
    fn start(&mut self, link: &mut dyn Link) {
        self.started = true;
        self.started_at = Some(Instant::now());
        let params = self.params_bytes.clone();
        for seat in 0..self.offer.params.n {
            if !self.offer.is_badge_seat(seat) {
                self.send_ring(link, Frame::Params as u8, seat, &params);
            }
        }
    }

    /// Seal a card's payload as its seat's channel message, in parts, for the poster.
    fn queue(&mut self, seat: u8, round: Round, payload: &[u8]) {
        for part in parts(payload) {
            let wire = self.offer.channel.seal(seat, round, &part, &mut OsRng);
            self.outbox.push(wire);
        }
    }

    /// One card frame from the cable. A frame from a seat above the cards means the pile is
    /// bigger than the offer, and an abort ends the run. Other frames for seats that are not
    /// cards are dropped.
    fn take_card_frame(
        &mut self,
        on_join_code: &mut dyn FnMut(u8, &str),
        kind: u8,
        payload: &[u8],
    ) -> Result<(), BridgeError> {
        let Some((&index, body)) = payload.split_first() else {
            return Ok(());
        };
        let badges = self.offer.badges;
        if index > badges && index < self.offer.params.n {
            return Err(BridgeError::Count {
                pile: index,
                offer: badges,
            });
        }
        if kind == Frame::Abort as u8 {
            return Err(BridgeError::Aborted {
                seat: index,
                reason: body.first().copied().unwrap_or(0),
            });
        }
        if !self.offer.is_badge_seat(index) {
            return Ok(());
        }
        self.highest_card = self.highest_card.max(index);
        let key = (kind, index, Sha256::digest(body).into());
        if !self.seen.insert(key) {
            return Ok(());
        }
        match kind {
            k if k == Frame::Params as u8 => {
                if body != self.params_bytes {
                    return Err(BridgeError::Params { seat: index });
                }
                if index == badges {
                    self.pile_whole = true;
                }
            }
            k if k == Frame::Commit as u8 => {
                self.queue(index, Round::Commit, body);
                on_join_code(index, &join_code(&self.offer.channel.id, index, body));
            }
            k if k == Frame::Reveal as u8 => self.queue(index, Round::Reveal, body),
            k if k == Frame::Attest as u8 => {
                self.queue(index, Round::Attest, body);
                self.attested[usize::from(index)] = true;
            }
            // A kind the ring does not have.
            _ => {}
        }
        Ok(())
    }

    /// Read the cable and hand on each whole frame.
    fn read_link(
        &mut self,
        link: &mut dyn Link,
        on_join_code: &mut dyn FnMut(u8, &str),
    ) -> Result<(), BridgeError> {
        let mut buf = [0u8; 4096];
        loop {
            let n = link.recv(&mut buf);
            if n == 0 {
                break;
            }
            self.rx.extend_from_slice(&buf[..n]);
        }
        loop {
            let (kind, payload, used) = match frame::decode(&self.rx) {
                Ok((f, used)) => (f.kind, f.payload.to_vec(), used),
                Err(FrameError::Incomplete) => break,
                Err(FrameError::Crc) => {
                    self.rx.remove(0);
                    continue;
                }
            };
            self.rx.drain(..used);
            if kind == KIND_HELLO {
                // A card that came on late, or restarted, asks where it is.
                self.greet(link);
            } else if kind < KIND_FIRST {
                if !self.started {
                    self.start(link);
                }
                self.take_card_frame(on_join_code, kind, &payload)?;
            }
        }
        Ok(())
    }

    /// Send each relay message from a seat that is not a card up the cable.
    fn read_inbox(&mut self, link: &mut dyn Link) {
        while let Some(wire) = self.inbox.pop_front() {
            // The seat is in the associated data, so a message that claims a card's seat
            // opens only as that seat's. A card's own messages come back this way too.
            if wire.len() > 21 && self.offer.is_badge_seat(wire[21]) {
                continue;
            }
            let Some((seat, round, payload)) = self.mailbox.receive(&wire) else {
                continue;
            };
            if self.offer.is_badge_seat(seat) || seat >= self.offer.params.n {
                continue;
            }
            if seat == 0 && round == Round::Commit {
                let hash = commitment_hash(&payload);
                if hash != self.offer.host {
                    // Not the host whose code was given: someone else has the code. Skip it
                    // and wait for the real one, as a joining phone does.
                    if self.strays.insert(hash) {
                        eprintln!("A seat 1 setup on the relay is not the host's. It is ignored.");
                    }
                    continue;
                }
            }
            let kind = match round {
                Round::Commit => Frame::Commit,
                Round::Reveal => Frame::Reveal,
                Round::Attest => Frame::Attest,
            };
            if round == Round::Commit {
                // The first commitment for a seat takes it, as it does for every member.
                self.mailbox.hold(seat, round);
            } else if !self
                .sent_up
                .insert((round as u8, seat, Sha256::digest(&payload).into()))
            {
                continue;
            }
            self.send_ring(link, kind as u8, seat, &payload);
            if round == Round::Attest {
                self.attested[usize::from(seat)] = true;
            }
        }
    }

    /// One pass over the cable: greet a card that appeared, carry what the cable and the
    /// delivered relay entries hold, and say whether every seat's signature has gone across.
    /// Relay entries wait until the pile has started.
    pub fn step(
        &mut self,
        link: &mut dyn Link,
        on_join_code: &mut dyn FnMut(u8, &str),
    ) -> Result<BridgeStep, BridgeError> {
        let present = link.peer_present();
        if present && !self.was_present {
            self.greet(link);
        }
        if !present && self.was_present {
            return Err(BridgeError::Link);
        }
        self.was_present = present;
        self.read_link(link, on_join_code)?;
        if self.started {
            self.read_inbox(link);
        }
        if let Some(at) = self.started_at {
            if !self.pile_whole && at.elapsed() >= self.pile_deadline {
                return Err(BridgeError::Count {
                    pile: self.highest_card,
                    offer: self.offer.badges,
                });
            }
        }
        Ok(if self.attested.iter().all(|&a| a) {
            BridgeStep::Done
        } else {
            BridgeStep::Working
        })
    }

    /// Carry the pile into the ceremony until the cable's peer is gone. `on_signed` is called
    /// once, when every card's signature is on the relay and at least one signature for every
    /// other seat has gone down the cable. The bridge cannot check a signature, so it keeps
    /// carrying after that: a forged signature that came first costs nothing, because the real
    /// one still goes down when it comes. Returns `Ok` when the pile is unplugged after
    /// `on_signed`, and `Err(Link)` when it is unplugged before. `poster` posts and `poller`
    /// reads, each on its own thread, and this thread only reads and writes the cable. A relay
    /// that fails is tried again for a while before the bridge gives up.
    pub fn run(
        &mut self,
        link: &mut dyn Link,
        poster: &mut (dyn Relay + Send),
        poller: &mut (dyn Relay + Send),
        on_join_code: &mut dyn FnMut(u8, &str),
        on_signed: &mut dyn FnMut(),
    ) -> Result<(), BridgeError> {
        let id = self.offer.channel.id;
        let stop = AtomicBool::new(false);
        let posted = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let (post_tx, post_rx) = mpsc::channel::<Vec<u8>>();
            let (read_tx, read_rx) = mpsc::channel::<Vec<(u64, Vec<u8>)>>();
            let (err_tx, err_rx) = mpsc::channel::<RelayError>();
            let post_err = err_tx.clone();
            let posted = &posted;
            scope.spawn(move || {
                for wire in post_rx {
                    let mut failures = 0;
                    while let Err(e) = poster.post(&id, &wire) {
                        failures += 1;
                        if failures > RELAY_RETRIES {
                            let _ = post_err.send(e);
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    posted.fetch_add(1, Ordering::SeqCst);
                }
            });
            let stop = &stop;
            scope.spawn(move || {
                let mut cursor = None;
                let mut failures = 0;
                while !stop.load(Ordering::SeqCst) {
                    let began = Instant::now();
                    match poller.poll(&id, cursor, POLL_WAIT_MS) {
                        Ok(entries) => {
                            failures = 0;
                            if let Some(&(seq, _)) = entries.last() {
                                cursor = Some(seq);
                            }
                            if entries.is_empty() {
                                if let Some(rest) = EMPTY_POLL_EVERY.checked_sub(began.elapsed()) {
                                    std::thread::sleep(rest);
                                }
                            } else if read_tx.send(entries).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            if failures > RELAY_RETRIES {
                                let _ = err_tx.send(e);
                                return;
                            }
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
            });
            let result = (|| {
                let mut queued = 0;
                let mut signed = false;
                loop {
                    while let Ok(entries) = read_rx.try_recv() {
                        self.deliver(entries);
                    }
                    let step = match self.step(link, on_join_code) {
                        // The pile was unplugged after everyone signed: the bridge is done.
                        Err(BridgeError::Link) if signed => return Ok(()),
                        other => other?,
                    };
                    for wire in self.take_posts() {
                        queued += 1;
                        let _ = post_tx.send(wire);
                    }
                    if let Ok(e) = err_rx.try_recv() {
                        return Err(BridgeError::Relay(e));
                    }
                    if !signed
                        && step == BridgeStep::Done
                        && posted.load(Ordering::SeqCst) == queued
                        && link.flush()
                    {
                        signed = true;
                        on_signed();
                    }
                    std::thread::sleep(LINK_EVERY);
                }
            })();
            stop.store(true, Ordering::SeqCst);
            result
        })
    }
}
