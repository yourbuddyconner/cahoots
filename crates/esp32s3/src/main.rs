//! Cahoots badge firmware for the ESP32-S3 on an Arduino Nano ESP32.
//!
//! The same `cahoots-hal` pile and ceremony code as the simulator, over this board's
//! e-paper, two buttons, USB link, flash and hardware random source. See the README for the
//! wiring, the build and the flashing steps, and `docs/superpowers/specs` for the design.
#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod link;
mod buttons;
mod display;
mod entropy;
mod store;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use buttons::{Button, Buttons};
use cahoots_core::checkwords::check_words;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::pile::{Pile, PileError, Ready};
use cahoots_hal::{Display as _, Entropy, Link, PadStore};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::rng::TrngSource;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use link::{Absent, UsbLink};
use store::{Circle, FlashPadStore, FlashRecordStore, SharedFlash};

esp_bootloader_esp_idf::esp_app_desc!();

/// This board's proposal for a circle, used only if it is the bottom of the pile.
const NAME: &str = "nano";
const PAD_LEN: u32 = 16 * 1024;
/// What the top line says about where the randomness came from. The radio is present and
/// off by firmware, and the pad is plaintext on flash, so this board is tier 2.
const LABEL: &str = "TRNG T2";

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let msg = format!("PANIC: {info}");
    let f = cahoots_core::frame::Frame {
        kind: cahoots_hal::pile::KIND_LOG,
        seq: u32::MAX,
        payload: msg.as_bytes(),
    };
    link::emergency_write(&cahoots_core::frame::encode(&f));
    loop {
        core::hint::spin_loop();
    }
}

#[esp_hal::main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(size: 200 * 1024);
    link::init(p.USB_DEVICE);
    log!("cahoots esp32s3 {} on an Arduino Nano ESP32", env!("CARGO_PKG_VERSION"));
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
    let mut screen = display::Epd::new(spi, cs, dc, rst, busy, delay, LABEL).expect("e-paper");
    log!("e-paper up");

    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let mut btn = Buttons {
        a: Button::new(Input::new(p.GPIO5, pull_up)),
        b: Button::new(Input::new(p.GPIO6, pull_up)),
    };

    let _trng_source = TrngSource::new(p.RNG, p.ADC1);
    let mut entropy = Tested::new(
        entropy::Hw::new().expect("trng"),
        HealthTests::for_min_entropy(6),
    );
    let flash = store::open(p.FLASH);

    // Self-test, on the screen and in the log.
    let mut lines: Vec<String> = Vec::new();
    let mut sample = vec![0u8; 4096];
    lines.push(match entropy.fill(&mut sample) {
        Ok(()) => "trng: health tests pass".into(),
        Err(e) => format!("trng: FAILED {e:?}"),
    });
    let circle = store::load_circle(&flash);
    let pad_in_the_way = circle
        .as_ref()
        .is_some_and(|c| c.params.pad_len > store::PAD_CAP - store::SECTOR);
    lines.push(if pad_in_the_way {
        "flash: skipped, pad fills the region".into()
    } else {
        match selftest_flash(&flash) {
            Ok(()) => "flash: erase, program, read back ok".into(),
            Err(s) => format!("flash: FAILED {s}"),
        }
    });
    lines.push(format!(
        "buttons: A {}  B {}",
        if btn.a.is_down() { "held" } else { "up" },
        if btn.b.is_down() { "held" } else { "up" }
    ));
    lines.push(match &circle {
        Some(c) => format!("circle: {} seat {} of {}", c.params.name, c.seat, c.params.n),
        None => "circle: none yet".into(),
    });
    for l in &lines {
        log!("{l}");
    }
    screen.screen("cahoots", &lines.iter().map(String::as_str).collect::<Vec<_>>());
    delay.delay_millis(2500);

    // A held through the self-test forgets the circle: record, ledger and pad sectors.
    let circle = if circle.is_some() && btn.a.is_down() {
        let pad_len = circle.as_ref().map(|c| c.params.pad_len).unwrap_or(0);
        match store::wipe(&flash, pad_len) {
            Ok(()) => {
                log!("A held at boot: circle forgotten");
                screen.screen("cahoots", &["circle forgotten", "release A"]);
                while btn.a.is_down() {
                    delay.delay_millis(10);
                }
                None
            }
            Err(()) => {
                log!("wipe failed");
                circle
            }
        }
    } else {
        circle
    };

    if let Some(c) = &circle {
        let store = FlashPadStore::open(flash.clone(), c.params.pad_len);
        let mut whole = vec![0u8; c.params.pad_len as usize];
        store.read(0, &mut whole);
        let words = check_words(&c.params, &whole);
        screen.circle = c.params.name.clone();
        screen.show_words(&words);
        log!("words from flash: {}", words.join(" "));
    }

    loop {
        match run_once(&mut screen, &mut btn, &mut entropy, &flash, delay) {
            Ok(()) => {}
            Err(e) => {
                log!("{e}");
                screen.show_status(&e);
                delay.delay_millis(1500);
            }
        }
    }
}

