//! Relay clients for the Numbers Station: the HTTP client, and an in-memory stand-in for tests.
//! Shared by the chat CLI and `cahoots-wg`. A relay carries sealed bytes and sequence numbers,
//! nothing else.
//!
//! A ceremony held through the relay uses the same endpoints: its channel id (16 random bytes,
//! from the host's code) goes where a circle id would, so `post` and `poll` take it as a
//! `CircleId` (see `cahoots_core::channel`).
//!
//! Channels have their own trait, `ChannelRelay`: the manifest, the signed objects a channel
//! keeps, and its messages.

use base64::Engine;
use cahoots_core::CircleId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum RelayError {
    Http(String),
    /// The station refused the message (HTTP 409 or 4xx) with this body.
    Refused(String),
    /// The station wants a gate token or a proof for this read (HTTP 401).
    Gate(String),
    /// The station found no grant that allows this (HTTP 403).
    Grant(String),
}

impl core::fmt::Display for RelayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RelayError::Http(s) => write!(f, "relay: {s}"),
            RelayError::Refused(s) => write!(f, "relay refused: {s}"),
            RelayError::Gate(s) => write!(f, "relay wants a gate token or proof: {s}"),
            RelayError::Grant(s) => write!(f, "relay found no grant: {s}"),
        }
    }
}

/// The kinds of signed object a channel holds besides its messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    Channel,
    Policy,
    Grant,
    Revocation,
    Deposit,
    Request,
    Gate,
}

impl ObjectKind {
    /// The path segment after `/v1/channels/{id}/`; `None` for the channel object itself.
    fn segment(self) -> Option<&'static str> {
        match self {
            ObjectKind::Channel => None,
            ObjectKind::Policy => Some("policy"),
            ObjectKind::Grant => Some("grants"),
            ObjectKind::Revocation => Some("revocations"),
            ObjectKind::Deposit => Some("deposits"),
            ObjectKind::Request => Some("requests"),
            ObjectKind::Gate => Some("gates"),
        }
    }
}

/// What a channel read or post presents to the station. The relay crate does not sign: the
/// caller builds the proof (see `poll_target` and `list_target` for the string it signs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    None,
    /// The channel's gate token, sent as `Cahoots-Gate`.
    Gate([u8; 32]),
    /// A COSE proof and the key id of the member who made it, sent as `Cahoots-Proof` and
    /// `Cahoots-Kid`.
    Proof {
        proof: Vec<u8>,
        kid: [u8; 32],
    },
}

