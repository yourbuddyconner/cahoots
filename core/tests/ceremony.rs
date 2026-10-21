use cahoots_core::ceremony::{Ceremony, CeremonyError, Phase};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::circle_id;
use cahoots_core::params::CircleParams;

fn params() -> CircleParams {
    CircleParams::new(3, 512, "ring", false).unwrap()
}

fn contribution(i: u8) -> Vec<u8> {
    (0..512u32)
        .map(|k| (k.wrapping_mul(13 + i as u32) ^ i as u32) as u8)
        .collect()
}

#[test]
fn three_devices_agree_on_pad_words_and_id() {
    let mut devs: Vec<Ceremony> = (0..3)
        .map(|i| Ceremony::new(params(), i, contribution(i)).unwrap())
        .collect();
    let commits: Vec<_> = devs.iter().map(|d| d.my_commitment()).collect();
    for d in devs.iter_mut() {
        assert_eq!(d.phase(), Phase::Commit);
        assert!(d.my_reveal().is_none());
        for (j, c) in commits.iter().enumerate() {
            d.add_commitment(j as u8, *c).unwrap();
        }
        assert_eq!(d.phase(), Phase::Reveal);
    }
    let reveals: Vec<Vec<u8>> = devs
        .iter()
        .map(|d| d.my_reveal().unwrap().to_vec())
        .collect();
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
fn reveal_not_matching_commit_aborts() {
    let mut d = Ceremony::new(params(), 0, contribution(0)).unwrap();
    let mut other = Ceremony::new(params(), 1, contribution(1)).unwrap();
    let third = Ceremony::new(params(), 2, contribution(2)).unwrap();
    for c in [
        d.my_commitment(),
        other.my_commitment(),
        third.my_commitment(),
    ] {
        d.add_commitment(0, c).ok();
    }
    d.add_commitment(0, d.my_commitment()).unwrap_or(());
    d.add_commitment(1, other.my_commitment()).unwrap();
    d.add_commitment(2, third.my_commitment()).unwrap();
    other.add_commitment(0, d.my_commitment()).unwrap();
    other.add_commitment(1, other.my_commitment()).unwrap();
    other.add_commitment(2, third.my_commitment()).unwrap();
    let mut bad = contribution(1);
    bad[3] ^= 0xFF;
    assert_eq!(
        d.add_reveal(1, &bad).unwrap_err(),
        CeremonyError::CommitMismatch(1)
    );
    assert_eq!(d.phase(), Phase::Aborted);
    assert_eq!(
        d.add_reveal(2, &contribution(2)).unwrap_err(),
        CeremonyError::Aborted
    );
}

#[test]
fn rejects_reveal_before_all_commitments() {
    let mut d = Ceremony::new(params(), 0, contribution(0)).unwrap();
    assert_eq!(
        d.add_reveal(1, &contribution(1)).unwrap_err(),
        CeremonyError::WrongPhase
    );
}

#[test]
fn rejects_conflicting_duplicate_commitment_and_bad_index() {
    let mut d = Ceremony::new(params(), 0, contribution(0)).unwrap();
    d.add_commitment(1, [1u8; 32]).unwrap();
    assert_eq!(
        d.add_commitment(1, [1u8; 32]),
        Ok(()),
        "identical duplicate is a ring re-forward"
    );
    assert_eq!(
        d.add_commitment(1, [2u8; 32]).unwrap_err(),
        CeremonyError::ConflictingCommit(1)
    );
    assert_eq!(
        d.add_commitment(3, [1u8; 32]).unwrap_err(),
        CeremonyError::BadIndex(3)
    );
    assert_eq!(
        d.add_commitment(0, [9u8; 32]).unwrap_err(),
        CeremonyError::ConflictingCommit(0)
    );
}

#[test]
fn rejects_wrong_contribution_length() {
    assert_eq!(
        Ceremony::new(params(), 0, vec![0u8; 100]).unwrap_err(),
        CeremonyError::ContributionLength
    );
    assert_eq!(
        Ceremony::new(params(), 3, contribution(0)).unwrap_err(),
        CeremonyError::BadIndex(3)
    );
}

#[test]
fn finish_before_complete_is_an_error() {
    let d = Ceremony::new(params(), 0, contribution(0)).unwrap();
    assert_eq!(d.finish().unwrap_err(), CeremonyError::WrongPhase);
}

#[test]
fn extra_source_is_xored_into_own_contribution_before_commit() {
    let base = contribution(0);
    let extra = contribution(7);
    let d = Ceremony::with_extra_source(params(), 0, base.clone(), &extra).unwrap();
    let mut mixed = base;
    for (m, e) in mixed.iter_mut().zip(&extra) {
        *m ^= e;
    }
    let plain = Ceremony::new(params(), 0, mixed).unwrap();
    assert_eq!(d.my_commitment(), plain.my_commitment());
}
