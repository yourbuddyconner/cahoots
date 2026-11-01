//! Pairing a phone: the pairing object, the sealed inbox a phone writes and the six words both
//! sides show.
//!
//! 1. The owner opens `pair a phone` on the badge. The badge draws a fresh 16-byte nonce and
//!    an ephemeral P-256 key pair ([`PairingKey`]), kept in RAM only, and writes the nonce and
//!    the key's public half to the pairing object ([`cahoots_chip::KeyId::PAIRING`]), which
//!    any tap reads and only the owner session writes ([`PairingObject`]). It keeps both in
//!    memory for as long as the page is open, and writes a zero nonce when the page closes;
//!    the private key is zeroized then, and at every boot.
//! 2. The phone taps: it reads the pairing object and writes the inbox
//!    ([`cahoots_chip::KeyId::INBOX`]) in no session: its public key, a name for itself, the
//!    AES key it made for the phone slot sealed to the badge's ephemeral key, and its
//!    signature over the nonce, the badge's key, the sealed key and the name ([`Inbox`]). Any
//!    session may write the inbox and only the owner session may read it. An antenna that
//!    listens to the tap learns the sealed key and nothing it can open.
//! 3. The owner presses both: the badge reads the inbox, checks the signature against the
//!    open page's nonce and key, opens the sealed key, and shows the name and six words
//!    ([`words`]) drawn from the nonce, its key, the phone's public key and the AES key. The
//!    phone shows the same six words from what it wrote. A tap that replaced the sealed key,
//!    or the whole inbox, gives a signature that does not check or other words; an inbox
//!    recorded from an earlier pairing is signed over another nonce and key.
//!
//! The seal: the phone draws its own ephemeral P-256 key, takes the x coordinate of ECDH with
//! the badge's key, and derives `k = HKDF-SHA256(ikm = shared, salt = nonce, info =
//! "cahoots/pair-seal/v1")`, 32 bytes. The AES key is sealed with ChaCha20-Poly1305 under `k`
//! with a zero 12-byte nonce (`k` is used once) and the associated data `nonce || phone public
//! key || name`. The sealed key is `phone ephemeral public key (33, compressed) || ciphertext
//! (16) || tag (16)`, [`SEALED_LEN`] bytes.
//!
//! The inbox holds canonical CBOR from its first byte: `{0: public key (bstr 33), 1: name
//! (text), 2: sealed key (bstr 65), 3: signature (bstr 64, P-256 r || s)}`. The signature is
//! ECDSA P-256 with SHA-256 by the phone's key over `"cahoots/pair-inbox/v1" || nonce ||
//! badge key || sealed key || name`. What follows the map is not read: a tap writes over the
//! start of the inbox and need not clear the rest. An inbox of zeros is empty.
//!
//! The pairing object holds `{0: nonce (bstr 16), 1: the badge's ephemeral public key (bstr
//! 33, compressed), 2: the badge's name (text, empty while the badge has none), 3: circles
//! (u16, bit `s` for each circle slot whose public object the badge wrote)}`, zero-filled to
//! [`PAIRING_LEN`]. A zero nonce means no pairing is open. The badge makes it at its first
//! boot and keeps it made, so its id is never empty, and rewrites it whenever the circles
//! change. Any tap reads it; only the owner and a paired phone read the public objects.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::checkwords::{indices_from_digest, WORDLIST};
use cahoots_core::seat::{SeatPublic, SeatSigner};
use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, Tag};
use core::fmt;
use minicbor::{Decoder, Encoder};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// The domain at the head of the bytes the six words are drawn from.
pub const WORDS_DOMAIN: &[u8] = b"cahoots/pair-phone/v1";

/// The domain at the head of the bytes a phone signs in the inbox.
pub const INBOX_DOMAIN: &[u8] = b"cahoots/pair-inbox/v1";

/// The HKDF info of the key that seals the phone's AES key.
pub const SEAL_DOMAIN: &[u8] = b"cahoots/pair-seal/v1";

/// The longest phone name, in bytes, that the inbox takes.
pub const MAX_NAME: usize = 64;

/// The sealed AES key: the phone's ephemeral public key, the ciphertext and the tag.
pub const SEALED_LEN: usize = 33 + 16 + 16;

/// The pairing object's length.
pub const PAIRING_LEN: usize = 128;

/// The six words for a pairing: the first 66 bits of `SHA-256("cahoots/pair-phone/v1" ||
/// nonce || badge_key || phone_public || key)` as six words of the check-word list, drawn as
/// [`cahoots_core::checkwords`] draws its six. Six and not four, since an NFC relay could
/// grind 44 bits in the time the owner takes to compare.
pub fn words(
    nonce: &[u8; 16],
    badge_key: &[u8; 33],
    phone_public: &[u8; 33],
    key: &[u8; 16],
) -> [&'static str; 6] {
    let mut h = Sha256::new();
    h.update(WORDS_DOMAIN);
    h.update(nonce);
    h.update(badge_key);
    h.update(phone_public);
    h.update(key);
    indices_from_digest(&h.finalize()).map(|i| WORDLIST[usize::from(i)])
}

