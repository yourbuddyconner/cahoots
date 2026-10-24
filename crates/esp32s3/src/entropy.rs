//! The ESP32-S3's hardware random number generator with the ADC noise source running.
//!
//! The RNG is only a true random source while an entropy source feeds it, so `main` keeps a
//! `TrngSource` alive for the whole run and this type reads through `Trng`. Wrap it in
//! `cahoots_hal::health::Tested` before use; the firmware claims 6 bits of min-entropy per
//! byte until the source has been estimated.

use cahoots_hal::{Entropy, HealthError};
use esp_hal::rng::Trng;

pub struct Hw(Trng);

impl Hw {
    pub fn new() -> Result<Self, HealthError> {
        Trng::try_new()
            .map(Hw)
            .map_err(|_| HealthError::SourceUnavailable)
    }
}

impl Entropy for Hw {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        self.0.read(buf);
        Ok(())
    }
}