/// Manifests, channel objects and channel messages. Object bodies are the signed bytes the
/// station stores; the relay crate never reads inside them.
pub trait ChannelRelay {
    fn put_manifest(&mut self, circle: &CircleId, sealed: &[u8]) -> Result<(), RelayError>;
    /// `None` when the relay holds no manifest for the circle.
    fn get_manifest(&mut self, circle: &CircleId) -> Result<Option<Vec<u8>>, RelayError>;
    /// Store an object. Returns the id the relay answered with (for a channel, the channel id).
    fn put_object(
        &mut self,
        channel: &[u8; 16],
        kind: ObjectKind,
        body: &[u8],
    ) -> Result<Vec<u8>, RelayError>;
    /// Stored objects of one kind, filtered by the query pairs the route takes (`kid`, `to`,
    /// `epoch`, `after`). For `Channel` the answer is `[channel, policy, manifest]`, with an
    /// empty entry where the station holds none. Revocations come back as their grant ids.
    /// Gates cannot be listed.
    fn list(
        &mut self,
        channel: &[u8; 16],
        kind: ObjectKind,
        query: &[(&str, &str)],
        auth: &Auth,
    ) -> Result<Vec<Vec<u8>>, RelayError>;
    /// Post a channel message. Returns the sequence number the relay assigned.
    fn post_channel(
        &mut self,
        channel: &[u8; 16],
        message: &[u8],
        auth: &Auth,
    ) -> Result<u64, RelayError>;
    fn poll_channel(
        &mut self,
        channel: &[u8; 16],
        after: Option<u64>,
        wait_ms: u64,
        auth: &Auth,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError>;
    /// A fresh single-use nonce for a proof.
    fn nonce(&mut self) -> Result<[u8; 16], RelayError>;
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
/// the station's. It enforces nothing: `Auth` is ignored and queries do not filter.
#[derive(Default)]
pub struct MemRelay {
    entries: Vec<(CircleId, Vec<u8>)>,
    manifests: BTreeMap<CircleId, Vec<u8>>,
    objects: BTreeMap<(CircleId, ObjectKind), Vec<Vec<u8>>>,
    nonces: u64,
    shared: Option<Arc<Mutex<MemRelay>>>,
}

impl MemRelay {
    pub fn shared(inner: Arc<Mutex<MemRelay>>) -> Self {
        MemRelay {
            shared: Some(inner),
            ..Default::default()
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

impl ChannelRelay for MemRelay {
    fn put_manifest(&mut self, circle: &CircleId, sealed: &[u8]) -> Result<(), RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().put_manifest(circle, sealed);
        }
        self.manifests.insert(*circle, sealed.to_vec());
        Ok(())
    }
    fn get_manifest(&mut self, circle: &CircleId) -> Result<Option<Vec<u8>>, RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().get_manifest(circle);
        }
        Ok(self.manifests.get(circle).cloned())
    }
    fn put_object(
        &mut self,
        channel: &[u8; 16],
        kind: ObjectKind,
        body: &[u8],
    ) -> Result<Vec<u8>, RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().put_object(channel, kind, body);
        }
        let list = self.objects.entry((*channel, kind)).or_default();
        list.push(body.to_vec());
        // No hashing here: the id is the object's place in its list.
        Ok((list.len() as u64 - 1).to_be_bytes().to_vec())
    }
    fn list(
        &mut self,
        channel: &[u8; 16],
        kind: ObjectKind,
        _query: &[(&str, &str)],
        _auth: &Auth,
    ) -> Result<Vec<Vec<u8>>, RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().list(channel, kind, &[], &Auth::None);
        }
        Ok(self
            .objects
            .get(&(*channel, kind))
            .cloned()
            .unwrap_or_default())
    }
    fn post_channel(
        &mut self,
        channel: &[u8; 16],
        message: &[u8],
        _auth: &Auth,
    ) -> Result<u64, RelayError> {
        self.post(channel, message)
    }
    fn poll_channel(
        &mut self,
        channel: &[u8; 16],
        after: Option<u64>,
        wait_ms: u64,
        _auth: &Auth,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError> {
        self.poll(channel, after, wait_ms)
    }
    fn nonce(&mut self) -> Result<[u8; 16], RelayError> {
        if let Some(s) = &self.shared {
            return s.lock().unwrap().nonce();
        }
        self.nonces += 1;
        let mut n = [0u8; 16];
        n[8..].copy_from_slice(&self.nonces.to_be_bytes());
        Ok(n)
    }
}

fn channel_path(channel: &[u8; 16]) -> String {
    format!("/v1/channels/{}", hex::encode(channel))
}

fn with_query(path: String, query: &[(&str, &str)]) -> String {
    let q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    // The station signs `PATH?QUERY` with the `?` always present.
    format!("{path}?{}", q.join("&"))
}

/// The request line a proof for `poll_channel` must sign: `GET /path?query`.
pub fn poll_target(channel: &[u8; 16], after: Option<u64>, wait_ms: u64) -> String {
    format!("GET {}", poll_path(channel, after, wait_ms))
}

fn poll_path(channel: &[u8; 16], after: Option<u64>, wait_ms: u64) -> String {
    let (a, w) = (after.map(|a| a.to_string()), wait_ms.to_string());
    let mut q = Vec::new();
    if let Some(a) = &a {
        q.push(("after", a.as_str()));
    }
    q.push(("wait", w.as_str()));
    with_query(format!("{}/messages", channel_path(channel)), &q)
}

/// The request line a proof for `list` must sign. `None` for a kind that has no read route.
pub fn list_target(channel: &[u8; 16], kind: ObjectKind, query: &[(&str, &str)]) -> Option<String> {
    list_path(channel, kind, query).map(|p| format!("GET {p}"))
}

