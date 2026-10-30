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
    let path = dir.join(KEY_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).ok_or_else(|| format!("{}: not a key", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => create(dir, &path),
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

/// Draw a key and write it to `path`, which must not exist yet.
fn create(dir: &Path, path: &Path) -> Result<SeatKey, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
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
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(text.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_made_once_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("home");
        let a = load_or_create(&dir).unwrap();
        let b = load_or_create(&dir).unwrap();
        assert_eq!(a.public(), b.public());
        let mode = std::fs::metadata(dir.join(KEY_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::write(dir.join(KEY_FILE), "not hex\n").unwrap();
        assert!(load_or_create(&dir).is_err());
    }
}
