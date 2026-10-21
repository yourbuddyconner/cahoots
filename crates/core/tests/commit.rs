use cahoots_core::commit::{commit, verify_commit, COMMIT_DOMAIN};
use cahoots_core::params::CircleParams;
use sha2::{Digest, Sha256};

fn p() -> CircleParams {
    CircleParams::new(2, 64, "t", false).unwrap()
}

#[test]
fn commitment_is_sha256_over_domain_params_index_key_and_contribution() {
    let r = vec![7u8; 64];
    let pk = vec![9u8; 1216];
    let mut h = Sha256::new();
    h.update(COMMIT_DOMAIN);
    h.update(p().encode());
    h.update([1u8]);
    h.update(&pk);
    h.update(&r);
    let expected: [u8; 32] = h.finalize().into();
    assert_eq!(commit(&p(), 1, &pk, &r), expected);
}

#[test]
fn verify_rejects_any_change() {
    let r = vec![1u8; 64];
    let pk = vec![2u8; 1216];
    let c = commit(&p(), 0, &pk, &r);
    assert!(verify_commit(&p(), 0, &pk, &r, &c));
    let mut r2 = r.clone();
    r2[10] ^= 1;
    assert!(!verify_commit(&p(), 0, &pk, &r2, &c));
    assert!(!verify_commit(&p(), 1, &pk, &r, &c));
    let mut pk2 = pk.clone();
    pk2[0] ^= 1;
    assert!(
        !verify_commit(&p(), 0, &pk2, &r, &c),
        "a substituted key must not verify"
    );
}
