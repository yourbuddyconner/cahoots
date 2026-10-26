//! The badge's screens, each drawn with `embedded-graphics` into a [`Frame`].
//!
//! A [`Screen`] is the frame plus the text it shows, so a test or the host can read the
//! screen without decoding pixels. A screen with a circle has a header: the circle name on
//! the left, the tier label on the right, and a rule under them. Main text is the 10 by 20
//! font and secondary text the 8 by 13 font. Emphasis is white text on a black box. A menu
//! page shows `<` and `>` at the edges of the middle row when there is a page on that side.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_hal::Frame;
use core::convert::Infallible;
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_8X13};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use profont::{PROFONT_18_POINT, PROFONT_24_POINT};

/// One screen: the text it shows, top to bottom, and the frame a board draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    /// One entry per line of text. The lines a caller passes to [`self_test`], [`panic`] and
    /// [`status`] are kept whole here; a line too wide for the screen wraps in the frame.
    pub lines: Vec<String>,
    pub frame: Frame,
    /// The screen shows key bytes. A board must not copy its lines anywhere that outlives
    /// the screen, such as a log or a trace on flash: the pad erased those bytes already.
    pub secret: bool,
}

/// The y of the rule under the header.
const RULE_Y: i32 = 26;
/// The y of the middle row, where the page arrows sit.
const MID_Y: i32 = 100;
const W: i32 = Frame::WIDTH as i32;

/// The boot self-test: its lines under the title "self-test".
pub fn self_test(lines: &[String]) -> Screen {
    let body: Vec<&str> = lines.iter().map(String::as_str).collect();
    titled("self-test", &body)
}

/// The panic recorded on the last run.
pub fn panic(msg: &str) -> Screen {
    titled("last panic", &[msg])
}

/// The pile: "stack the cards", and the seat once this card knows it.
pub fn pile(seat: Option<u8>) -> Screen {
    let mut s = Draw::new();
    s.centered(&FONT_10X20, 40, "stack the cards");
    if let Some(seat) = seat {
        s.inverted_centered(&FONT_10X20, 80, &format!("seat {seat}"));
    }
    s.centered(&FONT_8X13, 140, "Both on top starts");
    s.done()
}

/// The ceremony's counter: commitments, reveals and attestations held, out of `n`.
pub fn ceremony(commits: u8, reveals: u8, attestations: u8, n: u8) -> Screen {
    let mut s = Draw::new();
    s.inverted_centered(&FONT_10X20, 30, "ceremony");
    s.centered(&FONT_10X20, 80, &format!("commits {commits}/{n}"));
    s.centered(&FONT_10X20, 110, &format!("reveals {reveals}/{n}"));
    s.centered(
        &FONT_10X20,
        140,
        &format!("attestations {attestations}/{n}"),
    );
    s.done()
}

/// The six check words, one per row in the large face, under the circle's header.
pub fn words(circle: &str, tier: &str, words: &[&str; 6]) -> Screen {
    let mut s = Draw::new();
    s.header(circle, tier);
    s.word_rows(words);
    s.done()
}

/// The menu's words page: the header with "page/pages" in its middle, the six words one per
/// row in the large face, and an arrow on each side that has a neighbouring page.
pub fn words_page(
    circle: &str,
    tier: &str,
    page: usize,
    pages: usize,
    words: &[&str; 6],
    left: bool,
    right: bool,
) -> Screen {
    let mut s = Draw::new();
    s.header(circle, tier);
    s.centered(&FONT_8X13, 7, &format!("{page}/{pages}"));
    s.word_rows(words);
    if left {
        s.arrow(2, "<");
    }
    if right {
        s.arrow(W - 10, ">");
    }
    s.done()
}

/// The question after a ceremony: the words, and keep or discard.
pub fn keep_prompt(words: &[&str; 6]) -> Screen {
    let mut s = Draw::new();
    s.inverted_centered(&FONT_10X20, 4, "keep this circle?");
    for (i, w) in words.iter().enumerate() {
        s.centered(&PROFONT_18_POINT, 30 + 23 * i as i32, w);
    }
    s.centered(&FONT_8X13, 172, "Both to keep");
    s.centered(&FONT_8X13, 186, "one button discards");
    s.done()
}

