//! `cahoots-provider`: the pad provider API (handoff section 10.3).
//!
//! The provider is the only code that touches the spent-range ledger. Adapters never read
//! the pad directly. Every `psk` or `spend` call marks the range spent (durably), erases
//! it, and only then returns the bytes.
//!
//! Backends: [`MemProvider`] (tests, WASM), [`FileProvider`] (development, encrypted at
//! rest). Device-over-USB and [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) backends are later milestones.

mod circle;
mod file;
mod mem;
mod session;

pub use cahoots_core::CircleId;
pub use cahoots_custodian::{
    CircleInfo, Custodian as PadProvider, PadError, PskHandle, LOW_PAD_PERCENT, PSK_LEN,
};
pub use file::{FileLedger, FileProvider, FileStorage};
pub use mem::MemProvider;
pub use session::{derive_session_key, PSK_SALT_DOMAIN, SESSION_KEY_INFO};
