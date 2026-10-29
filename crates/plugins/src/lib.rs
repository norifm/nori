//! The plugin host. Plugins are WebAssembly components (`wit/`, the `nori:plugin` package), installed
//! under one folder, each in its own: its manifest, its wasm, its strings, its store and its compiled
//! code. With none enabled nothing here runs: no engine is made, and no file is read but the index
//! and each installed plugin's manifest.

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

mod app;
mod assets;
mod bluetooth;
mod cache;
mod cast;
pub mod convert;
mod devices;
mod door;
mod hooks;
mod host;
mod lan;
mod manifest;
mod package;
mod repo;
mod sink;
pub(crate) mod runtime;
mod session;
mod stage;
mod slots;
mod sockets;
mod store;
#[cfg(feature = "testing")]
pub mod testing;
mod timers;
mod view;
mod wake;
mod worker;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use app::{open, open_plugins, Client};
pub use bluetooth::{Bluetooth, BtDevice, BtFailure};
pub use devices::{controlled_output, output_device, Battery, ControlledDevice, Noise};
pub use repo::{Offer, Offers, Repo, RepoFailure, Updated, Version, OFFICIAL};
pub use slots::{Artwork, InfoGroup, InfoLine, Items, MenuEntry};
pub use view::{PluginCard, PluginPreview, PluginRow};
pub use assets::{AssetLine, AssetState};
pub use cast::{Card, Cards, Media, Service, TxtEntry};
pub use hooks::Press;
pub use host::{Side, Value};
pub use lan::{Cast, CastButton, CastKind};
pub use sink::{Encoding, Sink};
pub use manifest::{AssetSpec, DeviceFilter, Glyph, Hook, ItemKind, Manifest, MenuSpec, Permission, Place, Refusal, RowKind, RowSpec, Slot, WakeEvent, API, POINTS};
pub use session::{Control, Found, Library, Moment, Profile, Repeat, Route};
pub use wake::Command;
pub use runtime::{Host, LINKED};
pub use worker::Worker;

/// What a call through a [`Worker`] runs on.
pub type Store = wasmtime::Store<Host>;
/// What such a call answers.
pub type CallResult<T> = wasmtime::Result<T>;

/// The plugin API's generated types and calls, for the core's side of an extension point.
pub mod wit {
    pub use crate::runtime::points::{lifecycle, provider};
    pub use crate::runtime::{types, Plugin};
}

/// A stream URL a provider said is encrypted (the `streams` interface): AES-128 in counter mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encrypted {
    pub url: String,
    pub key: [u8; 16],
    /// The counter for the file's first byte.
    pub iv: [u8; 16],
    /// Bytes at its start, once decrypted, that are not the song's.
    pub skip: u64,
}

impl Host {
    /// What the plugin said during its last call of the URL it returned, taken.
    pub fn take_encrypted(&mut self) -> Option<Encrypted> {
        self.encrypted.take()
    }
}

use nori_player::stage::StageSpec;
use serde::{Deserialize, Serialize};

/// An installed plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub manifest: Arc<Manifest>,
    pub enabled: bool,
    /// The repo it came from, which alone updates it; None when installed from a file.
    pub repo: Option<String>,
    /// Kept at its version: not updated.
    pub held: bool,
}

/// The open profile's transport, which plugins' HTTP goes through; None before one is open.
pub(crate) type Transported = Arc<Mutex<Option<Arc<dyn nori_net::transport::Transport>>>>;

/// A plugin's sound stage maker, by its id.
type Made = (String, Arc<stage::Maker>);

/// `index.json`: the plugins in the user's order, and which are on.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    plugin: Vec<IndexEntry>,
    /// Repos the user added (nori's own is always there).
    #[serde(default)]
    repos: Vec<String>,
    /// When updates were last looked for, device ms.
    #[serde(default)]
    checked_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexEntry {
    id: String,
    enabled: bool,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    held: bool,
}

