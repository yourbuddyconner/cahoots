//! Two `HostBoard`s in a pile over a Unix socket, each running the badge crate in its own
//! thread. The test presses the buttons by writing `buttons.txt` and watches `screen.txt`.

use cahoots_badge::Badge;
use cahoots_host::board::HostBoard;
use cahoots_host::device::Device;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PAD_LEN: u32 = 16 * 1024;

/// Wait until the board's `screen.txt` satisfies `ok`, or panic after a minute.
fn wait_screen(dir: &Path, ok: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = std::fs::read_to_string(dir.join("screen.txt")).unwrap_or_default();
        if ok(&s) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}: stuck at {s:?}",
            dir.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Press a button: write it to a side file and rename it in, so the board never reads half.
fn press(dir: &Path, button: &str) {
    let tmp = dir.join("buttons.tmp");
    std::fs::write(&tmp, format!("{button}\n")).unwrap();
    std::fs::rename(&tmp, dir.join("buttons.txt")).unwrap();
}

fn circle_id_line(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("circle.txt"))
        .unwrap()
        .lines()
        .find(|l| l.starts_with("circle_id="))
        .unwrap()
        .to_string()
}

#[test]
fn two_host_boards_complete_a_ceremony_through_the_badge_crate() {
    let tmp = tempfile::tempdir().unwrap();
    let bottom: PathBuf = tmp.path().join("bottom");
    let top: PathBuf = tmp.path().join("top");
    let sock = tmp.path().join("top.sock").to_str().unwrap().to_string();

    // The bottom card has nothing below it and connects up to the top card's socket. The
    // top card listens for the bottom card and has nothing above it.
    let spawn = |dir: PathBuf, listen: String, next: String, seed: u64| {
        std::thread::spawn(move || {
            let board = HostBoard::new(&dir, &listen, &next, "bench", PAD_LEN, Some(seed));
            let mut badge = Badge::new(board);
            badge.boot();
            badge.run_once().unwrap();
            // The idle menu asks the board for its links again, as `cahoots-host badge` does.
            badge.idle_step();
            badge.circle().cloned().unwrap()
        })
    };
    let h_bottom = spawn(bottom.clone(), "-".into(), sock.clone(), 11);
    let h_top = spawn(top.clone(), sock, "-".into(), 22);

    // Both on top once it has a seat, then Both on each card at the keep prompt.
    wait_screen(&top, |s| {
        s.contains("seat 1") && s.contains("Both on top starts")
    });
    press(&top, "B");
    for dir in [&bottom, &top] {
        wait_screen(dir, |s| s.contains("keep this circle?"));
        press(dir, "B");
    }
    let c_bottom = h_bottom.join().unwrap();
    let c_top = h_top.join().unwrap();

    assert_eq!(c_bottom.circle_id, c_top.circle_id);
    assert_eq!(c_bottom.seat, 0);
    assert_eq!(c_top.seat, 1);
    assert_eq!(
        c_top.params.name, "bench",
        "the bottom card names the circle"
    );
    assert_eq!(circle_id_line(&bottom), circle_id_line(&top));

    let d_bottom = Device::open(&bottom).unwrap();
    let d_top = Device::open(&top).unwrap();
    assert_eq!(d_bottom.check_words(), d_top.check_words());
    assert_eq!(d_bottom.check_words(), c_bottom.check_words());
    assert_eq!(d_bottom.circle_id(), c_bottom.circle_id);
    assert_eq!(d_bottom.recompute_words(), d_bottom.check_words());

    // Each circle file carries this card's seat key and the sealed manifest; the two
    // manifests are the same one, and each lists its card's key at the host's tier, 0.
    let m_bottom = d_bottom.manifest().unwrap().verify().unwrap();
    let m_top = d_top.manifest().unwrap().verify().unwrap();
    assert_eq!(m_bottom.id(), m_top.id());
    assert_eq!(m_bottom.circle_id, c_bottom.circle_id);
    for (d, m, c) in [(&d_bottom, &m_bottom, &c_bottom), (&d_top, &m_top, &c_top)] {
        let key = d.seat_key().unwrap();
        assert_eq!(m.seat_key(c.seat), Some(&key.public()));
        assert_eq!(*key.to_bytes(), c.seat_key);
        assert_eq!(d.tier(), Some(0));
        assert_eq!(c.tier, 0);
        assert_eq!(m.seats[usize::from(c.seat)].tier, 0);
    }

    // A fresh HostBoard on the same directory loads the circle it kept.
    use cahoots_badge::Board;
    let again = HostBoard::new(&bottom, "-", "-", "bench", PAD_LEN, Some(1));
    assert_eq!(again.load_circle(), Some(c_bottom));

    // A circle file from before seat keys is no circle to a HostBoard.
    let path = bottom.join("circle.txt");
    let old: String = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("seat_key="))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, old).unwrap();
    assert_eq!(again.load_circle(), None);
}
