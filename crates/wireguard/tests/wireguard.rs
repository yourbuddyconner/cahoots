use cahoots_core::params::CircleParams;
use cahoots_provider::{FileProvider, MemProvider, PadError};
use cahoots_relay::{MemRelay, Relay};
use cahoots_wireguard::{
    accept, announce, issue, issue_on_feed, open_entry, overlay_address, overlay_subnet,
    render_peer, rotations_remaining, sync, wg_set_command, FeedItem, FeedState, PskToken,
    RotationSchedule, WgError,
};
use std::sync::{Arc, Mutex};

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(29) ^ (i >> 4)) as u8)
        .collect()
}

fn pair() -> (MemProvider, MemProvider, [u8; 16]) {
    let params = CircleParams::new(2, 4096, "wg", false).unwrap();
    let mut a = MemProvider::new();
    let mut b = MemProvider::new();
    let cid = a.add_circle(params.clone(), 0, pad_bytes(4096)).unwrap();
    b.add_circle(params, 1, pad_bytes(4096)).unwrap();
    (a, b, cid)
}

const PEER_PK: &str = "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=";

#[test]
fn both_sides_derive_the_same_preshared_key() {
    let (mut a, mut b, cid) = pair();
    let issued = issue(&mut a, &cid, 1).unwrap();
    assert_eq!(issued.token.handle.slice, 0);
    assert_eq!(issued.token.handle.offset, 0);
    assert_eq!(issued.token.to, 1);
    let accepted = accept(&mut b, &issued.token).unwrap();
    assert_eq!(*issued.preshared_key, *accepted);
    assert_eq!(issued.preshared_key.len(), 44);
    assert!(issued.preshared_key.ends_with('='));
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&*issued.preshared_key)
        .unwrap();
    assert_ne!(
        &raw[..],
        &pad_bytes(4096)[..32],
        "derived, not raw pad bytes"
    );
}

#[test]
fn the_key_is_bound_to_the_pair() {
    // The same pad range derived for a different acceptor seat is a different key.
    let (mut a, _, cid) = pair();
    let issued = issue(&mut a, &cid, 1).unwrap();
    let mut other = PskToken {
        to: 0,
        ..issued.token
    };
    let (mut c, _, _) = pair();
    other.handle = issued.token.handle;
    let k_other = accept(&mut c, &PskToken { to: 7, ..other })
        .unwrap_or_else(|_| zeroize::Zeroizing::new(String::new()));
    assert_ne!(*issued.preshared_key, *k_other);
}

#[test]
fn token_roundtrips_and_carries_the_pair() {
    let (mut a, _, cid) = pair();
    let issued = issue(&mut a, &cid, 1).unwrap();
    let token = issued.token.encode();
    assert!(token.starts_with("cahoots-wg:"));
    assert_eq!(token.len(), "cahoots-wg:".len() + 46);
    assert_eq!(PskToken::parse(&token).unwrap(), issued.token);
    assert_eq!(PskToken::parse("nope").unwrap_err(), WgError::Token);
    assert_eq!(
        PskToken::parse("cahoots-wg:abcd").unwrap_err(),
        WgError::Token
    );
}

#[test]
fn accepting_twice_is_refused() {
    let (mut a, mut b, cid) = pair();
    let issued = issue(&mut a, &cid, 1).unwrap();
    accept(&mut b, &issued.token).unwrap();
    assert_eq!(
        accept(&mut b, &issued.token).unwrap_err(),
        WgError::Pad(PadError::AlreadySpent)
    );
}

#[test]
fn renders_a_peer_section() {
    let psk = zeroize::Zeroizing::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string());
    let out = render_peer(
        PEER_PK,
        &psk,
        &["10.7.0.2/32"],
        Some("vpn.example:51820"),
        Some(25),
    );
    assert_eq!(
        out,
        "[Peer]\n\
         PublicKey = HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=\n\
         PresharedKey = AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n\
         AllowedIPs = 10.7.0.2/32\n\
         Endpoint = vpn.example:51820\n\
         PersistentKeepalive = 25\n"
    );
    let minimal = render_peer(PEER_PK, &psk, &["10.7.0.2/32", "fd00::2/128"], None, None);
    assert!(minimal.contains("AllowedIPs = 10.7.0.2/32, fd00::2/128\n"));
    assert!(!minimal.contains("Endpoint"));
}

#[test]
fn wg_set_command_reads_psk_from_stdin() {
    assert_eq!(
        wg_set_command("wg0", PEER_PK, None, None),
        vec![
            "wg",
            "set",
            "wg0",
            "peer",
            PEER_PK,
            "preshared-key",
            "/dev/stdin"
        ]
    );
    let full = wg_set_command("wg0", PEER_PK, Some("1.2.3.4:51820"), Some("10.9.0.2/32"));
    assert_eq!(
        &full[7..],
        &["endpoint", "1.2.3.4:51820", "allowed-ips", "10.9.0.2/32"]
    );
}

