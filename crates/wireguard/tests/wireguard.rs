use cahoots_core::params::CircleParams;
use cahoots_provider::{MemProvider, PadError};
use cahoots_wireguard::{
    accept, issue, parse_handle_token, render_peer, rotations_remaining, wg_set_command,
    RotationSchedule, WgError,
};

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
    let issued = issue(&mut a, &cid).unwrap();
    assert_eq!(issued.handle.slice, 0);
    assert_eq!(issued.handle.offset, 0);
    let accepted = accept(&mut b, &issued.handle).unwrap();
    assert_eq!(*issued.preshared_key, *accepted);
    // 32 bytes, base64 standard with padding: 44 chars.
    assert_eq!(issued.preshared_key.len(), 44);
    assert!(issued.preshared_key.ends_with('='));
    // The PSK is derived, not the raw pad bytes.
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&*issued.preshared_key)
        .unwrap();
    assert_ne!(&raw[..], &pad_bytes(4096)[..32]);
}

#[test]
fn handle_token_roundtrips() {
    let (mut a, _, cid) = pair();
    let issued = issue(&mut a, &cid).unwrap();
    let token = issued.handle_token();
    assert!(token.starts_with("cahoots-wg:"));
    assert_eq!(parse_handle_token(&token).unwrap(), issued.handle);
    assert_eq!(parse_handle_token("nope").unwrap_err(), WgError::Token);
}

#[test]
fn accepting_twice_is_refused() {
    let (mut a, mut b, cid) = pair();
    let issued = issue(&mut a, &cid).unwrap();
    accept(&mut b, &issued.handle).unwrap();
    assert_eq!(
        accept(&mut b, &issued.handle).unwrap_err(),
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
        wg_set_command("wg0", PEER_PK),
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
}

#[test]
fn rotation_schedule_and_budget() {
    let (mut a, _, cid) = pair();
    assert_eq!(rotations_remaining(&a, &cid), 64, "2048 / 32");
    issue(&mut a, &cid).unwrap();
    assert_eq!(rotations_remaining(&a, &cid), 63);
    let s = RotationSchedule {
        started_at: 1_000,
        interval_secs: 86_400,
    };
    assert_eq!(s.next_rotation_after(1_000), 87_400);
    assert_eq!(s.next_rotation_after(90_000), 173_800);
    assert_eq!(s.rotations_until(1_000 + 86_400 * 3), 3);
    // 63 rotations left at one per day lasts 63 days.
    assert_eq!(s.exhausted_at(63), 1_000 + 86_400 * 63);
}
