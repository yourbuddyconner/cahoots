//! Pure functions: key derivation, header codec, seal/open, bucket padding.

use alloc::vec::Vec;
use cahoots_core::circle_id::CircleId;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::ChaCha20Poly1305;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

pub const CHAT_VERSION: u8 = 2;
pub const EPOCH_ID_LEN: usize = 5;
pub const HEADER_LEN: usize = 1 + 16 + EPOCH_ID_LEN + 1 + 4 + 4 + 1;
pub const TAG_LEN: usize = 16;
pub const KEY_LEN: usize = 32;

pub const CHAIN_DOMAIN: &[u8] = b"cahoots/chat/chain/v1";
pub const MSG_DOMAIN: &[u8] = b"cahoots/chat/msg/v1";
pub const NEXT_DOMAIN: &[u8] = b"cahoots/chat/next/v1";

pub type Key = Zeroizing<[u8; KEY_LEN]>;

/// The slice byte that marks a root epoch. Any other value is a retired pad epoch.
pub const ROOT_EPOCH_SLICE: u8 = 0xFF;

/// The public name of an epoch, 5 bytes on the wire. A root epoch `n` is `slice = 0xFF`,
/// `offset = n`. Pad epochs, where the two fields were a pad address, are retired.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EpochId {
    pub slice: u8,
    pub offset: u32,
}

