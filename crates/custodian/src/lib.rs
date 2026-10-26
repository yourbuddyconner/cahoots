//! `cahoots-custodian`: the contract between code that needs pad bytes and the thing that
//! keeps the pad.
//!
//! A custodian holds one or more circles' pads and hands out keys from them. The invariant
//! every implementation must keep: a range is recorded spent and erased before its bytes
//! are returned, and no range is ever returned twice.
//!
//! Each custodian keeps its own on-flash or on-disk layout. Only this contract and the
//! ledger semantics are shared.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod contract;
pub mod wire;

pub use cahoots_core::CircleId;
pub use contract::*;
