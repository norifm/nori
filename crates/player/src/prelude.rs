//! What the modules that build without `std` take from `alloc` (and, then, from libm), so their code
//! reads the same either way.

#[allow(unused_imports)]
pub use alloc::{borrow::ToOwned, boxed::Box, format, string::{String, ToString}, vec, vec::Vec};

#[cfg(not(feature = "std"))]
pub use crate::math::Float;
