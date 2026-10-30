//! `cahoots-host agent`: an SSH agent whose keys are a badge's seat keys.
//!
//! It speaks OpenSSH's agent protocol (`PROTOCOL.agent`) on a Unix socket, one thread per
//! connection:
//!
//! - `SSH_AGENTC_REQUEST_IDENTITIES` (11): each circle's seat key as `ecdsa-sha2-nistp256`,
//!   comment `cahoots:<circle>:<seat>`, in `SSH_AGENT_IDENTITIES_ANSWER` (12). The keys are
//!   asked of the badge (`Describe`) the first time and kept: they are public.
//! - `SSH_AGENTC_SIGN_REQUEST` (13): one `SshSign` request to the badge, which asks its
//!   holder. The badge's signature comes back in `SSH_AGENT_SIGN_RESPONSE` (14); a refusal or
//!   any error is `SSH_AGENT_FAILURE` (5). Nothing is signed here and no signature is kept.
//! - `SSH_AGENTC_EXTENSION` (27) `session-bind@openssh.com`: the server's host key, the
//!   session id, the server's signature over it, and whether the connection is forwarded. The
//!   agent keeps it for the connection and sends it with the next request to sign for that
//!   session, with the host's name from `known_hosts`. The badge checks the signature.
//! - `SSH_AGENTC_EXTENSION` (27) `sign-with-content@cahoots`: the key, an `SSHSIG` envelope
//!   to sign, and the content it hashes. `cahoots-host-git-sign` sends it so the badge can show
//!   the commit it signs. Answered `SSH_AGENT_SUCCESS` (6) followed by the signature. A
//!   fourth string, the repository's `remote.origin.url` (empty when there is none), may
//!   follow. When the agent keeps a grant for the key's circle (see [`crate::grants`]) and the
//!   envelope is a git signature by the grant's author, it asks the badge for a nonce and sends
//!   `SshSignUnderGrant` with a proof by its own key: the badge signs without asking its holder.
//!   A grant is retired here when the badge's answers count it down to 0. A refusal under a
//!   grant (`Denied`) fails that signature and retires the grant too (`remaining` 0): the
//!   badge has revoked, ended or forgotten it, or its holder refused a prompt the badge fell
//!   back to, and the agent cannot tell which. The next commit asks the ordinary way.
//! - Anything else: `SSH_AGENT_FAILURE`.

use crate::badge_client::{describe_error, BadgeClient, ASK_TIMEOUT};
use crate::{agentkey, grants};
use base64::Engine;
use cahoots_custodian::wire::{ChipRequest, ChipResponse, ErrorCode, Request, Response, SshHost};
use cahoots_custodian::CircleId;
use cahoots_objects::authz::{location_matches, proof_sign, SignGrant};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const SSH_AGENT_FAILURE: u8 = 5;
pub const SSH_AGENT_SUCCESS: u8 = 6;
pub const SSH_AGENTC_REQUEST_IDENTITIES: u8 = 11;
pub const SSH_AGENT_IDENTITIES_ANSWER: u8 = 12;
pub const SSH_AGENTC_SIGN_REQUEST: u8 = 13;
pub const SSH_AGENT_SIGN_RESPONSE: u8 = 14;
pub const SSH_AGENTC_EXTENSION: u8 = 27;

/// OpenSSH's extension that binds a connection to the server's host key.
pub const SESSION_BIND: &str = "session-bind@openssh.com";
/// This agent's extension for a signature over content the badge is shown.
pub const SIGN_WITH_CONTENT: &str = "sign-with-content@cahoots";

/// The longest message read, as OpenSSH's agent: anything longer closes the connection.
const MAX_MESSAGE: usize = 256 * 1024;
/// The most session bindings one connection keeps, as OpenSSH's agent.
const MAX_BINDS: usize = 16;

/// Host names tried against hashed `known_hosts` entries, besides those in the ssh config
/// beside `known_hosts`: a hashed entry keeps no name, only a keyed hash of it.
const KNOWN_FORGES: &[&str] = &[
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "codeberg.org",
    "ssh.dev.azure.com",
];

