//! The stateful chat session: generations, their epochs, per-sender ratchets and skipped keys.

use crate::primitives::*;
use alloc::vec::Vec;
use cahoots_core::root::{chat_gen_key, gen0_id};
use cahoots_core::CircleId;
use zeroize::{Zeroize, Zeroizing};

/// Start a new epoch after this many of our own messages in the current one.
pub const ROTATE_AFTER: u32 = 1000;
/// Skipped message keys kept per sender per epoch, for out-of-order delivery.
pub const MAX_SKIP: u32 = 256;
/// Epoch states kept per generation, and for old root epochs. The oldest is evicted beyond this.
pub const MAX_EPOCHS: usize = 4;
/// Generations whose keys a session keeps: the current one and the one before it, so
/// messages sent by members who had not yet read a bump still open.
pub const MAX_GENERATIONS: usize = 2;
/// Ids of dropped generations kept, so a message in one is answered with its id.
const MAX_RETIRED: usize = 4;
/// How far one of our own messages may move our sending position in one step. Our own
/// counter rotates long before this, so a larger jump is not ours.
pub const MAX_OWN_SKIP: u32 = 1 << 16;

/// `CHS1` was the pad-epoch state. It does not import. `CHS2` was the root-epoch state, written
/// by the browser's state version 5. It imports as generation 0, with its root epochs kept for
/// reading old history.
const STATE_MAGIC: &[u8; 4] = b"CHS3";
const STATE_MAGIC_ROOT_EPOCHS: &[u8; 4] = ChatSession::STATE_MAGIC_ROOT_EPOCHS;

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

    /// Move our own chain past `counter` without keeping skipped keys, and return the key
    /// for `counter`. Our own messages are never decrypted, only checked.
    fn own_key_for(&mut self, counter: u32) -> Result<Key, ChatError> {
        if counter < self.next {
            return Err(ChatError::Replay);
        }
        if counter - self.next > MAX_OWN_SKIP {
            return Err(ChatError::TooFarAhead);
        }
        while self.next < counter {
            let (_, next) = step(&self.chain);
            self.chain = next;
            self.next += 1;
        }
        self.skipped.clear();
        Ok(self.advance().1)
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
    fn new(epoch_key: &[u8; 32], n: u32, members: u8) -> Self {
        let senders = (0..members)
            .map(|i| SenderChain::new(epoch_key, i))
            .collect();
        EpochState { n, senders }
    }
}

impl Drop for SenderChain {
    fn drop(&mut self) {
        self.next.zeroize();
    }
}

/// The epoch states of one generation, or of the old root epochs.
#[derive(Default)]
struct Epochs {
    states: Vec<EpochState>,
    /// The highest epoch held or started, 0 before any.
    highest: u32,
    /// Every epoch at or below this is refused: its state was dropped, or never held and
    /// older than one that was. Starts at 0, so epoch 0 is never used.
    floor: u32,
}

impl Epochs {
    fn get_mut(&mut self, n: u32) -> Option<&mut EpochState> {
        self.states.iter_mut().find(|e| e.n == n)
    }

    /// Keep `state` and drop the oldest beyond `MAX_EPOCHS`. True if `state` is the newest.
    fn install(&mut self, state: EpochState) -> bool {
        let n = state.n;
        self.states.push(state);
        while self.states.len() > MAX_EPOCHS {
            let oldest = (0..self.states.len())
                .min_by_key(|&i| self.states[i].n)
                .expect("not empty");
            let gone = self.states.remove(oldest);
            self.floor = self.floor.max(gone.n);
        }
        let newest = n > self.highest;
        if newest {
            self.highest = n;
        }
        newest
    }

