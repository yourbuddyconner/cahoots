//! A grant as a CBOR Web Token (RFC 8392): version 2 of the grant payload (chip apps spec §8).
//!
//! The payload is a CWT claims set, a canonical CBOR map with integer keys:
//!
//! | key | claim | value |
//! |---|---|---|
//! | 1 | `iss` (RFC 8392) | `"cahoots:circle:<circle_id hex>"` |
//! | 2 | `sub` (RFC 8392) | `"cahoots:key:<hex of SHA-256 of the grantee key>"`, the key id |
//! | 7 | `cti` (RFC 8392) | bstr 16, the grant's nonce |
//! | 8 | `cnf` (RFC 8747) | `{1: COSE_Key}`, the grantee's P-256 key: `{1: 2, -1: 1, -2: x, -3: y}` |
//! | -65537 | `cahoots-log` | uint, the last log position at which the grant holds |
//! | -65538 | `cahoots-parent` | bstr 32, SHA-256 of the parent's signed bytes; absent for a root |
//! | -65539 | `authorization_details` (RFC 9396) | array of detail maps |
//! | -65540 | `cahoots-depth` | uint, delegations still allowed below this grant |
//! | -65541 | `cahoots-grantor` | uint seat for a root grant, or bstr 33 key for a delegated one |
//! | -65542 | `cahoots-base` | uint, where a sign grant's use counter starts on the issuing badge, absent on a pad grant |
//!
//! `authorization_details` has no key in IANA's CWT Claims registry (RFC 9396 registers it for
//! JWT only; checked 2026-10-28), so it takes a private key, `-65539`, beside the Cahoots
//! claims. If a key is registered, this is the one place to change.
//!
//! Each detail is a map with text keys: `"type"` and, on the `cahoots:pad` detail only,
//! `"budget"` (bytes of pad). The details are one per purpose, ascending, typed
//! `cahoots:<purpose name>` (`cahoots:wireguard`, `cahoots:tls`, `cahoots:mls`, `cahoots:wifi`,
//! `cahoots:meshtastic`, `cahoots:other`) or `cahoots:purpose-<n>` for a purpose byte without a
//! name, then the `cahoots:pad` detail last.
//!
//! [`Claims::decode`] refuses anything that is not a grant: an unknown claim or detail type,
//! a missing claim, an issuer that is not a circle, a `sub` that is not the key id of the `cnf`
//! key, and any encoding that is not canonical.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::seat::SeatPublic;
use cahoots_core::CircleId;
use minicbor::data::Type;
use minicbor::Decoder;
use p256::elliptic_curve::sec1::ToEncodedPoint;

use crate::grant::{key_id, Grantor};
use crate::{canonical, encode_with, Enc, EncResult, ObjectError};

pub const ISS: i64 = 1;
pub const SUB: i64 = 2;
pub const AUD: i64 = 3;
pub const EXP: i64 = 4;
pub const CTI: i64 = 7;
pub const CNF: i64 = 8;
/// `cahoots-log`: `not_after` as a log position. Private.
pub const LOG_NOT_AFTER: i64 = -65537;
/// `cahoots-parent`: the parent's hash. Private.
pub const PARENT: i64 = -65538;
/// `authorization_details` (RFC 9396). Private: no CWT claim key is registered for it.
pub const AUTHORIZATION_DETAILS: i64 = -65539;
/// `cahoots-depth`: delegations still allowed. Private.
pub const DEPTH: i64 = -65540;
/// `cahoots-grantor`: who signed, as in version 1. Private.
pub const GRANTOR: i64 = -65541;
/// `cahoots-base`: the value a sign grant's use counter starts at, on the badge that issued
/// it. Signed into the grant so the count is read against it and never against a stored copy.
/// Private.
pub const BASE: i64 = -65542;

/// The `cnf` map's key for a COSE_Key (RFC 8747 §3.1).
const CNF_COSE_KEY: u8 = 1;

