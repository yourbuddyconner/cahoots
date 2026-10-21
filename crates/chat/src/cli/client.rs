//! The send/receive logic of the demo client, free of threads and stdin so tests can
//! drive it. `Client::poll` pulls relay entries after the last seen sequence number and
//! turns each into an `Event`. `send_line` seals and posts. State is persisted after
//! every send and every processed entry.

use super::relay::{Relay, RelayError};
use super::store::{self, CircleChat, OpenedCircle, StoreError};
use crate::{bucket_pad, bucket_unpad, parse_header, ChatError, EpochId, Header};
use cahoots_core::CircleId;
use std::path::{Path, PathBuf};
use std::string::{String, ToString};
use std::vec::Vec;
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// One of our own messages came back from the relay.
    Own {
        seq: u64,
    },
    Received {
        seq: u64,
        sender: u8,
        text: String,
    },
    Rejected {
        seq: u64,
        error: ChatError,
    },
    /// Decrypted, but the plaintext was not a padded UTF-8 line.
    Unreadable {
        seq: u64,
        sender: u8,
    },
}

#[derive(Debug)]
pub enum ClientError {
    Store(StoreError),
    Relay(RelayError),
    Chat(ChatError),
    LineTooLong,
}

impl From<StoreError> for ClientError {
    fn from(e: StoreError) -> Self {
        ClientError::Store(e)
    }
}
impl From<RelayError> for ClientError {
    fn from(e: RelayError) -> Self {
        ClientError::Relay(e)
    }
}
impl From<ChatError> for ClientError {
    fn from(e: ChatError) -> Self {
        ClientError::Chat(e)
    }
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClientError::Store(e) => write!(f, "{e}"),
            ClientError::Relay(e) => write!(f, "{e}"),
            ClientError::Chat(e) => write!(f, "chat: {e:?}"),
            ClientError::LineTooLong => write!(f, "line longer than the largest bucket"),
        }
    }
}

pub struct Client<R: Relay> {
    circle_dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
    circle_id: CircleId,
    my_index: u8,
    label: String,
    session: CircleChat,
    relay: R,
    last_seq: Option<u64>,
    rotate_after: u32,
    sent_in_epoch: u32,
}

impl<R: Relay> Client<R> {
    /// Open a member directory and resume any persisted chat state.
    pub fn open(member_dir: &Path, relay: R, rotate_after: u32) -> Result<Self, ClientError> {
        let OpenedCircle {
            dir,
            key,
            circle_id,
            my_slice,
            label,
            pad,
            ..
        } = store::open_circle(member_dir)?;
        let session = match store::load_chat_state(&dir, &key)? {
            Some(state) => CircleChat::import_state(pad, my_slice, &state)?,
            None => CircleChat::new(pad, my_slice),
        };
        let last_seq = store::load_seq(&dir)?;
        Ok(Client {
            circle_dir: dir,
            key,
            circle_id,
            my_index: my_slice,
            label,
            session,
            relay,
            last_seq,
            rotate_after: rotate_after.max(1),
            sent_in_epoch: 0,
        })
    }

    pub fn circle_id(&self) -> CircleId {
        self.circle_id
    }
    pub fn my_index(&self) -> u8 {
        self.my_index
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn last_seq(&self) -> Option<u64> {
        self.last_seq
    }
    pub fn current_epoch(&self) -> Option<EpochId> {
        self.session.current_epoch()
    }
    pub fn remaining(&self) -> u32 {
        self.session.pad().remaining()
    }

    fn persist(&mut self) -> Result<(), ClientError> {
        let state = self.session.export_state();
        store::save_chat_state(&self.circle_dir, &self.key, &state)?;
        store::save_seq(&self.circle_dir, self.last_seq)?;
        Ok(())
    }

    fn seal_line(&mut self, text: &str) -> Result<Vec<u8>, ClientError> {
        if self.session.current_epoch().is_none() || self.sent_in_epoch >= self.rotate_after {
            self.session.start_epoch()?;
            self.sent_in_epoch = 0;
        }
        let padded = bucket_pad(text.as_bytes()).ok_or(ClientError::LineTooLong)?;
        let wire = self.session.send(&padded)?;
        self.sent_in_epoch += 1;
        Ok(wire)
    }

    /// Seal `text`, persist, then post. Returns the relay sequence number.
    pub fn send_line(&mut self, text: &str) -> Result<u64, ClientError> {
        let wire = self.seal_line(text)?;
        self.persist()?;
        Ok(self.relay.post(&self.circle_id, &wire)?)
    }

    /// For the demo: seal a real message, flip one ciphertext byte, and post it.
    /// Every reader must reject it with a tag failure.
    pub fn post_tampered(&mut self, text: &str) -> Result<u64, ClientError> {
        let mut wire = self.seal_line(text)?;
        self.persist()?;
        let last = wire.len() - 1;
        wire[last] ^= 0x01;
        Ok(self.relay.post(&self.circle_id, &wire)?)
    }

    /// Process one relay entry without touching `last_seq` or persisting.
    pub fn handle_entry(&mut self, seq: u64, message: &[u8]) -> Event {
        if let Ok(h) = parse_header(message) {
            if h.sender == self.my_index && h.circle_id == self.circle_id {
                return Event::Own { seq };
            }
        }
        match self.session.receive(message) {
            Ok((Header { sender, .. }, pt)) => {
                match bucket_unpad(&pt).and_then(|b| core::str::from_utf8(b).ok()) {
                    Some(text) => Event::Received {
                        seq,
                        sender,
                        text: text.to_string(),
                    },
                    None => Event::Unreadable { seq, sender },
                }
            }
            Err(error) => Event::Rejected { seq, error },
        }
    }

    /// Fetch everything after the last seen entry, process it, persist, and return
    /// the events in order.
    pub fn poll(&mut self, wait_ms: u64) -> Result<Vec<Event>, ClientError> {
        let entries = self.relay.poll(&self.circle_id, self.last_seq, wait_ms)?;
        let mut events = Vec::with_capacity(entries.len());
        for (seq, msg) in entries {
            events.push(self.handle_entry(seq, &msg));
            self.last_seq = Some(seq);
        }
        if !events.is_empty() {
            self.persist()?;
        }
        Ok(events)
    }
}
