//! The phone crate end to end: two phones with software signers and an in-process relay.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_core::manifest::SealedManifest;
use cahoots_core::seat::SeatPublic;
use cahoots_phone::signer::SoftSigner;
use cahoots_phone::store::CircleRecord;
use cahoots_phone::{CeremonyPhase, CeremonyState, Http, Phone, PhoneError};
use cahoots_relay::{MemRelay, Relay};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const RELAY: &str = "https://relay.test";
/// A second relay, for a circle that moves.
const RELAY2: &str = "https://relay2.test";
const PAD: u32 = 1024;

/// An HTTP relay in process: the URLs and bodies the station speaks, over `MemRelay`.
#[derive(Clone)]
struct FakeHttp {
    relay: Arc<Mutex<MemRelay>>,
    /// The store behind `RELAY2`.
    other: Arc<Mutex<MemRelay>>,
    /// The next this many posts fail as if the network were down.
    fail_posts: Arc<AtomicU32>,
}

impl FakeHttp {
    fn new(relay: &Arc<Mutex<MemRelay>>, other: &Arc<Mutex<MemRelay>>) -> Self {
        FakeHttp {
            relay: relay.clone(),
            other: other.clone(),
            fail_posts: Arc::new(AtomicU32::new(0)),
        }
    }
}

fn circle_of(url: &str) -> [u8; 16] {
    let rest = url
        .strip_prefix(&format!("{RELAY}/v1/circles/"))
        .or_else(|| url.strip_prefix(&format!("{RELAY2}/v1/circles/")))
        .expect("relay url");
    let hex_id = &rest[..32];
    assert!(rest[32..].starts_with("/messages"), "{url}");
    hex::decode(hex_id).unwrap().try_into().unwrap()
}

impl FakeHttp {
    fn store(&self, url: &str) -> &Arc<Mutex<MemRelay>> {
        if url.starts_with(RELAY2) {
            &self.other
        } else {
            &self.relay
        }
    }
}

impl Http for FakeHttp {
    fn get(&self, url: String) -> Result<Vec<u8>, PhoneError> {
        let circle = circle_of(&url);
        let after = url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("after="))
            .map(|a| a.parse::<u64>().unwrap());
        let entries = self.store(&url).lock().unwrap().poll(&circle, after, 0).unwrap();
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(seq, m)| serde_json::json!({ "seq": seq, "message": URL_SAFE_NO_PAD.encode(m), "received_at": 0 }))
            .collect();
        Ok(serde_json::to_vec(&serde_json::json!({ "entries": entries })).unwrap())
    }

    fn post(&self, url: String, body: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        let circle = circle_of(&url);
        let left = self.fail_posts.load(Ordering::SeqCst);
        if left > 0 {
            self.fail_posts.store(left - 1, Ordering::SeqCst);
            return Err(PhoneError::Network {
                message: "offline".into(),
            });
        }
        let seq = self.store(&url).lock().unwrap().post(&circle, &body).unwrap();
        Ok(serde_json::to_vec(&serde_json::json!({ "seq": seq })).unwrap())
    }

    /// The channel calls; these tests make none (`tests/channel.rs` runs a real station).
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

struct Device {
    _dir: TempDir,
    path: std::path::PathBuf,
    signer: SoftSigner,
    http: FakeHttp,
    phone: Arc<Phone>,
}

fn device(relay: &Arc<Mutex<MemRelay>>) -> Device {
    device_on(relay, &self::relay())
}

/// A device that reaches `relay` at `RELAY` and `other` at `RELAY2`.
fn device_on(relay: &Arc<Mutex<MemRelay>>, other: &Arc<Mutex<MemRelay>>) -> Device {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    let signer = SoftSigner::new();
    let http = FakeHttp::new(relay, other);
    let phone = Phone::open(
        path.to_string_lossy().into_owned(),
        Box::new(signer.clone()),
        Box::new(http.clone()),
    )
    .unwrap();
    Device {
        _dir: dir,
        path,
        signer,
        http,
        phone,
    }
}

/// Show every frame of `from`'s current round to `to`'s camera.
/// The seat's signature is made when step three asks for it, as Swift does on `needs_signature`.
fn show(from: &Phone, from_id: &str, to: &Phone, to_id: &str) -> CeremonyState {
    if from.ceremony_state(from_id.into()).unwrap().needs_signature {
        from.ceremony_sign(from_id.into()).unwrap();
    }
    let mut last = None;
    for f in from.ceremony_frames(from_id.into()).unwrap() {
        last = Some(to.ceremony_scan(to_id.into(), f).unwrap());
    }
    last.unwrap()
}

fn show_unsigned(from: &Phone, from_id: &str, to: &Phone, to_id: &str) -> CeremonyState {
    let mut last = None;
    for f in from.ceremony_frames(from_id.into()).unwrap() {
        last = Some(to.ceremony_scan(to_id.into(), f).unwrap());
    }
    last.unwrap()
}

/// A starts, B joins from A's code, and they show each other codes until both are done.
/// Returns the two ceremony ids.
fn ceremony(a: &Device, b: &Device) -> (String, String) {
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, false).unwrap();
    let params_code = a.phone.ceremony_frames(ida.clone()).unwrap()[0].clone();
    assert!(params_code.starts_with("cahoots-params:0:"), "{params_code}");
    let idb = b.phone.join_ceremony(params_code, 1).unwrap();
    for _ in 0..4 {
        let sb = show(&a.phone, &ida, &b.phone, &idb);
        let sa = show(&b.phone, &idb, &a.phone, &ida);
        if sa.phase == CeremonyPhase::Complete && sb.phase == CeremonyPhase::Complete {
            break;
        }
    }
    (ida, idb)
}

fn finish(a: &Device, b: &Device) -> (String, String, String) {
    let (ida, idb) = ceremony(a, b);
    let sa = a.phone.ceremony_scan(ida.clone(), "nothing".into());
    // A stray scan is not a Cahoots code; it changes nothing.
    assert!(sa.is_ok() || matches!(sa, Err(PhoneError::BadCode { .. })));
    let sa = show(&b.phone, &idb, &a.phone, &ida);
    let sb = show(&a.phone, &ida, &b.phone, &idb);
    assert_eq!(sa.phase, CeremonyPhase::Complete);
    assert_eq!(sb.phase, CeremonyPhase::Complete);
    assert_eq!(sa.words.len(), 6);
    assert_eq!(sa.words, sb.words);
    let ca = a.phone.ceremony_keep(ida.clone()).unwrap();
    let cb = b.phone.ceremony_keep(idb.clone()).unwrap();
    assert_eq!(ca, cb);
    (ca, ida, idb)
}

fn texts(msgs: &[cahoots_phone::ChatMessage]) -> Vec<String> {
    msgs.iter().map(|m| m.text.clone()).collect()
}

fn relay() -> Arc<Mutex<MemRelay>> {
    Arc::new(Mutex::new(MemRelay::default()))
}

#[test]
fn two_phones_make_a_circle_and_chat() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    for d in [&a, &b] {
        d.phone.set_relay(id.clone(), RELAY.into()).unwrap();
        let list = d.phone.circles();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].name, "park");
        assert_eq!(list[0].members, 2);
        assert!(list[0].has_root);
        assert_eq!(list[0].relay, RELAY);
    }
    assert_eq!(a.phone.circles()[0].my_seat, 0);
    assert_eq!(b.phone.circles()[0].my_seat, 1);

    a.phone.send_chat(id.clone(), "hello from A".into()).unwrap();
    let got = b.phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["hello from A"]);
    assert_eq!(got[0].sender, 0);
    assert!(!got[0].mine);
    assert_eq!(b.phone.circles()[0].unread, 1);
    assert_eq!(texts(&b.phone.history(id.clone())), ["hello from A"]);
    assert_eq!(b.phone.circles()[0].unread, 0);

    b.phone.rotate(id.clone()).unwrap();
    b.phone.send_chat(id.clone(), "and back from B".into()).unwrap();
    let got = a.phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["and back from B"]);
    assert_eq!(got[0].epoch, 2);
    let hist = a.phone.history(id.clone());
    assert_eq!(texts(&hist), ["hello from A", "and back from B"]);
    assert!(hist[0].mine && !hist[0].pending);
    // A poll with nothing new returns nothing.
    assert!(a.phone.poll(id.clone()).unwrap().is_empty());

    // A phone opened again on the same files reads the relay before it sends.
    let again = Phone::open(
        a.path.to_string_lossy().into_owned(),
        Box::new(a.signer.clone()),
        Box::new(a.http.clone()),
    )
    .unwrap();
    again.send_chat(id.clone(), "after a restart".into()).unwrap();
    let got = b.phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["after a restart"]);
}

#[test]
fn a_ceremony_is_not_saved_until_the_words_match() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (ida, idb) = ceremony(&a, &b);
    let state = b.phone.ceremony_scan(idb.clone(), "nothing".into());
    assert!(state.is_ok() || matches!(state, Err(PhoneError::BadCode { .. })));
    a.phone.ceremony_discard(ida.clone());
    b.phone.ceremony_discard(idb.clone());
    for d in [&a, &b] {
        let circles = d.path.join("circles");
        let n = std::fs::read_dir(&circles).map(|r| r.count()).unwrap_or(0);
        assert_eq!(n, 0, "nothing under circles/");
        assert!(d.phone.circles().is_empty());
    }
    assert!(matches!(
        a.phone.ceremony_keep(ida),
        Err(PhoneError::NotFound { .. })
    ));
}

#[test]
fn a_message_posted_during_an_outage_is_sent_once() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    a.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    b.phone.set_relay(id.clone(), RELAY.into()).unwrap();

    a.http.fail_posts.store(2, Ordering::SeqCst);
    // Kept in the outbox: the send does not fail, the message waits.
    a.phone.send_chat(id.clone(), "through the outage".into()).unwrap();
    assert!(a.phone.history(id.clone())[0].pending);
    // A poll retries the outbox; the second post fails too.
    a.phone.poll(id.clone()).unwrap();
    assert!(a.phone.history(id.clone())[0].pending);
    // The third goes through.
    a.phone.poll(id.clone()).unwrap();
    assert!(!a.phone.history(id.clone())[0].pending);
    a.phone.poll(id.clone()).unwrap();

    let circle: [u8; 16] = hex::decode(&id).unwrap().try_into().unwrap();
    assert_eq!(r.lock().unwrap().poll(&circle, None, 0).unwrap().len(), 1);
    let got = b.phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["through the outage"]);
}

