//! `cahoots-hal`: the four traits every Cahoots badge target implements (handoff section 4).
//!
//! Status: traits only. The device ceremony driver that runs the ring protocol over `Link`
//! lands in M4a together with the `host` simulator.
#![no_std]

/// Raised when a TRNG health test fails. The ceremony MUST abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthError {
    RepetitionCount,
    AdaptiveProportion,
    SourceUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseError {
    OutOfRange,
    Unaligned,
    VerifyFailed,
}

/// A hardware randomness source. Implementations MUST run the SP 800-90B health tests.
pub trait Entropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError>;
}

/// Raw non-volatile pad storage with an explicit erase unit.
pub trait PadStore {
    fn read(&self, off: u32, buf: &mut [u8]);
    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError>;
    fn erase_unit(&self) -> u32;
}

pub trait Display {
    fn show_words(&mut self, w: &[&str; 6]);
    fn show_qr(&mut self, data: &[u8]);
    fn show_status(&mut self, s: &str);
}

/// The touch link: a framed byte channel to the next device in the ring.
pub trait Link {
    fn send(&mut self, frame: &[u8]);
    fn recv(&mut self, buf: &mut [u8]) -> usize;
    fn peer_present(&self) -> bool;
}
