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

    /// On the press edge, once per press: when the press was first seen. A new state is
    /// accepted only after the pin has read it for `DEBOUNCE` without interruption, so a
    /// single glitch, such as the ground bounce of an e-paper refresh on a breadboard, is
    /// not a press.
    fn pressed(&mut self) -> Option<Instant> {
        let now = self.pin.is_low();
        if now == self.down {
            self.candidate = None;
            return None;
        }
        match self.candidate {
            Some(c) if c == now => {
                if self.candidate_since.elapsed() > DEBOUNCE {
                    self.down = now;
                    self.candidate = None;
                    return now.then_some(self.candidate_since);
                }
                None
            }
            _ => {
                self.candidate = Some(now);
                self.candidate_since = Instant::now();
                None
            }
        }
    }

    /// When a press now being debounced was first seen, if one is.
    fn press_seen_since(&self) -> Option<Instant> {
        (!self.down && self.candidate == Some(true)).then_some(self.candidate_since)
    }
}

/// The time between two instants, whichever comes first.
fn gap(x: Instant, y: Instant) -> Duration {
    if x > y {
        x - y
    } else {
        y - x
    }
}

pub struct Buttons {
    pub a: Button,
    pub b: Button,
    /// A single press held back for the chord window, and when it was first seen.
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

    /// Drop every gesture decided or held back so far. Presses already under way are
    /// still debounced and reported once they are stable.
    pub fn clear(&mut self) {
        self.pending = None;
        self.ready.clear();
    }

    /// The next gesture, consumed, or `Press::None`. A press is seen only when the pins
    /// are read here, so call it often.
    ///
    /// Each press is timed from when it was first seen, not from when its debounce ended,
    /// so the window does not depend on how often this is called. A debounced press of one
    /// button is held back; if the other button's press was first seen within `CHORD` of
    /// it, the two are one `Both` and neither is reported alone. Otherwise the single press
    /// is reported once `CHORD` has passed and the other button has no press under way that
    /// started inside the window. Once reported, it can no longer become part of a chord.
    pub fn press(&mut self) -> Press {
        let mut a = self.a.pressed();
        let mut b = self.b.pressed();
        if let Some((side, start)) = self.pending {
            let left = side == Press::Left;
            let (mine, other) = if left {
                (&mut a, &mut b)
            } else {
                (&mut b, &mut a)
            };
            if let Some(o) = *other {
                if gap(o, start) <= CHORD {
                    // The chord: both presses are used up.
                    self.pending = None;
                    *mine = None;
                    *other = None;
                    self.ready.push_back(Press::Both);
                } else {
                    // Too late for a chord: the held press stands alone, and the other
                    // starts its own window below.
                    self.pending = None;
                    self.ready.push_back(side);
                }
            } else if mine.is_some() {
                // The same button again: the held press stands alone, the new one starts
                // its own window below.
                self.pending = None;
                self.ready.push_back(side);
            } else if start.elapsed() > CHORD {
                let other_btn = if left { &self.b } else { &self.a };
                let other_coming = other_btn
                    .press_seen_since()
                    .is_some_and(|o| gap(o, start) <= CHORD);
                if !other_coming {
                    self.pending = None;
                    self.ready.push_back(side);
                }
            }
        }
        match (a, b) {
            (Some(x), Some(y)) if gap(x, y) <= CHORD => self.ready.push_back(Press::Both),
            (Some(x), Some(y)) => {
                // Both became stable in this poll but started too far apart: the earlier
                // one's window is over, the later one starts its own.
                let (first, second, later) = if x < y {
                    (Press::Left, Press::Right, y)
                } else {
                    (Press::Right, Press::Left, x)
                };
                self.ready.push_back(first);
                self.pending = Some((second, later));
            }
            (Some(x), None) => self.pending = Some((Press::Left, x)),
            (None, Some(y)) => self.pending = Some((Press::Right, y)),
            (None, None) => {}
        }
        self.ready.pop_front().unwrap_or(Press::None)
    }
}
