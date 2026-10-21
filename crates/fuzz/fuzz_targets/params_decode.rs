#![no_main]
use cahoots_core::params::CircleParams;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = CircleParams::decode(data) {
        assert_eq!(p.encode(), data);
        let _ = p.slice_len();
        let _ = p.slice_start(p.slices() - 1);
    }
});
