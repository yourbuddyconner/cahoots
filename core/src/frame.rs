//! Touch-link framing (handoff 3.4): `type (u8) | seq (u32 LE) | len (u32 LE) | payload | CRC-32`.
//! The CRC catches transmission errors only. Commitments and check words provide integrity.

use alloc::vec::Vec;
use crc::{Crc, CRC_32_ISO_HDLC};

pub const HEADER_LEN: usize = 1 + 4 + 4;
pub const CRC_LEN: usize = 4;
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
    if buf.len() < HEADER_LEN {
        return Err(FrameError::Incomplete);
    }
    let len = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]) as usize;
    let total = HEADER_LEN + len + CRC_LEN;
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
