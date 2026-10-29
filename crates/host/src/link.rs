use cahoots_hal::Link;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

/// One cable to one neighbour: a Unix socket, written and read on this end.
///
/// Writes never block. Bytes that do not fit the socket buffer wait in `pending` and are
/// flushed on every later `send` or `recv`. A ring of devices that all wrote blocking
/// would deadlock once the reveals outgrow the socket buffers.
pub struct SocketLink {
    stream: UnixStream,
    pending: VecDeque<u8>,
    broken: bool,
}

impl SocketLink {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(SocketLink {
            stream,
            pending: VecDeque::new(),
            broken: false,
        })
    }

    fn flush(&mut self) {
        while !self.pending.is_empty() {
            let (head, _) = self.pending.as_slices();
            match self.stream.write(head) {
                Ok(0) => {
                    self.broken = true;
                    return;
                }
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.broken = true;
                    return;
                }
            }
        }
    }

    /// Bytes queued but not yet handed to the socket.
    pub fn backlog(&self) -> usize {
        self.pending.len()
    }
}

impl Link for SocketLink {
    fn send(&mut self, frame: &[u8]) {
        self.pending.extend(frame);
        self.flush();
    }

    fn recv(&mut self, buf: &mut [u8]) -> usize {
        self.flush();
        match self.stream.read(buf) {
            Ok(0) => {
                self.broken = true;
                0
            }
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => 0,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => 0,
            Err(_) => {
                self.broken = true;
                0
            }
        }
    }

    fn peer_present(&self) -> bool {
        !self.broken
    }

    fn flush(&mut self) -> bool {
        SocketLink::flush(self);
        self.pending.is_empty() || self.broken
    }
}

/// A connector with nothing plugged in: the bottom card's down link, the top card's up link.
pub struct Absent;

impl Link for Absent {
    fn send(&mut self, _frame: &[u8]) {}
    fn recv(&mut self, _buf: &mut [u8]) -> usize {
        0
    }
    fn peer_present(&self) -> bool {
        false
    }
}

/// Sorts the bytes arriving from a board into pass-through bytes for the pile and the
/// ceremony, and `Log` frames (kind 31) for a person to read. Bytes that do not parse as a
/// frame pass through one at a time, the same way the consumers resynchronise.
#[derive(Default)]
pub struct Inbox {
    buf: Vec<u8>,
}

/// If a prefix claims a frame longer than this, it is not a frame: pass a byte and move on.
const INBOX_LIMIT: usize = 1 << 20;

impl Inbox {
    pub fn push(&mut self, bytes: &[u8], through: &mut VecDeque<u8>, logs: &mut Vec<String>) {
        use cahoots_core::frame::{decode, FrameError};
        use cahoots_hal::pile::KIND_LOG;
        self.buf.extend_from_slice(bytes);
        loop {
            match decode(&self.buf) {
                Ok((f, used)) => {
                    if f.kind == KIND_LOG {
                        logs.push(String::from_utf8_lossy(f.payload).into_owned());
                    } else {
                        if trace_frames() {
                            logs.push(format!("<- frame kind {} seq {} ({used} bytes)", f.kind, f.seq));
                        }
                        through.extend(&self.buf[..used]);
                    }
                    self.buf.drain(..used);
                }
                Err(FrameError::Incomplete) if self.buf.len() < INBOX_LIMIT => {
                    // The front may be garbage whose header claims a length that never
                    // arrives, for example the tail of a frame cut by a port reopen. If a
                    // whole frame decodes further on, drop the bytes before it.
                    let skip =
                        (1..self.buf.len().min(4096)).find(|&i| decode(&self.buf[i..]).is_ok());
                    match skip {
                        Some(i) => {
                            for b in self.buf.drain(..i) {
                                through.push_back(b);
                            }
                        }
                        None => break,
                    }
                }
                Err(_) => {
                    if self.buf.is_empty() {
                        break;
                    }
                    through.push_back(self.buf.remove(0));
                }
            }
        }
    }
}

