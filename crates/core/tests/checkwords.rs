use cahoots_core::checkwords::{check_words, WORDLIST};
use cahoots_core::params::CircleParams;

#[test]
fn wordlist_is_bip39_english() {
    assert_eq!(WORDLIST.len(), 2048);
    assert_eq!(WORDLIST[0], "abandon");
    assert_eq!(WORDLIST[2047], "zoo");
    assert_eq!(WORDLIST[1024], "length");
}

#[test]
fn words_are_first_66_bits_of_digest_eleven_bits_each() {
    // digest with first bytes 0x00 0x08 ... would map to index 0 then 1
    let idx = cahoots_core::checkwords::indices_from_digest(&[
        0x00, 0x00, 0x04, 0x01, 0x00, 0x40, 0x10, 0x04, 0x00,
    ]);
    assert_eq!(idx, [0, 1, 2, 4, 8, 16]);
}

#[test]
fn same_pad_same_words_different_pad_different_words() {
    let p = CircleParams::new(2, 1024, "c", false).unwrap();
    let pad = [3u8; 1024];
    let w1 = check_words(&p, &pad);
    let w2 = check_words(&p, &pad);
    assert_eq!(w1, w2);
    let mut pad2 = pad;
    pad2[9] ^= 0x80;
    assert_ne!(check_words(&p, &pad2), w1);
    for w in w1 {
        assert!(WORDLIST.contains(&w));
    }
}
