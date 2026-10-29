//! Cahoots badge firmware for the RP2350 on a Raspberry Pi Pico 2, wired as V1.
//!
//! The badge's behaviour is `cahoots-badge`, the same code the simulator and the Nano run;
//! this crate is the board under it ([`board::PicoBoard`]) and the peripheral setup here.
//! See the README for the wiring, the build and the flashing steps.
#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod link;
mod board;
mod buttons;
mod clock;
mod display;
mod entropy;
mod store;

use alloc::format;
use board::PicoBoard;
use buttons::{Button, Buttons};
use cahoots_badge::{screens, Badge, Board as _};
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::Display as _;
use embedded_alloc::LlffHeap as Heap;
use rp235x_hal as hal;
use rp235x_hal::fugit::RateExtU32;
use embedded_hal::delay::DelayNs as _;
use rp235x_hal::uart::{DataBits, StopBits, UartConfig, UartPeripheral};
use rp235x_hal::Clock;

/// Tell the boot ROM about the program.
#[link_section = ".start_block"]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

/// The Pico 2's crystal.
const XTAL_FREQ_HZ: u32 = 12_000_000;
/// The pile UARTs. Short cables on a bench or V1's pogo pair; the laptop link is USB and
/// ignores this. Bytes are taken by interrupt from a 32-byte FIFO, and the badge holds
/// interrupts off for up to a few hundred microseconds at a time, so the rate is set where
/// the FIFO rides that out: at 115200 the handler has over a millisecond, and a ceremony
/// still crosses a hop in about two seconds.
const PILE_BAUD: u32 = 115_200;

#[global_allocator]
pub static HEAP: Heap = Heap::empty();
/// A three-member ceremony with 16 KiB contributions ran out of a 200 KiB heap while the
/// sealed reveals were in flight. The post-quantum sealing wants well over 100 KiB of
/// stack, and the stack grows down from the top of the 512 KiB; this leaves it about
/// 185 KiB. The board logs both as it goes.
const HEAP_SIZE: usize = 384 * 1024;

/// A fault that no handler caught: record where, as the panic handler does, so the next
/// boot shows it and the host's log sees it. A stack overflow lands here through the stack
/// limit, so the limit is dropped first to give this handler room.
#[cortex_m_rt::exception]
unsafe fn HardFault(ef: &cortex_m_rt::ExceptionFrame) -> ! {
    // SAFETY: lowering the limit only gives the handler the guard band to run in.
    unsafe { cortex_m::register::msplim::write(core::ptr::addr_of!(__ebss) as u32) };
    // SAFETY: reads of status registers.
    let scb = unsafe { &*cortex_m::peripheral::SCB::PTR };
    let msg = format!(
        "PANIC: hard fault: pc={:#010x} lr={:#010x} sp={:#010x} cfsr={:#010x} hfsr={:#010x} heap used {}",
        ef.pc(),
        ef.lr(),
        cortex_m::register::msp::read(),
        scb.cfsr.read(),
        scb.hfsr.read(),
        HEAP.used()
    );
    die(msg)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    die(format!("PANIC: {info} (heap used {})", HEAP.used()))
}

fn die(msg: alloc::string::String) -> ! {
    store::save_panic(msg.as_bytes());
    let fr = cahoots_core::frame::Frame {
        kind: cahoots_hal::pile::KIND_LOG,
        seq: u32::MAX,
        payload: msg.as_bytes(),
    };
    link::emergency_write(&cahoots_core::frame::encode(&fr));
    // Give the log frame a moment to leave, then start over: the record is shown at the
    // next boot. Spinning here would leave a faulted badge deaf to USB, with no way back
    // but a hand on BOOTSEL.
    for _ in 0..30_000_000u32 {
        core::hint::spin_loop();
    }
    cortex_m::peripheral::SCB::sys_reset();
}

unsafe extern "C" {
    static mut __ebss: u32;
}

