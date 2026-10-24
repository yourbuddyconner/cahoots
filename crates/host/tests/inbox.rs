//! The serial inbox passes pile and ceremony bytes through and lifts out log lines.

use cahoots_core::frame::{encode, Frame};
use cahoots_hal::pile::{KIND_HELLO, KIND_LOG, KIND_PING};
use cahoots_host::link::Inbox;
use std::collections::VecDeque;

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    encode(&Frame { kind, seq: 1, payload })
}

#[test]
fn log_frames_are_lifted_out_and_the_rest_passes_through_byte_exact() {
    let hello = frame(KIND_HELLO, b"");
    let log = frame(KIND_LOG, "trng ok".as_bytes());
    let ping = frame(KIND_PING, b"");
    let mut stream = Vec::new();
    stream.extend(&hello);
    stream.extend(&log);
    stream.extend(&ping);
    let mut inbox = Inbox::default();
    let mut through = VecDeque::new();
    let mut logs = Vec::new();
    // Feed it in awkward pieces, as a serial port would.
    for chunk in stream.chunks(5) {
        inbox.push(chunk, &mut through, &mut logs);
    }
    let out: Vec<u8> = through.into_iter().collect();
    let mut expect = hello.clone();
    expect.extend(&ping);
    assert_eq!(out, expect);
    assert_eq!(logs, vec!["trng ok".to_string()]);
}

#[test]
fn garbage_before_a_frame_passes_through_so_the_consumer_can_resync() {
    let mut inbox = Inbox::default();
    let mut through = VecDeque::new();
    let mut logs = Vec::new();
    let mut stream = vec![0xAA, 0xBB];
    stream.extend(frame(KIND_HELLO, b""));
    inbox.push(&stream, &mut through, &mut logs);
    let out: Vec<u8> = through.into_iter().collect();
    assert_eq!(out, stream);
    assert!(logs.is_empty());
}
