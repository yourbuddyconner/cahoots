//! The ceremony channel: the ceremony's three rounds carried through a relay, with one code
//! scanned as the handshake.
//!
//! The host makes a [`Channel`]: a random 16-byte id and a random 32-byte key. It shows
//! [`Channel::offer`] as one static QR code. Everyone joining scans it from the host's screen,
//! so only people in the room hold the key. Each member then posts its commitment, its sealed
//! reveal and its signature under the channel id, sealed under the key, and reads the others'
//! from the same place. The round logic is [`crate::ceremony::Ceremony`], the same as with
//! animated codes, so both ways make the same circle.
//!
//! The offer is `"cahoots:ceremony:v1:" || base64url(id || key || params)`, with the
//! parameters in their canonical encoding ([`CircleParams::encode`]).
//!
//! A message on the wire is what the station stores:
//!
//! ```text
//! version (3) | channel id (16) | len u32 LE | seat | round | nonce (12) | ciphertext and tag
//! ```
//!
//! `len` counts every byte after it. The cipher is ChaCha20-Poly1305 under
//! `HKDF-SHA256(ikm = key, salt = id, info = CHANNEL_DOMAIN)`, with a random nonce and
//! `id || seat || round` as associated data. The relay holds ciphertext it cannot open, and a
//! stranger who posts under the id makes a message that does not open and is dropped.
//!
//! A payload larger than one station message travels in parts. [`Mailbox`] seals each round's
//! payload once, in parts, and puts them back together on the other side, keeping the first
//! message for each seat and round.

use crate::params::CircleParams;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use rand_core::CryptoRngCore;
use sha2::Sha256;
use zeroize::Zeroizing;

/// The HKDF info that turns the offer's key into the cipher key.
pub const CHANNEL_DOMAIN: &[u8] = b"cahoots/ceremony/channel/v1";
/// The text every offer starts with.
pub const OFFER_PREFIX: &str = "cahoots:ceremony:v1:";
/// The station's message version for the ceremony channel.
pub const CEREMONY_VERSION: u8 = 3;
/// `version | channel id | len`.
pub const HEADER_LEN: usize = 1 + 16 + 4;
/// `seat | round | nonce` before the ciphertext, and the tag after it.
pub const SEALED_OVERHEAD: usize = 1 + 1 + 12 + 16;
/// The largest message the station takes.
pub const MAX_WIRE: usize = 64 * 1024;
/// `part u16 | parts u16` before each part's bytes.
pub const PART_HEADER: usize = 4;
/// The most payload bytes one part carries, so each message fits the station's cap.
pub const MAX_PART: usize = MAX_WIRE - HEADER_LEN - SEALED_OVERHEAD - PART_HEADER;

const ID_LEN: usize = 16;
const KEY_LEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Round {
    Commit = 1,
    Reveal = 2,
    Attest = 3,
}