#[test]
fn a_post_that_landed_without_an_answer_is_not_sent_again() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    a.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    b.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    a.http.fail_posts.store(1, Ordering::SeqCst);
    a.phone.send_chat(id.clone(), "once".into()).unwrap();
    // The relay got it by another road (the answer was lost): the poll sees the same bytes
    // and the outbox lets the entry go instead of posting it again.
    let circle: [u8; 16] = hex::decode(&id).unwrap().try_into().unwrap();
    let wire = cahoots_phone::store::ChatRecord::load(&a.path.join("circles").join(&id))
        .unwrap()
        .outbox[0]
        .wire
        .to_vec();
    r.lock().unwrap().post(&circle, &wire).unwrap();
    a.phone.poll(id.clone()).unwrap();
    assert!(!a.phone.history(id.clone())[0].pending);
    assert_eq!(r.lock().unwrap().poll(&circle, None, 0).unwrap().len(), 1);
    assert_eq!(texts(&b.phone.poll(id).unwrap()), ["once"]);
}

#[test]
fn hello_words_match_across_phones() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    let wa = a.phone.hello_words(id.clone(), 1).unwrap();
    let wb = b.phone.hello_words(id.clone(), 0).unwrap();
    assert_eq!(wa.mine, wb.theirs);
    assert_eq!(wa.theirs, wb.mine);
    assert_eq!(wa.number, 0);
    let na = a.phone.next_hello(id.clone(), 1).unwrap();
    let nb = b.phone.next_hello(id.clone(), 0).unwrap();
    assert_eq!(na.number, 1);
    assert_eq!(na.mine, nb.theirs);
    assert_ne!(na.mine, wa.mine);
    // The number is kept.
    assert_eq!(a.phone.hello_words(id.clone(), 1).unwrap().number, 1);
    assert!(a.phone.hello_words(id, 0).is_err());
}

#[test]
fn export_history_has_no_secrets() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    a.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    b.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    a.phone.send_chat(id.clone(), "meet at noon".into()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    b.phone.send_chat(id.clone(), "see you there".into()).unwrap();
    a.phone.poll(id.clone()).unwrap();

    let text = a.phone.export_history(id.clone()).unwrap();
    assert!(text.contains("meet at noon"), "{text}");
    assert!(text.contains("see you there"), "{text}");
    assert!(text.contains("park"));
    let dir = a.path.join("circles").join(&id);
    let rec = CircleRecord::load(&dir).unwrap();
    let pad = std::fs::read(dir.join("pad.bin")).unwrap();
    for secret in [hex::encode(&rec.root[..]), URL_SAFE_NO_PAD.encode(&rec.root[..])] {
        assert!(!text.contains(&secret));
    }
    assert!(!text.contains(&hex::encode(&pad[..16])));
    assert!(!text.contains(&URL_SAFE_NO_PAD.encode(&pad[..16])));
    assert!(!text.contains(&String::from_utf8_lossy(&pad[..16]).into_owned()));
}

#[test]
fn the_seat_key_is_signed_by_the_callback() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, ida, idb) = finish(&a, &b);
    for (d, cid, seat) in [(&a, &ida, 0u8), (&b, &idb, 1u8)] {
        let label = format!("cahoots:seat:{cid}");
        let calls = d.signer.sign_calls();
        assert_eq!(calls, vec![label.clone()], "one signature per ceremony");
        let public = d.signer.public(&label).expect("made by make_key");
        let rec = CircleRecord::load(&d.path.join("circles").join(&id)).unwrap();
        assert_eq!(rec.seat_label, label);
        let manifest = SealedManifest::decode(&rec.manifest).unwrap().verify().unwrap();
        assert_eq!(
            manifest.seat_key(seat),
            Some(&SeatPublic::parse(&public).unwrap())
        );
        assert_eq!(manifest.seats[usize::from(seat)].tier, 0);
    }
    // The phone's own key was made once, at first open, and is named in describe().
    let info = a.phone.describe();
    assert_eq!(info.key_store, "seat key in the Secure Enclave, secret protected by iOS");
    assert!(a.signer.public("cahoots:phone").is_some());
    assert!(!info.fingerprint.is_empty());
}

#[test]
fn signing_while_locked_fails_cleanly() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, false).unwrap();
    let code = a.phone.ceremony_frames(ida.clone()).unwrap()[0].clone();
    let idb = b.phone.join_ceremony(code, 1).unwrap();
    a.signer.set_locked(true);
    show(&a.phone, &ida, &b.phone, &idb); // A's setup code
    let sa = show_unsigned(&b.phone, &idb, &a.phone, &ida); // B's reveal stands in for its setup code
    assert_eq!(sa.phase, CeremonyPhase::Attest);
    assert!(sa.needs_signature);
    // A's seat key is asked for its signature; the phone is locked.
    assert!(matches!(
        a.phone.ceremony_sign(ida.clone()),
        Err(PhoneError::Locked)
    ));
    assert!(matches!(
        a.phone.ceremony_keep(ida.clone()),
        Err(PhoneError::Ceremony { .. })
    ));
    let st = a.phone.ceremony_state(ida.clone()).unwrap();
    assert_eq!(st.phase, CeremonyPhase::Attest);
    assert!(st.needs_signature);
    assert!(a.signer.sign_calls().is_empty());
    // Still locked, A shows its reveal to B, who is on step two.
    let sb = show_unsigned(&a.phone, &ida, &b.phone, &idb);
    assert_eq!(sb.phase, CeremonyPhase::Attest);
    a.signer.set_locked(false);
    show(&a.phone, &ida, &b.phone, &idb);
    let sa = show(&b.phone, &idb, &a.phone, &ida);
    let sb = b.phone.ceremony_state(idb.clone()).unwrap();
    assert_eq!(sa.phase, CeremonyPhase::Complete);
    assert_eq!(sb.phase, CeremonyPhase::Complete);
    assert_eq!(sa.words, sb.words);
    let id = a.phone.ceremony_keep(ida.clone()).unwrap();
    assert_eq!(b.phone.ceremony_keep(idb).unwrap(), id);
    assert_eq!(a.signer.sign_calls().len(), 1, "the refused attempt is not counted");
}

#[test]
fn qr_codes_round_trip_through_the_camera_path() {
    let r = relay();
    let a = device(&r);
    let text = "cahoots-params:0:AQIABAAABHBhcms";
    let m = a.phone.qr_matrix(text.into()).unwrap();
    assert_eq!(m.bits.len(), (m.size * m.size) as usize);
    // Draw it at 4 pixels a module with a quiet zone and read it back.
    let scale = 4usize;
    let quiet = 4usize;
    let side = (m.size as usize + 2 * quiet) * scale;
    let mut gray = vec![255u8; side * side];
    for y in 0..m.size as usize {
        for x in 0..m.size as usize {
            if m.bits[y * m.size as usize + x] {
                for dy in 0..scale {
                    for dx in 0..scale {
                        gray[((y + quiet) * scale + dy) * side + (x + quiet) * scale + dx] = 0;
                    }
                }
            }
        }
    }
    assert_eq!(
        a.phone.decode_frame(side as u32, side as u32, gray),
        Some(text.to_string())
    );
    assert_eq!(a.phone.decode_frame(2, 2, vec![0; 3]), None);
}

#[test]
fn three_phones_pick_their_seats_and_chat() {
    let r = relay();
    let ds = [device(&r), device(&r), device(&r)];
    let ida = ds[0].phone.start_ceremony("trio".into(), 3, 0, PAD, false).unwrap();
    // B joins from A's setup line. C reads A's step-one animation, as a phone joining a
    // browser host does, and both pick their seats.
    let frames = ds[0].phone.ceremony_frames(ida.clone()).unwrap();
    assert_eq!(ds[1].phone.ceremony_members(frames[0].clone()).unwrap(), 3);
    let idb = ds[1].phone.join_ceremony(frames[0].clone(), 1).unwrap();
    let mut offer = None;
    for f in &frames[1..] {
        if let Some(o) = ds[2].phone.join_scan(f.clone()).unwrap() {
            offer = Some(o);
            break;
        }
    }
    let offer = offer.expect("the animation reads");
    let idc = ds[2].phone.join_ceremony(offer.params_code, 2).unwrap();
    let ids = [ida, idb, idc];
    let seats: Vec<u8> = (0..3)
        .map(|i| ds[i].phone.ceremony_state(ids[i].clone()).unwrap().my_seat)
        .collect();
    assert_eq!(seats, [0, 1, 2]);
    // Everyone shows everyone, round after round. Setup codes and reveals from two seats
    // have the same length, so the camera must keep their animations apart.
    for _ in 0..4 {
        for from in 0..3 {
            for to in 0..3 {
                if from != to {
                    show(&ds[from].phone, &ids[from], &ds[to].phone, &ids[to]);
                }
            }
        }
    }
    let words: Vec<Vec<String>> = (0..3)
        .map(|i| {
            let s = ds[i].phone.ceremony_state(ids[i].clone()).unwrap();
            assert_eq!(s.phase, CeremonyPhase::Complete);
            s.words
        })
        .collect();
    assert_eq!(words[0], words[1]);
    assert_eq!(words[1], words[2]);
    let cid: Vec<String> = (0..3)
        .map(|i| ds[i].phone.ceremony_keep(ids[i].clone()).unwrap())
        .collect();
    assert!(cid.iter().all(|c| *c == cid[0]));
    let id = cid[0].clone();
    for d in &ds {
        d.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    }
    ds[0].phone.send_chat(id.clone(), "one".into()).unwrap();
    ds[2].phone.rotate(id.clone()).unwrap();
    ds[2].phone.send_chat(id.clone(), "three".into()).unwrap();
    // B catches up on both, in order, across the epoch C started.
    let got = ds[1].phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["one", "three"]);
    assert_eq!(got[1].sender, 2);
}