/// One seat key the agent offers.
#[derive(Clone, Debug)]
struct Identity {
    circle: CircleId,
    /// The circle's name.
    name: String,
    /// The SSH public key blob.
    blob: Vec<u8>,
    comment: String,
}

/// What `session-bind@openssh.com` told the agent.
#[derive(Clone, Debug)]
struct Bind {
    host_key: Vec<u8>,
    session_id: Vec<u8>,
    host_sig: Vec<u8>,
}

/// The agent: one badge, the circles whose seat keys it offers, and where host names come
/// from.
pub struct Agent {
    badge: Mutex<BadgeClient>,
    circles: Vec<String>,
    identities: Mutex<Option<Vec<Identity>>>,
    known_hosts: PathBuf,
    /// Where the agent's key and grants are kept: [`agentkey::dir`].
    home: PathBuf,
}

/// How a signature under a grant went.
enum UnderGrant {
    /// The badge signed, under the grant or after asking its holder.
    Signed(Vec<u8>),
    /// Ask again as an ordinary signature.
    Plain,
    /// Give up: the badge could not be asked, or refused for a reason a retry would not change.
    Failed,
}

impl Agent {
    /// An agent for the seat keys of `circles` (circle names) on `badge`, naming hosts from
    /// `known_hosts` (and the ssh `config` file in the same directory).
    pub fn new(badge: BadgeClient, circles: Vec<String>, known_hosts: PathBuf) -> Agent {
        Agent {
            badge: Mutex::new(badge),
            circles,
            identities: Mutex::new(None),
            known_hosts,
            home: agentkey::dir(),
        }
    }

    /// Answer connections on `listener` until it fails, one thread each.
    pub fn serve(self: Arc<Self>, listener: UnixListener) -> io::Result<()> {
        for conn in listener.incoming() {
            let conn = conn?;
            let agent = self.clone();
            std::thread::spawn(move || {
                let _ = agent.connection(conn);
            });
        }
        Ok(())
    }

    /// Answer one connection's messages until it closes.
    fn connection(&self, mut s: UnixStream) -> io::Result<()> {
        let mut binds = Vec::new();
        loop {
            let mut len = [0u8; 4];
            s.read_exact(&mut len)?;
            let len = u32::from_be_bytes(len) as usize;
            if len == 0 || len > MAX_MESSAGE {
                return Ok(());
            }
            let mut msg = vec![0u8; len];
            s.read_exact(&mut msg)?;
            let reply = self.answer(&msg, &mut binds);
            let mut out = (reply.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(&reply);
            s.write_all(&out)?;
        }
    }

    /// The answer to one message (type byte and body).
    fn answer(&self, msg: &[u8], binds: &mut Vec<Bind>) -> Vec<u8> {
        let mut r = Reader(&msg[1..]);
        let answer = match msg[0] {
            SSH_AGENTC_REQUEST_IDENTITIES if r.0.is_empty() => Some(self.identities_answer()),
            SSH_AGENTC_SIGN_REQUEST => self.sign_request(&mut r, binds),
            SSH_AGENTC_EXTENSION => self.extension(&mut r, binds),
            _ => None,
        };
        answer.unwrap_or_else(|| vec![SSH_AGENT_FAILURE])
    }

    fn identities_answer(&self) -> Vec<u8> {
        let ids = self.identities();
        let mut out = vec![SSH_AGENT_IDENTITIES_ANSWER];
        out.extend_from_slice(&(ids.len() as u32).to_be_bytes());
        for id in &ids {
            put_string(&mut out, &id.blob);
            put_string(&mut out, id.comment.as_bytes());
        }
        out
    }

    /// The seat keys, asked of the badge until every circle is found, then kept.
    fn identities(&self) -> Vec<Identity> {
        let mut kept = self.identities.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ids) = kept.as_ref() {
            return ids.clone();
        }
        let mut badge = self.badge.lock().unwrap_or_else(|e| e.into_inner());
        let (ids, complete) = match fetch_identities(&mut badge, &self.circles) {
            Ok(found) => found,
            Err(e) => {
                eprintln!("agent: no keys yet: {e}");
                return Vec::new();
            }
        };
        if complete {
            *kept = Some(ids.clone());
        }
        ids
    }

