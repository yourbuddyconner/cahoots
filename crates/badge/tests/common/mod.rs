#![allow(dead_code)]

use cahoots_badge::board::{assign_places, place_for};
use cahoots_badge::chip::ChipRecord;
use cahoots_badge::grants::GrantsRecord;
use cahoots_badge::{word_indices, Board, BoardError, Circle, Press};
use cahoots_chip::{ChipOps, SoftChip};
use cahoots_core::checkwords::{check_words, WORDLIST};
use cahoots_core::circle_id::{circle_id, CircleId};
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

/// A place on the fake board beyond the first: a circle record, its pad and its ledger.
#[derive(Clone, Default)]
pub struct FakePlace {
    pub circle: Option<Circle>,
    pub pad: Vec<u8>,
    pub records: Vec<u8>,
}

/// The tier the fake board reports. Non-zero, so a test can see it passed through.
pub const FAKE_TIER: u8 = 1;

pub struct FakeBoard {
    pub entropy: Tested<Xorshift>,
    pub display: FakeDisplay,
    /// The lines of every screen the badge noted, in order.
    pub screens: Vec<Vec<String>>,
    /// The first place: its pad, ledger and circle record. A badge with one circle uses
    /// only this place.
    pub pad: Vec<u8>,
    pub records: Vec<u8>,
    pub circle: Option<Circle>,
    /// The places after the first, `max_circles - 1` of them.
    pub more: Vec<FakePlace>,
    /// The place a pad was written to for a circle not stored yet.
    pub pending: Option<(CircleId, usize)>,
    /// The name this card proposes for a circle when it is the bottom of the pile.
    pub name: String,
    /// `forget` fails at the ledger erase, after the record and the pad are gone.
    pub fail_ledger_erase: bool,
    /// The gestures `press()` hands out, one per call, then `Press::None`.
    pub presses: VecDeque<Press>,
    pub a_down: bool,
    /// A is released once this many milliseconds have passed in `delay_ms`.
    pub release_at_ms: u32,
    /// A goes down once this many milliseconds have passed, if set (and before release).
    pub hold_a_at_ms: Option<u32>,
    /// The milliseconds passed in `delay_ms` so far.
    pub elapsed_ms: u32,
    pub log: Vec<String>,
    pub selftest_lines: Vec<String>,
    /// The `(down, up)` links handed out by the next `links()` call; open ends when `None`.
    pub links: Option<(FakeLink, FakeLink)>,
    /// The board's chip, if it has one.
    pub chip: Option<SoftChip>,
    /// The chip record as stored: its CBOR encoding, so a test sees what a board would keep.
    pub chip_record: Option<Vec<u8>>,
    /// The grants record as stored, its CBOR encoding.
    pub grants: Option<Vec<u8>>,
    /// Seconds since boot, as the board reports them.
    pub uptime: u64,
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
            more: vec![
                FakePlace {
                    circle: None,
                    pad: Vec::new(),
                    records: vec![0xFF; 4096],
                };
                3
            ],
            pending: None,
            name: "fakes".into(),
            fail_ledger_erase: false,
            presses: VecDeque::new(),
            a_down: false,
            release_at_ms: 3000,
            hold_a_at_ms: None,
            elapsed_ms: 0,
            log: Vec::new(),
            selftest_lines: vec!["fake: ok".into()],
            links: None,
            chip: None,
            chip_record: None,
            grants: None,
            uptime: 0,
        }
    }

    /// A board whose random source starts from `seed`, so two fakes in one circle differ.
    pub fn seeded(seed: u64) -> Self {
        let mut b = Self::new();
        b.entropy = Tested::new(Xorshift(seed), HealthTests::for_min_entropy(6));
        b
    }

    /// The circle id each place holds.
    fn held(&self) -> Vec<Option<CircleId>> {
        core::iter::once(&self.circle)
            .chain(self.more.iter().map(|p| &p.circle))
            .map(|c| c.as_ref().map(|c| c.circle_id))
            .collect()
    }

    fn place(&self, id: &CircleId) -> Option<usize> {
        place_for(&self.held(), self.pending, id)
    }

    /// The pad and ledger of place `p`.
    fn region(&mut self, p: usize) -> (&mut Vec<u8>, &mut Vec<u8>) {
        if p == 0 {
            (&mut self.pad, &mut self.records)
        } else {
            let place = &mut self.more[p - 1];
            (&mut place.pad, &mut place.records)
        }
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
            seat_key: [7; 32],
            chip_slot: None,
            root: None,
            tier: FAKE_TIER,
            // The menu never reads the manifest; a kept circle's is checked in ceremony.rs.
            manifest: Vec::new(),
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
        if self.hold_a_at_ms.is_some_and(|t| self.elapsed_ms >= t) {
            self.a_down = true;
        }
        if self.elapsed_ms >= self.release_at_ms {
            self.a_down = false;
        }
    }
    fn links(&mut self) -> (FakeLink, FakeLink) {
        self.links
            .take()
            .unwrap_or_else(|| (FakeLink::open(), FakeLink::open()))
    }
    fn max_circles(&self) -> usize {
        1 + self.more.len()
    }
    fn load_circles(&mut self) -> Vec<Circle> {
        core::iter::once(&self.circle)
            .chain(self.more.iter().map(|p| &p.circle))
            .filter_map(Clone::clone)
            .collect()
    }
    fn store_circles(&mut self, circles: &[Circle]) -> Result<(), BoardError> {
        let places = assign_places(&self.held(), self.pending, circles)?;
        for (p, i) in places.into_iter().enumerate() {
            let c = i.map(|i| circles[i].clone());
            if p == 0 {
                self.circle = c;
            } else {
                self.more[p - 1].circle = c;
            }
        }
        self.pending = None;
        Ok(())
    }
    /// The first place's circle: the one a badge with one circle keeps.
    fn load_circle(&self) -> Option<Circle> {
        self.circle.clone()
    }
    /// The record, then the pad, then the ledger, as a flash board does. With
    /// `fail_ledger_erase`, the ledger erase fails after the pad is erased.
    fn forget(&mut self, circle: &CircleId, pad_len: u32) -> Result<(), BoardError> {
        let p = self.place(circle).ok_or(BoardError::Storage)?;
        if p == 0 {
            self.circle = None;
        } else {
            self.more[p - 1].circle = None;
        }
        let fail = self.fail_ledger_erase;
        let (pad, records) = self.region(p);
        let n = (pad_len as usize).min(pad.len());
        pad[..n].fill(0);
        if fail {
            return Err(BoardError::Storage);
        }
        records.fill(0xFF);
        Ok(())
    }
    fn write_pad(&mut self, circle: &CircleId, pad: &[u8]) -> Result<VecPad, BoardError> {
        let p = self.place(circle).ok_or(BoardError::Storage)?;
        self.pending = Some((*circle, p));
        let (stored, _) = self.region(p);
        *stored = pad.to_vec();
        Ok(VecPad(pad.to_vec()))
    }
    fn open_pad(&mut self, circle: &CircleId, len: u32) -> Result<VecPad, BoardError> {
        let p = self.place(circle).ok_or(BoardError::Storage)?;
        let (pad, _) = self.region(p);
        Ok(VecPad(
            pad.get(..len as usize).ok_or(BoardError::Storage)?.to_vec(),
        ))
    }
    fn records(&mut self, circle: &CircleId, fresh: bool) -> Result<VecRecords, BoardError> {
        let p = self.place(circle).ok_or(BoardError::Storage)?;
        let (_, records) = self.region(p);
        if fresh {
            records.fill(0xFF);
        }
        Ok(VecRecords(records.clone()))
    }
    fn proposal(&self) -> (String, u32) {
        (self.name.clone(), 192)
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
    fn note_screen(&mut self, lines: &[String], _secret: bool) {
        self.screens.push(lines.to_vec());
    }
    fn chip(&mut self) -> Option<&mut dyn ChipOps> {
        self.chip.as_mut().map(|c| c as &mut dyn ChipOps)
    }
    fn chip_record_load(&mut self) -> Result<Option<ChipRecord>, BoardError> {
        match &self.chip_record {
            None => Ok(None),
            Some(b) => ChipRecord::decode(b)
                .map(Some)
                .map_err(|_| BoardError::Storage),
        }
    }
    fn chip_record_store(&mut self, r: &ChipRecord) -> Result<(), BoardError> {
        self.chip_record = Some(r.encode());
        Ok(())
    }
    fn grants_load(&mut self) -> Result<Option<GrantsRecord>, BoardError> {
        match &self.grants {
            None => Ok(None),
            Some(b) => GrantsRecord::decode(b)
                .map(Some)
                .map_err(|_| BoardError::Storage),
        }
    }
    fn grants_store(&mut self, r: &GrantsRecord) -> Result<(), BoardError> {
        self.grants = Some(r.encode());
        Ok(())
    }
    fn uptime_secs(&self) -> u64 {
        self.uptime
    }
}