#[test]
fn two_phones_on_one_seat_are_told() {
    let r = relay();
    let (a, b, c) = (device(&r), device(&r), device(&r));
    let ida = a.phone.start_ceremony("park".into(), 3, 0, PAD, false).unwrap();
    let code = a.phone.ceremony_frames(ida.clone()).unwrap()[0].clone();
    // B and C both pick seat 2.
    let idb = b.phone.join_ceremony(code.clone(), 1).unwrap();
    let idc = c.phone.join_ceremony(code, 1).unwrap();
    // B's own frames on B's camera change nothing.
    show(&b.phone, &idb, &b.phone, &idb);
    let mut refused = None;
    for f in c.phone.ceremony_frames(idc).unwrap() {
        if let Err(e) = b.phone.ceremony_scan(idb.clone(), f) {
            refused = Some(e);
        }
    }
    match refused {
        Some(PhoneError::Ceremony { message }) => assert!(message.contains("also seat 2"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn join_picks_a_seat_the_circle_has() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let ida = a.phone.start_ceremony("park".into(), 3, 0, PAD, false).unwrap();
    let frames = a.phone.ceremony_frames(ida).unwrap();
    let line = frames[0].clone();
    assert_eq!(b.phone.ceremony_members(line.clone()).unwrap(), 3);
    for seat in [0u8, 3, 9] {
        assert!(
            matches!(b.phone.join_ceremony(line.clone(), seat), Err(PhoneError::Ceremony { .. })),
            "seat {seat}"
        );
    }
    // A setup code read from the host's animation names the host's seat, which is taken.
    let offer = frames[1..]
        .iter()
        .find_map(|f| b.phone.join_scan(f.clone()).unwrap())
        .unwrap();
    assert_eq!(offer.members, 3);
    assert_eq!(offer.name, "park");
    assert!(offer.params_code.starts_with("cahoots-commit:0:"));
    let id = b.phone.join_ceremony(offer.params_code, 2).unwrap();
    // The host's setup code came with the offer.
    let st = b.phone.ceremony_state(id).unwrap();
    assert_eq!(st.my_seat, 2);
    assert_eq!(st.missing, vec![1]);
    // A step-two code is no way in.
    assert!(b.phone.join_scan("cahoots-reveal:1:AAAA".into()).is_err());
    // A name too long for the step-one code is refused in plain words.
    assert!(matches!(
        a.phone.start_ceremony("x".repeat(201), 2, 0, PAD, false),
        Err(PhoneError::Ceremony { .. })
    ));
    a.phone.start_ceremony("x".repeat(200), 2, 0, PAD, false).unwrap();
}

fn frame(oti: [u8; 12], body: &[u8]) -> String {
    let mut p = b"CHQ1".to_vec();
    p.extend_from_slice(&oti);
    p.extend_from_slice(body);
    URL_SAFE_NO_PAD.encode(p)
}

#[test]
fn a_hostile_code_is_ignored() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, false).unwrap();
    let frames = a.phone.ceremony_frames(ida.clone()).unwrap();
    let idb = b.phone.join_ceremony(frames[0].clone(), 1).unwrap();
    let good: [u8; 12] = URL_SAFE_NO_PAD.decode(&frames[1]).unwrap()[4..16]
        .try_into()
        .unwrap();
    let mut hostile = Vec::new();
    // Symbol size zero, no source blocks, no sub-blocks, a length near 2^40, alignment zero.
    for (at, bytes) in [
        (6usize, vec![0u8, 0]),
        (8, vec![0]),
        (9, vec![0, 0]),
        (0, vec![0xff, 0xff, 0xff, 0xff, 0xff]),
        (11, vec![0]),
    ] {
        let mut oti = good;
        oti[at..at + bytes.len()].copy_from_slice(&bytes);
        hostile.push(frame(oti, &[0u8; 304]));
    }
    // A good header with a symbol of the wrong size, or a block that does not exist.
    hostile.push(frame(good, &[0u8; 10]));
    hostile.push(frame(good, &[[9u8, 0, 0, 0].as_slice(), &[0u8; 296]].concat()));
    hostile.push(frame([0u8; 12], &[0u8; 8]));
    hostile.push("CHQ1".into());
    hostile.push("not a code at all".into());
    hostile.push(URL_SAFE_NO_PAD.encode([0u8; 3]));
    // Many distinct lengths, each one frame: the partial transfers kept stay few.
    for len in 1..200u64 {
        let mut oti = good;
        oti[..5].copy_from_slice(&len.to_be_bytes()[3..]);
        hostile.push(frame(oti, &[0u8; 304]));
    }
    for h in &hostile {
        let _ = b.phone.ceremony_scan(idb.clone(), h.clone());
        let _ = b.phone.join_scan(h.clone());
    }
    assert_eq!(b.phone.decode_frame(64, 64, vec![0x5a; 64 * 64]), None);
    assert_eq!(b.phone.decode_frame(0, 0, Vec::new()), None);
    assert_eq!(b.phone.decode_frame(u32::MAX, 2, vec![0; 4]), None);
    // The ceremony carries on.
    let sb = show(&a.phone, &ida, &b.phone, &idb);
    assert_eq!(sb.phase, CeremonyPhase::Reveal);
}

#[test]
fn a_history_line_saved_with_the_state_is_written_once() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    a.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    a.phone.send_chat(id.clone(), "kept".into()).unwrap();
    // As if the app stopped after the chat state was saved and before the log was written:
    // the journal holds the line just written and a line the log lacks.
    let dir = a.path.join("circles").join(&id);
    let mut chat = cahoots_phone::store::ChatRecord::load(&dir).unwrap();
    let mut lost = cahoots_phone::store::read_messages(&dir).unwrap();
    chat.journal.push(lost[0].clone());
    lost[0].id = "lost".into();
    lost[0].text = "saved with the state".into();
    chat.journal.push(lost.remove(0));
    chat.save(&dir).unwrap();
    let again = Phone::open(
        a.path.to_string_lossy().into_owned(),
        Box::new(a.signer.clone()),
        Box::new(a.http.clone()),
    )
    .unwrap();
    assert_eq!(texts(&again.history(id)), ["kept", "saved with the state"]);
}

#[test]
fn a_record_that_cannot_be_read_is_not_written_over() {
    let r = relay();
    let a = device(&r);
    let path = a.path.join("phone.cbor");
    std::fs::write(&path, b"locked").unwrap();
    let opened = Phone::open(
        a.path.to_string_lossy().into_owned(),
        Box::new(a.signer.clone()),
        Box::new(a.http.clone()),
    );
    assert!(matches!(opened, Err(PhoneError::Storage { .. })));
    assert_eq!(std::fs::read(&path).unwrap(), b"locked");
}

/// One second of the app's loop: tick, and sign when step three asks, as Swift does.
fn tick(d: &Device, id: &str) -> CeremonyState {
    let s = d.phone.ceremony_tick(id.into()).unwrap();
    if s.needs_signature {
        d.phone.ceremony_sign(id.into()).unwrap();
        return d.phone.ceremony_tick(id.into()).unwrap();
    }
    s
}

/// The host scans every joiner's code it can see, and taps Continue once everyone is scanned.
fn host_scans(host: &Device, id: &str, joiners: &[(&Device, &str)]) {
    for (d, jid) in joiners {
        if let Some(code) = d.phone.ceremony_state((*jid).into()).unwrap().my_join_code {
            host.phone.ceremony_scan(id.into(), code).unwrap();
        }
    }
    if host.phone.ceremony_state(id.into()).unwrap().needs_continue {
        host.phone.ceremony_continue(id.into()).unwrap();
    }
}

#[test]
fn two_phones_hold_a_ceremony_through_the_relay() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    assert_eq!(a.phone.default_relay(), "https://relay.cahoots.network");
    // Only the host sets the relay. The joiner takes it from the host's code.
    a.phone.set_default_relay(RELAY.into()).unwrap();
    assert!(a.phone.set_default_relay("relay.test".into()).is_err());
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, true).unwrap();
    let offer = a.phone.offer_code(ida.clone()).unwrap();
    assert!(offer.starts_with("cahoots:ceremony:v1:"), "{offer}");
    // The offer fits one small code.
    assert!(a.phone.qr_matrix(offer.clone()).unwrap().size <= 49, "{} characters", offer.len());
    // Scanned before joining: the circle and its size, to pick a seat.
    let seen = b.phone.join_scan(offer.clone()).unwrap().unwrap();
    assert!(seen.through_relay);
    assert_eq!((seen.members, seen.name.as_str()), (2, "park"));
    assert_eq!(b.phone.ceremony_members(seen.params_code.clone()).unwrap(), 2);
    assert!(matches!(
        b.phone.join_offer(offer.clone(), 0),
        Err(PhoneError::Ceremony { .. })
    ));
    let idb = b.phone.join_offer(seen.params_code, 1).unwrap();
    let sb = b.phone.ceremony_state(idb.clone()).unwrap();
    assert!(sb.through_relay);
    assert_eq!(sb.my_seat, 1);

    let (mut sa, mut sb) = (None, None);
    for _ in 0..6 {
        sa = Some(tick(&a, &ida));
        sb = Some(tick(&b, &idb));
        host_scans(&a, &ida, &[(&b, &idb)]);
    }
    let (sa, sb) = (sa.unwrap(), sb.unwrap());
    assert_eq!(sa.phase, CeremonyPhase::Complete);
    assert_eq!(sb.phase, CeremonyPhase::Complete);
    assert_eq!(sa.words.len(), 6);
    assert_eq!(sa.words, sb.words);
    // Each seat signed once.
    assert_eq!(a.signer.sign_calls().len(), 1);
    let ca = a.phone.ceremony_keep(ida).unwrap();
    let cb = b.phone.ceremony_keep(idb).unwrap();
    assert_eq!(ca, cb);
    // The circle chats through the relay the ceremony used, with no address entered.
    assert_eq!(a.phone.circles()[0].relay, RELAY);
    assert_eq!(b.phone.circles()[0].relay, RELAY);
    a.phone.send_chat(ca.clone(), "made through the relay".into()).unwrap();
    assert_eq!(texts(&b.phone.poll(cb).unwrap()), ["made through the relay"]);
}

