//! The host over a real plugin (fixture/, built to wasm by nori_plugins::testing): installing, its sound stage and
//! the state nori keeps of it, its lifecycle's overrides and HTTP, and refusing what it can't link.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

use nori_net::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
use nori_plugins::testing::{fixture, FIXTURE};
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

/// Answers each URL it serves with its body, any other with 200 "ok", noting each URL. A `/together/`
/// URL is answered only once two are asked at the same time (else 504 after 5 s).
#[derive(Default)]
struct Web {
    asked: Mutex<Vec<String>>,
    serves: Mutex<Vec<(String, Vec<u8>)>>,
    together: (Mutex<u32>, std::sync::Condvar),
}

#[async_trait::async_trait]
impl Transport for Web {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        self.asked.lock().unwrap().push(url.clone());
        if let Some(name) = url.split("/together/").nth(1) {
            let (waiting, both) = &self.together;
            let mut n = waiting.lock().unwrap();
            *n += 1;
            both.notify_all();
            let (n, timeout) = both.wait_timeout_while(n, std::time::Duration::from_secs(5), |n| *n < 2).unwrap();
            drop(n);
            let status = if timeout.timed_out() { 504 } else { 200 };
            return Ok(TransportResponse { status, body: name.as_bytes().to_vec(), set_cookies: Vec::new() });
        }
        let body = self.serves.lock().unwrap().iter().find(|(u, _)| *u == url).map_or(b"ok".to_vec(), |(_, b)| b.clone());
        Ok(TransportResponse { status: 200, body, set_cookies: Vec::new() })
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
    /// Notices shown: words and args.
    notices: Mutex<Vec<(String, Vec<String>)>>,
    laid: Mutex<Vec<String>>,
}

impl Side for Session {
    fn stored(&self, _: &str) -> Option<Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn laid(&self, id: &str) {
        self.laid.lock().unwrap().push(id.to_string());
    }
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, words: &str, args: &[String]) {
        self.notices.lock().unwrap().push((words.to_string(), args.to_vec()));
    }
}

const ID: &str = FIXTURE;

#[test]
fn a_stage_runs_and_its_state_is_kept() {
    let dir = nori_testdir::TempDir::new("plugins-stage");
    let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(Session::default()));
    plugins.install(&fixture("")).unwrap();
    assert!(plugins.stages().is_empty(), "installed off");
    plugins.turn(ID.into(), true);
    plugins.set(ID.into(), "gain".into(), Value::Level(0.25));
    let specs = plugins.stages();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].rows, vec![0.25], "its rows are its inputs");
    let inputs = |rows: Vec<f64>| nori_player::stage::Inputs { rows, around: Default::default(), song: nori_player::stage::no_facts() };
    let mut stage = specs[0].maker.make(48_000, 2, &inputs(specs[0].rows.clone())).unwrap();
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
    stage.inputs(&inputs(vec![1.0]));
    assert_eq!(block(&mut stage), 4.0, "new inputs, the same state");
    stage.reset();
    assert_eq!(block(&mut stage), 1.0, "a reset keeps the inputs");

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
fn requests_started_together_are_on_their_way_at_once() {
    let dir = nori_testdir::TempDir::new("plugins-together");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.set_transport(Some(session.web.clone()));
    plugins.install(&fixture("")).unwrap();
    plugins.turn(ID.into(), true);
    plugins.act(ID.into(), "together".into());
    let w = plugins.worker(ID).unwrap();
    assert_eq!(nori_tasks::block_on(w.call(|_, _| Ok(1))), Some(1));
    let said = session.notices.lock().unwrap().iter().find(|(w, _)| w == "together").map(|(_, a)| a.clone());
    assert_eq!(said, Some(vec!["200 one|200 two".to_string()]), "each answered, in order, neither waiting on the other");
}

