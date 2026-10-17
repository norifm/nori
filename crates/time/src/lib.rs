//! The core's clock. Every crate reads time here rather than through `std::time`, whose `Instant::now`
//! and `SystemTime::now` panic in a browser. Natively these are std's own types.

pub use std::time::Duration;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// The wall clock since the Unix epoch; zero for a clock set before it.
pub fn since_epoch() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

/// The wall clock in milliseconds since the Unix epoch.
pub fn unix_ms() -> i64 {
    since_epoch().as_millis() as i64
}
