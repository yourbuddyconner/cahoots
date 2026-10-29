//! Checking a chip's attestation of a seat key against a root certificate.
//!
//! A laptop sends the badge a fresh 16-byte value. The chip signs the seat key, its own unique id,
//! that value and a timestamp, and returns the signature with its attestation certificate. The
//! verifier checks that the value came back, that the signature is good, and that the
//! certificate is a root it trusts or was issued by one.
//!
//! The chain is one level: the attestation certificate must be a root or be signed by one.
//! Validity dates, basicConstraints and keyUsage are not checked. See `roots/README.md`.

use alloc::vec::Vec;
use cahoots_chip::Attested;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{DerSignature, Signature, VerifyingKey};
use x509_cert::der::{Decode, DecodePem, Encode};
use x509_cert::Certificate;

/// The OID of ecdsa-with-SHA256.
const ECDSA_SHA256: x509_cert::spki::ObjectIdentifier =
    x509_cert::spki::ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");

/// NXP's attestation roots, as PEM.
///
/// Empty until NXP's root is read from the first real SE051 and vendored (see
/// `roots/README.md`). With no roots, nothing verifies against this list.
pub const NXP_ROOTS: &[&str] = &[];

/// The software chip's test CA, as PEM (`roots/software-chip-test-root.pem`, the same
/// certificate as `cahoots_chip::testca::ROOT_PEM`).
///
/// Its private key is in the source, so anyone can make a bundle that passes against it. A
/// pass against this root shows only that the software is consistent, never that a key is in
/// hardware. Use it for tests and simulated badges, never as a root of trust.
pub const TEST_ROOTS: &[&str] = &[include_str!("../roots/software-chip-test-root.pem")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttestError {
    /// The signature over the attested payload does not verify, or the attestation key is not
    /// the certificate's key.
    Signature,
    /// The certificate is not a trusted root and no trusted root issued it.
    Chain,
    /// The freshness value is not the one asked for.
    Freshness,
    /// A certificate, root or key does not parse.
    Parse,
}

/// What a good attestation proves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The seat key, SEC1 compressed (33 bytes), whatever form the chip sent it in.
    pub public: Vec<u8>,
    /// The chip's unique id.
    pub uid: Vec<u8>,
    pub timestamp: Vec<u8>,
}

/// The bytes the attestation key signs.
///
/// This is the software chip's layout: `public || uid || freshness || timestamp`. The real
/// SE051 signs a TLV structure from its attest read; its layout is confirmed with the first real
/// bundle (Task 10, the bench) and replaces this function. Nothing else changes.
fn attested_payload(att: &Attested) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&att.public);
    body.extend_from_slice(&att.uid);
    body.extend_from_slice(&att.freshness);
    body.extend_from_slice(&att.timestamp);
    body
}

/// Check `att` against `roots` (PEM certificates) for the freshness value that was sent.
pub fn verify(att: &Attested, freshness: &[u8; 16], roots: &[&str]) -> Result<Verified, AttestError> {
    if &att.freshness != freshness {
        return Err(AttestError::Freshness);
    }
    let cert = Certificate::from_der(&att.attest_cert_der).map_err(|_| AttestError::Parse)?;
    let cert_key = cert.tbs_certificate.subject_public_key_info.subject_public_key.as_bytes();
    if cert_key != Some(att.attest_public.as_slice()) {
        return Err(AttestError::Signature);
    }
    let key = VerifyingKey::from_sec1_bytes(&att.attest_public).map_err(|_| AttestError::Parse)?;
    // The software chip signs raw r || s; a real chip may return DER.
    let sig = match Signature::from_slice(&att.signature) {
        Ok(s) => s,
        Err(_) => Signature::from_der(&att.signature).map_err(|_| AttestError::Signature)?,
    };
    key.verify(&attested_payload(att), &sig).map_err(|_| AttestError::Signature)?;

    // The software chip sends the seat key compressed; the SE051 sends it as 04 || X || Y.
    let public = compressed(&att.public).ok_or(AttestError::Parse)?;

    for pem in roots {
        let root = Certificate::from_pem(pem.as_bytes()).map_err(|_| AttestError::Parse)?;
        if trusts(&root, &cert, &att.attest_cert_der)? {
            return Ok(Verified { public, uid: att.uid.clone(), timestamp: att.timestamp.clone() });
        }
    }
    Err(AttestError::Chain)
}

/// A P-256 public key in either SEC1 form, as SEC1 compressed. None when it is not a point.
pub fn compressed(sec1: &[u8]) -> Option<Vec<u8>> {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let key = p256::PublicKey::from_sec1_bytes(sec1).ok()?;
    Some(key.to_encoded_point(true).as_bytes().to_vec())
}

/// Whether `root` is `cert` itself or issued it.
fn trusts(root: &Certificate, cert: &Certificate, cert_der: &[u8]) -> Result<bool, AttestError> {
    if root.to_der().map_err(|_| AttestError::Parse)? == cert_der {
        return Ok(true);
    }
    if root.tbs_certificate.subject != cert.tbs_certificate.issuer {
        return Ok(false);
    }
    if cert.signature_algorithm.oid != ECDSA_SHA256 || cert.tbs_certificate.signature.oid != ECDSA_SHA256 {
        return Ok(false);
    }
    let Some(root_key) = root.tbs_certificate.subject_public_key_info.subject_public_key.as_bytes() else {
        return Ok(false);
    };
    let Ok(root_key) = VerifyingKey::from_sec1_bytes(root_key) else { return Ok(false) };
    let Some(sig_bytes) = cert.signature.as_bytes() else { return Ok(false) };
    let Ok(sig) = DerSignature::from_bytes(sig_bytes) else { return Ok(false) };
    let Ok(tbs) = cert.tbs_certificate.to_der() else { return Err(AttestError::Parse) };
    Ok(root_key.verify(&tbs, &sig).is_ok())
}

/// The attestation as JSON for the bundle (informative; nothing verifies it).
#[cfg(feature = "std")]
pub fn to_json(att: &Attested) -> serde_json::Value {
    use crate::json::hex;
    serde_json::json!({
        "public": hex(&att.public),
        "uid": hex(&att.uid),
        "freshness": hex(&att.freshness),
        "timestamp": hex(&att.timestamp),
        "signature": hex(&att.signature),
        "attest_public": hex(&att.attest_public),
        "attest_cert": hex(&att.attest_cert_der),
    })
}
