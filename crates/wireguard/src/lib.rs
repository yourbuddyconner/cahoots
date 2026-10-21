//! WireGuard `PresharedKey` helper (handoff 10.2, S4).
//!
//! WireGuard's Noise handshake mixes an optional 32-byte pre-shared key into every
//! session key (the `psk2` modifier). If the PSK is secret, the tunnel stays confidential
//! even if Curve25519 falls. Cahoots supplies that PSK from a circle pad:
//!
//! 1. One peer calls [`issue`]: the provider spends 32 pad bytes and the helper derives
//!    the PSK with the `wireguard` context (handoff 10.1). The peer gets a handle token.
//! 2. The other peer calls [`accept`] with the token: the provider spends the same range
//!    and derives the same PSK. A second accept of the same token is refused.
//! 3. Both sides install it: `wg set <iface> peer <pk> preshared-key /dev/stdin`, or
//!    paste the rendered `[Peer]` section into the config.
//! 4. Rotate on a schedule. A 256 KiB pad with 4 members gives each member 2048
//!    rotations: one a day for five and a half years.
//!
//! The PSK is derived, not the raw pad bytes, so a PSK released for WireGuard can't be
//! reused in another protocol. The tunnel data is computationally secure, not a
//! one-time pad. The PSK is the layer that doesn't depend on Curve25519.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cahoots_provider::{derive_session_key, CircleId, PadError, PadProvider, PskHandle};
use zeroize::Zeroizing;

pub const CONTEXT: &[u8] = b"wireguard";
pub const TOKEN_PREFIX: &str = "cahoots-wg:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WgError {
    Pad(PadError),
    /// A handle token that is not `cahoots-wg:<42 hex chars>`.
    Token,
}

impl From<PadError> for WgError {
    fn from(e: PadError) -> Self {
        WgError::Pad(e)
    }
}

pub struct Issued {
    pub handle: PskHandle,
    /// Base64 (standard, padded) 32-byte key, as `wg` and `wg-quick` expect.
    pub preshared_key: Zeroizing<String>,
}

impl Issued {
    /// The string to hand to the peer. It names the pad range and carries no key material.
    pub fn handle_token(&self) -> String {
        handle_token(&self.handle)
    }
}

pub fn handle_token(handle: &PskHandle) -> String {
    format!("{TOKEN_PREFIX}{}", hex::encode(handle.to_bytes()))
}

pub fn parse_handle_token(token: &str) -> Result<PskHandle, WgError> {
    let hex_part = token.strip_prefix(TOKEN_PREFIX).ok_or(WgError::Token)?;
    let bytes = hex::decode(hex_part).map_err(|_| WgError::Token)?;
    PskHandle::from_bytes(&bytes).ok_or(WgError::Token)
}

fn to_wg_key(psk: &[u8; 32], handle: &PskHandle) -> Zeroizing<String> {
    let key = Zeroizing::new(derive_session_key(psk, handle, CONTEXT, b"", b""));
    Zeroizing::new(B64.encode(*key))
}

/// Spend 32 bytes of the caller's slice and derive a WireGuard PSK.
pub fn issue(provider: &mut impl PadProvider, circle: &CircleId) -> Result<Issued, WgError> {
    let (handle, psk) = provider.psk(circle, CONTEXT)?;
    Ok(Issued {
        preshared_key: to_wg_key(&psk, &handle),
        handle,
    })
}

/// Spend the peer's range named by `handle` and derive the same PSK.
pub fn accept(
    provider: &mut impl PadProvider,
    handle: &PskHandle,
) -> Result<Zeroizing<String>, WgError> {
    let psk = provider.psk_for(handle, CONTEXT)?;
    Ok(to_wg_key(&psk, handle))
}

/// Render a `[Peer]` section for a WireGuard config file.
pub fn render_peer(
    peer_public_key: &str,
    preshared_key: &str,
    allowed_ips: &[&str],
    endpoint: Option<&str>,
    persistent_keepalive: Option<u16>,
) -> String {
    let mut out = String::from("[Peer]\n");
    out.push_str(&format!("PublicKey = {peer_public_key}\n"));
    out.push_str(&format!("PresharedKey = {preshared_key}\n"));
    out.push_str(&format!("AllowedIPs = {}\n", allowed_ips.join(", ")));
    if let Some(e) = endpoint {
        out.push_str(&format!("Endpoint = {e}\n"));
    }
    if let Some(k) = persistent_keepalive {
        out.push_str(&format!("PersistentKeepalive = {k}\n"));
    }
    out
}

/// The `wg set` invocation that installs a PSK from stdin, so the key never appears in
/// a process argument list.
pub fn wg_set_command(iface: &str, peer_public_key: &str) -> Vec<String> {
    [
        "wg",
        "set",
        iface,
        "peer",
        peer_public_key,
        "preshared-key",
        "/dev/stdin",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// How many more PSKs the caller's slice can issue.
pub fn rotations_remaining(provider: &impl PadProvider, circle: &CircleId) -> u64 {
    provider.remaining(circle) / 32
}

/// A fixed-interval rotation plan, in Unix seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationSchedule {
    pub started_at: u64,
    pub interval_secs: u64,
}

impl RotationSchedule {
    /// The first rotation strictly after `now`.
    pub fn next_rotation_after(&self, now: u64) -> u64 {
        let elapsed = now.saturating_sub(self.started_at);
        self.started_at + (elapsed / self.interval_secs + 1) * self.interval_secs
    }

    /// Rotations due between the start and `now`, inclusive of one at exactly `now`.
    pub fn rotations_until(&self, now: u64) -> u64 {
        now.saturating_sub(self.started_at) / self.interval_secs
    }

    /// When a budget of `rotations` runs out. The fix is a new ceremony.
    pub fn exhausted_at(&self, rotations: u64) -> u64 {
        self.started_at + rotations * self.interval_secs
    }
}
