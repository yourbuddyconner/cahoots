//! WireGuard `PresharedKey` helper over a Cahoots pad.
//!
//! WireGuard's Noise handshake mixes an optional 32-byte pre-shared key into every session
//! key (the `psk2` modifier). If the PSK is secret, the tunnel stays confidential even if
//! Curve25519 falls. Cahoots supplies that PSK from a circle pad:
//!
//! 1. The issuer calls [`issue`] for a peer seat: the provider spends 32 pad bytes of the
//!    issuer's slice and the helper derives the PSK with the context `wireguard || a || b`.
//!    The issuer gets a token naming the range and the pair; it carries no key material.
//! 2. The acceptor calls [`accept`] with the token: the provider spends the same range and
//!    derives the same PSK. A second accept of the same token is refused by the ledger.
//! 3. Both install it: `wg set <iface> peer <pk> preshared-key /dev/stdin`, or the rendered
//!    `[Peer]` section.
//! 4. Rotate on a schedule. A 256 KiB pad with 4 members gives each member 2048 rotations.
//!
//! The token can travel by hand, or as a `psk` control message on the Numbers Station feed,
//! next to `peer` announcements that carry public keys and endpoints. The feed functions at
//! the bottom of this file seal and open those messages with the same pad.
//!
//! The PSK is derived, not the raw pad bytes, and the derivation is bound to the purpose and
//! the pair, so a key released for WireGuard between A and B cannot be reused elsewhere.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cahoots_core::control::{Control, ControlError, Proto, Purpose};
use cahoots_core::ledger::SpentLedger;
use cahoots_core::message::MessageType;
use cahoots_core::pad::{Pad, PadStorage};
use cahoots_core::CircleId;
use cahoots_provider::{derive_session_key, PadError, PadProvider, PskHandle};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub const TOKEN_PREFIX: &str = "cahoots-wg:";
pub const PURPOSE: Purpose = Purpose::WireGuard;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WgError {
    Pad(PadError),
    /// A token that is not `cahoots-wg:<46 hex chars>`.
    Token,
    /// The token names a different acceptor seat.
    NotForMe {
        to: u8,
    },
    Control(ControlError),
    Feed(String),
}

impl From<PadError> for WgError {
    fn from(e: PadError) -> Self {
        WgError::Pad(e)
    }
}

impl std::fmt::Display for WgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WgError::Pad(PadError::AlreadySpent) => {
                write!(f, "refused: that range is already spent (replayed handle?)")
            }
            WgError::Pad(PadError::Exhausted) => {
                write!(f, "this slice of the pad is used up; hold a new ceremony")
            }
            WgError::Pad(e) => write!(f, "pad: {e:?}"),
            WgError::Token => write!(f, "that is not a cahoots-wg token"),
            WgError::NotForMe { to } => {
                write!(f, "that token is for seat {}, not this one", to + 1)
            }
            WgError::Control(e) => write!(f, "control message: {e:?}"),
            WgError::Feed(s) => write!(f, "feed: {s}"),
        }
    }
}

/// A pre-shared key handle plus the pair it keys: the issuer is the handle's slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PskToken {
    pub handle: PskHandle,
    pub purpose: Purpose,
    pub to: u8,
}

impl PskToken {
    pub fn from_seat(&self) -> u8 {
        self.handle.slice
    }
    pub fn context(&self) -> [u8; 3] {
        self.purpose.context(self.handle.slice, self.to)
    }
    /// `cahoots-wg:` then 23 bytes in hex: the 21-byte handle, the purpose, the acceptor seat.
    pub fn encode(&self) -> String {
        let mut b = self.handle.to_bytes().to_vec();
        b.push(self.purpose as u8);
        b.push(self.to);
        format!("{TOKEN_PREFIX}{}", hex::encode(b))
    }
    pub fn parse(token: &str) -> Result<Self, WgError> {
        let hex_part = token
            .trim()
            .strip_prefix(TOKEN_PREFIX)
            .ok_or(WgError::Token)?;
        let bytes = hex::decode(hex_part).map_err(|_| WgError::Token)?;
        if bytes.len() != 23 {
            return Err(WgError::Token);
        }
        let handle = PskHandle::from_bytes(&bytes[..21]).ok_or(WgError::Token)?;
        let purpose = Purpose::try_from(bytes[21]).map_err(|_| WgError::Token)?;
        Ok(PskToken {
            handle,
            purpose,
            to: bytes[22],
        })
    }
}

pub struct Issued {
    pub token: PskToken,
    /// Base64 (standard, padded) 32-byte key, as `wg` and `wg-quick` expect.
    pub preshared_key: Zeroizing<String>,
}

