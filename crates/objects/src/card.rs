//! Contact cards (chip apps spec §6): a seat's profile for its circle, signed by the seat key
//! and sealed for the circle.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | circle_id bstr 16 |
//! | 2 | seat uint |
//! | 3 | name tstr, 1 to 64 bytes |
//! | 4 | handles: array of [kind tstr, value tstr], at most 16 |
//! | 5 | endpoints: array of [purpose uint, value tstr], at most 16 |
//! | 6 | log uint, the log position the laptop names |
//!
//! A handle's `kind` is a short text (`email`, `github`, `nostr`, `x`), 1 to 16 bytes. An
//! endpoint's `purpose` is a [`cahoots_core::control::Purpose`] byte (0 to 5), or
//! [`PURPOSE_SSH`] or [`PURPOSE_CHAT`]; its value is text the laptop knows how to use. Values
//! are 1 to 256 bytes. The whole encoded card is at most [`MAX_CARD_LEN`] bytes.
//!
//! Signed by the seat with kid `[seat]`. The signed card travels sealed: AES-256-GCM under the
//! circle's card seal key, `HMAC-SHA256(root, "seal/card" || circle_id)`, which the badge
//! releases on consent ([`cahoots_core::root::SEAL_CARD`]). The sealed form is a random 12-byte
//! nonce, then the ciphertext and its 16-byte tag; the additional data is [`DOMAIN`].

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatSigner;
use cahoots_core::CircleId;

use crate::{
    canonical, check_seat_and_circle, encode_with, open_seat_signed, Enc, EncResult, ObjectError,
    Reader, VERSION,
};

/// External AAD of a card's COSE_Sign1, and the additional data of its seal.
pub const DOMAIN: &[u8] = b"cahoots/card/v1";

/// An endpoint for ssh: a host, or `host:port`.
pub const PURPOSE_SSH: u8 = 6;
/// An endpoint for chat: a chat address.
pub const PURPOSE_CHAT: u8 = 7;

pub const MAX_NAME: usize = 64;
pub const MAX_KIND: usize = 16;
pub const MAX_VALUE: usize = 256;
pub const MAX_ENTRIES: usize = 16;
/// The longest encoded card. The per-field limits allow about 8.5 KB, but a request other than
/// an ssh signature may be at most 4096 bytes once encoded
/// (`cahoots_custodian::wire::MAX_SMALL_REQUEST_LEN`), and the request that carries a card adds
/// a few dozen bytes; 4000 leaves room for them, so a card that encodes fits its request.
pub const MAX_CARD_LEN: usize = 4000;

/// The length of the sealed form's nonce.
pub const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub circle_id: CircleId,
    pub seat: u8,
    pub name: String,
    /// `(kind, value)`: `("email", "ada@example.com")`.
    pub handles: Vec<(String, String)>,
    /// `(purpose, value)`: `(0, "203.0.113.7:51820")` for WireGuard.
    pub endpoints: Vec<(u8, String)>,
    pub log: u64,
}

/// A purpose's name, for people: the `Purpose` names, then ssh and chat.
pub fn purpose_name(purpose: u8) -> Option<&'static str> {
    Some(match purpose {
        0 => "wireguard",
        1 => "tls",
        2 => "mls",
        3 => "wifi",
        4 => "meshtastic",
        5 => "other",
        PURPOSE_SSH => "ssh",
        PURPOSE_CHAT => "chat",
        _ => return None,
    })
}

/// The purpose a name names, the inverse of [`purpose_name`].
pub fn purpose_from_name(name: &str) -> Option<u8> {
    (0..=PURPOSE_CHAT).find(|p| purpose_name(*p) == Some(name))
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max
}

impl Card {
    /// The canonical CBOR payload. `Malformed` when it is longer than [`MAX_CARD_LEN`].
    pub fn encode(&self) -> Result<Vec<u8>, ObjectError> {
        let bytes = self.encode_unchecked();
        if bytes.len() > MAX_CARD_LEN {
            return Err(ObjectError::Malformed);
        }
        Ok(bytes)
    }

    fn encode_unchecked(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(7)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.u8(self.seat)?;
        e.u8(3)?.str(&self.name)?;
        e.u8(4)?.array(self.handles.len() as u64)?;
        for (kind, value) in &self.handles {
            e.array(2)?.str(kind)?.str(value)?;
        }
        e.u8(5)?.array(self.endpoints.len() as u64)?;
        for (purpose, value) in &self.endpoints {
            e.array(2)?.u8(*purpose)?.str(value)?;
        }
        e.u8(6)?.u64(self.log)?;
        Ok(())
    }

