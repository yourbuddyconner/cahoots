//! Two tactile switches to ground on the chip's pull-ups: A is next, B is confirm.

use esp_hal::gpio::Input;
use esp_hal::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(30);

pub struct Button {
    pin: Input<'static>,
    /// The accepted state: true while the switch counts as closed.
    down: bool,
    /// What the pin has read since `candidate_since`, if it differs from `down`.
    candidate: Option<bool>,
    candidate_since: Instant,
}

impl Button {
    pub fn new(pin: Input<'static>) -> Self {
        let down = pin.is_low();
        Button {
            pin,
            down,
            candidate: None,
            candidate_since: Instant::now(),
        }
    }

    /// True while the switch reads closed.
    pub fn is_down(&self) -> bool {
        self.pin.is_low()
    }

    /// True once per press, on the press edge. A new state is accepted only after the pin
    /// has read it for `DEBOUNCE` without interruption, so a single glitch, such as the
    /// ground bounce of an e-paper refresh on a breadboard, is not a press.
    pub fn pressed(&mut self) -> bool {
        let now = self.pin.is_low();
        if now == self.down {
            self.candidate = None;
            return false;
        }
        match self.candidate {
            Some(c) if c == now => {
                if self.candidate_since.elapsed() > DEBOUNCE {
                    self.down = now;
                    self.candidate = None;
                    return now;
                }
                false
            }
            _ => {
                self.candidate = Some(now);
                self.candidate_since = Instant::now();
                false
            }
        }
    }
}

pub struct Buttons {
    pub a: Button,
    pub b: Button,
}
