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

pub mod board;
pub mod device;
pub mod display;
mod font;
pub mod link;
pub mod pile;
pub mod store;
