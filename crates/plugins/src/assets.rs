//! A plugin's assets (`assets`): the large files its manifest names, fetched by nori through the open
//! profile's transport in parts (each a range request, written as it comes and hashed), on an
//! unmetered network unless the user lets this plugin use mobile data. Each is kept in the plugin's
//! `assets/` as its sha256, so a changed declaration is fetched again and an update keeps the rest;
//! removed with the plugin. The client shows each one's state as a line on the plugin's page.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use nori_net::transport::{Exchange, Network, Transport};
use sha2::{Digest, Sha256};

use crate::manifest::AssetSpec;
use crate::runtime::nori::plugin::assets as a;
use crate::runtime::points::hooks::Event;
use crate::runtime::Host;
use crate::{store, Plugins};

/// Bytes asked for in one request.
const PART: u64 = 4 << 20;

/// Where an asset stands, as its line on the plugin's page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AssetState {
    Missing,
    /// Asked for on mobile data, which this plugin may not use.
    WaitingForWifi,
    Fetching { done: u64, total: u64 },
    Ready,
    Failed,
}

/// The assets being fetched or waiting, by plugin and name; one not here is ready or missing.
#[derive(Default)]
pub(crate) struct Fetches(std::sync::Mutex<HashMap<(String, String), AssetState>>);

/// Where `spec`'s file is kept in plugin folder `dir`.
fn kept(dir: &Path, spec: &AssetSpec) -> PathBuf {
    dir.join("assets").join(&spec.sha256)
}