#[hal::entry]
fn main() -> ! {
    // The stack grows down towards `.bss`, where the heap arena is. A limit just above it
    // turns an overflow into a fault instead of a silently corrupted heap.
    // SAFETY: the symbol is cortex-m-rt's end of `.bss`; the limit sits a little above it.
    unsafe { cortex_m::register::msplim::write(core::ptr::addr_of!(__ebss) as u32 + 4096) };
    {
        use core::mem::MaybeUninit;
        static mut ARENA: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        // SAFETY: called once, before any allocation.
        unsafe { HEAP.init(core::ptr::addr_of_mut!(ARENA) as usize, HEAP_SIZE) }
    }
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();
    let timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);
    clock::init(timer);
    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // USB to the laptop, and the two pile UARTs.
    link::init_usb(hal::usb::UsbBus::new(
        pac.USB,
        pac.USB_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));
    let uart0_pins: link::Uart0Pins = (pins.gpio0.reconfigure(), pins.gpio1.reconfigure());
    let uart1_pins: link::Uart1Pins = (pins.gpio4.reconfigure(), pins.gpio5.reconfigure());
    let cfg = || UartConfig::new(PILE_BAUD.Hz(), DataBits::Eight, None, StopBits::One);
    let up = UartPeripheral::new(pac.UART0, uart0_pins, &mut pac.RESETS)
        .enable(cfg(), clocks.peripheral_clock.freq())
        .expect("uart0")
        .split();
    let down = UartPeripheral::new(pac.UART1, uart1_pins, &mut pac.RESETS)
        .enable(cfg(), clocks.peripheral_clock.freq())
        .expect("uart1")
        .split();
    // Interrupt at four bytes in the FIFO rather than sixteen, for the most slack.
    // SAFETY: a register write on peripherals this program owns.
    unsafe {
        (*hal::pac::UART0::ptr()).uartifls().write(|w| w.rxiflsel().bits(0));
        (*hal::pac::UART1::ptr()).uartifls().write(|w| w.rxiflsel().bits(0));
    }
    link::init_uarts(up, down);
    log!(
        "cahoots rp2350 {} on a Raspberry Pi Pico 2; heap {} KiB, stack to {:#x}",
        env!("CARGO_PKG_VERSION"),
        HEAP_SIZE / 1024,
        core::ptr::addr_of!(__ebss) as u32 + 4096
    );
    #[cfg(feature = "bench")]
    log!("BENCH BUILD: the laptop can press the buttons over USB; not for anyone's keys");

    // The e-paper on SPI0, V1's pins: CLK GP18, DIN GP19, CS GP17, DC GP16, RST GP20, BUSY GP21.
    let spi_pins: display::SpiPins = (pins.gpio19.reconfigure(), pins.gpio18.reconfigure());
    let spi = hal::spi::Spi::<_, _, _, 8>::new(pac.SPI0, spi_pins).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        1.MHz(),
        embedded_hal::spi::MODE_0,
    );
    let cs = pins.gpio17.into_push_pull_output_in_state(hal::gpio::PinState::High);
    let dc = pins.gpio16.into_push_pull_output();
    let rst = pins.gpio20.into_push_pull_output_in_state(hal::gpio::PinState::High);
    let busy = pins.gpio21.into_pull_down_input();
    let screen = display::Epd::new(spi, cs, dc, rst, busy, timer).expect("e-paper");
    log!("e-paper up");

    // The buttons: A on GP14, B on GP15, each to ground, pulled up here.
    let _a = pins.gpio14.into_pull_up_input();
    let _b = pins.gpio15.into_pull_up_input();
    let btn = Buttons::new(Button::new(14), Button::new(15));

    // The SE051 (for now the Click board) on I2C0, V1's pins: SDA GP8, SCL GP9, ENA GP2 held
    // high for as long as the board runs. No answer leaves the badge without a chip: seat
    // keys and roots stay in software, as before the chip.
    let ena = pins.gpio2.into_push_pull_output_in_state(hal::gpio::PinState::High);
    let sda: board::Sda = pins.gpio8.reconfigure();
    let scl: board::Scl = pins.gpio9.reconfigure();
    let i2c = hal::I2C::i2c0(pac.I2C0, sda, scl, 400.kHz(), &mut pac.RESETS, &clocks.system_clock);
    // The chip wants a moment after its enable pin rises before it answers.
    {
        let mut d = timer;
        d.delay_ms(10);
    }
    let chip = match cahoots_chip::se05x::Se05xChip::new(i2c, timer) {
        Ok(c) => Some(c),
        Err(e) => {
            log!("se051: no answer ({e:?})");
            None
        }
    };
    #[cfg(feature = "bench")]
    let chip = chip.map(|mut c| {
        // Give the laptop time to open the port, or the probe's lines are gone before
        // anyone reads them.
        for _ in 0..80 {
            link::usb_pump();
            let mut d = timer;
            d.delay_ms(100);
        }
        for line in c.probe_counter() {
            log!("probe: {line}");
        }
        c
    });

    let entropy = Tested::new(
        entropy::Hw::new(pac.TRNG, &mut pac.RESETS),
        HealthTests::for_min_entropy(6),
    );
    let mut badge = Badge::new(PicoBoard {
        screen,
        buttons: btn,
        entropy,
        delay: timer,
        stack_logged: 0,
        heap_logged: 0,
        pending: None,
        chip,
        _chip_enable: ena,
    });
    badge.boot();
    loop {
        if badge.circle().is_none() {
            if let Err(e) = badge.run_once() {
                log!("{e}");
                let screen = screens::status(&[&e]);
                badge.board().note_screen(&screen.lines, screen.secret);
                badge.board().display().draw(&screen.frame);
                badge.board().delay_ms(1500);
            }
        } else {
            badge.idle_step();
        }
    }
}
