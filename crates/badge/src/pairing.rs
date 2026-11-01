//! Pairing a phone: the pairing object, the inbox a phone writes and the four words both sides
//! show.
//!
//! 1. The owner opens `pair a phone` on the badge. The badge draws a fresh 16-byte nonce and
//!    writes it to the pairing object ([`cahoots_chip::KeyId::PAIRING`]), which any tap reads
//!    and only the owner session writes ([`PairingObject`]). It keeps the nonce in memory for
//!    as long as the page is open, and writes a zero nonce when the page closes.
//! 2. The phone taps: it reads the nonce and writes the inbox ([`cahoots_chip::KeyId::INBOX`])
//!    in no session: its public key, a name for itself, the AES key it made for the phone
//!    slot, and its signature over the nonce, the key and the name ([`Inbox`]). Any session
//!    may write the inbox and only the owner session may read it.
//! 3. The owner presses both: the badge reads the inbox, checks the signature against the
//!    nonce of the open page, and shows the name and four words ([`words`]) drawn from the
//!    nonce, the phone's public key and the AES key. The phone shows the same four words from
//!    what it wrote. A tap that replaced the AES key, or the whole inbox, gives other words
//!    or a signature that does not check.
//!
//! The inbox holds canonical CBOR from its first byte: `{0: public key (bstr 33), 1: name
//! (text), 2: AES key (bstr 16), 3: signature (bstr 64, P-256 r || s)}`. The signature is
//! ECDSA P-256 with SHA-256 by the phone's key over `"cahoots/pair-inbox/v1" || nonce || AES
//! key || name`. What follows the map is not read: a tap writes over the start of the inbox
//! and need not clear the rest. An inbox of zeros is empty.
//!
//! The pairing object holds `{0: nonce (bstr 16), 1: the badge's name (text, empty while the
//! badge has none)}`, zero-filled to [`PAIRING_LEN`]. A zero nonce means no pairing is open.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::checkwords::{indices_from_digest, WORDLIST};
use cahoots_core::seat::{SeatPublic, SeatSigner};
use core::fmt;
use minicbor::{Decoder, Encoder};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

/// The domain at the head of the bytes the four words are drawn from.
pub const WORDS_DOMAIN: &[u8] = b"cahoots/pair-phone/v1";

/// The domain at the head of the bytes a phone signs in the inbox.
pub const INBOX_DOMAIN: &[u8] = b"cahoots/pair-inbox/v1";

/// The longest phone name, in bytes, that the inbox takes.
pub const MAX_NAME: usize = 64;

/// The pairing object's length.
pub const PAIRING_LEN: usize = 96;

/// The four words for a pairing: the first 44 bits of `SHA-256("cahoots/pair-phone/v1" ||
/// nonce || phone_public || key)` as four words of the check-word list, drawn as
/// [`cahoots_core::checkwords`] draws six.
pub fn words(nonce: &[u8; 16], phone_public: &[u8; 33], key: &[u8; 16]) -> [&'static str; 4] {
    let mut h = Sha256::new();
    h.update(WORDS_DOMAIN);
    h.update(nonce);
    h.update(phone_public);
    h.update(key);
    let idx = indices_from_digest(&h.finalize());
    [0, 1, 2, 3].map(|i| WORDLIST[usize::from(idx[i])])
}

