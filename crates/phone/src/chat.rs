//! One circle's chat on the phone: the session, the outbox, the relay cursor and the history.
//!
//! The relay is the station's HTTP interface, reached through Swift's `Http` callback: Rust
//! forms the URLs and reads the bodies as `cahoots_relay::HttpRelay` does. `GET
//! {relay}/v1/circles/{id}/messages?wait=0&after={seq}` answers `{"entries": [{"seq",
//! "message"}]}` with the message in base64url, and `POST` to the same path answers
//! `{"seq"}` with a 201. Swift returns an error for any other status.
//!
//! The order of writes keeps the chat crate's rule: the session state is saved before a
//! message leaves, and a session read from disk sends nothing until the relay has been read
//! to the end, its own messages included.

use crate::store::{
    self, CardsRecord, ChatRecord, CircleRecord, CursorRecord, MessageRecord, OutboxEntry,
    SettingsRecord,
};
use crate::{ChatMessage, Http, PhoneError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_chat::bump::{sign_bump, verify_bump};
use cahoots_chat::{bucket_pad, bucket_unpad, BumpOutcome, ChatError, ChatSession, MessageType};
use cahoots_core::params::CircleParams;
use std::path::{Path, PathBuf};

/// The relay the app ships with.
pub const DEFAULT_RELAY: &str = "https://relay.cahoots.network";

pub(crate) fn chat_err(e: ChatError) -> PhoneError {
    PhoneError::Chat {
        message: match e {
            ChatError::NotSynced => "Still reading the relay. Try again in a moment.".into(),
            ChatError::CannotBump => "This phone cannot reset the keys.".into(),
            ChatError::State => "The saved chat data for this circle is damaged or cannot be read.".into(),
            other => format!("The message could not be read ({other:?})."),
        },
    }
}

fn network(message: impl Into<String>) -> PhoneError {
    PhoneError::Network {
        message: message.into(),
    }
}

/// A circle held by this phone, loaded.
pub struct Circle {
    pub dir: PathBuf,
    pub id_hex: String,
    pub circle_id: [u8; 16],
    pub params: CircleParams,
    pub record: CircleRecord,
    pub session: ChatSession,
    pub chat: ChatRecord,
    pub cursor: CursorRecord,
    pub settings: SettingsRecord,
    /// The cards read from the chat, and this seat's own.
    pub cards: CardsRecord,
    /// For a card circle: generation keys a tap fetched for generations the chat has not
    /// reached yet, by id. Kept in memory only, and used when a bump moves the chat there.
    pub ahead: Vec<([u8; 32], zeroize::Zeroizing<[u8; 32]>)>,
}

/// What a card circle says when it cannot go on without a tap.
pub const NEEDS_TAP: &str = "Tap your card to catch up.";

impl Circle {
    pub fn load(dir: &Path) -> Result<Circle, PhoneError> {
        let record = CircleRecord::load(dir)?;
        let unreadable = || PhoneError::Storage {
            message: "The saved data for this circle is damaged or cannot be read.".into(),
        };
        let params = CircleParams::decode(&record.params).map_err(|_| unreadable())?;
        let circle_id: [u8; 16] = record.circle_id[..].try_into().map_err(|_| unreadable())?;
        let chat = ChatRecord::load(dir)?;
        let session = if let Some(card) = &record.card {
            // A card circle holds no root: one generation's key, from a tap.
            if chat.state.is_empty() {
                let key = card.generation_key.ok_or_else(unreadable)?;
                ChatSession::from_generation(
                    card.generation,
                    zeroize::Zeroizing::new(key),
                    circle_id,
                    params.n,
                    record.seat,
                )
            } else {
                ChatSession::import_generation_state(circle_id, params.n, record.seat, &chat.state)
                    .map_err(chat_err)?
            }
        } else {
            let root: zeroize::Zeroizing<[u8; 32]> =
                zeroize::Zeroizing::new(record.root[..].try_into().map_err(|_| unreadable())?);
            if chat.state.is_empty() {
                ChatSession::new(&root, circle_id, params.n, record.seat)
            } else {
                ChatSession::import_state(&root, circle_id, params.n, record.seat, &chat.state)
                    .map_err(chat_err)?
            }
        };
        let mut cursor: CursorRecord =
            store::load_if_present(&dir.join(store::CURSOR))?.unwrap_or_default();
        // A state from before generations: the build that wrote it read past messages it could
        // not open, a generation's and a reset among them. Read the relay again from its start.
        // What was read already comes back as replays and is passed over.
        if chat.state.starts_with(ChatSession::STATE_MAGIC_ROOT_EPOCHS) && cursor.after.is_some()
        {
            cursor = CursorRecord::default();
            cursor.save(dir)?;
        }
        let settings =
            store::load_if_present(&dir.join(store::SETTINGS))?.unwrap_or(SettingsRecord {
                relay: DEFAULT_RELAY.into(),
                unread: 0,
                face_override: None,
            });
        let cards = CardsRecord::load(dir)?;
        let mut circle = Circle {
            dir: dir.to_path_buf(),
            id_hex: hex::encode(circle_id),
            circle_id,
            params,
            record,
            session,
            chat,
            cursor,
            settings,
            cards,
            ahead: Vec::new(),
        };
        circle.copy_journal()?;
        Ok(circle)
    }

    /// A card circle whose chat waits for a generation key only a tap can give: a bump moved
    /// the circle on, and this phone holds no root to follow it.
    pub fn needs_tap(&self) -> bool {
        self.record.is_card() && self.session.pending_generation().is_some()
    }

    /// The generations a tap should fetch keys for: the one the chat waits for, and the ones
    /// the bumps still to post move to.
    pub fn wanted_generations(&self) -> Vec<[u8; 32]> {
        let mut out: Vec<[u8; 32]> = self.session.pending_generation().into_iter().collect();
        for b in self.record.card.iter().flat_map(|c| c.forward.iter()) {
            if let Some(to) = b.get(32..64).and_then(|t| <[u8; 32]>::try_from(t).ok()) {
                if !out.contains(&to) {
                    out.push(to);
                }
            }
        }
        out
    }

    /// Install the key a tap fetched for generation `id`, if there is one. Returns whether it
    /// was installed.
    pub fn install_ahead(&mut self, id: [u8; 32]) -> bool {
        let Some(i) = self.ahead.iter().position(|(g, _)| *g == id) else {
            return false;
        };
        let (_, key) = self.ahead.remove(i);
        self.session.install_generation(id, key);
        true
    }

    /// Keys a tap fetched: the one the chat waits for is installed now and saved, the rest
    /// are kept for the bumps that lead to them.
    pub fn take_keys(
        &mut self,
        keys: Vec<([u8; 32], zeroize::Zeroizing<[u8; 32]>)>,
    ) -> Result<(), PhoneError> {
        for (id, key) in keys {
            self.ahead.retain(|(g, _)| *g != id);
            self.ahead.push((id, key));
        }
        if let Some(p) = self.session.pending_generation() {
            if self.install_ahead(p) {
                self.save_chat()?;
            }
        }
        Ok(())
    }

    /// Post the bumps a badge signed at an unpair that this phone forwards, in order, once
    /// the relay has been read to the end. Each is sealed in the generation it moves from,
    /// and the chat then waits for the key of the one it moves to: from the keys a tap
    /// fetched, or from the next tap. A bump that moves from a generation the chat is not in
    /// is stale (another device posted it, or the circle moved on) and is dropped.
    fn forward_bumps(&mut self) -> Result<(), PhoneError> {
        let Some(list) = self.record.card.as_ref().map(|c| c.forward.clone()) else {
            return Ok(());
        };
        if list.is_empty() || !self.session.synced() {
            return Ok(());
        }
        let mut done = 0;
        let mut lines = Vec::new();
        for payload in &list {
            let ends = payload
                .get(..32)
                .and_then(|f| <[u8; 32]>::try_from(f).ok())
                .zip(payload.get(32..64).and_then(|t| <[u8; 32]>::try_from(t).ok()));
            let Some((from, to)) = ends else {
                done += 1;
                continue;
            };
            match self.session.pending_generation() {
                // Read from the relay already: someone posted it.
                Some(p) if p == to => {}
                // The chat waits for another key: this bump waits for it too.
                Some(_) => break,
                None if self.session.generation_id() == from => {
                    match self.session.send_bump(to, payload) {
                        Ok(wire) => {
                            self.chat.outbox.push(OutboxEntry {
                                id: random_id()?,
                                wire: wire.into(),
                            });
                            let seat = self.record.seat;
                            lines.push(MessageRecord {
                                id: random_id()?,
                                sender: seat,
                                mine: true,
                                text: bump_text(seat),
                                epoch: 0,
                                seq: None,
                                kind: Some("bump".into()),
                            });
                        }
                        Err(ChatError::CannotBump | ChatError::BadSignature) => {
                            done += 1;
                            continue;
                        }
                        Err(e) => return Err(chat_err(e)),
                    }
                }
                None => {
                    done += 1;
                    continue;
                }
            }
            done += 1;
            if !self.install_ahead(to) {
                break;
            }
        }
        // The chat state and outbox first, then the list: a crash between them leaves a bump
        // listed that the chat has sent, which the next pass finds stale or waiting.
        self.save_with(lines)?;
        if let Some(card) = self.record.card.as_mut() {
            card.forward.drain(..done.min(card.forward.len()));
        }
        self.record.save(&self.dir)
    }

    pub(crate) fn save_chat(&mut self) -> Result<(), PhoneError> {
        self.chat.state = self.session.export_state().to_vec().into();
        self.chat.save(&self.dir)?;
        // A card circle's first key is in the saved state now: the record need not keep it,
        // so a lost phone holds only the keys the chat still keeps.
        if let Some(card) = self.record.card.as_mut() {
            if let Some(mut k) = card.generation_key.take() {
                zeroize::Zeroize::zeroize(&mut k);
                self.record.save(&self.dir)?;
            }
        }
        Ok(())
    }

    /// Save the chain state and outbox with the history lines they made, in one write, then
    /// copy the lines to `messages.log`.
    fn save_with(&mut self, lines: Vec<MessageRecord>) -> Result<(), PhoneError> {
        self.chat.journal.extend(lines);
        self.save_chat()?;
        self.copy_journal()
    }

    /// Copy the journal's lines that `messages.log` lacks, then empty the journal.
    fn copy_journal(&mut self) -> Result<(), PhoneError> {
        if self.chat.journal.is_empty() {
            return Ok(());
        }
        let held: Vec<String> = store::read_messages(&self.dir)?
            .into_iter()
            .map(|m| m.id)
            .collect();
        for m in &self.chat.journal {
            if !held.contains(&m.id) {
                store::append_message(&self.dir, m)?;
            }
        }
        self.chat.journal.clear();
        self.chat.save(&self.dir)
    }

    fn messages_url(&self) -> String {
        format!(
            "{}/v1/circles/{}/messages",
            self.settings.relay.trim_end_matches('/'),
            self.id_hex
        )
    }

    /// Read the relay after the cursor to the end, decrypt, store, and return what others
    /// sent. Then post whatever waits in the outbox; a failure there leaves it waiting.
    pub fn poll(&mut self, http: &dyn Http) -> Result<Vec<ChatMessage>, PhoneError> {
        let me = self.record.seat;
        let mut new = Vec::new();
        loop {
            let mut url = format!("{}?wait=0", self.messages_url());
            if let Some(a) = self.cursor.after {
                url.push_str(&format!("&after={a}"));
            }
            let entries = parse_entries(&http.get(url)?)?;
            let fresh: Vec<_> = entries
                .into_iter()
                .filter(|(seq, _)| self.cursor.after.is_none_or(|a| *seq > a))
                .collect();
            if fresh.is_empty() {
                self.session.mark_synced();
                break;
            }
            let mut lines = Vec::new();
            let mut cards = false;
            let mut stalled = false;
            for (seq, bytes) in fresh {
                // Our own post that landed though its answer was lost: it is not sent again.
                if let Some(i) = self.chat.outbox.iter().position(|e| e.wire[..] == bytes[..]) {
                    self.chat.outbox.remove(i);
                }
                // Our own bump, read back: the relay has it, so it is not posted again.
                if self.chat.pending_bump.as_ref().is_some_and(|w| w[..] == bytes[..]) {
                    self.chat.pending_bump = None;
                }
                // Our own messages move our sending position; a message that does not open
                // (damaged, replayed, another kind) is passed over. A card circle that meets
                // a message in the generation it waits for stops here, without passing it,
                // until a tap gives the key. One in a generation it dropped is passed over
                // like any other that does not open: no tap gives that key back.
                let mut opened = self.session.receive(&bytes);
                if let Err(ChatError::NeedsGeneration(id)) = opened {
                    if self.record.is_card() && self.session.pending_generation() == Some(id) {
                        if !self.install_ahead(id) {
                            stalled = true;
                            break;
                        }
                        opened = self.session.receive(&bytes);
                    }
                }
                if let Ok((h, pt)) = opened {
                    // A bump counts only with a seat's signature from the manifest; a bad or
                    // stale one is passed over without a word. Our own is followed too, so a
                    // phone restored from older state moves with its own reset. Its line is
                    // in the history from when it was sent.
                    if h.msg_type == MessageType::Bump {
                        let moved = self.apply_bump(&pt);
                        if let Some(seat) = moved.filter(|_| h.sender != me) {
                            let m = MessageRecord {
                                id: format!("relay-{seq}"),
                                sender: seat,
                                mine: false,
                                text: bump_text(seat),
                                epoch: 0,
                                seq: Some(seq),
                                kind: Some("bump".into()),
                            };
                            new.push(to_message(&m, false));
                            lines.push(m);
                        }
                    }
                    // A card is kept for its seat when it verifies, and is never history.
                    if h.msg_type == MessageType::Card && h.sender != me {
                        let signed = bucket_unpad(&pt).unwrap_or(&[]);
                        if self.verify_card(signed, h.sender) {
                            cards |= self.cards.put(h.sender, signed.to_vec(), Some(seq));
                        }
                    }
                    // Only text is history. Bumps and cards are handled above.
                    if h.sender != me && h.msg_type == MessageType::Text {
                        let text = String::from_utf8_lossy(bucket_unpad(&pt).unwrap_or(&pt))
                            .into_owned();
                        let m = MessageRecord {
                            id: format!("relay-{seq}"),
                            sender: h.sender,
                            mine: false,
                            text,
                            epoch: epoch_number(h.epoch),
                            seq: Some(seq),
                            kind: None,
                        };
                        self.settings.unread = self.settings.unread.saturating_add(1);
                        new.push(to_message(&m, false));
                        lines.push(m);
                    }
                }
                self.cursor.after = Some(seq);
            }
            // The chain state with its history lines first, then the cursor. Read again after
            // a crash, an entry is passed over as a replay, and its line is already kept.
            let any = !lines.is_empty();
            if cards {
                self.cards.save(&self.dir)?;
            }
            self.save_with(lines)?;
            self.cursor.save(&self.dir)?;
            if any {
                self.settings.save(&self.dir)?;
            }
            if stalled {
                // What was read is kept and what waits is posted. The rest needs a tap.
                let _ = self.flush(http);
                return Ok(new);
            }
        }
        // Read to the end: a card circle posts the bumps it forwards, and a card waiting for
        // that can be sealed now.
        self.forward_bumps()?;
        let _ = self.queue_card();
        let _ = self.flush(http);
        Ok(new)
    }

    /// Post the outbox in order. An entry leaves it only when the relay answers with its
    /// sequence number.
    pub fn flush(&mut self, http: &dyn Http) -> Result<(), PhoneError> {
        // A reset goes first: what follows it is sealed in the generation it makes.
        if let Some(wire) = self.chat.pending_bump.clone() {
            post(http, self.messages_url(), wire.to_vec())?;
            self.chat.pending_bump = None;
            self.save_chat()?;
        }
        while let Some(entry) = self.chat.outbox.first().cloned() {
            post(http, self.messages_url(), entry.wire.to_vec())?;
            self.chat.outbox.retain(|e| e.id != entry.id);
            self.save_chat()?;
        }
        Ok(())
    }

    /// Encrypt `text`, keep it in the outbox and the history, and post it. A message the
    /// relay cannot take now waits in the outbox, and the next poll sends it.
    pub fn send(&mut self, http: &dyn Http, text: &str) -> Result<(), PhoneError> {
        if !self.session.synced() {
            self.poll(http)?;
        }
        if self.needs_tap() {
            return Err(PhoneError::Chat {
                message: NEEDS_TAP.into(),
            });
        }
        let padded = bucket_pad(text.as_bytes()).ok_or_else(|| PhoneError::Chat {
            message: "That message is too long.".into(),
        })?;
        let wire = self.session.send(&padded).map_err(chat_err)?;
        let id = random_id()?;
        self.chat.outbox.push(OutboxEntry {
            id: id.clone(),
            wire: wire.into(),
        });
        let m = MessageRecord {
            id,
            sender: self.record.seat,
            mine: true,
            text: text.into(),
            epoch: self
                .session
                .current_epoch()
                .map_or(0, epoch_number),
            seq: None,
            kind: None,
        };
        // The chain step, the ciphertext and the history line are on disk in one write
        // before anything leaves.
        self.save_with(vec![m])?;
        let _ = self.flush(http);
        Ok(())
    }

    /// Publish this seat's signed card in the chat: keep it as this seat's card, seal it as a
    /// card message once the relay has been read to the end, and post it through the outbox.
    /// A relay that cannot be read or posted to now leaves it waiting for the next poll.
    pub fn publish_card(&mut self, http: &dyn Http, card: Vec<u8>) -> Result<(), PhoneError> {
        if bucket_pad(&card).is_none() {
            return Err(PhoneError::Chat {
                message: "The card is too long.".into(),
            });
        }
        self.cards.put(self.record.seat, card.clone(), None);
        self.cards.save(&self.dir)?;
        self.chat.pending_card = Some(card.into());
        self.chat.card_due = false;
        self.save_chat()?;
        if self.session.synced() {
            self.queue_card()?;
            let _ = self.flush(http);
        } else {
            let _ = self.poll(http);
        }
        Ok(())
    }

    /// Seal the waiting card into the outbox. The session must have read the relay to the end.
    fn queue_card(&mut self) -> Result<(), PhoneError> {
        let Some(card) = self.chat.pending_card.clone() else {
            return Ok(());
        };
        let id = random_id()?;
        // Padded as a text is, so the length of the name does not show.
        let padded = bucket_pad(&card).ok_or_else(|| PhoneError::Chat {
            message: "The card is too long.".into(),
        })?;
        let wire = self
            .session
            .send_typed(MessageType::Card, &padded)
            .map_err(chat_err)?;
        self.chat.outbox.push(OutboxEntry {
            id,
            wire: wire.into(),
        });
        self.chat.pending_card = None;
        self.save_chat()
    }

    /// A card read from the chat counts when a seat of the manifest signed it, under its own
    /// seat as kid, for this circle, and the card names the seat that sent the message.
    fn verify_card(&self, signed: &[u8], sender: u8) -> bool {
        let Ok(sealed) = cahoots_core::manifest::SealedManifest::decode(&self.record.manifest)
        else {
            return false;
        };
        let Ok(manifest) = cahoots_core::manifest::Manifest::decode(&sealed.body) else {
            return false;
        };
        cahoots_objects::Card::verify(signed, &manifest).is_ok_and(|c| c.seat == sender)
    }

    /// Mark this seat's card as due to be signed and published again, or no longer due.
    pub fn set_card_due(&mut self, due: bool) -> Result<(), PhoneError> {
        if self.chat.card_due != due {
            self.chat.card_due = due;
            self.save_chat()?;
        }
        Ok(())
    }

    /// The card held for `seat`, decoded. It verified when it was kept.
    pub fn card(&self, seat: u8) -> Option<cahoots_objects::Card> {
        let entry = self.cards.get(seat)?;
        let payload = cahoots_core::cose::payload1(&entry.card).ok()?;
        cahoots_objects::Card::decode(&payload).ok()
    }

    /// Verify a bump just read from the relay against the manifest and follow it. Returns the
    /// seat that signed it when this phone moved, or `None` when it does not verify, is stale
    /// (it moves from a generation this phone is not in), lost to a bump made at the same
    /// time, or cannot be followed. When this phone's own unposted reset lost, it is dropped.
    fn apply_bump(&mut self, payload: &[u8]) -> Option<u8> {
        let sealed = cahoots_core::manifest::SealedManifest::decode(&self.record.manifest).ok()?;
        let manifest = cahoots_core::manifest::Manifest::decode(&sealed.body).ok()?;
        let (from, to, seat) = verify_bump(payload, &self.circle_id, &manifest).ok()?;
        match self.session.apply_bump(from, to) {
            Ok(BumpOutcome::Moved { dropped_own }) => {
                // Our own reset lost to one made at the same time: it is not posted.
                if dropped_own {
                    self.chat.pending_bump = None;
                }
                Some(seat)
            }
            Ok(BumpOutcome::Stale) => None,
            // A card circle holds no root: it moves once it has the key, from a tap that
            // fetched it ahead or from the next one.
            Err(ChatError::NeedsGeneration(id)) => {
                self.install_ahead(id);
                Some(seat)
            }
            Err(_) => None,
        }
    }

    /// Move everyone in the circle to new keys: sign a bump with this seat's key, apply it,
    /// keep its wire and post it. A relay that does not answer leaves it waiting.
    pub fn bump_generation(
        &mut self,
        http: &dyn Http,
        signer: &dyn crate::Signer,
    ) -> Result<(), PhoneError> {
        if self.chat.pending_bump.is_some() {
            let _ = self.flush(http);
            if self.chat.pending_bump.is_some() {
                return Err(PhoneError::Chat {
                    message: "A reset is still waiting for the relay.".into(),
                });
            }
        }
        if !self.session.synced() {
            self.poll(http)?;
        }
        let mut id = [0u8; 32];
        getrandom::getrandom(&mut id).map_err(|e| PhoneError::Storage {
            message: format!("No randomness: {e}"),
        })?;
        let public = cahoots_core::seat::SeatPublic::parse(&self.record.seat_public).map_err(|_| {
            PhoneError::Storage {
                message: "The saved data for this circle is damaged or cannot be read.".into(),
            }
        })?;
        let mut seat = crate::signer::Seat {
            signer,
            label: self.record.seat_label.clone(),
            public,
            error: None,
        };
        let from = self.session.generation_id();
        let payload = sign_bump(&mut seat, self.record.seat, &self.circle_id, &from, &id)
            .map_err(chat_err)?;
        if let Some(e) = seat.error.take() {
            return Err(e);
        }
        let wire = self.session.send_bump(id, &payload).map_err(chat_err)?;
        self.chat.pending_bump = Some(wire.into());
        let seat_no = self.record.seat;
        let m = MessageRecord {
            id: random_id()?,
            sender: seat_no,
            mine: true,
            text: bump_text(seat_no),
            epoch: 0,
            seq: None,
            kind: Some("bump".into()),
        };
        self.save_with(vec![m])?;
        let _ = self.flush(http);
        Ok(())
    }

    pub fn rotate(&mut self) -> Result<(), PhoneError> {
        self.session.start_epoch();
        self.save_chat()
    }

    pub fn history(&mut self) -> Vec<ChatMessage> {
        let all = store::read_messages(&self.dir).unwrap_or_default();
        if self.settings.unread != 0 {
            self.settings.unread = 0;
            let _ = self.settings.save(&self.dir);
        }
        all.iter()
            .map(|m| {
                let pending = self.chat.outbox.iter().any(|e| e.id == m.id);
                to_message(m, pending)
            })
            .collect()
    }

    pub fn set_relay(&mut self, url: &str) -> Result<(), PhoneError> {
        let url = check_relay(url)?;
        if url != self.settings.relay {
            // Another relay counts its entries from its own start.
            self.settings.relay = url;
            self.settings.save(&self.dir)?;
            self.cursor = CursorRecord::default();
            self.cursor.save(&self.dir)?;
            // The cards held were read at the old relay's positions, which mean nothing on the
            // new one: any card read there replaces them.
            for c in &mut self.cards.cards {
                c.seq = None;
            }
            self.cards.save(&self.dir)?;
            // The new relay has not seen this seat's card: post it again with the next poll.
            if let Some(own) = self.cards.get(self.record.seat) {
                self.chat.pending_card = Some(own.card.clone());
                self.save_chat()?;
            }
        }
        Ok(())
    }
}

/// A relay address as set: trimmed, without a trailing slash, starting with https:// or http://.
pub(crate) fn check_relay(url: &str) -> Result<String, PhoneError> {
    let bad = || network("A relay address is https:// and a host name, with nothing after it.");
    let parsed = url::Url::parse(url.trim()).map_err(|_| bad())?;
    let plain = matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|h| !h.is_empty())
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none();
    if !plain {
        return Err(bad());
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn to_message(m: &MessageRecord, pending: bool) -> ChatMessage {
    ChatMessage {
        id: m.id.clone(),
        sender: m.sender,
        mine: m.mine,
        text: m.text.clone(),
        epoch: m.epoch,
        pending,
        kind: m.kind.clone().unwrap_or_else(|| "text".into()),
    }
}

fn bump_text(seat: u8) -> String {
    format!("Keys were reset by seat {}.", u16::from(seat) + 1)
}

/// Post `body` and check the relay answered with a sequence number.
fn post(http: &dyn Http, url: String, body: Vec<u8>) -> Result<(), PhoneError> {
    let answer = http.post(url, body)?;
    let v: serde_json::Value = serde_json::from_slice(&answer)
        .map_err(|_| network("The relay returned an unreadable response. Try again in a moment."))?;
    v["seq"]
        .as_u64()
        .ok_or_else(|| network("The relay did not confirm that the message was saved."))?;
    Ok(())
}

pub(crate) fn random_id() -> Result<String, PhoneError> {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).map_err(|e| PhoneError::Storage {
        message: format!("No randomness: {e}"),
    })?;
    Ok(hex::encode(b))
}

pub(crate) fn parse_entries(body: &[u8]) -> Result<Vec<(u64, Vec<u8>)>, PhoneError> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| network("The relay returned an unreadable response. Try again in a moment."))?;
    let mut out = Vec::new();
    for e in v["entries"].as_array().into_iter().flatten() {
        let seq = e["seq"]
            .as_u64()
            .ok_or_else(|| network("The relay sent an entry without a number."))?;
        let msg = URL_SAFE_NO_PAD
            .decode(e["message"].as_str().unwrap_or(""))
            .map_err(|_| network("The relay sent an entry that does not read."))?;
        out.push((seq, msg));
    }
    Ok(out)
}

/// The number of an epoch: `n` in a generation, or an old root epoch's number. 0 for neither.
fn epoch_number(id: cahoots_chat::EpochId) -> u32 {
    id.generation_parts()
        .map(|(_, n)| u32::from(n))
        .or(id.number())
        .unwrap_or(0)
}
