//! A member's view of one circle: pad storage, spent ledger, and the send/receive
//! paths with the spend ordering the handoff requires (3.5, 3.6).
//!
//! Sending: read range, **mark spent, erase**, then seal and return.
//! Receiving: reject reuse, verify tag, decrypt, mark spent, erase.

use crate::circle_id::{circle_id, CircleId};
use crate::ledger::{LedgerError, SpentLedger};
use crate::message::{self, Header, MessageError, MessageType, KEY_LEN};
use crate::params::CircleParams;
use alloc::vec;
use alloc::vec::Vec;
use zeroize::Zeroizing;

/// Raw pad bytes. `erase` overwrites and MUST verify by reading back.
pub trait PadStorage {
    fn len(&self) -> u32;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read(&self, off: u32, buf: &mut [u8]);
    fn erase(&mut self, off: u32, len: u32) -> Result<(), StorageError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    OutOfRange,
    VerifyFailed,
}

/// In-memory pad for tests and the WASM client. Erasure here is best effort by nature.
#[derive(Clone, Debug)]
pub struct MemPadStorage {
    bytes: Zeroizing<Vec<u8>>,
}

impl MemPadStorage {
    pub fn new(bytes: Vec<u8>) -> Self {
        MemPadStorage {
            bytes: Zeroizing::new(bytes),
        }
    }
}

