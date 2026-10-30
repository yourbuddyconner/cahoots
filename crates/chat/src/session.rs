//! The stateful chat session: root epochs, per-sender ratchets and skipped keys.

use crate::primitives::*;
use alloc::vec::Vec;
use cahoots_core::CircleId;
use zeroize::{Zeroize, Zeroizing};

/// Start a new epoch after this many of our own messages in the current one.
pub const ROTATE_AFTER: u32 = 1000;
/// Skipped message keys kept per sender per epoch, for out-of-order delivery.
pub const MAX_SKIP: u32 = 256;
/// Epoch states kept per session. The oldest is evicted beyond this.
pub const MAX_EPOCHS: usize = 4;

/// `CHS1` was the pad-epoch state. It does not import.
const STATE_MAGIC: &[u8; 4] = b"CHS2";

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
    n: u32,
    senders: Vec<SenderChain>,
}

impl EpochState {
    fn new(root: &[u8; 32], circle_id: &CircleId, n: u32, members: u8) -> Self {
        let key = epoch_key(root, circle_id, n);
        let senders = (0..members).map(|i| SenderChain::new(&key, i)).collect();
        EpochState { n, senders }
    }
}

impl Drop for SenderChain {
    fn drop(&mut self) {
        self.next.zeroize();
    }
}

/// One member's chat in one circle, keyed by root epochs. Holds the circle root, the
/// chains of up to `MAX_EPOCHS` epochs, and where this member stands. No pad is spent.
pub struct ChatSession {
    root: Zeroizing<[u8; 32]>,
    circle_id: CircleId,
    members: u8,
    my_index: u8,
    epochs: Vec<EpochState>,
    /// The highest root epoch held or started, 0 before any. This member sends in it.
    highest: u32,
    /// Every epoch at or below this is refused: its state was dropped, or never held and
    /// older than one that was. Starts at 0, so epoch 0 is never used.
    floor: u32,
    sent_in_current: u32,
}

impl ChatSession {
    pub fn new(root: &[u8; 32], circle_id: CircleId, members: u8, my_index: u8) -> Self {
        ChatSession {
            root: Zeroizing::new(*root),
            circle_id,
            members,
            my_index,
            epochs: Vec::new(),
            highest: 0,
            floor: 0,
            sent_in_current: 0,
        }
    }

    pub fn circle_id(&self) -> CircleId {
        self.circle_id
    }
    pub fn members(&self) -> u8 {
        self.members
    }
    pub fn my_index(&self) -> u8 {
        self.my_index
    }
    /// Number of epoch states held.
    pub fn epochs(&self) -> usize {
        self.epochs.len()
    }
    /// The epoch this member sends in: the highest it has started or read.
    pub fn current_epoch(&self) -> Option<EpochId> {
        (self.highest > 0).then(|| EpochId::root(self.highest))
    }

    /// Keep `state`, drop the oldest beyond `MAX_EPOCHS`, and move to it if it is the newest.
    fn install(&mut self, state: EpochState) {
        let n = state.n;
        self.epochs.push(state);
        while self.epochs.len() > MAX_EPOCHS {
            let oldest = (0..self.epochs.len())
                .min_by_key(|&i| self.epochs[i].n)
                .expect("not empty");
            let gone = self.epochs.remove(oldest);
            self.floor = self.floor.max(gone.n);
        }
        if n > self.highest {
            self.highest = n;
            self.sent_in_current = 0;
        }
    }

    /// Move to the next root epoch: one past the highest seen, or 1 on a fresh session.
    /// Nothing is spent, so this cannot fail.
    pub fn start_epoch(&mut self) -> EpochId {
        let n = self.highest.saturating_add(1);
        if self.epoch_mut(n).is_none() {
            let state = EpochState::new(&self.root, &self.circle_id, n, self.members);
            self.install(state);
        }
        EpochId::root(n)
    }

    fn epoch_mut(&mut self, n: u32) -> Option<&mut EpochState> {
        self.epochs.iter_mut().find(|e| e.n == n)
    }

    pub fn send(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
        self.send_typed(MessageType::Text, plaintext)
    }

