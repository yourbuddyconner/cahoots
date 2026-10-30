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
//! The offer is `"cahoots:ceremony:v1:" || base64url(id || key || relay_len || relay || params)`:
//! the relay address the host uses, at most 255 bytes of UTF-8 after its one-byte length, and
//! the parameters in their canonical encoding ([`CircleParams::encode`]). Everyone joining uses
//! the host's relay.
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
//! payload once, in parts, and puts them back together on the other side. The caller marks a
//! seat and round held once the ceremony accepts its payload, and later messages for it are
//! ignored. The first commitment on the relay for a seat takes it: a device whose own
//! commitment comes second has lost its seat and must post nothing more.

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

    /// The code text to show: `"cahoots:ceremony:v1:" || base64url(id || key || relay_len ||
    /// relay || params)`. `relay` is the host's relay address, at most 255 bytes; a longer one
    /// is refused with `Malformed`.
    pub fn offer(&self, params: &CircleParams, relay: &str) -> Result<String, ChannelError> {
        let relay_len = u8::try_from(relay.len()).map_err(|_| ChannelError::Malformed)?;
        let mut raw = Zeroizing::new(Vec::with_capacity(
            ID_LEN + KEY_LEN + 1 + relay.len() + 8 + params.name.len(),
        ));
        raw.extend_from_slice(&self.id);
        raw.extend_from_slice(self.key.as_ref());
        raw.push(relay_len);
        raw.extend_from_slice(relay.as_bytes());
        raw.extend_from_slice(&params.encode());
        let mut out = String::from(OFFER_PREFIX);
        out.push_str(&URL_SAFE_NO_PAD.encode(&raw[..]));
        Ok(out)
    }

    /// The channel, the circle's parameters and the host's relay address in an offer.
    pub fn from_offer(text: &str) -> Result<(Channel, CircleParams, String), ChannelError> {
        let body = text
            .trim()
            .strip_prefix(OFFER_PREFIX)
            .ok_or(ChannelError::NotAnOffer)?;
        let raw = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(body)
                .map_err(|_| ChannelError::Malformed)?,
        );
        if raw.len() < ID_LEN + KEY_LEN + 1 {
            return Err(ChannelError::Malformed);
        }
        let relay_at = ID_LEN + KEY_LEN + 1;
        let relay_end = relay_at + usize::from(raw[ID_LEN + KEY_LEN]);
        let relay = raw
            .get(relay_at..relay_end)
            .and_then(|r| core::str::from_utf8(r).ok())
            .ok_or(ChannelError::Malformed)?;
        let relay = String::from(relay);
        let params =
            CircleParams::decode(&raw[relay_end..]).map_err(|_| ChannelError::Malformed)?;
        let mut id = [0u8; ID_LEN];
        id.copy_from_slice(&raw[..ID_LEN]);
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(&raw[ID_LEN..ID_LEN + KEY_LEN]);
        Ok((Channel { id, key }, params, relay))
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
    /// Seats and rounds whose payload the ceremony accepted. Later messages for them are ignored.
    held: BTreeSet<(u8, Round)>,
    /// Whether our seat is ours: `Some(true)` once our commitment is the first for it on the
    /// relay, `Some(false)` once another device's came first.
    seat_won: Option<bool>,
}

impl Mailbox {
    pub fn new(channel: Channel, seat: u8) -> Mailbox {
        Mailbox {
            channel,
            seat,
            outgoing: BTreeMap::new(),
            partial: BTreeMap::new(),
            held: BTreeSet::new(),
            seat_won: None,
        }
    }

    /// Another device's commitment for our seat came first on the relay. This device must
    /// post nothing more: `queue` does nothing and `unposted` is empty from then on.
    pub fn lost_seat(&self) -> bool {
        self.seat_won == Some(false)
    }

