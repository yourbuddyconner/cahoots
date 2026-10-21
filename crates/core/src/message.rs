//! Message format (handoff 3.5). Pure functions over a caller-supplied pad range.
//! The [`crate::pad::Pad`] type owns spend ordering; this module never touches storage.
//!
//! ```text
//! header  = version (u8 = 1) | circle_id (16 B) | slice (u8) | offset o (u32) | len (u16) | type (u8)
//! key     = PAD_slice[o .. o+32]
//! stream  = PAD_slice[o+32 .. o+32+len]
//! ct      = plaintext XOR stream
//! tag     = Poly1305(key, header || ct)
//! message = header || ct || tag
//! ```

use crate::circle_id::CircleId;
use alloc::vec::Vec;
use poly1305::universal_hash::KeyInit;
use poly1305::Poly1305;
use subtle::ConstantTimeEq;

pub const MESSAGE_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 1 + 16 + 1 + 4 + 2 + 1;
pub const KEY_LEN: usize = 32;
pub const TAG_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MessageType {
    Text = 0,
    /// Agent action approval. Plaintext is the SHA-256 of the pending action.
    Approve = 1,
    Vouch = 2,
    Control = 3,
}

impl TryFrom<u8> for MessageType {
    type Error = MessageError;
    fn try_from(v: u8) -> Result<Self, MessageError> {
        Ok(match v {
            0 => MessageType::Text,
            1 => MessageType::Approve,
            2 => MessageType::Vouch,
            3 => MessageType::Control,
            _ => return Err(MessageError::Type),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub circle_id: CircleId,
    pub slice: u8,
    pub offset: u32,
    pub len: u16,
    pub msg_type: MessageType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageError {
    Version,
    Type,
    /// Wire bytes are too short or do not match the header's `len`.
    Length,
    /// Plaintext longer than `u16::MAX`.
    TooLong,
    /// The pad range is not exactly `32 + len` bytes.
    PadLength,
    Tag,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0] = self.version;
        h[1..17].copy_from_slice(&self.circle_id);
        h[17] = self.slice;
        h[18..22].copy_from_slice(&self.offset.to_le_bytes());
        h[22..24].copy_from_slice(&self.len.to_le_bytes());
        h[24] = self.msg_type as u8;
        h
    }

    /// Total pad bytes this message spends: key plus stream.
    pub fn spend_len(&self) -> u32 {
        KEY_LEN as u32 + u32::from(self.len)
    }
}

fn tag(key: &[u8], data: &[u8]) -> [u8; TAG_LEN] {
    let key: [u8; KEY_LEN] = key.try_into().expect("key is 32 bytes");
    Poly1305::new(&key.into()).compute_unpadded(data).into()
}

/// Build `header || ct || tag`. `pad_range` MUST be exactly `32 + plaintext.len()` bytes
/// and MUST already be marked spent and erased by the caller.
pub fn seal(
    circle_id: &CircleId,
    slice: u8,
    offset: u32,
    msg_type: MessageType,
    plaintext: &[u8],
    pad_range: &[u8],
) -> Result<Vec<u8>, MessageError> {
    let len = u16::try_from(plaintext.len()).map_err(|_| MessageError::TooLong)?;
    if pad_range.len() != KEY_LEN + plaintext.len() {
        return Err(MessageError::PadLength);
    }
    let header = Header {
        version: MESSAGE_VERSION,
        circle_id: *circle_id,
        slice,
        offset,
        len,
        msg_type,
    };
    let (key, stream) = pad_range.split_at(KEY_LEN);
    let mut out = Vec::with_capacity(HEADER_LEN + plaintext.len() + TAG_LEN);
    out.extend_from_slice(&header.encode());
    out.extend(plaintext.iter().zip(stream).map(|(p, s)| p ^ s));
    let t = tag(key, &out);
    out.extend_from_slice(&t);
    Ok(out)
}

/// Split wire bytes into `(header, ciphertext, tag)` without touching any pad material.
pub fn parse(wire: &[u8]) -> Result<(Header, &[u8], &[u8]), MessageError> {
    if wire.len() < HEADER_LEN + TAG_LEN {
        return Err(MessageError::Length);
    }
    if wire[0] != MESSAGE_VERSION {
        return Err(MessageError::Version);
    }
    let mut circle_id = [0u8; 16];
    circle_id.copy_from_slice(&wire[1..17]);
    let len = u16::from_le_bytes([wire[22], wire[23]]);
    let msg_type = MessageType::try_from(wire[24])?;
    if wire.len() != HEADER_LEN + usize::from(len) + TAG_LEN {
        return Err(MessageError::Length);
    }
    let header = Header {
        version: wire[0],
        circle_id,
        slice: wire[17],
        offset: u32::from_le_bytes([wire[18], wire[19], wire[20], wire[21]]),
        len,
        msg_type,
    };
    let ct = &wire[HEADER_LEN..HEADER_LEN + usize::from(len)];
    let t = &wire[HEADER_LEN + usize::from(len)..];
    Ok((header, ct, t))
}

/// Verify the tag in constant time, then decrypt. `pad_range` is `32 + len` bytes.
pub fn open(
    header: &Header,
    ciphertext: &[u8],
    wire_tag: &[u8],
    pad_range: &[u8],
) -> Result<Vec<u8>, MessageError> {
    if ciphertext.len() != usize::from(header.len) || wire_tag.len() != TAG_LEN {
        return Err(MessageError::Length);
    }
    if pad_range.len() != KEY_LEN + ciphertext.len() {
        return Err(MessageError::PadLength);
    }
    let (key, stream) = pad_range.split_at(KEY_LEN);
    let mut authed = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    authed.extend_from_slice(&header.encode());
    authed.extend_from_slice(ciphertext);
    let expect = tag(key, &authed);
    if !bool::from(expect.ct_eq(wire_tag)) {
        return Err(MessageError::Tag);
    }
    Ok(ciphertext.iter().zip(stream).map(|(c, s)| c ^ s).collect())
}
