//! A monotonic clock for the whole firmware, read from timer 0, so the buttons and the
//! links can time things without carrying the timer around.

use core::cell::RefCell;
use critical_section::Mutex;
use rp235x_hal::timer::{CopyableTimer0, Timer};

static TIMER: Mutex<RefCell<Option<Timer<CopyableTimer0>>>> = Mutex::new(RefCell::new(None));

pub fn init(t: Timer<CopyableTimer0>) {
    critical_section::with(|cs| *TIMER.borrow_ref_mut(cs) = Some(t));
}

/// Microseconds since boot.
pub fn now_us() -> u64 {
    critical_section::with(|cs| {
        TIMER
            .borrow_ref(cs)
            .map(|t| t.get_counter().ticks())
            .unwrap_or(0)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

impl Instant {
    pub fn now() -> Self {
        Instant(now_us())
    }
    pub fn elapsed(self) -> Duration {
        Duration(now_us().saturating_sub(self.0))
    }
    /// The time between two instants, whichever comes first.
    pub fn gap(self, other: Instant) -> Duration {
        Duration(self.0.abs_diff(other.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration(u64);

impl Duration {
    pub const fn from_micros(us: u64) -> Self {
        Duration(us)
    }
    pub const fn from_millis(ms: u64) -> Self {
        Duration(ms * 1000)
    }
    pub const fn from_secs(s: u64) -> Self {
        Duration(s * 1_000_000)
    }
}
