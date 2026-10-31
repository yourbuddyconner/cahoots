//! Adapters over a `cahoots-provider` `FileProvider` directory.
//!
//! `FileProvider` owns the on-disk format but does not hand out its `Pad`, so the
//! client reads the same files through its own `PadStorage` and `SpentLedger`
//! implementations. The format is documented in `crates/provider/src/file.rs`:
//! `key`, `<circle_id hex>/{params, slice, pad.enc, ledger}`. Erase order matches the
//! provider: ledger line appended and fsynced first, then `pad.enc` rewritten
//! atomically with the range zeroed, then decrypted back and verified.
//!
//! The client adds files of its own next to them: `root` (the circle root from the
//! ceremony, encrypted under the same key, mode 0600), `chat.state` (the exported
//! `ChatSession` state, secrets included, encrypted under the same key) and `chat.seq`
//! (the last relay sequence number seen).

use crate::{ChatSession, CHAT_VERSION};
use cahoots_core::ledger::{LedgerError, MemLedger, SpentLedger};
use cahoots_core::pad::{Pad, PadStorage, StorageError};
use cahoots_core::params::CircleParams;
use cahoots_core::CircleId;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::string::{String, ToString};
use std::vec::Vec;
use zeroize::Zeroizing;

const NONCE_LEN: usize = 12;

#[derive(Debug)]
pub enum StoreError {
    Io(String),
    Format(String),
    Crypto,
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e.to_string())
    }
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StoreError::Io(s) => write!(f, "io: {s}"),
            StoreError::Format(s) => write!(f, "format: {s}"),
            StoreError::Crypto => write!(f, "pad.enc authentication failed"),
        }
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
}

