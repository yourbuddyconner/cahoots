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
            let mut signed = 0;
            let r = bridge.run(
                &mut link,
                &mut poster,
                &mut poller,
                &mut |seat, code: &str| {
                    codes_tx.send((seat, code.to_string())).unwrap();
                },
                &mut || signed += 1,
            );
            // The bridge ran until card 1 was gone, and said once that everyone signed.
            assert_eq!(signed, 1);
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
    let bridge = Bridge::new(Channel::from_offer(text).unwrap(), 2).unwrap();
    start(bridge)
}

/// `bridge` and the far end of its cable, with card 1's `Params` frame waiting on the cable.
fn start(bridge: Bridge) -> (Bridge, SocketLink, UnixStream) {
    let params = bridge.offer().params.clone();
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

/// A ring frame from the pile: `kind`, from the card in `index`, with `body`.
fn from_pile(far: &mut UnixStream, kind: u8, index: u8, body: &[u8]) {
    use std::io::Write;
    let mut payload = vec![index];
    payload.extend_from_slice(body);
    far.write_all(&frame::encode(&frame::Frame {
        kind,
        seq: 7,
        payload: &payload,
    }))
    .unwrap();
}

/// Step until the bridge stops, or panic after `within`.
fn step_until_error(bridge: &mut Bridge, link: &mut SocketLink, within: Duration) -> BridgeError {
    let deadline = Instant::now() + within;
    loop {
        assert!(Instant::now() < deadline, "the bridge kept going");
        if let Err(e) = bridge.step(link, &mut |_, _| {}) {
            return e;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_frame_from_a_seat_above_the_cards_means_the_pile_is_bigger() {
    let (_, text) = offer_text(5, 2, b"host");
    let (mut bridge, mut link, mut far) = started_bridge(&text);
    assert_eq!(bridge.step(&mut link, &mut |_, _| {}).unwrap(), BridgeStep::Working);
    // Seat 3 is a phone's. Nothing the bridge sends up comes back down, so a card holds it.
    from_pile(&mut far, 1, 3, &bridge.offer().params.encode());
    let e = step_until_error(&mut bridge, &mut link, Duration::from_secs(5));
    assert!(matches!(e, BridgeError::Count { pile: 3, offer: 2 }), "{e:?}");
    assert_eq!(
        e.to_string(),
        "the pile has at least 3 cards and the host said 2. Restack and start again."
    );
}

#[test]
fn an_abort_from_the_pile_ends_the_run() {
    let (_, text) = offer_text(4, 2, b"host");
    let (mut bridge, mut link, mut far) = started_bridge(&text);
    assert_eq!(bridge.step(&mut link, &mut |_, _| {}).unwrap(), BridgeStep::Working);
    from_pile(&mut far, 4, 2, &[7]);
    let e = step_until_error(&mut bridge, &mut link, Duration::from_secs(5));
    assert!(matches!(e, BridgeError::Aborted { seat: 2, reason: 7 }), "{e:?}");
    assert_eq!(e.to_string(), "card 3 left the ceremony (reason 7).");
}

#[test]
fn a_pile_with_too_few_cards_is_named_after_a_while() {
    let (_, text) = offer_text(4, 2, b"host");
    let bridge = Bridge::new(Channel::from_offer(&text).unwrap(), 2)
        .unwrap()
        .with_pile_deadline(Duration::from_millis(300));
    // Only card 1 is there: seat 2's parameters never come down.
    let (mut bridge, mut link, _far) = start(bridge);
    let begun = Instant::now();
    let e = step_until_error(&mut bridge, &mut link, Duration::from_secs(5));
    assert!(begun.elapsed() >= Duration::from_millis(300));
    assert!(matches!(e, BridgeError::Count { pile: 1, offer: 2 }), "{e:?}");
    assert_eq!(
        e.to_string(),
        "the pile has at least 1 cards and the host said 2. Restack and start again."
    );
}

#[test]
fn a_whole_pile_is_not_named_too_small() {
    let (_, text) = offer_text(4, 2, b"host");
    let bridge = Bridge::new(Channel::from_offer(&text).unwrap(), 2)
        .unwrap()
        .with_pile_deadline(Duration::from_millis(100));
    let (mut bridge, mut link, mut far) = start(bridge);
    from_pile(&mut far, 1, 2, &bridge.offer().params.encode());
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        assert_eq!(bridge.step(&mut link, &mut |_, _| {}).unwrap(), BridgeStep::Working);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn every_new_reveal_and_signature_goes_up_and_only_the_first_commitment() {
    let (channel, text) = offer_text(4, 2, b"host");
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    post(&relay, &channel, 3, Round::Commit, b"first");
    post(&relay, &channel, 3, Round::Commit, b"second");
    post(&relay, &channel, 3, Round::Reveal, b"forged");
    post(&relay, &channel, 3, Round::Reveal, b"real");
    post(&relay, &channel, 3, Round::Attest, b"forged");
    post(&relay, &channel, 3, Round::Attest, b"real");
    let (mut bridge, mut link, mut far) = started_bridge(&text);
    step_with(&mut bridge, &mut link, &relay, &channel.id);
    // The same reveal posted again, and read again, goes up once.
    post(&relay, &channel, 3, Round::Reveal, b"real");
    step_with(&mut bridge, &mut link, &relay, &channel.id);
    let by_kind = |kind: u8, frames: &[(u8, u8, Vec<u8>)]| -> Vec<Vec<u8>> {
        frames.iter().filter(|(k, i, _)| (*k, *i) == (kind, 3)).map(|(_, _, b)| b.clone()).collect()
    };
    let frames = ring_frames(&mut far);
    assert_eq!(by_kind(2, &frames), [b"first".to_vec()]);
    assert_eq!(by_kind(3, &frames), [b"forged".to_vec(), b"real".to_vec()]);
    assert_eq!(by_kind(5, &frames), [b"forged".to_vec(), b"real".to_vec()]);
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
    let r = bridge.run(&mut link, &mut SlowRelay, &mut SlowRelay, &mut |_, _| {}, &mut || {});
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

#[test]
fn a_forged_host_signature_first_does_not_keep_the_real_one_from_the_cards() {
    let (channel, text) = offer_text(4, 2, b"host");
    let relay = Arc::new(Mutex::new(MemRelay::default()));
    post(&relay, &channel, 0, Round::Commit, b"host");
    post(&relay, &channel, 0, Round::Attest, b"forged");
    post(&relay, &channel, 3, Round::Attest, b"seat 4");
    let (mut bridge, mut link, mut far) = started_bridge(&text);
    // The whole pile, and each card's signature.
    let params = bridge.offer().params.encode();
    from_pile(&mut far, 1, 2, &params);
    from_pile(&mut far, 5, 1, b"card 1");
    from_pile(&mut far, 5, 2, b"card 2");
    let signed = Arc::new(AtomicBool::new(false));
    let bridge_thread = {
        let signed = signed.clone();
        let mut poster = MemRelay::shared(relay.clone());
        let mut poller = MemRelay::shared(relay.clone());
        std::thread::spawn(move || {
            bridge.run(&mut link, &mut poster, &mut poller, &mut |_, _| {}, &mut || {
                signed.store(true, Ordering::SeqCst)
            })
        })
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while !signed.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the bridge never said everyone signed");
        seen.extend(ring_frames(&mut far));
        std::thread::sleep(Duration::from_millis(20));
    }
    let attests = |seen: &[(u8, u8, Vec<u8>)]| -> Vec<Vec<u8>> {
        seen.iter().filter(|(k, i, _)| (*k, *i) == (5, 0)).map(|(_, _, b)| b.clone()).collect()
    };
    // The forged one went down first, and the bridge thinks everyone signed.
    seen.extend(ring_frames(&mut far));
    assert_eq!(attests(&seen), [b"forged".to_vec()]);
    assert!(!bridge_thread.is_finished(), "the bridge stopped on a signature it cannot check");
    // The real one, posted later, still reaches the cards.
    post(&relay, &channel, 0, Round::Attest, b"real");
    while !attests(&seen).contains(&b"real".to_vec()) {
        assert!(Instant::now() < deadline, "the real signature never went down: {:?}", attests(&seen));
        seen.extend(ring_frames(&mut far));
        std::thread::sleep(Duration::from_millis(20));
    }
    // Unplugged after everyone signed: the bridge is done.
    drop(far);
    bridge_thread.join().unwrap().unwrap();
    // Each card's signature went to the relay.
    let on_relay = relay.lock().unwrap().poll(&channel.id, None, 0).unwrap();
    let cards: Vec<_> = on_relay
        .iter()
        .filter_map(|(_, w)| channel.open(w).ok())
        .filter(|(seat, round, _)| *round == Round::Attest && (1..=2).contains(seat))
        .collect();
    assert_eq!(cards.len(), 2);
}
