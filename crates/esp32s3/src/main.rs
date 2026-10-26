//! Cahoots badge firmware for the ESP32-S3 on an Arduino Nano ESP32.
//!
//! The badge's behaviour is `cahoots-badge`, the same code the simulator runs; this crate
//! is the board under it ([`board::NanoBoard`]) and the peripheral setup here. See the
//! README for the wiring, the build and the flashing steps, and `docs/superpowers/specs`
//! for the design.
#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod link;
mod board;
mod buttons;
mod display;
mod entropy;
mod store;

use alloc::format;
use board::NanoBoard;
use buttons::{Button, Buttons};
use cahoots_badge::{screens, Badge, Board as _};
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::Display as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::rng::TrngSource;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let msg = format!("PANIC: {info}");
    // Remember it in flash so the next boot can show it on the screen, through the one
    // flash handle, unless that handle is what panicked.
    store::save_panic_shared(msg.as_bytes());
    let fr = cahoots_core::frame::Frame {
        kind: cahoots_hal::pile::KIND_LOG,
        seq: u32::MAX,
        payload: msg.as_bytes(),
    };
    link::emergency_write(&cahoots_core::frame::encode(&fr));
    loop {
        core::hint::spin_loop();
    }
}

#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // Internal RAM is shared by this heap and the stack, and the post-quantum sealing in the
    // ceremony needs well over 100 KiB of stack. So the internal heap stays small, for the
    // allocations that want speed, and the pad, reveals and queues go to the 8 MB PSRAM.
    esp_alloc::heap_allocator!(size: 96 * 1024);
    esp_alloc::psram_allocator!(p.PSRAM, esp_hal::psram);
    let flash = store::open(p.FLASH);
    store::trace_begin(&flash);
    link::init(p.USB_DEVICE);
    log!(
        "cahoots esp32s3 {} on an Arduino Nano ESP32",
        env!("CARGO_PKG_VERSION")
    );
    let delay = Delay::new();

    let spi = Spi::new(
        p.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(1))
            .with_mode(Mode::_0),
    )
    .expect("spi config")
    .with_sck(p.GPIO48)
    .with_mosi(p.GPIO38);
    let cs = Output::new(p.GPIO21, Level::High, OutputConfig::default());
    let dc = Output::new(p.GPIO18, Level::Low, OutputConfig::default());
    let rst = Output::new(p.GPIO17, Level::High, OutputConfig::default());
    let busy = Input::new(p.GPIO10, InputConfig::default());
    let screen = display::Epd::new(spi, cs, dc, rst, busy, delay).expect("e-paper");
    log!("e-paper up");
    log!("heap: {}", esp_alloc::HEAP.stats());

    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let btn = Buttons::new(
        Button::new(Input::new(p.GPIO5, pull_up)),
        Button::new(Input::new(p.GPIO6, pull_up)),
    );

    let _trng_source = TrngSource::new(p.RNG, p.ADC1);
    let entropy = Tested::new(
        entropy::Hw::new().expect("trng"),
        HealthTests::for_min_entropy(6),
    );
    let mut badge = Badge::new(NanoBoard {
        screen,
        buttons: btn,
        entropy,
        flash,
        delay,
        stack_logged: 0,
    });
    badge.boot();
    loop {
        if badge.circle().is_none() {
            if let Err(e) = badge.run_once() {
                log!("{e}");
                let screen = screens::status(&[&e]);
                badge.board().note_screen(&screen.lines);
                badge.board().display().draw(&screen.frame);
                badge.board().delay_ms(1500);
            }
        } else {
            badge.idle_step();
        }
    }
}
