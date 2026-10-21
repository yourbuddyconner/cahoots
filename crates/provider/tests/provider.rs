use cahoots_core::params::CircleParams;
use cahoots_provider::{derive_session_key, MemProvider, PadError, PadProvider, PskHandle};

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(97) ^ (i >> 5)) as u8)
        .collect()
}

fn pair() -> (MemProvider, MemProvider, [u8; 16]) {
    let params = CircleParams::new(2, 4096, "psk", false).unwrap();
    let mut a = MemProvider::new();
    let mut b = MemProvider::new();
    let cid = a.add_circle(params.clone(), 0, pad_bytes(4096)).unwrap();
    let cid_b = b.add_circle(params, 1, pad_bytes(4096)).unwrap();
    assert_eq!(cid, cid_b);
    (a, b, cid)
}

#[test]
fn psk_spends_32_bytes_and_peer_derives_same_bytes() {
    let (mut a, mut b, cid) = pair();
    let (handle, psk_a) = a.psk(&cid, b"ctx").unwrap();
    assert_eq!(
        handle,
        PskHandle {
            circle_id: cid,
            slice: 0,
            offset: 0
        }
    );
    let psk_b = b.psk_for(&handle, b"ctx").unwrap();
    assert_eq!(*psk_a, *psk_b);
    assert_eq!(a.remaining(&cid), 2048 - 32);
    assert_eq!(b.remaining(&cid), 2048, "receiver's own slice is untouched");
    let (h2, psk2) = a.psk(&cid, b"ctx").unwrap();
    assert_eq!(h2.offset, 32);
    assert_ne!(*psk2, *psk_a);
}

#[test]
fn replayed_handle_is_refused() {
    let (mut a, mut b, cid) = pair();
    let (handle, _) = a.psk(&cid, b"").unwrap();
    b.psk_for(&handle, b"").unwrap();
    assert_eq!(b.psk_for(&handle, b"").unwrap_err(), PadError::AlreadySpent);
    let overlap = PskHandle {
        offset: 16,
        ..handle
    };
    assert_eq!(
        b.psk_for(&overlap, b"").unwrap_err(),
        PadError::AlreadySpent
    );
}

#[test]
fn psk_for_refuses_own_slice_and_unknown_circle() {
    let (mut a, _, cid) = pair();
    let (handle, _) = a.psk(&cid, b"").unwrap();
    assert_eq!(a.psk_for(&handle, b"").unwrap_err(), PadError::AlreadySpent);
    let other = PskHandle {
        circle_id: [9; 16],
        ..handle
    };
    assert_eq!(a.psk_for(&other, b"").unwrap_err(), PadError::UnknownCircle);
    assert_eq!(a.psk(&[9; 16], b"").unwrap_err(), PadError::UnknownCircle);
}

#[test]
fn spend_returns_raw_bytes_and_erases_them() {
    let (mut a, mut b, cid) = pair();
    let (h, bytes) = a.spend(&cid, 100).unwrap();
    assert_eq!(bytes.len(), 100);
    assert_eq!(h.offset, 0);
    assert_eq!(&bytes[..], &pad_bytes(4096)[..100]);
    assert_eq!(a.remaining(&cid), 2048 - 100);
    let peer = b.spend_for(&h, 100).unwrap();
    assert_eq!(*peer, *bytes);
    assert_eq!(b.spend_for(&h, 100).unwrap_err(), PadError::AlreadySpent);
}

#[test]
fn exhausted_slice_is_an_error() {
    let (mut a, _, cid) = pair();
    assert_eq!(a.spend(&cid, 2049).unwrap_err(), PadError::Exhausted);
    a.spend(&cid, 2048).unwrap();
    assert_eq!(a.psk(&cid, b"").unwrap_err(), PadError::Exhausted);
    assert_eq!(a.remaining(&cid), 0);
}

#[test]
fn low_pad_warning_at_ten_percent() {
    let (mut a, _, cid) = pair();
    assert!(!a.low_pad(&cid));
    a.spend(&cid, 1800).unwrap();
    assert!(!a.low_pad(&cid), "248 left of 2048 is 12%");
    a.spend(&cid, 48).unwrap();
    assert!(a.low_pad(&cid), "200 left is below 10%");
}

#[test]
fn session_key_binds_handle_and_context() {
    let h = PskHandle {
        circle_id: [1; 16],
        slice: 2,
        offset: 0x0102,
    };
    let k1 = derive_session_key(&[7; 32], &h, b"tls", b"classical", b"pq");
    let k2 = derive_session_key(&[7; 32], &h, b"tls", b"classical", b"pq");
    assert_eq!(k1, k2);
    assert_ne!(
        k1,
        derive_session_key(&[7; 32], &h, b"mls", b"classical", b"pq")
    );
    assert_ne!(
        k1,
        derive_session_key(&[8; 32], &h, b"tls", b"classical", b"pq")
    );
    let h2 = PskHandle {
        offset: 0x0103,
        ..h
    };
    assert_ne!(
        k1,
        derive_session_key(&[7; 32], &h2, b"tls", b"classical", b"pq")
    );
    use hkdf::Hkdf;
    use sha2::Sha256;
    let mut salt = b"cahoots/psk/v1".to_vec();
    salt.extend_from_slice(&[1; 16]);
    salt.push(2);
    salt.extend_from_slice(&0x0102u32.to_le_bytes());
    salt.extend_from_slice(b"tls");
    let mut ikm = b"classical".to_vec();
    ikm.extend_from_slice(b"pq");
    ikm.extend_from_slice(&[7; 32]);
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(b"cahoots/session-key/v1", &mut okm)
        .unwrap();
    assert_eq!(k1, okm);
}
