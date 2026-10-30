//! HPKE (RFC 9180) `mode_psk` adapter (handoff 10.2).
//!
//! The sender spends 32 pad bytes through the provider and passes them to HPKE as the
//! PSK, with `psk_id = circle_id || slice || offset (u32 LE)`. The receiver reads the
//! handle from `psk_id`, spends the same range, and opens. HPKE's key schedule mixes the
//! PSK into every derived key, so the message stays confidential if the KEM holds **or**
//! the pad stays secret.
//!
//! Suites: `X25519` is DHKEM(X25519, HKDF-SHA256). `XWing` is the X-Wing hybrid of
//! X25519 and ML-KEM-768, so the KEM itself is post-quantum as well. Both use
//! HKDF-SHA256 and ChaCha20-Poly1305.
//!
//! This is PSK mode. It is computationally secure for the data. It is not a one-time pad.

use cahoots_provider::{CircleId, PadError, PadProvider, PskHandle};
use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::{X25519HkdfSha256, XWing};
use hpke::{Deserializable, Kem, OpModeR, OpModeS, PskBundle, Serializable};
use rand_core::CryptoRng;
use zeroize::Zeroizing;

/// Context string bound into the provider call so a pad PSK released for HPKE is never
/// reused by another protocol.
pub const CONTEXT: &[u8] = b"hpke";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    /// DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305.
    X25519,
    /// X-Wing (X25519 + ML-KEM-768), HKDF-SHA256, ChaCha20-Poly1305.
    XWing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HpkeError {
    Pad(PadError),
    /// Malformed public key, private key, or encapsulated key.
    Key,
    Seal,
    /// Authentication failed: wrong pad, key, info, or AAD, or a tampered ciphertext.
    Open,
}

