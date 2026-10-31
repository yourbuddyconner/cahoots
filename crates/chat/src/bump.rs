//! A bump: a seat's signed word that the circle moves to a new chat generation.
//!
//! The payload of a `Bump` message is `generation_id (32) || COSE_Sign1`. The COSE_Sign1 is
//! detached, signed with kid `[seat]`, over `"cahoots/chat/bump/v1" || circle_id ||
//! generation_id`. A receiver verifies it against the manifest, and only then calls
//! `ChatSession::bump`. A device signing under a chain of its own is for later: this module
//! checks seat signatures only.

use crate::primitives::ChatError;
use alloc::vec::Vec;
use cahoots_core::circle_id::CircleId;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatSigner;

/// The domain at the head of the bytes a bump signs.
pub const BUMP_DOMAIN: &[u8] = b"cahoots/chat/bump/v1";

/// `"cahoots/chat/bump/v1" || circle_id || generation_id`.
fn signed_bytes(circle_id: &CircleId, generation_id: &[u8; 32]) -> Vec<u8> {
    let mut m = Vec::with_capacity(BUMP_DOMAIN.len() + 16 + 32);
    m.extend_from_slice(BUMP_DOMAIN);
    m.extend_from_slice(circle_id);
    m.extend_from_slice(generation_id);
    m
}

/// Sign a bump to `generation_id` as `seat`, and return the payload for
/// `ChatSession::send_bump`: `generation_id || COSE_Sign1`.
pub fn sign_bump<S: SeatSigner>(
    signer: S,
    seat: u8,
    circle_id: &CircleId,
    generation_id: &[u8; 32],
) -> Result<Vec<u8>, ChatError> {
    let sig = cose::sign1(
        signer,
        &[seat],
        &signed_bytes(circle_id, generation_id),
        b"",
        true,
    );
    let mut out = Vec::with_capacity(32 + sig.len());
    out.extend_from_slice(generation_id);
    out.extend_from_slice(&sig);
    Ok(out)
}

/// Check a bump's payload against the manifest of `circle_id`. Returns the generation id it
/// names and the seat that signed it. `BadSignature` when it is malformed, its kid is not a
/// seat of the manifest, or the seat's key did not sign it. `WrongCircle` when the manifest
/// is another circle's.
pub fn verify_bump(
    payload: &[u8],
    circle_id: &CircleId,
    manifest: &Manifest,
) -> Result<([u8; 32], u8), ChatError> {
    if &manifest.circle_id != circle_id {
        return Err(ChatError::WrongCircle);
    }
    if payload.len() < 32 {
        return Err(ChatError::BadSignature);
    }
    let (id, sig) = payload.split_at(32);
    let generation_id: [u8; 32] = id.try_into().map_err(|_| ChatError::BadSignature)?;
    let kid = cose::kid1(sig).map_err(|_| ChatError::BadSignature)?;
    let [seat] = kid[..] else {
        return Err(ChatError::BadSignature);
    };
    let key = manifest.seat_key(seat).ok_or(ChatError::BadSignature)?;
    cose::verify1(
        key,
        sig,
        Some(&signed_bytes(circle_id, &generation_id)),
        b"",
    )
    .map_err(|_| ChatError::BadSignature)?;
    Ok((generation_id, seat))
}
