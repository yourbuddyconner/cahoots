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
//! `[slot, circle id (bytes), has seat, has root, has epoch, chat generation]` in slot order,
//! the chat generation (bytes 32) left out while it is generation 0, the phone `null`
//! or `[name, public key (33 bytes)]`. The decoder re-encodes and refuses anything that
//! differs. A board stores it as `CAHC2` and the CBOR ([`ChipRecord::to_stored`]); the record
//! of earlier firmware, `CAHC`, reads as no record ([`ChipRecord::from_stored`]), and a badge
//! with no record of [`GENERATION`] wipes the chip and its circles at boot ([`first_boot`]).
//!
//! The badge owns its chip: [`first_boot`] makes the owner object, a user id only the badge
//! knows, then the phone slot as a placeholder whose key nobody holds, the inbox, the grant
//! issue counter and the pairing object. Every policy names identities by those ids, and none
//! of them is ever left empty for a tap to claim.
//!
//! A tap may make a binary object at any id that holds nothing, under the applet's default
//! policy (every right for every session), or under a policy that lets nobody delete it (bench,
//! SE051 applet 7.2.0). The badge never adopts one: a keep deletes whatever stands at the new
//! circle's public id before it writes ([`remake_public`]), and the badge reads back only a
//! public object the record says it wrote ([`SlotEntry::public_written`]). Such an object
//! would burn its circle slot: one whose ids hold an object nobody may delete is passed over
//! when the badge picks a slot for a new circle, and a public id it cannot clear marks the
//! slot stuck.
//!
//! So a tap finds no empty public id: [`first_boot`] makes a placeholder like the phone slot's
//! at the public id of every circle slot, and the badge keeps one at each public id no circle
//! uses ([`fill_slots`]). A keep deletes it just before it writes the circle's public object
//! there ([`remake_public`]), and a slot emptied by a forget or a discarded pass gets its
//! placeholder again in the same conversation. Before any of those empties a public id the
//! record says the placeholders must be made again ([`ChipRecord::placeholders`] `None`), and
//! after the placeholder is back it says what it said before. So a conversation cut off in
//! between leaves the next boot to delete whatever stands at those ids by then and make the
//! placeholders again: nothing a tap put there is kept. A public id the chip has no room for
//! stays empty until a boot finds room, and what stands there by then is kept as it is,
//! never read.
//!
//! The known limit: the hello ids, and the seat key, root and epoch ids, of a slot no circle
//! uses stay empty, and a tap that makes an object nobody may delete at one of them still
//! burns that slot. What the bench chip's free memory count showed (SE051, applet 7.2.0,
//! 2026-11-01): a placeholder took 160 bytes, and 50 deletes and makes of one left the count
//! as it was. A circle of three seats took 2948 bytes beside the placeholder its public object
//! replaced. Holding two such circles, with 14956 bytes free, the chip made 91 of the 138
//! placeholders the public and hello ids of sixteen slots needed, then refused one for want of
//! memory, and refused a third circle. With the public and hello ids of eight slots it held
//! three circles. With the public ids alone it held five and refused a sixth with 3232 bytes
//! free.

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
    /// The circle's current chat generation id, set when the badge unpaired a phone and
    /// signed a bump to a new one. `None` means generation 0, `cahoots_core::root::gen0_id`.
    #[cbor(n(5), with = "minicbor::bytes")]
    pub chat_generation: Option<[u8; 32]>,
    /// `Some(true)` once the badge wrote this circle's public object itself, `None` until then
    /// and after it is gone. The badge reads back only a public object it wrote: the applet
    /// lets any session make an object at an id that holds nothing, so one the badge did not
    /// write may be a tap's. A record of earlier firmware has no such field and reads as not
    /// written.
    #[n(6)]
    pub public_written: Option<bool>,
}

impl SlotEntry {
    /// True once the badge wrote this circle's public object.
    pub fn public_written(&self) -> bool {
        self.public_written == Some(true)
    }

    /// Note whether the badge's own public object is at the circle's public id.
    pub fn set_public_written(&mut self, written: bool) {
        self.public_written = written.then_some(true);
    }

