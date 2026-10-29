//! [`PicoBoard`]: the Raspberry Pi Pico 2 as a `cahoots_badge::Board`. The badge's behaviour
//! lives in `cahoots-badge`; this is the e-paper, the two buttons, the hardware random
//! source, the flash regions and the three links under it.

use crate::buttons::Buttons;
use crate::display::Epd;
use crate::entropy;
use crate::link::{self, DownLink, UpLink};
use crate::store::{self, FlashPadStore, FlashRecordStore};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_badge::board::{assign_places, place_for};
use cahoots_badge::{screens, Board, BoardError, Circle, Press};
use cahoots_core::circle_id::CircleId;
use cahoots_hal::health::Tested;
use cahoots_hal::{Display, Link};
use embedded_hal::delay::DelayNs;
use rp235x_hal::timer::{CopyableTimer0, Timer};

/// This board's proposal for a circle, used only if it is the bottom of the pile.
const NAME: &str = "pico";
const PAD_LEN: u32 = 16 * 1024;

pub struct PicoBoard {
    pub screen: Epd,
    pub buttons: Buttons,
    pub entropy: Tested<entropy::Hw>,
    pub delay: Timer<CopyableTimer0>,
    /// The deepest stack use already written to the log.
    pub stack_logged: u32,
    /// The most heap use already written to the log.
    pub heap_logged: usize,
    /// The place a pad was written to for a circle not stored yet.
    pub pending: Option<(CircleId, usize)>,
}

impl PicoBoard {
    /// The circle id each place holds.
    fn held(&self) -> Vec<Option<CircleId>> {
        (0..store::PLACES)
            .map(|p| store::load_circle(p).map(|c| c.circle_id))
            .collect()
    }

    fn place(&self, circle: &CircleId) -> Result<usize, BoardError> {
        place_for(&self.held(), self.pending, circle).ok_or(BoardError::Storage)
    }
}

