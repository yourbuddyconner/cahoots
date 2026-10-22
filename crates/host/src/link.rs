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
