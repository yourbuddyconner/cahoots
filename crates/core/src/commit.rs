//! Commit-then-reveal. Without it, the last member to reveal could choose a contribution
//! after seeing everyone else's and bias the pad. The member's KEM public key is bound in
//! too, so a reveal sealed to a substituted key cannot pass as the committed one. The seat
//! public key and the declared device tier are bound in as well, so nobody can change either
//! after committing.

use crate::params::CircleParams;
use crate::seat::SeatPublic;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const COMMIT_DOMAIN: &[u8] = b"cahoots/commit/v2";

pub type Commitment = [u8; 32];

/// `C_i = SHA-256("cahoots/commit/v2" || circle_params || i || kem_pk_i || seat_pk_i || tier_i || R_i)`.
pub fn commit(
    params: &CircleParams,
    index: u8,
    kem_pk: &[u8],
    seat_pk: &SeatPublic,
    tier: u8,
    contribution: &[u8],
) -> Commitment {
    let mut h = Sha256::new();
    h.update(COMMIT_DOMAIN);
    h.update(params.encode());
    h.update([index]);
    h.update(kem_pk);
    h.update(seat_pk.0);
    h.update([tier]);
    h.update(contribution);
    h.finalize().into()
}

/// Constant-time check of a revealed contribution against its commitment.
pub fn verify_commit(
    params: &CircleParams,
    index: u8,
    kem_pk: &[u8],
    seat_pk: &SeatPublic,
    tier: u8,
    contribution: &[u8],
    commitment: &Commitment,
) -> bool {
    commit(params, index, kem_pk, seat_pk, tier, contribution)
        .ct_eq(commitment)
        .into()
}
