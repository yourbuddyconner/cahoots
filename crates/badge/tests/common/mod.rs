#![allow(dead_code)]

use cahoots_badge::{word_indices, Board, BoardError, Circle, Press};
use cahoots_core::checkwords::{check_words, WORDLIST};
use cahoots_core::circle_id::circle_id;
use cahoots_core::params::CircleParams;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{Display, Entropy, EraseError, Frame, HealthError, Link, PadStore, StoreError};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

pub struct Xorshift(pub u64);
impl Entropy for Xorshift {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        for b in buf {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = (self.0 >> 24) as u8;
        }
        Ok(())
    }
}

/// Every frame drawn, in order.
#[derive(Default)]
pub struct FakeDisplay {
    pub frames: Vec<Frame>,
}
impl Display for FakeDisplay {
    fn draw(&mut self, frame: &Frame) {
        self.frames.push(frame.clone());
    }
}

/// A noted screen that shows six words from the word list in a row: the first run of six
/// word-list words among its lines' words.
fn as_words(lines: &[String]) -> Option<[String; 6]> {
    let tokens: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
    tokens
        .windows(6)
        .find(|w| w.iter().all(|t| WORDLIST.contains(t)))
        .map(|w| core::array::from_fn(|i| w[i].to_string()))
}

/// A noted screen titled "code" whose other lines are 32 bytes in hex: a key shown as a code.
fn as_code(lines: &[String]) -> Option<Vec<u8>> {
    let (title, rows) = lines.split_first()?;
    if title != "code" {
        return None;
    }
    let hex: String = rows.iter().flat_map(|l| l.split_whitespace()).collect();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok())
        .collect()
}

/// A snapshot of the pad.
pub struct VecPad(pub Vec<u8>);
impl PadStore for VecPad {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&self, off: u32, buf: &mut [u8]) {
        buf.copy_from_slice(&self.0[off as usize..off as usize + buf.len()]);
    }
    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError> {
        self.0[off as usize..(off + len) as usize].fill(0);
        Ok(())
    }
    fn erase_unit(&self) -> u32 {
        1
    }
}

pub struct VecRecords(pub Vec<u8>);
impl RecordStore for VecRecords {
    fn capacity(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&self, off: u32, buf: &mut [u8]) {
        buf.copy_from_slice(&self.0[off as usize..off as usize + buf.len()]);
    }
    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        self.0[off as usize..off as usize + data.len()].copy_from_slice(data);
        Ok(())
    }
}

/// One end of an in-memory cable. `tx` is the other end's `rx`, and both ends share the
/// detect line, so `peer_present` is true on both ends of a cable.
pub struct FakeLink {
    pub tx: Rc<RefCell<VecDeque<u8>>>,
    pub rx: Rc<RefCell<VecDeque<u8>>>,
    pub present: Rc<Cell<bool>>,
}
impl FakeLink {
    /// A connector with nothing plugged in.
    pub fn open() -> Self {
        Self {
            tx: Rc::default(),
            rx: Rc::default(),
            present: Rc::new(Cell::new(false)),
        }
    }
}
impl Link for FakeLink {
    fn send(&mut self, frame: &[u8]) {
        self.tx.borrow_mut().extend(frame);
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        let mut q = self.rx.borrow_mut();
        let n = buf.len().min(q.len());
        for b in buf.iter_mut().take(n) {
            *b = q.pop_front().unwrap();
        }
        n
    }
    fn peer_present(&self) -> bool {
        self.present.get()
    }
}

/// A cable: `(lower, upper)`. The lower end goes in the lower card's `up`, the upper end in
/// the upper card's `down`.
pub fn cable() -> (FakeLink, FakeLink) {
    let a: Rc<RefCell<VecDeque<u8>>> = Rc::default();
    let b: Rc<RefCell<VecDeque<u8>>> = Rc::default();
    let present = Rc::new(Cell::new(true));
    (
        FakeLink {
            tx: a.clone(),
            rx: b.clone(),
            present: present.clone(),
        },
        FakeLink {
            tx: b,
            rx: a,
            present,
        },
    )
}

/// The tier the fake board reports. Non-zero, so a test can see it passed through.
pub const FAKE_TIER: u8 = 1;

pub struct FakeBoard {
    pub entropy: Tested<Xorshift>,
    pub display: FakeDisplay,
    /// The lines of every screen the badge noted, in order.
    pub screens: Vec<Vec<String>>,
    pub pad: Vec<u8>,
    pub records: Vec<u8>,
    pub circle: Option<Circle>,
    /// The gestures `press()` hands out, one per call, then `Press::None`.
    pub presses: VecDeque<Press>,
    pub a_down: bool,
    /// A is released once this many milliseconds have passed in `delay_ms`.
    pub release_at_ms: u32,
    /// The milliseconds passed in `delay_ms` so far.
    pub elapsed_ms: u32,
    pub log: Vec<String>,
    pub selftest_lines: Vec<String>,
    /// The `(down, up)` links handed out by the next `links()` call; open ends when `None`.
    pub links: Option<(FakeLink, FakeLink)>,
}