fn to_wg_key(psk: &[u8; 32], token: &PskToken) -> Zeroizing<String> {
    let key = Zeroizing::new(derive_session_key(
        psk,
        &token.handle,
        &token.context(),
        b"",
        b"",
    ));
    Zeroizing::new(B64.encode(*key))
}

/// Spend 32 bytes of the caller's slice and derive the PSK for the pair (caller, `to`).
pub fn issue(
    provider: &mut impl PadProvider,
    circle: &CircleId,
    to: u8,
) -> Result<Issued, WgError> {
    issue_for(provider, circle, PURPOSE, to)
}

/// `issue` for any purpose. `to` may be [`cahoots_core::control::EVERYONE`] for a group key.
pub fn issue_for(
    provider: &mut impl PadProvider,
    circle: &CircleId,
    purpose: Purpose,
    to: u8,
) -> Result<Issued, WgError> {
    let (handle, psk) = provider.psk(circle, &purpose.context(0, 0))?;
    let token = PskToken {
        handle,
        purpose,
        to,
    };
    Ok(Issued {
        preshared_key: to_wg_key(&psk, &token),
        token,
    })
}

/// Spend the issuer's range named by the token and derive the same PSK.
pub fn accept(
    provider: &mut impl PadProvider,
    token: &PskToken,
) -> Result<Zeroizing<String>, WgError> {
    let psk = provider.psk_for(&token.handle, &token.context())?;
    Ok(to_wg_key(&psk, token))
}

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

