//! The ESP32-S3's hardware random number generator with the ADC noise source running.
//!
//! The RNG is only a true random source while an entropy source feeds it, so `main` keeps a
//! `TrngSource` alive for the whole run and this type reads through `Trng`. Wrap it in
//! `cahoots_hal::health::Tested` before use; the firmware claims 6 bits of min-entropy per
//! byte until the source has been estimated.

use cahoots_hal::{Entropy, HealthError};
use core::sync::atomic::{AtomicU32, Ordering};
use esp_hal::rng::Trng;

/// The lowest stack pointer seen in [`Hw::fill`], which the ceremony's sealing calls from
/// deep inside the post-quantum code. `stack_high_water` turns it into bytes used.
static MIN_SP: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe extern "C" {
    static _stack_start: u32;
}

fn note_stack_depth() {
    // The address of a local is within a few words of the stack pointer, which is close
    // enough for a high-water mark.
    let marker = 0u8;
    let sp = core::ptr::addr_of!(marker) as u32;
    MIN_SP.fetch_min(sp, Ordering::Relaxed);
}

/// Bytes of stack used at the deepest point seen so far, or 0 before any sample.
pub fn stack_high_water() -> u32 {
    let min = MIN_SP.load(Ordering::Relaxed);
    if min == u32::MAX {
        return 0;
    }
    let top = core::ptr::addr_of!(_stack_start) as u32;
    top.saturating_sub(min)
}

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
        note_stack_depth();
        self.0.read(buf);
        Ok(())
    }
}
