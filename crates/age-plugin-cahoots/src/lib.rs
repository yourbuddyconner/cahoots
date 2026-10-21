//! `age-plugin-cahoots`: encrypt age files to a Cahoots circle (handoff 10.2, S1).
//!
//! - Recipient `age1cahoots1...` encodes a circle ID. Anyone in the circle can decrypt.
//! - Identity `AGE-PLUGIN-CAHOOTS-1...` also encodes a circle ID. It carries no key
//!   material. The pad lives in the provider directory named by `CAHOOTS_DIR`.
//! - Stanza: tag `cahoots`, one argument (the base64 pad handle), body = ChaCha20-Poly1305
//!   of the 16-byte file key under `HKDF(psk, handle, "age")` (handoff 10.1).
//!
//! Wrapping spends 32 pad bytes from the sender's slice. Unwrapping spends the same
//! range on the reader's copy, so a stanza can be unwrapped once per member. The label
//! set is `{postquantum}`: the file key wrap depends only on the pad, so age refuses to
//! combine it with a classical recipient that would undermine that.
//!
//! The file data itself is protected by age's ordinary STREAM construction. That is
//! computationally secure. It is not a one-time pad.

use age_core::format::{FileKey, Stanza, FILE_KEY_BYTES};
use age_core::primitives::{aead_decrypt, aead_encrypt, bech32_decode, bech32_encode};
use age_core::secrecy::ExposeSecret;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use base64::Engine;
use bech32::Hrp;
use cahoots_provider::{derive_session_key, CircleId, PadError, PadProvider, PskHandle};
use zeroize::Zeroizing;

pub const PLUGIN_NAME: &str = "cahoots";
pub const STANZA_TAG: &str = "cahoots";
pub const CONTEXT: &[u8] = b"age";
const RECIPIENT_HRP: &str = "age1cahoots";
const IDENTITY_HRP: &str = "AGE-PLUGIN-CAHOOTS-";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginError {
    /// Malformed recipient, identity, or stanza.
    Encoding,
    Pad(PadError),
    /// The stanza body did not authenticate: wrong pad or tampered file.
    Unwrap,
}

impl From<PadError> for PluginError {
    fn from(e: PadError) -> Self {
        PluginError::Pad(e)
    }
}

pub fn encode_recipient(circle: &CircleId) -> String {
    bech32_encode(Hrp::parse_unchecked(RECIPIENT_HRP), circle)
}

pub fn encode_identity(circle: &CircleId) -> String {
    bech32_encode(Hrp::parse_unchecked(IDENTITY_HRP), circle).to_uppercase()
}

fn decode_with_hrp(s: &str, hrp: &str) -> Result<CircleId, PluginError> {
    bech32_decode(
        s,
        |_| PluginError::Encoding,
        |h| {
            if h.to_lowercase() == hrp.to_lowercase() {
                Ok(())
            } else {
                Err(PluginError::Encoding)
            }
        },
        |_, bytes| circle_from_bytes(&bytes.collect::<Vec<u8>>()),
    )
}

/// Parse the raw bytes an age client hands to `add_recipient` / `add_identity`.
pub fn circle_from_bytes(bytes: &[u8]) -> Result<CircleId, PluginError> {
    <[u8; 16]>::try_from(bytes).map_err(|_| PluginError::Encoding)
}

pub fn decode_recipient(s: &str) -> Result<CircleId, PluginError> {
    decode_with_hrp(s, RECIPIENT_HRP)
}

pub fn decode_identity(s: &str) -> Result<CircleId, PluginError> {
    decode_with_hrp(s, IDENTITY_HRP)
}

fn wrap_key(psk: &[u8; 32], handle: &PskHandle) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(derive_session_key(psk, handle, CONTEXT, b"", b""))
}

/// Spend 32 pad bytes of `circle` and wrap `file_key` into a stanza.
pub fn wrap_file_key(
    provider: &mut impl PadProvider,
    circle: &CircleId,
    file_key: &FileKey,
) -> Result<Stanza, PluginError> {
    let (handle, psk) = provider.psk(circle, CONTEXT)?;
    let key = wrap_key(&psk, &handle);
    Ok(Stanza {
        tag: STANZA_TAG.into(),
        args: vec![B64.encode(handle.to_bytes())],
        body: aead_encrypt(&key, file_key.expose_secret()),
    })
}

/// Parse the handle from a `cahoots` stanza. `None` for stanzas of other types.
pub fn stanza_handle(stanza: &Stanza) -> Result<Option<PskHandle>, PluginError> {
    if stanza.tag != STANZA_TAG {
        return Ok(None);
    }
    let [arg] = stanza.args.as_slice() else {
        return Err(PluginError::Encoding);
    };
    let bytes = B64.decode(arg).map_err(|_| PluginError::Encoding)?;
    PskHandle::from_bytes(&bytes)
        .map(Some)
        .ok_or(PluginError::Encoding)
}

/// Try to unwrap a stanza with any of `identities` (circle IDs we hold a pad for).
/// Returns `Ok(None)` when the stanza is not ours. Spends the peer's range on success
/// or on a tag failure, never on a stanza we ignore.
pub fn unwrap_stanza(
    provider: &mut impl PadProvider,
    identities: &[CircleId],
    stanza: &Stanza,
) -> Result<Option<FileKey>, PluginError> {
    let Some(handle) = stanza_handle(stanza)? else {
        return Ok(None);
    };
    if !identities.contains(&handle.circle_id) {
        return Ok(None);
    }
    if stanza.body.len() != FILE_KEY_BYTES + 16 {
        return Err(PluginError::Encoding);
    }
    let psk = provider.psk_for(&handle, CONTEXT)?;
    let key = wrap_key(&psk, &handle);
    let pt = Zeroizing::new(
        aead_decrypt(&key, FILE_KEY_BYTES, &stanza.body).map_err(|_| PluginError::Unwrap)?,
    );
    let fk = FileKey::init_with_mut(|fk| fk.copy_from_slice(&pt));
    Ok(Some(fk))
}
