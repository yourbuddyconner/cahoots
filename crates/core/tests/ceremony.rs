use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::circle_id;
use cahoots_core::cose;
use cahoots_core::manifest::{Manifest, MANIFEST_AAD};
use cahoots_core::params::CircleParams;
use cahoots_core::reveal;
use cahoots_core::seat::{SeatKey, SeatPublic};

fn params() -> CircleParams {
    CircleParams::new(3, 512, "ring", false).unwrap()
}

fn contribution(i: u8) -> Vec<u8> {
    (0..512u32)
        .map(|k| (k.wrapping_mul(13 + i as u32) ^ i as u32) as u8)
        .collect()
}

fn seed(i: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    for (k, b) in s.iter_mut().enumerate() {
        *b = (k as u8).wrapping_mul(7).wrapping_add(i.wrapping_mul(31));
    }
    s
}

fn seat_seed(i: u8) -> [u8; 32] {
    [i.wrapping_add(1); 32]
}

fn seat(i: u8) -> SeatPublic {
    SeatKey::from_seed(&seat_seed(i)).unwrap().public()
}

/// Each member declares a different tier, so a swap between members shows.
fn tier(i: u8) -> u8 {
    i % 5
}

/// Deterministic randomness for sealing: a counter stream per member.
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

fn new(i: u8) -> Ceremony {
    Ceremony::new(params(), i, contribution(i), seed(i), seat_seed(i), tier(i)).unwrap()
}

fn exchange_commitments(devs: &mut [Ceremony]) {
    let commits: Vec<_> = devs
        .iter()
        .map(|d| {
            (
                d.index(),
                d.my_commitment(),
                *d.my_public_key(),
                d.my_seat_public(),
                tier(d.index()),
            )
        })
        .collect();
    for d in devs.iter_mut() {
        for (j, c, pk, sp, t) in &commits {
            d.add_commitment(*j, *c, pk, &sp.0, *t).unwrap();
        }
    }
}

fn seal_all(devs: &mut [Ceremony]) -> Vec<Vec<u8>> {
    devs.iter_mut()
        .map(|d| {
            let i = d.index();
            d.my_reveal(&mut filler(i)).unwrap().to_vec()
        })
        .collect()
}

/// Every member's attestation, handed to every member.
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

/// Commitments and reveals exchanged: every member is in `Attest`.
fn to_attest() -> Vec<Ceremony> {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let reveals = seal_all(&mut devs);
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
    }
    devs
}

#[test]
fn three_devices_agree_on_pad_words_and_id() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    for d in devs.iter_mut() {
        assert_eq!(d.phase(), Phase::Commit);
        assert!(d.my_reveal(&mut filler(9)).is_none());
    }
    exchange_commitments(&mut devs);
    for d in &devs {
        assert_eq!(d.phase(), Phase::Reveal);
    }
    let reveals = seal_all(&mut devs);
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
        assert_eq!(d.phase(), Phase::Attest);
    }
    exchange_attestations(&mut devs);
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap()).collect();
    let mut expected = vec![0u8; 512];
    for i in 0..3u8 {
        for (e, r) in expected.iter_mut().zip(contribution(i)) {
            *e ^= r;
        }
    }
    for r in &results {
        assert_eq!(r.pad.as_slice(), expected.as_slice());
        assert_eq!(r.check_words, check_words(&params(), &expected));
        assert_eq!(r.circle_id, circle_id(&expected));
    }
}

#[test]
fn sealed_reveal_never_contains_the_contribution() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let blob = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    let r = contribution(1);
    assert!(
        !blob.windows(32).any(|w| w == &r[..32]),
        "contribution bytes appear in the clear"
    );
    assert_eq!(reveal::sender(&blob), Some(1));
    // Built once: a second call returns the same bytes without drawing more randomness.
    let mut calls = 0;
    let again = devs[1]
        .my_reveal(&mut |_: &mut [u8]| calls += 1)
        .unwrap()
        .to_vec();
    assert_eq!(again, blob);
    assert_eq!(calls, 0);
}

