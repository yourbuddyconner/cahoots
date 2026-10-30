//! One simulated badge. Its state directory holds `pad.bin`, `ledger.bin`, `circle.txt`,
//! `root.bin` and `display.png`.
//!
//! `root.bin` is the circle root from the ceremony, 32 bytes in plaintext like the pad, mode
//! 0600. Chat is keyed by it (`docs/chat.md`). A state directory from before it has none, and
//! [`Device::root`] is `None`.
//!
//! `circle.txt` holds one `key=value` per line: `params`, `index`, `circle_id`, `words`,
//! `label`, and from the signed manifest on, `seat_key`, `tier` and `manifest`. The seat key
//! is the seat's signing key in plaintext hex, as the pad is plaintext in `pad.bin`: that is
//! all a tier 0 device (or a tier 1 board) promises. Never print it or log it.

use crate::display::PngDisplay;
use crate::store::{FilePadStore, FileRecordStore};
use cahoots_core::checkwords::{check_words, WORDLIST};
use cahoots_core::circle_id::CircleId;
use cahoots_core::ledger::SpentLedger;
use cahoots_core::manifest::SealedManifest;
use cahoots_core::message::{Header, MessageType};
use cahoots_core::pad::{Pad, PadError};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use cahoots_hal::ledger::PersistentLedger;
use cahoots_hal::ring::{RingCeremony, RingError};
use cahoots_hal::store::StorePad;
use cahoots_hal::{Entropy, Link};
use std::fmt;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

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
    /// The circle root, from `root.bin`. `None` for a state directory written before it.
    root: Option<Zeroizing<[u8; 32]>>,
    /// The seat key's seed, the tier and the encoded sealed manifest. `None` for a circle
    /// file written before seat keys.
    seat: Option<Seat>,
}

/// What `circle.txt` keeps of a seat's place in the signed manifest.
struct Seat {
    /// All zeros when `in_chip`: the key is in the badge's chip, not here.
    key: Zeroizing<[u8; 32]>,
    /// `circle.txt` names a chip slot: the seat key is in the chip.
    in_chip: bool,
    tier: u8,
    manifest: Vec<u8>,
}

/// Show the check words at the end of a ceremony, as the ring driver used to.
pub(crate) fn show_words(display: &mut PngDisplay, words: &[&str; 6]) {
    let mut lines = vec!["check words:"];
    lines.extend_from_slice(words);
    display.show_lines(&lines);
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
            // The host is a T0 device.
            RingCeremony::new(params.clone(), index, entropy, links, 0).map_err(DeviceError::Ring)?;
        let (result, _links) = ring
            .run(
                |s| display.show_lines(&[s]),
                || std::thread::sleep(std::time::Duration::from_millis(1)),
            )
            .map_err(DeviceError::Ring)?;
        show_words(&mut display, &result.check_words);
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
        let seat = Seat {
            key: result
                .seat_key
                .as_ref()
                .ok_or_else(|| DeviceError::State("seat_key".into()))?
                .to_bytes(),
            tier: result.tier,
            manifest: result.manifest.encode(),
            in_chip: false,
        };
        let circle_txt = Zeroizing::new(format!(
            "params={}\nindex={}\ncircle_id={}\nwords={}\nlabel=CSPRNG\nseat_key={}\ntier={}\nmanifest={}\n",
            hex(&params.encode()),
            index,
            hex(&result.circle_id),
            result.check_words.join(" "),
            *Zeroizing::new(hex(&*seat.key)),
            seat.tier,
            hex(&seat.manifest)
        ));
        std::fs::write(dir.join("circle.txt"), circle_txt.as_bytes())?;
        let root = result
            .root
            .as_ref()
            .ok_or_else(|| DeviceError::State("root".into()))?
            .clone();
        write_secret(&dir.join("root.bin"), root.as_ref())?;
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
            root: Some(root),
            seat: Some(seat),
        })
    }

    /// Reopen a device from its state directory. The ledger is replayed from disk.
    pub fn open(dir: &Path) -> Result<Self, DeviceError> {
        let txt = Zeroizing::new(std::fs::read_to_string(dir.join("circle.txt"))?);
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
        // A circle file from before seat keys has none of the three lines; one with only some
        // of them is broken.
        let seat = match field("seat_key") {
            Err(_) if field("tier").is_err() && field("manifest").is_err() => None,
            key => {
                let hex_key = Zeroizing::new(key?);
                let bytes = Zeroizing::new(unhex(&hex_key)?);
                let mut key = Zeroizing::new([0u8; 32]);
                if bytes.len() != 32 {
                    return Err(DeviceError::State("seat_key".into()));
                }
                key.copy_from_slice(&bytes);
                let tier = field("tier")?
                    .parse()
                    .map_err(|_| DeviceError::State("tier".into()))?;
                Some(Seat {
                    key,
                    tier,
                    manifest: unhex(&field("manifest")?)?,
                    in_chip: field("chip_slot").is_ok(),
                })
            }
        };
        let root = match std::fs::read(dir.join("root.bin")) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                let mut root = Zeroizing::new([0u8; 32]);
                if bytes.len() != 32 {
                    return Err(DeviceError::State("root.bin is not 32 bytes".into()));
                }
                root.copy_from_slice(&bytes);
                Some(root)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let pad = Pad::with_circle_id(params, index, circle_id, StorePad::new(store), ledger)?;
        Ok(Device {
            dir: dir.to_path_buf(),
            pad,
            words,
            root,
            seat,
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
    /// The circle root, which keys chat, or `None` for a state directory kept before
    /// `root.bin`.
    pub fn root(&self) -> Option<&[u8; 32]> {
        self.root.as_deref()
    }
    /// This seat's signing key, or `None` for a circle kept before seat keys, a stored seed
    /// that is not a valid key, or a seat key kept in the badge's chip: the file holds zeros in
    /// its place, and only the badge signs with it.
    pub fn seat_key(&self) -> Option<SeatKey> {
        let seat = self.seat.as_ref()?;
        if seat.in_chip {
            return None;
        }
        SeatKey::from_seed(&seat.key).ok()
    }

    /// True when this seat's key is in the badge's chip.
    pub fn seat_in_chip(&self) -> bool {
        self.seat.as_ref().is_some_and(|s| s.in_chip)
    }
    /// The tier this seat declared in the ceremony, or `None` for a circle kept before seat
    /// keys.
    pub fn tier(&self) -> Option<u8> {
        Some(self.seat.as_ref()?.tier)
    }
    /// The sealed manifest every seat signed, or `None` for a circle kept before seat keys
    /// (or a stored manifest that does not decode). Call `verify` on it before trusting it.
    pub fn manifest(&self) -> Option<SealedManifest> {
        SealedManifest::decode(&self.seat.as_ref()?.manifest).ok()
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

/// Write a secret file readable by its owner only.
fn write_secret(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
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
