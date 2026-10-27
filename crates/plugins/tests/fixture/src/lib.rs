//! Each extension point the host's tests check, in its plainest form.

use nori_plugin_sdk::http::{Method, Request};
use nori_plugin_sdk::sound_stage::{Format, Inputs};
use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::{settings, state, store, Lifecycle, Manifest, Row, Slot, SoundStage};

#[derive(Default)]
struct Test {
    gain: f32,
    /// Blocks run: each block's gain grows with it, so a lost state is heard.
    blocks: u32,
}

fn fetch(url: &str) -> String {
    let r = nori_plugin_sdk::http::fetch(&Request { method: Method::Get, url: url.into(), headers: Vec::new(), body: None, timeout_ms: None });
    match r {
        Ok(r) => format!("{} {}", r.status, String::from_utf8_lossy(&r.body)),
        Err(e) => format!("{:?}", e.kind),
    }
}

impl Lifecycle for Test {
    fn start(&mut self) {
        settings::override_(&[("playback.speed".into(), Value::Level(1.25))]).unwrap();
        let _ = settings::override_(&[("playback.pitch".into(), Value::Level(2.0))]).map(|_| panic!("not its key"));
        let heard = [fetch("https://allowed.example/a"), fetch("https://elsewhere.example/b")].join("|");
        store::set("heard", heard.as_bytes()).unwrap();
        #[cfg(feature = "timers")]
        nori_plugin_sdk::timers::stop(0);
    }
}

impl SoundStage for Test {
    fn configure(&mut self, _format: Format, inputs: Inputs) {
        self.gain = inputs.rows[0] as f32;
    }

    fn process(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        self.blocks += 1;
        let g = self.gain * self.blocks as f32;
        samples.iter_mut().for_each(|s| *s *= g);
        samples
    }

    fn reset(&mut self) {
        self.blocks = 0;
    }

    fn save(&mut self) -> Vec<u8> {
        state::save(&self.blocks)
    }

    fn restore(&mut self, bytes: Vec<u8>) {
        state::restore(&mut self.blocks, &bytes)
    }
}

const MANIFEST: Manifest = Manifest {
    stage: Some(Slot::Master),
    net: &["allowed.example"],
    overrides: &["playback.speed"],
    rows: &[Row::level("gain", 0.0, 2.0, 0.1, 0.5)],
    ..Manifest::new("fm.nori.test")
};

nori_plugin_sdk::plugin!(Test, MANIFEST, lifecycle, sound_stage);
