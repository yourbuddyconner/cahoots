//! The demo client, driven without threads or stdin, against an in-memory relay and
//! against a real `cahoots-station` started in-process.
#![cfg(feature = "cli")]

use cahoots_chat::cli::ceremony::local_ceremony;
use cahoots_chat::cli::client::{Client, Event};
use cahoots_chat::cli::relay::{HttpRelay, MemRelay, Relay};
use cahoots_chat::cli::store::open_circle;
use cahoots_chat::ChatError;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn ceremony(dir: &Path, members: u8) -> cahoots_core::CircleId {
    let r = local_ceremony(members, 4096, "demo", dir).unwrap();
    assert_eq!(r.check_words.len(), 6);
    r.circle_id
}

#[test]
fn local_ceremony_writes_file_provider_circles_that_agree() {
    let dir = tempfile::tempdir().unwrap();
    let id = ceremony(dir.path(), 3);
    for i in 0..3u8 {
        let member = dir.path().join(format!("member-{i}"));
        // The provider crate can open what we wrote.
        let p = cahoots_provider::FileProvider::open(&member).unwrap();
        assert_eq!(p.circles(), vec![id]);
        assert_eq!(p.my_slice(&id), Some(i));
        // And so can our adapters.
        let c = open_circle(&member).unwrap();
        assert_eq!(c.circle_id, id);
        assert_eq!(c.my_slice, i);
        assert_eq!(c.params.n, 3);
        assert_eq!(c.label, "CSPRNG");
    }
}

fn clients(dir: &Path, relay: &Arc<Mutex<MemRelay>>, rotate_after: u32) -> Vec<Client<MemRelay>> {
    (0..3u8)
        .map(|i| {
            let member = dir.path_join(format!("member-{i}"));
            Client::open(&member, MemRelay::shared(relay.clone()), rotate_after).unwrap()
        })
        .collect()
}

trait PathJoin {
    fn path_join(&self, s: String) -> std::path::PathBuf;
}
impl PathJoin for Path {
    fn path_join(&self, s: String) -> std::path::PathBuf {
        self.join(s)
    }
}

/// Pump every pending relay entry into every client, collecting events per client.
fn pump(cs: &mut [Client<MemRelay>]) -> Vec<Vec<Event>> {
    cs.iter_mut().map(|c| c.poll(0).unwrap()).collect()
}

#[test]
fn three_clients_chat_rotate_reject_tamper_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    ceremony(dir.path(), 3);
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let mut cs = clients(dir.path(), &relay, 2);

    cs[0].send_line("hello from 0").unwrap();
    let ev = pump(&mut cs);
    assert_eq!(ev[0], vec![Event::Own { seq: 0 }]);
    assert_eq!(
        ev[1],
        vec![Event::Received {
            seq: 0,
            sender: 0,
            text: "hello from 0".into()
        }]
    );
    assert_eq!(ev[2].len(), 1);

    cs[1].send_line("hi from 1").unwrap();
    cs[2].send_line("hi from 2").unwrap();
    let ev = pump(&mut cs);
    assert_eq!(ev[0].len(), 2);
    assert!(matches!(&ev[0][1], Event::Received { sender: 2, text, .. } if text == "hi from 2"));

    // Rotation: client 0 sends its 2nd and 3rd message; the 3rd starts a new epoch.
    let e1 = cs[0].current_epoch().unwrap();
    cs[0].send_line("two").unwrap();
    assert_eq!(cs[0].current_epoch().unwrap(), e1);
    cs[0].send_line("three").unwrap();
    let e2 = cs[0].current_epoch().unwrap();
    assert_ne!(e1, e2);
    let ev = pump(&mut cs);
    assert!(matches!(&ev[1][1], Event::Received { text, .. } if text == "three"));
    assert_eq!(
        cs[1].current_epoch().unwrap(),
        e2,
        "receivers follow the newest epoch"
    );

    // Tamper: flip a ciphertext byte before posting.
    cs[2].post_tampered("secret").unwrap();
    let ev = pump(&mut cs);
    assert_eq!(
        ev[0],
        vec![Event::Rejected {
            seq: 5,
            error: ChatError::Tag
        }]
    );
    assert_eq!(
        ev[1],
        vec![Event::Rejected {
            seq: 5,
            error: ChatError::Tag
        }]
    );
    // The untouched chain still works afterwards.
    cs[2].send_line("after tamper").unwrap();
    let ev = pump(&mut cs);
    assert!(matches!(&ev[0][0], Event::Received { text, .. } if text == "after tamper"));

    // Restart client 1 from its persisted state: nothing pending, no replay.
    let last = cs[1].last_seq();
    drop(cs.remove(1));
    let mut c1 = Client::open(
        &dir.path().join("member-1"),
        MemRelay::shared(relay.clone()),
        2,
    )
    .unwrap();
    assert_eq!(c1.last_seq(), last);
    assert_eq!(c1.poll(0).unwrap(), vec![]);
    // A message sent now still decrypts on the restarted client.
    cs[0].send_line("welcome back").unwrap();
    let ev = c1.poll(0).unwrap();
    assert!(matches!(&ev[0], Event::Received { text, .. } if text == "welcome back"));
    // Feeding an old entry again is a replay, not a decrypt.
    let old = relay.lock().unwrap().entry(0).unwrap();
    assert_eq!(
        c1.handle_entry(0, &old),
        Event::Rejected {
            seq: 0,
            error: ChatError::Replay
        }
    );
}

#[test]
fn http_end_to_end_against_station() {
    let dir = tempfile::tempdir().unwrap();
    ceremony(dir.path(), 2);
    let log = dir.path().join("station.jsonl");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = cahoots_station::server::app(cahoots_station::server::State::open(&log).unwrap());
    rt.spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let mut a = Client::open(&dir.path().join("member-0"), HttpRelay::new(&base), 1000).unwrap();
    let mut b = Client::open(&dir.path().join("member-1"), HttpRelay::new(&base), 1000).unwrap();
    a.send_line("over http").unwrap();
    let ev = b.poll(2000).unwrap();
    assert!(matches!(&ev[0], Event::Received { sender: 0, text, .. } if text == "over http"));
    b.send_line("reply").unwrap();
    let ev = a.poll(2000).unwrap();
    assert_eq!(ev.len(), 2, "own echo then the reply");
    assert!(matches!(&ev[1], Event::Received { sender: 1, text, .. } if text == "reply"));
    a.post_tampered("x").unwrap();
    let ev = b.poll(2000).unwrap();
    assert!(matches!(
        ev.last(),
        Some(Event::Rejected {
            error: ChatError::Tag,
            ..
        })
    ));
    // The relay refused nothing: chat messages are never reuse-checked there.
    assert_eq!(
        HttpRelay::new(&base)
            .poll(&a.circle_id(), None, 0)
            .unwrap()
            .len(),
        3
    );
}