#[test]
fn overlay_addresses_come_from_the_circle_id() {
    let mut id = [0u8; 16];
    id[0] = 7;
    id[1] = 200;
    assert_eq!(overlay_subnet(&id), "10.8.200.0/24");
    assert_eq!(overlay_address(&id, 0), "10.8.200.1");
    assert_eq!(overlay_address(&id, 7), "10.8.200.8");
    id[0] = 253;
    assert_eq!(
        overlay_subnet(&id),
        "10.254.200.0/24",
        "never 10.0 or 10.255"
    );
    id[0] = 254;
    assert_eq!(overlay_subnet(&id), "10.1.200.0/24");
}

#[test]
fn rotation_schedule_and_budget() {
    let (mut a, _, cid) = pair();
    assert_eq!(rotations_remaining(&a, &cid), 64, "2048 / 32");
    issue(&mut a, &cid, 1).unwrap();
    assert_eq!(rotations_remaining(&a, &cid), 63);
    let s = RotationSchedule {
        started_at: 1_000,
        interval_secs: 86_400,
    };
    assert_eq!(s.next_rotation_after(1_000), 87_400);
    assert_eq!(s.next_rotation_after(90_000), 173_800);
    assert_eq!(s.rotations_until(1_000 + 86_400 * 3), 3);
    assert_eq!(s.exhausted_at(63), 1_000 + 86_400 * 63);
}

/// Two provider directories, one in-memory relay: announce, learn the peer, issue over the
/// feed, accept over the feed, same key both ends, replay refused, cursor persists.
#[test]
fn bootstrap_over_the_feed() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 4096, "feed", false).unwrap();
    let cid = FileProvider::open(dir_a.path())
        .unwrap()
        .add_circle(params.clone(), 0, pad_bytes(4096))
        .unwrap();
    FileProvider::open(dir_b.path())
        .unwrap()
        .add_circle(params, 1, pad_bytes(4096))
        .unwrap();
    let mut pad_a = FileProvider::open_pad(dir_a.path(), &cid).unwrap();
    let mut pad_b = FileProvider::open_pad(dir_b.path(), &cid).unwrap();
    let shared = Arc::new(Mutex::new(MemRelay::default()));
    let mut relay_a = MemRelay::shared(shared.clone());
    let mut relay_b = MemRelay::shared(shared);
    let (mut state_a, mut state_b) = (FeedState::default(), FeedState::default());

    let wire = announce(
        &mut pad_a,
        PEER_PK,
        Some("203.0.113.1:51820"),
        "10.9.0.1/32",
    )
    .unwrap();
    relay_a.post(&cid, &wire).unwrap();
    let wire = announce(&mut pad_b, PEER_PK, None, "10.9.0.2/32").unwrap();
    relay_b.post(&cid, &wire).unwrap();

    // Each side sees the other's announcement and skips its own.
    let items = sync(&mut pad_a, &mut relay_a, &mut state_a).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(state_a.peers.len(), 1);
    assert_eq!(state_a.peers[&1].allowed_ips, "10.9.0.2/32");
    assert_eq!(state_a.peers[&1].endpoint, None);
    assert_eq!(state_a.seq, Some(1));
    sync(&mut pad_b, &mut relay_b, &mut state_b).unwrap();
    assert_eq!(
        state_b.peers[&0].endpoint.as_deref(),
        Some("203.0.113.1:51820")
    );

    // A issues to seat 1 over the feed; B picks it up and derives the same key.
    let (issued, wire) = issue_on_feed(&mut pad_a, 1).unwrap();
    relay_a.post(&cid, &wire).unwrap();
    let items = sync(&mut pad_b, &mut relay_b, &mut state_b).unwrap();
    let token = match items.as_slice() {
        [FeedItem::Psk(t)] => *t,
        other => panic!("expected one psk item, got {other:?}"),
    };
    assert_eq!(token, issued.token);
    assert_eq!(token.to, 1);
    let key_b = accept(&mut pad_b, &token).unwrap();
    assert_eq!(*key_b, *issued.preshared_key);
    assert_eq!(
        accept(&mut pad_b, &token).unwrap_err(),
        WgError::Pad(PadError::AlreadySpent)
    );

    // Nothing new: the cursor means a second sync returns nothing and changes nothing.
    assert!(sync(&mut pad_b, &mut relay_b, &mut state_b)
        .unwrap()
        .is_empty());
    // Re-opening an already-opened entry from the start of the feed is reported as seen, not as an error.
    let first = relay_a.poll(&cid, None, 0).unwrap().remove(0).1;
    assert_eq!(
        open_entry(&mut pad_b, &first).unwrap_err(),
        WgError::Feed("seen".into())
    );
    // A chat message on the same feed is not a pure-pad message and is skipped.
    assert_eq!(
        open_entry(&mut pad_b, &[2u8; 40]).unwrap_err(),
        WgError::Feed("not a pure-pad message".into())
    );
}