#[test]
fn garbled_reveal_is_refused_without_aborting() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let reveals = seal_all(&mut devs);
    let mut bad = reveals[1].clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    assert_eq!(
        devs[0].add_reveal(1, &bad).unwrap_err(),
        CeremonyError::Corrupted(1)
    );
    assert_eq!(devs[0].phase(), Phase::Reveal, "nothing changed");
    assert!(devs[0].missing_reveals().contains(&1));
    // The untouched copy still works.
    devs[0].add_reveal(1, &reveals[1]).unwrap();
    assert!(!devs[0].missing_reveals().contains(&1));
}

#[test]
fn reveal_not_for_me_is_refused() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    // Seal member 1's contribution to member 2 only, then hand it to member 0.
    let pk2 = *devs[2].my_public_key();
    let blob = reveal::seal(
        &params(),
        1,
        devs[1].my_public_key(),
        &seat(1),
        tier(1),
        &[(2, &pk2)],
        &contribution(1),
        &mut filler(1),
    )
    .unwrap()
    .blob;
    assert_eq!(
        devs[0].add_reveal(1, &blob).unwrap_err(),
        CeremonyError::Corrupted(1)
    );
    assert_eq!(devs[0].phase(), Phase::Reveal);
}

#[test]
fn a_validly_sealed_wrong_contribution_aborts() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let mut bad = contribution(1);
    bad[3] ^= 0xFF;
    let recipients: Vec<(u8, &[u8])> =
        vec![(0, devs[0].my_public_key()), (2, devs[2].my_public_key())];
    let blob = reveal::seal(
        &params(),
        1,
        devs[1].my_public_key(),
        &seat(1),
        tier(1),
        &recipients,
        &bad,
        &mut filler(1),
    )
    .unwrap()
    .blob;
    assert_eq!(
        devs[0].add_reveal(1, &blob).unwrap_err(),
        CeremonyError::CommitMismatch(1)
    );
    assert_eq!(devs[0].phase(), Phase::Aborted);
    assert_eq!(
        devs[0].add_reveal(2, &[0u8; 8]).unwrap_err(),
        CeremonyError::Aborted
    );
}

#[test]
fn substituted_public_key_fails_the_commitment() {
    // Someone inline on the cable swaps member 1's key for their own. The commitment was
    // made over the real key, so the reveal sealed with the real key no longer matches.
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    let (_, fake_pk) = reveal::keypair(&seed(77));
    let c1 = devs[1].my_commitment();
    devs[0]
        .add_commitment(1, c1, &fake_pk, &seat(1).0, tier(1))
        .unwrap();
    let (c2, pk2) = (devs[2].my_commitment(), *devs[2].my_public_key());
    devs[0]
        .add_commitment(2, c2, &pk2, &seat(2).0, tier(2))
        .unwrap();
    exchange_commitments(&mut devs[1..]);
    let (c0, pk0) = (devs[0].my_commitment(), *devs[0].my_public_key());
    devs[1]
        .add_commitment(0, c0, &pk0, &seat(0).0, tier(0))
        .unwrap();
    let blob = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    // Member 0 cannot even open it: it was sealed to member 0's real key, which is fine,
    // but the commitment check uses the substituted key and fails.
    let err = devs[0].add_reveal(1, &blob).unwrap_err();
    assert!(
        matches!(
            err,
            CeremonyError::CommitMismatch(1) | CeremonyError::Corrupted(1)
        ),
        "{err:?}"
    );
}

#[test]
fn late_reveal_stands_in_for_its_commitment() {
    // Member 0 has only its own commitment. Members 1 and 2 are ahead and already sealing.
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    let c0 = (devs[0].my_commitment(), *devs[0].my_public_key());
    for d in devs[1..].iter_mut() {
        d.add_commitment(0, c0.0, &c0.1, &seat(0).0, tier(0))
            .unwrap();
    }
    exchange_commitments(&mut devs[1..]);
    let r1 = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    let r2 = devs[2].my_reveal(&mut filler(2)).unwrap().to_vec();
    assert_eq!(devs[0].phase(), Phase::Commit);
    devs[0].add_reveal(1, &r1).unwrap();
    assert_eq!(devs[0].phase(), Phase::Commit);
    assert_eq!(devs[0].missing_commitments(), vec![2]);
    assert_eq!(devs[0].public_key_of(1), Some(devs[1].my_public_key()));
    devs[0].add_reveal(2, &r2).unwrap();
    assert_eq!(devs[0].phase(), Phase::Attest);
    // The stand-in commitments equal the real ones.
    assert_eq!(devs[0].missing_commitments(), Vec::<u8>::new());
    assert!(devs[0].manifest().is_some());
}