#[test]
fn its_lifecycle_overrides_and_reaches_only_its_hosts() {
    let dir = nori_testdir::TempDir::new("plugins-lifecycle");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.set_transport(Some(session.web.clone()));
    plugins.install(&fixture("")).unwrap();
    assert!(plugins.overrides().is_empty());
    plugins.turn(ID.into(), true);
    let w = plugins.worker(ID).unwrap();
    // Calls run in order: once this one answers, `start` has run.
    assert_eq!(nori_tasks::block_on(w.call(|_, _| Ok(1))), Some(1));
    assert_eq!(plugins.overrides(), vec![("playback.speed".to_string(), "1.25".to_string())], "only the key it may override");
    assert_eq!(*session.web.asked.lock().unwrap(), vec!["https://allowed.example/a".to_string()], "the other host is refused before any request");
    plugins.turn(ID.into(), false);
    assert!(plugins.overrides().is_empty(), "off, it overrides nothing");
    assert!(session.laid.lock().unwrap().len() >= 2, "each change is laid over the settings again");

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

/// The fixture as a `.noriplugin`.
fn package() -> Vec<u8> {
    use std::io::Write;
    let folder = fixture("");
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in ["plugin.wasm", "strings/en.toml"] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&std::fs::read(folder.join(name)).unwrap()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn a_repo_offers_and_installs_the_version_asked() {
    use sha2::Digest;
    let dir = nori_testdir::TempDir::new("plugins-repo");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.set_transport(Some(session.web.clone()));
    let repo = "https://friend.example/nori/index.json";
    plugins.add_repo(repo.into());
    assert_eq!(plugins.repos().iter().map(|r| (r.url.as_str(), r.official)).collect::<Vec<_>>(), vec![(nori_plugins::OFFICIAL, true), (repo, false)]);
    let pkg = package();
    let sha: String = sha2::Sha256::digest(&pkg).iter().map(|b| format!("{b:02x}")).collect();
    // Two versions: the newer one's package is not the one listed.
    let index = format!(
        r#"{{"plugins": [{{"id": "{ID}", "name": "Test", "versions": [
            {{"version": "0.2.0", "api": 1, "url": "test-0.2.0.noriplugin", "sha256": "{}"}},
            {{"version": "0.1.0", "api": 1, "url": "test-0.1.0.noriplugin", "sha256": "{sha}"}}]}}]}}"#,
        "0".repeat(64)
    );
    *session.web.serves.lock().unwrap() = vec![(repo.into(), index.into_bytes()), ("https://friend.example/nori/test-0.2.0.noriplugin".into(), pkg.clone()), ("https://friend.example/nori/test-0.1.0.noriplugin".into(), pkg)];
    let got = nori_tasks::block_on(plugins.offers());
    assert_eq!(got.failed.len(), 1, "nori's own answers no index here: {:?}", got.failed);
    let offer = got.offers.into_iter().find(|o| o.id == ID).unwrap();
    assert_eq!((offer.installed.as_deref(), offer.versions.iter().map(|v| v.version.as_str()).collect::<Vec<_>>()), (None, vec!["0.2.0", "0.1.0"]));
    assert!(matches!(nori_tasks::block_on(plugins.install_version(offer.versions[0].clone())), Err(Refusal::Unreadable(_))), "a package that isn't the one listed");
    assert!(plugins.installed().is_empty());
    assert_eq!(nori_tasks::block_on(plugins.install_version(offer.versions[1].clone())), Ok(ID.to_string()), "an older version");
    let again = nori_tasks::block_on(plugins.offers()).offers.into_iter().find(|o| o.id == ID).unwrap();
    assert_eq!(again.installed.as_deref(), Some("0.1.0"), "listed with what is installed");
    plugins.remove_repo(repo.into());
    assert_eq!(Plugins::open(dir.path().to_path_buf(), session).repos().len(), 1, "kept in the index");
}

