use cahoots_chip::Attested;
use cahoots_custodian::wire::{
    ChipRequest, ChipResponse, ErrorCode, Request, Response, SshHost, KIND_REQUEST, KIND_RESPONSE,
    MAX_REQUEST_LEN, MAX_SMALL_REQUEST_LEN,
};
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
            chip: true,
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
        NoChip,
        ChipError,
        Unparseable,
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

#[test]
fn other_message_type_is_malformed() {
    let resp = Response::Identity {
        circles: vec![],
        tier: 0,
        name: "x".into(),
        chip: false,
    };
    assert_eq!(Request::decode(&resp.encode()), Err(ErrorCode::Malformed));
    let mut req = Request::Identify.encode();
    assert_eq!(Request::decode(&req), Ok(Request::Identify));
    req.push(0x00);
    assert_eq!(Request::decode(&req), Err(ErrorCode::Malformed));
    let mut r = Response::Key { key: [1; 32] }.encode();
    r.push(0x00);
    assert_eq!(Response::decode(&r), Err(ErrorCode::Malformed));
}

#[test]
fn wrong_lengths_are_malformed() {
    // PskFor [2, [bytes(20), bytes(0)]]: handle one byte short.
    let mut short_handle = vec![0x82, 0x02, 0x82, 0x54];
    short_handle.extend([0u8; 20]);
    short_handle.push(0x40);
    assert_eq!(Request::decode(&short_handle), Err(ErrorCode::Malformed));
    // Remaining [5, [bytes(15)]]: circle id one byte short.
    let mut short_circle = vec![0x82, 0x05, 0x81, 0x4F];
    short_circle.extend([0u8; 15]);
    assert_eq!(Request::decode(&short_circle), Err(ErrorCode::Malformed));
    // Key [2, [bytes(31)]]: key one byte short.
    let mut short_key = vec![0x82, 0x02, 0x81, 0x58, 0x1F];
    short_key.extend([0u8; 31]);
    assert_eq!(Response::decode(&short_key), Err(ErrorCode::Malformed));
}

#[test]
fn truncated_is_malformed() {
    let req = Request::Psk {
        circle: [1; 16],
        context: b"ctx".to_vec(),
    }
    .encode();
    assert_eq!(
        Request::decode(&req[..req.len() - 1]),
        Err(ErrorCode::Malformed)
    );
    let resp = Response::Bytes {
        handle: handle(),
        bytes: vec![1, 2, 3],
    }
    .encode();
    assert_eq!(
        Response::decode(&resp[..resp.len() - 1]),
        Err(ErrorCode::Malformed)
    );
}

fn host() -> SshHost {
    SshHost {
        host_key: vec![1, 2],
        session_id: vec![3],
        host_sig: vec![4],
        name: "h".into(),
        user: "u".into(),
    }
}

fn chip_requests() -> Vec<ChipRequest> {
    vec![
        ChipRequest::Describe,
        ChipRequest::Attest { freshness: [7; 16] },
        ChipRequest::SshSign {
            data: vec![9, 9],
            host: Some(host()),
            content: Some(vec![5]),
        },
        ChipRequest::SshSign {
            data: vec![],
            host: None,
            content: None,
        },
        ChipRequest::Card { card: vec![1] },
        ChipRequest::GrantSign { grant: vec![2] },
        ChipRequest::SealKey {
            label: "card".into(),
        },
        ChipRequest::Nonce,
        ChipRequest::GrantRequest {
            action: "ssh-sign".into(),
            author: "ana".into(),
            location: Some("github.com".into()),
            agent: "claude".into(),
            budget: 5,
            exp: Some(1_700_000_000),
            standing: true,
            cnf: [3; 33],
            time: 1_699_999_000,
        },
        ChipRequest::GrantRequest {
            action: "ssh-sign".into(),
            author: "ana".into(),
            location: None,
            agent: "claude".into(),
            budget: 0,
            exp: None,
            standing: false,
            cnf: [0; 33],
            time: 0,
        },
        ChipRequest::SshSignUnderGrant {
            data: vec![9, 9],
            content: vec![5],
            grant: vec![6, 6],
            proof: vec![7],
            location: Some("github.com".into()),
            time: 42,
        },
        ChipRequest::SshSignUnderGrant {
            data: vec![],
            content: vec![],
            grant: vec![],
            proof: vec![],
            location: None,
            time: 0,
        },
    ]
}

