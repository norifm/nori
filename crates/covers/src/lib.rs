//! Cover art: fetched through the core's [`Transport`](nori_core::transport::Transport), cached on disk
//! ([`DiskCache`]), decoded to the drawn size ([`Decoder`]) and cached in memory ([`MemoryCache`]).
//! [`Loader`] serves shared, cancellable requests on worker threads; [`Paint`] picks the output (RGBA
//! rows, or Android Bitmaps in crates/android covers.rs).

pub mod decode;
pub mod disk;
pub mod loader;
mod lru;
pub mod memory;
pub mod scale;

pub use decode::{format, header, Decoder, Error as DecodeError, Format, Header};
pub use disk::{DiskCache, Key};
pub use loader::{Config, Error, Loader, Paint, Rgba, Ticket};
pub use memory::{Image, MemoryCache, Sized};
pub use scale::{Alpha, Target};

/// The GPU driver family a device draws with, as far as covers care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gpu {
    /// Imagination's PowerVR (the Pixel 10's Tensor G5).
    PowerVr,
    Other,
}

/// Whether covers go to the GPU as hardware bitmaps, where the platform has them. Not over PowerVR: its
/// gralloc writes to freed memory while a hardware bitmap uploads, which a hardened allocator aborts on.
pub fn hardware_bitmaps(platform_has_them: bool, gpu: Gpu) -> bool {
    platform_has_them && gpu != Gpu::PowerVr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_stay_in_software_over_powervr() {
        for (platform, gpu, want) in [(true, Gpu::Other, true), (true, Gpu::PowerVr, false), (false, Gpu::Other, false), (false, Gpu::PowerVr, false)] {
            assert_eq!(hardware_bitmaps(platform, gpu), want, "{platform} {gpu:?}");
        }
    }
}
