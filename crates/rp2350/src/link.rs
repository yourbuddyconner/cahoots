//! The three ways out of the board: USB to a laptop, UART0 up the pile, UART1 down it.
//!
//! The pile links are V1's: UART0 (GP0 TX, GP1 RX) is the link up and UART1 (GP4 TX, GP5 RX)
//! the link down, each RX with a pull-down so a peer's idle-high TX is the detect line.
//! Received bytes are taken by interrupt into a queue, because a frame can arrive while the
//! badge is busy with a two-second screen refresh.
//!
//! The link down is the UART when a peer holds it high and the USB port otherwise, so the
//! bottom card of a bench pile talks to the laptop over USB, as the Nano does, and every
//! card above it over its cable. On USB, `peer_present` is "a byte arrived in the last two
//! seconds" (the host sends a `Ping` frame twice a second), log lines travel as `Log` frames
//! on the same port, and nothing blocks when no host is listening.

use crate::buttons::gpio_high;
use crate::clock::{Duration, Instant};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::frame::{encode, Frame};
use cahoots_hal::pile::KIND_LOG;
use cahoots_hal::Link;
use core::cell::RefCell;
use critical_section::Mutex;
use rp235x_hal::gpio::bank0::{Gpio0, Gpio1, Gpio4, Gpio5};
use rp235x_hal::gpio::{FunctionUart, Pin, PullDown};
use rp235x_hal::pac::{interrupt, UART0, UART1};
use rp235x_hal::uart::{Reader, Writer};
use rp235x_hal::usb::UsbBus;
use usb_device::class_prelude::UsbBusAllocator;
use usb_device::device::UsbDevice;
use usbd_serial::SerialPort;

pub const PRESENT_FOR: Duration = Duration::from_secs(2);
const OUT_CAP: usize = 64 * 1024;
const IN_CAP: usize = 64 * 1024;

pub type Uart0Pins = (
    Pin<Gpio0, FunctionUart, PullDown>,
    Pin<Gpio1, FunctionUart, PullDown>,
);
pub type Uart1Pins = (
    Pin<Gpio4, FunctionUart, PullDown>,
    Pin<Gpio5, FunctionUart, PullDown>,
);

// ---- the UARTs ----

struct UartPort<D: rp235x_hal::uart::UartDevice, P: rp235x_hal::uart::ValidUartPinout<D>> {
    rx: Reader<D, P>,
    tx: Writer<D, P>,
    inbox: VecDeque<u8>,
}

static UART_UP: Mutex<RefCell<Option<UartPort<UART0, Uart0Pins>>>> =
    Mutex::new(RefCell::new(None));
static UART_DOWN: Mutex<RefCell<Option<UartPort<UART1, Uart1Pins>>>> =
    Mutex::new(RefCell::new(None));

pub fn init_uarts(up: (Reader<UART0, Uart0Pins>, Writer<UART0, Uart0Pins>), down: (Reader<UART1, Uart1Pins>, Writer<UART1, Uart1Pins>)) {
    critical_section::with(|cs| {
        let (mut rx, tx) = up;
        rx.enable_rx_interrupt();
        *UART_UP.borrow_ref_mut(cs) = Some(UartPort {
            rx,
            tx,
            inbox: VecDeque::new(),
        });
        let (mut rx, tx) = down;
        rx.enable_rx_interrupt();
        *UART_DOWN.borrow_ref_mut(cs) = Some(UartPort {
            rx,
            tx,
            inbox: VecDeque::new(),
        });
    });
    // SAFETY: the handlers below only touch the statics under the critical section.
    unsafe {
        cortex_m::peripheral::NVIC::unmask(interrupt::UART0_IRQ);
        cortex_m::peripheral::NVIC::unmask(interrupt::UART1_IRQ);
        cortex_m::peripheral::NVIC::unmask(interrupt::USBCTRL_IRQ);
    }
}

impl<D: rp235x_hal::uart::UartDevice, P: rp235x_hal::uart::ValidUartPinout<D>> UartPort<D, P> {
    /// Drain the FIFO into the inbox. Called from the interrupt and before every read.
    fn pull(&mut self) {
        let mut buf = [0u8; 32];
        loop {
            let n = match self.rx.read_raw(&mut buf) {
                Ok(n) => n,
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => e.discarded.len(),
            };
            if n == 0 {
                break;
            }
            self.inbox.extend(&buf[..n]);
            while self.inbox.len() > IN_CAP {
                self.inbox.pop_front();
            }
        }
    }
}

