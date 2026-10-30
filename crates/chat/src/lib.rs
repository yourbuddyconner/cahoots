//! `cahoots-chat`: end-to-end encrypted group chat keyed by the circle root (`docs/chat.md`).
//!
//! An **epoch** is numbered. The key of root epoch `n` is `chat_key(root, circle_id, n)`
//! (`cahoots_core::root`), so starting one spends nothing. Inside an epoch every sender has
//! its own symmetric ratchet, so each message gets a fresh key and old keys are deleted.
//! Messages are ChaCha20-Poly1305 with the header as associated data. Pad epochs, keyed by
//! 32 pad bytes each, are retired; the pad is spent only by pure-pad messages.
//!
//! **Security claim.** Confidentiality and integrity are computational, from the AEAD under
//! keys only a holder of the root can derive. Deleting chain keys gives forward secrecy
//! inside an epoch. A new epoch does not recover from a device compromise that took the
//! root, because every epoch key comes from it. The UI MUST NOT call this mode
//! "unbreakable".
//!
//! **The `sender` field is a chain label, not a person.** Every member can derive every
//! sender's chain, exactly as every member holds every slice. It says which ratchet a
//! message belongs to, and nothing about who pressed send.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(feature = "cli")]
extern crate std;

#[cfg(feature = "cli")]
pub mod cli;
mod primitives;
mod session;

pub use primitives::*;
pub use session::*;
