//! Each extension point nori's tests check, in its plainest form (nori_plugins::testing).

use nori_plugin_sdk::http::{Method, Request};
use nori_plugin_sdk::sound_stage::{Format, Inputs};
use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::slots::{InfoLine, Sleeve};
use nori_plugin_sdk::types::{Item, Song};
use nori_plugin_sdk::{settings, state, store, Glyph, ItemKind, Lifecycle, Manifest, MenuAction, Place, Provider, Row, Slot, Slots, SoundStage};

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

/// nori's own answers: a profile with no Subsonic server.
impl Provider for Test {}

impl Slots for Test {
    /// The song titled "loop" plays a loop; any other keeps nori's cover.
    fn artwork(&mut self, song: Song) -> Option<Sleeve> {
        (song.title == "loop").then(|| Sleeve::Loop("https://allowed.example/loop.m3u8".into()))
    }

    /// Says what it ran on.
    fn menu(&mut self, action: String, items: Vec<Item>) {
        let titles: Vec<String> = items.into_iter().map(|i| match i {
            Item::Song(s) => s.title,
            Item::Album(a) => a.name,
            Item::Artist(a) => a.name,
            Item::Playlist(p) => p.name,
        }).collect();
        nori_plugin_sdk::notices::show(nori_plugin_sdk::notices::Tone::Info, &action, &titles);
    }

    fn song_info(&mut self, song: Song) -> Vec<InfoLine> {
        vec![InfoLine { words: format!("{} letters", song.title.len()), link: None }]
    }

    fn player_status(&mut self) -> Option<String> {
        Some("Testing".into())
    }
}

const MANIFEST: Manifest = Manifest {
    stage: Some(Slot::Master),
    net: &["allowed.example"],
    overrides: &["playback.speed"],
    rows: &[Row::level("gain", 0.0, 2.0, 0.1, 0.5)],
    menu: &[MenuAction { id: "count", glyph: Glyph::Info, kinds: &[ItemKind::Song] }],
    places: &[Place::Artwork, Place::SongInfo, Place::PlayerStatus, Place::Devices],
    ..Manifest::new("fm.nori.test")
};

nori_plugin_sdk::plugin!(Test, MANIFEST, lifecycle, sound_stage, provider, slots);
