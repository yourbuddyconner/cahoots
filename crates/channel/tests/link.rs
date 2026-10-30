use cahoots_channel::link::{Link, LinkSecret, BASE};

fn link(secret: bool) -> Link {
    Link {
        relay: "relay.cahoots.network".into(),
        channel_id: [1; 16],
        secret: secret.then_some(LinkSecret([9; 32])),
    }
}

#[test]
fn url_round_trips_with_and_without_a_secret() {
    for with in [true, false] {
        let l = link(with);
        let url = l.to_url();
        assert!(url.starts_with(&format!("{BASE}#/ch/relay.cahoots.network/01010101")));
        assert!(!url.ends_with('/'));
        assert_eq!(Link::parse(&url), Some(l.clone()));
        assert_eq!(Link::parse(&url[BASE.len()..]), Some(l));
    }
    let mut l = link(true);
    l.relay = "localhost:8787".into();
    assert_eq!(Link::parse(&l.to_url()), Some(l));
    // Bytes that use every base64url symbol class survive the encoder.
    let l = Link {
        secret: Some(LinkSecret(core::array::from_fn(|i| (i * 37 + 250) as u8))),
        ..link(true)
    };
    assert_eq!(Link::parse(&l.to_url()), Some(l));
}

#[test]
fn id_seal_and_gate_differ_and_are_deterministic() {
    let s = LinkSecret([9; 32]);
    let c = [1u8; 16];
    assert_eq!(s.id(), LinkSecret([9; 32]).id());
    assert_ne!(*s.seal_key(&c), s.gate_token(&c));
    assert_eq!(*s.seal_key(&c), *s.seal_key(&c));
    assert_eq!(s.gate_token(&c), s.gate_token(&c));
    assert_ne!(*s.seal_key(&c), *s.seal_key(&[2; 16]));
    assert_ne!(s.gate_token(&c), s.gate_token(&[2; 16]));
    assert_ne!(s.id(), LinkSecret([8; 32]).id());
    assert_ne!(s.id(), s.gate_token(&c));
}

#[test]
fn parse_refuses_other_routes_and_bad_lengths() {
    let good = link(true).to_url();
    let frag = &good[BASE.len()..];
    let id = "01010101010101010101010101010101";
    assert!(Link::parse(&frag.replace("#/ch/", "#/c/")).is_none());
    assert!(Link::parse(&format!("#/ch/https://relay/{id}")).is_none());
    assert!(Link::parse(&format!("#/ch/https:relay/{id}")).is_none());
    assert!(Link::parse(&frag[..frag.len() - 3]).is_none()); // short secret
    assert!(Link::parse(&format!("{frag}AA")).is_none()); // long secret
    assert!(Link::parse(&format!("{frag}/")).is_none());
    assert!(Link::parse("#/ch/relay/0101").is_none()); // short id
    assert!(Link::parse(&format!("#/ch//{id}")).is_none());
    assert!(Link::parse(&format!("https://example.com/#/ch/relay/{id}")).is_none());
}
