//! `cahoots-host bridge`: the cable's side of the bridge from [`cahoots_hal::bridge`], with the
//! relay threaded around it. One thread posts, one long-polls, and the loop that reads the
//! cable never waits on either, so a board that takes two seconds without bytes for an
//! unplugged laptop never sees a slow relay that way.

pub use cahoots_hal::bridge::{Bridge, BridgeEnv, BridgeError, BridgeStep, PILE_DEADLINE_MS};
use cahoots_hal::bridge::CryptoRngCore;
use cahoots_hal::Link;
use cahoots_relay::{Relay, RelayError};
use rand_core::OsRng;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Relay failures in a row that [`run`] waits out before it gives up.
const RELAY_RETRIES: u32 = 30;
/// How long the relay holds a read open when it has nothing new.
pub const POLL_WAIT_MS: u64 = 2000;
/// The least time between two reads that came back empty, for a relay that does not wait.
const EMPTY_POLL_EVERY: Duration = Duration::from_millis(500);
/// Between passes of the cable's loop.
const LINK_EVERY: Duration = Duration::from_millis(20);

/// The laptop's random source and clock for the bridge.
pub struct HostEnv {
    rng: OsRng,
    started: Instant,
}

impl Default for HostEnv {
    fn default() -> Self {
        HostEnv { rng: OsRng, started: Instant::now() }
    }
}

impl BridgeEnv for HostEnv {
    fn rng(&mut self) -> &mut dyn CryptoRngCore {
        &mut self.rng
    }
    fn now_ms(&mut self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

/// A bridge for the laptop: `Bridge::new` with the laptop's random source and clock.
pub fn new(offer: cahoots_core::channel::Offer, badges_in_pile: u8) -> Result<Bridge, BridgeError> {
    Bridge::new(offer, badges_in_pile, Box::new(HostEnv::default()))
}

/// What ends a [`run`]: the bridge, or the relay.
#[derive(Debug)]
pub enum RunError {
    Bridge(BridgeError),
    Relay(RelayError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Bridge(e) => write!(f, "{e}"),
            RunError::Relay(e) => write!(f, "relay: {e:?}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<BridgeError> for RunError {
    fn from(e: BridgeError) -> Self {
        RunError::Bridge(e)
    }
}

/// [`Bridge::run`] for the laptop.
pub trait Run {
    fn run(
        &mut self,
        link: &mut dyn Link,
        poster: &mut (dyn Relay + Send),
        poller: &mut (dyn Relay + Send),
        on_join_code: &mut dyn FnMut(u8, &str),
        on_signed: &mut dyn FnMut(),
    ) -> Result<(), RunError>;
}

impl Run for Bridge {
    /// Carry the pile into the ceremony until the cable's peer is gone. `on_signed` is called
    /// once, when every card's signature is on the relay and at least one signature for every
    /// other seat has gone down the cable. The bridge cannot check a signature, so it keeps
    /// carrying after that: a forged signature that came first costs nothing, because the real
    /// one still goes down when it comes. Returns `Ok` when the pile is unplugged after
    /// `on_signed`, and `Err(Link)` when it is unplugged before. `poster` posts and `poller`
    /// reads, each on its own thread, and this thread only reads and writes the cable. A relay
    /// that fails is tried again for a while before the bridge gives up.
    fn run(
        &mut self,
        link: &mut dyn Link,
        poster: &mut (dyn Relay + Send),
        poller: &mut (dyn Relay + Send),
        on_join_code: &mut dyn FnMut(u8, &str),
        on_signed: &mut dyn FnMut(),
    ) -> Result<(), RunError> {
        let id = self.offer().channel.id;
        let stop = AtomicBool::new(false);
        let posted = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let (post_tx, post_rx) = mpsc::channel::<Vec<u8>>();
            let (read_tx, read_rx) = mpsc::channel::<Vec<(u64, Vec<u8>)>>();
            let (err_tx, err_rx) = mpsc::channel::<RelayError>();
            let post_err = err_tx.clone();
            let posted = &posted;
            scope.spawn(move || {
                for wire in post_rx {
                    let mut failures = 0;
                    while let Err(e) = poster.post(&id, &wire) {
                        failures += 1;
                        if failures > RELAY_RETRIES {
                            let _ = post_err.send(e);
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    posted.fetch_add(1, Ordering::SeqCst);
                }
            });
            let stop = &stop;
            scope.spawn(move || {
                let mut cursor = None;
                let mut failures = 0;
                while !stop.load(Ordering::SeqCst) {
                    let began = Instant::now();
                    match poller.poll(&id, cursor, POLL_WAIT_MS) {
                        Ok(entries) => {
                            failures = 0;
                            if let Some(&(seq, _)) = entries.last() {
                                cursor = Some(seq);
                            }
                            if entries.is_empty() {
                                if let Some(rest) = EMPTY_POLL_EVERY.checked_sub(began.elapsed()) {
                                    std::thread::sleep(rest);
                                }
                            } else if read_tx.send(entries).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            if failures > RELAY_RETRIES {
                                let _ = err_tx.send(e);
                                return;
                            }
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
            });
            let result = (|| {
                let mut queued = 0;
                let mut signed = false;
                loop {
                    while let Ok(entries) = read_rx.try_recv() {
                        self.deliver(entries);
                    }
                    let step = match self.step(link, on_join_code) {
                        // The pile was unplugged after everyone signed: the bridge is done.
                        Err(BridgeError::Link) if signed => return Ok(()),
                        Err(e) => return Err(RunError::Bridge(e)),
                        Ok(s) => s,
                    };
                    for wire in self.take_posts() {
                        queued += 1;
                        let _ = post_tx.send(wire);
                    }
                    if let Ok(e) = err_rx.try_recv() {
                        return Err(RunError::Relay(e));
                    }
                    if !signed
                        && step == BridgeStep::Done
                        && posted.load(Ordering::SeqCst) == queued
                        && link.flush()
                    {
                        signed = true;
                        on_signed();
                    }
                    std::thread::sleep(LINK_EVERY);
                }
            })();
            stop.store(true, Ordering::SeqCst);
            result
        })
    }
}
