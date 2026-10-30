use cahoots_chip::Attested;
use cahoots_core::ledger::MemLedger;
use cahoots_core::pad::{MemPadStorage, Pad};
use cahoots_core::params::CircleParams;
use cahoots_custodian::wire::{
    ChipRequest, ChipResponse, ErrorCode, GrantRequestFields, Request, Response, SshHost,
    UnderGrantFields, MAX_SMALL_REQUEST_LEN,
};
use cahoots_custodian::{serve, AlwaysAllow, ChipApps, ChipInfo, CircleId, Consent, Custodian};

type P = Pad<MemPadStorage, MemLedger>;

fn pad() -> P {
    let params = CircleParams::new(3, 192, "vectors", false).unwrap();
    let bytes: Vec<u8> = (0..192u32).map(|i| (i * 7 + 3) as u8).collect();
    let slices = params.slices();
    Pad::new(params, 0, MemPadStorage::new(bytes), MemLedger::new(slices)).unwrap()
}

/// A pad with a fake chip: answers every operation, and refuses ssh data that does not
/// start with a 0x01 byte as unparseable.
struct Fake(P);

impl Custodian for Fake {
    fn circles(&self) -> Vec<cahoots_custodian::CircleInfo> {
        self.0.circles()
    }
    fn psk(
        &mut self,
        c: &CircleId,
        ctx: &[u8],
    ) -> Result<
        (cahoots_custodian::PskHandle, zeroize::Zeroizing<[u8; 32]>),
        cahoots_custodian::PadError,
    > {
        self.0.psk(c, ctx)
    }
    fn psk_for(
        &mut self,
        h: &cahoots_custodian::PskHandle,
        ctx: &[u8],
    ) -> Result<zeroize::Zeroizing<[u8; 32]>, cahoots_custodian::PadError> {
        self.0.psk_for(h, ctx)
    }
    fn spend(
        &mut self,
        c: &CircleId,
        n: u32,
    ) -> Result<
        (cahoots_custodian::PskHandle, zeroize::Zeroizing<Vec<u8>>),
        cahoots_custodian::PadError,
    > {
        self.0.spend(c, n)
    }
    fn spend_for(
        &mut self,
        h: &cahoots_custodian::PskHandle,
        n: u32,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, cahoots_custodian::PadError> {
        self.0.spend_for(h, n)
    }
    fn remaining(&self, c: &CircleId) -> u64 {
        Custodian::remaining(&self.0, c)
    }
    fn low_pad(&self, c: &CircleId) -> bool {
        Custodian::low_pad(&self.0, c)
    }
}

impl ChipApps for Fake {
    fn describe(&mut self, _: &CircleId) -> Result<ChipInfo, ErrorCode> {
        Ok(ChipInfo {
            public: vec![2; 33],
            curve: 1,
            in_chip: true,
            can_attest: true,
        })
    }
    fn attest(&mut self, _: &CircleId, f: &[u8; 16]) -> Result<Attested, ErrorCode> {
        Ok(Attested {
            public: vec![1],
            uid: vec![2],
            freshness: *f,
            timestamp: vec![3],
            signature: vec![4],
            attest_public: vec![5],
            attest_cert_der: vec![6],
        })
    }
    fn ssh_sign(
        &mut self,
        _: &CircleId,
        data: &[u8],
        _: Option<&SshHost>,
        _: Option<&[u8]>,
    ) -> Result<Vec<u8>, ErrorCode> {
        if data.first() == Some(&1) {
            Ok(vec![0xAA; 4])
        } else {
            Err(ErrorCode::Unparseable)
        }
    }
    fn card_sign(&mut self, _: &CircleId, _: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        Ok(vec![0xBB])
    }
    fn grant_sign(&mut self, _: &CircleId, _: &[u8]) -> Result<Vec<u8>, ErrorCode> {
        Ok(vec![0xCC])
    }
    fn seal_key(&mut self, _: &CircleId, _: &str) -> Result<[u8; 32], ErrorCode> {
        Ok([0xDD; 32])
    }
    fn nonce(&mut self, _: &CircleId) -> Result<[u8; 16], ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn grant_request(
        &mut self,
        _: &CircleId,
        _: &GrantRequestFields,
    ) -> Result<(Vec<u8>, u8), ErrorCode> {
        Err(ErrorCode::NoChip)
    }
    fn ssh_sign_under_grant(
        &mut self,
        _: &CircleId,
        _: &UnderGrantFields,
    ) -> Result<(Vec<u8>, u32), ErrorCode> {
        Err(ErrorCode::NoChip)
    }
}

#[derive(Default)]
struct Counting {
    asked: usize,
    say: bool,
}

impl Consent for Counting {
    fn allow(&mut self, _: &Request) -> bool {
        self.asked += 1;
        self.say
    }
}

fn ask<C: Consent>(c: &mut Fake, k: &mut C, op: ChipRequest) -> Response {
    let req = Request::Chip {
        circle: [1; 16],
        op,
    };
    Response::decode(&serve(c, k, &req.encode())).unwrap()
}

fn ssh(data: Vec<u8>, content: Option<Vec<u8>>) -> ChipRequest {
    ChipRequest::SshSign {
        data,
        host: None,
        content,
    }
}

#[test]
fn serve_asks_consent_for_signing_only() {
    let mut c = Fake(pad());
    let mut k = Counting {
        asked: 0,
        say: true,
    };
    for op in [
        ChipRequest::Describe,
        ChipRequest::Attest { freshness: [0; 16] },
    ] {
        assert!(matches!(ask(&mut c, &mut k, op), Response::Chip { .. }));
    }
    assert_eq!(k.asked, 0);
    let ops = [
        ssh(vec![1], None),
        ChipRequest::Card { card: vec![1] },
        ChipRequest::GrantSign { grant: vec![1] },
        ChipRequest::SealKey {
            label: "card".into(),
        },
    ];
    for (i, op) in ops.into_iter().enumerate() {
        assert!(matches!(ask(&mut c, &mut k, op), Response::Chip { .. }));
        assert_eq!(k.asked, i + 1);
    }
}

#[test]
fn refused_consent_is_denied() {
    let mut c = Fake(pad());
    let mut k = Counting {
        asked: 0,
        say: false,
    };
    for op in [
        ssh(vec![1], None),
        ChipRequest::Card { card: vec![1] },
        ChipRequest::GrantSign { grant: vec![1] },
        ChipRequest::SealKey { label: "x".into() },
    ] {
        assert_eq!(
            ask(&mut c, &mut k, op),
            Response::Error {
                code: ErrorCode::Denied
            }
        );
    }
    // Public material is still answered.
    assert!(matches!(
        ask(&mut c, &mut k, ChipRequest::Describe),
        Response::Chip {
            result: ChipResponse::Description { .. }
        }
    ));
}

#[test]
fn serve_refuses_unparseable_ssh() {
    let mut c = Fake(pad());
    assert_eq!(
        ask(&mut c, &mut AlwaysAllow, ssh(vec![0, 1, 2], None)),
        Response::Error {
            code: ErrorCode::Unparseable
        }
    );
    assert_eq!(
        ask(&mut c, &mut AlwaysAllow, ssh(vec![1], None)),
        Response::Chip {
            result: ChipResponse::Signature {
                signature: vec![0xAA; 4]
            }
        }
    );
}

#[test]
fn answers_map_to_the_wire() {
    let mut c = Fake(pad());
    let k = &mut AlwaysAllow;
    assert_eq!(
        ask(&mut c, k, ChipRequest::Card { card: vec![] }),
        Response::Chip {
            result: ChipResponse::Signature {
                signature: vec![0xBB]
            }
        }
    );
    assert_eq!(
        ask(
            &mut c,
            k,
            ChipRequest::SealKey {
                label: "card".into()
            }
        ),
        Response::Chip {
            result: ChipResponse::Key { key: [0xDD; 32] }
        }
    );
    match ask(&mut c, k, ChipRequest::Attest { freshness: [9; 16] }) {
        Response::Chip {
            result: ChipResponse::Attested { attested },
        } => {
            assert_eq!(attested.freshness, [9; 16]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_pad_without_a_chip_says_no_chip() {
    let mut p = pad();
    let req = Request::Chip {
        circle: [1; 16],
        op: ChipRequest::Describe,
    };
    let r = Response::decode(&serve(&mut p, &mut AlwaysAllow, &req.encode())).unwrap();
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::NoChip
        }
    );
}

#[test]
fn only_ssh_sign_may_be_long() {
    let mut c = Fake(pad());
    let big = vec![1u8; MAX_SMALL_REQUEST_LEN + 100];
    assert!(matches!(
        ask(
            &mut c,
            &mut AlwaysAllow,
            ssh(big.clone(), Some(big.clone()))
        ),
        Response::Chip { .. }
    ));
    assert_eq!(
        ask(
            &mut c,
            &mut AlwaysAllow,
            ChipRequest::Card { card: big.clone() }
        ),
        Response::Error {
            code: ErrorCode::Malformed
        }
    );
    let psk = Request::Psk {
        circle: [1; 16],
        context: big,
    };
    assert_eq!(
        Response::decode(&serve(&mut c, &mut AlwaysAllow, &psk.encode())).unwrap(),
        Response::Error {
            code: ErrorCode::Malformed
        }
    );
}

#[test]
fn a_pad_without_a_chip_says_no_chip_to_grant_requests() {
    let mut p = pad();
    for op in [
        ChipRequest::Nonce,
        ChipRequest::GrantRequest {
            action: "ssh-sign".into(),
            author: "ana".into(),
            location: None,
            agent: "claude".into(),
            budget: 1,
            exp: None,
            standing: false,
            cnf: [2; 33],
            time: 1,
        },
        ChipRequest::SshSignUnderGrant {
            data: vec![1],
            content: vec![],
            grant: vec![],
            proof: vec![],
            location: None,
            time: 1,
        },
    ] {
        let req = Request::Chip {
            circle: [1; 16],
            op,
        };
        let r = Response::decode(&serve(&mut p, &mut AlwaysAllow, &req.encode())).unwrap();
        assert_eq!(
            r,
            Response::Error {
                code: ErrorCode::NoChip
            }
        );
    }
}
