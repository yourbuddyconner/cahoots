//! The version 4 message (channels spec §5).
//!
//! ```text
//! header  = version (u8 = 4) | channel_id (16) | epoch_id (20) | kid (32) | counter (u32 LE)
//!           | len (u32 LE) | type (u8)
//! nonce   = counter (u32 LE) | 0x00 * 8
//! ct      = ChaCha20-Poly1305(msg_key, nonce, aad = header, plaintext)
//! body    = header || ct
//! message = body || COSE_Sign1(payload = approvals, kid = kid,
//!                              external AAD = "cahoots/channel-msg/v1" || body)
//! ```
//!
//! The approvals are a canonical CBOR array of byte strings, outside the ciphertext so a gating
//! relay can count them. The signature covers them and the body.

use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::ChaCha20Poly1305;
use minicbor::Decoder;
use sha2::{Digest, Sha256};

use crate::cbor::{array_len, canonical, encode_with};
use crate::keys::{kid_of, EpochId, EPOCH_ID_LEN};
use crate::{ChannelError, ChannelId};

pub const VERSION: u8 = 4;
pub const HEADER_LEN: usize = 78;
/// The external AAD prefix of a message's COSE_Sign1.
pub const DOMAIN: &[u8] = b"cahoots/channel-msg/v1";

const TAG_LEN: usize = 16;
const M: ChannelError = ChannelError::Malformed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Text = 0,
    Control = 1,
    Announcement = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub channel_id: ChannelId,
    pub epoch_id: EpochId,
    /// The sender's key id, `SHA-256` of its member key.
    pub kid: [u8; 32],
    pub counter: u32,
    /// The plaintext length; the ciphertext is 16 bytes longer.
    pub len: u32,
    pub msg_type: MessageType,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0] = VERSION;
        h[1..17].copy_from_slice(&self.channel_id);
        h[17..37].copy_from_slice(&self.epoch_id.encode());
        h[37..69].copy_from_slice(&self.kid);
        h[69..73].copy_from_slice(&self.counter.to_le_bytes());
        h[73..77].copy_from_slice(&self.len.to_le_bytes());
        h[77] = self.msg_type as u8;
        h
    }

    /// Parse the first [`HEADER_LEN`] bytes. Too short, another version or an unknown type is
    /// [`ChannelError::Malformed`]; an epoch id naming another channel than the header's is
    /// [`ChannelError::WrongChannel`].
    pub fn parse(b: &[u8]) -> Result<Header, ChannelError> {
        let b: &[u8; HEADER_LEN] = b.get(..HEADER_LEN).ok_or(M)?.try_into().map_err(|_| M)?;
        if b[0] != VERSION {
            return Err(M);
        }
        let msg_type = match b[77] {
            0 => MessageType::Text,
            1 => MessageType::Control,
            2 => MessageType::Announcement,
            _ => return Err(M),
        };
        let channel_id: ChannelId = b[1..17].try_into().map_err(|_| M)?;
        let epoch_id = EpochId::decode(&b[17..17 + EPOCH_ID_LEN]).ok_or(M)?;
        if epoch_id.channel_id != channel_id {
            return Err(ChannelError::WrongChannel);
        }
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Ok(Header {
            channel_id,
            epoch_id,
            kid: b[37..69].try_into().map_err(|_| M)?,
            counter: u32_at(69),
            len: u32_at(73),
            msg_type,
        })
    }
}

/// `counter (u32 LE) || 0x00 * 8`.
pub fn nonce(counter: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(&counter.to_le_bytes());
    n
}

fn signing_aad(body: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(DOMAIN.len() + body.len());
    aad.extend_from_slice(DOMAIN);
    aad.extend_from_slice(body);
    aad
}

fn encode_approvals(approvals: &[Vec<u8>]) -> Vec<u8> {
    encode_with(|e| {
        e.array(approvals.len() as u64)?;
        for a in approvals {
            e.bytes(a)?;
        }
        Ok(())
    })
}

