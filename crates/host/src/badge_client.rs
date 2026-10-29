//! [`BadgeClient`]: send a custodian [`Request`] to a badge and wait for its [`Response`].
//!
//! The badge answers on its down link while its Serve page is serving. A request goes out as a
//! `KIND_REQUEST` frame with a sequence number; the answer is the `KIND_RESPONSE` frame that
//! echoes it. Frames with another number (a late answer to a request that timed out) and any
//! other bytes are skipped.
//!
//! - [`BadgeClient::Serial`]: a real board on a serial port (USB). The link pings the board
//!   twice a second and prints the board's log lines to stderr.
//! - [`BadgeClient::Sim`]: a simulated badge (`cahoots-host badge`), on the Unix socket it
//!   listens on for the card below (`--listen`).
//!
//! A request the badge asks its holder about waits up to [`ASK_TIMEOUT`], long enough to read
//! the screen and press the buttons; anything else waits [`QUICK_TIMEOUT`], asking again while
//! the badge is busy.

use crate::link::{SerialLink, SocketLink};
use cahoots_core::frame::{self, Frame, FrameError};
use cahoots_custodian::wire::{
    ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE, MAX_REQUEST_LEN,
};
use cahoots_hal::Link;
use std::fmt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a request the holder is asked about waits for the answer.
pub const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// How long any other request waits.
pub const QUICK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait before asking a busy badge again.
const BUSY_RETRY: Duration = Duration::from_millis(250);
/// How long [`BadgeClient::sim`] keeps trying to reach the simulated badge's socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a request got no answer.
#[derive(Debug)]
pub enum ClientError {
    /// The port or socket would not open.
    Open(String),
    /// The request does not fit the badge's request limit.
    TooLong(usize),
    /// No answer in time.
    Timeout,
    /// The link went away.
    Closed,
    /// An answer came that does not decode.
    Malformed,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Open(e) => write!(f, "could not reach the badge: {e}"),
            ClientError::TooLong(n) => write!(
                f,
                "the request is {n} bytes, more than the {MAX_REQUEST_LEN} a badge reads"
            ),
            ClientError::Timeout => write!(f, "the badge did not answer in time"),
            ClientError::Closed => write!(f, "the link to the badge closed"),
            ClientError::Malformed => write!(f, "the badge's answer does not decode"),
        }
    }
}

impl std::error::Error for ClientError {}

/// One link to a badge and the sequence numbers sent on it.
pub struct Channel<L: Link> {
    link: L,
    seq: u32,
    rx: Vec<u8>,
}

impl<L: Link> Channel<L> {
    fn new(link: L) -> Self {
        Channel {
            link,
            // Start somewhere else each run, so an answer meant for an earlier process on the
            // same link is not taken for this one's.
            seq: std::process::id().wrapping_mul(0x9E37_79B9),
            rx: Vec::new(),
        }
    }

