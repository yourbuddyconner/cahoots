//! The object tables, in the log's database. Every function takes the connection that
//! [`crate::log::Log::conn`] hands out; none of them touches `entries` or `spent`.

use rusqlite::{params, Connection, OptionalExtension};

pub type Result<T> = rusqlite::Result<T>;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS manifests (circle BLOB PRIMARY KEY, body BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS channels (
    channel BLOB PRIMARY KEY,
    circle BLOB NOT NULL,
    body BLOB NOT NULL,
    created_log INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS policies (
    id BLOB PRIMARY KEY,
    channel BLOB NOT NULL,
    body BLOB NOT NULL,
    log INTEGER NOT NULL,
    approvals BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS policies_channel_log ON policies (channel, log);
CREATE TABLE IF NOT EXISTS grants (
    cti BLOB PRIMARY KEY,
    channel BLOB NOT NULL,
    kid BLOB NOT NULL,
    chain BLOB NOT NULL,
    not_after INTEGER
);
CREATE INDEX IF NOT EXISTS grants_channel_kid ON grants (channel, kid);
CREATE TABLE IF NOT EXISTS revocations (
    channel BLOB NOT NULL,
    grant_id BLOB NOT NULL,
    seq INTEGER,
    body BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS revocations_channel ON revocations (channel, grant_id);
CREATE TABLE IF NOT EXISTS deposits (
    id BLOB PRIMARY KEY,
    channel BLOB NOT NULL,
    epoch BLOB NOT NULL,
    to_ BLOB NOT NULL,
    body BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS deposits_channel_epoch_to ON deposits (channel, epoch, to_);
CREATE TABLE IF NOT EXISTS requests (
    id BLOB PRIMARY KEY,
    channel BLOB NOT NULL,
    body BLOB NOT NULL,
    seq INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS requests_channel_seq ON requests (channel, seq);
CREATE TABLE IF NOT EXISTS gates (
    channel BLOB NOT NULL,
    token_hash BLOB NOT NULL,
    PRIMARY KEY (channel, token_hash)
);
";

/// Create the object tables when they are missing.
pub fn schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)
}

/// What a write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Put {
    /// A new row, or a row replaced by a newer one.
    Stored,
    /// The same bytes were already stored.
    Same,
    /// Another object holds the key: a different manifest or grant under the same id.
    Exists,
    /// An object at the same or a later log position is already stored.
    Stale,
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

// Manifests.

pub fn manifest(conn: &Connection, circle: &[u8; 16]) -> Result<Option<Vec<u8>>> {
    conn.query_row(
        "SELECT body FROM manifests WHERE circle = ?1",
        [&circle[..]],
        |r| r.get(0),
    )
    .optional()
}

/// Store a verified sealed manifest. A circle keeps its first manifest.
pub fn put_manifest(conn: &Connection, circle: &[u8; 16], body: &[u8]) -> Result<Put> {
    match manifest(conn, circle)? {
        Some(held) if held == body => Ok(Put::Same),
        Some(_) => Ok(Put::Exists),
        None => {
            conn.execute(
                "INSERT INTO manifests (circle, body) VALUES (?1, ?2)",
                params![&circle[..], body],
            )?;
            Ok(Put::Stored)
        }
    }
}

// Channel objects.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRow {
    pub circle: [u8; 16],
    /// The signed channel object.
    pub body: Vec<u8>,
    pub created_log: u64,
}

pub fn channel(conn: &Connection, channel: &[u8; 16]) -> Result<Option<ChannelRow>> {
    conn.query_row(
        "SELECT circle, body, created_log FROM channels WHERE channel = ?1",
        [&channel[..]],
        |r| {
            let circle: Vec<u8> = r.get(0)?;
            let created_log: i64 = r.get(2)?;
            Ok(ChannelRow {
                circle: circle.try_into().unwrap_or_default(),
                body: r.get(1)?,
                created_log: created_log as u64,
            })
        },
    )
    .optional()
}

/// Store a verified channel object. A later one replaces the held one only when its
/// `created_log` is greater.
pub fn put_channel(conn: &Connection, channel_id: &[u8; 16], row: &ChannelRow) -> Result<Put> {
    if let Some(held) = channel(conn, channel_id)? {
        if held.body == row.body {
            return Ok(Put::Same);
        }
        if row.created_log <= held.created_log {
            return Ok(Put::Stale);
        }
    }
    conn.execute(
        "INSERT OR REPLACE INTO channels (channel, circle, body, created_log) VALUES (?1, ?2, ?3, ?4)",
        params![&channel_id[..], &row.circle[..], &row.body, to_i64(row.created_log)],
    )?;
    Ok(Put::Stored)
}

// Policies.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRow {
    pub id: [u8; 32],
    /// The signed policy.
    pub body: Vec<u8>,
    pub log: u64,
    /// The CBOR array of approvals that came with it, for the gate to check.
    pub approvals: Vec<u8>,
}

fn policy_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PolicyRow> {
    let id: Vec<u8> = r.get(0)?;
    let log: i64 = r.get(2)?;
    Ok(PolicyRow {
        id: id.try_into().unwrap_or_default(),
        body: r.get(1)?,
        log: log as u64,
        approvals: r.get(3)?,
    })
}

pub fn policy(conn: &Connection, id: &[u8; 32]) -> Result<Option<PolicyRow>> {
    conn.query_row(
        "SELECT id, body, log, approvals FROM policies WHERE id = ?1",
        [&id[..]],
        policy_row,
    )
    .optional()
}

/// The channel's current policy: the one stored with the greatest `log`.
pub fn current_policy(conn: &Connection, channel: &[u8; 16]) -> Result<Option<PolicyRow>> {
    conn.query_row(
        "SELECT id, body, log, approvals FROM policies WHERE channel = ?1 ORDER BY log DESC LIMIT 1",
        [&channel[..]],
        policy_row,
    )
    .optional()
}

/// Store a verified policy. It becomes current only when its `log` is greater than the current
/// policy's; otherwise it is refused as stale.
pub fn put_policy(conn: &Connection, channel: &[u8; 16], row: &PolicyRow) -> Result<Put> {
    if policy(conn, &row.id)?.is_some() {
        return Ok(Put::Same);
    }
    if current_policy(conn, channel)?.is_some_and(|c| row.log <= c.log) {
        return Ok(Put::Stale);
    }
    conn.execute(
        "INSERT INTO policies (id, channel, body, log, approvals) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            &row.id[..],
            &channel[..],
            &row.body,
            to_i64(row.log),
            &row.approvals
        ],
    )?;
    Ok(Put::Stored)
}

// Grants.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantRow {
    pub cti: [u8; 16],
    pub channel: [u8; 16],
    /// `cahoots_objects::grant::key_id` of the leaf's `cnf`.
    pub kid: [u8; 32],
    /// The canonical CBOR array of signed links, root first.
    pub chain: Vec<u8>,
    /// The leaf's `log_not_after`.
    pub not_after: Option<u64>,
}