pub const ISS_PREFIX: &str = "cahoots:circle:";
pub const SUB_PREFIX: &str = "cahoots:key:";
pub const TYPE_PREFIX: &str = "cahoots:";
/// The detail type that carries the pad budget.
pub const PAD_TYPE: &str = "cahoots:pad";
const NUMBERED: &str = "cahoots:purpose-";
/// The detail type of a sign grant.
pub const SIGN_TYPE: &str = "cahoots:sign";
/// The detail type of a channel grant.
pub const CHANNEL_TYPE: &str = "cahoots:channel";
/// An `aud` naming a seat: `"cahoots:seat:<circle id hex>:<seat>"`.
pub const AUD_PREFIX: &str = "cahoots:seat:";

/// The actions a sign grant can allow.
const SIGN_ACTIONS: [&str; 3] = ["git-commit", "ssh-login", "file"];
/// The actions a channel grant can allow, in the order they are written.
const CHANNEL_ACTIONS: [&str; 4] = ["read", "post", "admit", "govern"];
/// The most signatures a sign grant can allow.
const MAX_SIGN_BUDGET: u32 = 1000;
/// The most entries in a detail's `actions` or `locations`.
const MAX_LIST: u64 = 64;

/// The most details a grant has: one per purpose byte, and the pad.
const MAX_DETAILS: u64 = 257;

/// One RFC 9396 authorization detail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    pub r#type: String,
    /// Bytes of pad on the [`PAD_TYPE`] detail; signatures on the [`SIGN_TYPE`] detail.
    pub budget: Option<u32>,
    /// The actions a [`SIGN_TYPE`] detail allows.
    pub actions: Option<Vec<String>>,
    /// Who commits are signed as, on a [`SIGN_TYPE`] detail.
    pub author: Option<String>,
    /// Where a [`SIGN_TYPE`] detail applies; may be empty.
    pub locations: Option<Vec<String>>,
    /// Which agent holds the grant, on a [`SIGN_TYPE`] detail; may be empty.
    pub agent: Option<String>,
    /// The channel id, 32 lowercase hex characters, on a [`CHANNEL_TYPE`] detail.
    pub channel: Option<String>,
}

impl Detail {
    /// A detail with a type and nothing else.
    pub(crate) fn bare(r#type: String) -> Detail {
        Detail {
            r#type,
            ..Detail::default()
        }
    }

    fn field_count(&self) -> u64 {
        1 + u64::from(self.budget.is_some())
            + u64::from(self.actions.is_some())
            + u64::from(self.author.is_some())
            + u64::from(self.locations.is_some())
            + u64::from(self.agent.is_some())
            + u64::from(self.channel.is_some())
    }
}

/// What a grant allows, checked: see [`Claims::kind`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Pad bytes for the listed purposes.
    Pad { purposes: Vec<u8>, budget: u32 },
    /// Signatures by an agent.
    Sign(SignDetail),
    /// Actions in one channel. A budget of 0 is no limit.
    Channel {
        channel: [u8; 16],
        actions: Vec<String>,
        budget: u32,
    },
}

/// The one detail of a sign grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignDetail {
    pub actions: Vec<String>,
    pub author: String,
    pub locations: Vec<String>,
    pub agent: String,
    pub budget: u32,
}

/// A grant's CWT claims set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    /// The seat a sign grant is for ([`audience`]); absent on a pad grant.
    pub aud: Option<String>,
    /// When a sign grant ends, in Unix seconds; absent on a pad grant.
    pub exp: Option<u64>,
    pub cnf: SeatPublic,
    pub cti: [u8; 16],
    pub authorization_details: Vec<Detail>,
    /// The last log position a pad grant holds at; absent on a sign grant.
    pub log_not_after: Option<u64>,
    pub parent: Option<[u8; 32]>,
    pub depth: u8,
    pub grantor: Grantor,
    /// Where a sign grant's use counter starts on the issuing badge. Absent on a pad grant.
    pub base: Option<u64>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != 2 * N {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = digit(s[2 * i])? << 4 | digit(s[2 * i + 1])?;
    }
    Some(out)
}

/// `iss` for a circle.
pub fn issuer(circle_id: &CircleId) -> String {
    format!("{ISS_PREFIX}{}", hex(circle_id))
}

/// The circle an `iss` names, if it is one.
pub fn circle_of(iss: &str) -> Option<CircleId> {
    unhex(iss.strip_prefix(ISS_PREFIX)?)
}

/// `aud` for a seat of a circle.
pub fn audience(circle_id: &CircleId, seat: u8) -> String {
    format!("{AUD_PREFIX}{}:{seat}", hex(circle_id))
}

