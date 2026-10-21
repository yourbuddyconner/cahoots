use cahoots_yk::health::{rct_cutoff, HealthCheck};
use cahoots_yk::{Card, OpenPgp, YkError};
use std::cell::RefCell;
use std::collections::VecDeque;

/// Scripted card: records every APDU and replays canned responses in order.
#[derive(Debug)]
struct FakeCard {
    responses: RefCell<VecDeque<Vec<u8>>>,
    sent: RefCell<Vec<Vec<u8>>>,
}

impl FakeCard {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        FakeCard {
            responses: RefCell::new(responses.into()),
            sent: RefCell::new(Vec::new()),
        }
    }
}

impl Card for FakeCard {
    fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, YkError> {
        self.sent.borrow_mut().push(apdu.to_vec());
        self.responses
            .borrow_mut()
            .pop_front()
            .ok_or(YkError::CardRemoved)
    }
}

fn ok(mut body: Vec<u8>) -> Vec<u8> {
    body.extend_from_slice(&[0x90, 0x00]);
    body
}

/// Application-related data (6E) wrapping extended capabilities (C0) with the given
/// max challenge length, plus an unrelated DO to make sure parsing skips it.
fn app_data(max_challenge: u16) -> Vec<u8> {
    let mut c0 = vec![0x7C, 0x00];
    c0.extend_from_slice(&max_challenge.to_be_bytes());
    c0.extend_from_slice(&[0x08, 0x00, 0x00, 0xFF, 0x01, 0x01]);
    let mut inner = vec![0x4F, 0x02, 0xAA, 0xBB]; // AID-ish filler
    inner.push(0xC0);
    inner.push(c0.len() as u8);
    inner.extend_from_slice(&c0);
    let mut out = vec![0x6E, inner.len() as u8];
    out.extend_from_slice(&inner);
    out
}

fn chunk(seed: u8, len: usize) -> Vec<u8> {
    // Period 65536 and no runs, so it never trips a health check by itself.
    (0..len as u32)
        .map(|i| ((i * 40503 + u32::from(seed) * 7919) >> 8) as u8)
        .collect()
}

#[test]
fn open_selects_openpgp_aid_and_reads_max_challenge_length() {
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(255))]);
    let pgp = OpenPgp::open(card).unwrap();
    assert_eq!(pgp.max_challenge(), 255);
    let sent = pgp.card().sent.borrow();
    assert_eq!(
        sent[0],
        [0x00, 0xA4, 0x04, 0x00, 0x06, 0xD2, 0x76, 0x00, 0x01, 0x24, 0x01, 0x00]
    );
    assert_eq!(sent[1], [0x00, 0xCA, 0x00, 0x6E, 0x00]);
}

#[test]
fn open_fails_when_select_status_is_not_9000() {
    let card = FakeCard::new(vec![vec![0x6A, 0x82]]);
    assert_eq!(OpenPgp::open(card).unwrap_err(), YkError::Status(0x6A82));
}

#[test]
fn open_fails_when_challenge_unsupported() {
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(0))]);
    assert_eq!(
        OpenPgp::open(card).unwrap_err(),
        YkError::NoChallengeSupport
    );
}

#[test]
fn random_requests_chunks_of_max_challenge_then_remainder() {
    let card = FakeCard::new(vec![
        ok(vec![]),
        ok(app_data(64)),
        ok(chunk(1, 64)),
        ok(chunk(2, 64)),
        ok(chunk(3, 10)),
    ]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 138];
    pgp.random(&mut buf).unwrap();
    assert_eq!(&buf[..64], chunk(1, 64).as_slice());
    assert_eq!(&buf[128..], chunk(3, 10).as_slice());
    let sent = pgp.card().sent.borrow();
    assert_eq!(sent[2], [0x00, 0x84, 0x00, 0x00, 64]);
    assert_eq!(sent[4], [0x00, 0x84, 0x00, 0x00, 10]);
}

#[test]
fn le_256_is_encoded_as_zero_and_larger_uses_extended_length() {
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(256)), ok(chunk(1, 256))]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 256];
    pgp.random(&mut buf).unwrap();
    assert_eq!(pgp.card().sent.borrow()[2], [0x00, 0x84, 0x00, 0x00, 0x00]);

    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(512)), ok(chunk(1, 300))]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 300];
    pgp.random(&mut buf).unwrap();
    assert_eq!(
        pgp.card().sent.borrow()[2],
        [0x00, 0x84, 0x00, 0x00, 0x00, 0x01, 0x2C]
    );
}

