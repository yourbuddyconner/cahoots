//! A pad-bridge finder for a hand-wired board. Every GPIO is read with its pull-up and
//! then its pull-down on, and then each one is driven low and high in turn while the rest
//! are read, so two pads that are soldered together show up as one following the other.
//! Runs forever, logging over USB; the badge itself never starts.
use alloc::string::String;
use alloc::vec::Vec;
use embedded_hal::delay::DelayNs as _;
use rp235x_hal as hal;

const PINS: [u8; 30] = [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29];
/// Pins that are safe to drive: not the SMPS mode pin, VBUS sense or the LED.
const DRIVE: [u8; 27] = [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,26,27,28,29];

fn pads() -> &'static hal::pac::pads_bank0::RegisterBlock { unsafe { &*hal::pac::PADS_BANK0::ptr() } }
fn io() -> &'static hal::pac::io_bank0::RegisterBlock { unsafe { &*hal::pac::IO_BANK0::ptr() } }
fn sio() -> &'static hal::pac::sio::RegisterBlock { unsafe { &*hal::pac::SIO::ptr() } }

fn input(n: u8, up: bool) {
    sio().gpio_oe_clr().write(|w| unsafe { w.bits(1 << n) });
    pads().gpio(n as usize).modify(|_, w| {
        w.pue().bit(up).pde().bit(!up).ie().set_bit().od().clear_bit().iso().clear_bit()
    });
    io().gpio(n as usize).gpio_ctrl().modify(|_, w| w.funcsel().sio());
}

fn output(n: u8, high: bool) {
    pads().gpio(n as usize).modify(|_, w| w.pue().clear_bit().pde().clear_bit().ie().set_bit().od().clear_bit().iso().clear_bit().drive()._2m_a());
    io().gpio(n as usize).gpio_ctrl().modify(|_, w| w.funcsel().sio());
    if high { sio().gpio_out_set().write(|w| unsafe { w.bits(1 << n) }); } else { sio().gpio_out_clr().write(|w| unsafe { w.bits(1 << n) }); }
    sio().gpio_oe_set().write(|w| unsafe { w.bits(1 << n) });
}

fn read() -> u32 { sio().gpio_in().read().bits() }

fn list(mask: u32) -> String {
    let v: Vec<String> = PINS.iter().filter(|&&p| mask & (1 << p) != 0).map(|p| alloc::format!("GP{p}")).collect();
    if v.is_empty() { "none".into() } else { v.join(" ") }
}

pub fn run(mut delay: hal::Timer<hal::timer::CopyableTimer0>) -> ! {
    let all: u32 = PINS.iter().fold(0, |m, &p| m | (1 << p));
    loop {
        for &p in &PINS { input(p, true); }
        delay.delay_ms(20);
        let low_with_pullup = !read() & all;
        for &p in &PINS { input(p, false); }
        delay.delay_ms(20);
        let high_with_pulldown = read() & all;
        log!("pintest: pulled up, these read LOW: {}", list(low_with_pullup));
        log!("pintest: pulled down, these read HIGH: {}", list(high_with_pulldown));
        for &d in &DRIVE {
            // Drive d low with everyone else pulled up: a follower goes low with it.
            for &p in &PINS { input(p, true); }
            output(d, false);
            delay.delay_ms(5);
            let followers_low = !read() & all & !(1 << d) & !low_with_pullup;
            // Drive d high with everyone else pulled down: a follower goes high with it.
            for &p in &PINS { input(p, false); }
            output(d, true);
            delay.delay_ms(5);
            let followers_high = read() & all & !(1 << d) & !high_with_pulldown;
            input(d, true);
            if followers_low | followers_high != 0 {
                log!("pintest: GP{d} driven low pulls down {}; driven high pulls up {}", list(followers_low), list(followers_high));
            }
            crate::link::usb_pump();
        }
        log!("pintest: sweep done");
        for _ in 0..50 { crate::link::usb_pump(); delay.delay_ms(100); }
    }
}
