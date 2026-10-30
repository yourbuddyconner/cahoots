//! The append-only, hash-chained log. Every entry commits to the one before it, so the
//! head is a 32-byte summary of the whole history. On open, the file is replayed and the
//! chain re-verified; a broken chain refuses to open rather than serve a forged history.

use crate::rules::Submission;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_core::circle_id::CircleId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub type Hash = [u8; 32];
pub const GENESIS: Hash = [0u8; 32];

/// `SHA-256(prev || seq (u64 LE) || received_at (u64 LE) || message)`.
pub fn entry_hash(prev: &Hash, seq: u64, received_at: u64, message: &[u8]) -> Hash {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(seq.to_le_bytes());
    h.update(received_at.to_le_bytes());
    h.update(message);
    h.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub seq: u64,
    pub received_at: u64,
    pub circle_id: CircleId,
    pub message: Vec<u8>,
    pub hash: Hash,
}

impl Entry {
    /// Whether the public listings show it. A ceremony channel's messages (version 3) are
    /// served only to whoever asks for the channel id, which comes from the host's code.
    pub fn listed(&self) -> bool {
        self.message.first() != Some(&crate::rules::CEREMONY_VERSION)
    }
}

/// On-disk and on-the-wire form. Bytes are base64url, hashes are hex.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntryRecord {
    pub seq: u64,
    pub received_at: u64,
    pub circle_id: String,
    pub message: String,
    pub hash: String,
}