#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Plugins {
    pub(crate) dir: PathBuf,
    side: Arc<dyn Side>,
    installed: Mutex<Vec<Installed>>,
    /// The enabled sound stages' makers, made at first need and kept while the set is the same.
    stages: Mutex<Option<Vec<Made>>>,
    /// The enabled plugins' own instances, each made at its first call.
    workers: Arc<lan::Workers>,
    /// What they hold on the LAN.
    lan: Arc<lan::Lan>,
    pub(crate) transport: Transported,
    /// The repos the user added, as the index keeps them.
    repos: Mutex<Vec<String>>,
    /// When updates were last looked for, device ms.
    checked_ms: Mutex<i64>,
    /// The platform's Bluetooth, the devices connected and the links plugins hold.
    pub(crate) radio: Arc<bluetooth::Radio>,
    /// Which plugin controls which connected device.
    pub(crate) controls: Arc<devices::Controls>,
    /// Whether the platform watches devices connecting (while an enabled plugin reaches devices).
    pub(crate) watching: Mutex<bool>,
    me: std::sync::Weak<Plugins>,
    /// The open profile's queue and library, the core's.
    library: std::sync::RwLock<Option<Arc<dyn Library>>>,
    pub(crate) live: hooks::Live,
    pub(crate) gate: Arc<timers::Gate>,
    pub(crate) timers: timers::Timers,
    /// Told when a lyric line's wake may have to let go.
    pub(crate) lyrics_changed: Arc<nori_tasks::Notify>,
    /// Where timers and lyric lines wait: made at the first that needs it.
    tasks: std::sync::OnceLock<Arc<dyn nori_tasks::Tasks>>,
    pub(crate) fetches: assets::Fetches,
    /// The wakes and commands the client was last told to keep.
    pub(crate) told: Mutex<(Vec<WakeEvent>, Vec<wake::Command>)>,
}

impl Plugins {
    /// The plugins installed under `dir`, as its index lists them.
    pub fn open(dir: PathBuf, side: Arc<dyn Side>) -> Arc<Plugins> {
        let index: Index = std::fs::read(dir.join("index.json")).ok().and_then(|t| serde_json::from_slice(&t).ok()).unwrap_or_default();
        let installed = index
            .plugin
            .iter()
            .filter_map(|e| {
                let section = std::fs::read(dir.join(&e.id).join(package::MANIFEST)).ok()?;
                let manifest = Manifest::of_section(&section).ok().filter(|m| m.id == e.id)?;
                Some(Installed { manifest: Arc::new(manifest), enabled: e.enabled, repo: e.repo.clone(), held: e.held })
            })
            .collect();
        let workers: Arc<lan::Workers> = Arc::default();
        let gate: Arc<timers::Gate> = Arc::default();
        let lan = lan::Lan::new(side.clone(), Arc::downgrade(&workers), gate.clone());
        let plugins = Arc::new_cyclic(|me| Plugins {
            dir,
            side,
            installed: Mutex::new(installed),
            stages: Mutex::new(None),
            workers,
            lan,
            transport: Arc::default(),
            repos: Mutex::new(index.repos),
            checked_ms: Mutex::new(index.checked_ms),
            radio: Arc::default(),
            controls: Arc::default(),
            watching: Mutex::new(false),
            me: me.clone(),
            library: Default::default(),
            live: Default::default(),
            gate,
            timers: Default::default(),
            lyrics_changed: Default::default(),
            tasks: Default::default(),
            fetches: Default::default(),
            told: Default::default(),
        });
        plugins.refresh();
        plugins
    }

    /// The open profile's transport, for plugins' HTTP; assets waiting for Wi-Fi try again.
    pub fn set_transport(&self, t: Option<Arc<dyn nori_net::transport::Transport>>) {
        *self.transport.lock().expect("not poisoned") = t;
        self.fetch_waiting();
    }

    /// The open profile's queue and library, for plugins' `session` and `subsonic`.
    pub fn set_library(&self, library: Option<Arc<dyn Library>>) {
        *self.library.write().expect("not poisoned") = library;
    }

    pub(crate) fn library(&self) -> Option<Arc<dyn Library>> {
        self.library.read().expect("not poisoned").clone()
    }

    /// Where the timers and lyric lines wait: the client's executor, given before the first plugin
    /// needs one, else one of the host's own.
    pub fn use_tasks(&self, tasks: Arc<dyn nori_tasks::Tasks>) {
        let _ = self.tasks.set(tasks);
    }

    pub(crate) fn tasks(&self) -> Arc<dyn nori_tasks::Tasks> {
        self.tasks.get_or_init(|| Arc::new(nori_tasks::Native::new("nori-plugin-timers").expect("a thread for the plugins' timers"))).clone()
    }

    pub fn installed(&self) -> Vec<Installed> {
        self.installed.lock().expect("not poisoned").clone()
    }