#[test]
fn a_phone_joins_a_browser_offer() {
    use cahoots_wasm::{b64_decode, WasmCeremony};
    let r = relay();
    // The phone's own default is not the browser's relay: it uses the one in the code.
    let p = device(&r);
    let contribution: Vec<u8> = (0..PAD).map(|i| (i * 31) as u8).collect();
    let mut browser = WasmCeremony::new(
        2, PAD, "mixed", false, 0, &contribution, None, &[0x41; 32], &[0x61; 32], 0,
    )
    .unwrap();
    let offer = browser.offer(RELAY).unwrap();
    let id: [u8; 16] = hex::decode(browser.channel_hex().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let idp = p.phone.join_offer(offer, 1).unwrap();
    let mut cursor = None;
    let mut state = None;
    for _ in 0..6 {
        for w in browser.outbound().unwrap() {
            r.lock().unwrap().post(&id, &b64_decode(&w).unwrap()).unwrap();
            browser.posted(&w);
        }
        for (seq, m) in r.lock().unwrap().poll(&id, cursor, 0).unwrap() {
            browser.inbound(&m).unwrap();
            cursor = Some(seq);
        }
        state = Some(tick(&p, &idp));
        // The browser host scans the phone's code.
        if let Some(code) = state.as_ref().unwrap().my_join_code.clone() {
            browser.scan_join(&code).unwrap();
        }
        if browser.needs_continue() {
            browser.proceed();
        }
    }
    let state = state.unwrap();
    assert_eq!(browser.phase(), "complete");
    assert_eq!(state.phase, CeremonyPhase::Complete);
    let words = browser.finish("CSPRNG").unwrap().words();
    assert_eq!(state.words.join(" "), words);
}

#[test]
fn a_relay_ceremony_survives_a_failed_post() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    for d in [&a, &b] {
        d.phone.set_default_relay(RELAY.into()).unwrap();
    }
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, true).unwrap();
    let idb = b.phone.join_offer(a.phone.offer_code(ida.clone()).unwrap(), 1).unwrap();
    a.http.fail_posts.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        a.phone.ceremony_tick(ida.clone()),
        Err(PhoneError::Network { .. })
    ));
    for _ in 0..6 {
        tick(&a, &ida);
        tick(&b, &idb);
        host_scans(&a, &ida, &[(&b, &idb)]);
    }
    assert_eq!(
        a.phone.ceremony_state(ida.clone()).unwrap().words,
        b.phone.ceremony_state(idb).unwrap().words
    );
    // Nothing was posted twice: two commitments, two reveals, two signatures.
    let id = a.phone.offer_code(ida).unwrap();
    let channel = cahoots_core::channel::Channel::from_offer(&id).unwrap().channel;
    assert_eq!(r.lock().unwrap().poll(&channel.id, None, 0).unwrap().len(), 6);
    // A codes-only ceremony has no offer.
    let idc = a.phone.start_ceremony("codes".into(), 2, 0, PAD, false).unwrap();
    assert!(a.phone.offer_code(idc.clone()).is_err());
    assert!(a.phone.ceremony_tick(idc).is_err());
}

#[test]
fn the_device_that_lost_a_seat_posts_nothing_more() {
    let r = relay();
    let (a, b, c) = (device(&r), device(&r), device(&r));
    a.phone.set_default_relay(RELAY.into()).unwrap();
    let ida = a.phone.start_ceremony("park".into(), 3, 0, PAD, true).unwrap();
    let offer = a.phone.offer_code(ida.clone()).unwrap();
    let channel = cahoots_core::channel::Channel::from_offer(&offer).unwrap().channel;
    // B and C both pick seat 2. B's setup reaches the relay first.
    let idb = b.phone.join_offer(offer.clone(), 1).unwrap();
    let idc = c.phone.join_offer(offer, 1).unwrap();
    tick(&b, &idb);
    match c.phone.ceremony_tick(idc.clone()) {
        Err(PhoneError::Ceremony { message }) => {
            assert_eq!(message, "Seat 2 was taken by someone else. Leave and join again with a free seat.")
        }
        other => panic!("{other:?}"),
    }
    assert!(c.phone.ceremony_state(idc.clone()).unwrap().seat_lost);
    // B keeps the seat, and nobody else sees an error.
    let sb = tick(&b, &idb);
    assert!(!sb.seat_lost);
    tick(&a, &ida);
    host_scans(&a, &ida, &[(&b, &idb), (&c, &idc)]);
    let sa = tick(&a, &ida);
    assert_eq!(sa.missing, vec![2]);
    assert_eq!(sa.scanned, vec![1]);
    // A third person takes seat 3, and the ceremony finishes without C.
    let d = device(&r);
    let idd = d.phone.join_offer(a.phone.offer_code(ida.clone()).unwrap(), 2).unwrap();
    for _ in 0..6 {
        tick(&a, &ida);
        tick(&b, &idb);
        tick(&d, &idd);
        host_scans(&a, &ida, &[(&b, &idb), (&d, &idd)]);
        assert!(c.phone.ceremony_tick(idc.clone()).is_err());
        assert!(c.phone.ceremony_state(idc.clone()).unwrap().my_join_code.is_none());
    }
    let words: Vec<_> = [(&a, &ida), (&b, &idb), (&d, &idd)]
        .iter()
        .map(|(x, id)| x.phone.ceremony_state((*id).clone()).unwrap())
        .map(|s| {
            assert_eq!(s.phase, CeremonyPhase::Complete);
            s.words
        })
        .collect();
    assert_eq!(words[0], words[1]);
    assert_eq!(words[0], words[2]);
    // C posted its setup and nothing else: three setups, three reveals, three signatures, and C's one.
    assert_eq!(r.lock().unwrap().poll(&channel.id, None, 0).unwrap().len(), 10);
}

#[test]
fn a_lost_ceremony_says_to_start_over() {
    let r = relay();
    let a = device(&r);
    match a.phone.ceremony_tick("gone".into()) {
        Err(PhoneError::NotFound { message }) => assert_eq!(
            message,
            "Setup couldn’t be restored after Cahoots closed. Ask everyone to start a new ceremony and scan the new code."
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn two_phones_finish_only_after_the_host_scans() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    a.phone.set_default_relay(RELAY.into()).unwrap();
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, true).unwrap();
    let idb = b.phone.join_offer(a.phone.offer_code(ida.clone()).unwrap(), 1).unwrap();
    assert!(a.phone.ceremony_state(ida.clone()).unwrap().my_join_code.is_none(), "the host shows the offer");
    for _ in 0..4 {
        tick(&a, &ida);
        tick(&b, &idb);
    }
    // B's commitment is on the relay, but A has not scanned B: A stays in step one.
    let sa = a.phone.ceremony_state(ida.clone()).unwrap();
    assert_eq!(sa.phase, CeremonyPhase::Commit);
    assert_eq!(sa.unscanned, vec![1]);
    assert!(sa.scanned.is_empty() && !sa.needs_continue);
    assert!(a.phone.ceremony_continue(ida.clone()).is_err());
    let code = b.phone.ceremony_state(idb.clone()).unwrap().my_join_code.unwrap();
    assert!(code.starts_with("cahoots:joined:v1:"), "{code}");
    // A joiner cannot take another joiner's code.
    assert!(matches!(b.phone.ceremony_scan(idb.clone(), code.clone()), Err(PhoneError::BadCode { .. })));
    let sa = a.phone.ceremony_scan(ida.clone(), code).unwrap();
    assert_eq!(sa.scanned, vec![1]);
    assert!(sa.unscanned.is_empty());
    assert!(sa.needs_continue);
    // Nothing moves until Continue: A posts no reveal, and asks for no signature.
    for _ in 0..3 {
        assert!(!tick(&a, &ida).needs_signature);
        tick(&b, &idb);
    }
    assert_eq!(b.phone.ceremony_state(idb.clone()).unwrap().phase, CeremonyPhase::Reveal);
    assert!(b.phone.ceremony_state(idb.clone()).unwrap().my_join_code.is_some());
    a.phone.ceremony_continue(ida.clone()).unwrap();
    for _ in 0..4 {
        tick(&a, &ida);
        tick(&b, &idb);
    }
    let (sa, sb) = (
        a.phone.ceremony_state(ida).unwrap(),
        b.phone.ceremony_state(idb).unwrap(),
    );
    assert_eq!(sa.phase, CeremonyPhase::Complete);
    assert_eq!(sb.phase, CeremonyPhase::Complete);
    assert_eq!(sa.words, sb.words);
    assert!(sb.my_join_code.is_none());
}

#[test]
fn a_joiner_checks_the_host_against_the_offer() {
    use cahoots_core::channel::{Channel, Mailbox, Round};
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    a.phone.set_default_relay(RELAY.into()).unwrap();
    let ida = a.phone.start_ceremony("park".into(), 2, 0, PAD, true).unwrap();
    // The offer is ready before the host has ticked, and it binds the host's commitment.
    let offer = a.phone.offer_code(ida.clone()).unwrap();
    let parsed = Channel::from_offer(&offer).unwrap();
    // An intruder with the offer posts its own seat-0 commitment before the host's lands.
    let fake = cahoots_wasm::WasmCeremony::new(2, PAD, "park", false, 0, &vec![7u8; PAD as usize], None, &[0x49; 32], &[0x69; 32], 0).unwrap();
    let mut box0 = Mailbox::new(parsed.channel.clone(), 0);
    box0.queue(Round::Commit, &fake.commitment(), &mut rand_core::OsRng);
    r.lock().unwrap().post(&parsed.channel.id, &box0.unposted()[0]).unwrap();
    let idb = b.phone.join_offer(offer, 1).unwrap();
    let sb = tick(&b, &idb);
    assert!(sb.host_mismatch);
    assert_eq!(sb.phase, CeremonyPhase::Commit);
    // The host keeps its seat, and the joiner takes the host's real commitment.
    for _ in 0..6 {
        tick(&a, &ida);
        tick(&b, &idb);
        host_scans(&a, &ida, &[(&b, &idb)]);
    }
    let (sa, sb) = (
        a.phone.ceremony_state(ida).unwrap(),
        b.phone.ceremony_state(idb).unwrap(),
    );
    assert!(!sa.seat_lost);
    assert_eq!(sa.phase, CeremonyPhase::Complete);
    assert_eq!(sb.phase, CeremonyPhase::Complete);
    assert_eq!(sa.words, sb.words);
}

#[test]
fn the_roster_lists_every_seat_with_its_tier() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    let seen: Vec<(u8, u8, bool)> = a
        .phone
        .circle_members(id.clone())
        .unwrap()
        .iter()
        .map(|m| (m.seat, m.tier, m.mine))
        .collect();
    assert_eq!(seen, [(0, 0, true), (1, 0, false)]);
    let theirs = b.phone.circle_members(id).unwrap();
    assert!(!theirs[0].mine && theirs[1].mine);
    assert!(matches!(a.phone.circle_members("gone".into()), Err(PhoneError::NotFound { .. })));
}