/// One cable to a real board: a serial port, usually a USB one. Mirrors [`SocketLink`], and
/// adds two things a USB port needs: a `Ping` frame every half second so the board can tell
/// the laptop is there (a USB port has no detect line), and `Log` frames from the board
/// printed to stderr instead of handed to the pile.
pub struct SerialLink {
    port: Box<dyn serialport::SerialPort>,
    label: String,
    pending: VecDeque<u8>,
    inbox: Inbox,
    through: VecDeque<u8>,
    last_ping: std::time::Instant,
    seq: u32,
    broken: bool,
    errors: u32,
    bench: Option<Bench>,
}

/// What the bench driver is steering the badge toward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    /// Through the menu to the pile, count it when the badge is the top card, keep the words.
    NewCircle,
    /// Leave the menu alone and allow every prompt: the laptop itself, then each operation.
    Serve,
}

/// Presses the badge's buttons from the laptop, for a firmware built with its `bench`
/// feature. It reads the badge's own screen lines (`screen: ...` log frames) and answers
/// each new one with at most one press, chosen for the goal. It never presses on a screen
/// that discards or forgets a circle.
///
/// Turned on by `CAHOOTS_BENCH=new-circle` or `CAHOOTS_BENCH=serve` in the environment;
/// `CAHOOTS_BENCH_SEAT=<n>` names the seat the badge is expected to be given in the pile.
pub struct Bench {
    goal: Goal,
    seat: Option<u8>,
    last: String,
    opened: std::time::Instant,
    nudged: bool,
    /// A press for a consent prompt, sent once the badge says the prompt is drawn: a
    /// gesture seen during the refresh is thrown away as not an answer.
    after_draw: Option<u8>,
    /// A press held until this moment: the badge discards a gesture on its first loop pass
    /// after a prompt is drawn, which is a few milliseconds after "drawn" reaches the laptop.
    due: Option<(std::time::Instant, u8)>,
}

impl Bench {
    pub fn from_env() -> Option<Bench> {
        let goal = match std::env::var("CAHOOTS_BENCH").ok()?.as_str() {
            "new-circle" => Goal::NewCircle,
            "serve" => Goal::Serve,
            _ => return None,
        };
        let seat = std::env::var("CAHOOTS_BENCH_SEAT")
            .ok()
            .and_then(|s| s.parse().ok());
        Some(Bench {
            goal,
            seat,
            last: String::new(),
            opened: std::time::Instant::now(),
            nudged: false,
            after_draw: None,
            due: None,
        })
    }

    /// A held press whose time has come.
    fn ready(&mut self) -> Option<u8> {
        match self.due {
            Some((t, code)) if std::time::Instant::now() >= t => {
                self.due = None;
                Some(code)
            }
            _ => None,
        }
    }

    /// A badge that was already sitting on a screen when the port opened logs nothing until
    /// something changes, so after a moment with no screen line the driver turns one page
    /// (Right: a page in a menu, back from the pile, never a consent) to see where it is.
    fn nudge(&mut self) -> Option<u8> {
        if self.nudged || !self.last.is_empty() || self.opened.elapsed() < std::time::Duration::from_millis(1500) {
            return None;
        }
        self.nudged = true;
        Some(1)
    }

