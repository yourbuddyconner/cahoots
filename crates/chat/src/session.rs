//! The stateful chat session: epochs, per-sender ratchets, skipped keys, and the
//! spend discipline for epoch ranges.

use crate::primitives::*;
use alloc::vec::Vec;
use cahoots_core::ledger::SpentLedger;
use cahoots_core::pad::{Pad, PadStorage};
use zeroize::{Zeroize, Zeroizing};

/// Start a new epoch after this many of our own messages in the current one.
pub const ROTATE_AFTER: u32 = 1000;
/// Skipped message keys kept per sender per epoch, for out-of-order delivery.
pub const MAX_SKIP: u32 = 256;
/// Epoch states kept per session. The oldest is evicted beyond this.
pub const MAX_EPOCHS: usize = 4;

const STATE_MAGIC: &[u8; 4] = b"CHS1";

#[derive(Clone)]
struct SenderChain {
    chain: Key,
    next: u32,
    skipped: Vec<(u32, Key)>,
}

impl SenderChain {
    fn new(epoch_key: &[u8; 32], sender: u8) -> Self {
        SenderChain {
            chain: chain_init(epoch_key, sender),
            next: 0,
            skipped: Vec::new(),
        }
    }

    /// Produce the message key for `counter`, advancing and caching as needed.
    /// Mutates `self` only on success.
    fn key_for(&mut self, counter: u32) -> Result<Key, ChatError> {
        if counter < self.next {
            let pos = self
                .skipped
                .iter()
                .position(|(c, _)| *c == counter)
                .ok_or(ChatError::Replay)?;
            let (_, k) = self.skipped.remove(pos);
            return Ok(k);
        }
        if counter - self.next > MAX_SKIP {
            return Err(ChatError::TooFarAhead);
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
        work.next += 1;
        while work.skipped.len() > MAX_SKIP as usize {
            work.skipped.remove(0);
        }
        *self = work;
        Ok(mk)
    }

    /// Next key for our own sends.
    fn advance(&mut self) -> (u32, Key) {
        let (mk, next) = step(&self.chain);
        self.chain = next;
        let c = self.next;
        self.next += 1;
        (c, mk)
    }
}

struct EpochState {
    id: EpochId,
    key: Key,
    senders: Vec<SenderChain>,
}

impl EpochState {
    fn new(id: EpochId, key: Key, members: u8) -> Self {
        let senders = (0..members).map(|i| SenderChain::new(&key, i)).collect();
        EpochState { id, key, senders }
    }
}

impl Drop for SenderChain {
    fn drop(&mut self) {
        self.next.zeroize();
    }
}

pub struct ChatSession<S: PadStorage, L: SpentLedger> {
    pad: Pad<S, L>,
    my_index: u8,
    epochs: Vec<EpochState>,
    current: Option<EpochId>,
    sent_in_current: u32,
}

impl<S: PadStorage, L: SpentLedger> ChatSession<S, L> {
    pub fn new(pad: Pad<S, L>, my_index: u8) -> Self {
        ChatSession {
            pad,
            my_index,
            epochs: Vec::new(),
            current: None,
            sent_in_current: 0,
        }
    }

    pub fn pad(&self) -> &Pad<S, L> {
        &self.pad
    }
    pub fn pad_mut(&mut self) -> &mut Pad<S, L> {
        &mut self.pad
    }
    pub fn into_pad(self) -> Pad<S, L> {
        self.pad
    }
    pub fn my_index(&self) -> u8 {
        self.my_index
    }
    /// Number of epoch states held.
    pub fn epochs(&self) -> usize {
        self.epochs.len()
    }
    pub fn current_epoch(&self) -> Option<EpochId> {
        self.current
    }

    fn members(&self) -> u8 {
        self.pad.params().n
    }

    fn install_epoch(&mut self, id: EpochId, psk: &[u8; 32]) {
        let key = epoch_key(&self.pad.circle_id(), &id, psk);
        self.epochs.push(EpochState::new(id, key, self.members()));
        while self.epochs.len() > MAX_EPOCHS {
            self.epochs.remove(0);
        }
        self.current = Some(id);
        self.sent_in_current = 0;
    }

    /// Spend 32 bytes from our slice and switch to the new epoch.
    pub fn start_epoch(&mut self) -> Result<EpochId, ChatError> {
        let (offset, bytes) = self.pad.spend_next(32)?;
        let id = EpochId {
            slice: self.pad.my_slice(),
            offset,
        };
        let psk: Zeroizing<[u8; 32]> =
            Zeroizing::new(bytes.as_slice().try_into().expect("spent 32 bytes"));
        self.install_epoch(id, &psk);
        Ok(id)
    }

    fn epoch_mut(&mut self, id: EpochId) -> Option<&mut EpochState> {
        self.epochs.iter_mut().find(|e| e.id == id)
    }

    pub fn send(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
        self.send_typed(MessageType::Text, plaintext)
    }

