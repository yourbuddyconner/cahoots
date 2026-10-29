//! The circle root every member's card derives after the reveals, and the seat signer trait
//! the ceremony signs its manifest attestation through.

use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::params::CircleParams;
use cahoots_core::root;
use cahoots_core::seat::{SeatKey, SeatPublic, SeatSigner};
use cahoots_core::CircleId;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn params() -> CircleParams {
    CircleParams::new(3, 512, "ring", false).unwrap()
}

fn contribution(i: u8, salt: u8) -> Vec<u8> {
    (0..512u32)
        .map(|k| (k.wrapping_mul(13 + i as u32) ^ i as u32 ^ salt as u32) as u8)
        .collect()
}

fn kem_seed(i: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    for (k, b) in s.iter_mut().enumerate() {
        *b = (k as u8).wrapping_mul(7).wrapping_add(i.wrapping_mul(31));
    }
    s
}

fn seat_seed(i: u8) -> [u8; 32] {
    [i.wrapping_add(1); 32]
}

fn filler(i: u8) -> impl FnMut(&mut [u8]) {
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ u64::from(i);
    move |buf: &mut [u8]| {
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = (x >> 56) as u8;
        }
    }
}

fn exchange_commitments(devs: &mut [Ceremony]) {
    let commits: Vec<_> = devs
        .iter()
        .map(|d| {
            let i = d.index();
            (
                i,
                d.my_commitment(),
                *d.my_public_key(),
                d.my_seat_public(),
                i % 5,
            )
        })
        .collect();
    for d in devs.iter_mut() {
        for (j, c, pk, sp, t) in &commits {
            d.add_commitment(*j, *c, pk, &sp.0, *t).unwrap();
        }
    }
}

fn exchange_reveals(devs: &mut [Ceremony]) {
    let reveals: Vec<Vec<u8>> = devs
        .iter_mut()
        .map(|d| {
            let i = d.index();
            d.my_reveal(&mut filler(i)).unwrap().to_vec()
        })
        .collect();
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
    }
}

fn exchange_attestations(devs: &mut [Ceremony]) {
    let atts: Vec<_> = devs
        .iter()
        .map(|d| (d.index(), d.my_attestation().unwrap()))
        .collect();
    for d in devs.iter_mut() {
        for (j, a) in &atts {
            d.add_attestation(*j, a).unwrap();
        }
    }
}

/// Three members with seed-held seat keys, through the reveals: every member is in `Attest`.
fn to_attest(salt: u8) -> Vec<Ceremony> {
    let mut devs: Vec<Ceremony> = (0..3)
        .map(|i| {
            Ceremony::new(
                params(),
                i,
                contribution(i, salt),
                kem_seed(i),
                seat_seed(i),
                i % 5,
            )
            .unwrap()
        })
        .collect();
    exchange_commitments(&mut devs);
    exchange_reveals(&mut devs);
    devs
}

fn root_of(d: &Ceremony) -> [u8; 32] {
    let secrets = d.reveal_secrets().expect("every reveal is in");
    root::derive(&secrets, &d.circle_id().expect("in Attest"))
}

#[test]
fn root_is_same_for_every_member() {
    let devs = to_attest(0);
    let roots: Vec<[u8; 32]> = devs.iter().map(root_of).collect();
    assert_eq!(roots[0], roots[1]);
    assert_eq!(roots[1], roots[2]);
    let secrets: Vec<_> = devs.iter().map(|d| d.reveal_secrets().unwrap()).collect();
    assert_eq!(secrets[0].len(), 3, "one content key per seat");
    assert_eq!(*secrets[0], *secrets[1]);
    assert_eq!(*secrets[1], *secrets[2]);
}

#[test]
fn root_is_hkdf_over_the_content_keys_in_seat_order() {
    let devs = to_attest(0);
    let secrets = devs[0].reveal_secrets().unwrap();
    let id = devs[0].circle_id().unwrap();
    let ikm: Vec<u8> = secrets.iter().flatten().copied().collect();
    let mut expected = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&id), &ikm)
        .expand(b"cahoots/root/v1", &mut expected)
        .unwrap();
    assert_eq!(root::derive(&secrets, &id), expected);
}

#[test]
fn the_result_carries_the_root() {
    let mut devs = to_attest(0);
    let expected = root_of(&devs[0]);
    exchange_attestations(&mut devs);
    for d in devs {
        let r = d.finish().unwrap();
        assert_eq!(r.root.as_deref(), Some(&expected));
    }
}

#[test]
fn reveal_secrets_wait_for_every_reveal() {
    let mut devs: Vec<Ceremony> = (0..3)
        .map(|i| {
            Ceremony::new(
                params(),
                i,
                contribution(i, 0),
                kem_seed(i),
                seat_seed(i),
                i % 5,
            )
            .unwrap()
        })
        .collect();
    assert!(devs[0].reveal_secrets().is_none());
    exchange_commitments(&mut devs);
    let r1 = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    devs[0].my_reveal(&mut filler(0)).unwrap();
    devs[0].add_reveal(1, &r1).unwrap();
    assert!(
        devs[0].reveal_secrets().is_none(),
        "member 2's reveal is missing"
    );
}

