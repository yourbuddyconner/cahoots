mod common;

use cahoots_objects::fact::{fact_nonce, Fact};
use cahoots_objects::ObjectError;
use common::*;
use sha2::{Digest, Sha256};

fn fact() -> Fact {
    Fact {
        circle_id: CIRCLE,
        seat: 0,
        module: "sht4x/temperature".into(),
        reading: vec![0x01, 0x02, 0x03],
        counter: 9,
        nonce: fact_nonce(&CIRCLE, 77),
        log: 77,
        attested: true,
        evidence: vec![0xee; 8],
    }
}

#[test]
fn fact_nonce_is_the_domain_hash() {
    let mut h = Sha256::new();
    h.update(b"cahoots/fact-nonce/v1");
    h.update(CIRCLE);
    h.update(77u64.to_le_bytes());
    let want: [u8; 32] = h.finalize().into();
    assert_eq!(fact_nonce(&CIRCLE, 77), want);
}

#[test]
fn nonce_is_recomputed_and_a_wrong_one_fails() {
    let signed = fact().sign(&seat_key(0), &[0]);
    assert_eq!(Fact::verify(&signed, &manifest()).unwrap(), fact());

    let mut f = fact();
    f.nonce = fact_nonce(&CIRCLE, 78);
    let signed = f.sign(&seat_key(0), &[0]);
    assert_eq!(
        Fact::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::Nonce
    );
}

#[test]
fn attested_false_requires_empty_evidence() {
    let mut f = fact();
    f.attested = false;
    assert_eq!(
        Fact::decode(&f.encode()).unwrap_err(),
        ObjectError::Malformed
    );
    f.evidence.clear();
    assert_eq!(Fact::decode(&f.encode()).unwrap(), f);
}