    fn sign_request(&self, r: &mut Reader, binds: &[Bind]) -> Option<Vec<u8>> {
        let key = r.string()?;
        let data = r.string()?;
        let _flags = r.u32()?;
        r.end()?;
        let id = self.identity(key)?;
        let host = self.host_for(data, binds);
        let sig = self.badge_sign(&id, data.to_vec(), host, None)?;
        let mut out = vec![SSH_AGENT_SIGN_RESPONSE];
        put_string(&mut out, &sig);
        Some(out)
    }

    fn extension(&self, r: &mut Reader, binds: &mut Vec<Bind>) -> Option<Vec<u8>> {
        let name = r.string()?;
        if name == SESSION_BIND.as_bytes() {
            let host_key = r.string()?.to_vec();
            let session_id = r.string()?.to_vec();
            let host_sig = r.string()?.to_vec();
            let _forwarding = r.byte()?;
            r.end()?;
            if binds.len() >= MAX_BINDS {
                return None;
            }
            binds.push(Bind {
                host_key,
                session_id,
                host_sig,
            });
            Some(vec![SSH_AGENT_SUCCESS])
        } else if name == SIGN_WITH_CONTENT.as_bytes() {
            let key = r.string()?;
            let data = r.string()?.to_vec();
            let content = r.string()?.to_vec();
            // The repository, from a helper that sends it; empty means none.
            let location = if r.0.is_empty() {
                None
            } else {
                let l = r.string()?;
                (!l.is_empty()).then(|| String::from_utf8_lossy(l).into_owned())
            };
            r.end()?;
            let id = self.identity(key)?;
            let sig = self.sign_content(&id, data, content, location)?;
            let mut out = vec![SSH_AGENT_SUCCESS];
            put_string(&mut out, &sig);
            Some(out)
        } else {
            None
        }
    }

    /// The identity whose key blob is `key`.
    fn identity(&self, key: &[u8]) -> Option<Identity> {
        self.identities().into_iter().find(|i| i.blob == key)
    }

    /// The session binding for the session `data` signs in, with the host's name, if the
    /// connection bound one and `known_hosts` names its key. A host with no name is sent as
    /// no host: the badge then says "host not named".
    fn host_for(&self, data: &[u8], binds: &[Bind]) -> Option<SshHost> {
        let (session_id, user) = userauth_session(data)?;
        let bind = binds.iter().rev().find(|b| b.session_id == session_id)?;
        let name = host_name(&self.known_hosts, &bind.host_key)?;
        Some(SshHost {
            host_key: bind.host_key.clone(),
            session_id: bind.session_id.clone(),
            host_sig: bind.host_sig.clone(),
            name,
            user,
        })
    }

    /// A signature over `data` with `content` shown: under a grant when the agent keeps one
    /// that fits, else (or when the badge turns the grant down) as an ordinary signature.
    fn sign_content(
        &self,
        id: &Identity,
        data: Vec<u8>,
        content: Vec<u8>,
        location: Option<String>,
    ) -> Option<Vec<u8>> {
        if sshsig_namespace(&data) == Some(b"git".as_slice()) {
            if let Some(stored) = grants::find(&self.home, &id.name) {
                if grant_fits(&stored, &content, location.as_deref()) {
                    match self.sign_under_grant(id, stored, &data, &content, location) {
                        UnderGrant::Signed(sig) => return Some(sig),
                        UnderGrant::Failed => return None,
                        UnderGrant::Plain => {}
                    }
                }
            }
        }
        self.badge_sign(id, data, None, Some(content))
    }

