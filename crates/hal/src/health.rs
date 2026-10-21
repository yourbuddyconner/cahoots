//! NIST SP 800-90B section 4.4 continuous health tests on raw TRNG output:
//! the Repetition Count Test (4.4.1) and the Adaptive Proportion Test (4.4.2).
//!
//! Samples are bytes. Cutoffs use `alpha = 2^-30` and a window `W = 512`, as the
//! standard recommends for non-binary sources:
//!
//! - RCT cutoff `C = 1 + ceil(-log2(alpha) / H) = 1 + ceil(30 / H)`.
//! - APT cutoff `= 1 + CRITBINOM(W, 2^-H, 1 - alpha)`, precomputed below.
//!
//! `H` is the claimed min-entropy per byte. Claim conservatively. A source that fails
//! either test is dead for the rest of the session: [`Tested`] zeroes the output buffer and
//! returns the error on every later call.

use crate::{Entropy, HealthError};

const WINDOW: u32 = 512;
/// APT cutoffs for H = 1..=8, W = 512, alpha = 2^-30.
const APT_CUTOFFS: [u32; 8] = [325, 190, 114, 71, 46, 31, 22, 16];

#[derive(Debug, Clone)]
pub struct HealthTests {
    rct_cutoff: u32,
    apt_cutoff: u32,
    last: Option<u8>,
    run: u32,
    apt_ref: Option<u8>,
    apt_count: u32,
    apt_pos: u32,
    failed: Option<HealthError>,
}

impl HealthTests {
    pub fn new(rct_cutoff: u32, apt_cutoff: u32) -> Self {
        HealthTests {
            rct_cutoff,
            apt_cutoff,
            last: None,
            run: 0,
            apt_ref: None,
            apt_count: 0,
            apt_pos: 0,
            failed: None,
        }
    }

    /// Cutoffs for a claimed min-entropy of `h` bits per byte, `1..=8`.
    pub fn for_min_entropy(h: u8) -> Self {
        let h = h.clamp(1, 8) as u32;
        let rct = 1 + 30u32.div_ceil(h);
        Self::new(rct, APT_CUTOFFS[(h - 1) as usize])
    }

    pub fn rct_cutoff(&self) -> u32 {
        self.rct_cutoff
    }
    pub fn apt_cutoff(&self) -> u32 {
        self.apt_cutoff
    }

    /// Feed raw samples. State persists across calls. Once failed, always fails.
    pub fn check(&mut self, data: &[u8]) -> Result<(), HealthError> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        for &b in data {
            // Repetition Count Test.
            if self.last == Some(b) {
                self.run += 1;
            } else {
                self.last = Some(b);
                self.run = 1;
            }
            if self.run >= self.rct_cutoff {
                self.failed = Some(HealthError::RepetitionCount);
                return Err(HealthError::RepetitionCount);
            }
            // Adaptive Proportion Test.
            match self.apt_ref {
                None => {
                    self.apt_ref = Some(b);
                    self.apt_count = 1;
                    self.apt_pos = 1;
                }
                Some(r) => {
                    if b == r {
                        self.apt_count += 1;
                    }
                    self.apt_pos += 1;
                    if self.apt_count >= self.apt_cutoff {
                        self.failed = Some(HealthError::AdaptiveProportion);
                        return Err(HealthError::AdaptiveProportion);
                    }
                    if self.apt_pos == WINDOW {
                        self.apt_ref = None;
                    }
                }
            }
        }
        Ok(())
    }
}

/// An [`Entropy`] source with continuous health tests. Fails closed.
pub struct Tested<E: Entropy> {
    inner: E,
    tests: HealthTests,
}

impl<E: Entropy> Tested<E> {
    pub fn new(inner: E, tests: HealthTests) -> Self {
        Tested { inner, tests }
    }
    pub fn into_inner(self) -> E {
        self.inner
    }
}

impl<E: Entropy> Entropy for Tested<E> {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        let r = self.inner.fill(buf).and_then(|_| self.tests.check(buf));
        if r.is_err() {
            buf.fill(0);
        }
        r
    }
}