    /// The press for this screen line, if any. `0` Left, `1` Right, `2` Both.
    pub fn react(&mut self, line: &str) -> Option<u8> {
        // The keep prompt: the badge logs the words right after drawing it and then throws
        // away one gesture (one seen while the prompt was being drawn), so the press waits
        // for the words line, which comes after that.
        if line.starts_with("words: ") {
            return Some(2);
        }
        if line == "drawn" {
            if let Some(code) = self.after_draw.take() {
                self.due = Some((std::time::Instant::now() + std::time::Duration::from_millis(600), code));
            }
            return None;
        }
        let screen = line.strip_prefix("screen: ")?;
        if screen == self.last {
            return None;
        }
        self.last = screen.to_string();
        let fields: Vec<&str> = screen.split(" | ").map(str::trim).collect();
        let title = fields.first().copied().unwrap_or("");
        let footer = fields.last().copied().unwrap_or("");
        // Never the screens that lose something.
        if footer.contains("A+B  discard") || footer.contains("A+B  forget") {
            return None;
        }
        if title == "self-test" || footer == "A+B  continue" {
            return Some(2);
        }
        if footer.contains("A+B  allow") {
            self.after_draw = Some(2);
            return None;
        }
        if title == "stack the cards" {
            let seat = fields
                .iter()
                .find_map(|f| f.strip_prefix("seat ").and_then(|n| n.parse::<u8>().ok()));
            return match (seat, self.seat) {
                (Some(s), Some(want)) if s == want => Some(2),
                _ => None,
            };
        }
        if self.goal == Goal::Serve {
            // Nothing to walk to: the badge answers from any page.
            return None;
        }
        if title == "circles" {
            // The list: `> name` marks the chosen entry.
            let chosen = fields.iter().find(|f| f.starts_with("> ")).map(|f| &f[2..]);
            return match chosen {
                Some("new circle") => Some(2),
                Some(_) => Some(1),
                None => None,
            };
        }
        // A circle's menu: `name  tier n | k/5 | page title | ...`. The words page logs no
        // footer, so the page counter is what marks a menu page.
        let is_menu = fields
            .get(1)
            .is_some_and(|f| f.split_once('/').is_some_and(|(a, b)| a.parse::<u8>().is_ok() && b.parse::<u8>().is_ok()));
        if is_menu || footer.contains("A+B  choose") {
            let page = fields.get(2).copied().unwrap_or("");
            return Some(if page == "circles" { 2 } else { 1 });
        }
        None
    }
}

pub const PING_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

impl SerialLink {
    pub fn open(path: &str, label: &str) -> io::Result<Self> {
        let port = serialport::new(path, 115_200)
            .timeout(std::time::Duration::from_millis(1))
            .open_native()
            .map_err(|e| io::Error::other(e.to_string()))?;
        // Writes must never block: when the board has no room it answers the USB host with
        // NAK, and a blocking write would hold this thread in the kernel, so nothing would
        // read what the board sends back and both sides would wait on each other. With
        // O_NONBLOCK the write returns WouldBlock and the bytes wait in `pending`.
        {
            use std::os::fd::AsRawFd;
            let fd = port.as_raw_fd();
            // SAFETY: fcntl on a file descriptor this struct owns and keeps open.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
        }
        let mut port: Box<dyn serialport::SerialPort> = Box::new(port);
        // Leave both control lines released: the ESP32-S3's USB serial resets the chip, or
        // drops it into its download mode, on the same DTR/RTS dance a flasher uses.
        let _ = port.write_request_to_send(false);
        let _ = port.write_data_terminal_ready(false);
        Ok(SerialLink {
            port,
            label: label.to_string(),
            pending: VecDeque::new(),
            inbox: Inbox::default(),
            through: VecDeque::new(),
            last_ping: std::time::Instant::now(),
            seq: 0,
            broken: false,
            errors: 0,
            bench: Bench::from_env(),
        })
    }

    fn press(&mut self, code: u8) {
        let f = cahoots_core::frame::Frame {
            kind: cahoots_hal::pile::KIND_PRESS,
            seq: self.seq,
            payload: &[code],
        };
        self.seq = self.seq.wrapping_add(1);
        self.pending.extend(cahoots_core::frame::encode(&f));
        eprintln!(
            "[{}] bench: pressed {}",
            self.label,
            ["Left", "Right", "Both"][usize::from(code.min(2))]
        );
    }

    fn ping_if_due(&mut self) {
        if self.last_ping.elapsed() < PING_EVERY {
            return;
        }
        self.last_ping = std::time::Instant::now();
        let f = cahoots_core::frame::Frame {
            kind: cahoots_hal::pile::KIND_PING,
            seq: self.seq,
            payload: &[],
        };
        self.seq = self.seq.wrapping_add(1);
        self.pending.extend(cahoots_core::frame::encode(&f));
    }

    fn push_out(&mut self) {
        while !self.pending.is_empty() {
            let (head, _) = self.pending.as_slices();
            match self.port.write(head) {
                Ok(0) => return,
                Ok(n) => {
                    self.errors = 0;
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => return,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.port_error("write", &e);
                    return;
                }
            }
        }
    }

