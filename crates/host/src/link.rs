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
}

pub const PING_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

impl SerialLink {
    pub fn open(path: &str, label: &str) -> io::Result<Self> {
        let mut port = serialport::new(path, 115_200)
            .timeout(std::time::Duration::from_millis(1))
            .open()
            .map_err(|e| io::Error::other(e.to_string()))?;
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
        })
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
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => return,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.broken = true;
                    return;
                }
            }
        }
    }

    fn pull_in(&mut self) {
        let avail = match self.port.bytes_to_read() {
            Ok(n) => n as usize,
            Err(_) => {
                self.broken = true;
                return;
            }
        };
        if avail == 0 {
            return;
        }
        let mut tmp = vec![0u8; avail.min(4096)];
        match self.port.read(&mut tmp) {
            Ok(n) => {
                let mut logs = Vec::new();
                self.inbox.push(&tmp[..n], &mut self.through, &mut logs);
                for line in logs {
                    eprintln!("[{}] {line}", self.label);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => self.broken = true,
        }
    }
}

impl Link for SerialLink {
    fn send(&mut self, frame: &[u8]) {
        self.pending.extend(frame);
        self.ping_if_due();
        self.push_out();
    }

    fn recv(&mut self, buf: &mut [u8]) -> usize {
        self.ping_if_due();
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
