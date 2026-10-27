//! wasmtime for plugins: one engine, made when the first plugin is needed; each plugin compiled once by
//! Cranelift and kept compiled beside it; the host interfaces this nori implements.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use sha2::{Digest, Sha256};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::host::Side;
use crate::manifest::{Manifest, Refusal};
use crate::store;

wasmtime::component::bindgen!({
    world: "nori:plugin/plugin",
    path: "../../wit",
});

pub use exports::nori::plugin as points;
pub use nori::plugin::types;

/// The host interfaces linked for a plugin; a plugin importing another can't be installed here.
pub const LINKED: [&str; 6] = ["log", "store", "http", "settings", "rows", "notices"];

/// Fuel for one call that isn't a sound stage's (about a few seconds of work).
pub(crate) const CALL_FUEL: u64 = 20_000_000_000;
/// The most memory a plugin instance may grow to.
const MEMORY_CAP: usize = 256 << 20;

/// The one engine, made at the first plugin compiled or loaded.
pub(crate) fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut c = Config::new();
        c.consume_fuel(true);
        c.cranelift_opt_level(wasmtime::OptLevel::Speed);
        Engine::new(&c).expect("the plugin engine's settings are valid")
    })
}

/// The plugin at `dir` (its `plugin.wasm`), compiled once and kept as `compiled.cwasm` under a key of
/// the wasm's hash and this engine's.
pub(crate) fn component(dir: &Path) -> Result<Component, Refusal> {
    let wasm = std::fs::read(dir.join("plugin.wasm")).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    let mut hash = Sha256::new();
    hash.update(&wasm);
    std::hash::Hash::hash(&engine().precompile_compatibility_hash(), &mut HashInto(&mut hash));
    let key: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let (compiled, key_file) = (dir.join("compiled.cwasm"), dir.join("compiled.key"));
    if std::fs::read_to_string(&key_file).is_ok_and(|k| k == key) {
        // SAFETY: written by this host's `precompile_component` for the same engine (the key says so).
        if let Ok(c) = unsafe { Component::deserialize_file(engine(), &compiled) } {
            return Ok(c);
        }
    }
    let bytes = engine().precompile_component(&wasm).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    let _ = std::fs::write(&compiled, &bytes).and_then(|_| std::fs::write(&key_file, &key));
    // SAFETY: just made by this engine.
    unsafe { Component::deserialize(engine(), &bytes) }.map_err(|e| Refusal::Unreadable(e.to_string()))
}

struct HashInto<'a>(&'a mut Sha256);

impl std::hash::Hasher for HashInto<'_> {
    fn finish(&self) -> u64 {
        0
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

/// The nori interfaces whose calls `component` imports that this host doesn't link.
pub(crate) fn missing(component: &Component) -> Vec<String> {
    use wasmtime::component::types::ComponentItem;
    let ty = component.component_type();
    ty.imports(engine())
        .filter_map(|(name, item)| {
            let iface = name.strip_prefix("nori:plugin/")?.split('@').next()?;
            let calls = match item.ty {
                ComponentItem::ComponentInstance(i) => i.exports(engine()).any(|(_, e)| matches!(e.ty, ComponentItem::ComponentFunc(_))),
                _ => true,
            };
            (calls && !LINKED.contains(&iface)).then(|| iface.to_string())
        })
        .collect()
}

/// What a plugin's calls into the host reach.
pub struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    pub(crate) id: String,
    pub(crate) manifest: Arc<Manifest>,
    pub(crate) store: store::Store,
    pub(crate) side: Arc<dyn Side>,
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

/// A store for a plugin instance whose imports reach the host, with its memory capped.
pub(crate) fn store(manifest: Arc<Manifest>, dir: &Path, side: Arc<dyn Side>) -> Store<Host> {
    let host = Host {
        wasi: WasiCtx::builder().build(),
        table: ResourceTable::new(),
        limits: StoreLimitsBuilder::new().memory_size(MEMORY_CAP).instances(16).build(),
        id: manifest.id.clone(),
        store: store::Store::new(dir.join("store")),
        manifest,
        side,
    };
    let mut s = Store::new(engine(), host);
    s.limiter(|h| &mut h.limits);
    s
}

/// The linker for a plugin's own instance: WASI, and the host interfaces in [`LINKED`].
pub(crate) fn linker() -> Result<Linker<Host>, Refusal> {
    let mut l = Linker::new(engine());
    let err = |e: wasmtime::Error| Refusal::Unreadable(e.to_string());
    wasmtime_wasi::p2::add_to_linker_sync(&mut l).map_err(err)?;
    nori::plugin::types::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::log::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::store::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::http::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::settings::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::rows::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::notices::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    Ok(l)
}
