//! The member session: the epoch keys a member holds, its own sending chain in each epoch, and a
//! receiving chain per sender key id, with the chat spec's out-of-order rules.
//!
//! The session does not make epochs: the owner draws them and installs them from a rotation, and
//! every member installs each epoch key it opens from a deposit.

use alloc::vec::Vec;
use cahoots_core::seat::{SeatKey, SeatPublic};
use minicbor::data::Type;
use minicbor::encode::{Error as EncodeError, Write};
use minicbor::{Decoder, Encoder};
use zeroize::{Zeroize, Zeroizing};

use crate::cbor::{array_len, Nested, Reader, VERSION};
use crate::keys::{chain_init, kid_of, step, EpochId, Key, EPOCH_ID_LEN};
use crate::message::{self, Header, MessageType};
use crate::{ChannelError, ChannelId};

pub use cahoots_chat::MAX_SKIP;

/// Epoch states kept per session. The oldest is dropped beyond this.
pub const MAX_EPOCHS: usize = 4;

const M: ChannelError = ChannelError::Malformed;

/// What [`ChannelSession::receive`] returns: the header, the plaintext and the approvals.
pub type Received = (Header, Vec<u8>, Vec<Vec<u8>>);

/// One sender's ratchet in one epoch. Copied from `cahoots-chat`, where it is private, with the
/// chain started from a 32-byte key id.
#[derive(Clone)]
struct SenderChain {
    chain: Key,
    next: u32,
    skipped: Vec<(u32, Key)>,
}

impl SenderChain {
    fn new(epoch_key: &[u8; 32], kid: &[u8; 32]) -> Self {
        SenderChain {
            chain: chain_init(epoch_key, kid),
            next: 0,
            skipped: Vec::new(),
        }
    }

    /// The message key for `counter`, advancing and caching skipped keys as needed. Changes
    /// `self` only on success.
    fn key_for(&mut self, counter: u32) -> Result<Key, ChannelError> {
        if counter < self.next {
            let pos = self
                .skipped
                .iter()
                .position(|(c, _)| *c == counter)
                .ok_or(ChannelError::Reuse)?;
            let (_, k) = self.skipped.remove(pos);
            return Ok(k);
        }
        if counter - self.next > MAX_SKIP {
            return Err(ChannelError::TooFarAhead);
        }
        let mut work = self.clone();
        while work.next < counter {
            let (mk, next) = step(&work.chain);
            work.chain = next;
            work.skipped.push((work.next, mk));
            work.next += 1;
        }
        let (mk, next) = step(&work.chain);
        work.chain = next;
        work.next = work.next.checked_add(1).ok_or(ChannelError::Reuse)?;
        while work.skipped.len() > MAX_SKIP as usize {
            work.skipped.remove(0);
        }
        *self = work;
        Ok(mk)
    }

    /// The next key for our own sends. A chain at `u32::MAX` is spent.
    fn advance(&mut self) -> Result<(u32, Key), ChannelError> {
        let c = self.next;
        let next_counter = c.checked_add(1).ok_or(ChannelError::Reuse)?;
        let (mk, next) = step(&self.chain);
        self.chain = next;
        self.next = next_counter;
        Ok((c, mk))
    }
}

impl Drop for SenderChain {
    fn drop(&mut self) {
        self.next.zeroize();
    }
}

struct EpochState {
    id: EpochId,
    key: Key,
    /// My chain for sending.
    send: SenderChain,
    /// A receiving chain per sender key id, mine included, made on first sight.
    receive: Vec<([u8; 32], SenderChain)>,
}

pub struct ChannelSession {
    channel_id: ChannelId,
    my_key: SeatKey,
    my_kid: [u8; 32],
    epochs: Vec<EpochState>,
    /// Always the held epoch with the highest number.
    current: Option<EpochId>,
    sent_in_current: u32,
    /// The highest epoch number dropped from the window. An epoch at or below it is never
    /// installed again, so the session never holds a second key for one epoch number.
    dropped_through: Option<u32>,
}

impl core::fmt::Debug for ChannelSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelSession")
            .field("channel_id", &self.channel_id)
            .field("my_kid", &self.my_kid)
            .field("epochs", &self.epochs.len())
            .field("current", &self.current)
            .field("sent_in_current", &self.sent_in_current)
            .field("dropped_through", &self.dropped_through)
            .finish_non_exhaustive()
    }
}