/// The circle and seat an `aud` names, if it names one.
pub fn seat_of(aud: &str) -> Option<(CircleId, u8)> {
    let (circle, seat) = aud.strip_prefix(AUD_PREFIX)?.split_once(':')?;
    // Only the shortest decimal spelling.
    if seat.is_empty()
        || (seat.len() > 1 && seat.starts_with('0'))
        || !seat.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((unhex(circle)?, seat.parse().ok()?))
}

/// `sub` for a grantee key: its key id.
pub fn subject(key: &SeatPublic) -> String {
    format!("{SUB_PREFIX}{}", hex(&key_id(key)))
}

/// The detail type of a purpose byte.
pub fn purpose_type(purpose: u8) -> String {
    match crate::card::purpose_name(purpose) {
        Some(name) if purpose <= 5 => format!("{TYPE_PREFIX}{name}"),
        _ => format!("{NUMBERED}{purpose}"),
    }
}

/// The purpose byte a detail type names, if it is a purpose type.
pub fn type_purpose(t: &str) -> Option<u8> {
    if let Some(n) = t.strip_prefix(NUMBERED) {
        // Only the shortest decimal spelling; the re-encode check refuses a named purpose.
        if n.is_empty()
            || (n.len() > 1 && n.starts_with('0'))
            || !n.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        return n.parse().ok();
    }
    let name = t.strip_prefix(TYPE_PREFIX)?;
    (0..=5u8).find(|p| crate::card::purpose_name(*p) == Some(name))
}

/// The uncompressed coordinates of a valid key, or zeros for one that is not a point (which a
/// decoded `Claims` never holds).
fn coordinates(key: &SeatPublic) -> ([u8; 32], [u8; 32]) {
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    if let Ok(pk) = p256::PublicKey::from_sec1_bytes(&key.0) {
        let point = pk.to_encoded_point(false);
        if let (Some(px), Some(py)) = (point.x(), point.y()) {
            x.copy_from_slice(px);
            y.copy_from_slice(py);
        }
    }
    (x, y)
}

fn from_coordinates(x: &[u8], y: &[u8]) -> Option<SeatPublic> {
    if x.len() != 32 || y.len() != 32 {
        return None;
    }
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..33].copy_from_slice(x);
    sec1[33..].copy_from_slice(y);
    let pk = p256::PublicKey::from_sec1_bytes(&sec1).ok()?;
    SeatPublic::parse(pk.to_encoded_point(true).as_bytes()).ok()
}

