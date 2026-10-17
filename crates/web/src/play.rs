//! The engine in the worker: driven by the page (no threads), its songs fetched whole, its sound pulled
//! by the AudioWorklet through the page's messages.

use std::collections::HashMap;
use std::sync::Arc;

use nori_core::client::Client;
use nori_engine::core::{about, settings, CoreApp, CoreQueue};
use nori_engine::{AudioOutput, Config, Driven, Engine, Event, Feed, Library, Located, OutputFormat, Source};
use nori_player::transitions::WindowSong;
use parking_lot::Mutex;

/// Songs fetched whole, by id.
pub type Fetched = Arc<Mutex<HashMap<String, (Arc<[u8]>, Option<String>)>>>;

pub struct Songs {
    pub client: Arc<Client>,
    pub fetched: Fetched,
}

impl Library for Songs {
    fn locate(&mut self, id: &str) -> Result<Located, String> {
        let (bytes, hint) = self.fetched.lock().get(id).cloned().ok_or("not fetched")?;
        let duration_ms = self.client.session().song(id).map(|s| s.duration as i64 * 1000).filter(|&d| d > 0);
        Ok(Located { source: Source::Memory(bytes), hint, duration_ms, estimated: false })
    }

    fn about(&self, id: &str) -> WindowSong {
        about(self.client.session(), id)
    }

    /// Only what was fetched: the page fetches a song when it is asked for.
    fn fetch_ahead(&self, id: &str) -> bool {
        self.fetched.lock().contains_key(id)
    }
}

/// The AudioWorklet's side: what the page pulls for it.
#[derive(Default)]
pub struct Card {
    pub feed: Option<Feed>,
    pub playing: bool,
    /// The AudioContext's rate.
    pub rate: u32,
}

pub struct Output(pub Arc<Mutex<Card>>);

impl AudioOutput for Output {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        let rate = self.0.lock().rate;
        Ok(OutputFormat { rate, channels: want.channels.min(2), bits: 0 })
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        self.0.lock().feed = Some(feed);
        Ok(())
    }

    fn pause(&mut self) {
        self.0.lock().playing = false;
    }

    fn resume(&mut self) {
        self.0.lock().playing = true;
    }

    /// What the worklet holds ahead of the ear: the page keeps about this much queued.
    fn latency_us(&self) -> u64 {
        200_000
    }

    fn takes_float(&mut self) -> bool {
        true
    }

    fn close(&mut self) {
        self.0.lock().feed = None;
    }
}

pub type Said = Arc<Mutex<Vec<String>>>;
type Events = Box<dyn FnMut(Event) + Send>;

pub struct Player {
    pub engine: Engine,
    pub driven: Driven<Songs, CoreApp, CoreQueue, Events, nori_engine::Monotonic>,
    pub card: Arc<Mutex<Card>>,
    pub said: Said,
}

impl Player {
    pub fn new(client: Arc<Client>, fetched: Fetched, rate: u32) -> Player {
        let session = client.session().clone();
        let prefs = session.settings.current().unwrap_or_default();
        let card = Arc::new(Mutex::new(Card { rate, ..Card::default() }));
        let said: Said = Arc::default();
        let heard = said.clone();
        let events: Events = Box::new(move |e| heard.lock().push(format!("{e:?}")));
        let config = Config { memory_mb: 128, settings: settings(&prefs, 0.0), ..Config::default() };
        let (engine, driven) = Engine::driven(Songs { client, fetched }, CoreApp::new(session.clone()), CoreQueue(session), Box::new(Output(card.clone())), config, nori_engine::Monotonic::new(), || {}, events);
        Player { engine, driven, card, said }
    }

    /// Fills `out` (interleaved) from the ring; returns the frames written.
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        let mut card = self.card.lock();
        if !card.playing {
            return 0;
        }
        card.feed.as_mut().map_or(0, |f| f.pull(out))
    }
}
