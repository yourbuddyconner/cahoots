//! A 200 by 200 one-bit image: what a badge draws on its screen.
//!
//! Row-major, [`Frame::ROW_BYTES`] bytes per row, most significant bit first: pixel `x` of
//! a row is bit `7 - (x % 8)` of byte `x / 8`. A set bit is black.

/// One full screen. A board's [`crate::Display::draw`] shows it with a full refresh.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub bits: [u8; 5000],
}

impl Frame {
    pub const WIDTH: usize = 200;
    pub const HEIGHT: usize = 200;
    pub const ROW_BYTES: usize = 25;

    /// An all-white frame.
    pub fn blank() -> Self {
        Frame { bits: [0; 5000] }
    }

    /// True if the pixel at `(x, y)` is black. Outside the frame reads as white.
    pub fn get(&self, x: usize, y: usize) -> bool {
        match Self::index(x, y) {
            Some((i, mask)) => self.bits[i] & mask != 0,
            None => false,
        }
    }

    /// Set the pixel at `(x, y)` black or white. Outside the frame does nothing.
    pub fn set(&mut self, x: usize, y: usize, black: bool) {
        if let Some((i, mask)) = Self::index(x, y) {
            if black {
                self.bits[i] |= mask;
            } else {
                self.bits[i] &= !mask;
            }
        }
    }

    fn index(x: usize, y: usize) -> Option<(usize, u8)> {
        if x >= Self::WIDTH || y >= Self::HEIGHT {
            return None;
        }
        Some((y * Self::ROW_BYTES + x / 8, 0x80 >> (x % 8)))
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::blank()
    }
}

impl core::fmt::Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let black = self.bits.iter().map(|b| b.count_ones()).sum::<u32>();
        write!(f, "Frame {{ {black} black pixels }}")
    }
}
