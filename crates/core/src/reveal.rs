//! Sealed reveals: a contribution encrypted to every other member of the ceremony.
//!
//! The ceremony link (a wire, a USB cable, or a phone screen) radiates. Without this, an
//! attacker within antenna or camera range during the ceremony records every contribution
//! in the clear and holds the pad. With it, each contribution is encrypted once under a
//! fresh content key, and that key is wrapped to every other member with X-Wing (X25519
//! plus ML-KEM-768). The nearby attacker is reduced to breaking both; everyone else is
//! unchanged, because the pad is still the XOR of the contributions and never depends on
//! the key exchange.
//!
//! Each member's KEM public key is bound into its commitment, so someone spliced into the
//! cable cannot substitute keys without the reveal failing its commitment check. The seat
//! public key and declared tier travel in the header and are bound the same way: in the
//! commitment, and in the content's associated data.
//!
//! Blob layout (all lengths fixed except the contribution):
//!
//! ```text
//! version (u8 = 2) | from (u8) | pk_from (1216) | seat_pk (33) | tier (u8) | k (u8)
//! k × [ to (u8) | ct (1120) | wrap (48) ]
//! ChaCha20-Poly1305(content_key, nonce 0,
//!     aad = "cahoots/reveal/v1" || params || from || pk_from || seat_pk || tier, R_from)
//! ```
//!
//! `wrap` is `ChaCha20-Poly1305(wrap_key, nonce 0, aad = from || to, content_key)` where
//! `wrap_key = HKDF-SHA256(salt = "cahoots/reveal-wrap/v1", ikm = shared secret,
//! info = params || from || to)`. Every key here is used exactly once, so the nonces are zero.

use crate::params::CircleParams;
use crate::seat::SeatPublic;
use alloc::vec::Vec;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hybrid_array::Array;
use sha2::Sha256;
use x_wing::{
    Ciphertext, Decapsulate, DecapsulationKey, Decapsulator, EncapsulationKey, KeyExport,
};
use zeroize::Zeroizing;

pub const REVEAL_VERSION: u8 = 2;
pub const PUBLIC_KEY_LEN: usize = x_wing::ENCAPSULATION_KEY_SIZE;
pub const KEM_SEED_LEN: usize = x_wing::DECAPSULATION_KEY_SIZE;
pub const CIPHERTEXT_LEN: usize = x_wing::CIPHERTEXT_SIZE;
pub const ENCAPS_RANDOMNESS_LEN: usize = x_wing::ENCAPSULATION_RANDOMNESS_SIZE;
pub const CONTENT_KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
pub const WRAP_LEN: usize = CONTENT_KEY_LEN + TAG_LEN;
pub const CONTENT_DOMAIN: &[u8] = b"cahoots/reveal/v1";
pub const WRAP_DOMAIN: &[u8] = b"cahoots/reveal-wrap/v1";
/// A seat public key in the header: a SEC1 compressed P-256 point.
const SEAT_PUBLIC_LEN: usize = 33;
const HEADER_LEN: usize = 1 + 1 + PUBLIC_KEY_LEN + SEAT_PUBLIC_LEN + 1 + 1;
const ENTRY_LEN: usize = 1 + CIPHERTEXT_LEN + WRAP_LEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealError {
    /// The blob does not parse, or a public key is not a valid X-Wing key.
    Malformed,
    /// The blob is addressed to other members only.
    NotForMe,
    /// A wrap or the content failed authentication. Nothing was learned; scan again.
    Corrupted,
}

/// Random bytes `seal` consumes for a ceremony of `n` members: one content key, then one
/// encapsulation per other member, in ascending member order.
pub fn randomness_len(n: u8) -> usize {
    CONTENT_KEY_LEN + ENCAPS_RANDOMNESS_LEN * (usize::from(n).saturating_sub(1))
}

/// Derive a keypair from a 32-byte seed. The seed is the only secret; keep it with the contribution.
pub fn keypair(seed: &[u8; KEM_SEED_LEN]) -> (DecapsulationKey, [u8; PUBLIC_KEY_LEN]) {
    let dk = DecapsulationKey::from(*seed);
    let pk = dk.encapsulation_key().to_bytes();
    let mut out = [0u8; PUBLIC_KEY_LEN];
    out.copy_from_slice(&pk);
    (dk, out)
}

