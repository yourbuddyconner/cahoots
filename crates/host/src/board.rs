//! [`HostBoard`]: the simulator as a [`Board`], so `cahoots-badge` runs on a laptop.
//!
//! Everything lives in one state directory:
//!
//! - `pad.bin`, `ledger.bin` and `circle.txt`: the first circle, in the same files
//!   [`crate::device::Device`] keeps, so `Device::open` (and `cahoots-host status`, `send`,
//!   `recv`) work on a directory a `HostBoard` kept.
//! - `circle-<k>.txt`, `pad-<k>.bin` and `ledger-<k>.bin` for `k` from 1 to 15: the circles
//!   after the first, one place each. A circle keeps its place; a new one takes the lowest
//!   free place.
//! - `display.png`: the screen. `screen.txt` holds the text of the latest screen, so a
//!   script or a test can follow the badge without reading the image.
//! - `buttons.txt`: the buttons. Each line `L`, `R` or `B` (Both) is one gesture. The board reads the
//!   file in `delay_ms` and removes it, so each press is delivered once. Write it in one go
//!   (for example `echo B > buttons.txt`, or write a side file and rename it in).
//! - `a_down`: while this file exists, button A is held.
//! - `chip.cbor`: the chip record (see `cahoots_badge::chip`), once a chip has made a key.
//! - `softchip.bin`: the software chip's objects, for a board made with
//!   [`HostBoard::with_soft_chip`] (`cahoots-host badge --chip soft`). Written after every
//!   chip call, so a restarted simulator keeps its chip keys. It holds the keys in the clear:
//!   a simulated chip is no safer than the files beside it.

use crate::display::PngDisplay;
use crate::link::{Absent, SocketLink};
use crate::store::{FilePadStore, FileRecordStore};
use cahoots_badge::chip::ChipRecord;
use cahoots_badge::board::{assign_places, place_for};
use cahoots_badge::{word_indices, Board, BoardError, Circle, Press};
use cahoots_core::circle_id::CircleId;
use cahoots_chip::{Attested, ChipError, ChipOps, Curve, KeyId, Signature64, SoftChip, Version};
use cahoots_core::checkwords::WORDLIST;
use cahoots_core::params::CircleParams;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::{Display, Entropy, Frame, HealthError, Link, PadStore};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// The ledger region's size, as `Device` uses.
const LEDGER_BYTES: u32 = 64 * 1024;

/// How many circles a simulated badge holds: the Pico's count.
pub const MAX_CIRCLES: usize = 16;

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
    seed: Option<u64>,
    chip: Option<HostChip>,
    /// The place a pad was written to for a circle not stored yet.
    pending: Option<(CircleId, usize)>,
}

/// The simulated chip: a [`SoftChip`] kept in a file and written after every call that can
/// change it, so the chip a simulator restarts with is the chip it stopped with. The badge
/// reconciles its chip record against the chip at boot; an empty chip would make it forget
/// keys the record names.
pub struct HostChip {
    chip: SoftChip,
    path: PathBuf,
}

impl HostChip {
    /// Open the chip kept at `path`, or make an empty one there. `seed` makes the new chip's
    /// keys repeatable; a kept chip carries on from where it was.
    pub fn open(path: &Path, seed: Option<u64>) -> io::Result<HostChip> {
        let chip = match std::fs::read(path) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                SoftChip::import(&bytes).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "not a software chip")
                })?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let chip = match seed {
                    Some(s) => {
                        let mut bytes = [0u8; 32];
                        bytes[..8].copy_from_slice(b"softchip");
                        bytes[8..16].copy_from_slice(&s.to_le_bytes());
                        SoftChip::with_seed(bytes)
                    }
                    None => SoftChip::new(),
                };
                let made = HostChip {
                    chip,
                    path: path.to_path_buf(),
                };
                made.save()?;
                return Ok(made);
            }
            Err(e) => return Err(e),
        };
        Ok(HostChip {
            chip,
            path: path.to_path_buf(),
        })
    }

    /// Write the chip to a side file readable only by this user, then rename it in.
    fn save(&self) -> io::Result<()> {
        let bytes = Zeroizing::new(self.chip.export());
        let tmp = self.path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_data()?;
        std::fs::rename(&tmp, &self.path)
    }

    /// `r`, once the chip is written; a chip that cannot be written is a failed call.
    fn kept<T>(&mut self, r: Result<T, ChipError>) -> Result<T, ChipError> {
        let v = r?;
        self.save().map_err(|_| ChipError::Transport)?;
        Ok(v)
    }
}

