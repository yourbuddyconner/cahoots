use cahoots_custodian::wire::{ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE};
use cahoots_custodian::{CircleInfo, PskHandle};

fn handle() -> PskHandle {
    PskHandle {
        circle_id: [7; 16],
        slice: 2,
        offset: 0x0102_0304,
    }
}

#[test]
fn every_request_round_trips() {
    let all = [
        Request::Identify,
        Request::Psk {
            circle: [1; 16],
            context: b"wireguard".to_vec(),
        },
        Request::PskFor {
            handle: handle(),
            context: vec![],
        },
        Request::Spend {
            circle: [3; 16],
            len: 4096,
        },
        Request::SpendFor {
            handle: handle(),
            len: 17,
        },
        Request::Remaining { circle: [9; 16] },
    ];
    for r in all {
        assert_eq!(Request::decode(&r.encode()), Ok(r));
    }
}

#[test]
fn every_response_round_trips() {
    let mut all = vec![
        Response::Identity {
            circles: vec![CircleInfo {
                circle_id: [5; 16],
                seat: 1,
                n: 3,
                name: "trail crew".into(),
                tier: 2,
            }],
            tier: 2,
            name: "badge".into(),
        },
        Response::Psk {
            handle: handle(),
            key: [0xAB; 32],
        },
        Response::Key { key: [0xCD; 32] },
        Response::Bytes {
            handle: handle(),
            bytes: vec![1, 2, 3, 250],
        },
        Response::Remaining {
            bytes: u64::MAX,
            low: true,
        },
    ];
    use ErrorCode::*;
    for code in [
        UnknownCircle,
        Exhausted,
        AlreadySpent,
        OutOfRange,
        Config,
        Storage,
        Malformed,
        Denied,
        Busy,
    ] {
        all.push(Response::Error { code });
    }
    for r in all {
        assert_eq!(Response::decode(&r.encode()), Ok(r));
    }
}

#[test]
fn garbage_is_malformed() {
    assert_eq!(Request::decode(&[0xFF, 0x00]), Err(ErrorCode::Malformed));
    assert_eq!(Request::decode(&[]), Err(ErrorCode::Malformed));
    assert_eq!(Response::decode(&[0xFF, 0x00]), Err(ErrorCode::Malformed));
}

#[test]
fn identify_is_small() {
    assert!(Request::Identify.encode().len() <= 4);
}

#[test]
fn keys_are_byte_strings_and_kinds_are_fixed() {
    let enc = Response::Key { key: [0; 32] }.encode();
    // array(2), variant 2, array(1) of fields, byte string of 32 (0x58 0x20)
    assert_eq!(&enc[..5], &[0x82, 0x02, 0x81, 0x58, 0x20]);
    assert_eq!((KIND_REQUEST, KIND_RESPONSE), (40, 41));
}

#[test]
fn pad_errors_map_to_codes() {
    use cahoots_custodian::PadError;
    assert_eq!(
        ErrorCode::from(PadError::Storage("x".into())),
        ErrorCode::Storage
    );
    assert_eq!(ErrorCode::from(PadError::Exhausted), ErrorCode::Exhausted);
}
