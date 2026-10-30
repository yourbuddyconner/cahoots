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
    self, ChatRecord, CircleRecord, CursorRecord, MessageRecord, OutboxEntry, SettingsRecord,
};
use crate::{ChatMessage, Http, PhoneError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_chat::{bucket_pad, bucket_unpad, ChatError, ChatSession};
use cahoots_core::params::CircleParams;
use std::path::{Path, PathBuf};

/// The relay the app ships with.
pub const DEFAULT_RELAY: &str = "https://relay.cahoots.network";

pub(crate) fn chat_err(e: ChatError) -> PhoneError {
    PhoneError::Chat {
        message: match e {
            ChatError::NotSynced => "Still reading the relay. Try again in a moment.".into(),
            ChatError::State => "This circle's chat state does not read.".into(),
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
}

impl Circle {
    pub fn load(dir: &Path) -> Result<Circle, PhoneError> {
        let record = CircleRecord::load(dir)?;
        let unreadable = || PhoneError::Storage {
            message: "This circle's record does not read.".into(),
        };
        let params = CircleParams::decode(&record.params).map_err(|_| unreadable())?;
        let circle_id: [u8; 16] = record.circle_id[..].try_into().map_err(|_| unreadable())?;
        let root: zeroize::Zeroizing<[u8; 32]> =
            zeroize::Zeroizing::new(record.root[..].try_into().map_err(|_| unreadable())?);
        let chat = ChatRecord::load(dir)?;
        let session = if chat.state.is_empty() {
            ChatSession::new(&root, circle_id, params.n, record.seat)
        } else {
            ChatSession::import_state(&root, circle_id, params.n, record.seat, &chat.state)
                .map_err(chat_err)?
        };
        let cursor = store::load_if_present(&dir.join(store::CURSOR))?.unwrap_or_default();
        let settings =
            store::load_if_present(&dir.join(store::SETTINGS))?.unwrap_or(SettingsRecord {
                relay: DEFAULT_RELAY.into(),
                unread: 0,
            });
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
        };
        circle.copy_journal()?;
        Ok(circle)
    }

    fn save_chat(&mut self) -> Result<(), PhoneError> {
        self.chat.state = self.session.export_state().to_vec().into();
        self.chat.save(&self.dir)
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
            for (seq, bytes) in fresh {
                // Our own post that landed though its answer was lost: it is not sent again.
                if let Some(i) = self.chat.outbox.iter().position(|e| e.wire[..] == bytes[..]) {
                    self.chat.outbox.remove(i);
                }
                // Our own messages move our sending position; a message that does not open
                // (damaged, replayed, another kind) is passed over.
                if let Ok((h, pt)) = self.session.receive(&bytes) {
                    if h.sender != me {
                        let text = String::from_utf8_lossy(bucket_unpad(&pt).unwrap_or(&pt))
                            .into_owned();
                        let m = MessageRecord {
                            id: format!("relay-{seq}"),
                            sender: h.sender,
                            mine: false,
                            text,
                            epoch: h.epoch.number().unwrap_or(0),
                            seq: Some(seq),
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
            self.save_with(lines)?;
            self.cursor.save(&self.dir)?;
            if any {
                self.settings.save(&self.dir)?;
            }
        }
        let _ = self.flush(http);
        Ok(new)
    }

    /// Post the outbox in order. An entry leaves it only when the relay answers with its
    /// sequence number.
    pub fn flush(&mut self, http: &dyn Http) -> Result<(), PhoneError> {
        while let Some(entry) = self.chat.outbox.first().cloned() {
            let body = http.post(self.messages_url(), entry.wire.to_vec())?;
            let v: serde_json::Value = serde_json::from_slice(&body)
                .map_err(|_| network("The relay's answer does not read."))?;
            v["seq"]
                .as_u64()
                .ok_or_else(|| network("The relay did not say where it put the message."))?;
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
                .and_then(|e| e.number())
                .unwrap_or(0),
            seq: None,
        };
        // The chain step, the ciphertext and the history line are on disk in one write
        // before anything leaves.
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
        }
        Ok(())
    }
}

/// A relay address as set: trimmed, without a trailing slash, starting with https:// or http://.
pub(crate) fn check_relay(url: &str) -> Result<String, PhoneError> {
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.contains(' ') {
        return Err(network("A relay address starts with https://."));
    }
    Ok(url.into())
}

fn to_message(m: &MessageRecord, pending: bool) -> ChatMessage {
    ChatMessage {
        id: m.id.clone(),
        sender: m.sender,
        mine: m.mine,
        text: m.text.clone(),
        epoch: m.epoch,
        pending,
    }
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
        serde_json::from_slice(body).map_err(|_| network("The relay's answer does not read."))?;
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