impl Claims {
    /// The canonical CBOR claims set: keys in the order of their encodings.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(
            7 + u64::from(self.aud.is_some())
                + u64::from(self.exp.is_some())
                + u64::from(self.log_not_after.is_some())
                + u64::from(self.parent.is_some())
                + u64::from(self.base.is_some()),
        )?;
        e.i64(ISS)?.str(&self.iss)?;
        e.i64(SUB)?.str(&self.sub)?;
        if let Some(a) = &self.aud {
            e.i64(AUD)?.str(a)?;
        }
        if let Some(t) = self.exp {
            e.i64(EXP)?.u64(t)?;
        }
        e.i64(CTI)?.bytes(&self.cti)?;
        let (x, y) = coordinates(&self.cnf);
        e.i64(CNF)?.map(1)?.u8(CNF_COSE_KEY)?.map(4)?;
        e.u8(1)?.u8(2)?; // kty: EC2
        e.i8(-1)?.u8(1)?; // crv: P-256
        e.i8(-2)?.bytes(&x)?;
        e.i8(-3)?.bytes(&y)?;
        if let Some(l) = self.log_not_after {
            e.i64(LOG_NOT_AFTER)?.u64(l)?;
        }
        if let Some(p) = &self.parent {
            e.i64(PARENT)?.bytes(p)?;
        }
        e.i64(AUTHORIZATION_DETAILS)?
            .array(self.authorization_details.len() as u64)?;
        for d in &self.authorization_details {
            // Text keys in canonical order: shorter first, then bytewise.
            e.map(d.field_count())?;
            e.str("type")?.str(&d.r#type)?;
            if let Some(a) = &d.agent {
                e.str("agent")?.str(a)?;
            }
            if let Some(a) = &d.author {
                e.str("author")?.str(a)?;
            }
            if let Some(b) = d.budget {
                e.str("budget")?.u32(b)?;
            }
            if let Some(list) = &d.actions {
                e.str("actions")?.array(list.len() as u64)?;
                for a in list {
                    e.str(a)?;
                }
            }
            if let Some(c) = &d.channel {
                e.str("channel")?.str(c)?;
            }
            if let Some(list) = &d.locations {
                e.str("locations")?.array(list.len() as u64)?;
                for l in list {
                    e.str(l)?;
                }
            }
        }
        e.i64(DEPTH)?.u8(self.depth)?;
        e.i64(GRANTOR)?;
        match &self.grantor {
            Grantor::Seat(s) => e.u8(*s)?,
            Grantor::Key(k) => e.bytes(&k.0)?,
        };
        if let Some(b) = self.base {
            e.i64(BASE)?.u64(b)?;
        }
        Ok(())
    }

    /// Parse a canonical claims set that is a grant.
    pub fn decode(bytes: &[u8]) -> Result<Claims, ObjectError> {
        let m = ObjectError::Malformed;
        let mut d = Decoder::new(bytes);
        let n = definite_map(&mut d)?;
        if n > 18 {
            return Err(m);
        }
        let mut iss = None;
        let mut sub = None;
        let mut aud = None;
        let mut exp = None;
        let mut cti = None;
        let mut cnf = None;
        let mut log = None;
        let mut parent = None;
        let mut details = None;
        let mut depth = None;
        let mut grantor = None;
        let mut base = None;
        for _ in 0..n {
            let key = i64::try_from(d.int().map_err(|_| m)?).map_err(|_| m)?;
            match key {
                ISS => once(&mut iss, String::from(d.str().map_err(|_| m)?))?,
                SUB => once(&mut sub, String::from(d.str().map_err(|_| m)?))?,
                AUD => once(&mut aud, String::from(d.str().map_err(|_| m)?))?,
                EXP => once(&mut exp, d.u64().map_err(|_| m)?)?,
                CTI => once(
                    &mut cti,
                    d.bytes().map_err(|_| m)?.try_into().map_err(|_| m)?,
                )?,
                CNF => once(&mut cnf, read_cnf(&mut d)?)?,
                LOG_NOT_AFTER => once(&mut log, d.u64().map_err(|_| m)?)?,
                PARENT => once(
                    &mut parent,
                    d.bytes().map_err(|_| m)?.try_into().map_err(|_| m)?,
                )?,
                AUTHORIZATION_DETAILS => once(&mut details, read_details(&mut d)?)?,
                DEPTH => once(
                    &mut depth,
                    d.u64().map_err(|_| m)?.try_into().map_err(|_| m)?,
                )?,
                GRANTOR => once(&mut grantor, read_grantor(&mut d)?)?,
                BASE => once(&mut base, d.u64().map_err(|_| m)?)?,
                _ => return Err(m),
            }
        }
        if d.position() != bytes.len() {
            return Err(m);
        }
        let claims = Claims {
            iss: iss.ok_or(m)?,
            sub: sub.ok_or(m)?,
            aud,
            exp,
            cnf: cnf.ok_or(m)?,
            cti: cti.ok_or(m)?,
            authorization_details: details.ok_or(m)?,
            log_not_after: log,
            parent,
            depth: depth.ok_or(m)?,
            grantor: grantor.ok_or(m)?,
            base,
        };
        // The kind must be a grant's: this checks the issuer, the details and that `sub`
        // is the key id of `cnf`.
        claims.kind()?;
        let encoded = claims.encode();
        canonical(claims, encoded, bytes)
    }

    /// The circle and what the grant allows, checked: what [`crate::Grant::from_v2`] reads for
    /// a pad grant. A pad grant has a log claim, the [`PAD_TYPE`] detail last, no `aud`, no
    /// `exp` and no base; a sign grant has one [`SIGN_TYPE`] detail, an `aud` naming the
    /// issuing circle, a base, no log claim and depth 0; a channel grant has one
    /// [`CHANNEL_TYPE`] detail, a log claim, and no `aud`, `exp` or base. Anything else is
    /// `Malformed`.
    pub fn kind(&self) -> Result<(CircleId, Kind), ObjectError> {
        let m = ObjectError::Malformed;
        let circle_id = circle_of(&self.iss).ok_or(m)?;
        if self.sub != subject(&self.cnf) {
            return Err(m);
        }
        let kind = if self
            .authorization_details
            .iter()
            .any(|d| d.r#type == CHANNEL_TYPE)
        {
            self.channel_kind()?
        } else if self.log_not_after.is_some() {
            self.pad_kind()?
        } else {
            self.sign_kind(&circle_id)?
        };
        Ok((circle_id, kind))
    }

    fn pad_kind(&self) -> Result<Kind, ObjectError> {
        let m = ObjectError::Malformed;
        if self.aud.is_some() || self.exp.is_some() || self.base.is_some() {
            return Err(m);
        }
        let (pad, purposes) = self.authorization_details.split_last().ok_or(m)?;
        if pad.r#type != PAD_TYPE
            || pad.actions.is_some()
            || pad.author.is_some()
            || pad.locations.is_some()
            || pad.agent.is_some()
            || pad.channel.is_some()
        {
            return Err(m);
        }
        let budget = pad.budget.ok_or(m)?;
        let mut list = Vec::new();
        for d in purposes {
            if *d != Detail::bare(d.r#type.clone()) {
                return Err(m);
            }
            let p = type_purpose(&d.r#type).ok_or(m)?;
            // One spelling per purpose: a named purpose is never written as a number.
            if d.r#type != purpose_type(p) {
                return Err(ObjectError::NotCanonical);
            }
            list.push(p);
        }
        if !list.windows(2).all(|w| w[0] < w[1]) {
            return Err(m);
        }
        Ok(Kind::Pad {
            purposes: list,
            budget,
        })
    }

    fn channel_kind(&self) -> Result<Kind, ObjectError> {
        let m = ObjectError::Malformed;
        if self.aud.is_some()
            || self.exp.is_some()
            || self.base.is_some()
            || self.log_not_after.is_none()
        {
            return Err(m);
        }
        let [d] = self.authorization_details.as_slice() else {
            return Err(m);
        };
        if d.author.is_some() || d.locations.is_some() || d.agent.is_some() {
            return Err(m);
        }
        let actions = d.actions.clone().ok_or(m)?;
        let budget = d.budget.ok_or(m)?;
        let channel = unhex(d.channel.as_deref().ok_or(m)?).ok_or(m)?;
        // Each action once, in the order of CHANNEL_ACTIONS.
        let mut last = None;
        for a in &actions {
            let at = CHANNEL_ACTIONS.iter().position(|c| *c == a).ok_or(m)?;
            if last.is_some_and(|l| at <= l) {
                return Err(m);
            }
            last = Some(at);
        }
        if actions.is_empty() {
            return Err(m);
        }
        Ok(Kind::Channel {
            channel,
            actions,
            budget,
        })
    }

    fn sign_kind(&self, circle_id: &CircleId) -> Result<Kind, ObjectError> {
        let m = ObjectError::Malformed;
        let aud = self.aud.as_deref().ok_or(m)?;
        if seat_of(aud).ok_or(m)?.0 != *circle_id
            || self.depth != 0
            || self.parent.is_some()
            || self.base.is_none()
        {
            return Err(m);
        }
        let [d] = self.authorization_details.as_slice() else {
            return Err(m);
        };
        if d.r#type != SIGN_TYPE || d.channel.is_some() {
            return Err(m);
        }
        let actions = d.actions.clone().ok_or(m)?;
        let author = d.author.clone().ok_or(m)?;
        let budget = d.budget.ok_or(m)?;
        if actions.is_empty()
            || author.is_empty()
            || !(1..=MAX_SIGN_BUDGET).contains(&budget)
            || !actions.iter().all(|a| SIGN_ACTIONS.contains(&a.as_str()))
        {
            return Err(m);
        }
        Ok(Kind::Sign(SignDetail {
            actions,
            author,
            locations: d.locations.clone().unwrap_or_default(),
            agent: d.agent.clone().unwrap_or_default(),
            budget,
        }))
    }
}

