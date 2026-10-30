//! What the station checks before it accepts a message. It cannot verify any tag, because
//! it holds no pad. It can parse the public header, match the circle, and refuse pure-pad
//! address reuse. Chat epochs are reused legitimately, so chat messages are never refused
//! for reuse (see `docs/chat.md` section 7).

use cahoots_core::circle_id::CircleId;
use cahoots_core::message::{self, MessageError};

/// Largest accepted body, in bytes.
pub const MAX_BODY: usize = 64 * 1024;

pub const CHAT_VERSION: u8 = 2;
/// `version | circle_id | epoch_id (5) | sender | counter u32 | len u32 | type`.
pub const CHAT_HEADER_LEN: usize = 1 + 16 + 5 + 1 + 4 + 4 + 1;
pub const AEAD_TAG_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("body larger than {MAX_BODY} bytes")]
    TooLarge,
    #[error("unknown message version")]
    Version,
    #[error("length fields do not match the body")]
    Length,
    #[error("message names a different circle than the path")]
    CircleMismatch,
    #[error("malformed pure-pad message: {0:?}")]
    Message(MessageError),
}

/// A parsed, rule-checked body ready to append.
#[derive(Clone, Debug)]
pub struct Submission {
    circle_id: CircleId,
    bytes: Vec<u8>,
    /// `(slice, start, end)` for pure-pad messages. `None` for chat.
    pad_range: Option<(u8, u32, u32)>,
}

impl Submission {
    pub fn parse(path_circle: &CircleId, body: &[u8]) -> Result<Self, RuleError> {
        if body.len() > MAX_BODY {
            return Err(RuleError::TooLarge);
        }
        let Some(&version) = body.first() else {
            return Err(RuleError::Length);
        };
        let (circle_id, pad_range) = match version {
            message::MESSAGE_VERSION => {
                let (hdr, _, _) = message::parse(body).map_err(|e| match e {
                    MessageError::Length => RuleError::Length,
                    MessageError::Version => RuleError::Version,
                    other => RuleError::Message(other),
                })?;
                let end = hdr
                    .offset
                    .checked_add(hdr.spend_len())
                    .ok_or(RuleError::Length)?;
                (hdr.circle_id, Some((hdr.slice, hdr.offset, end)))
            }
            CHAT_VERSION => {
                if body.len() < CHAT_HEADER_LEN {
                    return Err(RuleError::Length);
                }
                let mut cid = [0u8; 16];
                cid.copy_from_slice(&body[1..17]);
                let len = u32::from_le_bytes([body[27], body[28], body[29], body[30]]) as usize;
                let want = CHAT_HEADER_LEN
                    .checked_add(len)
                    .and_then(|n| n.checked_add(AEAD_TAG_LEN))
                    .ok_or(RuleError::Length)?;
                if body.len() != want {
                    return Err(RuleError::Length);
                }
                (cid, None)
            }
            _ => return Err(RuleError::Version),
        };
        if &circle_id != path_circle {
            return Err(RuleError::CircleMismatch);
        }
        Ok(Submission {
            circle_id,
            bytes: body.to_vec(),
            pad_range,
        })
    }

    pub fn circle_id(&self) -> CircleId {
        self.circle_id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn pad_range(&self) -> Option<(u8, u32, u32)> {
        self.pad_range
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
