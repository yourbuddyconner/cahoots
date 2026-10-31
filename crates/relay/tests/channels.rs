//! The channel API: `MemRelay` for every object kind, and `HttpRelay` against a real station.

use cahoots_relay::{Auth, ChannelRelay, HttpRelay, MemRelay, ObjectKind, Relay, RelayError};
use std::sync::{Arc, Mutex};

const CH: [u8; 16] = [7; 16];

#[test]
fn every_kind_round_trips_in_memory() {
    let mut r = MemRelay::default();
    for (i, kind) in [
        ObjectKind::Channel,
        ObjectKind::Policy,
        ObjectKind::Grant,
        ObjectKind::Revocation,
        ObjectKind::Deposit,
        ObjectKind::Request,
        ObjectKind::Gate,
    ]
    .into_iter()
    .enumerate()
    {
        assert!(r.list(&CH, kind, &[], &Auth::None).unwrap().is_empty());
        let body = vec![i as u8; 5];
        r.put_object(&CH, kind, &body).unwrap();
        assert_eq!(r.list(&CH, kind, &[], &Auth::None).unwrap(), vec![body]);
        assert!(r.list(&[8; 16], kind, &[], &Auth::None).unwrap().is_empty());
    }
}

#[test]
fn manifests_and_shared_handles() {
    let shared = Arc::new(Mutex::new(MemRelay::default()));
    let mut a = MemRelay::shared(shared.clone());
    let mut b = MemRelay::shared(shared);
    assert_eq!(b.get_manifest(&CH).unwrap(), None);
    a.put_manifest(&CH, b"sealed").unwrap();
    assert_eq!(b.get_manifest(&CH).unwrap(), Some(b"sealed".to_vec()));
    a.put_object(&CH, ObjectKind::Request, b"hi").unwrap();
    assert_eq!(
        b.list(&CH, ObjectKind::Request, &[], &Auth::None).unwrap(),
        vec![b"hi".to_vec()]
    );
}

#[test]
fn channel_post_and_poll() {
    let mut r = MemRelay::default();
    let g = Auth::Gate([1; 32]);
    assert_eq!(r.post_channel(&CH, b"one", &g).unwrap(), 0);
    assert_eq!(r.post_channel(&[9; 16], b"other", &Auth::None).unwrap(), 1);
    assert_eq!(r.post_channel(&CH, b"two", &g).unwrap(), 2);
    let all = r.poll_channel(&CH, None, 0, &g).unwrap();
    assert_eq!(all, vec![(0, b"one".to_vec()), (2, b"two".to_vec())]);
    let rest = r.poll_channel(&CH, Some(0), 0, &g).unwrap();
    assert_eq!(rest, vec![(2, b"two".to_vec())]);
    // The same storage as the plain relay.
    assert_eq!(r.poll(&CH, None, 0).unwrap().len(), 2);
}

#[test]
fn nonces_differ() {
    let mut r = MemRelay::default();
    let a = r.nonce().unwrap();
    assert_ne!(a, r.nonce().unwrap());
}

fn station() -> (tokio::runtime::Runtime, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let state = cahoots_station::server::State::open(&dir.path().join("s.db")).unwrap();
    let app = cahoots_station::server::app(state);
    rt.spawn(async move { axum::serve(listener, app).await.unwrap() });
    (rt, format!("http://{addr}"), dir)
}

#[test]
fn http_against_a_station() {
    let (_rt, base, _dir) = station();
    let mut r = HttpRelay::new(&base);
    assert_eq!(r.get_manifest(&CH).unwrap(), None);
    // A request object is open to anyone, and reads back through the list.
    let id = r
        .put_object(&CH, ObjectKind::Request, b"let me in")
        .unwrap();
    assert_eq!(id.len(), 32);
    r.put_object(&CH, ObjectKind::Request, b"and me").unwrap();
    let after = [("after", "0")];
    let q = [("to", &*"00".repeat(32)), ("epoch", &*"00".repeat(20))];
    // A workspace build with every feature turns the station's gate on: then the queue and
    // the deposits of a channel it does not hold are refused rather than listed.
    if cahoots_station::server::features().contains(&"gate") {
        for (kind, query) in [
            (ObjectKind::Request, &after[..]),
            (ObjectKind::Deposit, &q[..]),
        ] {
            assert!(matches!(
                r.list(&CH, kind, query, &Auth::None),
                Err(RelayError::Refused(_))
            ));
        }
    } else {
        let all = r.list(&CH, ObjectKind::Request, &[], &Auth::None).unwrap();
        assert_eq!(all, vec![b"let me in".to_vec(), b"and me".to_vec()]);
        let later = r
            .list(&CH, ObjectKind::Request, &after, &Auth::None)
            .unwrap();
        assert_eq!(later, vec![b"and me".to_vec()]);
        // Deposits for an unknown epoch: an empty list.
        assert!(r
            .list(&CH, ObjectKind::Deposit, &q, &Auth::None)
            .unwrap()
            .is_empty());
    }
    // A channel the station does not hold: 4xx comes back as Refused.
    assert!(matches!(
        r.post_channel(&CH, b"x", &Auth::None),
        Err(RelayError::Refused(_))
    ));
    assert!(matches!(
        r.put_object(&CH, ObjectKind::Request, b""),
        Err(RelayError::Refused(_))
    ));
}
