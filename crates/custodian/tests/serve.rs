use cahoots_core::ledger::MemLedger;
use cahoots_core::pad::{MemPadStorage, Pad};
use cahoots_core::params::CircleParams;
use cahoots_custodian::wire::{ErrorCode, Request, Response};
use cahoots_custodian::{serve, AlwaysAllow, Consent, Custodian};

fn fixture() -> Pad<MemPadStorage, MemLedger> {
    let params = CircleParams::new(3, 192, "vectors", false).unwrap();
    let bytes: Vec<u8> = (0..192u32).map(|i| (i * 7 + 3) as u8).collect();
    let slices = params.slices();
    Pad::new(params, 0, MemPadStorage::new(bytes), MemLedger::new(slices)).unwrap()
}

fn decode(bytes: Vec<u8>) -> Response {
    Response::decode(&bytes).unwrap()
}

fn ask(pad: &mut Pad<MemPadStorage, MemLedger>, req: &Request) -> Response {
    decode(serve(pad, &mut AlwaysAllow, &req.encode()))
}

#[test]
fn identify_lists_the_circle() {
    let mut pad = fixture();
    match ask(&mut pad, &Request::Identify) {
        Response::Identity {
            circles,
            tier,
            name,
            chip,
        } => {
            assert_eq!(circles.len(), 1);
            assert_eq!(circles[0].name, "vectors");
            assert_eq!(tier, 0);
            assert_eq!(name, "");
            assert!(!chip);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn psk_spends_and_returns_a_handle() {
    let mut pad = fixture();
    let id = pad.circles()[0].circle_id;
    let before = Custodian::remaining(&pad, &id);
    let req = Request::Psk {
        circle: id,
        context: b"t".to_vec(),
    };
    let h1 = match ask(&mut pad, &req) {
        Response::Psk { handle, .. } => handle,
        other => panic!("{other:?}"),
    };
    assert_eq!(Custodian::remaining(&pad, &id), before - 32);
    let h2 = match ask(&mut pad, &req) {
        Response::Psk { handle, .. } => handle,
        other => panic!("{other:?}"),
    };
    assert_ne!(h1.offset, h2.offset);
}

#[test]
fn unknown_circle_is_an_error_and_spends_nothing() {
    let mut pad = fixture();
    let id = pad.circles()[0].circle_id;
    let before = Custodian::remaining(&pad, &id);
    let r = ask(
        &mut pad,
        &Request::Psk {
            circle: [9u8; 16],
            context: vec![],
        },
    );
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::UnknownCircle
        }
    );
    assert_eq!(Custodian::remaining(&pad, &id), before);
}

#[test]
fn malformed_bytes_are_an_error() {
    let mut pad = fixture();
    assert_eq!(
        decode(serve(&mut pad, &mut AlwaysAllow, &[0xFF])),
        Response::Error {
            code: ErrorCode::Malformed
        }
    );
}

#[test]
fn denied_consent_spends_nothing() {
    struct Never;
    impl Consent for Never {
        fn allow(&mut self, _: &Request) -> bool {
            false
        }
    }
    let mut pad = fixture();
    let id = pad.circles()[0].circle_id;
    let before = Custodian::remaining(&pad, &id);
    let req = Request::Spend {
        circle: id,
        len: 16,
    };
    let r = decode(serve(&mut pad, &mut Never, &req.encode()));
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::Denied
        }
    );
    assert_eq!(Custodian::remaining(&pad, &id), before);
    // Reads never ask.
    let r = decode(serve(&mut pad, &mut Never, &Request::Identify.encode()));
    assert!(matches!(r, Response::Identity { .. }));
    let r = decode(serve(
        &mut pad,
        &mut Never,
        &Request::Remaining { circle: id }.encode(),
    ));
    assert!(matches!(r, Response::Remaining { .. }));
}

#[test]
fn spend_past_the_slice_is_exhausted() {
    let mut pad = fixture();
    let id = pad.circles()[0].circle_id;
    let before = Custodian::remaining(&pad, &id);
    let r = ask(
        &mut pad,
        &Request::Spend {
            circle: id,
            len: before as u32 + 1,
        },
    );
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::Exhausted
        }
    );
    assert_eq!(Custodian::remaining(&pad, &id), before);
}
