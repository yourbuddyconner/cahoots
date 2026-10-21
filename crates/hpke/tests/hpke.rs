use cahoots_core::params::CircleParams;
use cahoots_hpke::{gen_keypair, open_from, seal_to, HpkeError, Suite};
use cahoots_provider::{MemProvider, PadError, PadProvider, PskHandle};

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(71) ^ (i >> 2)) as u8)
        .collect()
}

fn providers(pad: Vec<u8>) -> (MemProvider, MemProvider, [u8; 16]) {
    let params = CircleParams::new(2, 4096, "hpke", false).unwrap();
    let mut a = MemProvider::new();
    let mut b = MemProvider::new();
    let cid = a.add_circle(params.clone(), 0, pad.clone()).unwrap();
    b.add_circle(params, 1, pad).unwrap();
    (a, b, cid)
}

#[test]
fn roundtrip_x25519_and_xwing() {
    for suite in [Suite::X25519, Suite::XWing] {
        let (mut a, mut b, cid) = providers(pad_bytes(4096));
        let (sk, pk) = gen_keypair(suite, &mut rand::rng());
        let sealed = seal_to(
            &mut a,
            &cid,
            suite,
            &pk,
            b"info",
            b"aad",
            b"secret",
            &mut rand::rng(),
        )
        .unwrap();
        assert_eq!(
            sealed.handle,
            PskHandle {
                circle_id: cid,
                slice: 0,
                offset: 0
            }
        );
        let pt = open_from(
            &mut b,
            &sealed.handle,
            suite,
            &sk,
            &sealed.enc,
            b"info",
            b"aad",
            &sealed.ciphertext,
        )
        .unwrap();
        assert_eq!(pt, b"secret");
        assert_eq!(a.remaining(&cid), 2048 - 32);
        assert_eq!(b.remaining(&cid), 2048);
    }
}

#[test]
fn replayed_handle_is_refused() {
    let (mut a, mut b, cid) = providers(pad_bytes(4096));
    let (sk, pk) = gen_keypair(Suite::X25519, &mut rand::rng());
    let s = seal_to(
        &mut a,
        &cid,
        Suite::X25519,
        &pk,
        b"",
        b"",
        b"x",
        &mut rand::rng(),
    )
    .unwrap();
    open_from(
        &mut b,
        &s.handle,
        Suite::X25519,
        &sk,
        &s.enc,
        b"",
        b"",
        &s.ciphertext,
    )
    .unwrap();
    let err = open_from(
        &mut b,
        &s.handle,
        Suite::X25519,
        &sk,
        &s.enc,
        b"",
        b"",
        &s.ciphertext,
    )
    .unwrap_err();
    assert_eq!(err, HpkeError::Pad(PadError::AlreadySpent));
}

#[test]
fn wrong_pad_fails_to_open() {
    let (mut a, _, cid) = providers(pad_bytes(4096));
    let mut other = pad_bytes(4096);
    other[10] ^= 1; // inside slice 0's first 32 bytes
    let params = CircleParams::new(2, 4096, "hpke", false).unwrap();
    let mut c = MemProvider::new();
    let fake_cid = c.add_circle(params, 1, other).unwrap();
    assert_ne!(fake_cid, cid);
    let (sk, pk) = gen_keypair(Suite::X25519, &mut rand::rng());
    let s = seal_to(
        &mut a,
        &cid,
        Suite::X25519,
        &pk,
        b"",
        b"",
        b"x",
        &mut rand::rng(),
    )
    .unwrap();
    let handle = PskHandle {
        circle_id: fake_cid,
        ..s.handle
    };
    let err = open_from(
        &mut c,
        &handle,
        Suite::X25519,
        &sk,
        &s.enc,
        b"",
        b"",
        &s.ciphertext,
    )
    .unwrap_err();
    assert_eq!(err, HpkeError::Open);
}

#[test]
fn wrong_aad_fails_and_range_stays_spent() {
    let (mut a, mut b, cid) = providers(pad_bytes(4096));
    let (sk, pk) = gen_keypair(Suite::X25519, &mut rand::rng());
    let s = seal_to(
        &mut a,
        &cid,
        Suite::X25519,
        &pk,
        b"i",
        b"a",
        b"x",
        &mut rand::rng(),
    )
    .unwrap();
    let err = open_from(
        &mut b,
        &s.handle,
        Suite::X25519,
        &sk,
        &s.enc,
        b"i",
        b"b",
        &s.ciphertext,
    )
    .unwrap_err();
    assert_eq!(err, HpkeError::Open);
    // The receiver spent the range before trying to open. A retry is refused.
    let err = open_from(
        &mut b,
        &s.handle,
        Suite::X25519,
        &sk,
        &s.enc,
        b"i",
        b"a",
        &s.ciphertext,
    )
    .unwrap_err();
    assert_eq!(err, HpkeError::Pad(PadError::AlreadySpent));
}

#[test]
fn psk_id_is_handle_bytes() {
    let h = PskHandle {
        circle_id: [1; 16],
        slice: 2,
        offset: 258,
    };
    assert_eq!(cahoots_hpke::psk_id(&h).len(), 21);
    assert_eq!(cahoots_hpke::psk_id(&h)[16], 2);
    assert_eq!(&cahoots_hpke::psk_id(&h)[17..], &[2, 1, 0, 0]);
}