impl ChannelSession {
    pub fn new(channel_id: ChannelId, my_key: SeatKey) -> Self {
        let my_kid = kid_of(&my_key.public());
        ChannelSession {
            channel_id,
            my_key,
            my_kid,
            epochs: Vec::new(),
            current: None,
            sent_in_current: 0,
            dropped_through: None,
        }
    }

    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub fn my_kid(&self) -> [u8; 32] {
        self.my_kid
    }

    pub fn my_public(&self) -> SeatPublic {
        self.my_key.public()
    }

    pub fn current_epoch(&self) -> Option<EpochId> {
        self.current
    }

    /// Messages I have sent in the current epoch.
    pub fn sent_in_current(&self) -> u32 {
        self.sent_in_current
    }

    pub fn has_epoch(&self, id: &EpochId) -> bool {
        self.epochs.iter().any(|e| e.id == *id)
    }

    /// Install an epoch key. It becomes current only when its number is higher than the
    /// current epoch's; an older epoch is installed for reading only. Beyond [`MAX_EPOCHS`] the
    /// lowest epoch number is dropped.
    ///
    /// Refused with [`ChannelError::Reuse`], changing nothing: an epoch already held, an epoch
    /// at or below one already dropped, or, with the window full, an epoch lower than every one
    /// held (it would be dropped at once). The session never holds two keys for one epoch. An
    /// epoch of another channel is [`ChannelError::WrongChannel`].
    pub fn install_epoch(&mut self, id: EpochId, key: Key) -> Result<(), ChannelError> {
        if id.channel_id != self.channel_id {
            return Err(ChannelError::WrongChannel);
        }
        if self.has_epoch(&id) || self.dropped_through.is_some_and(|d| id.epoch <= d) {
            return Err(ChannelError::Reuse);
        }
        // Epochs are kept in ascending number order.
        let at = self.epochs.partition_point(|e| e.id.epoch < id.epoch);
        if at == 0 && self.epochs.len() >= MAX_EPOCHS {
            return Err(ChannelError::Reuse);
        }
        let send = SenderChain::new(&key, &self.my_kid);
        self.epochs.insert(
            at,
            EpochState {
                id,
                key,
                send,
                receive: Vec::new(),
            },
        );
        while self.epochs.len() > MAX_EPOCHS {
            let dropped = self.epochs.remove(0);
            self.dropped_through = Some(dropped.id.epoch);
        }
        if self.current.is_none_or(|c| id.epoch > c.epoch) {
            self.current = Some(id);
            self.sent_in_current = 0;
        }
        Ok(())
    }

    /// Encrypt and sign a message in the current epoch. The plaintext is sent as given; the
    /// caller pads it. No current epoch is [`ChannelError::UnknownEpoch`].
    pub fn send(
        &mut self,
        msg_type: MessageType,
        plaintext: &[u8],
        approvals: &[Vec<u8>],
    ) -> Result<Vec<u8>, ChannelError> {
        let id = self.current.ok_or(ChannelError::UnknownEpoch)?;
        let len = u32::try_from(plaintext.len()).map_err(|_| M)?;
        let epoch = self
            .epochs
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(ChannelError::UnknownEpoch)?;
        let (counter, msg_key) = epoch.send.advance()?;
        let header = Header {
            channel_id: self.channel_id,
            epoch_id: id,
            kid: self.my_kid,
            counter,
            len,
            msg_type,
        };
        let wire = message::seal_sign(&header, &msg_key, plaintext, approvals, &self.my_key);
        self.sent_in_current = self.sent_in_current.saturating_add(1);
        Ok(wire)
    }

