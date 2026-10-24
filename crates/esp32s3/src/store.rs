//! The pad, the ledger and the circle record on the board's NOR flash, by address, in the
//! regions `partitions.csv` reserves.
//!
//! NOR flash erases to `0xFF` by the 4 KiB sector and programs bits from 1 to 0 by the word.
//! So the pad is written once onto erased sectors, and `PadStore::erase` of a spent range
//! programs zeros over it, word by word with read-modify-write at the edges: the trait's
//! "reads back as zero" at byte granularity, with no sector ever re-erased under a live pad.
//! The ledger's 16-byte records program onto `0xFF` as the ledger appends. The circle record
//! is small and rewritten whole.
//!
//! One `FlashStorage` exists per chip, so the stores share it through an `Rc<RefCell>`.

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_core::circle_id::CircleId;
use cahoots_core::params::CircleParams;
use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{EraseError, PadStore, StoreError};
use core::cell::RefCell;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

pub const PAD_BASE: u32 = 0x30_0000;
pub const PAD_CAP: u32 = 0x10_0000;
pub const LEDGER_BASE: u32 = 0x40_0000;
pub const LEDGER_CAP: u32 = 0x1_0000;
pub const CIRCLE_BASE: u32 = 0x41_0000;
pub const SECTOR: u32 = 4096;
const WORD: u32 = 4;
const CIRCLE_MAGIC: &[u8; 4] = b"CAH1";

pub type SharedFlash = Rc<RefCell<FlashStorage<'static>>>;

pub fn open(flash: FLASH<'static>) -> SharedFlash {
    Rc::new(RefCell::new(FlashStorage::new(flash)))
}

/// A word-aligned byte buffer: esp-storage wants its buffers aligned.
#[repr(align(4))]
struct Aligned([u8; 4096]);

fn down(x: u32) -> u32 {
    x & !(WORD - 1)
}
fn up(x: u32) -> u32 {
    (x + WORD - 1) & !(WORD - 1)
}

/// Read any byte range.
pub fn read_any(f: &mut FlashStorage<'_>, addr: u32, out: &mut [u8]) -> Result<(), ()> {
    let mut tmp = Aligned([0; 4096]);
    let mut pos = addr;
    let end = addr + out.len() as u32;
    while pos < end {
        let a0 = down(pos);
        let a1 = up(end).min(a0 + 4096);
        f.read_nor(a0, &mut tmp.0[..(a1 - a0) as usize])
            .map_err(|_| ())?;
        let take_end = a1.min(end);
        let n = (take_end - pos) as usize;
        let from = (pos - a0) as usize;
        let off = (pos - addr) as usize;
        out[off..off + n].copy_from_slice(&tmp.0[from..from + n]);
        pos = take_end;
    }
    Ok(())
}

/// Program bytes over any range. Only clears bits; the bytes around the edges are rewritten
/// with their current value, which NOR flash accepts.
pub fn program_any(f: &mut FlashStorage<'_>, addr: u32, data: &[u8]) -> Result<(), ()> {
    let mut tmp = Aligned([0; 4096]);
    let mut pos = addr;
    let end = addr + data.len() as u32;
    while pos < end {
        let a0 = down(pos);
        let a1 = up(end).min(a0 + 4096);
        let len = (a1 - a0) as usize;
        f.read_nor(a0, &mut tmp.0[..len]).map_err(|_| ())?;
        let take_end = a1.min(end);
        let n = (take_end - pos) as usize;
        let from = (pos - a0) as usize;
        let off = (pos - addr) as usize;
        tmp.0[from..from + n].copy_from_slice(&data[off..off + n]);
        f.write_nor(a0, &tmp.0[..len]).map_err(|_| ())?;
        pos = take_end;
    }
    Ok(())
}

/// Erase whole sectors covering `[addr, addr + len)` to `0xFF`.
pub fn erase_sectors(f: &mut FlashStorage<'_>, addr: u32, len: u32) -> Result<(), ()> {
    let from = addr & !(SECTOR - 1);
    let to = (addr + len + SECTOR - 1) & !(SECTOR - 1);
    f.erase(from, to).map_err(|_| ())
}

/// The pad region, sized to this circle's pad.
pub struct FlashPadStore {
    flash: SharedFlash,
    len: u32,
}

impl FlashPadStore {
    /// Write a fresh pad: erase its sectors, then program it.
    pub fn write(flash: SharedFlash, pad: &[u8]) -> Result<Self, ()> {
        if pad.len() as u32 > PAD_CAP {
            return Err(());
        }
        {
            let mut f = flash.borrow_mut();
            erase_sectors(&mut f, PAD_BASE, pad.len() as u32)?;
            program_any(&mut f, PAD_BASE, pad)?;
        }
        Ok(FlashPadStore {
            flash,
            len: pad.len() as u32,
        })
    }

