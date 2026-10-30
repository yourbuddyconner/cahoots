use cahoots_chip::{Attested, ChipError, ChipOps, Curve, KeyId, KeyKind, SoftChip};
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::signature::Verifier;
use sha2::{Digest, Sha256};

#[test]
fn key_ids_round_trip() {
    let id = KeyId::new(KeyKind::Seat, 3);
    assert_eq!(id.0, 0x4C41_0103);
    assert_eq!(KeyId::parse(id.0), Some((KeyKind::Seat, 3)));
    assert_eq!(KeyId::parse(KeyId::new(KeyKind::Root, 255).0), Some((KeyKind::Root, 255)));
    assert_eq!(KeyId::parse(KeyId::new(KeyKind::Epoch, 0).0), Some((KeyKind::Epoch, 0)));
    assert_eq!(KeyId::parse(0), None);
    assert_eq!(KeyId::parse(0x4C41_0503), None, "kind 5 is not ours");
    assert_eq!(KeyId::parse(0x4C41_0003), None, "kind 0 is not ours");
    assert_eq!(KeyId::parse(0x4C42_0103), None, "outside the range");
}

#[test]
fn generate_sign_verify_p256() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 0);
    let public = chip.generate(id, Curve::P256).unwrap();
    assert_eq!(public.len(), 33);
    assert_eq!(chip.public(id).unwrap(), public);
    let digest: [u8; 32] = Sha256::digest(b"hello").into();
    let sig = chip.sign_p256(id, &digest).unwrap();
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&public).unwrap();
    let sig = p256::ecdsa::Signature::from_slice(&sig).unwrap();
    vk.verify_prehash(&digest, &sig).unwrap();
    assert_eq!(chip.sign_ed25519(id, b"x"), Err(ChipError::NoSuchKey));
}

#[test]
fn generate_sign_verify_ed25519() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 1);
    let public = chip.generate(id, Curve::Ed25519).unwrap();
    assert_eq!(public.len(), 32);
    let sig = chip.sign_ed25519(id, b"message").unwrap();
    let vk = ed25519_dalek::VerifyingKey::from_bytes(public.as_slice().try_into().unwrap()).unwrap();
    vk.verify(b"message", &ed25519_dalek::Signature::from_bytes(&sig)).unwrap();
    assert_eq!(chip.sign_p256(id, &[0; 32]), Err(ChipError::NoSuchKey));
}

#[test]
fn hmac_key_is_usable_not_readable() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Root, 0);
    let key = [7u8; 32];
    chip.put_hmac_key(id, &key).unwrap();
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&key).unwrap();
    m.update(b"chat");
    let want: [u8; 32] = m.finalize().into_bytes().into();
    assert_eq!(chip.hmac(id, b"chat").unwrap(), want);
    assert_eq!(chip.public(id), Err(ChipError::NoSuchKey));
    assert_eq!(chip.hmac(KeyId::new(KeyKind::Root, 1), b"x"), Err(ChipError::NoSuchKey));
}

#[test]
fn counter_only_moves_forward() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Epoch, 0);
    chip.put_counter(id, 5).unwrap();
    assert_eq!(chip.counter_next(id).unwrap(), 6);
    assert_eq!(chip.counter_next(id).unwrap(), 7);
    assert_eq!(chip.counter_next(KeyId::new(KeyKind::Epoch, 1)), Err(ChipError::NoSuchKey));
}

#[test]
fn delete_then_exists_false() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 2);
    assert!(!chip.exists(id).unwrap());
    chip.generate(id, Curve::P256).unwrap();
    assert!(chip.exists(id).unwrap());
    chip.delete(id).unwrap();
    assert!(!chip.exists(id).unwrap());
    assert_eq!(chip.delete(id), Err(ChipError::NoSuchKey));
}

#[test]
fn ids_lists_what_is_there() {
    let mut chip = SoftChip::new();
    let a = KeyId::new(KeyKind::Seat, 0);
    let b = KeyId::new(KeyKind::Seat, 1);
    assert!(chip.ids().unwrap().is_empty());
    chip.generate(a, Curve::P256).unwrap();
    chip.generate(b, Curve::Ed25519).unwrap();
    chip.delete(a).unwrap();
    assert_eq!(chip.ids().unwrap(), vec![b]);
}

