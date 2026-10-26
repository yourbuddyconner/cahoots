//! The [`Board`] trait: everything the badge needs from the hardware under it.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::checkwords::WORDLIST;
use cahoots_core::circle_id::CircleId;
use cahoots_core::params::CircleParams;
use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{Display, Entropy, Link, PadStore};

/// A board-level storage or link failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardError {
    Storage,
    Link,
}

/// The circle a badge belongs to: its parameters, this card's seat, the circle id and the
/// six check words the ceremony produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Circle {
    pub params: CircleParams,
    pub seat: u8,
    pub circle_id: CircleId,
    /// The check words as indices into `WORDLIST`. Stored at keep time, because spending
    /// erases pad bytes and words worked out from the pad later would differ.
    pub words: [u16; 6],
}

impl Circle {
    /// The six check words, looked up in `WORDLIST`.
    pub fn check_words(&self) -> [&'static str; 6] {
        self.words
            .map(|i| WORDLIST[usize::from(i) % WORDLIST.len()])
    }
}

/// The indices into `WORDLIST` of six check words, or `None` if one is not in the list.
pub fn word_indices(words: &[&str; 6]) -> Option<[u16; 6]> {
    let mut out = [0u16; 6];
    for (slot, w) in out.iter_mut().zip(words) {
        *slot = WORDLIST.iter().position(|x| x == w)? as u16;
    }
    Some(out)
}

/// One board: drivers plus a pin table. The badge logic is generic over this trait.
pub trait Board {
    type Entropy: Entropy;
    type Pad: PadStore;
    type Records: RecordStore;
    type Display: Display;
    type Link: Link;

    /// The random source. The board wraps it in `cahoots_hal::health::Tested`.
    fn entropy(&mut self) -> &mut Self::Entropy;
    fn display(&mut self) -> &mut Self::Display;
    /// Button A went down since the last call (edge, consumed).
    fn a_pressed(&mut self) -> bool;
    /// Button B went down since the last call (edge, consumed).
    fn b_pressed(&mut self) -> bool;
    /// Button A is held right now (level).
    fn a_down(&self) -> bool;
    fn delay_ms(&mut self, ms: u32);
    /// The two links, `(down, up)`. Returns owned links, constructed fresh on each call
    /// (a board that builds its links once must hand them out from an `Option`, and the
    /// badge calls this once per ceremony and once when it enters the idle menu).
    fn links(&mut self) -> (Self::Link, Self::Link);
    fn load_circle(&self) -> Option<Circle>;
    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError>;
    /// Forget the circle: the record, the ledger and the `pad_len` bytes of pad.
    fn wipe(&mut self, pad_len: u32) -> Result<(), BoardError>;
    /// Store a new pad and return a handle to it.
    fn write_pad(&mut self, pad: &[u8]) -> Result<Self::Pad, BoardError>;
    /// Open the stored pad of `len` bytes.
    fn open_pad(&mut self, len: u32) -> Self::Pad;
    /// The ledger's record region. `fresh` clears it first.
    fn records(&mut self, fresh: bool) -> Result<Self::Records, BoardError>;
    /// `(name, pad_len)` if this card is the bottom of the pile and proposes a circle.
    fn proposal(&self) -> (String, u32);
    fn tier(&self) -> u8;
    /// Board-specific self-test lines. The badge adds the common ones.
    fn selftest(&mut self) -> Vec<String>;
    fn log(&mut self, line: &str);
}
