use cahoots_core::channel::{
    join_code, parse_join_code, parts, Channel, ChannelError, Gate, Mailbox, Round,
    CEREMONY_VERSION, MAX_WIRE, OFFER_PREFIX,
};
use cahoots_core::params::CircleParams;
use rand_core::{CryptoRng, RngCore};

/// A deterministic generator for the tests: xorshift, not for keys anyone keeps.
struct TestRng(u64);

impl RngCore for TestRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for b in dest {
            *b = (self.next_u64() >> 56) as u8;
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for TestRng {}

fn rng(seed: u64) -> TestRng {
    TestRng(0x9E37_79B9_7F4A_7C15 ^ seed)
}

const RELAY: &str = "https://relay.cahoots.network";
const HOST: [u8; 32] = [0x42; 32];

fn params() -> CircleParams {
    CircleParams::new(3, 65536, "park", false).unwrap()
}

#[test]
fn offer_round_trips() {
    let ch = Channel::new(&mut rng(1));
    let text = ch.offer(&params(), RELAY, &HOST).unwrap();
    assert!(text.starts_with(OFFER_PREFIX), "{text}");
    // One small static code: well under what a version 10 QR code holds at low correction.
    assert!(text.len() < 200, "{} characters", text.len());
    let o = Channel::from_offer(&text).unwrap();
    let (back, p) = (o.channel, o.params);
    assert_eq!(o.relay, RELAY);
    assert_eq!(o.host, HOST);
    assert_eq!(back.id, ch.id);
    assert_eq!(p, params());
    // The same key: a message sealed by one opens with the other.
    let wire = ch.seal(1, Round::Commit, b"hello", &mut rng(2));
    assert_eq!(back.open(&wire).unwrap(), (1, Round::Commit, b"hello".to_vec()));

    assert_eq!(
        Channel::from_offer("cahoots-params:0:AQ").unwrap_err(),
        ChannelError::NotAnOffer
    );
    assert_eq!(
        Channel::from_offer(&format!("{OFFER_PREFIX}AAAA")).unwrap_err(),
        ChannelError::Malformed
    );
    // Parameters that do not read are refused.
    // A relay address longer than its one-byte length is refused.
    assert_eq!(ch.offer(&params(), &"x".repeat(256), &HOST).unwrap_err(), ChannelError::Malformed);
    let mut bad = text.clone();
    bad.truncate(text.len() - 2);
    assert!(Channel::from_offer(&bad).is_err());
}

#[test]
fn a_sealed_message_opens_only_with_the_key() {
    let ch = Channel::new(&mut rng(3));
    let wire = ch.seal(2, Round::Reveal, b"a sealed reveal", &mut rng(4));
    // The station's header: version, the channel id, the length of the rest.
    assert_eq!(wire[0], CEREMONY_VERSION);
    assert_eq!(&wire[1..17], &ch.id);
    let len = u32::from_le_bytes(wire[17..21].try_into().unwrap()) as usize;
    assert_eq!(wire.len(), 21 + len);
    assert!(!wire.windows(15).any(|w| w == b"a sealed reveal"));
    assert_eq!(
        ch.open(&wire).unwrap(),
        (2, Round::Reveal, b"a sealed reveal".to_vec())
    );

    // A stranger with its own key, posting under the same id, is refused.
    let mut stranger = Channel::new(&mut rng(5));
    stranger.id = ch.id;
    let theirs = stranger.seal(2, Round::Reveal, b"a sealed reveal", &mut rng(6));
    assert_eq!(ch.open(&theirs).unwrap_err(), ChannelError::Open);
    // Another channel's message is not this channel's.
    let other = Channel::new(&mut rng(7));
    assert_eq!(other.open(&wire).unwrap_err(), ChannelError::OtherChannel);
}

#[test]
fn aad_binds_seat_and_round() {
    let ch = Channel::new(&mut rng(8));
    let wire = ch.seal(1, Round::Commit, b"commitment", &mut rng(9));
    // The seat and the round travel in the clear after the length, and the tag covers them.
    let mut seat = wire.clone();
    seat[21] = 2;
    assert_eq!(ch.open(&seat).unwrap_err(), ChannelError::Open);
    let mut round = wire.clone();
    round[22] = Round::Attest as u8;
    assert_eq!(ch.open(&round).unwrap_err(), ChannelError::Open);
    let mut unknown = wire.clone();
    unknown[22] = 9;
    assert_eq!(ch.open(&unknown).unwrap_err(), ChannelError::Malformed);
}

#[test]
fn a_tampered_message_is_refused() {
    let ch = Channel::new(&mut rng(10));
    let wire = ch.seal(0, Round::Attest, b"signature", &mut rng(11));
    for i in 23..wire.len() {
        let mut t = wire.clone();
        t[i] ^= 1;
        assert_eq!(ch.open(&t).unwrap_err(), ChannelError::Open, "byte {i}");
    }
    let mut short = wire.clone();
    short.pop();
    assert_eq!(ch.open(&short).unwrap_err(), ChannelError::Malformed);
    let mut version = wire.clone();
    version[0] = 2;
    assert_eq!(ch.open(&version).unwrap_err(), ChannelError::Malformed);
    assert_eq!(ch.open(&[]).unwrap_err(), ChannelError::Malformed);
}

#[test]
fn a_large_payload_travels_in_parts_under_the_station_cap() {
    let big: Vec<u8> = (0..300_000u32).map(|i| (i * 7) as u8).collect();
    let ch = Channel::new(&mut rng(12));
    let mut host = Mailbox::new(ch.clone(), 0);
    let mut guest = Mailbox::new(ch, 1);
    host.queue(Round::Reveal, &big, &mut rng(13));
    let wires = host.unposted();
    assert_eq!(wires.len(), parts(&big).len());
    assert!(wires.len() > 1);
    assert!(wires.iter().all(|w| w.len() <= MAX_WIRE));
    // Out of order, with a repeat: the payload comes out once, whole.
    let mut got = Vec::new();
    for w in wires.iter().rev().chain(wires.iter()) {
        if let Some(x) = guest.receive(w) {
            guest.hold(x.0, x.1);
            got.push(x);
        }
    }
    assert_eq!(got, vec![(0, Round::Reveal, big)]);
}

#[test]
fn a_mailbox_keeps_the_first_message_per_seat_and_round() {
    let ch = Channel::new(&mut rng(14));
    let mut a = Mailbox::new(ch.clone(), 0);
    let mut b = Mailbox::new(ch.clone(), 1);
    let mut c = Mailbox::new(ch.clone(), 2);
    b.queue(Round::Commit, b"first", &mut rng(15));
    // Queued once: a second queue for the round changes nothing.
    b.queue(Round::Commit, b"second", &mut rng(16));
    assert_eq!(b.unposted().len(), 1);
    let first = b.unposted().remove(0);
    // A second sealing of another commitment for seat 1, as a second device on that seat.
    let mut d = Mailbox::new(ch.clone(), 1);
    d.queue(Round::Commit, b"other", &mut rng(17));
    let second = d.unposted().remove(0);

    assert_eq!(a.receive(&first), Some((1, Round::Commit, b"first".to_vec())));
    a.hold(1, Round::Commit);
    assert_eq!(a.receive(&second), None);
    assert_eq!(a.receive(&first), None);
    // Our own message seen on the relay counts as posted, and being first it wins the seat.
    assert_eq!(b.receive(&first), None);
    assert!(b.unposted().is_empty());
    assert!(!b.lost_seat());
    // The other device's message for our seat is never handed up.
    assert_eq!(b.receive(&second), None);
    assert!(!b.lost_seat());
    // The other device reads ours first: it lost the seat, and posts nothing more.
    assert_eq!(d.receive(&first), None);
    assert_eq!(d.receive(&second), None);
    assert!(d.lost_seat());
    d.queue(Round::Reveal, b"a reveal", &mut rng(21));
    assert!(d.unposted().is_empty());
    assert!(!d.queued(Round::Reveal));
    // A message that does not open is dropped.
    let stranger = Channel::new(&mut rng(18));
    let mut s = stranger.seal(1, Round::Commit, b"x", &mut rng(19));
    s[1..17].copy_from_slice(&ch.id);
    assert_eq!(c.receive(&s), None);
    // Posting by hand clears it too.
    c.queue(Round::Commit, b"c", &mut rng(20));
    let w = c.unposted().remove(0);
    c.posted(&w);
    assert!(c.unposted().is_empty());
    assert!(c.queued(Round::Commit));
    assert!(!c.queued(Round::Reveal));
}

#[test]
fn a_refused_message_does_not_hold_the_slot() {
    let ch = Channel::new(&mut rng(22));
    let mut a = Mailbox::new(ch.clone(), 0);
    let mut early = Mailbox::new(ch.clone(), 1);
    early.queue(Round::Reveal, b"too early, refused", &mut rng(23));
    let bogus = early.unposted().remove(0);
    let mut real = Mailbox::new(ch.clone(), 1);
    real.queue(Round::Reveal, b"the real reveal", &mut rng(24));
    let good = real.unposted().remove(0);
    // The ceremony refused the first one, so it is not held, and the real one still gets in.
    assert_eq!(a.receive(&bogus), Some((1, Round::Reveal, b"too early, refused".to_vec())));
    assert!(!a.held(1, Round::Reveal));
    assert_eq!(a.receive(&good), Some((1, Round::Reveal, b"the real reveal".to_vec())));
    a.hold(1, Round::Reveal);
    // Once accepted, later ones are ignored.
    assert_eq!(a.receive(&bogus), None);
}

#[test]
fn reproduces_the_channel_vector() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vectors/channel.json"
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let h = |k: &serde_json::Value| hex::decode(k.as_str().unwrap()).unwrap();
    let id: [u8; 16] = h(&v["id"]).try_into().unwrap();
    let key: [u8; 32] = h(&v["key"]).try_into().unwrap();
    let params = CircleParams::decode(&h(&v["params"])).unwrap();
    let ch = Channel::from_key(id, key);
    let relay = v["relay"].as_str().unwrap();
    let host: [u8; 32] = h(&v["host_commitment_hash"]).try_into().unwrap();
    assert_eq!(host, cahoots_core::channel::commitment_hash(&h(&v["host_commitment"])));
    assert_eq!(ch.offer(&params, relay, &host).unwrap(), v["offer"].as_str().unwrap());
    let o = Channel::from_offer(v["offer"].as_str().unwrap()).unwrap();
    assert_eq!((o.relay.as_str(), o.host), (relay, host));
    let j = &v["join"];
    assert_eq!(
        join_code(&id, j["seat"].as_u64().unwrap() as u8, &h(&j["commitment"])),
        j["code"].as_str().unwrap()
    );
    let m = &v["message"];
    let nonce: [u8; 12] = h(&m["nonce"]).try_into().unwrap();
    let round = Round::from_u8(m["round"].as_u64().unwrap() as u8).unwrap();
    let seat = m["seat"].as_u64().unwrap() as u8;
    let wire = ch.seal_with_nonce(seat, round, &h(&m["payload"]), nonce);
    assert_eq!(wire, h(&m["wire"]));
    assert_eq!(ch.open(&wire).unwrap(), (seat, round, h(&m["payload"])));
}

