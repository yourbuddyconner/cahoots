//! Session-key derivation (handoff 10.1).
//!
//! ```text
//! session_key = HKDF-SHA256(
//!     salt = "cahoots/psk/v1" || circle_id || slice || offset (u32 LE) || context,
//!     ikm  = classical_secret || pq_secret || psk,
//!     info = "cahoots/session-key/v1")
//! ```
//!
//! The session stays confidential if the key exchange holds **or** the pad stays secret.
//! `context` names the protocol so a PSK released for one can't be reused in another.
//! PSK mode is computationally secure for the data. It is not a one-time pad.

use crate::PskHandle;
use hkdf::Hkdf;
use sha2::Sha256;

pub const PSK_SALT_DOMAIN: &[u8] = b"cahoots/psk/v1";
pub const SESSION_KEY_INFO: &[u8] = b"cahoots/session-key/v1";

pub fn derive_session_key(
    psk: &[u8; 32],
    handle: &PskHandle,
    context: &[u8],
    classical_secret: &[u8],
    pq_secret: &[u8],
) -> [u8; 32] {
    let mut salt = Vec::with_capacity(PSK_SALT_DOMAIN.len() + 21 + context.len());
    salt.extend_from_slice(PSK_SALT_DOMAIN);
    salt.extend_from_slice(&handle.to_bytes());
    salt.extend_from_slice(context);
    let mut ikm = zeroize::Zeroizing::new(Vec::with_capacity(
        classical_secret.len() + pq_secret.len() + 32,
    ));
    ikm.extend_from_slice(classical_secret);
    ikm.extend_from_slice(pq_secret);
    ikm.extend_from_slice(psk);
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(SESSION_KEY_INFO, &mut okm)
        .expect("32 bytes is a valid HKDF output length");
    okm
}
