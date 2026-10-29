//! The pad, the ledger and the circle record on the Pico 2's 4 MiB flash, by offset, above
//! the 2 MiB the program may use (`memory.x`).
//!
//! The chip runs from that flash (XIP), so writing it means handing the QSPI bus to the
//! boot ROM's flash routines from a function that lives in RAM, with interrupts off:
//! connect, leave XIP, erase or program, flush the cache, and return to XIP. Reads are
//! plain memory reads through the XIP window.
//!
//! NOR flash erases to `0xFF` by the 4 KiB sector and programs bits from 1 to 0 by the
//! 256-byte page. So the pad is written once onto erased sectors, and `PadStore::erase` of
//! a spent range programs zeros over it with read-modify-write at the edges: the trait's
//! "reads back as zero" at byte granularity, with no sector ever re-erased under a live pad.
//! The ledger's 16-byte records program onto `0xFF` as the ledger appends. The circle record
//! is small and rewritten whole, in the format `cahoots_badge::Circle::encode_record` writes
//! (`CAH3`), the same as the Nano's.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_badge::board::record_body_len;
use cahoots_badge::Circle;
use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{EraseError, PadStore, StoreError};
use alloc::collections::VecDeque;
use core::cell::RefCell;
use core::sync::atomic::Ordering;
use critical_section::Mutex;
use rp235x_hal::rom_data;

const XIP_BASE: u32 = 0x1000_0000;
pub const PAD_BASE: u32 = 0x20_0000;
pub const PAD_CAP: u32 = 0x10_0000;
pub const LEDGER_BASE: u32 = 0x30_0000;
pub const LEDGER_CAP: u32 = 0x1_0000;
pub const CIRCLE_BASE: u32 = 0x31_0000;
/// One sector after the circle record: the message of the last panic, shown at the next boot.
pub const PANIC_BASE: u32 = 0x31_1000;
/// One sector after the panic record: the last log lines and screens before a panic,
/// written with the panic record, readable with `picotool save -r`.
pub const TRACE_BASE: u32 = 0x31_2000;
pub const SECTOR: u32 = 4096;
const PAGE: u32 = 256;

/// The boot ROM's flash routines, looked up before the bus is handed over, so nothing in
/// the RAM function touches flash.
struct RomFns {
    connect: unsafe extern "C" fn(),
    exit_xip: unsafe extern "C" fn(),
    erase: Option<unsafe extern "C" fn(u32, usize, u32, u8)>,
    program: Option<unsafe extern "C" fn(u32, *const u8, usize)>,
    flush: unsafe extern "C" fn(),
    enter_xip: unsafe extern "C" fn(),
}

fn rom_fns(erase: bool, program: bool) -> RomFns {
    RomFns {
        connect: rom_data::connect_internal_flash::ptr(),
        exit_xip: rom_data::flash_exit_xip::ptr(),
        erase: erase.then(rom_data::flash_range_erase::ptr),
        program: program.then(rom_data::flash_range_program::ptr),
        flush: rom_data::flash_flush_cache::ptr(),
        enter_xip: rom_data::flash_enter_cmd_xip::ptr(),
    }
}

/// The part that runs with flash unreadable. It lives in RAM and calls only ROM.
#[link_section = ".data.ram_func"]
#[inline(never)]
unsafe fn in_ram(f: &RomFns, addr: u32, len: usize, data: *const u8) {
    (f.connect)();
    (f.exit_xip)();
    if let Some(erase) = f.erase {
        erase(addr, len, 1 << 31, 0);
    }
    if let Some(program) = f.program {
        program(addr, data, len);
    }
    (f.flush)();
    (f.enter_xip)();
}

fn flash_op(f: &RomFns, addr: u32, len: usize, data: *const u8) {
    critical_section::with(|_| {
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        // SAFETY: interrupts are off, nothing else runs, `addr` and `len` are aligned by
        // the callers, and `data` points into RAM.
        unsafe { in_ram(f, addr, len, data) };
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
    });
}

