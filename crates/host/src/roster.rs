//! `cahoots-host roster --ssh --circle <name> --state <dir>`: an `authorized_keys` line for
//! every seat in a circle, from the circle's sealed manifest.
//!
//! The manifest comes from a badge's state directory (`circle.txt`, line `manifest=`): a
//! simulated badge's, or a simulated card's from a pile run with a real board on top, since
//! every seat keeps the same sealed manifest. The badge's `Identify` answer names circles but
//! carries no manifest. Every seat's signature on the manifest is checked before a line is
//! printed.

use crate::agent::authorized_key;
use crate::device::Device;
use cahoots_core::manifest::Manifest;
use std::path::Path;

/// The verified manifest of the circle kept in `state`, if its name is `circle`.
pub fn manifest(state: &Path, circle: &str) -> Result<Manifest, String> {
    let device = Device::open(state).map_err(|e| format!("{}: {e}", state.display()))?;
    let sealed = device
        .manifest()
        .ok_or_else(|| format!("{}: no manifest kept", state.display()))?;
    let manifest = sealed
        .verify()
        .map_err(|e| format!("{}: the manifest does not verify: {e:?}", state.display()))?;
    if manifest.params.name != circle {
        return Err(format!(
            "{} keeps the circle {}, not {circle}",
            state.display(),
            manifest.params.name
        ));
    }
    Ok(manifest)
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
