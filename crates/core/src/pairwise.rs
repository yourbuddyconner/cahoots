//! Pairwise keys and spoken words.
//!
//! During the sealed reveal step every member encapsulates a secret to every other member and
//! decapsulates one from each of them, so each pair ends the ceremony holding two X-Wing
//! shared secrets that nobody else can compute. Combined, they are a key for that pair alone.
//! Unlike the pad it is computational (as strong as X-Wing), and unlike the pad it names an
//! individual: a message or a spoken word pair under it could only come from the other member
//! of the pair.
//!
//! Spoken words: a counter per pair, and for each counter value two words each side says, so
//! a listener cannot repeat one side's words back as the other. Either side moves the counter
//! on; if the person on the line cannot produce the new words, they do not hold the key.

use alloc::vec::Vec;

use crate::checkwords::WORDLIST;
use crate::params::CircleParams;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

pub const PAIR_DOMAIN: &[u8] = b"cahoots/pair/v1";
pub const WORDS_DOMAIN: &[u8] = b"cahoots/words/v1";

/// The pair key for members `a` and `b`. `to_b` is the shared secret `a` encapsulated to `b`;
/// `to_a` the one `b` encapsulated to `a`. Both members hold both, so both derive the same key.
pub fn pair_key(
    params: &CircleParams,
    a: u8,
    b: u8,
    to_b: &[u8],
    to_a: &[u8],
) -> Zeroizing<[u8; 32]> {
    let (lo, hi, lo_to_hi, hi_to_lo) = if a < b {
        (a, b, to_b, to_a)
    } else {
        (b, a, to_a, to_b)
    };
    let mut ikm = Zeroizing::new(Vec::with_capacity(64));
    ikm.extend_from_slice(lo_to_hi);
    ikm.extend_from_slice(hi_to_lo);
    let mut info = params.encode();
    info.push(lo);
    info.push(hi);
    let hk = Hkdf::<Sha256>::new(Some(PAIR_DOMAIN), &ikm);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(&info, out.as_mut())
        .expect("32 bytes is a valid length");
    out
}

/// The two words member `speaker` says at `counter`, under a pair key.
pub fn words(pair_key: &[u8; 32], speaker: u8, counter: u32) -> [&'static str; 2] {
    let mut info = Vec::with_capacity(WORDS_DOMAIN.len() + 5);
    info.extend_from_slice(WORDS_DOMAIN);
    info.push(speaker);
    info.extend_from_slice(&counter.to_le_bytes());
    let hk = Hkdf::<Sha256>::new(None, pair_key);
    let mut out = [0u8; 4];
    hk.expand(&info, &mut out)
        .expect("4 bytes is a valid length");
    let v = u32::from_le_bytes(out);
    [
        WORDLIST[(v & 0x7ff) as usize],
        WORDLIST[((v >> 11) & 0x7ff) as usize],
    ]
}
