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
const PAD: u32 = 1024;

/// An HTTP relay in process: the URLs and bodies the station speaks, over `MemRelay`.
#[derive(Clone)]
struct FakeHttp {
    relay: Arc<Mutex<MemRelay>>,
    /// The next this many posts fail as if the network were down.
    fail_posts: Arc<AtomicU32>,
}

impl FakeHttp {
    fn new(relay: &Arc<Mutex<MemRelay>>) -> Self {
        FakeHttp {
            relay: relay.clone(),
            fail_posts: Arc::new(AtomicU32::new(0)),
        }
    }
}

fn circle_of(url: &str) -> [u8; 16] {
    let rest = url
        .strip_prefix(&format!("{RELAY}/v1/circles/"))
        .expect("relay url");
    let hex_id = &rest[..32];
    assert!(rest[32..].starts_with("/messages"), "{url}");
    hex::decode(hex_id).unwrap().try_into().unwrap()
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
        let left = self.fail_posts.load(Ordering::SeqCst);
        if left > 0 {
            self.fail_posts.store(left - 1, Ordering::SeqCst);
            return Err(PhoneError::Network {
                message: "offline".into(),
            });
        }
        let seq = self.relay.lock().unwrap().post(&circle, &body).unwrap();
        Ok(serde_json::to_vec(&serde_json::json!({ "seq": seq })).unwrap())
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
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    let signer = SoftSigner::new();
    let http = FakeHttp::new(relay);
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
    let ida = a.phone.start_ceremony("park".into(), 2, PAD, false).unwrap();
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
    let ida = a.phone.start_ceremony("park".into(), 2, PAD, false).unwrap();
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
    let ida = ds[0].phone.start_ceremony("trio".into(), 3, PAD, false).unwrap();
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
    let ida = a.phone.start_ceremony("park".into(), 3, PAD, false).unwrap();
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
    let ida = a.phone.start_ceremony("park".into(), 3, PAD, false).unwrap();
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
        a.phone.start_ceremony("x".repeat(201), 2, PAD, false),
        Err(PhoneError::Ceremony { .. })
    ));
    a.phone.start_ceremony("x".repeat(200), 2, PAD, false).unwrap();
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
    let ida = a.phone.start_ceremony("park".into(), 2, PAD, false).unwrap();
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

#[test]
fn two_phones_hold_a_ceremony_through_the_relay() {
    let r = relay();
    let (a, b) = (device(&r), device(&r));
    assert_eq!(a.phone.default_relay(), "https://relay.cahoots.network");
    for d in [&a, &b] {
        d.phone.set_default_relay(RELAY.into()).unwrap();
    }
    assert!(a.phone.set_default_relay("relay.test".into()).is_err());
    let ida = a.phone.start_ceremony("park".into(), 2, PAD, true).unwrap();
    let offer = a.phone.offer_code(ida.clone()).unwrap();
    assert!(offer.starts_with("cahoots:ceremony:v1:"), "{offer}");
    // The offer fits one small code.
    assert!(a.phone.qr_matrix(offer.clone()).unwrap().size <= 41);
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
    for _ in 0..5 {
        sa = Some(tick(&a, &ida));
        sb = Some(tick(&b, &idb));
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
    a.phone.send_chat(ca.clone(), "made through the relay".into()).unwrap();
    assert_eq!(texts(&b.phone.poll(cb).unwrap()), ["made through the relay"]);
}

#[test]
fn a_phone_joins_a_browser_offer() {
    use cahoots_wasm::{b64_decode, WasmCeremony};
    let r = relay();
    let p = device(&r);
    p.phone.set_default_relay(RELAY.into()).unwrap();
    let contribution: Vec<u8> = (0..PAD).map(|i| (i * 31) as u8).collect();
    let mut browser = WasmCeremony::new(
        2, PAD, "mixed", false, 0, &contribution, None, &[0x41; 32], &[0x61; 32], 0,
    )
    .unwrap();
    let offer = browser.offer();
    let id: [u8; 16] = hex::decode(browser.channel_hex().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let idp = p.phone.join_offer(offer, 1).unwrap();
    let mut cursor = None;
    let mut state = None;
    for _ in 0..5 {
        for w in browser.outbound().unwrap() {
            r.lock().unwrap().post(&id, &b64_decode(&w).unwrap()).unwrap();
            browser.posted(&w);
        }
        for (seq, m) in r.lock().unwrap().poll(&id, cursor, 0).unwrap() {
            browser.inbound(&m).unwrap();
            cursor = Some(seq);
        }
        state = Some(tick(&p, &idp));
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
    let ida = a.phone.start_ceremony("park".into(), 2, PAD, true).unwrap();
    let idb = b.phone.join_offer(a.phone.offer_code(ida.clone()).unwrap(), 1).unwrap();
    a.http.fail_posts.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        a.phone.ceremony_tick(ida.clone()),
        Err(PhoneError::Network { .. })
    ));
    for _ in 0..5 {
        tick(&a, &ida);
        tick(&b, &idb);
    }
    assert_eq!(
        a.phone.ceremony_state(ida.clone()).unwrap().words,
        b.phone.ceremony_state(idb).unwrap().words
    );
    // Nothing was posted twice: two commitments, two reveals, two signatures.
    let id = a.phone.offer_code(ida).unwrap();
    let (channel, _) = cahoots_core::channel::Channel::from_offer(&id).unwrap();
    assert_eq!(r.lock().unwrap().poll(&channel.id, None, 0).unwrap().len(), 6);
    // A codes-only ceremony has no offer.
    let idc = a.phone.start_ceremony("codes".into(), 2, PAD, false).unwrap();
    assert!(a.phone.offer_code(idc.clone()).is_err());
    assert!(a.phone.ceremony_tick(idc).is_err());
}
