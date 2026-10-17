//! The web client's core, run in a Dedicated Worker. Built two ways (web/build.sh): `core.wasm`, the
//! core and its databases alone, which signs in and reads the library; and `player.wasm` (the `engine`
//! feature), which runs the session every client shares (`nori_host`) with the engine driven by the
//! worker, and takes over from the first on the first Play. Every answer to the page is JSON; words are
//! the page's.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

mod fetch;
mod pages;
#[cfg(feature = "engine")]
mod later;
#[cfg(feature = "engine")]
mod play;
#[cfg(feature = "engine")]
mod stream;

use std::sync::Arc;

#[cfg(not(feature = "engine"))]
use nori_core::client::Client;
use nori_core::settings::SavedServer;
use nori_core::settings_store::Settings;
use nori_core::ServerConfig;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use fetch::Fetch;

fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// The databases: the app's settings and every profile's, in one file as on the other clients.
fn db() -> String {
    nori_core::db::DB_FILE.into()
}

#[wasm_bindgen]
pub struct Nori {
    /// The app's queue session over its settings.
    app: Arc<nori_core::queue::Session>,
    /// The databases are kept in the origin's private file system, not in memory.
    stored: bool,
    #[cfg(not(feature = "engine"))]
    client: std::cell::RefCell<Option<Arc<Client>>>,
    #[cfg(feature = "engine")]
    live: play::Live,
}

#[wasm_bindgen]
impl Nori {
    /// Opens the databases in the origin's private file system (OPFS, through SQLite's
    /// sync-access-handle pool, which needs a Dedicated Worker); in memory where it cannot. `tell` gets
    /// the session's reports as JSON; `wake` schedules the engine's next turn.
    pub async fn open(tell: js_sys::Function, wake: js_sys::Function) -> Result<Nori, JsValue> {
        let kept = sqlite_wasm_vfs::sahpool::install::<sqlite_wasm_rs::WasmOsCallback>(&Default::default(), true).await;
        if let Err(e) = &kept {
            web_sys::console::warn_1(&format!("the databases stay in memory: {e}").into());
        }
        let settings = Settings::new();
        let path = if kept.is_ok() { db() } else { String::new() };
        settings.open(&path).map_err(error)?;
        let app = Arc::new(nori_core::queue::Session::new(settings));
        #[cfg(not(feature = "engine"))]
        let _ = (tell, wake);
        Ok(Nori {
            #[cfg(feature = "engine")]
            live: play::Live::new(tell, wake),
            app,
            stored: kept.is_ok(),
            #[cfg(not(feature = "engine"))]
            client: Default::default(),
        })
    }

    #[wasm_bindgen(getter)]
    pub fn stored(&self) -> bool {
        self.stored
    }

    /// The profile signed in, as `{ url, user }`; null for none.
    pub fn profile(&self) -> String {
        match self.active() {
            Some(p) => json!({ "url": p.url, "user": p.user }).to_string(),
            None => "null".into(),
        }
    }

    fn active(&self) -> Option<SavedServer> {
        self.app.settings.prefs(|p| p.servers.iter().find(|s| s.id == p.active_server_id).cloned())
    }

    /// Signs in to `url` and keeps the profile, the active one from now on.
    pub async fn login(&self, url: String, user: String, password: String) -> Result<(), JsValue> {
        let config = ServerConfig { url: url.clone(), user: user.clone(), password: password.clone(), api_key: None, legacy_auth: false };
        let legacy = nori_core::client::login_check(Arc::new(Fetch), config, String::new()).await.map_err(error)?;
        let mut prefs = self.app.settings.current().unwrap_or_default();
        let saved = prefs.servers.iter().find(|s| s.url == url && s.user == user).cloned();
        let profile = SavedServer { password, legacy_auth: legacy, ..saved.unwrap_or_else(|| SavedServer { id: nori_core::settings::new_server_id(), name: user.clone(), url, user, ..Default::default() }) };
        prefs.servers.retain(|s| s.id != profile.id);
        prefs.servers.push(profile.clone());
        prefs.active_server_id = profile.id.clone();
        self.app.settings.put(prefs);
        self.enter(profile)
    }

    /// Opens the profile signed in last; false when there is none.
    pub fn resume(&self) -> Result<bool, JsValue> {
        match self.active() {
            Some(p) => self.enter(p).map(|_| true),
            None => Ok(false),
        }
    }

    /// Signs out: no profile is open, and the next start asks to sign in. The profile's data stays.
    pub fn logout(&self) {
        self.leave();
        let mut prefs = self.app.settings.current().unwrap_or_default();
        prefs.active_server_id.clear();
        self.app.settings.put(prefs);
    }

    /// Saves the session and closes it, for another tab to take the databases over.
    pub fn close(&self) {
        self.leave();
    }

    /// Asks the server for a change to the library (`{ kind, ... }`, see `pages::Change`): playlists made,
    /// renamed, filled, reordered or deleted, and hearts. Kept for later when offline.
    pub async fn change(&self, change: String) -> Result<(), JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        pages::change(&client, &change).await.map_err(error)
    }

    /// Reads a page (`{ kind, ... }`, see `pages::Ask`): its JSON.
    pub async fn read(&self, ask: String) -> Result<String, JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        pages::read(&client, &ask).await.map_err(error)
    }

    /// Cover `id`'s address for `px` square.
    pub fn cover(&self, id: String, px: u32) -> String {
        self.client().map_or_else(String::new, |c| pages::cover(c.core(), &id, px))
    }

    /// The settings `names` (comma separated) as `{ name: value }`.
    pub fn settings(&self, names: String) -> String {
        let prefs = self.app.settings.current().unwrap_or_default();
        let v: serde_json::Map<String, Value> = names.split(',').filter_map(|n| Some((n.to_string(), Value::String(nori_core::settings_model::value_of(&prefs, n)?)))).collect();
        Value::Object(v).to_string()
    }
}

/// Without the engine: the core alone, for the library.
#[cfg(not(feature = "engine"))]
#[wasm_bindgen]
impl Nori {
    fn enter(&self, profile: SavedServer) -> Result<(), JsValue> {
        let core = nori_core::Core::new(db(), nori_core::settings::server_db_id(&profile.id), self.app.clone()).map_err(error)?;
        core.configure(nori_core::server_config(&profile)).map_err(error)?;
        let client = Client::new(core, Arc::new(Fetch), nori_core::covers::CoverNet::new());
        client.set_profile(nori_core::net_profile(&profile));
        *self.client.borrow_mut() = Some(client);
        Ok(())
    }

    fn client(&self) -> Option<Arc<Client>> {
        self.client.borrow().clone()
    }

    fn leave(&self) {
        *self.client.borrow_mut() = None;
    }

    /// Sets a setting by name; false if there is no such setting.
    pub fn setting(&self, name: String, value: String) -> bool {
        self.app.settings.edit_by_name(&name, &value).is_some()
    }
}
