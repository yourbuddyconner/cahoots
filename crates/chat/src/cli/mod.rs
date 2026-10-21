//! The demo client: a local development ceremony, file-backed circles in the
//! `cahoots-provider` directory format, a relay client, and the send/receive logic
//! factored so tests can drive it without threads or stdin.

pub mod ceremony;
pub mod client;
pub mod relay;
pub mod store;
