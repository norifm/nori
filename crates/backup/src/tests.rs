use std::sync::{Arc, Mutex};

use nori_net::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
use nori_plugins::testing::{fixture, FIXTURE, SECOND};
use nori_plugins::{KeptPlugin, PluginReturn, Value};
use nori_settings::settings::{SavedServer, StoredPrefs};

use super::*;

const CHEAP: seal::Cost = seal::Cost { memory_kib: 64, passes: 1, lanes: 1 };
const REPO: &str = "https://friend.example/nori/index.json";

/// Serves each URL it has, any other with 200 "ok".
#[derive(Default)]
struct Web {
    serves: Mutex<Vec<(String, Vec<u8>)>>,
    /// What plugins sent.
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Transport for Web {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        let found = self.serves.lock().unwrap().iter().find(|(u, _)| *u == url).map(|(_, b)| b.clone());
        Ok(TransportResponse { status: if found.is_some() || !url.ends_with(".json") { 200 } else { 404 }, body: found.unwrap_or_else(|| b"ok".to_vec()), headers: Vec::new() })
    }

    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.sent.lock().unwrap().push(e.url.clone());
        self.get(e.url, e.timeout_ms).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> Network {
        Network::Unmetered
    }
}

struct Quiet;

impl nori_plugins::Client for Quiet {
    fn layer_changed(&self, _: u32) {}
    fn devices_changed(&self) {}
    fn status(&self, _: String, _: String, _: String, _: Vec<String>) {}
    fn shown(&self, _: String, _: String, _: bool, _: bool) {}
    fn notice(&self, _: String, _: bool, _: String, _: Vec<String>) {}
    fn browse(&self, _: String, _: bool) {}
    fn multicast(&self, _: bool) {}
    fn streaming(&self, _: bool) {}
    fn pressed(&self, _: nori_model::deciders::Press) {}
    fn cast(&self, _: Option<nori_plugins::Cast>) {}
    fn control(&self, _: nori_plugins::Control) {}
    fn sign_in(&self, _: String, _: String) -> bool {
        false
    }
    fn wake(&self, _: Vec<nori_plugins::WakeEvent>) {}
    fn commands(&self, _: Vec<nori_plugins::Command>) {}
    fn watch_screen(&self, _: bool) {}
    fn asset(&self, _: String, _: String, _: nori_plugins::AssetState) {}
}

/// The fixture built with `features` as a `.noriplugin`, and its sha256.
fn package(features: &str) -> (Vec<u8>, String) {
    use sha2::Digest;
    use std::io::Write;
    let folder = fixture(features);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in ["plugin.wasm", "strings/en.toml"] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&std::fs::read(folder.join(name)).unwrap()).unwrap();
    }
    let pkg = z.finish().unwrap().into_inner();
    let sha = sha2::Sha256::digest(&pkg).iter().map(|b| format!("{b:02x}")).collect();
    (pkg, sha)
}

/// One device: its settings, its plugins and its connection.
struct Device {
    _dir: nori_testdir::TempDir,
    settings: Arc<Settings>,
    plugins: Arc<Plugins>,
    web: Arc<Web>,
}