    /// The circle's current chat generation id: the one the last bump this badge signed
    /// moved to, or generation 0.
    pub fn current_generation(&self) -> [u8; 32] {
        self.chat_generation
            .unwrap_or_else(|| cahoots_core::root::gen0_id(&self.circle_id))
    }

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

/// The generation of a record of a chip this badge owns whose antenna objects ([`ANTENNA_IDS`])
/// must be made again at the next boot: written before a conversation that leaves one
/// of those ids empty for a moment, so a cut-off conversation never leaves one for a tap to
/// fill and the badge to adopt.
pub const REMAKE_GENERATION: u8 = 3;

/// The generation of the record a badge stores when it refuses its chip for good: an object a
/// tap made stays where the badge's own must be, or the setup failed after the owner object,
/// which nobody may delete, was made. Later boots refuse at once and touch nothing.
pub const REFUSED_GENERATION: u8 = 0xFF;

/// The bytes a stored chip record starts with.
pub const RECORD_MAGIC: &[u8; 5] = b"CAHC2";

/// The inbox's length: room for a phone's key, its name, the slot key and its signature, as
/// CBOR ([`crate::pairing::Inbox`]).
pub const INBOX_LEN: usize = 256;

/// One circle's bumps after an unpair, kept in the chip record until its public object carries
/// them: the circle slot, the generation the last moves to, and the whole list for field 6,
/// oldest first.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct PendingBump {
    #[n(0)]
    pub slot: u8,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub to: [u8; 32],
    #[n(2)]
    pub bumps: Vec<minicbor::bytes::ByteVec>,
}

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
    /// An unpair under way: the bumps it signed, stored before it touches the phone slot or a
    /// public object. Boot finishes it ([`crate::Badge::boot`]): each bump goes into its
    /// circle's public object, and the entry goes once every one is written.
    #[n(5)]
    pub pending_unpair: Option<Vec<PendingBump>>,
    /// The circle slots where a phone was paired since the circle's last bump was made: that
    /// phone may have posted it, so the next unpair appends rather than replaces. In order.
    #[n(6)]
    pub paired_since_bump: Option<Vec<u8>>,
    /// The policy the badge's public objects were made under: `None` for the first one, which
    /// any session could read, or [`PUBLIC_POLICY`]. Boot makes every public object again
    /// under the current policy once, and then stores [`PUBLIC_POLICY`]
    /// ([`crate::Badge::boot`]).
    #[n(7)]
    pub public_policy: Option<u8>,
    /// [`PLACEHOLDERS`] while nothing at a circle slot's public id that the record does not
    /// vouch for is anyone's but the badge's: its placeholders ([`fill_slots`]), or nothing.
    /// `None` in a record of earlier firmware, which made none, and from before a conversation
    /// empties a public id until its placeholder is back. With `None` the next boot deletes
    /// whatever stands at those ids, but where a circle is held, and makes the placeholders
    /// ([`crate::Badge::boot`]).
    #[n(8)]
    pub placeholders: Option<u8>,
}

/// The policy the public objects are made under now ([`cahoots_chip::ObjectPolicy::public_object`]):
/// read by the owner and the phone slot only. A record of earlier firmware has no such field,
/// which means policy 1, readable by any session.
pub const PUBLIC_POLICY: u8 = 2;