#[test]
fn a_relay_address_is_a_plain_http_url() {
    let r = relay();
    let a = device(&r);
    for good in [
        "https://relay.cahoots.network",
        "https://relay.cahoots.network/",
        "http://127.0.0.1:8787",
        "http://localhost:8080/",
    ] {
        a.phone.set_default_relay(good.into()).unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    assert_eq!(a.phone.default_relay(), "http://localhost:8080");
    for bad in [
        "http://127.0.0.1:8787network",
        "ftp://relay.cahoots.network",
        "https://",
        "https://relay.cahoots.network/v1",
        "https://relay.cahoots.network/?a=1",
        "https://relay.cahoots.network/#x",
        "relay.cahoots.network",
        "https://user@relay.cahoots.network",
    ] {
        assert!(
            matches!(a.phone.set_default_relay(bad.into()), Err(PhoneError::Network { .. })),
            "{bad} was taken"
        );
    }
    assert_eq!(a.phone.default_relay(), "http://localhost:8080");
}

#[test]
fn the_key_store_sentence_says_where_the_key_is() {
    let r = relay();
    let a = device(&r);
    assert!(a.phone.describe().key_store.contains("Secure Enclave"));
    a.phone.set_enclave(false);
    let s = a.phone.describe().key_store;
    assert!(!s.contains("Secure Enclave") && s.contains("keychain"), "{s}");
}

#[test]
fn faces_are_stable_and_differ_between_seats() {
    use sha2::{Digest, Sha256};
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, ida, _) = finish(&a, &b);

    // Without cards, another seat's seed is SHA-256 of the domain and that seat's key. A
    // phone's own seat shows the face it will publish.
    let seat0 = b.phone.seat_face_seed(id.clone(), 0).unwrap();
    let seat1 = a.phone.seat_face_seed(id.clone(), 1).unwrap();
    assert_eq!(seat0.len(), 32);
    assert_ne!(seat0, seat1);
    assert_eq!(a.phone.seat_face_seed(id.clone(), 0).unwrap(), a.phone.my_face_seed());
    assert_eq!(b.phone.seat_face_seed(id.clone(), 1).unwrap(), b.phone.my_face_seed());
    let key = a.signer.public(&format!("cahoots:seat:{ida}")).unwrap();
    let want = Sha256::new().chain_update(b"cahoots/face/v1").chain_update(&key).finalize();
    assert_eq!(seat0, want.to_vec());
    assert!(matches!(a.phone.seat_face_seed(id.clone(), 2), Err(PhoneError::NotFound { .. })));
    assert!(matches!(a.phone.seat_face_seed("gone".into(), 0), Err(PhoneError::NotFound { .. })));

    // The phone's own seed is drawn at random and kept, not taken from its key.
    let mine = a.phone.my_face_seed();
    assert_eq!(mine.len(), 32);
    let phone_key = a.signer.public("cahoots:phone").unwrap();
    let from_key = Sha256::new().chain_update(b"cahoots/face/v1").chain_update(&phone_key).finalize();
    assert_ne!(mine, from_key.to_vec());
    assert_eq!(a.phone.my_face_seed(), mine);
    assert_ne!(mine, b.phone.my_face_seed());

    // A face is 576 cells of 0 or 1, the same every time, matching the core crate.
    let face0 = a.phone.face_pixels(seat0.clone());
    assert_eq!(face0.len(), 576);
    assert!(face0.iter().all(|&c| c <= 1));
    assert_eq!(b.phone.face_pixels(seat0.clone()), face0);
    let rows = cahoots_core::faces::rows(&seat0);
    for (i, &c) in face0.iter().enumerate() {
        assert_eq!(c == 1, rows[i / 24][i % 24] == b'#');
    }
    assert_ne!(a.phone.face_pixels(seat1), face0);
    // The seat named "seat" in the fixture draws the same through the phone.
    assert_eq!(
        a.phone.face_pixels(b"seat".to_vec()),
        cahoots_core::faces::pixels(b"seat").iter().flatten().map(|&p| u8::from(p)).collect::<Vec<_>>()
    );
}

fn message(e: PhoneError) -> String {
    match e {
        PhoneError::Ceremony { message } | PhoneError::BadCode { message } => message,
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_host_with_two_badges_makes_an_offer_that_says_so() {
    let r = relay();
    let a = device(&r);
    let id = a.phone.start_ceremony("pile".into(), 4, 2, 16384, true).unwrap();
    let code = a.phone.offer_code(id.clone()).unwrap();
    assert_eq!(cahoots_core::channel::Channel::from_offer(&code).unwrap().badges, 2);
    assert_eq!(a.phone.ceremony_state(id).unwrap().badges, 2);
    assert_eq!(a.phone.join_scan(code).unwrap().unwrap().badges, 2);
}

#[test]
fn a_joiner_cannot_take_a_badge_seat() {
    let r = relay();
    let a = device(&r);
    let b = device(&r);
    let id = a.phone.start_ceremony("pile".into(), 4, 2, 16384, true).unwrap();
    let code = a.phone.offer_code(id).unwrap();
    let e = b.phone.join_offer(code.clone(), 1).unwrap_err();
    assert_eq!(message(e), "Seats 2 to 3 are cards in the pile. Pick a seat above them.");
    b.phone.join_offer(code, 3).unwrap();
}

#[test]
fn one_card_is_named_as_one_seat() {
    let r = relay();
    let a = device(&r);
    let b = device(&r);
    let id = a.phone.start_ceremony("pile".into(), 3, 1, 16384, true).unwrap();
    let code = a.phone.offer_code(id).unwrap();
    let e = b.phone.join_offer(code.clone(), 1).unwrap_err();
    assert_eq!(message(e), "Seat 2 is a card in the pile. Pick a seat above it.");
    b.phone.join_offer(code, 2).unwrap();
}

#[test]
fn a_circle_with_cards_keeps_a_small_secret() {
    let r = relay();
    let a = device(&r);
    let e = a.phone.start_ceremony("x".into(), 3, 1, 65536, true).unwrap_err();
    assert_eq!(message(e), "A circle with cards keeps a small secret, 16 KiB. Choose Small.");
    a.phone.start_ceremony("x".into(), 3, 1, 16384, true).unwrap();
    // Without cards, a larger secret is fine.
    a.phone.start_ceremony("x".into(), 3, 0, 65536, true).unwrap();
}

#[test]
fn too_many_badges_is_refused_before_a_code_exists() {
    let r = relay();
    let a = device(&r);
    let e = a.phone.start_ceremony("x".into(), 3, 3, 16384, true).unwrap_err();
    assert!(message(e).contains("at most 2 cards"));
    let e = a.phone.start_ceremony("x".into(), 3, 1, 16384, false).unwrap_err();
    assert!(message(e).contains("Turn the relay on"));
}

#[test]
fn a_v1_code_joins_as_a_circle_without_cards() {
    // A v1 code is what a host made before cards, or makes now with none.
    use cahoots_core::channel::{commitment_hash, Channel};
    use cahoots_core::params::CircleParams;
    let r = relay();
    let a = device(&r);
    let params = CircleParams::new(3, 16384, "park", false).unwrap();
    let channel = Channel::new(&mut rand_core::OsRng);
    let text = channel.offer(&params, RELAY, &commitment_hash(b"host"), 0).unwrap();
    assert!(text.starts_with("cahoots:ceremony:v1:"), "{text}");
    let seen = a.phone.join_scan(text.clone()).unwrap().unwrap();
    assert_eq!((seen.members, seen.badges), (3, 0));
    assert!(seen.through_relay);
    // Seat 1, which a card would hold in a v2 code, is a phone's seat here.
    let id = a.phone.join_offer(text, 1).unwrap();
    let state = a.phone.ceremony_state(id).unwrap();
    assert_eq!(state.badges, 0);
}

#[test]
fn a_newer_offer_is_explained() {
    let r = relay();
    let a = device(&r);
    let e = a.phone.join_scan("cahoots:ceremony:v3:AAAA".into()).unwrap_err();
    assert!(message(e).contains("newer version"));
    let e = a.phone.join_offer("cahoots:ceremony:v9:AAAA".into(), 1).unwrap_err();
    assert!(message(e).contains("newer version"));
}

#[test]
fn a_reset_by_one_phone_moves_the_other() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    for d in [&a, &b] {
        d.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    }
    a.phone.send_chat(id.clone(), "before".into()).unwrap();
    assert_eq!(texts(&b.phone.poll(id.clone()).unwrap()), ["before"]);

    a.phone.bump_chat_generation(id.clone()).unwrap();
    assert!(!a.phone.chat_reset_waiting(id.clone()));
    let mine = a.phone.history(id.clone());
    assert_eq!(mine.last().unwrap().kind, "bump");
    assert_eq!(mine.last().unwrap().text, "Keys were reset by seat 1.");

    a.phone.send_chat(id.clone(), "after".into()).unwrap();
    let got = b.phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["Keys were reset by seat 1.", "after"]);
    assert_eq!(got[0].kind, "bump");
    assert_eq!(got[1].kind, "text");
    // And the other way, in the new keys.
    b.phone.send_chat(id.clone(), "back".into()).unwrap();
    assert_eq!(texts(&a.phone.poll(id.clone()).unwrap()), ["back"]);
    // The history keeps the line in its place, after a restart too.
    let again = Phone::open(
        b.path.to_string_lossy().into_owned(),
        Box::new(b.signer.clone()),
        Box::new(b.http.clone()),
    )
    .unwrap();
    assert_eq!(
        texts(&again.history(id)),
        ["before", "Keys were reset by seat 1.", "after", "back"]
    );
}