    /// The ceremony accepted the payload from `seat` for `round`. Later messages for it are
    /// ignored. A payload the ceremony refused is not held, so the slot stays open.
    pub fn hold(&mut self, seat: u8, round: Round) {
        self.held.insert((seat, round));
    }

    pub fn held(&self, seat: u8, round: Round) -> bool {
        self.held.contains(&(seat, round))
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
        if self.queued(round) || self.lost_seat() {
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
        if self.lost_seat() {
            return Vec::new();
        }
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

    /// The relay took `wire`. The round it belongs to, if it is ours.
    pub fn posted(&mut self, wire: &[u8]) -> Option<Round> {
        for (round, o) in self.outgoing.iter_mut() {
            if let Some(i) = o.wires.iter().position(|w| w[..] == *wire) {
                o.posted[i] = true;
                return Some(*round);
            }
        }
        None
    }

    /// Read one message from the relay, in the relay's order. `Some` when it completes a
    /// payload from another seat for a round not yet held: apply it, then `hold` it if the
    /// ceremony accepted it. Our own messages mark themselves posted, and our commitment read
    /// back before any other for our seat wins the seat. Another device's commitment for our
    /// seat read first loses it (`lost_seat`). Messages for our seat from other devices are
    /// never handed up. A message that does not open, or a part that does not fit what came
    /// first, is dropped.
    pub fn receive(&mut self, wire: &[u8]) -> Option<(u8, Round, Vec<u8>)> {
        if let Some(round) = self.posted(wire) {
            if round == Round::Commit && self.seat_won.is_none() {
                self.seat_won = Some(true);
            }
            return None;
        }
        let (seat, round, part) = self.channel.open(wire).ok()?;
        if seat == self.seat {
            if round == Round::Commit && self.seat_won.is_none() {
                self.seat_won = Some(false);
            }
            return None;
        }
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
        let payload = p.parts.into_iter().flatten().flatten().collect();
        Some((seat, round, payload))
    }
}

/// The text every joiner's code starts with.
pub const JOINED_PREFIX: &str = "cahoots:joined:v1:";
/// Commitments kept per seat while the host waits to scan, so an intruder's cannot crowd out
/// the real one.
const CANDIDATES_PER_SEAT: usize = 4;
/// Messages kept per seat and round while that seat's commitment waits for its scan.
const DEFERRED_PER_SLOT: usize = 4;

/// A joiner's code, shown to the host after the joiner posts its commitment:
/// `"cahoots:joined:v1:" || base64url(channel id || seat || SHA-256(commitment))`. The
/// commitment is the commitment payload as posted.
pub fn join_code(channel_id: &[u8; 16], seat: u8, commitment: &[u8]) -> String {
    use sha2::Digest;
    let mut raw = Vec::with_capacity(ID_LEN + 1 + 32);
    raw.extend_from_slice(channel_id);
    raw.push(seat);
    raw.extend_from_slice(&Sha256::digest(commitment));
    let mut out = String::from(JOINED_PREFIX);
    out.push_str(&URL_SAFE_NO_PAD.encode(&raw));
    out
}

/// The channel id, seat and commitment hash in a joiner's code.
pub fn parse_join_code(text: &str) -> Result<([u8; 16], u8, [u8; 32]), ChannelError> {
    let body = text
        .trim()
        .strip_prefix(JOINED_PREFIX)
        .ok_or(ChannelError::NotAnOffer)?;
    let raw = URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|_| ChannelError::Malformed)?;
    if raw.len() != ID_LEN + 1 + 32 {
        return Err(ChannelError::Malformed);
    }
    let mut id = [0u8; ID_LEN];
    id.copy_from_slice(&raw[..ID_LEN]);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&raw[ID_LEN + 1..]);
    Ok((id, raw[ID_LEN], hash))
}

