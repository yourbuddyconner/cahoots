//! Invite links: `https://cahoots.network/app/#/ch/<relay>/<channel id hex>/<secret base64url>`.
//!
//! The secret never leaves the URL fragment. Three things come from it, each under its own domain:
//! the link id (what a policy names), the seal key (wraps key deposits) and the gate token (what
//! the relay checks).

use crate::ChannelId;
use alloc::string::String;
use alloc::vec::Vec;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const BASE: &str = "https://cahoots.network/app/";

const FRAGMENT: &str = "#/ch/";
const ID_DOMAIN: &[u8] = b"cahoots/link/id/v1";
const SEAL_SALT: &[u8] = b"cahoots/link/seal/v1";
const GATE_SALT: &[u8] = b"cahoots/link/gate/v1";

/// The 32-byte secret in a link.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkSecret(pub [u8; 32]);

impl core::fmt::Debug for LinkSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("LinkSecret(..)")
    }
}

impl LinkSecret {
    /// `SHA-256("cahoots/link/id/v1" || secret)`.
    pub fn id(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(ID_DOMAIN);
        h.update(self.0);
        h.finalize().into()
    }

    fn derive(&self, salt: &[u8], channel: &ChannelId) -> Zeroizing<[u8; 32]> {
        let hk = Hkdf::<Sha256>::new(Some(salt), &self.0);
        let mut out = Zeroizing::new([0u8; 32]);
        hk.expand(channel, out.as_mut())
            .expect("32 bytes is a valid HKDF length");
        out
    }

    /// The key that wraps key deposits for this channel.
    pub fn seal_key(&self, channel: &ChannelId) -> Zeroizing<[u8; 32]> {
        self.derive(SEAL_SALT, channel)
    }

    /// The token the relay checks for this channel.
    pub fn gate_token(&self, channel: &ChannelId) -> [u8; 32] {
        *self.derive(GATE_SALT, channel)
    }
}

/// A parsed invite link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Host with an optional port; no scheme, no slash.
    pub relay: String,
    pub channel_id: ChannelId,
    pub secret: Option<LinkSecret>,
}

impl Link {
    pub fn to_url(&self) -> String {
        let mut s = String::from(BASE);
        s.push_str(FRAGMENT);
        s.push_str(&self.relay);
        s.push('/');
        for b in self.channel_id {
            s.push(char::from(HEX[usize::from(b >> 4)]));
            s.push(char::from(HEX[usize::from(b & 15)]));
        }
        if let Some(secret) = &self.secret {
            s.push('/');
            b64_encode(&secret.0, &mut s);
        }
        s
    }

    /// Parse a full URL or just its `#/ch/...` fragment.
    pub fn parse(url: &str) -> Option<Link> {
        let frag = url.strip_prefix(BASE).unwrap_or(url);
        let rest = frag.strip_prefix(FRAGMENT)?;
        let mut parts = rest.split('/');
        let relay = parts.next()?;
        let id = parts.next()?;
        let secret = parts.next();
        if parts.next().is_some() || !valid_relay(relay) {
            return None;
        }
        let channel_id: ChannelId = hex_decode(id)?.try_into().ok()?;
        let secret = match secret {
            None => None,
            Some(s) => Some(LinkSecret(b64_decode(s)?.try_into().ok()?)),
        };
        Some(Link {
            relay: String::from(relay),
            channel_id,
            secret,
        })
    }
}

/// A host or `host:port`: letters, digits, `.`, `-`, and an optional `:digits`.
fn valid_relay(relay: &str) -> bool {
    let (host, port) = match relay.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (relay, None),
    };
    let host_ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    host_ok && port_ok
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let nib = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    let s = s.as_bytes();
    if s.len() % 2 != 0 {
        return None;
    }
    s.chunks(2)
        .map(|p| Some(nib(p[0])? << 4 | nib(p[1])?))
        .collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(data: &[u8], out: &mut String) {
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &b)| a | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(B64[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = B64.iter().position(|&x| x == c)? as u32;
            n |= v << (18 - 6 * i);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..chunk.len()]);
        // Unused low bits must be zero, so one secret has one spelling.
        let used = chunk.len() * 6 / 8 * 8;
        if n & ((1 << (24 - used)) - 1) != 0 {
            return None;
        }
    }
    Some(out)
}
