//! The plugin host. Plugins are WebAssembly components (`wit/`, the `nori:plugin` package), installed
//! under one folder, each in its own: its manifest, its wasm, its strings, its store and its compiled
//! code. With none enabled nothing here runs: no engine is made, and no file is read but the index,
//! each installed plugin's manifest and safe mode's marks (safe.rs).

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

mod app;
mod assets;
mod automix;
mod bluetooth;
mod cache;
mod cast;
pub mod convert;
mod deciders;
pub mod deep;
mod devices;
mod door;
mod hooks;
mod host;
mod kept;
mod lan;
mod lyrics;
mod manifest;
mod package;
mod prepare;
mod repo;
mod sink;
mod scrobbler;
pub(crate) mod runtime;
mod safe;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub use app::{open, open_plugins, Client};
pub use bluetooth::{Bluetooth, BtDevice, BtFailure};
pub use devices::{controlled_output, output_device, Battery, ControlledDevice, Noise};
pub use safe::SafeStart;
pub use repo::{Offer, Offers, Repo, RepoFailure, Updated, Version, OFFICIAL};
pub use slots::{Artwork, InfoGroup, InfoLine, Items, MenuEntry};
pub use view::{PluginCard, PluginPreview, PluginRow};
pub use assets::{AssetLine, AssetState};
pub use cast::{Card, Cards, Media, Service, TxtEntry};
pub use hooks::Press;
pub use host::{Side, Value};
pub use kept::{KeptPlugin, KeptPlugins, PluginRestored, PluginReturn};
pub use lan::{Cast, CastButton, CastKind};
pub use sink::{Encoding, Sink};
pub use manifest::{AssetSpec, DeviceFilter, Glyph, Hook, ItemKind, Manifest, MenuSpec, Permission, Place, Refusal, RowKind, RowSpec, Slot, WakeEvent, API, POINTS};
pub use session::{Control, Found, Library, Moment, Profile, Repeat, Route};
pub use wake::Command;
pub use runtime::{Host, LINKED};
pub use worker::Worker;
pub use prepare::Songs;

/// What a call through a [`Worker`] runs on.
pub type Store = wasmtime::Store<Host>;
/// What such a call answers.
pub type CallResult<T> = wasmtime::Result<T>;

/// The plugin API's generated types and calls, for the core's side of an extension point.
pub mod wit {
    pub use crate::runtime::points::{lifecycle, provider};
    pub use crate::runtime::{types, Plugin};
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
    /// Its author's key (hex), pinned by the first signed version installed: every later version must
    /// be signed by it. None while it has had no signed version.
    pub key: Option<String>,
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
    #[serde(default)]
    key: Option<String>,
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
    /// Whether the platform watches devices connecting (while an enabled plugin reaches devices, or
    /// hears outputs connect where the platform's Bluetooth tells of them).
    pub(crate) watching: Mutex<bool>,
    /// Whether the platform's Bluetooth is how this client hears output devices connect (the desktop's
    /// BlueZ; Android has its audio device callback).
    pub(crate) bluetooth_outputs: std::sync::atomic::AtomicBool,
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
    /// The plugins hearing songs as they are read, made at first need while one is enabled.
    preparation: Mutex<Option<Arc<prepare::Preparation>>>,
    /// Finds the open profile's songs by id, for what plugins are handed of a song.
    songs: Arc<Mutex<Option<prepare::Songs>>>,
    /// Which plugin is being called, for the next start (safe.rs).
    marks: Arc<safe::Marks>,
    /// Every plugin is off for this start (a safe start), until one is turned on.
    paused: AtomicBool,
    /// Why plugins are off at this start, until the client takes it.
    safe_start: Mutex<Option<SafeStart>>,
}

