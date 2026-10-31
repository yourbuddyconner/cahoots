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
//! The bridge checks two things the cards cannot: seat 0's commitment on the relay must be
//! the one the offer names, and every card must propose the offer's parameters.

use cahoots_core::channel::{commitment_hash, join_code, parts, Mailbox, Offer, Round};
use cahoots_core::frame::{self, FrameError};
use cahoots_hal::pile::{bridged_seat_frame, count_frame, KIND_FIRST, KIND_HELLO};
use cahoots_hal::ring::Frame;
use cahoots_hal::Link;
use cahoots_relay::{Relay, RelayError};
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

/// The bridge's mailbox seat. No real seat has it, so the mailbox hands up every seat's
/// messages.
const NO_SEAT: u8 = u8::MAX;
/// Relay failures in a row that [`Bridge::run`] waits out before it gives up.
const RELAY_RETRIES: u32 = 30;

#[derive(Debug)]
pub enum BridgeError {
    /// The pile has `pile` cards and the host's offer counts `offer`.
    Count { pile: u8, offer: u8 },
    /// Seat 0's commitment on the relay is not the host's the offer names.
    HostMismatch,
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
                "the pile has {pile} cards and the host's code is for {offer}. Start again on the phone with {pile} cards, or change the pile"
            ),
            BridgeError::HostMismatch => write!(
                f,
                "seat 1 on the relay is not the phone whose code you gave. Stop and start again"
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
    Done,
}

pub struct Bridge {
    offer: Offer,
    params_bytes: Vec<u8>,
    mailbox: Mailbox,
    /// The last relay entry read.
    cursor: Option<u64>,
    /// Bytes from the cable not yet decoded.
    rx: Vec<u8>,
    seq: u32,
    was_present: bool,
    /// A card's ring frame came down: the pile is in the ceremony. Until then nothing but
    /// pile frames goes up, so a `Seat` sent again for a late `Hello` is never stuck behind
    /// a ring frame the card's pile layer will not read past.
    started: bool,
    /// (kind, index, SHA-256 of the body) of each card frame taken from the cable.
    seen: HashSet<(u8, u8, [u8; 32])>,
    /// Per seat: its signature has gone where it was going (a card's to the relay, a
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
            cursor: None,
            rx: Vec::new(),
            seq: 0,
            was_present: false,
            started: false,
            seen: HashSet::new(),
            attested: vec![false; n],
        })
    }

    pub fn offer(&self) -> &Offer {
        &self.offer
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
        let params = self.params_bytes.clone();
        for seat in 0..self.offer.params.n {
            if !self.offer.is_badge_seat(seat) {
                self.send_ring(link, Frame::Params as u8, seat, &params);
            }
        }
    }

    /// Post a card's payload as its seat's channel message, in parts.
    fn post(
        &self,
        relay: &mut dyn Relay,
        seat: u8,
        round: Round,
        payload: &[u8],
    ) -> Result<(), RelayError> {
        for part in parts(payload) {
            let wire = self.offer.channel.seal(seat, round, &part, &mut OsRng);
            relay.post(&self.offer.channel.id, &wire)?;
        }
        Ok(())
    }

    /// One card frame from the cable. Frames for seats that are not cards are dropped.
    fn take_card_frame(
        &mut self,
        relay: &mut dyn Relay,
        on_join_code: &mut dyn FnMut(u8, &str),
        kind: u8,
        payload: &[u8],
    ) -> Result<(), BridgeError> {
        let Some((&index, body)) = payload.split_first() else {
            return Ok(());
        };
        if !self.offer.is_badge_seat(index) || index >= self.offer.params.n {
            return Ok(());
        }
        let key = (kind, index, Sha256::digest(body).into());
        if self.seen.contains(&key) {
            return Ok(());
        }
        match kind {
            k if k == Frame::Params as u8 => {
                if body != self.params_bytes {
                    return Err(BridgeError::Params { seat: index });
                }
            }
            k if k == Frame::Commit as u8 => {
                self.post(relay, index, Round::Commit, body)?;
                on_join_code(index, &join_code(&self.offer.channel.id, index, body));
            }
            k if k == Frame::Reveal as u8 => self.post(relay, index, Round::Reveal, body)?,
            k if k == Frame::Attest as u8 => {
                self.post(relay, index, Round::Attest, body)?;
                self.attested[usize::from(index)] = true;
            }
            // An abort, or a kind the ring does not have: the cards tell each other, and the
            // phones see the ceremony stop.
            _ => {}
        }
        self.seen.insert(key);
        Ok(())
    }

    /// Read the cable and hand on each whole frame. A frame leaves the buffer only once it
    /// was handled, so a relay failure loses nothing.
    fn read_link(
        &mut self,
        link: &mut dyn Link,
        relay: &mut dyn Relay,
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
            if kind == KIND_HELLO {
                // A card that came on late, or restarted, asks where it is.
                self.greet(link);
            } else if kind < KIND_FIRST {
                if !self.started {
                    self.start(link);
                }
                self.take_card_frame(relay, on_join_code, kind, &payload)?;
            }
            self.rx.drain(..used);
        }
        Ok(())
    }

    /// Read the relay and send each message from a seat that is not a card up the cable.
    fn read_relay(&mut self, link: &mut dyn Link, relay: &mut dyn Relay) -> Result<(), BridgeError> {
        let entries = relay.poll(&self.offer.channel.id, self.cursor, 0)?;
        for (seq, wire) in entries {
            self.cursor = Some(seq);
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
            if seat == 0 && round == Round::Commit && commitment_hash(&payload) != self.offer.host {
                return Err(BridgeError::HostMismatch);
            }
            let kind = match round {
                Round::Commit => Frame::Commit,
                Round::Reveal => Frame::Reveal,
                Round::Attest => Frame::Attest,
            };
            self.send_ring(link, kind as u8, seat, &payload);
            self.mailbox.hold(seat, round);
            if round == Round::Attest {
                self.attested[usize::from(seat)] = true;
            }
        }
        Ok(())
    }

    /// One pass: greet a card that appeared, carry what the cable and the relay hold, and
    /// say whether every seat's signature has gone across.
    pub fn step(
        &mut self,
        link: &mut dyn Link,
        relay: &mut dyn Relay,
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
        self.read_link(link, relay, on_join_code)?;
        if self.started {
            self.read_relay(link, relay)?;
        }
        Ok(if self.attested.iter().all(|&a| a) {
            BridgeStep::Done
        } else {
            BridgeStep::Working
        })
    }

    /// Step until every seat's signature has gone across, then push the last frames down the
    /// cable. A relay that fails is tried again for a while before the bridge gives up.
    pub fn run(
        &mut self,
        link: &mut dyn Link,
        relay: &mut dyn Relay,
        on_join_code: &mut dyn FnMut(u8, &str),
    ) -> Result<(), BridgeError> {
        let mut failures = 0;
        loop {
            match self.step(link, relay, on_join_code) {
                Ok(BridgeStep::Done) => break,
                Ok(BridgeStep::Working) => failures = 0,
                Err(BridgeError::Relay(_)) if failures < RELAY_RETRIES => {
                    failures += 1;
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(e) => return Err(e),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut buf = [0u8; 4096];
        while !link.flush() {
            if !link.peer_present() {
                return Err(BridgeError::Link);
            }
            while link.recv(&mut buf) > 0 {}
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}
