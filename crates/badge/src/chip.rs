//! The badge's side of the chip: a seat key that signs in the chip, and the chip record.
//!
//! [`ChipSeat`] is a [`SeatSigner`] whose key is a P-256 object in the chip: the badge hashes
//! the message and the chip signs the digest, so the key never leaves the chip.
//!
//! The chip record is the badge's own list of the slots it made objects in: for each slot, the
//! circle id and which of the seat key, the circle root and the epoch counter were made. It is
//! kept apart from the circle record. An object in a slot the record does not know (a wiped
//! record, a reflashed badge, a ceremony cut off before the keep) is deleted at boot and logged
//! by [`ChipRecord::reconcile`], never reused. The badge-wide objects (the owner object, the
//! phone slot and the inbox) belong to no circle slot and are never deleted that way.
//!
//! The record is CBOR: `[slots, phone, badge nonce, generation]`, the slots an array of
//! `[slot, circle id (bytes), has seat, has root, has epoch]` in slot order, the phone `null`
//! or `[name, public key (33 bytes)]`. The decoder re-encodes and refuses anything that
//! differs. A board stores it as `CAHC2` and the CBOR ([`ChipRecord::to_stored`]); the record
//! of earlier firmware, `CAHC`, reads as no record ([`ChipRecord::from_stored`]), and a badge
//! with no record of [`GENERATION`] wipes the chip and its circles at boot ([`first_boot`]).
//!
//! The badge owns its chip: [`first_boot`] makes the owner object, a user id only the badge
//! knows, then the phone slot as a placeholder whose key nobody holds, and the inbox. Every
//! policy names identities by those ids, and none of them is ever left empty for a tap to
//! claim.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_chip::{ChipError, ChipOps, KeyId, KeyKind, ObjectPolicy};
use cahoots_core::circle_id::CircleId;
use cahoots_core::seat::{SeatPublic, SeatSigner, Signature};
use minicbor::{Decode, Encode};
use sha2::{Digest, Sha256};

/// A seat key held in the chip as object `id`.
///
/// `SeatSigner` cannot fail, so a chip error is kept in `error` for the caller to log, and the
/// call returns a value no check accepts: an all-zero public key or signature. The ceremony
/// verifies its own manifest signature before sending it, so a zero signature is refused there.
pub struct ChipSeat<'a> {
    pub chip: &'a mut dyn ChipOps,
    pub id: KeyId,
    /// The last chip error, if a call failed.
    pub error: Option<ChipError>,
}

impl<'a> ChipSeat<'a> {
    pub fn new(chip: &'a mut dyn ChipOps, id: KeyId) -> Self {
        ChipSeat {
            chip,
            id,
            error: None,
        }
    }
}

impl SeatSigner for ChipSeat<'_> {
    fn public(&mut self) -> SeatPublic {
        let public = self
            .chip
            .public(self.id)
            .and_then(|b| SeatPublic::parse(&b).map_err(|_| ChipError::NoSuchKey));
        match public {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e);
                SeatPublic([0; 33])
            }
        }
    }

    /// P-256 ECDSA over SHA-256(`msg`), the digest computed here and signed in the chip.
    fn sign(&mut self, msg: &[u8]) -> Signature {
        let digest: [u8; 32] = Sha256::digest(msg).into();
        match self.chip.sign_p256(self.id, &digest) {
            Ok(sig) => sig,
            Err(e) => {
                self.error = Some(e);
                [0; 64]
            }
        }
    }
}

/// The objects a circle has in the chip, by kind.
pub const SLOT_KINDS: [KeyKind; 3] = [KeyKind::Seat, KeyKind::Root, KeyKind::Epoch];

/// One slot the badge made objects in.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct SlotEntry {
    #[n(0)]
    pub slot: u8,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub circle_id: CircleId,
    #[n(2)]
    pub has_seat: bool,
    #[n(3)]
    pub has_root: bool,
    #[n(4)]
    pub has_epoch: bool,
}

impl SlotEntry {
    fn has(&self, kind: KeyKind) -> bool {
        match kind {
            KeyKind::Seat => self.has_seat,
            KeyKind::Root => self.has_root,
            KeyKind::Epoch => self.has_epoch,
            // Grant counters are in the grants record, not here, and the other kinds are not
            // flagged in a slot entry.
            _ => false,
        }
    }

