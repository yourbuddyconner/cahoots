//! A `HostBoard` with a kept circle, driven into Serve with `buttons.txt`, answers an
//! `Identify` request from a plain socket client on its down link.

use cahoots_badge::{word_indices, Badge, Board, Circle};
use cahoots_core::checkwords::check_words;
use cahoots_core::circle_id::circle_id;
use cahoots_core::frame::{self, Frame, FrameError};
use cahoots_core::params::CircleParams;
use cahoots_custodian::wire::{Request, Response, KIND_REQUEST, KIND_RESPONSE};
use cahoots_host::board::HostBoard;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PAD_LEN: u32 = 16 * 1024;

fn wait_status(dir: &Path, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = std::fs::read_to_string(dir.join("status.txt")).unwrap_or_default();
        if s == want {
            return;
        }
        assert!(Instant::now() < deadline, "stuck at {s:?}, want {want:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Keep a circle in `dir` the way a `HostBoard` keeps one after a ceremony.
fn keep_a_circle(dir: &Path) -> Circle {
    let params = CircleParams::new(2, PAD_LEN, "bench", false).unwrap();
    let pad: Vec<u8> = (0..PAD_LEN).map(|i| (i * 31 + 7) as u8).collect();
    let circle = Circle {
        params: params.clone(),
        seat: 1,
        circle_id: circle_id(&pad),
        words: word_indices(&check_words(&params, &pad)).unwrap(),
    };
    let mut board = HostBoard::new(dir, "-", "-", "bench", PAD_LEN, Some(5));
    board.write_pad(&pad).unwrap();
    board.records(true).unwrap();
    board.save_circle(&circle).unwrap();
    circle
}

#[test]
fn a_host_board_in_serve_answers_identify_on_its_down_link() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("card");
    let sock = tmp.path().join("card.sock");
    let kept = keep_a_circle(&dir);

    let stop = Arc::new(AtomicBool::new(false));
    let badge = {
        let (dir, sock, stop) = (dir.clone(), sock.clone(), stop.clone());
        std::thread::spawn(move || {
            // Blocks until the laptop below connects to `sock`.
            let board = HostBoard::new(&dir, sock.to_str().unwrap(), "-", "bench", PAD_LEN, None);
            let mut badge = Badge::new(board);
            badge.boot();
            while !stop.load(Ordering::Relaxed) {
                badge.idle_step();
            }
        })
    };

    // The laptop: a plain Unix socket client carrying raw frame bytes.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut laptop = loop {
        match UnixStream::connect(&sock) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("connect {}: {e}", sock.display()),
        }
    };

    // Words, then A to key as code, A to serve, B to start serving.
    wait_status(&dir, "words");
    std::fs::write(dir.join("buttons.tmp"), "A\nA\nB\n").unwrap();
    std::fs::rename(dir.join("buttons.tmp"), dir.join("buttons.txt")).unwrap();
    wait_status(&dir, "serving. A for menu");

    let payload = Request::Identify.encode();
    laptop
        .write_all(&frame::encode(&Frame {
            kind: KIND_REQUEST,
            seq: 77,
            payload: &payload,
        }))
        .unwrap();

    laptop
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut rx = Vec::new();
    let mut buf = [0u8; 512];
    let (kind, seq, response) = loop {
        match frame::decode(&rx) {
            Ok((f, _)) => break (f.kind, f.seq, Response::decode(f.payload).unwrap()),
            Err(FrameError::Incomplete) => {}
            Err(e) => panic!("not a frame: {e:?} in {rx:?}"),
        }
        assert!(Instant::now() < deadline, "no response; got {rx:?}");
        match laptop.read(&mut buf) {
            Ok(0) => panic!("the badge closed the link"),
            Ok(n) => rx.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => panic!("read: {e}"),
        }
    };

    stop.store(true, Ordering::Relaxed);
    badge.join().unwrap();

    assert_eq!(kind, KIND_RESPONSE);
    assert_eq!(seq, 77);
    match response {
        Response::Identity { circles, tier, .. } => {
            assert_eq!(tier, 0, "the simulator is tier 0");
            assert_eq!(circles.len(), 1);
            assert_eq!(circles[0].circle_id, kept.circle_id);
            assert_eq!(circles[0].name, "bench");
            assert_eq!(circles[0].seat, 1);
            assert_eq!(circles[0].n, 2);
        }
        other => panic!("not an identity: {other:?}"),
    }
}
