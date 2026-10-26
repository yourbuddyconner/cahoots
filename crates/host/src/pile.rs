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
use std::time::{Duration, Instant};

pub type BoxLink = Box<dyn Link + Send>;

/// How long a simulated card waits for its seat and the count before giving up.
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
    let ready = loop {
        if let Some(r) = pile.poll().map_err(pile_err)? {
            break r;
        }
        if press && !pressed && pile.is_top() && pile.seat().is_some() {
            pile.start().map_err(pile_err)?;
            pressed = true;
        }
        if started.elapsed() > PILE_TIMEOUT {
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
        };
        handles.push(std::thread::spawn(move || run_card(card)));
    }
    handles
        .into_iter()
        .map(|h| {
            h.join()
                .map_err(|_| DeviceError::State("card thread panicked".into()))?
        })
        .collect()
}
