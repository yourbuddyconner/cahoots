use cahoots_core::control::Purpose;
use cahoots_core::cose;
use cahoots_core::manifest::{
    FloorEntry, Manifest, ManifestError, Requirement, SealedManifest, Seat, MANIFEST_AAD,
};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use sha2::{Digest, Sha256};

fn params() -> CircleParams {
    CircleParams::new(3, 512, "ring", false).unwrap()
}

fn key(i: u8) -> SeatKey {
    SeatKey::from_seed(&[i + 1; 32]).unwrap()
}

fn manifest() -> Manifest {
    let p = params();
    Manifest {
        circle_id: [7u8; 16],
        seats: (0..p.n)
            .map(|i| Seat {
                seat: i,
                key: key(i).public(),
                tier: i % 5,
                evidence: Vec::new(),
            })
            .collect(),
        floor: Manifest::default_floor(&p),
        params: p,
        genesis: 0,
    }
}

fn attest(i: u8, body: &[u8]) -> Vec<u8> {
    cose::sign1(&key(i), &[i], body, MANIFEST_AAD, true)
}

fn sealed() -> SealedManifest {
    let body = manifest().encode();
    let attestations = (0..3).map(|i| attest(i, &body)).collect();
    SealedManifest { body, attestations }
}

#[test]
fn encode_is_canonical_and_decodes() {
    let m = manifest();
    let bytes = m.encode();
    // A map of six entries, keys 0 to 5 in order; version 1 first.
    assert_eq!(&bytes[..3], &[0xA6, 0x00, 0x01]);
    assert_eq!(bytes[3], 0x01, "key 1, circle_id");
    assert_eq!(Manifest::decode(&bytes).unwrap(), m);
    assert_eq!(Manifest::decode(&bytes).unwrap().encode(), bytes);
    assert_eq!(MANIFEST_AAD, b"cahoots/manifest/v1");
}

#[test]
fn a_map_key_out_of_order_is_not_canonical() {
    // Swap the first two entries of the outer map: {1: circle_id, 0: 1, ...}.
    let bytes = manifest().encode();
    let version = &bytes[1..3]; // 0x00 0x01
    let circle = &bytes[3..3 + 1 + 1 + 16]; // 0x01, 0x50, 16 bytes
    let mut swapped = vec![bytes[0]];
    swapped.extend_from_slice(circle);
    swapped.extend_from_slice(version);
    swapped.extend_from_slice(&bytes[3 + 18..]);
    assert_eq!(swapped.len(), bytes.len());
    assert_eq!(
        Manifest::decode(&swapped).unwrap_err(),
        ManifestError::NotCanonical
    );
}

#[test]
fn a_long_form_integer_is_not_canonical() {
    // genesis is the last entry: key 5, value 0. Write the 0 as a one-byte-argument uint.
    let mut bytes = manifest().encode();
    assert_eq!(&bytes[bytes.len() - 2..], &[0x05, 0x00]);
    bytes.pop();
    bytes.extend_from_slice(&[0x18, 0x00]);
    assert_eq!(
        Manifest::decode(&bytes).unwrap_err(),
        ManifestError::NotCanonical
    );
}

#[test]
fn malformed_and_version_are_refused() {
    assert_eq!(
        Manifest::decode(&[0x01, 0x02]).unwrap_err(),
        ManifestError::Malformed
    );
    let mut bytes = manifest().encode();
    bytes.push(0x00);
    assert_eq!(
        Manifest::decode(&bytes).unwrap_err(),
        ManifestError::Malformed,
        "trailing bytes"
    );
    let mut bytes = manifest().encode();
    bytes[2] = 0x02;
    assert_eq!(
        Manifest::decode(&bytes).unwrap_err(),
        ManifestError::Version
    );
}

#[test]
fn seats_must_be_every_index_in_order() {
    let mut m = manifest();
    m.seats.swap(0, 1);
    assert_eq!(
        Manifest::decode(&m.encode()).unwrap_err(),
        ManifestError::Malformed
    );
    let mut m = manifest();
    m.seats.pop();
    assert_eq!(
        Manifest::decode(&m.encode()).unwrap_err(),
        ManifestError::Malformed
    );
}

#[test]
fn id_is_sha256_of_the_bytes() {
    let m = manifest();
    let expected: [u8; 32] = Sha256::digest(m.encode()).into();
    assert_eq!(m.id(), expected);
}

#[test]
fn seat_key_looks_up_the_roster() {
    let m = manifest();
    assert_eq!(m.seat_key(1), Some(&key(1).public()));
    assert_eq!(m.seat_key(3), None);
}

#[test]
fn default_floor_lists_every_purpose_at_present_with_slice_len_budget() {
    let p = params();
    let floor = Manifest::default_floor(&p);
    let purposes = [
        Purpose::WireGuard,
        Purpose::Tls,
        Purpose::Mls,
        Purpose::Wifi,
        Purpose::Meshtastic,
        Purpose::Other,
    ];
    assert_eq!(floor.len(), purposes.len());
    for (entry, purpose) in floor.iter().zip(purposes) {
        assert_eq!(
            *entry,
            FloorEntry {
                purpose: purpose as u8,
                requirement: Requirement::Present,
                budget: p.slice_len(),
            }
        );
    }
    assert_eq!(Requirement::Never as u8, 0);
    assert_eq!(Requirement::Grant as u8, 1);
    assert_eq!(Requirement::Present as u8, 2);
}

#[test]
fn sealed_manifest_verifies_with_n_attestations_and_refuses_n_minus_one() {
    let s = sealed();
    assert_eq!(s.verify().unwrap(), manifest());
    let bytes = s.encode();
    assert_eq!(SealedManifest::decode(&bytes).unwrap(), s);
    let mut short = s.clone();
    short.attestations.pop();
    assert_eq!(short.verify().unwrap_err(), ManifestError::AttestationCount);
}

#[test]
fn an_attestation_over_other_bytes_is_refused() {
    let mut s = sealed();
    let mut other = manifest();
    other.genesis = 1;
    s.attestations[2] = attest(2, &other.encode());
    assert_eq!(s.verify().unwrap_err(), ManifestError::BadAttestation(2));
}

#[test]
fn an_attestation_from_a_seat_not_in_the_roster_is_refused() {
    let mut s = sealed();
    s.attestations[2] = attest(5, &s.body);
    assert_eq!(s.verify().unwrap_err(), ManifestError::BadAttestation(5));
}

#[test]
fn two_attestations_from_one_seat_are_refused() {
    let mut s = sealed();
    s.attestations[2] = attest(1, &s.body);
    assert_eq!(
        s.verify().unwrap_err(),
        ManifestError::DuplicateAttestation(1)
    );
}

#[test]
fn a_sealed_manifest_with_trailing_bytes_is_malformed() {
    let mut bytes = sealed().encode();
    bytes.push(0);
    assert_eq!(
        SealedManifest::decode(&bytes).unwrap_err(),
        ManifestError::Malformed
    );
}
