//! Host simulator target (M4a). Runs the same `cahoots-hal` device logic as the firmware
//! with desktop stand-ins for the hardware:
//!
//! - [`link::SocketLink`]: the touch link over Unix-domain sockets.
//! - [`store::FilePadStore`]: the pad region as a file with a 4 KiB erase unit.
//! - [`store::FileRecordStore`]: the ledger region as a file.
//! - [`display::PngDisplay`]: a 200x200 1-bit "e-paper" rendered to PNG.
//! - [`device::Device`]: one simulated badge with a state directory.
//! - [`board::HostBoard`]: the same stand-ins as a `cahoots_badge::Board`, so the badge crate
//!   runs here as it does on a board.
//! - [`badge_client::BadgeClient`]: a laptop's requests to a badge, real or simulated.
//! - [`agent`], [`gitsign`], [`roster`], [`attest`]: SSH logins and git signatures with a
//!   badge's seat key, the circle as `authorized_keys`, and the chip's attestation.
//! - [`card`]: contact cards and grants the badge signs.
//! - [`agentkey`], [`grants`]: an agent's own key, and the grants that let it sign git commits
//!   without a press on the badge.

pub mod agent;
pub mod agentkey;
pub mod attest;
pub mod badge_client;
pub mod board;
pub mod card;
pub mod device;
pub mod display;
mod font;
pub mod gitsign;
pub mod grants;
pub mod link;
pub mod pile;
pub mod roster;
pub mod sign;
pub mod store;
