//! The circle manifest (spec §5): the roster of seat keys and tiers, the circle's parameters,
//! and the floor table, as one canonical CBOR map that every seat signs at the end of the
//! ceremony. The signed manifest is the circle's root document.
//!
//! | key | name | value |
//! |---|---|---|
//! | 0 | version | 1 |
//! | 1 | circle_id | bstr, 16 bytes |
//! | 2 | params | bstr, `CircleParams::encode()` |
//! | 3 | seats | array of `{0: seat, 1: key bstr 33, 2: tier, 3: evidence bstr}`, in seat order |
//! | 4 | floor | array of `{0: purpose, 1: requirement, 2: budget}`, in purpose order |
//! | 5 | genesis | uint, the log position of the ceremony |
//!
//! Canonical means RFC 8949 §4.2.1: definite lengths, shortest integers, integer map keys in
//! ascending order. `decode` re-encodes what it read and refuses any difference.

use alloc::vec::Vec;
use core::convert::Infallible;
use minicbor::encode::Error as EncodeError;
use minicbor::{Decoder, Encoder};
use sha2::{Digest, Sha256};

use crate::circle_id::CircleId;
use crate::control::Purpose;
use crate::cose;
use crate::params::CircleParams;
use crate::seat::SeatPublic;

/// The manifest format version, key 0 of the body.
pub const MANIFEST_VERSION: u8 = 1;
/// External AAD of every attestation: a seat's detached COSE_Sign1 over the manifest body.
pub const MANIFEST_AAD: &[u8] = b"cahoots/manifest/v1";

/// The highest declared device tier.
const MAX_TIER: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// Not a manifest: does not parse, a field missing or of the wrong type, trailing bytes,
    /// seats that are not exactly `0..n` in order, a key that is not a valid point.
    Malformed,
    /// Parses, but is not the canonical encoding of what it holds.
    NotCanonical,
    /// A manifest version other than 1.
    Version,
    /// Not exactly `n` attestations.
    AttestationCount,
    /// An attestation that does not verify, or names a seat outside the roster. Carries the
    /// seat its key id names.
    BadAttestation(u8),
    /// Two attestations name the same seat.
    DuplicateAttestation(u8),
}

/// What a floor entry asks for before a slice of the pad is spent on its purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Requirement {
    /// The purpose is never served.
    Never = 0,
    /// A grant from the owner suffices.
    Grant = 1,
    /// The owner must be present.
    Present = 2,
}