#[test]
fn root_differs_per_circle() {
    let a = root_of(&to_attest(0)[0]);
    let b = root_of(&to_attest(1)[0]);
    assert_ne!(a, b);
}

#[test]
fn derive_uses_the_circle_id_as_salt() {
    let s = [[1u8; 32], [2u8; 32]];
    assert_ne!(root::derive(&s, &[0u8; 16]), root::derive(&s, &[1u8; 16]));
    let swapped = [[2u8; 32], [1u8; 32]];
    assert_ne!(
        root::derive(&s, &[0u8; 16]),
        root::derive(&swapped, &[0u8; 16])
    );
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

#[test]
fn chat_key_changes_with_epoch() {
    let r = [7u8; 32];
    let id: CircleId = [3u8; 16];
    assert_ne!(root::chat_key(&r, &id, 0), root::chat_key(&r, &id, 1));
    assert_eq!(
        root::chat_key(&r, &id, 0x0102),
        hmac(&r, &[b"chat", &id, &0x0102u64.to_be_bytes()])
    );
    assert_ne!(
        root::chat_key(&r, &id, 0),
        root::chat_key(&r, &[4u8; 16], 0)
    );
}

#[test]
fn seal_key_is_hmac_of_label_and_circle() {
    let r = [7u8; 32];
    let id: CircleId = [3u8; 16];
    assert_eq!(
        root::seal_key(&r, b"seal/card", &id),
        hmac(&r, &[b"seal/card", &id])
    );
    assert_ne!(
        root::seal_key(&r, b"seal/card", &id),
        root::seal_key(&r, b"seal/x", &id)
    );
}

#[test]
fn seat_key_implements_seat_signer() {
    let mut key = SeatKey::from_seed(&seat_seed(0)).unwrap();
    let signer: &mut dyn SeatSigner = &mut key;
    let public: SeatPublic = signer.public();
    let sig = signer.sign(b"hello circle");
    assert!(public.verify(b"hello circle", &sig));
    assert_eq!(public, SeatKey::from_seed(&seat_seed(0)).unwrap().public());
}

/// Member 1's seat key is held outside the ceremony, as a chip holds it: the ceremony gets only
/// the public key and the caller signs the manifest through a borrowed signer.
#[test]
fn a_ceremony_built_from_a_seat_public_signs_through_a_borrowed_signer() {
    let mut outside = SeatKey::from_seed(&seat_seed(1)).unwrap();
    let mut devs: Vec<Ceremony> = (0..3)
        .map(|i| {
            if i == 1 {
                let public = SeatSigner::public(&mut outside);
                Ceremony::with_seat_public(params(), i, contribution(i, 0), kem_seed(i), public, 1)
                    .unwrap()
            } else {
                Ceremony::new(
                    params(),
                    i,
                    contribution(i, 0),
                    kem_seed(i),
                    seat_seed(i),
                    i % 5,
                )
                .unwrap()
            }
        })
        .collect();
    exchange_commitments(&mut devs);
    exchange_reveals(&mut devs);
    assert_eq!(devs[1].phase(), Phase::Attest);
    assert!(
        devs[1].my_attestation().is_none(),
        "no key held: nothing signed yet"
    );
    assert!(devs[1].missing_attestations().contains(&1));

    // The wrong signer changes nothing.
    let mut wrong = SeatKey::from_seed(&[0x55; 32]).unwrap();
    assert_eq!(
        devs[1].sign_manifest(&mut wrong),
        Err(CeremonyError::BadAttestation(1))
    );
    assert!(devs[1].my_attestation().is_none());

    devs[1].sign_manifest(&mut outside).unwrap();
    let mine = devs[1].my_attestation().unwrap();
    // A second call keeps the attestation made first.
    devs[1].sign_manifest(&mut outside).unwrap();
    assert_eq!(devs[1].my_attestation().unwrap(), mine);

    exchange_attestations(&mut devs);
    let roots: Vec<_> = devs.iter().map(root_of).collect();
    for d in devs {
        let i = d.index();
        assert_eq!(d.phase(), Phase::Complete);
        let r = d.finish().unwrap();
        assert_eq!(r.seat_key.is_none(), i == 1);
        assert_eq!(r.root.as_deref(), Some(&roots[0]));
        r.manifest.verify().unwrap();
    }
}

#[test]
fn sign_manifest_before_attest_is_wrong_phase() {
    let mut key = SeatKey::from_seed(&seat_seed(0)).unwrap();
    let public = key.public();
    let mut d = Ceremony::with_seat_public(params(), 0, contribution(0, 0), kem_seed(0), public, 0)
        .unwrap();
    assert_eq!(d.sign_manifest(&mut key), Err(CeremonyError::WrongPhase));
}

#[test]
fn a_seat_public_that_is_not_a_point_is_refused() {
    assert_eq!(
        Ceremony::with_seat_public(
            params(),
            0,
            contribution(0, 0),
            kem_seed(0),
            SeatPublic([0; 33]),
            0
        )
        .err(),
        Some(CeremonyError::BadKey(0))
    );
}

/// Member 1 seals the same contribution a second time under another content key and other
/// encapsulations. Both blobs match its commitment, so without a check the second would replace
/// the content key (and the pair-key secret) member 0 holds, and the root would move.
#[test]
fn a_second_different_reveal_from_one_sender_is_refused_and_the_root_does_not_move() {
    let mut devs: Vec<Ceremony> = (0..3)
        .map(|i| {
            Ceremony::new(
                params(),
                i,
                contribution(i, 0),
                kem_seed(i),
                seat_seed(i),
                i % 5,
            )
            .unwrap()
        })
        .collect();
    let mut twin = Ceremony::new(
        params(),
        1,
        contribution(1, 0),
        kem_seed(1),
        seat_seed(1),
        1,
    )
    .unwrap();
    exchange_commitments(&mut devs);
    let commits: Vec<_> = devs
        .iter()
        .map(|d| {
            let i = d.index();
            (
                i,
                d.my_commitment(),
                *d.my_public_key(),
                d.my_seat_public(),
                i % 5,
            )
        })
        .collect();
    for (j, c, pk, sp, t) in &commits {
        twin.add_commitment(*j, *c, pk, &sp.0, *t).unwrap();
    }
    let first = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    let second = twin.my_reveal(&mut filler(99)).unwrap().to_vec();
    assert_ne!(first, second);

    // In Reveal: the first is kept, the second refused, nothing changes.
    devs[0].my_reveal(&mut filler(0)).unwrap();
    devs[0].add_reveal(1, &first).unwrap();
    assert_eq!(
        devs[0].add_reveal(1, &second),
        Err(CeremonyError::ConflictingReveal(1))
    );
    assert_eq!(devs[0].phase(), Phase::Reveal);

    exchange_reveals(&mut devs);
    assert_eq!(devs[0].phase(), Phase::Attest);
    let root = root_of(&devs[0]);
    assert_eq!(*devs[0].circle_root().unwrap(), root);
    assert_eq!(*devs[1].circle_root().unwrap(), root);

    // After Attest: still refused, root unchanged; an identical re-forward is accepted.
    assert_eq!(
        devs[0].add_reveal(1, &second),
        Err(CeremonyError::ConflictingReveal(1))
    );
    assert_eq!(devs[0].phase(), Phase::Attest);
    assert_eq!(root_of(&devs[0]), root);
    devs[0].add_reveal(1, &first).unwrap();

    exchange_attestations(&mut devs);
    assert_eq!(
        devs[0].add_reveal(1, &second),
        Err(CeremonyError::ConflictingReveal(1))
    );
    assert_eq!(devs[0].phase(), Phase::Complete);
    let pair_keys: Vec<_> = devs
        .into_iter()
        .map(|d| {
            let r = d.finish().unwrap();
            assert_eq!(r.root.as_deref(), Some(&root));
            r.pair_keys
        })
        .collect();
    // Member 0's pair key with member 1 is still the one member 1 derived: the second blob's
    // X-Wing secret did not replace the first.
    assert_eq!(
        pair_keys[0][1].as_deref(),
        pair_keys[1][0].as_deref(),
        "pair key 0<->1 agrees"
    );
}

/// Rebuilds `crates/vectors/root.json`: the root from the ceremony's content keys, the chat
/// keys and the seal keys.
#[test]
fn the_root_vectors_are_reproduced() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../vectors/root.json")).unwrap();
    let bytes = |x: &serde_json::Value| hex::decode(x.as_str().unwrap()).unwrap();
    let circle_id: CircleId = bytes(&v["circle_id"]).try_into().unwrap();
    // The circle id is the one in the ceremony vectors the content keys come from.
    let ceremony: serde_json::Value =
        serde_json::from_str(include_str!("../../vectors/ceremony.json")).unwrap();
    assert_eq!(v["circle_id"], ceremony["circle_id"]);
    let keys: Vec<[u8; 32]> = v["content_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| bytes(k).try_into().unwrap())
        .collect();
    assert_eq!(keys.len(), 3);
    let root: [u8; 32] = bytes(&v["root"]).try_into().unwrap();
    assert_eq!(root::derive(&keys, &circle_id), root);
    for c in v["chat_keys"].as_array().unwrap() {
        let epoch = c["epoch"].as_u64().unwrap();
        assert_eq!(root::chat_key(&root, &circle_id, epoch).to_vec(), bytes(&c["key"]));
    }
    let seal = v["seal_keys"].as_array().unwrap();
    assert_eq!(seal[0]["label"], "seal/card");
    for c in seal {
        let label = c["label"].as_str().unwrap().as_bytes();
        assert_eq!(root::seal_key(&root, label, &circle_id).to_vec(), bytes(&c["key"]));
    }
}
