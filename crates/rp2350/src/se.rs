//! A sign of life from the SE051 on I2C0, V1's pins (SDA GP8, SCL GP9): a scan of the bus,
//! then a T=1 over I2C S(RESYNCH request) frame to the chip's address, and whatever it
//! answers. The chip ignores reads until it has a frame to give, so the scan alone can miss
//! it; the resync is the real test. The frame's CRC follows UM11225 as read here (CRC-16,
//! polynomial 0x1021, initial 0xFFFF, low byte first); a wrong CRC still gets an answer, an
//! R-block that says so, which proves the chip is there just as well.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::{ErrorKind, I2c, NoAcknowledgeSource};

/// The SE05x family's I2C address.
pub const ADDR: u8 = 0x48;
/// Node address byte, host to secure element.
const NAD: u8 = 0x5A;
/// S-block, RESYNCH request.
const PCB_RESYNCH: u8 = 0xC0;

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn nack(e: &impl embedded_hal::i2c::Error) -> bool {
    matches!(e.kind(), ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address))
}

/// Every address that acknowledges a one-byte read.
pub fn scan<I: I2c>(i2c: &mut I) -> Vec<u8> {
    let mut found = Vec::new();
    let mut buf = [0u8; 1];
    for addr in 0x08u8..=0x77 {
        if i2c.read(addr, &mut buf).is_ok() {
            found.push(addr);
        }
    }
    found
}

/// Send a resync and read the reply, polling for up to about 100 ms.
pub fn resync<I: I2c, D: DelayNs>(i2c: &mut I, delay: &mut D) -> String {
    let mut frame = [NAD, PCB_RESYNCH, 0, 0, 0];
    let crc = crc16(&frame[..3]);
    frame[3] = crc as u8;
    frame[4] = (crc >> 8) as u8;
    match i2c.write(ADDR, &frame) {
        Ok(()) => {}
        Err(e) if nack(&e) => return format!("se051: no device at {ADDR:#04x} (address not acknowledged)"),
        Err(e) => return format!("se051: write failed: {e:?}"),
    }
    let mut head = [0u8; 3];
    for _ in 0..100 {
        delay.delay_ms(1);
        match i2c.read(ADDR, &mut head) {
            Ok(()) => {
                let len = head[2] as usize;
                let mut rest = alloc::vec![0u8; (len + 2).min(64)];
                let tail = match i2c.read(ADDR, &mut rest) {
                    Ok(()) => hex(&rest),
                    Err(e) => format!("(tail read failed: {e:?})"),
                };
                let kind = match head[1] {
                    0xE0 => "resynch response",
                    p if p & 0x80 == 0x80 && p & 0x40 == 0 => "R-block (an error report)",
                    _ => "unexpected block",
                };
                return format!("se051: alive, replied {} {} ({kind})", hex(&head), tail);
            }
            Err(e) if nack(&e) => continue,
            Err(e) => return format!("se051: read failed: {e:?}"),
        }
    }
    format!("se051: sent resync to {ADDR:#04x} (acknowledged) but nothing came back in 100 ms")
}

/// The lines for the boot log.
pub fn probe<I: I2c, D: DelayNs>(i2c: &mut I, delay: &mut D) -> Vec<String> {
    let found = scan(i2c);
    let scan_line = if found.is_empty() {
        "i2c: scan found nothing that answers a read (the SE051 may still be there)".into()
    } else {
        format!("i2c: scan found {}", found.iter().map(|a| format!("{a:#04x}")).collect::<Vec<_>>().join(" "))
    };
    alloc::vec![scan_line, resync(i2c, delay)]
}
