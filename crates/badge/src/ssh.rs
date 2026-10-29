//! What OpenSSH asks an agent to sign, read by the badge so it can show it.
//!
//! [`parse`] accepts two shapes and refuses everything else:
//!
//! - a userauth request (RFC 4252 section 7): the session id, byte 50, the user, the service
//!   `ssh-connection`, the method `publickey` (or OpenSSH's `publickey-hostbound-v00@openssh.com`,
//!   which adds the server's host key at the end), true, the algorithm `ecdsa-sha2-nistp256`
//!   and the client's key;
//! - an `SSHSIG` envelope (OpenSSH `PROTOCOL.sshsig`): the six bytes `SSHSIG`, a namespace, a
//!   reserved string, the hash algorithm `sha512` or `sha256`, and the hash of the content.
//!
//! [`check_host`] checks the laptop's session binding: the session id it names must be the one
//! in the request, and the server's signature over it must verify under the host key. Only
//! Ed25519 and P-256 host keys are checked; any other key names no host.
//!
//! The reader is hand-written over a byte slice: every length is checked against what is left,
//! and nothing here panics on any input.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::seat::SeatPublic;
use cahoots_custodian::wire::SshHost;
use p256::ecdsa::signature::Verifier;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::{Digest, Sha256, Sha512};

/// The key and signature algorithm of a seat key.
pub const SEAT_ALG: &str = "ecdsa-sha2-nistp256";
const NISTP256: &str = "nistp256";
const ED25519: &str = "ssh-ed25519";
const USERAUTH_REQUEST: u8 = 50;
const SSHSIG_MAGIC: &[u8; 6] = b"SSHSIG";

/// What a request to sign is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signed {
    /// A login: the user the laptop logs in as, the session's id, the key it logs in with (an
    /// SSH public key blob), and the server's host key when the method is host-bound.
    Login {
        user: String,
        session_id: Vec<u8>,
        key: Vec<u8>,
        host_key: Option<Vec<u8>>,
    },
    /// A signature over content the request names only by its hash.
    Sig {
        namespace: String,
        hash_alg: String,
        hash: Vec<u8>,
    },
}

/// Why `parse` refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshError {
    /// A length runs past the end.
    Truncated,
    /// Bytes are left after a whole request.
    Trailing,
    /// Neither a userauth request nor an `SSHSIG` envelope.
    Unknown,
    /// A field holds a value the badge does not sign: another service, method or algorithm, a
    /// name that is not UTF-8, a hash of the wrong length.
    Field,
}

/// The laptop's session binding, as the badge checked it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostCheck {
    /// The session id matches and the server signed it with the host key the laptop named.
    /// The name is still the laptop's: it came from `known_hosts`.
    Verified(String),
    /// A name the badge could not check. [`check_host`] does not return it today: a binding
    /// that does not verify names no host.
    Unverified(String),
    /// No binding, a binding for another session, a signature that does not verify, or a host
    /// key of a type the badge does not check (RSA).
    None,
}

