//! Pairing a phone: the inbox a phone writes and the four words both sides show.
//!
//! A phone that wants to pair writes the inbox ([`cahoots_chip::KeyId::INBOX`]) in one tap,
//! in no session: its public key, a name for itself and the AES key it made for the phone
//! slot. Any session may write the inbox and only the badge's owner session may read it, so
//! the key in it is seen by the badge alone. The owner then opens `pair a phone` on the badge,
//! which reads the inbox and shows the phone's name and four words ([`words`]) that the phone
//! shows too: if they differ, the phone that wrote the inbox is not the phone in the owner's
//! hand.
//!
//! The inbox holds canonical CBOR, `{0: public key (bstr 33), 1: name (text), 2: AES key (bstr
//! 16)}`, from its first byte. What follows the map is not read: a tap writes over the start
//! of the inbox and need not clear the rest. An inbox of zeros is empty.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::checkwords::{indices_from_digest, WORDLIST};
use cahoots_core::seat::SeatPublic;
use core::fmt;
use minicbor::{Decoder, Encoder};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

/// The domain at the head of the bytes the four words are drawn from.
pub const WORDS_DOMAIN: &[u8] = b"cahoots/pair-phone/v1";

/// The longest phone name, in bytes, that the inbox takes.
pub const MAX_NAME: usize = 64;

/// The four words for a phone's public key and a badge's nonce: the first 44 bits of
/// `SHA-256("cahoots/pair-phone/v1" || phone_public || badge_nonce)` as four words of the
/// check-word list, drawn as [`cahoots_core::checkwords`] draws six.
pub fn words(phone_public: &[u8; 33], badge_nonce: &[u8; 16]) -> [&'static str; 4] {
    let mut h = Sha256::new();
    h.update(WORDS_DOMAIN);
    h.update(phone_public);
    h.update(badge_nonce);
    let idx = indices_from_digest(&h.finalize());
    [0, 1, 2, 3].map(|i| WORDLIST[usize::from(idx[i])])
}

/// What a phone wrote in the inbox. The AES key is a secret: `Debug` leaves it out and it is
/// zeroized on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct Inbox {
    /// The phone's P-256 public key, SEC1 compressed.
    pub public: [u8; 33],
    /// The name the phone gave itself, 1 to [`MAX_NAME`] bytes.
    pub name: String,
    /// The phone slot's AES key, which the phone keeps and opens the slot's session with.
    pub key: [u8; 16],
}

impl fmt::Debug for Inbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inbox")
            .field("public", &self.public)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Drop for Inbox {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl Inbox {
    /// The canonical CBOR a phone writes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        e.map(3)
            .and_then(|e| e.u8(0))
            .and_then(|e| e.bytes(&self.public))
            .and_then(|e| e.u8(1))
            .and_then(|e| e.str(&self.name))
            .and_then(|e| e.u8(2))
            .and_then(|e| e.bytes(&self.key))
            .expect("writing to a Vec cannot fail");
        e.into_writer()
    }

    /// The inbox's contents, from the start of `bytes`: `None` for an empty inbox and for
    /// anything that is not a canonical inbox with a P-256 key and a name of 1 to
    /// [`MAX_NAME`] bytes.
    pub fn decode(bytes: &[u8]) -> Option<Inbox> {
        let mut d = Decoder::new(bytes);
        if d.map().ok()? != Some(3) || d.u8().ok()? != 0 {
            return None;
        }
        let public: [u8; 33] = d.bytes().ok()?.try_into().ok()?;
        if d.u8().ok()? != 1 {
            return None;
        }
        let name = String::from(d.str().ok()?);
        if d.u8().ok()? != 2 {
            return None;
        }
        let key: [u8; 16] = d.bytes().ok()?.try_into().ok()?;
        let end = d.position();
        let inbox = Inbox { public, name, key };
        if inbox.name.is_empty()
            || inbox.name.len() > MAX_NAME
            || SeatPublic::parse(&inbox.public).is_err()
            || inbox.encode() != bytes[..end]
        {
            return None;
        }
        Some(inbox)
    }
}
