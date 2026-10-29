use cahoots_chip::ops::{ChipOps, Curve};
use cahoots_chip::testca::ROOT_PEM;
use cahoots_chip::{KeyId, KeyKind, SoftChip};
use cahoots_objects::attestation::{verify, AttestError, NXP_ROOTS};

const FRESH: [u8; 16] = [7; 16];

fn attested() -> cahoots_chip::Attested {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 1);
    chip.generate(id, Curve::P256).unwrap();
    chip.attest(id, &FRESH).unwrap()
}

#[test]
fn soft_chip_attestation_verifies_with_test_root() {
    let att = attested();
    let v = verify(&att, &FRESH, &[ROOT_PEM]).unwrap();
    assert_eq!(v.public, att.public);
    assert_eq!(v.uid, att.uid);
    assert_eq!(v.timestamp, att.timestamp);
    // The placeholder file shipped as NXP_ROOTS is the test CA.
    assert!(verify(&att, &FRESH, NXP_ROOTS).is_ok());
}

#[test]
fn wrong_freshness_fails() {
    assert_eq!(verify(&attested(), &[8; 16], &[ROOT_PEM]), Err(AttestError::Freshness));
}

#[test]
fn tampered_public_fails() {
    let mut att = attested();
    let n = att.public.len();
    att.public[n - 1] ^= 1;
    assert_eq!(verify(&att, &FRESH, &[ROOT_PEM]), Err(AttestError::Signature));
    let mut att = attested();
    att.uid[0] ^= 1;
    assert_eq!(verify(&att, &FRESH, &[ROOT_PEM]), Err(AttestError::Signature));
}

#[test]
fn unknown_root_fails() {
    assert_eq!(verify(&attested(), &FRESH, &[]), Err(AttestError::Chain));
}

#[test]
fn other_root_fails() {
    // A certificate with a different subject and key, not related to the attestation.
    let other = other_root();
    assert_eq!(verify(&attested(), &FRESH, &[&other]), Err(AttestError::Chain));
}

#[test]
fn root_with_the_right_name_and_another_key_fails() {
    let mut d = cahoots_chip::testca::ROOT_DER.to_vec();
    // Flip a bit of the public key's y coordinate: same names, no longer the issuing key.
    let n = d.len();
    let pos = d.windows(4).position(|w| w == [0x03, 0x42, 0x00, 0x04]).unwrap() + 40;
    assert!(pos < n);
    d[pos] ^= 1;
    let b64 = base64(&d).as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("\n");
    let pem = format!("-----BEGIN CERTIFICATE-----\n{b64}\n-----END CERTIFICATE-----\n");
    assert_eq!(verify(&attested(), &FRESH, &[&pem]), Err(AttestError::Chain));
}

#[test]
fn mismatched_attest_public_fails() {
    let mut att = attested();
    let mut chip2 = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 2);
    chip2.generate(id, Curve::P256).unwrap();
    att.attest_public = {
        // any other valid uncompressed point: the seat key decompressed is not needed, reuse a mutated copy
        let mut p = att.attest_public.clone();
        p[2] ^= 1;
        p
    };
    assert!(verify(&att, &FRESH, &[ROOT_PEM]).is_err());
}

#[test]
fn garbage_does_not_panic() {
    let mut att = attested();
    att.attest_cert_der = vec![1, 2, 3];
    assert_eq!(verify(&att, &FRESH, &[ROOT_PEM]), Err(AttestError::Parse));
    let att = attested();
    assert_eq!(verify(&att, &FRESH, &["not a pem"]), Err(AttestError::Parse));
    let mut att = attested();
    att.signature = vec![0; 3];
    assert!(verify(&att, &FRESH, &[ROOT_PEM]).is_err());
    att.attest_public = vec![];
    assert!(verify(&att, &FRESH, &[ROOT_PEM]).is_err());
}

#[test]
fn json_has_hex_fields() {
    let att = attested();
    let j = cahoots_objects::attestation::to_json(&att);
    assert_eq!(j["uid"], "736f6674636869702d3030303030303030");
    assert_eq!(j["freshness"].as_str().unwrap().len(), 32);
}

/// A second self-signed certificate, built by the test with a different name and key.
fn other_root() -> String {
    // The CA certificate with its subject name changed (the second "Test CA"): it parses, but
    // its name is not the issuer the certificate names.
    let der = cahoots_chip::testca::ROOT_DER;
    let mut d = der.to_vec();
    let pos = d.windows(7).rposition(|w| w == b"Test CA").unwrap();
    d[pos] = b'X';
    let b64 = base64(&d).as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("\n");
    format!("-----BEGIN CERTIFICATE-----\n{b64}\n-----END CERTIFICATE-----\n")
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let mut t = [0u8; 3];
        t[..c.len()].copy_from_slice(c);
        let n = (t[0] as u32) << 16 | (t[1] as u32) << 8 | t[2] as u32;
        for i in 0..4 {
            if i <= c.len() { s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char) } else { s.push('=') }
        }
    }
    s
}

// A leaf certificate (key 0x43..43) issued by the test CA, built once with another tool.
const LEAF_DER: &[u8] = include_bytes!("fixtures/leaf.der");
const LEAF_KEY: [u8; 32] = [0x43; 32];

fn leaf_attested(cert: &[u8]) -> cahoots_chip::Attested {
    use p256::ecdsa::signature::Signer;
    let mut att = attested();
    let key = p256::ecdsa::SigningKey::from_slice(&LEAF_KEY).unwrap();
    att.attest_public = key.verifying_key().to_encoded_point(false).as_bytes().to_vec();
    att.attest_cert_der = cert.to_vec();
    let mut body = att.public.clone();
    body.extend_from_slice(&att.uid);
    body.extend_from_slice(&att.freshness);
    body.extend_from_slice(&att.timestamp);
    let sig: p256::ecdsa::Signature = key.sign(&body);
    att.signature = sig.to_bytes().to_vec();
    att
}

#[test]
fn certificate_issued_by_a_root_verifies() {
    let att = leaf_attested(LEAF_DER);
    assert!(verify(&att, &FRESH, &[ROOT_PEM]).is_ok());
}

#[test]
fn leaf_with_tampered_issuer_signature_fails_chain() {
    let mut der = LEAF_DER.to_vec();
    let n = der.len();
    der[n - 1] ^= 1;
    assert_eq!(verify(&leaf_attested(&der), &FRESH, &[ROOT_PEM]), Err(AttestError::Chain));
}

#[test]
fn der_encoded_signature_verifies() {
    let mut att = attested();
    let sig = p256::ecdsa::Signature::from_slice(&att.signature).unwrap();
    att.signature = sig.to_der().as_bytes().to_vec();
    assert!(verify(&att, &FRESH, &[ROOT_PEM]).is_ok());
}