    fn clear(&mut self, kind: KeyKind) {
        match kind {
            KeyKind::Seat => self.has_seat = false,
            KeyKind::Root => self.has_root = false,
            KeyKind::Epoch => self.has_epoch = false,
            _ => {}
        }
    }
}

/// The generation of a chip record made by firmware that owns its chip. A record of any other
/// generation, or none, means the chip's objects were made the old way, reachable by any
/// session: the badge wipes them at boot.
pub const GENERATION: u8 = 2;

/// The generation of a record of a chip this badge owns whose phone slot, inbox and issue
/// counter must be made again at the next boot: written before a conversation that leaves one
/// of those ids empty for a moment, so a cut-off conversation never leaves one for a tap to
/// fill and the badge to adopt.
pub const REMAKE_GENERATION: u8 = 3;

/// The generation of the record a badge stores when it refuses its chip for good: an object a
/// tap made stays where the badge's own must be, or the setup failed after the owner object,
/// which nobody may delete, was made. Later boots refuse at once and touch nothing.
pub const REFUSED_GENERATION: u8 = 0xFF;

/// The bytes a stored chip record starts with.
pub const RECORD_MAGIC: &[u8; 5] = b"CAHC2";

/// The inbox's length: room for a phone's key, its name and the slot key, as CBOR.
pub const INBOX_LEN: usize = 128;

/// The phone paired to this badge.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct PhoneSlot {
    #[n(0)]
    pub name: String,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub public: [u8; 33],
}

/// The chip record: every slot the badge made objects in, in slot order, the paired phone,
/// the badge nonce drawn at first boot, and the record's generation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
pub struct ChipRecord {
    #[n(0)]
    pub slots: Vec<SlotEntry>,
    #[n(1)]
    pub phone: Option<PhoneSlot>,
    #[cbor(n(2), with = "minicbor::bytes")]
    pub badge_nonce: [u8; 16],
    #[n(3)]
    pub generation: u8,
    /// The circle slots a wipe left an object in that could not be deleted, in order, or
    /// `None` for none. Boot tries them again ([`crate::Badge::boot`]).
    #[n(4)]
    pub stuck: Option<Vec<u8>>,
}

/// The record of earlier firmware, which only [`ChipRecord::from_stored`] reads, to know it.
#[derive(Decode)]
struct RecordV1 {
    #[n(0)]
    #[allow(dead_code)]
    slots: Vec<SlotEntry>,
}

/// A stored chip record that does not decode, or decodes to something not canonical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

impl ChipRecord {
    pub fn encode(&self) -> Vec<u8> {
        minicbor::to_vec(self).expect("encoding into a Vec cannot fail")
    }

    /// Strict: exactly one canonical encoding, slots in increasing order with none twice.
    pub fn decode(bytes: &[u8]) -> Result<ChipRecord, Malformed> {
        let r: ChipRecord = minicbor::decode(bytes).map_err(|_| Malformed)?;
        let stuck_ok = r
            .stuck
            .as_ref()
            .is_none_or(|v| !v.is_empty() && v.windows(2).all(|w| w[0] < w[1]));
        if r.encode() != bytes || r.slots.windows(2).any(|w| w[0].slot >= w[1].slot) || !stuck_ok {
            return Err(Malformed);
        }
        Ok(r)
    }

    /// The record as a board stores it: `CAHC2` and the CBOR.
    pub fn to_stored(&self) -> Vec<u8> {
        let mut out = RECORD_MAGIC.to_vec();
        out.extend_from_slice(&self.encode());
        out
    }

    /// A stored record: `Ok(None)` for the record of earlier firmware (`CAHC` in a sector, or
    /// its bare CBOR in a file), which the badge reads as no record; `Malformed` for anything
    /// else that is not [`ChipRecord::to_stored`]'s output.
    pub fn from_stored(bytes: &[u8]) -> Result<Option<ChipRecord>, Malformed> {
        if let Some(body) = bytes.strip_prefix(RECORD_MAGIC.as_slice()) {
            return ChipRecord::decode(body).map(Some);
        }
        if bytes.starts_with(b"CAHC") || minicbor::decode::<RecordV1>(bytes).is_ok() {
            return Ok(None);
        }
        Err(Malformed)
    }

