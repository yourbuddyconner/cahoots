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
//! Keys derived from the root are HMAC-SHA256 under it: a chat key per generation, and seal
//! keys named by a label. The chat key per root epoch is the old form, kept so old history
//! still opens.

use crate::circle_id::CircleId;
use alloc::vec::Vec;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const ROOT_INFO: &[u8] = b"cahoots/root/v1";
pub const CHAT_LABEL: &[u8] = b"chat";
/// The label of a chat generation key: `chat_gen_key(root, circle_id, generation_id)`.
pub const CHAT_GEN_LABEL: &[u8] = b"chat/gen";
/// The domain of generation 0's id: `gen0_id(circle_id)`.
pub const GEN0_DOMAIN: &[u8] = b"cahoots/chat/gen0";
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

/// The chat key of root epoch `epoch`: `HMAC-SHA256(root, "chat" || circle_id || epoch as u64
/// BE)`. Root epochs are the old form: no new message uses one, and this key opens old history.
pub fn chat_key(root: &[u8; 32], circle_id: &CircleId, epoch: u64) -> [u8; 32] {
    hmac(root, &[CHAT_LABEL, circle_id, &epoch.to_be_bytes()])
}

/// The id of a circle's chat generation 0: `SHA-256("cahoots/chat/gen0" || circle_id)`. Every
/// member derives it, so a circle starts in generation 0 without anyone sending anything. Later
/// generations have random ids.
pub fn gen0_id(circle_id: &CircleId) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(GEN0_DOMAIN);
    h.update(circle_id);
    h.finalize().into()
}

/// The key of a chat generation:
/// `HMAC-SHA256(root, "chat/gen" || circle_id || generation_id)`. Epoch keys derive from it,
/// so a device that holds this key and not the root can chat in this generation only.
pub fn chat_gen_key(
    root: &[u8; 32],
    circle_id: &CircleId,
    generation_id: &[u8; 32],
) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(hmac(root, &[CHAT_GEN_LABEL, circle_id, generation_id]))
}

/// A seal key named by `label`: `HMAC-SHA256(root, label || circle_id)`.
pub fn seal_key(root: &[u8; 32], label: &[u8], circle_id: &CircleId) -> [u8; 32] {
    hmac(root, &[label, circle_id])
}