/// The value of [`ChipRecord::placeholders`] once the badge made its placeholders.
pub const PLACEHOLDERS: u8 = 1;

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
        let unpair_ok = r.pending_unpair.as_ref().is_none_or(|v| !v.is_empty())
            && r
                .paired_since_bump
                .as_ref()
                .is_none_or(|v| !v.is_empty() && v.windows(2).all(|w| w[0] < w[1]));
        if r.encode() != bytes
            || r.slots.iter().any(|e| e.public_written == Some(false))
            || r.slots.windows(2).any(|w| w[0].slot >= w[1].slot)
            || !stuck_ok
            || !unpair_ok
            || r.public_policy.is_some_and(|p| p != PUBLIC_POLICY)
            || r.placeholders.is_some_and(|p| p != PLACEHOLDERS)
        {
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

    /// The policy the public objects in the chip were made under: 1 for the first, which any
    /// session could read, or [`PUBLIC_POLICY`].
    pub fn public_policy(&self) -> u8 {
        self.public_policy.unwrap_or(1)
    }

    /// The circle slots whose public objects the badge wrote and vouches for, as the pairing
    /// object carries them: bit `s` for slot `s`. A stuck slot is not among them.
    pub fn circle_mask(&self) -> u16 {
        self.slots
            .iter()
            .filter(|e| e.slot < 16 && e.public_written() && !self.is_stuck(e.slot))
            .fold(0, |m, e| m | 1 << e.slot)
    }

    /// True if a phone was paired since circle slot `slot`'s last bump was made.
    pub fn paired_since_bump(&self, slot: u8) -> bool {
        self.paired_since_bump.as_ref().is_some_and(|v| v.contains(&slot))
    }

    /// Mark or unmark circle slot `slot` as having had a phone paired since its last bump.
    pub fn set_paired_since_bump(&mut self, slot: u8, paired: bool) {
        let mut v = self.paired_since_bump.take().unwrap_or_default();
        v.retain(|&s| s != slot);
        if paired {
            v.push(slot);
            v.sort_unstable();
        }
        self.paired_since_bump = (!v.is_empty()).then_some(v);
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

    /// True if the antenna objects ([`ANTENNA_IDS`]) must be made again at the next boot,
    /// whatever stands at their ids.
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

    pub fn entry_mut(&mut self, slot: u8) -> Option<&mut SlotEntry> {
        self.slots.iter_mut().find(|e| e.slot == slot)
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
    /// each change. Grant counters are left alone: [`crate::grants::sweep`] owns them. So are
    /// the public ids of the circle slots, where the badge keeps its placeholders
    /// ([`fill_slots`]).
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
            // The public id of a slot no circle uses holds the badge's placeholder, which stays
            // ([`fill_slots`]).
            if is_placeholder_id(*id) {
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
/// above 31 would not fit it. Each slot's public id holds a placeholder while no circle uses
/// it ([`fill_slots`]).
pub const CIRCLE_SLOTS: u8 = 16;
const _: () = assert!((CIRCLE_SLOTS as usize) * 8 <= 256);

/// True for an id the badge keeps a placeholder at while no circle uses it: the public id of a
/// circle slot below [`CIRCLE_SLOTS`]. The pairing object's id, public id 255, is not one.
pub fn is_placeholder_id(id: KeyId) -> bool {
    matches!(KeyId::parse(id.0), Some((KeyKind::Public, slot)) if slot < CIRCLE_SLOTS)
}

fn delete_into(chip: &mut dyn ChipOps, id: KeyId, left: &mut Left) {
    if let Err(e) = delete_quietly(chip, id) {
        if !left.iter().any(|(x, _)| *x == id) {
            left.push((id, e));
        }
    }
}

/// Delete every object of circle slot `slot`: the seat key, the root, the epoch counter, the
/// public object or the placeholder at its id, and the hello objects. An object that is not
/// there is not an error. Every object is tried, and the ones that stay are returned. The
/// caller makes the slot's placeholder again ([`fill_slots`]) before the conversation ends.
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

/// Delete every circle object and grant counter in the Cahoots range, and the placeholders at
/// the circle slots' public ids: the wipe at boot, after which the caller makes the
/// placeholders again ([`fill_slots`]). The grant issue counter stays, so a grant base is
/// never given twice on one chip, and so do the badge-wide objects and objects of kinds this
/// firmware does not know. Every object is tried, and the ones that stay are returned.
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
/// inbox, the grant issue counter and the pairing object.
pub const ANTENNA_IDS: [KeyId; 4] = [
    KeyId::SLOT,
    KeyId::INBOX,
    KeyId::new(KeyKind::Grant, crate::grants::ISSUE_SLOT),
    KeyId::PAIRING,
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

/// What [`ensure_owned`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    /// Every object was there and nothing was asked of it.
    Kept,
    /// The antenna objects must be deleted and made again ([`remake_antenna`]): the record
    /// asked for it, or one of them is missing. Nothing was changed yet.
    Remake,
}

/// Why [`ensure_owned`] failed. Nothing was changed, so a later boot may try again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnsureError(pub ChipError);

/// On a chip this badge owns: open the owner session, and say whether the antenna objects
/// ([`ANTENNA_IDS`]) must be made again: when `remake` is set, or when any of them is
/// missing. It deletes and makes nothing itself.
///
/// The badge never adopts an object at those ids it did not make in the conversation that
/// left them in place: a tap can make one only while the pin rests low. So the caller stores
/// the record flagged ([`REMAKE_GENERATION`]) before it deletes anything, then runs
/// [`remake_antenna`] with the pin high, then stores the record back at [`GENERATION`]. A
/// conversation cut off between the delete and the make leaves the flag, and the next boot
/// remakes the ids whatever stands there by then.
pub fn ensure_owned(
    chip: &mut dyn ChipOps,
    owner_secret: &[u8; 16],
    remake: bool,
) -> Result<Ensured, EnsureError> {
    chip.open_owner(owner_secret).map_err(EnsureError)?;
    let mut missing = false;
    for id in ANTENNA_IDS {
        missing |= !chip.exists(id).map_err(EnsureError)?;
    }
    Ok(if remake || missing { Ensured::Remake } else { Ensured::Kept })
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

/// In the owner session: delete whatever is at the antenna ids ([`ANTENNA_IDS`]), then make
/// the slot as a placeholder holding a random key nobody keeps, an empty inbox, the issue
/// counter at [`issue_floor`], and the pairing object with no pairing open and `circles`. An
/// object there that cannot be deleted is an error.
pub fn remake_antenna(chip: &mut dyn ChipOps, circles: u16) -> Result<(), ChipError> {
    let floor = issue_floor(chip)?;
    for id in ANTENNA_IDS {
        delete_quietly(chip, id)?;
    }
    make_placeholder(chip)?;
    chip.write_binary(KeyId::INBOX, &[0; INBOX_LEN], &ObjectPolicy::inbox())?;
    chip.put_counter(issue_id(), floor, &ObjectPolicy::owner_counter())?;
    let pairing = crate::pairing::PairingObject::closed(circles)
        .padded()
        .expect("an empty name fits");
    chip.write_binary(KeyId::PAIRING, &pairing, &ObjectPolicy::pairing_object())?;
    Ok(())
}

/// Make the phone slot as a placeholder ([`make_placeholder_at`]). The slot's id must be free.
pub fn make_placeholder(chip: &mut dyn ChipOps) -> Result<(), ChipError> {
    make_placeholder_at(chip, KeyId::SLOT)
}

/// Make a placeholder at `id`: an AES authentication object holding a random key nobody
/// keeps, under [`ObjectPolicy::placeholder_slot`], so only the owner may delete it and no
/// session may do anything else with it. No policy names it, so a session opened against it,
/// were its key known, would get no more than no session does. The id must be free.
pub fn make_placeholder_at(chip: &mut dyn ChipOps, id: KeyId) -> Result<(), ChipError> {
    let random = chip.random(16)?;
    let mut key = [0u8; 16];
    key.copy_from_slice(&random);
    let made = chip.put_aes_auth(id, &key, &ObjectPolicy::placeholder_slot());
    zeroize::Zeroize::zeroize(&mut key);
    made
}

/// What [`fill_slots`] did: how many placeholders it made, and the chip error that stopped it,
/// if one did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filled {
    pub made: usize,
    pub error: Option<ChipError>,
}

impl Filled {
    /// True when the chip refused a placeholder for want of memory (`6A84`).
    pub fn out_of_memory(&self) -> bool {
        self.error == Some(ChipError::Status(0x6A84))
    }
}

/// In the owner session: a placeholder ([`make_placeholder_at`]) at the public id of each
/// circle slot in `slots` that holds nothing. A public object the record vouches for
/// ([`ChipRecord::circle_mask`]) is the circle's, and its id is left: if the object is gone,
/// the badge writes it again there. Nothing is deleted, so what stands at an id stays. Stops
/// at the first error, `6A84` among them when the chip is short of memory.
pub fn fill_slots(chip: &mut dyn ChipOps, record: &ChipRecord, slots: &[u8]) -> Filled {
    let mut filled = Filled::default();
    let present = match chip.ids() {
        Ok(ids) => ids,
        Err(e) => {
            filled.error = Some(e);
            return filled;
        }
    };
    let vouched = record.circle_mask();
    for &slot in slots {
        let id = public_id(slot);
        if slot >= CIRCLE_SLOTS || vouched & 1 << slot != 0 || present.contains(&id) {
            continue;
        }
        if let Err(e) = make_placeholder_at(chip, id) {
            filled.error = Some(e);
            return filled;
        }
        filled.made += 1;
    }
    filled
}

/// In the owner session: delete whatever stands at the public id of each circle slot in
/// `slots`, each one tried, and return what stays. For a chip whose record does not know of
/// placeholders: an object there is not one the badge made as a placeholder, so it is not
/// kept as one.
pub fn clear_placeholder_ids(chip: &mut dyn ChipOps, slots: &[u8]) -> Result<Left, ChipError> {
    let present = chip.ids()?;
    let mut left = Left::new();
    for id in slots.iter().map(|&s| public_id(s)) {
        if present.contains(&id) {
            delete_into(chip, id, &mut left);
        }
    }
    Ok(left)
}

/// Why [`replace_slot`] did not put what it was asked to in the phone slot, and what the slot
/// holds now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotFailure {
    /// The old slot could not be deleted: it is as it was.
    Unchanged(ChipError),
    /// The phone's key was not made; a placeholder was, in its place.
    Placeholder(ChipError),
    /// Nothing could be made: the slot's id is empty. The conversation must not end, or a
    /// tap could fill it: the caller holds the chip until the next boot
    /// ([`crate::Board::chip_hold`]), which makes the slot again.
    Empty(ChipError),
}

/// Replace the phone slot with one holding `key` (a paired phone's) or, with `None`, with a
/// placeholder ([`make_placeholder`]). Either way the slot keeps the placeholder's policy:
/// delete for the owner and nothing else, so no session but the owner's can remove it. In the
/// owner session. The caller has flagged the record with [`REMAKE_GENERATION`] first: between
/// the delete and the make the slot's id is empty. A make that fails is followed by a
/// placeholder, tried twice, so the id is left empty only if the chip makes nothing.
pub fn replace_slot(chip: &mut dyn ChipOps, key: Option<&[u8; 16]>) -> Result<(), SlotFailure> {
    delete_quietly(chip, KeyId::SLOT).map_err(SlotFailure::Unchanged)?;
    let first = match key {
        Some(k) => match chip.put_aes_auth(KeyId::SLOT, k, &ObjectPolicy::placeholder_slot()) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        },
        None => match make_placeholder(chip) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        },
    };
    let tries = if key.is_some() { 2 } else { 1 };
    for _ in 0..tries {
        if make_placeholder(chip).is_ok() {
            return match key {
                Some(_) => Err(SlotFailure::Placeholder(first)),
                None => Ok(()),
            };
        }
    }
    Err(SlotFailure::Empty(first))
}

/// True if the inbox in the chip is shorter than [`INBOX_LEN`]: one made by earlier firmware,
/// too short for a signed request. [`remake_inbox`] makes it again.
pub fn inbox_short(chip: &mut dyn ChipOps) -> Result<bool, ChipError> {
    Ok(chip.read_binary(KeyId::INBOX)?.len() < INBOX_LEN)
}

/// Delete the inbox and make it again, empty, at [`INBOX_LEN`], in the owner session. The
/// caller has flagged the record with [`REMAKE_GENERATION`] first, as for the slot, and holds
/// the chip if this fails: an inbox a tap made could be readable by any session.
pub fn remake_inbox(chip: &mut dyn ChipOps) -> Result<(), ChipError> {
    delete_quietly(chip, KeyId::INBOX)?;
    chip.write_binary(KeyId::INBOX, &[0; INBOX_LEN], &ObjectPolicy::inbox())
}

/// Write `object` over the pairing object ([`KeyId::PAIRING`]), in the owner session. The
/// badge made the object itself ([`remake_antenna`]) and keeps it made, so it is written in
/// place.
pub fn write_pairing(chip: &mut dyn ChipOps, object: &crate::pairing::PairingObject) -> Result<(), ChipError> {
    let bytes = object.padded().ok_or(ChipError::Status(0x6A80))?;
    chip.rewrite_binary(KeyId::PAIRING, &bytes)
}

/// Empty the inbox: zeros over the whole object. In the owner session.
pub fn clear_inbox(chip: &mut dyn ChipOps) -> Result<(), ChipError> {
    chip.rewrite_binary(KeyId::INBOX, &[0; INBOX_LEN])
}

/// The id of the hello object circle slot `slot` keeps for seat `other`.
pub fn hello_id(slot: u8, other: u8) -> KeyId {
    KeyId::new(KeyKind::Hello, slot * 8 + other)
}

/// The id of circle slot `slot`'s public object.
pub fn public_id(slot: u8) -> KeyId {
    KeyId::new(KeyKind::Public, slot)
}

/// A hello object's value for a pair key: `HMAC-SHA256(32 zero bytes, pair_key)`, the extract
/// step of `cahoots_core::pairwise::words`. A slot session that MACs `"cahoots/words/v1" ||
/// speaker || counter_le || 0x01` with it gets the expand step, whose first four bytes read
/// little-endian give the two words.
pub fn hello_value(pair_key: &[u8; 32]) -> zeroize::Zeroizing<[u8; 32]> {
    use hmac::{Hmac, Mac};
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&[0u8; 32])
        .expect("HMAC takes a key of any length");
    m.update(pair_key);
    zeroize::Zeroizing::new(m.finalize().into_bytes().into())
}

