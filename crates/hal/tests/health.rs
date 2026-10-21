use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::{Entropy, HealthError};

/// Deterministic test source: a fixed byte sequence.
struct Seq(Vec<u8>, usize);
impl Entropy for Seq {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        for b in buf.iter_mut() {
            *b = self.0[self.1 % self.0.len()];
            self.1 += 1;
        }
        Ok(())
    }
}

fn xorshift(n: usize) -> Vec<u8> {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 56) as u8
        })
        .collect()
}

#[test]
fn cutoffs_follow_sp800_90b_for_byte_samples() {
    // alpha = 2^-30, W = 512. RCT C = 1 + ceil(30 / H). APT cutoff = 1 + CRITBINOM(512, 2^-H, 1 - alpha).
    let h7 = HealthTests::for_min_entropy(7);
    assert_eq!(h7.rct_cutoff(), 6);
    assert_eq!(h7.apt_cutoff(), 22);
    let h8 = HealthTests::for_min_entropy(8);
    assert_eq!(h8.rct_cutoff(), 5);
    assert_eq!(h8.apt_cutoff(), 16);
    let h1 = HealthTests::for_min_entropy(1);
    assert_eq!(h1.rct_cutoff(), 31);
    assert_eq!(h1.apt_cutoff(), 325);
}

#[test]
fn good_source_passes() {
    let mut t = HealthTests::for_min_entropy(7);
    assert_eq!(t.check(&xorshift(64 * 1024)), Ok(()));
}

#[test]
fn stuck_source_fails_repetition_count() {
    let mut t = HealthTests::for_min_entropy(7);
    let mut data = xorshift(1000);
    data.extend_from_slice(&[0x42; 6]);
    assert_eq!(t.check(&data), Err(HealthError::RepetitionCount));
    // Five repeats are allowed at H=7 (cutoff 6), six are not.
    let mut t = HealthTests::for_min_entropy(7);
    let mut data = xorshift(1000);
    data.extend_from_slice(&[0x42; 5]);
    data.extend_from_slice(&xorshift(100));
    assert_eq!(t.check(&data), Ok(()));
}

#[test]
fn biased_source_fails_adaptive_proportion() {
    // One value appears 22 times in a 512 window with no run longer than the RCT cutoff.
    let mut t = HealthTests::for_min_entropy(7);
    let good = xorshift(2048);
    let mut data = good.clone();
    // The APT reference value is the first sample of the window.
    data[0] = 0x77;
    for i in 0..21 {
        data[100 + i * 3] = 0x77;
    }
    assert_eq!(t.check(&data), Err(HealthError::AdaptiveProportion));
}

#[test]
fn state_carries_across_calls() {
    let mut t = HealthTests::for_min_entropy(7);
    assert_eq!(t.check(&[0x11; 3]), Ok(()));
    assert_eq!(t.check(&[0x11; 3]), Err(HealthError::RepetitionCount));
}

#[test]
fn tested_wrapper_fails_closed_and_zeroes_output() {
    let mut src = Tested::new(Seq(vec![0xAA], 0), HealthTests::for_min_entropy(7));
    let mut buf = [1u8; 64];
    assert_eq!(src.fill(&mut buf), Err(HealthError::RepetitionCount));
    assert_eq!(buf, [0u8; 64], "failed output must not leak");
    // Once failed, it stays failed.
    assert_eq!(src.fill(&mut buf), Err(HealthError::RepetitionCount));

    let mut ok = Tested::new(Seq(xorshift(4096), 0), HealthTests::for_min_entropy(7));
    let mut buf = [0u8; 4096];
    assert_eq!(ok.fill(&mut buf), Ok(()));
    assert_eq!(buf.to_vec(), xorshift(4096));
}