impl ChipOps for HostChip {
    fn version(&mut self) -> Result<Version, ChipError> {
        self.chip.version()
    }
    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError> {
        let r = self.chip.random(n);
        self.kept(r)
    }
    fn generate(&mut self, id: KeyId, curve: Curve) -> Result<Vec<u8>, ChipError> {
        let r = self.chip.generate(id, curve);
        self.kept(r)
    }
    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        self.chip.public(id)
    }
    fn delete(&mut self, id: KeyId) -> Result<(), ChipError> {
        let r = self.chip.delete(id);
        self.kept(r)
    }
    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError> {
        self.chip.exists(id)
    }
    fn ids(&mut self) -> Result<Vec<KeyId>, ChipError> {
        self.chip.ids()
    }
    fn sign_p256(&mut self, id: KeyId, digest: &[u8; 32]) -> Result<Signature64, ChipError> {
        self.chip.sign_p256(id, digest)
    }
    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError> {
        self.chip.sign_ed25519(id, message)
    }
    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError> {
        // The chip's clock moves.
        let r = self.chip.attest(id, freshness);
        self.kept(r)
    }
    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32]) -> Result<(), ChipError> {
        let r = self.chip.put_hmac_key(id, key);
        self.kept(r)
    }
    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError> {
        self.chip.hmac(id, data)
    }
    fn put_counter(&mut self, id: KeyId, value: u64) -> Result<(), ChipError> {
        let r = self.chip.put_counter(id, value);
        self.kept(r)
    }
    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError> {
        let r = self.chip.counter_next(id);
        self.kept(r)
    }
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
            seed,
            chip: None,
            pending: None,
        }
    }

    /// Give the board a software chip, kept in `softchip.bin` in its state directory: the one
    /// kept there, or a new empty one. With a chip the badge makes seat keys in it.
    ///
    /// Panics if `softchip.bin` exists but cannot be read: a board must not boot with an empty
    /// chip in place of one whose keys its circle names.
    pub fn with_soft_chip(mut self) -> HostBoard {
        let path = self.dir.join("softchip.bin");
        let chip = HostChip::open(&path, self.seed)
            .unwrap_or_else(|e| panic!("software chip {}: {e}", path.display()));
        self.chip = Some(chip);
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The circle record, pad and ledger files of place `p`.
    fn place_files(&self, p: usize) -> (PathBuf, PathBuf, PathBuf) {
        if p == 0 {
            (
                self.dir.join("circle.txt"),
                self.dir.join("pad.bin"),
                self.dir.join("ledger.bin"),
            )
        } else {
            (
                self.dir.join(format!("circle-{p}.txt")),
                self.dir.join(format!("pad-{p}.bin")),
                self.dir.join(format!("ledger-{p}.bin")),
            )
        }
    }

    /// The circle id each place holds.
    fn held(&self) -> Vec<Option<CircleId>> {
        (0..MAX_CIRCLES)
            .map(|p| read_circle(&self.place_files(p).0).map(|c| c.circle_id))
            .collect()
    }

    fn place(&self, circle: &CircleId) -> Result<usize, BoardError> {
        place_for(&self.held(), self.pending, circle).ok_or(BoardError::Storage)
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

    fn max_circles(&self) -> usize {
        MAX_CIRCLES
    }

    fn load_circles(&mut self) -> Vec<Circle> {
        (0..MAX_CIRCLES)
            .filter_map(|p| read_circle(&self.place_files(p).0))
            .collect()
    }

    /// Writes each circle's file in the format `Device::from_result` writes, and removes the
    /// file of a place whose circle is not in the list.
    fn store_circles(&mut self, circles: &[Circle]) -> Result<(), BoardError> {
        let places = assign_places(&self.held(), self.pending, circles)?;
        for (p, i) in places.into_iter().enumerate() {
            let path = self.place_files(p).0;
            match i {
                Some(i) => write_circle(&path, &circles[i])?,
                None => match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => return Err(BoardError::Storage),
                },
            }
        }
        self.pending = None;
        Ok(())
    }

    /// Removes the circle's file first, then erases the first `pad_len` bytes of its pad and
    /// its ledger.
    fn forget(&mut self, circle: &CircleId, pad_len: u32) -> Result<(), BoardError> {
        let (record, pad, ledger) = self.place_files(self.place(circle)?);
        match std::fs::remove_file(&record) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(BoardError::Storage),
        }
        if pad.exists() {
            let mut store = FilePadStore::open(&pad).map_err(|_| BoardError::Storage)?;
            let n = pad_len.min(store.len());
            store.erase(0, n).map_err(|_| BoardError::Storage)?;
        }
        FileRecordStore::create(&ledger, LEDGER_BYTES).map_err(|_| BoardError::Storage)?;
        Ok(())
    }

    fn write_pad(&mut self, circle: &CircleId, pad: &[u8]) -> Result<FilePadStore, BoardError> {
        let p = self.place(circle)?;
        self.pending = Some((*circle, p));
        FilePadStore::create(&self.place_files(p).1, pad).map_err(|_| BoardError::Storage)
    }

    /// An error if the circle's pad file is missing or shorter than `len`: its record names a
    /// pad this directory does not hold.
    fn open_pad(&mut self, circle: &CircleId, len: u32) -> Result<FilePadStore, BoardError> {
        let path = self.place_files(self.place(circle)?).1;
        let store = FilePadStore::open(&path).map_err(|_| BoardError::Storage)?;
        if store.len() < len {
            return Err(BoardError::Storage);
        }
        Ok(store)
    }

    fn records(&mut self, circle: &CircleId, fresh: bool) -> Result<FileRecordStore, BoardError> {
        let path = self.place_files(self.place(circle)?).2;
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
        let mut lines = vec![format!("files: {}", self.dir.display())];
        if self.chip.is_some() {
            lines.push("chip: software, in softchip.bin".into());
        }
        lines
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

    fn chip(&mut self) -> Option<&mut dyn ChipOps> {
        self.chip.as_mut().map(|c| c as &mut dyn ChipOps)
    }

    /// `chip.cbor`: `Ok(None)` if it is missing, an error if it cannot be read or does not
    /// decode.
    fn chip_record_load(&mut self) -> Result<Option<ChipRecord>, BoardError> {
        let bytes = match std::fs::read(self.dir.join("chip.cbor")) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(BoardError::Storage),
        };
        ChipRecord::decode(&bytes)
            .map(Some)
            .map_err(|_| BoardError::Storage)
    }

    fn chip_record_store(&mut self, r: &ChipRecord) -> Result<(), BoardError> {
        std::fs::write(self.dir.join("chip.cbor"), r.encode()).map_err(|_| BoardError::Storage)
    }
}