#[test]
fn grant_requests_decode_strictly() {
    let r = Request::Chip {
        circle: [4; 16],
        op: ChipRequest::Nonce,
    };
    let mut bytes = r.encode();
    bytes.push(0);
    assert_eq!(Request::decode(&bytes), Err(ErrorCode::Malformed));
    // A key of the wrong length is not a request.
    let short = Request::Chip {
        circle: [4; 16],
        op: ChipRequest::GrantRequest {
            action: "a".into(),
            author: "b".into(),
            location: None,
            agent: "c".into(),
            budget: 1,
            exp: None,
            standing: false,
            cnf: [1; 33],
            time: 1,
        },
    }
    .encode();
    let at = short.windows(2).position(|w| w == [0x58, 33]).unwrap();
    let mut bad = short.clone();
    bad[at + 1] = 32;
    bad.remove(at + 2);
    assert_eq!(Request::decode(&bad), Err(ErrorCode::Malformed));
}

#[test]
fn grant_ops_consent_and_length() {
    let nonce = ChipRequest::Nonce;
    assert!(!nonce.needs_consent());
    let ask = chip_requests()
        .into_iter()
        .find(|o| matches!(o, ChipRequest::GrantRequest { .. }))
        .unwrap();
    assert!(ask.needs_consent());
    let under = chip_requests()
        .into_iter()
        .find(|o| matches!(o, ChipRequest::SshSignUnderGrant { .. }))
        .unwrap();
    assert!(!under.needs_consent());
    let long = |op| Request::Chip {
        circle: [0; 16],
        op,
    };
    let under = long(under);
    assert!(under.length_ok(MAX_REQUEST_LEN));
    assert!(!under.length_ok(MAX_REQUEST_LEN + 1));
    assert!(!long(ChipRequest::Nonce).length_ok(MAX_SMALL_REQUEST_LEN + 1));
}

#[test]
fn chip_requests_round_trip() {
    for op in chip_requests() {
        let r = Request::Chip {
            circle: [4; 16],
            op,
        };
        assert_eq!(Request::decode(&r.encode()), Ok(r));
    }
}

#[test]
fn chip_responses_round_trip() {
    let attested = Attested {
        public: vec![1],
        uid: vec![2],
        freshness: [3; 16],
        timestamp: vec![4],
        signature: vec![5],
        attest_public: vec![6],
        attest_cert_der: vec![7],
    };
    for result in [
        ChipResponse::Description {
            public: vec![2; 33],
            curve: 1,
            in_chip: true,
            can_attest: false,
        },
        ChipResponse::Attested { attested },
        ChipResponse::Signature {
            signature: vec![8; 3],
        },
        ChipResponse::Key { key: [6; 32] },
        ChipResponse::Nonce { nonce: [9; 16] },
        ChipResponse::Grant {
            grant: vec![1, 2, 3],
            slot: 4,
        },
        ChipResponse::GrantSignature {
            signature: vec![8; 5],
            remaining: 3,
        },
    ] {
        let r = Response::Chip { result };
        assert_eq!(Response::decode(&r.encode()), Ok(r));
    }
}