impl Round {
    pub fn from_u8(b: u8) -> Option<Round> {
        match b {
            1 => Some(Round::Commit),
            2 => Some(Round::Reveal),
            3 => Some(Round::Attest),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// The text is not a ceremony offer.
    NotAnOffer,
    /// An offer or a message that does not parse.
    Malformed,
    /// A message for another channel.
    OtherChannel,
    /// A message that does not open under this key: damaged, or sealed by someone else.
    Open,
}

/// One ceremony's channel: the id the relay files messages under, and the key that seals them.
#[derive(Clone)]
pub struct Channel {
    pub id: [u8; 16],
    key: Zeroizing<[u8; 32]>,
}

impl core::fmt::Debug for Channel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Channel")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Channel {
    pub fn new(rng: &mut impl CryptoRngCore) -> Channel {
        let mut id = [0u8; ID_LEN];
        rng.fill_bytes(&mut id);
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        rng.fill_bytes(key.as_mut());
        Channel { id, key }
    }

    /// A channel from a known id and key, for test vectors.
    pub fn from_key(id: [u8; 16], key: [u8; 32]) -> Channel {
        Channel {
            id,
            key: Zeroizing::new(key),
        }
    }

    /// The code text to show: `"cahoots:ceremony:v1:" || base64url(id || key || params)`.
    pub fn offer(&self, params: &CircleParams) -> String {
        let mut raw = Zeroizing::new(Vec::with_capacity(ID_LEN + KEY_LEN + 8 + params.name.len()));
        raw.extend_from_slice(&self.id);
        raw.extend_from_slice(self.key.as_ref());
        raw.extend_from_slice(&params.encode());
        let mut out = String::from(OFFER_PREFIX);
        out.push_str(&URL_SAFE_NO_PAD.encode(&raw[..]));
        out
    }

    pub fn from_offer(text: &str) -> Result<(Channel, CircleParams), ChannelError> {
        let body = text
            .trim()
            .strip_prefix(OFFER_PREFIX)
            .ok_or(ChannelError::NotAnOffer)?;
        let raw = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(body)
                .map_err(|_| ChannelError::Malformed)?,
        );
        if raw.len() < ID_LEN + KEY_LEN {
            return Err(ChannelError::Malformed);
        }
        let params =
            CircleParams::decode(&raw[ID_LEN + KEY_LEN..]).map_err(|_| ChannelError::Malformed)?;
        let mut id = [0u8; ID_LEN];
        id.copy_from_slice(&raw[..ID_LEN]);
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(&raw[ID_LEN..ID_LEN + KEY_LEN]);
        Ok((Channel { id, key }, params))
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        let mut k = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(Some(&self.id), self.key.as_ref())
            .expand(CHANNEL_DOMAIN, k.as_mut())
            .expect("32 bytes is a valid HKDF output length");
        ChaCha20Poly1305::new(Key::from_slice(k.as_ref()))
    }

    fn aad(&self, seat: u8, round: Round) -> [u8; ID_LEN + 2] {
        let mut aad = [0u8; ID_LEN + 2];
        aad[..ID_LEN].copy_from_slice(&self.id);
        aad[ID_LEN] = seat;
        aad[ID_LEN + 1] = round as u8;
        aad
    }

    /// Seal `payload` from `seat` for `round` under a random nonce. The result is a whole
    /// station message. A payload over `MAX_WIRE - HEADER_LEN - SEALED_OVERHEAD` bytes makes a
    /// message the station refuses: use [`Mailbox`], which splits it.
    pub fn seal(
        &self,
        seat: u8,
        round: Round,
        payload: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Vec<u8> {
        let mut nonce = [0u8; 12];
        rng.fill_bytes(&mut nonce);
        self.seal_with_nonce(seat, round, payload, nonce)
    }

    /// `seal` with a given nonce, for test vectors. A nonce must never repeat under one key.
    pub fn seal_with_nonce(&self, seat: u8, round: Round, payload: &[u8], nonce: [u8; 12]) -> Vec<u8> {
        let ct = self
            .cipher()
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: payload,
                    aad: &self.aad(seat, round),
                },
            )
            .expect("ChaCha20-Poly1305 encrypts any length a message can have");
        let len = (2 + nonce.len() + ct.len()) as u32;
        let mut out = Vec::with_capacity(HEADER_LEN + len as usize);
        out.push(CEREMONY_VERSION);
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&len.to_le_bytes());
        out.push(seat);
        out.push(round as u8);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    /// Open a station message: the seat that sealed it, the round, and the payload.
    pub fn open(&self, wire: &[u8]) -> Result<(u8, Round, Vec<u8>), ChannelError> {
        if wire.len() < HEADER_LEN + SEALED_OVERHEAD || wire[0] != CEREMONY_VERSION {
            return Err(ChannelError::Malformed);
        }
        let len = u32::from_le_bytes([wire[17], wire[18], wire[19], wire[20]]) as usize;
        if wire.len() - HEADER_LEN != len {
            return Err(ChannelError::Malformed);
        }
        if wire[1..17] != self.id {
            return Err(ChannelError::OtherChannel);
        }
        let seat = wire[21];
        let round = Round::from_u8(wire[22]).ok_or(ChannelError::Malformed)?;
        let nonce = &wire[23..35];
        let payload = self
            .cipher()
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: &wire[35..],
                    aad: &self.aad(seat, round),
                },
            )
            .map_err(|_| ChannelError::Open)?;
        Ok((seat, round, payload))
    }
}