/// A page of the menu: the header, "page/pages" under it, the title inverted, the body, and
/// an arrow on each side that has a neighbouring page.
#[allow(clippy::too_many_arguments)]
pub fn menu_page(
    circle: &str,
    tier: &str,
    page: usize,
    pages: usize,
    title: &str,
    body: &[&str],
    left: bool,
    right: bool,
) -> Screen {
    let mut s = Draw::new();
    s.header(circle, tier);
    s.centered(&FONT_8X13, 30, &format!("{page}/{pages}"));
    s.inverted_centered(&FONT_10X20, 50, title);
    let mut y = 120;
    for line in body {
        y = s.wrapped(&FONT_8X13, y, line);
    }
    if left {
        s.arrow(2, "<");
    }
    if right {
        s.arrow(190, ">");
    }
    s.done()
}

/// Bytes shown as hex under the inverted title "code": eight bytes per row, in groups of two.
pub fn code(bytes: &[u8]) -> Screen {
    let mut s = Draw::new();
    s.inverted_centered(&FONT_10X20, 4, "code");
    for (i, row) in bytes.chunks(8).enumerate() {
        let groups: Vec<String> = row
            .chunks(2)
            .map(|g| g.iter().map(|b| format!("{b:02x}")).collect())
            .collect();
        s.centered(&FONT_10X20, 40 + 26 * i as i32, &groups.join(" "));
    }
    let mut s = s.done();
    s.secret = true;
    s
}

/// The question a spending request asks: spend `len` bytes for `circle`?
pub fn consent(len: u32, circle: &str) -> Screen {
    let mut s = Draw::new();
    s.inverted_centered(&FONT_10X20, 4, &format!("spend {len} bytes"));
    s.wrapped(&FONT_10X20, 60, &format!("for {circle}?"));
    s.centered(&FONT_8X13, 140, "Both to allow");
    s.centered(&FONT_8X13, 158, "one button refuses");
    s.done()
}

/// One or a few lines of text, for errors and short notes. `lines` keeps the caller's lines;
/// a line too wide for the screen wraps in the frame.
pub fn status(lines: &[&str]) -> Screen {
    let mut s = Draw::new();
    let rows: usize = lines.iter().map(|l| wrap(l, cols(&FONT_10X20)).len()).sum();
    let font = if rows * 22 <= 190 {
        &FONT_10X20
    } else {
        &FONT_8X13
    };
    let mut y = 6;
    for line in lines {
        y = s.wrapped_unrecorded(font, y, line);
    }
    s.lines = lines.iter().map(|l| String::from(*l)).collect();
    s.done()
}

/// A screen with an inverted title and secondary lines under it, wrapped in the frame.
fn titled(title: &str, body: &[&str]) -> Screen {
    let mut s = Draw::new();
    s.inverted_centered(&FONT_10X20, 4, title);
    let mut y = 34;
    for line in body {
        y = s.wrapped_unrecorded(&FONT_8X13, y, line);
        s.lines.push(String::from(*line));
    }
    s.done()
}

/// How many characters of `font` fit across the screen, with a 4 pixel margin each side.
fn cols(font: &MonoFont) -> usize {
    (Frame::WIDTH - 8) / font.character_size.width as usize
}

/// Split `text` into lines of at most `cols` characters, at spaces where it can.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut word = word;
        while word.chars().count() > cols {
            if !line.is_empty() {
                out.push(core::mem::take(&mut line));
            }
            let cut = word.char_indices().nth(cols).map_or(word.len(), |(i, _)| i);
            out.push(String::from(&word[..cut]));
            word = &word[cut..];
        }
        let need = if line.is_empty() {
            word.chars().count()
        } else {
            line.chars().count() + 1 + word.chars().count()
        };
        if need > cols && !line.is_empty() {
            out.push(core::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

/// A [`Frame`] as an `embedded-graphics` target. `BinaryColor::On` is black.
struct Canvas<'a>(&'a mut Frame);

impl OriginDimensions for Canvas<'_> {
    fn size(&self) -> Size {
        Size::new(Frame::WIDTH as u32, Frame::HEIGHT as u32)
    }
}