pub fn parse_public_key(bytes: &[u8]) -> Result<EncapsulationKey, RevealError> {
    if bytes.len() != PUBLIC_KEY_LEN {
        return Err(RevealError::Malformed);
    }
    EncapsulationKey::try_from(bytes).map_err(|_| RevealError::Malformed)
}

fn wrap_key(params: &CircleParams, from: u8, to: u8, shared: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut info = params.encode();
    info.push(from);
    info.push(to);
    let hk = Hkdf::<Sha256>::new(Some(WRAP_DOMAIN), shared);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(&info, out.as_mut())
        .expect("32 bytes is a valid HKDF output length");
    out
}

/// The content's associated data covers the sender's public keys and tier too, so a blob
/// whose header was altered in transit fails to open instead of opening with other values.
fn content_aad(
    params: &CircleParams,
    from: u8,
    pk_from: &[u8],
    seat_pk: &SeatPublic,
    tier: u8,
) -> Vec<u8> {
    let mut aad =
        Vec::with_capacity(CONTENT_DOMAIN.len() + 80 + PUBLIC_KEY_LEN + SEAT_PUBLIC_LEN + 1);
    aad.extend_from_slice(CONTENT_DOMAIN);
    aad.extend_from_slice(&params.encode());
    aad.push(from);
    aad.extend_from_slice(pk_from);
    aad.extend_from_slice(&seat_pk.0);
    aad.push(tier);
    aad
}

/// Seal `contribution` from member `from` to every `(index, public key)` in `recipients`.
/// `seat_pk` and `tier` are the sender's seat public key and declared tier, carried in the
/// header. `fill` supplies `randomness_len(n)` bytes in total, drawn as the layout above describes.
#[allow(clippy::too_many_arguments)]
pub fn seal(
    params: &CircleParams,
    from: u8,
    pk_from: &[u8],
    seat_pk: &SeatPublic,
    tier: u8,
    recipients: &[(u8, &[u8])],
    contribution: &[u8],
    fill: &mut dyn FnMut(&mut [u8]),
) -> Result<Sealed, RevealError> {
    if pk_from.len() != PUBLIC_KEY_LEN || recipients.len() > 255 {
        return Err(RevealError::Malformed);
    }
    let keys = recipients
        .iter()
        .map(|(_, pk)| parse_public_key(pk))
        .collect::<Result<Vec<_>, _>>()?;
    let mut content_key = Zeroizing::new([0u8; CONTENT_KEY_LEN]);
    fill(content_key.as_mut());
    let mut secrets = Vec::with_capacity(recipients.len());

    let mut out = Vec::with_capacity(
        HEADER_LEN + ENTRY_LEN * recipients.len() + contribution.len() + TAG_LEN,
    );
    out.push(REVEAL_VERSION);
    out.push(from);
    out.extend_from_slice(pk_from);
    out.extend_from_slice(&seat_pk.0);
    out.push(tier);
    out.push(recipients.len() as u8);
    for ((to, _), ek) in recipients.iter().zip(&keys) {
        let mut eseed = Zeroizing::new([0u8; ENCAPS_RANDOMNESS_LEN]);
        fill(eseed.as_mut());
        let randomness: Array<u8, hybrid_array::sizes::U64> = (*eseed).into();
        let (ct, shared) = ek.encapsulate_deterministic(&randomness);
        let wk = wrap_key(params, from, *to, &shared);
        let mut s = Zeroizing::new([0u8; 32]);
        s.copy_from_slice(&shared);
        secrets.push((*to, s));
        let cipher = ChaCha20Poly1305::new(Key::from_slice(wk.as_ref()));
        let wrap = cipher
            .encrypt(
                Nonce::from_slice(&[0u8; 12]),
                Payload {
                    msg: content_key.as_ref(),
                    aad: &[from, *to],
                },
            )
            .expect("encryption cannot fail");
        out.push(*to);
        out.extend_from_slice(&ct);
        out.extend_from_slice(&wrap);
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(content_key.as_ref()));
    let body = cipher
        .encrypt(
            Nonce::from_slice(&[0u8; 12]),
            Payload {
                msg: contribution,
                aad: &content_aad(params, from, pk_from, seat_pk, tier),
            },
        )
        .expect("encryption cannot fail");
    out.extend_from_slice(&body);
    Ok(Sealed { blob: out, secrets })
}

