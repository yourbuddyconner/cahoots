#![no_main]
use cahoots_core::message::{open, parse};
use libfuzzer_sys::fuzz_target;

// Input: wire bytes || pad bytes, split at the first byte's value scaled over the length.
fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let split = usize::from(data[0]) * data.len() / 256;
    let (wire, pad) = data[1..].split_at(split.min(data.len() - 1));
    if let Ok((h, ct, tag)) = parse(wire) {
        let _ = open(&h, ct, tag, pad);
        let _ = h.encode();
        let _ = h.spend_len();
    }
});
