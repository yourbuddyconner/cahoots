//! COSE_Sign1 (RFC 9052) with ES256 and seat keys (spec §3).
//!
//! The one shape Cahoots emits and accepts: the untagged array
//! `[protected: bstr, unprotected: {}, payload: bstr / nil, signature: bstr .size 64]`, where
//! `protected` is the canonical CBOR map `{1: -7, 4: kid}` (alg ES256, key id). The signature
//! covers `Sig_structure = ["Signature1", protected, external_aad, payload]`.

use alloc::vec::Vec;
use minicbor::data::Type;
use minicbor::{Decoder, Encoder};

use crate::seat::{SeatPublic, SeatSigner};

/// COSE header label `alg`.
const LABEL_ALG: i64 = 1;
/// COSE header label `kid`.
const LABEL_KID: i64 = 4;
/// COSE algorithm ES256 (ECDSA P-256 with SHA-256).
const ALG_ES256: i64 = -7;
const CONTEXT_SIGNATURE1: &str = "Signature1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoseError {
    /// Not the COSE_Sign1 shape above, a protected header with anything but `alg` and `kid`,
    /// or a payload supplied both embedded and detached (or neither).
    Malformed,
    /// The protected header names an algorithm other than ES256.
    Algorithm,
    /// The signature does not verify.
    Signature,
}

fn protected_header(kid: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = e
        .map(2)
        .and_then(|e| e.i64(LABEL_ALG))
        .and_then(|e| e.i64(ALG_ES256))
        .and_then(|e| e.i64(LABEL_KID))
        .and_then(|e| e.bytes(kid));
    e.into_writer()
}

fn sig_structure(protected: &[u8], external_aad: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new(Vec::new());
    let _ = e
        .array(4)
        .and_then(|e| e.str(CONTEXT_SIGNATURE1))
        .and_then(|e| e.bytes(protected))
        .and_then(|e| e.bytes(external_aad))
        .and_then(|e| e.bytes(payload));
    e.into_writer()
}

/// Sign `payload` as a COSE_Sign1 with key id `kid`. With `detached`, the payload slot is nil and
/// the verifier supplies the payload. `key` is any seat signer: `&seat_key`, `&mut chip_seat`, or
/// a borrowed `&mut dyn SeatSigner`.
pub fn sign1<S: SeatSigner>(
    mut key: S,
    kid: &[u8],
    payload: &[u8],
    external_aad: &[u8],
    detached: bool,
) -> Vec<u8> {
    let protected = protected_header(kid);
    let signature = key.sign(&sig_structure(&protected, external_aad, payload));
    let mut e = Encoder::new(Vec::new());
    let _ = e
        .array(4)
        .and_then(|e| e.bytes(&protected))
        .and_then(|e| e.map(0));
    let _ = if detached { e.null() } else { e.bytes(payload) };
    let _ = e.bytes(&signature);
    e.into_writer()
}

struct Parsed<'a> {
    protected: &'a [u8],
    kid: &'a [u8],
    payload: Option<&'a [u8]>,
    signature: [u8; 64],
}

/// Read the protected header: exactly `{1: -7, 4: kid}` in canonical form. Returns the kid.
fn parse_protected(protected: &[u8]) -> Result<&[u8], CoseError> {
    let m = CoseError::Malformed;
    let mut d = Decoder::new(protected);
    if d.map().map_err(|_| m)? != Some(2) {
        return Err(m);
    }
    if d.i64().map_err(|_| m)? != LABEL_ALG {
        return Err(m);
    }
    match d.datatype().map_err(|_| m)? {
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::Int => {
            if d.i64().map_err(|_| CoseError::Algorithm)? != ALG_ES256 {
                return Err(CoseError::Algorithm);
            }
        }
        Type::String => return Err(CoseError::Algorithm),
        _ => return Err(m),
    }
    if d.i64().map_err(|_| m)? != LABEL_KID {
        return Err(m);
    }
    let kid = d.bytes().map_err(|_| m)?;
    if d.position() != protected.len() {
        return Err(m);
    }
    // Canonical form only: re-encoding must give the same bytes.
    if protected_header(kid) != protected {
        return Err(m);
    }
    Ok(kid)
}

fn parse(bytes: &[u8]) -> Result<Parsed<'_>, CoseError> {
    let m = CoseError::Malformed;
    let mut d = Decoder::new(bytes);
    if d.array().map_err(|_| m)? != Some(4) {
        return Err(m);
    }
    let protected = d.bytes().map_err(|_| m)?;
    if d.map().map_err(|_| m)? != Some(0) {
        return Err(m);
    }
    let payload = match d.datatype().map_err(|_| m)? {
        Type::Null => {
            d.null().map_err(|_| m)?;
            None
        }
        Type::Bytes => Some(d.bytes().map_err(|_| m)?),
        _ => return Err(m),
    };
    let signature: [u8; 64] = d.bytes().map_err(|_| m)?.try_into().map_err(|_| m)?;
    if d.position() != bytes.len() {
        return Err(m);
    }
    let kid = parse_protected(protected)?;
    Ok(Parsed {
        protected,
        kid,
        payload,
        signature,
    })
}

/// Verify a COSE_Sign1 made by `public` and return its payload. `detached_payload` must be
/// `Some` exactly when the payload slot is nil.
pub fn verify1(
    public: &SeatPublic,
    bytes: &[u8],
    detached_payload: Option<&[u8]>,
    external_aad: &[u8],
) -> Result<Vec<u8>, CoseError> {
    let p = parse(bytes)?;
    let payload = match (p.payload, detached_payload) {
        (Some(embedded), None) => embedded,
        (None, Some(detached)) => detached,
        _ => return Err(CoseError::Malformed),
    };
    let tbs = sig_structure(p.protected, external_aad, payload);
    if !public.verify(&tbs, &p.signature) {
        return Err(CoseError::Signature);
    }
    Ok(payload.to_vec())
}

/// The kid of a COSE_Sign1, without verifying the signature.
pub fn kid1(bytes: &[u8]) -> Result<Vec<u8>, CoseError> {
    parse(bytes).map(|p| p.kid.to_vec())
}

/// The embedded payload of a COSE_Sign1, without verifying the signature: for showing what an
/// object says before (or without) checking who signed it.
pub fn payload1(bytes: &[u8]) -> Result<Vec<u8>, CoseError> {
    parse(bytes)?
        .payload
        .map(<[u8]>::to_vec)
        .ok_or(CoseError::Malformed)
}