#[interrupt]
fn UART0_IRQ() {
    critical_section::with(|cs| {
        if let Some(p) = UART_UP.borrow_ref_mut(cs).as_mut() {
            p.pull();
        }
    });
}

#[interrupt]
fn UART1_IRQ() {
    critical_section::with(|cs| {
        if let Some(p) = UART_DOWN.borrow_ref_mut(cs).as_mut() {
            p.pull();
        }
    });
}

/// Whether a peer's TX holds an RX pin: high when idle, and high again within one byte
/// time whenever it is sending, since every byte ends in a stop bit. A single low sample
/// is a data bit, not an empty connector, so the pin is watched for a little over a byte
/// at `PILE_BAUD` before the peer is called absent.
fn line_present(gpio: u8) -> bool {
    let start = Instant::now();
    loop {
        if gpio_high(gpio) {
            return true;
        }
        if start.elapsed() > Duration::from_micros(200) {
            return false;
        }
    }
}

/// The pile link up: UART0.
pub struct UpLink;

impl Link for UpLink {
    fn send(&mut self, frame: &[u8]) {
        uart_send(&UART_UP, frame);
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        critical_section::with(|cs| {
            let mut port = UART_UP.borrow_ref_mut(cs);
            let Some(p) = port.as_mut() else { return 0 };
            p.pull();
            take(&mut p.inbox, buf)
        })
    }
    /// GP1 held high by the peer's TX.
    fn peer_present(&self) -> bool {
        line_present(1)
    }
}

/// Write a frame to a UART with interrupts on between FIFO fills: each critical section
/// hands the FIFO as many bytes as it takes and returns, so the other UART's interrupt
/// keeps draining while a long frame goes out.
fn uart_send<D: rp235x_hal::uart::UartDevice, P: rp235x_hal::uart::ValidUartPinout<D>>(
    port: &Mutex<RefCell<Option<UartPort<D, P>>>>,
    frame: &[u8],
) {
    let mut rest = frame;
    while !rest.is_empty() {
        let done = critical_section::with(|cs| {
            let mut guard = port.borrow_ref_mut(cs);
            let Some(p) = guard.as_mut() else { return true };
            if let Ok(remaining) = p.tx.write_raw(rest) {
                rest = remaining;
            }
            false
        });
        if done {
            return;
        }
    }
}

fn take(q: &mut VecDeque<u8>, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match q.pop_front() {
            Some(b) => {
                buf[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    n
}

fn uart_down_present() -> bool {
    line_present(5)
}

// ---- USB ----

struct UsbPort {
    dev: UsbDevice<'static, UsbBus>,
    serial: SerialPort<'static, UsbBus>,
    pending: VecDeque<u8>,
    inbox: VecDeque<u8>,
    logs: VecDeque<Vec<u8>>,
    last_rx: Option<Instant>,
    seq: u32,
    dropped: u32,
}

static mut USB_BUS: Option<UsbBusAllocator<UsbBus>> = None;
static USB: Mutex<RefCell<Option<UsbPort>>> = Mutex::new(RefCell::new(None));

// SAFETY: the USB device is only ever touched inside the critical section that guards
// `USB`, from the main thread and the USB interrupt.
unsafe impl Send for UsbPort {}

pub fn init_usb(bus: UsbBus) {
    use usb_device::device::{StringDescriptors, UsbDeviceBuilder, UsbVidPid};
    // SAFETY: written once, before the interrupt is unmasked, and never again.
    let alloc: &'static UsbBusAllocator<UsbBus> = unsafe {
        USB_BUS = Some(UsbBusAllocator::new(bus));
        #[allow(static_mut_refs)]
        USB_BUS.as_ref().unwrap()
    };
    let serial = SerialPort::new(alloc);
    let dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x2E8A, 0x000A))
        .strings(&[StringDescriptors::default()
            .manufacturer("Cahoots")
            .product("Cahoots badge (Pico 2)")
            .serial_number("pico2")])
        .unwrap()
        .max_packet_size_0(64)
        .unwrap()
        .device_class(2)
        .build();
    critical_section::with(|cs| {
        *USB.borrow_ref_mut(cs) = Some(UsbPort {
            dev,
            serial,
            pending: VecDeque::new(),
            inbox: VecDeque::new(),
            logs: VecDeque::new(),
            last_rx: None,
            seq: 0,
            dropped: 0,
        })
    });
}

