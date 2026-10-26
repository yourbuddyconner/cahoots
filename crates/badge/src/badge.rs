//! [`Badge`]: the boot screen and, later, the ceremony and the idle menu.

use crate::board::{Board, Circle};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::checkwords::check_words;
use cahoots_hal::{Display, Entropy, PadStore};

pub struct Badge<B: Board> {
    board: B,
    circle: Option<Circle>,
}

impl<B: Board> Badge<B> {
    pub fn new(board: B) -> Self {
        Self {
            board,
            circle: None,
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
}
