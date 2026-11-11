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

/// A client of the plugin host that ignores what it is told.
pub struct Quiet;

impl crate::Client for Quiet {
    fn layer_changed(&self, _: u32) {}
    fn devices_changed(&self) {}
    fn status(&self, _: String, _: String, _: String, _: Vec<String>) {}
    fn shown(&self, _: String, _: String, _: bool, _: bool) {}
    fn notice(&self, _: String, _: bool, _: String, _: Vec<String>) {}
    fn browse(&self, _: String, _: bool) {}
    fn multicast(&self, _: bool) {}
    fn streaming(&self, _: bool) {}
    fn pressed(&self, _: nori_model::deciders::Press) {}
    fn cast(&self, _: Option<crate::Cast>) {}
    fn control(&self, _: crate::Control) {}
    fn sign_in(&self, _: String, _: String) -> bool {
        false
    }
    fn wake(&self, _: Vec<crate::WakeEvent>) {}
    fn commands(&self, _: Vec<crate::Command>) {}
    fn watch_screen(&self, _: bool) {}
    fn asset(&self, _: String, _: String, _: crate::AssetState) {}
}

/// Marks `plugins` as calling plugin `id` from now on, as when nori stops in one of its calls.
pub fn stop_in_a_call(plugins: &crate::Plugins, id: &str) {
    plugins.marks.mark(id, crate::safe::Caller::Worker).set(crate::safe::CALLING, 1);
}

/// Waits for the run before over plugins folder `dir` to let its marks go: its threads end after it.
pub fn marks_released(dir: &Path) {
    if let Ok(file) = std::fs::File::open(dir.join("safe")) {
        file.lock().unwrap();
    }
}

/// The test plugin built with `features`, in a plugin folder of its own.
pub fn fixture(features: &str) -> PathBuf {
    static MADE: AtomicU32 = AtomicU32::new(0);
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/plugin-fixture").join(if features.is_empty() { "plain" } else { features });
    // Held over the build and the copy: cargo links the wasm again on every build, even one with nothing
    // to do, so another test process building it would remove the file being copied.
    std::fs::create_dir_all(&target).unwrap();
    let lock = std::fs::File::create(target.join("lock")).unwrap();
    lock.lock().unwrap();
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

/// The test plugin as nori 0.8 built it, for plugin API 3 (tests/data/test-api3.noriplugin): fixture/ at
/// d9ac50703, built with `--release --target wasm32-wasip2` and `profile.release` opt-level "z", strip,
/// LTO and one codegen unit, so it stays small. Its exports are the `@3.0.0` interfaces; it can't run here.
pub fn older_api_package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/test-api3.noriplugin")
}

/// Lays out [`older_api_package`] under plugins folder `dir` as an older nori left it installed: on, from
/// `repo`, held or not, with a key in its store. Installing refuses it here, so it is written as is.
pub fn installed_by_an_older_nori(dir: &Path, repo: Option<&str>, held: bool) {
    let folder = dir.join(FIXTURE);
    std::fs::create_dir_all(folder.join("strings")).unwrap();
    let files = crate::package::files(&older_api_package()).unwrap();
    for (name, bytes) in &files {
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    let wasm = &files.iter().find(|(n, _)| n == "plugin.wasm").unwrap().1;
    std::fs::write(folder.join(crate::package::MANIFEST), nori_plugin_index::section::custom_section(wasm, "nori:plugin").unwrap()).unwrap();
    crate::store::Store::new(folder.join("store")).set("kept", b"yes");
    let entry = crate::IndexEntry { id: FIXTURE.into(), enabled: true, repo: repo.map(Into::into), held, key: None };
    let index = crate::Index { plugin: vec![entry], repos: repo.into_iter().map(Into::into).collect(), checked_ms: 0 };
    std::fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
}

/// The test plugin built with `features`, as a `.noriplugin`.
pub fn package(features: &str) -> Vec<u8> {
    use std::io::Write;
    let folder = fixture(features);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in ["plugin.wasm", "strings/en.toml"] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&std::fs::read(folder.join(name)).unwrap()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// A repo whose index is at `index` (under https://own.example/), offering the test plugin built now as
/// `version`, for this API: each URL it answers, with its body.
pub fn repo_offering(index: &str, version: &str) -> Vec<(String, Vec<u8>)> {
    use sha2::Digest;
    let pkg = package("");
    let sha: String = sha2::Sha256::digest(&pkg).iter().map(|b| format!("{b:02x}")).collect();
    let listed = format!(r#"{{"plugins": [{{"id": "{FIXTURE}", "name": "Test", "versions": [{{"version": "{version}", "api": {}, "url": "t.noriplugin", "sha256": "{sha}"}}]}}]}}"#, crate::API);
    vec![(index.to_string(), listed.into_bytes()), (nori_plugin_index::resolved(index, "t.noriplugin"), pkg)]
}