/// The host's side of the handshake in both directions. Every joiner scanned the host's offer.
/// The host scans each joiner's code, and only a commitment from the relay whose hash matches
/// that scan reaches the host's ceremony. Until then the seat's commitments are held here, and
/// its later messages wait too. An intruder who photographed the offer can post a commitment,
/// but the host never takes it, so the host's manifest differs from the one the joiners build
/// with the intruder in it, and nobody completes.
#[derive(Default)]
pub struct Gate {
    candidates: BTreeMap<u8, Vec<Vec<u8>>>,
    scanned: BTreeMap<u8, [u8; 32]>,
    fed: BTreeSet<u8>,
    deferred: Vec<(u8, Round, Vec<u8>)>,
}

impl Gate {
    pub fn new() -> Gate {
        Gate::default()
    }

    /// A commitment read from the relay for `seat`. Held until a scan matches it.
    pub fn commitment(&mut self, seat: u8, payload: Vec<u8>) {
        if self.fed.contains(&seat) {
            return;
        }
        let c = self.candidates.entry(seat).or_default();
        if c.len() < CANDIDATES_PER_SEAT && !c.contains(&payload) {
            c.push(payload);
        }
    }

    /// A joiner's code scanned on the host's screen. Returns its seat. A code for another
    /// channel is `OtherChannel`. A second scan for a seat replaces the first until the seat's
    /// commitment is taken.
    pub fn scan(&mut self, channel: &Channel, text: &str) -> Result<u8, ChannelError> {
        let (id, seat, hash) = parse_join_code(text)?;
        if id != channel.id {
            return Err(ChannelError::OtherChannel);
        }
        if !self.fed.contains(&seat) {
            self.scanned.insert(seat, hash);
        }
        Ok(seat)
    }

    /// Whether the host's ceremony has taken `seat`'s commitment.
    pub fn is_fed(&self, seat: u8) -> bool {
        self.fed.contains(&seat)
    }

    /// A reveal or a signature from a seat whose commitment is not taken yet. It waits.
    pub fn defer(&mut self, seat: u8, round: Round, payload: Vec<u8>) {
        let n = self
            .deferred
            .iter()
            .filter(|(s, r, _)| *s == seat && *r == round)
            .count();
        if n < DEFERRED_PER_SLOT && !self.deferred.iter().any(|(s, r, p)| *s == seat && *r == round && *p == payload) {
            self.deferred.push((seat, round, payload));
        }
    }

    /// The commitments whose hash matches a scan and are not yet taken: give each to the
    /// ceremony, then call `fed` for the ones it accepted.
    pub fn ready(&self) -> Vec<(u8, Vec<u8>)> {
        use sha2::Digest;
        self.scanned
            .iter()
            .filter(|(seat, _)| !self.fed.contains(seat))
            .filter_map(|(seat, hash)| {
                let c = self.candidates.get(seat)?;
                c.iter()
                    .find(|p| Sha256::digest(p).as_slice() == hash)
                    .map(|p| (*seat, p.clone()))
            })
            .collect()
    }

    /// The ceremony took `seat`'s commitment. Its waiting messages can go in now.
    pub fn fed(&mut self, seat: u8) {
        self.fed.insert(seat);
        self.candidates.remove(&seat);
    }

    /// The waiting messages of seats whose commitment is taken, in the order they arrived.
    pub fn take_deferred(&mut self) -> Vec<(u8, Round, Vec<u8>)> {
        let (go, wait): (Vec<_>, Vec<_>) = core::mem::take(&mut self.deferred)
            .into_iter()
            .partition(|(s, _, _)| self.fed.contains(s));
        self.deferred = wait;
        go
    }

    /// Seats scanned here whose commitment the ceremony took.
    pub fn scanned(&self) -> Vec<u8> {
        self.fed.iter().copied().collect()
    }

    /// Seats with a commitment on the relay that no scan here matches.
    pub fn unscanned(&self) -> Vec<u8> {
        self.candidates
            .iter()
            .filter(|(s, c)| !self.fed.contains(s) && !c.is_empty())
            .map(|(s, _)| *s)
            .collect()
    }
}
