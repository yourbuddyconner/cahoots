//! The 1.54" e-paper as the badge's `Display`: the words screen in the layout the bring-up
//! settled on, the status line by partial refresh of the footer band, and a screen of lines
//! for a status of more than one line (the self-test).
//!
//! SPI runs at 1 MHz: the module's level shifter on jumper wires drops a 10 MHz stream.
//! `show_qr` draws the bytes as text for now; nothing in the ceremony calls it yet.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_hal::Display;
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10, FONT_8X13, FONT_9X15_BOLD};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_hal_bus::spi::ExclusiveDevice;
use epd_waveshare::color::Color;
use epd_waveshare::epd1in54_v2::{Display1in54, Epd1in54};
use epd_waveshare::prelude::*;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, Output};
use esp_hal::spi::master::Spi;
use esp_hal::Blocking;

type Bus = ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>;
type Panel = Epd1in54<Bus, Input<'static>, Output<'static>, Output<'static>, Delay>;

pub const W: u32 = 200;
const ROW_BYTES: usize = 25;
/// The footer band the status line lives in.
const BAND_TOP: u32 = 168;

pub struct Epd {
    spi: Bus,
    epd: Panel,
    fb: Display1in54,
    delay: Delay,
    pub circle: String,
    pub label: &'static str,
    /// Quick partial refreshes since the last full one; see `band`.
    partials: u32,
}

impl Epd {
    pub fn new(
        spi: Spi<'static, Blocking>,
        cs: Output<'static>,
        dc: Output<'static>,
        rst: Output<'static>,
        busy: Input<'static>,
        mut delay: Delay,
        label: &'static str,
    ) -> Result<Self, ()> {
        let mut bus = ExclusiveDevice::new(spi, cs, delay).map_err(|_| ())?;
        let epd = Epd1in54::new(&mut bus, busy, dc, rst, &mut delay, None).map_err(|_| ())?;
        let mut fb = Display1in54::default();
        fb.set_rotation(DisplayRotation::Rotate0);
        let _ = fb.clear(Color::White);
        Ok(Epd {
            spi: bus,
            epd,
            fb,
            delay,
            circle: String::new(),
            label,
            partials: 0,
        })
    }

    fn full(&mut self) {
        crate::store::trace("display: full refresh");
        // The panel keeps two images: the one to show and the previous one, which quick
        // partial refreshes compare against. The driver writes only the first, so after a
        // full refresh the previous image could be whatever was shown before, and a partial
        // refresh would bring it back. Clearing both first leaves the previous image blank.
        let _ = self.epd.clear_frame(&mut self.spi, &mut self.delay);
        let _ = self
            .epd
            .set_lut(&mut self.spi, &mut self.delay, Some(RefreshLut::Full));
        let _ = self
            .epd
            .update_frame(&mut self.spi, self.fb.buffer(), &mut self.delay);
        let _ = self.epd.display_frame(&mut self.spi, &mut self.delay);
    }

    /// A quick partial refresh of the rows `y0..y1`. Quick refreshes leave ghosts of the
    /// previous text, so every third one is replaced by a full refresh of the whole frame,
    /// which is slower and flashes but wipes the panel clean.
    fn band(&mut self, y0: u32, y1: u32) {
        crate::store::trace("display: band refresh");
        self.partials += 1;
        if self.partials % 3 == 0 {
            self.full();
            return;
        }
        let _ = self
            .epd
            .set_lut(&mut self.spi, &mut self.delay, Some(RefreshLut::Quick));
        let buf = &self.fb.buffer()[y0 as usize * ROW_BYTES..y1 as usize * ROW_BYTES];
        let _ =
            self.epd
                .update_partial_frame(&mut self.spi, &mut self.delay, buf, 0, y0, W, y1 - y0);
        let _ = self.epd.display_frame(&mut self.spi, &mut self.delay);
    }

    fn text(&mut self, x: i32, y: i32, s: &str, font: &MonoFont<'static>, align: Alignment) {
        let style = MonoTextStyle::new(font, Color::Black);
        let ts = TextStyleBuilder::new()
            .baseline(Baseline::Alphabetic)
            .alignment(align)
            .build();
        let _ = Text::with_text_style(s, Point::new(x, y), style, ts).draw(&mut self.fb);
    }