#[test]
fn late_path_still_lets_the_others_finish() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    let c0 = (devs[0].my_commitment(), *devs[0].my_public_key());
    for d in devs[1..].iter_mut() {
        d.add_commitment(0, c0.0, &c0.1, &seat(0).0, tier(0))
            .unwrap();
    }
    exchange_commitments(&mut devs[1..]);
    let r1 = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    let r2 = devs[2].my_reveal(&mut filler(2)).unwrap().to_vec();
    devs[0].add_reveal(1, &r1).unwrap();
    devs[0].add_reveal(2, &r2).unwrap();
    let r0 = devs[0].my_reveal(&mut filler(0)).unwrap().to_vec();
    devs[1].add_reveal(0, &r0).unwrap();
    devs[1].add_reveal(2, &r2).unwrap();
    devs[2].add_reveal(0, &r0).unwrap();
    devs[2].add_reveal(1, &r1).unwrap();
    exchange_attestations(&mut devs);
    let ids: Vec<_> = devs
        .into_iter()
        .map(|d| d.finish().unwrap().circle_id)
        .collect();
    assert!(ids.iter().all(|i| i == &ids[0]));
}

#[test]
fn own_reveal_is_a_no_op_and_rejects_before_commit_phase_only_for_others() {
    let mut d = new(0);
    assert_eq!(d.add_reveal(0, &[1, 2, 3]), Ok(()));
    assert_eq!(
        d.add_reveal(1, &[1, 2, 3]).unwrap_err(),
        CeremonyError::Corrupted(1)
    );
    assert_eq!(
        d.add_reveal(5, &[1, 2, 3]).unwrap_err(),
        CeremonyError::BadIndex(5)
    );
}

#[test]
fn rejects_conflicting_duplicate_commitment_bad_index_and_bad_key() {
    let mut d = new(0);
    let other = new(1);
    let (c, pk) = (other.my_commitment(), *other.my_public_key());
    let (sp, t) = (seat(1).0, tier(1));
    d.add_commitment(1, c, &pk, &sp, t).unwrap();
    assert_eq!(
        d.add_commitment(1, c, &pk, &sp, t),
        Ok(()),
        "identical duplicate is a ring re-forward"
    );
    assert_eq!(
        d.add_commitment(1, [2u8; 32], &pk, &sp, t).unwrap_err(),
        CeremonyError::ConflictingCommit(1)
    );
    let (_, other_pk) = reveal::keypair(&seed(9));
    assert_eq!(
        d.add_commitment(1, c, &other_pk, &sp, t).unwrap_err(),
        CeremonyError::ConflictingCommit(1)
    );
    assert_eq!(
        d.add_commitment(3, c, &pk, &sp, t).unwrap_err(),
        CeremonyError::BadIndex(3)
    );
    let my_pk = *d.my_public_key();
    assert_eq!(
        d.add_commitment(0, [9u8; 32], &my_pk, &seat(0).0, tier(0))
            .unwrap_err(),
        CeremonyError::ConflictingCommit(0)
    );
    assert_eq!(
        d.add_commitment(2, c, &[0u8; 10], &sp, t).unwrap_err(),
        CeremonyError::BadKey(2)
    );
    assert_eq!(
        d.add_commitment(2, c, &pk, &[0u8; 33], t).unwrap_err(),
        CeremonyError::BadKey(2),
        "a seat key that does not parse"
    );
    assert_eq!(
        d.add_commitment(1, c, &pk, &seat(9).0, t).unwrap_err(),
        CeremonyError::ConflictingCommit(1),
        "another seat key for a held commitment"
    );
    assert_eq!(
        d.add_commitment(1, c, &pk, &sp, 4).unwrap_err(),
        CeremonyError::ConflictingCommit(1),
        "another tier for a held commitment"
    );
}

#[test]
fn rejects_wrong_contribution_length() {
    assert_eq!(
        Ceremony::new(params(), 0, vec![0u8; 100], seed(0), seat_seed(0), 0).unwrap_err(),
        CeremonyError::ContributionLength
    );
    assert_eq!(
        Ceremony::new(params(), 3, contribution(0), seed(0), seat_seed(0), 0).unwrap_err(),
        CeremonyError::BadIndex(3)
    );
}

