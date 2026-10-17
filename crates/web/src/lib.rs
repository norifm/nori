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
#[cfg(not(feature = "engine"))]
use nori_core::browse::ProfileRules;
use nori_core::settings::SavedServer;
use nori_core::settings_pages::{self, SettingsClient, SettingsPage};
use nori_core::settings_store::Settings;
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
        self.app.settings.prefs(|p| p.active_server().cloned())
    }

    /// Signs in to `url` and keeps the profile, the active one from now on (`providers::log_in`).
    pub async fn login(&self, url: String, user: String, password: String) -> Result<(), JsValue> {
        let form = SavedServer { id: nori_core::settings::new_server_id(), name: user.clone(), url, user, password, ..Default::default() };
        let profile = nori_core::providers::log_in(Arc::new(Fetch(())), &self.app.settings, form).await.map_err(error)?;
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

    /// The values the settings `names` (comma separated) offer, as `{ name: [value, ...] }`
    /// (`settings_model::specs`).
    pub fn options(&self, names: String) -> String {
        let specs = nori_core::settings_model::specs();
        let v: serde_json::Map<String, Value> =
            names.split(',').filter_map(|n| Some((n.to_string(), Value::from(specs.iter().find(|s| s.name == n)?.options.clone())))).collect();
        Value::Object(v).to_string()
    }

    /// The settings pages a browser has for the profile open, by name (`settings_pages`).
    pub fn settings_pages(&self) -> String {
        let pages = settings_pages::pages(SettingsClient::Web, &self.rules().settings);
        Value::from(pages.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>()).to_string()
    }

    /// Settings page `name` as a browser draws it: `[{ id, rows: [row] }]`, each row `{ item, shown,
    /// enabled, depth }` with `item` `{ kind: "setting", name, type, options, min, max, value, chips }`
    /// or `{ kind: "note", note }`. Words are the page's.
    pub fn settings_page(&self, name: String) -> Result<String, JsValue> {
        let page = SettingsPage::ALL.into_iter().find(|p| format!("{p:?}") == name).ok_or_else(|| error(format!("no settings page {name}")))?;
        let prefs = self.app.settings.current().unwrap_or_default();
        // What the pages read of the device: a browser has no DAC, no offload and no beat model, and its
        // pages list no lyrics services (`settings_model::state` would bring the player's policy in).
        let state = nori_core::settings_model::SettingsState {
            values: Default::default(),
            untouched: false,
            sound_chain_on: prefs.sound_chain_on(),
            offload_paused: false,
            lyrics_sources: Vec::new(),
            beat_model: nori_core::settings_model::BeatModel::Unavailable,
            beat_model_mb: 0,
        };
        let facts = settings_pages::PageFacts { parts: self.rules().settings, ..Default::default() };
        let sections: Vec<Value> = settings_pages::page(page, SettingsClient::Web, &prefs, &state, &facts)
            .into_iter()
            .map(|sec| json!({ "id": format!("{:?}", sec.id), "rows": sec.rows.iter().filter_map(settings_row).collect::<Vec<_>>() }))
            .collect();
        Ok(Value::from(sections).to_string())
    }

    /// Song `song`'s menu (JSON, as a page read it), most used first (`menus::song_menu`): `[{ action,
    /// more, on, id, name, named }]`. `player`: opened from the player, showing `lyrics`.
    pub fn menu(&self, song: String, player: bool, lyrics: bool) -> Result<String, JsValue> {
        use nori_core::menus::{song_menu, SongAction, SongDownload};
        let song: nori_core::Song = serde_json::from_str(&song).map_err(error)?;
        let rules = self.rules();
        let items: Vec<Value> = song_menu(song.clone(), song.starred, SongDownload::None, player, lyrics, rules.stars, rules.account, self.jam_offer())
            .into_iter()
            .map(|m| {
                let mut v = json!({ "more": m.more });
                let action = match m.action {
                    SongAction::Favourite { on } => {
                        v["on"] = json!(on);
                        "Favourite"
                    }
                    SongAction::GoToAlbum { id } => {
                        v["id"] = json!(id);
                        "GoToAlbum"
                    }
                    SongAction::GoToArtist { id, name, named } => {
                        v["id"] = json!(id);
                        v["name"] = json!(name);
                        v["named"] = json!(named);
                        "GoToArtist"
                    }
                    a => return json!({ "action": format!("{a:?}"), "more": m.more }),
                };
                v["action"] = json!(action);
                v
            })
            .collect();
        Ok(Value::from(items).to_string())
    }
}

/// A settings row as the page draws it; None for a kind a browser never has.
fn settings_row(r: &settings_pages::SettingsRow) -> Option<Value> {
    use settings_pages::RowItem;
    let item = match &r.item {
        RowItem::Setting { spec, value, chips } => json!({ "kind": "setting", "name": spec.name, "type": format!("{:?}", spec.kind), "options": spec.options, "min": spec.min, "max": spec.max, "value": value, "chips": chips }),
        RowItem::Note { note } => json!({ "kind": "note", "note": format!("{note:?}") }),
        _ => return None,
    };
    Some(json!({ "item": item, "shown": r.shown, "enabled": r.enabled, "depth": r.depth }))
}

/// Without the engine: the core alone, for the library.
#[cfg(not(feature = "engine"))]
#[wasm_bindgen]
impl Nori {
    fn enter(&self, profile: SavedServer) -> Result<(), JsValue> {
        let core = nori_core::Core::new(db(), nori_core::settings::server_db_id(&profile.id), self.app.clone()).map_err(error)?;
        core.configure(nori_core::server_config(&profile)).map_err(error)?;
        let client = Client::new(core, Arc::new(Fetch(())), nori_core::covers::CoverNet::new());
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

    /// An account's: the core module opens no guest's profile.
    fn rules(&self) -> ProfileRules {
        ProfileRules::default()
    }

    fn jam_offer(&self) -> Option<nori_core::menus::JamOffer> {
        None
    }

    /// Sets a setting by name; false if there is no such setting.
    pub fn setting(&self, name: String, value: String) -> bool {
        self.app.settings.edit_by_name(&name, &value).is_some()
    }
}