#[test]
fn chip_wire_bytes_are_fixed() {
    // Each vector was produced once, read by hand as canonical CBOR (definite lengths,
    // shortest integers, trailing absent options omitted) and pasted.
    let h = |v: Vec<u8>| v.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let pre = format!("82068250{}", "04".repeat(16));
    let req = |op| {
        h(Request::Chip {
            circle: [4; 16],
            op,
        }
        .encode())
    };
    assert_eq!(req(ChipRequest::Describe), format!("{pre}820080"));
    assert_eq!(
        req(ChipRequest::Attest { freshness: [7; 16] }),
        format!("{pre}8201815007070707070707070707070707070707")
    );
    assert_eq!(
        req(ChipRequest::SshSign {
            data: vec![9, 9],
            host: None,
            content: None
        }),
        format!("{pre}820281420909")
    );
    assert_eq!(
        req(ChipRequest::SshSign {
            data: vec![9, 9],
            host: Some(host()),
            content: Some(vec![5])
        }),
        format!("{pre}8202834209098542010241034104616861754105")
    );
    assert_eq!(
        req(ChipRequest::Card { card: vec![1] }),
        format!("{pre}8203814101")
    );
    assert_eq!(
        req(ChipRequest::GrantSign { grant: vec![2] }),
        format!("{pre}8204814102")
    );
    assert_eq!(
        req(ChipRequest::SealKey {
            label: "card".into()
        }),
        format!("{pre}8205816463617264")
    );
    let resp = |result| h(Response::Chip { result }.encode());
    assert_eq!(
        resp(ChipResponse::Signature {
            signature: vec![8; 3]
        }),
        "82068182028143080808"
    );
    assert_eq!(
        resp(ChipResponse::Key { key: [6; 32] }),
        format!("8206818203815820{}", "06".repeat(32))
    );
    assert_eq!(
        resp(ChipResponse::Description {
            public: vec![2; 3],
            curve: 1,
            in_chip: true,
            can_attest: false
        }),
        "8206818200844302020201f5f4"
    );
    assert_eq!(
        h(Response::Error {
            code: ErrorCode::NoChip
        }
        .encode()),
        "82058109"
    );
    assert_eq!(
        h(Response::Error {
            code: ErrorCode::Unparseable
        }
        .encode()),
        "8205810b"
    );
    assert_eq!(
        h(Response::Identity {
            circles: vec![],
            tier: 1,
            name: "b".into(),
            chip: true
        }
        .encode()),
        "82008480016162f5"
    );
}

#[test]
fn identity_has_chip_flag() {
    for chip in [false, true] {
        let r = Response::Identity {
            circles: vec![],
            tier: 1,
            name: "b".into(),
            chip,
        };
        let enc = r.encode();
        assert_eq!(*enc.last().unwrap(), if chip { 0xF5 } else { 0xF4 });
        assert_eq!(Response::decode(&enc), Ok(r));
    }
}

#[test]
fn old_indices_unchanged() {
    // Vectors from before the family: Identify, Remaining, and the older error codes.
    assert_eq!(Request::decode(&[0x82, 0x00, 0x80]), Ok(Request::Identify));
    let mut rem = vec![0x82, 0x05, 0x81, 0x50];
    rem.extend([9u8; 16]);
    assert_eq!(
        Request::decode(&rem),
        Ok(Request::Remaining { circle: [9; 16] })
    );
    for (i, code) in [
        ErrorCode::UnknownCircle,
        ErrorCode::Exhausted,
        ErrorCode::AlreadySpent,
        ErrorCode::OutOfRange,
        ErrorCode::Config,
        ErrorCode::Storage,
        ErrorCode::Malformed,
        ErrorCode::Denied,
        ErrorCode::Busy,
        ErrorCode::NoChip,
        ErrorCode::ChipError,
        ErrorCode::Unparseable,
    ]
    .into_iter()
    .enumerate()
    {
        let enc = Response::Error { code }.encode();
        assert_eq!(enc[..3], [0x82, 0x05, 0x81]);
        assert_eq!(Response::decode(&enc), Ok(Response::Error { code }));
        assert_eq!(enc.len(), 4);
        assert_eq!(enc[3] as usize, i);
    }
}

#[test]
fn decode_cap_is_sixteen_kib() {
    let big = |n| {
        Request::Chip {
            circle: [0; 16],
            op: ChipRequest::SshSign {
                data: vec![0; n],
                host: None,
                content: None,
            },
        }
        .encode()
    };
    assert!(Request::decode(&big(MAX_REQUEST_LEN - 64)).is_ok());
    assert_eq!(
        Request::decode(&big(MAX_REQUEST_LEN)),
        Err(ErrorCode::Malformed)
    );
}