#[test]
fn two_keys_differ() {
    let mut chip = SoftChip::new();
    let a = chip.generate(KeyId::new(KeyKind::Seat, 0), Curve::P256).unwrap();
    let b = chip.generate(KeyId::new(KeyKind::Seat, 1), Curve::P256).unwrap();
    assert_ne!(a, b);
    assert_ne!(chip.random(16).unwrap(), chip.random(16).unwrap());
}

#[test]
fn attest_verifies_against_test_ca() {
    use x509_cert::der::Decode;
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 0);
    let public = chip.generate(id, Curve::P256).unwrap();
    let fresh = [9u8; 16];
    let at: Attested = chip.attest(id, &fresh).unwrap();
    assert_eq!(at.public, public);
    assert_eq!(at.freshness, fresh);

    let mut body = at.public.clone();
    body.extend_from_slice(&at.uid);
    body.extend_from_slice(&at.freshness);
    body.extend_from_slice(&at.timestamp);
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&at.attest_public).unwrap();
    let sig = p256::ecdsa::Signature::from_slice(&at.signature).unwrap();
    vk.verify(&body, &sig).unwrap();

    let cert = x509_cert::Certificate::from_der(&at.attest_cert_der).unwrap();
    let spki = cert.tbs_certificate.subject_public_key_info.subject_public_key.raw_bytes();
    assert_eq!(spki, at.attest_public.as_slice());
    assert_eq!(cahoots_chip::testca::ROOT_DER.as_slice(), at.attest_cert_der.as_slice());
    assert!(cahoots_chip::testca::ROOT_PEM.starts_with("-----BEGIN CERTIFICATE-----"));

    let again = chip.attest(id, &fresh).unwrap();
    assert!(again.timestamp > at.timestamp);
    assert_eq!(chip.attest(KeyId::new(KeyKind::Seat, 9), &fresh), Err(ChipError::NoSuchKey));
}

#[test]
fn attested_round_trips_cbor() {
    let mut chip = SoftChip::new();
    let id = KeyId::new(KeyKind::Seat, 0);
    chip.generate(id, Curve::Ed25519).unwrap();
    let at = chip.attest(id, &[1; 16]).unwrap();
    let bytes = minicbor::to_vec(&at).unwrap();
    assert_eq!(minicbor::decode::<Attested>(&bytes).unwrap(), at);
}

#[test]
fn export_then_import_keeps_every_object_and_the_generator() {
    let mut chip = SoftChip::with_seed([3; 32]);
    let seat = KeyId::new(KeyKind::Seat, 0);
    let ed = KeyId::new(KeyKind::Seat, 1);
    let root = KeyId::new(KeyKind::Root, 0);
    let epoch = KeyId::new(KeyKind::Epoch, 0);
    let public = chip.generate(seat, Curve::P256).unwrap();
    let ed_public = chip.generate(ed, Curve::Ed25519).unwrap();
    chip.put_hmac_key(root, &[9; 32]).unwrap();
    chip.put_counter(epoch, 41).unwrap();
    chip.attest(seat, &[1; 16]).unwrap();

    let saved = chip.export();
    let mut back = SoftChip::import(&saved).expect("imports");
    assert_eq!(back.ids().unwrap(), chip.ids().unwrap());
    assert_eq!(back.public(seat).unwrap(), public);
    assert_eq!(back.public(ed).unwrap(), ed_public);
    assert_eq!(back.hmac(root, b"x").unwrap(), chip.hmac(root, b"x").unwrap());
    assert_eq!(back.counter_next(epoch).unwrap(), 42);
    // The generator and the clock carry on where they were, so no key or timestamp repeats.
    assert_eq!(back.random(16).unwrap(), chip.random(16).unwrap());
    assert_eq!(
        back.attest(seat, &[2; 16]).unwrap().timestamp,
        chip.attest(seat, &[2; 16]).unwrap().timestamp
    );
    let digest: [u8; 32] = Sha256::digest(b"hello").into();
    let sig = back.sign_p256(seat, &digest).unwrap();
    let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&public).unwrap();
    vk.verify_prehash(&digest, &p256::ecdsa::Signature::from_slice(&sig).unwrap())
        .unwrap();

    assert!(SoftChip::import(b"").is_none());
    assert!(SoftChip::import(&saved[..saved.len() - 1]).is_none());
    let mut longer = saved.clone();
    longer.push(0);
    assert!(SoftChip::import(&longer).is_none());
}
