//! The host over a real plugin (tests/fixture, built to wasm here): installing, its sound stage and
//! the state nori keeps of it, its lifecycle's overrides and HTTP, and refusing what it can't link.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use nori_net::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
use nori_plugins::{Plugins, Refusal, Side, Value};

/// Counts this thread's allocations while `COUNTING` is on.
struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocations `f` makes on this thread.
fn allocations(f: impl FnOnce()) -> u64 {
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(Cell::get)
}

/// The fixture built with `features`, as a plugin folder.
fn fixture(features: &str) -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/plugin-fixture").join(if features.is_empty() { "plain" } else { features });
    let status = Command::new(env!("CARGO"))
        .args(["build", "-q", "--release", "--target", "wasm32-wasip2", "--features", features, "--manifest-path"])
        .arg(here.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the fixture builds (rustup target add wasm32-wasip2)");
    let folder = target.join("plugin");
    std::fs::create_dir_all(folder.join("strings")).unwrap();
    std::fs::copy(target.join("wasm32-wasip2/release/nori_test_plugin.wasm"), folder.join("plugin.wasm")).unwrap();
    std::fs::copy(here.join("strings/en.toml"), folder.join("strings/en.toml")).unwrap();
    folder
}

/// Answers every request with 200 "ok", noting each URL.
#[derive(Default)]
struct Web {
    asked: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Transport for Web {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        self.asked.lock().unwrap().push(url);
        Ok(TransportResponse { status: 200, body: b"ok".to_vec(), set_cookies: Vec::new() })
    }

    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(e.url, e.timeout_ms).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> Network {
        Network::Unmetered
    }
}

#[derive(Default)]
struct Session {
    web: Arc<Web>,
    overrides_changed: Mutex<Vec<String>>,
}

impl Side for Session {
    fn transport(&self) -> Option<Arc<dyn Transport>> {
        Some(self.web.clone())
    }
    fn stored(&self, _: &str) -> Option<Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn overrides_changed(&self, id: &str) {
        self.overrides_changed.lock().unwrap().push(id.to_string());
    }
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, _: &str, _: &[String]) {}
}

const ID: &str = "fm.nori.test";

#[test]
fn a_stage_runs_and_its_state_is_kept() {
    let dir = nori_testdir::TempDir::new("plugins-stage");
    let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(Session::default()));
    plugins.install(&fixture("")).unwrap();
    assert!(plugins.stages().is_empty(), "installed off");
    plugins.set_enabled(ID, true);
    plugins.set_row(ID, "gain", Value::Level(0.25));
    let specs = plugins.stages();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].inputs, vec![0.25], "its rows are its inputs");
    let mut stage = specs[0].maker.make(48_000, 2, &specs[0].inputs).unwrap();
    let block = |stage: &mut Box<dyn nori_player::stage::SoundStage>| {
        let mut b = vec![1.0f32; 16];
        stage.process(&mut b);
        b[0]
    };
    assert_eq!(block(&mut stage), 0.25);
    assert_eq!(block(&mut stage), 0.5);
    let mut kept = Vec::new();
    stage.save(&mut kept);
    assert_eq!(block(&mut stage), 0.75);
    stage.restore(&kept);
    assert_eq!(block(&mut stage), 0.75, "back where it was kept");
    stage.inputs(&[1.0]);
    assert_eq!(block(&mut stage), 4.0, "new inputs, the same state");
    stage.restore(&kept);
    assert_eq!(block(&mut stage), 0.75, "the inputs it had then come back with it");
    stage.reset();
    assert_eq!(block(&mut stage), 0.25);

    // On the audio path, once its buffers have grown, nothing is allocated.
    let (mut b, mut kept) = (vec![0.1f32; 16_384], Vec::new());
    stage.process(&mut b);
    stage.save(&mut kept);
    stage.restore(&kept);
    assert_eq!(allocations(|| for _ in 0..20 {
        stage.process(&mut b);
        stage.save(&mut kept);
    }), 0);
}

#[test]
fn its_lifecycle_overrides_and_reaches_only_its_hosts() {
    let dir = nori_testdir::TempDir::new("plugins-lifecycle");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.install(&fixture("")).unwrap();
    assert!(plugins.overrides().is_empty());
    plugins.set_enabled(ID, true);
    let w = plugins.worker(ID).unwrap();
    // Calls run in order: once this one answers, `start` has run.
    assert_eq!(nori_tasks::block_on(w.call(|_, _| Ok(1))), Some(1));
    assert_eq!(plugins.overrides().into_iter().collect::<Vec<_>>(), vec![("playback.speed".to_string(), Value::Level(1.25))], "only the key it may override");
    assert_eq!(*session.web.asked.lock().unwrap(), vec!["https://allowed.example/a".to_string()], "the other host is refused before any request");
    plugins.set_enabled(ID, false);
    assert!(plugins.overrides().is_empty(), "off, it overrides nothing");
    assert!(session.overrides_changed.lock().unwrap().len() >= 2);

    // Reopened, the folder keeps what was installed and set.
    let again = Plugins::open(dir.path().to_path_buf(), session);
    assert_eq!(again.installed().iter().map(|i| (i.manifest.id.as_str(), i.enabled)).collect::<Vec<_>>(), vec![(ID, false)]);
}

#[test]
fn a_plugin_needing_what_this_host_lacks_is_refused() {
    let dir = nori_testdir::TempDir::new("plugins-refused");
    let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(Session::default()));
    assert_eq!(plugins.install(&fixture("timers")), Err(Refusal::Needs(vec!["timers".to_string()])));
    assert!(plugins.installed().is_empty());
}