#[test]
fn short_read_fails_closed() {
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), ok(chunk(1, 40))]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(
        pgp.random(&mut buf).unwrap_err(),
        YkError::ShortRead { want: 64, got: 40 }
    );
}

#[test]
fn card_removed_mid_read_fails_closed() {
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), ok(chunk(1, 64))]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 128];
    assert_eq!(pgp.random(&mut buf).unwrap_err(), YkError::CardRemoved);
}

#[test]
fn error_status_mid_read_fails_closed() {
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), vec![0x69, 0x85]]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(pgp.random(&mut buf).unwrap_err(), YkError::Status(0x6985));
}

#[test]
fn get_response_chaining_is_followed() {
    // Card answers 61 0A: ten more bytes available via GET RESPONSE.
    let mut first = chunk(1, 54);
    first.extend_from_slice(&[0x61, 0x0A]);
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), first, ok(chunk(9, 10))]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    pgp.random(&mut buf).unwrap();
    assert_eq!(&buf[54..], chunk(9, 10).as_slice());
    assert_eq!(pgp.card().sent.borrow()[3], [0x00, 0xC0, 0x00, 0x00, 0x0A]);
}

#[test]
fn all_zero_block_is_rejected() {
    let mut data = chunk(1, 64);
    data[32..].fill(0);
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), ok(data)]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(
        pgp.random(&mut buf).unwrap_err(),
        YkError::Health(HealthCheck::ZeroBlock)
    );
}

#[test]
fn repeated_block_is_rejected_across_chunks() {
    let card = FakeCard::new(vec![
        ok(vec![]),
        ok(app_data(32)),
        ok(chunk(1, 32)),
        ok(chunk(1, 32)),
    ]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(
        pgp.random(&mut buf).unwrap_err(),
        YkError::Health(HealthCheck::RepeatedBlock)
    );
}

#[test]
fn repetition_count_test_rejects_stuck_bytes() {
    assert_eq!(rct_cutoff(), 5, "H=8 bits/sample, alpha=2^-30");
    let mut data = chunk(1, 64);
    data[10..15].copy_from_slice(&[0x5A; 5]);
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), ok(data)]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(
        pgp.random(&mut buf).unwrap_err(),
        YkError::Health(HealthCheck::RepetitionCount)
    );
    // Four in a row is under the cutoff.
    let mut data = chunk(1, 64);
    data[10..14].copy_from_slice(&[0x5A; 4]);
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(64)), ok(data)]);
    let mut pgp = OpenPgp::open(card).unwrap();
    pgp.random(&mut buf).unwrap();
}

#[test]
fn rct_state_carries_across_chunks() {
    let mut a = chunk(1, 32);
    a[29..].fill(0x77);
    let mut b = chunk(2, 32);
    b[..2].fill(0x77);
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(32)), ok(a), ok(b)]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(
        pgp.random(&mut buf).unwrap_err(),
        YkError::Health(HealthCheck::RepetitionCount)
    );
}

#[test]
fn implements_hal_entropy_and_maps_errors() {
    use cahoots_hal::{Entropy, HealthError};
    let card = FakeCard::new(vec![ok(vec![]), ok(app_data(32)), ok(chunk(1, 32))]);
    let mut pgp = OpenPgp::open(card).unwrap();
    let mut buf = [0u8; 32];
    Entropy::fill(&mut pgp, &mut buf).unwrap();
    let mut more = [0u8; 32];
    assert_eq!(
        Entropy::fill(&mut pgp, &mut more),
        Err(HealthError::SourceUnavailable)
    );
}

#[test]
fn chi_square_sanity_accepts_uniform_rejects_constant() {
    use cahoots_yk::health::chi_square_bytes;
    let uniform: Vec<u8> = (0..65536u32)
        .map(|i| ((i.wrapping_mul(2654435761)) >> 24) as u8)
        .collect();
    let x = chi_square_bytes(&uniform);
    assert!(x < 400.0, "{x}");
    let constant = vec![7u8; 65536];
    assert!(chi_square_bytes(&constant) > 10_000.0);
}