impl UsbPort {
    fn queue_logs(&mut self) {
        while let Some(l) = self.logs.pop_front() {
            self.pending.extend(l);
        }
        while self.pending.len() > OUT_CAP {
            self.pending.pop_front();
            self.dropped = self.dropped.wrapping_add(1);
        }
    }

    /// Run the device, take what arrived, push what is queued.
    fn service(&mut self) {
        let _ = self.dev.poll(&mut [&mut self.serial]);
        let mut buf = [0u8; 64];
        loop {
            match self.serial.read(&mut buf) {
                Ok(n) if n > 0 => {
                    self.inbox.extend(&buf[..n]);
                    self.last_rx = Some(Instant::now());
                    while self.inbox.len() > IN_CAP {
                        self.inbox.pop_front();
                    }
                }
                _ => break,
            }
        }
        while !self.pending.is_empty() {
            let (head, _) = self.pending.as_slices();
            let chunk = &head[..head.len().min(64)];
            match self.serial.write(chunk) {
                Ok(n) if n > 0 => {
                    self.pending.drain(..n);
                }
                _ => break,
            }
        }
        let _ = self.serial.flush();
    }

    fn present(&self) -> bool {
        self.last_rx
            .map(|t| t.elapsed() < PRESENT_FOR)
            .unwrap_or(false)
    }
}

#[interrupt]
fn USBCTRL_IRQ() {
    with_usb(|p| p.service());
}

fn with_usb<R>(f: impl FnOnce(&mut UsbPort) -> R) -> Option<R> {
    critical_section::with(|cs| USB.borrow_ref_mut(cs).as_mut().map(f))
}

/// Queue one log line as a `Log` frame on the USB port. Safe to call from anywhere after
/// [`init_usb`].
pub fn log_line(s: String) {
    crate::store::trace(&s);
    with_usb(|p| {
        let f = Frame {
            kind: KIND_LOG,
            seq: p.seq,
            payload: s.as_bytes(),
        };
        p.seq = p.seq.wrapping_add(1);
        p.logs.push_back(encode(&f));
        p.queue_logs();
        p.service();
    });
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => {
        $crate::link::log_line(alloc::format!($($t)*))
    };
}

/// Push queued USB bytes out. Called from every wait in the badge.
pub fn usb_pump() {
    with_usb(|p| {
        p.queue_logs();
        p.service();
    });
}

/// The link down: UART1 when a card is below, the USB port otherwise.
pub struct DownLink;

impl Link for DownLink {
    fn send(&mut self, frame: &[u8]) {
        if uart_down_present() {
            uart_send(&UART_DOWN, frame);
        } else {
            with_usb(|p| {
                p.queue_logs();
                p.pending.extend(frame);
                p.queue_logs();
                p.service();
            });
        }
    }

    fn recv(&mut self, buf: &mut [u8]) -> usize {
        if uart_down_present() {
            critical_section::with(|cs| {
                let mut port = UART_DOWN.borrow_ref_mut(cs);
                let Some(p) = port.as_mut() else { return 0 };
                p.pull();
                take(&mut p.inbox, buf)
            })
        } else {
            with_usb(|p| {
                p.queue_logs();
                p.service();
                take(&mut p.inbox, buf)
            })
            .unwrap_or(0)
        }
    }

    fn peer_present(&self) -> bool {
        uart_down_present() || with_usb(|p| p.present()).unwrap_or(false)
    }

    fn flush(&mut self) -> bool {
        if uart_down_present() {
            return true;
        }
        with_usb(|p| {
            p.queue_logs();
            p.service();
            p.pending.is_empty() || !p.present()
        })
        .unwrap_or(true)
    }
}

/// From the panic handler: write one last `Log` frame straight to the USB port, with a
/// bounded spin so a host that is not reading cannot hang the handler.
pub fn emergency_write(bytes: &[u8]) {
    critical_section::with(|cs| {
        let Ok(mut port) = USB.borrow(cs).try_borrow_mut() else {
            return;
        };
        let Some(p) = port.as_mut() else { return };
        p.pending.clear();
        p.pending.extend(bytes);
        for _ in 0..200_000u32 {
            p.service();
            if p.pending.is_empty() {
                break;
            }
        }
    });
}