/// Read any byte range through the XIP window.
pub fn read_any(addr: u32, out: &mut [u8]) {
    // SAFETY: the flash is memory-mapped at XIP_BASE and the callers stay inside it.
    let src = unsafe { core::slice::from_raw_parts((XIP_BASE + addr) as *const u8, out.len()) };
    out.copy_from_slice(src);
}

/// Program bytes over any range. Only clears bits; the bytes around the edges are rewritten
/// with their current value, which NOR flash accepts.
pub fn program_any(addr: u32, data: &[u8]) -> Result<(), ()> {
    if data.is_empty() {
        return Ok(());
    }
    let end = addr + data.len() as u32;
    let a0 = addr & !(PAGE - 1);
    let a1 = (end + PAGE - 1) & !(PAGE - 1);
    let mut buf = vec![0u8; (a1 - a0) as usize];
    read_any(a0, &mut buf);
    let off = (addr - a0) as usize;
    buf[off..off + data.len()].copy_from_slice(data);
    let fns = rom_fns(false, true);
    flash_op(&fns, a0, buf.len(), buf.as_ptr());
    let mut back = vec![0u8; data.len()];
    read_any(addr, &mut back);
    buf.fill(0);
    if back == data {
        Ok(())
    } else {
        Err(())
    }
}

/// Erase whole sectors covering `[addr, addr + len)` to `0xFF`.
pub fn erase_sectors(addr: u32, len: u32) -> Result<(), ()> {
    let from = addr & !(SECTOR - 1);
    let to = (addr + len + SECTOR - 1) & !(SECTOR - 1);
    let fns = rom_fns(true, false);
    flash_op(&fns, from, (to - from) as usize, core::ptr::null());
    let mut check = [0u8; 64];
    read_any(from, &mut check);
    if check.iter().all(|&b| b == 0xFF) {
        Ok(())
    } else {
        Err(())
    }
}

/// The pad region, sized to this circle's pad.
pub struct FlashPadStore {
    len: u32,
}

impl FlashPadStore {
    /// Write a fresh pad: erase its sectors, then program it.
    pub fn write(pad: &[u8]) -> Result<Self, ()> {
        if pad.len() as u32 > PAD_CAP {
            return Err(());
        }
        erase_sectors(PAD_BASE, pad.len() as u32)?;
        program_any(PAD_BASE, pad)?;
        Ok(FlashPadStore {
            len: pad.len() as u32,
        })
    }

    pub fn open(len: u32) -> Self {
        FlashPadStore { len }
    }
}

impl PadStore for FlashPadStore {
    fn len(&self) -> u32 {
        self.len
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        if off.saturating_add(buf.len() as u32) > self.len {
            buf.fill(0);
            return;
        }
        read_any(PAD_BASE + off, buf);
    }

