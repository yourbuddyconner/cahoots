//! [`HostBoard`]: the simulator as a [`Board`], so `cahoots-badge` runs on a laptop.
//!
//! Everything lives in one state directory:
//!
//! - `pad.bin`, `ledger.bin` and `circle.txt`: the same files [`crate::device::Device`]
//!   keeps, so `Device::open` (and `cahoots-host status`, `send`, `recv`) work on a
//!   directory a `HostBoard` kept.
//! - `display.png`: the screen. `screen.txt` holds the text of the latest screen, so a
//!   script or a test can follow the badge without reading the image.
//! - `buttons.txt`: the buttons. Each line `L`, `R` or `B` (Both) is one gesture. The board reads the
//!   file in `delay_ms` and removes it, so each press is delivered once. Write it in one go
//!   (for example `echo B > buttons.txt`, or write a side file and rename it in).
//! - `a_down`: while this file exists, button A is held.

use crate::display::PngDisplay;
use crate::link::{Absent, SocketLink};
use crate::store::{FilePadStore, FileRecordStore};
use cahoots_badge::{word_indices, Board, BoardError, Circle, Press};
use cahoots_core::checkwords::WORDLIST;
use cahoots_core::params::CircleParams;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::{Display, Entropy, Frame, HealthError, Link, PadStore};
use std::collections::VecDeque;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// The ledger region's size, as `Device` uses.
const LEDGER_BYTES: u32 = 64 * 1024;

/// How long `HostBoard::new` keeps trying to reach the `next` socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The value of `listen` or `next` that means nothing is plugged in on that side.
pub const NOTHING: &str = "-";

/// One link of a host board: a handle to a socket, or to nothing plugged in.
///
/// A socket cannot be reconnected cheaply, so `HostBoard` connects its two links once in
/// `new`, and every `links()` call hands out new handles to the same two. Dropping a handle
/// keeps the socket open. Use one handle at a time: two handles reading at once would split
/// the incoming bytes between them.
#[derive(Clone)]
pub struct HostLink(Arc<Mutex<Box<dyn Link + Send>>>);

impl HostLink {
    fn new(link: impl Link + Send + 'static) -> Self {
        HostLink(Arc::new(Mutex::new(Box::new(link))))
    }

    fn with<T>(&self, f: impl FnOnce(&mut Box<dyn Link + Send>) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Link for HostLink {
    fn send(&mut self, frame: &[u8]) {
        self.with(|l| l.send(frame))
    }
    fn recv(&mut self, buf: &mut [u8]) -> usize {
        self.with(|l| l.recv(buf))
    }
    fn peer_present(&self) -> bool {
        self.with(|l| l.peer_present())
    }
    fn flush(&mut self) -> bool {
        self.with(|l| l.flush())
    }
}

/// The host's random source: the OS CSPRNG, or a seeded xorshift for a reproducible demo.
/// A simulated badge is a `CSPRNG` source, not a hardware TRNG. Never seed a real circle.
pub enum HostEntropy {
    Os,
    Seeded(u64),
}

impl Entropy for HostEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        match self {
            HostEntropy::Os => getrandom::fill(buf).map_err(|_| HealthError::SourceUnavailable),
            HostEntropy::Seeded(s) => {
                for b in buf.iter_mut() {
                    *s ^= *s << 13;
                    *s ^= *s >> 7;
                    *s ^= *s << 17;
                    *b = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
                }
                Ok(())
            }
        }
    }
}

/// [`PngDisplay`] plus `screen.txt`, the text of the latest screen.
pub struct HostDisplay {
    png: PngDisplay,
    screen_path: PathBuf,
}

impl HostDisplay {
    pub fn new(dir: &Path, name: &str) -> Self {
        HostDisplay {
            png: PngDisplay::new(&dir.join("display.png"), name),
            screen_path: dir.join("screen.txt"),
        }
    }
}

impl HostDisplay {
    /// Write the screen's lines to `screen.txt`, one per line.
    fn note(&mut self, lines: &[String]) {
        // Write a side file and rename it in, so a reader never sees half a screen.
        let tmp = self.screen_path.with_extension("tmp");
        if std::fs::write(&tmp, lines.join("\n")).is_ok() {
            let _ = std::fs::rename(&tmp, &self.screen_path);
        }
    }
}

