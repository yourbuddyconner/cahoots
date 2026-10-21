#![no_main]
use cahoots_core::frame::{decode, encode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok((f, used)) = decode(data) {
        assert!(used <= data.len());
        let re = encode(&f);
        assert_eq!(&re[..], &data[..used]);
    }
});
