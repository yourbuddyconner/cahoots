//! The 1.54" e-paper as the badge's `Display`: the badge renders each screen into a
//! [`Frame`] and `draw` copies it into the panel's buffer and shows it. A small change, such
//! as the caret moving down a list or a page counter, is a quick update (the panel's partial
//! lookup table: a fraction of a second, no flash, against the image the panel already
//! shows); a big change, or every tenth quick update, is a full refresh, which clears the
//! ghosting quick updates leave behind. The Nano firmware still refreshes in full every time.

use cahoots_hal::{Display, Frame};
use embedded_hal_bus::spi::ExclusiveDevice;
use epd_waveshare::epd1in54_v2::Epd1in54;
use epd_waveshare::prelude::*;
use rp235x_hal::gpio::bank0::{Gpio16, Gpio17, Gpio18, Gpio19, Gpio20, Gpio21};
use rp235x_hal::gpio::{FunctionSio, FunctionSpi, Pin, PullDown, SioInput, SioOutput};
use rp235x_hal::pac::SPI0;
use rp235x_hal::spi::{Enabled, Spi};
use rp235x_hal::timer::{CopyableTimer0, Timer};

pub type SpiPins = (
    Pin<Gpio19, FunctionSpi, PullDown>,
    Pin<Gpio18, FunctionSpi, PullDown>,
);
pub type Cs = Pin<Gpio17, FunctionSio<SioOutput>, PullDown>;
pub type Dc = Pin<Gpio16, FunctionSio<SioOutput>, PullDown>;
pub type Rst = Pin<Gpio20, FunctionSio<SioOutput>, PullDown>;
pub type Busy = Pin<Gpio21, FunctionSio<SioInput>, PullDown>;
type Delay = Timer<CopyableTimer0>;
type Bus = ExclusiveDevice<Spi<Enabled, SPI0, SpiPins>, Cs, Delay>;
type Panel = Epd1in54<Bus, Busy, Dc, Rst, Delay>;

pub struct Epd {
    spi: Bus,
    epd: Panel,
    /// The image in the panel's own layout: the same rows of 25 bytes, most significant bit
    /// first, as a `Frame`, but a set bit is white.
    buf: [u8; 5000],
    /// What the panel shows, in the same layout; `None` before the first refresh.
    shown: Option<[u8; 5000]>,
    /// The lookup table the panel was last given.
    lut: RefreshLut,
    /// Quick updates since the last full refresh.
    quick_streak: u8,
    delay: Delay,
}

/// A change confined to a band of at most this many of the 200 rows is drawn with a quick
/// update: the caret moving between entries of a list, a count going up. A page turn changes
/// rows from the header to the footer and gets the full refresh, which a quick update draws
/// badly.
const QUICK_MAX_ROWS: usize = 70;
/// Quick updates in a row before a full refresh clears the ghosting.
const QUICK_MAX_STREAK: u8 = 10;

impl Epd {
    pub fn new(
        spi: Spi<Enabled, SPI0, SpiPins>,
        cs: Cs,
        dc: Dc,
        rst: Rst,
        busy: Busy,
        mut delay: Delay,
    ) -> Result<Self, ()> {
        let mut bus = ExclusiveDevice::new(spi, cs, delay).map_err(|_| ())?;
        let epd = Epd1in54::new(&mut bus, busy, dc, rst, &mut delay, None).map_err(|_| ())?;
        Ok(Epd {
            spi: bus,
            epd,
            buf: [0xFF; 5000],
            shown: None,
            lut: RefreshLut::Full,
            quick_streak: 0,
            delay,
        })
    }

    /// Give the panel `lut` if it does not have it: setting the quick table runs an
    /// activation of its own, so it is not done on every draw.
    fn use_lut(&mut self, lut: RefreshLut) {
        if self.lut != lut {
            let _ = self.epd.set_lut(&mut self.spi, &mut self.delay, Some(lut));
            self.lut = lut;
        }
    }

    fn full(&mut self) {
        crate::store::trace("display: full refresh");
        self.use_lut(RefreshLut::Full);
        let _ = self
            .epd
            .update_frame(&mut self.spi, &self.buf, &mut self.delay);
        let _ = self.epd.display_frame(&mut self.spi, &mut self.delay);
        self.quick_streak = 0;
    }

    /// A quick update: the whole image is written, and the panel changes only the pixels
    /// that differ from what it shows, without the flash. The controller keeps the image it
    /// showed as the base for the next one.
    fn quick(&mut self) {
        crate::store::trace("display: quick update");
        self.use_lut(RefreshLut::Quick);
        let _ = self
            .epd
            .update_frame(&mut self.spi, &self.buf, &mut self.delay);
        let _ = self.epd.display_frame(&mut self.spi, &mut self.delay);
        self.quick_streak += 1;
    }
}

impl Display for Epd {
    /// The frame's pixels map one to one onto the panel's; only the bit polarity is
    /// opposite, so each byte is inverted.
    fn draw(&mut self, frame: &Frame) {
        crate::store::trace("display: draw");
        for (out, &b) in self.buf.iter_mut().zip(frame.bits.iter()) {
            *out = !b;
        }
        // The band of rows that changed: the first and the last row with a byte that differs.
        let band = match &self.shown {
            Some(s) => {
                let mut rows = s
                    .iter()
                    .zip(self.buf.iter())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .map(|(i, _)| i / 25);
                match (rows.next(), rows.next_back()) {
                    (Some(first), Some(last)) => Some(last - first + 1),
                    _ => None,
                }
            }
            None => Some(usize::MAX),
        };
        let Some(band) = band else {
            return;
        };
        if band <= QUICK_MAX_ROWS && self.quick_streak < QUICK_MAX_STREAK {
            self.quick();
        } else {
            self.full();
        }
        self.shown = Some(self.buf);
    }
}