impl Display for HostDisplay {
    fn draw(&mut self, frame: &Frame) {
        self.png.draw(frame);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Left,
    Right,
    Both,
}

/// One simulated badge as a [`Board`]. See the module docs for the files it keeps.
pub struct HostBoard {
    dir: PathBuf,
    name: String,
    pad_len: u32,
    entropy: Tested<HostEntropy>,
    display: HostDisplay,
    buttons: VecDeque<Button>,
    /// The `(down, up)` links, connected once in `new`. `links()` hands out handles.
    links: (HostLink, HostLink),
}

impl HostBoard {
    /// Open or create the board under `state_dir` and plug in its two links.
    ///
    /// `listen` is the down link: a Unix socket path the card below connects to. `next` is
    /// the up link: the card above's `listen` path, which this board connects to, retrying
    /// for up to 30 seconds. Either may be [`NOTHING`] (`"-"`) or empty, for the bottom
    /// card's down link and the top card's up link. `new` blocks until a card below has
    /// connected. `name` and `pad_len` are what this card proposes if it is the bottom of
    /// the pile. `seed` replaces the OS random source with a reproducible one.
    ///
    /// Panics if the state directory cannot be created or a socket cannot be connected.
    pub fn new(
        state_dir: &Path,
        listen: &str,
        next: &str,
        name: &str,
        pad_len: u32,
        seed: Option<u64>,
    ) -> HostBoard {
        std::fs::create_dir_all(state_dir)
            .unwrap_or_else(|e| panic!("state dir {}: {e}", state_dir.display()));
        let links = connect(listen, next).unwrap_or_else(|e| panic!("link: {e}"));
        let source = match seed {
            Some(s) => HostEntropy::Seeded(s),
            None => HostEntropy::Os,
        };
        HostBoard {
            dir: state_dir.to_path_buf(),
            name: name.to_string(),
            pad_len,
            entropy: Tested::new(source, HealthTests::for_min_entropy(7)),
            display: HostDisplay::new(state_dir, name),
            buttons: VecDeque::new(),
            links,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Move any presses written to `buttons.txt` into the queue, and remove the file.
    fn poll_buttons(&mut self) {
        let path = self.dir.join("buttons.txt");
        let taken = self.dir.join("buttons.taken");
        // Rename first: a press written after this lands in a new file, not a lost one.
        if std::fs::rename(&path, &taken).is_err() {
            return;
        }
        let text = std::fs::read_to_string(&taken).unwrap_or_default();
        let _ = std::fs::remove_file(&taken);
        for line in text.lines() {
            match line.trim() {
                "L" | "l" => self.buttons.push_back(Button::Left),
                "R" | "r" => self.buttons.push_back(Button::Right),
                "B" | "b" => self.buttons.push_back(Button::Both),
                "" => {}
                other => eprintln!("[{}] buttons.txt: ignored {other:?}", self.name),
            }
        }
    }
}

/// Plug in `(down, up)`: accept the card below on `listen` and connect to `next`. Binds
/// `listen` before connecting, so two boards that start at once find each other.
pub fn connect(listen: &str, next: &str) -> io::Result<(HostLink, HostLink)> {
    let absent = |s: &str| s.is_empty() || s == NOTHING;
    let listener = if absent(listen) {
        None
    } else {
        let _ = std::fs::remove_file(listen);
        Some(UnixListener::bind(listen)?)
    };
    let up = if absent(next) {
        HostLink::new(Absent)
    } else {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let stream = loop {
            match UnixStream::connect(next) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => return Err(e),
            }
        };
        HostLink::new(SocketLink::new(stream)?)
    };
    let down = match listener {
        Some(l) => HostLink::new(SocketLink::new(l.accept()?.0)?),
        None => HostLink::new(Absent),
    };
    Ok((down, up))
}

impl Board for HostBoard {
    type Entropy = Tested<HostEntropy>;
    type Pad = FilePadStore;
    type Records = FileRecordStore;
    type Display = HostDisplay;
    type Link = HostLink;

    fn entropy(&mut self) -> &mut Self::Entropy {
        &mut self.entropy
    }

    fn display(&mut self) -> &mut HostDisplay {
        &mut self.display
    }

    /// The next queued gesture, one per call: `L`, `R` or `B` (Both).
    fn press(&mut self) -> Press {
        match self.buttons.pop_front() {
            Some(Button::Left) => Press::Left,
            Some(Button::Right) => Press::Right,
            Some(Button::Both) => Press::Both,
            None => Press::None,
        }
    }

    fn a_down(&self) -> bool {
        self.dir.join("a_down").exists()
    }

    fn delay_ms(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
        self.poll_buttons();
    }

    /// New handles to the two links connected in `new`. See [`HostLink`].
    fn links(&mut self) -> (HostLink, HostLink) {
        self.links.clone()
    }

