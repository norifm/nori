//! Shared types: library records (model.rs), derived display lines (lines.rs), numbers as every client
//! writes them (numbers.rs), what plugins decide in nori's place (deciders.rs), `CoreError`, the log (alog.rs)
//! and the heap counter (heap.rs). Without `std`, the records and their JSON only.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
pub mod alog;
#[cfg(feature = "std")]
pub mod deciders;
#[cfg(feature = "std")]
mod error;
#[cfg(feature = "std")]
pub mod heap;
pub mod lines;
pub mod model;
#[cfg(feature = "std")]
pub mod numbers;

pub use model::*;
#[cfg(feature = "std")]
pub use error::*;