    pub fn open(flash: SharedFlash, len: u32) -> Self {
        FlashPadStore { flash, len }
    }
}

impl PadStore for FlashPadStore {
    fn len(&self) -> u32 {
        self.len
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        let mut f = self.flash.borrow_mut();
        if read_any(&mut f, PAD_BASE + off, buf).is_err() {
            buf.fill(0);
        }
    }

    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError> {
        let end = off.checked_add(len).ok_or(EraseError::OutOfRange)?;
        if end > self.len {
            return Err(EraseError::OutOfRange);
        }
        let mut f = self.flash.borrow_mut();
        let zeros = vec![0u8; len.min(4096) as usize];
        let mut pos = off;
        while pos < end {
            let n = (end - pos).min(4096);
            program_any(&mut f, PAD_BASE + pos, &zeros[..n as usize])
                .map_err(|_| EraseError::VerifyFailed)?;
            pos += n;
        }
        Ok(())
    }

    fn erase_unit(&self) -> u32 {
        SECTOR
    }
}

/// The ledger region. Records program onto erased flash.
pub struct FlashRecordStore {
    flash: SharedFlash,
}

impl FlashRecordStore {
    /// Erase the region for a new circle.
    pub fn fresh(flash: SharedFlash) -> Result<Self, ()> {
        flash
            .borrow_mut()
            .erase(LEDGER_BASE, LEDGER_BASE + LEDGER_CAP)
            .map_err(|_| ())?;
        Ok(FlashRecordStore { flash })
    }

}

impl RecordStore for FlashRecordStore {
    fn capacity(&self) -> u32 {
        LEDGER_CAP
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        let mut f = self.flash.borrow_mut();
        if read_any(&mut f, LEDGER_BASE + off, buf).is_err() {
            buf.fill(0xFF);
        }
    }

    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        if off as usize + data.len() > LEDGER_CAP as usize {
            return Err(StoreError::OutOfRange);
        }
        let mut f = self.flash.borrow_mut();
        program_any(&mut f, LEDGER_BASE + off, data).map_err(|_| StoreError::WriteFailed)
    }
}

/// What the board remembers about its circle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Circle {
    pub params: CircleParams,
    pub seat: u8,
    pub circle_id: CircleId,
}

/// `CAH1 | len u16 LE | params | seat | circle id`, rewritten whole in its own sector.
pub fn save_circle(flash: &SharedFlash, c: &Circle) -> Result<(), ()> {
    let params = c.params.encode();
    let mut body = Vec::with_capacity(6 + params.len() + 1 + c.circle_id.len());
    body.extend_from_slice(CIRCLE_MAGIC);
    let len = (params.len() + 1 + c.circle_id.len()) as u16;
    body.extend_from_slice(&len.to_le_bytes());
    body.extend_from_slice(&params);
    body.push(c.seat);
    body.extend_from_slice(&c.circle_id);
    let mut f = flash.borrow_mut();
    erase_sectors(&mut f, CIRCLE_BASE, SECTOR)?;
    program_any(&mut f, CIRCLE_BASE, &body)
}

pub fn load_circle(flash: &SharedFlash) -> Option<Circle> {
    let mut head = [0u8; 6];
    let mut f = flash.borrow_mut();
    read_any(&mut f, CIRCLE_BASE, &mut head).ok()?;
    if &head[..4] != CIRCLE_MAGIC {
        return None;
    }
    let len = u16::from_le_bytes([head[4], head[5]]) as usize;
    let id_len = core::mem::size_of::<CircleId>();
    if len < 1 + id_len || len > 1024 {
        return None;
    }
    let mut body = vec![0u8; len];
    read_any(&mut f, CIRCLE_BASE + 6, &mut body).ok()?;
    let params_len = len - 1 - id_len;
    let params = CircleParams::decode(&body[..params_len]).ok()?;
    let seat = body[params_len];
    let circle_id: CircleId = body[params_len + 1..].try_into().ok()?;
    Some(Circle {
        params,
        seat,
        circle_id,
    })
}

/// Forget the circle: the record, the ledger, and the pad's sectors.
pub fn wipe(flash: &SharedFlash, pad_len: u32) -> Result<(), ()> {
    let mut f = flash.borrow_mut();
    erase_sectors(&mut f, CIRCLE_BASE, SECTOR)?;
    erase_sectors(&mut f, PAD_BASE, pad_len.max(SECTOR))?;
    f.erase(LEDGER_BASE, LEDGER_BASE + LEDGER_CAP).map_err(|_| ())
}