impl Plugins {
    /// The plugins installed under `dir`, as its index lists them. One the last run stopped in a call
    /// of is turned off; a last start that never finished with none marked starts this one with every
    /// plugin off.
    pub fn open(dir: PathBuf, side: Arc<dyn Side>) -> Arc<Plugins> {
        let index: Index = std::fs::read(dir.join("index.json")).ok().and_then(|t| serde_json::from_slice(&t).ok()).unwrap_or_default();
        let mut installed: Vec<Installed> = index
            .plugin
            .iter()
            .filter_map(|e| {
                let section = std::fs::read(dir.join(&e.id).join(package::MANIFEST)).ok()?;
                let manifest = Manifest::of_section(&section).ok().filter(|m| m.id == e.id)?;
                Some(Installed { manifest: Arc::new(manifest), enabled: e.enabled, repo: e.repo.clone(), held: e.held, key: e.key.clone() })
            })
            .collect();
        let (marks, last) = safe::Marks::open(dir.join("safe"), !installed.is_empty());
        let mut safe_start = None;
        if let Some(last) = last {
            let crashed: Vec<String> = installed
                .iter_mut()
                .filter(|i| i.enabled && last.stopped_in.iter().any(|s| s == safe::cut(&i.manifest.id)))
                .map(|i| {
                    i.enabled = false;
                    i.manifest.id.clone()
                })
                .collect();
            if !crashed.is_empty() {
                nori_model::alog::info(&format!("safe mode: nori stopped while calling {}, off now", crashed.join(", ")));
                safe_start = Some(SafeStart::Crashed { plugins: crashed });
            } else if last.starting && installed.iter().any(|i| i.enabled) {
                nori_model::alog::info("safe mode: the last start didn't finish, with no plugin being called; every plugin is off for this one");
                safe_start = Some(SafeStart::AllOff);
            }
        }
        let paused = safe_start == Some(SafeStart::AllOff);
        let crashed = matches!(safe_start, Some(SafeStart::Crashed { .. }));
        let starts = !paused && installed.iter().any(|i| i.enabled);
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
            bluetooth_outputs: Default::default(),
            me: me.clone(),
            library: Default::default(),
            live: Default::default(),
            gate,
            timers: Default::default(),
            lyrics_changed: Default::default(),
            tasks: Default::default(),
            fetches: Default::default(),
            told: Default::default(),
            preparation: Mutex::new(None),
            songs: Arc::default(),
            marks,
            paused: AtomicBool::new(paused),
            safe_start: Mutex::new(safe_start),
        });
        if crashed {
            plugins.keep(&plugins.installed());
        }
        plugins.marks.starting(starts);
        plugins.refresh();
        plugins
    }

    /// The plugins that run now: those on, unless every plugin is off (the switch, or a safe start).
    pub fn running(&self) -> Vec<Installed> {
        if self.side.all_off() {
            return Vec::new();
        }
        self.laying()
    }

    /// The plugins whose overrides and stages are laid over the settings: those on, unless this is a
    /// safe start. The settings leave the layer out themselves while the switch turns every plugin off,
    /// so it is whole again when the switch goes back.
    fn laying(&self) -> Vec<Installed> {
        if self.paused.load(Ordering::Relaxed) {
            return Vec::new();
        }
        self.installed().into_iter().filter(|i| i.enabled).collect()
    }

    /// What an instance's imports reach beyond its folder.
    fn shared(&self) -> runtime::Shared {
        runtime::Shared { side: self.side.clone(), transport: self.transport.clone(), radio: self.radio.clone(), lan: self.lan.clone(), plugins: self.me.clone() }
    }

    /// How the open profile's songs are found by id.
    pub fn set_songs(&self, songs: prepare::Songs) {
        *self.songs.lock().expect("not poisoned") = Some(songs);
    }

    /// The sound stages and the preparation are made again at their next need: the plugins changed.
    fn forget_made(&self) {
        *self.stages.lock().expect("not poisoned") = None;
        *self.preparation.lock().expect("not poisoned") = None;
    }

    /// The enabled plugins exporting `preparation` or `automix`, hearing songs as they are read;
    /// None, with nothing made, while there are none.
    pub(crate) fn preparation(&self) -> Option<Arc<prepare::Preparation>> {
        let hearing: Vec<Installed> = self.running().into_iter().filter(|i| i.manifest.extends("preparation") || i.manifest.extends("automix")).collect();
        if hearing.is_empty() {
            return None;
        }
        let mut made = self.preparation.lock().expect("not poisoned");
        if made.is_none() {
            let hearers = hearing
                .iter()
                .filter_map(|i| {
                    let folder = self.folder(&i.manifest.id);
                    let mark = self.marks.mark(&i.manifest.id, safe::Caller::Preparation);
                    let component = mark.calling(|| runtime::component(&folder)).map_err(|e| nori_model::alog::info(&format!("plugin {}: {e:?}", i.manifest.id))).ok()?;
                    // An instance of its own: preparing a song takes long, and its other calls go on meanwhile.
                    let worker = Worker::start(i.manifest.clone(), folder, component, self.shared(), mark).ok()?;
                    Some(prepare::Hearer { id: i.manifest.id.clone(), worker, prepares: i.manifest.extends("preparation"), analyses: i.manifest.extends("automix") })
                })
                .collect();
            let songs = self.songs.clone();
            let find: prepare::Songs = Arc::new(move |id: &str| {
                let found = songs.lock().expect("not poisoned").as_ref().and_then(|f| f(id));
                Some(found.unwrap_or_else(|| nori_model::Song::only_id(id.to_string())))
            });
            *made = Some(prepare::Preparation::start(hearers, find));
        }
        made.clone()
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

    /// Installs (or updates) the `.noriplugin` package or plugin folder at `from`, off when new. A file
    /// is unsigned: it can't update a plugin whose author's key is pinned.
    pub fn install(&self, from: &Path) -> Result<Arc<Manifest>, Refusal> {
        self.install_as(from, None, None)
    }

    /// Whether a version of plugin `id` signed by `key` (None: unsigned) may replace the one installed:
    /// any may while none is, or while the one installed has no pinned key.
    pub(crate) fn admits(&self, id: &str, key: Option<&str>) -> Result<(), Refusal> {
        let list = self.installed.lock().expect("not poisoned");
        match list.iter().find(|i| i.manifest.id == id).and_then(|i| i.key.as_deref()) {
            Some(pinned) if Some(pinned) != key => Err(Refusal::OtherKey),
            _ => Ok(()),
        }
    }

    /// [`Plugins::install`] of a package from `repo`, which it is updated from from now on, signed by
    /// `key` (already checked). An update keeps its place, its being on and its being held; the first
    /// signed version pins its key.
    pub(crate) fn install_as(&self, from: &Path, repo: Option<String>, key: Option<String>) -> Result<Arc<Manifest>, Refusal> {
        let manifest = Arc::new(package::install(from, &self.dir, |m| self.admits(&m.id, key.as_deref()))?);
        let mut list = self.installed.lock().expect("not poisoned");
        match list.iter_mut().find(|i| i.manifest.id == manifest.id) {
            Some(i) => {
                (i.manifest, i.repo) = (manifest.clone(), repo);
                i.key = i.key.take().or(key);
            }
            None => list.push(Installed { manifest: manifest.clone(), enabled: false, repo, held: false, key }),
        }
        self.keep(&list);
        let on = list.iter().any(|i| i.manifest.id == manifest.id && i.enabled);
        drop(list);
        self.forget_made();
        self.workers.lock().expect("not poisoned").remove(&manifest.id);
        self.lan.forget(&manifest.id);
        assets::prune(&self.folder(&manifest.id), &manifest.assets);
        if on {
            self.side.laid(&manifest.id);
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
        let plugin = list.iter().map(|i| IndexEntry { id: i.manifest.id.clone(), enabled: i.enabled, repo: i.repo.clone(), held: i.held, key: i.key.clone() }).collect();
        let index = Index { plugin, repos: self.added_repos(), checked_ms: self.checked_ms() };
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join("index.json"), serde_json::to_vec(&index).unwrap_or_default());
    }

    /// Plugin `id`'s stored row values.
    pub fn rows(&self, id: &str) -> BTreeMap<String, Value> {
        let s = store::Store::new(self.folder(id).join("store"));
        s.keys(ROWS).into_iter().filter_map(|k| Some((k[ROWS.len()..].to_string(), Value::decode(&s.get(&k)?)?))).collect()
    }

    /// The running plugins exporting `point`, in their order; no instance is made for it.
    pub fn extending(&self, point: &str) -> Vec<String> {
        self.running().into_iter().filter(|i| i.manifest.extends(point)).map(|i| i.manifest.id.clone()).collect()
    }

    /// Running plugin `id`'s own instance.
    pub fn worker(&self, id: &str) -> Option<Worker> {
        let i = self.running().into_iter().find(|i| i.manifest.id == id)?;
        let mut workers = self.workers.lock().expect("not poisoned");
        if let Some(w) = workers.get(id) {
            return Some(w.clone());
        }
        let folder = self.folder(id);
        let mark = self.marks.mark(id, safe::Caller::Worker);
        let component = mark.calling(|| runtime::component(&folder)).map_err(|e| nori_model::alog::info(&format!("plugin {id}: {e:?}"))).ok()?;
        let w = Worker::start(i.manifest.clone(), folder, component, self.shared(), mark).ok()?;
        // Started at once, so its overrides are in effect without waiting for a first call.
        w.post(|_, _| Ok(()));
        workers.insert(id.to_string(), w.clone());
        Some(w)
    }

    /// The overrides of the enabled plugins as text, in their order: applied in turn, a later
    /// plugin's value of a key wins (`nori_settings::overrides`).
    pub fn overrides(&self) -> Vec<(String, String)> {
        self.laying().iter().flat_map(|i| self.overrides_of(i)).collect()
    }

    /// What plugin `i` overrides, as text.
    fn overrides_of(&self, i: &Installed) -> Vec<(String, String)> {
        if i.manifest.overrides.is_empty() {
            return Vec::new();
        }
        let s = store::Store::new(self.folder(&i.manifest.id).join("store"));
        s.keys(host::OVERRIDES)
            .into_iter()
            .filter_map(|k| {
                let key = &k[host::OVERRIDES.len()..];
                if !i.manifest.overrides.iter().any(|o| o == key) {
                    return None;
                }
                Some((key.to_string(), Value::decode(&s.get(&k)?)?.text()))
            })
            .collect()
    }

    /// The enabled plugins' sound stages, with their rows as inputs. Empty, with nothing compiled,
    /// while none is enabled.
    pub fn stages(&self) -> Vec<StageSpec> {
        let wanted: Vec<Installed> = self.laying().into_iter().filter(|i| i.manifest.extends("sound-stage")).collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        let mut made = self.stages.lock().expect("not poisoned");
        let makers = made.get_or_insert_with(|| {
            wanted
                .iter()
                .filter_map(|i| {
                    let mark = self.marks.mark(&i.manifest.id, safe::Caller::Stage);
                    let c = mark.calling(|| runtime::component(&self.folder(&i.manifest.id))).map_err(|e| nori_model::alog::info(&format!("plugin {}: {e:?}", i.manifest.id))).ok()?;
                    Some((i.manifest.id.clone(), Arc::new(stage::Maker { id: i.manifest.id.clone(), component: c, mark })))
                })
                .collect()
        });
        makers
            .iter()
            .filter_map(|(id, maker)| {
                let i = wanted.iter().find(|i| &i.manifest.id == id)?;
                let rows: BTreeMap<String, f64> = self.rows(id).into_iter().filter_map(|(k, v)| Some((k, v.number()?))).collect();
                Some(StageSpec { maker: maker.clone(), slot: i.manifest.stage?, rows: i.manifest.row_numbers(&rows) })
            })
            .collect()
    }

    /// What the enabled plugins lay over the settings.
    pub(crate) fn layer(&self) -> nori_settings::overrides::Layer {
        let preparing = self.preparing().map(nori_player::stage::Prepares);
        nori_settings::overrides::Layer { overrides: self.overrides(), stages: self.stages(), preparing }
    }

    /// What the enabled plugins make of songs as the player reads them; None while none hears songs.
    pub fn preparing(&self) -> Option<Arc<dyn nori_player::stage::Preparing>> {
        self.preparation().map(|p| p as Arc<dyn nori_player::stage::Preparing>)
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
        // A plugin turned on in a safe start ends it: the user has taken the plugins in hand.
        if on {
            self.paused.store(false, Ordering::Relaxed);
        }
        if !on {
            self.workers.lock().expect("not poisoned").remove(id);
            self.lan.forget(id);
        }
        self.forget_made();
        self.side.laid(id);
        self.refresh();
        if starts {
            self.worker(id);
        }
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
        self.forget_made();
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
        self.forget_made();
        self.timers.stop_all(id);
        self.side.laid(id);
        self.refresh();
    }

    /// Why plugins are off at this start, once: the client says so when it shows.
    pub fn take_safe_start(&self) -> Option<SafeStart> {
        self.safe_start.lock().expect("not poisoned").take()
    }

    /// The client has started (its first screen is up, or its player is): a crash from here on is no
    /// startup crash.
    pub fn started(&self) {
        self.marks.starting(false);
    }

    /// The switch over every plugin (`pluginsOff`) changed: what the running plugins hear and want
    /// is read again.
    pub fn switched(&self) {
        self.refresh();
    }

    /// The plugins as a report of a problem lists them (English): whether every plugin is off, and
    /// each one on with what it overrides.
    pub fn report(&self) -> String {
        let mut out = String::new();
        if self.side.all_off() {
            out.push_str("every plugin off (the switch)\n");
        } else if self.paused.load(Ordering::Relaxed) {
            out.push_str("every plugin off for this start (safe mode)\n");
        }
        let installed = self.installed();
        let on: Vec<&Installed> = installed.iter().filter(|i| i.enabled).collect();
        for i in &on {
            let from = i.repo.as_deref().unwrap_or("a file");
            let signed = if i.key.is_some() { "signed" } else { "unsigned" };
            out.push_str(&format!("{} {}, from {from}, {signed}{}\n", i.manifest.id, i.manifest.version, if i.held { ", held" } else { "" }));
            for (key, value) in self.overrides_of(i) {
                out.push_str(&format!("  overrides {key} = {value}\n"));
            }
        }
        let off = installed.len() - on.len();
        if off > 0 {
            out.push_str(&format!("{off} more installed, off\n"));
        }
        out
    }
}
