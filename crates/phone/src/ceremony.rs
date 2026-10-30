//! The ceremony as the browser app runs it: three rounds of codes, shown as animated QR frames
//! and read by the camera. The frames and the text forms are the browser app's
//! (`web/app/app.js`), so a phone and a browser run one ceremony together.
//!
//! Text forms: `cahoots-params:<b64>`, `cahoots-commit:<seat>:<b64>[:<params b64>]`,
//! `cahoots-reveal:<seat>:<b64>`, `cahoots-attest:<seat>:<b64>`. Animated forms: RaptorQ
//! packets, each `CHQ1 || OTI (12) || packet`, base64url, over one of
//! `1 || seat || params len || params || commitment`, `2 || sealed reveal`, `3 || seat || attestation`.
//!
//! A ceremony can also be held through the relay (`cahoots_core::channel`): the host shows one
//! static code, `cahoots:ceremony:v1:...`, everyone joining scans it, and each round's
//! messages go through the relay, sealed under the code's key. The payloads are the same
//! bytes as above, and the rounds are the same `Ceremony`.

use crate::signer::{Seat, Signer};
use crate::{Http, PhoneError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_core::ceremony::{Ceremony, CeremonyError, CeremonyResult, Phase};
use cahoots_core::channel::{Channel, Mailbox, Round};
use cahoots_core::params::CircleParams;
use cahoots_core::reveal::PUBLIC_KEY_LEN;
use cahoots_core::seat::SeatPublic;
use rand_core::OsRng;
use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation, PayloadId};
use std::collections::HashMap;
use zeroize::Zeroizing;

/// Payload bytes per animated frame, as the browser app: about 77 modules a side.
pub const QR_PACKET: u16 = 300;
const MAGIC: &[u8; 4] = b"CHQ1";
const OTI_LEN: usize = 12;
const SEAT_PUBLIC_LEN: usize = 33;
/// Commitment hash, KEM public key, seat public key, tier.
const COMMITMENT_LEN: usize = 32 + PUBLIC_KEY_LEN + SEAT_PUBLIC_LEN + 1;
/// A phone declares tier 0: its pad and root are in ordinary storage.
pub const PHONE_TIER: u8 = 0;
const PACK_COMMIT: u8 = 1;
const PACK_REVEAL: u8 = 2;
const PACK_ATTEST: u8 = 3;

pub(crate) fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim()).ok()
}

fn bad(message: &str) -> PhoneError {
    PhoneError::BadCode {
        message: message.into(),
    }
}

fn plain(message: impl Into<String>) -> PhoneError {
    PhoneError::Ceremony {
        message: message.into(),
    }
}

/// The core's refusals in words the app can show.
pub(crate) fn ceremony_err(e: CeremonyError) -> PhoneError {
    let seat = |i: u8| i + 1;
    plain(match e {
        CeremonyError::BadIndex(i) => format!("There is no seat {} in this circle.", seat(i)),
        CeremonyError::ContributionLength => "The random contribution has the wrong size.".into(),
        CeremonyError::WrongPhase => "That code belongs to another step.".into(),
        CeremonyError::ConflictingCommit(i) => format!(
            "Seat {} showed two different setup codes. Two people may have picked the same seat.",
            seat(i)
        ),
        CeremonyError::BadKey(i) => format!("Seat {}'s setup code does not read. Scan it again.", seat(i)),
        CeremonyError::BadSeed => "A key could not be made. Try again.".into(),
        CeremonyError::BadTier => "A code declares a device tier that does not exist.".into(),
        CeremonyError::Corrupted(i) => format!(
            "Seat {}'s reveal could not be opened. Scan it again.",
            seat(i)
        ),
        CeremonyError::CommitMismatch(i) => format!(
            "Seat {}'s reveal does not match its setup code. The ceremony stopped and nothing was kept.",
            seat(i)
        ),
        CeremonyError::ConflictingReveal(i) => {
            format!("Seat {} showed a second, different reveal. The first one is kept.", seat(i))
        }
        CeremonyError::BadAttestation(i) => format!(
            "Seat {}'s signature does not check out. Ask them to show it again.",
            seat(i)
        ),
        CeremonyError::Aborted => "The ceremony stopped. Nothing was kept.".into(),
    })
}