#[test]
fn finish_before_complete_is_an_error() {
    let d = new(0);
    assert_eq!(d.finish().unwrap_err(), CeremonyError::WrongPhase);
}

#[test]
fn extra_source_is_xored_into_own_contribution_before_commit() {
    let base = contribution(0);
    let extra = contribution(7);
    let d =
        Ceremony::with_extra_source(params(), 0, base.clone(), &extra, seed(0), seat_seed(0), 0)
            .unwrap();
    let mut mixed = base;
    for (m, e) in mixed.iter_mut().zip(&extra) {
        *m ^= e;
    }
    let plain = Ceremony::new(params(), 0, mixed, seed(0), seat_seed(0), 0).unwrap();
    assert_eq!(d.my_commitment(), plain.my_commitment());
    assert_eq!(d.my_public_key(), plain.my_public_key());
}

#[test]
fn every_pair_shares_a_key_nobody_else_has() {
    use cahoots_core::pairwise::words;
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let reveals = seal_all(&mut devs);
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
    }
    exchange_attestations(&mut devs);
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap()).collect();
    for r in &results {
        assert_eq!(r.pair_keys.len(), 3);
    }
    let k = |i: usize, j: usize| results[i].pair_keys[j].as_ref().unwrap().clone();
    assert!(results[0].pair_keys[0].is_none(), "no key with yourself");
    assert_eq!(*k(0, 1), *k(1, 0), "0 and 1 agree");
    assert_eq!(*k(0, 2), *k(2, 0), "0 and 2 agree");
    assert_eq!(*k(1, 2), *k(2, 1), "1 and 2 agree");
    assert_ne!(*k(0, 1), *k(0, 2), "different pairs, different keys");
    assert_ne!(*k(0, 1), *k(1, 2));
    // Words: each speaker has its own pair, and the counter changes them.
    let a = words(&k(0, 1), 0, 7);
    let b = words(&k(0, 1), 1, 7);
    assert_ne!(a, b);
    assert_eq!(
        a,
        words(&k(1, 0), 0, 7),
        "both sides compute the same words"
    );
    assert_ne!(a, words(&k(0, 1), 0, 8));
    assert_ne!(
        a,
        words(&k(0, 2), 0, 7),
        "a third member gets different words"
    );
}

#[test]
fn late_path_still_yields_pair_keys() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    let c0 = (devs[0].my_commitment(), *devs[0].my_public_key());
    for d in devs[1..].iter_mut() {
        d.add_commitment(0, c0.0, &c0.1, &seat(0).0, tier(0))
            .unwrap();
    }
    exchange_commitments(&mut devs[1..]);
    let r1 = devs[1].my_reveal(&mut filler(1)).unwrap().to_vec();
    let r2 = devs[2].my_reveal(&mut filler(2)).unwrap().to_vec();
    devs[0].add_reveal(1, &r1).unwrap();
    devs[0].add_reveal(2, &r2).unwrap();
    let r0 = devs[0].my_reveal(&mut filler(0)).unwrap().to_vec();
    devs[1].add_reveal(0, &r0).unwrap();
    devs[1].add_reveal(2, &r2).unwrap();
    devs[2].add_reveal(0, &r0).unwrap();
    devs[2].add_reveal(1, &r1).unwrap();
    exchange_attestations(&mut devs);
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap()).collect();
    assert_eq!(
        *results[0].pair_keys[1].as_ref().unwrap(),
        *results[1].pair_keys[0].as_ref().unwrap()
    );
    assert_eq!(
        *results[0].pair_keys[2].as_ref().unwrap(),
        *results[2].pair_keys[0].as_ref().unwrap()
    );
}

/// Member 1's own contribution, sealed to members 0 and 2 under the given seat key and tier.
fn reseal_1(devs: &[Ceremony], seat_pk: &SeatPublic, t: u8) -> Vec<u8> {
    let recipients: Vec<(u8, &[u8])> =
        vec![(0, devs[0].my_public_key()), (2, devs[2].my_public_key())];
    reveal::seal(
        &params(),
        1,
        devs[1].my_public_key(),
        seat_pk,
        t,
        &recipients,
        &contribution(1),
        &mut filler(1),
    )
    .unwrap()
    .blob
}