fn grant_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<GrantRow> {
    let cti: Vec<u8> = r.get(0)?;
    let channel: Vec<u8> = r.get(1)?;
    let kid: Vec<u8> = r.get(2)?;
    let not_after: Option<i64> = r.get(4)?;
    Ok(GrantRow {
        cti: cti.try_into().unwrap_or_default(),
        channel: channel.try_into().unwrap_or_default(),
        kid: kid.try_into().unwrap_or_default(),
        chain: r.get(3)?,
        not_after: not_after.map(|n| n as u64),
    })
}

pub fn grant(conn: &Connection, channel: &[u8; 16], cti: &[u8; 16]) -> Result<Option<GrantRow>> {
    conn.query_row(
        "SELECT cti, channel, kid, chain, not_after FROM grants WHERE cti = ?1 AND channel = ?2",
        params![&cti[..], &channel[..]],
        grant_row,
    )
    .optional()
}

/// The channel's grants whose leaf is held by `kid`, in insertion order.
pub fn grants_for_kid(
    conn: &Connection,
    channel: &[u8; 16],
    kid: &[u8; 32],
) -> Result<Vec<GrantRow>> {
    let mut stmt = conn.prepare(
        "SELECT cti, channel, kid, chain, not_after FROM grants WHERE channel = ?1 AND kid = ?2 ORDER BY rowid",
    )?;
    let rows = stmt.query_map(params![&channel[..], &kid[..]], grant_row)?;
    rows.collect()
}

/// Store a grant chain under its leaf's `cti`. A `cti` keeps its first chain.
pub fn put_grant(conn: &Connection, row: &GrantRow) -> Result<Put> {
    let held: Option<Vec<u8>> = conn
        .query_row(
            "SELECT chain FROM grants WHERE cti = ?1",
            [&row.cti[..]],
            |r| r.get(0),
        )
        .optional()?;
    match held {
        Some(chain) if chain == row.chain => Ok(Put::Same),
        Some(_) => Ok(Put::Exists),
        None => {
            conn.execute(
                "INSERT INTO grants (cti, channel, kid, chain, not_after) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    &row.cti[..],
                    &row.channel[..],
                    &row.kid[..],
                    &row.chain,
                    row.not_after.map(to_i64)
                ],
            )?;
            Ok(Put::Stored)
        }
    }
}

// Revocations.

/// Whether `grant_id` is revoked in the channel.
pub fn revoked(conn: &Connection, channel: &[u8; 16], grant_id: &[u8; 16]) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM revocations WHERE channel = ?1 AND grant_id = ?2)",
        params![&channel[..], &grant_id[..]],
        |r| r.get(0),
    )
}