    pub(crate) fn folder(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    /// Installs (or updates) the `.noriplugin` package or plugin folder at `from`, off when new.
    pub fn install(&self, from: &Path) -> Result<Arc<Manifest>, Refusal> {
        self.install_as(from, None)
    }

    /// [`Plugins::install`] of a package from `repo`, which it is updated from from now on. An
    /// update keeps its place, its being on and its being held.
    pub(crate) fn install_as(&self, from: &Path, repo: Option<String>) -> Result<Arc<Manifest>, Refusal> {
        let manifest = Arc::new(package::install(from, &self.dir)?);
        let mut list = self.installed.lock().expect("not poisoned");
        match list.iter_mut().find(|i| i.manifest.id == manifest.id) {
            Some(i) => (i.manifest, i.repo) = (manifest.clone(), repo),
            None => list.push(Installed { manifest: manifest.clone(), enabled: false, repo, held: false }),
        }
        self.keep(&list);
        let on = list.iter().any(|i| i.manifest.id == manifest.id && i.enabled);
        drop(list);
        *self.stages.lock().expect("not poisoned") = None;
        self.workers.lock().expect("not poisoned").remove(&manifest.id);
        self.lan.forget(&manifest.id);
        assets::prune(&self.folder(&manifest.id), &manifest.assets);
        if on {
            self.side.laid(&manifest.id);
            self.rewatch();
        }
        self.refresh();
        Ok(manifest)
    }

    pub(crate) fn added_repos(&self) -> Vec<String> {
        self.repos.lock().expect("not poisoned").clone()
    }

    pub(crate) fn keep_repos(&self, repos: Vec<String>) {
        *self.repos.lock().expect("not poisoned") = repos;
        self.keep(&self.installed());
    }

    pub(crate) fn checked_ms(&self) -> i64 {
        *self.checked_ms.lock().expect("not poisoned")
    }

    pub(crate) fn keep_checked(&self, ms: i64) {
        *self.checked_ms.lock().expect("not poisoned") = ms;
        self.keep(&self.installed());
    }

    fn keep(&self, list: &[Installed]) {
        let plugin = list.iter().map(|i| IndexEntry { id: i.manifest.id.clone(), enabled: i.enabled, repo: i.repo.clone(), held: i.held }).collect();
        let index = Index { plugin, repos: self.added_repos(), checked_ms: self.checked_ms() };
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join("index.json"), serde_json::to_vec(&index).unwrap_or_default());
    }

    /// Plugin `id`'s stored row values.
    pub fn rows(&self, id: &str) -> BTreeMap<String, Value> {
        let s = store::Store::new(self.folder(id).join("store"));
        s.keys(ROWS).into_iter().filter_map(|k| Some((k[ROWS.len()..].to_string(), Value::decode(&s.get(&k)?)?))).collect()
    }

    /// Enabled plugin `id`'s own instance.
    pub fn worker(&self, id: &str) -> Option<Worker> {
        let mut workers = self.workers.lock().expect("not poisoned");
        if let Some(w) = workers.get(id) {
            return Some(w.clone());
        }
        let i = self.installed().into_iter().find(|i| i.enabled && i.manifest.id == id)?;
        let folder = self.folder(id);
        let component = runtime::component(&folder).map_err(|e| nori_model::alog::info(&format!("plugin {id}: {e:?}"))).ok()?;
        let shared = runtime::Shared { side: self.side.clone(), transport: self.transport.clone(), radio: self.radio.clone(), lan: self.lan.clone(), plugins: self.me.clone() };
        let w = Worker::start(i.manifest.clone(), folder, component, shared).ok()?;
        // Started at once, so its overrides are in effect without waiting for a first call.
        w.post(|_, _| Ok(()));
        workers.insert(id.to_string(), w.clone());
        Some(w)
    }

    /// The overrides of the enabled plugins as text, in their order: applied in turn, a later
    /// plugin's value of a key wins (`nori_settings::overrides`).
    pub fn overrides(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for i in self.installed().iter().filter(|i| i.enabled && !i.manifest.overrides.is_empty()) {
            let s = store::Store::new(self.folder(&i.manifest.id).join("store"));
            for k in s.keys(host::OVERRIDES) {
                let key = &k[host::OVERRIDES.len()..];
                if let (true, Some(v)) = (i.manifest.overrides.iter().any(|o| o == key), s.get(&k).and_then(|b| Value::decode(&b))) {
                    out.push((key.to_string(), v.text()));
                }
            }
        }
        out
    }

