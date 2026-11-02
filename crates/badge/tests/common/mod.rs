#![allow(dead_code, unused_imports)]

pub use cahoots_badge::test_support::*;

use cahoots_chip::{ChipOps, KeyId, KeyKind, ObjectPolicy, SoftChip};

/// The public id of every circle slot, where the badge keeps a placeholder while no circle
/// uses it.
pub fn every_public_id() -> Vec<KeyId> {
    (0..cahoots_badge::chip::CIRCLE_SLOTS).map(|slot| KeyId::new(KeyKind::Public, slot)).collect()
}

/// True when `id` is a circle slot's public id and holds the badge's placeholder: an object
/// only the owner may delete, and that nobody may use.
pub fn is_placeholder(c: &SoftChip, id: KeyId) -> bool {
    every_public_id().contains(&id) && c.policy_for_test(id) == Some(ObjectPolicy::placeholder_slot())
}

/// The circle slots' public ids that hold nothing, for a tap to fill: none, on a badge that
/// keeps its placeholders.
pub fn empty_public_ids(c: &mut SoftChip) -> Vec<KeyId> {
    let present = c.ids().unwrap();
    every_public_id().into_iter().filter(|id| !present.contains(id)).collect()
}
