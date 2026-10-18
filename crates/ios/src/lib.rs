//! The C ABI the UIKit app calls (`ios/Sources/nori_ios.h`, kept in step by hand).
//!
//! uniffi in this tree generates Kotlin only (`crates/uniffi-bindgen` has no Swift backend), and the
//! render callback cannot cross a generated binding, so every door is `extern "C"`. One page per call
//! on the coarse path; per-buffer work is `nori_ios_render`.
//!
//! `mod output` is the sound card. `mod session` is the one open `nori_host` session the controls call.
//! `mod pages` reads pages, covers, lyrics and settings for the screens. `mod sound` is each output's
//! sound and AutoEQ. `mod menu` is the song menu, playlists and the sleep timer. `mod remote` is remote
//! control: Bonjour and the devices the music can move to. `mod jam` is a jam this iPod is a guest
//! in. `mod account` saves servers.

mod account;
mod jam;
mod lyrics;
mod menu;
mod output;
mod pages;
mod remote;
mod session;
mod sound;

pub use output::{
    take_interruption, take_route_lost, Interrupt, IosOutput, Sink, PORT_BLUETOOTH, PORT_SPEAKER,
    PORT_WIRED,
};

/// Counts allocations on the calling thread, so the render path can show it makes none.
#[cfg(test)]
pub(crate) mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static N: Cell<u64> = const { Cell::new(0) };
    }

    struct Counting;

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = N.try_with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let _ = N.try_with(|c| c.set(c.get() + 1));
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
            let _ = N.try_with(|c| c.set(c.get() + 1));
            unsafe { System.realloc(ptr, layout, new) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static GLOBAL: Counting = Counting;

    pub fn n() -> u64 {
        N.with(Cell::get)
    }
}

use std::ffi::{c_char, CString};

/// The workspace version, for the about page and the log. A static string: never freed.
#[no_mangle]
pub extern "C" fn nori_ios_version() -> *const c_char {
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr() as *const c_char
}

/// Frees a string this library handed out.
///
/// # Safety
/// `s` came from this library and is freed once.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: made by `CString::into_raw` here.
        drop(unsafe { CString::from_raw(s) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn strings_round_trip_through_the_abi() {
        let s = crate::pages::owned(&serde_json::json!({ "t": "héllo" }));
        assert!(!s.is_null());
        let back = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_string();
        unsafe { nori_ios_free(s) };
        assert_eq!(back, r#"{"t":"héllo"}"#);
        assert_eq!(
            unsafe { CStr::from_ptr(nori_ios_version()) }
                .to_str()
                .unwrap(),
            env!("CARGO_PKG_VERSION")
        );
    }
}
