//! The operations a badge's secure element offers, as a trait, plus a software chip for tests.
//!
//! Keys made in the chip sign but never leave it. `ChipOps` is what the rest of the system
//! calls; the firmware backs it with the real chip and tests back it with [`SoftChip`].
#![no_std]
extern crate alloc;

pub mod ids;
pub mod ops;
pub mod soft;
pub mod testca;

pub use ids::{KeyId, KeyKind};
pub use ops::{Attested, ChipError, ChipOps, Curve, Signature64, Version};
pub use soft::SoftChip;
