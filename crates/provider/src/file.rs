//! Development backend: one directory per circle, pad encrypted at rest.
//!
//! Layout under the root:
//! ```text
//! key                      32-byte ChaCha20-Poly1305 key, mode 0600
//! <circle_id hex>/params   CircleParams encoding
//! <circle_id hex>/slice    one byte: our slice index
//! <circle_id hex>/pad.enc  nonce (12) || AEAD(key, nonce, pad)
//! <circle_id hex>/ledger   append-only lines: "<slice> <offset> <len>\n"
//! ```
//!
//! Spend ordering: append the ledger line and fsync, then rewrite `pad.enc` with the
//! range zeroed (write to a temp file, fsync, rename), then decrypt it back and verify.
//! This is a development backend. The key sits next to the data, so it protects against
//! casual disk reads and backups, not against an attacker with the same account.

use crate::circle::Circle;
use crate::mem::to_psk;
use crate::{CircleId, PadError, PadProvider, PskHandle, PSK_LEN};
use cahoots_core::circle_id::circle_id;
use cahoots_core::ledger::{LedgerError, MemLedger, SpentLedger};
use cahoots_core::pad::{PadStorage, StorageError};
use cahoots_core::params::CircleParams;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const NONCE_LEN: usize = 12;

fn io(e: impl std::fmt::Display) -> PadError {
    PadError::Storage(e.to_string())
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

fn seal_pad(key: &[u8; 32], pad: &[u8]) -> Result<Vec<u8>, PadError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(io)?;
    let ct = ChaCha20Poly1305::new(key.into())
        .encrypt(Nonce::from_slice(&nonce), pad)
        .map_err(|_| PadError::Storage("encrypt".into()))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

fn open_pad(key: &[u8; 32], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, PadError> {
    if blob.len() < NONCE_LEN {
        return Err(PadError::Storage("pad.enc too short".into()));
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    ChaCha20Poly1305::new(key.into())
        .decrypt(Nonce::from_slice(nonce), ct)
        .map(Zeroizing::new)
        .map_err(|_| PadError::Storage("pad.enc authentication failed".into()))
}

/// Decrypted pad held in memory; every erase is written through to disk and verified.
pub struct FileStorage {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    bytes: Zeroizing<Vec<u8>>,
}

impl PadStorage for FileStorage {
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
        let blob = seal_pad(&self.key, &self.bytes).map_err(|_| StorageError::VerifyFailed)?;
        write_atomic(&self.path, &blob).map_err(|_| StorageError::VerifyFailed)?;
        let back = fs::read(&self.path)
            .ok()
            .and_then(|b| open_pad(&self.key, &b).ok())
            .ok_or(StorageError::VerifyFailed)?;
        if back[s..e].iter().any(|&b| b != 0) {
            return Err(StorageError::VerifyFailed);
        }
        Ok(())
    }
}

/// Append-only ledger file with an in-memory index.
pub struct FileLedger {
    file: File,
    mem: MemLedger,
}

impl FileLedger {
    fn open(path: &Path, slices: u8) -> Result<Self, PadError> {
        let mut mem = MemLedger::new(slices);
        if let Ok(text) = fs::read_to_string(path) {
            for line in text.lines() {
                let mut it = line.split_whitespace().map(|x| x.parse::<u32>());
                match (it.next(), it.next(), it.next()) {
                    (Some(Ok(s)), Some(Ok(o)), Some(Ok(l))) => mem
                        .mark_spent(s as u8, o, l)
                        .map_err(|e| io(format!("{e:?}")))?,
                    _ => return Err(PadError::Storage(format!("bad ledger line: {line:?}"))),
                }
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(io)?;
        Ok(FileLedger { file, mem })
    }
}

impl SpentLedger for FileLedger {
    fn mark_spent(&mut self, slice: u8, off: u32, len: u32) -> Result<(), LedgerError> {
        self.file
            .write_all(format!("{slice} {off} {len}\n").as_bytes())
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

pub struct FileProvider {
    root: PathBuf,
    key: Zeroizing<[u8; 32]>,
    circles: HashMap<CircleId, Circle<FileStorage, FileLedger>>,
}

impl FileProvider {
    /// Open or create a provider directory. Loads every circle found under it.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, PadError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(io)?;
        let key = Self::load_or_create_key(&root)?;
        let mut p = FileProvider {
            root,
            key,
            circles: HashMap::new(),
        };
        for entry in fs::read_dir(&p.root).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name();
            let Ok(id) = hex::decode(name.to_string_lossy().as_bytes()) else {
                continue;
            };
            let Ok(id) = <[u8; 16]>::try_from(id) else {
                continue;
            };
            let c = p.load_circle(&id)?;
            p.circles.insert(id, c);
        }
        Ok(p)
    }

    fn load_or_create_key(root: &Path) -> Result<Zeroizing<[u8; 32]>, PadError> {
        let path = root.join("key");
        let mut key = Zeroizing::new([0u8; 32]);
        if let Ok(bytes) = fs::read(&path) {
            if bytes.len() != 32 {
                return Err(PadError::Storage("key file is not 32 bytes".into()));
            }
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        getrandom::getrandom(&mut *key).map_err(io)?;
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path).map_err(io)?;
        f.write_all(&*key).map_err(io)?;
        f.sync_all().map_err(io)?;
        Ok(key)
    }

    fn circle_dir(&self, id: &CircleId) -> PathBuf {
        self.root.join(hex::encode(id))
    }

    fn load_circle(&self, id: &CircleId) -> Result<Circle<FileStorage, FileLedger>, PadError> {
        let dir = self.circle_dir(id);
        let params = CircleParams::decode(&fs::read(dir.join("params")).map_err(io)?)
            .map_err(|e| PadError::Storage(format!("params: {e:?}")))?;
        let slice = fs::read(dir.join("slice")).map_err(io)?;
        let my_slice = *slice.first().ok_or(PadError::Config)?;
        let pad_path = dir.join("pad.enc");
        let bytes = open_pad(&self.key, &fs::read(&pad_path).map_err(io)?)?;
        let storage = FileStorage {
            path: pad_path,
            key: self.key.clone(),
            bytes,
        };
        let ledger = FileLedger::open(&dir.join("ledger"), params.slices())?;
        Circle::new(params, my_slice, *id, storage, ledger)
    }

    /// Store a freshly combined pad. Returns its circle ID.
    pub fn add_circle(
        &mut self,
        params: CircleParams,
        my_slice: u8,
        pad: Vec<u8>,
    ) -> Result<CircleId, PadError> {
        let pad = Zeroizing::new(pad);
        if pad.len() != params.pad_len as usize || my_slice >= params.slices() {
            return Err(PadError::Config);
        }
        let id = circle_id(&pad);
        let dir = self.circle_dir(&id);
        if dir.exists() {
            return Err(PadError::Storage("circle already stored".into()));
        }
        fs::create_dir_all(&dir).map_err(io)?;
        write_atomic(&dir.join("params"), &params.encode()).map_err(io)?;
        write_atomic(&dir.join("slice"), &[my_slice]).map_err(io)?;
        write_atomic(&dir.join("pad.enc"), &seal_pad(&self.key, &pad)?).map_err(io)?;
        let c = self.load_circle(&id)?;
        self.circles.insert(id, c);
        Ok(id)
    }

    pub fn circles(&self) -> Vec<CircleId> {
        self.circles.keys().copied().collect()
    }

    pub fn params(&self, id: &CircleId) -> Option<&CircleParams> {
        self.circles.get(id).map(|c| &c.params)
    }

    pub fn my_slice(&self, id: &CircleId) -> Option<u8> {
        self.circles.get(id).map(|c| c.my_slice)
    }

    /// Decrypt `pad.enc` straight from disk. For tests and audits only.
    pub fn read_pad_for_test(&self, id: &CircleId) -> Result<Zeroizing<Vec<u8>>, PadError> {
        open_pad(
            &self.key,
            &fs::read(self.circle_dir(id).join("pad.enc")).map_err(io)?,
        )
    }

    fn get(&mut self, id: &CircleId) -> Result<&mut Circle<FileStorage, FileLedger>, PadError> {
        self.circles.get_mut(id).ok_or(PadError::UnknownCircle)
    }
}

impl PadProvider for FileProvider {
    fn psk(
        &mut self,
        circle: &CircleId,
        _context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError> {
        let (h, b) = self.get(circle)?.spend_own(PSK_LEN)?;
        Ok((h, to_psk(b)))
    }

    fn psk_for(
        &mut self,
        handle: &PskHandle,
        _context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError> {
        Ok(to_psk(
            self.get(&handle.circle_id)?.spend_peer(handle, PSK_LEN)?,
        ))
    }

    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        self.get(circle)?.spend_own(len)
    }

    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        self.get(&handle.circle_id)?.spend_peer(handle, len)
    }

    fn remaining(&self, circle: &CircleId) -> u64 {
        self.circles
            .get(circle)
            .map(|c| u64::from(c.remaining()))
            .unwrap_or(0)
    }

    fn low_pad(&self, circle: &CircleId) -> bool {
        self.circles
            .get(circle)
            .map(|c| c.low_pad())
            .unwrap_or(true)
    }
}