    /// Ask for a fresh nonce, then for a signature under `stored` with the agent key's proof
    /// over the hash of `data` and the nonce. A badge that is busy is asked once more.
    fn sign_under_grant(
        &self,
        id: &Identity,
        mut stored: grants::Stored,
        data: &[u8],
        content: &[u8],
        location: Option<String>,
    ) -> UnderGrant {
        let key = match agentkey::load(&self.home) {
            Ok(Some(k)) => k,
            Ok(None) => {
                eprintln!("agent: {}: a grant is kept but no agent key; not using it", id.comment);
                return UnderGrant::Plain;
            }
            Err(e) => {
                eprintln!("agent: {}: no agent key: {e}", id.comment);
                return UnderGrant::Plain;
            }
        };
        let hash: [u8; 32] = Sha256::digest(data).into();
        let mut badge = self.badge.lock().unwrap_or_else(|e| e.into_inner());
        for attempt in 0..2 {
            let again = attempt == 0;
            let nonce = match badge.request(Request::Chip {
                circle: id.circle,
                op: ChipRequest::Nonce,
            }) {
                Ok(Response::Chip {
                    result: ChipResponse::Nonce { nonce },
                }) => nonce,
                Ok(Response::Error {
                    code: ErrorCode::Busy,
                }) if again => {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    continue;
                }
                Ok(Response::Error { code }) => {
                    eprintln!("agent: {}: no nonce: {}", id.comment, describe_error(code));
                    return UnderGrant::Plain;
                }
                Ok(other) => {
                    eprintln!("agent: {}: unexpected answer {other:?}", id.comment);
                    return UnderGrant::Failed;
                }
                Err(e) => {
                    eprintln!("agent: {}: {e}", id.comment);
                    return UnderGrant::Failed;
                }
            };
            let req = Request::Chip {
                circle: id.circle,
                op: ChipRequest::SshSignUnderGrant {
                    data: data.to_vec(),
                    content: content.to_vec(),
                    grant: stored.bytes.clone(),
                    proof: proof_sign(&key, &hash, &nonce),
                    location: location.clone(),
                    time: grants::now(),
                },
            };
            // The badge may fall back to asking its holder, so wait as long as for any prompt.
            match badge.request_within(req, ASK_TIMEOUT) {
                Ok(Response::Chip {
                    result: ChipResponse::GrantSignature {
                        signature,
                        remaining,
                    },
                }) => {
                    eprintln!(
                        "agent: {} signed under a grant, {remaining} left",
                        id.comment
                    );
                    stored.remaining = Some(remaining);
                    if let Err(e) = grants::save(&self.home, &stored) {
                        eprintln!("agent: {e}");
                    }
                    return UnderGrant::Signed(signature);
                }
                // The badge did not take the grant for this one and asked its holder instead.
                Ok(Response::Chip {
                    result: ChipResponse::Signature { signature },
                }) => {
                    eprintln!("agent: {} signed after a prompt", id.comment);
                    return UnderGrant::Signed(signature);
                }
                Ok(Response::Error {
                    code: ErrorCode::Busy,
                }) if again => {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Ok(Response::Error {
                    code: ErrorCode::Denied,
                }) => {
                    // The holder refusing a fallback prompt and a grant the badge no longer
                    // honours look the same. This commit fails without a second question,
                    // and the grant is retired, so the next commit asks the ordinary way.
                    eprintln!(
                        "agent: {}: refused on the badge, or the grant is no longer valid: run `cahoots-host grant list`",
                        id.comment
                    );
                    stored.remaining = Some(0);
                    match grants::save(&self.home, &stored) {
                        Ok(_) => eprintln!(
                            "agent: {}: the grant is no longer used; the next commit asks on the badge",
                            id.comment
                        ),
                        Err(e) => eprintln!("agent: {e}"),
                    }
                    return UnderGrant::Failed;
                }
                Ok(Response::Error {
                    code: ErrorCode::Unparseable,
                }) => {
                    eprintln!(
                        "agent: {}: the badge could not check the grant or the proof; asking as an ordinary signature",
                        id.comment
                    );
                    return UnderGrant::Plain;
                }
                Ok(Response::Error { code }) => {
                    eprintln!("agent: {}: {}", id.comment, describe_error(code));
                    return UnderGrant::Failed;
                }
                Ok(other) => {
                    eprintln!("agent: {}: unexpected answer {other:?}", id.comment);
                    return UnderGrant::Failed;
                }
                Err(e) => {
                    eprintln!("agent: {}: {e}", id.comment);
                    return UnderGrant::Failed;
                }
            }
        }
        eprintln!("agent: {}: the badge stayed busy; asking as an ordinary signature", id.comment);
        UnderGrant::Plain
    }

    /// One `SshSign` request to the badge. `None` for a refusal or any error, logged.
    fn badge_sign(
        &self,
        id: &Identity,
        data: Vec<u8>,
        host: Option<SshHost>,
        content: Option<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let req = Request::Chip {
            circle: id.circle,
            op: ChipRequest::SshSign {
                data,
                host,
                content,
            },
        };
        let mut badge = self.badge.lock().unwrap_or_else(|e| e.into_inner());
        match badge.request(req) {
            Ok(Response::Chip {
                result: ChipResponse::Signature { signature },
            }) => {
                eprintln!("agent: {} signed", id.comment);
                Some(signature)
            }
            Ok(Response::Error { code }) => {
                eprintln!("agent: {}: {}", id.comment, describe_error(code));
                None
            }
            Ok(other) => {
                eprintln!("agent: {}: unexpected answer {other:?}", id.comment);
                None
            }
            Err(e) => {
                eprintln!("agent: {}: {e}", id.comment);
                None
            }
        }
    }
}

/// Ask the badge for the seat key of each circle in `names`. Returns the keys found and
/// whether every circle was found.
fn fetch_identities(
    badge: &mut BadgeClient,
    names: &[String],
) -> Result<(Vec<Identity>, bool), String> {
    let circles = match badge
        .request(Request::Identify)
        .map_err(|e| e.to_string())?
    {
        Response::Identity { circles, .. } => circles,
        Response::Error { code } => return Err(describe_error(code).into()),
        other => return Err(format!("unexpected answer {other:?}")),
    };
    let mut ids = Vec::new();
    let mut complete = true;
    for name in names {
        let Some(info) = circles.iter().find(|c| &c.name == name) else {
            eprintln!("agent: the badge holds no circle named {name}");
            complete = false;
            continue;
        };
        let req = Request::Chip {
            circle: info.circle_id,
            op: ChipRequest::Describe,
        };
        match badge.request(req).map_err(|e| e.to_string())? {
            Response::Chip {
                result: ChipResponse::Description { public, .. },
            } => match seat_blob(&public) {
                Some(blob) => ids.push(Identity {
                    circle: info.circle_id,
                    name: info.name.clone(),
                    blob,
                    comment: format!("cahoots:{}:{}", info.name, info.seat),
                }),
                None => {
                    eprintln!("agent: {name}: the badge's key does not parse");
                    complete = false;
                }
            },
            Response::Error { code } => {
                eprintln!("agent: {name}: {}", describe_error(code));
                complete = false;
            }
            other => return Err(format!("unexpected answer {other:?}")),
        }
    }
    Ok((ids, complete))
}

/// The namespace of an `SSHSIG` envelope (the data `ssh-keygen -Y sign` has signed), or
/// `None` for anything else.
fn sshsig_namespace(data: &[u8]) -> Option<&[u8]> {
    let mut r = Reader(data.strip_prefix(b"SSHSIG")?);
    r.string()
}

/// The author of a commit: its `author` header line, less the time and zone at its end.
fn commit_author(content: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(content).ok()?;
    let line = text
        .lines()
        .take_while(|l| !l.is_empty())
        .find_map(|l| l.strip_prefix("author "))?;
    let (rest, _zone) = line.rsplit_once(' ')?;
    let (author, _time) = rest.rsplit_once(' ')?;
    Some(author)
}

/// Whether the badge would sign `content` at `location` under `stored` without asking: the
/// commit's author and the repository are the grant's, and it has not ended by the laptop's
/// clock. The badge checks all of it again; this only keeps a commit the grant does not cover
/// from being sent under it, where a refusal on the badge would be taken for the grant's end.
fn grant_fits(stored: &grants::Stored, content: &[u8], location: Option<&str>) -> bool {
    let Some(g) = stored
        .claims()
        .ok()
        .and_then(|c| SignGrant::from_claims(&c).ok())
    else {
        return false;
    };
    let place = g.locations.is_empty()
        || location.is_some_and(|l| g.locations.iter().any(|x| location_matches(x, l)));
    commit_author(content) == Some(g.author.as_str())
        && place
        && g.exp.is_none_or(|e| grants::now() < e)
}

/// A P-256 public key (SEC1, either form) as an SSH public key blob.
pub fn seat_blob(sec1: &[u8]) -> Option<Vec<u8>> {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let point = p256::PublicKey::from_sec1_bytes(sec1)
        .ok()?
        .to_encoded_point(false);
    let mut out = Vec::new();
    put_string(&mut out, b"ecdsa-sha2-nistp256");
    put_string(&mut out, b"nistp256");
    put_string(&mut out, point.as_bytes());
    Some(out)
}

/// A seat key as an `authorized_keys` line: `ecdsa-sha2-nistp256 <base64> <comment>`.
pub fn authorized_key(sec1: &[u8], comment: &str) -> Option<String> {
    let blob = seat_blob(sec1)?;
    Some(format!(
        "ecdsa-sha2-nistp256 {} {comment}",
        base64::engine::general_purpose::STANDARD.encode(blob)
    ))
}

/// The session id and user of a userauth request, or `None` for anything else.
fn userauth_session(data: &[u8]) -> Option<(Vec<u8>, String)> {
    let mut r = Reader(data);
    let session_id = r.string()?.to_vec();
    if r.byte()? != 50 {
        return None;
    }
    let user = String::from_utf8(r.string()?.to_vec()).ok()?;
    Some((session_id, user))
}

/// The name `known_hosts` gives `host_key` (an SSH public key blob), or `None`.
///
/// A plain entry names its first host that is not a pattern or a negation (`[host]:port` is
/// shown as `host:port`). A hashed entry (`|1|salt|hash`, HMAC-SHA1 of the name under the
/// salt) keeps no name, so names are tried against it: the `Host` and `HostName` values in the
/// ssh `config` file in the same directory, then a few well-known git hosts. Marked lines
/// (`@cert-authority`, `@revoked`) name nothing.
pub fn host_name(known_hosts: &Path, host_key: &[u8]) -> Option<String> {
    let text = std::fs::read_to_string(known_hosts).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut candidates: Option<Vec<String>> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(hosts), Some(_kind), Some(key)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if b64.decode(key).ok().as_deref() != Some(host_key) {
            continue;
        }
        for pattern in hosts.split(',') {
            if let Some(hashed) = pattern.strip_prefix("|1|") {
                let tried = candidates.get_or_insert_with(|| names_to_try(known_hosts));
                if let Some(name) = tried.iter().find(|n| hashed_matches(hashed, n)) {
                    return Some(name.clone());
                }
            } else if !pattern.starts_with('!') && !pattern.contains(['*', '?']) {
                return Some(show_host(pattern));
            }
        }
    }
    None
}

