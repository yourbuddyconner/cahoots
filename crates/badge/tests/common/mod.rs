#![allow(dead_code)]

use cahoots_badge::{Board, BoardError, Circle};
use cahoots_core::circle_id::circle_id;
use cahoots_core::params::CircleParams;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{Display, Entropy, EraseError, HealthError, Link, PadStore, StoreError};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    A,
    B,
}

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

#[derive(Default)]
pub struct FakeDisplay {
    pub status: Vec<String>,
    pub words: Vec<[String; 6]>,
    pub qr: Vec<Vec<u8>>,
}
impl Display for FakeDisplay {
    fn show_words(&mut self, w: &[&str; 6]) {
        self.words.push(w.map(String::from));
    }
    fn show_qr(&mut self, data: &[u8]) {
        self.qr.push(data.to_vec());
    }
    fn show_status(&mut self, s: &str) {
        self.status.push(s.to_string());
    }
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

/// One end of an in-memory link; `peer_present` is a flag.
#[derive(Default)]
pub struct FakeLink {
    pub inbox: VecDeque<u8>,
    pub outbox: Vec<u8>,
    pub present: bool,
}
impl Link for FakeLink {
    fn send(&mut self, frame: &[u8]) {
        self.outbox.extend_from_slice(frame);
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.inbox.len());
        for b in buf.iter_mut().take(n) {
            *b = self.inbox.pop_front().unwrap();
        }
        n
    }
    fn peer_present(&self) -> bool {
        self.present
    }
}

pub struct FakeBoard {
    pub entropy: Tested<Xorshift>,
    pub display: FakeDisplay,
    pub pad: Vec<u8>,
    pub records: Vec<u8>,
    pub circle: Option<Circle>,
    pub buttons: VecDeque<Button>,
    pub a_down: bool,
    /// A is released after this many `delay_ms` calls.
    pub release_after: u32,
    pub delays: u32,
    pub log: Vec<String>,
    pub selftest_lines: Vec<String>,
}

impl FakeBoard {
    pub fn new() -> Self {
        Self {
            entropy: Tested::new(
                Xorshift(0x9E37_79B9_7F4A_7C15),
                HealthTests::for_min_entropy(6),
            ),
            display: FakeDisplay::default(),
            pad: Vec::new(),
            records: vec![0xFF; 4096],
            circle: None,
            buttons: VecDeque::new(),
            a_down: false,
            release_after: 3,
            delays: 0,
            log: Vec::new(),
            selftest_lines: vec!["fake: ok".into()],
        }
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
    fn a_pressed(&mut self) -> bool {
        if self.buttons.front() == Some(&Button::A) {
            self.buttons.pop_front();
            true
        } else {
            false
        }
    }
    fn b_pressed(&mut self) -> bool {
        if self.buttons.front() == Some(&Button::B) {
            self.buttons.pop_front();
            true
        } else {
            false
        }
    }
    fn a_down(&self) -> bool {
        self.a_down
    }
    fn delay_ms(&mut self, _ms: u32) {
        self.delays += 1;
        if self.delays >= self.release_after {
            self.a_down = false;
        }
    }
    fn links(&mut self) -> (FakeLink, FakeLink) {
        (FakeLink::default(), FakeLink::default())
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
        0
    }
    fn selftest(&mut self) -> Vec<String> {
        self.selftest_lines.clone()
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
}
