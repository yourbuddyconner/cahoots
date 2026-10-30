//! The ceremony through a relay, as three browsers hold it: one offer scanned, then every
//! round's messages posted to and read from the relay.

use cahoots_relay::{MemRelay, Relay};
use cahoots_wasm::{b64_decode, offer_params, WasmCeremony};

const L: usize = 2048;
const RELAY: &str = "https://relay.test";

fn contribution(i: u8) -> Vec<u8> {
    (0..L)
        .map(|k| ((k as u32).wrapping_mul(17 + i as u32) ^ i as u32) as u8)
        .collect()
}

fn member(i: u8, offer: Option<&str>) -> WasmCeremony {
    let (kem, seat) = (vec![0x40 + i; 32], vec![0x60 + i; 32]);
    match offer {
        None => WasmCeremony::new(3, L as u32, "trio", false, i, &contribution(i), None, &kem, &seat, 0),
        Some(text) => WasmCeremony::from_offer(text, i, &contribution(i), None, &kem, &seat, 0),
    }
    .unwrap()
}

/// One tick of the browser's loop: post what waits, then read what is new.
fn tick(c: &mut WasmCeremony, relay: &mut MemRelay, id: &[u8; 16], cursor: &mut Option<u64>) {
    for w in c.outbound().unwrap() {
        relay.post(id, &b64_decode(&w).unwrap()).unwrap();
        c.posted(&w);
    }
    for (seq, m) in relay.poll(id, *cursor, 0).unwrap() {
        c.inbound(&m).unwrap();
        *cursor = Some(seq);
    }
}

/// A tick that keeps going past an abort, for the intruder case.
fn tick_any(c: &mut WasmCeremony, relay: &mut MemRelay, id: &[u8; 16], cursor: &mut Option<u64>) {
    for w in c.outbound().unwrap_or_default() {
        relay.post(id, &b64_decode(&w).unwrap()).unwrap();
        c.posted(&w);
    }
    for (seq, m) in relay.poll(id, *cursor, 0).unwrap() {
        let _ = c.inbound(&m);
        *cursor = Some(seq);
    }
}

#[test]
fn three_browsers_hold_a_ceremony_through_a_mem_relay() {
    let mut host = member(0, None);
    assert!(!host.through_relay());
    let offer = host.offer(RELAY).unwrap();
    assert!(offer.starts_with("cahoots:ceremony:v1:"), "{offer}");
    assert!(host.through_relay());
    // The offer is the same each time it is shown.
    assert_eq!(host.offer("https://elsewhere.test").unwrap(), offer);
    assert_eq!(host.relay().as_deref(), Some(RELAY));
    let params = offer_params(&offer).unwrap();
    assert_eq!(params, host.params_bytes());
    assert_eq!(offer_params("cahoots-params:0:AQ").unwrap_err(), "not-an-offer");

    let mut ms = vec![host, member(1, Some(&offer)), member(2, Some(&offer))];
    let id: [u8; 16] = hex::decode(ms[0].channel_hex().unwrap()).unwrap().try_into().unwrap();
    assert_eq!(ms[1].channel_hex(), ms[0].channel_hex());
    // A joiner is on the host's relay.
    assert_eq!(ms[2].relay().as_deref(), Some(RELAY));
    // A stranger posts into the channel; it does not open and is dropped.
    let mut relay = MemRelay::default();
    let mut junk = vec![3u8];
    junk.extend_from_slice(&id);
    junk.extend_from_slice(&40u32.to_le_bytes());
    junk.extend_from_slice(&[7u8; 40]);
    relay.post(&id, &junk).unwrap();

    let mut cursors = [None; 3];
    for _ in 0..3 {
        for (i, c) in ms.iter_mut().enumerate() {
            tick(c, &mut relay, &id, &mut cursors[i]);
        }
    }
    // Every commitment is on the relay, but the host has scanned nobody: it stays in step one,
    // and its reveal does not go.
    assert_eq!(ms[0].phase(), "commit");
    assert_eq!(ms[0].unscanned(), vec![1, 2]);
    assert!(ms[0].join_code().is_none());
    // The host scans each joiner's code from their screen.
    let codes: Vec<String> = ms[1..].iter().map(|c| c.join_code().unwrap()).collect();
    assert!(codes[0].starts_with("cahoots:joined:v1:"));
    assert_eq!(ms[1].scan_join(&codes[1]).unwrap_err(), "not-host");
    assert_eq!(ms[0].scan_join(&codes[0]).unwrap(), 1);
    assert_eq!(ms[0].scanned(), vec![1]);
    assert!(!ms[0].needs_continue());
    assert_eq!(ms[0].scan_join(&codes[1]).unwrap(), 2);
    assert_eq!(ms[0].scanned(), vec![1, 2]);
    assert!(ms[0].needs_continue());
    for _ in 0..2 {
        for (i, c) in ms.iter_mut().enumerate() {
            tick(c, &mut relay, &id, &mut cursors[i]);
        }
    }
    // Before Continue the host posts no reveal, so nobody gets past step two.
    assert!(ms[0].needs_continue());
    assert!(ms[1..].iter().all(|c| c.phase() == "reveal"));
    ms[0].proceed();
    for _ in 0..4 {
        for (i, c) in ms.iter_mut().enumerate() {
            tick(c, &mut relay, &id, &mut cursors[i]);
        }
    }
    assert!(ms[1].join_code().is_none(), "the host's reveal says it took the joiner");
    for c in ms.iter_mut() {
        assert_eq!(c.phase(), "complete");
        assert!(c.outbound().unwrap().is_empty(), "everything posted");
    }
    // Each member posted three messages: commitment, reveal and signature.
    assert_eq!(relay.poll(&id, None, 0).unwrap().len(), 1 + 3 * 3);
    let words: Vec<String> = ms
        .into_iter()
        .map(|c| c.finish("CSPRNG").unwrap().words())
        .collect();
    assert_eq!(words[0], words[1]);
    assert_eq!(words[0], words[2]);
}