#[test]
fn the_host_takes_only_a_scanned_commitment() {
    let ch = Channel::new(&mut rng(30));
    let real = b"the joiner's commitment".to_vec();
    let intruder = b"an intruder's commitment".to_vec();
    let code = join_code(&ch.id, 2, &real);
    assert!(code.starts_with("cahoots:joined:v1:"));
    assert_eq!(parse_join_code(&code).unwrap().1, 2);
    let mut g = Gate::new();
    // The intruder's comes first on the relay. Nothing is ready without a scan.
    g.commitment(2, intruder.clone());
    g.commitment(2, real.clone());
    g.defer(2, Round::Reveal, b"a reveal".to_vec());
    assert!(g.ready().is_empty());
    assert_eq!(g.unscanned(), vec![2]);
    assert!(g.take_deferred().is_empty());
    // A code for another channel is refused.
    let other = Channel::new(&mut rng(31));
    assert_eq!(g.scan(&other, &code).unwrap_err(), ChannelError::OtherChannel);
    // The scan picks the matching one, not the first.
    assert_eq!(g.scan(&ch, &code).unwrap(), 2);
    assert_eq!(g.ready(), vec![(2, real)]);
    g.fed(2);
    assert_eq!(g.scanned(), vec![2]);
    assert!(g.unscanned().is_empty());
    assert_eq!(g.take_deferred(), vec![(2, Round::Reveal, b"a reveal".to_vec())]);
    // Once taken, later commitments for the seat are ignored.
    g.commitment(2, intruder);
    assert!(g.ready().is_empty() && g.unscanned().is_empty());
}