fn once<T>(slot: &mut Option<T>, value: T) -> Result<(), ObjectError> {
    if slot.is_some() {
        return Err(ObjectError::Malformed);
    }
    *slot = Some(value);
    Ok(())
}

fn definite_map(d: &mut Decoder<'_>) -> Result<u64, ObjectError> {
    match d.map() {
        Ok(Some(n)) => Ok(n),
        Ok(None) => Err(ObjectError::NotCanonical),
        Err(_) => Err(ObjectError::Malformed),
    }
}

fn definite_array(d: &mut Decoder<'_>) -> Result<u64, ObjectError> {
    match d.array() {
        Ok(Some(n)) => Ok(n),
        Ok(None) => Err(ObjectError::NotCanonical),
        Err(_) => Err(ObjectError::Malformed),
    }
}

/// `{1: {1: 2, -1: 1, -2: x, -3: y}}`, in that order.
fn read_cnf(d: &mut Decoder<'_>) -> Result<SeatPublic, ObjectError> {
    let m = ObjectError::Malformed;
    if definite_map(d)? != 1 || d.u8().map_err(|_| m)? != CNF_COSE_KEY {
        return Err(m);
    }
    if definite_map(d)? != 4 {
        return Err(m);
    }
    expect_int(d, 1)?;
    expect_int(d, 2)?; // kty: EC2
    expect_int(d, -1)?;
    expect_int(d, 1)?; // crv: P-256
    expect_int(d, -2)?;
    let x = d.bytes().map_err(|_| m)?;
    expect_int(d, -3)?;
    let y = d.bytes().map_err(|_| m)?;
    from_coordinates(x, y).ok_or(m)
}