/// `payload` split into parts, each `part u16 LE | parts u16 LE | at most MAX_PART bytes`. An
/// empty payload is one empty part.
pub fn parts(payload: &[u8]) -> Vec<Vec<u8>> {
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[][..]]
    } else {
        payload.chunks(MAX_PART).collect()
    };
    let count = chunks.len() as u16;
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut p = Vec::with_capacity(PART_HEADER + c.len());
            p.extend_from_slice(&(i as u16).to_le_bytes());
            p.extend_from_slice(&count.to_le_bytes());
            p.extend_from_slice(c);
            p
        })
        .collect()
}

struct Partial {
    count: u16,
    parts: Vec<Option<Vec<u8>>>,
}

struct Outgoing {
    wires: Vec<Vec<u8>>,
    posted: Vec<bool>,
}

/// One member's side of the channel: what it has to post, and what it has read.
pub struct Mailbox {
    channel: Channel,
    seat: u8,
    outgoing: BTreeMap<Round, Outgoing>,
    partial: BTreeMap<(u8, Round), Partial>,
    /// Seats and rounds whose payload was handed up. Later messages for them are ignored.
    held: BTreeSet<(u8, Round)>,
}

impl Mailbox {
    pub fn new(channel: Channel, seat: u8) -> Mailbox {
        Mailbox {
            channel,
            seat,
            outgoing: BTreeMap::new(),
            partial: BTreeMap::new(),
            held: BTreeSet::new(),
        }
    }

    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    pub fn seat(&self) -> u8 {
        self.seat
    }

    /// Whether this round's payload is sealed and waiting or posted.
    pub fn queued(&self, round: Round) -> bool {
        self.outgoing.contains_key(&round)
    }

    /// Seal this seat's payload for `round`, once. A second call for the round does nothing.
    pub fn queue(&mut self, round: Round, payload: &[u8], rng: &mut impl CryptoRngCore) {
        if self.queued(round) {
            return;
        }
        let wires: Vec<Vec<u8>> = parts(payload)
            .iter()
            .map(|p| self.channel.seal(self.seat, round, p, rng))
            .collect();
        let posted = vec![false; wires.len()];
        self.outgoing.insert(round, Outgoing { wires, posted });
    }

    /// The messages to post: everything queued that the relay has not taken yet, in round order.
    pub fn unposted(&self) -> Vec<Vec<u8>> {
        self.outgoing
            .values()
            .flat_map(|o| {
                o.wires
                    .iter()
                    .zip(&o.posted)
                    .filter(|(_, p)| !**p)
                    .map(|(w, _)| w.clone())
            })
            .collect()
    }

    /// The relay took `wire`.
    pub fn posted(&mut self, wire: &[u8]) -> bool {
        for o in self.outgoing.values_mut() {
            if let Some(i) = o.wires.iter().position(|w| w[..] == *wire) {
                o.posted[i] = true;
                return true;
            }
        }
        false
    }

    /// Read one message from the relay. `Some` when it completes a payload from a seat and
    /// round not yet held. Our own messages mark themselves posted. Another device's message
    /// for our seat is handed up too, so the caller can say two people picked one seat. A
    /// message that does not open, or a part that does not fit what came first, is dropped.
    pub fn receive(&mut self, wire: &[u8]) -> Option<(u8, Round, Vec<u8>)> {
        if self.posted(wire) {
            return None;
        }
        let (seat, round, part) = self.channel.open(wire).ok()?;
        if self.held.contains(&(seat, round)) || part.len() < PART_HEADER {
            return None;
        }
        let index = u16::from_le_bytes([part[0], part[1]]);
        let count = u16::from_le_bytes([part[2], part[3]]);
        if count == 0 || index >= count {
            return None;
        }
        let p = self.partial.entry((seat, round)).or_insert_with(|| Partial {
            count,
            parts: vec![None; usize::from(count)],
        });
        if p.count != count {
            return None;
        }
        let slot = &mut p.parts[usize::from(index)];
        if slot.is_none() {
            *slot = Some(part[PART_HEADER..].to_vec());
        }
        if p.parts.iter().any(Option::is_none) {
            return None;
        }
        let p = self.partial.remove(&(seat, round)).expect("present");
        self.held.insert((seat, round));
        let payload = p.parts.into_iter().flatten().flatten().collect();
        Some((seat, round, payload))
    }
}
