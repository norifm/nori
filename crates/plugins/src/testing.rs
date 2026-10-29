//! For tests in nori's crates: the test plugin (fixture/), built to wasm and laid out as a plugin
//! folder to install. It has a lifecycle that overrides `playback.speed` and fetches over HTTP, a
//! sound stage whose gain grows each block (its state), and a provider with nori's own answers (a
//! profile with no Subsonic server); with `headphones`, device control instead (fixture/src/headphones.rs).
//! Needs `rustup target add wasm32-wasip2`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// The test plugin's id.
pub const FIXTURE: &str = "fm.nori.test";
/// Its id built with the `second` feature.
pub const SECOND: &str = "fm.nori.test.second";

/// Marks `plugins` as calling plugin `id` from now on, as when nori stops in one of its calls.
pub fn stop_in_a_call(plugins: &crate::Plugins, id: &str) {
    plugins.marks.mark(id, crate::safe::Caller::Worker).set(crate::safe::CALLING, 1);
}

/// The test plugin built with `features`, in a plugin folder of its own.
pub fn fixture(features: &str) -> PathBuf {
    static MADE: AtomicU32 = AtomicU32::new(0);
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/plugin-fixture").join(if features.is_empty() { "plain" } else { features });
    let status = Command::new(env!("CARGO"))
        .args(["build", "-q", "--release", "--target", "wasm32-wasip2", "--features", features, "--manifest-path"])
        .arg(here.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the fixture builds (rustup target add wasm32-wasip2)");
    // One folder per call: tests side by side don't copy over each other's.
    let folder = target.join(format!("plugin-{}-{}", std::process::id(), MADE.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(folder.join("strings")).unwrap();
    std::fs::copy(target.join("wasm32-wasip2/release/nori_test_plugin.wasm"), folder.join("plugin.wasm")).unwrap();
    std::fs::copy(here.join("strings/en.toml"), folder.join("strings/en.toml")).unwrap();
    folder
}