    fn request(&mut self, req: &Request, timeout: Duration) -> Result<Response, ClientError> {
        let payload = req.encode();
        if payload.len() > MAX_REQUEST_LEN {
            return Err(ClientError::TooLong(payload.len()));
        }
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        self.link.send(&frame::encode(&Frame {
            kind: KIND_REQUEST,
            seq,
            payload: &payload,
        }));
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 4096];
        loop {
            // Whole frames first: skip what is not our answer, resync past bytes that are not
            // a frame.
            loop {
                match frame::decode(&self.rx) {
                    Ok((f, used)) => {
                        let answer = (f.kind == KIND_RESPONSE && f.seq == seq)
                            .then(|| Response::decode(f.payload));
                        self.rx.drain(..used);
                        if let Some(r) = answer {
                            return r.map_err(|_| ClientError::Malformed);
                        }
                    }
                    Err(FrameError::Incomplete) => {
                        let whole = |i: &usize| frame::decode(&self.rx[*i..]).is_ok();
                        match (1..self.rx.len()).find(whole) {
                            Some(skip) => {
                                self.rx.drain(..skip);
                            }
                            None => break,
                        }
                    }
                    Err(_) => {
                        self.rx.remove(0);
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(ClientError::Timeout);
            }
            let n = self.link.recv(&mut buf);
            if n == 0 {
                if !self.link.peer_present() {
                    return Err(ClientError::Closed);
                }
                std::thread::sleep(Duration::from_millis(2));
            } else {
                self.rx.extend_from_slice(&buf[..n]);
            }
        }
    }
}

/// A link to one badge. See the module docs.
pub enum BadgeClient {
    Serial(Channel<SerialLink>),
    Sim(Channel<SocketLink>),
}

impl BadgeClient {
    /// A real board on the serial port at `path`.
    pub fn serial(path: &str) -> Result<BadgeClient, ClientError> {
        SerialLink::open(path, "badge")
            .map(|mut l| {
                // The bench driver, if on, walks the badge to its Serve page first.
                l.settle(Duration::from_secs(90));
                BadgeClient::Serial(Channel::new(l))
            })
            .map_err(|e| ClientError::Open(format!("{path}: {e}")))
    }

    /// A simulated badge listening on the Unix socket `path`. Keeps trying for 30 seconds,
    /// so the badge may start after the caller.
    pub fn sim(path: &Path) -> Result<BadgeClient, ClientError> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let stream = loop {
            match UnixStream::connect(path) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => return Err(ClientError::Open(format!("{}: {e}", path.display()))),
            }
        };
        SocketLink::new(stream)
            .map(|l| BadgeClient::Sim(Channel::new(l)))
            .map_err(|e| ClientError::Open(e.to_string()))
    }

    /// Send `req` and wait for the answer: [`ASK_TIMEOUT`] for a request the holder is asked
    /// about, [`QUICK_TIMEOUT`] for the rest.
    pub fn request(&mut self, req: Request) -> Result<Response, ClientError> {
        let timeout = if asks(&req) {
            ASK_TIMEOUT
        } else {
            QUICK_TIMEOUT
        };
        self.request_within(req, timeout)
    }

    /// Send `req` and wait at most `timeout` for the answer. A request the holder is not
    /// asked about is sent again while the badge answers `Busy` (it is not serving yet, or is
    /// asking about another request) and time is left; one the holder is asked about is not,
    /// so nobody is asked twice.
    pub fn request_within(
        &mut self,
        req: Request,
        timeout: Duration,
    ) -> Result<Response, ClientError> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let answer = match self {
                BadgeClient::Serial(c) => c.request(&req, left),
                BadgeClient::Sim(c) => c.request(&req, left),
            }?;
            let busy = matches!(
                answer,
                Response::Error {
                    code: ErrorCode::Busy
                }
            );
            if !busy || asks(&req) || Instant::now() + BUSY_RETRY >= deadline {
                return Ok(answer);
            }
            std::thread::sleep(BUSY_RETRY);
        }
    }
}

/// True for a request the badge shows its holder before answering.
fn asks(req: &Request) -> bool {
    match req {
        Request::Identify | Request::Remaining { .. } => false,
        Request::Chip { op, .. } => op.needs_consent(),
        _ => true,
    }
}

/// A badge's error answer in words, for messages to a person.
pub fn describe_error(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Denied => "refused on the badge",
        ErrorCode::Busy => {
            "the badge is busy: it is not serving (choose Serve and press both buttons) or is asking about another request"
        }
        ErrorCode::UnknownCircle => "the badge does not hold that circle",
        ErrorCode::NoChip => "the badge cannot do this: the key is not in a chip, or the circle keeps no root",
        ErrorCode::ChipError => "the badge's chip failed",
        ErrorCode::Unparseable => "the badge will not sign this: it cannot show what it is",
        ErrorCode::Malformed => "the badge could not read the request",
        _ => "the badge refused the request",
    }
}
