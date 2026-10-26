//! [`NanoBoard`]: the Arduino Nano ESP32 as a `cahoots_badge::Board`. The badge's behaviour
//! lives in `cahoots-badge`; this is the e-paper, the two buttons, the hardware random
//! source, the flash regions and the USB link under it.

use crate::buttons::Buttons;
use crate::display::Epd;
use crate::entropy;
use crate::link::{Absent, UsbLink};
use crate::store::{self, FlashPadStore, FlashRecordStore, SharedFlash};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_badge::{screens, Board, BoardError, Circle, Press};
use cahoots_hal::health::Tested;
use cahoots_hal::{Display, Link};
use esp_hal::delay::Delay;

/// This board's proposal for a circle, used only if it is the bottom of the pile.
const NAME: &str = "nano";
const PAD_LEN: u32 = 16 * 1024;

pub struct NanoBoard {
    pub screen: Epd,
    pub buttons: Buttons,
    pub entropy: Tested<entropy::Hw>,
    pub flash: SharedFlash,
    pub delay: Delay,
    /// The deepest stack use already written to the log, so each new high-water mark is
    /// logged once.
    pub stack_logged: u32,
}

impl Board for NanoBoard {
    type Entropy = Tested<entropy::Hw>;
    type Pad = FlashPadStore;
    type Records = FlashRecordStore;
    type Display = Epd;
    type Link = Box<dyn Link>;

    fn entropy(&mut self) -> &mut Self::Entropy {
        &mut self.entropy
    }

    fn display(&mut self) -> &mut Epd {
        &mut self.screen
    }

    fn press(&mut self) -> Press {
        self.buttons.press()
    }

    fn a_down(&self) -> bool {
        self.buttons.a.is_down()
    }

    fn delay_ms(&mut self, ms: u32) {
        // Every state of the badge waits here, so this is where queued log lines leave
        // the board even when nothing else touches the USB link.
        let _ = <UsbLink as Link>::flush(&mut UsbLink);
        let deepest = entropy::stack_high_water();
        if deepest > self.stack_logged {
            self.stack_logged = deepest;
            log!("stack: {deepest} bytes used at the deepest point so far");
        }
        self.delay.delay_millis(ms);
    }

    /// The USB port below and nothing above. Both are unit handles, so every call returns
    /// usable links to the same two connectors.
    fn links(&mut self) -> (Box<dyn Link>, Box<dyn Link>) {
        (Box::new(UsbLink), Box::new(Absent))
    }

    /// The circle record on flash.
    fn load_circle(&self) -> Option<Circle> {
        store::load_circle(&self.flash)
    }

    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError> {
        store::save_circle(&self.flash, c).map_err(|()| BoardError::Storage)
    }

    fn wipe(&mut self, pad_len: u32) -> Result<(), BoardError> {
        store::wipe(&self.flash, pad_len).map_err(|()| BoardError::Storage)
    }

    fn write_pad(&mut self, pad: &[u8]) -> Result<FlashPadStore, BoardError> {
        FlashPadStore::write(self.flash.clone(), pad).map_err(|()| BoardError::Storage)
    }

    fn open_pad(&mut self, len: u32) -> FlashPadStore {
        FlashPadStore::open(self.flash.clone(), len)
    }

    fn records(&mut self, fresh: bool) -> Result<FlashRecordStore, BoardError> {
        if fresh {
            FlashRecordStore::fresh(self.flash.clone()).map_err(|()| BoardError::Storage)
        } else {
            Ok(FlashRecordStore::open(self.flash.clone()))
        }
    }

    fn proposal(&self) -> (String, u32) {
        (NAME.into(), PAD_LEN)
    }

    /// Tier 1: a hardware random source with health tests, no radio code linked, and the
    /// pad plaintext on flash.
    fn tier(&self) -> u8 {
        1
    }

    /// The flash line and the buttons line. The badge adds the TRNG and circle lines.
    fn selftest(&mut self) -> Vec<String> {
        if let Some(msg) = store::take_panic(&self.flash) {
            // Keep the file name and line, drop the directory. The screen wraps the rest.
            let msg = msg.replace("PANIC: panicked at ", "");
            let compact: String = msg
                .split('\n')
                .map(|l| l.rsplit('/').next().unwrap_or(l).trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            log!("last boot panicked: {compact}");
            let screen = screens::panic(&compact);
            self.note_screen(&screen.lines);
            self.screen.draw(&screen.frame);
            while self.buttons.press() != Press::Both {
                let _ = <UsbLink as Link>::flush(&mut UsbLink);
                self.delay.delay_millis(10);
            }
            // The badge reads A after this returns to decide on the wipe, so the Both that
            // dismissed this screen must be over first.
            while self.buttons.a.is_down() || self.buttons.b.is_down() {
                let _ = <UsbLink as Link>::flush(&mut UsbLink);
                self.delay.delay_millis(10);
            }
        }
        let pad_in_the_way = store::load_circle(&self.flash)
            .is_some_and(|c| c.params.pad_len > store::PAD_CAP - store::SECTOR);
        let flash = if pad_in_the_way {
            "flash: skipped, pad fills the region".into()
        } else {
            match selftest_flash(&self.flash) {
                Ok(()) => "flash: erase, program, read back ok".into(),
                Err(s) => format!("flash: FAILED {s}"),
            }
        };
        let state = |down: bool| if down { "held" } else { "up" };
        let buttons = format!(
            "buttons: A {}  B {}",
            state(self.buttons.a.is_down()),
            state(self.buttons.b.is_down())
        );
        // Nothing pressed during the self-test carries into the badge's first screen.
        self.buttons.clear();
        vec![flash, buttons]
    }

    fn log(&mut self, line: &str) {
        log!("{line}");
    }

    /// Each line of the screen goes to the trace sector, so the last screens can be read
    /// back with espflash after a hang.
    fn note_screen(&mut self, lines: &[String]) {
        for line in lines {
            store::trace(&format!("screen: {line}"));
        }
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
