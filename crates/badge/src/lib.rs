//! `cahoots-badge`: the badge's behaviour, written once against the [`Board`] trait, so a
//! board crate is drivers plus a pin table. Every board runs this code.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod badge;
pub mod board;

pub use badge::{Badge, Step};
pub use board::{word_indices, Board, BoardError, Circle};
