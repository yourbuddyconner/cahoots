//! The circle root: one 32-byte secret every member's card holds after the ceremony, the same
//! on every card (spec §2).
//!
//! Every sealed reveal encrypts its contribution under a fresh content key and wraps that key
//! to every other member (see [`crate::reveal`]). After the reveals each member holds all `n`
//! content keys: its own from sealing, every other one from opening. The root is
//!
//! ```text
//! root = HKDF-SHA256(ikm = content_key_0 || ... || content_key_{n-1}, salt = circle_id,
//!                    info = "cahoots/root/v1")
//! ```
//!
//! The pairwise X-Wing secrets are not used: each is known to two members only.
//!
//! Keys derived from the root are HMAC-SHA256 under it: a chat key per epoch, and seal keys
//! named by a label.

use crate::circle_id::CircleId;
use alloc::vec::Vec;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

pub const ROOT_INFO: &[u8] = b"cahoots/root/v1";
pub const CHAT_LABEL: &[u8] = b"chat";
/// The label of the key a card seals its circle record under: `seal_key(root, SEAL_CARD, id)`.
pub const SEAL_CARD: &[u8] = b"seal/card";

/// The root from every seat's reveal content key, in seat order.
pub fn derive(secrets: &[[u8; 32]], circle_id: &CircleId) -> [u8; 32] {
    let mut ikm = Zeroizing::new(Vec::with_capacity(secrets.len() * 32));
    for s in secrets {
        ikm.extend_from_slice(s);
    }
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(circle_id), &ikm)
        .expand(ROOT_INFO, &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

fn hmac(root: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(root).expect("HMAC takes any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// The chat key for `epoch`: `HMAC-SHA256(root, "chat" || circle_id || epoch as u64 BE)`.
pub fn chat_key(root: &[u8; 32], circle_id: &CircleId, epoch: u64) -> [u8; 32] {
    hmac(root, &[CHAT_LABEL, circle_id, &epoch.to_be_bytes()])
}

/// A seal key named by `label`: `HMAC-SHA256(root, label || circle_id)`.
pub fn seal_key(root: &[u8; 32], label: &[u8], circle_id: &CircleId) -> [u8; 32] {
    hmac(root, &[label, circle_id])
}
