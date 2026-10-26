//! The request and response messages, CBOR (RFC 8949), array-encoded.
//!
//! Each message is a CBOR array of the variant index and an array of that variant's
//! fields in order. Keys, circle ids and spent bytes are byte strings. A `PskHandle` is its
//! 21-byte `to_bytes` form as one byte string.

use crate::contract::{CircleInfo, PadError, PskHandle};
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::CircleId;
use minicbor::decode::Error as DecodeError;
use minicbor::{Decode, Decoder, Encode, Encoder};

/// Frame kind for a request.
pub const KIND_REQUEST: u8 = 40;
/// Frame kind for a response.
pub const KIND_RESPONSE: u8 = 41;

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

    pub fn decode(bytes: &[u8]) -> Result<Request, ErrorCode> {
        minicbor::decode(bytes).map_err(|_| ErrorCode::Malformed)
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        minicbor::to_vec(self).expect("encoding into a Vec cannot fail")
    }

    pub fn decode(bytes: &[u8]) -> Result<Response, ErrorCode> {
        minicbor::decode(bytes).map_err(|_| ErrorCode::Malformed)
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
