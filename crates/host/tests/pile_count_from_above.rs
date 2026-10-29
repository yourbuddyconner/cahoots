//! A board on top of two simulated cards presses Both: its Count comes down the serial link,
//! followed at once by its first ceremony frames. Both cards must become ready.

use cahoots_core::frame::{encode, Frame};
use cahoots_hal::pile::{Pile, KIND_COUNT, KIND_HELLO, KIND_SEAT};
use cahoots_hal::Link;
use cahoots_host::link::{Absent, SocketLink};
use std::os::unix::net::UnixStream;

fn frame(kind: u8, seq: u32, payload: &[u8]) -> Vec<u8> {
    encode(&Frame { kind, seq, payload })
}

#[test]
fn count_from_a_board_above_makes_both_cards_ready() {
    let (a, b) = UnixStream::pair().unwrap();
    let (top_up, mut board) = UnixStream::pair().unwrap();
    board.set_nonblocking(true).unwrap();
    let mut card0 = Pile::new(Box::new(Absent) as Box<dyn Link>, Box::new(SocketLink::new(a).unwrap()), "bench", 16384, false);
    let mut card1 = Pile::new(Box::new(SocketLink::new(b).unwrap()) as Box<dyn Link>, Box::new(SocketLink::new(top_up).unwrap()), "card1", 16384, false);
    let mut spin = |n: usize| {
        for _ in 0..n {
            card0.poll().unwrap();
            card1.poll().unwrap();
        }
    };
    spin(10);
    // The board looks for the pile: Hello down.
    use std::io::{Read, Write};
    board.write_all(&frame(KIND_HELLO, 0, &[])).unwrap();
    spin(10);
    let mut buf = vec![0u8; 4096];
    let n = board.read(&mut buf).unwrap_or(0);
    let (f, _) = cahoots_core::frame::decode(&buf[..n]).expect("a seat frame up");
    assert_eq!(f.kind, KIND_SEAT);
    assert_eq!(f.payload[0], 1, "card1 is seat 1");
    // Both on the board: Count down, then the board's first ceremony frames right behind it.
    let mut bytes = frame(KIND_COUNT, 1, &[3]);
    bytes.extend(frame(1, 0, &[0u8; 14]));
    bytes.extend(frame(2, 1, &[7u8; 1283]));
    board.write_all(&bytes).unwrap();
    let mut ready1 = None;
    let mut ready0 = None;
    for _ in 0..200 {
        if ready1.is_none() { ready1 = card1.poll().unwrap(); }
        if ready0.is_none() { ready0 = card0.poll().unwrap(); }
        if ready0.is_some() && ready1.is_some() { break; }
    }
    assert!(ready1.is_some(), "card1 never became ready");
    assert!(ready0.is_some(), "card0 never became ready");
}
