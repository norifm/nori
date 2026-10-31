//! Each extension point nori's tests check, in its plainest form (nori_plugins::testing).

// The headphones are a plugin of their own: the test plugin's points go unused.
#![cfg_attr(feature = "headphones", allow(dead_code))]

#[cfg(feature = "headphones")]
mod headphones;
#[cfg(feature = "headphones")]
nori_plugin_sdk::plugin!(headphones::Headphones, headphones::MANIFEST, device_control, hooks);
mod automix;
mod deciders;
mod lan;
mod lyrics;
mod preparation;
mod scrobbler;

use nori_plugin_sdk::http::{Method, Request};
use nori_plugin_sdk::sound_stage::{Format, Inputs};
use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::slots::{InfoLine, Motion, Sleeve};
use nori_plugin_sdk::provider;
use nori_plugin_sdk::types::{Error, Explicit, Found, Item, Song};
use nori_plugin_sdk::{settings, state, store, Glyph, Hooks, ItemKind, Lifecycle, Manifest, MenuAction, Place, Provider, Row, Slot, Slots, SoundStage};

#[derive(Default)]
struct Test {
    gain: f32,
    /// Blocks run: each block's gain grows with it, so a lost state is heard.
    blocks: u32,
    /// Its stage's other inputs as numbers: volume dB, route, the song's gain dB and loudness (NaN
    /// unknown), the frames its prepared data counts (-1 without). A gain below 0 writes them over
    /// a block's first samples instead of scaling it.
    heard: [f32; 5],
    /// A song being prepared: frames fed, the frame of the first, channels.
    prepared: Option<(u64, Option<u64>, u64)>,
    /// Samples of a song being analysed.
    analysed: Option<u64>,
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

mod hooked;
#[cfg(feature = "unlinked")]
mod unlinked;

impl Lifecycle for Test {
    fn action(&mut self, id: String) {
        deciders::acted(&id);
        if hooked::action(&id) {
            return;
        }
        if id == "put" {
            let header = |name: &str, value: &str| nori_plugin_sdk::types::Header { name: name.into(), value: value.into() };
            let sent = Request { method: Method::Put, url: "https://allowed.example/put".into(), headers: vec![header("content-type", "application/x-protobuf")], body: Some(vec![0, 159, 146, 150]), timeout_ms: None };
            let heard = match nori_plugin_sdk::http::fetch(&sent) {
                Ok(r) => r.headers.iter().map(|h| format!("{}={}", h.name, h.value)).collect(),
                Err(e) => vec![format!("{:?}", e.kind)],
            };
            nori_plugin_sdk::notices::show(nori_plugin_sdk::notices::Tone::Info, "put", &heard);
        }
        if id == "together" {
            let heard = fetch_together("https://allowed.example/together/one", "https://allowed.example/together/two");
            nori_plugin_sdk::notices::show(nori_plugin_sdk::notices::Tone::Info, "together", &[heard]);
        }
        lan::action(&id);
    }

    fn start(&mut self) {
        #[cfg(feature = "fails-to-start")]
        panic!("told to");
        settings::override_(&[("playback.speed".into(), Value::Level(1.25))]).unwrap();
        let _ = settings::override_(&[("playback.pitch".into(), Value::Level(2.0))]).map(|_| panic!("not its key"));
        let heard = [fetch("https://allowed.example/a"), fetch("https://elsewhere.example/b")].join("|");
        store::set("heard", heard.as_bytes()).unwrap();
        #[cfg(feature = "unlinked")]
        unlinked::nori::plugin::made_up::ping();
    }
}

impl SoundStage for Test {
    fn configure(&mut self, _format: Format, inputs: Inputs) {
        self.gain = inputs.rows[0] as f32;
        let prepared = inputs.prepared.as_ref().and_then(|d| Some(u64::from_le_bytes(d.get(..8)?.try_into().ok()?) as f32)).unwrap_or(-1.0);
        self.heard = [inputs.volume_db, inputs.route as u8 as f32, inputs.gain_db.unwrap_or(f32::NAN), inputs.loudness_lufs.unwrap_or(f32::NAN), prepared];
    }

    fn process(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        if self.gain < 0.0 {
            let n = samples.len().min(5);
            samples[..n].copy_from_slice(&self.heard[..n]);
            return samples;
        }
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

impl Hooks for Test {
    fn on(&mut self, event: nori_plugin_sdk::hooks::Event) {
        use nori_plugin_sdk::hooks::Event;
        match event {
            Event::Discovered(_) | Event::MediaButton(_) | Event::OutputPicker(_) => lan::on(event),
            _ => hooked::on(event),
        }
    }

    fn served(&mut self, request: nori_plugin_sdk::hooks::ServedRequest) -> Option<nori_plugin_sdk::hooks::ServedResponse> {
        lan::served(request)
    }
}

const MANIFEST: Manifest = Manifest {
    stage: Some(if cfg!(feature = "song") { Slot::Song } else { Slot::Master }),
    net: &["allowed.example", "lan"],
    overrides: &["playback.speed"],
    rows: &[Row::level("gain", -1.0, 2.0, 0.1, 0.5)],
    menu: &[MenuAction { id: "count", glyph: Glyph::Info, kinds: &[ItemKind::Song] }],
    places: &[Place::Artwork, Place::SongInfo, Place::PlayerStatus, Place::Devices],
    signin: hooked::SIGNIN,
    permissions: hooked::PERMISSIONS,
    hooks: hooked::HOOKS,
    wake: hooked::WAKE,
    commands: hooked::COMMANDS,
    cache_mb: hooked::CACHE_MB,
    assets: hooked::ASSETS,
    ..Manifest::new(if cfg!(feature = "second") { "fm.nori.test.second" } else { "fm.nori.test" })
};

#[cfg(not(feature = "headphones"))]
nori_plugin_sdk::plugin!(Test, MANIFEST, lifecycle, hooks, sound_stage, provider, slots, preparation, automix, deciders, lyrics, scrobbler);
