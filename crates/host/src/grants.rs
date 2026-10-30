//! Grants for an agent, on the laptop: ask the badge for one, keep it, list it, show it, and
//! check it as any CWT verifier would.
//!
//! A grant lets the agent's key (see [`crate::agentkey`]) sign git commits as one author, a
//! fixed number of times, until a date or until revoked, without a press on the badge. The
//! badge's holder allows it once, on the badge's screen.
//!
//! Each grant is kept in `<dir>/grants/` as `<circle>-<first 4 bytes of its cti, hex>.cwt`
//! (the signed token, as the badge sent it) with a `.json` beside it that says which circle it
//! is for, the agent's name, and how many signatures the badge last said were left. The badge
//! keeps the real count; the file only saves asking for a grant that is used up.

use crate::badge_client::{describe_error, BadgeClient};
use crate::card;
use cahoots_core::cose;
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_custodian::wire::{ChipRequest, ChipResponse, Request, Response};
use cahoots_objects::authz::{Action, SignGrant};
use cahoots_objects::cwt::{self, Claims};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// One grant the agent keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    /// The circle's name.
    pub circle: String,
    pub cti: [u8; 16],
    /// The signed grant.
    pub bytes: Vec<u8>,
    /// What the badge last said was left; `None` when it has not said.
    pub remaining: Option<u32>,
    pub agent: String,
}

impl Stored {
    /// The file the grant is kept in, in `dir/grants`.
    /// A character of the circle's name that does not belong in a file name is written `_`;
    /// the `.json` keeps the name as it is.
    pub fn path(&self, dir: &Path) -> PathBuf {
        let name: String = self
            .circle
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' })
            .collect();
        dir.join("grants")
            .join(format!("{name}-{}.cwt", hex(&self.cti[..4])))
    }

    /// The grant's claims, read without checking the signature.
    pub fn claims(&self) -> Result<Claims, String> {
        claims(&self.bytes)
    }
}

/// The claims of a signed grant, read without checking the signature.
fn claims(bytes: &[u8]) -> Result<Claims, String> {
    let payload = cose::payload1(bytes).map_err(|e| format!("not a signed token: {e:?}"))?;
    Claims::decode(&payload).map_err(|e| format!("not a grant: {e:?}"))
}

/// The laptop's clock, in Unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Ask the badge for a grant: the agent named `agent` may sign git commits as `author` (at
/// `location` only, if given) `budget` times, until `until` (Unix seconds) or, with
/// `standing`, until revoked. The badge asks its holder; the call waits for the answer. The
/// grant that comes back is checked against the seat key the badge reports before it is
/// returned. It is not saved: see [`save`].
#[allow(clippy::too_many_arguments)]
pub fn request(
    badge: &mut BadgeClient,
    circle: &str,
    agent: &str,
    author: &str,
    location: Option<&str>,
    budget: u32,
    until: Option<u64>,
    standing: bool,
    key: &SeatKey,
) -> Result<Stored, String> {
    if standing == until.is_some() {
        return Err("give an end date or ask for a grant until revoked, not both".into());
    }
    let info = card::circle(badge, circle)?;
    let chip = |badge: &mut BadgeClient, op| -> Result<ChipResponse, String> {
        match badge
            .request(Request::Chip {
                circle: info.circle_id,
                op,
            })
            .map_err(|e| e.to_string())?
        {
            Response::Chip { result } => Ok(result),
            Response::Error { code } => Err(describe_error(code).into()),
            other => Err(format!("unexpected answer {other:?}")),
        }
    };
    let seat_key = match chip(badge, ChipRequest::Describe)? {
        ChipResponse::Description { public, .. } => compressed(&public)
            .ok_or("the badge's seat key does not parse")?,
        other => return Err(format!("unexpected answer {other:?}")),
    };
    let cnf = key.public();
    let op = ChipRequest::GrantRequest {
        action: Action::GitCommit.name().into(),
        author: author.into(),
        location: location.map(String::from),
        agent: agent.into(),
        budget,
        exp: until,
        standing,
        cnf: cnf.0,
        time: now(),
    };
    let signed = match chip(badge, op)? {
        ChipResponse::Grant { grant, .. } => grant,
        other => return Err(format!("unexpected answer {other:?}")),
    };
    let g = SignGrant::verify(&signed, &seat_key, &info.circle_id, info.seat)
        .map_err(|e| format!("the badge's grant does not verify: {e:?}"))?;
    let asked = g.grantee == cnf
        && g.actions == [Action::GitCommit]
        && g.author == author
        && g.agent == agent
        && g.budget == budget
        && g.exp == until
        && g.locations == location.map(String::from).into_iter().collect::<Vec<_>>();
    if !asked {
        return Err("the badge's grant is not the one asked for".into());
    }
    Ok(Stored {
        circle: circle.into(),
        cti: g.cti,
        bytes: signed,
        remaining: Some(budget),
        agent: agent.into(),
    })
}

/// A P-256 public key in either SEC1 form, compressed.
fn compressed(sec1: &[u8]) -> Option<SeatPublic> {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let point = p256::PublicKey::from_sec1_bytes(sec1)
        .ok()?
        .to_encoded_point(true);
    SeatPublic::parse(point.as_bytes()).ok()
}

/// Keep `s` in `dir/grants`: the token, and the `.json` beside it. Returns the token's path.
pub fn save(dir: &Path, s: &Stored) -> Result<PathBuf, String> {
    let path = s.path(dir);
    let folder = dir.join("grants");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let meta = json!({
        "circle": s.circle,
        "agent": s.agent,
        "remaining": s.remaining,
    });
    write_atomic(&path, &s.bytes)?;
    let text = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())? + "\n";
    write_atomic(&path.with_extension("json"), text.as_bytes())?;
    Ok(path)
}