#[test]
fn updates_come_daily_from_a_plugins_own_repo_unless_held() {
    use sha2::Digest;
    let dir = nori_testdir::TempDir::new("plugins-update");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.set_transport(Some(session.web.clone()));
    let (own, other) = ("https://own.example/index.json", "https://other.example/index.json");
    plugins.add_repo(own.into());
    plugins.add_repo(other.into());
    let pkg = package();
    let sha: String = sha2::Sha256::digest(&pkg).iter().map(|b| format!("{b:02x}")).collect();
    let index = |versions: &[&str]| {
        let v: Vec<String> = versions.iter().map(|v| format!(r#"{{"version": "{v}", "api": 1, "url": "t-{v}.noriplugin", "sha256": "{sha}"}}"#)).collect();
        format!(r#"{{"plugins": [{{"id": "{ID}", "name": "Test", "versions": [{}]}}]}}"#, v.join(",")).into_bytes()
    };
    let serve = |own_versions: &[&str]| {
        let mut s = vec![(own.to_string(), index(own_versions)), (other.to_string(), index(&["9.0.0"]))];
        for (host, v) in own_versions.iter().map(|v| ("own", *v)).chain([("other", "9.0.0")]) {
            s.push((format!("https://{host}.example/t-{v}.noriplugin"), pkg.clone()));
        }
        *session.web.serves.lock().unwrap() = s;
    };
    serve(&["0.1.0"]);
    let from_own = nori_tasks::block_on(plugins.offers()).offers.into_iter().find(|o| o.repo.url == own).unwrap();
    nori_tasks::block_on(plugins.install_version(from_own.versions[0].clone())).unwrap();
    assert_eq!(plugins.installed()[0].repo.as_deref(), Some(own));

    let day = 24 * 60 * 60 * 1000;
    assert!(plugins.updates_due(day), "never looked");
    assert!(nori_tasks::block_on(plugins.update(day)).is_empty(), "nothing newer from its own repo; the other's 9.0.0 is not its");
    assert!(!plugins.updates_due(day + day / 2), "looked within the day");
    assert!(plugins.updates_due(2 * day));

    serve(&["0.3.0", "0.1.0"]);
    plugins.hold(ID.into(), true);
    assert!(nori_tasks::block_on(plugins.update(2 * day)).is_empty(), "held");
    plugins.hold(ID.into(), false);
    session.web.asked.lock().unwrap().clear();
    let done = nori_tasks::block_on(plugins.update(3 * day));
    assert_eq!(done.iter().map(|u| (u.from.as_str(), u.to.as_str())).collect::<Vec<_>>(), vec![("0.1.0", "0.3.0")]);
    let asked = session.web.asked.lock().unwrap().clone();
    assert!(asked.contains(&"https://own.example/t-0.3.0.noriplugin".to_string()) && !asked.iter().any(|u| u.contains("other.example/t-")), "{asked:?}");
}

#[test]
fn a_call_no_one_waits_for_is_skipped() {
    use std::future::Future;
    let dir = nori_testdir::TempDir::new("plugins-skip");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.install(&fixture("")).unwrap();
    plugins.turn(ID.into(), true);
    let w = plugins.worker(ID).unwrap();
    let ran = Arc::new(Mutex::new(Vec::new()));
    let (go, held) = std::sync::mpsc::channel::<()>();
    // The first call holds the plugin's thread until the second has been dropped.
    let first = {
        let ran = ran.clone();
        w.call(move |_, _| {
            held.recv().unwrap();
            ran.lock().unwrap().push(1);
            Ok(1)
        })
    };
    let second = {
        let ran = ran.clone();
        w.call(move |_, _| {
            ran.lock().unwrap().push(2);
            Ok(2)
        })
    };
    // A call's job is queued at its first poll.
    let (mut first, mut second) = (Box::pin(first), Box::pin(second));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let _ = first.as_mut().poll(&mut cx);
    let _ = second.as_mut().poll(&mut cx);
    drop(second);
    go.send(()).unwrap();
    assert_eq!(nori_tasks::block_on(first), Some(1));
    let third = w.call(|_, _| Ok(3));
    assert_eq!(nori_tasks::block_on(third), Some(3));
    assert_eq!(*ran.lock().unwrap(), vec![1], "the dropped call never ran");
}

#[test]
fn a_preview_shows_a_plugin_without_installing_it() {
    let dir = nori_testdir::TempDir::new("plugins-preview");
    let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(Session::default()));
    let p = plugins.preview_file(fixture("").to_string_lossy().into_owned(), "en".into()).unwrap();
    assert_eq!((p.card.id.as_str(), p.card.name.as_str(), p.card.enabled, p.card.net.as_slice()), (ID, "Test", false, &["allowed.example".to_string()][..]));
    assert_eq!((p.card.rows[0].label.as_str(), p.card.rows[0].value.clone()), ("Gain", Some(Value::Level(0.5))));
    assert!(plugins.installed().is_empty(), "nothing installed by a preview");
    assert_eq!(plugins.install_preview(p), Ok(ID.to_string()));
    assert_eq!(plugins.installed().len(), 1);
    assert!(matches!(plugins.preview_file("/no/such/plugin".into(), "en".into()), Err(Refusal::Unreadable(_))));
}

#[test]
fn slots_ask_the_plugins_that_fill_them() {
    use nori_plugins::{Items, ItemKind, MenuEntry};
    let dir = nori_testdir::TempDir::new("plugins-slots");
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    plugins.install(&fixture("")).unwrap();
    let song = |title: &str| nori_model::Song { id: "s".into(), title: title.into(), ..Default::default() };
    assert!(plugins.menu_entries(ItemKind::Song, "en".into()).is_empty(), "off, it fills nothing");
    assert_eq!(nori_tasks::block_on(plugins.artwork(song("loop"))), None);
    plugins.turn(ID.into(), true);

    let entries = plugins.menu_entries(ItemKind::Song, "en".into());
    assert_eq!(entries, vec![MenuEntry { plugin: ID.into(), action: "count".into(), label: "Count letters".into(), glyph: nori_plugins::Glyph::Info }]);
    assert!(plugins.menu_entries(ItemKind::Album, "en".into()).is_empty(), "only the kinds it names");
    nori_tasks::block_on(plugins.run_menu(entries[0].clone(), Items::Songs(vec![song("a"), song("bc")])));
    assert_eq!(*session.notices.lock().unwrap(), vec![("count".to_string(), vec!["a".to_string(), "bc".to_string()])], "it ran on the songs given");

    let art = nori_tasks::block_on(plugins.artwork(song("loop"))).unwrap();
    assert_eq!((art.plugin.as_str(), art.looping, art.url.as_str()), (ID, true, "https://allowed.example/loop.m3u8"));
    assert_eq!(art.poster.as_deref(), Some("https://allowed.example/first/{size}.jpg"), "a loop carries its first frame");
    assert_eq!(nori_tasks::block_on(plugins.artwork(song("still"))), None, "none: nori's cover");
    let info = nori_tasks::block_on(plugins.song_info(song("four"), "en".into()));
    assert_eq!((info[0].name.as_str(), info[0].lines[0].words.as_str()), ("Test", "4 letters"));
    assert_eq!(nori_tasks::block_on(plugins.player_status()), vec!["Testing".to_string()]);
    assert_eq!(plugins.device_cards("en".into()).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec![ID]);
}