    /// Verify a message under `signer`, then decrypt it. Returns the header, the plaintext and
    /// the approvals. State changes only on success.
    pub fn receive(&mut self, wire: &[u8], signer: &SeatPublic) -> Result<Received, ChannelError> {
        // Checks the COSE kid and the header kid against `kid_of(signer)`.
        let (header, ct, approvals) = message::verify_signature(wire, signer)?;
        if header.channel_id != self.channel_id {
            return Err(ChannelError::WrongChannel);
        }
        let epoch = self
            .epochs
            .iter_mut()
            .find(|e| e.id == header.epoch_id)
            .ok_or(ChannelError::UnknownEpoch)?;
        let pos = epoch.receive.iter().position(|(k, _)| *k == header.kid);
        let mut chain = match pos {
            Some(i) => epoch.receive[i].1.clone(),
            None => SenderChain::new(&epoch.key, &header.kid),
        };
        let msg_key = chain.key_for(header.counter)?;
        let plaintext = message::open(&header, &msg_key, ct)?;
        match pos {
            Some(i) => epoch.receive[i].1 = chain,
            None => epoch.receive.push((header.kid, chain)),
        }
        Ok((header, plaintext, approvals))
    }

    /// The whole session as a canonical CBOR map. It holds the seat key's seed and every epoch
    /// key, so callers store it under their own protection.
    ///
    /// ```text
    /// {0: 1, 1: seed, 2: channel_id, 3: current epoch_id or nil, 4: sent_in_current,
    ///  5: [{0: epoch_id, 1: epoch_key, 2: send chain, 3: [{0: kid, 1: chain}, ...]}, ...],
    ///  6: highest dropped epoch number or nil}
    /// ```
    ///
    /// Epochs are in ascending number order, and the current epoch is the last.
    ///
    /// ```text
    /// chain = {0: chain key, 1: next counter, 2: [{0: counter, 1: message key}, ...]}
    /// ```
    pub fn export_state(&self) -> Zeroizing<Vec<u8>> {
        let mut w = SecretBuf(Zeroizing::new(Vec::new()));
        let _ = self.encode(&mut Encoder::new(&mut w));
        w.0
    }

    /// Read back what [`export_state`](Self::export_state) wrote. Anything else, or the right
    /// shape with an impossible state, is [`ChannelError::Malformed`] or
    /// [`ChannelError::NotCanonical`].
    pub fn import_state(bytes: &[u8]) -> Result<Self, ChannelError> {
        let mut r = Reader::open(bytes, 7)?;
        let seed: Zeroizing<[u8; 32]> = Zeroizing::new(r.array()?);
        let my_key = SeatKey::from_seed(&seed).map_err(|_| M)?;
        let channel_id: ChannelId = r.array()?;
        let current = {
            let d = r.key()?;
            match d.datatype().map_err(|_| M)? {
                Type::Null => {
                    d.null().map_err(|_| M)?;
                    None
                }
                _ => Some(epoch_id(d.bytes().map_err(|_| M)?)?),
            }
        };
        let sent_in_current = u32::try_from(r.u64()?).map_err(|_| M)?;
        let mut s = ChannelSession::new(channel_id, my_key);
        s.current = current;
        s.sent_in_current = sent_in_current;
        let d = r.key()?;
        let n = array_len(d)?;
        if n > MAX_EPOCHS as u64 {
            return Err(M);
        }
        for _ in 0..n {
            let mut e = Nested::open(d, 4)?;
            let id = epoch_id(e.bytes()?)?;
            let ascending = s.epochs.last().is_none_or(|l| l.id.epoch < id.epoch);
            if id.channel_id != channel_id || !ascending {
                return Err(M);
            }
            let key = key(e.bytes()?)?;
            let send = decode_chain(e.key()?)?;
            let rd = e.key()?;
            let n_senders = array_len(rd)?;
            let mut receive = Vec::new();
            for _ in 0..n_senders {
                let mut p = Nested::open(rd, 2)?;
                let kid: [u8; 32] = p.bytes()?.try_into().map_err(|_| M)?;
                let chain = decode_chain(p.key()?)?;
                p.finish()?;
                if receive.iter().any(|(k, _)| *k == kid) {
                    return Err(M);
                }
                receive.push((kid, chain));
            }
            e.finish()?;
            s.epochs.push(EpochState {
                id,
                key,
                send,
                receive,
            });
        }
        let dropped_through = {
            let d = r.key()?;
            match d.datatype().map_err(|_| M)? {
                Type::Null => {
                    d.null().map_err(|_| M)?;
                    None
                }
                _ => Some(d.u32().map_err(|_| M)?),
            }
        };
        r.finish()?;
        s.dropped_through = dropped_through;
        if current != s.epochs.last().map(|e| e.id) || (current.is_none() && sent_in_current != 0) {
            return Err(M);
        }
        if let (Some(d), Some(first)) = (dropped_through, s.epochs.first()) {
            if first.id.epoch <= d {
                return Err(M);
            }
        }
        if *s.export_state() != *bytes {
            return Err(ChannelError::NotCanonical);
        }
        Ok(s)
    }