#[test]
fn a_second_browser_on_a_taken_seat_is_told() {
    let mut host = member(0, None);
    let offer = host.offer(RELAY).unwrap();
    let mut a = member(1, Some(&offer));
    let mut b = member(1, Some(&offer));
    let id: [u8; 16] = hex::decode(host.channel_hex().unwrap()).unwrap().try_into().unwrap();
    let mut relay = MemRelay::default();
    for c in [&mut a, &mut b] {
        for w in c.outbound().unwrap() {
            relay.post(&id, &b64_decode(&w).unwrap()).unwrap();
        }
    }
    let msgs = relay.poll(&id, None, 0).unwrap();
    // A's commitment is first on the relay: A keeps the seat and ignores B's.
    for (_, m) in &msgs {
        assert!(!a.inbound(m).unwrap());
    }
    assert!(!a.lost_seat());
    assert!(a.outbound().unwrap().is_empty());
    // B reads A's first: B lost the seat and posts nothing more, ever.
    for (_, m) in &msgs {
        assert!(!b.inbound(m).unwrap());
    }
    assert!(b.lost_seat());
    assert!(b.outbound().unwrap().is_empty());
    // The host holds both until it scans one.
    assert!(!host.inbound(&msgs[0].1).unwrap());
    assert!(!host.inbound(&msgs[1].1).unwrap());
    assert_eq!(host.unscanned(), vec![1]);
}

#[test]
fn an_unscanned_commitment_does_not_reach_the_host() {
    // An intruder photographed the offer and takes seat 3 on the relay before the real person.
    let mut host = member(0, None);
    let offer = host.offer(RELAY).unwrap();
    let id: [u8; 16] = hex::decode(host.channel_hex().unwrap()).unwrap().try_into().unwrap();
    let mut intruder = WasmCeremony::from_offer(&offer, 2, &contribution(9), None, &[0x49; 32], &[0x69; 32], 0).unwrap();
    let mut ms = [host, member(1, Some(&offer)), member(2, Some(&offer))];
    let mut relay = MemRelay::default();
    let mut cx = None;
    tick(&mut intruder, &mut relay, &id, &mut cx);
    let mut cursors = [None; 3];
    for _ in 0..3 {
        for (i, c) in ms.iter_mut().enumerate() {
            tick(c, &mut relay, &id, &mut cursors[i]);
        }
        tick(&mut intruder, &mut relay, &id, &mut cx);
    }
    // The real seat 3 lost the seat on the relay and stops. The intruder's commitment is on
    // the relay, but the host has not scanned it and holds it back.
    assert!(ms[2].lost_seat());
    assert!(ms[2].join_code().is_none());
    let code = ms[1].join_code().unwrap();
    ms[0].scan_join(&code).unwrap();
    assert_eq!(ms[0].scanned(), vec![1]);
    assert_eq!(ms[0].unscanned(), vec![2]);
    assert!(!ms[0].needs_continue());
    // The real person's code, scanned before they saw their seat was lost, matches their own
    // commitment, which is on the relay after the intruder's. The host takes that one. The
    // others took the intruder's. Nobody completes either way: the host waits or stops, and
    // the joiner and the intruder never get the host's reveal and signature for their manifest.
    let mut stand_in = member(2, Some(&offer));
    let real_code = stand_in.join_code().unwrap();
    let _ = stand_in.outbound();
    let _ = ms[0].scan_join(&real_code);
    for _ in 0..6 {
        for (i, c) in ms.iter_mut().enumerate() {
            tick_any(c, &mut relay, &id, &mut cursors[i]);
        }
        tick_any(&mut intruder, &mut relay, &id, &mut cx);
    }
    // The intruder's reveal does not match the commitment the host took, so the host stops.
    assert!(["commit", "reveal", "aborted"].contains(&ms[0].phase().as_str()), "{}", ms[0].phase());
    if ms[0].needs_continue() {
        ms[0].proceed();
        for _ in 0..6 {
            for (i, c) in ms.iter_mut().enumerate() {
                tick_any(c, &mut relay, &id, &mut cursors[i]);
            }
            tick_any(&mut intruder, &mut relay, &id, &mut cx);
        }
    }
    for c in [&ms[0], &ms[1], &intruder] {
        assert_ne!(c.phase(), "complete");
    }
}