    /// The record a badge stores when it refuses its chip for good: no slots, no phone, and
    /// [`REFUSED_GENERATION`].
    pub fn refused() -> ChipRecord {
        ChipRecord {
            generation: REFUSED_GENERATION,
            ..ChipRecord::default()
        }
    }

    /// True for the record of a chip the badge refused for good.
    pub fn is_refused(&self) -> bool {
        self.generation == REFUSED_GENERATION
    }

    /// True if a wipe left an object in circle slot `slot`.
    pub fn is_stuck(&self, slot: u8) -> bool {
        self.stuck.as_ref().is_some_and(|v| v.contains(&slot))
    }

    /// Mark or unmark circle slot `slot` as holding an object a wipe could not delete.
    pub fn set_stuck(&mut self, slot: u8, stuck: bool) {
        let mut v = self.stuck.take().unwrap_or_default();
        v.retain(|&s| s != slot);
        if stuck {
            v.push(slot);
            v.sort_unstable();
        }
        self.stuck = (!v.is_empty()).then_some(v);
    }

    /// True if the phone slot, the inbox and the issue counter must be made again at the next
    /// boot, whatever stands at their ids.
    pub fn needs_remake(&self) -> bool {
        self.generation == REMAKE_GENERATION
    }

    /// True for a record made by firmware that owns its chip.
    pub fn is_current(&self) -> bool {
        self.generation == GENERATION || self.generation == REMAKE_GENERATION
    }

    pub fn entry(&self, slot: u8) -> Option<&SlotEntry> {
        self.slots.iter().find(|e| e.slot == slot)
    }

    /// The lowest slot the record does not use, or `None` if all 256 are used.
    pub fn next_free_slot(&self) -> Option<u8> {
        (0..=u8::MAX).find(|&s| self.entry(s).is_none())
    }

    /// Add or replace the entry for `e.slot`, keeping slot order.
    pub fn insert(&mut self, e: SlotEntry) {
        self.slots.retain(|x| x.slot != e.slot);
        let at = self.slots.partition_point(|x| x.slot < e.slot);
        self.slots.insert(at, e);
    }

    pub fn remove(&mut self, slot: u8) {
        self.slots.retain(|x| x.slot != slot);
    }

    /// Delete every Cahoots object in a slot this record does not know, and clear the flag of
    /// any object the record says was made but the chip does not hold. Returns a log line for
    /// each change. Grant counters are left alone: [`crate::grants::sweep`] owns them.
    pub fn reconcile(&mut self, chip: &mut dyn ChipOps) -> Vec<String> {
        self.reconcile_keeping(chip, &[])
    }

    /// Like [`reconcile`](Self::reconcile), never deleting objects in the slots in `keep`
    /// (the slot a stored circle names), whether the record knows them or not.
    pub fn reconcile_keeping(&mut self, chip: &mut dyn ChipOps, keep: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let ids = match chip.ids() {
            Ok(ids) => ids,
            Err(e) => {
                lines.push(format!("chip: could not list its objects: {e:?}"));
                return lines;
            }
        };
        for id in &ids {
            // A grant counter's slot is a grant's, not a circle's: the grants record owns it,
            // and the grant sweep deletes the ones it does not know. The owner object, the
            // phone slot and the inbox belong to the badge, not to a circle.
            let Some(slot) = circle_slot(*id) else {
                continue;
            };
            if self.entry(slot).is_some() {
                continue;
            }
            if keep.contains(&slot) {
                lines.push(format!(
                    "chip: slot {slot} is not in the chip record but a circle names it, kept object {:08x}",
                    id.0
                ));
                continue;
            }
            lines.push(match chip.delete(*id) {
                Ok(()) => format!(
                    "chip: slot {slot} is not in the chip record, deleted object {:08x}",
                    id.0
                ),
                Err(e) => format!(
                    "chip: slot {slot} is not in the chip record, deleting object {:08x} failed: {e:?}",
                    id.0
                ),
            });
        }
        for e in &mut self.slots {
            for kind in SLOT_KINDS {
                if e.has(kind) && !ids.contains(&KeyId::new(kind, e.slot)) {
                    e.clear(kind);
                    lines.push(format!(
                        "chip: slot {} has no {kind:?} object, the record now says so",
                        e.slot
                    ));
                }
            }
        }
        lines
    }
}

