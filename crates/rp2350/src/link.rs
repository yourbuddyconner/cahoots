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
use core::sync::atomic::{AtomicU32, Ordering};
use critical_section::Mutex;
use rp235x_hal::gpio::bank0::{Gpio0, Gpio1, Gpio4, Gpio5};
use rp235x_hal::gpio::{FunctionUart, Pin, PullDown};
use rp235x_hal::pac::{interrupt, UART0, UART1};
use rp235x_hal::uart::{Reader, Writer};
use rp235x_hal::usb::UsbBus;
use usb_device::class_prelude::{ControlOut, DescriptorWriter, InterfaceNumber, UsbBusAllocator, UsbClass};
use usb_device::control::{Recipient, RequestType};
use usb_device::device::UsbDevice;
use usbd_serial::SerialPort;

pub const PRESENT_FOR: Duration = Duration::from_secs(2);
/// Every queue is allocated to its full size once, at boot. A queue that grew as bytes
/// arrived would copy itself inside the UART interrupt, with interrupts off for far longer
/// than the 32-byte hardware FIFO covers at `PILE_BAUD`, and the bytes arriving meanwhile
/// would be lost: that was how a 20 KB reveal failed where a 1.3 KB commit passed. The
/// sizes fit the largest frame of a ceremony, a sealed reveal, with room to spare.
const OUT_CAP: usize = 24 * 1024;
const IN_CAP: usize = 24 * 1024;

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
            inbox: VecDeque::with_capacity(IN_CAP),
        });
        let (mut rx, tx) = down;
        rx.enable_rx_interrupt();
        *UART_DOWN.borrow_ref_mut(cs) = Some(UartPort {
            rx,
            tx,
            inbox: VecDeque::with_capacity(IN_CAP),
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
    /// Returns how many bytes arrived.
    fn pull(&mut self) -> usize {
        let mut total = 0;
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
            total += n;
            // Never past the capacity allocated at boot: the oldest bytes go first.
            while self.inbox.len() + n > IN_CAP {
                self.inbox.pop_front();
            }
            self.inbox.extend(&buf[..n]);
        }
        total
    }
}

#[interrupt]
fn UART0_IRQ() {
    critical_section::with(|cs| {
        if let Some(p) = UART_UP.borrow_ref_mut(cs).as_mut() {
            if p.pull() > 0 {
                UP_SEEN.store((crate::clock::now_us() / 1000) as u32, Ordering::Relaxed);
            }
        }
    });
}

#[interrupt]
fn UART1_IRQ() {
    critical_section::with(|cs| {
        if let Some(p) = UART_DOWN.borrow_ref_mut(cs).as_mut() {
            if p.pull() > 0 {
                DOWN_SEEN.store((crate::clock::now_us() / 1000) as u32, Ordering::Relaxed);
            }
        }
    });
}

/// Whether a peer's TX holds an RX pin. An idle UART line is high; one carrying bytes is
/// low for most of a zero byte, and a frame can be a kilobyte of them, so the level at any
/// instant says nothing. A peer is present if the pin was high, or delivered a byte, within
/// the last `PEER_HOLD`. An empty connector sits on its pull-down and never does either.
fn line_present(gpio: u8, seen: &AtomicU32) -> bool {
    let now = (crate::clock::now_us() / 1000) as u32;
    if gpio_high(gpio) {
        seen.store(now, Ordering::Relaxed);
        return true;
    }
    now.wrapping_sub(seen.load(Ordering::Relaxed)) < PEER_HOLD_MS
}

const PEER_HOLD_MS: u32 = 500;
/// When each RX line was last seen high or last delivered a byte, in milliseconds since
/// boot; `u32::MAX / 2` at boot so that nothing counts as seen until it is.
static UP_SEEN: AtomicU32 = AtomicU32::new(u32::MAX / 2);
static DOWN_SEEN: AtomicU32 = AtomicU32::new(u32::MAX / 2);

const SAMPLES: u32 = 64;

/// How many of `SAMPLES` reads of the pin, 4 µs apart, were high: for the self-test.
pub fn line_highs(gpio: u8) -> u32 {
    let mut highs = 0;
    for _ in 0..SAMPLES {
        if gpio_high(gpio) {
            highs += 1;
        }
        let t = Instant::now();
        while t.elapsed() < Duration::from_micros(4) {}
    }
    highs
}

/// The self-test line for the pile links: how each detect line reads.
pub fn links_selftest() -> String {
    alloc::format!(
        "links: up GP1 {}/{} high, down GP5 {}/{} high",
        line_highs(1),
        SAMPLES,
        line_highs(5),
        SAMPLES
    )
}