impl PadStorage for MemPadStorage {
    fn len(&self) -> u32 {
        self.bytes.len() as u32
    }
    fn read(&self, off: u32, buf: &mut [u8]) {
        let off = off as usize;
        buf.copy_from_slice(&self.bytes[off..off + buf.len()]);
    }
    fn erase(&mut self, off: u32, len: u32) -> Result<(), StorageError> {
        let (s, e) = (off as usize, (off + len) as usize);
        if e > self.bytes.len() {
            return Err(StorageError::OutOfRange);
        }
        self.bytes[s..e].fill(0);
        if self.bytes[s..e].iter().any(|&b| b != 0) {
            return Err(StorageError::VerifyFailed);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadError {
    /// The message names a different circle.
    WrongCircle,
    /// Slice or offset range lies outside this pad.
    OutOfRange,
    /// Some byte of the range is already spent. Possible pad reuse: flag it.
    Reuse,
    /// Not enough unspent bytes left in this member's slice.
    Exhausted,
    /// Storage length does not match the parameters, or the index is bad.
    Config,
    Message(MessageError),
    Ledger(LedgerError),
    Storage(StorageError),
}

impl From<MessageError> for PadError {
    fn from(e: MessageError) -> Self {
        PadError::Message(e)
    }
}
impl From<LedgerError> for PadError {
    fn from(e: LedgerError) -> Self {
        PadError::Ledger(e)
    }
}
impl From<StorageError> for PadError {
    fn from(e: StorageError) -> Self {
        PadError::Storage(e)
    }
}

pub struct Pad<S: PadStorage, L: SpentLedger> {
    params: CircleParams,
    my_slice: u8,
    circle_id: CircleId,
    storage: S,
    ledger: L,
}

impl<S: PadStorage, L: SpentLedger> Pad<S, L> {
    /// Wrap a freshly combined pad. Computes the circle ID from storage contents.
    pub fn new(
        params: CircleParams,
        my_slice: u8,
        storage: S,
        ledger: L,
    ) -> Result<Self, PadError> {
        if storage.len() != params.pad_len || my_slice >= params.slices() {
            return Err(PadError::Config);
        }
        let mut whole = Zeroizing::new(vec![0u8; params.pad_len as usize]);
        storage.read(0, &mut whole);
        let id = circle_id(&whole);
        Self::with_circle_id(params, my_slice, id, storage, ledger)
    }

    /// Reopen a pad whose circle ID is already known (bytes may be partly spent).
    pub fn with_circle_id(
        params: CircleParams,
        my_slice: u8,
        circle_id: CircleId,
        storage: S,
        ledger: L,
    ) -> Result<Self, PadError> {
        if storage.len() != params.pad_len || my_slice >= params.slices() {
            return Err(PadError::Config);
        }
        Ok(Pad {
            params,
            my_slice,
            circle_id,
            storage,
            ledger,
        })
    }

    pub fn circle_id(&self) -> CircleId {
        self.circle_id
    }
    pub fn params(&self) -> &CircleParams {
        &self.params
    }
    pub fn my_slice(&self) -> u8 {
        self.my_slice
    }
    pub fn storage(&self) -> &S {
        &self.storage
    }
    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    /// Unspent bytes left in this member's slice.
    pub fn remaining(&self) -> u32 {
        self.params
            .slice_len()
            .saturating_sub(self.ledger.next_offset(self.my_slice))
    }

    fn absolute(&self, slice: u8, off: u32, len: u32) -> Result<u32, PadError> {
        if slice >= self.params.slices() {
            return Err(PadError::OutOfRange);
        }
        let end = off.checked_add(len).ok_or(PadError::OutOfRange)?;
        if end > self.params.slice_len() {
            return Err(PadError::OutOfRange);
        }
        Ok(self.params.slice_start(slice) + off)
    }

    /// Spend an explicit range of any slice: mark spent (durable), then erase and verify.
    /// Never returns bytes that are still readable. This is the primitive the provider
    /// uses for PSKs; `send` and `receive` use it for messages.
    pub fn spend_range(
        &mut self,
        slice: u8,
        off: u32,
        len: u32,
    ) -> Result<Zeroizing<Vec<u8>>, PadError> {
        if self.ledger.is_spent(slice, off, len) {
            return Err(PadError::Reuse);
        }
        self.spend(slice, off, len)
    }

    /// Spend the next `len` bytes of this member's slice.
    pub fn spend_next(&mut self, len: u32) -> Result<(u32, Zeroizing<Vec<u8>>), PadError> {
        let off = self.ledger.next_offset(self.my_slice);
        if len > self.remaining() {
            return Err(PadError::Exhausted);
        }
        Ok((off, self.spend(self.my_slice, off, len)?))
    }

    /// Mark spent (durable), then erase and verify. Never returns bytes that are still readable.
    fn spend(&mut self, slice: u8, off: u32, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        let abs = self.absolute(slice, off, len)?;
        let mut buf = Zeroizing::new(vec![0u8; len as usize]);
        self.storage.read(abs, &mut buf);
        self.ledger.mark_spent(slice, off, len)?;
        self.storage.erase(abs, len)?;
        Ok(buf)
    }

    /// Encrypt `plaintext` from this member's slice. The range is spent and erased
    /// before the wire bytes exist.
    pub fn send(&mut self, msg_type: MessageType, plaintext: &[u8]) -> Result<Vec<u8>, PadError> {
        let len = u16::try_from(plaintext.len()).map_err(|_| MessageError::TooLong)?;
        let need = KEY_LEN as u32 + u32::from(len);
        let off = self.ledger.next_offset(self.my_slice);
        if need > self.remaining() {
            return Err(PadError::Exhausted);
        }
        let range = self.spend(self.my_slice, off, need)?;
        Ok(message::seal(
            &self.circle_id,
            self.my_slice,
            off,
            msg_type,
            plaintext,
            &range,
        )?)
    }

    /// Verify and decrypt a message from any slice of this circle.
    pub fn receive(&mut self, wire: &[u8]) -> Result<(Header, Vec<u8>), PadError> {
        let (hdr, ct, tag) = message::parse(wire)?;
        if hdr.circle_id != self.circle_id {
            return Err(PadError::WrongCircle);
        }
        let need = hdr.spend_len();
        let abs = self.absolute(hdr.slice, hdr.offset, need)?;
        if self.ledger.is_spent(hdr.slice, hdr.offset, need) {
            return Err(PadError::Reuse);
        }
        let mut range = Zeroizing::new(vec![0u8; need as usize]);
        self.storage.read(abs, &mut range);
        let pt = message::open(&hdr, ct, tag, &range)?;
        drop(range);
        self.ledger.mark_spent(hdr.slice, hdr.offset, need)?;
        self.storage.erase(abs, need)?;
        Ok((hdr, pt))
    }
}

/// Buckets clients SHOULD pad to, to hide plaintext length (handoff 3.5).
pub const BUCKETS: [usize; 4] = [32, 64, 128, 256];

/// Pad to the smallest bucket that fits `plaintext` plus one trailing length byte.
/// Returns `None` when the plaintext is longer than 255 bytes.
pub fn bucket_pad(plaintext: &[u8]) -> Option<Vec<u8>> {
    let size = *BUCKETS.iter().find(|&&b| plaintext.len() < b)?;
    let mut out = vec![0u8; size];
    out[..plaintext.len()].copy_from_slice(plaintext);
    out[size - 1] = plaintext.len() as u8;
    Some(out)
}

pub fn bucket_unpad(padded: &[u8]) -> Option<&[u8]> {
    let size = padded.len();
    if !BUCKETS.contains(&size) {
        return None;
    }
    let len = usize::from(padded[size - 1]);
    (len < size).then(|| &padded[..len])
}
