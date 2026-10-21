//! Commit-then-reveal. Without it, the last member to reveal could choose a contribution
//! after seeing everyone else's and bias the pad. The member's KEM public key is bound in
//! too, so a reveal sealed to a substituted key cannot pass as the committed one.

use crate::params::CircleParams;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const COMMIT_DOMAIN: &[u8] = b"cahoots/commit/v1";

pub type Commitment = [u8; 32];

/// `C_i = SHA-256("cahoots/commit/v1" || circle_params || i || pk_i || R_i)`.
pub fn commit(
    params: &CircleParams,
    index: u8,
    public_key: &[u8],
    contribution: &[u8],
) -> Commitment {
    let mut h = Sha256::new();
    h.update(COMMIT_DOMAIN);
    h.update(params.encode());
    h.update([index]);
    h.update(public_key);
    h.update(contribution);
    h.finalize().into()
}

/// Constant-time check of a revealed contribution against its commitment.
pub fn verify_commit(
    params: &CircleParams,
    index: u8,
    public_key: &[u8],
    contribution: &[u8],
    commitment: &Commitment,
) -> bool {
    commit(params, index, public_key, contribution)
        .ct_eq(commitment)
        .into()
}
