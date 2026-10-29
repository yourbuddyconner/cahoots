//! The request and response messages, CBOR (RFC 8949), array-encoded.
//!
//! Each message is a CBOR array of the variant index and an array of that variant's
//! fields in order. Keys, circle ids and spent bytes are byte strings. A `PskHandle` is its
//! 21-byte `to_bytes` form as one byte string.

use crate::contract::{CircleInfo, PadError, PskHandle};
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_chip::Attested;
use cahoots_core::CircleId;
use minicbor::decode::Error as DecodeError;
use minicbor::{Decode, Decoder, Encode, Encoder};

/// Frame kind for a request.
pub const KIND_REQUEST: u8 = 40;
/// Frame kind for a response.
pub const KIND_RESPONSE: u8 = 41;
/// The longest request payload a custodian reads, in bytes. A request frame that claims more
/// is not a request. Only an `SshSign` request, which can carry the text of a commit, is
/// allowed to be this long.
pub const MAX_REQUEST_LEN: usize = 16 * 1024;
/// The longest any other request may be once decoded. A request is a few fields and a short
/// context label, far under this; `serve` refuses a longer one as it refuses an oversize frame.
pub const MAX_SMALL_REQUEST_LEN: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum Request {
    #[n(0)]
    Identify,
    #[n(1)]
    Psk {
        #[cbor(n(0), with = "minicbor::bytes")]
        circle: CircleId,
        #[cbor(n(1), with = "minicbor::bytes")]
        context: Vec<u8>,
    },
    #[n(2)]
    PskFor {
        #[n(0)]
        handle: PskHandle,
        #[cbor(n(1), with = "minicbor::bytes")]
        context: Vec<u8>,
    },
    #[n(3)]
    Spend {
        #[cbor(n(0), with = "minicbor::bytes")]
        circle: CircleId,
        #[n(1)]
        len: u32,
    },
    #[n(4)]
    SpendFor {
        #[n(0)]
        handle: PskHandle,
        #[n(1)]
        len: u32,
    },
    #[n(5)]
    Remaining {
        #[cbor(n(0), with = "minicbor::bytes")]
        circle: CircleId,
    },
    /// Something a badge does with the keys of its seat in `circle`.
    #[n(6)]
    Chip {
        #[cbor(n(0), with = "minicbor::bytes")]
        circle: CircleId,
        #[n(1)]
        op: ChipRequest,
    },
}

/// The operations of the `Chip` request family.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum ChipRequest {
    /// The seat's public key and what holds it.
    #[n(0)]
    Describe,
    /// The chip's attestation of the seat key.
    #[n(1)]
    Attest {
        #[cbor(n(0), with = "minicbor::bytes")]
        freshness: [u8; 16],
    },
    /// Sign the bytes OpenSSH asks an agent to sign.
    #[n(2)]
    SshSign {
        #[cbor(n(0), with = "minicbor::bytes")]
        data: Vec<u8>,
        #[n(1)]
        host: Option<SshHost>,
        /// For git: the text that `data`'s hash covers.
        #[cbor(n(2), with = "minicbor::bytes")]
        content: Option<Vec<u8>>,
    },
    /// Sign a contact card.
    #[n(3)]
    Card {
        #[cbor(n(0), with = "minicbor::bytes")]
        card: Vec<u8>,
    },
    /// Sign a grant.
    #[n(4)]
    GrantSign {
        #[cbor(n(0), with = "minicbor::bytes")]
        grant: Vec<u8>,
    },
    /// Release the key that seals things for the circle under `label`.
    #[n(5)]
    SealKey {
        #[n(0)]
        label: String,
    },
}

impl Request {
    /// Whether this request, `encoded_len` bytes long on the wire, is within the length its
    /// kind allows: [`MAX_REQUEST_LEN`] for an `SshSign`, [`MAX_SMALL_REQUEST_LEN`] for
    /// anything else. A badge checks this before it asks its holder anything.
    pub fn length_ok(&self, encoded_len: usize) -> bool {
        let long_ok = matches!(
            self,
            Request::Chip {
                op: ChipRequest::SshSign { .. },
                ..
            }
        );
        encoded_len
            <= if long_ok {
                MAX_REQUEST_LEN
            } else {
                MAX_SMALL_REQUEST_LEN
            }
    }
}

impl ChipRequest {
    /// True when the badge asks its holder before answering.
    pub fn needs_consent(&self) -> bool {
        !matches!(self, ChipRequest::Describe | ChipRequest::Attest { .. })
    }
}

/// What the laptop learned from OpenSSH's `session-bind@openssh.com` message, plus the
/// name it found for the host key in `known_hosts`. None of it is trusted until the badge
/// checks it.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct SshHost {
    #[cbor(n(0), with = "minicbor::bytes")]
    pub host_key: Vec<u8>,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub session_id: Vec<u8>,
    #[cbor(n(2), with = "minicbor::bytes")]
    pub host_sig: Vec<u8>,
    #[n(3)]
    pub name: String,
    #[n(4)]
    pub user: String,
}