/// The circle slot an object belongs to, by its kind: `None` for a grant counter, for the
/// badge-wide objects, and for a kind this firmware does not know, which is no circle's.
fn circle_slot(id: KeyId) -> Option<u8> {
    id.circle_slot()
}

fn delete_quietly(chip: &mut dyn ChipOps, id: KeyId) -> Result<(), ChipError> {
    match chip.delete(id) {
        Ok(()) | Err(ChipError::NoSuchKey) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The objects a wipe could not delete, with why. A tap can make an object whose policy lets
/// nobody delete it, so a wipe tries every object and reports what stayed.
pub type Left = Vec<(KeyId, ChipError)>;

/// The circle slots the badge makes objects in: 0 to 15, one for each place a board keeps a
/// circle in. A hello object's slot byte is `circle_slot * 8 + other_seat`, so a circle slot
/// above 31 would not fit it.
pub const CIRCLE_SLOTS: u8 = 16;
const _: () = assert!((CIRCLE_SLOTS as usize) * 8 <= 256);

fn delete_into(chip: &mut dyn ChipOps, id: KeyId, left: &mut Left) {
    if let Err(e) = delete_quietly(chip, id) {
        if !left.iter().any(|(x, _)| *x == id) {
            left.push((id, e));
        }
    }
}

/// Delete every object of circle slot `slot`: the seat key, the root, the epoch counter, the
/// public object and the hello objects. An object that is not there is not an error. Every
/// object is tried; the ones that stay are returned.
pub fn delete_slot(chip: &mut dyn ChipOps, slot: u8) -> Result<Left, ChipError> {
    let mut left = Left::new();
    for kind in SLOT_KINDS {
        delete_into(chip, KeyId::new(kind, slot), &mut left);
    }
    for id in chip.ids()? {
        if circle_slot(id) == Some(slot) {
            delete_into(chip, id, &mut left);
        }
    }
    Ok(left)
}

/// Delete every circle object and grant counter in the Cahoots range: the wipe at boot. The
/// grant issue counter stays, so a grant base is never given twice on one chip, and so do the
/// badge-wide objects and objects of kinds this firmware does not know. Every object is
/// tried; the ones that stay are returned.
pub fn delete_all(chip: &mut dyn ChipOps) -> Result<Left, ChipError> {
    let issue = issue_id();
    let mut left = Left::new();
    for id in chip.ids()? {
        if id == issue || id.is_badge_wide() || KeyId::parse(id.0).is_none() {
            continue;
        }
        delete_into(chip, id, &mut left);
    }
    Ok(left)
}

/// The grant issue counter's id.
pub fn issue_id() -> KeyId {
    KeyId::new(KeyKind::Grant, crate::grants::ISSUE_SLOT)
}

/// The ids a tap reaches or the badge relies on and must find its own: the phone slot, the
/// inbox and the grant issue counter.
pub const ANTENNA_IDS: [KeyId; 3] = [
    KeyId::SLOT,
    KeyId::INBOX,
    KeyId::new(KeyKind::Grant, crate::grants::ISSUE_SLOT),
];

/// The upgrade wipe: delete every object in the Cahoots range but the owner object, of every
/// kind, the phone slot, the inbox and the issue counter included, since every object made
/// before the badge owned its chip is reachable by any session, and one at the slot or the
/// inbox id may be a tap's. Every object is tried. Returns how many went, and the ones that
/// did not with why.
pub fn delete_for_upgrade(chip: &mut dyn ChipOps) -> Result<(usize, Left), ChipError> {
    let mut deleted = 0;
    let mut left = Left::new();
    for id in chip.ids()? {
        if id == KeyId::OWNER {
            continue;
        }
        match delete_quietly(chip, id) {
            Ok(()) => deleted += 1,
            Err(e) => left.push((id, e)),
        }
    }
    Ok((deleted, left))
}

/// What [`ensure_owned`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    /// Every object was there and nothing was asked of it.
    Kept,
    /// The phone slot, the inbox and the issue counter were deleted and made again.
    Remade,
}

/// Why [`ensure_owned`] failed: before it changed anything, so a later boot may try again, or
/// while it remade the objects, so the chip may be left with an id a tap could claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsureError {
    Before(ChipError),
    During(ChipError),
}