impl EpochId {
    /// Root epoch `n`.
    pub fn root(n: u32) -> EpochId {
        EpochId {
            slice: ROOT_EPOCH_SLICE,
            offset: n,
        }
    }
    /// The root epoch number, or `None` for a pad epoch id.
    pub fn number(&self) -> Option<u32> {
        (self.slice == ROOT_EPOCH_SLICE).then_some(self.offset)
    }
    pub fn encode(&self) -> [u8; EPOCH_ID_LEN] {
        let mut b = [0u8; EPOCH_ID_LEN];
        b[0] = self.slice;
        b[1..5].copy_from_slice(&self.offset.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Self {
        EpochId {
            slice: b[0],
            offset: u32::from_le_bytes([b[1], b[2], b[3], b[4]]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    Text = 0,
    Control = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub circle_id: CircleId,
    pub epoch: EpochId,
    pub sender: u8,
    pub counter: u32,
    pub len: u32,
    pub msg_type: MessageType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatError {
    Version,
    Type,
    /// Wire bytes too short, or body length does not match the header.
    Length,
    Tag,
    WrongCircle,
    /// `sender` is not a member index of this circle.
    BadSender,
    /// This counter was already consumed.
    Replay,
    /// More than `MAX_SKIP` messages ahead of the chain.
    TooFarAhead,
    /// No state for this epoch, and it cannot be made again: the epoch was dropped to make
    /// room for newer ones, or is older than one that was. Epoch 0 is never used.
    UnknownEpoch,
    /// This session has not read the relay to the end since it was made or imported, so it
    /// may not know its own sending position. Call `mark_synced` after a full read.
    NotSynced,
    /// The epoch id names a pad epoch (a slice other than `0xFF`). Pad epochs are retired.
    PadEpoch,
    /// Malformed exported state.
    State,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0] = self.version;
        h[1..17].copy_from_slice(&self.circle_id);
        h[17..22].copy_from_slice(&self.epoch.encode());
        h[22] = self.sender;
        h[23..27].copy_from_slice(&self.counter.to_le_bytes());
        h[27..31].copy_from_slice(&self.len.to_le_bytes());
        h[31] = self.msg_type as u8;
        h
    }
}

pub fn parse_header(b: &[u8]) -> Result<Header, ChatError> {
    if b.len() < HEADER_LEN {
        return Err(ChatError::Length);
    }
    if b[0] != CHAT_VERSION {
        return Err(ChatError::Version);
    }
    let msg_type = match b[31] {
        0 => MessageType::Text,
        1 => MessageType::Control,
        _ => return Err(ChatError::Type),
    };
    let mut circle_id = [0u8; 16];
    circle_id.copy_from_slice(&b[1..17]);
    Ok(Header {
        version: b[0],
        circle_id,
        epoch: EpochId::decode(&b[17..22]),
        sender: b[22],
        counter: u32::from_le_bytes([b[23], b[24], b[25], b[26]]),
        len: u32::from_le_bytes([b[27], b[28], b[29], b[30]]),
        msg_type,
    })
}

/// Split a wire message into its header and body (ciphertext plus AEAD tag).
pub fn parse(wire: &[u8]) -> Result<(Header, &[u8]), ChatError> {
    let h = parse_header(wire)?;
    let body = &wire[HEADER_LEN..];
    if body.len() != (h.len as usize).saturating_add(TAG_LEN) {
        return Err(ChatError::Length);
    }
    Ok((h, body))
}

/// The key of root epoch `n`: `HMAC-SHA256(root, "chat" || circle_id || n as u64 BE)`,
/// which is `cahoots_core::root::chat_key`.
pub fn epoch_key(root: &[u8; 32], circle_id: &CircleId, n: u32) -> Key {
    Zeroizing::new(cahoots_core::root::chat_key(root, circle_id, u64::from(n)))
}

fn expand(prk: &[u8; 32], info: &[u8]) -> Key {
    let hk = Hkdf::<Sha256>::from_prk(prk).expect("32-byte PRK");
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(info, out.as_mut())
        .expect("32 bytes is a valid HKDF length");
    out
}

/// `chain_0[sender] = HKDF-Expand(epoch_key, domain || sender)`.
pub fn chain_init(epoch_key: &[u8; 32], sender: u8) -> Key {
    let mut info = Vec::with_capacity(CHAIN_DOMAIN.len() + 1);
    info.extend_from_slice(CHAIN_DOMAIN);
    info.push(sender);
    expand(epoch_key, &info)
}

/// Advance a chain one step: returns `(msg_key, next_chain)`. The caller owns
/// the old chain and must drop it; `Key` zeroizes on drop.
pub fn step(chain: &[u8; 32]) -> (Key, Key) {
    (expand(chain, MSG_DOMAIN), expand(chain, NEXT_DOMAIN))
}

pub fn nonce(sender: u8, counter: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[0] = sender;
    n[1..5].copy_from_slice(&counter.to_le_bytes());
    n
}

/// `header || ChaCha20-Poly1305(msg_key, nonce, aad = header, plaintext)`.
pub fn seal(header: &Header, msg_key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let h = header.encode();
    let cipher = ChaCha20Poly1305::new(msg_key.into());
    let ct = cipher
        .encrypt(
            &nonce(header.sender, header.counter).into(),
            Payload {
                msg: plaintext,
                aad: &h,
            },
        )
        .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory inputs");
    let mut out = Vec::with_capacity(HEADER_LEN + ct.len());
    out.extend_from_slice(&h);
    out.extend_from_slice(&ct);
    out
}

/// Verify and decrypt `body` (ciphertext plus tag) under `msg_key`.
pub fn open(header: &Header, msg_key: &[u8; 32], body: &[u8]) -> Result<Vec<u8>, ChatError> {
    if body.len() != (header.len as usize).saturating_add(TAG_LEN) {
        return Err(ChatError::Length);
    }
    let h = header.encode();
    let cipher = ChaCha20Poly1305::new(msg_key.into());
    cipher
        .decrypt(
            &nonce(header.sender, header.counter).into(),
            Payload { msg: body, aad: &h },
        )
        .map_err(|_| ChatError::Tag)
}

/// Chat plaintext buckets. The last two bytes carry the plaintext length (u16 LE).
pub const BUCKETS: [usize; 4] = [64, 256, 1024, 4096];

pub fn bucket_pad(plaintext: &[u8]) -> Option<Vec<u8>> {
    let size = *BUCKETS.iter().find(|&&b| plaintext.len() + 2 <= b)?;
    let mut out = alloc::vec![0u8; size];
    out[..plaintext.len()].copy_from_slice(plaintext);
    out[size - 2..].copy_from_slice(&(plaintext.len() as u16).to_le_bytes());
    Some(out)
}

pub fn bucket_unpad(padded: &[u8]) -> Option<&[u8]> {
    let size = padded.len();
    if !BUCKETS.contains(&size) {
        return None;
    }
    let len = usize::from(u16::from_le_bytes([padded[size - 2], padded[size - 1]]));
    (len + 2 <= size).then(|| &padded[..len])
}