#[test]
fn commitments_record_seat_keys_and_tiers() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    assert_eq!(devs[0].my_seat_public(), seat(0));
    assert_eq!(devs[0].seat_public_of(0), Some(&seat(0)));
    assert_eq!(devs[0].tier_of(0), Some(tier(0)));
    assert_eq!(devs[0].seat_public_of(1), None);
    assert_eq!(devs[0].tier_of(1), None);
    exchange_commitments(&mut devs);
    for i in 0..3u8 {
        assert_eq!(devs[0].seat_public_of(i), Some(&seat(i)));
        assert_eq!(devs[0].tier_of(i), Some(tier(i)));
    }
    assert_eq!(devs[0].seat_public_of(3), None);
}

#[test]
fn reveal_with_another_seat_key_than_committed_aborts_with_commit_mismatch() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let blob = reseal_1(&devs, &seat(9), tier(1));
    assert_eq!(
        devs[0].add_reveal(1, &blob).unwrap_err(),
        CeremonyError::CommitMismatch(1)
    );
    assert_eq!(devs[0].phase(), Phase::Aborted);
}

#[test]
fn reveal_with_another_tier_than_committed_aborts() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let blob = reseal_1(&devs, &seat(1), tier(1) + 1);
    assert_eq!(
        devs[0].add_reveal(1, &blob).unwrap_err(),
        CeremonyError::CommitMismatch(1)
    );
    assert_eq!(devs[0].phase(), Phase::Aborted);
}

#[test]
fn a_late_reveal_stands_in_and_records_seat_key_and_tier() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    let c0 = (devs[0].my_commitment(), *devs[0].my_public_key());
    for d in devs[1..].iter_mut() {
        d.add_commitment(0, c0.0, &c0.1, &seat(0).0, tier(0))
            .unwrap();
    }
    exchange_commitments(&mut devs[1..]);
    let r2 = devs[2].my_reveal(&mut filler(2)).unwrap().to_vec();
    assert_eq!(devs[0].seat_public_of(2), None);
    devs[0].add_reveal(2, &r2).unwrap();
    assert_eq!(devs[0].seat_public_of(2), Some(&seat(2)));
    assert_eq!(devs[0].tier_of(2), Some(tier(2)));
    // The stand-in is the real commitment: the real one arriving later is a duplicate.
    let (c2, pk2) = (devs[2].my_commitment(), *devs[2].my_public_key());
    devs[0]
        .add_commitment(2, c2, &pk2, &seat(2).0, tier(2))
        .unwrap();
}

#[test]
fn bad_seed_is_reported() {
    for bad in [[0u8; 32], [0xFFu8; 32]] {
        assert_eq!(
            Ceremony::new(params(), 0, contribution(0), seed(0), bad, 0).unwrap_err(),
            CeremonyError::BadSeed
        );
    }
}

#[test]
fn tier_above_4_is_refused() {
    assert_eq!(
        Ceremony::new(params(), 0, contribution(0), seed(0), seat_seed(0), 5).unwrap_err(),
        CeremonyError::BadTier
    );
    Ceremony::new(params(), 0, contribution(0), seed(0), seat_seed(0), 4).unwrap();
    let mut d = new(0);
    let other = new(1);
    assert_eq!(
        d.add_commitment(
            1,
            other.my_commitment(),
            other.my_public_key(),
            &seat(1).0,
            5
        )
        .unwrap_err(),
        CeremonyError::BadTier
    );
}

#[test]
fn a_stand_in_reveal_then_a_commitment_with_another_seat_key_or_tier_conflicts() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    let c0 = (devs[0].my_commitment(), *devs[0].my_public_key());
    for d in devs[1..].iter_mut() {
        d.add_commitment(0, c0.0, &c0.1, &seat(0).0, tier(0))
            .unwrap();
    }
    exchange_commitments(&mut devs[1..]);
    let r2 = devs[2].my_reveal(&mut filler(2)).unwrap().to_vec();
    devs[0].add_reveal(2, &r2).unwrap();
    let (c2, pk2) = (devs[2].my_commitment(), *devs[2].my_public_key());
    assert_eq!(
        devs[0]
            .add_commitment(2, c2, &pk2, &seat(9).0, tier(2))
            .unwrap_err(),
        CeremonyError::ConflictingCommit(2),
        "another seat key than the stand-in"
    );
    assert_eq!(
        devs[0]
            .add_commitment(2, c2, &pk2, &seat(2).0, tier(2) + 1)
            .unwrap_err(),
        CeremonyError::ConflictingCommit(2),
        "another tier than the stand-in"
    );
}