fn list_path(channel: &[u8; 16], kind: ObjectKind, query: &[(&str, &str)]) -> Option<String> {
    let mut p = channel_path(channel);
    match kind {
        // The channel read carries the policy too.
        ObjectKind::Channel | ObjectKind::Policy => {}
        ObjectKind::Gate => return None,
        k => {
            p.push('/');
            p.push_str(k.segment()?);
        }
    }
    Some(with_query(p, query))
}

pub struct HttpRelay {
    base: String,
    agent: ureq::Agent,
}

fn map_err(e: ureq::Error) -> RelayError {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            match code {
                401 => RelayError::Gate(body),
                403 => RelayError::Grant(body),
                _ => RelayError::Refused(body),
            }
        }
        e => RelayError::Http(e.to_string()),
    }
}

fn b64(s: &str) -> Result<Vec<u8>, RelayError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| RelayError::Http(e.to_string()))
}

fn json(resp: ureq::Response) -> Result<serde_json::Value, RelayError> {
    resp.into_json()
        .map_err(|e| RelayError::Http(e.to_string()))
}

fn auth_headers(req: ureq::Request, auth: &Auth) -> ureq::Request {
    match auth {
        Auth::None => req,
        Auth::Gate(t) => req.set("Cahoots-Gate", &hex::encode(t)),
        Auth::Proof { proof, kid } => req
            .set(
                "Cahoots-Proof",
                &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof),
            )
            .set("Cahoots-Kid", &hex::encode(kid)),
    }
}

/// Entries of a `…/messages` answer.
fn entries(v: &serde_json::Value) -> Result<Vec<(u64, Vec<u8>)>, RelayError> {
    let mut out = Vec::new();
    for e in v["entries"].as_array().into_iter().flatten() {
        let seq = e["seq"]
            .as_u64()
            .ok_or_else(|| RelayError::Http("entry seq".into()))?;
        out.push((seq, b64(e["message"].as_str().unwrap_or(""))?));
    }
    Ok(out)
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

    fn post_bytes(
        &self,
        path: &str,
        body: &[u8],
        auth: &Auth,
    ) -> Result<serde_json::Value, RelayError> {
        let req = self
            .agent
            .post(&format!("{}{path}", self.base))
            .set("content-type", "application/octet-stream");
        json(auth_headers(req, auth).send_bytes(body).map_err(map_err)?)
    }

    fn get(&self, path_and_query: &str, auth: &Auth) -> Result<ureq::Response, RelayError> {
        let req = self.agent.get(&format!("{}{path_and_query}", self.base));
        auth_headers(req, auth).call().map_err(map_err)
    }
}

impl Relay for HttpRelay {
    fn post(&mut self, circle: &CircleId, message: &[u8]) -> Result<u64, RelayError> {
        let path = format!("/v1/circles/{}/messages", hex::encode(circle));
        let v = self.post_bytes(&path, message, &Auth::None)?;
        v["seq"]
            .as_u64()
            .ok_or_else(|| RelayError::Http("no seq in response".into()))
    }

    fn poll(
        &mut self,
        circle: &CircleId,
        after: Option<u64>,
        wait_ms: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError> {
        let mut url = format!(
            "{}/v1/circles/{}/messages?wait={wait_ms}",
            self.base,
            hex::encode(circle)
        );
        if let Some(a) = after {
            url.push_str(&format!("&after={a}"));
        }
        let v = json(self.agent.get(&url).call().map_err(map_err)?)?;
        entries(&v)
    }
}

impl ChannelRelay for HttpRelay {
    fn put_manifest(&mut self, circle: &CircleId, sealed: &[u8]) -> Result<(), RelayError> {
        let url = format!("{}/v1/circles/{}/manifest", self.base, hex::encode(circle));
        self.agent
            .put(&url)
            .set("content-type", "application/octet-stream")
            .send_bytes(sealed)
            .map_err(map_err)?;
        Ok(())
    }

    fn get_manifest(&mut self, circle: &CircleId) -> Result<Option<Vec<u8>>, RelayError> {
        let url = format!("{}/v1/circles/{}/manifest", self.base, hex::encode(circle));
        match self.agent.get(&url).call() {
            Ok(resp) => {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes)
                    .map_err(|e| RelayError::Http(e.to_string()))?;
                Ok(Some(bytes))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(map_err(e)),
        }
    }