fn expect_int(d: &mut Decoder<'_>, want: i64) -> Result<(), ObjectError> {
    let m = ObjectError::Malformed;
    if i64::try_from(d.int().map_err(|_| m)?).map_err(|_| m)? != want {
        return Err(m);
    }
    Ok(())
}

fn read_details(d: &mut Decoder<'_>) -> Result<Vec<Detail>, ObjectError> {
    let m = ObjectError::Malformed;
    let n = definite_array(d)?;
    if n > MAX_DETAILS {
        return Err(m);
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let fields = definite_map(d)?;
        if !(1..=7).contains(&fields) {
            return Err(m);
        }
        let mut r#type = None;
        let mut budget = None;
        let mut actions = None;
        let mut author = None;
        let mut locations = None;
        let mut agent = None;
        let mut channel = None;
        for _ in 0..fields {
            match d.str().map_err(|_| m)? {
                "type" => once(&mut r#type, String::from(d.str().map_err(|_| m)?))?,
                "budget" => once(
                    &mut budget,
                    d.u64().map_err(|_| m)?.try_into().map_err(|_| m)?,
                )?,
                "actions" => once(&mut actions, read_strings(d)?)?,
                "author" => once(&mut author, String::from(d.str().map_err(|_| m)?))?,
                "locations" => once(&mut locations, read_strings(d)?)?,
                "agent" => once(&mut agent, String::from(d.str().map_err(|_| m)?))?,
                "channel" => once(&mut channel, String::from(d.str().map_err(|_| m)?))?,
                _ => return Err(m),
            }
        }
        out.push(Detail {
            r#type: r#type.ok_or(m)?,
            budget,
            actions,
            author,
            locations,
            agent,
            channel,
        });
    }
    Ok(out)
}

fn read_strings(d: &mut Decoder<'_>) -> Result<Vec<String>, ObjectError> {
    let m = ObjectError::Malformed;
    let n = definite_array(d)?;
    if n > MAX_LIST {
        return Err(m);
    }
    (0..n)
        .map(|_| d.str().map(String::from).map_err(|_| m))
        .collect()
}

fn read_grantor(d: &mut Decoder<'_>) -> Result<Grantor, ObjectError> {
    let m = ObjectError::Malformed;
    match d.datatype().map_err(|_| m)? {
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => Ok(Grantor::Seat(
            d.u64().map_err(|_| m)?.try_into().map_err(|_| m)?,
        )),
        Type::Bytes => Ok(Grantor::Key(
            SeatPublic::parse(d.bytes().map_err(|_| m)?).map_err(|_| m)?,
        )),
        _ => Err(m),
    }
}
