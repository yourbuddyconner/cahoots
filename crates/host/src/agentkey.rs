//! The agent's own key: a P-256 key the badge grants signatures to. It proves to the badge,
//! on every use of a grant, that the laptop asking is the one the grant was made for.
//!
//! It lives in a file for now, `agent.key` in [`dir`]: the 32 secret bytes as lowercase hex,
//! readable by its owner only. It is made from the operating system's random source the first
//! time it is needed.

use cahoots_core::seat::SeatKey;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// The file name of the key in [`dir`].
pub const KEY_FILE: &str = "agent.key";

/// Where the agent's key and grants are kept: `$CAHOOTS_HOME`, or `~/.config/cahoots`.
pub fn dir() -> PathBuf {
    if let Some(home) = std::env::var_os("CAHOOTS_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".config").join("cahoots")
}

/// The key in `dir/agent.key`, made there if there is none.
pub fn load_or_create(dir: &Path) -> Result<SeatKey, String> {
    match load(dir)? {
        Some(key) => Ok(key),
        None => create(dir, &dir.join(KEY_FILE)),
    }
}

/// The key in `dir/agent.key`, or `None` when there is no such file.
pub fn load(dir: &Path) -> Result<Option<SeatKey>, String> {
    let path = dir.join(KEY_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text)
            .map(Some)
            .ok_or_else(|| format!("{}: not a key", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The key in `text`: 64 hex digits, with any white space around them.
fn parse(text: &str) -> Option<SeatKey> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    SeatKey::from_seed(&seed).ok()
}

/// Draw a key and write it to `path`, which must not exist yet. A directory made here is
/// readable by its owner only; one that was there already is left as it is.
fn create(dir: &Path, path: &Path) -> Result<SeatKey, String> {
    if !dir.is_dir() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let key = loop {
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut seed[..]).map_err(|e| format!("the random source failed: {e}"))?;
        // A seed of zero or past the curve order is drawn again.
        if let Ok(k) = SeatKey::from_seed(&seed) {
            break k;
        }
    };
    let bytes = key.to_bytes();
    let text = Zeroizing::new(
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>() + "\n",
    );
    // Written whole to a side file first, then linked in under its name: a partial write
    // never leaves a key that cannot load, and a key made meanwhile is never replaced.
    let tmp = dir.join(format!("{KEY_FILE}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(text.as_bytes())?;
            f.sync_all()
        })
        .and_then(|()| std::fs::hard_link(&tmp, path));
    let _ = std::fs::remove_file(&tmp);
    match written {
        Ok(()) => Ok(key),
        // Another process made the key first: use that one.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => load(dir)?
            .ok_or_else(|| format!("{}: {e}", path.display())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_made_once_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("home");
        assert!(load(&dir).unwrap().is_none());
        let a = load_or_create(&dir).unwrap();
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700, "a directory made here");
        assert_eq!(load(&dir).unwrap().unwrap().public(), a.public());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no side file left");
        let b = load_or_create(&dir).unwrap();
        assert_eq!(a.public(), b.public());
        let mode = std::fs::metadata(dir.join(KEY_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::write(dir.join(KEY_FILE), "not hex\n").unwrap();
        assert!(load_or_create(&dir).is_err());
    }

    #[test]
    fn a_directory_that_was_there_keeps_its_mode() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        load_or_create(tmp.path()).unwrap();
        let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}