    /// Parse a canonical payload. Anything past the limits in the table, or longer than
    /// [`MAX_CARD_LEN`], is `Malformed`.
    pub fn decode(bytes: &[u8]) -> Result<Card, ObjectError> {
        let m = ObjectError::Malformed;
        if bytes.len() > MAX_CARD_LEN {
            return Err(m);
        }
        let mut r = Reader::open(bytes, 7)?;
        let circle_id = r.array()?;
        let seat = r.u8()?;
        let name = r.string()?;
        if !text_ok(&name, MAX_NAME) {
            return Err(m);
        }
        let handles = {
            let d = r.key()?;
            let n = Reader::array_len(d)?;
            if n > MAX_ENTRIES as u64 {
                return Err(m);
            }
            let mut list = Vec::new();
            for _ in 0..n {
                if Reader::array_len(d)? != 2 {
                    return Err(m);
                }
                let kind = d.str().map_err(|_| m)?;
                let value = d.str().map_err(|_| m)?;
                if !text_ok(kind, MAX_KIND) || !text_ok(value, MAX_VALUE) {
                    return Err(m);
                }
                list.push((String::from(kind), String::from(value)));
            }
            list
        };
        let endpoints = {
            let d = r.key()?;
            let n = Reader::array_len(d)?;
            if n > MAX_ENTRIES as u64 {
                return Err(m);
            }
            let mut list = Vec::new();
            for _ in 0..n {
                if Reader::array_len(d)? != 2 {
                    return Err(m);
                }
                let purpose: u8 = d.u64().map_err(|_| m)?.try_into().map_err(|_| m)?;
                let value = d.str().map_err(|_| m)?;
                if purpose_name(purpose).is_none() || !text_ok(value, MAX_VALUE) {
                    return Err(m);
                }
                list.push((purpose, String::from(value)));
            }
            list
        };
        let card = Card {
            circle_id,
            seat,
            name,
            handles,
            endpoints,
            log: r.u64()?,
        };
        r.finish()?;
        let encoded = card.encode_unchecked();
        canonical(card, encoded, bytes)
    }

    /// Sign as a COSE_Sign1 with the payload embedded; `kid` is `[seat]`. `Malformed` when
    /// the card is longer than [`MAX_CARD_LEN`].
    pub fn sign<S: SeatSigner>(&self, key: S, kid: &[u8]) -> Result<Vec<u8>, ObjectError> {
        Ok(cose::sign1(key, kid, &self.encode()?, DOMAIN, false))
    }

    /// Verify a signed card against the manifest: the kid's seat signed it, the card names
    /// that seat, and the card is the manifest's circle's.
    pub fn verify(bytes: &[u8], manifest: &Manifest) -> Result<Card, ObjectError> {
        let (seat, payload) = open_seat_signed(bytes, manifest, DOMAIN)?;
        let card = Card::decode(&payload)?;
        check_seat_and_circle(seat, card.seat, &card.circle_id, manifest)?;
        Ok(card)
    }
}

/// Seal `signed` (a signed card) under `key` with the given nonce. Use [`seal`] unless the
/// nonce comes from somewhere else; a nonce must never repeat under one key.
pub fn seal_with_nonce(signed: &[u8], key: &[u8; 32], nonce: [u8; NONCE_LEN]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(key.into());
    let payload = Payload {
        msg: signed,
        aad: DOMAIN,
    };
    // Encrypting into a Vec fails only for inputs of gigabytes.
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), payload)
        .unwrap_or_default();
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Seal `signed` under `key` with a random nonce from the operating system.
#[cfg(feature = "std")]
pub fn seal(signed: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, ObjectError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| ObjectError::Random)?;
    Ok(seal_with_nonce(signed, key, nonce))
}

/// Open a sealed card: the signed card's bytes, or `Malformed` if the key is wrong or a byte
/// changed. The caller verifies the card with [`Card::verify`].
pub fn open(sealed: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, ObjectError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(ObjectError::Malformed);
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: DOMAIN,
            },
        )
        .map_err(|_| ObjectError::Malformed)
}
