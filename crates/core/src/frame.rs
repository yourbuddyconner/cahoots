//! Touch-link framing (handoff 3.4): `type (u8) | seq (u32 LE) | len (u32 LE) | payload | CRC-32`.
//! The CRC catches transmission errors only. Commitments and check words provide integrity.

use alloc::vec::Vec;
use crc::{Crc, CRC_32_ISO_HDLC};

pub const HEADER_LEN: usize = 1 + 4 + 4;
pub const CRC_LEN: usize = 4;
/// The longest payload [`decode`] accepts: 2 MiB. The largest real frame is a ceremony
/// reveal, which carries the whole pad sealed to every other member: the pad plus about
/// 10 KiB for eight members. The ESP32-S3's pad partition is 1 MiB and the app's largest pad
/// is 256 KiB, so every real reveal fits. A header that claims more is not a frame.
pub const MAX_PAYLOAD: usize = 2 * 1024 * 1024;
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    pub kind: u8,
    pub seq: u32,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// More bytes are needed before the frame can be decoded.
    Incomplete,
    /// The bytes at the front are not a frame: the CRC does not match, or the header claims
    /// a payload longer than the limit. Drop one byte and decode again to resynchronise.
    Crc,
}

pub fn encode(f: &Frame<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + f.payload.len() + CRC_LEN);
    out.push(f.kind);
    out.extend_from_slice(&f.seq.to_le_bytes());
    out.extend_from_slice(&(f.payload.len() as u32).to_le_bytes());
    out.extend_from_slice(f.payload);
    let crc = CRC32.checksum(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Decode one frame from the front of `buf`. Returns the frame and the bytes consumed.
pub fn decode(buf: &[u8]) -> Result<(Frame<'_>, usize), FrameError> {
    decode_max(buf, MAX_PAYLOAD)
}

/// [`decode`] with a smaller payload limit, for a reader that expects only short frames.
/// A header claiming more than `max_payload` (or more than [`MAX_PAYLOAD`]) is
/// [`FrameError::Crc`], so the reader drops a byte at once instead of waiting for bytes
/// that will never come.
pub fn decode_max(buf: &[u8], max_payload: usize) -> Result<(Frame<'_>, usize), FrameError> {
    if buf.len() < HEADER_LEN {
        return Err(FrameError::Incomplete);
    }
    let len = usize::try_from(u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]))
        .map_err(|_| FrameError::Crc)?;
    if len > max_payload.min(MAX_PAYLOAD) {
        return Err(FrameError::Crc);
    }
    let total = HEADER_LEN
        .checked_add(len)
        .and_then(|t| t.checked_add(CRC_LEN))
        .ok_or(FrameError::Crc)?;
    if buf.len() < total {
        return Err(FrameError::Incomplete);
    }
    let body = &buf[..HEADER_LEN + len];
    let want = u32::from_le_bytes([
        buf[total - 4],
        buf[total - 3],
        buf[total - 2],
        buf[total - 1],
    ]);
    if CRC32.checksum(body) != want {
        return Err(FrameError::Crc);
    }
    Ok((
        Frame {
            kind: buf[0],
            seq: u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]),
            payload: &buf[HEADER_LEN..HEADER_LEN + len],
        },
        total,
    ))
}
