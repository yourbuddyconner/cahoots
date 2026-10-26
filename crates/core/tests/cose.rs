//! COSE_Sign1 (RFC 9052) with ES256: our writer and verifier, and a cross-check against coset.

use cahoots_core::cose::{kid1, sign1, verify1, CoseError};
use cahoots_core::seat::SeatKey;
use minicbor::data::Type;
use minicbor::{Decoder, Encoder};

const SEED: [u8; 32] = [7u8; 32];
const KID: &[u8] = &[3u8];
const PAYLOAD: &[u8] = b"the manifest, say";
const AAD: &[u8] = b"cahoots/test/v1";

fn key() -> SeatKey {
    SeatKey::from_seed(&SEED).unwrap()
}

/// Encode a COSE_Sign1 by hand with an arbitrary protected header, signed with `key`.
fn hand_built(key: &SeatKey, protected: &[u8], payload: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut tbs = Encoder::new(Vec::new());
    tbs.array(4)
        .unwrap()
        .str("Signature1")
        .unwrap()
        .bytes(protected)
        .unwrap()
        .bytes(aad)
        .unwrap()
        .bytes(payload)
        .unwrap();
    let sig = key.sign(&tbs.into_writer());
    let mut out = Encoder::new(Vec::new());
    out.array(4)
        .unwrap()
        .bytes(protected)
        .unwrap()
        .map(0)
        .unwrap()
        .bytes(payload)
        .unwrap()
        .bytes(&sig)
        .unwrap();
    out.into_writer()
}

#[test]
fn sign1_round_trips_embedded_payload() {
    let k = key();
    let bytes = sign1(&k, KID, PAYLOAD, AAD, false);
    assert_eq!(verify1(&k.public(), &bytes, None, AAD).unwrap(), PAYLOAD);
    assert_eq!(kid1(&bytes).unwrap(), KID);
    // A detached payload supplied alongside an embedded one is not accepted.
    assert_eq!(
        verify1(&k.public(), &bytes, Some(PAYLOAD), AAD),
        Err(CoseError::Malformed)
    );
}

#[test]
fn sign1_round_trips_detached_payload() {
    let k = key();
    let bytes = sign1(&k, KID, PAYLOAD, AAD, true);
    let mut d = Decoder::new(&bytes);
    assert_eq!(d.array().unwrap(), Some(4));
    d.skip().unwrap();
    d.skip().unwrap();
    assert_eq!(d.datatype().unwrap(), Type::Null);
    assert_eq!(
        verify1(&k.public(), &bytes, Some(PAYLOAD), AAD).unwrap(),
        PAYLOAD
    );
    assert_eq!(
        verify1(&k.public(), &bytes, None, AAD),
        Err(CoseError::Malformed)
    );
    assert_eq!(
        verify1(&k.public(), &bytes, Some(b"another payload"), AAD),
        Err(CoseError::Signature)
    );
    assert_eq!(kid1(&bytes).unwrap(), KID);
}

#[test]
fn wrong_external_aad_fails() {
    let k = key();
    let bytes = sign1(&k, KID, PAYLOAD, AAD, false);
    assert_eq!(
        verify1(&k.public(), &bytes, None, b"cahoots/other/v1"),
        Err(CoseError::Signature)
    );
    let other = SeatKey::from_seed(&[9u8; 32]).unwrap();
    assert_eq!(
        verify1(&other.public(), &bytes, None, AAD),
        Err(CoseError::Signature)
    );
}

#[test]
fn protected_header_is_alg_es256_and_kid() {
    let k = key();
    let bytes = sign1(&k, KID, PAYLOAD, AAD, false);
    let mut d = Decoder::new(&bytes);
    assert_eq!(d.array().unwrap(), Some(4));
    let protected = d.bytes().unwrap();
    // Canonical: {1: -7, 4: h'03'} is a2 01 26 04 41 03.
    assert_eq!(protected, &[0xa2, 0x01, 0x26, 0x04, 0x41, 0x03]);
    let mut h = Decoder::new(protected);
    assert_eq!(h.map().unwrap(), Some(2));
    assert_eq!(h.i64().unwrap(), 1);
    assert_eq!(h.i64().unwrap(), -7);
    assert_eq!(h.i64().unwrap(), 4);
    assert_eq!(h.bytes().unwrap(), KID);
    assert_eq!(h.position(), protected.len());
    // Unprotected header is the empty map.
    assert_eq!(d.map().unwrap(), Some(0));
}

#[test]
fn extra_protected_header_entry_is_malformed() {
    let k = key();
    // {1: -7, 3: 0, 4: h'03'}: a content type slipped into the protected header.
    let protected = [0xa3, 0x01, 0x26, 0x03, 0x00, 0x04, 0x41, 0x03];
    let bytes = hand_built(&k, &protected, PAYLOAD, AAD);
    assert_eq!(
        verify1(&k.public(), &bytes, None, AAD),
        Err(CoseError::Malformed)
    );
    // Only the alg: no kid.
    let bytes = hand_built(&k, &[0xa1, 0x01, 0x26], PAYLOAD, AAD);
    assert_eq!(
        verify1(&k.public(), &bytes, None, AAD),
        Err(CoseError::Malformed)
    );
    // Keys out of order.
    let bytes = hand_built(&k, &[0xa2, 0x04, 0x41, 0x03, 0x01, 0x26], PAYLOAD, AAD);
    assert_eq!(
        verify1(&k.public(), &bytes, None, AAD),
        Err(CoseError::Malformed)
    );
}

