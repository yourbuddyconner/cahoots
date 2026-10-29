//! The badge's screens, each drawn with `embedded-graphics` into a [`Frame`].
//!
//! A [`Screen`] is the frame plus the text it shows, so a test or the host can read the
//! screen without decoding pixels. A screen with a circle has a header: the circle name on
//! the left, the tier label on the right, and a rule under them. A screen that asks or tells
//! has a title bar: white bold text on a black band across the top. Body text is the 10 by
//! 20 font, the bold 9 by 18 for the one line that matters, and the 8 by 13 for the rest. A
//! screen that waits for a button has a footer under a rule: what A does on the left, what
//! both do in the middle, what B does on the right, in the places the buttons are. A menu
//! page shows `<` and `>` at the edges of the middle row when there is a page on that side.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_hal::Frame;
use core::convert::Infallible;
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X13, FONT_6X13_BOLD, FONT_7X13_BOLD, FONT_8X13, FONT_9X18_BOLD};
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
/// The y of the rule above the footer, which names what the buttons do.
const FOOT_Y: i32 = 177;
const W: i32 = Frame::WIDTH as i32;

/// The boot self-test: its lines under the title "self-test".
pub fn self_test(lines: &[String]) -> Screen {
    let body: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut s = titled("self-test", &body);
    s.footer(None, Some("A+B  skip the wait"), None);
    s.done()
}

/// The panic recorded on the last run.
pub fn panic(msg: &str) -> Screen {
    let mut s = titled("last panic", &[msg]);
    s.footer(None, Some("A+B  continue"), None);
    s.done()
}

/// The pile: "stack the cards", and the seat once this card knows it, large.
pub fn pile(seat: Option<u8>) -> Screen {
    let mut s = Draw::new();
    s.title("stack the cards");
    match seat {
        Some(seat) => {
            s.centered(&FONT_8X13, 52, "this card is");
            s.centered(&PROFONT_24_POINT, 76, &format!("seat {seat}"));
            s.centered(&FONT_8X13, 130, "seat 0 is at the bottom");
        }
        None => {
            s.centered(&FONT_10X20, 80, "looking for");
            s.centered(&FONT_10X20, 104, "the pile...");
        }
    }
    s.footer(None, Some("A+B on the top card  start"), None);
    s.done()
}

/// The ceremony's counter: commitments, reveals and attestations held, out of `n`, each as a
/// bar that fills.
pub fn ceremony(commits: u8, reveals: u8, attestations: u8, n: u8) -> Screen {
    let mut s = Draw::new();
    s.title("ceremony");
    s.bar(40, "commits", commits, n);
    s.bar(86, "reveals", reveals, n);
    s.bar(132, "signatures", attestations, n);
    s.lines.push(format!("attestations {attestations}/{n}"));
    s.done()
}

/// Shown while the ceremony makes this card's seat key: where the key is made, in the chip
/// or in software. Nothing is asked and no button does anything, so the footer says to wait.
pub fn making_key(in_chip: bool) -> Screen {
    let mut s = Draw::new();
    s.title("making a key");
    s.centered(&FONT_9X18_BOLD, 66, "this circle's key");
    s.centered(
        &FONT_10X20,
        96,
        if in_chip { "in the chip" } else { "in software" },
    );
    s.footer_note("one moment");
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
    s.title("keep this circle?");
    for (i, w) in words.iter().enumerate() {
        s.centered(&PROFONT_18_POINT, 32 + 23 * i as i32, w);
    }
    s.footer(Some("no"), Some("A+B  keep"), Some("no"));
    s.done()
}

/// The second question after a single press on the keep prompt.
pub fn discard_prompt() -> Screen {
    let mut s = Draw::new();
    s.title("discard this circle?");
    s.centered(&FONT_9X18_BOLD, 62, "the words and");
    s.centered(&FONT_9X18_BOLD, 86, "the pad are lost");
    s.centered(&FONT_8X13, 124, "nothing is saved yet");
    s.footer(Some("back"), Some("A+B  discard"), Some("back"));
    s.done()
}

/// A page of the menu: the header, "page/pages" under it, the title, the body, and an
/// arrow on each side that has a neighbouring page.
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
    s.centered(&FONT_8X13, 34, &format!("{page}/{pages}"));
    s.centered(&FONT_9X18_BOLD, 56, title);
    // Each body line is kept whole in `lines` and drawn wrapped, narrower than the screen
    // so the rows clear the arrows.
    let mut y = 90;
    for line in body {
        y = s.wrapped_centered_cols(&FONT_8X13, y, line, cols(&FONT_8X13) - 2, false);
        s.lines.push(String::from(*line));
    }
    if left {
        s.arrow(2, "<");
    }
    if right {
        s.arrow(190, ">");
    }
    s.footer(Some("page"), Some("A+B  choose"), Some("page"));
    s.done()
}

/// Bytes shown as hex under the inverted title "code": eight bytes per row, in groups of two.
pub fn code(bytes: &[u8]) -> Screen {
    let mut s = Draw::new();
    s.title("code");
    for (i, row) in bytes.chunks(8).enumerate() {
        let groups: Vec<String> = row
            .chunks(2)
            .map(|g| g.iter().map(|b| format!("{b:02x}")).collect())
            .collect();
        s.centered(&FONT_10X20, 44 + 28 * i as i32, &groups.join(" "));
    }
    let mut s = s.done();
    s.secret = true;
    s
}

