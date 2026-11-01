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
        if r.encode() != bytes || r.slots.windows(2).any(|w| w[0].slot >= w[1].slot) {
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

    /// True for a record made by firmware that owns its chip.
    pub fn is_current(&self) -> bool {
        self.generation == GENERATION
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

/// Delete every object of circle slot `slot`: the seat key, the root, the epoch counter, the
/// public object and the hello objects. An object that is not there is not an error.
pub fn delete_slot(chip: &mut dyn ChipOps, slot: u8) -> Result<(), ChipError> {
    for kind in SLOT_KINDS {
        delete_quietly(chip, KeyId::new(kind, slot))?;
    }
    for id in chip.ids()? {
        if circle_slot(id) == Some(slot) {
            delete_quietly(chip, id)?;
        }
    }
    Ok(())
}

/// Delete every object in the Cahoots range but the grant issue counter and the badge-wide
/// objects: the wipe at boot. The issue counter stays so a grant base is never given twice on
/// one chip.
pub fn delete_all(chip: &mut dyn ChipOps) -> Result<(), ChipError> {
    let issue = KeyId::new(KeyKind::Grant, crate::grants::ISSUE_SLOT);
    for id in chip.ids()? {
        if id != issue && !id.is_badge_wide() {
            delete_quietly(chip, id)?;
        }
    }
    Ok(())
}

/// The upgrade wipe: delete every object in the Cahoots range but the badge-wide ones,
/// the issue counter too, since every object made before the badge owned its chip is
/// reachable by any session. Every object is tried. Returns how many went, and the ones that
/// did not with why.
pub fn delete_for_upgrade(chip: &mut dyn ChipOps) -> Result<(usize, Vec<(KeyId, ChipError)>), ChipError> {
    let mut deleted = 0;
    let mut failed = Vec::new();
    for id in chip.ids()? {
        if id.is_badge_wide() {
            continue;
        }
        match delete_quietly(chip, id) {
            Ok(()) => deleted += 1,
            Err(e) => failed.push((id, e)),
        }
    }
    Ok((deleted, failed))
}

/// Make the objects the badge owns its chip by, where they are missing, and open the owner
/// session: the owner object (a user id holding `owner_secret`), the phone slot as a
/// placeholder holding a random key nobody keeps, and the inbox. Run in one conversation, so
/// no id a policy names is left empty at its end.
pub fn ensure_owned(chip: &mut dyn ChipOps, owner_secret: &[u8; 16]) -> Result<(), ChipError> {
    if !chip.exists(KeyId::OWNER)? {
        chip.put_user_id(KeyId::OWNER, owner_secret, &ObjectPolicy::owner_object())?;
    }
    chip.open_owner(owner_secret)?;
    if !chip.exists(KeyId::SLOT)? {
        let random = chip.random(16)?;
        let mut key = [0u8; 16];
        key.copy_from_slice(&random);
        let made = chip.put_aes_auth(KeyId::SLOT, &key, &ObjectPolicy::placeholder_slot());
        zeroize::Zeroize::zeroize(&mut key);
        made?;
    }
    if !chip.exists(KeyId::INBOX)? {
        chip.write_binary(KeyId::INBOX, &[0; INBOX_LEN], &ObjectPolicy::inbox())?;
    }
    Ok(())
}

/// The first boot of a chip under this firmware: [`ensure_owned`], and the new record, with no
/// circle slots, no phone and `badge_nonce`.
pub fn first_boot(
    chip: &mut dyn ChipOps,
    owner_secret: &[u8; 16],
    badge_nonce: [u8; 16],
) -> Result<ChipRecord, ChipError> {
    ensure_owned(chip, owner_secret)?;
    Ok(ChipRecord {
        slots: Vec::new(),
        phone: None,
        badge_nonce,
        generation: GENERATION,
    })
}