/// One pass: find the pile, run the ceremony, keep or discard the result.
fn run_once(
    screen: &mut display::Epd,
    btn: &mut Buttons,
    entropy: &mut dyn Entropy,
    flash: &SharedFlash,
    delay: Delay,
) -> Result<(), String> {
    screen.show_status("plug into a pile. B on top starts");
    let down: Box<dyn Link> = Box::new(UsbLink);
    let up: Box<dyn Link> = Box::new(Absent);
    let mut pile = Pile::new(down, up, NAME, PAD_LEN, false);
    let mut shown: Option<u8> = None;
    let ready: Ready = loop {
        if let Some(r) = pile.poll().map_err(|e| format!("pile: {e:?}"))? {
            break r;
        }
        if pile.seat() != shown {
            shown = pile.seat();
            if let Some(s) = shown {
                log!("seat {s}");
                screen.show_status(&format!("seat {s}. B when the pile is whole"));
            }
        }
        if btn.b.pressed() {
            match pile.start() {
                Ok(()) => {
                    log!("B: counting the pile");
                    screen.show_status("counting the pile...");
                }
                Err(PileError::NoSeat) => screen.show_status("nothing below yet"),
                Err(e) => log!("start: {e:?}"),
            }
        }
        let _ = btn.a.pressed();
        delay.delay_millis(1);
    };
    let params = ready.params.clone();
    let seat = ready.seat;
    log!("pile counted: seat {seat} of {}, circle {}", params.n, params.name);
    screen.circle = params.name.clone();
    screen.show_status(&format!("seat {seat} of {}: {}", params.n, params.name));
    let ring = pile
        .into_ceremony(ready, entropy)
        .map_err(|e| format!("ceremony: {e:?}"))?;
    let (result, _links) = ring
        .run(screen, || delay.delay_micros(500))
        .map_err(|e| format!("ceremony: {e:?}"))?;
    log!("words: {}", result.check_words.join(" "));
    screen.show_status("B keep this circle . A discard");
    loop {
        if btn.b.pressed() {
            screen.show_status("saving...");
            FlashPadStore::write(flash.clone(), &result.pad).map_err(|_| "flash: pad write failed")?;
            FlashRecordStore::fresh(flash.clone()).map_err(|_| "flash: ledger erase failed")?;
            store::save_circle(
                flash,
                &Circle {
                    params: params.clone(),
                    seat,
                    circle_id: result.circle_id,
                },
            )
            .map_err(|_| "flash: circle record failed")?;
            log!("saved: {} seat {seat}", params.name);
            screen.show_status("saved. read the words aloud");
            return Ok(());
        }
        if btn.a.pressed() {
            drop(result);
            log!("discarded");
            screen.show_status("discarded");
            return Ok(());
        }
        delay.delay_millis(1);
    }
}

/// Erase, program and read back the last sector of the pad region.
fn selftest_flash(flash: &SharedFlash) -> Result<(), String> {
    let addr = store::PAD_BASE + store::PAD_CAP - store::SECTOR;
    let mut f = flash.borrow_mut();
    store::erase_sectors(&mut f, addr, store::SECTOR).map_err(|_| "erase")?;
    let mut buf = [0u8; 128];
    store::read_any(&mut f, addr + 1, &mut buf).map_err(|_| "read")?;
    if buf.iter().any(|&b| b != 0xFF) {
        return Err("not blank after erase".into());
    }
    let pat: Vec<u8> = (0..100u8).collect();
    store::program_any(&mut f, addr + 7, &pat).map_err(|_| "program")?;
    store::read_any(&mut f, addr + 6, &mut buf[..102]).map_err(|_| "read")?;
    if buf[0] != 0xFF || buf[1..101] != pat[..] || buf[101] != 0xFF {
        return Err("pattern read back wrong".into());
    }
    store::program_any(&mut f, addr + 10, &[0u8; 33]).map_err(|_| "zero")?;
    store::read_any(&mut f, addr + 9, &mut buf[..35]).map_err(|_| "read")?;
    if buf[0] != 2 || buf[1..34].iter().any(|&b| b != 0) || buf[34] != 36 {
        return Err("zeroing read back wrong".into());
    }
    store::erase_sectors(&mut f, addr, store::SECTOR).map_err(|_| "erase")?;
    Ok(())
}
