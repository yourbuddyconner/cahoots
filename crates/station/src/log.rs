//! The append-only, hash-chained log, stored in one SQLite database. Every entry commits to
//! the one before it, so the head is a 32-byte summary of the whole history. On open, every
//! entry is replayed in `seq` order and the chain re-verified; a broken chain refuses to open
//! rather than serve a forged history.

use crate::rules::Submission;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_core::circle_id::CircleId;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// The entry's stream: a circle id, or a channel id for a version 3 message.
    pub circle_id: CircleId,
    pub message: Vec<u8>,
    pub hash: Hash,
}

/// On-the-wire form. Bytes are base64url, hashes are hex.
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

/// One stream's count and last `seq`, before it is told apart as a circle or a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSummary {
    pub stream: [u8; 16],
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
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("malformed entry at seq {seq}: {reason}")]
    Malformed { seq: u64, reason: String },
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

/// One `entries` row as stored: `(seq, stream, received_at, message, hash)`.
type Row = (i64, Vec<u8>, i64, Vec<u8>, Vec<u8>);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    seq INTEGER PRIMARY KEY,
    stream BLOB NOT NULL,
    received_at INTEGER NOT NULL,
    message BLOB NOT NULL,
    hash BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS entries_stream_seq ON entries (stream, seq);
CREATE TABLE IF NOT EXISTS spent (
    stream BLOB NOT NULL,
    slice INTEGER NOT NULL,
    start INTEGER NOT NULL,
    \"end\" INTEGER NOT NULL,
    seq INTEGER NOT NULL
);
";

pub struct Log {
    conn: Connection,
    path: PathBuf,
    /// Every entry, replayed and verified on open, kept in step with every append.
    entries: Vec<Entry>,
    /// Rebuilt on open from the verified entries' bytes; the `spent` table is written
    /// alongside for readers of the database, but the overlap check uses this map.
    spent: SpentRanges,
}

impl Log {
    /// Open or create the database, replay every entry in `seq` order, and verify the chain.
    pub fn open(path: &Path) -> Result<Self, LogError> {
        let conn = Connection::open(path)?;
        // `journal_mode` answers with the mode now in force, so it is read as a row.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        // FULL: a commit is on disk before `append` returns, so a 201 means the entry is durable.
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA)?;
        #[cfg(feature = "objects")]
        crate::objects::store::schema(&conn)?;
        let mut log = Log {
            conn,
            path: path.to_path_buf(),
            entries: Vec::new(),
            spent: HashMap::new(),
        };
        log.replay()?;
        Ok(log)
    }

    fn replay(&mut self) -> Result<(), LogError> {
        let rows: Vec<Row> = {
            let mut stmt = self.conn.prepare(
                "SELECT seq, stream, received_at, message, hash FROM entries ORDER BY seq",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        for (seq, stream, received_at, message, hash) in rows {
            let expect_seq = self.entries.len() as u64;
            if seq != expect_seq as i64 {
                return Err(LogError::BrokenChain { seq: expect_seq });
            }
            let malformed = |reason: &str| LogError::Malformed {
                seq: expect_seq,
                reason: reason.to_string(),
            };
            let circle_id: CircleId = stream.try_into().map_err(|_| malformed("stream"))?;
            let hash: Hash = hash.try_into().map_err(|_| malformed("hash"))?;
            let received_at = u64::try_from(received_at).map_err(|_| malformed("received_at"))?;
            let prev = self.head().hash;
            if entry_hash(&prev, expect_seq, received_at, &message) != hash {
                return Err(LogError::BrokenChain { seq: expect_seq });
            }
            // Re-derive the spent ranges from the stored bytes; the entry is trusted only
            // after its hash verified.
            if let Ok(sub) = Submission::parse(&circle_id, &message) {
                self.note_spent(circle_id, sub.pad_range(), expect_seq);
            }
            self.entries.push(Entry {
                seq: expect_seq,
                received_at,
                circle_id,
                message,
                hash,
            });
        }
        Ok(())
    }

    /// The database connection, for stores that share this file. Do not write `entries` or
    /// `spent` through it: the in-memory entries and spent ranges are the source of truth
    /// for reads and the reuse check, and would not see such rows until the next open.
    pub fn conn(&mut self) -> &mut Connection {
        &mut self.conn
    }

    fn note_spent(&mut self, circle_id: CircleId, range: Option<(u8, u32, u32)>, seq: u64) {
        if let Some((slice, start, end)) = range {
            self.spent
                .entry((circle_id, slice))
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
        let circle_id = sub.stream();
        let range = sub.pad_range();
        let message = sub.into_bytes();
        let hash = entry_hash(&prev, seq, received_at, &message);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO entries (seq, stream, received_at, message, hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![seq as i64, &circle_id[..], received_at as i64, &message, &hash[..]],
        )?;
        if let Some((slice, start, end)) = range {
            tx.execute(
                "INSERT INTO spent (stream, slice, start, \"end\", seq) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![&circle_id[..], slice, start, end, seq as i64],
            )?;
        }
        tx.commit()?;
        self.note_spent(circle_id, range, seq);
        let entry = Entry {
            seq,
            received_at,
            circle_id,
            message,
            hash,
        };
        self.entries.push(entry.clone());
        Ok(entry)
    }

    /// Size of the database in bytes: the main file plus its write-ahead log.
    pub fn bytes(&self) -> u64 {
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let mut wal = self.path.clone().into_os_string();
        wal.push("-wal");
        size(&self.path) + size(Path::new(&wal))
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

    /// The last entry of `stream`, a circle or a channel; the genesis hash when it has none.
    pub fn stream_head(&self, stream: &[u8; 16]) -> Head {
        match self.entries.iter().rev().find(|e| &e.circle_id == stream) {
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

    /// Entries of `stream` with `seq > after` (all of them when `after` is `None`).
    pub fn entries_after_stream(&self, stream: &[u8; 16], after: Option<u64>) -> Vec<&Entry> {
        let from = after.map(|a| a as usize + 1).unwrap_or(0);
        self.entries
            .iter()
            .skip(from)
            .filter(|e| &e.circle_id == stream)
            .collect()
    }

    pub fn circle_head(&self, circle: &CircleId) -> Head {
        self.stream_head(circle)
    }

    /// Entries for `circle` with `seq > after` (all of them when `after` is `None`).
    pub fn entries_after(&self, circle: &CircleId, after: Option<u64>) -> Vec<&Entry> {
        self.entries_after_stream(circle, after)
    }

    pub fn all_after(&self, after: Option<u64>, limit: usize) -> Vec<&Entry> {
        let from = after.map(|a| a as usize + 1).unwrap_or(0);
        self.entries.iter().skip(from).take(limit).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every stream with its count and last `seq`, the most recently written first.
    pub fn streams(&self) -> Vec<StreamSummary> {
        let mut map: HashMap<[u8; 16], StreamSummary> = HashMap::new();
        for e in &self.entries {
            let s = map.entry(e.circle_id).or_insert(StreamSummary {
                stream: e.circle_id,
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

    /// Every stream as a circle. With `channels`, the server leaves out the channel streams.
    pub fn circles(&self) -> Vec<CircleSummary> {
        self.streams()
            .into_iter()
            .map(|s| CircleSummary {
                circle_id: s.stream,
                count: s.count,
                last_seq: s.last_seq,
            })
            .collect()
    }
}
