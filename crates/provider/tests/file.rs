use cahoots_core::params::CircleParams;
use cahoots_provider::{FileProvider, PadError, PadProvider};

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(53) ^ (i >> 3)) as u8)
        .collect()
}

#[test]
fn file_provider_persists_pad_and_ledger_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 2048, "disk", false).unwrap();
    let cid = {
        let mut p = FileProvider::open(dir.path()).unwrap();
        let cid = p.add_circle(params.clone(), 0, pad_bytes(2048)).unwrap();
        let (h, _) = p.psk(&cid, b"").unwrap();
        assert_eq!(h.offset, 0);
        cid
    };
    let mut p = FileProvider::open(dir.path()).unwrap();
    assert_eq!(p.circles(), vec![cid]);
    assert_eq!(p.remaining(&cid), 1024 - 32);
    let (h, _) = p.psk(&cid, b"").unwrap();
    assert_eq!(h.offset, 32, "ledger survived reopen");
}

#[test]
fn spent_bytes_are_erased_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 2048, "disk", false).unwrap();
    let mut p = FileProvider::open(dir.path()).unwrap();
    let cid = p.add_circle(params, 0, pad_bytes(2048)).unwrap();
    let (_, bytes) = p.spend(&cid, 64).unwrap();
    assert_eq!(&bytes[..], &pad_bytes(2048)[..64]);
    let raw = p.read_pad_for_test(&cid).unwrap();
    assert_eq!(&raw[..64], &[0u8; 64]);
    assert_eq!(&raw[64..128], &pad_bytes(2048)[64..128]);
}

#[test]
fn pad_file_is_encrypted_at_rest() {
    let dir = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 2048, "disk", false).unwrap();
    let mut p = FileProvider::open(dir.path()).unwrap();
    let cid = p.add_circle(params, 0, pad_bytes(2048)).unwrap();
    let pad_path = dir.path().join(hex::encode(cid)).join("pad.enc");
    let on_disk = std::fs::read(&pad_path).unwrap();
    assert!(!on_disk.windows(64).any(|w| w == &pad_bytes(2048)[..64]));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn peer_replay_refused_after_reopen() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 2048, "disk", false).unwrap();
    let mut a = FileProvider::open(dir_a.path()).unwrap();
    let cid = a.add_circle(params.clone(), 0, pad_bytes(2048)).unwrap();
    let (h, psk_a) = a.psk(&cid, b"x").unwrap();
    let psk_b = {
        let mut b = FileProvider::open(dir_b.path()).unwrap();
        b.add_circle(params, 1, pad_bytes(2048)).unwrap();
        b.psk_for(&h, b"x").unwrap()
    };
    assert_eq!(*psk_a, *psk_b);
    let mut b = FileProvider::open(dir_b.path()).unwrap();
    assert_eq!(b.psk_for(&h, b"x").unwrap_err(), PadError::AlreadySpent);
}