/// `wg set` arguments that install a PSK read from stdin, plus the peer's address and routes if known.
pub fn wg_set_command(
    iface: &str,
    peer_public_key: &str,
    endpoint: Option<&str>,
    allowed_ips: Option<&str>,
) -> Vec<String> {
    let mut v: Vec<String> = [
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
    .collect();
    if let Some(e) = endpoint {
        v.push("endpoint".into());
        v.push(e.into());
    }
    if let Some(a) = allowed_ips {
        v.push("allowed-ips".into());
        v.push(a.into());
    }
    v
}

pub fn rotations_remaining(provider: &impl PadProvider, circle: &CircleId) -> u64 {
    provider.remaining(circle) / 32
}

/// The circle's own subnet and a member's address in it: `10.x.y.0/24` from the circle ID,
/// seat k at `.k+1`. No address server, and no collision between circles in practice.
pub fn overlay_subnet(circle: &CircleId) -> String {
    format!("10.{}.{}.0/24", 1 + circle[0] % 254, circle[1])
}
pub fn overlay_address(circle: &CircleId, seat: u8) -> String {
    format!(
        "10.{}.{}.{}",
        1 + circle[0] % 254,
        circle[1],
        u16::from(seat) + 1
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationSchedule {
    pub started_at: u64,
    pub interval_secs: u64,
}

impl RotationSchedule {
    pub fn next_rotation_after(&self, now: u64) -> u64 {
        let elapsed = now.saturating_sub(self.started_at);
        self.started_at + (elapsed / self.interval_secs + 1) * self.interval_secs
    }
    pub fn rotations_until(&self, now: u64) -> u64 {
        now.saturating_sub(self.started_at) / self.interval_secs
    }
    pub fn exhausted_at(&self, rotations: u64) -> u64 {
        self.started_at + rotations * self.interval_secs
    }
}

// ---------------------------------------------------------------- the feed

/// What this host knows about a peer, from its `peer` announcement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub seat: u8,
    pub public_key: String,
    pub endpoint: Option<String>,
    pub allowed_ips: String,
}

/// Per-circle state this host keeps between runs: the feed cursor, the peers it has seen,
/// and the tokens it has already accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedState {
    pub seq: Option<u64>,
    pub peers: BTreeMap<u8, PeerInfo>,
    pub accepted: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedItem {
    Peer(PeerInfo),
    Psk(PskToken),
    Rotate { from: u8, to: u8, not_after: u64 },
    SliceTransfer { member: u8 },
    Other { seat: u8 },
}

/// Seal a `peer` announcement for this host.
pub fn announce<S: PadStorage, L: SpentLedger>(
    pad: &mut Pad<S, L>,
    public_key: &str,
    endpoint: Option<&str>,
    allowed_ips: &str,
) -> Result<Vec<u8>, WgError> {
    let pk = B64
        .decode(public_key)
        .map_err(|_| WgError::Feed("public key is not base64".into()))?;
    let body = Control::Peer {
        proto: Proto::WireGuard,
        public_key: pk,
        endpoint: endpoint.unwrap_or("").into(),
        allowed_ips: allowed_ips.into(),
    }
    .encode()
    .map_err(WgError::Control)?;
    pad.send(MessageType::Control, &body).map_err(core_pad)
}

/// Issue a PSK to `to` and seal the `psk` message that tells it so.
pub fn issue_on_feed<S: PadStorage, L: SpentLedger>(
    pad: &mut Pad<S, L>,
    to: u8,
) -> Result<(Issued, Vec<u8>), WgError> {
    issue_on_feed_for(pad, PURPOSE, to)
}

/// The raw 32-byte key for a token, for adapters that do not want base64.
pub fn accept_bytes(
    provider: &mut impl PadProvider,
    token: &PskToken,
) -> Result<Zeroizing<[u8; 32]>, WgError> {
    let psk = provider.psk_for(&token.handle, &token.context())?;
    Ok(Zeroizing::new(derive_session_key(
        &psk,
        &token.handle,
        &token.context(),
        b"",
        b"",
    )))
}

/// `issue_on_feed` for any purpose.
pub fn issue_on_feed_for<S: PadStorage, L: SpentLedger>(
    pad: &mut Pad<S, L>,
    purpose: Purpose,
    to: u8,
) -> Result<(Issued, Vec<u8>), WgError> {
    let circle = pad.circle_id();
    let issued = issue_for(pad, &circle, purpose, to)?;
    let body = Control::Psk {
        purpose,
        peer_a: issued.token.from_seat(),
        peer_b: to,
        offset: issued.token.handle.offset,
    }
    .encode()
    .map_err(WgError::Control)?;
    let wire = pad.send(MessageType::Control, &body).map_err(core_pad)?;
    Ok((issued, wire))
}

/// Open one feed entry. `Err(Feed("seen"))` for an entry this pad already opened or sent.
pub fn open_entry<S: PadStorage, L: SpentLedger>(
    pad: &mut Pad<S, L>,
    wire: &[u8],
) -> Result<FeedItem, WgError> {
    if wire.first() != Some(&1) {
        return Err(WgError::Feed("not a pure-pad message".into()));
    }
    let (header, plaintext) = match pad.receive(wire) {
        Ok(x) => x,
        Err(cahoots_core::pad::PadError::Reuse) => return Err(WgError::Feed("seen".into())),
        Err(e) => return Err(core_pad(e)),
    };
    let seat = header.slice;
    if header.msg_type != MessageType::Control {
        return Ok(FeedItem::Other { seat });
    }
    Ok(
        match Control::decode(&plaintext).map_err(WgError::Control)? {
            Control::Peer {
                public_key,
                endpoint,
                allowed_ips,
                ..
            } => FeedItem::Peer(PeerInfo {
                seat,
                public_key: B64.encode(public_key),
                endpoint: (!endpoint.is_empty()).then_some(endpoint),
                allowed_ips,
            }),
            Control::Psk {
                purpose,
                peer_a,
                peer_b,
                offset,
            } => {
                if peer_a != seat {
                    return Err(WgError::Feed(format!(
                        "psk message from seat {} claims issuer seat {}",
                        seat + 1,
                        peer_a + 1
                    )));
                }
                FeedItem::Psk(PskToken {
                    handle: PskHandle {
                        circle_id: header.circle_id,
                        slice: seat,
                        offset,
                    },
                    purpose,
                    to: peer_b,
                })
            }
            Control::Rotate {
                peer_a,
                peer_b,
                not_after,
                ..
            } => FeedItem::Rotate {
                from: peer_a,
                to: peer_b,
                not_after,
            },
            Control::SliceTransfer { member } => FeedItem::SliceTransfer { member },
        },
    )
}

/// Pull everything new from the relay, open it, and fold peers into `state`. Returns the
/// items in feed order. Entries the pad cannot open (its own, or already opened) are skipped.
pub fn sync<S: PadStorage, L: SpentLedger>(
    pad: &mut Pad<S, L>,
    relay: &mut dyn cahoots_relay::Relay,
    state: &mut FeedState,
) -> Result<Vec<FeedItem>, WgError> {
    let circle = pad.circle_id();
    let entries = relay
        .poll(&circle, state.seq, 0)
        .map_err(|e| WgError::Feed(e.to_string()))?;
    let mut items = Vec::new();
    for (seq, wire) in entries {
        state.seq = Some(seq);
        match open_entry(pad, &wire) {
            Ok(item) => {
                if let FeedItem::Peer(p) = &item {
                    state.peers.insert(p.seat, p.clone());
                }
                items.push(item);
            }
            Err(WgError::Feed(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(items)
}

fn core_pad(e: cahoots_core::pad::PadError) -> WgError {
    use cahoots_core::pad::PadError as E;
    WgError::Pad(match e {
        E::WrongCircle => PadError::UnknownCircle,
        E::OutOfRange => PadError::OutOfRange,
        E::Reuse => PadError::AlreadySpent,
        E::Exhausted => PadError::Exhausted,
        E::Config => PadError::Config,
        other => PadError::Storage(format!("{other:?}")),
    })
}
