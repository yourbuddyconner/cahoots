//! `circle_id = first 16 bytes of SHA-256("cahoots/circle-id/v1" || PAD)` (handoff 3.4 step 7).

use sha2::{Digest, Sha256};

pub const CIRCLE_ID_DOMAIN: &[u8] = b"cahoots/circle-id/v1";

pub type CircleId = [u8; 16];

pub fn circle_id(pad: &[u8]) -> CircleId {
    let mut h = Sha256::new();
    h.update(CIRCLE_ID_DOMAIN);
    h.update(pad);
    let d = h.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&d[..16]);
    id
}