/// A sealed reveal and the shared secret established with each recipient.
#[derive(Debug)]
pub struct Sealed {
    pub blob: Vec<u8>,
    pub secrets: Vec<(u8, Zeroizing<[u8; 32]>)>,
}

/// Who sealed this blob, read without decrypting anything.
pub fn sender(blob: &[u8]) -> Option<u8> {
    (blob.len() >= HEADER_LEN && blob[0] == REVEAL_VERSION).then(|| blob[1])
}

#[derive(Debug)]
pub struct Opened {
    pub from: u8,
    pub public_key: [u8; PUBLIC_KEY_LEN],
    /// The sender's seat public key, as the header carries it.
    pub seat_public: SeatPublic,
    /// The sender's declared device tier, as the header carries it.
    pub tier: u8,
    pub contribution: Zeroizing<Vec<u8>>,
    /// The X-Wing shared secret the sender encapsulated to us. Input to the pair key.
    pub shared: Zeroizing<[u8; 32]>,
}

/// Open a sealed reveal as member `me` with decapsulation key `dk`.
pub fn open(
    params: &CircleParams,
    me: u8,
    dk: &DecapsulationKey,
    blob: &[u8],
) -> Result<Opened, RevealError> {
    if blob.len() < HEADER_LEN || blob[0] != REVEAL_VERSION {
        return Err(RevealError::Malformed);
    }
    let from = blob[1];
    let mut public_key = [0u8; PUBLIC_KEY_LEN];
    public_key.copy_from_slice(&blob[2..2 + PUBLIC_KEY_LEN]);
    let seat_at = 2 + PUBLIC_KEY_LEN;
    let seat_public = SeatPublic::parse(&blob[seat_at..seat_at + SEAT_PUBLIC_LEN])
        .map_err(|_| RevealError::Malformed)?;
    let tier = blob[seat_at + SEAT_PUBLIC_LEN];
    let k = usize::from(blob[HEADER_LEN - 1]);
    let entries_end = HEADER_LEN + k * ENTRY_LEN;
    if blob.len() < entries_end + TAG_LEN {
        return Err(RevealError::Malformed);
    }
    let mine = (0..k)
        .map(|i| &blob[HEADER_LEN + i * ENTRY_LEN..HEADER_LEN + (i + 1) * ENTRY_LEN])
        .find(|e| e[0] == me)
        .ok_or(RevealError::NotForMe)?;
    let ct =
        Ciphertext::try_from(&mine[1..1 + CIPHERTEXT_LEN]).map_err(|_| RevealError::Malformed)?;
    let shared = dk.decapsulate(&ct);
    let wk = wrap_key(params, from, me, &shared);
    let mut shared_out = Zeroizing::new([0u8; 32]);
    shared_out.copy_from_slice(&shared);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(wk.as_ref()));
    let content_key = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&[0u8; 12]),
                Payload {
                    msg: &mine[1 + CIPHERTEXT_LEN..],
                    aad: &[from, me],
                },
            )
            .map_err(|_| RevealError::Corrupted)?,
    );
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&content_key));
    let contribution = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&[0u8; 12]),
                Payload {
                    msg: &blob[entries_end..],
                    aad: &content_aad(params, from, &public_key, &seat_public, tier),
                },
            )
            .map_err(|_| RevealError::Corrupted)?,
    );
    if contribution.len() != params.pad_len as usize {
        return Err(RevealError::Malformed);
    }
    Ok(Opened {
        from,
        public_key,
        seat_public,
        tier,
        contribution,
        shared: shared_out,
    })
}
