//! Two tactile switches to ground on the chip's pull-ups: A is next, B is confirm.

use esp_hal::gpio::Input;
use esp_hal::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(30);

pub struct Button {
    pin: Input<'static>,
    down: bool,
    since: Instant,
}

impl Button {
    pub fn new(pin: Input<'static>) -> Self {
        let down = pin.is_low();
        Button {
            pin,
            down,
            since: Instant::now(),
        }
    }

    /// True while the switch reads closed.
    pub fn is_down(&self) -> bool {
        self.pin.is_low()
    }

    /// True once per press, on the press edge.
    pub fn pressed(&mut self) -> bool {
        let now = self.pin.is_low();
        if now != self.down && self.since.elapsed() > DEBOUNCE {
            self.down = now;
            self.since = Instant::now();
            return now;
        }
        false
    }
}

pub struct Buttons {
    pub a: Button,
    pub b: Button,
}
