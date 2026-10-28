//! A plugin's sound stage on the audio path: an instance of its own whose every import traps, so it
//! can never wait on anything. Each block gets a fuel budget; a trap or a block over it bypasses the
//! stage until the next reset (a seek or another song), when a fresh instance is made.

use nori_player::stage::{SoundStage, StageMaker};
use wasmtime::component::{Component, Func, Instance, Linker, TypedFunc, WasmList};
use wasmtime::Store;

use crate::runtime::{engine, points::sound_stage};

/// Fuel for one block (8192 frames or fewer): a few ms of work, well under the block's play time.
const BLOCK_FUEL: u64 = 400_000_000;

/// Makes the stage of one plugin.
pub struct Maker {
    pub(crate) id: String,
    pub(crate) component: Component,
}

impl StageMaker for Maker {
    fn make(&self, rate: u32, channels: usize, inputs: &[f64]) -> Option<Box<dyn SoundStage>> {
        let mut s = Stage { id: self.id.clone(), component: self.component.clone(), live: None, rate, channels, inputs: inputs.to_vec(), bypassed: false };
        s.start();
        Some(Box::new(s))
    }
}

/// An instance and the calls made per block.
struct Live {
    store: Store<()>,
    instance: Instance,
    process: Func,
    save: Func,
    restore: Func,
}

struct Stage {
    id: String,
    component: Component,
    live: Option<Live>,
    rate: u32,
    channels: usize,
    inputs: Vec<f64>,
    /// It trapped or ran over: samples pass as they are until the next reset.
    bypassed: bool,
}

/// `name` of the sound-stage export.
fn func(store: &mut Store<()>, instance: &Instance, name: &str) -> wasmtime::Result<Func> {
    let iface = instance.get_export_index(&mut *store, None, "nori:plugin/sound-stage@1.0.0").ok_or_else(|| wasmtime::Error::msg("no sound-stage export"))?;
    let f = instance.get_export_index(&mut *store, Some(&iface), name).ok_or_else(|| wasmtime::Error::msg(format!("no {name}")))?;
    instance.get_func(&mut *store, f).ok_or_else(|| wasmtime::Error::msg(format!("{name} is no function")))
}

/// Calls `f` with `params`; a list it answers is read from the plugin's memory until the next call.
fn call<P, R>(store: &mut Store<()>, f: Func, params: P) -> wasmtime::Result<R>
where
    P: wasmtime::component::ComponentNamedList + wasmtime::component::Lower,
    R: wasmtime::component::ComponentNamedList + wasmtime::component::Lift,
{
    let typed: TypedFunc<P, R> = f.typed(&*store)?;
    typed.call(&mut *store, params)
}

impl Stage {
    /// A fresh instance, configured; bypassed when one can't be made.
    fn start(&mut self) {
        self.live = self.instantiate().map_err(|e| nori_model::alog::info(&format!("plugin {}: its sound stage can't start: {e}", self.id))).ok();
        self.bypassed = self.live.is_none();
    }

    fn instantiate(&self) -> wasmtime::Result<Live> {
        let mut store = Store::new(engine(), ());
        store.set_fuel(BLOCK_FUEL)?;
        let mut linker: Linker<()> = Linker::new(engine());
        linker.define_unknown_imports_as_traps(&self.component)?;
        let instance = linker.instantiate(&mut store, &self.component)?;
        let configure = func(&mut store, &instance, "configure")?;
        call::<_, ()>(&mut store, configure, (self.format(), &self.inputs_record()))?;
        Ok(Live {
            process: func(&mut store, &instance, "process")?,
            save: func(&mut store, &instance, "save")?,
            restore: func(&mut store, &instance, "restore")?,
            store,
            instance,
        })
    }

    fn format(&self) -> sound_stage::Format {
        sound_stage::Format { rate: self.rate, channels: self.channels as u8 }
    }

    fn inputs_record(&self) -> sound_stage::Inputs {
        sound_stage::Inputs { rows: self.inputs.clone(), volume_db: 0.0, route: crate::runtime::types::Route::Other, gain_db: None, loudness_lufs: None, prepared: None }
    }

    /// Runs `call` with a block's fuel; a trap or running out bypasses the stage.
    fn guarded<R>(&mut self, call: impl FnOnce(&mut Live) -> wasmtime::Result<R>) -> Option<R> {
        if self.bypassed {
            return None;
        }
        let live = self.live.as_mut()?;
        let r = live.store.set_fuel(BLOCK_FUEL).and_then(|_| call(live));
        match r {
            Ok(r) => Some(r),
            Err(e) => {
                nori_model::alog::info(&format!("plugin {}: its sound stage stopped for this song: {e}", self.id));
                self.bypassed = true;
                None
            }
        }
    }
}

impl SoundStage for Stage {
    fn inputs(&mut self, inputs: &[f64]) {
        if self.inputs == inputs {
            return;
        }
        self.inputs.clear();
        self.inputs.extend_from_slice(inputs);
        let (format, record) = (self.format(), self.inputs_record());
        self.guarded(|l| {
            let configure = func(&mut l.store, &l.instance, "configure")?;
            call::<_, ()>(&mut l.store, configure, (format, &record))
        });
    }

    fn process(&mut self, planar: &mut [f32]) {
        self.guarded(|l| {
            let (out,): (WasmList<f32>,) = call(&mut l.store, l.process, (&*planar,))?;
            if out.len() != planar.len() {
                return Err(wasmtime::Error::msg("handed back another number of samples"));
            }
            for (d, s) in planar.iter_mut().zip(out.iter(&mut l.store)?) {
                *d = s?;
            }
            Ok(())
        });
    }

    fn reset(&mut self) {
        // After a trap an instance can't be entered again: a fresh one.
        if self.bypassed {
            self.start();
            return;
        }
        self.guarded(|l| {
            let reset = func(&mut l.store, &l.instance, "reset")?;
            call::<_, ()>(&mut l.store, reset, ())
        });
    }

    /// Its inputs' count and values, then the plugin's own state.
    fn save(&mut self, into: &mut Vec<u8>) {
        into.clear();
        into.extend_from_slice(&(self.inputs.len() as u32).to_le_bytes());
        for v in &self.inputs {
            into.extend_from_slice(&v.to_le_bytes());
        }
        self.guarded(|l| {
            let (state,): (WasmList<u8>,) = call(&mut l.store, l.save, ())?;
            into.extend_from_slice(state.as_le_slice(&l.store));
            Ok(())
        });
    }

    fn restore(&mut self, from: &[u8]) {
        let Some((inputs, state)) = saved(from) else { return };
        // The inputs as they were then; the same ones (the usual case) are compared in place.
        let same = inputs.len() == self.inputs.len() * 8 && inputs.as_chunks::<8>().0.iter().zip(&self.inputs).all(|(b, v)| f64::from_le_bytes(*b) == *v);
        if !same {
            let was: Vec<f64> = inputs.as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect();
            self.inputs(&was);
        }
        self.guarded(|l| call::<_, ()>(&mut l.store, l.restore, (state,)));
    }
}

/// What [`Stage::save`] wrote: its inputs' bytes, and the plugin's state.
fn saved(from: &[u8]) -> Option<(&[u8], &[u8])> {
    let n = u32::from_le_bytes(from.get(..4)?.try_into().ok()?) as usize;
    Some((from.get(4..4 + n * 8)?, &from[4 + n * 8..]))
}