// ---------------------------------------------------------------------------------------------
// Fountain frames
// ---------------------------------------------------------------------------------------------

/// The frames of one transfer: every source packet, then about half as many repair packets
/// again, so a camera that misses frames on one pass catches up on the next.
pub fn fountain_frames(data: &[u8]) -> Vec<String> {
    let encoder = Encoder::with_defaults(data, QR_PACKET);
    let oti = encoder.get_config().serialize();
    let mut packets: Vec<EncodingPacket> = Vec::new();
    for block in encoder.get_block_encoders() {
        let source = block.source_packets();
        let repair = (source.len() as u32).div_ceil(2) + 2;
        packets.extend(source);
        packets.extend(block.repair_packets(0, repair));
    }
    packets
        .into_iter()
        .map(|p| {
            let body = p.serialize();
            let mut out = Vec::with_capacity(4 + OTI_LEN + body.len());
            out.extend_from_slice(MAGIC);
            out.extend_from_slice(&oti);
            out.extend_from_slice(&body);
            b64(&out)
        })
        .collect()
}

/// Two transfers' frames taken in turn, so one animation carries both.
fn interleave(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    loop {
        match (a.next(), b.next()) {
            (None, None) => return out,
            (x, y) => out.extend(x.into_iter().chain(y)),
        }
    }
}

struct Partial {
    decoder: Decoder,
    /// Each packet id seen, with its symbol: a second transfer of the same length shows up as
    /// the same id with other bytes, and the partial one is started again.
    seen: HashMap<[u8; 4], Vec<u8>>,
    /// When this transfer last got a packet, to drop the stalest first.
    touched: u64,
}

/// Transfers read at once. A camera sees one or two screens; more is noise.
const MAX_PARTIAL: usize = 4;
/// Finished transfers remembered, so their leftover frames are dropped.
const MAX_DONE: usize = 8;
/// The largest step-one transfer: type, seat, params length, params (at most 255 bytes) and
/// the setup code, with room to spare. Enough to join from.
pub const JOIN_MAX_LEN: u64 = 4096;

/// The largest transfer a ceremony with `params` can show: a sealed reveal (header, one
/// wrapped key per other seat, the contribution and its tag), with room to spare.
pub fn max_transfer(params: &CircleParams) -> u64 {
    let per_seat = 1 + 1120 + 48;
    let header = 1 + 1 + PUBLIC_KEY_LEN + SEAT_PUBLIC_LEN + 1 + 1;
    (1 + header + per_seat * usize::from(params.n) + params.pad_len as usize + 16) as u64 + 1024
}

/// Reads animated transfers, one per transfer length at a time. Everything in a frame is
/// checked before the fountain decoder sees it: the decoder trusts its header, and a crafted
/// one would divide by zero or allocate without bound.
pub struct Receiver {
    partial: HashMap<[u8; OTI_LEN], Partial>,
    /// The last transfers finished, re-encoded, so their leftover frames are recognised and
    /// dropped instead of starting a new transfer. Oldest first.
    done: Vec<([u8; OTI_LEN], Encoder)>,
    /// The longest transfer accepted.
    max_len: u64,
    clock: u64,
    /// Packets seen of the transfer most recently fed, and the minimum it needs.
    pub progress: (u32, u32),
}

impl Receiver {
    pub fn new(max_len: u64) -> Self {
        Receiver {
            partial: HashMap::new(),
            done: Vec::new(),
            max_len,
            clock: 0,
            progress: (0, 0),
        }
    }

