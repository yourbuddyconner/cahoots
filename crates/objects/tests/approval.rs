mod common;

use cahoots_core::cose::CoseError;
use cahoots_objects::approval::Approval;
use cahoots_objects::assurance;
use cahoots_objects::ObjectError;
use common::*;

fn approval() -> Approval {
    Approval {
        circle_id: CIRCLE,
        seat: 2,
        profile: "payment".into(),
        fields: vec![
            ("Payee".into(), "Acme Ltd".into()),
            ("Amount".into(), "120.00 EUR".into()),
            ("Account".into(), "DE89 3704".into()),
        ],
        request_digest: [0x33; 32],
        policy_id: b"policy-7".to_vec(),
        verdict: true,
        assurance: assurance::PRESENT,
        log: 42,
        nonce: [0x44; 16],
    }
}

#[test]
fn verifies_and_reports_the_fields_in_order() {
    let signed = approval().sign(&seat_key(2), &[2]);
    let got = Approval::verify(&signed, &manifest()).unwrap();
    assert_eq!(got, approval());
    let labels: Vec<&str> = got.fields.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, ["Payee", "Amount", "Account"]);
}

#[test]
fn a_tampered_field_fails() {
    let signed = approval().sign(&seat_key(2), &[2]);
    let at = signed
        .windows(6)
        .position(|w| w == b"120.00")
        .expect("the amount is in the payload");
    let mut tampered = signed.clone();
    tampered[at] = b'9';
    assert_eq!(
        Approval::verify(&tampered, &manifest()).unwrap_err(),
        ObjectError::Cose(CoseError::Signature)
    );
}

#[test]
fn assurance_above_present_pin_is_refused() {
    let mut a = approval();
    a.assurance = assurance::PRESENT_PIN;
    assert_eq!(Approval::decode(&a.encode()).unwrap(), a);
    a.assurance = 4;
    assert_eq!(
        Approval::decode(&a.encode()).unwrap_err(),
        ObjectError::Malformed
    );
    let signed = a.sign(&seat_key(2), &[2]);
    assert_eq!(
        Approval::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::Malformed
    );
}

#[test]
fn the_assurance_lattice_is_ordered() {
    assert_eq!(
        [
            assurance::CLAIMED,
            assurance::KEY,
            assurance::PRESENT,
            assurance::PRESENT_PIN
        ],
        [0, 1, 2, 3]
    );
}

#[test]
fn the_kid_must_be_the_approving_seat() {
    let signed = approval().sign(&seat_key(1), &[1]);
    assert_eq!(
        Approval::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::WrongKid
    );
}

#[test]
fn another_circle_is_refused() {
    let mut a = approval();
    a.circle_id = [1; 16];
    let signed = a.sign(&seat_key(2), &[2]);
    assert_eq!(
        Approval::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::Circle
    );
}
