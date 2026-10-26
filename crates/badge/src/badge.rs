//! [`Badge`]: the boot screen, the pile and the ceremony, and later the idle menu.

use crate::board::{Board, Circle};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::ceremony::CeremonyResult;
use cahoots_core::checkwords::check_words;
use cahoots_core::params::CircleParams;
use cahoots_hal::pile::{Pile, PileError};
use cahoots_hal::ring::{RingCeremony, Step as RingStep};
use cahoots_hal::{Display, Entropy, Link, PadStore};

/// What one [`Badge::step`] did, or where the pass stands after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// No pass is under way.
    Idle,
    /// In the pile, waiting for a seat and the count.
    InPile,
    /// The ceremony is running.
    InCeremony,
    /// The ceremony is done and the badge asks: B keeps the circle, A discards it.
    Prompt,
    /// The circle is saved. The pass is over.
    Kept,
    /// The result is dropped. The pass is over.
    Discarded,
    /// The pass failed with this message. The next step starts a new pass.
    Failed(String),
}

/// Where the pass is between two calls to [`Badge::step`].
enum State<L: Link> {
    Idle,
    Pile {
        pile: Pile<L>,
        shown: Option<u8>,
    },
    Ceremony {
        /// Boxed: the ceremony is a few kilobytes, and the state moves on every step.
        ring: Box<RingCeremony<L>>,
        last: (u8, u8),
        params: CircleParams,
        seat: u8,
    },
    /// The ceremony is complete. Push the last forwarded frames out before the prompt.
    Flush {
        links: Vec<L>,
        result: CeremonyResult,
        seat: u8,
    },
    Prompt {
        result: CeremonyResult,
        seat: u8,
    },
}

pub struct Badge<B: Board> {
    board: B,
    circle: Option<Circle>,
    state: State<B::Link>,
}

impl<B: Board> Badge<B> {
    pub fn new(board: B) -> Self {
        Self {
            board,
            circle: None,
            state: State::Idle,
        }
    }

    pub fn board(&mut self) -> &mut B {
        &mut self.board
    }

    pub fn circle(&self) -> Option<&Circle> {
        self.circle.as_ref()
    }

    /// The self-test screen, the A-held wipe, and the words of a stored circle.
    pub fn boot(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        let mut sample = vec![0u8; 4096];
        lines.push(match self.board.entropy().fill(&mut sample) {
            Ok(()) => "trng: health tests pass".into(),
            Err(e) => format!("trng: FAILED {e:?}"),
        });
        lines.extend(self.board.selftest());
        let circle = self.board.load_circle();
        lines.push(match &circle {
            Some(c) => format!(
                "circle: {} seat {} of {}",
                c.params.name, c.seat, c.params.n
            ),
            None => "circle: none yet".into(),
        });
        for l in &lines {
            self.board.log(l);
        }
        self.board.display().show_status(&lines.join("\n"));
        self.board.delay_ms(2500);

        // A held through the self-test forgets the circle: record, ledger and pad.
        self.circle = if circle.is_some() && self.board.a_down() {
            let pad_len = circle.as_ref().map_or(0, |c| c.params.pad_len);
            match self.board.wipe(pad_len) {
                Ok(()) => {
                    self.board.log("A held at boot: circle forgotten");
                    self.board
                        .display()
                        .show_status("circle forgotten\nrelease A");
                    while self.board.a_down() {
                        self.board.delay_ms(10);
                    }
                    None
                }
                Err(_) => {
                    self.board.log("wipe failed");
                    circle
                }
            }
        } else {
            circle
        };

        if let Some(c) = &self.circle {
            let pad = self.board.open_pad(c.params.pad_len);
            let mut whole = vec![0u8; c.params.pad_len as usize];
            pad.read(0, &mut whole);
            let words = check_words(&c.params, &whole);
            self.board.display().show_words(&words);
            self.board
                .log(&format!("words from flash: {}", words.join(" ")));
        }
    }

    /// One pass: find the pile, run the ceremony, keep or discard the result. Steps until
    /// the pass ends, with a 1 ms delay between steps.
    pub fn run_once(&mut self) -> Result<(), String> {
        loop {
            match self.step() {
                Step::Kept | Step::Discarded => return Ok(()),
                Step::Failed(e) => return Err(e),
                _ => self.board.delay_ms(1),
            }
        }
    }

    /// Advance the pass by one poll. Never blocks, so two badges can run on one thread.
    pub fn step(&mut self) -> Step {
        let state = core::mem::replace(&mut self.state, State::Idle);
        match self.advance(state) {
            Ok((next, step)) => {
                self.state = next;
                step
            }
            Err(e) => Step::Failed(e),
        }
    }