/// The answers of the `Chip` request family.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum ChipResponse {
    #[n(0)]
    Description {
        #[cbor(n(0), with = "minicbor::bytes")]
        public: Vec<u8>,
        #[n(1)]
        curve: u8,
        #[n(2)]
        in_chip: bool,
        #[n(3)]
        can_attest: bool,
    },
    #[n(1)]
    Attested {
        #[n(0)]
        attested: Attested,
    },
    /// SSH wire format for `SshSign`; COSE_Sign1 bytes for `Card` and `GrantSign`.
    #[n(2)]
    Signature {
        #[cbor(n(0), with = "minicbor::bytes")]
        signature: Vec<u8>,
    },
    #[n(3)]
    Key {
        #[cbor(n(0), with = "minicbor::bytes")]
        key: [u8; 32],
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum Response {
    #[n(0)]
    Identity {
        #[n(0)]
        circles: Vec<CircleInfo>,
        #[n(1)]
        tier: u8,
        #[n(2)]
        name: String,
        /// True when the badge keeps its seat keys in a chip.
        #[n(3)]
        chip: bool,
    },
    #[n(1)]
    Psk {
        #[n(0)]
        handle: PskHandle,
        #[cbor(n(1), with = "minicbor::bytes")]
        key: [u8; 32],
    },
    #[n(2)]
    Key {
        #[cbor(n(0), with = "minicbor::bytes")]
        key: [u8; 32],
    },
    /// The answer to both `Spend` and `SpendFor`.
    #[n(3)]
    Bytes {
        #[n(0)]
        handle: PskHandle,
        #[cbor(n(1), with = "minicbor::bytes")]
        bytes: Vec<u8>,
    },
    #[n(4)]
    Remaining {
        #[n(0)]
        bytes: u64,
        #[n(1)]
        low: bool,
    },
    #[n(5)]
    Error {
        #[n(0)]
        code: ErrorCode,
    },
    #[n(6)]
    Chip {
        #[n(0)]
        result: ChipResponse,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum ErrorCode {
    #[n(0)]
    UnknownCircle,
    #[n(1)]
    Exhausted,
    #[n(2)]
    AlreadySpent,
    #[n(3)]
    OutOfRange,
    #[n(4)]
    Config,
    #[n(5)]
    Storage,
    #[n(6)]
    Malformed,
    #[n(7)]
    Denied,
    #[n(8)]
    Busy,
    /// The badge has no chip to do this with.
    #[n(9)]
    NoChip,
    /// The chip failed or refused.
    #[n(10)]
    ChipError,
    /// The bytes to sign are not something the badge will sign.
    #[n(11)]
    Unparseable,
}

impl From<PadError> for ErrorCode {
    fn from(e: PadError) -> Self {
        match e {
            PadError::UnknownCircle => ErrorCode::UnknownCircle,
            PadError::Exhausted => ErrorCode::Exhausted,
            PadError::AlreadySpent => ErrorCode::AlreadySpent,
            PadError::OutOfRange => ErrorCode::OutOfRange,
            PadError::Config => ErrorCode::Config,
            PadError::Storage(_) => ErrorCode::Storage,
        }
    }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        minicbor::to_vec(self).expect("encoding into a Vec cannot fail")
    }

    /// Strict: the input must be exactly one canonical encoding. Trailing bytes, extra
    /// fields (such as another message type's) and truncation are all `Malformed`.
    pub fn decode(bytes: &[u8]) -> Result<Request, ErrorCode> {
        if bytes.len() > MAX_REQUEST_LEN {
            return Err(ErrorCode::Malformed);
        }
        let msg: Request = minicbor::decode(bytes).map_err(|_| ErrorCode::Malformed)?;
        if msg.encode() != bytes {
            return Err(ErrorCode::Malformed);
        }
        Ok(msg)
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        minicbor::to_vec(self).expect("encoding into a Vec cannot fail")
    }

    /// Strict: the input must be exactly one canonical encoding. Trailing bytes, extra
    /// fields (such as another message type's) and truncation are all `Malformed`.
    pub fn decode(bytes: &[u8]) -> Result<Response, ErrorCode> {
        let msg: Response = minicbor::decode(bytes).map_err(|_| ErrorCode::Malformed)?;
        if msg.encode() != bytes {
            return Err(ErrorCode::Malformed);
        }
        Ok(msg)
    }
}

impl<C> Encode<C> for PskHandle {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut Encoder<W>,
        _: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.bytes(&self.to_bytes())?.ok()
    }
}

impl<'b, C> Decode<'b, C> for PskHandle {
    fn decode(d: &mut Decoder<'b>, _: &mut C) -> Result<Self, DecodeError> {
        PskHandle::from_bytes(d.bytes()?)
            .ok_or_else(|| DecodeError::message("a handle is 21 bytes"))
    }
}

impl<C> Encode<C> for CircleInfo {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut Encoder<W>,
        _: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        e.array(5)?
            .bytes(&self.circle_id)?
            .u8(self.seat)?
            .u8(self.n)?
            .str(&self.name)?
            .u8(self.tier)?
            .ok()
    }
}

impl<'b, C> Decode<'b, C> for CircleInfo {
    fn decode(d: &mut Decoder<'b>, _: &mut C) -> Result<Self, DecodeError> {
        if d.array()? != Some(5) {
            return Err(DecodeError::message("a circle is an array of 5"));
        }
        let id = d.bytes()?;
        let circle_id: CircleId = id
            .try_into()
            .map_err(|_| DecodeError::message("a circle id is 16 bytes"))?;
        Ok(CircleInfo {
            circle_id,
            seat: d.u8()?,
            n: d.u8()?,
            name: String::from(d.str()?),
            tier: d.u8()?,
        })
    }
}