/// A cursor over SSH wire data.
struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b }
    }

    fn raw(&mut self, n: usize) -> Result<&'a [u8], SshError> {
        if n > self.b.len() {
            return Err(SshError::Truncated);
        }
        let (head, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, SshError> {
        Ok(self.raw(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, SshError> {
        let b = self.raw(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Result<&'a [u8], SshError> {
        let n = usize::try_from(self.u32()?).map_err(|_| SshError::Truncated)?;
        self.raw(n)
    }

    fn text(&mut self) -> Result<&'a str, SshError> {
        core::str::from_utf8(self.string()?).map_err(|_| SshError::Field)
    }

    fn end(&self) -> Result<(), SshError> {
        if self.b.is_empty() {
            Ok(())
        } else {
            Err(SshError::Trailing)
        }
    }
}

/// Read `data` as a userauth request or an `SSHSIG` envelope.
pub fn parse(data: &[u8]) -> Result<Signed, SshError> {
    if data.starts_with(SSHSIG_MAGIC) {
        parse_sshsig(&data[SSHSIG_MAGIC.len()..])
    } else {
        parse_userauth(data)
    }
}

fn parse_userauth(data: &[u8]) -> Result<Signed, SshError> {
    let mut r = Reader::new(data);
    let session_id = r.string()?;
    if r.byte()? != USERAUTH_REQUEST {
        return Err(SshError::Unknown);
    }
    let user = r.text()?;
    if r.string()? != b"ssh-connection" {
        return Err(SshError::Field);
    }
    let hostbound = match r.string()? {
        b"publickey" => false,
        b"publickey-hostbound-v00@openssh.com" => true,
        _ => return Err(SshError::Field),
    };
    // The "has signature" flag: a request without it is a query and is never signed.
    if r.byte()? != 1 {
        return Err(SshError::Field);
    }
    if r.string()? != SEAT_ALG.as_bytes() {
        return Err(SshError::Field);
    }
    let key = r.string()?;
    let host_key = if hostbound {
        Some(r.string()?.to_vec())
    } else {
        None
    };
    r.end()?;
    Ok(Signed::Login {
        user: String::from(user),
        session_id: session_id.to_vec(),
        key: key.to_vec(),
        host_key,
    })
}

fn parse_sshsig(data: &[u8]) -> Result<Signed, SshError> {
    let mut r = Reader::new(data);
    let namespace = r.text()?;
    let _reserved = r.string()?;
    let hash_alg = r.text()?;
    let hash = r.string()?;
    r.end()?;
    let want = match hash_alg {
        "sha512" => 64,
        "sha256" => 32,
        _ => return Err(SshError::Field),
    };
    if namespace.is_empty() || hash.len() != want {
        return Err(SshError::Field);
    }
    Ok(Signed::Sig {
        namespace: String::from(namespace),
        hash_alg: String::from(hash_alg),
        hash: hash.to_vec(),
    })
}

/// True if `sig` is an `SSHSIG` envelope whose hash is the hash of `content`.
pub fn check_content(sig: &Signed, content: &[u8]) -> bool {
    let Signed::Sig { hash_alg, hash, .. } = sig else {
        return false;
    };
    match hash_alg.as_str() {
        "sha512" => Sha512::digest(content).as_slice() == hash.as_slice(),
        "sha256" => Sha256::digest(content).as_slice() == hash.as_slice(),
        _ => false,
    }
}

/// Check the laptop's session binding against the session id in the request.
pub fn check_host(host: &SshHost, session_id: &[u8]) -> HostCheck {
    if host.session_id != session_id || !host_signed(&host.host_key, &host.host_sig, session_id) {
        return HostCheck::None;
    }
    HostCheck::Verified(host.name.clone())
}

/// True if `sig` (an SSH signature blob) is the server's signature over `session_id` under
/// `key` (an SSH public key blob), for an Ed25519 or P-256 key.
fn host_signed(key: &[u8], sig: &[u8], session_id: &[u8]) -> bool {
    host_signed_inner(key, sig, session_id).unwrap_or(false)
}

fn host_signed_inner(key: &[u8], sig: &[u8], session_id: &[u8]) -> Result<bool, SshError> {
    let mut k = Reader::new(key);
    let mut s = Reader::new(sig);
    let kind = k.string()?;
    if s.string()? != kind {
        return Ok(false);
    }
    let sig_bytes = s.string()?;
    s.end()?;
    if kind == ED25519.as_bytes() {
        let pk: [u8; 32] = k.string()?.try_into().map_err(|_| SshError::Field)?;
        k.end()?;
        let sig: [u8; 64] = sig_bytes.try_into().map_err(|_| SshError::Field)?;
        let Ok(pk) = ed25519_dalek::VerifyingKey::from_bytes(&pk) else {
            return Ok(false);
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig);
        Ok(pk.verify_strict(session_id, &sig).is_ok())
    } else if kind == SEAT_ALG.as_bytes() {
        if k.string()? != NISTP256.as_bytes() {
            return Ok(false);
        }
        let q = k.string()?;
        k.end()?;
        let Ok(pk) = p256::ecdsa::VerifyingKey::from_sec1_bytes(q) else {
            return Ok(false);
        };
        let mut rs = Reader::new(sig_bytes);
        let r = mpint32(rs.string()?)?;
        let sc = mpint32(rs.string()?)?;
        rs.end()?;
        let Ok(sig) = p256::ecdsa::Signature::from_scalars(r, sc) else {
            return Ok(false);
        };
        // P-256 ECDSA over SHA-256(session id), as the server signs the exchange hash.
        Ok(pk.verify(session_id, &sig).is_ok())
    } else {
        // RSA and anything else: not checked here.
        Ok(false)
    }
}

/// A positive SSH mpint of at most 32 bytes of value, left-padded to 32.
fn mpint32(b: &[u8]) -> Result<[u8; 32], SshError> {
    if b.first().is_some_and(|x| x & 0x80 != 0) {
        return Err(SshError::Field);
    }
    let start = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    let v = &b[start..];
    if v.len() > 32 {
        return Err(SshError::Field);
    }
    let mut out = [0u8; 32];
    out[32 - v.len()..].copy_from_slice(v);
    Ok(out)
}

/// Append `b` as an SSH string.
fn put_string(out: &mut Vec<u8>, b: &[u8]) {
    // A seat signature's fields are tens of bytes; no length here comes near u32::MAX.
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// Append a 32-byte big-endian unsigned value as an SSH mpint: no leading zeros, and one
/// zero byte in front when the top bit is set.
fn put_mpint(out: &mut Vec<u8>, v: &[u8; 32]) {
    let start = v.iter().position(|&x| x != 0).unwrap_or(v.len());
    let v = &v[start..];
    let mut m = Vec::with_capacity(v.len() + 1);
    if v.first().is_some_and(|x| x & 0x80 != 0) {
        m.push(0);
    }
    m.extend_from_slice(v);
    put_string(out, &m);
}

/// A seat public key as an SSH public key blob: `ecdsa-sha2-nistp256`, `nistp256`, and the
/// point uncompressed. All zeros for a key that does not parse.
pub fn public_key_blob(seat: &SeatPublic) -> Vec<u8> {
    let point = match p256::PublicKey::from_sec1_bytes(&seat.0) {
        Ok(p) => p.to_encoded_point(false),
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    put_string(&mut out, SEAT_ALG.as_bytes());
    put_string(&mut out, NISTP256.as_bytes());
    put_string(&mut out, point.as_bytes());
    out
}

/// A seat signature (`r || s`) as an SSH signature blob: `ecdsa-sha2-nistp256`, then a string
/// holding `r` and `s` as mpints.
pub fn encode_signature(sig: &[u8; 64]) -> Vec<u8> {
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&sig[..32]);
    s.copy_from_slice(&sig[32..]);
    let mut inner = Vec::new();
    put_mpint(&mut inner, &r);
    put_mpint(&mut inner, &s);
    let mut out = Vec::new();
    put_string(&mut out, SEAT_ALG.as_bytes());
    put_string(&mut out, &inner);
    out
}

/// What the badge reads from a commit's text: the author's name and the subject line.
/// `None` if the text is not a commit (it has no `tree` line first, or no `author` line), or
/// it has more than one `author` line.
pub fn describe_commit(content: &[u8]) -> Option<(String, String)> {
    let text = String::from_utf8_lossy(content);
    let mut lines = text.split('\n');
    if !lines.next()?.starts_with("tree ") {
        return None;
    }
    let mut author = None;
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("author ") {
            // git shows the first author; text with two (git fsck's multipleAuthors) could
            // show one name here and another there, so it is refused.
            if author.is_some() {
                return None;
            }
            // "Name <email> time zone": the name is before the address.
            let name = rest.split(" <").next().unwrap_or(rest).trim();
            author = Some(String::from(name));
        }
    }
    let subject = lines.next().unwrap_or("").trim_end_matches('\r');
    Some((author?, String::from(subject)))
}