    pub fn send_typed(
        &mut self,
        msg_type: MessageType,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ChatError> {
        let needs_epoch = match self.current {
            None => true,
            Some(id) => self.sent_in_current >= ROTATE_AFTER || self.epoch_mut(id).is_none(),
        };
        if needs_epoch {
            self.start_epoch()?;
        }
        let id = self.current.expect("epoch installed");
        let circle_id = self.pad.circle_id();
        let me = self.my_index;
        let epoch = self.epoch_mut(id).ok_or(ChatError::UnknownEpoch)?;
        let (counter, mk) = epoch.senders[usize::from(me)].advance();
        let header = Header {
            version: CHAT_VERSION,
            circle_id,
            epoch: id,
            sender: me,
            counter,
            len: plaintext.len() as u32,
            msg_type,
        };
        self.sent_in_current += 1;
        Ok(seal(&header, &mk, plaintext))
    }

    /// Verify and decrypt. An unknown epoch is spent from the pad on first sight,
    /// before the tag is checked. State for a known epoch changes only on success.
    pub fn receive(&mut self, wire: &[u8]) -> Result<(Header, Vec<u8>), ChatError> {
        let (header, body) = parse(wire)?;
        if header.circle_id != self.pad.circle_id() {
            return Err(ChatError::WrongCircle);
        }
        if header.sender >= self.members() {
            return Err(ChatError::BadSender);
        }
        if self.epoch_mut(header.epoch).is_none() {
            let bytes = self
                .pad
                .spend_range(header.epoch.slice, header.epoch.offset, 32)?;
            let psk: Zeroizing<[u8; 32]> =
                Zeroizing::new(bytes.as_slice().try_into().expect("32 bytes"));
            self.install_epoch(header.epoch, &psk);
        }
        let epoch = self.epoch_mut(header.epoch).expect("just installed");
        let chain = &mut epoch.senders[usize::from(header.sender)];
        // Work on a copy so a tag failure leaves the chain untouched.
        let mut trial = chain.clone();
        let mk = trial.key_for(header.counter)?;
        let pt = open(&header, &mk, body)?;
        *chain = trial;
        Ok((header, pt))
    }

    /// Serialize epoch and chain state (secrets included) for persistence.
    pub fn export_state(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::new());
        out.extend_from_slice(STATE_MAGIC);
        out.push(self.my_index);
        match self.current {
            Some(id) => {
                out.push(1);
                out.extend_from_slice(&id.encode());
            }
            None => out.extend_from_slice(&[0; 1 + EPOCH_ID_LEN]),
        }
        out.extend_from_slice(&self.sent_in_current.to_le_bytes());
        out.push(self.epochs.len() as u8);
        for e in &self.epochs {
            out.extend_from_slice(&e.id.encode());
            out.extend_from_slice(e.key.as_ref());
            out.push(e.senders.len() as u8);
            for s in &e.senders {
                out.extend_from_slice(s.chain.as_ref());
                out.extend_from_slice(&s.next.to_le_bytes());
                out.extend_from_slice(&(s.skipped.len() as u16).to_le_bytes());
                for (c, k) in &s.skipped {
                    out.extend_from_slice(&c.to_le_bytes());
                    out.extend_from_slice(k.as_ref());
                }
            }
        }
        out
    }

    pub fn import_state(pad: Pad<S, L>, my_index: u8, state: &[u8]) -> Result<Self, ChatError> {
        let mut r = Reader { b: state, pos: 0 };
        if r.take(4)? != STATE_MAGIC || r.u8()? != my_index {
            return Err(ChatError::State);
        }
        let has_current = r.u8()?;
        let cur = EpochId::decode(r.take(EPOCH_ID_LEN)?);
        let current = match has_current {
            0 => None,
            1 => Some(cur),
            _ => return Err(ChatError::State),
        };
        let sent_in_current = r.u32()?;
        let n_epochs = usize::from(r.u8()?);
        if n_epochs > MAX_EPOCHS {
            return Err(ChatError::State);
        }
        let members = pad.params().n;
        let mut epochs = Vec::with_capacity(n_epochs);
        for _ in 0..n_epochs {
            let id = EpochId::decode(r.take(EPOCH_ID_LEN)?);
            let key = r.key()?;
            let n_senders = r.u8()?;
            if n_senders != members {
                return Err(ChatError::State);
            }
            let mut senders = Vec::with_capacity(usize::from(n_senders));
            for _ in 0..n_senders {
                let chain = r.key()?;
                let next = r.u32()?;
                let n_skipped = usize::from(r.u16()?);
                if n_skipped > MAX_SKIP as usize {
                    return Err(ChatError::State);
                }
                let mut skipped = Vec::with_capacity(n_skipped);
                for _ in 0..n_skipped {
                    let c = r.u32()?;
                    skipped.push((c, r.key()?));
                }
                senders.push(SenderChain {
                    chain,
                    next,
                    skipped,
                });
            }
            epochs.push(EpochState { id, key, senders });
        }
        if r.pos != state.len() {
            return Err(ChatError::State);
        }
        Ok(ChatSession {
            pad,
            my_index,
            epochs,
            current,
            sent_in_current,
        })
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ChatError> {
        let end = self.pos.checked_add(n).ok_or(ChatError::State)?;
        let s = self.b.get(self.pos..end).ok_or(ChatError::State)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ChatError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ChatError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, ChatError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn key(&mut self) -> Result<Key, ChatError> {
        let b = self.take(KEY_LEN)?;
        let mut k = Zeroizing::new([0u8; KEY_LEN]);
        k.copy_from_slice(b);
        Ok(k)
    }
}
