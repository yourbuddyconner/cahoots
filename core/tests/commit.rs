use cahoots_core::commit::{commit, verify_commit};
use cahoots_core::params::CircleParams;
use sha2::{Digest, Sha256};

fn p() -> CircleParams {
    CircleParams::new(2, 1024, "c", false).unwrap()
}

#[test]
fn commit_is_sha256_of_domain_params_index_contribution() {
    let r = [7u8; 1024];
    let mut h = Sha256::new();
    h.update(b"cahoots/commit/v1");
    h.update(p().encode());
    h.update([1u8]);
    h.update(r);
    let expected: [u8; 32] = h.finalize().into();
    assert_eq!(commit(&p(), 1, &r), expected);
}

#[test]
fn verify_detects_changed_contribution() {
    let r = [7u8; 1024];
    let c = commit(&p(), 0, &r);
    assert!(verify_commit(&p(), 0, &r, &c));
    let mut r2 = r;
    r2[500] ^= 1;
    assert!(!verify_commit(&p(), 0, &r2, &c));
    assert!(!verify_commit(&p(), 1, &r, &c));
}