#[test]
fn a_different_alg_is_rejected() {
    let k = key();
    // {1: -35, 4: h'03'}: ES384.
    let bytes = hand_built(
        &k,
        &[0xa2, 0x01, 0x38, 0x22, 0x04, 0x41, 0x03],
        PAYLOAD,
        AAD,
    );
    assert_eq!(
        verify1(&k.public(), &bytes, None, AAD),
        Err(CoseError::Algorithm)
    );
}

#[test]
fn a_hand_built_message_with_the_right_header_verifies() {
    let k = key();
    let bytes = hand_built(&k, &[0xa2, 0x01, 0x26, 0x04, 0x41, 0x03], PAYLOAD, AAD);
    assert_eq!(verify1(&k.public(), &bytes, None, AAD).unwrap(), PAYLOAD);
    assert_eq!(bytes, sign1(&k, KID, PAYLOAD, AAD, false));
}

#[test]
fn garbage_is_malformed() {
    let k = key();
    for bad in [&[][..], &[0x80], &[0x84, 0x40, 0xa0, 0x40], &[0xff; 8]] {
        assert_eq!(
            verify1(&k.public(), bad, None, AAD),
            Err(CoseError::Malformed)
        );
        assert_eq!(kid1(bad), Err(CoseError::Malformed));
    }
    // Trailing bytes after the array.
    let mut bytes = sign1(&k, KID, PAYLOAD, AAD, false);
    bytes.push(0x00);
    assert_eq!(
        verify1(&k.public(), &bytes, None, AAD),
        Err(CoseError::Malformed)
    );
    // A signature of the wrong length.
    let good = sign1(&k, KID, PAYLOAD, AAD, false);
    let mut d = Decoder::new(&good);
    d.array().unwrap();
    let protected = d.bytes().unwrap().to_vec();
    let mut e = Encoder::new(Vec::new());
    e.array(4)
        .unwrap()
        .bytes(&protected)
        .unwrap()
        .map(0)
        .unwrap()
        .bytes(PAYLOAD)
        .unwrap()
        .bytes(&[0u8; 63])
        .unwrap();
    assert_eq!(
        verify1(&k.public(), &e.into_writer(), None, AAD),
        Err(CoseError::Malformed)
    );
}

#[cfg(feature = "std")]
mod coset_cross_check {
    use super::*;
    use coset::{iana, CborSerializable, CoseSign1, CoseSign1Builder, HeaderBuilder};
    use p256::ecdsa::signature::{Signer, Verifier};
    use p256::ecdsa::{Signature, SigningKey, VerifyingKey};

    fn their_key() -> SigningKey {
        SigningKey::from_slice(&SEED).unwrap()
    }

    fn their_verify(sig: &[u8], data: &[u8]) -> Result<(), ()> {
        let vk = VerifyingKey::from_sec1_bytes(&key().public().0).map_err(|_| ())?;
        let sig = Signature::from_slice(sig).map_err(|_| ())?;
        vk.verify(data, &sig).map_err(|_| ())
    }

    fn their_sign(data: &[u8]) -> Vec<u8> {
        let sig: Signature = their_key().sign(data);
        sig.to_bytes().to_vec()
    }

    fn header() -> coset::Header {
        HeaderBuilder::new()
            .algorithm(iana::Algorithm::ES256)
            .key_id(KID.to_vec())
            .build()
    }

    #[test]
    fn coset_verifies_ours() {
        let k = key();
        let ours = sign1(&k, KID, PAYLOAD, AAD, false);
        let theirs = CoseSign1::from_slice(&ours).unwrap();
        assert_eq!(
            theirs.protected.header.alg,
            Some(coset::RegisteredLabelWithPrivate::Assigned(
                iana::Algorithm::ES256
            ))
        );
        assert_eq!(theirs.protected.header.key_id, KID);
        assert_eq!(theirs.payload.as_deref(), Some(PAYLOAD));
        theirs.verify_signature(AAD, their_verify).unwrap();
        assert!(theirs.verify_signature(b"wrong", their_verify).is_err());

        let detached = sign1(&k, KID, PAYLOAD, AAD, true);
        let theirs = CoseSign1::from_slice(&detached).unwrap();
        assert_eq!(theirs.payload, None);
        theirs
            .verify_detached_signature(PAYLOAD, AAD, their_verify)
            .unwrap();
    }

    #[test]
    fn we_verify_cosets() {
        let k = key();
        let theirs = CoseSign1Builder::new()
            .protected(header())
            .payload(PAYLOAD.to_vec())
            .create_signature(AAD, their_sign)
            .build()
            .to_vec()
            .unwrap();
        assert_eq!(verify1(&k.public(), &theirs, None, AAD).unwrap(), PAYLOAD);
        assert_eq!(kid1(&theirs).unwrap(), KID);
        // Same header bytes, same deterministic signature: identical encodings.
        assert_eq!(theirs, sign1(&k, KID, PAYLOAD, AAD, false));

        let detached = CoseSign1Builder::new()
            .protected(header())
            .create_detached_signature(PAYLOAD, AAD, their_sign)
            .build()
            .to_vec()
            .unwrap();
        assert_eq!(
            verify1(&k.public(), &detached, Some(PAYLOAD), AAD).unwrap(),
            PAYLOAD
        );
    }
}
