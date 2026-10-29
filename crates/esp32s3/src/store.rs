//! The pad, the ledger and the circle record on the board's NOR flash, by address, in the
//! regions `partitions.csv` reserves.
//!
//! NOR flash erases to `0xFF` by the 4 KiB sector and programs bits from 1 to 0 by the word.
//! So the pad is written once onto erased sectors, and `PadStore::erase` of a spent range
//! programs zeros over it, word by word with read-modify-write at the edges: the trait's
//! "reads back as zero" at byte granularity, with no sector ever re-erased under a live pad.
//! The ledger's 16-byte records program onto `0xFF` as the ledger appends. The circle record
//! is small and rewritten whole, and carries the check words.
//!
//! One `FlashStorage` exists per chip, so the stores share it through an `Rc<RefCell>`.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_badge::board::record_body_len;
use cahoots_badge::Circle;
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
/// One sector after the circle record: the message of the last panic, shown at the next boot.
pub const PANIC_BASE: u32 = 0x41_1000;
/// One sector after the panic record: every log line of the current boot, so a stuck board
/// can be read with `espflash read-flash` when neither the screen nor USB says anything.
pub const TRACE_BASE: u32 = 0x41_2000;
pub const SECTOR: u32 = 4096;
const WORD: u32 = 4;

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

    /// The region as it is, for the circle already stored.
    pub fn open(flash: SharedFlash) -> Self {
        FlashRecordStore { flash }
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

/// The circle record, `Circle::encode_record`'s `CAH3` (or the older `CAH2`, which still
/// loads), rewritten whole in its own sector. The record must fit the 4 KiB sector, header
/// included; for n = 8 the manifest is under 2 KiB, so there is room, and `save_circle`
/// refuses a record over the limit.
///
/// The seat key and the root are secrets: they go to flash here and nowhere else, never into
/// a log line, the trace or the screen.
pub fn save_circle(flash: &SharedFlash, c: &Circle) -> Result<(), ()> {
    let body = c.encode_record().ok_or(())?;
    if body.len() > SECTOR as usize {
        return Err(());
    }
    let mut f = flash.borrow_mut();
    erase_sectors(&mut f, CIRCLE_BASE, SECTOR).and_then(|()| program_any(&mut f, CIRCLE_BASE, &body))
}

/// The stored circle, or `None` if there is none or the record does not parse.
///
/// A `CAH1` record, from before the seat key and the manifest, has another magic and loads
/// as `None`: the holder runs the ceremony again. The seat key cannot be worked out from
/// the pad, and the words cannot either, because spending erases pad bytes.
pub fn load_circle(flash: &SharedFlash) -> Option<Circle> {
    let mut head = [0u8; 6];
    let mut f = flash.borrow_mut();
    read_any(&mut f, CIRCLE_BASE, &mut head).ok()?;
    let len = record_body_len(&head)?;
    let mut record = vec![0u8; 6 + len];
    record[..6].copy_from_slice(&head);
    read_any(&mut f, CIRCLE_BASE + 6, &mut record[6..]).ok()?;
    let c = Circle::decode_record(&record);
    record.fill(0);
    c
}

/// Forget the circle: the record, the ledger, and the pad's sectors.
pub fn wipe(flash: &SharedFlash, pad_len: u32) -> Result<(), ()> {
    let mut f = flash.borrow_mut();
    erase_sectors(&mut f, CIRCLE_BASE, SECTOR)?;
    erase_sectors(&mut f, PAD_BASE, pad_len.max(SECTOR))?;
    f.erase(LEDGER_BASE, LEDGER_BASE + LEDGER_CAP)
        .map_err(|_| ())
}

const PANIC_MAGIC: &[u8; 4] = b"CAHP";

/// Record a panic message so the next boot can show it. Called from the panic handler with
/// a flash handle of its own, since the shared one may be the thing that panicked.
pub fn save_panic(f: &mut FlashStorage<'_>, msg: &[u8]) {
    let msg = &msg[..msg.len().min(SECTOR as usize - 6)];
    let mut body = Vec::with_capacity(6 + msg.len());
    body.extend_from_slice(PANIC_MAGIC);
    body.extend_from_slice(&(msg.len() as u16).to_le_bytes());
    body.extend_from_slice(msg);
    if erase_sectors(f, PANIC_BASE, SECTOR).is_ok() {
        let _ = program_any(f, PANIC_BASE, &body);
    }
}

/// The last panic's message, if one was recorded, erased once read.
pub fn take_panic(flash: &SharedFlash) -> Option<String> {
    let mut head = [0u8; 6];
    let mut f = flash.borrow_mut();
    read_any(&mut f, PANIC_BASE, &mut head).ok()?;
    if &head[..4] != PANIC_MAGIC {
        return None;
    }
    let len = u16::from_le_bytes([head[4], head[5]]) as usize;
    if len == 0 || len > SECTOR as usize - 6 {
        return None;
    }
    let mut body = vec![0u8; len];
    read_any(&mut f, PANIC_BASE + 6, &mut body).ok()?;
    let _ = erase_sectors(&mut f, PANIC_BASE, SECTOR);
    Some(String::from_utf8_lossy(&body).into_owned())
}

static TRACE_OFF: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
/// The board's one flash handle, for the trace and the panic record. A second handle on the
/// flash peripheral hangs the chip, so everything goes through this one.
static TRACE_FLASH: core::sync::atomic::AtomicPtr<RefCell<FlashStorage<'static>>> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

fn trace_flash() -> Option<&'static RefCell<FlashStorage<'static>>> {
    let p = TRACE_FLASH.load(core::sync::atomic::Ordering::Acquire);
    // SAFETY: set once from the Rc the board keeps for its whole life; never freed.
    unsafe { p.as_ref() }
}

/// Erase the trace sector and remember the flash handle. `flash` must live for the rest
/// of the program, which the board's handle does.
pub fn trace_begin(flash: &SharedFlash) {
    TRACE_FLASH.store(
        Rc::as_ptr(flash) as *mut RefCell<FlashStorage<'static>>,
        core::sync::atomic::Ordering::Release,
    );
    let mut f = flash.borrow_mut();
    if erase_sectors(&mut f, TRACE_BASE, SECTOR).is_ok() {
        TRACE_OFF.store(0, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Append one line to the trace sector, if `trace_begin` ran, the flash handle is free,
/// and there is room.
pub fn trace(line: &str) {
    use core::sync::atomic::Ordering;
    let off = TRACE_OFF.load(Ordering::Relaxed);
    if off == u32::MAX {
        return;
    }
    let Some(cell) = trace_flash() else { return };
    let Ok(mut f) = cell.try_borrow_mut() else {
        return;
    };
    let mut bytes: Vec<u8> = line.bytes().filter(|b| *b != 0xFF).collect();
    bytes.push(b'\n');
    if off as usize + bytes.len() > SECTOR as usize {
        return;
    }
    if program_any(&mut f, TRACE_BASE + off, &bytes).is_ok() {
        TRACE_OFF.store(off + bytes.len() as u32, Ordering::Relaxed);
    }
}

/// Record a panic through the board's flash handle, if it is not the thing that panicked.
pub fn save_panic_shared(msg: &[u8]) {
    let Some(cell) = trace_flash() else { return };
    let Ok(mut f) = cell.try_borrow_mut() else {
        return;
    };
    save_panic(&mut f, msg);
}