fn decode_approvals(bytes: &[u8]) -> Result<Vec<Vec<u8>>, ChannelError> {
    let mut d = Decoder::new(bytes);
    let n = array_len(&mut d)?;
    let mut approvals = Vec::new();
    for _ in 0..n {
        approvals.push(d.bytes().map_err(|_| M)?.to_vec());
    }
    if d.position() != bytes.len() {
        return Err(M);
    }
    let encoded = encode_approvals(&approvals);
    canonical(approvals, encoded, bytes)
}

/// Encrypt `plaintext` under `msg_key` with the header as AAD, then sign the body and the
/// approvals with the sender's member key. `header.len` must be `plaintext.len()`.
pub fn seal_sign<S: SeatSigner>(
    header: &Header,
    msg_key: &[u8; 32],
    plaintext: &[u8],
    approvals: &[Vec<u8>],
    signer: S,
) -> Vec<u8> {
    debug_assert_eq!(header.len as usize, plaintext.len());
    let h = header.encode();
    let ct = ChaCha20Poly1305::new(msg_key.into())
        .encrypt(
            &nonce(header.counter).into(),
            Payload {
                msg: plaintext,
                aad: &h,
            },
        )
        .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory inputs");
    let mut wire = Vec::with_capacity(HEADER_LEN + ct.len());
    wire.extend_from_slice(&h);
    wire.extend_from_slice(&ct);
    let signature = cose::sign1(
        signer,
        &header.kid,
        &encode_approvals(approvals),
        &signing_aad(&wire),
        false,
    );
    wire.extend_from_slice(&signature);
    wire
}

/// Split a message into its body, its COSE_Sign1 and its parsed header, without checking the
/// signature. The body is the header and `header.len + 16` bytes of ciphertext; a message with
/// nothing after the body is [`ChannelError::Malformed`].
pub fn split(wire: &[u8]) -> Result<(&[u8], &[u8], Header), ChannelError> {
    let header = Header::parse(wire)?;
    let body_len = usize::try_from(header.len)
        .ok()
        .and_then(|n| n.checked_add(HEADER_LEN + TAG_LEN))
        .ok_or(M)?;
    if wire.len() <= body_len {
        return Err(M);
    }
    let (body, signature) = wire.split_at(body_len);
    Ok((body, signature, header))
}

/// What [`verify_signature`] returns: the header, the ciphertext with its tag, and the approvals.
pub type Verified<'w> = (Header, &'w [u8], Vec<Vec<u8>>);

/// Check the signature under `key`: the COSE kid and the header's kid are `kid_of(key)`, and the
/// signature verifies over the approvals with `DOMAIN || body`. Returns the header, the
/// ciphertext and the approvals. Any mismatch or bad signature is [`ChannelError::Signature`].
pub fn verify_signature<'w>(
    wire: &'w [u8],
    key: &SeatPublic,
) -> Result<Verified<'w>, ChannelError> {
    let (body, signature, header) = split(wire)?;
    let kid = kid_of(key);
    let cose_kid = cose::kid1(signature).map_err(ChannelError::Cose)?;
    if cose_kid[..] != kid || header.kid != kid {
        return Err(ChannelError::Signature);
    }
    let payload = cose::verify1(key, signature, None, &signing_aad(body))
        .map_err(|_| ChannelError::Signature)?;
    let approvals = decode_approvals(&payload)?;
    Ok((header, &body[HEADER_LEN..], approvals))
}

/// Decrypt the ciphertext (with its tag) under `msg_key`. A length other than `header.len + 16`
/// is [`ChannelError::Malformed`]; a tag that does not verify is [`ChannelError::Tag`].
pub fn open(header: &Header, msg_key: &[u8; 32], ct: &[u8]) -> Result<Vec<u8>, ChannelError> {
    if usize::try_from(header.len)
        .ok()
        .and_then(|n| n.checked_add(TAG_LEN))
        != Some(ct.len())
    {
        return Err(M);
    }
    ChaCha20Poly1305::new(msg_key.into())
        .decrypt(
            &nonce(header.counter).into(),
            Payload {
                msg: ct,
                aad: &header.encode(),
            },
        )
        .map_err(|_| ChannelError::Tag)
}

/// `SHA-256(body)`: the `request_digest` an announcement's approvals sign.
pub fn body_digest(body: &[u8]) -> [u8; 32] {
    Sha256::digest(body).into()
}