    fn advance(&mut self, state: State<B::Link>) -> Result<(State<B::Link>, Step), String> {
        match state {
            State::Idle => {
                self.board
                    .display()
                    .show_status("plug into a pile. B on top starts");
                let (down, up) = self.board.links();
                let (name, pad_len) = self.board.proposal();
                let pile = Pile::new(down, up, &name, pad_len, false);
                Ok((State::Pile { pile, shown: None }, Step::InPile))
            }
            State::Pile {
                mut pile,
                mut shown,
            } => {
                if let Some(ready) = pile.poll().map_err(|e| format!("pile: {e:?}"))? {
                    let params = ready.params.clone();
                    let seat = ready.seat;
                    self.board.log(&format!(
                        "pile counted: seat {seat} of {}, circle {}",
                        params.n, params.name
                    ));
                    self.board
                        .display()
                        .show_status(&format!("seat {seat} of {}: {}", params.n, params.name));
                    let mut ring = pile
                        .into_ceremony(ready, self.board.entropy())
                        .map_err(|e| format!("ceremony: {e:?}"))?;
                    self.board
                        .display()
                        .show_status("ceremony: waiting for the others");
                    ring.start();
                    let state = State::Ceremony {
                        ring: Box::new(ring),
                        last: (0, 0),
                        params,
                        seat,
                    };
                    return Ok((state, Step::InCeremony));
                }
                if pile.seat() != shown {
                    shown = pile.seat();
                    if let Some(s) = shown {
                        self.board.log(&format!("seat {s}"));
                        self.board
                            .display()
                            .show_status(&format!("seat {s}. B when the pile is whole"));
                    }
                }
                if self.board.b_pressed() {
                    match pile.start() {
                        Ok(()) => {
                            self.board.log("B: counting the pile");
                            self.board.display().show_status("counting the pile...");
                        }
                        Err(PileError::NoSeat) => {
                            self.board.display().show_status("nothing below yet")
                        }
                        Err(e) => self.board.log(&format!("start: {e:?}")),
                    }
                }
                let _ = self.board.a_pressed();
                Ok((State::Pile { pile, shown }, Step::InPile))
            }
            State::Ceremony {
                mut ring,
                mut last,
                params,
                seat,
            } => {
                let step = ring.poll().map_err(|e| {
                    self.board.display().show_status("ceremony: ABORTED");
                    format!("ceremony: {e:?}")
                })?;
                let now = (ring.commitments_held(), ring.reveals_held());
                if now != last {
                    last = now;
                    self.board.display().show_status(&format!(
                        "commits {}/{}  reveals {}/{}",
                        now.0, params.n, now.1, params.n
                    ));
                }
                if step != RingStep::Complete {
                    let state = State::Ceremony {
                        ring,
                        last,
                        params,
                        seat,
                    };
                    return Ok((state, Step::InCeremony));
                }
                let (result, links) = (*ring).finish().map_err(|e| format!("ceremony: {e:?}"))?;
                Ok((
                    State::Flush {
                        links,
                        result,
                        seat,
                    },
                    Step::InCeremony,
                ))
            }
            State::Flush {
                mut links,
                result,
                seat,
            } => {
                // Our neighbours may still need the frames we forwarded last. Keep reading
                // (and dropping: the ceremony is over) while we flush, so a neighbour that
                // is also flushing toward us is never blocked.
                let mut done = true;
                let mut buf = [0u8; 256];
                for link in links.iter_mut() {
                    if !link.flush() {
                        done = false;
                    }
                    while link.recv(&mut buf) > 0 {}
                }
                if !done {
                    let state = State::Flush {
                        links,
                        result,
                        seat,
                    };
                    return Ok((state, Step::InCeremony));
                }
                drop(links);
                self.board.display().show_words(&result.check_words);
                self.board
                    .log(&format!("words: {}", result.check_words.join(" ")));
                self.board
                    .display()
                    .show_status("B keep this circle . A discard");
                Ok((State::Prompt { result, seat }, Step::Prompt))
            }
            State::Prompt { result, seat } => {
                if self.board.b_pressed() {
                    self.board.display().show_status("saving...");
                    self.board
                        .write_pad(&result.pad)
                        .map_err(|_| "flash: pad write failed")?;
                    self.board
                        .records(true)
                        .map_err(|_| "flash: ledger erase failed")?;
                    let circle = Circle {
                        params: result.params.clone(),
                        seat,
                        circle_id: result.circle_id,
                    };
                    self.board
                        .save_circle(&circle)
                        .map_err(|_| "flash: circle record failed")?;
                    self.board
                        .log(&format!("saved: {} seat {seat}", circle.params.name));
                    self.board
                        .display()
                        .show_status("saved. read the words aloud");
                    self.circle = Some(circle);
                    return Ok((State::Idle, Step::Kept));
                }
                if self.board.a_pressed() {
                    drop(result);
                    self.board.log("discarded");
                    self.board.display().show_status("discarded");
                    return Ok((State::Idle, Step::Discarded));
                }
                Ok((State::Prompt { result, seat }, Step::Prompt))
            }
        }
    }
}
