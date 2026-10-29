//! Object ids in the chip. Cahoots keeps its objects in the range `0x4C41_0000..=0x4C41_FFFF`
//! (the bytes "LA"), above the ids the chip's maker reserves.

/// What an object in the chip is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KeyKind {
    /// A seat's signing key.
    Seat = 1,
    /// A circle's root, an HMAC key.
    Root = 2,
    /// A circle's chat epoch counter.
    Epoch = 3,
}

/// A 32-bit object id: `0x4C41_0000 | kind << 8 | slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub u32);

impl KeyId {
    /// The first id of the Cahoots range.
    pub const BASE: u32 = 0x4C41_0000;

    /// The id for `kind` in the circle's `slot` (0 to 255).
    pub const fn new(kind: KeyKind, slot: u8) -> KeyId {
        KeyId(Self::BASE | (kind as u32) << 8 | slot as u32)
    }

    /// Splits an id back into kind and slot. `None` for ids outside the Cahoots range or with an
    /// unknown kind.
    pub fn parse(id: u32) -> Option<(KeyKind, u8)> {
        if id & 0xFFFF_0000 != Self::BASE {
            return None;
        }
        let kind = match (id >> 8) & 0xFF {
            1 => KeyKind::Seat,
            2 => KeyKind::Root,
            3 => KeyKind::Epoch,
            _ => return None,
        };
        Some((kind, id as u8))
    }

    /// True when the id is in the Cahoots range, whatever its kind.
    pub fn in_range(self) -> bool {
        self.0 & 0xFFFF_0000 == Self::BASE
    }
}