    fn erase(&mut self, off: u32, len: u32) -> Result<(), EraseError> {
        let end = off.checked_add(len).ok_or(EraseError::OutOfRange)?;
        if end > self.len {
            return Err(EraseError::OutOfRange);
        }
        let zeros = vec![0u8; len.min(4096) as usize];
        let mut pos = off;
        while pos < end {
            let n = (end - pos).min(4096);
            program_any(PAD_BASE + pos, &zeros[..n as usize])
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
pub struct FlashRecordStore;

impl FlashRecordStore {
    /// Erase the region for a new circle.
    pub fn fresh() -> Result<Self, ()> {
        erase_sectors(LEDGER_BASE, LEDGER_CAP)?;
        Ok(FlashRecordStore)
    }

    /// The region as it is, for the circle already stored.
    pub fn open() -> Self {
        FlashRecordStore
    }
}

impl RecordStore for FlashRecordStore {
    fn capacity(&self) -> u32 {
        LEDGER_CAP
    }

    fn read(&self, off: u32, buf: &mut [u8]) {
        if off.saturating_add(buf.len() as u32) > LEDGER_CAP {
            buf.fill(0xFF);
            return;
        }
        read_any(LEDGER_BASE + off, buf);
    }

    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        if off as usize + data.len() > LEDGER_CAP as usize {
            return Err(StoreError::OutOfRange);
        }
        program_any(LEDGER_BASE + off, data).map_err(|_| StoreError::WriteFailed)
    }
}

/// The circle record, `Circle::encode_record`'s `CAH3` (or the older `CAH2`, which still
/// loads), rewritten whole in its own sector: the Nano's record. The seat key and the root
/// are secrets: they go to flash here and nowhere else.
pub fn save_circle(c: &Circle) -> Result<(), ()> {
    let body = c.encode_record().ok_or(())?;
    if body.len() > SECTOR as usize {
        return Err(());
    }
    erase_sectors(CIRCLE_BASE, SECTOR).and_then(|()| program_any(CIRCLE_BASE, &body))
}

/// The stored circle, or `None` if there is none or the record does not parse.
pub fn load_circle() -> Option<Circle> {
    let mut head = [0u8; 6];
    read_any(CIRCLE_BASE, &mut head);
    let len = record_body_len(&head)?;
    let mut record = vec![0u8; 6 + len];
    record[..6].copy_from_slice(&head);
    read_any(CIRCLE_BASE + 6, &mut record[6..]);
    let c = Circle::decode_record(&record);
    record.fill(0);
    c
}

/// Forget the circle: the record, the ledger, and the pad's sectors.
pub fn wipe(pad_len: u32) -> Result<(), ()> {
    erase_sectors(CIRCLE_BASE, SECTOR)?;
    erase_sectors(PAD_BASE, pad_len.max(SECTOR))?;
    erase_sectors(LEDGER_BASE, LEDGER_CAP)
}

const PANIC_MAGIC: &[u8; 4] = b"CAHP";

/// Record a panic message so the next boot can show it, and the trace behind it.
pub fn save_panic(msg: &[u8]) {
    let msg = &msg[..msg.len().min(SECTOR as usize - 6)];
    let mut body = Vec::with_capacity(6 + msg.len());
    body.extend_from_slice(PANIC_MAGIC);
    body.extend_from_slice(&(msg.len() as u16).to_le_bytes());
    body.extend_from_slice(msg);
    if erase_sectors(PANIC_BASE, SECTOR).is_ok() {
        let _ = program_any(PANIC_BASE, &body);
    }
    let trace: Vec<u8> = critical_section::with(|cs| TRACE.borrow_ref(cs).iter().copied().collect());
    if !trace.is_empty() && erase_sectors(TRACE_BASE, SECTOR).is_ok() {
        let _ = program_any(TRACE_BASE, &trace[..trace.len().min(SECTOR as usize)]);
    }
}

/// The last panic's message, if one was recorded, erased once read.
pub fn take_panic() -> Option<String> {
    let mut head = [0u8; 6];
    read_any(PANIC_BASE, &mut head);
    if &head[..4] != PANIC_MAGIC {
        return None;
    }
    let len = u16::from_le_bytes([head[4], head[5]]) as usize;
    if len == 0 || len > SECTOR as usize - 6 {
        return None;
    }
    let mut body = vec![0u8; len];
    read_any(PANIC_BASE + 6, &mut body);
    let _ = erase_sectors(PANIC_BASE, SECTOR);
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// The last lines of the log and the screens, in RAM. The Nano writes each line to flash
/// as it goes; here a flash program holds interrupts off for milliseconds and a UART FIFO
/// holds a third of one, so the trace stays in RAM and reaches flash only with a panic.
static TRACE: Mutex<RefCell<VecDeque<u8>>> = Mutex::new(RefCell::new(VecDeque::new()));
const TRACE_CAP: usize = SECTOR as usize;

/// Append one line to the trace, dropping the oldest when it is full.
pub fn trace(line: &str) {
    critical_section::with(|cs| {
        let mut t = TRACE.borrow_ref_mut(cs);
        t.extend(line.bytes().filter(|b| *b != 0xFF));
        t.push_back(b'\n');
        while t.len() > TRACE_CAP {
            t.pop_front();
        }
    });
}