    /// The enabled plugins' sound stages, with their rows as inputs. Empty, with nothing compiled,
    /// while none is enabled.
    pub fn stages(&self) -> Vec<StageSpec> {
        let wanted: Vec<Installed> = self.installed().into_iter().filter(|i| i.enabled && i.manifest.extends("sound-stage")).collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        let mut made = self.stages.lock().expect("not poisoned");
        let makers = made.get_or_insert_with(|| {
            wanted
                .iter()
                .filter_map(|i| {
                    let c = runtime::component(&self.folder(&i.manifest.id)).map_err(|e| nori_model::alog::info(&format!("plugin {}: {e:?}", i.manifest.id))).ok()?;
                    Some((i.manifest.id.clone(), Arc::new(stage::Maker { id: i.manifest.id.clone(), component: c })))
                })
                .collect()
        });
        makers
            .iter()
            .filter_map(|(id, maker)| {
                let i = wanted.iter().find(|i| &i.manifest.id == id)?;
                let rows: BTreeMap<String, f64> = self.rows(id).into_iter().filter_map(|(k, v)| Some((k, v.number()?))).collect();
                Some(StageSpec { maker: maker.clone(), inputs: i.manifest.row_numbers(&rows) })
            })
            .collect()
    }

    /// What the enabled plugins lay over the settings.
    pub(crate) fn layer(&self) -> nori_settings::overrides::Layer {
        nori_settings::overrides::Layer { overrides: self.overrides(), stages: self.stages() }
    }
}

/// Where a plugin's row values are kept in its store.
pub(crate) const ROWS: &str = "nori/row/";

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// Keeps plugin `id` at its version, or lets it update again.
    pub fn hold(&self, id: String, held: bool) {
        let mut list = self.installed.lock().expect("not poisoned");
        if let Some(i) = list.iter_mut().find(|i| i.manifest.id == *id) {
            i.held = held;
        }
        self.keep(&list);
    }

    /// Turns plugin `id` on or off. On, one with a lifecycle starts at once (what it sets, it sets
    /// now); off, its instance goes.
    pub fn turn(&self, id: String, on: bool) {
        let id = id.as_str();
        let mut list = self.installed.lock().expect("not poisoned");
        let Some(i) = list.iter_mut().find(|i| i.manifest.id == id) else { return };
        i.enabled = on;
        let starts = on && i.manifest.extends("lifecycle");
        self.keep(&list);
        drop(list);
        if !on {
            self.workers.lock().expect("not poisoned").remove(id);
            self.lan.forget(id);
        }
        *self.stages.lock().expect("not poisoned") = None;
        self.side.laid(id);
        self.refresh();
        if starts {
            self.worker(id);
        }
        self.rewatch();
    }

    /// Moves plugin `id` to `index` in the order they apply in.
    pub fn place(&self, id: String, index: u32) {
        let id = id.as_str();
        let mut list = self.installed.lock().expect("not poisoned");
        let Some(at) = list.iter().position(|i| i.manifest.id == id) else { return };
        let p = list.remove(at);
        let index = (index as usize).min(list.len());
        list.insert(index, p);
        self.keep(&list);
        drop(list);
        *self.stages.lock().expect("not poisoned") = None;
        self.side.laid(id);
    }

    /// Sets one of plugin `id`'s rows: a sound stage takes it as an input, a lifecycle hears of it.
    pub fn set(&self, id: String, row: String, value: Value) {
        let id = id.as_str();
        store::Store::new(self.folder(id).join("store")).set(&format!("{ROWS}{row}"), &value.encode());
        let Some(i) = self.installed().into_iter().find(|i| i.enabled && i.manifest.id == id) else { return };
        if i.manifest.extends("sound-stage") {
            self.side.laid(id);
        }
        if i.manifest.extends("lifecycle") {
            if let Some(w) = self.worker(id) {
                let value = value.into();
                w.post(move |store, p| p.nori_plugin_lifecycle().call_changed(store, &row, &value));
            }
        }
    }

    pub fn uninstall(&self, id: String) {
        let id = id.as_str();
        let mut list = self.installed.lock().expect("not poisoned");
        list.retain(|i| i.manifest.id != id);
        self.keep(&list);
        drop(list);
        self.workers.lock().expect("not poisoned").remove(id);
        self.lan.forget(id);
        let _ = std::fs::remove_dir_all(self.folder(id));
        *self.stages.lock().expect("not poisoned") = None;
        self.timers.stop_all(id);
        self.side.laid(id);
        self.refresh();
        self.rewatch();
    }
}
