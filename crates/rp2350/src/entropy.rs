//! The RP2350's hardware true random number generator (an Arm CryptoCell TRNG block),
//! driven through its registers: a ring oscillator sampled every `SAMPLE_COUNT` clocks,
//! with the block's own autocorrelation, continuous and von Neumann tests left on. Wrap it
//! in `cahoots_hal::health::Tested`; the firmware claims 6 bits of min-entropy per byte
//! until the source has been estimated, as the Nano does.

use crate::clock::{Duration, Instant};
use cahoots_hal::{Entropy, HealthError};
use core::sync::atomic::{AtomicU32, Ordering};
use rp235x_hal::pac::{RESETS, TRNG};

/// Clocks between ring-oscillator samples. 200 is what embassy's driver defaults to: a
/// 192-bit block in about 40 ms, with the autocorrelation test passing reliably.
const SAMPLE_COUNT: u32 = 200;
const BLOCK_BYTES: usize = 24;
/// A block normally takes about 40 ms. One that is not done by this deadline, whatever
/// the block's own tests say, means the source is unavailable, so a stuck oscillator or
/// endless test failures end in an error and not a hang.
const BLOCK_DEADLINE: Duration = Duration::from_millis(500);

/// The lowest stack pointer seen in [`Hw::fill`], which the ceremony's sealing calls from
/// deep inside the post-quantum code.
static MIN_SP: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe extern "C" {
    static mut _stack_start: u32;
}

fn note_stack_depth() {
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

pub struct Hw {
    trng: TRNG,
}

impl Hw {
    pub fn new(trng: TRNG, resets: &mut RESETS) -> Self {
        resets.reset().modify(|_, w| w.trng().clear_bit());
        while resets.reset_done().read().trng().bit_is_clear() {}
        let hw = Hw { trng };
        hw.reset();
        hw
    }

    /// Soft-reset the block and configure it. Writes during the reset are lost, so the
    /// sample count is written until it reads back, as the reference driver does.
    fn reset(&self) {
        let r = &self.trng;
        r.trng_sw_reset().write(|w| w.trng_sw_reset().set_bit());
        loop {
            r.sample_cnt1().write(|w| unsafe { w.bits(SAMPLE_COUNT) });
            if r.sample_cnt1().read().bits() == SAMPLE_COUNT {
                break;
            }
        }
        r.trng_config().write(|w| unsafe { w.rnd_src_sel().bits(0) });
        r.sample_cnt1().write(|w| unsafe { w.bits(SAMPLE_COUNT) });
        r.trng_debug_control().write(|w| {
            w.auto_correlate_bypass()
                .clear_bit()
                .trng_crngt_bypass()
                .clear_bit()
                .vnc_bypass()
                .clear_bit()
        });
    }

    fn start(&self) {
        let r = &self.trng;
        r.rng_icr().write(|w| {
            w.ehr_valid()
                .set_bit()
                .autocorr_err()
                .set_bit()
                .crngt_err()
                .set_bit()
                .vn_err()
                .set_bit()
        });
        r.rnd_source_enable().write(|w| w.rnd_src_en().set_bit());
    }

    fn stop(&self) {
        let r = &self.trng;
        r.rnd_source_enable().write(|w| w.rnd_src_en().clear_bit());
        r.rst_bits_counter().write(|w| w.rst_bits_counter().set_bit());
    }

    /// Wait for a 192-bit block. A continuous-test or von Neumann error only rejects a
    /// stretch of bits and is cleared; an autocorrelation error stops the block, which is
    /// reset and restarted.
    fn wait_block(&self) -> Result<(), HealthError> {
        let r = &self.trng;
        let start = Instant::now();
        while r.trng_valid().read().ehr_valid().bit_is_clear() {
            if start.elapsed() > BLOCK_DEADLINE {
                return Err(HealthError::SourceUnavailable);
            }
            let isr = r.rng_isr().read();
            if isr.autocorr_err().bit_is_set() {
                self.reset();
                self.start();
            } else if isr.crngt_err().bit_is_set() || isr.vn_err().bit_is_set() {
                r.rng_icr()
                    .write(|w| w.crngt_err().set_bit().vn_err().set_bit());
            }
        }
        Ok(())
    }

    /// Read a completed block. Reading the last register clears them all and starts the
    /// next block.
    fn read_block(&self, out: &mut [u8; BLOCK_BYTES]) {
        let r = &self.trng;
        let words = [
            r.ehr_data0().read().bits(),
            r.ehr_data1().read().bits(),
            r.ehr_data2().read().bits(),
            r.ehr_data3().read().bits(),
            r.ehr_data4().read().bits(),
            r.ehr_data5().read().bits(),
        ];
        for (chunk, w) in out.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&w.to_le_bytes());
        }
    }
}

impl Entropy for Hw {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        note_stack_depth();
        if buf.is_empty() {
            return Ok(());
        }
        self.start();
        let mut block = [0u8; BLOCK_BYTES];
        for chunk in buf.chunks_mut(BLOCK_BYTES) {
            if let Err(e) = self.wait_block() {
                self.stop();
                return Err(e);
            }
            self.read_block(&mut block);
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
        self.stop();
        Ok(())
    }
}
