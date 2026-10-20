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

/// Monotonic time since the process first asked.
pub fn monotonic() -> Duration {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// The wall clock in milliseconds since the Unix epoch.
pub fn unix_ms() -> i64 {
    since_epoch().as_millis() as i64
}

/// This device's clock for remote timing, µs: monotonic, and running on while the device sleeps
/// (CLOCK_BOOTTIME on Android and Linux, which Android's `SystemClock.elapsedRealtimeNanos` reads).
/// On Apple's systems CLOCK_MONOTONIC_RAW, `mach_continuous_time`: their CLOCK_MONOTONIC is the wall
/// clock less the boot time, so NTP's corrections slew it, and `mach_absolute_time` (CLOCK_UPTIME_RAW,
/// the audio timestamps' clock) stops while the device sleeps.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast, reason = "time_t and c_long are 32 bits on 32-bit Android")]
pub fn device_us() -> i64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
    #[cfg(target_vendor = "apple")]
    const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC_RAW;
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `t` is a valid timespec to write into.
    unsafe { libc::clock_gettime(CLOCK, &mut t) };
    t.tv_sec as i64 * 1_000_000 + t.tv_nsec as i64 / 1_000
}

#[cfg(not(unix))]
pub fn device_us() -> i64 {
    monotonic().as_micros() as i64
}
