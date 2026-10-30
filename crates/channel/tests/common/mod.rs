#![allow(dead_code)]

use cahoots_core::manifest::{Manifest, Seat};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;

pub const CIRCLE: [u8; 16] = [7; 16];

/// The deterministic filler `crates/vectors` uses.
pub fn fill(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 56) as u8
        })
        .collect()
}

/// Seat `i`'s key, from the vectors' seat seed `fill(350 + i, 32)`.
pub fn seat_key(i: u8) -> SeatKey {
    SeatKey::from_seed(&fill(350 + u64::from(i), 32).try_into().unwrap()).unwrap()
}

/// A three-seat manifest whose seat keys are `seat_key(0..3)`.
pub fn manifest() -> Manifest {
    manifest_for(CIRCLE)
}

/// The same three seats under another circle id.
pub fn manifest_for(circle_id: [u8; 16]) -> Manifest {
    let params = CircleParams::new(3, 192, "channel", false).unwrap();
    Manifest {
        circle_id,
        seats: (0..3)
            .map(|i| Seat {
                seat: i,
                key: seat_key(i).public(),
                tier: 0,
                evidence: Vec::new(),
            })
            .collect(),
        floor: Manifest::default_floor(&params),
        params,
        genesis: 0,
    }
}
