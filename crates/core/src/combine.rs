//! `PAD = R_1 XOR R_2 XOR ... XOR R_n` (handoff 3.4 step 4).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LengthMismatch;

/// XOR `contribution` into `pad` in place.
pub fn combine_into(pad: &mut [u8], contribution: &[u8]) -> Result<(), LengthMismatch> {
    if pad.len() != contribution.len() {
        return Err(LengthMismatch);
    }
    for (p, c) in pad.iter_mut().zip(contribution) {
        *p ^= c;
    }
    Ok(())
}