/// Write `bytes` to a side file and rename it over `path`, so a reader never sees half.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Every grant kept in `dir/grants`, oldest first. A file that does not read as a grant is
/// skipped.
pub fn load_all(dir: &Path) -> Vec<Stored> {
    let Ok(entries) = std::fs::read_dir(dir.join("grants")) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf, Stored)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("cwt") {
            continue;
        }
        if let Some(s) = load_one(&path) {
            let born = entry
                .metadata()
                .and_then(|m| m.created().or_else(|_| m.modified()))
                .unwrap_or(std::time::UNIX_EPOCH);
            found.push((born, path, s));
        }
    }
    found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    found.into_iter().map(|(_, _, s)| s).collect()
}

/// The grant in `path` and the `.json` beside it.
fn load_one(path: &Path) -> Option<Stored> {
    let bytes = std::fs::read(path).ok()?;
    let c = claims(&bytes).ok()?;
    let meta: Value = std::fs::read_to_string(path.with_extension("json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    // Without the .json, the circle is the file name less its `-<cti>` ending.
    let circle = match meta["circle"].as_str() {
        Some(c) => c.to_string(),
        None => path
            .file_stem()?
            .to_str()?
            .rsplit_once('-')?
            .0
            .to_string(),
    };
    let agent = match meta["agent"].as_str() {
        Some(a) => a.to_string(),
        None => SignGrant::from_claims(&c).ok()?.agent,
    };
    let remaining = meta["remaining"].as_u64().and_then(|n| u32::try_from(n).ok());
    Some(Stored {
        circle,
        cti: c.cti,
        bytes,
        remaining,
        agent,
    })
}

/// The newest grant kept for `circle` that the badge has not said is used up.
pub fn find(dir: &Path, circle: &str) -> Option<Stored> {
    load_all(dir)
        .into_iter()
        .rev()
        .find(|s| s.circle == circle && s.remaining != Some(0))
}

/// A signed grant's claims as JSON (`iss`, `sub`, `aud`, `exp`, `cti`, the authorization
/// details and the rest), read without checking the signature.
pub fn show(bytes: &[u8]) -> Result<Value, String> {
    claims(bytes).map(|c| c.to_json())
}

/// The seat a signed grant says issued it, read without checking the signature.
pub fn seat_of(bytes: &[u8]) -> Result<u8, String> {
    let c = claims(bytes)?;
    c.aud
        .as_deref()
        .and_then(cwt::seat_of)
        .map(|(_, seat)| seat)
        .ok_or_else(|| "the grant names no seat".into())
}

/// Check a signed grant as a plain verifier would: signed by `seat` of the circle `manifest`
/// describes, under that seat's key from the manifest, for that circle.
pub fn verify(bytes: &[u8], manifest: &Manifest, seat: u8) -> Result<SignGrant, String> {
    let key = manifest
        .seats
        .iter()
        .find(|s| s.seat == seat)
        .map(|s| s.key)
        .ok_or_else(|| format!("the circle has no seat {seat}"))?;
    SignGrant::verify(bytes, &key, &manifest.circle_id, seat)
        .map_err(|e| format!("the grant does not verify: {e:?}"))
}

/// `YYYY-MM-DD` as Unix seconds at midnight UTC that day.
pub fn parse_date(s: &str) -> Option<u64> {
    let mut parts = s.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=12).contains(&m) || d < 1 || d > month_days[(m - 1) as usize] {
        return None;
    }
    // Days since 1970-01-01 from a civil date (H. Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days).ok().map(|d| d * 86_400)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_midnight_utc() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2030-01-01"), Some(1_893_456_000));
        assert_eq!(parse_date("2024-02-29"), Some(1_709_164_800));
        assert_eq!(parse_date("2023-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-1-01"), None);
        assert_eq!(parse_date("1969-12-31"), None);
    }

    #[test]
    fn grants_are_kept_and_the_newest_unspent_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let seat = SeatKey::from_seed(&[7; 32]).unwrap();
        let agent = SeatKey::from_seed(&[9; 32]).unwrap();
        let grant = |cti: u8| SignGrant {
            circle_id: [1; 16],
            seat: 0,
            grantee: agent.public(),
            cti: [cti; 16],
            actions: vec![Action::GitCommit],
            author: "Ada <ada@example.com>".into(),
            locations: vec![],
            agent: "claude".into(),
            budget: 3,
            exp: None,
            base: 1 << 32,
        };
        let stored = |cti: u8, remaining| Stored {
            circle: "my-circle".into(),
            cti: [cti; 16],
            bytes: grant(cti).sign(&seat, 0),
            remaining,
            agent: "claude".into(),
        };
        assert_eq!(find(tmp.path(), "my-circle"), None);
        save(tmp.path(), &stored(1, Some(2))).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let path = save(tmp.path(), &stored(2, Some(0))).unwrap();
        assert!(path.ends_with("grants/my-circle-02020202.cwt"));
        assert_eq!(load_all(tmp.path()).len(), 2);
        assert_eq!(find(tmp.path(), "my-circle"), Some(stored(1, Some(2))));
        assert_eq!(find(tmp.path(), "other"), None);
        // Without its .json a grant still reads, its circle from the file name.
        std::fs::remove_file(path.with_extension("json")).unwrap();
        let back = load_all(tmp.path());
        assert_eq!(back[1].circle, "my-circle");
        assert_eq!(back[1].remaining, None);
        let shown = show(&back[1].bytes).unwrap();
        assert_eq!(shown["authorization_details"][0]["budget"], 3);
        assert_eq!(seat_of(&back[1].bytes), Ok(0));
    }
}