/// The bytes a phone signs: `"cahoots/pair-inbox/v1" || nonce || badge_key || sealed ||
/// name`.
pub fn signed_bytes(nonce: &[u8; 16], badge_key: &[u8; 33], sealed: &[u8; SEALED_LEN], name: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(INBOX_DOMAIN.len() + 16 + 33 + SEALED_LEN + name.len());
    m.extend_from_slice(INBOX_DOMAIN);
    m.extend_from_slice(nonce);
    m.extend_from_slice(badge_key);
    m.extend_from_slice(sealed);
    m.extend_from_slice(name.as_bytes());
    m
}

/// The badge's ephemeral key for one pairing. Kept in RAM only; the private key is zeroized
/// when this is dropped.
pub struct PairingKey {
    secret: p256::SecretKey,
    public: [u8; 33],
}

impl fmt::Debug for PairingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingKey").field("public", &self.public).finish_non_exhaustive()
    }
}

fn compressed(secret: &p256::SecretKey) -> [u8; 33] {
    secret
        .public_key()
        .to_encoded_point(true)
        .as_bytes()
        .try_into()
        .expect("a compressed P-256 point is 33 bytes")
}

impl PairingKey {
    /// The key whose secret scalar is `bytes`, drawn at random. `None` for the rare draw that
    /// is not a valid scalar: draw again.
    pub fn from_scalar(bytes: &[u8; 32]) -> Option<PairingKey> {
        let secret = p256::SecretKey::from_slice(bytes).ok()?;
        let public = compressed(&secret);
        Some(PairingKey { secret, public })
    }

    /// The public key, SEC1 compressed, as the pairing object carries it.
    pub fn public(&self) -> [u8; 33] {
        self.public
    }
}

/// `HKDF-SHA256(ikm = x(ECDH(secret, public)), salt = nonce, info = SEAL_DOMAIN)`, or `None`
/// if `public` is not a point.
fn seal_key(secret: &p256::SecretKey, public: &[u8; 33], nonce: &[u8; 16]) -> Option<Zeroizing<[u8; 32]>> {
    let public = p256::PublicKey::from_sec1_bytes(public).ok()?;
    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), public.as_affine());
    let hk = hkdf::Hkdf::<Sha256>::new(Some(nonce), shared.raw_secret_bytes());
    let mut k = Zeroizing::new([0u8; 32]);
    hk.expand(SEAL_DOMAIN, k.as_mut()).expect("32 bytes is a valid HKDF length");
    Some(k)
}

fn aad(nonce: &[u8; 16], public: &[u8; 33], name: &str) -> Vec<u8> {
    let mut a = Vec::with_capacity(16 + 33 + name.len());
    a.extend_from_slice(nonce);
    a.extend_from_slice(public);
    a.extend_from_slice(name.as_bytes());
    a
}

/// What a phone wrote in the inbox. Nothing in it is secret: the AES key is sealed to the
/// badge's ephemeral key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inbox {
    /// The phone's P-256 public key, SEC1 compressed.
    pub public: [u8; 33],
    /// The name the phone gave itself, 1 to [`MAX_NAME`] bytes.
    pub name: String,
    /// The phone slot's AES key, sealed to the badge's key for this pairing.
    pub sealed: [u8; SEALED_LEN],
    /// The phone's signature over [`signed_bytes`].
    pub sig: [u8; 64],
}

impl Inbox {
    /// The inbox a phone holding `signer` writes for the badge's `nonce` and `badge_key`, with
    /// `key` sealed under the phone's ephemeral secret scalar `ephemeral` (32 random bytes,
    /// used for this inbox only). `None` if `ephemeral` is not a valid scalar (draw again) or
    /// `badge_key` is not a point.
    pub fn sealed<S: SeatSigner>(
        mut signer: S,
        nonce: &[u8; 16],
        badge_key: &[u8; 33],
        name: &str,
        key: &[u8; 16],
        ephemeral: &[u8; 32],
    ) -> Option<Inbox> {
        let eph = p256::SecretKey::from_slice(ephemeral).ok()?;
        let k = seal_key(&eph, badge_key, nonce)?;
        let public = signer.public().0;
        let mut buf = Zeroizing::new(*key);
        let tag = ChaCha20Poly1305::new(k.as_ref().into())
            .encrypt_in_place_detached(&Nonce::default(), &aad(nonce, &public, name), buf.as_mut())
            .ok()?;
        let mut sealed = [0u8; SEALED_LEN];
        sealed[..33].copy_from_slice(&compressed(&eph));
        sealed[33..49].copy_from_slice(buf.as_ref());
        sealed[49..].copy_from_slice(&tag);
        let sig = signer.sign(&signed_bytes(nonce, badge_key, &sealed, name));
        Some(Inbox {
            public,
            name: String::from(name),
            sealed,
            sig,
        })
    }

