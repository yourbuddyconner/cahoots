use cahoots_core::commit::{commit, verify_commit, COMMIT_DOMAIN};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::{SeatKey, SeatPublic};
use sha2::{Digest, Sha256};

fn p() -> CircleParams {
    CircleParams::new(2, 64, "t", false).unwrap()
}

fn seat(i: u8) -> SeatPublic {
    SeatKey::from_seed(&[i; 32]).unwrap().public()
}

#[test]
fn commit_domain_is_version_2() {
    assert_eq!(COMMIT_DOMAIN, b"cahoots/commit/v2");
}

#[test]
fn commitment_is_sha256_over_domain_params_index_keys_tier_and_contribution() {
    let r = vec![7u8; 64];
    let pk = vec![9u8; 1216];
    let sp = seat(3);
    let mut h = Sha256::new();
    h.update(COMMIT_DOMAIN);
    h.update(p().encode());
    h.update([1u8]);
    h.update(&pk);
    h.update(sp.0);
    h.update([2u8]);
    h.update(&r);
    let expected: [u8; 32] = h.finalize().into();
    assert_eq!(commit(&p(), 1, &pk, &sp, 2, &r), expected);
}

#[test]
fn commitment_binds_seat_key_and_tier() {
    let r = vec![7u8; 64];
    let pk = vec![9u8; 1216];
    let base = commit(&p(), 1, &pk, &seat(3), 2, &r);
    assert_ne!(base, commit(&p(), 1, &pk, &seat(4), 2, &r), "seat key");
    assert_ne!(base, commit(&p(), 1, &pk, &seat(3), 1, &r), "tier");
    assert!(verify_commit(&p(), 1, &pk, &seat(3), 2, &r, &base));
    assert!(!verify_commit(&p(), 1, &pk, &seat(4), 2, &r, &base));
    assert!(!verify_commit(&p(), 1, &pk, &seat(3), 3, &r, &base));
}

#[test]
fn verify_rejects_any_change() {
    let r = vec![1u8; 64];
    let pk = vec![2u8; 1216];
    let sp = seat(1);
    let c = commit(&p(), 0, &pk, &sp, 0, &r);
    assert!(verify_commit(&p(), 0, &pk, &sp, 0, &r, &c));
    let mut r2 = r.clone();
    r2[10] ^= 1;
    assert!(!verify_commit(&p(), 0, &pk, &sp, 0, &r2, &c));
    assert!(!verify_commit(&p(), 1, &pk, &sp, 0, &r, &c));
    let mut pk2 = pk.clone();
    pk2[0] ^= 1;
    assert!(
        !verify_commit(&p(), 0, &pk2, &sp, 0, &r, &c),
        "a substituted key must not verify"
    );
}