impl DrawTarget for Canvas<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Infallible>
    where
        I: IntoIterator<Item = Pixel<BinaryColor>>,
    {
        for Pixel(p, c) in pixels {
            if let (Ok(x), Ok(y)) = (usize::try_from(p.x), usize::try_from(p.y)) {
                self.0.set(x, y, c.is_on());
            }
        }
        Ok(())
    }
}

/// A screen being drawn: the frame and the lines written so far.
struct Draw {
    frame: Frame,
    lines: Vec<String>,
}

impl Draw {
    fn new() -> Self {
        Draw {
            frame: Frame::blank(),
            lines: Vec::new(),
        }
    }

    fn done(self) -> Screen {
        Screen {
            lines: self.lines,
            frame: self.frame,
            secret: false,
        }
    }

    /// Draw `text` with its top at `y`. Drawing into a frame cannot fail.
    fn text(&mut self, font: &MonoFont, at: Point, align: Alignment, color: BinaryColor, s: &str) {
        let style = MonoTextStyle::new(font, color);
        let layout = TextStyleBuilder::new()
            .alignment(align)
            .baseline(Baseline::Top)
            .build();
        let _ = Text::with_text_style(s, at, style, layout).draw(&mut Canvas(&mut self.frame));
    }

    fn fill(&mut self, at: Point, size: Size) {
        let _ = Rectangle::new(at, size)
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut Canvas(&mut self.frame));
    }

    /// The circle name on the left, the tier label on the right, and the rule under them.
    fn header(&mut self, circle: &str, tier: &str) {
        let on = BinaryColor::On;
        self.text(&FONT_8X13, Point::new(4, 7), Alignment::Left, on, circle);
        self.text(&FONT_8X13, Point::new(W - 4, 7), Alignment::Right, on, tier);
        let _ = Line::new(Point::new(0, RULE_Y), Point::new(W - 1, RULE_Y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut Canvas(&mut self.frame));
        self.lines.push(format!("{circle}  {tier}"));
    }

    fn centered(&mut self, font: &MonoFont, y: i32, s: &str) {
        let at = Point::new(W / 2, y);
        self.text(font, at, Alignment::Center, BinaryColor::On, s);
        self.lines.push(String::from(s));
    }

    /// `s` in white on a black box as wide as the screen, with its top at `y`.
    fn inverted_centered(&mut self, font: &MonoFont, y: i32, s: &str) {
        let h = font.character_size.height;
        self.fill(Point::new(0, y - 2), Size::new(Frame::WIDTH as u32, h + 4));
        let at = Point::new(W / 2, y);
        self.text(font, at, Alignment::Center, BinaryColor::Off, s);
        self.lines.push(String::from(s));
    }

    /// `s` wrapped to the screen's width, left-aligned from `y`. Returns the y after it.
    fn wrapped(&mut self, font: &MonoFont, y: i32, s: &str) -> i32 {
        let rows = wrap(s, cols(font));
        self.lines.extend(rows.iter().cloned());
        self.rows(font, y, &rows)
    }

    /// As [`Draw::wrapped`], without adding the rows to `lines`.
    fn wrapped_unrecorded(&mut self, font: &MonoFont, y: i32, s: &str) -> i32 {
        self.rows(font, y, &wrap(s, cols(font)))
    }

    fn rows(&mut self, font: &MonoFont, mut y: i32, rows: &[String]) -> i32 {
        let pitch = font.character_size.height as i32 + 2;
        for row in rows {
            self.text(
                font,
                Point::new(4, y),
                Alignment::Left,
                BinaryColor::On,
                row,
            );
            y += pitch;
        }
        y
    }

    /// The six words one per row, centred, in the large face, filling the screen below the
    /// header.
    fn word_rows(&mut self, words: &[&str; 6]) {
        for (i, w) in words.iter().enumerate() {
            self.centered(&PROFONT_24_POINT, 30 + 28 * i as i32, w);
        }
    }

    /// An arrow glyph with its left edge at `x`, centred on the middle row.
    fn arrow(&mut self, x: i32, glyph: &str) {
        let style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
        let layout = TextStyleBuilder::new().baseline(Baseline::Middle).build();
        let at = Point::new(x, MID_Y);
        let _ = Text::with_text_style(glyph, at, style, layout).draw(&mut Canvas(&mut self.frame));
    }
}
