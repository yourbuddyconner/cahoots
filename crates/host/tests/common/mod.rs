//! What the agent, git-sign and roster tests share: a ceremony between two simulated badges
//! with software chips, a badge serving its down link in a thread, the agent as a child
//! process, and the button presses and screen reads the simulator offers.

#![allow(dead_code)]

use cahoots_badge::Badge;
use cahoots_host::board::HostBoard;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const PAD_LEN: u32 = 16 * 1024;
pub const HOST: &str = env!("CARGO_BIN_EXE_cahoots-host");
pub const GIT_SIGN: &str = env!("CARGO_BIN_EXE_cahoots-host-git-sign");

/// Wait until the board's `screen.txt` satisfies `ok`, or panic after a minute.
pub fn wait_screen(dir: &Path, ok: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = std::fs::read_to_string(dir.join("screen.txt")).unwrap_or_default();
        if ok(&s) {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "{}: stuck at {s:?}",
            dir.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait until one whole line of the board's `screen.txt` is `want`.
pub fn wait_line(dir: &Path, want: &str) -> String {
    wait_screen(dir, |s| s.lines().any(|l| l == want))
}

/// Press buttons: write them to a side file and rename it in, so the board never reads half.
pub fn press(dir: &Path, buttons: &str) {
    let tmp = dir.join("buttons.tmp");
    std::fs::write(&tmp, buttons.replace(' ', "\n") + "\n").unwrap();
    std::fs::rename(&tmp, dir.join("buttons.txt")).unwrap();
}

/// Two simulated badges with software chips run a ceremony and keep the circle "bench". The
/// bottom card is seat 0, the top card seat 1. Returns their state directories.
pub fn chip_ceremony(root: &Path) -> (PathBuf, PathBuf) {
    chip_ceremony_named(root, "bench")
}

/// [`chip_ceremony`] for a circle named `name`.
pub fn chip_ceremony_named(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let bottom = root.join("bottom");
    let top = root.join("top");
    let sock = root.join("top.sock").to_str().unwrap().to_string();
    let spawn = |dir: PathBuf, listen: String, next: String, seed: u64| {
        let name = name.to_string();
        std::thread::spawn(move || {
            let board =
                HostBoard::new(&dir, &listen, &next, &name, PAD_LEN, Some(seed)).with_soft_chip();
            let mut badge = Badge::new(board);
            badge.boot();
            badge.run_once().unwrap();
            badge.circle().cloned().unwrap()
        })
    };
    let h_bottom = spawn(bottom.clone(), "-".into(), sock.clone(), 11);
    let h_top = spawn(top.clone(), sock, "-".into(), 22);
    wait_screen(&top, |s| {
        s.contains("seat 1") && s.contains("A+B on the top card")
    });
    press(&top, "B");
    for dir in [&bottom, &top] {
        wait_screen(dir, |s| s.contains("keep this circle?"));
        press(dir, "B");
    }
    let c_bottom = h_bottom.join().unwrap();
    let c_top = h_top.join().unwrap();
    assert_eq!(c_bottom.chip_slot, Some(0), "the seat key is in the chip");
    assert_eq!(c_top.chip_slot, Some(0), "the seat key is in the chip");
    (bottom, top)
}

/// A badge in a thread, its down link on `sock`, running the idle menu until dropped.
pub struct Serving {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub dir: PathBuf,
}

impl Drop for Serving {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Start the badge kept in `dir` (with its software chip) listening on `sock`. It boots once
/// something connects to `sock`; then call [`Serving::start`].
pub fn badge_on(dir: &Path, sock: &Path) -> Serving {
    let stop = Arc::new(AtomicBool::new(false));
    let handle = {
        let (dir, sock, stop) = (dir.to_path_buf(), sock.to_path_buf(), stop.clone());
        std::thread::spawn(move || {
            let board = HostBoard::new(&dir, sock.to_str().unwrap(), "-", "bench", PAD_LEN, None)
                .with_soft_chip();
            let mut badge = Badge::new(board);
            badge.boot();
            while !stop.load(Ordering::Relaxed) {
                badge.idle_step();
            }
        })
    };
    Serving {
        stop,
        handle: Some(handle),
        dir: dir.to_path_buf(),
    }
}

impl Serving {
    /// From the words page, Right twice to Serve and Both to start serving.
    pub fn start(&self) {
        wait_line(&self.dir, "1/5");
        press(&self.dir, "R R B");
        wait_line(&self.dir, "serving");
    }
}

/// A child process killed when dropped.
pub struct Kill(pub Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `cahoots-host agent` on `agent_sock`, talking to the simulated badge listening on
/// `badge_sock`, naming hosts from `known_hosts`. Returns once the badge thread is connected
/// and the agent's socket exists.
pub fn spawn_agent(badge_sock: &Path, agent_sock: &Path, known_hosts: &Path) -> Kill {
    let child = Command::new(HOST)
        .args(["agent", "--sim"])
        .arg(badge_sock)
        .args(["--circle", "bench", "--socket"])
        .arg(agent_sock)
        .arg("--known-hosts")
        .arg(known_hosts)
        .spawn()
        .unwrap();
    let kill = Kill(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    while UnixStream::connect(agent_sock).is_err() {
        assert!(Instant::now() < deadline, "the agent never listened");
        std::thread::sleep(Duration::from_millis(20));
    }
    kill
}

/// Append `b` as an SSH string.
pub fn put_string(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// Read an SSH string from the front of `b`.
pub fn take_string<'a>(b: &mut &'a [u8]) -> &'a [u8] {
    let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
    let s = &b[4..4 + n];
    *b = &b[4 + n..];
    s
}

/// Send one agent message (type byte and body) and read the answer.
pub fn agent_call(s: &mut UnixStream, msg: &[u8]) -> Vec<u8> {
    let mut framed = (msg.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(msg);
    s.write_all(&framed).unwrap();
    let mut len = [0u8; 4];
    s.read_exact(&mut len).unwrap();
    let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
    s.read_exact(&mut body).unwrap();
    body
}

/// The agent's identities: (key blob, comment).
pub fn identities(agent_sock: &Path) -> Vec<(Vec<u8>, String)> {
    let mut s = UnixStream::connect(agent_sock).unwrap();
    let answer = agent_call(&mut s, &[11]);
    assert_eq!(answer[0], 12, "an identities answer");
    let mut b = &answer[1..];
    let n = u32::from_be_bytes(b[..4].try_into().unwrap());
    b = &b[4..];
    (0..n)
        .map(|_| {
            let key = take_string(&mut b).to_vec();
            let comment = String::from_utf8(take_string(&mut b).to_vec()).unwrap();
            (key, comment)
        })
        .collect()
}