/// The channel's revocations as `(grant_id, seq)`, in insertion order. `seq` is the log head
/// when the revocation was stored, `None` when the log was empty.
pub fn revocations(conn: &Connection, channel: &[u8; 16]) -> Result<Vec<([u8; 16], Option<u64>)>> {
    let mut stmt =
        conn.prepare("SELECT grant_id, seq FROM revocations WHERE channel = ?1 ORDER BY rowid")?;
    let rows = stmt.query_map([&channel[..]], |r| {
        let id: Vec<u8> = r.get(0)?;
        let seq: Option<i64> = r.get(1)?;
        Ok((id.try_into().unwrap_or_default(), seq.map(|s| s as u64)))
    })?;
    rows.collect()
}

/// Store a revocation. A grant already revoked keeps its first revocation.
pub fn put_revocation(
    conn: &Connection,
    channel: &[u8; 16],
    grant_id: &[u8; 16],
    seq: Option<u64>,
    body: &[u8],
) -> Result<Put> {
    if revoked(conn, channel, grant_id)? {
        return Ok(Put::Same);
    }
    conn.execute(
        "INSERT INTO revocations (channel, grant_id, seq, body) VALUES (?1, ?2, ?3, ?4)",
        params![&channel[..], &grant_id[..], seq.map(to_i64), body],
    )?;
    Ok(Put::Stored)
}

// Deposits.

/// Store a deposit under its id. The same id is the same bytes, so a repeat is `Same`.
pub fn put_deposit(
    conn: &Connection,
    id: &[u8; 32],
    channel: &[u8; 16],
    epoch: &[u8; 20],
    to: &[u8; 32],
    body: &[u8],
) -> Result<Put> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO deposits (id, channel, epoch, to_, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![&id[..], &channel[..], &epoch[..], &to[..], body],
    )?;
    Ok(if n == 0 { Put::Same } else { Put::Stored })
}

/// The channel's deposits, filtered by recipient and epoch when given, in insertion order.
pub fn deposits(
    conn: &Connection,
    channel: &[u8; 16],
    to: Option<&[u8; 32]>,
    epoch: Option<&[u8; 20]>,
) -> Result<Vec<Vec<u8>>> {
    let mut stmt = conn.prepare(
        "SELECT body FROM deposits WHERE channel = ?1 AND (?2 IS NULL OR to_ = ?2) \
         AND (?3 IS NULL OR epoch = ?3) ORDER BY rowid",
    )?;
    let rows = stmt.query_map(
        params![&channel[..], to.map(|t| &t[..]), epoch.map(|e| &e[..])],
        |r| r.get(0),
    )?;
    rows.collect()
}

// Join requests.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRow {
    pub id: [u8; 32],
    pub body: Vec<u8>,
    pub seq: u64,
}

/// Store a request under `id` with the channel's next `seq` (from 0). A repeat keeps its
/// first `seq`, which is returned.
pub fn put_request(
    conn: &Connection,
    channel: &[u8; 16],
    id: &[u8; 32],
    body: &[u8],
) -> Result<u64> {
    let held: Option<i64> = conn
        .query_row("SELECT seq FROM requests WHERE id = ?1", [&id[..]], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(seq) = held {
        return Ok(seq as u64);
    }
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq) + 1, 0) FROM requests WHERE channel = ?1",
        [&channel[..]],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO requests (id, channel, body, seq) VALUES (?1, ?2, ?3, ?4)",
        params![&id[..], &channel[..], body, seq],
    )?;
    Ok(seq as u64)
}

/// The channel's requests with `seq > after` (all when `None`), at most `limit`, by `seq`.
pub fn requests_after(
    conn: &Connection,
    channel: &[u8; 16],
    after: Option<u64>,
    limit: usize,
) -> Result<Vec<RequestRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, body, seq FROM requests WHERE channel = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
    )?;
    let after = after.map(to_i64).unwrap_or(-1);
    let rows = stmt.query_map(params![&channel[..], after, limit as i64], |r| {
        let id: Vec<u8> = r.get(0)?;
        let seq: i64 = r.get(2)?;
        Ok(RequestRow {
            id: id.try_into().unwrap_or_default(),
            body: r.get(1)?,
            seq: seq as u64,
        })
    })?;
    rows.collect()
}

// Gate tokens.

pub fn put_gate(conn: &Connection, channel: &[u8; 16], token_hash: &[u8; 32]) -> Result<Put> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO gates (channel, token_hash) VALUES (?1, ?2)",
        params![&channel[..], &token_hash[..]],
    )?;
    Ok(if n == 0 { Put::Same } else { Put::Stored })
}

/// Remove a gate token. `true` when one was removed.
pub fn delete_gate(conn: &Connection, channel: &[u8; 16], token_hash: &[u8; 32]) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM gates WHERE channel = ?1 AND token_hash = ?2",
        params![&channel[..], &token_hash[..]],
    )?;
    Ok(n > 0)
}

pub fn has_gate(conn: &Connection, channel: &[u8; 16], token_hash: &[u8; 32]) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM gates WHERE channel = ?1 AND token_hash = ?2)",
        params![&channel[..], &token_hash[..]],
        |r| r.get(0),
    )
}
