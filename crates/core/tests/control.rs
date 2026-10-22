use cahoots_core::control::{Control, ControlError, Proto, Purpose};

#[test]
fn peer_round_trips_and_has_the_documented_layout() {
    let c = Control::Peer {
        proto: Proto::WireGuard,
        public_key: vec![0xAB; 32],
        endpoint: "203.0.113.7:51820".into(),
        allowed_ips: "10.1.0.1/32".into(),
    };
    let bytes = c.encode().unwrap();
    assert_eq!(bytes[0], 0, "subtype peer");
    assert_eq!(bytes[1], 0, "proto wireguard");
    assert_eq!(bytes[2], 32, "pubkey_len");
    assert_eq!(&bytes[3..35], &[0xAB; 32]);
    assert_eq!(bytes[35], 17, "endpoint_len");
    assert_eq!(&bytes[36..53], b"203.0.113.7:51820");
    assert_eq!(bytes[53], 11, "allowed_len");
    assert_eq!(&bytes[54..], b"10.1.0.1/32");
    assert_eq!(Control::decode(&bytes).unwrap(), c);
}

#[test]
fn psk_and_rotate_round_trip_little_endian() {
    let p = Control::Psk {
        purpose: Purpose::WireGuard,
        peer_a: 0,
        peer_b: 3,
        offset: 0x0102_0304,
    };
    let b = p.encode().unwrap();
    assert_eq!(b, vec![1, 0, 0, 3, 0x04, 0x03, 0x02, 0x01]);
    assert_eq!(Control::decode(&b).unwrap(), p);
    let r = Control::Rotate {
        purpose: Purpose::Tls,
        peer_a: 2,
        peer_b: 1,
        not_after: 1_760_000_000,
    };
    let b = r.encode().unwrap();
    assert_eq!(b.len(), 12);
    assert_eq!(b[..4], [2, 1, 2, 1]);
    assert_eq!(Control::decode(&b).unwrap(), r);
    let s = Control::SliceTransfer { member: 5 };
    assert_eq!(Control::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn malformed_inputs_are_refused() {
    assert_eq!(Control::decode(&[]).unwrap_err(), ControlError::Length);
    assert_eq!(
        Control::decode(&[9, 0]).unwrap_err(),
        ControlError::Subtype(9)
    );
    assert_eq!(
        Control::decode(&[1, 0, 0, 1]).unwrap_err(),
        ControlError::Length,
        "psk too short"
    );
    assert_eq!(
        Control::decode(&[1, 0, 0, 1, 0, 0, 0, 0, 0xFF]).unwrap_err(),
        ControlError::Length,
        "trailing byte"
    );
    assert_eq!(
        Control::decode(&[1, 7, 0, 1, 0, 0, 0, 0]).unwrap_err(),
        ControlError::Value,
        "unknown purpose"
    );
    assert_eq!(
        Control::decode(&[0, 4, 0, 0, 0]).unwrap_err(),
        ControlError::Value,
        "unknown proto"
    );
    assert_eq!(
        Control::decode(&[0, 0, 5, 1, 2]).unwrap_err(),
        ControlError::Length,
        "pubkey shorter than its length byte"
    );
    assert_eq!(
        Control::decode(&[0, 0, 0, 2, 0xFF, 0xFE, 0]).unwrap_err(),
        ControlError::Utf8
    );
    let too_long = Control::Peer {
        proto: Proto::WireGuard,
        public_key: vec![0; 300],
        endpoint: String::new(),
        allowed_ips: String::new(),
    };
    assert_eq!(too_long.encode().unwrap_err(), ControlError::Value);
}

#[test]
fn psk_context_binds_purpose_and_pair() {
    assert_eq!(Purpose::WireGuard.context(0, 1), [0, 0, 1]);
    assert_ne!(Purpose::WireGuard.context(0, 1), Purpose::Tls.context(0, 1));
    assert_ne!(
        Purpose::WireGuard.context(0, 1),
        Purpose::WireGuard.context(1, 0)
    );
}
