//! The plugin host. Plugins are WebAssembly components (`wit/`, the `nori:plugin` package), installed
//! under one folder, each in its own: its manifest, its wasm, its strings, its store and its compiled
//! code. With none enabled nothing here runs: no engine is made, no file is read past the index.

mod host;
mod manifest;
mod package;
pub(crate) mod runtime;
mod stage;
mod store;
mod worker;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use host::{Side, Value};
pub use manifest::{Manifest, Refusal, RowKind, RowSpec, Slot, API, POINTS};
pub use runtime::{Host, LINKED};
pub use worker::Worker;

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
}

/// A plugin's sound stage maker, by its id.
type Made = (String, Arc<stage::Maker>);

/// `index.toml`: the plugins in the user's order, and which are on.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    plugin: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexEntry {
    id: String,
    enabled: bool,
}

pub struct Plugins {
    dir: PathBuf,
    side: Arc<dyn Side>,
    installed: Mutex<Vec<Installed>>,
    /// The enabled sound stages' makers, made at first need and kept while the set is the same.
    stages: Mutex<Option<Vec<Made>>>,
    /// The enabled plugins' own instances, each made at its first call.
    workers: Mutex<BTreeMap<String, Worker>>,
}

impl Plugins {
    /// The plugins installed under `dir`, as its index lists them.
    pub fn open(dir: PathBuf, side: Arc<dyn Side>) -> Plugins {
        let index: Index = std::fs::read_to_string(dir.join("index.toml")).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        let installed = index
            .plugin
            .iter()
            .filter_map(|e| {
                let wasm = std::fs::read(dir.join(&e.id).join("plugin.wasm")).ok()?;
                let manifest = Manifest::of_wasm(&wasm).ok().filter(|m| m.id == e.id)?;
                Some(Installed { manifest: Arc::new(manifest), enabled: e.enabled })
            })
            .collect();
        Plugins { dir, side, installed: Mutex::new(installed), stages: Mutex::new(None), workers: Mutex::new(BTreeMap::new()) }
    }

    pub fn installed(&self) -> Vec<Installed> {
        self.installed.lock().expect("not poisoned").clone()
    }

    fn folder(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    /// Installs (or updates) the `.noriplugin` package or plugin folder at `from`, disabled.
    pub fn install(&self, from: &Path) -> Result<Arc<Manifest>, Refusal> {
        let manifest = Arc::new(package::install(from, &self.dir)?);
        let mut list = self.installed.lock().expect("not poisoned");
        let enabled = list.iter().find(|i| i.manifest.id == manifest.id).is_some_and(|i| i.enabled);
        list.retain(|i| i.manifest.id != manifest.id);
        list.push(Installed { manifest: manifest.clone(), enabled });
        self.keep(&list);
        drop(list);
        *self.stages.lock().expect("not poisoned") = None;
        Ok(manifest)
    }

    pub fn remove(&self, id: &str) {
        let mut list = self.installed.lock().expect("not poisoned");
        list.retain(|i| i.manifest.id != id);
        self.keep(&list);
        drop(list);
        self.workers.lock().expect("not poisoned").remove(id);
        let _ = std::fs::remove_dir_all(self.folder(id));
        *self.stages.lock().expect("not poisoned") = None;
        self.side.overrides_changed(id);
    }

    pub fn set_enabled(&self, id: &str, on: bool) {
        let mut list = self.installed.lock().expect("not poisoned");
        let Some(i) = list.iter_mut().find(|i| i.manifest.id == id) else { return };
        i.enabled = on;
        self.keep(&list);
        drop(list);
        if !on {
            self.workers.lock().expect("not poisoned").remove(id);
        }
        *self.stages.lock().expect("not poisoned") = None;
        self.side.overrides_changed(id);
    }

    /// Moves plugin `id` to `index` in the order they apply in.
    pub fn move_to(&self, id: &str, index: usize) {
        let mut list = self.installed.lock().expect("not poisoned");
        let Some(at) = list.iter().position(|i| i.manifest.id == id) else { return };
        let p = list.remove(at);
        let index = index.min(list.len());
        list.insert(index, p);
        self.keep(&list);
        drop(list);
        *self.stages.lock().expect("not poisoned") = None;
        self.side.overrides_changed(id);
    }

    fn keep(&self, list: &[Installed]) {
        let index = Index { plugin: list.iter().map(|i| IndexEntry { id: i.manifest.id.clone(), enabled: i.enabled }).collect() };
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join("index.toml"), toml::to_string(&index).unwrap_or_default());
    }

    /// Plugin `id`'s stored row values.
    pub fn rows(&self, id: &str) -> BTreeMap<String, Value> {
        let s = store::Store::new(self.folder(id).join("store"));
        s.keys(ROWS).into_iter().filter_map(|k| Some((k[ROWS.len()..].to_string(), Value::decode(&s.get(&k)?)?))).collect()
    }

    /// Sets one of plugin `id`'s rows; a plugin with a lifecycle hears of it.
    pub fn set_row(&self, id: &str, row: &str, value: Value) {
        store::Store::new(self.folder(id).join("store")).set(&format!("{ROWS}{row}"), &value.encode());
        if self.installed().iter().any(|i| i.enabled && i.manifest.id == id && i.manifest.extends("lifecycle")) {
            if let Some(w) = self.worker(id) {
                let (row, value) = (row.to_string(), value.into());
                w.post(move |store, p| p.nori_plugin_lifecycle().call_changed(store, &row, &value));
            }
        }
    }

    /// Enabled plugin `id`'s own instance, started (and told so, with a lifecycle) at the first call.
    pub fn worker(&self, id: &str) -> Option<Worker> {
        let mut workers = self.workers.lock().expect("not poisoned");
        if let Some(w) = workers.get(id) {
            return Some(w.clone());
        }
        let i = self.installed().into_iter().find(|i| i.enabled && i.manifest.id == id)?;
        let folder = self.folder(id);
        let component = runtime::component(&folder).map_err(|e| nori_model::alog::info(&format!("plugin {id}: {e:?}"))).ok()?;
        let w = Worker::start(i.manifest.clone(), folder, component, self.side.clone()).ok()?;
        if i.manifest.extends("lifecycle") {
            w.post(|store, p| p.nori_plugin_lifecycle().call_start(store));
        }
        workers.insert(id.to_string(), w.clone());
        Some(w)
    }

    /// The overrides of the enabled plugins, in order: a later plugin's value of a key wins.
    pub fn overrides(&self) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        for i in self.installed().iter().filter(|i| i.enabled && !i.manifest.overrides.is_empty()) {
            let s = store::Store::new(self.folder(&i.manifest.id).join("store"));
            for k in s.keys(host::OVERRIDES) {
                let key = &k[host::OVERRIDES.len()..];
                if let (true, Some(v)) = (i.manifest.overrides.iter().any(|o| o == key), s.get(&k).and_then(|b| Value::decode(&b))) {
                    out.insert(key.to_string(), v);
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
}

/// Where a plugin's row values are kept in its store.
const ROWS: &str = "nori/row/";
