//! Host simulator target (M4a). Runs the same `cahoots-hal` device logic as the firmware
//! with desktop stand-ins for the hardware:
//!
//! - [`link::SocketLink`]: the touch link over Unix-domain sockets.
//! - [`store::FilePadStore`]: the pad region as a file with a 4 KiB erase unit.
//! - [`store::FileRecordStore`]: the ledger region as a file.
//! - [`display::PngDisplay`]: a 200x200 1-bit "e-paper" rendered to PNG.
//! - [`device::Device`]: one simulated badge with a state directory.

pub mod device;
pub mod display;
mod font;
pub mod link;
pub mod store;
