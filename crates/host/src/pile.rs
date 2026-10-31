//! A pile of cards on a laptop: simulated cards on Unix sockets, with a real board on a serial
//! port as the top card if there is one.
//!
//! Card 0 is the bottom. Each simulated card runs in its own thread: it learns its seat from
//! the card below, and the top card presses B itself unless a board is on top, in which case
//! the board's button starts the count. Every card then runs the ceremony and persists like
//! `cahoots-host ceremony` does.

use crate::device::{Device, DeviceError};
use crate::display::PngDisplay;
use crate::link::{Absent, SerialLink, SocketLink};
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::pile::{Pile, PileError};
use cahoots_hal::{Entropy, HealthError, Link};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type BoxLink = Box<dyn Link + Send>;

/// How long an all-simulated pile waits for its seat and the count before giving up. With a
/// real board on top the pile waits for the person holding it, however long that takes.
pub const PILE_TIMEOUT: Duration = Duration::from_secs(120);

pub struct Card {
    pub down: BoxLink,
    pub up: BoxLink,
    pub name: String,
    pub pad_len: u32,
    /// Press B once the seat is known and this card is the top: the all-simulated case.
    pub press: bool,
    pub dir: PathBuf,
    pub seed: Option<u64>,
    /// Give up on the count after [`PILE_TIMEOUT`]. Off when a person's board is on top.
    pub timeout: bool,
}

struct OsEntropy;
impl Entropy for OsEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        getrandom::fill(buf).map_err(|_| HealthError::SourceUnavailable)
    }
}

struct Seeded(u64);
impl Entropy for Seeded {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        for b in buf.iter_mut() {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
        Ok(())
    }
}

fn pile_err(e: PileError) -> DeviceError {
    DeviceError::State(format!("pile: {e:?}"))
}

/// Run one card to completion: find the seat, wait for the count, run the ceremony, persist.
pub fn run_card(card: Card) -> Result<Device, DeviceError> {
    let Card {
        down,
        up,
        name,
        pad_len,
        press,
        dir,
        seed,
        timeout,
    } = card;
    let mut pile = Pile::new(down, up, &name, pad_len, false);
    let mut display = PngDisplay::new(&dir.join("display.png"), &name);
    std::fs::create_dir_all(&dir)?;
    let tests = HealthTests::for_min_entropy(7);
    let mut entropy: Box<dyn Entropy> = match seed {
        Some(s) => Box::new(Tested::new(Seeded(s), tests)),
        None => Box::new(Tested::new(OsEntropy, tests)),
    };
    let started = Instant::now();
    let mut pressed = false;
    let mut seen = (None, None);
    let ready = loop {
        if let Some(r) = pile.poll().map_err(pile_err)? {
            break r;
        }
        if crate::link::trace_frames() && seen != (pile.seat(), pile.count()) {
            seen = (pile.seat(), pile.count());
            eprintln!("[{name}] pile: seat {:?} count {:?}", seen.0, seen.1);
        }
        if press && !pressed && pile.is_top() && pile.seat().is_some() {
            pile.start().map_err(pile_err)?;
            pressed = true;
        }
        if timeout && started.elapsed() > PILE_TIMEOUT {
            return Err(DeviceError::State(format!(
                "{name}: no count after {}s (seat {:?})",
                PILE_TIMEOUT.as_secs(),
                pile.seat()
            )));
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    display.show_lines(&[&format!("seat {} of {}", ready.seat, ready.params.n)]);
    let seat = ready.seat;
    let params = ready.params.clone();
    let ring = pile
        // The host is a T0 device.
        .into_ceremony(ready, entropy.as_mut(), 0)
        .map_err(DeviceError::Ring)?;
    let (result, _links) = ring
        .run(
            |s| display.show_lines(&[s]),
            || std::thread::sleep(Duration::from_millis(1)),
        )
        .map_err(DeviceError::Ring)?;
    crate::device::show_words(&mut display, &result.check_words);
    Device::from_result(&dir, params, seat, &result)
}

/// Run one card in its own thread.
fn spawn(card: Card) -> JoinHandle<Result<Device, DeviceError>> {
    std::thread::spawn(move || run_card(card))
}

/// Run a simulated card on links the caller made: `down` to the card or bridge below, `up`
/// to the card above ([`Absent`] for the top). It never presses B, so a card below or a
/// bridge starts the count, and it gives up after [`PILE_TIMEOUT`] without one. It proposes
/// a 16 KiB pad, used only if it turns out to be the bottom.
pub fn spawn_card(
    down: BoxLink,
    up: BoxLink,
    name: &str,
    dir: PathBuf,
    seed: Option<u64>,
) -> JoinHandle<Result<Device, DeviceError>> {
    spawn(Card {
        down,
        up,
        name: name.to_string(),
        pad_len: 16 * 1024,
        press: false,
        dir,
        seed,
        timeout: true,
    })
}

/// Build and run a pile of `cards` cards under `state`. With `serial`, the top card is the
/// board on that port and `cards` counts it; the laptop simulates the other `cards - 1`.
/// Returns the simulated devices, bottom first.
pub fn run_pile(
    cards: usize,
    serial: Option<&str>,
    name: &str,
    pad_len: u32,
    state: &Path,
    seed: Option<u64>,
) -> Result<Vec<Device>, DeviceError> {
    if cards < 2 {
        return Err(DeviceError::State("a pile needs at least two cards".into()));
    }
    let simulated = if serial.is_some() { cards - 1 } else { cards };
    // Cable i joins card i (its up end) and card i + 1 (its down end).
    let mut ups: Vec<Option<BoxLink>> = (0..simulated).map(|_| None).collect();
    let mut downs: Vec<Option<BoxLink>> = (0..simulated).map(|_| None).collect();
    for i in 0..simulated.saturating_sub(1) {
        let (a, b) = UnixStream::pair()?;
        ups[i] = Some(Box::new(SocketLink::new(a)?));
        downs[i + 1] = Some(Box::new(SocketLink::new(b)?));
    }
    if let Some(port) = serial {
        // The bench driver, if on, presses Both on the badge once it is shown this seat.
        std::env::set_var("CAHOOTS_BENCH_SEAT", (cards - 1).to_string());
        let link = SerialLink::open(port, "board")?;
        ups[simulated - 1] = Some(Box::new(link));
    }
    let mut handles = Vec::new();
    for i in 0..simulated {
        let card = Card {
            down: downs[i].take().unwrap_or_else(|| Box::new(Absent)),
            up: ups[i].take().unwrap_or_else(|| Box::new(Absent)),
            name: if i == 0 {
                name.to_string()
            } else {
                format!("card{i}")
            },
            pad_len,
            press: serial.is_none() && i == simulated - 1,
            dir: state.join(format!("card{i}")),
            seed: seed.map(|s| s.wrapping_add(i as u64)),
            timeout: serial.is_none(),
        };
        handles.push(spawn(card));
    }
    handles
        .into_iter()
        .map(|h| {
            h.join()
                .map_err(|_| DeviceError::State("card thread panicked".into()))?
        })
        .collect()
}