#[test]
fn three_members_build_identical_manifest_bytes() {
    let devs = to_attest();
    let body = devs[0].manifest().unwrap().to_vec();
    for d in &devs {
        assert_eq!(d.manifest().unwrap(), body.as_slice());
    }
    let m = Manifest::decode(&body).unwrap();
    assert_eq!(m.params, params());
    assert_eq!(m.genesis, 0);
    assert_eq!(m.floor, Manifest::default_floor(&params()));
    assert_eq!(m.seats.len(), 3);
    for (i, s) in m.seats.iter().enumerate() {
        let i = i as u8;
        assert_eq!(s.seat, i);
        assert_eq!(s.key, seat(i));
        assert_eq!(s.tier, tier(i));
        assert!(s.evidence.is_empty());
    }
    let mut expected = vec![0u8; 512];
    for i in 0..3u8 {
        for (e, r) in expected.iter_mut().zip(contribution(i)) {
            *e ^= r;
        }
    }
    assert_eq!(m.circle_id, circle_id(&expected));
}

#[test]
fn the_phase_is_attest_after_the_last_reveal_and_complete_after_the_last_attestation() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    exchange_commitments(&mut devs);
    let reveals = seal_all(&mut devs);
    devs[0].add_reveal(1, &reveals[1]).unwrap();
    assert_eq!(devs[0].phase(), Phase::Reveal);
    assert!(devs[0].manifest().is_none());
    assert!(devs[0].my_attestation().is_none());
    devs[0].add_reveal(2, &reveals[2]).unwrap();
    assert_eq!(devs[0].phase(), Phase::Attest);
    assert_eq!(devs[0].missing_attestations(), vec![1, 2]);
    for d in devs[1..].iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
    }
    let a1 = devs[1].my_attestation().unwrap();
    let a2 = devs[2].my_attestation().unwrap();
    // Built once: the same bytes every call.
    assert_eq!(devs[1].my_attestation().unwrap(), a1);
    devs[0].add_attestation(1, &a1).unwrap();
    assert_eq!(devs[0].phase(), Phase::Attest);
    assert_eq!(devs[0].missing_attestations(), vec![2]);
    devs[0].add_attestation(2, &a2).unwrap();
    assert_eq!(devs[0].phase(), Phase::Complete);
    assert!(devs[0].missing_attestations().is_empty());
    // A re-forward after completion is accepted.
    devs[0].add_attestation(2, &a2).unwrap();
    // A re-forwarded reveal after Attest is still verified and changes nothing.
    devs[0].add_reveal(1, &reveals[1]).unwrap();
    assert_eq!(devs[0].phase(), Phase::Complete);
}

#[test]
fn an_attestation_before_attest_is_wrong_phase() {
    let mut devs: Vec<Ceremony> = (0..3).map(new).collect();
    assert!(devs[0].manifest().is_none());
    assert_eq!(
        devs[0].add_attestation(1, &[0u8; 4]).unwrap_err(),
        CeremonyError::WrongPhase
    );
    exchange_commitments(&mut devs);
    assert_eq!(
        devs[0].add_attestation(1, &[0u8; 4]).unwrap_err(),
        CeremonyError::WrongPhase
    );
    let finished = to_attest();
    let a1 = finished[1].my_attestation().unwrap();
    assert_eq!(
        devs[0].add_attestation(1, &a1).unwrap_err(),
        CeremonyError::WrongPhase
    );
    assert_eq!(
        devs.remove(0).finish().unwrap_err(),
        CeremonyError::WrongPhase
    );
}

