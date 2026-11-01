//! Object ids in the chip. Cahoots keeps its objects in the range `0x4C41_0000..=0x4C41_FFFF`
//! (the bytes "LA"), above the ids the chip's maker reserves.

/// What an object in the chip is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KeyKind {
    /// A seat's signing key. Slot byte: the circle slot.
    Seat = 1,
    /// A circle's root, an HMAC key. Slot byte: the circle slot.
    Root = 2,
    /// A circle's chat epoch counter. Slot byte: the circle slot.
    Epoch = 3,
    /// A grant counter: the uses of one sign grant, in slots below 8, or the badge's issue
    /// counter, which gives each grant its base, in slot 255.
    Grant = 4,
    /// The one phone slot, an AES authentication object. Badge-wide, slot byte 0.
    Slot = 5,
    /// A hello object, an HMAC key. Slot byte: `circle_slot * 8 + other_seat`.
    Hello = 6,
    /// A circle's public object, a binary object. Slot byte: the circle slot.
    Public = 7,
    /// The inbox a phone writes at pairing, a binary object. Badge-wide, slot byte 0.
    Inbox = 8,
    /// The badge's own user-id authentication object. Badge-wide, slot byte 0.
    Owner = 9,
}

impl KeyKind {
    /// True for the kinds the badge keeps one of for its whole life, outside any circle: the
    /// phone slot, the inbox and the owner object. No circle's cleanup deletes them.
    pub const fn is_badge_wide(self) -> bool {
        matches!(self, KeyKind::Slot | KeyKind::Inbox | KeyKind::Owner)
    }
}

/// A 32-bit object id: `0x4C41_0000 | kind << 8 | slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub u32);

impl KeyId {
    /// The first id of the Cahoots range.
    pub const BASE: u32 = 0x4C41_0000;
    /// The badge's user-id authentication object.
    pub const OWNER: KeyId = KeyId::new(KeyKind::Owner, 0);
    /// The phone slot's AES authentication object.
    pub const SLOT: KeyId = KeyId::new(KeyKind::Slot, 0);
    /// The inbox.
    pub const INBOX: KeyId = KeyId::new(KeyKind::Inbox, 0);

    /// The id for `kind` in the circle's `slot` (0 to 255).
    pub const fn new(kind: KeyKind, slot: u8) -> KeyId {
        KeyId(Self::BASE | (kind as u32) << 8 | slot as u32)
    }

    /// Splits an id back into kind and the raw slot byte. `None` for ids outside the Cahoots
    /// range or with an unknown kind.
    pub fn parse(id: u32) -> Option<(KeyKind, u8)> {
        if id & 0xFFFF_0000 != Self::BASE {
            return None;
        }
        let kind = match (id >> 8) & 0xFF {
            1 => KeyKind::Seat,
            2 => KeyKind::Root,
            3 => KeyKind::Epoch,
            4 => KeyKind::Grant,
            5 => KeyKind::Slot,
            6 => KeyKind::Hello,
            7 => KeyKind::Public,
            8 => KeyKind::Inbox,
            9 => KeyKind::Owner,
            _ => return None,
        };
        Some((kind, id as u8))
    }

    /// True when the id is in the Cahoots range, whatever its kind.
    pub fn in_range(self) -> bool {
        self.0 & 0xFFFF_0000 == Self::BASE
    }

    /// True for the owner object, the phone slot and the inbox.
    pub fn is_badge_wide(self) -> bool {
        KeyId::parse(self.0).is_some_and(|(k, _)| k.is_badge_wide())
    }

    /// The circle slot an object belongs to: the slot byte for the per-circle kinds, the
    /// slot byte divided by 8 for a hello object. `None` for the badge-wide kinds, grant
    /// counters and ids outside the range.
    pub fn circle_slot(self) -> Option<u8> {
        match KeyId::parse(self.0)? {
            (KeyKind::Seat | KeyKind::Root | KeyKind::Epoch | KeyKind::Public, s) => Some(s),
            (KeyKind::Hello, s) => Some(s / 8),
            _ => None,
        }
    }
}
