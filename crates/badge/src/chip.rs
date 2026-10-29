//! The badge's side of the chip: a seat key that signs in the chip, and the chip record.
//!
//! [`ChipSeat`] is a [`SeatSigner`] whose key is a P-256 object in the chip: the badge hashes
//! the message and the chip signs the digest, so the key never leaves the chip.
//!
//! The chip record is the badge's own list of the slots it made objects in: for each slot, the
//! circle id and which of the seat key, the circle root and the epoch counter were made. It is
//! kept apart from the circle record. An object in a slot the record does not know (a wiped
//! record, a reflashed badge, a ceremony cut off before the keep) is deleted at boot and logged
//! by [`ChipRecord::reconcile`], never reused.
//!
//! The record is CBOR: an array of one array of slot entries, each
//! `[slot, circle id (bytes), has seat, has root, has epoch]`, in slot order. The decoder
//! re-encodes and refuses anything that differs.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cahoots_chip::{ChipError, ChipOps, KeyId, KeyKind};
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
        }
    }

    fn clear(&mut self, kind: KeyKind) {
        match kind {
            KeyKind::Seat => self.has_seat = false,
            KeyKind::Root => self.has_root = false,
            KeyKind::Epoch => self.has_epoch = false,
        }
    }
}

/// The chip record: every slot the badge made objects in, in slot order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
pub struct ChipRecord {
    #[n(0)]
    pub slots: Vec<SlotEntry>,
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
    /// each change.
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
            // Every id in the range ends in its slot, whatever its kind.
            let slot = id.0 as u8;
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

/// Delete the seat key, the root and the epoch counter of `slot`. An object that is not there
/// is not an error.
pub fn delete_slot(chip: &mut dyn ChipOps, slot: u8) -> Result<(), ChipError> {
    for kind in SLOT_KINDS {
        match chip.delete(KeyId::new(kind, slot)) {
            Ok(()) | Err(ChipError::NoSuchKey) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Delete every object in the Cahoots range: the wipe at boot.
pub fn delete_all(chip: &mut dyn ChipOps) -> Result<(), ChipError> {
    for id in chip.ids()? {
        match chip.delete(id) {
            Ok(()) | Err(ChipError::NoSuchKey) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