impl Device {
    fn new(name: &str) -> Device {
        let dir = nori_testdir::TempDir::new(name);
        let settings = Settings::new();
        settings.open(&dir.join("nori.db").display().to_string()).unwrap();
        let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(Quiet));
        let web = Arc::new(Web::default());
        plugins.set_transport(Some(web.clone()));
        Device { _dir: dir, settings, plugins, web }
    }

    /// Its repo lists each `(id, versions)` with `features`' package.
    fn serve(&self, listed: &[(&str, &str, &[&str])]) {
        let mut serves = Vec::new();
        let mut entries = Vec::new();
        for (id, features, versions) in listed {
            let (pkg, sha) = package(features);
            let v: Vec<String> = versions.iter().map(|v| format!(r#"{{"version": "{v}", "api": 2, "url": "{id}-{v}.noriplugin", "sha256": "{sha}"}}"#)).collect();
            entries.push(format!(r#"{{"id": "{id}", "name": "{id}", "versions": [{}]}}"#, v.join(",")));
            serves.extend(versions.iter().map(|v| (format!("https://friend.example/nori/{id}-{v}.noriplugin"), pkg.clone())));
        }
        serves.push((REPO.to_string(), format!(r#"{{"plugins": [{}]}}"#, entries.join(",")).into_bytes()));
        *self.web.serves.lock().unwrap() = serves;
    }

    fn install(&self, id: &str) {
        let offers = nori_tasks::block_on(self.plugins.offers()).offers;
        let v = offers.into_iter().find(|o| o.id == id).unwrap().versions.remove(0);
        nori_tasks::block_on(self.plugins.install_version(v)).unwrap();
    }

    /// Once its lifecycle has started, what it set then is in its store.
    fn started(&self, id: &str) {
        assert_eq!(nori_tasks::block_on(self.plugins.worker(id).unwrap().call(|_, _| Ok(1))), Some(1));
    }

    fn restore(&self, file: &[u8], password: &str) -> Result<Restored, BackupError> {
        nori_tasks::block_on(restore(&self.settings, &self.plugins, file, password))
    }
}

/// A device set up in every way a backup keeps.
fn set_up() -> Device {
    let a = Device::new("backup-made");
    a.serve(&[(FIXTURE, "", &["0.1.0"]), (SECOND, "second", &["0.1.0"])]);
    a.plugins.add_repo(REPO.into());
    a.install(FIXTURE);
    a.install(SECOND);
    a.started(SECOND);
    a.plugins.turn(SECOND.into(), false);
    a.plugins.set(FIXTURE.into(), "gain".into(), Value::Level(1.5));
    a.plugins.turn(FIXTURE.into(), true);
    a.started(FIXTURE);
    a.plugins.hold(SECOND.into(), true);
    a.plugins.place(SECOND.into(), 0);
    let server = SavedServer { id: "s1".into(), url: "https://music.example".into(), user: "me".into(), password: "hunter2".into(), ..SavedServer::default() };
    a.settings.put(StoredPrefs { servers: vec![server], active_server_id: "s1".into(), crossfade_sec: 4, ..StoredPrefs::default() });
    a.settings.keep_sound_profiles(&[SoundProfile { name: "Desk".into(), json: "{}".into(), outputs: vec!["usb:dac".into()] }]).unwrap();
    a
}

#[test]
fn settings_profiles_and_plugins_come_back() {
    let a = set_up();
    let kept = a.plugins.kept();
    assert!(kept.plugins[1].store.keys().any(|k| k.starts_with("nori/override/")) && kept.plugins[1].store.contains_key("heard"), "its overrides and own keys: {:?}", kept.plugins[1].store.keys());
    let file = make_at(&a.settings, &a.plugins, "pw", CHEAP).unwrap();
    assert!(!file.windows(7).any(|w| w == b"hunter2"), "secrets are sealed");

    let b = Device::new("backup-restored");
    b.serve(&[(FIXTURE, "", &["0.1.0"]), (SECOND, "second", &["0.1.0"])]);
    let restored = b.restore(&file, "pw").unwrap();
    b.started(FIXTURE);
    assert_eq!(restored.plugins.iter().map(|p| (p.id.as_str(), p.outcome.clone())).collect::<Vec<_>>(), [(SECOND, PluginReturn::AsKept), (FIXTURE, PluginReturn::AsKept)]);
    assert_eq!(b.settings.current(), a.settings.current());
    assert_eq!((restored.settings.prefs, restored.settings.server), (a.settings.current().unwrap(), true), "with no profile here, the backup's is in use");
    assert_eq!(b.settings.sound_profiles().unwrap(), a.settings.sound_profiles().unwrap());
    assert_eq!(b.plugins.kept(), kept, "the same order, being on and held, repos and stores");
    assert_eq!(b.settings.layer().overrides, a.settings.layer().overrides, "the enabled plugin's overrides are laid over the settings");
    assert!(!b.settings.layer().overrides.is_empty());
}

#[test]
fn a_plugin_kept_off_never_starts_on_a_restore() {
    let a = Device::new("backup-kept-off");
    a.serve(&[(FIXTURE, "", &["0.1.0"])]);
    a.plugins.add_repo(REPO.into());
    a.install(FIXTURE);
    a.plugins.turn(FIXTURE.into(), false);
    let file = make_at(&a.settings, &a.plugins, "pw", CHEAP).unwrap();

    let b = Device::new("backup-kept-off-here");
    b.serve(&[(FIXTURE, "", &["0.1.0"])]);
    b.restore(&file, "pw").unwrap();
    assert_eq!(b.plugins.installed().iter().map(|i| i.enabled).collect::<Vec<_>>(), [false]);
    let Device { _dir, settings, plugins, web } = b;
    drop((settings, plugins));
    nori_plugins::testing::marks_released(&_dir.join("plugins"));
    assert_eq!(*web.sent.lock().unwrap(), Vec::<String>::new(), "its start fetches");
}

#[test]
fn a_restore_puts_back_what_it_finds() {
    let a = set_up();
    let mut kept = a.plugins.kept();
    // Backed up at a version its repo no longer lists, one installed from a file, one its repo dropped.
    kept.plugins[0].version = "0.0.9".into();
    kept.plugins.push(KeptPlugin { repo: None, ..kept.plugins[0].clone() });
    kept.plugins.last_mut().unwrap().id = "fm.nori.sideloaded".into();
    kept.plugins.push(KeptPlugin { id: "fm.nori.dropped".into(), ..kept.plugins[1].clone() });
    let contents = Contents { settings: a.settings.backed_up(), sound_profiles: Vec::new(), plugins: kept };
    let file = seal::seal(&serde_json::to_vec(&contents).unwrap(), "pw", CHEAP).unwrap();

    let b = Device::new("backup-found");
    b.serve(&[(FIXTURE, "", &["0.1.0"]), (SECOND, "second", &["0.1.0"])]);
    let got: Vec<(String, PluginReturn)> = b.restore(&file, "pw").unwrap().plugins.into_iter().map(|p| (p.id, p.outcome)).collect();
    let want = [(SECOND, PluginReturn::Other { version: "0.1.0".into() }), (FIXTURE, PluginReturn::AsKept), ("fm.nori.sideloaded", PluginReturn::FromFile), ("fm.nori.dropped", PluginReturn::Unavailable)];
    assert_eq!(got, want.map(|(id, o)| (id.to_string(), o)));
    assert_eq!(b.plugins.installed().iter().map(|i| i.manifest.id.as_str()).collect::<Vec<_>>(), [SECOND, FIXTURE]);
}

#[test]
fn a_profile_here_stays_in_use() {
    let a = set_up();
    let file = make_at(&a.settings, &a.plugins, "pw", CHEAP).unwrap();
    let b = Device::new("backup-here");
    b.serve(&[]);
    let mine = SavedServer { id: "h1".into(), url: "https://music.example".into(), user: "me".into(), password: "newer".into(), ..SavedServer::default() };
    b.settings.put(StoredPrefs { servers: vec![mine.clone()], active_server_id: "h1".into(), ..StoredPrefs::default() });
    let r = b.restore(&file, "pw").unwrap();
    assert_eq!((r.settings.prefs.servers, r.settings.prefs.active_server_id.as_str(), r.settings.server), (vec![mine], "h1", false));
    assert_eq!(r.settings.prefs.crossfade_sec, 4);
    assert!(r.plugins.iter().all(|p| p.outcome == PluginReturn::Unavailable), "its repo lists nothing now");
}

#[test]
fn a_backup_that_cant_be_opened_changes_nothing() {
    let a = set_up();
    let file = make_at(&a.settings, &a.plugins, "pw", CHEAP).unwrap();
    let b = Device::new("backup-refused");
    b.serve(&[(FIXTURE, "", &["0.1.0"]), (SECOND, "second", &["0.1.0"])]);
    let before = b.settings.current();
    assert_eq!(b.restore(&file, "Pw"), Err(BackupError::WrongPassword));
    let mut tampered = file.clone();
    let at = tampered.len() / 2;
    tampered[at] ^= 4;
    assert_eq!(b.restore(&tampered, "pw"), Err(BackupError::Damaged));
    assert_eq!(b.restore(b"{\"settings\": {}}", "pw"), Err(BackupError::NotABackup));
    assert_eq!(make_at(&a.settings, &a.plugins, "", CHEAP), Err(BackupError::NoPassword));
    assert_eq!(b.settings.current(), before);
    assert!(b.plugins.installed().is_empty() && b.settings.sound_profiles().unwrap().is_empty());
}