impl Board for PicoBoard {
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
        // the board even when nothing else touches the USB port.
        link::usb_pump();
        let deepest = entropy::stack_high_water();
        if deepest > self.stack_logged {
            self.stack_logged = deepest;
            log!("stack: {deepest} bytes used at the deepest point so far");
        }
        let used = crate::HEAP.used();
        if used > self.heap_logged + 16 * 1024 {
            self.heap_logged = used;
            log!("heap: {used} bytes in use, {} free", crate::HEAP.free());
        }
        self.delay.delay_ms(ms);
    }

    /// The link down (UART1, or USB when nothing is on it) and the link up (UART0). Both
    /// are unit handles, so every call returns usable links to the same connectors.
    fn links(&mut self) -> (Box<dyn Link>, Box<dyn Link>) {
        (Box::new(DownLink), Box::new(UpLink))
    }

    fn max_circles(&self) -> usize {
        store::PLACES
    }

    fn load_circles(&mut self) -> Vec<Circle> {
        (0..store::PLACES).filter_map(store::load_circle).collect()
    }

    /// Rewrites the record of each place whose circle changed, and erases the record of a
    /// place whose circle is not in the list.
    fn store_circles(&mut self, circles: &[Circle]) -> Result<(), BoardError> {
        let places = assign_places(&self.held(), self.pending, circles)?;
        for (p, i) in places.into_iter().enumerate() {
            match i {
                Some(i) if store::load_circle(p).as_ref() == Some(&circles[i]) => {}
                Some(i) => store::save_circle(p, &circles[i]).map_err(|()| BoardError::Storage)?,
                None => store::erase_circle(p).map_err(|()| BoardError::Storage)?,
            }
        }
        self.pending = None;
        Ok(())
    }

    fn forget(&mut self, circle: &CircleId, pad_len: u32) -> Result<(), BoardError> {
        store::wipe(self.place(circle)?, pad_len).map_err(|()| BoardError::Storage)
    }

    fn write_pad(&mut self, circle: &CircleId, pad: &[u8]) -> Result<FlashPadStore, BoardError> {
        let p = self.place(circle)?;
        self.pending = Some((*circle, p));
        FlashPadStore::write(store::place(p), pad).map_err(|()| BoardError::Storage)
    }

    fn open_pad(&mut self, circle: &CircleId, len: u32) -> Result<FlashPadStore, BoardError> {
        FlashPadStore::open(store::place(self.place(circle)?), len).map_err(|()| BoardError::Storage)
    }

    fn records(&mut self, circle: &CircleId, fresh: bool) -> Result<FlashRecordStore, BoardError> {
        let at = store::place(self.place(circle)?);
        if fresh {
            FlashRecordStore::fresh(at).map_err(|()| BoardError::Storage)
        } else {
            Ok(FlashRecordStore::open(at))
        }
    }

    fn proposal(&self) -> (String, u32) {
        (NAME.into(), PAD_LEN)
    }

    /// Tier 1: a hardware random source with health tests, no radio on the chip, and the
    /// pad plaintext on flash until the key store chip (M4f).
    fn tier(&self) -> u8 {
        1
    }

    /// The flash line and the buttons line. The badge adds the TRNG and circle lines.
    fn selftest(&mut self) -> Vec<String> {
        if let Some(msg) = store::take_panic() {
            let msg = msg.replace("PANIC: panicked at ", "");
            let compact: String = msg
                .split('\n')
                .map(|l| l.rsplit('/').next().unwrap_or(l).trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            log!("last boot panicked: {compact}");
            let screen = screens::panic(&compact);
            self.note_screen(&screen.lines, screen.secret);
            self.screen.draw(&screen.frame);
            while self.buttons.press() != Press::Both {
                link::usb_pump();
                self.delay.delay_ms(10);
            }
            while self.buttons.a.is_down() || self.buttons.b.is_down() {
                link::usb_pump();
                self.delay.delay_ms(10);
            }
        }
        let pad_in_the_way = store::load_circle(0)
            .is_some_and(|c| c.params.pad_len > store::PAD_CAP - store::SECTOR);
        let flash = if pad_in_the_way {
            "flash: skipped, pad fills the region".into()
        } else {
            match selftest_flash() {
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
        self.buttons.clear();
        vec![flash, buttons, link::links_selftest()]
    }

    fn log(&mut self, line: &str) {
        log!("{line}");
    }

    /// Each line of the screen goes to the log and the trace, so the host sees what the
    /// panel shows. A key screen is not copied.
    fn note_screen(&mut self, lines: &[String], secret: bool) {
        if secret {
            store::trace("screen: a key, not traced");
            return;
        }
        log!("screen: {}", lines.join(" | "));
    }
}

/// Erase, program and read back the last sector of the pad region.
fn selftest_flash() -> Result<(), String> {
    let addr = store::PAD_BASE + store::PAD_CAP - store::SECTOR;
    store::erase_sectors(addr, store::SECTOR).map_err(|_| "erase")?;
    let mut buf = [0u8; 128];
    store::read_any(addr + 1, &mut buf);
    if buf.iter().any(|&b| b != 0xFF) {
        return Err("not blank after erase".into());
    }
    let pat: Vec<u8> = (0..100u8).collect();
    store::program_any(addr + 7, &pat).map_err(|_| "program")?;
    store::read_any(addr + 6, &mut buf[..102]);
    if buf[0] != 0xFF || buf[1..101] != pat[..] || buf[101] != 0xFF {
        return Err("pattern read back wrong".into());
    }
    store::program_any(addr + 10, &[0u8; 33]).map_err(|_| "zero")?;
    store::read_any(addr + 9, &mut buf[..35]);
    if buf[0] != 2 || buf[1..34].iter().any(|&b| b != 0) || buf[34] != 36 {
        return Err("zeroing read back wrong".into());
    }
    store::erase_sectors(addr, store::SECTOR).map_err(|_| "erase")?;
    Ok(())
}
