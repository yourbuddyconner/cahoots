use cahoots_core::control::EVERYONE;
use cahoots_core::params::CircleParams;
use cahoots_meshtastic::{accept, channel_set, issue, issue_on_feed, URL_PREFIX};
use cahoots_provider::{FileProvider, MemProvider, PadError};
use cahoots_relay::{MemRelay, Relay};
use cahoots_wireguard::{sync, FeedItem, FeedState, WgError};
use std::sync::{Arc, Mutex};

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 3)) as u8)
        .collect()
}

fn members(n: u8) -> (Vec<MemProvider>, [u8; 16]) {
    let params = CircleParams::new(n, 4096, "mesh", false).unwrap();
    let mut out = Vec::new();
    let mut cid = [0u8; 16];
    for i in 0..n {
        let mut p = MemProvider::new();
        cid = p.add_circle(params.clone(), i, pad_bytes(4096)).unwrap();
        out.push(p);
    }
    (out, cid)
}

#[test]
fn channel_set_protobuf_matches_the_schema() {
    let key = [0x11u8; 32];
    let bytes = channel_set(&key, "sf-meetup");
    // ChannelSet.settings (field 1, length-delimited) wrapping ChannelSettings { psk = 2, name = 3 }.
    assert_eq!(bytes[0], 0x0A);
    assert_eq!(bytes[1] as usize, bytes.len() - 2);
    let settings = &bytes[2..];
    assert_eq!(settings[0], 0x12, "psk tag: field 2, wire type 2");
    assert_eq!(settings[1], 32);
    assert_eq!(&settings[2..34], &key);
    assert_eq!(settings[34], 0x1A, "name tag: field 3, wire type 2");
    assert_eq!(settings[35], 9);
    assert_eq!(&settings[36..], b"sf-meetup");
    let no_name = channel_set(&key, "");
    assert_eq!(no_name.len(), 2 + 34);
}

#[test]
fn everyone_derives_the_same_channel_key_from_one_token() {
    let (mut m, cid) = members(3);
    let ch = issue(&mut m[0], &cid, "demo").unwrap();
    assert_eq!(ch.token.to, EVERYONE);
    assert_eq!(ch.token.handle.slice, 0);
    let a = accept(&mut m[1], &ch.token, "demo").unwrap();
    let b = accept(&mut m[2], &ch.token, "demo").unwrap();
    assert_eq!(*ch.key, *a.key);
    assert_eq!(*ch.key, *b.key);
    assert_eq!(*ch.url(), *a.url());
    assert!(ch.url().starts_with(URL_PREFIX));
    assert!(!ch.url().contains('='), "base64url without padding");
    assert!(ch
        .cli_command(0)
        .starts_with("meshtastic --ch-index 0 --ch-set psk base64:"));
    assert!(ch.cli_command(0).ends_with("--ch-set name demo"));
    // Accepting the same token twice is pad reuse and is refused.
    assert_eq!(
        accept(&mut m[1], &ch.token, "demo").unwrap_err(),
        WgError::Pad(PadError::AlreadySpent)
    );
    // The issuer rotating gives everyone a different key.
    let next = issue(&mut m[0], &cid, "demo").unwrap();
    assert_ne!(*next.key, *ch.key);
    assert_ne!(ch.token.handle.offset, next.token.handle.offset);
}

#[test]
fn a_wireguard_token_is_not_a_channel_key() {
    let (mut m, cid) = members(2);
    let wg = cahoots_wireguard::issue(&mut m[0], &cid, 1).unwrap();
    assert_eq!(
        accept(&mut m[1], &wg.token, "x").unwrap_err(),
        WgError::NotForMe { to: 1 }
    );
}

#[test]
fn channel_key_over_the_feed() {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let params = CircleParams::new(3, 4096, "feed", false).unwrap();
    let mut cid = [0u8; 16];
    for (i, d) in dirs.iter().enumerate() {
        cid = FileProvider::open(d.path())
            .unwrap()
            .add_circle(params.clone(), i as u8, pad_bytes(4096))
            .unwrap();
    }
    let mut pads: Vec<_> = dirs
        .iter()
        .map(|d| FileProvider::open_pad(d.path(), &cid).unwrap())
        .collect();
    let shared = Arc::new(Mutex::new(MemRelay::default()));
    let mut relays: Vec<_> = (0..3).map(|_| MemRelay::shared(shared.clone())).collect();

    let (ch, wire) = issue_on_feed(&mut pads[0], "feed").unwrap();
    relays[0].post(&cid, &wire).unwrap();
    for i in 1..3 {
        let mut state = FeedState::default();
        let items = sync(&mut pads[i], &mut relays[i], &mut state).unwrap();
        let token = match items.as_slice() {
            [FeedItem::Psk(t)] => *t,
            other => panic!("{other:?}"),
        };
        let got = accept(&mut pads[i], &token, "feed").unwrap();
        assert_eq!(*got.key, *ch.key);
    }
}