    fn put_object(
        &mut self,
        channel: &[u8; 16],
        kind: ObjectKind,
        body: &[u8],
    ) -> Result<Vec<u8>, RelayError> {
        let (path, id_key) = match kind {
            ObjectKind::Channel => ("/v1/channels".to_string(), "channel_id"),
            ObjectKind::Policy => (format!("{}/policy", channel_path(channel)), "policy_id"),
            ObjectKind::Grant => (format!("{}/grants", channel_path(channel)), "grant_id"),
            ObjectKind::Revocation => {
                (format!("{}/revocations", channel_path(channel)), "grant_id")
            }
            ObjectKind::Deposit => (format!("{}/deposits", channel_path(channel)), "deposit_id"),
            ObjectKind::Request => (format!("{}/requests", channel_path(channel)), "request_id"),
            ObjectKind::Gate => (format!("{}/gates", channel_path(channel)), "token_hash"),
        };
        let v = self.post_bytes(&path, body, &Auth::None)?;
        let id = v[id_key]
            .as_str()
            .ok_or_else(|| RelayError::Http(format!("no {id_key} in response")))?;
        hex::decode(id).map_err(|e| RelayError::Http(e.to_string()))
    }

    fn list(
        &mut self,
        channel: &[u8; 16],
        kind: ObjectKind,
        query: &[(&str, &str)],
        auth: &Auth,
    ) -> Result<Vec<Vec<u8>>, RelayError> {
        let path = list_path(channel, kind, query)
            .ok_or_else(|| RelayError::Http("gates cannot be listed".into()))?;
        let v = json(self.get(&path, auth)?)?;
        let many = |key: &str, f: &dyn Fn(&serde_json::Value) -> Result<Vec<u8>, RelayError>| {
            v[key].as_array().into_iter().flatten().map(f).collect()
        };
        let text = |x: &serde_json::Value| b64(x.as_str().unwrap_or(""));
        match kind {
            ObjectKind::Channel => ["channel", "policy", "manifest"]
                .iter()
                .map(|k| match v[k].as_str() {
                    Some(s) => b64(s),
                    None => Ok(Vec::new()),
                })
                .collect(),
            ObjectKind::Policy => Ok(v["policy"]
                .as_str()
                .map(b64)
                .transpose()?
                .into_iter()
                .collect()),
            ObjectKind::Grant => many("grants", &text),
            ObjectKind::Deposit => many("deposits", &text),
            ObjectKind::Request => many("requests", &|x| text(&x["body"])),
            ObjectKind::Revocation => many("revocations", &|x| {
                hex::decode(x["grant_id"].as_str().unwrap_or(""))
                    .map_err(|e| RelayError::Http(e.to_string()))
            }),
            ObjectKind::Gate => unreachable!("refused above"),
        }
    }

    fn post_channel(
        &mut self,
        channel: &[u8; 16],
        message: &[u8],
        auth: &Auth,
    ) -> Result<u64, RelayError> {
        let path = format!("{}/messages", channel_path(channel));
        let v = self.post_bytes(&path, message, auth)?;
        v["seq"]
            .as_u64()
            .ok_or_else(|| RelayError::Http("no seq in response".into()))
    }

    fn poll_channel(
        &mut self,
        channel: &[u8; 16],
        after: Option<u64>,
        wait_ms: u64,
        auth: &Auth,
    ) -> Result<Vec<(u64, Vec<u8>)>, RelayError> {
        let v = json(self.get(&poll_path(channel, after, wait_ms), auth)?)?;
        entries(&v)
    }

    fn nonce(&mut self) -> Result<[u8; 16], RelayError> {
        let v = json(self.get("/v1/nonce", &Auth::None)?)?;
        let n = hex::decode(v["nonce"].as_str().unwrap_or(""))
            .map_err(|e| RelayError::Http(e.to_string()))?;
        n.try_into()
            .map_err(|_| RelayError::Http("nonce is not 16 bytes".into()))
    }
}
