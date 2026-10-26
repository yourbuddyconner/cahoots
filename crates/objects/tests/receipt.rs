mod common;

use cahoots_core::cose::CoseError;
use cahoots_objects::receipt::Receipt;
use cahoots_objects::ObjectError;
use common::*;

fn receipt(grant: Option<[u8; 32]>) -> Receipt {
    Receipt {
        circle_id: CIRCLE,
        seat: 1,
        slice: 3,
        offset: 64,
        len: 32,
        purpose: 0,
        grant,
        log: 120,
    }
}

#[test]
fn a_receipt_round_trips() {
    for r in [receipt(None), receipt(Some([0x55; 32]))] {
        assert_eq!(Receipt::decode(&r.encode()).unwrap(), r);
        let signed = r.sign(&seat_key(1), &[1]);
        assert_eq!(Receipt::verify(&signed, &manifest()).unwrap(), r);
    }
}

#[test]
fn a_signature_from_the_wrong_seat_fails() {
    // Seat 0's key under seat 1's kid.
    let signed = receipt(None).sign(&seat_key(0), &[1]);
    assert_eq!(
        Receipt::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::Cose(CoseError::Signature)
    );
    // Seat 0 signing honestly as itself, for a receipt that names seat 1.
    let signed = receipt(None).sign(&seat_key(0), &[0]);
    assert_eq!(
        Receipt::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::WrongKid
    );
    // A seat past the roster.
    let signed = receipt(None).sign(&seat_key(0), &[7]);
    assert_eq!(
        Receipt::verify(&signed, &manifest()).unwrap_err(),
        ObjectError::NotInRoster(7)
    );
}