    /// A port error that is not a timeout or a would-block. The link is broken after
    /// several in a row; one on its own is logged and forgiven, since macOS has been seen
    /// to answer a single query on a live USB serial port with an error.
    fn port_error(&mut self, what: &str, e: &dyn std::fmt::Display) {
        self.errors += 1;
        if trace_frames() || self.errors == BROKEN_AFTER {
            eprintln!("[{}] port {what} failed ({} in a row): {e}", self.label, self.errors);
        }
        if self.errors >= BROKEN_AFTER {
            self.broken = true;
        }
    }

    fn pull_in(&mut self) {
        let avail = match self.port.bytes_to_read() {
            Ok(n) => n as usize,
            Err(e) => {
                self.port_error("bytes_to_read", &e);
                return;
            }
        };
        if avail == 0 {
            return;
        }
        let mut tmp = vec![0u8; avail.min(4096)];
        match self.port.read(&mut tmp) {
            Ok(n) => {
                self.errors = 0;
                if trace_frames() {
                    // Every byte the port delivered, for replaying a bench run offline.
                    use std::io::Write as _;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(format!("/tmp/cahoots-rx-{}.bin", self.label)) {
                        let _ = f.write_all(&tmp[..n]);
                    }
                }
                let mut logs = Vec::new();
                self.inbox.push(&tmp[..n], &mut self.through, &mut logs);
                let mut presses = Vec::new();
                for line in logs {
                    eprintln!("[{}] {line}", self.label);
                    if let Some(b) = self.bench.as_mut() {
                        presses.extend(b.react(&line));
                    }
                }
                for code in presses {
                    self.press(code);
                }
                self.push_out();
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => self.port_error("read", &e),
        }
    }
}

/// Consecutive port errors before a serial link is taken for gone.
const BROKEN_AFTER: u32 = 20;

/// `CAHOOTS_FRAMES=1` prints every frame that crosses a serial link, for the bench.
pub fn trace_frames() -> bool {
    std::env::var_os("CAHOOTS_FRAMES").is_some()
}

impl Link for SerialLink {
    fn send(&mut self, frame: &[u8]) {
        if trace_frames() {
            if let Ok((f, _)) = cahoots_core::frame::decode(frame) {
                eprintln!("[{}] -> frame kind {} seq {} ({} bytes)", self.label, f.kind, f.seq, frame.len());
            }
        }
        self.pending.extend(frame);
        self.ping_if_due();
        self.push_out();
    }

    fn recv(&mut self, buf: &mut [u8]) -> usize {
        self.ping_if_due();
        if let Some(code) = self.bench.as_mut().and_then(Bench::nudge) {
            self.press(code);
        }
        if let Some(code) = self.bench.as_mut().and_then(Bench::ready) {
            self.press(code);
        }
        self.push_out();
        self.pull_in();
        let n = buf.len().min(self.through.len());
        for b in buf.iter_mut().take(n) {
            *b = self.through.pop_front().expect("counted");
        }
        n
    }

    fn peer_present(&self) -> bool {
        !self.broken
    }

    fn flush(&mut self) -> bool {
        self.push_out();
        self.pending.is_empty() || self.broken
    }
}

/// Open a board's serial port and print its `Log` frames until the port goes away. Sends
/// nothing, so the board keeps whatever idea it has of what is plugged into it.
pub fn tail_logs(path: &str, label: &str) -> io::Result<()> {
    let mut port = serialport::new(path, 115_200)
        .timeout(std::time::Duration::from_millis(50))
        .open()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let _ = port.write_request_to_send(false);
    let _ = port.write_data_terminal_ready(false);
    let mut inbox = Inbox::default();
    let mut through = VecDeque::new();
    let mut buf = [0u8; 4096];
    loop {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                let mut logs = Vec::new();
                inbox.push(&buf[..n], &mut through, &mut logs);
                through.clear();
                for line in logs {
                    println!("[{label}] {line}");
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
}
