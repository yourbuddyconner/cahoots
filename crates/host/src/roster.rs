//! `cahoots-host roster --ssh --circle <name> --state <dir>`: an `authorized_keys` line for
//! every seat in a circle, from the circle's sealed manifest.
//!
//! The manifest comes from a badge's state directory: a simulated badge's, or a simulated
//! card's from a pile run with a real board on top, since every seat keeps the same sealed
//! manifest. A badge keeps its first circle in `circle.txt` and the others in
//! `circle-<k>.txt` (see [`crate::board`]); each file's `manifest=` line is read, and the one
//! whose circle has the name asked for is used. The badge's `Identify` answer names circles but
//! carries no manifest. Every seat's signature on the manifest is checked before a line is
//! printed.

use crate::agent::authorized_key;
use crate::board::MAX_CIRCLES;
use cahoots_core::manifest::{Manifest, SealedManifest};
use std::path::{Path, PathBuf};

/// The circle files a state directory may hold, first place first.
fn circle_files(state: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    (0..MAX_CIRCLES).map(move |k| {
        if k == 0 {
            state.join("circle.txt")
        } else {
            state.join(format!("circle-{k}.txt"))
        }
    })
}

/// The verified manifest of the circle named `circle`, from whichever circle file in `state`
/// keeps it.
pub fn manifest(state: &Path, circle: &str) -> Result<Manifest, String> {
    let mut names = Vec::new();
    let mut problems = Vec::new();
    for path in circle_files(state) {
        let Ok(txt) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Only the manifest line is read: the seat key and root lines stay untouched.
        let Some(line) = txt.lines().find_map(|l| l.strip_prefix("manifest=")) else {
            problems.push(format!("{}: no manifest kept", path.display()));
            continue;
        };
        let manifest = unhex(line)
            .ok_or_else(|| "not hex".to_string())
            .and_then(|b| SealedManifest::decode(&b).map_err(|e| format!("{e:?}")))
            .and_then(|m| m.verify().map_err(|e| format!("{e:?}")));
        match manifest {
            Ok(m) if m.params.name == circle => return Ok(m),
            Ok(m) => names.push(m.params.name),
            Err(e) => problems.push(format!(
                "{}: the manifest does not verify: {e}",
                path.display()
            )),
        }
    }
    if names.is_empty() && problems.is_empty() {
        return Err(format!("{}: no circle kept", state.display()));
    }
    let mut msg = if names.is_empty() {
        format!("{} keeps no circle named {circle}", state.display())
    } else {
        format!(
            "{} keeps the circle{} {}, not {circle}",
            state.display(),
            if names.len() == 1 { "" } else { "s" },
            names.join(", ")
        )
    };
    for p in problems {
        msg.push_str("; ");
        msg.push_str(&p);
    }
    Err(msg)
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// One `authorized_keys` line per seat, in seat order, comment `cahoots:<circle>:<seat>`.
pub fn ssh_lines(manifest: &Manifest) -> Result<Vec<String>, String> {
    manifest
        .seats
        .iter()
        .map(|s| {
            authorized_key(
                &s.key.0,
                &format!("cahoots:{}:{}", manifest.params.name, s.seat),
            )
            .ok_or_else(|| format!("seat {}: the key does not parse", s.seat))
        })
        .collect()
}