pub fn seal_blob(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, StoreError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|e| StoreError::Io(e.to_string()))?;
    let ct = ChaCha20Poly1305::new(key.into())
        .encrypt(Nonce::from_slice(&nonce), data)
        .map_err(|_| StoreError::Crypto)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open_blob(key: &[u8; 32], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    if blob.len() < NONCE_LEN {
        return Err(StoreError::Format("blob too short".into()));
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    ChaCha20Poly1305::new(key.into())
        .decrypt(Nonce::from_slice(nonce), ct)
        .map(Zeroizing::new)
        .map_err(|_| StoreError::Crypto)
}

/// Decrypted pad in memory; every erase is written through and verified.
pub struct CircleStorage {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    bytes: Zeroizing<Vec<u8>>,
}

impl PadStorage for CircleStorage {
    fn len(&self) -> u32 {
        self.bytes.len() as u32
    }
    fn read(&self, off: u32, buf: &mut [u8]) {
        let off = off as usize;
        buf.copy_from_slice(&self.bytes[off..off + buf.len()]);
    }
    fn erase(&mut self, off: u32, len: u32) -> Result<(), StorageError> {
        let (s, e) = (off as usize, (off + len) as usize);
        if e > self.bytes.len() {
            return Err(StorageError::OutOfRange);
        }
        self.bytes[s..e].fill(0);
        let blob = seal_blob(&self.key, &self.bytes).map_err(|_| StorageError::VerifyFailed)?;
        write_atomic(&self.path, &blob).map_err(|_| StorageError::VerifyFailed)?;
        let back = fs::read(&self.path)
            .ok()
            .and_then(|b| open_blob(&self.key, &b).ok())
            .ok_or(StorageError::VerifyFailed)?;
        if back[s..e].iter().any(|&b| b != 0) {
            return Err(StorageError::VerifyFailed);
        }
        Ok(())
    }
}

pub struct CircleLedger {
    file: File,
    mem: MemLedger,
}

impl CircleLedger {
    fn open(path: &Path, slices: u8) -> Result<Self, StoreError> {
        let mut mem = MemLedger::new(slices);
        if let Ok(text) = fs::read_to_string(path) {
            for line in text.lines() {
                let mut it = line.split_whitespace().map(|x| x.parse::<u32>());
                match (it.next(), it.next(), it.next()) {
                    (Some(Ok(s)), Some(Ok(o)), Some(Ok(l))) => mem
                        .mark_spent(s as u8, o, l)
                        .map_err(|e| StoreError::Format(std::format!("ledger: {e:?}")))?,
                    _ => return Err(StoreError::Format(std::format!("bad ledger line {line:?}"))),
                }
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(CircleLedger { file, mem })
    }
}

impl SpentLedger for CircleLedger {
    fn mark_spent(&mut self, slice: u8, off: u32, len: u32) -> Result<(), LedgerError> {
        self.file
            .write_all(std::format!("{slice} {off} {len}\n").as_bytes())
            .and_then(|_| self.file.sync_all())
            .map_err(|_| LedgerError::Storage)?;
        self.mem.mark_spent(slice, off, len)
    }
    fn is_spent(&self, slice: u8, off: u32, len: u32) -> bool {
        self.mem.is_spent(slice, off, len)
    }
    fn next_offset(&self, slice: u8) -> u32 {
        self.mem.next_offset(slice)
    }
}

pub type CirclePad = Pad<CircleStorage, CircleLedger>;
pub type CircleChat = ChatSession;

/// One member's circle, opened from a provider directory holding exactly one circle.
pub struct OpenedCircle {
    pub dir: PathBuf,
    pub key: Zeroizing<[u8; 32]>,
    pub circle_id: CircleId,
    pub params: CircleParams,
    pub my_slice: u8,
    pub label: String,
    pub pad: CirclePad,
    /// The circle root, which keys the chat. `None` for a circle written before the client
    /// kept it.
    pub root: Option<Zeroizing<[u8; 32]>>,
}

fn read_key(root: &Path) -> Result<Zeroizing<[u8; 32]>, StoreError> {
    let bytes = fs::read(root.join("key"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return Err(StoreError::Format("key file is not 32 bytes".into()));
    }
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Find the single `<circle_id hex>` directory under a provider root.
fn find_circle_dir(root: &Path) -> Result<(CircleId, PathBuf), StoreError> {
    let mut found = None;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(raw) = hex::decode(&name) else {
            continue;
        };
        let Ok(id) = <[u8; 16]>::try_from(raw) else {
            continue;
        };
        if found.is_some() {
            return Err(StoreError::Format(
                "more than one circle in directory".into(),
            ));
        }
        found = Some((id, entry.path()));
    }
    found.ok_or_else(|| StoreError::Format("no circle directory found".into()))
}

pub fn open_circle(root: &Path) -> Result<OpenedCircle, StoreError> {
    let key = read_key(root)?;
    let (circle_id, dir) = find_circle_dir(root)?;
    let params = CircleParams::decode(&fs::read(dir.join("params"))?)
        .map_err(|e| StoreError::Format(std::format!("params: {e:?}")))?;
    let my_slice = *fs::read(dir.join("slice"))?
        .first()
        .ok_or_else(|| StoreError::Format("slice file empty".into()))?;
    let label = fs::read_to_string(dir.join("label"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "CSPRNG".into());
    let pad_path = dir.join("pad.enc");
    let bytes = open_blob(&key, &fs::read(&pad_path)?)?;
    let storage = CircleStorage {
        path: pad_path,
        key: key.clone(),
        bytes,
    };
    let ledger = CircleLedger::open(&dir.join("ledger"), params.slices())?;
    let pad = Pad::with_circle_id(params.clone(), my_slice, circle_id, storage, ledger)
        .map_err(|e| StoreError::Format(std::format!("pad: {e:?}")))?;
    let root = load_root(&dir, &key)?;
    Ok(OpenedCircle {
        dir,
        key,
        circle_id,
        params,
        my_slice,
        label,
        pad,
        root,
    })
}

/// Write the circle root next to the pad, encrypted under the provider key, mode 0600.
/// The temporary file is created with mode 0600 before any byte is written.
pub fn write_root(circle_dir: &Path, key: &[u8; 32], root: &[u8; 32]) -> Result<(), StoreError> {
    let path = circle_dir.join("root");
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(&seal_blob(key, root)?)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, &path)?;
    File::open(circle_dir)?.sync_all()?;
    Ok(())
}

/// The circle root, or `None` if this circle has no `root` file.
pub fn load_root(circle_dir: &Path, key: &[u8; 32]) -> Result<Option<Zeroizing<[u8; 32]>>, StoreError> {
    let path = circle_dir.join("root");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = open_blob(key, &fs::read(path)?)?;
    let mut root = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return Err(StoreError::Format("root file is not 32 bytes".into()));
    }
    root.copy_from_slice(&bytes);
    Ok(Some(root))
}

/// Write the label file the provider format does not have. `TRNG` or `CSPRNG`.
pub fn write_label(circle_dir: &Path, label: &str) -> Result<(), StoreError> {
    write_atomic(
        &circle_dir.join("label"),
        std::format!("{label}\n").as_bytes(),
    )?;
    Ok(())
}

pub fn save_chat_state(dir: &Path, key: &[u8; 32], state: &[u8]) -> Result<(), StoreError> {
    let mut framed = Zeroizing::new(std::vec![CHAT_VERSION]);
    framed.extend_from_slice(state);
    write_atomic(&dir.join("chat.state"), &seal_blob(key, &framed)?)?;
    Ok(())
}

pub fn load_chat_state(
    dir: &Path,
    key: &[u8; 32],
) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
    let path = dir.join("chat.state");
    if !path.exists() {
        return Ok(None);
    }
    let framed = open_blob(key, &fs::read(path)?)?;
    if framed.first() != Some(&CHAT_VERSION) {
        return Err(StoreError::Format("chat.state version".into()));
    }
    Ok(Some(Zeroizing::new(framed[1..].to_vec())))
}

pub fn save_seq(dir: &Path, seq: Option<u64>) -> Result<(), StoreError> {
    let text = match seq {
        Some(s) => std::format!("{s}\n"),
        None => String::from("none\n"),
    };
    write_atomic(&dir.join("chat.seq"), text.as_bytes())?;
    Ok(())
}

pub fn load_seq(dir: &Path) -> Result<Option<u64>, StoreError> {
    match fs::read_to_string(dir.join("chat.seq")) {
        Ok(s) if s.trim() == "none" => Ok(None),
        Ok(s) => s
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| StoreError::Format("chat.seq".into())),
        Err(_) => Ok(None),
    }
}