/// The bytes a phone signs: `"cahoots/pair-inbox/v1" || nonce || key || name`.
pub fn signed_bytes(nonce: &[u8; 16], key: &[u8; 16], name: &str) -> Zeroizing<Vec<u8>> {
    let mut m = Zeroizing::new(Vec::with_capacity(INBOX_DOMAIN.len() + 32 + name.len()));
    m.extend_from_slice(INBOX_DOMAIN);
    m.extend_from_slice(nonce);
    m.extend_from_slice(key);
    m.extend_from_slice(name.as_bytes());
    m
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
    /// The phone's signature over [`signed_bytes`] for the badge's nonce.
    pub sig: [u8; 64],
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
    /// The inbox a phone holding `signer` writes for the badge's `nonce`.
    pub fn signed<S: SeatSigner>(mut signer: S, nonce: &[u8; 16], name: &str, key: [u8; 16]) -> Inbox {
        let public = signer.public().0;
        let sig = signer.sign(&signed_bytes(nonce, &key, name));
        Inbox {
            public,
            name: String::from(name),
            key,
            sig,
        }
    }

    /// True if the signature is the phone key's over `nonce`, this AES key and this name.
    pub fn verify(&self, nonce: &[u8; 16]) -> bool {
        SeatPublic::parse(&self.public)
            .is_ok_and(|p| p.verify(&signed_bytes(nonce, &self.key, &self.name), &self.sig))
    }

    /// The canonical CBOR a phone writes. It holds the AES key: zeroized on drop.
    pub fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut e = Encoder::new(Vec::new());
        e.map(4)
            .and_then(|e| e.u8(0))
            .and_then(|e| e.bytes(&self.public))
            .and_then(|e| e.u8(1))
            .and_then(|e| e.str(&self.name))
            .and_then(|e| e.u8(2))
            .and_then(|e| e.bytes(&self.key))
            .and_then(|e| e.u8(3))
            .and_then(|e| e.bytes(&self.sig))
            .expect("writing to a Vec cannot fail");
        Zeroizing::new(e.into_writer())
    }

    /// The inbox's contents, from the start of `bytes`: `None` for an empty inbox and for
    /// anything that is not a canonical inbox with a P-256 key and a name of 1 to
    /// [`MAX_NAME`] bytes. The signature is not checked here: see [`Inbox::verify`].
    pub fn decode(bytes: &[u8]) -> Option<Inbox> {
        let mut d = Decoder::new(bytes);
        if d.map().ok()? != Some(4) || d.u8().ok()? != 0 {
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
        if d.u8().ok()? != 3 {
            return None;
        }
        let sig: [u8; 64] = d.bytes().ok()?.try_into().ok()?;
        let end = d.position();
        let inbox = Inbox { public, name, key, sig };
        if inbox.name.is_empty()
            || inbox.name.len() > MAX_NAME
            || SeatPublic::parse(&inbox.public).is_err()
            || *inbox.encode() != bytes[..end]
        {
            return None;
        }
        Some(inbox)
    }
}

/// What the pairing object says: the nonce of the open pairing, zeros when none is open, and
/// the badge's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingObject {
    pub nonce: [u8; 16],
    pub name: String,
}

impl PairingObject {
    /// The object as stored: the CBOR, zero-filled to [`PAIRING_LEN`]. `None` if the name is
    /// too long to fit.
    pub fn padded(&self) -> Option<Vec<u8>> {
        let mut e = Encoder::new(Vec::new());
        e.map(2)
            .and_then(|e| e.u8(0))
            .and_then(|e| e.bytes(&self.nonce))
            .and_then(|e| e.u8(1))
            .and_then(|e| e.str(&self.name))
            .expect("writing to a Vec cannot fail");
        let mut out = e.into_writer();
        if out.len() > PAIRING_LEN {
            return None;
        }
        out.resize(PAIRING_LEN, 0);
        Some(out)
    }

    /// Strict: the canonical map, then only zeros.
    pub fn decode(bytes: &[u8]) -> Option<PairingObject> {
        let mut d = Decoder::new(bytes);
        if d.map().ok()? != Some(2) || d.u8().ok()? != 0 {
            return None;
        }
        let nonce: [u8; 16] = d.bytes().ok()?.try_into().ok()?;
        if d.u8().ok()? != 1 {
            return None;
        }
        let o = PairingObject {
            nonce,
            name: String::from(d.str().ok()?),
        };
        (o.padded()? == bytes).then_some(o)
    }
}
