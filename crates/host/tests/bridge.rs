//! The bridge: a phone hosts a ceremony through the relay, and a pile of cards on a laptop
//! joins it through `cahoots-host bridge`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_core::channel::{commitment_hash, parts, Channel, Round};
use cahoots_core::frame;
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_core::params::CircleParams;
use cahoots_hal::Link;
use cahoots_host::bridge::{Bridge, BridgeError, BridgeStep};
use cahoots_host::link::{Absent, SocketLink};
use cahoots_host::pile::spawn_card;
use cahoots_phone::signer::SoftSigner;
use cahoots_phone::{CeremonyState, Http, Phone, PhoneError};
use cahoots_relay::{MemRelay, Relay};
use rand_core::OsRng;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const RELAY: &str = "https://relay.test";
const PAD: u32 = 16 * 1024;

/// An HTTP relay in process: the URLs and bodies the station speaks, over `MemRelay`.
#[derive(Clone)]
struct FakeHttp {
    relay: Arc<Mutex<MemRelay>>,
}

fn circle_of(url: &str) -> [u8; 16] {
    let rest = url
        .strip_prefix(&format!("{RELAY}/v1/circles/"))
        .expect("relay url");
    assert!(rest[32..].starts_with("/messages"), "{url}");
    hex::decode(&rest[..32]).unwrap().try_into().unwrap()
}

impl Http for FakeHttp {
    fn get(&self, url: String) -> Result<Vec<u8>, PhoneError> {
        let circle = circle_of(&url);
        let after = url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("after="))
            .map(|a| a.parse::<u64>().unwrap());
        let entries = self.relay.lock().unwrap().poll(&circle, after, 0).unwrap();
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(seq, m)| serde_json::json!({ "seq": seq, "message": URL_SAFE_NO_PAD.encode(m), "received_at": 0 }))
            .collect();
        Ok(serde_json::to_vec(&serde_json::json!({ "entries": entries })).unwrap())
    }

    fn post(&self, url: String, body: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        let circle = circle_of(&url);
        let seq = self.relay.lock().unwrap().post(&circle, &body).unwrap();
        Ok(serde_json::to_vec(&serde_json::json!({ "seq": seq })).unwrap())
    }

    fn request(
        &self,
        _method: String,
        _url: String,
        _headers: Vec<cahoots_phone::HttpHeader>,
        _body: Option<Vec<u8>>,
    ) -> Result<cahoots_phone::HttpResponse, PhoneError> {
        Ok(cahoots_phone::HttpResponse {
            status: 404,
            body: Vec::new(),
        })
    }
}

struct PhoneDevice {
    _dir: TempDir,
    phone: Arc<Phone>,
}

fn phone(relay: &Arc<Mutex<MemRelay>>) -> PhoneDevice {
    let dir = TempDir::new().unwrap();
    let phone = Phone::open(
        dir.path().to_string_lossy().into_owned(),
        Box::new(SoftSigner::new()),
        Box::new(FakeHttp {
            relay: relay.clone(),
        }),
    )
    .unwrap();
    PhoneDevice { _dir: dir, phone }
}

/// One pass of the app's loop: tick, and sign when step three asks.
fn tick(d: &PhoneDevice, id: &str) -> CeremonyState {
    let s = d.phone.ceremony_tick(id.into()).unwrap();
    if s.needs_signature {
        d.phone.ceremony_sign(id.into()).unwrap();
        return d.phone.ceremony_tick(id.into()).unwrap();
    }
    s
}

fn manifest_seats(sealed: &SealedManifest) -> usize {
    Manifest::decode(&sealed.body).unwrap().seats.len()
}

