//! wasmtime for plugins: one engine, made when the first plugin is needed; each plugin compiled once by
//! Cranelift and kept compiled beside it; the host interfaces this nori implements.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use sha2::{Digest, Sha256};
use wasmtime::component::{Component, ComponentType, HasSelf, Lift, Linker, Lower};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::host::Side;
use crate::manifest::{Manifest, Refusal};
use crate::store;

wasmtime::component::bindgen!({
    world: "nori:plugin/plugin",
    path: "../../wit",
    with: {
        "nori:plugin/sockets.tcp": crate::sockets::Tcp,
        "nori:plugin/sockets.udp": crate::sockets::Udp,
        "nori:plugin/bluetooth.rfcomm": crate::bluetooth::Rfcomm,
        "nori:plugin/bluetooth.gatt": crate::bluetooth::Gatt,
    },
});

pub use exports::nori::plugin as points;
pub use nori::plugin::types;

/// The host interfaces linked for a plugin; a plugin importing another can't be installed here.
pub const LINKED: [&str; 21] = ["log", "store", "http", "settings", "rows", "notices", "sockets", "streams", "bluetooth", "discovery", "lan-serve", "audio-out", "media", "cast", "session", "subsonic", "timers", "sign-in", "wake", "cache", "assets"];

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

/// The plugin at `dir` (its `plugin.wasm`), compiled once and kept as `compiled.cwasm` for this
/// engine. Installing replaces the folder, compiled code and all, so a load reads only the compiled
/// code and the engine it was made by, never the wasm.
pub(crate) fn component(dir: &Path) -> Result<Component, Refusal> {
    let mut hash = Sha256::new();
    std::hash::Hash::hash(&engine().precompile_compatibility_hash(), &mut HashInto(&mut hash));
    let key: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let (compiled, key_file) = (dir.join("compiled.cwasm"), dir.join("compiled.key"));
    if std::fs::read_to_string(&key_file).is_ok_and(|k| k == key) {
        // SAFETY: written by this host's `precompile_component` for an engine like this one (the key says so).
        if let Ok(c) = unsafe { Component::deserialize_file(engine(), &compiled) } {
            return Ok(c);
        }
    }
    let wasm = std::fs::read(dir.join("plugin.wasm")).map_err(|e| Refusal::Unreadable(e.to_string()))?;
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
    limits: StoreLimits,
    pub(crate) id: String,
    pub(crate) manifest: Arc<Manifest>,
    pub(crate) store: store::Store,
    pub(crate) side: Arc<dyn Side>,
    /// The open profile's transport, shared with the plugin host.
    pub(crate) transport: crate::Transported,
    /// Its requests on their way.
    pub(crate) started: crate::host::Started,
    /// Its sockets.
    pub(crate) table: wasmtime::component::ResourceTable,
    /// What the last `source` call said of the URL it returned ([`crate::Encrypted`]).
    pub(crate) encrypted: Option<crate::Encrypted>,
    /// The platform's Bluetooth, shared with the plugin host.
    pub(crate) radio: Arc<crate::bluetooth::Radio>,
    /// What the plugins hold on the LAN.
    pub(crate) lan: Arc<crate::lan::Lan>,
    /// Its folder.
    pub(crate) dir: std::path::PathBuf,
    pub(crate) cache: crate::cache::Cache,
    /// The host it runs in, for what outlives a call (timers, assets).
    pub(crate) plugins: std::sync::Weak<crate::Plugins>,
}

/// An instance gone (its plugin stopped or turned off, or a call failed) lets go of what it held on
/// the LAN: its browsing, endpoints, audio and cast.
impl Drop for Host {
    fn drop(&mut self) {
        self.lan.forget(&self.id);
    }
}

impl Host {
    pub(crate) fn plugins(&self) -> Option<Arc<crate::Plugins>> {
        self.plugins.upgrade()
    }
}

/// What every instance's imports reach beyond its own folder, shared with the plugin host.
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) side: Arc<dyn Side>,
    pub(crate) transport: crate::Transported,
    pub(crate) radio: Arc<crate::bluetooth::Radio>,
    pub(crate) lan: Arc<crate::lan::Lan>,
    pub(crate) plugins: std::sync::Weak<crate::Plugins>,
}

