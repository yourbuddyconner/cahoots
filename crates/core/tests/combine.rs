use cahoots_core::combine::{combine_into, LengthMismatch};

#[test]
fn xors_contribution_into_pad() {
    let mut pad = [0b1010u8; 4];
    combine_into(&mut pad, &[0b0110u8; 4]).unwrap();
    assert_eq!(pad, [0b1100u8; 4]);
}

#[test]
fn rejects_length_mismatch() {
    let mut pad = [0u8; 4];
    assert_eq!(combine_into(&mut pad, &[0u8; 3]), Err(LengthMismatch));
}
