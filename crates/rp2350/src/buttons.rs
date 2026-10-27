//! Two tactile switches to ground on the chip's pull-ups: A is left, B is right, and both
//! together are the chord. The same debounce and chord window as the Nano firmware.

use crate::clock::{Duration, Instant};
use alloc::collections::VecDeque;
use cahoots_badge::Press;

const DEBOUNCE: Duration = Duration::from_millis(30);
/// How long a single press waits for the other button before it counts as a single press.
const CHORD: Duration = Duration::from_millis(80);

/// The level of one GPIO, read from SIO whatever function the pin is in.
pub fn gpio_high(n: u8) -> bool {
    // SAFETY: a read of a read-only register.
    let sio = unsafe { &*rp235x_hal::pac::SIO::ptr() };
    sio.gpio_in().read().bits() & (1 << n) != 0
}

pub struct Button {
    /// The GPIO number. The pin itself is configured as a pull-up input by `main` and kept
    /// there; the level is read from SIO.
    gpio: u8,
    /// The accepted state: true while the switch counts as closed.
    down: bool,
    /// What the pin has read since `candidate_since`, if it differs from `down`.
    candidate: Option<bool>,
    candidate_since: Instant,
}

impl Button {
    pub fn new(gpio: u8) -> Self {
        let down = !gpio_high(gpio);
        Button {
            gpio,
            down,
            candidate: None,
            candidate_since: Instant::now(),
        }
    }

    /// True while the switch reads closed.
    pub fn is_down(&self) -> bool {
        !gpio_high(self.gpio)
    }

    /// On the press edge, once per press: when the press was first seen. A new state is
    /// accepted only after the pin has read it for `DEBOUNCE` without interruption.
    fn pressed(&mut self) -> Option<Instant> {
        let now = self.is_down();
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

pub struct Buttons {
    pub a: Button,
    pub b: Button,
    /// A single press held back for the chord window, and when it was first seen.
    pending: Option<(Press, Instant)>,
    /// Gestures decided but not yet returned.
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

    /// Drop every gesture decided or held back so far.
    pub fn clear(&mut self) {
        self.pending = None;
        self.ready.clear();
    }

    /// The next gesture, consumed, or `Press::None`. Each press is timed from when it was
    /// first seen; a debounced press of one button is held back for `CHORD`, and if the
    /// other button's press was first seen within that window the two are one `Both`.
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
                if o.gap(start) <= CHORD {
                    self.pending = None;
                    *mine = None;
                    *other = None;
                    self.ready.push_back(Press::Both);
                } else {
                    self.pending = None;
                    self.ready.push_back(side);
                }
            } else if mine.is_some() {
                self.pending = None;
                self.ready.push_back(side);
            } else if start.elapsed() > CHORD {
                let other_btn = if left { &self.b } else { &self.a };
                let other_coming = other_btn
                    .press_seen_since()
                    .is_some_and(|o| o.gap(start) <= CHORD);
                if !other_coming {
                    self.pending = None;
                    self.ready.push_back(side);
                }
            }
        }
        match (a, b) {
            (Some(x), Some(y)) if x.gap(y) <= CHORD => self.ready.push_back(Press::Both),
            (Some(x), Some(y)) => {
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