    /// Verify and decrypt in epoch `n` of this set. An epoch not held and above the floor is
    /// made from `key` and kept only if the message opens. Returns the plaintext (empty for
    /// one of our own messages we are already past) and whether the set moved to a newer
    /// epoch. State changes only on success.
    fn receive(
        &mut self,
        n: u32,
        key: impl FnOnce() -> Result<Key, ChatError>,
        members: u8,
        own: bool,
        header: &Header,
        body: &[u8],
    ) -> Result<(Vec<u8>, bool), ChatError> {
        let held = self.get_mut(n).is_some();
        if !held && n <= self.floor {
            return Err(ChatError::UnknownEpoch);
        }
        let mut fresh = None;
        let epoch = match self.get_mut(n) {
            Some(e) => e,
            None => fresh.insert(EpochState::new(&*key()?, n, members)),
        };
        let chain = &mut epoch.senders[usize::from(header.sender)];
        if own && header.counter < chain.next {
            return Ok((Vec::new(), false));
        }
        // Work on a copy so a tag failure leaves the chain untouched.
        let mut trial = chain.clone();
        let mk = if own {
            trial.own_key_for(header.counter)?
        } else {
            trial.key_for(header.counter)?
        };
        let pt = open(header, &mk, body)?;
        *chain = trial;
        let moved = match fresh {
            Some(state) => self.install(state),
            None => false,
        };
        Ok((pt, moved))
    }

    fn export(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.highest.to_le_bytes());
        out.extend_from_slice(&self.floor.to_le_bytes());
        out.push(self.states.len() as u8);
        for e in &self.states {
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
    }

    /// The epoch states after `highest` and `floor` have been read.
    fn import_states(
        r: &mut Reader,
        highest: u32,
        floor: u32,
        members: u8,
    ) -> Result<Epochs, ChatError> {
        let n_epochs = usize::from(r.u8()?);
        if n_epochs > MAX_EPOCHS || floor > highest {
            return Err(ChatError::State);
        }
        let mut states: Vec<EpochState> = Vec::with_capacity(n_epochs);
        for _ in 0..n_epochs {
            let n = r.u32()?;
            if n <= floor || n > highest || states.iter().any(|e| e.n == n) {
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
            states.push(EpochState { n, senders });
        }
        Ok(Epochs {
            states,
            highest,
            floor,
        })
    }

    fn import(r: &mut Reader, members: u8) -> Result<Epochs, ChatError> {
        let highest = r.u32()?;
        let floor = r.u32()?;
        Epochs::import_states(r, highest, floor, members)
    }
}

/// One generation: its id, its key, and its epochs.
struct Generation {
    id: [u8; 32],
    tag: u16,
    key: Key,
    epochs: Epochs,
}

impl Generation {
    fn new(id: [u8; 32], key: Zeroizing<[u8; 32]>) -> Self {
        Generation {
            id,
            tag: generation_tag(&id),
            key,
            epochs: Epochs::default(),
        }
    }
}

/// One member's chat in one circle, keyed by generations.
///
/// A generation has an id and a key. Generation 0's id is derived from the circle id, and
/// later ones are random, named by a signed bump. Epoch keys derive from the generation key,
/// so starting an epoch spends nothing and needs no root. A session made with the root
/// derives any generation's key from its id. A session made from one generation's key
/// (`from_generation`) can chat in that generation only, until it is given the next key.
///
/// The session keeps the current generation and the one before it, up to `MAX_EPOCHS` epoch
/// states in each, and the old root epochs a circle used before generations, for reading only.
pub struct ChatSession {
    root: Option<Zeroizing<[u8; 32]>>,
    circle_id: CircleId,
    members: u8,
    my_index: u8,
    /// Old root epochs, for reading old history. Never sent in. Empty without the root.
    old: Epochs,
    /// Oldest first, at most `MAX_GENERATIONS`. The last is current: this member sends in it.
    gens: Vec<Generation>,
    /// A generation a bump named whose key this session could not derive.
    pending: Option<[u8; 32]>,
    /// Ids of generations whose keys were dropped, oldest first.
    retired: Vec<[u8; 32]>,
    /// Our own messages in the current epoch of the current generation.
    sent_in_current: u32,
    /// Whether the caller has read the relay to the end since this session was made or
    /// imported. Not exported: an imported session starts unsynced.
    synced: bool,
    /// The id of the generation the last message `receive` opened was sealed in. `None` after
    /// one in an old root epoch, or before any. Not exported.
    opened_in: Option<[u8; 32]>,
}

impl ChatSession {
    /// The first four bytes of a state written before generations. It imports, and the
    /// caller may want to read its relay again from the start: the build that wrote it passed
    /// over messages in generations it could not open.
    pub const STATE_MAGIC_ROOT_EPOCHS: &'static [u8; 4] = b"CHS2";

