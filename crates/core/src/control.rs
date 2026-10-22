//! Control messages: the plaintext of a `control` message (`MessageType::Control`). They let a
//! circle bootstrap a network: announce peers, issue pre-shared keys, schedule rotation. They
//! are sealed with pad bytes like any message, so only members can read or forge them.
//!
//! Layout: one subtype byte, then the fields below. All integers little-endian.
//!
//! | subtype | name | payload |
//! |---|---|---|
//! | 0 | `peer` | `proto u8 \| pubkey_len u8 \| pubkey \| endpoint_len u8 \| endpoint \| allowed_len u8 \| allowed_ips` |
//! | 1 | `psk` | `purpose u8 \| peer_a u8 \| peer_b u8 \| offset u32`. The issuer's slice is the message's own slice. |
//! | 2 | `rotate` | `purpose u8 \| peer_a u8 \| peer_b u8 \| not_after u64` |
//! | 3 | `slice-transfer` | `member u8` |
//!
//! A pre-shared key is derived with the context `purpose || peer_a || peer_b`, so a key issued
//! for WireGuard between A and B cannot be reused for TLS or for another pair.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Proto {
    WireGuard = 0,
    Reticulum = 1,
    Yggdrasil = 2,
    Meshtastic = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Purpose {
    WireGuard = 0,
    Tls = 1,
    Mls = 2,
    Wifi = 3,
    Meshtastic = 4,
    Other = 5,
}

impl Purpose {
    /// The context bound into a pre-shared key for this purpose between two members.
    pub fn context(self, peer_a: u8, peer_b: u8) -> [u8; 3] {
        [self as u8, peer_a, peer_b]
    }
}

impl TryFrom<u8> for Proto {
    type Error = ControlError;
    fn try_from(v: u8) -> Result<Self, ControlError> {
        Ok(match v {
            0 => Proto::WireGuard,
            1 => Proto::Reticulum,
            2 => Proto::Yggdrasil,
            3 => Proto::Meshtastic,
            _ => return Err(ControlError::Value),
        })
    }
}

impl TryFrom<u8> for Purpose {
    type Error = ControlError;
    fn try_from(v: u8) -> Result<Self, ControlError> {
        Ok(match v {
            0 => Purpose::WireGuard,
            1 => Purpose::Tls,
            2 => Purpose::Mls,
            3 => Purpose::Wifi,
            4 => Purpose::Meshtastic,
            5 => Purpose::Other,
            _ => return Err(ControlError::Value),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    /// The payload is shorter than its fields say, or has trailing bytes.
    Length,
    /// An unknown subtype byte.
    Subtype(u8),
    /// An unknown proto or purpose, or a field that does not fit its length byte.
    Value,
    Utf8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    /// A member's network identity: its public key, where to reach it, and what it routes.
    Peer {
        proto: Proto,
        public_key: Vec<u8>,
        endpoint: String,
        allowed_ips: String,
    },
    /// A pre-shared key for the pair `(peer_a, peer_b)`, spent from the sender's slice at `offset`.
    Psk {
        purpose: Purpose,
        peer_a: u8,
        peer_b: u8,
        offset: u32,
    },
    /// The pair should re-key before `not_after` (Unix seconds).
    Rotate {
        purpose: Purpose,
        peer_a: u8,
        peer_b: u8,
        not_after: u64,
    },
    /// The member's slice now lives on a node, not a badge.
    SliceTransfer { member: u8 },
}

/// `peer_b` of a `psk` message that keys every member at once, such as a Meshtastic channel.
/// Every member accepts it and spends the same range of the issuer's slice.
pub const EVERYONE: u8 = 255;

pub const SUBTYPE_PEER: u8 = 0;
pub const SUBTYPE_PSK: u8 = 1;
pub const SUBTYPE_ROTATE: u8 = 2;
pub const SUBTYPE_SLICE_TRANSFER: u8 = 3;

fn push_bytes(out: &mut Vec<u8>, b: &[u8]) -> Result<(), ControlError> {
    let len = u8::try_from(b.len()).map_err(|_| ControlError::Value)?;
    out.push(len);
    out.extend_from_slice(b);
    Ok(())
}

fn take<'a>(buf: &mut &'a [u8], n: usize) -> Result<&'a [u8], ControlError> {
    if buf.len() < n {
        return Err(ControlError::Length);
    }
    let (head, rest) = buf.split_at(n);
    *buf = rest;
    Ok(head)
}

fn take_var<'a>(buf: &mut &'a [u8]) -> Result<&'a [u8], ControlError> {
    let n = usize::from(take(buf, 1)?[0]);
    take(buf, n)
}

impl Control {
    pub fn encode(&self) -> Result<Vec<u8>, ControlError> {
        let mut out = Vec::new();
        match self {
            Control::Peer {
                proto,
                public_key,
                endpoint,
                allowed_ips,
            } => {
                out.push(SUBTYPE_PEER);
                out.push(*proto as u8);
                push_bytes(&mut out, public_key)?;
                push_bytes(&mut out, endpoint.as_bytes())?;
                push_bytes(&mut out, allowed_ips.as_bytes())?;
            }
            Control::Psk {
                purpose,
                peer_a,
                peer_b,
                offset,
            } => {
                out.push(SUBTYPE_PSK);
                out.push(*purpose as u8);
                out.push(*peer_a);
                out.push(*peer_b);
                out.extend_from_slice(&offset.to_le_bytes());
            }
            Control::Rotate {
                purpose,
                peer_a,
                peer_b,
                not_after,
            } => {
                out.push(SUBTYPE_ROTATE);
                out.push(*purpose as u8);
                out.push(*peer_a);
                out.push(*peer_b);
                out.extend_from_slice(&not_after.to_le_bytes());
            }
            Control::SliceTransfer { member } => {
                out.push(SUBTYPE_SLICE_TRANSFER);
                out.push(*member);
            }
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Control, ControlError> {
        let mut buf = bytes;
        let subtype = take(&mut buf, 1)?[0];
        let c = match subtype {
            SUBTYPE_PEER => {
                let proto = Proto::try_from(take(&mut buf, 1)?[0])?;
                let public_key = take_var(&mut buf)?.to_vec();
                let endpoint = core::str::from_utf8(take_var(&mut buf)?)
                    .map_err(|_| ControlError::Utf8)?
                    .into();
                let allowed_ips = core::str::from_utf8(take_var(&mut buf)?)
                    .map_err(|_| ControlError::Utf8)?
                    .into();
                Control::Peer {
                    proto,
                    public_key,
                    endpoint,
                    allowed_ips,
                }
            }
            SUBTYPE_PSK => {
                let f = take(&mut buf, 7)?;
                Control::Psk {
                    purpose: Purpose::try_from(f[0])?,
                    peer_a: f[1],
                    peer_b: f[2],
                    offset: u32::from_le_bytes([f[3], f[4], f[5], f[6]]),
                }
            }
            SUBTYPE_ROTATE => {
                let f = take(&mut buf, 11)?;
                let mut t = [0u8; 8];
                t.copy_from_slice(&f[3..11]);
                Control::Rotate {
                    purpose: Purpose::try_from(f[0])?,
                    peer_a: f[1],
                    peer_b: f[2],
                    not_after: u64::from_le_bytes(t),
                }
            }
            SUBTYPE_SLICE_TRANSFER => Control::SliceTransfer {
                member: take(&mut buf, 1)?[0],
            },
            other => return Err(ControlError::Subtype(other)),
        };
        if !buf.is_empty() {
            return Err(ControlError::Length);
        }
        Ok(c)
    }
}