/// The circle in a `circle.txt` file, or `None` if there is none or it does not parse.
fn read_circle(path: &Path) -> Option<Circle> {
    let txt = Zeroizing::new(std::fs::read_to_string(path).ok()?);
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
    let seat_key = <[u8; 32]>::try_from(&Zeroizing::new(unhex(field("seat_key")?)?)[..]).ok()?;
    let tier = field("tier")?.parse().ok()?;
    let manifest = unhex(field("manifest")?)?;
    // Both optional: a file from before the chip has neither. One that does not parse makes
    // the file no circle.
    let chip_slot = match field("chip_slot") {
        Some(v) => Some(v.parse().ok()?),
        None => None,
    };
    let root = match field("root") {
        Some(v) => Some(<[u8; 32]>::try_from(&Zeroizing::new(unhex(v)?)[..]).ok()?),
        None => None,
    };
    Some(Circle {
        params,
        seat,
        circle_id,
        words,
        seat_key,
        chip_slot,
        root,
        tier,
        manifest,
    })
}

/// Write `c` as a `circle.txt` file in the format `Device::from_result` writes. The seat key
/// goes in as plaintext hex, as the pad sits in `pad.bin`: a simulated badge is tier 0. A
/// `chip_slot=` line says the seat key is in the chip, and a `root=` line holds the circle
/// root of a badge without one; each is written only when set.
fn write_circle(path: &Path, c: &Circle) -> Result<(), BoardError> {
    let words: Vec<&str> = c
        .words
        .iter()
        .map(|&i| WORDLIST[usize::from(i) % WORDLIST.len()])
        .collect();
    let mut txt = Zeroizing::new(format!(
        "params={}\nindex={}\ncircle_id={}\nwords={}\nlabel=CSPRNG\nseat_key={}\ntier={}\nmanifest={}\n",
        hex(&c.params.encode()),
        c.seat,
        hex(&c.circle_id),
        words.join(" "),
        *Zeroizing::new(hex(&c.seat_key)),
        c.tier,
        hex(&c.manifest)
    ));
    if let Some(slot) = c.chip_slot {
        txt.push_str(&format!("chip_slot={slot}\n"));
    }
    if let Some(root) = &c.root {
        txt.push_str(&format!("root={}\n", *Zeroizing::new(hex(root))));
    }
    std::fs::write(path, txt.as_bytes()).map_err(|_| BoardError::Storage)
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
