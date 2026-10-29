//! Pile discovery: a card in a stack learns its seat and how many cards there are, then hands
//! its links to the ceremony.
//!
//! Each card has two connectors: `down` (the spring fingers) and `up` (the pads). The detect
//! line on each says whether something is plugged in. The bottom card, with nothing below it,
//! is seat 0 and names the circle. Each card tells the card above it `Seat` with its own seat
//! and the bottom card's proposal, so the card above is one seat higher. A card that lands on a
//! pile asks below with `Hello`, and asks again now and then until it is answered. When the
//! top card's button is pressed, it sends `Count` down the pile. A card that holds its seat,
//! the proposal and the count is ready, and [`Pile::into_ceremony`] turns it into a
//! [`RingCeremony`] over the same two links.
//!
//! Frames use the ceremony's framing with kinds 16 and up, which the ceremony driver ignores,
//! so a late `Hello` cannot abort anything.
//!
//! | kind | payload |
//! |---|---|
//! | `Hello` 16 | none |
//! | `Seat` 17 | `seat u8 \| pad_len u32 LE \| agent u8 \| name` |
//! | `Count` 18 | `n u8` |
//! | `Ping` 30 | none. A presence beacon for links with no detect line, such as a USB serial port: the side that can see the port is open sends one now and then, and the other side treats a recent `Ping` as "plugged in". Both layers ignore it. |
//! | `Log` 31 | UTF-8 text. A line of the sender's log, for a host on the other end of the cable to print. Both layers ignore it. |

use crate::ring::{RingCeremony, RingError};
use crate::{Entropy, Link};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::frame::{self, FrameError};
use cahoots_core::params::{CircleParams, ParamsError};
use cahoots_core::seat::SeatPublic;

/// Kinds from here up belong to the pile layer.
pub const KIND_FIRST: u8 = 16;
pub const KIND_HELLO: u8 = 16;
pub const KIND_SEAT: u8 = 17;
pub const KIND_COUNT: u8 = 18;
pub const KIND_PING: u8 = 30;
pub const KIND_LOG: u8 = 31;

const DOWN: usize = 0;
const UP: usize = 1;
/// Polls between repeated `Hello`s while a card below has not answered.
const HELLO_EVERY: u16 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PileError {
    /// Only the top card can start the count.
    NotTop,
    /// We do not know our seat yet.
    NoSeat,
    /// A frame had an impossible payload.
    Protocol,
    Params(ParamsError),
}

/// What a card knows once the pile has been counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub params: CircleParams,
    pub seat: u8,
}

#[derive(Clone)]
struct Proposal {
    name: String,
    pad_len: u32,
    agent: bool,
}

pub struct Pile<L: Link> {
    links: [L; 2],
    mine: Proposal,
    proposal: Option<Proposal>,
    seat: Option<u8>,
    count: Option<u8>,
    rx: [Vec<u8>; 2],
    seq: u32,
    was_present: [bool; 2],
    hello_in: u16,
}

impl<L: Link> Pile<L> {
    /// `name`, `pad_len` and `agent` are this card's proposal for the circle, used only if it
    /// turns out to be the bottom of the pile.
    pub fn new(down: L, up: L, name: &str, pad_len: u32, agent: bool) -> Self {
        Pile {
            links: [down, up],
            mine: Proposal {
                name: String::from(name),
                pad_len,
                agent,
            },
            proposal: None,
            seat: None,
            count: None,
            rx: [Vec::new(), Vec::new()],
            seq: 0,
            was_present: [false, false],
            hello_in: 0,
        }
    }

    pub fn seat(&self) -> Option<u8> {
        self.seat
    }
    pub fn count(&self) -> Option<u8> {
        self.count
    }
    pub fn is_bottom(&self) -> bool {
        !self.links[DOWN].peer_present()
    }
    pub fn is_top(&self) -> bool {
        !self.links[UP].peer_present()
    }
    #[doc(hidden)]
    pub fn up_present_for_test(&self) -> &L {
        &self.links[UP]
    }

    fn send(&mut self, side: usize, kind: u8, payload: &[u8]) {
        if !self.links[side].peer_present() {
            return;
        }
        let f = frame::Frame {
            kind,
            seq: self.seq,
            payload,
        };
        self.seq = self.seq.wrapping_add(1);
        let bytes = frame::encode(&f);
        self.links[side].send(&bytes);
    }

    fn send_seat_up(&mut self) {
        let (Some(seat), Some(p)) = (self.seat, self.proposal.as_ref()) else {
            return;
        };
        let mut payload = Vec::with_capacity(6 + p.name.len());
        payload.push(seat);
        payload.extend_from_slice(&p.pad_len.to_le_bytes());
        payload.push(p.agent as u8);
        payload.extend_from_slice(p.name.as_bytes());
        self.send(UP, KIND_SEAT, &payload);
    }

    /// The top card's button: announce how many cards are in the pile.
    pub fn start(&mut self) -> Result<(), PileError> {
        if !self.is_top() {
            return Err(PileError::NotTop);
        }
        let seat = self.seat.ok_or(PileError::NoSeat)?;
        let n = seat + 1;
        self.count = Some(n);
        self.send(DOWN, KIND_COUNT, &[n]);
        Ok(())
    }

