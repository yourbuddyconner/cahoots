use cahoots_core::params::CircleParams;
use cahoots_core::reveal::{self, RevealError, CIPHERTEXT_LEN, PUBLIC_KEY_LEN, WRAP_LEN};

fn params() -> CircleParams {
    CircleParams::new(4, 256, "seal", false).unwrap()
}

fn seed(i: u8) -> [u8; 32] {
    [i; 32]
}

fn filler(tag: u8) -> impl FnMut(&mut [u8]) {
    let mut x = 0xD1B5_4A32_D192_ED03u64 ^ u64::from(tag);
    move |buf: &mut [u8]| {
        for b in buf.iter_mut() {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (x >> 56) as u8;
        }
    }
}

#[test]
fn keypair_is_deterministic_and_public_key_parses() {
    let (_, a) = reveal::keypair(&seed(1));
    let (_, b) = reveal::keypair(&seed(1));
    let (_, c) = reveal::keypair(&seed(2));
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), PUBLIC_KEY_LEN);
    reveal::parse_public_key(&a).unwrap();
    assert_eq!(
        reveal::parse_public_key(&a[..100]).unwrap_err(),
        RevealError::Malformed
    );
}

#[test]
fn every_recipient_opens_and_nobody_else() {
    let keys: Vec<_> = (0..4u8).map(|i| reveal::keypair(&seed(i))).collect();
    let contribution: Vec<u8> = (0..256u32).map(|k| (k * 7 % 251) as u8).collect();
    let recipients: Vec<(u8, &[u8])> = [0u8, 2, 3]
        .iter()
        .map(|&j| (j, keys[j as usize].1.as_slice()))
        .collect();
    let blob = reveal::seal(
        &params(),
        1,
        &keys[1].1,
        &recipients,
        &contribution,
        &mut filler(1),
    )
    .unwrap();
    let expected_len = 1 + 1 + PUBLIC_KEY_LEN + 1 + 3 * (1 + CIPHERTEXT_LEN + WRAP_LEN) + 256 + 16;
    assert_eq!(blob.len(), expected_len);
    assert_eq!(reveal::sender(&blob), Some(1));
    for &j in &[0u8, 2, 3] {
        let o = reveal::open(&params(), j, &keys[j as usize].0, &blob).unwrap();
        assert_eq!(o.from, 1);
        assert_eq!(o.public_key, keys[1].1);
        assert_eq!(o.contribution.as_slice(), contribution.as_slice());
    }
    assert_eq!(
        reveal::open(&params(), 1, &keys[1].0, &blob).unwrap_err(),
        RevealError::NotForMe
    );
    // Right index, wrong key: the wrap fails to open.
    let (stranger, _) = reveal::keypair(&seed(99));
    assert_eq!(
        reveal::open(&params(), 0, &stranger, &blob).unwrap_err(),
        RevealError::Corrupted
    );
}

#[test]
fn any_flipped_byte_is_corrupted_not_a_different_contribution() {
    let keys: Vec<_> = (0..2u8).map(|i| reveal::keypair(&seed(i))).collect();
    let contribution = vec![0xA5u8; 256];
    let recipients: Vec<(u8, &[u8])> = vec![(0, &keys[0].1)];
    let blob = reveal::seal(
        &CircleParams::new(2, 256, "seal", false).unwrap(),
        1,
        &keys[1].1,
        &recipients,
        &contribution,
        &mut filler(2),
    )
    .unwrap();
    let p = CircleParams::new(2, 256, "seal", false).unwrap();
    for pos in [
        3usize,
        2 + PUBLIC_KEY_LEN + 5,
        blob.len() - 200,
        blob.len() - 1,
    ] {
        let mut bad = blob.clone();
        bad[pos] ^= 0x80;
        let r = reveal::open(&p, 0, &keys[0].0, &bad);
        assert!(
            matches!(r, Err(RevealError::Corrupted) | Err(RevealError::Malformed)),
            "byte {pos}: {r:?}"
        );
    }
    // Different parameters are different associated data.
    let other = CircleParams::new(2, 256, "other", false).unwrap();
    assert_eq!(
        reveal::open(&other, 0, &keys[0].0, &blob).unwrap_err(),
        RevealError::Corrupted
    );
}

#[test]
fn randomness_budget_matches_what_seal_draws() {
    let keys: Vec<_> = (0..4u8).map(|i| reveal::keypair(&seed(i))).collect();
    let recipients: Vec<(u8, &[u8])> = (1..4u8)
        .map(|j| (j, keys[j as usize].1.as_slice()))
        .collect();
    let mut drawn = 0usize;
    let mut f = |buf: &mut [u8]| {
        drawn += buf.len();
        buf.fill(0x42);
    };
    reveal::seal(&params(), 0, &keys[0].1, &recipients, &[0u8; 256], &mut f).unwrap();
    assert_eq!(drawn, reveal::randomness_len(4));
}
