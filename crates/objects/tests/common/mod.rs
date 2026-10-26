#![allow(dead_code)]

use cahoots_core::manifest::Manifest;
use cahoots_core::manifest::Seat;
use cahoots_core::params::CircleParams;
use cahoots_core::seat::{SeatKey, SeatPublic};

pub const CIRCLE: [u8; 16] = [7; 16];

pub fn seat_key(i: u8) -> SeatKey {
    SeatKey::from_seed(&[i + 1; 32]).unwrap()
}

/// A key that is not in any roster: the grantee of a root grant, say.
pub fn other_key(i: u8) -> SeatKey {
    SeatKey::from_seed(&[0x40 + i; 32]).unwrap()
}

pub fn public(k: &SeatKey) -> SeatPublic {
    k.public()
}

/// A three-seat manifest whose seat keys are `seat_key(0..3)`.
pub fn manifest() -> Manifest {
    let params = CircleParams::new(3, 192, "objects", false).unwrap();
    Manifest {
        circle_id: CIRCLE,
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
