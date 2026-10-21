//! `cahoots-chat`: pad-seeded end-to-end encrypted chat (`docs/chat.md`).
//!
//! One 32-byte pad spend keys an **epoch**. Inside an epoch every sender has its own
//! symmetric ratchet, so each message gets a fresh key and old keys are deleted.
//! Messages are ChaCha20-Poly1305 with the header as associated data.
//!
//! **Security claim.** Confidentiality and integrity are computational, from the AEAD,
//! hedged by the pad: there is no public-key step an attacker could break instead.
//! Deleting chain keys gives forward secrecy inside an epoch. The next epoch spend gives
//! post-compromise security, because the attacker never had the pad. The UI MUST label
//! this mode "end-to-end encrypted, pad-hedged", never "unbreakable".
//!
//! **The `sender` field is a chain label, not a person.** Every member can derive every
//! sender's chain, exactly as every member holds every slice. It says which ratchet a
//! message belongs to, and nothing about who pressed send.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod primitives;
mod session;

pub use primitives::*;
pub use session::*;