    fn encode<W: Write>(&self, e: &mut Encoder<W>) -> Result<(), EncodeError<W::Error>> {
        e.map(7)?.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(self.my_key.to_bytes().as_ref())?;
        e.u8(2)?.bytes(&self.channel_id)?;
        e.u8(3)?;
        match self.current {
            Some(id) => e.bytes(&id.encode())?,
            None => e.null()?,
        };
        e.u8(4)?.u32(self.sent_in_current)?;
        e.u8(5)?.array(self.epochs.len() as u64)?;
        for ep in &self.epochs {
            e.map(4)?;
            e.u8(0)?.bytes(&ep.id.encode())?;
            e.u8(1)?.bytes(ep.key.as_ref())?;
            e.u8(2)?;
            encode_chain(e, &ep.send)?;
            e.u8(3)?.array(ep.receive.len() as u64)?;
            for (kid, chain) in &ep.receive {
                e.map(2)?.u8(0)?.bytes(kid)?.u8(1)?;
                encode_chain(e, chain)?;
            }
        }
        e.u8(6)?;
        match self.dropped_through {
            Some(n) => e.u32(n)?,
            None => e.null()?,
        };
        Ok(())
    }
}

fn encode_chain<W: Write>(
    e: &mut Encoder<W>,
    c: &SenderChain,
) -> Result<(), EncodeError<W::Error>> {
    e.map(3)?;
    e.u8(0)?.bytes(c.chain.as_ref())?;
    e.u8(1)?.u32(c.next)?;
    e.u8(2)?.array(c.skipped.len() as u64)?;
    for (counter, k) in &c.skipped {
        e.map(2)?.u8(0)?.u32(*counter)?.u8(1)?.bytes(k.as_ref())?;
    }
    Ok(())
}

fn decode_chain(d: &mut Decoder<'_>) -> Result<SenderChain, ChannelError> {
    let mut c = Nested::open(d, 3)?;
    let chain = key(c.bytes()?)?;
    let next = c.u32()?;
    let sd = c.key()?;
    let n = array_len(sd)?;
    if n > u64::from(MAX_SKIP) {
        return Err(M);
    }
    let mut skipped: Vec<(u32, Key)> = Vec::new();
    for _ in 0..n {
        let mut p = Nested::open(sd, 2)?;
        let counter = p.u32()?;
        let k = key(p.bytes()?)?;
        p.finish()?;
        if counter >= next || skipped.iter().any(|(c, _)| *c == counter) {
            return Err(M);
        }
        skipped.push((counter, k));
    }
    c.finish()?;
    Ok(SenderChain {
        chain,
        next,
        skipped,
    })
}

fn key(bytes: &[u8]) -> Result<Key, ChannelError> {
    let mut k = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return Err(M);
    }
    k.copy_from_slice(bytes);
    Ok(k)
}

fn epoch_id(bytes: &[u8]) -> Result<EpochId, ChannelError> {
    if bytes.len() != EPOCH_ID_LEN {
        return Err(M);
    }
    EpochId::decode(bytes).ok_or(M)
}

/// A buffer that never leaves a copy of its contents behind when it grows: it moves to a new
/// allocation itself and zeroizes the old one.
struct SecretBuf(Zeroizing<Vec<u8>>);

impl Write for SecretBuf {
    type Error = core::convert::Infallible;

    fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        let need = self.0.len() + buf.len();
        if need > self.0.capacity() {
            let mut grown =
                Zeroizing::new(Vec::with_capacity(need.max(2 * self.0.capacity()).max(256)));
            grown.extend_from_slice(&self.0);
            self.0 = grown;
        }
        self.0.extend_from_slice(buf);
        Ok(())
    }
}
