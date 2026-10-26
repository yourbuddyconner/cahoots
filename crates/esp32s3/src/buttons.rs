//! Two tactile switches to ground on the chip's pull-ups: A is left, B is right, and both
//! together are the chord.

use alloc::collections::VecDeque;
use cahoots_badge::Press;
use esp_hal::gpio::Input;
use esp_hal::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(30);
/// How long a single press waits for the other button before it counts as a single press.
const CHORD: Duration = Duration::from_millis(80);

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
    fn pressed(&mut self) -> bool {
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
    /// A single press held back for the chord window, and when it became stable.
    pending: Option<(Press, Instant)>,
    /// Gestures decided but not yet returned. One poll can decide two: a single press
    /// whose window has passed, and a chord that both buttons started in that same poll.
    ready: VecDeque<Press>,
}

impl Buttons {
    pub fn new(a: Button, b: Button) -> Self {
        Buttons {
            a,
            b,
            pending: None,
            ready: VecDeque::new(),
        }
    }

    /// The next gesture, consumed, or `Press::None`. Call it often: a press is seen only
    /// when the pins are read here.
    ///
    /// A debounced press of one button is held back for `CHORD`. If the other button's
    /// press becomes stable inside that window, the two are one `Both` and neither is
    /// reported alone. Otherwise the single press is reported once the window has passed,
    /// and from then on it can no longer become part of a chord.
    pub fn press(&mut self) -> Press {
        let mut a = self.a.pressed();
        let mut b = self.b.pressed();
        if let Some((side, since)) = self.pending {
            let (mine, other) = if side == Press::Left {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            let open = since.elapsed() < CHORD;
            if open && *other {
                self.pending = None;
                *mine = false;
                *other = false;
                self.ready.push_back(Press::Both);
            } else if !open || *mine {
                // The window has passed, or the same button was pressed again: the held
                // press stands alone, and a new press of that button starts a new window.
                self.pending = None;
                self.ready.push_back(side);
            }
        }
        match (a, b) {
            (true, true) => self.ready.push_back(Press::Both),
            (true, false) => self.pending = Some((Press::Left, Instant::now())),
            (false, true) => self.pending = Some((Press::Right, Instant::now())),
            (false, false) => {}
        }
        self.ready.pop_front().unwrap_or(Press::None)
    }
}
