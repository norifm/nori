//! A plugin's own instance, on a thread of its own: its calls may wait (its HTTP goes through the
//! core's transport and blocks there), so no executor of the core's ever waits on a plugin. The thread
//! ends when the last handle to it goes.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;

use wasmtime::component::Component;
use wasmtime::Store;

use crate::host::Side;
use crate::manifest::{Manifest, Refusal};
use crate::runtime::{self, Host, Plugin};

/// A running instance: its store and its exports.
pub struct Running {
    pub store: Store<Host>,
    pub plugin: Plugin,
}

type Job = Box<dyn FnOnce(&mut Option<Running>) + Send>;

/// The handle the core calls a plugin through.
#[derive(Clone)]
pub struct Worker {
    jobs: mpsc::Sender<Job>,
}

impl Worker {
    pub(crate) fn start(manifest: Arc<Manifest>, dir: PathBuf, component: Component, side: Arc<dyn Side>) -> Result<Worker, Refusal> {
        let linker = runtime::linker()?;
        let (jobs, rx) = mpsc::channel::<Job>();
        let name = format!("plugin {}", manifest.id);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let make = || -> wasmtime::Result<Running> {
                    let mut store = runtime::store(manifest.clone(), &dir, side.clone());
                    store.set_fuel(runtime::CALL_FUEL)?;
                    let plugin = Plugin::instantiate(&mut store, &component, &linker)?;
                    Ok(Running { store, plugin })
                };
                let mut running: Option<Running> = None;
                for job in rx {
                    if running.is_none() {
                        running = make().map_err(|e| nori_model::alog::info(&format!("plugin {}: can't start: {e}", manifest.id))).ok();
                    }
                    if let Some(r) = running.as_mut() {
                        let _ = r.store.set_fuel(runtime::CALL_FUEL);
                    }
                    job(&mut running);
                }
            })
            .map_err(|e| Refusal::Unreadable(e.to_string()))?;
        Ok(Worker { jobs })
    }

    /// Runs `f` on the plugin's thread without waiting for it.
    pub fn post(&self, f: impl FnOnce(&mut Store<Host>, &Plugin) -> wasmtime::Result<()> + Send + 'static) {
        let job: Job = Box::new(move |running| {
            let Some(r) = running.as_mut() else { return };
            if let Err(e) = f(&mut r.store, &r.plugin) {
                nori_model::alog::info(&format!("plugin {}: a call failed: {e}", r.store.data().id));
                *running = None;
            }
        });
        let _ = self.jobs.send(job);
    }

    /// Runs `f` on the plugin's thread; None when it isn't running or its call failed (a trap, its
    /// budget spent), after which the next call starts a fresh instance.
    pub async fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut Store<Host>, &Plugin) -> wasmtime::Result<R> + Send + 'static) -> Option<R> {
        let (tx, rx) = futures_channel::oneshot::channel();
        let job: Job = Box::new(move |running| {
            let Some(r) = running.as_mut() else { return };
            match f(&mut r.store, &r.plugin) {
                Ok(v) => {
                    let _ = tx.send(v);
                }
                Err(e) => {
                    nori_model::alog::info(&format!("plugin {}: a call failed: {e}", r.store.data().id));
                    *running = None;
                }
            }
        });
        self.jobs.send(job).ok()?;
        rx.await.ok()
    }
}