#[test]
fn a_bad_attestation_is_reported_and_does_not_abort() {
    let mut devs = to_attest();
    let body = devs[0].manifest().unwrap().to_vec();
    let a1 = devs[1].my_attestation().unwrap();
    // Garbled signature.
    let mut bad = a1.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    assert_eq!(
        devs[0].add_attestation(1, &bad).unwrap_err(),
        CeremonyError::BadAttestation(1)
    );
    // Seat 2's attestation presented as seat 1's: wrong kid and wrong key.
    let a2 = devs[2].my_attestation().unwrap();
    assert_eq!(
        devs[0].add_attestation(1, &a2).unwrap_err(),
        CeremonyError::BadAttestation(1)
    );
    // Seat 1's key over other bytes.
    let k1 = SeatKey::from_seed(&seat_seed(1)).unwrap();
    let mut other = body.clone();
    other.push(0);
    let wrong_body = cose::sign1(&k1, &[1], &other, MANIFEST_AAD, true);
    assert_eq!(
        devs[0].add_attestation(1, &wrong_body).unwrap_err(),
        CeremonyError::BadAttestation(1)
    );
    // Right key and bytes, but another kid.
    let wrong_kid = cose::sign1(&k1, &[2], &body, MANIFEST_AAD, true);
    assert_eq!(
        devs[0].add_attestation(1, &wrong_kid).unwrap_err(),
        CeremonyError::BadAttestation(1)
    );
    assert_eq!(
        devs[0].add_attestation(1, &[1, 2, 3]).unwrap_err(),
        CeremonyError::BadAttestation(1)
    );
    assert_eq!(
        devs[0].add_attestation(3, &a1).unwrap_err(),
        CeremonyError::BadIndex(3)
    );
    assert_eq!(devs[0].phase(), Phase::Attest, "nothing changed");
    assert_eq!(devs[0].missing_attestations(), vec![1, 2]);
    // Our own index: ours is a no-op, anything else conflicts.
    let mine = devs[0].my_attestation().unwrap();
    devs[0].add_attestation(0, &mine).unwrap();
    assert_eq!(
        devs[0].add_attestation(0, &a1).unwrap_err(),
        CeremonyError::ConflictingAttestation(0)
    );
    devs[0].add_attestation(1, &a1).unwrap();
    // An embedded payload instead of a detached one is not an attestation.
    let embedded = cose::sign1(&k1, &[1], &body, MANIFEST_AAD, false);
    assert_eq!(
        devs[0].add_attestation(1, &embedded).unwrap_err(),
        CeremonyError::BadAttestation(1)
    );
    // A different, valid attestation from seat 1: the same signature with s negated.
    assert_eq!(
        devs[0].add_attestation(1, &high_s(&a1)).unwrap_err(),
        CeremonyError::ConflictingAttestation(1)
    );
    devs[0].add_attestation(2, &a2).unwrap();
    assert_eq!(devs[0].phase(), Phase::Complete);
}

/// The other valid ECDSA signature over the same message: `s` replaced by `order - s`. The
/// signature is the last 64 bytes of a COSE_Sign1.
fn high_s(cose: &[u8]) -> Vec<u8> {
    const ORDER: [u8; 32] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xBC, 0xE6, 0xFA, 0xAD, 0xA7, 0x17, 0x9E, 0x84, 0xF3, 0xB9, 0xCA, 0xC2, 0xFC, 0x63,
        0x25, 0x51,
    ];
    let mut out = cose.to_vec();
    let at = out.len() - 32;
    let mut borrow = 0i16;
    for k in (0..32).rev() {
        let v = i16::from(ORDER[k]) - i16::from(out[at + k]) - borrow;
        borrow = i16::from(v < 0);
        out[at + k] = v.rem_euclid(256) as u8;
    }
    out
}

#[test]
fn finish_returns_the_sealed_manifest_that_verifies() {
    let mut devs = to_attest();
    let body = devs[0].manifest().unwrap().to_vec();
    exchange_attestations(&mut devs);
    let atts: Vec<_> = devs.iter().map(|d| d.my_attestation().unwrap()).collect();
    for d in devs {
        let r = d.finish().unwrap();
        assert_eq!(r.manifest.body, body);
        assert_eq!(r.manifest.attestations, atts, "in seat order");
        let m = r.manifest.verify().unwrap();
        assert_eq!(m.circle_id, r.circle_id);
        assert_eq!(m.params, r.params);
    }
}