#[test]
fn a_reset_the_relay_has_not_taken_is_posted_again_and_not_doubled() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    for d in [&a, &b] {
        d.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    }
    a.phone.send_chat(id.clone(), "before".into()).unwrap();
    b.phone.poll(id.clone()).unwrap();

    a.http.fail_posts.store(2, Ordering::SeqCst);
    a.phone.bump_chat_generation(id.clone()).unwrap();
    assert!(a.phone.chat_reset_waiting(id.clone()));
    // A message sent now waits behind the reset. The one failed post left is the retry here.
    a.phone.send_chat(id.clone(), "after".into()).unwrap();
    assert!(a.phone.chat_reset_waiting(id.clone()));
    // A second reset is refused while the first waits (and the relay is still failing).
    a.http.fail_posts.store(1, Ordering::SeqCst);
    let e = a.phone.bump_chat_generation(id.clone()).unwrap_err();
    assert!(matches!(&e, PhoneError::Chat { message } if message == "A reset is still waiting for the relay."), "{e:?}");
    // The relay answers: the reset goes, then the message after it.
    a.phone.poll(id.clone()).unwrap();
    assert!(!a.phone.chat_reset_waiting(id.clone()));
    let circle: [u8; 16] = hex::decode(&id).unwrap().try_into().unwrap();
    assert_eq!(r.lock().unwrap().poll(&circle, None, 0).unwrap().len(), 3);
    assert_eq!(
        texts(&b.phone.poll(id.clone()).unwrap()),
        ["Keys were reset by seat 1.", "after"]
    );
    // Reading its own copy back does not post it again.
    a.phone.poll(id).unwrap();
    assert_eq!(r.lock().unwrap().poll(&circle, None, 0).unwrap().len(), 3);
}

#[test]
fn history_saved_before_generations_still_opens() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chat/tests/fixtures");
    let state = std::fs::read(fixtures.join("chat-v5.bin")).unwrap();
    let blob = std::fs::read(fixtures.join("chat-v5-wires.bin")).unwrap();
    let mut wires = Vec::new();
    let mut at = 0;
    while at < blob.len() {
        let n = u32::from_le_bytes(blob[at..at + 4].try_into().unwrap()) as usize;
        wires.push(blob[at + 4..at + 4 + n].to_vec());
        at += 4 + n;
    }
    assert_eq!(wires.len(), 2);

    let r = relay();
    let d = device(&r);
    let circle_id = [0x0cu8; 16];
    let id = hex::encode(circle_id);
    let params = cahoots_core::params::CircleParams {
        n: 3,
        pad_len: PAD,
        name: "old".into(),
        agent_seat: false,
    };
    let record = CircleRecord {
        params: params.encode().into(),
        circle_id: circle_id.to_vec().into(),
        seat: 1,
        tier: 0,
        label: "CSPRNG".into(),
        words: Vec::new(),
        manifest: Vec::new().into(),
        root: vec![0x42; 32].into(),
        seat_label: "cahoots:seat:old".into(),
        seat_public: vec![2; 33].into(),
        pair_keys: vec![Vec::new().into(); 3],
        pair_counters: vec![0; 3],
    };
    let chat = cahoots_phone::store::ChatRecord {
        state: state.into(),
        outbox: Vec::new(),
        journal: Vec::new(),
        pending_bump: None,
        pending_card: None,
        card_due: false,
    };
    cahoots_phone::store::create_circle(
        &d.path,
        &id,
        &cahoots_phone::store::NewCircle {
            record: &record,
            pad: &[0; 16],
            chat: &chat,
            settings: &cahoots_phone::store::SettingsRecord {
                relay: RELAY.into(),
                unread: 0,
                face_override: None,
            },
            slices: params.slices(),
        },
    )
    .unwrap();
    let phone = Phone::open(
        d.path.to_string_lossy().into_owned(),
        Box::new(d.signer.clone()),
        Box::new(d.http.clone()),
    )
    .unwrap();
    for w in &wires {
        r.lock().unwrap().post(&circle_id, w).unwrap();
    }
    // The state had read the first message already: the relay's copy of it is a replay, the
    // second is new, and it opens in the root epoch.
    let got = phone.poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["old history two"]);
    assert_eq!(got[0].kind, "text");
    assert_eq!(got[0].epoch, 1);
    phone.send_chat(id.clone(), "new keys".into()).unwrap();
    assert_eq!(phone.history(id).last().unwrap().text, "new keys");
}

/// Two phones that read and post to the test relay from the ceremony on.
fn named(r: &Arc<Mutex<MemRelay>>, names: [&str; 2]) -> (Device, Device) {
    named_on(r, &relay(), names)
}

/// The same, with `other` behind `RELAY2` for both.
fn named_on(
    r: &Arc<Mutex<MemRelay>>,
    other: &Arc<Mutex<MemRelay>>,
    names: [&str; 2],
) -> (Device, Device) {
    let (a, b) = (device_on(r, other), device_on(r, other));
    for (d, n) in [(&a, names[0]), (&b, names[1])] {
        d.phone.set_default_relay(RELAY.into()).unwrap();
        if !n.is_empty() {
            d.phone.set_my_name(n.into()).unwrap();
        }
    }
    (a, b)
}

fn member(d: &Device, circle: &str, seat: u8) -> cahoots_phone::Member {
    d.phone.circle_members(circle.into()).unwrap()[usize::from(seat)].clone()
}

#[test]
fn after_a_ceremony_both_phones_publish_cards_and_see_each_other() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    assert_eq!(a.phone.my_name(), "Ada");
    let (id, _, _) = finish(&a, &b);
    // A kept first and published before B's circle existed; each poll reads the other's card.
    a.phone.poll(id.clone()).unwrap();
    let got = b.phone.poll(id.clone()).unwrap();
    assert!(got.is_empty(), "a card is not a message: {got:?}");
    a.phone.poll(id.clone()).unwrap();

    for (d, other, other_seat) in [(&a, &b, 1), (&b, &a, 0)] {
        let m = member(d, &id, other_seat);
        assert_eq!(m.name, other.phone.my_name());
        assert!(m.has_card);
        assert_eq!(m.device, Some(0), "a phone");
        assert_eq!(m.face_seed, other.phone.my_face_seed());
        assert_eq!(
            d.phone.seat_face_seed(id.clone(), other_seat).unwrap(),
            other.phone.my_face_seed()
        );
        // Its own seat shows the card it published.
        let mine = member(d, &id, 1 - other_seat);
        assert!(mine.mine && mine.has_card);
        assert_eq!(mine.name, d.phone.my_name());
        assert_eq!(mine.face_seed, d.phone.my_face_seed());
        // No history line for a card.
        assert!(d.phone.history(id.clone()).is_empty());
    }
    // The device's face seed is 32 random bytes, kept across opens.
    let seed = a.phone.my_face_seed();
    assert_eq!(seed.len(), 32);
    assert_ne!(seed, b.phone.my_face_seed());
    let again = Phone::open(
        a.path.to_string_lossy().into_owned(),
        Box::new(a.signer.clone()),
        Box::new(a.http.clone()),
    )
    .unwrap();
    assert_eq!(again.my_face_seed(), seed);
    assert_eq!(again.my_name(), "Ada");
    assert_eq!(again.circle_members(id.clone()).unwrap()[1].name, "Bea");
}