impl From<PadError> for HpkeError {
    fn from(e: PadError) -> Self {
        HpkeError::Pad(e)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    /// Where the peer finds the same 32 pad bytes. Equal to the HPKE `psk_id`.
    pub handle: PskHandle,
    /// Encapsulated key.
    pub enc: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// `circle_id || slice || offset (u32 LE)`.
pub fn psk_id(handle: &PskHandle) -> [u8; 21] {
    handle.to_bytes()
}

/// Generate a recipient keypair as `(private, public)` byte encodings.
pub fn gen_keypair(suite: Suite, rng: &mut impl CryptoRng) -> (Zeroizing<Vec<u8>>, Vec<u8>) {
    match suite {
        Suite::X25519 => {
            let (sk, pk) = X25519HkdfSha256::gen_keypair_with_rng(rng);
            (
                Zeroizing::new(sk.to_bytes().to_vec()),
                pk.to_bytes().to_vec(),
            )
        }
        Suite::XWing => {
            let (sk, pk) = XWing::gen_keypair_with_rng(rng);
            (
                Zeroizing::new(sk.to_bytes().to_vec()),
                pk.to_bytes().to_vec(),
            )
        }
    }
}

fn seal_with<K: Kem>(
    psk: &[u8],
    id: &[u8],
    pk: &[u8],
    info: &[u8],
    aad: &[u8],
    pt: &[u8],
    rng: &mut impl CryptoRng,
) -> Result<(Vec<u8>, Vec<u8>), HpkeError> {
    let pk = K::PublicKey::from_bytes(pk).map_err(|_| HpkeError::Key)?;
    let bundle = PskBundle::new(psk, id).map_err(|_| HpkeError::Seal)?;
    let (enc, ct) = hpke::single_shot_seal_with_rng::<ChaCha20Poly1305, HkdfSha256, K>(
        &OpModeS::Psk(bundle),
        &pk,
        info,
        pt,
        aad,
        rng,
    )
    .map_err(|_| HpkeError::Seal)?;
    Ok((enc.to_bytes().to_vec(), ct))
}

fn open_with<K: Kem>(
    psk: &[u8],
    id: &[u8],
    sk: &[u8],
    enc: &[u8],
    info: &[u8],
    aad: &[u8],
    ct: &[u8],
) -> Result<Vec<u8>, HpkeError> {
    let sk = K::PrivateKey::from_bytes(sk).map_err(|_| HpkeError::Key)?;
    let enc = K::EncappedKey::from_bytes(enc).map_err(|_| HpkeError::Key)?;
    let bundle = PskBundle::new(psk, id).map_err(|_| HpkeError::Open)?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, K>(
        &OpModeR::Psk(bundle),
        &sk,
        &enc,
        info,
        ct,
        aad,
    )
    .map_err(|_| HpkeError::Open)
}

/// Spend 32 pad bytes from the caller's slice of `circle` and seal `plaintext` to
/// `recipient_pk` in HPKE PSK mode.
#[allow(clippy::too_many_arguments)]
pub fn seal_to(
    provider: &mut impl PadProvider,
    circle: &CircleId,
    suite: Suite,
    recipient_pk: &[u8],
    info: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    rng: &mut impl CryptoRng,
) -> Result<Sealed, HpkeError> {
    let (handle, psk) = provider.psk(circle, CONTEXT)?;
    let id = psk_id(&handle);
    let (enc, ciphertext) = match suite {
        Suite::X25519 => {
            seal_with::<X25519HkdfSha256>(&*psk, &id, recipient_pk, info, aad, plaintext, rng)?
        }
        Suite::XWing => seal_with::<XWing>(&*psk, &id, recipient_pk, info, aad, plaintext, rng)?,
    };
    Ok(Sealed {
        handle,
        enc,
        ciphertext,
    })
}

/// Spend the peer's range named by `handle` and open the message. The range is spent
/// before the open is attempted, so a failed open can't be retried with the same bytes.
#[allow(clippy::too_many_arguments)]
pub fn open_from(
    provider: &mut impl PadProvider,
    handle: &PskHandle,
    suite: Suite,
    recipient_sk: &[u8],
    enc: &[u8],
    info: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, HpkeError> {
    let psk = provider.psk_for(handle, CONTEXT)?;
    let id = psk_id(handle);
    match suite {
        Suite::X25519 => {
            open_with::<X25519HkdfSha256>(&*psk, &id, recipient_sk, enc, info, aad, ciphertext)
        }
        Suite::XWing => open_with::<XWing>(&*psk, &id, recipient_sk, enc, info, aad, ciphertext),
    }
}

fn seal_base_with<K: Kem>(
    pk: &[u8],
    info: &[u8],
    aad: &[u8],
    pt: &[u8],
    rng: &mut impl CryptoRng,
) -> Result<(Vec<u8>, Vec<u8>), HpkeError> {
    let pk = K::PublicKey::from_bytes(pk).map_err(|_| HpkeError::Key)?;
    let (enc, ct) = hpke::single_shot_seal_with_rng::<ChaCha20Poly1305, HkdfSha256, K>(
        &OpModeS::Base,
        &pk,
        info,
        pt,
        aad,
        rng,
    )
    .map_err(|_| HpkeError::Seal)?;
    Ok((enc.to_bytes().to_vec(), ct))
}

fn open_base_with<K: Kem>(
    sk: &[u8],
    enc: &[u8],
    info: &[u8],
    aad: &[u8],
    ct: &[u8],
) -> Result<Vec<u8>, HpkeError> {
    let sk = K::PrivateKey::from_bytes(sk).map_err(|_| HpkeError::Key)?;
    let enc = K::EncappedKey::from_bytes(enc).map_err(|_| HpkeError::Key)?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, K>(
        &OpModeR::Base,
        &sk,
        &enc,
        info,
        ct,
        aad,
    )
    .map_err(|_| HpkeError::Open)
}

/// Seal `plaintext` to `recipient_pk` in HPKE base mode: no pad, no PSK. Returns
/// `(enc, ciphertext)`. For a reader who holds no pad.
pub fn seal_base(
    suite: Suite,
    recipient_pk: &[u8],
    info: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    rng: &mut impl CryptoRng,
) -> Result<(Vec<u8>, Vec<u8>), HpkeError> {
    match suite {
        Suite::X25519 => {
            seal_base_with::<X25519HkdfSha256>(recipient_pk, info, aad, plaintext, rng)
        }
        Suite::XWing => seal_base_with::<XWing>(recipient_pk, info, aad, plaintext, rng),
    }
}

/// Open a message sealed by [`seal_base`].
pub fn open_base(
    suite: Suite,
    recipient_sk: &[u8],
    enc: &[u8],
    info: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, HpkeError> {
    match suite {
        Suite::X25519 => {
            open_base_with::<X25519HkdfSha256>(recipient_sk, enc, info, aad, ciphertext)
        }
        Suite::XWing => open_base_with::<XWing>(recipient_sk, enc, info, aad, ciphertext),
    }
}
