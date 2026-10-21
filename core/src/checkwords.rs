//! Check words (handoff 3.4 step 5): the first 66 bits of
//! `SHA-256("cahoots/check/v1" || circle_params || PAD)` as six BIP-39 English words.

use crate::params::CircleParams;
use sha2::{Digest, Sha256};

pub const CHECK_DOMAIN: &[u8] = b"cahoots/check/v1";

/// The BIP-39 English wordlist, 2048 words, in canonical order.
pub static WORDLIST: [&str; 2048] = include_wordlist();

const fn include_wordlist() -> [&'static str; 2048] {
    const RAW: &str = include_str!("bip39-english.txt");
    let bytes = RAW.as_bytes();
    let mut out = [""; 2048];
    let mut word = 0;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            // SAFETY-free slicing: the file is ASCII, so byte offsets are char boundaries.
            let (_, rest) = RAW.as_bytes().split_at(start);
            let (w, _) = rest.split_at(i - start);
            out[word] = match core::str::from_utf8(w) {
                Ok(s) => s,
                Err(_) => panic!("wordlist is not UTF-8"),
            };
            word += 1;
            start = i + 1;
        }
        i += 1;
    }
    assert!(word == 2048, "wordlist must have 2048 lines");
    out
}

/// Split the first 66 bits of `digest` into six 11-bit big-endian indices.
pub fn indices_from_digest(digest: &[u8]) -> [u16; 6] {
    assert!(digest.len() >= 9);
    let mut idx = [0u16; 6];
    for (w, slot) in idx.iter_mut().enumerate() {
        let mut v = 0u16;
        for bit in 0..11 {
            let pos = w * 11 + bit;
            let b = (digest[pos / 8] >> (7 - (pos % 8))) & 1;
            v = (v << 1) | u16::from(b);
        }
        *slot = v;
    }
    idx
}

pub fn check_words(params: &CircleParams, pad: &[u8]) -> [&'static str; 6] {
    let mut h = Sha256::new();
    h.update(CHECK_DOMAIN);
    h.update(params.encode());
    h.update(pad);
    let digest = h.finalize();
    indices_from_digest(&digest).map(|i| WORDLIST[usize::from(i)])
}
