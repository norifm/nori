//! nori's plugin API for plugin authors: the `nori:plugin` WIT package (`wit/`) as Rust.
//!
//! A plugin is one type implementing the extension points it uses, each a trait here (made from
//! `wit/` by build.rs) whose every call answers as nori would on its own. `plugin!` exports it; the
//! points not named answer with nori's own:
//!
//! ```ignore
//! #[derive(Default)]
//! struct Orbit { phase: f32 }
//! impl nori_plugin_sdk::SoundStage for Orbit { /* ... */ }
//! const MANIFEST: Manifest = Manifest { stage: Some(Slot::Master), ..Manifest::new("fm.nori.orbit") };
//! nori_plugin_sdk::plugin!(Orbit, MANIFEST, sound_stage);
//! ```
//!
//! Build with `cargo build --target wasm32-wasip2 --release`; the `.wasm` is the plugin's `plugin.wasm`.

wit_bindgen::generate!({
    world: "nori:plugin/plugin",
    path: "../../wit",
    pub_export_macro: true,
    export_macro_name: "__export",
    default_bindings_module: "nori_plugin_sdk",
    generate_all,
    // Exported by hand (`stage_abi`): their block must outlive the call for the host to read it.
    skip: ["process", "save"],
});

use std::any::{Any, TypeId};
use std::cell::RefCell;

pub mod manifest;

pub use exports::nori::plugin::*;
pub use manifest::{Manifest, Row, RowKind, Slot};
pub use nori::plugin::{assets, audio_out, bluetooth, cache, cast, discovery, http, lan_serve, log, media, notices, rows, session, settings, sign_in, sockets, store, subsonic, timers, types, wake};

include!(concat!(env!("OUT_DIR"), "/points.rs"));

/// The answer of a call the plugin does not make.
fn unsupported() -> types::Error {
    types::Error { kind: types::ErrorKind::Unsupported, detail: String::new() }
}

/// The extension points a plugin does not name.
#[derive(Default)]
pub struct Nothing;

thread_local! {
    /// The plugin's instances by type, made at first use (a plugin instance runs on one thread). Not a
    /// `HashMap`: its seed asks WASI for randomness, which traps in a sound stage's instance.
    static INSTANCES: RefCell<Vec<(TypeId, Box<dyn Any>)>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` on the instance of `T`, made with its `Default` at first use.
pub fn with<T: Default + 'static, R>(f: impl FnOnce(&mut T) -> R) -> R {
    let key = TypeId::of::<T>();
    let taken = INSTANCES.with(|i| {
        let mut i = i.borrow_mut();
        i.iter().position(|(k, _)| *k == key).map(|at| i.swap_remove(at).1)
    });
    let mut taken = taken.unwrap_or_else(|| Box::new(T::default()));
    let r = f(taken.downcast_mut::<T>().expect("kept by its type"));
    INSTANCES.with(|i| i.borrow_mut().push((key, taken)));
    r
}

/// A sound stage's `process` and `save`, exported by `plugin!` without a post-return: the list each
/// answers stays where it is until the next such call, so the host reads it in place on the audio
/// thread instead of copying it into memory of its own.
#[doc(hidden)]
pub mod stage_abi {
    use std::cell::RefCell;

    use crate::{with, SoundStage};

    thread_local! {
        /// The last block handed back, and the last state; freed at the next call.
        static BLOCK: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
        static STATE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    /// Where an answered list is said: its pointer and length (wasm32's).
    static mut ANSWER: [u32; 2] = [0; 2];

    fn answer(ptr: *const u8, len: usize) -> *const u32 {
        // SAFETY: one thread, and the host reads it before its next call.
        unsafe {
            ANSWER = [ptr as u32, len as u32];
            std::ptr::addr_of!(ANSWER).cast()
        }
    }

    /// `ptr`/`len`: the samples the host placed with `cabi_realloc` (the global allocator, f32s).
    ///
    /// # Safety
    /// Called only by the host, with a list it lowered.
    pub unsafe fn process<S: SoundStage + Default + 'static>(ptr: *mut f32, len: usize) -> *const u32 {
        // SAFETY: allocated as a `Vec<f32>` of `len` would be (size len * 4, align 4).
        let samples = unsafe { Vec::from_raw_parts(ptr, len, len) };
        let out = with::<S, _>(|s| s.process(samples));
        let (p, n) = (out.as_ptr().cast::<u8>(), out.len());
        BLOCK.with(|b| *b.borrow_mut() = out);
        answer(p, n)
    }

    /// # Safety
    /// Called only by the host.
    pub unsafe fn save<S: SoundStage + Default + 'static>() -> *const u32 {
        let state = with::<S, _>(|s| s.save());
        let (p, n) = (state.as_ptr(), state.len());
        STATE.with(|b| *b.borrow_mut() = state);
        answer(p, n)
    }
}

/// A sound stage's state as bytes, for a plain `Copy` struct holding all of it.
pub mod state {
    /// `state`'s bytes.
    pub fn save<T: Copy>(state: &T) -> Vec<u8> {
        // SAFETY: `T` is `Copy`, so it has no drop glue or owned pointers; its bytes are all of it.
        unsafe { std::slice::from_raw_parts((state as *const T).cast::<u8>(), std::mem::size_of::<T>()) }.to_vec()
    }

    /// Puts `bytes` from [`save`] back into `state`; other bytes (another version's) leave it as it is.
    pub fn restore<T: Copy>(state: &mut T, bytes: &[u8]) {
        if bytes.len() == std::mem::size_of::<T>() {
            // SAFETY: as long as `T`, and made by `save` from a `T`.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), (state as *mut T).cast::<u8>(), bytes.len()) }
        }
    }
}
