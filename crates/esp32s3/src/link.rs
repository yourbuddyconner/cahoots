//! The link to the laptop: the ESP32-S3's USB-Serial-JTAG peripheral as a `Link`.
//!
//! The peripheral lives in a static, and [`UsbLink`] is a handle to it, so the firmware can
//! make a fresh handle after a ceremony consumed the last one. Three things a USB port needs
//! that a cable does not: `peer_present` is "a byte arrived in the last two seconds" (the host
//! sends a `Ping` frame twice a second); log lines travel on the same port as `Log` frames,
//! queued here and drained ahead of other traffic; and nothing blocks when no host is
//! listening, since the output queue is capped and the oldest bytes are dropped.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::frame::{encode, Frame};
use cahoots_hal::pile::KIND_LOG;
use cahoots_hal::Link;
use core::cell::RefCell;
use critical_section::Mutex;
use esp_hal::peripherals::USB_DEVICE;
use esp_hal::time::{Duration, Instant};
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_hal::Blocking;

pub const PRESENT_FOR: Duration = Duration::from_secs(2);
const OUT_CAP: usize = 64 * 1024;

struct Port {
    dev: UsbSerialJtag<'static, Blocking>,
    pending: VecDeque<u8>,
    logs: VecDeque<Vec<u8>>,
    last_rx: Option<Instant>,
    seq: u32,
    dropped: u32,
}

static PORT: Mutex<RefCell<Option<Port>>> = Mutex::new(RefCell::new(None));

pub fn init(usb: USB_DEVICE<'static>) {
    critical_section::with(|cs| {
        *PORT.borrow_ref_mut(cs) = Some(Port {
            dev: UsbSerialJtag::new(usb),
            pending: VecDeque::new(),
            logs: VecDeque::new(),
            last_rx: None,
            seq: 0,
            dropped: 0,
        })
    });
}

fn with<R>(f: impl FnOnce(&mut Port) -> R) -> Option<R> {
    critical_section::with(|cs| PORT.borrow_ref_mut(cs).as_mut().map(f))
}

impl Port {
    fn queue_logs(&mut self) {
        while let Some(l) = self.logs.pop_front() {
            self.pending.extend(l);
        }
        while self.pending.len() > OUT_CAP {
            self.pending.pop_front();
            self.dropped = self.dropped.wrapping_add(1);
        }
    }

    fn push_out(&mut self) {
        while let Some(&b) = self.pending.front() {
            match self.dev.write_byte_nb(b) {
                Ok(()) => {
                    self.pending.pop_front();
                }
                Err(_) => break,
            }
        }
        let _ = self.dev.flush_tx_nb();
    }

    fn pull_in(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() {
            match self.dev.read_byte() {
                Ok(b) => {
                    buf[n] = b;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        if n > 0 {
            self.last_rx = Some(Instant::now());
        }
        n
    }

    fn present(&self) -> bool {
        self.last_rx
            .map(|t| t.elapsed() < PRESENT_FOR)
            .unwrap_or(false)
    }
}

/// Queue one log line as a `Log` frame. Safe to call from anywhere after [`init`].
pub fn log_line(s: String) {
    with(|p| {
        let f = Frame {
            kind: KIND_LOG,
            seq: p.seq,
            payload: s.as_bytes(),
        };
        p.seq = p.seq.wrapping_add(1);
        p.logs.push_back(encode(&f));
        p.queue_logs();
        p.push_out();
    });
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => {
        $crate::link::log_line(alloc::format!($($t)*))
    };
}

/// A handle to the USB port as a touch link.
pub struct UsbLink;

impl Link for UsbLink {
    fn send(&mut self, frame: &[u8]) {
        with(|p| {
            p.queue_logs();
            p.pending.extend(frame);
            p.queue_logs();
            p.push_out();
        });
    }

    fn recv(&mut self, buf: &mut [u8]) -> usize {
        with(|p| {
            p.queue_logs();
            p.push_out();
            p.pull_in(buf)
        })
        .unwrap_or(0)
    }

    fn peer_present(&self) -> bool {
        with(|p| p.present()).unwrap_or(false)
    }

    fn flush(&mut self) -> bool {
        with(|p| {
            p.queue_logs();
            p.push_out();
            p.pending.is_empty() || !p.present()
        })
        .unwrap_or(true)
    }
}

/// A connector with nothing plugged in.
pub struct Absent;

impl Link for Absent {
    fn send(&mut self, _frame: &[u8]) {}
    fn recv(&mut self, _buf: &mut [u8]) -> usize {
        0
    }
    fn peer_present(&self) -> bool {
        false
    }
}

/// From the panic handler: write one last `Log` frame straight to the port, blocking.
pub fn emergency_write(bytes: &[u8]) {
    with(|p| {
        let _ = p.dev.write(bytes);
        let _ = p.dev.flush_tx();
    });
}