    /// True if the signature is the phone key's over this pairing's `nonce` and `badge_key`,
    /// this sealed key and this name.
    pub fn verify(&self, nonce: &[u8; 16], badge_key: &[u8; 33]) -> bool {
        SeatPublic::parse(&self.public).is_ok_and(|p| {
            p.verify(&signed_bytes(nonce, badge_key, &self.sealed, &self.name), &self.sig)
        })
    }

    /// The AES key, for the badge holding `badge` for the pairing of `nonce`: the signature
    /// checked first, then the seal opened. `None` if either fails.
    pub fn open(&self, nonce: &[u8; 16], badge: &PairingKey) -> Option<Zeroizing<[u8; 16]>> {
        if !self.verify(nonce, &badge.public) {
            return None;
        }
        let eph: [u8; 33] = self.sealed[..33].try_into().expect("33 bytes");
        let k = seal_key(&badge.secret, &eph, nonce)?;
        let mut key = Zeroizing::new([0u8; 16]);
        key.copy_from_slice(&self.sealed[33..49]);
        let tag = Tag::from_slice(&self.sealed[49..]);
        ChaCha20Poly1305::new(k.as_ref().into())
            .decrypt_in_place_detached(&Nonce::default(), &aad(nonce, &self.public, &self.name), key.as_mut(), tag)
            .ok()?;
        Some(key)
    }

    /// The canonical CBOR a phone writes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        e.map(4)
            .and_then(|e| e.u8(0))
            .and_then(|e| e.bytes(&self.public))
            .and_then(|e| e.u8(1))
            .and_then(|e| e.str(&self.name))
            .and_then(|e| e.u8(2))
            .and_then(|e| e.bytes(&self.sealed))
            .and_then(|e| e.u8(3))
            .and_then(|e| e.bytes(&self.sig))
            .expect("writing to a Vec cannot fail");
        e.into_writer()
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
        let sealed: [u8; SEALED_LEN] = d.bytes().ok()?.try_into().ok()?;
        if d.u8().ok()? != 3 {
            return None;
        }
        let sig: [u8; 64] = d.bytes().ok()?.try_into().ok()?;
        let end = d.position();
        let inbox = Inbox { public, name, sealed, sig };
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

/// What the pairing object says: the nonce of the open pairing (zeros when none is open), the
/// badge's ephemeral public key for it, the badge's name, and the circle slots whose public
/// objects the badge vouches for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingObject {
    pub nonce: [u8; 16],
    pub badge_key: [u8; 33],
    pub name: String,
    /// Bit `s` set for each circle slot `s` (0 to 15) whose public object the badge wrote
    /// itself. A phone reads only these: an object at any other public id may be a tap's.
    pub circles: u16,
}

impl PairingObject {
    /// No pairing open: a zero nonce and a zero key, with the badge's `circles`.
    pub fn closed(circles: u16) -> PairingObject {
        PairingObject {
            nonce: [0; 16],
            badge_key: [0; 33],
            name: String::new(),
            circles,
        }
    }

    /// The circle slots in [`PairingObject::circles`], in order.
    pub fn circle_slots(&self) -> Vec<u8> {
        (0..16).filter(|s| self.circles & (1 << s) != 0).collect()
    }

    /// The object as stored: the CBOR, zero-filled to [`PAIRING_LEN`]. `None` if the name is
    /// too long to fit.
    pub fn padded(&self) -> Option<Vec<u8>> {
        let mut e = Encoder::new(Vec::new());
        e.map(4)
            .and_then(|e| e.u8(0))
            .and_then(|e| e.bytes(&self.nonce))
            .and_then(|e| e.u8(1))
            .and_then(|e| e.bytes(&self.badge_key))
            .and_then(|e| e.u8(2))
            .and_then(|e| e.str(&self.name))
            .and_then(|e| e.u8(3))
            .and_then(|e| e.u16(self.circles))
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
        if d.map().ok()? != Some(4) || d.u8().ok()? != 0 {
            return None;
        }
        let nonce: [u8; 16] = d.bytes().ok()?.try_into().ok()?;
        if d.u8().ok()? != 1 {
            return None;
        }
        let badge_key: [u8; 33] = d.bytes().ok()?.try_into().ok()?;
        if d.u8().ok()? != 2 {
            return None;
        }
        let name = String::from(d.str().ok()?);
        if d.u8().ok()? != 3 {
            return None;
        }
        let o = PairingObject {
            nonce,
            badge_key,
            name,
            circles: d.u16().ok()?,
        };
        (o.padded()? == bytes).then_some(o)
    }
}