#[test]
fn set_my_name_republishes_and_the_other_side_updates() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (id, _, _) = finish(&a, &b);
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada");

    a.phone.set_my_name("  Ada King  ".into()).unwrap();
    assert_eq!(a.phone.my_name(), "Ada King");
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada King");

    let before = a.phone.my_face_seed();
    a.phone.redraw_face().unwrap();
    assert_ne!(a.phone.my_face_seed(), before);
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).face_seed, a.phone.my_face_seed());
    assert_eq!(member(&b, &id, 0).name, "Ada King");

    // A name the relay could not take now waits, and goes with the next poll.
    a.http.fail_posts.store(1, Ordering::SeqCst);
    a.phone.set_my_name("Ada L".into()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada King");
    a.phone.poll(id.clone()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada L");
}

#[test]
fn without_cards_the_members_have_the_interim_face_and_no_name() {
    use sha2::{Digest, Sha256};
    let r = relay();
    let (a, b) = named(&r, ["", ""]);
    assert_eq!(a.phone.my_name(), "");
    let (id, ida, _) = finish(&a, &b);
    a.phone.poll(id.clone()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    let key = a.signer.public(&format!("cahoots:seat:{ida}")).unwrap();
    let interim = Sha256::new().chain_update(b"cahoots/face/v1").chain_update(&key).finalize().to_vec();
    for d in [&a, &b] {
        let m = member(d, &id, 0);
        assert_eq!(m.name, "", "the app shows its own name for the seat");
        assert!(!m.has_card);
        assert_eq!(m.device, None);
    }
    // B sees the interim face. A, on its own seat, sees the face it will publish, the same
    // in the roster and in Settings.
    assert_eq!(member(&b, &id, 0).face_seed, interim);
    assert_eq!(b.phone.seat_face_seed(id.clone(), 0).unwrap(), interim);
    assert_eq!(member(&a, &id, 0).face_seed, a.phone.my_face_seed());
    assert_eq!(a.phone.seat_face_seed(id.clone(), 0).unwrap(), a.phone.my_face_seed());
    // A circle's own face shows on the seat before any card.
    a.phone.set_circle_face(id.clone(), true).unwrap();
    let own = a.phone.seat_face_seed(id.clone(), 0).unwrap();
    assert_ne!(own, a.phone.my_face_seed());
    assert_eq!(member(&a, &id, 0).face_seed, own);
    assert!(!member(&a, &id, 0).has_card, "no name, so no card");

    // Names are 1 to 64 characters after trimming.
    for bad in ["", "   ", &"x".repeat(65), &"é".repeat(65)] {
        match a.phone.set_my_name(bad.to_string()) {
            Err(e) => assert_eq!(e.to_string(), "A name is 1 to 64 characters."),
            Ok(()) => panic!("{bad:?} was taken"),
        }
    }
    assert_eq!(a.phone.my_name(), "");
    // 64 characters of four bytes each: 256 bytes, and the card carries it.
    let wide = "\u{1F600}".repeat(64);
    assert_eq!(wide.len(), 256);
    for good in ["x".repeat(64), "é".repeat(64), wide.clone()] {
        a.phone.set_my_name(good.clone()).unwrap();
        assert_eq!(a.phone.my_name(), good);
    }
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, wide);
}

#[test]
fn a_circle_face_changes_that_circle_only() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (one, _, _) = finish(&a, &b);
    let (two, _, _) = finish(&a, &b);
    assert_ne!(one, two);
    let device_face = a.phone.my_face_seed();

    assert!(!a.phone.circle_has_own_face(one.clone()).unwrap());
    a.phone.set_circle_face(one.clone(), true).unwrap();
    assert!(a.phone.circle_has_own_face(one.clone()).unwrap());
    assert!(!a.phone.circle_has_own_face(two.clone()).unwrap());
    assert_eq!(a.phone.my_face_seed(), device_face, "the device's face stays");
    for id in [&one, &two] {
        b.phone.poll(id.clone()).unwrap();
    }
    let in_one = member(&b, &one, 0).face_seed;
    assert_ne!(in_one, device_face);
    assert_eq!(member(&a, &one, 0).face_seed, in_one);
    assert_eq!(member(&b, &two, 0).face_seed, device_face);

    // Redrawing the device's face leaves the circle with its own face as it is.
    a.phone.redraw_face().unwrap();
    for id in [&one, &two] {
        b.phone.poll(id.clone()).unwrap();
    }
    assert_eq!(member(&b, &one, 0).face_seed, in_one);
    assert_eq!(member(&b, &two, 0).face_seed, a.phone.my_face_seed());

    // Back to the device's face.
    a.phone.set_circle_face(one.clone(), false).unwrap();
    assert!(!a.phone.circle_has_own_face(one.clone()).unwrap());
    b.phone.poll(one.clone()).unwrap();
    assert_eq!(member(&b, &one, 0).face_seed, a.phone.my_face_seed());
}

/// A `SeatSigner` over the test's soft signer, to sign a card as a seat would.
struct Soft<'a>(&'a SoftSigner, String);

impl cahoots_core::seat::SeatSigner for Soft<'_> {
    fn public(&mut self) -> SeatPublic {
        SeatPublic::parse(&self.0.public(&self.1).unwrap()).unwrap()
    }
    fn sign(&mut self, msg: &[u8]) -> cahoots_core::seat::Signature {
        use cahoots_phone::Signer;
        self.0.sign(self.1.clone(), msg.to_vec()).unwrap().try_into().unwrap()
    }
}

#[test]
fn a_card_for_another_seat_is_refused() {
    use cahoots_objects::card::Card;
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (id, _, idb) = finish(&a, &b);
    a.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&a, &id, 1).name, "Bea");
    let bea_face = b.phone.my_face_seed();

    // B's seat signs a card that names seat 1 (index 0, A's seat) and posts it in the chat:
    // under its own kid [1], where the kid and the card disagree, and under kid [0], where
    // the signature is not seat 0's. A would keep an accepted card under the sender, seat 2
    // (index 1), so B's name and face there must stay as B published them.
    let circle_id: [u8; 16] = hex::decode(&id).unwrap().try_into().unwrap();
    let forged = Card {
        circle_id,
        seat: 0,
        name: "Mallory".into(),
        handles: vec![],
        endpoints: vec![],
        log: 99,
        face: Some([1; 32]),
        device: Some(0),
    };
    let label = format!("cahoots:seat:{idb}");
    let dir = b.path.join("circles").join(&id);
    for kid in [[1u8], [0u8]] {
        let signed = forged.sign(Soft(&b.signer, label.clone()), &kid).unwrap();
        let mut c = cahoots_phone::chat::Circle::load(&dir).unwrap();
        c.publish_card(&b.http, signed).unwrap();
        a.phone.poll(id.clone()).unwrap();
        let m = member(&a, &id, 1);
        assert_eq!(m.name, "Bea");
        assert_eq!(m.face_seed, bea_face);
        assert_eq!(member(&a, &id, 0).name, "Ada");
    }
}

#[test]
fn a_seat_posting_another_seats_genuine_card_is_refused() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (id, _, _) = finish(&a, &b);
    a.phone.poll(id.clone()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&a, &id, 1).name, "Bea");

    // B holds A's card, signed by A's seat and verifying. B posts it as its own message.
    let dir = b.path.join("circles").join(&id);
    let ada = cahoots_phone::store::CardsRecord::load(&dir)
        .unwrap()
        .get(0)
        .unwrap()
        .card
        .to_vec();
    let mut c = cahoots_phone::chat::Circle::load(&dir).unwrap();
    c.publish_card(&b.http, ada).unwrap();
    // A reads it from sender seat 2 (index 1): the card names seat 1, so it is not kept.
    a.phone.poll(id.clone()).unwrap();
    let m = member(&a, &id, 1);
    assert_eq!(m.name, "Bea");
    assert_eq!(m.face_seed, b.phone.my_face_seed());
}

#[test]
fn cards_follow_a_circle_to_a_new_relay() {
    let (r, r2) = (relay(), relay());
    let (a, b) = named_on(&r, &r2, ["Ada", "Bea"]);
    let (id, _, _) = finish(&a, &b);
    // Enough traffic on the first relay that its positions run past the new one's.
    for i in 0..5 {
        a.phone.send_chat(id.clone(), format!("old {i}")).unwrap();
    }
    a.phone.set_my_name("Ada One".into()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    a.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada One");

    for d in [&a, &b] {
        d.phone.set_relay(id.clone(), RELAY2.into()).unwrap();
    }
    let circle_id: [u8; 16] = hex::decode(&id).unwrap().try_into().unwrap();
    // Each phone's next poll posts its own card on the new relay.
    a.phone.poll(id.clone()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(r2.lock().unwrap().poll(&circle_id, None, 0).unwrap().len(), 2);
    a.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&a, &id, 1).name, "Bea");
    assert!(member(&a, &id, 1).has_card);

    // A name changed on the new relay is read at a low position, and still kept.
    a.phone.set_my_name("Ada Two".into()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada Two");
}

#[test]
fn a_refused_face_id_asks_once_and_share_my_card_shares_the_name() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (one, _, _) = finish(&a, &b);
    let (two, _, _) = finish(&a, &b);
    for id in [&one, &two] {
        b.phone.poll(id.clone()).unwrap();
    }

    a.signer.set_locked(true);
    let signs = a.signer.sign_calls().len();
    assert!(matches!(a.phone.set_my_name("Ada King".into()), Err(PhoneError::Locked)));
    // The name is saved, nothing was signed, and nothing reached B.
    assert_eq!(a.phone.my_name(), "Ada King");
    assert_eq!(a.signer.sign_calls().len(), signs);
    for id in [&one, &two] {
        b.phone.poll(id.clone()).unwrap();
        assert_eq!(member(&b, id, 0).name, "Ada");
    }
    // A poll while still locked changes nothing.
    a.phone.poll(one.clone()).unwrap();
    assert!(a.phone.card_waiting());

    // After the refusal the polls do not sign, even once the signer would allow it, so
    // Face ID is not asked on every poll.
    a.signer.set_locked(false);
    for id in [&one, &two] {
        a.phone.poll(id.clone()).unwrap();
        b.phone.poll(id.clone()).unwrap();
        assert_eq!(member(&b, id, 0).name, "Ada");
    }
    assert_eq!(a.signer.sign_calls().len(), signs);

    // Sharing again signs and posts the card in every circle where it waits.
    a.phone.share_my_card().unwrap();
    assert_eq!(a.signer.sign_calls().len(), signs + 2);
    assert!(!a.phone.card_waiting());
    for id in [&one, &two] {
        b.phone.poll(id.clone()).unwrap();
        assert_eq!(member(&b, id, 0).name, "Ada King");
    }
    // And it is not signed again after that.
    let signs = a.signer.sign_calls().len();
    for id in [&one, &two] {
        a.phone.poll(id.clone()).unwrap();
    }
    assert_eq!(a.signer.sign_calls().len(), signs);
}

#[test]
fn a_refused_share_stops_the_polls_until_share_my_card() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (id, _, _) = finish(&a, &b);
    assert!(!a.phone.card_waiting());
    a.phone.share_my_card().unwrap();

    a.signer.set_locked(true);
    assert!(matches!(a.phone.set_my_name("Ada Two".into()), Err(PhoneError::Locked)));
    assert!(matches!(a.phone.share_my_card(), Err(PhoneError::Locked)));
    a.signer.set_locked(false);
    let signs = a.signer.sign_calls().len();
    a.phone.poll(id.clone()).unwrap();
    assert_eq!(a.signer.sign_calls().len(), signs, "the poll does not sign");

    a.phone.share_my_card().unwrap();
    assert_eq!(a.signer.sign_calls().len(), signs + 1);
    b.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&b, &id, 0).name, "Ada Two");
}

