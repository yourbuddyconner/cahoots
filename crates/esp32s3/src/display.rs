//! The 1.54" e-paper as the badge's `Display`: the badge renders each screen into a
//! [`Frame`] and `draw` copies it into the panel's buffer and shows it with a full refresh.
//!
//! SPI runs at 1 MHz: the module's level shifter on jumper wires drops a 10 MHz stream.

use cahoots_hal::{Display, Frame};
use embedded_hal_bus::spi::ExclusiveDevice;
use epd_waveshare::epd1in54_v2::Epd1in54;
use epd_waveshare::prelude::*;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, Output};
use esp_hal::spi::master::Spi;
use esp_hal::Blocking;

type Bus = ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>;
type Panel = Epd1in54<Bus, Input<'static>, Output<'static>, Output<'static>, Delay>;

pub struct Epd {
    spi: Bus,
    epd: Panel,
    /// The image in the panel's own layout: the same rows of 25 bytes, most significant bit
    /// first, as a `Frame`, but a set bit is white (epd-waveshare's `Color::White`).
    buf: [u8; 5000],
    delay: Delay,
}

impl Epd {
    pub fn new(
        spi: Spi<'static, Blocking>,
        cs: Output<'static>,
        dc: Output<'static>,
        rst: Output<'static>,
        busy: Input<'static>,
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
        // The panel keeps two images: the one to show and the previous one, which quick
        // partial refreshes compare against. The driver writes only the first, so after a
        // full refresh the previous image could be whatever was shown before, and a partial
        // refresh would bring it back. Clearing both first leaves the previous image blank.
        // Every refresh is a full one for now, because the driver cannot write that
        // previous image: slower and it flashes, but the screen only shows what the badge
        // drew.
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
    /// The frame's pixels map one to one onto the panel's (no rotation); only the bit
    /// polarity is opposite, so each byte is inverted.
    fn draw(&mut self, frame: &Frame) {
        crate::store::trace("display: draw");
        for (out, &b) in self.buf.iter_mut().zip(frame.bits.iter()) {
            *out = !b;
        }
        self.full();
    }
}
