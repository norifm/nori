//! A plugin's sound stage on the audio path: an instance of its own whose every import traps, so it
//! can never wait on anything. Each block gets a fuel budget; a trap or a block over it bypasses the
//! stage until the next reset (a seek or another song), when a fresh instance is made.

use nori_player::outputs::OutputKind;
use nori_player::stage::{Inputs, SoundStage, StageMaker};
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
    fn make(&self, rate: u32, channels: usize, inputs: &Inputs) -> Option<Box<dyn SoundStage>> {
        let mut s = Stage { id: self.id.clone(), component: self.component.clone(), live: None, rate, channels, inputs: inputs.clone(), bypassed: false };
        s.start();
        Some(Box::new(s))
    }
}

/// An instance and the calls made per block.
struct Live {
    store: Store<()>,
    configure: Func,
    process: Func,
    reset: Func,
    save: Func,
    restore: Func,
}

struct Stage {
    id: String,
    component: Component,
    live: Option<Live>,
    rate: u32,
    channels: usize,
    inputs: Inputs,
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
            configure,
            process: func(&mut store, &instance, "process")?,
            reset: func(&mut store, &instance, "reset")?,
            save: func(&mut store, &instance, "save")?,
            restore: func(&mut store, &instance, "restore")?,
            store,
        })
    }

    fn format(&self) -> sound_stage::Format {
        sound_stage::Format { rate: self.rate, channels: self.channels as u8 }
    }

    /// Its inputs as the WIT has them; its prepared data is copied into the plugin's memory at each
    /// change (a new song, its data coming), never per block.
    fn inputs_record(&self) -> sound_stage::Inputs {
        let i = &self.inputs;
        sound_stage::Inputs {
            rows: i.rows.clone(),
            volume_db: i.around.volume_db,
            route: route(i.around.route),
            gain_db: i.song.gain_db,
            loudness_lufs: i.song.loudness_lufs,
            prepared: i.song.prepared_by(&self.id).map(|d| d.to_vec()),
        }
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
    fn inputs(&mut self, inputs: &Inputs) {
        self.inputs.clone_from(inputs);
        let (format, record) = (self.format(), self.inputs_record());
        self.guarded(|l| call::<_, ()>(&mut l.store, l.configure, (format, &record)));
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
        self.guarded(|l| call::<_, ()>(&mut l.store, l.reset, ()));
    }

    fn save(&mut self, into: &mut Vec<u8>) {
        into.clear();
        self.guarded(|l| {
            let (state,): (WasmList<u8>,) = call(&mut l.store, l.save, ())?;
            into.extend_from_slice(state.as_le_slice(&l.store));
            Ok(())
        });
    }

    fn restore(&mut self, from: &[u8]) {
        self.guarded(|l| call::<_, ()>(&mut l.store, l.restore, (from,)));
    }
}

/// Where the music plays, as the WIT says it. Nori tells no car or cast route apart from the others.
fn route(kind: OutputKind) -> crate::runtime::types::Route {
    use crate::runtime::types::Route;
    match kind {
        OutputKind::Speaker => Route::Speaker,
        OutputKind::Wired => Route::Wired,
        OutputKind::Bluetooth => Route::Bluetooth,
        OutputKind::Usb | OutputKind::UsbAccessory => Route::Usb,
        OutputKind::Line | OutputKind::Other => Route::Other,
    }
}
