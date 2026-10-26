//! The assurance lattice (spec §6.1): how sure a verifier can be that the owner meant it. Higher
//! includes lower. The device tier travels separately, in the manifest.

/// Nothing was checked.
pub const CLAIMED: u8 = 0;
/// A signature from a key under a grant.
pub const KEY: u8 = 1;
/// Both buttons pressed on a badge.
pub const PRESENT: u8 = 2;
/// Both buttons pressed on a badge after a PIN.
pub const PRESENT_PIN: u8 = 3;
