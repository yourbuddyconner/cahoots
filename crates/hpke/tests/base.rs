use cahoots_hpke::{gen_keypair, open_base, seal_base, HpkeError, Suite};

#[test]
fn xwing_base_seal_open_round_trip_and_aad_binding() {
    let mut rng = rand::rng();
    let (sk, pk) = gen_keypair(Suite::XWing, &mut rng);
    let (enc, ct) = seal_base(
        Suite::XWing,
        &pk,
        b"cahoots/deposit/v1",
        b"aad",
        b"epoch key",
        &mut rng,
    )
    .unwrap();
    assert_eq!(
        open_base(Suite::XWing, &sk, &enc, b"cahoots/deposit/v1", b"aad", &ct).unwrap(),
        b"epoch key"
    );
    assert_eq!(
        open_base(
            Suite::XWing,
            &sk,
            &enc,
            b"cahoots/deposit/v1",
            b"other",
            &ct
        ),
        Err(HpkeError::Open)
    );
}