/// `[host]:port` as `host:port`; anything else as it is.
fn show_host(pattern: &str) -> String {
    match pattern.strip_prefix('[').and_then(|p| p.split_once("]:")) {
        Some((host, port)) => format!("{host}:{port}"),
        None => pattern.to_string(),
    }
}

/// True if `salt|hash` (both base64) is the HMAC-SHA1 of `name` under the salt.
fn hashed_matches(salt_hash: &str, name: &str) -> bool {
    let b64 = base64::engine::general_purpose::STANDARD;
    let Some((salt, hash)) = salt_hash.split_once('|') else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (b64.decode(salt), b64.decode(hash)) else {
        return false;
    };
    let Ok(mut mac) = <Hmac<sha1::Sha1> as Mac>::new_from_slice(&salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// The names tried against hashed entries: from the ssh `config` beside `known_hosts`, then
/// [`KNOWN_FORGES`].
fn names_to_try(known_hosts: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let config = known_hosts.with_file_name("config");
    if let Ok(text) = std::fs::read_to_string(config) {
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let Some(key) = words.next() else { continue };
            if key.eq_ignore_ascii_case("host") || key.eq_ignore_ascii_case("hostname") {
                for w in words {
                    if !w.starts_with('!') && !w.contains(['*', '?', '%']) {
                        names.push(w.to_string());
                    }
                }
            }
        }
    }
    names.extend(KNOWN_FORGES.iter().map(|s| s.to_string()));
    names
}

/// Append `b` as an SSH string.
fn put_string(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// A cursor over SSH wire data. Every read checks the length; nothing here panics.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn raw(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.0.len() {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.raw(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.raw(4)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.raw(n)
    }
    fn end(&self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

/// Ask the agent at `socket` for its keys: each as `(blob, comment)`.
pub fn list_identities(socket: &Path) -> Result<Vec<(Vec<u8>, String)>, String> {
    let reply = agent_call(socket, &[SSH_AGENTC_REQUEST_IDENTITIES])?;
    if reply.first() != Some(&SSH_AGENT_IDENTITIES_ANSWER) {
        return Err("agent: no identities answer".into());
    }
    let mut r = Reader(&reply[1..]);
    let n = r.u32().ok_or("agent: a short answer")?;
    let mut out = Vec::new();
    for _ in 0..n {
        let blob = r.string().ok_or("agent: a short answer")?.to_vec();
        let comment = r.string().ok_or("agent: a short answer")?;
        out.push((blob, String::from_utf8_lossy(comment).into_owned()));
    }
    Ok(out)
}

/// A plain `SSH_AGENTC_SIGN_REQUEST`: sign `data` with the key `key`, with nothing shown
/// to the badge but what `data` itself says (for an `SSHSIG` envelope, its namespace and
/// hash). The SSH signature blob, or an error in words.
pub fn sign_plain(socket: &Path, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let mut msg = vec![SSH_AGENTC_SIGN_REQUEST];
    put_string(&mut msg, key);
    put_string(&mut msg, data);
    msg.extend_from_slice(&0u32.to_be_bytes());
    let reply = agent_call(socket, &msg)?;
    let mut r = Reader(&reply[1..]);
    match reply[0] {
        SSH_AGENT_SIGN_RESPONSE => {
            let sig = r.string().ok_or("agent: a short answer")?.to_vec();
            r.end().ok_or("agent: a long answer")?;
            Ok(sig)
        }
        _ => Err(REFUSED.into()),
    }
}

const REFUSED: &str = "the agent did not sign: refused on the badge, the key is not one the agent \
                       holds, or the badge could not be asked (the agent's log says which)";

/// One framed message to the agent and its framed answer.
fn agent_call(socket: &Path, msg: &[u8]) -> Result<Vec<u8>, String> {
    let mut s = UnixStream::connect(socket)
        .map_err(|e| format!("no agent at {}: {e}", socket.display()))?;
    let mut framed = (msg.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(msg);
    s.write_all(&framed).map_err(|e| format!("agent: {e}"))?;
    let mut len = [0u8; 4];
    s.read_exact(&mut len).map_err(|e| format!("agent: {e}"))?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Err("agent: an answer of the wrong length".into());
    }
    let mut reply = vec![0u8; len];
    s.read_exact(&mut reply)
        .map_err(|e| format!("agent: {e}"))?;
    Ok(reply)
}

/// Call the agent at `socket` with `sign-with-content@cahoots`: sign `data` with the key
/// `key` (an SSH public key blob), showing the badge `content`, for the repository `location`
/// (its `remote.origin.url`, or empty). The SSH signature blob, or an error in words.
pub fn sign_with_content(
    socket: &Path,
    key: &[u8],
    data: &[u8],
    content: &[u8],
    location: &str,
) -> Result<Vec<u8>, String> {
    let mut msg = vec![SSH_AGENTC_EXTENSION];
    put_string(&mut msg, SIGN_WITH_CONTENT.as_bytes());
    put_string(&mut msg, key);
    put_string(&mut msg, data);
    put_string(&mut msg, content);
    put_string(&mut msg, location.as_bytes());
    let reply = agent_call(socket, &msg)?;
    let mut r = Reader(&reply[1..]);
    match reply[0] {
        SSH_AGENT_SUCCESS => {
            let sig = r.string().ok_or("agent: a short answer")?.to_vec();
            r.end().ok_or("agent: a long answer")?;
            Ok(sig)
        }
        _ => Err(REFUSED.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn hashed(salt: &[u8], name: &str) -> String {
        let mut mac = <Hmac<sha1::Sha1> as Mac>::new_from_slice(salt).unwrap();
        mac.update(name.as_bytes());
        format!("|1|{}|{}", b64(salt), b64(&mac.finalize().into_bytes()))
    }

    #[test]
    fn known_hosts_names_plain_and_hashed_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("known_hosts");
        let (k1, k2, k3, k4, k5) = (b"key one", b"key two", b"key six", b"key ten", b"key red");
        let text = format!(
            "# comment\n\
             @revoked bad.example {kind} {r}\n\
             *.example,!x.example,box.example {kind} {a}\n\
             {h} {kind} {b}\n\
             [git.example]:2222 {kind} {c}\n\
             {h2} {kind} {d}\n",
            kind = "ssh-ed25519",
            r = b64(k5),
            a = b64(k1),
            h = hashed(b"0123456789abcdef0123", "github.com"),
            b = b64(k2),
            c = b64(k3),
            h2 = hashed(b"saltsaltsaltsaltsalt", "build.internal"),
            d = b64(k4),
        );
        std::fs::write(&path, text).unwrap();
        assert_eq!(host_name(&path, k1).as_deref(), Some("box.example"));
        assert_eq!(host_name(&path, k2).as_deref(), Some("github.com"));
        assert_eq!(host_name(&path, k3).as_deref(), Some("git.example:2222"));
        assert_eq!(host_name(&path, k4), None, "a hashed name nobody offers");
        assert_eq!(host_name(&path, k5), None, "a revoked key names nothing");
        assert_eq!(host_name(&path, b"no such key"), None);

        // A name from the ssh config beside known_hosts opens a hashed entry.
        std::fs::write(
            tmp.path().join("config"),
            "Host build\n  HostName build.internal\n",
        )
        .unwrap();
        assert_eq!(host_name(&path, k4).as_deref(), Some("build.internal"));
    }

    #[test]
    fn sshsig_namespace_and_commit_author() {
        let data = ssh_key::SshSig::signed_data("git", ssh_key::HashAlg::Sha512, b"x").unwrap();
        assert_eq!(sshsig_namespace(&data), Some(b"git".as_slice()));
        assert_eq!(sshsig_namespace(b"session"), None);
        let commit = b"tree 4b82\nauthor Ada Lovelace <ada@example.com> 1700000000 +0000\n\
                       committer Bob <b@example.com> 1700000000 +0000\n\nauthor in the body\n";
        assert_eq!(commit_author(commit), Some("Ada Lovelace <ada@example.com>"));
        assert_eq!(commit_author(b"tree 4b82\n\nauthor A <a> 1 +0000\n"), None);
    }

    #[test]
    fn userauth_session_reads_the_session_and_user() {
        let mut d = Vec::new();
        put_string(&mut d, b"session");
        d.push(50);
        put_string(&mut d, b"git");
        assert_eq!(
            userauth_session(&d),
            Some((b"session".to_vec(), "git".into()))
        );
        assert_eq!(userauth_session(b"SSHSIG"), None);
        assert_eq!(userauth_session(&[]), None);
    }
}