static DOWN_WAS_UART: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static USB_WAS_PRESENT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// For the boot-time probes: whether the laptop is listening yet.
pub fn host_present() -> bool {
    usb_present()
}

/// Whether the laptop is there, logging each change, since a frame sent while it is not is
/// dropped on purpose and that must be visible.
fn usb_present() -> bool {
    let now = with_usb(|p| p.present()).unwrap_or(false);
    let was = USB_WAS_PRESENT.swap(now, core::sync::atomic::Ordering::Relaxed);
    if now != was {
        log_line(alloc::format!(
            "usb host: {}",
            if now { "present" } else { "absent for 2 s, frames down are dropped" }
        ));
    }
    now
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
    /// GP1 held high by the peer's TX, or bytes from it lately.
    fn peer_present(&self) -> bool {
        line_present(1, &UP_SEEN)
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

/// Logs each time the link down changes between the cable and USB.
fn uart_down_present() -> bool {
    let now = line_present(5, &DOWN_SEEN);
    let was = DOWN_WAS_UART.swap(now, core::sync::atomic::Ordering::Relaxed);
    if now != was {
        log_line(alloc::format!(
            "down link: {}",
            if now { "UART1, a card below" } else { "USB" }
        ));
    }
    now
}

// ---- USB ----

/// The interface picotool looks for to reboot a running board into BOOTSEL (`picotool
/// reboot -f -u`), so reflashing needs no hands on the board: a vendor-specific interface
/// taking one class request.
struct ResetClass {
    intf: InterfaceNumber,
}

impl<B: usb_device::bus::UsbBus> UsbClass<B> for ResetClass {
    fn get_configuration_descriptors(&self, w: &mut DescriptorWriter) -> usb_device::Result<()> {
        w.interface(self.intf, 0xFF, 0x00, 0x01)
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = *xfer.request();
        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Interface
            || req.index != u16::from(u8::from(self.intf))
        {
            return;
        }
        if req.request == 0x01 {
            let _ = xfer.accept();
            rp235x_hal::reboot::reboot(
                rp235x_hal::reboot::RebootKind::BootSel {
                    picoboot_disabled: false,
                    msd_disabled: false,
                },
                rp235x_hal::reboot::RebootArch::Normal,
            );
        }
        let _ = xfer.reject();
    }
}

struct UsbPort {
    dev: UsbDevice<'static, UsbBus>,
    serial: SerialPort<'static, UsbBus>,
    reset: ResetClass,
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
    let reset = ResetClass {
        intf: alloc.interface(),
    };
    let dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x2E8A, 0x000A))
        .strings(&[StringDescriptors::default()
            .manufacturer("Cahoots")
            .product("Cahoots badge (Pico 2)")
            .serial_number("pico2")])
        .unwrap()
        .max_packet_size_0(64)
        .unwrap()
        .composite_with_iads()
        .build();
    critical_section::with(|cs| {
        *USB.borrow_ref_mut(cs) = Some(UsbPort {
            dev,
            serial,
            reset,
            pending: VecDeque::with_capacity(OUT_CAP),
            inbox: VecDeque::with_capacity(IN_CAP),
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
            self.push_pending(&l);
        }
    }

    /// Queue bytes to go out, never past the capacity allocated at boot: the oldest go first.
    fn push_pending(&mut self, bytes: &[u8]) {
        let bytes = &bytes[bytes.len().saturating_sub(OUT_CAP)..];
        while self.pending.len() + bytes.len() > OUT_CAP {
            self.pending.pop_front();
            self.dropped = self.dropped.wrapping_add(1);
        }
        self.pending.extend(bytes);
    }

    /// Run the device, take what arrived, push what is queued.
    fn service(&mut self) {
        let _ = self.dev.poll(&mut [&mut self.serial, &mut self.reset]);
        let mut buf = [0u8; 64];
        loop {
            match self.serial.read(&mut buf) {
                Ok(n) if n > 0 => {
                    while self.inbox.len() + n > IN_CAP {
                        self.inbox.pop_front();
                    }
                    self.inbox.extend(&buf[..n]);
                    self.last_rx = Some(Instant::now());
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
        // Not serviced here: the badge's next wait pushes it out, and this keeps the
        // critical section a log line holds short.
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
                p.push_pending(frame);
                p.service();
            });
            log_line(alloc::format!("usb: sent {} bytes down", frame.len()));
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
        uart_down_present() || usb_present()
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
        p.push_pending(bytes);
        for _ in 0..200_000u32 {
            p.service();
            if p.pending.is_empty() {
                break;
            }
        }
    });
}