impl TryFrom<u8> for Requirement {
    type Error = ManifestError;
    fn try_from(v: u8) -> Result<Self, ManifestError> {
        Ok(match v {
            0 => Requirement::Never,
            1 => Requirement::Grant,
            2 => Requirement::Present,
            _ => return Err(ManifestError::Malformed),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub seat: u8,
    pub key: SeatPublic,
    pub tier: u8,
    /// Empty in this version; will hold a chip's proof that the key lives in it.
    pub evidence: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloorEntry {
    /// A [`Purpose`] as its byte.
    pub purpose: u8,
    pub requirement: Requirement,
    /// Bytes of pad the purpose may spend.
    pub budget: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub circle_id: CircleId,
    pub params: CircleParams,
    /// One per member, in seat order.
    pub seats: Vec<Seat>,
    /// In ascending purpose order.
    pub floor: Vec<FloorEntry>,
    /// The log position of the ceremony; 0.
    pub genesis: u64,
}

/// A manifest body with one attestation per seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedManifest {
    /// The canonical manifest bytes.
    pub body: Vec<u8>,
    /// Detached COSE_Sign1 over `body`, one per seat.
    pub attestations: Vec<Vec<u8>>,
}

const PURPOSES: [Purpose; 6] = [
    Purpose::WireGuard,
    Purpose::Tls,
    Purpose::Mls,
    Purpose::Wifi,
    Purpose::Meshtastic,
    Purpose::Other,
];

type Enc = Encoder<Vec<u8>>;
type EncResult = Result<(), EncodeError<Infallible>>;

impl Manifest {
    /// The floor written at the ceremony: every purpose, owner present, a slice of budget.
    pub fn default_floor(params: &CircleParams) -> Vec<FloorEntry> {
        PURPOSES
            .iter()
            .map(|&p| FloorEntry {
                purpose: p as u8,
                requirement: Requirement::Present,
                budget: params.slice_len(),
            })
            .collect()
    }

    /// The canonical CBOR body.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        // Writing to a Vec cannot fail.
        let _ = self.write(&mut e);
        e.into_writer()
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(6)?;
        e.u8(0)?.u8(MANIFEST_VERSION)?;
        e.u8(1)?.bytes(&self.circle_id)?;
        e.u8(2)?.bytes(&self.params.encode())?;
        e.u8(3)?.array(self.seats.len() as u64)?;
        for s in &self.seats {
            e.map(4)?;
            e.u8(0)?.u8(s.seat)?;
            e.u8(1)?.bytes(&s.key.0)?;
            e.u8(2)?.u8(s.tier)?;
            e.u8(3)?.bytes(&s.evidence)?;
        }
        e.u8(4)?.array(self.floor.len() as u64)?;
        for f in &self.floor {
            e.map(3)?;
            e.u8(0)?.u8(f.purpose)?;
            e.u8(1)?.u8(f.requirement as u8)?;
            e.u8(2)?.u32(f.budget)?;
        }
        e.u8(5)?.u64(self.genesis)?;
        Ok(())
    }

    /// Parse a canonical body. Anything that re-encodes differently is `NotCanonical`.
    pub fn decode(bytes: &[u8]) -> Result<Manifest, ManifestError> {
        let m = ManifestError::Malformed;
        let mut d = Decoder::new(bytes);
        let mut version = None;
        let mut circle_id = None;
        let mut params = None;
        let mut seats = None;
        let mut floor = None;
        let mut genesis = None;
        for _ in 0..map_len(&mut d)? {
            match d.u64().map_err(|_| m)? {
                0 => {
                    let v = d.u64().map_err(|_| m)?;
                    if v != u64::from(MANIFEST_VERSION) {
                        return Err(ManifestError::Version);
                    }
                    set(&mut version, v)?;
                }
                1 => {
                    let id: CircleId = d.bytes().map_err(|_| m)?.try_into().map_err(|_| m)?;
                    set(&mut circle_id, id)?;
                }
                2 => {
                    let p = CircleParams::decode(d.bytes().map_err(|_| m)?).map_err(|_| m)?;
                    set(&mut params, p)?;
                }
                3 => {
                    let mut list = Vec::new();
                    for _ in 0..array_len(&mut d)? {
                        list.push(decode_seat(&mut d)?);
                    }
                    set(&mut seats, list)?;
                }
                4 => {
                    let mut list = Vec::new();
                    for _ in 0..array_len(&mut d)? {
                        list.push(decode_floor_entry(&mut d)?);
                    }
                    set(&mut floor, list)?;
                }
                5 => set(&mut genesis, d.u64().map_err(|_| m)?)?,
                _ => return Err(m),
            }
        }
        if d.position() != bytes.len() || version.is_none() {
            return Err(m);
        }
        let manifest = Manifest {
            circle_id: circle_id.ok_or(m)?,
            params: params.ok_or(m)?,
            seats: seats.ok_or(m)?,
            floor: floor.ok_or(m)?,
            genesis: genesis.ok_or(m)?,
        };
        let in_order = manifest.seats.len() == usize::from(manifest.params.n)
            && manifest
                .seats
                .iter()
                .enumerate()
                .all(|(i, s)| usize::from(s.seat) == i);
        let floor_ascending = manifest
            .floor
            .windows(2)
            .all(|w| w[0].purpose < w[1].purpose);
        if !in_order || !floor_ascending {
            return Err(m);
        }
        if manifest.encode() != bytes {
            return Err(ManifestError::NotCanonical);
        }
        Ok(manifest)
    }

    /// SHA-256 of the canonical bytes.
    pub fn id(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    /// The public key of `seat`, if it is in the roster.
    pub fn seat_key(&self, seat: u8) -> Option<&SeatPublic> {
        self.seats.get(usize::from(seat)).map(|s| &s.key)
    }
}

impl SealedManifest {
    /// The CBOR array `[body, [attestation, ...]]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        // Writing to a Vec cannot fail.
        let _ = self.write(&mut e);
        e.into_writer()
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.array(2)?.bytes(&self.body)?;
        e.array(self.attestations.len() as u64)?;
        for a in &self.attestations {
            e.bytes(a)?;
        }
        Ok(())
    }

    /// Parse the canonical array. The body is not checked here; `verify` does that.
    pub fn decode(bytes: &[u8]) -> Result<SealedManifest, ManifestError> {
        let m = ManifestError::Malformed;
        let mut d = Decoder::new(bytes);
        if array_len(&mut d)? != 2 {
            return Err(m);
        }
        let body = d.bytes().map_err(|_| m)?.to_vec();
        let mut attestations = Vec::new();
        for _ in 0..array_len(&mut d)? {
            attestations.push(d.bytes().map_err(|_| m)?.to_vec());
        }
        if d.position() != bytes.len() {
            return Err(m);
        }
        let sealed = SealedManifest { body, attestations };
        if sealed.encode() != bytes {
            return Err(ManifestError::NotCanonical);
        }
        Ok(sealed)
    }

    /// Decode the body and check that every seat signed it exactly once.
    pub fn verify(&self) -> Result<Manifest, ManifestError> {
        let manifest = Manifest::decode(&self.body)?;
        let n = manifest.params.n;
        if self.attestations.len() != usize::from(n) {
            return Err(ManifestError::AttestationCount);
        }
        let mut held = [false; 256];
        for a in &self.attestations {
            let kid = cose::kid1(a).map_err(|_| ManifestError::Malformed)?;
            let [seat] = kid[..] else {
                return Err(ManifestError::Malformed);
            };
            let key = manifest
                .seat_key(seat)
                .ok_or(ManifestError::BadAttestation(seat))?;
            if held[usize::from(seat)] {
                return Err(ManifestError::DuplicateAttestation(seat));
            }
            cose::verify1(key, a, Some(&self.body), MANIFEST_AAD)
                .map_err(|_| ManifestError::BadAttestation(seat))?;
            held[usize::from(seat)] = true;
        }
        Ok(manifest)
    }
}

fn set<T>(slot: &mut Option<T>, v: T) -> Result<(), ManifestError> {
    if slot.is_some() {
        // A key that appears twice.
        return Err(ManifestError::Malformed);
    }
    *slot = Some(v);
    Ok(())
}

/// A definite map length; an indefinite one parses but is not canonical.
fn map_len(d: &mut Decoder<'_>) -> Result<u64, ManifestError> {
    match d.map() {
        Ok(Some(n)) => Ok(n),
        Ok(None) => Err(ManifestError::NotCanonical),
        Err(_) => Err(ManifestError::Malformed),
    }
}

/// A definite array length; an indefinite one parses but is not canonical.
fn array_len(d: &mut Decoder<'_>) -> Result<u64, ManifestError> {
    match d.array() {
        Ok(Some(n)) => Ok(n),
        Ok(None) => Err(ManifestError::NotCanonical),
        Err(_) => Err(ManifestError::Malformed),
    }
}

fn decode_seat(d: &mut Decoder<'_>) -> Result<Seat, ManifestError> {
    let m = ManifestError::Malformed;
    let (mut seat, mut key, mut tier, mut evidence) = (None, None, None, None);
    for _ in 0..map_len(d)? {
        match d.u64().map_err(|_| m)? {
            0 => set(&mut seat, d.u8().map_err(|_| m)?)?,
            1 => set(
                &mut key,
                SeatPublic::parse(d.bytes().map_err(|_| m)?).map_err(|_| m)?,
            )?,
            2 => set(&mut tier, d.u8().map_err(|_| m)?)?,
            3 => set(&mut evidence, d.bytes().map_err(|_| m)?.to_vec())?,
            _ => return Err(m),
        }
    }
    let tier = tier.ok_or(m)?;
    if tier > MAX_TIER {
        return Err(m);
    }
    Ok(Seat {
        seat: seat.ok_or(m)?,
        key: key.ok_or(m)?,
        tier,
        evidence: evidence.ok_or(m)?,
    })
}

fn decode_floor_entry(d: &mut Decoder<'_>) -> Result<FloorEntry, ManifestError> {
    let m = ManifestError::Malformed;
    let (mut purpose, mut requirement, mut budget) = (None, None, None);
    for _ in 0..map_len(d)? {
        match d.u64().map_err(|_| m)? {
            0 => set(&mut purpose, d.u8().map_err(|_| m)?)?,
            1 => set(
                &mut requirement,
                Requirement::try_from(d.u8().map_err(|_| m)?)?,
            )?,
            2 => set(&mut budget, d.u32().map_err(|_| m)?)?,
            _ => return Err(m),
        }
    }
    Ok(FloorEntry {
        purpose: purpose.ok_or(m)?,
        requirement: requirement.ok_or(m)?,
        budget: budget.ok_or(m)?,
    })
}