impl From<&Entry> for EntryRecord {
    fn from(e: &Entry) -> Self {
        EntryRecord {
            seq: e.seq,
            received_at: e.received_at,
            circle_id: hex::encode(e.circle_id),
            message: URL_SAFE_NO_PAD.encode(&e.message),
            hash: hex::encode(e.hash),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// `None` for an empty log.
    pub seq: Option<u64>,
    pub hash: Hash,
}

#[derive(Clone, Debug, Serialize)]
pub struct CircleSummary {
    #[serde(serialize_with = "hex_cid")]
    pub circle_id: CircleId,
    pub count: u64,
    pub last_seq: u64,
}

fn hex_cid<S: serde::Serializer>(c: &CircleId, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(c))
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed record at line {line}: {reason}")]
    Malformed { line: usize, reason: String },
    #[error("hash chain broken at seq {seq}")]
    BrokenChain { seq: u64 },
    #[error("pad range reuse: overlaps entry {prior_seq} (slice {slice}, bytes {start}..{end})")]
    Reuse {
        prior_seq: u64,
        slice: u8,
        start: u32,
        end: u32,
    },
}

/// Accepted pure-pad ranges per `(circle, slice)`: `(start, end, seq)`.
type SpentRanges = HashMap<(CircleId, u8), Vec<(u32, u32, u64)>>;

pub struct Log {
    file: File,
    entries: Vec<Entry>,
    spent: SpentRanges,
    /// Bytes on disk, kept in step with every append.
    bytes: u64,
}

impl Log {
    /// Open or create the log file, replay it, and verify the chain.
    pub fn open(path: &Path) -> Result<Self, LogError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
        let bytes = file.metadata()?.len();
        let mut log = Log {
            file,
            entries: Vec::new(),
            spent: HashMap::new(),
            bytes,
        };
        let reader = BufReader::new(File::open(path)?);
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let rec: EntryRecord =
                serde_json::from_str(&line).map_err(|e| LogError::Malformed {
                    line: i + 1,
                    reason: e.to_string(),
                })?;
            let malformed = |reason: &str| LogError::Malformed {
                line: i + 1,
                reason: reason.to_string(),
            };
            let circle_id: CircleId = hex::decode(&rec.circle_id)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| malformed("circle_id"))?;
            let message = URL_SAFE_NO_PAD
                .decode(&rec.message)
                .map_err(|_| malformed("message"))?;
            let hash: Hash = hex::decode(&rec.hash)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| malformed("hash"))?;
            let expect_seq = log.entries.len() as u64;
            if rec.seq != expect_seq {
                return Err(LogError::BrokenChain { seq: expect_seq });
            }
            let prev = log.head().hash;
            if entry_hash(&prev, rec.seq, rec.received_at, &message) != hash {
                return Err(LogError::BrokenChain { seq: rec.seq });
            }
            // Re-derive the spent ranges from the stored bytes; the record is trusted
            // only after its hash verified.
            if let Ok(sub) = Submission::parse(&circle_id, &message) {
                log.note_spent(&sub, rec.seq);
            }
            log.entries.push(Entry {
                seq: rec.seq,
                received_at: rec.received_at,
                circle_id,
                message,
                hash,
            });
        }
        Ok(log)
    }

    fn note_spent(&mut self, sub: &Submission, seq: u64) {
        if let Some((slice, start, end)) = sub.pad_range() {
            self.spent
                .entry((sub.circle_id(), slice))
                .or_default()
                .push((start, end, seq));
        }
    }

    fn find_overlap(&self, sub: &Submission) -> Option<LogError> {
        let (slice, start, end) = sub.pad_range()?;
        let ranges = self.spent.get(&(sub.circle_id(), slice))?;
        ranges
            .iter()
            .find(|&&(s, e, _)| start < e && s < end)
            .map(|&(s, e, seq)| LogError::Reuse {
                prior_seq: seq,
                slice,
                start: s,
                end: e,
            })
    }

    /// Append a rule-checked submission. Refuses pure-pad range reuse. Durable on return.
    pub fn append(&mut self, sub: Submission, received_at: u64) -> Result<Entry, LogError> {
        if let Some(err) = self.find_overlap(&sub) {
            return Err(err);
        }
        let seq = self.entries.len() as u64;
        let prev = self.head().hash;
        let circle_id = sub.circle_id();
        self.note_spent(&sub, seq);
        let message = sub.into_bytes();
        let hash = entry_hash(&prev, seq, received_at, &message);
        let entry = Entry {
            seq,
            received_at,
            circle_id,
            message,
            hash,
        };
        let mut line = serde_json::to_string(&EntryRecord::from(&entry)).expect("serializable");
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        self.bytes += line.len() as u64;
        self.entries.push(entry.clone());
        Ok(entry)
    }

    /// Size of the log file in bytes.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn head(&self) -> Head {
        match self.entries.last() {
            Some(e) => Head {
                seq: Some(e.seq),
                hash: e.hash,
            },
            None => Head {
                seq: None,
                hash: GENESIS,
            },
        }
    }

    pub fn circle_head(&self, circle: &CircleId) -> Head {
        match self.entries.iter().rev().find(|e| &e.circle_id == circle) {
            Some(e) => Head {
                seq: Some(e.seq),
                hash: e.hash,
            },
            None => Head {
                seq: None,
                hash: GENESIS,
            },
        }
    }

    /// Entries for `circle` with `seq > after` (all of them when `after` is `None`).
    pub fn entries_after(&self, circle: &CircleId, after: Option<u64>) -> Vec<&Entry> {
        let from = after.map(|a| a as usize + 1).unwrap_or(0);
        self.entries
            .iter()
            .skip(from)
            .filter(|e| &e.circle_id == circle)
            .collect()
    }

    /// Entries for the public feed: every circle's, but no ceremony channel's.
    pub fn all_after(&self, after: Option<u64>, limit: usize) -> Vec<&Entry> {
        let from = after.map(|a| a as usize + 1).unwrap_or(0);
        self.entries
            .iter()
            .skip(from)
            .filter(|e| e.listed())
            .take(limit)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn circles(&self) -> Vec<CircleSummary> {
        let mut map: HashMap<CircleId, CircleSummary> = HashMap::new();
        for e in self.entries.iter().filter(|e| e.listed()) {
            let s = map.entry(e.circle_id).or_insert(CircleSummary {
                circle_id: e.circle_id,
                count: 0,
                last_seq: e.seq,
            });
            s.count += 1;
            s.last_seq = e.seq;
        }
        let mut v: Vec<_> = map.into_values().collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.last_seq));
        v
    }
}
