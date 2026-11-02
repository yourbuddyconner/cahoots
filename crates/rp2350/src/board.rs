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
use cahoots_badge::chip::ChipRecord;
use cahoots_badge::grants::GrantsRecord;
use cahoots_badge::{screens, Board, BoardError, Circle, Press};
use cahoots_chip::se05x::Se05xChip;
use cahoots_chip::{ChipError, ChipOps, KeyId, KeyKind};
use embedded_hal::digital::OutputPin;
use cahoots_core::circle_id::CircleId;
use cahoots_hal::health::Tested;
use cahoots_hal::{Display, Link};
use embedded_hal::delay::DelayNs;
use rp235x_hal::gpio::bank0::{Gpio2, Gpio3, Gpio8, Gpio9};
use rp235x_hal::gpio::{FunctionI2C, FunctionSioOutput, Pin, PullDown, PullNone};
use rp235x_hal::pac::I2C0;
use rp235x_hal::timer::{CopyableTimer0, Timer};

/// The SE051's bus: I2C0 on GP8 (SDA) and GP9 (SCL).
// No pull-ups on the Pico's side: the Click board has its own, and a pull-up here would
// feed the chip through its data pads while its power is cut for a phone's tap.
pub type Sda = Pin<Gpio8, FunctionI2C, PullNone>;
pub type Scl = Pin<Gpio9, FunctionI2C, PullNone>;
pub type ChipBus = rp235x_hal::I2C<I2C0, (Sda, Scl)>;
/// The SE051 through the `se05x` driver.
pub type Chip = Se05xChip<ChipBus, Timer<CopyableTimer0>>;

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
    /// The SE051, or `None` when it did not answer at boot.
    pub chip: Option<Chip>,
    /// The SE051's enable pin. Low at rest, so a phone's field starts the chip on its
    /// antenna; high only between `chip_begin` and `chip_end`, when the chip answers on I2C
    /// alone and a tap finds no tag.
    pub chip_enable: Pin<Gpio2, FunctionSioOutput, PullDown>,
    /// The Click board's 3.3 V, through a PNP transistor whose base this pin drives: low
    /// powers the board, high cuts it. A phone's tap reaches the chip only while it is cut,
    /// since a chip the badge powers boots onto the wire. Without the transistor the pin
    /// does nothing and the chip stays powered.
    pub chip_rail: Pin<Gpio3, FunctionSioOutput, PullDown>,
    /// True while the chip's power is cut for a phone's tap (`tap_begin` to `tap_end`).
    pub tap_open: bool,
    /// True while the pin is high.
    pub chip_up: bool,
    /// The badge held the chip until a restart: the pin stays high and no conversation ends
    /// or begins, so no tap reaches the chip before the next boot.
    pub chip_held: bool,
    /// The secret of the chip's owner object, from the RP2350's unique id, or `None` if the
    /// ROM did not give the id.
    pub owner_secret: Option<[u8; 16]>,
    /// The owner secret of builds before 2026-11-01, from the device and wafer ids alone, tried
    /// when the chip refuses `owner_secret`. Once it opens, it becomes `owner_secret` for the
    /// rest of the boot. `None` unless a current chip record was stored before this boot, so a
    /// fresh badge never adopts an owner object a tap planted under it.
    pub legacy_owner_secret: Option<[u8; 16]>,
    /// The last conversation found the owner object and the chip refused every secret the
    /// badge has (`BadSession`), not a bus error.
    pub owner_refused: bool,
    /// Bench builds: the pin is high for the laptop's APDUs, in no session, since this moment
    /// (the last APDU). `None` when the pass-through holds nothing.
    #[cfg(feature = "bench")]
    pub bench_raw: Option<crate::clock::Instant>,
    /// Bench builds: the pin is high and the wire silent, for a phone's tap to find the chip
    /// powered. Set by the laptop's window request, cleared by its first APDU or the idle.
    #[cfg(feature = "bench")]
    pub bench_window: bool,
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
        #[cfg(feature = "bench")]
        if let Some(p) = link::bench_press() {
            log!("bench: the laptop pressed {p:?}");
            return p;
        }
        self.buttons.press()
    }

    fn a_down(&self) -> bool {
        self.buttons.a.is_down()
    }

    #[cfg(feature = "bench")]
    fn screen_drawn(&mut self) {
        log!("drawn");
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
        #[cfg(feature = "bench")]
        self.bench_serve();
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
        vec![flash, buttons, link::links_selftest(), self.selftest_chip()]
    }

    fn has_chip(&self) -> bool {
        self.chip.is_some() && !self.chip_held
    }

    fn chip_begin(&mut self) -> Option<&mut dyn ChipOps> {
        if self.chip_held {
            return None;
        }
        self.tap_end();
        // The badge's own conversation starts on a chip that the laptop's session is gone from.
        #[cfg(feature = "bench")]
        self.chip_raw_end();
        self.wake().map(|c| c as &mut dyn ChipOps)
    }

    fn tap_begin(&mut self) {
        if self.tap_open || self.chip_held || self.chip.is_none() {
            return;
        }
        self.chip_end();
        let _ = self.chip_enable.set_low();
        let _ = self.chip_rail.set_high();
        self.tap_open = true;
        log!("se051: power cut for a phone's tap");
    }

    fn tap_end(&mut self) {
        if !self.tap_open {
            return;
        }
        let _ = self.chip_rail.set_low();
        self.tap_open = false;
        self.delay.delay_ms(RAIL_START_MS);
        log!("se051: power back");
    }

    fn tap_open(&self) -> bool {
        self.tap_open
    }

    /// Close the session and drop the pin.
    fn chip_end(&mut self) {
        if !self.chip_up || self.chip_held {
            return;
        }
        if let Some(chip) = self.chip.as_mut() {
            if let Err(e) = chip.close_session() {
                log!("se051: the session did not close ({e:?})");
            }
        }
        let _ = self.chip_enable.set_low();
        self.chip_up = false;
        log!("se051: pin low");
    }

    fn chip_unusable(&mut self) {
        self.chip_end();
        self.chip = None;
    }

    fn chip_hold(&mut self) {
        self.chip_held = true;
        log!("se051: held, pin high until the badge restarts");
    }

    fn owner_secret(&self) -> Option<[u8; 16]> {
        self.owner_secret
    }

    fn owner_refused(&self) -> bool {
        self.owner_refused
    }

    /// `CAHC2` in the sector; the record of earlier firmware (`CAHC`) reads as none.
    fn chip_record_load(&mut self) -> Result<Option<ChipRecord>, BoardError> {
        match store::load_chip_record() {
            Ok(None) => Ok(None),
            Ok(Some(body)) => ChipRecord::decode(&body).map(Some).map_err(|_| BoardError::Storage),
            Err(()) => Err(BoardError::Storage),
        }
    }

    fn chip_record_store(&mut self, r: &ChipRecord) -> Result<(), BoardError> {
        store::save_chip_record(&r.encode()).map_err(|()| BoardError::Storage)
    }

    fn grants_load(&mut self) -> Result<Option<GrantsRecord>, BoardError> {
        match store::load_grants() {
            Ok(None) => Ok(None),
            Ok(Some(body)) => GrantsRecord::decode(&body).map(Some).map_err(|_| BoardError::Storage),
            Err(()) => Err(BoardError::Storage),
        }
    }

    fn grants_store(&mut self, r: &GrantsRecord) -> Result<(), BoardError> {
        store::save_grants(&r.encode()).map_err(|()| BoardError::Storage)
    }

    fn uptime_secs(&self) -> u64 {
        // The timer counts microseconds since boot.
        self.delay.get_counter().ticks() / 1_000_000
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

impl PicoBoard {
    /// Raise the pin, give the chip its start time, wake the driver, and open the owner
    /// session if the chip has an owner object. The same conversation if one is open. `None`
    /// when the owner object is there and its session does not open: no command then runs in
    /// no session in the badge's name. At boot the badge then runs without its chip, and says
    /// the chip could not be set up.
    fn wake(&mut self) -> Option<&mut Chip> {
        if self.chip.is_none() {
            return None;
        }
        if !self.chip_up {
            let _ = self.chip_enable.set_high();
            self.chip_up = true;
            log!("se051: pin high");
            self.delay.delay_ms(CHIP_START_MS);
            let chip = self.chip.as_mut()?;
            if let Err(e) = chip.restart() {
                log!("se051: no answer after the pin rose ({e:?})");
                self.chip_end();
                return None;
            }
            match (self.owner_secret, chip.exists(KeyId::OWNER)) {
                (Some(secret), Ok(true)) => match chip.open_owner(&secret) {
                    Ok(()) => {
                        self.owner_refused = false;
                        log!("se051: owner session open");
                    }
                    Err(e) => {
                        let legacy = self.legacy_owner_secret.filter(|_| e == ChipError::BadSession);
                        let older = legacy.map(|l| chip.open_owner(&l));
                        if older == Some(Ok(())) {
                            // From now on this boot's conversations open with it, and say
                            // so no more.
                            self.owner_secret = legacy;
                            self.legacy_owner_secret = None;
                            self.owner_refused = false;
                            log!("se051: owner session open with the older secret");
                        } else {
                            log!("se051: the owner session did not open ({e:?})");
                            // Only the chip refusing every secret the badge has means the
                            // chip is another's. A bus error may pass at the next boot.
                            let refused = |r: Result<(), ChipError>| r == Err(ChipError::BadSession);
                            self.owner_refused = refused(Err(e)) && older.is_none_or(refused);
                            self.chip_end();
                            return None;
                        }
                    }
                },
                (_, Ok(false)) => log!("se051: no owner object yet, no session"),
                (None, Ok(true)) => log!("se051: no owner secret, no session"),
                (_, Err(e)) => log!("se051: could not look for the owner object ({e:?})"),
            }
        }
        self.chip.as_mut()
    }

    /// `se051: applet x.y.z in the owner session, N circles`, N the seat keys the chip holds;
    /// `se051: not found` when it did not answer at boot. A chip that answered at boot but not
    /// now is dropped, so the badge runs without one. The badge ends the conversation.
    fn selftest_chip(&mut self) -> String {
        let Some(chip) = self.wake() else {
            // The chip answered but is not this badge's: the boot refuses it and says so.
            if self.owner_refused {
                return "se051: the owner session did not open".into();
            }
            if self.chip.take().is_some() {
                log!("se051: answered at boot, not now");
            }
            return "se051: not found".into();
        };
        let session = if chip.in_session() { "in the owner session" } else { "in no session" };
        let line = chip.version().and_then(|v| {
            let seats = chip
                .ids()?
                .iter()
                .filter(|id| KeyId::parse(id.0).is_some_and(|(k, _)| k == KeyKind::Seat))
                .count();
            Ok(format!(
                "se051: applet {}.{}.{} {session}, {seats} circles",
                v.major, v.minor, v.patch
            ))
        });
        line.unwrap_or_else(|e| {
            log!("se051: answered at boot, not now ({e:?})");
            self.chip_end();
            self.chip = None;
            "se051: not found".into()
        })
    }
}

/// The laptop drops the pin after this long with no APDU, as a phone's field goes away.
#[cfg(feature = "bench")]
const BENCH_RAW_IDLE: crate::clock::Duration = crate::clock::Duration::from_secs(5);

/// A window the laptop opened for a phone's tap stays open this long with nothing sent.
#[cfg(feature = "bench")]
const BENCH_WINDOW_IDLE: crate::clock::Duration = crate::clock::Duration::from_secs(120);

/// The laptop's one-byte request for a window: the pin high, the wire silent, no session.
#[cfg(feature = "bench")]
pub const BENCH_WINDOW: u8 = 0x01;

/// The laptop's one-byte request to find out whether the pin really cuts the chip's power:
/// the badge speaks to the chip with the pin low and logs whether it answered.
#[cfg(feature = "bench")]
pub const BENCH_PIN_TEST: u8 = 0x02;

/// The laptop's one-byte request to time the chip's start: the pin low for 200 ms with
/// nothing said, then high, then the version asked every 20 ms until it answers.
#[cfg(feature = "bench")]
pub const BENCH_START_TIME: u8 = 0x03;


#[cfg(feature = "bench")]
impl PicoBoard {
    /// Raise the pin for the laptop's APDUs and wake the chip, in no session: the chip as a
    /// phone's tap finds it. The owner session is never opened here. `None` while the badge's
    /// own conversation is open or the chip is held.
    fn chip_raw(&mut self) -> Option<&mut Chip> {
        if self.chip.is_none() || self.chip_held || self.chip_up {
            return None;
        }
        if self.bench_window {
            log!("bench: the laptop speaks on the wire inside the window, which ends");
            self.tap_end();
            self.bench_window = false;
            self.bench_raw = None;
        } else if self.tap_open {
            // The badge cut the chip's power for a phone's tap: the pairing page, the note
            // after a pairing, or home's window. The laptop stands in for that phone, on the
            // wire, which needs the power. The window ends, as a phone's tap ends it.
            log!("bench: the laptop's tap stands in for a phone's, the chip gets its power back");
            self.tap_end();
            self.bench_raw = None;
        }
        if self.bench_raw.is_none() {
            let _ = self.chip_enable.set_high();
            log!("bench: pin high for the laptop, no session");
            self.delay.delay_ms(CHIP_START_MS);
            if let Err(e) = self.chip.as_mut()?.restart() {
                log!("bench: the chip did not answer after the pin rose ({e:?})");
                let _ = self.chip_enable.set_low();
                return None;
            }
        }
        self.bench_raw = Some(crate::clock::Instant::now());
        self.chip.as_mut()
    }

    /// Drop the pin the laptop's APDUs held, and give the chip a moment without power, so
    /// the next conversation finds it with no session.
    fn chip_raw_end(&mut self) {
        if self.bench_raw.take().is_some() {
            let _ = self.chip_enable.set_low();
            if self.bench_window {
                self.bench_window = false;
                self.tap_end();
                log!("bench: the window is over");
            }
            log!("bench: pin low, the laptop's exchange is over");
            self.delay.delay_ms(BENCH_RAW_GAP_MS);
        }
    }

    /// Cut the chip's power for a phone's tap, as the badge's own tap pages do, and keep it
    /// cut until the laptop's next APDU or the idle. For checking the power switch on the
    /// bench: a phone's NFC check should find the chip while the window is open.
    fn chip_window(&mut self) {
        if self.chip.is_none() || self.chip_held || self.chip_up {
            log!("bench: no window, the badge holds its chip");
            return;
        }
        if self.bench_raw.is_some() {
            self.chip_raw_end();
        }
        self.tap_begin();
        self.bench_raw = Some(crate::clock::Instant::now());
        self.bench_window = true;
        log!("bench: window open, the chip's power is cut, a phone may tap");
    }

    /// With the pin low for 200 ms, ask the chip for its version over the wire. A chip whose
    /// power the pin cuts cannot answer. Then the pin rises again and the same is asked.
    fn chip_pin_test(&mut self) {
        if self.chip.is_none() || self.chip_held || self.chip_up {
            log!("bench: no pin test, the badge holds its chip");
            return;
        }
        self.chip_raw_end();
        let _ = self.chip_enable.set_low();
        self.delay.delay_ms(200);
        let low = self.chip.as_mut().map(|c| c.restart().and_then(|_| c.version()));
        match low {
            Some(Ok(v)) => log!("bench: pin LOW and the chip answered, applet {}.{}.{}: the pin does not cut its power", v.major, v.minor, v.patch),
            Some(Err(e)) => log!("bench: pin low and the chip did not answer ({e:?}): the pin cuts its power"),
            None => {}
        }
        let _ = self.chip_enable.set_high();
        self.delay.delay_ms(CHIP_START_MS);
        let high = self.chip.as_mut().map(|c| c.restart().and_then(|_| c.version()));
        match high {
            Some(Ok(v)) => log!("bench: pin high and the chip answered, applet {}.{}.{}", v.major, v.minor, v.patch),
            Some(Err(e)) => log!("bench: pin high and the chip did not answer ({e:?})"),
            None => {}
        }
        let _ = self.chip_enable.set_low();
        self.delay.delay_ms(BENCH_RAW_GAP_MS);
    }

    /// How long after the pin rises the chip first answers, with no wire traffic while it
    /// was down. Logged in ms, or "no answer within 2 s".
    fn chip_start_time(&mut self) {
        if self.chip.is_none() || self.chip_held || self.chip_up {
            log!("bench: no start timing, the badge holds its chip");
            return;
        }
        self.chip_raw_end();
        let _ = self.chip_enable.set_low();
        self.delay.delay_ms(200);
        let _ = self.chip_enable.set_high();
        let t = crate::clock::Instant::now();
        let mut answered = None;
        for _ in 0..100 {
            self.delay.delay_ms(20);
            let ok = self.chip.as_mut().map(|c| c.restart().and_then(|_| c.version()).is_ok());
            if ok == Some(true) {
                answered = Some(t.elapsed());
                break;
            }
        }
        match answered {
            Some(d) => log!("bench: the chip answered {} ms after the pin rose", d.to_millis()),
            None => log!("bench: the chip gave no answer within 2 s of the pin rising"),
        }
        let _ = self.chip_enable.set_low();
        self.delay.delay_ms(BENCH_RAW_GAP_MS);
    }

    /// Answer the laptop's APDUs. Between the badge's own conversations only: while one is
    /// open the APDUs wait for its end.
    fn bench_serve(&mut self) {
        if self.chip_up {
            return;
        }
        let idle = if self.bench_window { BENCH_WINDOW_IDLE } else { BENCH_RAW_IDLE };
        if self.bench_raw.is_some_and(|t| t.elapsed() > idle) {
            self.chip_raw_end();
        }
        while let Some(apdu) = link::bench_apdu() {
            if apdu.is_empty() {
                self.chip_raw_end();
                link::bench_reply(&[]);
                continue;
            }
            if apdu == [BENCH_WINDOW] {
                self.chip_window();
                link::bench_reply(&[0x90, 0x00]);
                continue;
            }
            if apdu == [BENCH_PIN_TEST] {
                link::bench_reply(&[0x90, 0x00]);
                self.chip_pin_test();
                continue;
            }
            if apdu == [BENCH_START_TIME] {
                link::bench_reply(&[0x90, 0x00]);
                self.chip_start_time();
                continue;
            }
            let head = format!("{:02x?}", &apdu[..apdu.len().min(4)]);
            let answer = match self.chip_raw() {
                Some(chip) => match chip.raw(&apdu) {
                    Ok(a) => a,
                    Err(e) => {
                        log!("bench: apdu {head} got no answer ({e:?})");
                        Vec::new()
                    }
                },
                None => Vec::new(),
            };
            if answer.len() >= 2 {
                let sw = &answer[answer.len() - 2..];
                log!("bench: apdu {head} {} bytes, answer {} bytes {:02x}{:02x}", apdu.len(), answer.len() - 2, sw[0], sw[1]);
            }
            link::bench_reply(&answer);
        }
    }
}

/// How long the pin stays low after the laptop's exchange, so the chip powers down.
#[cfg(feature = "bench")]
const BENCH_RAW_GAP_MS: u32 = 100;

/// The Click board's rail wants this long after the transistor turns on, before the enable
/// pin may rise.
const RAIL_START_MS: u32 = 20;

/// The chip wants this long after its enable pin rises before it answers. From a real
/// power-down (the pin low with the Click board's ENA SEL switch on EN) the bench chip first
/// answered about 60 ms after the pin rose (2026-11-01), so 10 ms was never enough.
pub const CHIP_START_MS: u32 = 80;

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
