//! One simulated badge. Its state directory holds `pad.bin`, `ledger.bin`, `circle.txt`,
//! and `display.png`.

use crate::display::PngDisplay;
use crate::store::{FilePadStore, FileRecordStore};
use cahoots_core::checkwords::{check_words, WORDLIST};
use cahoots_core::circle_id::CircleId;
use cahoots_core::ledger::SpentLedger;
use cahoots_core::message::{Header, MessageType};
use cahoots_core::pad::{Pad, PadError};
use cahoots_core::params::CircleParams;
use cahoots_hal::ledger::PersistentLedger;
use cahoots_hal::ring::{RingCeremony, RingError};
use cahoots_hal::store::StorePad;
use cahoots_hal::{Entropy, Link};
use std::fmt;
use std::path::{Path, PathBuf};

const LEDGER_BYTES: u32 = 64 * 1024;

#[derive(Debug)]
pub enum DeviceError {
    Io(std::io::Error),
    Ring(RingError),
    Pad(PadError),
    State(String),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Io(e) => write!(f, "io: {e}"),
            DeviceError::Ring(e) => write!(f, "ceremony: {e:?}"),
            DeviceError::Pad(e) => write!(f, "pad: {e:?}"),
            DeviceError::State(s) => write!(f, "state: {s}"),
        }
    }
}

impl From<std::io::Error> for DeviceError {
    fn from(e: std::io::Error) -> Self {
        DeviceError::Io(e)
    }
}
impl From<PadError> for DeviceError {
    fn from(e: PadError) -> Self {
        DeviceError::Pad(e)
    }
}

pub struct Device {
    dir: PathBuf,
    pad: Pad<StorePad<FilePadStore>, PersistentLedger<FileRecordStore>>,
    words: [&'static str; 6],
}

impl Device {
    /// Run a ceremony over `links` (one per neighbour) and persist the result under `dir`.
    pub fn ceremony<L: Link>(
        dir: &Path,
        params: CircleParams,
        index: u8,
        entropy: &mut dyn Entropy,
        links: Vec<L>,
    ) -> Result<Self, DeviceError> {
        std::fs::create_dir_all(dir)?;
        let name = format!("dev{index}");
        let mut display = PngDisplay::new(&dir.join("display.png"), &name);
        let ring =
            RingCeremony::new(params.clone(), index, entropy, links).map_err(DeviceError::Ring)?;
        let (result, _links) = ring
            .run(&mut display, || {
                std::thread::sleep(std::time::Duration::from_millis(1))
            })
            .map_err(DeviceError::Ring)?;
        Self::from_result(dir, params, index, &result)
    }

    /// Persist a finished ceremony under `dir` and open the device on it.
    pub fn from_result(
        dir: &Path,
        params: CircleParams,
        index: u8,
        result: &cahoots_core::ceremony::CeremonyResult,
    ) -> Result<Self, DeviceError> {
        std::fs::create_dir_all(dir)?;
        let store = FilePadStore::create(&dir.join("pad.bin"), &result.pad)?;
        let records = FileRecordStore::create(&dir.join("ledger.bin"), LEDGER_BYTES)?;
        let ledger = PersistentLedger::open(records, params.slices())
            .map_err(|e| DeviceError::State(format!("ledger: {e:?}")))?;
        let circle_txt = format!(
            "params={}\nindex={}\ncircle_id={}\nwords={}\nlabel=CSPRNG\n",
            hex(&params.encode()),
            index,
            hex(&result.circle_id),
            result.check_words.join(" ")
        );
        std::fs::write(dir.join("circle.txt"), circle_txt)?;
        let pad = Pad::with_circle_id(
            params,
            index,
            result.circle_id,
            StorePad::new(store),
            ledger,
        )?;
        Ok(Device {
            dir: dir.to_path_buf(),
            pad,
            words: result.check_words,
        })
    }

    /// Reopen a device from its state directory. The ledger is replayed from disk.
    pub fn open(dir: &Path) -> Result<Self, DeviceError> {
        let txt = std::fs::read_to_string(dir.join("circle.txt"))?;
        let field = |k: &str| -> Result<String, DeviceError> {
            txt.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .map(String::from)
                .ok_or_else(|| DeviceError::State(format!("missing {k}")))
        };
        let params = CircleParams::decode(&unhex(&field("params")?)?)
            .map_err(|e| DeviceError::State(format!("params: {e:?}")))?;
        let index: u8 = field("index")?
            .parse()
            .map_err(|_| DeviceError::State("index".into()))?;
        let id_bytes = unhex(&field("circle_id")?)?;
        let circle_id: CircleId = id_bytes
            .as_slice()
            .try_into()
            .map_err(|_| DeviceError::State("circle_id".into()))?;
        let store = FilePadStore::open(&dir.join("pad.bin"))?;
        let records = FileRecordStore::open(&dir.join("ledger.bin"))?;
        let ledger = PersistentLedger::open(records, params.slices())
            .map_err(|e| DeviceError::State(format!("ledger: {e:?}")))?;
        if ledger.torn_records() > 0 {
            eprintln!("[dev{index}] warning: torn ledger record ignored");
        }
        let words_txt = field("words")?;
        let mut words = [""; 6];
        for (slot, w) in words.iter_mut().zip(words_txt.split(' ')) {
            *slot = WORDLIST
                .iter()
                .copied()
                .find(|&x| x == w)
                .ok_or_else(|| DeviceError::State(format!("unknown word {w}")))?;
        }
        let pad = Pad::with_circle_id(params, index, circle_id, StorePad::new(store), ledger)?;
        Ok(Device {
            dir: dir.to_path_buf(),
            pad,
            words,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn check_words(&self) -> [&'static str; 6] {
        self.words
    }
    pub fn circle_id(&self) -> CircleId {
        self.pad.circle_id()
    }
    pub fn params(&self) -> &CircleParams {
        self.pad.params()
    }
    pub fn remaining(&self) -> u32 {
        self.pad.remaining()
    }
    pub fn spent_ranges(&self, slice: u8) -> &[(u32, u32)] {
        self.pad.ledger().cache().ranges(slice)
    }
    pub fn is_spent(&self, slice: u8, off: u32, len: u32) -> bool {
        self.pad.ledger().is_spent(slice, off, len)
    }

    pub fn send(
        &mut self,
        msg_type: MessageType,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, DeviceError> {
        Ok(self.pad.send(msg_type, plaintext)?)
    }

    pub fn receive(&mut self, wire: &[u8]) -> Result<(Header, Vec<u8>), DeviceError> {
        Ok(self.pad.receive(wire)?)
    }

    /// Recompute the check words from the pad on disk. Only meaningful before any spend.
    pub fn recompute_words(&self) -> [&'static str; 6] {
        use cahoots_core::pad::PadStorage;
        let mut whole = vec![0u8; self.pad.params().pad_len as usize];
        self.pad.storage().read(0, &mut whole);
        check_words(self.pad.params(), &whole)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, DeviceError> {
    if s.len() % 2 != 0 {
        return Err(DeviceError::State("odd hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| DeviceError::State("hex".into())))
        .collect()
}
