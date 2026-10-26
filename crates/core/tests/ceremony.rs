use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::circle_id;
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
        assert_eq!(d.phase(), Phase::Complete);
    }
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
    assert_eq!(devs[0].phase(), Phase::Complete);
    // The stand-in commitments equal the real ones.
    assert_eq!(devs[0].missing_commitments(), Vec::<u8>::new());
    devs.remove(0).finish().unwrap();
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
