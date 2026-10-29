//! Each extension point nori's tests check, in its plainest form (nori_plugins::testing).

mod automix;
mod deciders;
mod lyrics;
mod preparation;
mod scrobbler;

use nori_plugin_sdk::http::{Method, Request};
use nori_plugin_sdk::sound_stage::{Format, Inputs};
use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::slots::{InfoLine, Motion, Sleeve};
use nori_plugin_sdk::provider;
use nori_plugin_sdk::types::{Error, Explicit, Found, Item, Song};
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

/// Two requests on their way together, their answers in order.
fn fetch_together(a: &str, b: &str) -> String {
    use std::future::Future;
    use std::task::Poll;
    let get = |url: &str| nori_plugin_sdk::fetching::send(Request { method: Method::Get, url: url.into(), headers: Vec::new(), body: None, timeout_ms: None });
    let (mut x, mut y) = (Box::pin(get(a)), Box::pin(get(b)));
    let (mut ax, mut ay) = (None, None);
    nori_plugin_sdk::fetching::run(std::future::poll_fn(|cx| {
        if ax.is_none() {
            if let Poll::Ready(v) = x.as_mut().poll(cx) {
                ax = Some(v);
            }
        }
        if ay.is_none() {
            if let Poll::Ready(v) = y.as_mut().poll(cx) {
                ay = Some(v);
            }
        }
        if ax.is_some() && ay.is_some() { Poll::Ready(()) } else { Poll::Pending }
    }));
    let said = |r: Option<Result<nori_plugin_sdk::http::Response, nori_plugin_sdk::types::Error>>| match r {
        Some(Ok(r)) => format!("{} {}", r.status, String::from_utf8_lossy(&r.body)),
        Some(Err(e)) => format!("{:?}", e.kind),
        None => "none".into(),
    };
    format!("{}|{}", said(ax), said(ay))
}

impl Lifecycle for Test {
    fn action(&mut self, id: String) {
        deciders::acted(&id);
        if id == "together" {
            let heard = fetch_together("https://allowed.example/together/one", "https://allowed.example/together/two");
            nori_plugin_sdk::notices::show(nori_plugin_sdk::notices::Tone::Info, "together", &[heard]);
        }
    }

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
        // SAFETY: a number.
        unsafe { state::save(&self.blocks) }
    }

    fn restore(&mut self, bytes: Vec<u8>) {
        // SAFETY: a number.
        unsafe { state::restore(&mut self.blocks, &bytes) }
    }
}

/// A profile with no Subsonic server, whose search finds one song named after what was searched and
/// whose songs stream from allowed.example in parts.
impl Provider for Test {
    fn read(&mut self, request: provider::Request) -> Result<provider::Page, Error> {
        let provider::Request::Search((query, ..)) = request else { return Ok(provider::Page::None) };
        let song = Song {
            id: format!("test-{query}"),
            title: query,
            artist: String::new(),
            artist_id: None,
            artists: Vec::new(),
            album: String::new(),
            album_id: None,
            cover: None,
            duration: 0,
            track: 0,
            disc: 0,
            year: 0,
            genre: None,
            // The host's own favorite.
            starred: true,
            external: false,
            explicit: Explicit::Unknown,
            replay_gain: None,
            bpm: 0,
            rating: 0,
        };
        Ok(provider::Page::Found(Found { songs: vec![song], albums: Vec::new(), artists: Vec::new(), playlists: Vec::new() }))
    }

    fn source(&mut self, id: String, _: Option<u32>, _: provider::StreamUse) -> Result<provider::StreamSource, Error> {
        Ok(provider::StreamSource { url: format!("https://allowed.example/song/{id}"), length: Some(1000), part: Some(100) })
    }
}

impl Slots for Test {
    /// The song titled "loop" plays a loop with a poster; any other keeps nori's cover.
    fn artwork(&mut self, song: Song) -> Option<Sleeve> {
        (song.title == "loop").then(|| Sleeve::Loop(Motion { video: "https://allowed.example/loop.m3u8".into(), poster: Some("https://allowed.example/first/{size}.jpg".into()) }))
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

nori_plugin_sdk::plugin!(Test, MANIFEST, lifecycle, sound_stage, provider, slots, preparation, automix, deciders, lyrics, scrobbler);