impl FakeBoard {
    pub fn new() -> Self {
        Self {
            entropy: Tested::new(
                Xorshift(0x9E37_79B9_7F4A_7C15),
                HealthTests::for_min_entropy(6),
            ),
            display: FakeDisplay::default(),
            screens: Vec::new(),
            pad: Vec::new(),
            records: vec![0xFF; 4096],
            circle: None,
            presses: VecDeque::new(),
            a_down: false,
            release_at_ms: 3000,
            elapsed_ms: 0,
            log: Vec::new(),
            selftest_lines: vec!["fake: ok".into()],
            links: None,
        }
    }

    /// A board whose random source starts from `seed`, so two fakes in one circle differ.
    pub fn seeded(seed: u64) -> Self {
        let mut b = Self::new();
        b.entropy = Tested::new(Xorshift(seed), HealthTests::for_min_entropy(6));
        b
    }

    /// The status screens, each one's lines joined by newlines.
    pub fn statuses(&self) -> Vec<String> {
        self.screens
            .iter()
            .filter(|l| as_words(l).is_none() && as_code(l).is_none())
            .map(|l| l.join("\n"))
            .collect()
    }

    /// The lines of the last screen noted.
    pub fn last(&self) -> &[String] {
        self.screens.last().map_or(&[], Vec::as_slice)
    }

    /// True if the last screen noted has a line equal to `line`.
    pub fn shows(&self, line: &str) -> bool {
        self.last().iter().any(|l| l == line)
    }

    /// The word screens.
    pub fn words(&self) -> Vec<[String; 6]> {
        self.screens.iter().filter_map(|l| as_words(l)).collect()
    }

    /// The keys shown as codes.
    pub fn codes(&self) -> Vec<Vec<u8>> {
        self.screens.iter().filter_map(|l| as_code(l)).collect()
    }

    /// Pre-loaded with a 3-member circle and a 192-byte pad.
    pub fn with_circle() -> (Self, CircleParams, Vec<u8>) {
        let mut b = Self::new();
        let params = CircleParams::new(3, 192, "fakes", false).unwrap();
        let pad: Vec<u8> = (0..192u32).map(|i| (i * 7 + 3) as u8).collect();
        b.pad = pad.clone();
        b.circle = Some(Circle {
            params: params.clone(),
            seat: 1,
            circle_id: circle_id(&pad),
            words: word_indices(&check_words(&params, &pad)).unwrap(),
        });
        (b, params, pad)
    }
}

impl Board for FakeBoard {
    type Entropy = Tested<Xorshift>;
    type Pad = VecPad;
    type Records = VecRecords;
    type Display = FakeDisplay;
    type Link = FakeLink;

    fn entropy(&mut self) -> &mut Self::Entropy {
        &mut self.entropy
    }
    fn display(&mut self) -> &mut FakeDisplay {
        &mut self.display
    }
    fn press(&mut self) -> Press {
        self.presses.pop_front().unwrap_or(Press::None)
    }
    fn a_down(&self) -> bool {
        self.a_down
    }
    fn delay_ms(&mut self, ms: u32) {
        self.elapsed_ms += ms;
        if self.elapsed_ms >= self.release_at_ms {
            self.a_down = false;
        }
    }
    fn links(&mut self) -> (FakeLink, FakeLink) {
        self.links
            .take()
            .unwrap_or_else(|| (FakeLink::open(), FakeLink::open()))
    }
    fn load_circle(&self) -> Option<Circle> {
        self.circle.clone()
    }
    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError> {
        self.circle = Some(c.clone());
        Ok(())
    }
    fn wipe(&mut self, pad_len: u32) -> Result<(), BoardError> {
        self.circle = None;
        self.records.fill(0xFF);
        let n = (pad_len as usize).min(self.pad.len());
        self.pad[..n].fill(0);
        Ok(())
    }
    fn write_pad(&mut self, pad: &[u8]) -> Result<VecPad, BoardError> {
        self.pad = pad.to_vec();
        Ok(VecPad(self.pad.clone()))
    }
    fn open_pad(&mut self, len: u32) -> VecPad {
        VecPad(self.pad[..len as usize].to_vec())
    }
    fn records(&mut self, fresh: bool) -> Result<VecRecords, BoardError> {
        if fresh {
            self.records.fill(0xFF);
        }
        Ok(VecRecords(self.records.clone()))
    }
    fn proposal(&self) -> (String, u32) {
        ("fakes".into(), 192)
    }
    fn tier(&self) -> u8 {
        FAKE_TIER
    }
    fn selftest(&mut self) -> Vec<String> {
        self.selftest_lines.clone()
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
    fn note_screen(&mut self, lines: &[String]) {
        self.screens.push(lines.to_vec());
    }
}