/// A store for a plugin instance whose imports reach the host, with its memory capped.
pub(crate) fn store(manifest: Arc<Manifest>, dir: &Path, shared: Shared) -> Store<Host> {
    let Shared { side, transport, radio, lan, plugins } = shared;
    let host = Host {
        limits: StoreLimitsBuilder::new().memory_size(MEMORY_CAP).instances(16).build(),
        id: manifest.id.clone(),
        store: store::Store::new(dir.join("store")),
        cache: crate::cache::Cache::new(dir, manifest.cache_bytes),
        manifest,
        side,
        transport,
        started: Default::default(),
        table: Default::default(),
        encrypted: None,
        radio,
        lan,
        dir: dir.to_path_buf(),
        plugins,
    };
    let mut s = Store::new(engine(), host);
    s.limiter(|h| &mut h.limits);
    s
}

/// A wall clock's time, as WASI says it.
#[derive(ComponentType, Lift, Lower, Clone, Copy)]
#[component(record)]
struct Datetime {
    seconds: u64,
    nanoseconds: u32,
}

fn random(n: u64) -> Vec<u8> {
    use std::io::Read;
    let mut b = vec![0; n.min(1 << 20) as usize];
    // The system's randomness; a plugin seeding a HashMap needs it, never secrets of nori's.
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    b
}

fn u64_random() -> u64 {
    u64::from_le_bytes(random(8).try_into().expect("8 bytes"))
}

/// What a Rust plugin's std takes of WASI, at the versions `component` imports: randomness, the
/// clocks, an empty environment. Its other WASI imports (files, sockets, stdio) trap: a plugin speaks
/// through nori's `log` and its other imports.
fn wasi(l: &mut Linker<Host>, component: &Component) -> wasmtime::Result<()> {
    let names: Vec<String> = component.component_type().imports(engine()).map(|(n, _)| n.to_string()).filter(|n| n.starts_with("wasi:")).collect();
    for name in names {
        let iface = name.split('@').next().unwrap_or_default();
        let mut i = l.instance(&name)?;
        match iface {
            "wasi:random/random" => {
                i.func_wrap("get-random-bytes", |_, (n,): (u64,)| Ok((random(n),)))?;
                i.func_wrap("get-random-u64", |_, ()| Ok((u64_random(),)))?;
            }
            "wasi:random/insecure" => {
                i.func_wrap("get-insecure-random-bytes", |_, (n,): (u64,)| Ok((random(n),)))?;
                i.func_wrap("get-insecure-random-u64", |_, ()| Ok((u64_random(),)))?;
            }
            "wasi:random/insecure-seed" => {
                i.func_wrap("insecure-seed", |_, ()| Ok(((u64_random(), u64_random()),)))?;
            }
            "wasi:clocks/monotonic-clock" => {
                i.func_wrap("now", |_, ()| Ok((nori_time::device_us().max(0) as u64 * 1000,)))?;
                i.func_wrap("resolution", |_, ()| Ok((1_000u64,)))?;
            }
            "wasi:clocks/wall-clock" => {
                i.func_wrap("now", |_, ()| {
                    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                    Ok((Datetime { seconds: d.as_secs(), nanoseconds: d.subsec_nanos() },))
                })?;
                i.func_wrap("resolution", |_, ()| Ok((Datetime { seconds: 0, nanoseconds: 1_000 },)))?;
            }
            "wasi:cli/environment" => {
                i.func_wrap("get-environment", |_, ()| Ok((Vec::<(String, String)>::new(),)))?;
                i.func_wrap("get-arguments", |_, ()| Ok((Vec::<String>::new(),)))?;
                i.func_wrap("initial-cwd", |_, ()| Ok((None::<String>,)))?;
            }
            _ => {}
        }
    }
    l.define_unknown_imports_as_traps(component)
}

/// The linker for `component`'s own instance: the WASI its std needs, and the host interfaces in
/// [`LINKED`].
pub(crate) fn linker(component: &Component) -> Result<Linker<Host>, Refusal> {
    let mut l = Linker::new(engine());
    let err = |e: wasmtime::Error| Refusal::Unreadable(e.to_string());
    nori::plugin::types::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::log::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::store::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::http::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::settings::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::rows::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::notices::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::sockets::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::streams::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::bluetooth::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::discovery::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::lan_serve::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::audio_out::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::media::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::cast::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::session::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::subsonic::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::timers::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::sign_in::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::wake::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::cache::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    nori::plugin::assets::add_to_linker::<Host, HasSelf<Host>>(&mut l, |h| h).map_err(err)?;
    wasi(&mut l, component).map_err(err)?;
    Ok(l)
}