#[test]
fn set_my_name_signs_every_circle_in_one_batch() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", "Bea"]);
    let (one, _, _) = finish(&a, &b);
    let (two, _, _) = finish(&a, &b);
    for id in [&one, &two] {
        a.phone.poll(id.clone()).unwrap();
    }
    let before = a.signer.events().len();
    a.phone.set_my_name("Ada King".into()).unwrap();
    let events = a.signer.events()[before..].to_vec();
    // One begin, both cards signed inside it, one end: Face ID is asked once.
    assert_eq!(events.first().map(String::as_str), Some("begin"));
    assert_eq!(events.last().map(String::as_str), Some("end"));
    assert_eq!(events.iter().filter(|e| *e == "begin").count(), 1);
    assert_eq!(events.iter().filter(|e| *e == "end").count(), 1);
    assert_eq!(events.iter().filter(|e| e.starts_with("sign ")).count(), 2);
    // A refusal ends the batch too.
    a.signer.set_locked(true);
    let before = a.signer.events().len();
    assert!(matches!(a.phone.redraw_face(), Err(PhoneError::Locked)));
    assert_eq!(a.signer.events()[before..], ["begin", "end"]);
}

#[test]
fn a_name_is_shown_without_control_or_direction_characters() {
    let r = relay();
    let (a, b) = named(&r, ["Ada", ""]);
    let (id, _, _) = finish(&a, &b);
    b.phone
        .set_my_name("Bea\u{202E}lin\u{7}\u{2066}da\u{200F}".into())
        .unwrap();
    a.phone.poll(id.clone()).unwrap();
    assert_eq!(member(&a, &id, 1).name, "Bealinda");
    assert_eq!(cahoots_phone::display_name("\u{202D}\u{1b}[31m Ann \u{2069}"), "[31m Ann");
}

#[test]
fn a_chat_record_from_before_resets_and_cards_still_loads() {
    #[derive(minicbor::Encode)]
    struct OldChatRecord {
        #[n(0)]
        state: minicbor::bytes::ByteVec,
        #[n(1)]
        outbox: Vec<cahoots_phone::store::OutboxEntry>,
        #[n(2)]
        journal: Vec<cahoots_phone::store::MessageRecord>,
    }
    let old = OldChatRecord {
        state: b"CHS2 state bytes".to_vec().into(),
        outbox: vec![cahoots_phone::store::OutboxEntry {
            id: "out-1".into(),
            wire: vec![2, 3, 4].into(),
        }],
        journal: Vec::new(),
    };
    let bytes = minicbor::to_vec(&old).unwrap();
    // Built by hand too: an array of three, the state, the outbox, the journal.
    let mut by_hand = vec![0x83, 0x50];
    by_hand.extend_from_slice(b"CHS2 state bytes");
    by_hand.extend_from_slice(&[0x81, 0x82, 0x65]);
    by_hand.extend_from_slice(b"out-1");
    by_hand.extend_from_slice(&[0x43, 2, 3, 4, 0x80]);
    assert_eq!(bytes, by_hand);
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(cahoots_phone::store::CHAT), &by_hand).unwrap();
    let got = cahoots_phone::store::ChatRecord::load(dir.path()).unwrap();
    assert_eq!(&got.state[..], b"CHS2 state bytes");
    assert_eq!(got.outbox.len(), 1);
    assert_eq!(got.outbox[0].id, "out-1");
    assert!(got.journal.is_empty());
    assert!(got.pending_bump.is_none());
    assert!(got.pending_card.is_none());
    assert!(!got.card_due);
}

#[test]
fn a_phone_restored_from_before_its_reset_follows_it() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    let (id, _, _) = finish(&a, &b);
    for d in [&a, &b] {
        d.phone.set_relay(id.clone(), RELAY.into()).unwrap();
    }
    a.phone.send_chat(id.clone(), "before".into()).unwrap();
    b.phone.poll(id.clone()).unwrap();
    let dir = cahoots_phone::store::circles_dir(&a.path).join(&id);
    let saved: Vec<(String, Vec<u8>)> = [cahoots_phone::store::CHAT, cahoots_phone::store::CURSOR]
        .iter()
        .map(|f| (f.to_string(), std::fs::read(dir.join(f)).unwrap()))
        .collect();
    a.phone.bump_chat_generation(id.clone()).unwrap();
    assert!(!a.phone.chat_reset_waiting(id.clone()));

    // The phone comes back from a copy made before the reset.
    for (f, bytes) in &saved {
        std::fs::write(dir.join(f), bytes).unwrap();
    }
    let restored = Phone::open(
        a.path.to_string_lossy().into_owned(),
        Box::new(a.signer.clone()),
        Box::new(a.http.clone()),
    )
    .unwrap();
    restored.poll(id.clone()).unwrap();
    restored.send_chat(id.clone(), "after".into()).unwrap();
    // Its message is in the new generation, which B reads.
    let circle: [u8; 16] = hex::decode(&id).unwrap().try_into().unwrap();
    let all = r.lock().unwrap().poll(&circle, None, 0).unwrap();
    let (bump_header, _) = cahoots_chat::parse(&all[all.len() - 2].1).unwrap();
    let (after_header, _) = cahoots_chat::parse(&all[all.len() - 1].1).unwrap();
    assert_eq!(bump_header.msg_type, cahoots_chat::MessageType::Bump);
    assert_ne!(
        after_header.epoch.generation_parts().unwrap().0,
        bump_header.epoch.generation_parts().unwrap().0
    );
    assert_eq!(
        texts(&b.phone.poll(id.clone()).unwrap()),
        ["Keys were reset by seat 1.", "after"]
    );
}

#[test]
fn a_phone_that_updates_late_reads_what_it_passed_over_and_follows_the_reset() {
    use cahoots_chat::{sign_bump, ChatSession};
    use cahoots_core::manifest::{Manifest, Seat};
    use cahoots_core::root::gen0_id;
    use cahoots_core::seat::SeatKey;

    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chat/tests/fixtures");
    let state = std::fs::read(fixtures.join("chat-v5.bin")).unwrap();
    assert_eq!(&state[..4], b"CHS2");
    let blob = std::fs::read(fixtures.join("chat-v5-wires.bin")).unwrap();
    let mut wires = Vec::new();
    let mut at = 0;
    while at < blob.len() {
        let n = u32::from_le_bytes(blob[at..at + 4].try_into().unwrap()) as usize;
        wires.push(blob[at + 4..at + 4 + n].to_vec());
        at += 4 + n;
    }

    let root = [0x42u8; 32];
    let circle_id = [0x0cu8; 16];
    let id = hex::encode(circle_id);
    let params = cahoots_core::params::CircleParams {
        n: 3,
        pad_len: PAD,
        name: "old".into(),
        agent_seat: false,
    };
    let seat_key = |i: u8| SeatKey::from_seed(&[i + 1; 32]).unwrap();
    let manifest = Manifest {
        circle_id,
        floor: Manifest::default_floor(&params),
        params: params.clone(),
        seats: (0..3)
            .map(|i| Seat {
                seat: i,
                key: seat_key(i).public(),
                tier: 0,
                evidence: Vec::new(),
            })
            .collect(),
        genesis: 0,
    };
    let sealed = SealedManifest {
        body: manifest.encode(),
        attestations: Vec::new(),
    };

    // While this phone ran the build before generations, seat 1 of the circle chatted in
    // generation 0 and then reset the keys. The old build passed over all of it.
    let r = relay();
    for w in &wires {
        r.lock().unwrap().post(&circle_id, w).unwrap();
    }
    let mut first = ChatSession::new(&root, circle_id, 3, 0);
    first.mark_synced();
    let in_gen0 = first.send(b"in generation 0").unwrap();
    let to = [0x77u8; 32];
    let payload = sign_bump(&seat_key(0), 0, &circle_id, &gen0_id(&circle_id), &to).unwrap();
    let bump = first.send_bump(to, &payload).unwrap();
    let after = first.send(b"after the reset").unwrap();
    let mut last = 0;
    for w in [&in_gen0, &bump, &after] {
        last = r.lock().unwrap().post(&circle_id, w).unwrap();
    }

    let d = device(&r);
    let record = CircleRecord {
        params: params.encode().into(),
        circle_id: circle_id.to_vec().into(),
        seat: 1,
        tier: 0,
        label: "CSPRNG".into(),
        words: Vec::new(),
        manifest: sealed.encode().into(),
        root: root.to_vec().into(),
        seat_label: "cahoots:seat:old".into(),
        seat_public: vec![2; 33].into(),
        pair_keys: vec![Vec::new().into(); 3],
        pair_counters: vec![0; 3],
    };
    let chat = cahoots_phone::store::ChatRecord {
        state: state.into(),
        outbox: Vec::new(),
        journal: Vec::new(),
        pending_bump: None,
        pending_card: None,
        card_due: false,
    };
    let dir = cahoots_phone::store::create_circle(
        &d.path,
        &id,
        &cahoots_phone::store::NewCircle {
            record: &record,
            pad: &[0; 16],
            chat: &chat,
            settings: &cahoots_phone::store::SettingsRecord {
                relay: RELAY.into(),
                unread: 0,
                face_override: None,
            },
            slices: params.slices(),
        },
    )
    .unwrap();
    cahoots_phone::store::CursorRecord { after: Some(last) }
        .save(&dir)
        .unwrap();

    let phone = Phone::open(
        d.path.to_string_lossy().into_owned(),
        Box::new(d.signer.clone()),
        Box::new(d.http.clone()),
    )
    .unwrap();
    // Read again from the start: the message it had read is a replay, the rest is new.
    let got = phone.poll(id.clone()).unwrap();
    assert_eq!(
        texts(&got),
        [
            "old history two",
            "in generation 0",
            "Keys were reset by seat 1.",
            "after the reset"
        ]
    );
    // It ended in the new generation: seat 1 reads its reply there.
    phone.send_chat(id.clone(), "caught up".into()).unwrap();
    let all = r.lock().unwrap().poll(&circle_id, Some(last), 0).unwrap();
    assert_eq!(all.len(), 1);
    let (_, text) = first.receive(&all[0].1).unwrap();
    assert_eq!(cahoots_chat::bucket_unpad(&text).unwrap(), b"caught up");
    assert_eq!(first.generation_id(), to);
    // A second open does not read from the start again.
    drop(phone);
    let again = Phone::open(
        d.path.to_string_lossy().into_owned(),
        Box::new(d.signer.clone()),
        Box::new(d.http.clone()),
    )
    .unwrap();
    assert!(again.poll(id).unwrap().is_empty());
}