    fn rule(&mut self, y: i32) {
        let _ = Rectangle::new(Point::new(8, y), Size::new(184, 1))
            .into_styled(PrimitiveStyle::with_fill(Color::Black))
            .draw(&mut self.fb);
    }

    fn fill(&mut self, x: i32, y: i32, w: u32, h: u32, c: Color) {
        let _ = Rectangle::new(Point::new(x, y), Size::new(w, h))
            .into_styled(PrimitiveStyle::with_fill(c))
            .draw(&mut self.fb);
    }

    fn header(&mut self) {
        let circle = self.circle.clone();
        self.text(8, 19, &circle, &FONT_9X15_BOLD, Alignment::Left);
        self.text(192, 19, self.label, &FONT_6X10, Alignment::Right);
        self.rule(26);
    }

    /// The status line at the bottom: up to two lines of a font a person can read from
    /// a normal distance, wrapped at word boundaries. Anything past the second line is cut.
    fn footer(&mut self, s: &str) {
        const COLS: usize = 23;
        self.fill(0, BAND_TOP as i32, W, W - BAND_TOP, Color::White);
        self.rule(170);
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for word in s.split_whitespace() {
            let fits = cur.is_empty() || cur.chars().count() + 1 + word.chars().count() <= COLS;
            if fits {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(word);
            } else {
                lines.push(core::mem::take(&mut cur));
                cur.push_str(word);
            }
            if lines.len() == 2 {
                break;
            }
        }
        if lines.len() < 2 && !cur.is_empty() {
            lines.push(cur);
        }
        for (i, line) in lines.iter().take(2).enumerate() {
            let shown: String = line.chars().take(COLS).collect();
            self.text(8, 183 + 14 * i as i32, &shown, &FONT_8X13, Alignment::Left);
        }
    }

    /// A screen of lines under a title: the self-test, errors.
    pub fn screen(&mut self, title: &str, lines: &[&str]) {
        let _ = self.fb.clear(Color::White);
        self.text(8, 19, title, &FONT_9X15_BOLD, Alignment::Left);
        self.text(192, 19, self.label, &FONT_6X10, Alignment::Right);
        self.rule(26);
        for (i, l) in lines.iter().take(9).enumerate() {
            let line: String = l.chars().take(31).collect();
            self.text(8, 44 + 14 * i as i32, &line, &FONT_6X10, Alignment::Left);
        }
        self.full();
    }
}

impl Display for Epd {
    fn show_words(&mut self, w: &[&str; 6]) {
        let _ = self.fb.clear(Color::White);
        self.header();
        for r in 0..3 {
            let y = 62 + 36 * r as i32;
            self.text(8, y, w[2 * r], &FONT_10X20, Alignment::Left);
            self.text(192, y, w[2 * r + 1], &FONT_10X20, Alignment::Right);
        }
        // The badge sets the footer for the state it is in; the words screen itself says nothing.
        self.footer("");
        self.full();
    }

    fn show_qr(&mut self, data: &[u8]) {
        let _ = self.fb.clear(Color::White);
        self.header();
        let mut rows: Vec<String> = Vec::new();
        for chunk in data.chunks(10).take(8) {
            let mut s = String::new();
            for b in chunk {
                let hex = b"0123456789abcdef";
                s.push(hex[(b >> 4) as usize] as char);
                s.push(hex[(b & 15) as usize] as char);
                s.push(' ');
            }
            rows.push(s);
        }
        for (i, r) in rows.iter().enumerate() {
            self.text(8, 48 + 14 * i as i32, r, &FONT_6X10, Alignment::Left);
        }
        let n = alloc::format!("code: {} bytes", data.len());
        self.footer(&n);
        self.full();
    }

    /// One line goes in the footer band. Several lines (the badge's self-test, "circle
    /// forgotten") fill the screen instead, since the band holds one.
    fn show_status(&mut self, s: &str) {
        crate::store::trace(&alloc::format!("display: status {s:?}"));
        if s.contains('\n') {
            let lines: Vec<&str> = s.lines().collect();
            self.screen("cahoots", &lines);
            return;
        }
        self.footer(s);
        self.band(BAND_TOP, W);
    }
}