    /// Feed one frame. `Ok(Some(data))` when a transfer completes. `Err` for a frame that
    /// is not a Cahoots animation this receiver would make sense of.
    pub fn add(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, PhoneError> {
        if packet.len() < 4 + OTI_LEN + 4 || &packet[..4] != MAGIC {
            return Err(bad("That is not a Cahoots code."));
        }
        let mut oti = [0u8; OTI_LEN];
        oti.copy_from_slice(&packet[4..4 + OTI_LEN]);
        let config = ObjectTransmissionInformation::deserialize(&oti);
        let len = config.transfer_length();
        // Only the header an encoder of ours makes for this length, and only lengths a
        // ceremony shows.
        if len == 0
            || len > self.max_len
            || ObjectTransmissionInformation::with_defaults(len, QR_PACKET).serialize() != oti
        {
            return Err(bad("That code is not one this step can read."));
        }
        let body = &packet[4 + OTI_LEN..];
        let mut id = [0u8; 4];
        id.copy_from_slice(&body[..4]);
        let symbol = &body[4..];
        if symbol.len() != usize::from(config.symbol_size())
            || PayloadId::deserialize(&id).source_block_number() >= config.source_blocks()
        {
            return Err(bad("That code is not one this step can read."));
        }
        let pkt = EncodingPacket::deserialize(body);
        if let Some((_, enc)) = self.done.iter().find(|(o, _)| *o == oti) {
            if regenerate(enc, &pkt).as_deref() == Some(body) {
                return Ok(None);
            }
        }
        let needed = len.div_ceil(u64::from(config.symbol_size())).max(1) as u32;
        self.clock += 1;
        if !self.partial.contains_key(&oti) && self.partial.len() >= MAX_PARTIAL {
            let stalest = *self
                .partial
                .iter()
                .min_by_key(|(_, p)| p.touched)
                .map(|(o, _)| o)
                .expect("not empty");
            self.partial.remove(&stalest);
        }
        let clock = self.clock;
        let fresh = || Partial {
            decoder: Decoder::new(config),
            seen: HashMap::new(),
            touched: clock,
        };
        let p = self.partial.entry(oti).or_insert_with(fresh);
        if p.seen.get(&id).is_some_and(|s| s != symbol) {
            *p = fresh();
        }
        p.touched = clock;
        p.seen.insert(id, symbol.to_vec());
        self.progress = (p.seen.len() as u32, needed);
        match p.decoder.decode(pkt) {
            Some(data) => {
                self.partial.remove(&oti);
                self.done.retain(|(o, _)| *o != oti);
                if self.done.len() >= MAX_DONE {
                    self.done.remove(0);
                }
                self.done.push((oti, Encoder::new(&data, config)));
                self.progress = (needed, needed);
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }
}

/// The packet `enc` makes under `pkt`'s id, serialized.
fn regenerate(enc: &Encoder, pkt: &EncodingPacket) -> Option<Vec<u8>> {
    let id = pkt.payload_id();
    let block = enc.get_block_encoders().get(usize::from(id.source_block_number()))?;
    let source = block.source_packets();
    let esi = id.encoding_symbol_id();
    let p = if (esi as usize) < source.len() {
        source.into_iter().nth(esi as usize)?
    } else {
        block
            .repair_packets(esi - source.len() as u32, 1)
            .into_iter()
            .next()?
    };
    Some(p.serialize())
}

// ---------------------------------------------------------------------------------------------
// Codes
// ---------------------------------------------------------------------------------------------

/// One decoded code.
#[derive(Debug)]
pub enum Code {
    /// The circle's parameters, and the seat that showed them when it says.
    Params(Option<u8>, Vec<u8>),
    Commit {
        seat: u8,
        bytes: Vec<u8>,
        params: Option<Vec<u8>>,
    },
    Reveal {
        seat: u8,
        bytes: Vec<u8>,
    },
    Attest {
        seat: u8,
        bytes: Vec<u8>,
    },
}

/// The setup line, naming the seat that shows it. The browser app reads it as a setup line.
/// The text form of a setup code, so a join offer read from an animation can be handed back.
pub fn commit_code(seat: u8, bytes: &[u8], params: &[u8]) -> String {
    format!("cahoots-commit:{seat}:{}:{}", b64(bytes), b64(params))
}

pub fn params_code(params: &CircleParams, seat: u8) -> String {
    format!("cahoots-params:{seat}:{}", b64(&params.encode()))
}

/// The text forms. `None` for anything else.
pub fn parse_tagged(text: &str) -> Option<Code> {
    let rest = text.trim().strip_prefix("cahoots-")?;
    let (kind, rest) = rest.split_once(':')?;
    let parts: Vec<&str> = rest.split(':').collect();
    let seat = |s: &str| s.parse::<u8>().ok();
    Some(match (kind, parts.as_slice()) {
        ("params", [b]) => Code::Params(None, unb64(b)?),
        ("params", [i, b]) => Code::Params(Some(seat(i)?), unb64(b)?),
        ("commit", [i, b]) => Code::Commit {
            seat: seat(i)?,
            bytes: unb64(b)?,
            params: None,
        },
        ("commit", [i, b, p]) => Code::Commit {
            seat: seat(i)?,
            bytes: unb64(b)?,
            params: Some(unb64(p)?),
        },
        ("reveal", [i, b]) => Code::Reveal {
            seat: seat(i)?,
            bytes: unb64(b)?,
        },
        ("attest", [i, b]) => Code::Attest {
            seat: seat(i)?,
            bytes: unb64(b)?,
        },
        _ => return None,
    })
}

/// An animated transfer's bytes.
pub fn parse_packed(bytes: &[u8]) -> Option<Code> {
    match *bytes.first()? {
        PACK_COMMIT => {
            let seat = *bytes.get(1)?;
            let plen = usize::from(*bytes.get(2)?);
            let params = bytes.get(3..3 + plen)?.to_vec();
            Some(Code::Commit {
                seat,
                bytes: bytes.get(3 + plen..)?.to_vec(),
                params: Some(params),
            })
        }
        PACK_REVEAL => Some(Code::Reveal {
            // The sealed reveal's second byte is its sender.
            seat: *bytes.get(2)?,
            bytes: bytes.get(1..)?.to_vec(),
        }),
        PACK_ATTEST => {
            if bytes.len() < 3 {
                return None;
            }
            Some(Code::Attest {
                seat: bytes[1],
                bytes: bytes[2..].to_vec(),
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// One ceremony in progress
// ---------------------------------------------------------------------------------------------

/// A ceremony this phone is in. It lives in memory only: nothing is written before
/// `ceremony_keep`, so a ceremony is lost if the app is closed.
pub struct Pending {
    pub params: CircleParams,
    pub seat: u8,
    pub seat_label: String,
    pub seat_public: SeatPublic,
    /// `None` once finished.
    ceremony: Option<Ceremony>,
    /// Set when every attestation is in.
    pub result: Option<CeremonyResult>,
    /// Our sealed reveal and our attestation, kept to show to anyone still behind.
    reveal: Option<Vec<u8>>,
    attestation: Option<Vec<u8>>,
    /// Signatures scanned before every reveal was in, checked once the manifest exists.
    early_attestations: Vec<(u8, Vec<u8>)>,
    receiver: Receiver,
    /// The relay channel, for a ceremony held through the relay.
    mailbox: Option<Mailbox>,
    /// The relay this ceremony uses, and the circle chats through once kept.
    pub relay: String,
    /// The last relay sequence number read from the channel.
    cursor: Option<u64>,
}

fn random(len: usize) -> Result<Zeroizing<Vec<u8>>, PhoneError> {
    let mut out = Zeroizing::new(vec![0u8; len]);
    // The system generator fills at most 256 bytes a call on some platforms.
    for chunk in out.chunks_mut(256) {
        getrandom::getrandom(chunk).map_err(|e| plain(format!("No randomness: {e}")))?;
    }
    Ok(out)
}

impl Pending {
    /// Start as `seat` with a key made by the signer under `seat_label`.
    pub fn new(
        params: CircleParams,
        seat: u8,
        signer: &dyn Signer,
        seat_label: String,
        channel: Option<Channel>,
        relay: String,
    ) -> Result<Self, PhoneError> {
        let public = signer.make_key(seat_label.clone())?;
        let seat_public = SeatPublic::parse(&public).map_err(|_| PhoneError::Signer {
            message: "The key store returned a key that is not a P-256 public key.".into(),
        })?;
        let contribution = random(params.pad_len as usize)?;
        let kem = random(32)?;
        let kem_seed: Zeroizing<[u8; 32]> = Zeroizing::new(kem[..].try_into().expect("32 bytes"));
        let ceremony = Ceremony::with_seat_public(
            params.clone(),
            seat,
            contribution.to_vec(),
            *kem_seed,
            seat_public,
            PHONE_TIER,
        )
        .map_err(ceremony_err)?;
        let receiver = Receiver::new(max_transfer(&params));
        Ok(Pending {
            params,
            seat,
            seat_label,
            seat_public,
            ceremony: Some(ceremony),
            result: None,
            reveal: None,
            attestation: None,
            early_attestations: Vec::new(),
            receiver,
            mailbox: channel.map(|c| Mailbox::new(c, seat)),
            relay,
            cursor: None,
        })
    }

    pub fn through_relay(&self) -> bool {
        self.mailbox.is_some()
    }

    /// The code everyone joining scans, for a ceremony held through the relay.
    pub fn offer(&self) -> Result<String, PhoneError> {
        let mb = self.mailbox.as_ref().ok_or_else(|| {
            plain("This ceremony is held with codes only. It has no code to join through the relay.")
        })?;
        Ok(mb.channel().offer(&self.params))
    }

    pub fn phase(&self) -> Phase {
        match &self.ceremony {
            Some(c) => c.phase(),
            None => Phase::Complete,
        }
    }

    /// The seats this round still needs something from.
    pub fn missing(&self) -> Vec<u8> {
        let Some(c) = &self.ceremony else {
            return Vec::new();
        };
        let all = match c.phase() {
            Phase::Commit => c.missing_commitments(),
            Phase::Reveal => c.missing_reveals(),
            Phase::Attest => c.missing_attestations(),
            Phase::Complete | Phase::Aborted => Vec::new(),
        };
        all.into_iter().filter(|&i| i != self.seat).collect()
    }

    pub fn progress(&self) -> (u32, u32) {
        self.receiver.progress
    }

    /// Our setup code's content: commitment, KEM public key, seat public key, tier. `None`
    /// once finished.
    fn my_commitment(&self) -> Option<Vec<u8>> {
        let c = self.ceremony.as_ref()?;
        let mut out = Vec::with_capacity(COMMITMENT_LEN);
        out.extend_from_slice(&c.my_commitment());
        out.extend_from_slice(c.my_public_key());
        out.extend_from_slice(&c.my_seat_public().0);
        out.push(PHONE_TIER);
        Some(out)
    }

    /// Our sealed reveal, built once.
    fn my_reveal(&mut self) -> Result<Vec<u8>, PhoneError> {
        if let Some(r) = &self.reveal {
            return Ok(r.clone());
        }
        let c = self.ceremony.as_mut().ok_or_else(|| plain("The ceremony is over."))?;
        let randomness = random(c.reveal_randomness_len())?;
        let mut pos = 0usize;
        let mut fill = |buf: &mut [u8]| {
            buf.copy_from_slice(&randomness[pos..pos + buf.len()]);
            pos += buf.len();
        };
        let r = c
            .my_reveal(&mut fill)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| plain("Every setup code is needed before the reveal."))?;
        self.reveal = Some(r.clone());
        Ok(r)
    }

    /// Whether step three waits for this seat's signature.
    pub fn needs_signature(&self) -> bool {
        self.attestation.is_none()
            && self
                .ceremony
                .as_ref()
                .is_some_and(|c| c.phase() == Phase::Attest && c.my_attestation().is_none())
    }

    /// Sign the manifest through the callback, once. A refused signature (a locked phone)
    /// changes nothing, and the next call asks again. `WrongPhase` before step three.
    pub fn sign(&mut self, signer: &dyn Signer) -> Result<(), PhoneError> {
        if self.attestation.is_some() {
            return Ok(());
        }
        let c = self.ceremony.as_ref().ok_or_else(|| plain("The ceremony is over."))?;
        if !matches!(c.phase(), Phase::Attest | Phase::Complete) {
            return Err(plain("There is nothing to sign until every reveal is in."));
        }
        // Anyone still on step two needs our reveal, and the root needs it sealed.
        self.my_reveal()?;
        self.my_attestation(signer).map(|_| ())
    }

    fn my_attestation(&mut self, signer: &dyn Signer) -> Result<Vec<u8>, PhoneError> {
        if let Some(a) = &self.attestation {
            return Ok(a.clone());
        }
        let c = self.ceremony.as_mut().ok_or_else(|| plain("The ceremony is over."))?;
        if c.my_attestation().is_none() {
            let mut seat = Seat {
                signer,
                label: self.seat_label.clone(),
                public: self.seat_public,
                error: None,
            };
            let r = c.sign_manifest(&mut seat);
            if let Some(e) = seat.error {
                return Err(e);
            }
            r.map_err(ceremony_err)?;
        }
        let a = c
            .my_attestation()
            .ok_or_else(|| plain("The manifest is not ready to sign."))?;
        self.attestation = Some(a.clone());
        self.finish_if_complete()?;
        Ok(a)
    }

    /// The codes to show now, as text payloads. Step three shows this seat's reveal, for
    /// anyone still on step two, and its signature once `sign` has made it.
    pub fn frames(&mut self) -> Result<Vec<String>, PhoneError> {
        match self.phase() {
            Phase::Commit => {
                let commitment = self.my_commitment().expect("not finished in Commit");
                let params = self.params.encode();
                let plen = u8::try_from(params.len())
                    .map_err(|_| plain("The circle's name is too long to show in a code."))?;
                let mut packed = vec![PACK_COMMIT, self.seat, plen];
                packed.extend_from_slice(&params);
                packed.extend_from_slice(&commitment);
                // The setup line first: a single code anyone can join from.
                let mut out = vec![params_code(&self.params, self.seat)];
                out.extend(fountain_frames(&packed));
                Ok(out)
            }
            Phase::Reveal => {
                let r = self.my_reveal()?;
                Ok(fountain_frames(&pack_reveal(&r)))
            }
            Phase::Attest | Phase::Complete => {
                let reveal = fountain_frames(&pack_reveal(&self.my_reveal()?));
                let Some(a) = &self.attestation else {
                    return Ok(reveal);
                };
                let mut packed = vec![PACK_ATTEST, self.seat];
                packed.extend_from_slice(a);
                Ok(interleave(fountain_frames(&packed), reveal))
            }
            Phase::Aborted => Err(ceremony_err(CeremonyError::Aborted)),
        }
    }

    /// Feed one decoded camera code or pasted text.
    pub fn scan(&mut self, text: &str) -> Result<(), PhoneError> {
        let code = if text.trim_start().starts_with("cahoots-") {
            parse_tagged(text).ok_or_else(|| bad("That Cahoots code does not read."))?
        } else {
            let bytes = unb64(text).ok_or_else(|| bad("That is not a Cahoots code."))?;
            match self.receiver.add(&bytes)? {
                None => return Ok(()),
                Some(data) => {
                    parse_packed(&data).ok_or_else(|| bad("That Cahoots code does not read."))?
                }
            }
        };
        self.apply(code)
    }

    fn apply(&mut self, code: Code) -> Result<(), PhoneError> {
        let seat = match &code {
            Code::Params(..) => return Ok(()),
            Code::Commit { seat, .. } | Code::Reveal { seat, .. } | Code::Attest { seat, .. } => {
                *seat
            }
        };
        if seat == self.seat {
            // Our own code seen on our own screen changes nothing. Another phone's code for
            // our seat means two people picked the same one.
            let ours = match &code {
                Code::Commit { bytes, .. } => self.my_commitment().is_none_or(|c| c == *bytes),
                Code::Reveal { bytes, .. } => self.reveal.as_ref() == Some(bytes),
                Code::Attest { bytes, .. } => self.attestation.as_ref() == Some(bytes),
                Code::Params(..) => true,
            };
            if ours {
                return Ok(());
            }
            return Err(plain(format!(
                "Another phone is also seat {}. One of you must leave and join again.",
                seat + 1
            )));
        }
        let Some(c) = self.ceremony.as_mut() else {
            // Finished: everything is in. Codes from people still behind change nothing here.
            return Ok(());
        };
        match code {
            Code::Params(..) => {}
            Code::Commit {
                bytes,
                params,
                seat,
            } => {
                if params.is_some_and(|p| p != self.params.encode()) {
                    return Err(bad("That code is for another circle."));
                }
                if bytes.len() != COMMITMENT_LEN {
                    return Err(bad("That setup code has the wrong size."));
                }
                let (h, rest) = bytes.split_at(32);
                let (kem, rest) = rest.split_at(PUBLIC_KEY_LEN);
                let (seat_pk, tier) = rest.split_at(SEAT_PUBLIC_LEN);
                let h: [u8; 32] = h.try_into().expect("32 bytes");
                c.add_commitment(seat, h, kem, seat_pk, tier[0])
                    .map_err(ceremony_err)?;
            }
            Code::Reveal { seat, bytes } => c.add_reveal(seat, &bytes).map_err(ceremony_err)?,
            Code::Attest { seat, bytes } => {
                if matches!(c.phase(), Phase::Commit | Phase::Reveal) {
                    self.early_attestations.retain(|(i, _)| *i != seat);
                    self.early_attestations.push((seat, bytes));
                } else {
                    c.add_attestation(seat, &bytes).map_err(ceremony_err)?;
                }
            }
        }
        if let Some(c) = self.ceremony.as_mut() {
            if c.phase() == Phase::Attest {
                for (i, a) in std::mem::take(&mut self.early_attestations) {
                    // A bad one is dropped; the seat shows it again.
                    let _ = c.add_attestation(i, &a);
                }
            }
        }
        self.finish_if_complete()
    }

    fn finish_if_complete(&mut self) -> Result<(), PhoneError> {
        if self.ceremony.as_ref().map(Ceremony::phase) != Some(Phase::Complete) {
            return Ok(());
        }
        if self.attestation.is_none() {
            self.attestation = self.ceremony.as_ref().and_then(Ceremony::my_attestation);
        }
        // The circle root needs our own reveal sealed. Showing the frames seals it before
        // anything can complete, so this only draws randomness if that order ever changes.
        self.my_reveal()?;
        let c = self.ceremony.take().expect("checked");
        self.result = Some(c.finish().map_err(ceremony_err)?);
        Ok(())
    }

    pub fn words(&self) -> Vec<String> {
        self.result
            .as_ref()
            .map(|r| r.check_words.iter().map(|w| w.to_string()).collect())
            .unwrap_or_default()
    }
}

impl Pending {
    fn channel_url(&self) -> Option<String> {
        let mb = self.mailbox.as_ref()?;
        Some(format!(
            "{}/v1/circles/{}/messages",
            self.relay.trim_end_matches('/'),
            hex::encode(mb.channel().id)
        ))
    }

    /// Seal what this seat can send now, each round once: the commitment, the reveal once
    /// every commitment is in, the signature once it is made.
    fn queue(&mut self) -> Result<(), PhoneError> {
        let Some(mb) = &self.mailbox else {
            return Ok(());
        };
        let (commit, reveal, attest) = (
            mb.queued(Round::Commit),
            mb.queued(Round::Reveal),
            mb.queued(Round::Attest),
        );
        let commitment = if commit { None } else { self.my_commitment() };
        let reveal = if !reveal
            && matches!(self.phase(), Phase::Reveal | Phase::Attest | Phase::Complete)
        {
            Some(self.my_reveal()?)
        } else {
            None
        };
        let attestation = if attest { None } else { self.attestation.clone() };
        let mb = self.mailbox.as_mut().expect("checked");
        if let Some(c) = commitment {
            mb.queue(Round::Commit, &c, &mut OsRng);
        }
        if let Some(r) = reveal {
            mb.queue(Round::Reveal, &r, &mut OsRng);
        }
        if let Some(a) = attestation {
            mb.queue(Round::Attest, &a, &mut OsRng);
        }
        Ok(())
    }

    /// Post what waits, in order. The first failure stops it, and the rest wait for the next tick.
    fn post(&mut self, http: &dyn Http) -> Result<(), PhoneError> {
        let Some(url) = self.channel_url() else {
            return Ok(());
        };
        let mb = self.mailbox.as_mut().expect("has a url");
        for wire in mb.unposted() {
            let body = http.post(url.clone(), wire.clone())?;
            let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| PhoneError::Network {
                message: "The relay's answer does not read.".into(),
            })?;
            if v["seq"].as_u64().is_none() {
                return Err(PhoneError::Network {
                    message: "The relay did not say where it put the message.".into(),
                });
            }
            mb.posted(&wire);
        }
        Ok(())
    }

    /// One step through the relay: post what waits, read what is new and apply it, then post
    /// what that made ready. The first refusal from a message read is returned after the rest
    /// are read.
    pub fn tick(&mut self, http: &dyn Http) -> Result<(), PhoneError> {
        let url = self.channel_url().ok_or_else(|| {
            plain("This ceremony is held with codes only. Show and scan the codes instead.")
        })?;
        self.queue()?;
        self.post(http)?;
        let mut refused = None;
        loop {
            let mut get = format!("{url}?wait=0");
            if let Some(a) = self.cursor {
                get.push_str(&format!("&after={a}"));
            }
            let entries = crate::chat::parse_entries(&http.get(get)?)?;
            let fresh: Vec<_> = entries
                .into_iter()
                .filter(|(seq, _)| self.cursor.is_none_or(|a| *seq > a))
                .collect();
            if fresh.is_empty() {
                break;
            }
            for (seq, wire) in fresh {
                self.cursor = Some(seq);
                let mb = self.mailbox.as_mut().expect("has a url");
                // A message that does not open, our own, or a second one for a seat and round
                // already held is passed over.
                let Some((seat, round, bytes)) = mb.receive(&wire) else {
                    continue;
                };
                let code = match round {
                    // Every commitment is held once the phase has moved on.
                    Round::Commit if seat != self.seat && self.phase() != Phase::Commit => continue,
                    Round::Commit => Code::Commit {
                        seat,
                        bytes,
                        params: None,
                    },
                    Round::Reveal => Code::Reveal { seat, bytes },
                    Round::Attest => Code::Attest { seat, bytes },
                };
                if let Err(e) = self.apply(code) {
                    refused.get_or_insert(e);
                }
            }
        }
        self.queue()?;
        self.post(http)?;
        refused.map_or(Ok(()), Err)
    }
}

fn pack_reveal(r: &[u8]) -> Vec<u8> {
    let mut packed = Vec::with_capacity(r.len() + 1);
    packed.push(PACK_REVEAL);
    packed.extend_from_slice(r);
    packed
}

/// Read a join code: the setup line, or a setup code that carries the circle's parameters.
/// Returns the parameters and the setup code to keep, if it was one.
pub fn read_join(text: &str) -> Result<(CircleParams, Option<Code>), PhoneError> {
    let code = parse_tagged(text).ok_or_else(|| bad("That is not a code to join a circle from."))?;
    join_from(code)
}

/// The parameters in a join code, and the setup code to keep, if it was one.
pub fn join_from(code: Code) -> Result<(CircleParams, Option<Code>), PhoneError> {
    let (params, shown_by, keep) = match code {
        Code::Params(seat, p) => (p, seat.unwrap_or(0), None),
        Code::Commit {
            seat,
            bytes,
            params: Some(p),
        } => (
            p.clone(),
            seat,
            Some(Code::Commit {
                seat,
                bytes,
                params: Some(p),
            }),
        ),
        Code::Commit { params: None, .. } => {
            return Err(bad("That code has no setup in it. Scan the host instead."))
        }
        Code::Reveal { .. } | Code::Attest { .. } => {
            return Err(bad(
                "That code is from a later step. Join from someone still on step one.",
            ))
        }
    };
    let params = CircleParams::decode(&params).map_err(|_| bad("That setup code does not read."))?;
    if shown_by >= params.n || params.encode().len() > 255 {
        return Err(bad("That setup code does not read."));
    }
    Ok((params, keep))
}

impl Pending {
    /// Apply a code kept from joining.
    pub fn apply_kept(&mut self, code: Code) -> Result<(), PhoneError> {
        self.apply(code)
    }
}