    /// A member's session, holding the root. It starts in generation 0.
    pub fn new(root: &[u8; 32], circle_id: CircleId, members: u8, my_index: u8) -> Self {
        let id = gen0_id(&circle_id);
        let key = chat_gen_key(root, &circle_id, &id);
        let mut s = Self::from_generation(id, key, circle_id, members, my_index);
        s.root = Some(Zeroizing::new(*root));
        s
    }

    /// A session for a device that holds one generation's key and not the root. It chats in
    /// that generation, and a bump leaves it behind until it is given the next key with
    /// `install_generation`.
    pub fn from_generation(
        generation_id: [u8; 32],
        generation_key: Zeroizing<[u8; 32]>,
        circle_id: CircleId,
        members: u8,
        my_index: u8,
    ) -> Self {
        ChatSession {
            root: None,
            circle_id,
            members,
            my_index,
            old: Epochs::default(),
            gens: alloc::vec![Generation::new(generation_id, generation_key)],
            pending: None,
            retired: Vec::new(),
            sent_in_current: 0,
            synced: false,
            opened_in: None,
        }
    }

    /// Record that the relay has been read to the end, our own messages included. Until
    /// then `send` refuses with `NotSynced`, so a session restored from older state cannot
    /// send from a counter it already used.
    pub fn mark_synced(&mut self) {
        self.synced = true;
    }
    pub fn synced(&self) -> bool {
        self.synced
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
    /// Number of epoch states held, in every generation and in the old root epochs.
    pub fn epochs(&self) -> usize {
        self.old.states.len()
            + self
                .gens
                .iter()
                .map(|g| g.epochs.states.len())
                .sum::<usize>()
    }

    fn current(&self) -> &Generation {
        self.gens
            .last()
            .expect("a session always holds a generation")
    }
    fn current_mut(&mut self) -> &mut Generation {
        self.gens
            .last_mut()
            .expect("a session always holds a generation")
    }

    /// The id of the generation this member sends in.
    pub fn generation_id(&self) -> [u8; 32] {
        self.current().id
    }

    /// The epoch this member sends in: the highest of the current generation it has started
    /// or read.
    pub fn current_epoch(&self) -> Option<EpochId> {
        let g = self.current();
        (g.epochs.highest > 0).then(|| EpochId::generation(g.tag, g.epochs.highest as u16))
    }

    /// Move to the next epoch of the current generation: one past the highest seen, or 1.
    /// Nothing is spent, so this cannot fail. At epoch 65535 it stays there.
    pub fn start_epoch(&mut self) -> EpochId {
        let members = self.members;
        let g = self.current_mut();
        let n = g.epochs.highest.saturating_add(1).min(u32::from(u16::MAX));
        let tag = g.tag;
        if g.epochs.get_mut(n).is_none() {
            let state = EpochState::new(&epoch_key_in(&g.key, n as u16), n, members);
            if g.epochs.install(state) {
                self.sent_in_current = 0;
            }
        }
        EpochId::generation(tag, n as u16)
    }

    /// Follow a verified bump from `from` to `to`, read with the last call to `receive`, and
    /// start epoch 1 of `to`. Returns true when the session moved.
    ///
    /// The session moves only when the bump was sealed in generation `from` and `from` is the
    /// current generation. Any other bump is stale and moves nothing (false): one read again
    /// after the circle moved on, one sealed in a generation it does not name, or one to a
    /// generation already held or dropped. So an old bump replayed never takes the session
    /// back or sideways.
    ///
    /// With the root the key of `to` is derived. Without it the answer is
    /// `NeedsGeneration(to)`: fetch the key and call `install_generation`.
    pub fn apply_bump(&mut self, from: [u8; 32], to: [u8; 32]) -> Result<bool, ChatError> {
        if self.opened_in != Some(from) || self.current().id != from || self.knows(&to) {
            return Ok(false);
        }
        self.follow(to)?;
        Ok(true)
    }

    /// Move to `to`, deriving its key with the root.
    fn follow(&mut self, to: [u8; 32]) -> Result<(), ChatError> {
        let key = match &self.root {
            Some(root) => chat_gen_key(root, &self.circle_id, &to),
            None => {
                self.pending = Some(to);
                return Err(ChatError::NeedsGeneration(to));
            }
        };
        self.enter(to, key);
        Ok(())
    }

    /// Move to a generation whose key this device was given, and start its epoch 1. A
    /// generation already held or dropped moves nothing.
    pub fn install_generation(
        &mut self,
        generation_id: [u8; 32],
        generation_key: Zeroizing<[u8; 32]>,
    ) {
        if !self.knows(&generation_id) {
            self.enter(generation_id, generation_key);
        }
    }

    fn knows(&self, generation_id: &[u8; 32]) -> bool {
        self.gens.iter().any(|g| &g.id == generation_id) || self.retired.contains(generation_id)
    }

    fn enter(&mut self, generation_id: [u8; 32], generation_key: Zeroizing<[u8; 32]>) {
        self.gens
            .push(Generation::new(generation_id, generation_key));
        while self.gens.len() > MAX_GENERATIONS {
            let gone = self.gens.remove(0);
            self.retired.push(gone.id);
        }
        while self.retired.len() > MAX_RETIRED {
            self.retired.remove(0);
        }
        if self.pending == Some(generation_id) {
            self.pending = None;
        }
        self.sent_in_current = 0;
        self.start_epoch();
    }

    /// Send a bump in the current epoch of the current generation, then move this session to
    /// `generation_id`. `payload` is what `crate::bump::sign_bump` returned for a move from the
    /// current generation to this id.
    ///
    /// Only a session with the root sends a bump: it seals the bump in the generation the
    /// others are in and derives the new key itself. Without the root this refuses with
    /// `CannotBump`. It refuses with `CannotBump` too when the payload moves from a generation
    /// other than the current one, or to a generation this session holds or dropped, because
    /// the bump would move nobody. `BadSignature` when the payload does not name
    /// `generation_id`. Nothing is sent when it refuses.
    pub fn send_bump(
        &mut self,
        generation_id: [u8; 32],
        payload: &[u8],
    ) -> Result<Vec<u8>, ChatError> {
        if payload.get(32..64) != Some(&generation_id[..]) {
            return Err(ChatError::BadSignature);
        }
        let current = self.current().id;
        if self.root.is_none()
            || payload.get(..32) != Some(&current[..])
            || self.knows(&generation_id)
        {
            return Err(ChatError::CannotBump);
        }
        let wire = self.send_typed(MessageType::Bump, payload)?;
        self.follow(generation_id)?;
        Ok(wire)
    }

    pub fn send(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
        self.send_typed(MessageType::Text, plaintext)
    }

    /// Seal `plaintext` in the current epoch of the current generation, starting epoch 1 if
    /// there is none and the next one after `ROTATE_AFTER` of our own messages.
    pub fn send_typed(
        &mut self,
        msg_type: MessageType,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ChatError> {
        if !self.synced {
            return Err(ChatError::NotSynced);
        }
        let highest = self.current().epochs.highest;
        if highest == 0
            || self.sent_in_current >= ROTATE_AFTER
            || self.current_mut().epochs.get_mut(highest).is_none()
        {
            self.start_epoch();
        }
        let circle_id = self.circle_id;
        let me = self.my_index;
        let g = self.current_mut();
        let tag = g.tag;
        let n = g.epochs.highest;
        let epoch = g.epochs.get_mut(n).ok_or(ChatError::UnknownEpoch)?;
        let (counter, mk) = epoch.senders[usize::from(me)].advance();
        let header = Header {
            version: CHAT_VERSION,
            circle_id,
            epoch: EpochId::generation(tag, n as u16),
            sender: me,
            counter,
            len: plaintext.len() as u32,
            msg_type,
        };
        self.sent_in_current += 1;
        Ok(seal(&header, &mk, plaintext))
    }

    /// Verify and decrypt. An unknown epoch above the floor of a held generation is derived
    /// from the generation key and kept only if the message opens. State changes only on
    /// success.
    ///
    /// A message in a generation this session does not hold is `NeedsGeneration` with that
    /// generation's id when the session knows it (a bump it could not follow, or a generation
    /// it dropped), and `UnknownEpoch` otherwise. A message in an old root epoch opens with the
    /// root, and is `UnknownEpoch` without it.
    ///
    /// A message from our own seat is not decrypted. If it is past our sending position and
    /// its tag checks, our chain moves past it, and the plaintext returned is empty, except for
    /// a bump, whose payload is returned so a session rebuilt from the relay can follow it. If
    /// we are already past it, nothing changes and the plaintext is empty. This is how a
    /// device that lost or rolled back its chat state finds its place again.
    pub fn receive(&mut self, wire: &[u8]) -> Result<(Header, Vec<u8>), ChatError> {
        self.opened_in = None;
        let (header, body) = parse(wire)?;
        if header.circle_id != self.circle_id {
            return Err(ChatError::WrongCircle);
        }
        if header.sender >= self.members {
            return Err(ChatError::BadSender);
        }
        let own = header.sender == self.my_index;
        let members = self.members;
        let mut opened = None;
        let pt = if let Some((tag, n)) = header.epoch.generation_parts() {
            let current = self.gens.len() - 1;
            let mut result = Err(self.missing(tag));
            // The tag is 2 bytes, so the two held generations share one by chance once in
            // 65536. Try every generation with this tag, newest first: the first whose key
            // opens the message wins, and a key that fails changes nothing. If none opens,
            // the last error stands.
            let matching: Vec<usize> = (0..self.gens.len())
                .rev()
                .filter(|&i| self.gens[i].tag == tag)
                .collect();
            for i in matching {
                let Generation {
                    id, key, epochs, ..
                } = &mut self.gens[i];
                let id = *id;
                let key: &Key = key;
                result = epochs.receive(
                    u32::from(n),
                    || Ok(epoch_key_in(key, n)),
                    members,
                    own,
                    &header,
                    body,
                );
                if let Ok((_, moved)) = &result {
                    if *moved && i == current {
                        self.sent_in_current = 0;
                    }
                    opened = Some(id);
                    break;
                }
            }
            result?.0
        } else if let Some(n) = header.epoch.number() {
            let root = self.root.as_ref().ok_or(ChatError::UnknownEpoch)?;
            let circle_id = self.circle_id;
            self.old
                .receive(
                    n,
                    || Ok(epoch_key(root, &circle_id, n)),
                    members,
                    own,
                    &header,
                    body,
                )?
                .0
        } else {
            return Err(ChatError::PadEpoch);
        };
        self.opened_in = opened;
        let keep = !own || header.msg_type == MessageType::Bump;
        Ok((header, if keep { pt } else { Vec::new() }))
    }

    /// The answer for a message whose generation tag matches no held generation.
    fn missing(&self, tag: u16) -> ChatError {
        self.pending
            .iter()
            .chain(self.retired.iter().rev())
            .find(|id| generation_tag(id) == tag)
            .map_or(ChatError::UnknownEpoch, |id| {
                ChatError::NeedsGeneration(*id)
            })
    }

    /// Serialize generations, epochs and chains (secrets included, generation keys too) for
    /// persistence. The root is not in it: `import_state` takes the root again.
    pub fn export_state(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::new());
        out.extend_from_slice(STATE_MAGIC);
        out.push(self.my_index);
        out.push(self.members);
        out.extend_from_slice(&self.sent_in_current.to_le_bytes());
        self.old.export(&mut out);
        match &self.pending {
            Some(id) => {
                out.push(1);
                out.extend_from_slice(id);
            }
            None => out.push(0),
        }
        out.push(self.retired.len() as u8);
        for id in &self.retired {
            out.extend_from_slice(id);
        }
        out.push(self.gens.len() as u8);
        for g in &self.gens {
            out.extend_from_slice(&g.id);
            out.extend_from_slice(g.key.as_ref());
            g.epochs.export(&mut out);
        }
        out
    }

    /// Restore a member's session, which holds the root. Takes the current state, and the
    /// state written before generations (`CHS2`), which comes back as generation 0 with its
    /// root epochs kept for reading.
    pub fn import_state(
        root: &[u8; 32],
        circle_id: CircleId,
        members: u8,
        my_index: u8,
        state: &[u8],
    ) -> Result<Self, ChatError> {
        Self::import(Some(root), circle_id, members, my_index, state)
    }

    /// Restore the session of a device without the root, as `from_generation` made it.
    pub fn import_generation_state(
        circle_id: CircleId,
        members: u8,
        my_index: u8,
        state: &[u8],
    ) -> Result<Self, ChatError> {
        Self::import(None, circle_id, members, my_index, state)
    }

    fn import(
        root: Option<&[u8; 32]>,
        circle_id: CircleId,
        members: u8,
        my_index: u8,
        state: &[u8],
    ) -> Result<Self, ChatError> {
        let mut r = Reader { b: state, pos: 0 };
        let magic = r.take(4)?;
        if r.u8()? != my_index || r.u8()? != members {
            return Err(ChatError::State);
        }
        let mut s = if magic == STATE_MAGIC_ROOT_EPOCHS {
            // highest | floor | sent_in_current | epochs: the sending position was in a root
            // epoch, which is never sent in again, so it starts over in generation 0.
            let root = root.ok_or(ChatError::State)?;
            let highest = r.u32()?;
            let floor = r.u32()?;
            let _sent_in_root_epoch = r.u32()?;
            let old = Epochs::import_states(&mut r, highest, floor, members)?;
            let mut s = ChatSession::new(root, circle_id, members, my_index);
            s.old = old;
            s
        } else if magic == STATE_MAGIC {
            let sent_in_current = r.u32()?;
            let old = Epochs::import(&mut r, members)?;
            if root.is_none() && !old.states.is_empty() {
                return Err(ChatError::State);
            }
            let pending = match r.u8()? {
                0 => None,
                1 => Some(r.id()?),
                _ => return Err(ChatError::State),
            };
            let n_retired = usize::from(r.u8()?);
            if n_retired > MAX_RETIRED {
                return Err(ChatError::State);
            }
            let mut retired = Vec::with_capacity(n_retired);
            for _ in 0..n_retired {
                retired.push(r.id()?);
            }
            let n_gens = usize::from(r.u8()?);
            if !(1..=MAX_GENERATIONS).contains(&n_gens) {
                return Err(ChatError::State);
            }
            let mut gens: Vec<Generation> = Vec::with_capacity(n_gens);
            for _ in 0..n_gens {
                let id = r.id()?;
                let key = r.key()?;
                let epochs = Epochs::import(&mut r, members)?;
                if epochs.highest > u32::from(u16::MAX)
                    || gens.iter().any(|g| g.id == id)
                    || retired.contains(&id)
                {
                    return Err(ChatError::State);
                }
                let mut g = Generation::new(id, Zeroizing::new(*key));
                g.epochs = epochs;
                gens.push(g);
            }
            ChatSession {
                root: root.map(|k| Zeroizing::new(*k)),
                circle_id,
                members,
                my_index,
                old,
                gens,
                pending,
                retired,
                sent_in_current,
                synced: false,
                opened_in: None,
            }
        } else {
            return Err(ChatError::State);
        };
        if r.pos != state.len() {
            return Err(ChatError::State);
        }
        s.synced = false;
        Ok(s)
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
    fn id(&mut self) -> Result<[u8; 32], ChatError> {
        let mut id = [0u8; 32];
        id.copy_from_slice(self.take(32)?);
        Ok(id)
    }
    fn key(&mut self) -> Result<Key, ChatError> {
        let b = self.take(KEY_LEN)?;
        let mut k = Zeroizing::new([0u8; KEY_LEN]);
        k.copy_from_slice(b);
        Ok(k)
    }
}