    fn drain(&mut self) {
        let mut buf = [0u8; 256];
        for side in [DOWN, UP] {
            loop {
                let n = self.links[side].recv(&mut buf);
                if n == 0 {
                    break;
                }
                self.rx[side].extend_from_slice(&buf[..n]);
            }
        }
    }

    /// Read the links and advance. Returns `Some` once the seat, the proposal and the count are
    /// all known. Call repeatedly.
    pub fn poll(&mut self) -> Result<Option<Ready>, PileError> {
        self.drain();
        // Nothing below us: we are the bottom, seat 0, and the circle is ours to name.
        if self.is_bottom() && self.seat != Some(0) {
            self.seat = Some(0);
            self.proposal = Some(self.mine.clone());
            self.send_seat_up();
        }
        // A card landed on us: tell it where it is. A pile appeared under us: ask.
        for side in [DOWN, UP] {
            let now = self.links[side].peer_present();
            if now && !self.was_present[side] {
                if side == UP {
                    self.send_seat_up();
                } else {
                    self.seat = None;
                    self.proposal = None;
                    self.hello_in = 0;
                }
            }
            self.was_present[side] = now;
        }
        if !self.is_bottom() && self.seat.is_none() {
            if self.hello_in == 0 {
                self.send(DOWN, KIND_HELLO, &[]);
                self.hello_in = HELLO_EVERY;
            } else {
                self.hello_in -= 1;
            }
        }
        for side in [DOWN, UP] {
            loop {
                let (kind, payload, used) = match frame::decode(&self.rx[side]) {
                    Ok((f, used)) => (f.kind, f.payload.to_vec(), used),
                    Err(FrameError::Incomplete) => break,
                    Err(FrameError::Crc) => {
                        self.rx[side].remove(0);
                        continue;
                    }
                };
                if kind < KIND_FIRST {
                    // The ceremony has begun on this link. Leave its frames for the driver.
                    break;
                }
                self.rx[side].drain(..used);
                self.handle(side, kind, &payload)?;
            }
        }
        Ok(self.ready())
    }

    fn handle(&mut self, side: usize, kind: u8, payload: &[u8]) -> Result<(), PileError> {
        match (side, kind) {
            (DOWN, KIND_SEAT) => {
                if payload.len() < 6 {
                    return Err(PileError::Protocol);
                }
                let name = core::str::from_utf8(&payload[6..]).map_err(|_| PileError::Protocol)?;
                self.seat = Some(payload[0].checked_add(1).ok_or(PileError::Protocol)?);
                self.proposal = Some(Proposal {
                    name: String::from(name),
                    pad_len: u32::from_le_bytes(payload[1..5].try_into().expect("4 bytes")),
                    agent: payload[5] != 0,
                });
                self.send_seat_up();
            }
            (UP, KIND_HELLO) => self.send_seat_up(),
            (UP, KIND_COUNT) => {
                let n = *payload.first().ok_or(PileError::Protocol)?;
                self.count = Some(n);
                self.send(DOWN, KIND_COUNT, &[n]);
            }
            // A Hello from below or a Seat from above cannot happen in a consistent pile; a
            // Count from below is an echo. Ignore them all.
            _ => {}
        }
        Ok(())
    }

    fn ready(&self) -> Option<Ready> {
        let (Some(seat), Some(n), Some(p)) = (self.seat, self.count, self.proposal.as_ref()) else {
            return None;
        };
        CircleParams::new(n, p.pad_len, &p.name, p.agent)
            .ok()
            .map(|params| Ready { params, seat })
    }

    /// Hand the links, and any ceremony bytes that already arrived on them, to the ceremony.
    /// `tier` is this device's declared tier (see [`RingCeremony::new`]).
    pub fn into_ceremony(
        self,
        ready: Ready,
        entropy: &mut dyn Entropy,
        tier: u8,
    ) -> Result<RingCeremony<L>, RingError> {
        let [down, up] = self.links;
        let [rx_down, rx_up] = self.rx;
        RingCeremony::new_buffered(
            ready.params,
            ready.seat,
            entropy,
            vec![down, up],
            vec![rx_down, rx_up],
            tier,
        )
    }

    /// Like [`into_ceremony`](Self::into_ceremony), for a seat key held outside (a chip) whose
    /// public key is `seat_public`. Poll the ceremony with
    /// [`RingCeremony::poll_signing`].
    pub fn into_ceremony_with_seat(
        self,
        ready: Ready,
        entropy: &mut dyn Entropy,
        seat_public: SeatPublic,
        tier: u8,
    ) -> Result<RingCeremony<L>, RingError> {
        let [down, up] = self.links;
        let [rx_down, rx_up] = self.rx;
        RingCeremony::with_seat_public(
            ready.params,
            ready.seat,
            entropy,
            vec![down, up],
            vec![rx_down, rx_up],
            seat_public,
            tier,
        )
    }
}
