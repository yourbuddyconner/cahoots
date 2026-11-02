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
    /// Log lines kept for [`SerialLink::take_logs`], when asked for.
    kept: Option<Vec<String>>,
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
/// that discards or forgets a circle. From home it opens the list with Both, never with Left,
/// which opens or closes the window for a phone's tap. A note gets no press: it goes home by
/// itself.
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
    /// The new circle's keep was answered: home, where the badge goes after its note, is
    /// where this goal ends, and the list is not opened again.
    kept: bool,
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
        Some(Bench::new(goal, seat))
    }

    pub fn new(goal: Goal, seat: Option<u8>) -> Bench {
        Bench {
            goal,
            seat,
            last: String::new(),
            opened: std::time::Instant::now(),
            nudged: false,
            after_draw: None,
            due: None,
            kept: false,
        }
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
    /// something changes, so after a moment with no screen line the driver presses Right to
    /// see where it is: the list from home, the next entry on a list, back from the pile or
    /// a page, never a consent.
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
            self.kept = true;
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
        let home = title == "badge" && footer.contains("A+B  menu");
        if self.goal == Goal::Serve {
            // Nothing to walk to: the badge answers on home and on a circle's pages. The list
            // does not, so "back" there goes home.
            if title == "circles" || title == "chip needs a restart" {
                return Some(if fields.contains(&"> back") { 2 } else { 1 });
            }
            return None;
        }
        if self.kept {
            // The circle is kept: its note goes home by itself, and the goal ends there.
            return None;
        }
        if home {
            return Some(2);
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
        // A circle's actions, a list under its name: "back" is the list of circles.
        if footer == "prev  A+B  open  next" {
            return Some(if fields.contains(&"> back") { 2 } else { 1 });
        }
        // A page one of them opened, under the header `name  tier n`: Left goes back.
        if title.contains("  tier ") && footer.starts_with("back  ") {
            return Some(0);
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
            kept: None,
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

    /// Send one command APDU to the badge's chip through a bench firmware's pass-through
    /// (frame kind 32) and return the answer: its data, then SW1 and SW2. An empty `apdu`
    /// ends the exchange: the badge drops the chip's pin, as when a phone moves away, and the
    /// answer is empty. `TimedOut` when no answer comes within five seconds, which is what a
    /// firmware without the `bench` feature does.
    pub fn apdu(&mut self, apdu: &[u8]) -> io::Result<Vec<u8>> {
        use cahoots_core::frame::{decode, FrameError};
        use cahoots_hal::pile::{APDU_MAX, KIND_APDU, KIND_APDU_REPLY};
        if apdu.len() > APDU_MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("an APDU of {} bytes is longer than the badge takes ({APDU_MAX})", apdu.len()),
            ));
        }
        let f = cahoots_core::frame::Frame { kind: KIND_APDU, seq: self.seq, payload: apdu };
        self.seq = self.seq.wrapping_add(1);
        // Whatever arrived before this command is not its answer.
        self.through.clear();
        self.pending.extend(cahoots_core::frame::encode(&f));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut rx: Vec<u8> = Vec::new();
        loop {
            self.ping_if_due();
            self.push_out();
            self.pull_in();
            rx.extend(self.through.drain(..));
            loop {
                match decode(&rx) {
                    Ok((f, used)) => {
                        let reply = (f.kind == KIND_APDU_REPLY).then(|| f.payload.to_vec());
                        rx.drain(..used);
                        if let Some(r) = reply {
                            return Ok(r);
                        }
                    }
                    Err(FrameError::Incomplete) => break,
                    Err(FrameError::Crc) => {
                        rx.remove(0);
                    }
                }
            }
            if self.broken {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the badge's port went away"));
            }
            if std::time::Instant::now() > deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the badge did not answer. Is it running a bench build?",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Keep the board's log lines from now on, for [`SerialLink::take_logs`].
    pub fn keep_logs(&mut self) {
        self.kept.get_or_insert_with(Vec::new);
    }

    /// The log lines that arrived since the last call, once [`SerialLink::keep_logs`] is on.
    pub fn take_logs(&mut self) -> Vec<String> {
        self.kept.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Send what is queued and take what arrived, with a `Ping` when one is due.
    pub fn pump(&mut self) {
        self.ping_if_due();
        self.push_out();
        self.pull_in();
        self.through.clear();
    }

    /// Press a button on a bench build: `0` Left, `1` Right, `2` Both.
    pub fn send_press(&mut self, code: u8) {
        self.press(code);
        self.push_out();
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
                    if let Some(k) = self.kept.as_mut() {
                        k.push(line);
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

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "screen: badge | owner-bench3 | closed to taps | tap  A+B  menu  menu";
    const LIST_AT_BACK: &str =
        "screen: circles |   owner-bench3 |   new circle |   pair a phone | > back | prev  A+B  open  next";

    #[test]
    fn a_new_circle_goes_from_home_to_the_list_with_both() {
        let mut b = Bench::new(Goal::NewCircle, None);
        assert_eq!(b.react(HOME), Some(2));
        let list = "screen: circles | > owner-bench3 |   new circle |   back | prev  A+B  open  next";
        assert_eq!(b.react(list), Some(1));
        let at_new = "screen: circles |   owner-bench3 | > new circle |   back | prev  A+B  open  next";
        assert_eq!(b.react(at_new), Some(2));
    }

    #[test]
    fn after_the_keep_the_saved_note_and_home_get_no_press() {
        let mut b = Bench::new(Goal::NewCircle, None);
        assert_eq!(b.react("words: a b c d e f"), Some(2));
        assert_eq!(b.react("screen: saved pair-1712"), None);
        assert_eq!(b.react(HOME), None);
    }

    #[test]
    fn serving_stays_on_home_and_leaves_the_list_by_back() {
        let mut b = Bench::new(Goal::Serve, None);
        assert_eq!(b.react(HOME), None);
        let list = "screen: circles | > owner-bench3 |   new circle |   back | prev  A+B  open  next";
        assert_eq!(b.react(list), Some(1));
        assert_eq!(b.react(LIST_AT_BACK), Some(2));
    }

    #[test]
    fn a_note_gets_no_press() {
        let mut b = Bench::new(Goal::NewCircle, None);
        assert_eq!(b.react("screen: unpair the current phone first"), None);
        assert_eq!(b.react("screen: phone unpaired | its taps end now"), None);
    }
}