/// Why a public object was not written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicError {
    Chip(ChipError),
    /// An object the badge did not write stands at the public id and cannot be deleted.
    Undeletable(ChipError),
    /// The contents do not fit the object, or [`crate::public::MAX_PUBLIC`] for a new one.
    TooLarge,
}

impl From<ChipError> for PublicError {
    fn from(e: ChipError) -> Self {
        PublicError::Chip(e)
    }
}

/// Write circle slot `slot`'s public object, in the owner session: over the object that is
/// there, zero-filled to its length, or as a new object with room to grow
/// ([`crate::public::PublicObject::new_len`]) under [`ObjectPolicy::public_object`]. A card
/// that does not fit is left out rather than the whole object.
pub fn write_public(
    chip: &mut dyn ChipOps,
    slot: u8,
    object: &crate::public::PublicObject,
) -> Result<(), PublicError> {
    let id = public_id(slot);
    let without_card = || crate::public::PublicObject {
        card: None,
        ..object.clone()
    };
    match chip.read_binary(id) {
        Ok(old) => {
            let bytes = object
                .padded(old.len())
                .or_else(|| without_card().padded(old.len()))
                .ok_or(PublicError::TooLarge)?;
            chip.rewrite_binary(id, &bytes)?;
        }
        Err(ChipError::NoSuchKey) => {
            let o = match object.new_len() {
                Some(_) => object.clone(),
                None => without_card(),
            };
            let len = o.new_len().ok_or(PublicError::TooLarge)?;
            let bytes = o.padded(len).ok_or(PublicError::TooLarge)?;
            chip.write_binary(id, &bytes, &ObjectPolicy::public_object())?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Delete whatever is at circle slot `slot`'s public object id and write `object` there as a
/// new object under [`ObjectPolicy::public_object`], in the owner session. For a public id the
/// badge has not written (see [`SlotEntry::public_written`]): the applet lets any session
/// make an object at an id that holds nothing, so what is there may be a tap's, and is never
/// read or written over. `Undeletable` if it stays.
pub fn remake_public(
    chip: &mut dyn ChipOps,
    slot: u8,
    object: &crate::public::PublicObject,
) -> Result<(), PublicError> {
    match delete_quietly(chip, public_id(slot)) {
        Ok(()) => {}
        Err(e @ ChipError::NotAllowed) => return Err(PublicError::Undeletable(e)),
        Err(e) => return Err(PublicError::Chip(e)),
    }
    write_public(chip, slot, object)
}

/// Circle slot `slot`'s public object as stored, or why not: a chip error, or `None` for
/// bytes that are not a public object.
pub fn read_public(
    chip: &mut dyn ChipOps,
    slot: u8,
) -> Result<Option<crate::public::PublicObject>, ChipError> {
    let bytes = chip.read_binary(public_id(slot))?;
    Ok(crate::public::PublicObject::decode(&bytes).ok())
}

/// The first boot of a chip under this firmware, in one conversation: the owner object (a user
/// id holding `owner_secret`) unless it is there, the owner session, then the phone slot, the
/// inbox, the issue counter and the pairing object ([`remake_antenna`]), a placeholder at
/// every circle slot's public id ([`fill_slots`]), and the new record, with no circle slots, no
/// phone and `badge_nonce`. The caller has emptied the chip but for the owner object first
/// ([`delete_for_upgrade`]), so nothing a tap made is adopted. A placeholder the chip will
/// not make leaves the record without [`PLACEHOLDERS`], and the next boot tries again: the
/// chip is owned either way.
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
    remake_antenna(chip, 0)?;
    let mut record = ChipRecord {
        slots: Vec::new(),
        phone: None,
        badge_nonce,
        generation: GENERATION,
        stuck: None,
        pending_unpair: None,
        paired_since_bump: None,
        public_policy: Some(PUBLIC_POLICY),
        placeholders: None,
    };
    let all: Vec<u8> = (0..CIRCLE_SLOTS).collect();
    let filled = fill_slots(chip, &record, &all);
    if filled.error.is_none() || filled.out_of_memory() {
        record.placeholders = Some(PLACEHOLDERS);
    }
    Ok(record)
}