    fn load_circle(&self) -> Option<Circle> {
        let txt = Zeroizing::new(std::fs::read_to_string(self.dir.join("circle.txt")).ok()?);
        let field = |k: &str| {
            txt.lines()
                .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
        };
        let params = CircleParams::decode(&unhex(field("params")?)?).ok()?;
        let seat = field("index")?.parse().ok()?;
        let circle_id = unhex(field("circle_id")?)?.try_into().ok()?;
        let words: Vec<&str> = field("words")?.split(' ').collect();
        let words = word_indices(&words.try_into().ok()?)?;
        // A circle file from before seat keys has no `seat_key=` line: no circle.
        let seat_key =
            <[u8; 32]>::try_from(&Zeroizing::new(unhex(field("seat_key")?)?)[..]).ok()?;
        let tier = field("tier")?.parse().ok()?;
        let manifest = unhex(field("manifest")?)?;
        Some(Circle {
            params,
            seat,
            circle_id,
            words,
            seat_key,
            tier,
            manifest,
        })
    }

    /// Writes `circle.txt` in the format `Device::from_result` writes. The seat key goes in
    /// as plaintext hex, as the pad sits in `pad.bin`: a simulated badge is tier 0.
    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError> {
        let words: Vec<&str> = c
            .words
            .iter()
            .map(|&i| WORDLIST[usize::from(i) % WORDLIST.len()])
            .collect();
        let txt = Zeroizing::new(format!(
            "params={}\nindex={}\ncircle_id={}\nwords={}\nlabel=CSPRNG\nseat_key={}\ntier={}\nmanifest={}\n",
            hex(&c.params.encode()),
            c.seat,
            hex(&c.circle_id),
            words.join(" "),
            *Zeroizing::new(hex(&c.seat_key)),
            c.tier,
            hex(&c.manifest)
        ));
        std::fs::write(self.dir.join("circle.txt"), txt.as_bytes()).map_err(|_| BoardError::Storage)
    }

    fn wipe(&mut self, pad_len: u32) -> Result<(), BoardError> {
        match std::fs::remove_file(self.dir.join("circle.txt")) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(BoardError::Storage),
        }
        FileRecordStore::create(&self.dir.join("ledger.bin"), LEDGER_BYTES)
            .map_err(|_| BoardError::Storage)?;
        let pad = self.dir.join("pad.bin");
        if pad.exists() {
            let mut store = FilePadStore::open(&pad).map_err(|_| BoardError::Storage)?;
            let n = pad_len.min(store.len());
            store.erase(0, n).map_err(|_| BoardError::Storage)?;
        }
        Ok(())
    }

    fn write_pad(&mut self, pad: &[u8]) -> Result<FilePadStore, BoardError> {
        FilePadStore::create(&self.dir.join("pad.bin"), pad).map_err(|_| BoardError::Storage)
    }

    /// Panics if `pad.bin` is missing or shorter than `len`: the circle record names a pad
    /// this directory does not hold.
    fn open_pad(&mut self, len: u32) -> FilePadStore {
        let path = self.dir.join("pad.bin");
        let store = FilePadStore::open(&path)
            .unwrap_or_else(|e| panic!("open_pad {}: {e}", path.display()));
        assert!(
            store.len() >= len,
            "pad.bin holds {} of {len} bytes",
            store.len()
        );
        store
    }

    fn records(&mut self, fresh: bool) -> Result<FileRecordStore, BoardError> {
        let path = self.dir.join("ledger.bin");
        if fresh || !path.exists() {
            FileRecordStore::create(&path, LEDGER_BYTES)
        } else {
            FileRecordStore::open(&path)
        }
        .map_err(|_| BoardError::Storage)
    }

    fn proposal(&self) -> (String, u32) {
        (self.name.clone(), self.pad_len)
    }

    fn tier(&self) -> u8 {
        0
    }

    fn selftest(&mut self) -> Vec<String> {
        vec![format!("files: {}", self.dir.display())]
    }

    fn log(&mut self, line: &str) {
        eprintln!("[{}] {line}", self.name);
    }

    /// The screen's text goes to stderr and `screen.txt`. A key screen goes only to the
    /// file, which the simulator's tests read; the terminal log keeps no copy of a key.
    fn note_screen(&mut self, lines: &[String], secret: bool) {
        if secret {
            eprintln!("[{}] (a key screen)", self.name);
        } else {
            eprintln!("[{}] {}", self.name, lines.join("\n"));
        }
        self.display.note(lines);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
