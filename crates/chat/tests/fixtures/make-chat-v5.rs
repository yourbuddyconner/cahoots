//! The code that wrote `chat-v5.bin` and `chat-v5-wires.bin`, run at commit 2496588, before
//! generations, as `crates/chat/tests/zz_make_v5_fixture.rs` with
//! `cargo test -p cahoots-chat --test zz_make_v5_fixture`. It is kept here, outside the test
//! targets, so it does not build against the current crate. To check the fixtures, copy it into
//! `crates/chat/tests/` in a checkout of 2496588, run it, and compare the files: nothing in it
//! is random.
//!
//! Seat 1 of 3 reads the first of two root-epoch messages from seat 0 and exports its state
//! (`chat-v5.bin`, magic `CHS2`). `chat-v5-wires.bin` holds both messages, each after its
//! length as u32 LE.

use cahoots_chat::ChatSession;

#[test]
fn make() {
    let root = [0x42u8; 32];
    let cid = [0x0cu8; 16];
    let mut a = ChatSession::new(&root, cid, 3, 0);
    a.mark_synced();
    let mut b = ChatSession::new(&root, cid, 3, 1);
    b.mark_synced();
    let w1 = a.send(b"old history one").unwrap();
    let w2 = a.send(b"old history two").unwrap();
    assert_eq!(b.receive(&w1).unwrap().1, b"old history one");
    let state = b.export_state();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::write(dir.join("chat-v5.bin"), &state[..]).unwrap();
    let mut wires = Vec::new();
    for w in [&w1, &w2] {
        wires.extend_from_slice(&(w.len() as u32).to_le_bytes());
        wires.extend_from_slice(w);
    }
    std::fs::write(dir.join("chat-v5-wires.bin"), wires).unwrap();
}