#[test]
fn the_result_carries_the_seat_key_whose_public_is_in_the_roster() {
    let mut devs = to_attest();
    exchange_attestations(&mut devs);
    for d in devs {
        let i = d.index();
        let r = d.finish().unwrap();
        assert_eq!(r.tier, tier(i));
        let m = r.manifest.verify().unwrap();
        let key = r.seat_key.as_ref().expect("built from a seed");
        assert_eq!(m.seat_key(i), Some(&key.public()));
        assert_eq!(key.public(), seat(i));
    }
}

fn unhex(v: &serde_json::Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

/// Rebuilds `crates/vectors/ceremony.json` from its inputs and compares every derived field.
#[test]
fn the_vectors_file_is_reproduced_from_its_inputs() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../vectors/ceremony.json")).unwrap();
    let p = &v["params"];
    let params = CircleParams::new(
        p["n"].as_u64().unwrap() as u8,
        p["pad_len"].as_u64().unwrap() as u32,
        p["name"].as_str().unwrap(),
        p["agent_seat"].as_bool().unwrap(),
    )
    .unwrap();
    assert_eq!(hex::encode(params.encode()), p["encoded"].as_str().unwrap());
    let n = usize::from(params.n);
    // Per-member fields are parallel arrays at the top level.
    let field = |k: &str, i: usize| v[k].as_array().unwrap()[i].clone();
    let tier_of = |i: usize| field("tiers", i).as_u64().unwrap() as u8;
    let mut devs: Vec<Ceremony> = (0..n)
        .map(|i| {
            let seat_seed: [u8; 32] = unhex(&field("seat_seeds", i)).try_into().unwrap();
            let kem_seed: [u8; 32] = unhex(&field("kem_seeds", i)).try_into().unwrap();
            let d = Ceremony::new(
                params.clone(),
                i as u8,
                unhex(&field("contributions", i)),
                kem_seed,
                seat_seed,
                tier_of(i),
            )
            .unwrap();
            let sp = SeatKey::from_seed(&seat_seed).unwrap().public();
            assert_eq!(d.my_seat_public(), sp);
            assert_eq!(sp.0.to_vec(), unhex(&field("seat_publics", i)));
            assert_eq!(d.my_public_key().to_vec(), unhex(&field("public_keys", i)));
            assert_eq!(d.my_commitment().to_vec(), unhex(&field("commitments", i)));
            d
        })
        .collect();
    let commits: Vec<_> = devs
        .iter()
        .map(|d| (d.my_commitment(), *d.my_public_key(), d.my_seat_public()))
        .collect();
    for d in devs.iter_mut() {
        for (j, (c, pk, sp)) in commits.iter().enumerate() {
            d.add_commitment(j as u8, *c, pk, &sp.0, tier_of(j))
                .unwrap();
        }
    }
    let reveals: Vec<Vec<u8>> = devs
        .iter_mut()
        .enumerate()
        .map(|(i, d)| {
            let rand = unhex(&field("seal_randomness", i));
            let mut pos = 0usize;
            let mut f = |buf: &mut [u8]| {
                buf.copy_from_slice(&rand[pos..pos + buf.len()]);
                pos += buf.len();
            };
            let blob = d.my_reveal(&mut f).unwrap().to_vec();
            assert_eq!(blob, unhex(&field("sealed_reveals", i)));
            blob
        })
        .collect();
    for d in devs.iter_mut() {
        for (j, r) in reveals.iter().enumerate() {
            d.add_reveal(j as u8, r).unwrap();
        }
    }
    let body = unhex(&v["manifest"]);
    for d in &devs {
        assert_eq!(d.manifest().unwrap(), body.as_slice());
    }
    let atts: Vec<Vec<u8>> = devs.iter().map(|d| d.my_attestation().unwrap()).collect();
    for (i, a) in atts.iter().enumerate() {
        assert_eq!(*a, unhex(&field("attestations", i)));
    }
    for d in devs.iter_mut() {
        for (j, a) in atts.iter().enumerate() {
            d.add_attestation(j as u8, a).unwrap();
        }
    }
    let m = Manifest::decode(&body).unwrap();
    assert_eq!(hex::encode(m.id()), v["manifest_id"].as_str().unwrap());
    for d in devs {
        let r = d.finish().unwrap();
        assert_eq!(r.manifest.encode(), unhex(&v["sealed_manifest"]));
        assert_eq!(r.manifest.verify().unwrap().id(), m.id());
        assert_eq!(r.pad.to_vec(), unhex(&v["pad"]));
    }
}