/// Lets go of the files in `dir`'s `assets/` that `specs` no longer name (after an update).
pub(crate) fn prune(dir: &Path, specs: &[AssetSpec]) {
    let named: HashSet<&str> = specs.iter().map(|s| s.sha256.as_str()).collect();
    let Ok(entries) = std::fs::read_dir(dir.join("assets")) else { return };
    for e in entries.flatten() {
        if !named.contains(e.file_name().to_string_lossy().as_ref()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Where mobile data is allowed for its assets, in a plugin's store.
const MOBILE: &str = "nori/assets-mobile";

/// `spec` fetched into `to` through `transport`, `said` told of each part.
fn fetch(transport: &dyn Transport, spec: &AssetSpec, to: &Path, said: &dyn Fn(u64)) -> Result<(), String> {
    std::fs::create_dir_all(to.parent().expect("in assets/")).map_err(|e| e.to_string())?;
    let part = to.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut done = 0u64;
    while done < spec.size {
        let end = (done + PART).min(spec.size) - 1;
        let headers = HashMap::from([("Range".to_string(), format!("bytes={done}-{end}"))]);
        let r = nori_tasks::block_on(transport.send(Exchange { url: spec.url.clone(), headers, json: None, form: None, timeout_ms: 0 })).map_err(|e| e.to_string())?;
        let whole = r.status == 200 && done == 0 && r.body.len() as u64 == spec.size;
        if !(whole || (r.status == 206 && r.body.len() as u64 == end + 1 - done)) {
            return Err(format!("{} answered {} with {} bytes", spec.url, r.status, r.body.len()));
        }
        file.write_all(&r.body).map_err(|e| e.to_string())?;
        hash.update(&r.body);
        done += r.body.len() as u64;
        said(done);
    }
    file.sync_all().map_err(|e| e.to_string())?;
    let sha: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if sha != spec.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{} is not the file declared", spec.url));
    }
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

impl Plugins {
    fn asset_spec(&self, id: &str, name: &str) -> Option<AssetSpec> {
        self.installed().into_iter().find(|i| i.manifest.id == id)?.manifest.assets.iter().find(|a| a.name == name).cloned()
    }

    fn asset_state(&self, id: &str, spec: &AssetSpec) -> AssetState {
        if kept(&self.folder(id), spec).is_file() {
            return AssetState::Ready;
        }
        self.fetches.0.lock().expect("not poisoned").get(&(id.to_string(), spec.name.clone())).cloned().unwrap_or(AssetState::Missing)
    }

    fn asset_is(&self, id: &str, name: &str, state: AssetState) {
        let key = (id.to_string(), name.to_string());
        match &state {
            AssetState::Ready | AssetState::Missing => self.fetches.0.lock().expect("not poisoned").remove(&key),
            s => self.fetches.0.lock().expect("not poisoned").insert(key, s.clone()),
        };
        self.side.asset(id, name, state);
    }

    /// Fetches plugin `id`'s asset `name` unless it is in or on its way; tells its hooks once it is in.
    pub(crate) fn fetch_asset(&self, id: &str, name: &str) {
        let Some(spec) = self.asset_spec(id, name) else { return };
        match self.asset_state(id, &spec) {
            AssetState::Ready => return self.tell_one(id, Event::AssetReady(name.to_string())),
            AssetState::Fetching { .. } => return,
            _ => {}
        }
        let Some(transport) = self.transport.lock().expect("not poisoned").clone() else { return self.asset_is(id, name, AssetState::Failed) };
        let mobile = store::Store::new(self.folder(id).join("store")).get(MOBILE).is_some();
        if transport.network() == Network::Metered && !mobile {
            return self.asset_is(id, name, AssetState::WaitingForWifi);
        }
        self.asset_is(id, name, AssetState::Fetching { done: 0, total: spec.size });
        let (me, to) = (self.me.clone(), kept(&self.folder(id), &spec));
        let id = id.to_string();
        let failed = id.clone();
        let run = move || {
            let said = |done| {
                if let Some(p) = me.upgrade() {
                    p.asset_is(&id, &spec.name, AssetState::Fetching { done, total: spec.size });
                }
            };
            let fetched = fetch(&*transport, &spec, &to, &said);
            let Some(p) = me.upgrade() else { return };
            match fetched {
                Ok(()) => {
                    p.asset_is(&id, &spec.name, AssetState::Ready);
                    p.tell_one(&id, Event::AssetReady(spec.name.clone()));
                }
                Err(e) => {
                    nori_model::alog::info(&format!("plugin {id}: asset {}: {e}", spec.name));
                    p.asset_is(&id, &spec.name, AssetState::Failed);
                }
            }
        };
        if std::thread::Builder::new().name("nori-plugin-asset".into()).spawn(run).is_err() {
            self.asset_is(&failed, name, AssetState::Failed);
        }
    }

    /// Fetches the assets that wait for Wi-Fi again (the network or the profile changed).
    pub(crate) fn fetch_waiting(&self) {
        let waiting: Vec<(String, String)> = self.fetches.0.lock().expect("not poisoned").iter().filter(|(_, s)| **s == AssetState::WaitingForWifi).map(|(k, _)| k.clone()).collect();
        for (id, name) in waiting {
            self.fetch_asset(&id, &name);
        }
    }
}

/// An asset as its plugin's page lists it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AssetLine {
    pub name: String,
    /// Bytes.
    pub size: u64,
    pub state: AssetState,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// Plugin `id`'s assets and where each stands, for its page.
    pub fn assets(&self, id: String) -> Vec<AssetLine> {
        let Some(i) = self.installed().into_iter().find(|i| i.manifest.id == id) else { return Vec::new() };
        i.manifest.assets.iter().map(|s| AssetLine { name: s.name.clone(), size: s.size, state: self.asset_state(&id, s) }).collect()
    }

    /// The user lets plugin `id` fetch its assets on mobile data; those waiting are fetched now.
    pub fn allow_mobile_assets(&self, id: String) {
        store::Store::new(self.folder(&id).join("store")).set(MOBILE, &[1]);
        self.fetch_waiting();
    }
}

impl a::Host for Host {
    fn state(&mut self, name: String) -> a::AssetState {
        let state = self.plugins().and_then(|p| Some(p.asset_state(&self.id, &p.asset_spec(&self.id, &name)?)));
        match state {
            Some(AssetState::Ready) => a::AssetState::Ready,
            Some(AssetState::Fetching { .. }) => a::AssetState::Fetching,
            Some(AssetState::Failed) => a::AssetState::Failed,
            Some(AssetState::Missing | AssetState::WaitingForWifi) | None => a::AssetState::Missing,
        }
    }

    fn fetch(&mut self, name: String) {
        if let Some(p) = self.plugins() {
            p.fetch_asset(&self.id, &name);
        }
    }

    fn read(&mut self, name: String) -> Option<Vec<u8>> {
        let spec = self.manifest.assets.iter().find(|a| a.name == name)?;
        std::fs::read(kept(&self.dir, spec)).ok()
    }
}

