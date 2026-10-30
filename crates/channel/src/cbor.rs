//! Canonical CBOR helpers, copied from `cahoots-objects` (where they are crate-private) with
//! [`ChannelError`] in place of `ObjectError`.

use alloc::string::String;
use alloc::vec::Vec;
use core::convert::Infallible;
use minicbor::data::Type;
use minicbor::encode::Error as EncodeError;
use minicbor::{Decoder, Encoder};

use crate::ChannelError;

/// The format version, key 0 of every top-level channel object.
pub(crate) const VERSION: u8 = 1;

pub(crate) type Enc = Encoder<Vec<u8>>;
pub(crate) type EncResult = Result<(), EncodeError<Infallible>>;

const M: ChannelError = ChannelError::Malformed;

/// Run a writer into a fresh buffer. Writing to a `Vec` cannot fail.
pub(crate) fn encode_with(f: impl FnOnce(&mut Enc) -> EncResult) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = f(&mut e);
    e.into_writer()
}

/// The last step of every `decode`: re-encoding must give back the input.
pub(crate) fn canonical<T>(value: T, encoded: Vec<u8>, bytes: &[u8]) -> Result<T, ChannelError> {
    if encoded != bytes {
        return Err(ChannelError::NotCanonical);
    }
    Ok(value)
}

/// Reads a map whose keys are `0..n` in order, the only canonical shape our objects have.
pub(crate) struct Reader<'b> {
    d: Decoder<'b>,
    len: usize,
    n: u64,
    next: u64,
}

impl<'b> Reader<'b> {
    /// Open the map and read key 0, the version.
    pub(crate) fn open(bytes: &'b [u8], n: u64) -> Result<Reader<'b>, ChannelError> {
        let mut d = Decoder::new(bytes);
        match d.map() {
            Ok(Some(len)) if len == n => {}
            Ok(None) => return Err(ChannelError::NotCanonical),
            _ => return Err(M),
        }
        let mut r = Reader {
            d,
            len: bytes.len(),
            n,
            next: 0,
        };
        r.key()?;
        if r.d.u64().map_err(|_| M)? != u64::from(VERSION) {
            return Err(M);
        }
        Ok(r)
    }

    /// Read the next key. A known key out of place is not canonical; an unknown one is not ours.
    pub(crate) fn key(&mut self) -> Result<&mut Decoder<'b>, ChannelError> {
        next_key(&mut self.d, &mut self.next, self.n)?;
        Ok(&mut self.d)
    }

    pub(crate) fn u64(&mut self) -> Result<u64, ChannelError> {
        self.key()?.u64().map_err(|_| M)
    }

    pub(crate) fn bytes(&mut self) -> Result<&'b [u8], ChannelError> {
        self.key()?.bytes().map_err(|_| M)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], ChannelError> {
        self.bytes()?.try_into().map_err(|_| M)
    }

    pub(crate) fn string(&mut self) -> Result<String, ChannelError> {
        self.key()?.str().map(String::from).map_err(|_| M)
    }

    /// A 32-byte string, or nil.
    pub(crate) fn opt_digest(&mut self) -> Result<Option<[u8; 32]>, ChannelError> {
        opt_digest(self.key()?)
    }

    /// Every key read and nothing after the map.
    pub(crate) fn finish(self) -> Result<(), ChannelError> {
        if self.next != self.n || self.d.position() != self.len {
            return Err(M);
        }
        Ok(())
    }
}

/// Reads a map nested inside another, keys `0..n` in order and no version key.
pub(crate) struct Nested<'a, 'b> {
    d: &'a mut Decoder<'b>,
    n: u64,
    next: u64,
}

impl<'a, 'b> Nested<'a, 'b> {
    /// Open a nested map of exactly `n` keys.
    pub(crate) fn open(d: &'a mut Decoder<'b>, n: u64) -> Result<Nested<'a, 'b>, ChannelError> {
        match d.map() {
            Ok(Some(len)) if len == n => {}
            Ok(None) => return Err(ChannelError::NotCanonical),
            _ => return Err(M),
        }
        Ok(Nested { d, n, next: 0 })
    }

    pub(crate) fn key(&mut self) -> Result<&mut Decoder<'b>, ChannelError> {
        next_key(self.d, &mut self.next, self.n)?;
        Ok(self.d)
    }

    pub(crate) fn u64(&mut self) -> Result<u64, ChannelError> {
        self.key()?.u64().map_err(|_| M)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, ChannelError> {
        self.u64()?.try_into().map_err(|_| M)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ChannelError> {
        self.u64()?.try_into().map_err(|_| M)
    }

    pub(crate) fn bytes(&mut self) -> Result<&'b [u8], ChannelError> {
        self.key()?.bytes().map_err(|_| M)
    }

    pub(crate) fn string(&mut self) -> Result<String, ChannelError> {
        self.key()?.str().map(String::from).map_err(|_| M)
    }

    pub(crate) fn opt_digest(&mut self) -> Result<Option<[u8; 32]>, ChannelError> {
        opt_digest(self.key()?)
    }

    /// Every key read.
    pub(crate) fn finish(self) -> Result<(), ChannelError> {
        if self.next != self.n {
            return Err(M);
        }
        Ok(())
    }
}

fn next_key(d: &mut Decoder<'_>, next: &mut u64, n: u64) -> Result<(), ChannelError> {
    let k = d.u64().map_err(|_| M)?;
    if k != *next {
        return Err(if k < n { ChannelError::NotCanonical } else { M });
    }
    *next += 1;
    Ok(())
}

fn opt_digest(d: &mut Decoder<'_>) -> Result<Option<[u8; 32]>, ChannelError> {
    match d.datatype().map_err(|_| M)? {
        Type::Null => {
            d.null().map_err(|_| M)?;
            Ok(None)
        }
        Type::Bytes => Ok(Some(d.bytes().map_err(|_| M)?.try_into().map_err(|_| M)?)),
        _ => Err(M),
    }
}

/// A definite array length; an indefinite one parses but is not canonical.
pub(crate) fn array_len(d: &mut Decoder<'_>) -> Result<u64, ChannelError> {
    match d.array() {
        Ok(Some(n)) => Ok(n),
        Ok(None) => Err(ChannelError::NotCanonical),
        Err(_) => Err(M),
    }
}
