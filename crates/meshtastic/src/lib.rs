//! Meshtastic channel keys from a Cahoots circle.
//!
//! A Meshtastic channel's whole security is its pre-shared key, which people share today as
//! a QR code or a URL. Here the key comes from the pad: one member spends 32 bytes and posts a
//! `psk` control message addressed to everyone; every other member spends the same range and
//! derives the same 32-byte AES-256 key. Rotation is the same step again. The output is what
//! Meshtastic already accepts: a channel URL for the app, and the CLI's `--ch-set psk` form.
//!
//! The key is derived with the context `meshtastic || issuer || 255`, so it is unrelated to
//! any WireGuard key from the same pad.

use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use base64::Engine;
use cahoots_core::control::{Purpose, EVERYONE};
use cahoots_core::ledger::SpentLedger;
use cahoots_core::pad::{Pad, PadStorage};
use cahoots_core::CircleId;
use cahoots_provider::PadProvider;
use cahoots_wireguard::{accept_bytes, issue_for, issue_on_feed_for, PskToken, WgError};
use zeroize::Zeroizing;

pub const PURPOSE: Purpose = Purpose::Meshtastic;
pub const URL_PREFIX: &str = "https://meshtastic.org/e/#";

/// A channel the whole circle shares.
pub struct Channel {
    pub token: PskToken,
    pub name: String,
    pub key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("token", &self.token)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Channel {
    pub fn key_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(B64.encode(*self.key))
    }
    /// `meshtastic --ch-index N --ch-set psk base64:<key> --ch-set name <name>`.
    pub fn cli_command(&self, index: u8) -> Zeroizing<String> {
        Zeroizing::new(format!(
            "meshtastic --ch-index {index} --ch-set psk base64:{} --ch-set name {}",
            *self.key_base64(),
            shell_quote(&self.name)
        ))
    }
    /// The URL the Meshtastic apps import: a `ChannelSet` protobuf, base64url without padding.
    pub fn url(&self) -> Zeroizing<String> {
        Zeroizing::new(format!(
            "{URL_PREFIX}{}",
            B64URL.encode(channel_set(&self.key, &self.name))
        ))
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Protobuf by hand: `ChannelSet { settings = 1: [ChannelSettings { psk = 2, name = 3 }] }`.
/// Both fields are length-delimited (wire type 2), so a tag is `field << 3 | 2`.
pub fn channel_set(key: &[u8; 32], name: &str) -> Vec<u8> {
    let mut settings = Vec::with_capacity(4 + 32 + name.len());
    settings.push((2 << 3) | 2);
    settings.push(32);
    settings.extend_from_slice(key);
    if !name.is_empty() {
        settings.push((3 << 3) | 2);
        settings.extend_from_slice(&varint(name.len()));
        settings.extend_from_slice(name.as_bytes());
    }
    let mut set = Vec::with_capacity(2 + settings.len());
    set.push((1 << 3) | 2);
    set.extend_from_slice(&varint(settings.len()));
    set.extend_from_slice(&settings);
    set
}

fn varint(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

/// Spend 32 bytes and derive the circle's channel key. The token names it for the others.
pub fn issue(
    provider: &mut impl PadProvider,
    circle: &CircleId,
    name: &str,
) -> Result<Channel, WgError> {
    let issued = issue_for(provider, circle, PURPOSE, EVERYONE)?;
    Ok(Channel {
        token: issued.token,
        name: name.to_string(),
        key: decode_key(&issued)?,
    })
}

fn decode_key(issued: &cahoots_wireguard::Issued) -> Result<Zeroizing<[u8; 32]>, WgError> {
    let bytes = B64
        .decode(&*issued.preshared_key)
        .map_err(|_| WgError::Token)?;
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Issue and seal the `psk` message that tells every member.
pub fn issue_on_feed<S: PadStorage, L: SpentLedger>(
    pad: &mut Pad<S, L>,
    name: &str,
) -> Result<(Channel, Vec<u8>), WgError> {
    let (issued, wire) = issue_on_feed_for(pad, PURPOSE, EVERYONE)?;
    Ok((
        Channel {
            token: issued.token,
            name: name.to_string(),
            key: decode_key(&issued)?,
        },
        wire,
    ))
}

/// Accept a channel token: spend the issuer's range and derive the same key.
pub fn accept(
    provider: &mut impl PadProvider,
    token: &PskToken,
    name: &str,
) -> Result<Channel, WgError> {
    if token.purpose != PURPOSE || token.to != EVERYONE {
        return Err(WgError::NotForMe { to: token.to });
    }
    let key = accept_bytes(provider, token)?;
    Ok(Channel {
        token: *token,
        name: name.to_string(),
        key,
    })
}