#[test]
fn a_phone_hosts_two_cards_through_the_bridge_and_a_phone_joiner() {
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    let host = phone(&relay);
    let joiner = phone(&relay);
    host.phone.set_default_relay(RELAY.into()).unwrap();
    let id = host
        .phone
        .start_ceremony("mixed".into(), 4, 2, PAD, true)
        .unwrap();
    let code = host.phone.offer_code(id.clone()).unwrap();
    let jid = joiner.phone.join_offer(code.clone(), 3).unwrap();

    // The pile: the bridge's link, then card 1, then card 2 on top with nothing above it.
    let (bridge_end, card1_down) = UnixStream::pair().unwrap();
    let (card1_up, card2_down) = UnixStream::pair().unwrap();
    let dirs = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let card1 = spawn_card(
        Box::new(SocketLink::new(card1_down).unwrap()),
        Box::new(SocketLink::new(card1_up).unwrap()),
        "card1",
        dirs.0.path().to_path_buf(),
        Some(11),
    );
    let card2 = spawn_card(
        Box::new(SocketLink::new(card2_down).unwrap()),
        Box::new(Absent),
        "card2",
        dirs.1.path().to_path_buf(),
        Some(12),
    );

    let mut bridge = Bridge::new(Channel::from_offer(&code).unwrap(), 2).unwrap();
    let (codes_tx, codes_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let bridge_thread = {
        let stop = stop.clone();
        let mut poster = MemRelay::shared(relay.clone());
        let mut poller = MemRelay::shared(relay.clone());
        std::thread::spawn(move || {
            let mut link = SocketLink::new(bridge_end).unwrap();
            let r = bridge.run(&mut link, &mut poster, &mut poller, &mut |seat, code: &str| {
                codes_tx.send((seat, code.to_string())).unwrap();
            });
            // Keep the cable in until the cards are done with it.
            while !stop.load(Ordering::SeqCst) {
                link.flush();
                let mut buf = [0u8; 256];
                while link.recv(&mut buf) > 0 {}
                std::thread::sleep(Duration::from_millis(2));
            }
            r
        })
    };

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut joiner_scanned = false;
    let mut card_seats = Vec::new();
    let (sh, sj) = loop {
        assert!(Instant::now() < deadline, "the ceremony did not finish");
        let sh = tick(&host, &id);
        let sj = tick(&joiner, &jid);
        while let Ok((seat, c)) = codes_rx.try_recv() {
            card_seats.push(seat);
            host.phone.ceremony_scan(id.clone(), c).unwrap();
        }
        if !joiner_scanned {
            if let Some(c) = sj.my_join_code.clone() {
                host.phone.ceremony_scan(id.clone(), c).unwrap();
                joiner_scanned = true;
            }
        }
        if host.phone.ceremony_state(id.clone()).unwrap().needs_continue {
            host.phone.ceremony_continue(id.clone()).unwrap();
        }
        if sh.words.len() == 6 && sj.words.len() == 6 {
            break (sh, sj);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    card_seats.sort();
    assert_eq!(card_seats, [1, 2], "one join code per card");

    let d1 = card1.join().unwrap().unwrap();
    let d2 = card2.join().unwrap().unwrap();
    stop.store(true, Ordering::SeqCst);
    bridge_thread.join().unwrap().unwrap();

    assert_eq!(sh.words, sj.words);
    let words: Vec<String> = d1.check_words().iter().map(|w| w.to_string()).collect();
    assert_eq!(words, sh.words);
    assert_eq!(d2.check_words(), d1.check_words());
    assert_eq!(d1.params().n, 4);
    assert_eq!(d1.params().name, "mixed");
    assert_eq!(manifest_seats(&d1.manifest().unwrap()), 4);
    assert_eq!(manifest_seats(&d2.manifest().unwrap()), 4);
    let circle = host.phone.ceremony_keep(id).unwrap();
    assert_eq!(host.phone.circle_members(circle).unwrap().len(), 4);
}

/// A host's offer for a circle of `n` with `badges` cards, made without a phone.
fn offer_text(n: u8, badges: u8, host_commitment: &[u8]) -> (Channel, String) {
    let channel = Channel::new(&mut OsRng);
    let params = CircleParams::new(n, PAD, "pile", false).unwrap();
    let text = channel
        .offer(&params, RELAY, &commitment_hash(host_commitment), badges)
        .unwrap();
    (channel, text)
}

#[test]
fn the_bridge_refuses_a_pile_that_does_not_match_the_offer() {
    let (_, text) = offer_text(4, 2, b"host");
    let offer = Channel::from_offer(&text).unwrap();
    assert!(matches!(
        Bridge::new(offer, 3),
        Err(BridgeError::Count { pile: 3, offer: 2 })
    ));
}

/// A bridge and the far end of its cable, with a card's first ring frame already read so
/// the bridge reads the relay.
fn started_bridge(text: &str) -> (Bridge, SocketLink, UnixStream) {
    let offer = Channel::from_offer(text).unwrap();
    let params = offer.params.clone();
    let bridge = Bridge::new(offer, 2).unwrap();
    let (near, mut far) = UnixStream::pair().unwrap();
    let link = SocketLink::new(near).unwrap();
    let mut payload = vec![1u8];
    payload.extend_from_slice(&params.encode());
    let f = frame::encode(&frame::Frame {
        kind: 1,
        seq: 0,
        payload: &payload,
    });
    use std::io::Write;
    far.write_all(&f).unwrap();
    far.set_nonblocking(true).unwrap();
    (bridge, link, far)
}

/// Every ring frame (kind below 16) the bridge sent up the cable: (kind, index, body).
fn ring_frames(far: &mut UnixStream) -> Vec<(u8, u8, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = far.read(&mut buf) {
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let mut out = Vec::new();
    let mut at = 0;
    while let Ok((f, used)) = frame::decode(&bytes[at..]) {
        if f.kind < 16 {
            out.push((f.kind, f.payload[0], f.payload[1..].to_vec()));
        }
        at += used;
    }
    out
}

fn post(relay: &Arc<Mutex<MemRelay>>, channel: &Channel, seat: u8, round: Round, payload: &[u8]) {
    for p in parts(payload) {
        let wire = channel.seal(seat, round, &p, &mut OsRng);
        relay.lock().unwrap().post(&channel.id, &wire).unwrap();
    }
}

/// Hand the bridge everything on the relay, then step it a few times.
fn step_with(bridge: &mut Bridge, link: &mut SocketLink, relay: &Arc<Mutex<MemRelay>>, id: &[u8; 16]) {
    let entries = relay.lock().unwrap().poll(id, None, 0).unwrap();
    bridge.deliver(entries);
    for _ in 0..4 {
        assert_eq!(bridge.step(link, &mut |_, _| {}).unwrap(), BridgeStep::Working);
    }
}

#[test]
fn a_relay_message_for_a_badge_seat_never_enters_the_pile() {
    let (channel, text) = offer_text(4, 2, b"host");
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    post(&relay, &channel, 1, Round::Commit, b"a card's seat, from someone else");
    post(&relay, &channel, 0, Round::Commit, b"host");
    let (mut bridge, mut link, mut far) = started_bridge(&text);
    step_with(&mut bridge, &mut link, &relay, &channel.id);
    let frames = ring_frames(&mut far);
    assert!(frames.iter().any(|(k, i, _)| (*k, *i) == (2, 0)), "the host's commitment went up: {frames:?}");
    assert!(!frames.iter().any(|(_, i, _)| *i == 1), "{frames:?}");
}

#[test]
fn a_host_commitment_that_does_not_match_the_offer_is_skipped() {
    let (channel, text) = offer_text(4, 2, b"host");
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    post(&relay, &channel, 0, Round::Commit, b"not the host");
    let (mut bridge, mut link, mut far) = started_bridge(&text);
    step_with(&mut bridge, &mut link, &relay, &channel.id);
    assert!(!ring_frames(&mut far).iter().any(|(k, _, _)| *k == 2), "the stray stayed out");
    // The real one, posted after the stray, still goes up.
    post(&relay, &channel, 0, Round::Commit, b"host");
    step_with(&mut bridge, &mut link, &relay, &channel.id);
    let commits: Vec<_> = ring_frames(&mut far)
        .into_iter()
        .filter(|(k, _, _)| *k == 2)
        .collect();
    assert_eq!(commits, [(2, 0, b"host".to_vec())]);
}

/// A cable to a card that is always there and never says anything, which notes when it
/// was read. It is unplugged after `for_how_long`.
struct WatchedLink {
    reads: Arc<Mutex<Vec<Instant>>>,
    until: Instant,
}

impl Link for WatchedLink {
    fn send(&mut self, _frame: &[u8]) {}
    fn recv(&mut self, _buf: &mut [u8]) -> usize {
        self.reads.lock().unwrap().push(Instant::now());
        0
    }
    fn peer_present(&self) -> bool {
        Instant::now() < self.until
    }
}

/// A relay whose every read takes three seconds.
struct SlowRelay;

impl Relay for SlowRelay {
    fn post(&mut self, _circle: &[u8; 16], _message: &[u8]) -> Result<u64, cahoots_relay::RelayError> {
        Ok(0)
    }
    fn poll(
        &mut self,
        _circle: &[u8; 16],
        _after: Option<u64>,
        _wait_ms: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, cahoots_relay::RelayError> {
        std::thread::sleep(Duration::from_secs(3));
        Ok(Vec::new())
    }
}

#[test]
fn a_slow_relay_never_stops_the_bridge_reading_the_cable() {
    let (_, text) = offer_text(4, 2, b"host");
    let mut bridge = Bridge::new(Channel::from_offer(&text).unwrap(), 2).unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let begun = Instant::now();
    let mut link = WatchedLink {
        reads: reads.clone(),
        until: begun + Duration::from_millis(4500),
    };
    let r = bridge.run(&mut link, &mut SlowRelay, &mut SlowRelay, &mut |_, _| {});
    assert!(matches!(r, Err(BridgeError::Link)), "{r:?}");
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 10, "{} reads", reads.len());
    let gap = reads
        .windows(2)
        .map(|w| w[1] - w[0])
        .max()
        .unwrap();
    assert!(gap < Duration::from_secs(1), "the cable went {gap:?} without a read");
    assert!(reads.last().unwrap().duration_since(begun) > Duration::from_secs(4));
}
