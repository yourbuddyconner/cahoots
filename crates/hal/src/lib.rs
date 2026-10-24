//! `cahoots-hal`: the four traits every Cahoots badge target implements (handoff section 4),
//! plus the device logic written against them:
//!
//! - [`health`]: SP 800-90B continuous health tests wrapping any [`Entropy`] source.
//! - [`ring`]: the touch-link ceremony driver over any set of [`Link`]s: a pile, a ring, a pair.
//! - [`pile`]: how a card in a stack learns its seat and the count before the ceremony.
//! - [`store`]: adapts a [`PadStore`] to `cahoots_core::pad::PadStorage` with verified erase.
//! - [`ledger`]: a power-fail-safe spent-range ledger over a raw record region.
//!
//! Every target (host simulator, ESP32-S3, Dabao) runs the same code; only the trait
//! implementations differ.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod health;
pub mod ledger;
pub mod pile;
pub mod ring;
pub mod store;

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

/// Errors from raw record storage (see [`ledger::RecordStore`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    OutOfRange,
    WriteFailed,
}

/// A hardware randomness source. Wrap it in [`health::Tested`] before use.
pub trait Entropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError>;
}

/// Raw non-volatile pad storage with an explicit erase unit.
///
/// `erase(off, len)` MUST leave `[off, off+len)` reading back as zero. For ranges smaller
/// than the erase unit an implementation may program bits to zero instead of erasing a
/// whole unit. [`store::StorePad`] verifies the result by reading back.
pub trait PadStore {
    fn len(&self) -> u32;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read(&self, off: u32, buf: &mut [u8]);
    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError>;
    fn erase_unit(&self) -> u32;
}

pub trait Display {
    fn show_words(&mut self, w: &[&str; 6]);
    fn show_qr(&mut self, data: &[u8]);
    fn show_status(&mut self, s: &str);
}

/// A touch link: a byte channel over one cable or one set of contacts to one neighbour.
/// A card in a pile has two, down and up; a device in a closed ring has two, previous and next.
///
/// `recv` is non-blocking: it copies whatever bytes are available (possibly none) and
/// returns the count. Bytes arrive in order on a given hop.
pub trait Link {
    fn send(&mut self, frame: &[u8]);
    fn recv(&mut self, buf: &mut [u8]) -> usize;
    /// The detect line: true while something is plugged into this connector.
    fn peer_present(&self) -> bool;
    /// Push queued bytes toward the wire. Returns true once nothing is left queued.
    /// The ring driver calls this until it returns true before it finishes, so the last
    /// forwarded frame is never lost when a device completes first.
    fn flush(&mut self) -> bool {
        true
    }
}

/// A boxed link is a link, so a device can hold links of different kinds: a cable to one
/// neighbour and a serial port to a laptop standing in for the rest of the pile.
impl<L: Link + ?Sized> Link for alloc::boxed::Box<L> {
    fn send(&mut self, frame: &[u8]) {
        (**self).send(frame)
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        (**self).recv(buf)
    }
    fn peer_present(&self) -> bool {
        (**self).peer_present()
    }
    fn flush(&mut self) -> bool {
        (**self).flush()
    }
}
