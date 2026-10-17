//! The web client's core, run in a Dedicated Worker: nori-core over the worker's `fetch`, and the engine
//! driven by the page. Every answer to the page is JSON; words are the page's.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

mod fetch;
#[cfg(feature = "engine")]
mod play;

#[cfg(feature = "engine")]
use std::cell::RefCell;
#[cfg(feature = "engine")]
use std::rc::Rc;
use std::sync::Arc;

use nori_core::browse::AlbumSort;
use nori_core::cache_policy::Read;
use nori_core::client::{Client, NetProfile};
use nori_core::settings_store::Settings;
#[cfg(feature = "engine")]
use nori_core::transport::Exchange;
use nori_core::{Core, ServerConfig};
use serde_json::json;
use wasm_bindgen::prelude::*;

use fetch::Fetch;
#[cfg(feature = "engine")]
use play::{Fetched, Player};

fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub struct Nori {
    client: Arc<Client>,
    /// The databases are kept in OPFS, not in memory.
    stored: bool,
    #[cfg(feature = "engine")]
    fetched: Fetched,
    #[cfg(feature = "engine")]
    player: Rc<RefCell<Option<Player>>>,
}

#[wasm_bindgen]
impl Nori {
    /// Opens the app's and the profile's databases in the origin's private file system (OPFS, through
    /// SQLite's sync-access-handle pool, which needs a Dedicated Worker); in memory where it cannot.
    pub async fn open() -> Result<Nori, JsValue> {
        let kept = sqlite_wasm_vfs::sahpool::install::<sqlite_wasm_rs::WasmOsCallback>(&Default::default(), true).await;
        let (app, profile) = match &kept {
            Ok(_) => ("app.db", "nori.db"),
            Err(e) => {
                web_sys::console::warn_1(&format!("the databases stay in memory: {e}").into());
                ("", "")
            }
        };
        let settings = Settings::new();
        settings.open(app).map_err(error)?;
        let session = Arc::new(nori_core::queue::Session::new(settings));
        let core = Core::new(profile.into(), "web".into(), session).map_err(error)?;
        let client = Client::new(core, Arc::new(Fetch), nori_core::covers::CoverNet::new());
        Ok(Nori {
            client,
            stored: kept.is_ok(),
            #[cfg(feature = "engine")]
            fetched: Arc::default(),
            #[cfg(feature = "engine")]
            player: Rc::default(),
        })
    }

    #[wasm_bindgen(getter)]
    pub fn stored(&self) -> bool {
        self.stored
    }

    /// Signs in to `url`: true when the server answered.
    pub async fn login(&self, url: String, user: String, password: String) -> Result<bool, JsValue> {
        let config = ServerConfig { url: url.clone(), user, password, api_key: None, legacy_auth: false };
        let legacy = nori_core::client::login_check(Arc::new(Fetch), config.clone(), String::new()).await.map_err(error)?;
        self.client.core().configure(ServerConfig { legacy_auth: legacy, ..config }).map_err(error)?;
        self.client.set_profile(NetProfile { url, ..NetProfile::default() });
        Ok(true)
    }

    /// The newest albums, as JSON.
    pub async fn albums(&self, size: i32) -> Result<String, JsValue> {
        let mut albums = Vec::new();
        let read = Read::AlbumList { kind: AlbumSort::Newest, size, offset: 0, genre: None };
        self.client.read_each(read, |p| albums = p.albums()).await.map_err(error)?;
        let core = self.client.core();
        let list: Vec<_> = albums.iter().map(|a| json!({ "id": a.id, "name": a.name, "artist": a.artist, "year": a.year, "cover": a.cover_art.as_ref().map(|c| core.url("getCoverArt".into(), vec![param("id", c), param("size", "300")])) })).collect();
        serde_json::to_string(&list).map_err(error)
    }

    /// An album's songs, as JSON; the queue knows them from now on.
    pub async fn album(&self, id: String) -> Result<String, JsValue> {
        let mut songs = Vec::new();
        self.client.read_each(Read::AlbumById { id }, |p| songs = p.songs()).await.map_err(error)?;
        self.client.session().queue_register(songs.clone());
        let list: Vec<_> = songs.iter().map(|s| json!({ "id": s.id, "title": s.title, "artist": s.artist, "duration": s.duration, "suffix": s.suffix })).collect();
        serde_json::to_string(&list).map_err(error)
    }
}

/// Playback, in a build with the engine.
#[cfg(feature = "engine")]
#[wasm_bindgen]
impl Nori {
    /// Fetches song `id` whole and plays it through the engine, its sound pulled at `rate` Hz.
    pub async fn play(&self, id: String, rate: u32) -> Result<(), JsValue> {
        let target = self.client.resolve(id.clone(), false, false);
        let song = self.client.session().song(&id);
        let started = nori_time::Instant::now();
        let got = fetch::fetch(Exchange { url: target.url, headers: Default::default(), json: None, timeout_ms: 0 }).await.map_err(error)?;
        if got.status != 200 {
            return Err(error(format!("the server answered {}", got.status)));
        }
        nori_core::alog::info(&format!("fetched {id}: {} bytes in {} ms", got.body.len(), started.elapsed().as_millis()));
        let hint = song.map(|s| s.suffix).filter(|s| !s.is_empty());
        self.fetched.lock().insert(id.clone(), (got.body.into(), hint));
        let mut player = self.player.borrow_mut();
        let p = player.get_or_insert_with(|| Player::new(self.client.clone(), self.fetched.clone(), rate));
        self.client.session().playlist_set(vec![id], Some(0), false, None);
        p.engine.queue_changed();
        p.engine.play_at(0, 0);
        Ok(())
    }

    pub fn pause(&self) {
        if let Some(p) = self.player.borrow().as_ref() {
            p.engine.toggle();
        }
    }

    /// Runs the engine: ms until it wants to run again, -1 for when the page next pulls or asks.
    pub fn turn(&self) -> f64 {
        match self.player.borrow_mut().as_mut().and_then(|p| p.driven.turn()) {
            Some(Some(ms)) => ms as f64,
            _ => -1.0,
        }
    }

    /// Fills `out` with interleaved samples for the worklet; returns the frames written.
    pub fn pull(&self, out: &mut [f32]) -> usize {
        self.player.borrow_mut().as_mut().map_or(0, |p| p.pull(out))
    }

    /// The output's channels once open, else 0.
    pub fn channels(&self) -> usize {
        self.player.borrow().as_ref().and_then(|p| p.card.lock().feed.as_ref().map(|f| f.format().channels)).unwrap_or(0)
    }

    /// The engine's status and the events since last asked, as JSON.
    pub fn status(&self) -> String {
        let player = self.player.borrow();
        let Some(p) = player.as_ref() else { return "{}".into() };
        let s = p.engine.status();
        let said: Vec<String> = std::mem::take(&mut *p.said.lock());
        json!({ "state": format!("{:?}", s.state), "id": s.id, "positionMs": s.position_now(), "underruns": s.underruns, "events": said }).to_string()
    }
}

fn param(key: &str, value: &str) -> nori_core::Param {
    nori_core::Param { key: key.into(), value: value.into() }
}
