//! A pile of simulated cards on sockets: the top card presses B, everyone agrees.

use cahoots_core::message::MessageType;
use cahoots_host::device::Device;
use cahoots_host::pile::run_pile;

#[test]
fn three_simulated_cards_in_a_pile_make_one_circle() {
    let tmp = tempfile::tempdir().unwrap();
    let devices = run_pile(3, None, "bench", 16 * 1024, tmp.path(), Some(77)).unwrap();
    assert_eq!(devices.len(), 3);
    let words = devices[0].check_words();
    let id = devices[0].circle_id();
    for d in &devices {
        assert_eq!(d.check_words(), words);
        assert_eq!(d.circle_id(), id);
        assert_eq!(d.params().n, 3);
        assert_eq!(d.params().name, "bench", "the bottom card names the circle");
    }
    let mut devices = devices;
    let wire = devices[2].send(MessageType::Text, b"from the top").unwrap();
    let (_, pt) = devices[0].receive(&wire).unwrap();
    assert_eq!(pt, b"from the top");
}

#[test]
fn each_circle_file_keeps_its_seat_key_and_the_one_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let devices = run_pile(3, None, "bench", 16 * 1024, tmp.path(), Some(78)).unwrap();
    let mut ids = Vec::new();
    for (seat, d) in devices.iter().enumerate() {
        // Read back from disk, not from the device the ceremony returned.
        let d = Device::open(d.dir()).unwrap();
        let m = d.manifest().unwrap().verify().unwrap();
        let key = d.seat_key().unwrap();
        assert_eq!(m.seat_key(seat as u8), Some(&key.public()));
        assert_eq!(m.seats[seat].tier, 0);
        assert_eq!(d.tier(), Some(0));
        ids.push(m.id());
    }
    assert!(ids.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn the_cards_chat_on_the_root_they_keep() {
    let tmp = tempfile::tempdir().unwrap();
    let devices = run_pile(3, None, "bench", 16 * 1024, tmp.path(), Some(80)).unwrap();
    // Read back from disk: every card keeps the same root in root.bin.
    let devices: Vec<Device> = devices
        .iter()
        .map(|d| Device::open(d.dir()).unwrap())
        .collect();
    let root = *devices[0].root().unwrap();
    let id = devices[0].circle_id();
    let mut chats: Vec<cahoots_chat::ChatSession> = (0..3u8)
        .map(|seat| {
            let d = &devices[usize::from(seat)];
            assert_eq!(d.root(), Some(&root));
            let mut c = cahoots_chat::ChatSession::new(d.root().unwrap(), id, 3, seat);
            c.mark_synced(); // nothing is on any relay yet
            c
        })
        .collect();
    let before = devices[1].remaining();
    let w = chats[1].send(b"from seat 1").unwrap();
    assert_eq!(chats[0].receive(&w).unwrap().1, b"from seat 1");
    assert_eq!(chats[2].receive(&w).unwrap().1, b"from seat 1");
    assert_eq!(devices[1].remaining(), before, "chat spends no pad");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(devices[0].dir().join("root.bin")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
    // A state directory from before root.bin opens with no root.
    std::fs::remove_file(devices[2].dir().join("root.bin")).unwrap();
    assert!(Device::open(devices[2].dir()).unwrap().root().is_none());
}

#[test]
fn a_circle_file_from_before_seat_keys_still_opens() {
    let tmp = tempfile::tempdir().unwrap();
    let devices = run_pile(2, None, "bench", 16 * 1024, tmp.path(), Some(79)).unwrap();
    let dir = devices[0].dir().to_path_buf();
    drop(devices);
    let path = dir.join("circle.txt");
    let old: String = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|l| {
            !["seat_key=", "tier=", "manifest="]
                .iter()
                .any(|k| l.starts_with(k))
        })
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, old).unwrap();
    let d = Device::open(&dir).unwrap();
    assert!(d.seat_key().is_none());
    assert!(d.manifest().is_none());
    assert_eq!(d.tier(), None);
}

#[test]
fn a_pile_needs_two_cards() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(run_pile(1, None, "x", 16 * 1024, tmp.path(), Some(1)).is_err());
}
