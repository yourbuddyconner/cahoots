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
//!
//! The board holds [`PLACES`] circles, each in a place of its own: a record sector, a 1 MiB pad
//! region and a 64 KiB ledger, shared with no other circle. Place 0 is where the one circle of
//! earlier firmware lives (`pad`, `ledger` and `circle` in `partitions.csv`), so a board
//! flashed over keeps it. Places 1 to 3 have their record sectors from `0x41_3000`, their
//! ledgers from `0x42_0000` and their pads from `0x50_0000` (`circles`, `ledgers` and `pads`).

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
/// How many circles the board holds.
pub const PLACES: usize = 4;
const RECORDS_BASE: u32 = 0x41_3000;
const LEDGERS_BASE: u32 = 0x42_0000;
const PADS_BASE: u32 = 0x50_0000;

/// Where one place's record, pad and ledger are.
#[derive(Clone, Copy)]
pub struct Place {
    pub record: u32,
    pub pad: u32,
    pub ledger: u32,
}

/// Place `p`, from 0 to [`PLACES`] - 1. Every place's pad region is `PAD_CAP` and its ledger
/// `LEDGER_CAP`.
pub fn place(p: usize) -> Place {
    if p == 0 {
        return Place {
            record: CIRCLE_BASE,
            pad: PAD_BASE,
            ledger: LEDGER_BASE,
        };
    }
    let k = p as u32 - 1;
    Place {
        record: RECORDS_BASE + k * SECTOR,
        pad: PADS_BASE + k * PAD_CAP,
        ledger: LEDGERS_BASE + k * LEDGER_CAP,
    }
}

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

/// A place's pad region, sized to its circle's pad.
pub struct FlashPadStore {
    flash: SharedFlash,
    base: u32,
    len: u32,
}

impl FlashPadStore {
    /// Write a fresh pad into place `at`: erase its sectors, then program it.
    pub fn write(flash: SharedFlash, at: Place, pad: &[u8]) -> Result<Self, ()> {
        if pad.len() as u32 > PAD_CAP {
            return Err(());
        }
        {
            let mut f = flash.borrow_mut();
            erase_sectors(&mut f, at.pad, pad.len() as u32)?;
            program_any(&mut f, at.pad, pad)?;
        }
        Ok(FlashPadStore {
            flash,
            base: at.pad,
            len: pad.len() as u32,
        })
    }

    pub fn open(flash: SharedFlash, at: Place, len: u32) -> Result<Self, ()> {
        if len > PAD_CAP {
            return Err(());
        }
        Ok(FlashPadStore {
            flash,
            base: at.pad,
            len,
        })
    }
}

impl PadStore for FlashPadStore {
    fn len(&self) -> u32 {
        self.len
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        let mut f = self.flash.borrow_mut();
        if off.saturating_add(buf.len() as u32) > self.len
            || read_any(&mut f, self.base + off, buf).is_err()
        {
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
            program_any(&mut f, self.base + pos, &zeros[..n as usize])
                .map_err(|_| EraseError::VerifyFailed)?;
            pos += n;
        }
        Ok(())
    }

    fn erase_unit(&self) -> u32 {
        SECTOR
    }
}

/// A place's ledger region. Records program onto erased flash.
pub struct FlashRecordStore {
    flash: SharedFlash,
    base: u32,
}

impl FlashRecordStore {
    /// Erase the region of place `at` for a new circle.
    pub fn fresh(flash: SharedFlash, at: Place) -> Result<Self, ()> {
        flash
            .borrow_mut()
            .erase(at.ledger, at.ledger + LEDGER_CAP)
            .map_err(|_| ())?;
        Ok(Self::open(flash, at))
    }

    /// The region of place `at` as it is, for the circle already stored there.
    pub fn open(flash: SharedFlash, at: Place) -> Self {
        FlashRecordStore {
            flash,
            base: at.ledger,
        }
    }
}

impl RecordStore for FlashRecordStore {
    fn capacity(&self) -> u32 {
        LEDGER_CAP
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        let mut f = self.flash.borrow_mut();
        if off.saturating_add(buf.len() as u32) > LEDGER_CAP
            || read_any(&mut f, self.base + off, buf).is_err()
        {
            buf.fill(0xFF);
        }
    }

    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        if off as usize + data.len() > LEDGER_CAP as usize {
            return Err(StoreError::OutOfRange);
        }
        let mut f = self.flash.borrow_mut();
        program_any(&mut f, self.base + off, data).map_err(|_| StoreError::WriteFailed)
    }
}

/// Write circle `c` into the record sector of place `p`: `Circle::encode_record`'s `CAH3`,
/// rewritten whole in its own sector. The record must fit the 4 KiB sector, header included;
/// for n = 8 the manifest is under 2 KiB, so there is room, and this refuses a record over
/// the limit.
///
/// The seat key and the root are secrets: they go to flash here and nowhere else, never into
/// a log line, the trace or the screen.
pub fn save_circle(flash: &SharedFlash, p: usize, c: &Circle) -> Result<(), ()> {
    let body = c.encode_record().ok_or(())?;
    if body.len() > SECTOR as usize {
        return Err(());
    }
    let at = place(p).record;
    let mut f = flash.borrow_mut();
    erase_sectors(&mut f, at, SECTOR).and_then(|()| program_any(&mut f, at, &body))
}

/// The circle in place `p`, or `None` if there is none or the record does not parse.
///
/// A `CAH1` record, from before the seat key and the manifest, has another magic and loads
/// as `None`: the holder runs the ceremony again. The seat key cannot be worked out from
/// the pad, and the words cannot either, because spending erases pad bytes.
pub fn load_circle(flash: &SharedFlash, p: usize) -> Option<Circle> {
    let at = place(p).record;
    let mut head = [0u8; 6];
    let mut f = flash.borrow_mut();
    read_any(&mut f, at, &mut head).ok()?;
    let len = record_body_len(&head)?;
    let mut record = vec![0u8; 6 + len];
    record[..6].copy_from_slice(&head);
    read_any(&mut f, at + 6, &mut record[6..]).ok()?;
    let c = Circle::decode_record(&record);
    record.fill(0);
    c
}

/// Erase the record sector of place `p`, if it holds anything.
pub fn erase_circle(flash: &SharedFlash, p: usize) -> Result<(), ()> {
    let at = place(p).record;
    let mut f = flash.borrow_mut();
    let mut head = [0u8; 4];
    read_any(&mut f, at, &mut head)?;
    if head == [0xFF; 4] {
        return Ok(());
    }
    erase_sectors(&mut f, at, SECTOR)
}

/// Forget the circle in place `p`: its record first, so no record is ever left over a blank
/// pad, then the pad's sectors, then the ledger.
pub fn wipe(flash: &SharedFlash, p: usize, pad_len: u32) -> Result<(), ()> {
    erase_circle(flash, p)?;
    let at = place(p);
    let mut f = flash.borrow_mut();
    erase_sectors(&mut f, at.pad, pad_len.clamp(SECTOR, PAD_CAP))?;
    f.erase(at.ledger, at.ledger + LEDGER_CAP).map_err(|_| ())
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