/// The question a spending request asks: spend `len` bytes for `circle`?
pub fn consent(len: u32, circle: &str) -> Screen {
    let mut s = Draw::new();
    s.title(&format!("spend {len} bytes"));
    s.centered(&FONT_8X13, 54, "a laptop asks for a key");
    s.wrapped_centered(&FONT_9X18_BOLD, 84, &format!("for {circle}?"));
    s.footer(Some("refuse"), Some("A+B  allow"), Some("refuse"));
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
    let pitch = font.character_size.height as i32 + 2;
    let all: Vec<String> = lines.iter().flat_map(|l| wrap(l, cols(font))).collect();
    // The block of text sits in the middle of the screen, each row centred.
    let mut y = ((Frame::HEIGHT as i32) - all.len() as i32 * pitch).max(6) / 2;
    for row in &all {
        s.text(font, Point::new(W / 2, y), Alignment::Center, BinaryColor::On, row);
        y += pitch;
    }
    s.lines = lines.iter().map(|l| String::from(*l)).collect();
    s.done()
}

/// A screen with a title bar and secondary lines under it, wrapped in the frame.
fn titled(title: &str, body: &[&str]) -> Draw {
    let mut s = Draw::new();
    s.title(title);
    let mut y = 34;
    for line in body {
        y = s.wrapped_unrecorded(&FONT_6X13, y, line);
        s.lines.push(String::from(*line));
    }
    s
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

    /// The title bar: `s` in white on a black band across the top of the screen.
    fn title(&mut self, s: &str) {
        self.fill(Point::new(0, 0), Size::new(Frame::WIDTH as u32, 26));
        let at = Point::new(W / 2, 4);
        self.text(&FONT_9X18_BOLD, at, Alignment::Center, BinaryColor::Off, s);
        self.lines.push(String::from(s));
    }

    /// The footer: a rule, then what A alone, both buttons, and B alone do, in the places
    /// the buttons are. A missing side means that button does nothing here.
    fn footer(&mut self, left: Option<&str>, both: Option<&str>, right: Option<&str>) {
        let _ = Line::new(Point::new(0, FOOT_Y), Point::new(W - 1, FOOT_Y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut Canvas(&mut self.frame));
        let y = FOOT_Y + 6;
        let on = BinaryColor::On;
        if let Some(l) = left {
            let s = format!("< {l}");
            self.text(&FONT_6X13_BOLD, Point::new(3, y), Alignment::Left, on, &s);
        }
        if let Some(r) = right {
            let s = format!("{r} >");
            self.text(&FONT_6X13_BOLD, Point::new(W - 3, y), Alignment::Right, on, &s);
        }
        if let Some(b) = both {
            self.text(&FONT_6X13_BOLD, Point::new(W / 2, y), Alignment::Center, on, b);
        }
        let legend: Vec<String> = [left, both, right].iter().flatten().map(|s| String::from(*s)).collect();
        self.lines.push(legend.join("  "));
    }

    /// The footer of a screen no button acts on: the rule, and `note` in the middle.
    fn footer_note(&mut self, note: &str) {
        let _ = Line::new(Point::new(0, FOOT_Y), Point::new(W - 1, FOOT_Y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut Canvas(&mut self.frame));
        let at = Point::new(W / 2, FOOT_Y + 6);
        self.text(&FONT_6X13_BOLD, at, Alignment::Center, BinaryColor::On, note);
        self.lines.push(String::from(note));
    }

    /// A labelled progress bar with its top at `y`: the label on the left, `have/n` on
    /// the right, and under them a box that fills from the left.
    fn bar(&mut self, y: i32, label: &str, have: u8, n: u8) {
        let on = BinaryColor::On;
        self.text(&FONT_7X13_BOLD, Point::new(10, y), Alignment::Left, on, label);
        let count = format!("{have}/{n}");
        self.text(&FONT_7X13_BOLD, Point::new(W - 10, y), Alignment::Right, on, &count);
        let (x0, x1, top) = (10, W - 10, y + 17);
        let _ = Rectangle::new(Point::new(x0, top), Size::new((x1 - x0) as u32, 12))
            .into_styled(PrimitiveStyle::with_stroke(on, 1))
            .draw(&mut Canvas(&mut self.frame));
        if n > 0 && have > 0 {
            let w = (x1 - x0 - 4) * i32::from(have.min(n)) / i32::from(n);
            self.fill(Point::new(x0 + 2, top + 2), Size::new(w as u32, 8));
        }
        self.lines.push(format!("{label} {have}/{n}"));
    }

    /// `s` wrapped to the screen's width, each row centred, from `y`. Returns the y after.
    fn wrapped_centered(&mut self, font: &MonoFont, y: i32, s: &str) -> i32 {
        self.wrapped_centered_cols(font, y, s, cols(font), true)
    }

    /// As [`Draw::wrapped_centered`], wrapped at `cols` characters, narrower than the screen
    /// where something sits at the edges, and recording the rows in `lines` only if `record`.
    fn wrapped_centered_cols(&mut self, font: &MonoFont, mut y: i32, s: &str, cols: usize, record: bool) -> i32 {
        let pitch = font.character_size.height as i32 + 2;
        for row in wrap(s, cols) {
            self.text(font, Point::new(W / 2, y), Alignment::Center, BinaryColor::On, &row);
            if record {
                self.lines.push(row);
            }
            y += pitch;
        }
        y
    }

    /// `s` wrapped to the screen's width, left-aligned from `y`, without adding the rows to
    /// `lines`. Returns the y after it.
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
