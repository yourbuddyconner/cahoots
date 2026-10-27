//! The 1.54" e-paper as the badge's `Display`: the badge renders each screen into a
//! [`Frame`] and `draw` copies it into the panel's buffer and shows it with a full refresh.
//! The same driver and the same full-refresh policy as the Nano firmware.

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
    delay: Delay,
}

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
            delay,
        })
    }

    fn full(&mut self) {
        crate::store::trace("display: full refresh");
        let _ = self.epd.clear_frame(&mut self.spi, &mut self.delay);
        let _ = self
            .epd
            .set_lut(&mut self.spi, &mut self.delay, Some(RefreshLut::Full));
        let _ = self
            .epd
            .update_frame(&mut self.spi, &self.buf, &mut self.delay);
        let _ = self.epd.display_frame(&mut self.spi, &mut self.delay);
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
        self.full();
    }
}