/// On a chip this badge owns: open the owner session, and if `remake` is set or any of the
/// phone slot, the inbox and the issue counter is missing, delete whatever is at all three ids
/// and make them again, in this one conversation ([`remake_antenna`]).
///
/// The badge never adopts an object at those ids it did not make in the conversation that
/// left them in place: a tap can make one only while the pin rests low, and this delete and
/// make run with the pin high. An id left empty by a conversation that was cut off is flagged
/// by the record ([`REMAKE_GENERATION`]) before the conversation starts, so the next boot
/// remakes it whatever stands there by then.
pub fn ensure_owned(
    chip: &mut dyn ChipOps,
    owner_secret: &[u8; 16],
    remake: bool,
) -> Result<Ensured, EnsureError> {
    chip.open_owner(owner_secret).map_err(EnsureError::Before)?;
    let mut missing = false;
    for id in ANTENNA_IDS {
        missing |= !chip.exists(id).map_err(EnsureError::Before)?;
    }
    if !remake && !missing {
        return Ok(Ensured::Kept);
    }
    remake_antenna(chip).map_err(EnsureError::During)?;
    Ok(Ensured::Remade)
}

/// The highest issue number any grant counter on the chip was based on: a remade issue
/// counter starts there, so no base is given twice.
fn issue_floor(chip: &mut dyn ChipOps) -> Result<u64, ChipError> {
    let mut floor = 0;
    for id in chip.ids()? {
        match KeyId::parse(id.0) {
            Some((KeyKind::Grant, slot)) if slot != crate::grants::ISSUE_SLOT => {
                if let Ok(raw) = chip.counter_get(id) {
                    floor = floor.max(raw >> 32);
                }
            }
            _ => {}
        }
    }
    Ok(floor)
}

/// In the owner session: delete whatever is at the phone slot, inbox and issue counter ids,
/// then make the slot as a placeholder holding a random key nobody keeps, an empty inbox, and
/// the issue counter at [`issue_floor`]. An object there that cannot be deleted is an error.
pub fn remake_antenna(chip: &mut dyn ChipOps) -> Result<(), ChipError> {
    let floor = issue_floor(chip)?;
    for id in ANTENNA_IDS {
        delete_quietly(chip, id)?;
    }
    let random = chip.random(16)?;
    let mut key = [0u8; 16];
    key.copy_from_slice(&random);
    let made = chip.put_aes_auth(KeyId::SLOT, &key, &ObjectPolicy::placeholder_slot());
    zeroize::Zeroize::zeroize(&mut key);
    made?;
    chip.write_binary(KeyId::INBOX, &[0; INBOX_LEN], &ObjectPolicy::inbox())?;
    chip.put_counter(issue_id(), floor, &ObjectPolicy::owner_counter())?;
    Ok(())
}

/// The first boot of a chip under this firmware, in one conversation: the owner object (a user
/// id holding `owner_secret`) unless it is there, the owner session, then the phone slot, the
/// inbox and the issue counter ([`remake_antenna`]), and the new record, with no circle
/// slots, no phone and `badge_nonce`. The caller has emptied the chip but for the owner object
/// first ([`delete_for_upgrade`]), so nothing a tap made is adopted.
///
/// No factory reset undoes this, on the bench chip (SE051, applet 7.2.0, 2026-10-31): it has
/// no factory-reset authentication object (`7FFF0205` does not exist), and `DeleteAll` is
/// refused with 6985 in no session and in the owner session. So a tap cannot wipe a badge's
/// chip, and the owner object, which nobody may delete, is the chip's for good: a badge that
/// lost its owner secret cannot get its chip back.
pub fn first_boot(
    chip: &mut dyn ChipOps,
    owner_secret: &[u8; 16],
    badge_nonce: [u8; 16],
) -> Result<ChipRecord, ChipError> {
    if !chip.exists(KeyId::OWNER)? {
        chip.put_user_id(KeyId::OWNER, owner_secret, &ObjectPolicy::owner_object())?;
    }
    chip.open_owner(owner_secret)?;
    remake_antenna(chip)?;
    Ok(ChipRecord {
        slots: Vec::new(),
        phone: None,
        badge_nonce,
        generation: GENERATION,
        stuck: None,
    })
}
