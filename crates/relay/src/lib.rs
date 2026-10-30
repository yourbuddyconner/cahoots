//! Relay clients for the Numbers Station: the HTTP client, and an in-memory stand-in for tests.
//! Shared by the chat CLI and `cahoots-wg`. A relay carries sealed bytes and sequence numbers,
//! nothing else.
//!
//! A ceremony held through the relay uses the same endpoints: its channel id (16 random bytes,
//! from the host's code) goes where a circle id would, so `post` and `poll` take it as a
//! `CircleId` (see `cahoots_core::channel`).

use cahoots_core::CircleId;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum RelayError {
    Http(String),
    /// The station refused the message (HTTP 409 or 4xx) with this body.
    Refused(String),
}

impl core::fmt::Display for RelayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RelayError::Http(s) => write!(f, "relay: {s}"),
            RelayError::Refused(s) => write!(f, "relay refused: {s}"),
        }
    }
}

pub trait Relay {
    /// Post raw message bytes. Returns the sequence number the relay assigned.
    fn post(&mut self, circle: &CircleId, message: &[u8]) -> Result<u64, RelayError>;
    /// Entries after `after` (all entries when `None`), waiting up to `wait_ms` for one.
    fn poll(
        &mut self,
        circle: &CircleId,
        after: Option<u64>,
        wait_ms: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError>;
}

/// In-memory relay shared between test clients. Sequence numbers are global, like
/// the station's.
#[derive(Default)]
pub struct MemRelay {
    entries: Vec<(CircleId, Vec<u8>)>,
    shared: Option<Arc<Mutex<MemRelay>>>,
}

impl MemRelay {
    pub fn shared(inner: Arc<Mutex<MemRelay>>) -> Self {
        MemRelay {
            entries: Vec::new(),
            shared: Some(inner),
        }
    }
    pub fn entry(&self, seq: u64) -> Option<Vec<u8>> {
        self.entries.get(seq as usize).map(|(_, m)| m.clone())
    }
}

impl Relay for MemRelay {
    fn post(&mut self, circle: &CircleId, message: &[u8]) -> Result<u64, RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().post(circle, message);
        }
        self.entries.push((*circle, message.to_vec()));
        Ok(self.entries.len() as u64 - 1)
    }
    fn poll(
        &mut self,
        circle: &CircleId,
        after: Option<u64>,
        wait_ms: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().poll(circle, after, 0);
        }
        // An in-memory relay never waits: everything posted is already here.
        let _ = wait_ms;
        let start = after.map(|a| a as usize + 1).unwrap_or(0);
        Ok(self
            .entries
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, (c, _))| c == circle)
            .map(|(i, (_, m))| (i as u64, m.clone()))
            .collect())
    }
}

pub struct HttpRelay {
    base: String,
    agent: ureq::Agent,
}

impl HttpRelay {
    pub fn new(base: &str) -> Self {
        HttpRelay {
            base: base.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout_read(core::time::Duration::from_secs(40))
                .build(),
        }
    }
}

impl Relay for HttpRelay {
    fn post(&mut self, circle: &CircleId, message: &[u8]) -> Result<u64, RelayError> {
        let url = format!("{}/v1/circles/{}/messages", self.base, hex::encode(circle));
        match self
            .agent
            .post(&url)
            .set("content-type", "application/octet-stream")
            .send_bytes(message)
        {
            Ok(resp) => {
                let v: serde_json::Value = resp
                    .into_json()
                    .map_err(|e| RelayError::Http(e.to_string()))?;
                v["seq"]
                    .as_u64()
                    .ok_or_else(|| RelayError::Http("no seq in response".into()))
            }
            Err(ureq::Error::Status(_, resp)) => {
                Err(RelayError::Refused(resp.into_string().unwrap_or_default()))
            }
            Err(e) => Err(RelayError::Http(e.to_string())),
        }
    }

    fn poll(
        &mut self,
        circle: &CircleId,
        after: Option<u64>,
        wait_ms: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError> {
        use base64::Engine;
        let mut url = format!(
            "{}/v1/circles/{}/messages?wait={wait_ms}",
            self.base,
            hex::encode(circle)
        );
        if let Some(a) = after {
            url.push_str(&format!("&after={a}"));
        }
        let v: serde_json::Value = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| RelayError::Http(e.to_string()))?
            .into_json()
            .map_err(|e| RelayError::Http(e.to_string()))?;
        let mut out = Vec::new();
        for e in v["entries"].as_array().into_iter().flatten() {
            let seq = e["seq"]
                .as_u64()
                .ok_or_else(|| RelayError::Http("entry seq".into()))?;
            let msg = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(e["message"].as_str().unwrap_or(""))
                .map_err(|e| RelayError::Http(e.to_string()))?;
            out.push((seq, msg));
        }
        Ok(out)
    }
}
