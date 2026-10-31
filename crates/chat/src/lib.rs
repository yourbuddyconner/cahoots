//! `cahoots-chat`: end-to-end encrypted group chat keyed by the circle root (`docs/chat.md`).
//!
//! Keys come in **generations**. A generation has an id and a key,
//! `chat_gen_key(root, circle_id, id)` (`cahoots_core::root`). Generation 0's id is derived
//! from the circle id, and later ones are random, named by a seat-signed bump ([`bump`]). An
//! **epoch** is numbered inside a generation, and its key comes from the generation key, so
//! starting one spends nothing and a device that holds one generation key and not the root can
//! chat until the next bump. Inside an epoch every sender has its own symmetric ratchet, so
//! each message gets a fresh key and old keys are deleted. Messages are ChaCha20-Poly1305 with
//! the header as associated data. Root epochs, keyed by the root directly, are the old form:
//! they still open and are never sent. Pad epochs are retired.
//!
//! **Security claim.** Confidentiality and integrity are computational, from the AEAD under
//! keys derived from the root. Chat is as secret as the root. Every member's device holds
//! it, so compromising one member's device reveals every message the relay still holds.
//! Deleting chain keys bounds what one message key reveals and nothing more. The UI MUST
//! NOT call this mode "unbreakable".
//!
//! **Sending position.** Keys depend only on the generation, epoch, sender and counter. Write the
//! exported state before posting, never roll it back, and after a restart feed the whole
//! relay to `receive`, own messages included, then call `mark_synced`. `send` refuses with
//! `NotSynced` until then.
//!
//! **The `sender` field is a chain label, not a person.** Every member can derive every
//! sender's chain, exactly as every member holds every slice. It says which ratchet a
//! message belongs to, and nothing about who pressed send.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(feature = "cli")]
extern crate std;

pub mod bump;
#[cfg(feature = "cli")]
pub mod cli;
mod primitives;
mod session;

pub use bump::{sign_bump, verify_bump, BUMP_DOMAIN};
pub use primitives::*;
pub use session::*;