    /// Seal `plaintext` in the current epoch, starting epoch 1 if there is none and the
    /// next one after `ROTATE_AFTER` of our own messages.
    pub fn send_typed(
        &mut self,
        msg_type: MessageType,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ChatError> {
        if self.highest == 0
            || self.sent_in_current >= ROTATE_AFTER
            || self.epoch_mut(self.highest).is_none()
        {
            self.start_epoch();
        }
        let n = self.highest;
        let circle_id = self.circle_id;
        let me = self.my_index;
        let epoch = self.epoch_mut(n).ok_or(ChatError::UnknownEpoch)?;
        let (counter, mk) = epoch.senders[usize::from(me)].advance();
        let header = Header {
            version: CHAT_VERSION,
            circle_id,
            epoch: EpochId::root(n),
            sender: me,
            counter,
            len: plaintext.len() as u32,
            msg_type,
        };
        self.sent_in_current += 1;
        Ok(seal(&header, &mk, plaintext))
    }

    /// Verify and decrypt. An unknown root epoch `n` is derived from the root if it is above
    /// the floor and at most `MAX_AHEAD` past the highest seen, and kept only if the message
    /// opens. State changes only on success.
    pub fn receive(&mut self, wire: &[u8]) -> Result<(Header, Vec<u8>), ChatError> {
        let (header, body) = parse(wire)?;
        if header.circle_id != self.circle_id {
            return Err(ChatError::WrongCircle);
        }
        if header.sender >= self.members {
            return Err(ChatError::BadSender);
        }
        let n = header.epoch.number().ok_or(ChatError::PadEpoch)?;
        let sender = usize::from(header.sender);
        if let Some(epoch) = self.epoch_mut(n) {
            let chain = &mut epoch.senders[sender];
            // Work on a copy so a tag failure leaves the chain untouched.
            let mut trial = chain.clone();
            let mk = trial.key_for(header.counter)?;
            let pt = open(&header, &mk, body)?;
            *chain = trial;
            return Ok((header, pt));
        }
        if n <= self.floor {
            return Err(ChatError::UnknownEpoch);
        }
        if u64::from(n) > u64::from(self.highest) + u64::from(MAX_AHEAD) {
            return Err(ChatError::EpochTooFarAhead);
        }
        let mut state = EpochState::new(&self.root, &self.circle_id, n, self.members);
        let mk = state.senders[sender].key_for(header.counter)?;
        let pt = open(&header, &mk, body)?;
        self.install(state);
        Ok((header, pt))
    }

    /// Serialize epoch and chain state (secrets included) for persistence. The root is not
    /// in it: `import_state` takes the root again.
    pub fn export_state(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::new());
        out.extend_from_slice(STATE_MAGIC);
        out.push(self.my_index);
        out.push(self.members);
        out.extend_from_slice(&self.highest.to_le_bytes());
        out.extend_from_slice(&self.floor.to_le_bytes());
        out.extend_from_slice(&self.sent_in_current.to_le_bytes());
        out.push(self.epochs.len() as u8);
        for e in &self.epochs {
            out.extend_from_slice(&e.n.to_le_bytes());
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

    pub fn import_state(
        root: &[u8; 32],
        circle_id: CircleId,
        members: u8,
        my_index: u8,
        state: &[u8],
    ) -> Result<Self, ChatError> {
        let mut r = Reader { b: state, pos: 0 };
        if r.take(4)? != STATE_MAGIC || r.u8()? != my_index || r.u8()? != members {
            return Err(ChatError::State);
        }
        let highest = r.u32()?;
        let floor = r.u32()?;
        let sent_in_current = r.u32()?;
        let n_epochs = usize::from(r.u8()?);
        if n_epochs > MAX_EPOCHS || floor > highest {
            return Err(ChatError::State);
        }
        let mut epochs: Vec<EpochState> = Vec::with_capacity(n_epochs);
        for _ in 0..n_epochs {
            let n = r.u32()?;
            if n <= floor || n > highest || epochs.iter().any(|e| e.n == n) {
                return Err(ChatError::State);
            }
            let mut senders = Vec::with_capacity(usize::from(members));
            for _ in 0..members {
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
            epochs.push(EpochState { n, senders });
        }
        if r.pos != state.len() {
            return Err(ChatError::State);
        }
        Ok(ChatSession {
            root: Zeroizing::new(*root),
            circle_id,
            members,
            my_index,
            epochs,
            highest,
            floor,
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
