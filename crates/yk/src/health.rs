//! Health checks on YubiKey output (handoff 3.3.1).
//!
//! The card conditions its output internally, so the raw-noise tests of SP 800-90B do
//! not apply. We run the Repetition Count Test to catch a stuck or failed device, and
//! reject any all-zero or repeated 32-byte block.

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthCheck {
    /// More than `rct_cutoff()` identical consecutive bytes.
    RepetitionCount,
    /// A 32-byte block of all zeros.
    ZeroBlock,
    /// A 32-byte block seen earlier in the same read.
    RepeatedBlock,
}

/// Repetition Count Test cutoff: `C = 1 + ceil(-log2(alpha) / H)`.
///
/// We treat each byte as a sample with `H = 8` bits of entropy (the card's output is
/// conditioned, so this is the nominal rate) and pick `alpha = 2^-30`, so `C = 5`.
/// With 262144 samples the chance of a false alarm per full read is about 2^-14.
pub const fn rct_cutoff() -> u32 {
    1 + 30_u32.div_ceil(8)
}

pub const BLOCK: usize = 32;

/// Streaming health state. Call `check` on every chunk in order.
#[derive(Debug, Default)]
pub struct Health {
    last: Option<u8>,
    run: u32,
    seen: HashSet<[u8; BLOCK]>,
    partial: Vec<u8>,
}

impl Health {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&mut self, chunk: &[u8]) -> Result<(), HealthCheck> {
        self.partial.extend_from_slice(chunk);
        while self.partial.len() >= BLOCK {
            let mut block = [0u8; BLOCK];
            block.copy_from_slice(&self.partial[..BLOCK]);
            self.partial.drain(..BLOCK);
            if block == [0u8; BLOCK] {
                return Err(HealthCheck::ZeroBlock);
            }
            if !self.seen.insert(block) {
                return Err(HealthCheck::RepeatedBlock);
            }
        }
        for &b in chunk {
            if self.last == Some(b) {
                self.run += 1;
                if self.run >= rct_cutoff() {
                    return Err(HealthCheck::RepetitionCount);
                }
            } else {
                self.last = Some(b);
                self.run = 1;
            }
        }
        Ok(())
    }
}

/// Chi-square statistic of the byte histogram against uniform (255 degrees of freedom).
/// A quick `ent`-style sanity check, not a certification. For 64 KiB of good output,
/// values near 255 are expected and anything over about 400 deserves a look.
pub fn chi_square_bytes(data: &[u8]) -> f64 {
    let mut hist = [0u64; 256];
    for &b in data {
        hist[usize::from(b)] += 1;
    }
    let expected = data.len() as f64 / 256.0;
    hist.iter()
        .map(|&o| {
            let d = o as f64 - expected;
            d * d / expected
        })
        .sum()
}
