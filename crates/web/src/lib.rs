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

    /// The queue another device or app left on the server, offered while nothing plays here
    /// (`Client::queue_offer`; `playing` and `shown`, the song the player shows, are the player's):
    /// `{ songs, index, positionMs }` (the songs as a page reads them), or null.
    pub async fn queue_offer(&self, playing: bool, shown: Option<String>) -> Result<String, JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        let Some(o) = client.queue_offer(playing, shown).await.map_err(error)? else { return Ok("null".into()) };
        let songs: Vec<Value> = o.songs.iter().map(|s| pages::song(client.core(), s)).collect();
        Ok(json!({ "songs": songs, "index": o.index, "positionMs": o.position_ms }).to_string())
    }

    /// The offer of `songs` (JSON) from `index`, `position_ms` in, was taken or turned down: that queue is
    /// not offered again (`Client::queue_offer_done`, which keeps where it stands, not who left it).
    pub fn queue_offer_done(&self, songs: String, index: u32, position_ms: f64) -> Result<(), JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        let songs: Vec<nori_core::Song> = serde_json::from_str(&songs).map_err(error)?;
        client.queue_offer_done(nori_core::library::QueueOffer { by: nori_core::library::QueueBy::Nori, songs, index, position_ms: position_ms as u64 });
        Ok(())
    }

    /// Cover `id`'s address for `px` square.
    pub fn cover(&self, id: String, px: u32) -> String {
        self.client().map_or_else(String::new, |c| pages::cover(c.core(), &id, px))
    }

    /// The colors the page wears on a date in the browser's zone, from the season and appearance
    /// settings: `{ season (the calendar's now), next, nextMonth, nextDay (the season after and its first day),
    /// dark, page, surface, ink, ink2, hair, outline, accent, on, onAccent, still, tiles }`, the colors each a
    /// `#rrggbb`; `still` when the Motion setting asks for less than full motion; `tiles` the four seasons'
    /// `{ season, page, accent }` in the appearance worn (Night's, not black, in Midnight).
    /// `system_dark`: the browser's color scheme, followed while the appearance is System.
    pub fn theme(&self, month: u32, day: u32, zone: String, system_dark: bool) -> String {
        use nori_look::theme::{roles, season_now, southern, SEASONS};
        let prefs = self.app.settings.current().unwrap_or_default();
        let (palette, dark) = nori_core::settings::theme_palette(&prefs, system_dark, month, day, southern(&zone));
        let r = roles(palette);
        let now = season_now(month, day, southern(&zone));
        let name = |s: nori_look::theme::Season| format!("{s:?}").to_uppercase();
        let hex = |c: u32| format!("#{:06x}", c & 0xFF_FFFF);
        let tiles: Vec<Value> = SEASONS
            .iter()
            .map(|&s| {
                let p = s.palette(dark);
                json!({ "season": name(s), "page": hex(p.page), "accent": hex(p.accent) })
            })
            .collect();
        json!({
            "season": name(now.season), "next": name(now.next), "nextMonth": now.next_month, "nextDay": now.next_day, "dark": dark, "page": hex(palette.page), "surface": hex(palette.surface), "ink": hex(palette.ink),
            "ink2": hex(r.on_surface_variant), "hair": hex(r.outline_variant), "outline": hex(r.outline),
            "accent": hex(palette.accent), "on": hex(palette.on), "onAccent": hex(r.on_primary),
            "still": prefs.motion != nori_core::settings::Motion::Full, "tiles": tiles,
        })
        .to_string()
    }

    /// What a cover's page wears (an album's, an artist's, the player's), from `rgba` (a `w` x `h` copy
    /// the page decoded), dyed on the season's paper as Android's pages are: `[colors, wash]`, the colors JSON
    /// `{ page, surface, ink, ink2, outline, hair, accent, onAccent, heroEdge, heroMid, heroStops, melt,
    /// washSide }` (each color a `#rrggbbaa`; `heroStops` where the artwork's dissolve reaches `heroEdge`,
    /// `heroMid` and the page; `melt` the share of the player's sleeve that fades) and the wash (the cover
    /// blurred, RGBA, `washSide` square) or null. Both null where the appearance wears no cover colors
    /// (Midnight). The date, zone and `system_dark` as for [`Nori::theme`].
    #[allow(clippy::too_many_arguments)]
    pub fn cover_look(&self, rgba: Vec<u8>, w: u32, h: u32, month: u32, day: u32, zone: String, system_dark: bool) -> js_sys::Array {
        use nori_look::dress;
        let prefs = self.app.settings.current().unwrap_or_default();
        if !prefs.theme.covers_tint_pages() {
            return js_sys::Array::of2(&JsValue::NULL, &JsValue::NULL);
        }
        let argb: Vec<u32> = rgba.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| u32::from_be_bytes([a, r, g, b])).collect();
        let c = nori_look::cover::derive(&argb, w.max(1) as usize, h.max(1) as usize, false);
        let south = nori_look::theme::southern(&zone);
        let paper = |dark| nori_core::settings::season_palette(prefs.season, prefs.theme, dark, month, day, south);
        let page = dress::page(c.edge, c.background, c.on, c.accent, c.wash_edge);
        let dye = nori_look::theme::Dye::new(page[dress::EDGE], paper(false), paper(true), prefs.theme.is_dark(system_dark));
        let t = dress::dyed(&page, &dye);
        let hex = |argb: u32| format!("#{:06x}{:02x}", argb & 0xFF_FFFF, argb >> 24);
        let side = c.wash.as_ref().map_or(0, |w| (w.len() as f64).sqrt() as u32);
        let colors = json!({
            "page": hex(t[dress::BACKGROUND]), "surface": hex(t[dress::SURFACE_CONTAINER]), "ink": hex(t[dress::ON]),
            "ink2": hex(t[dress::ON_VARIANT]), "outline": hex(t[dress::ON_35]), "hair": hex(t[dress::OUTLINE_VARIANT]),
            "accent": hex(t[dress::ACCENT]), "onAccent": hex(t[dress::ON_PRIMARY]),
            "heroEdge": hex(t[dress::HERO_EDGE]), "heroMid": hex(t[dress::HERO_MID]), "heroStops": nori_look::sleeve::HERO_STOPS,
            "melt": nori_look::cover::FADE_SHARE, "washSide": side,
        });
        let wash = c.wash.map_or(JsValue::NULL, |w| {
            let bytes: Vec<u8> = w.iter().flat_map(|&p| { let [a, r, g, b] = dye.wash(p).to_be_bytes(); [r, g, b, a] }).collect();
            js_sys::Uint8ClampedArray::from(bytes.as_slice()).into()
        });
        js_sys::Array::of2(&JsValue::from_str(&colors.to_string()), &wash)
    }

    /// The album grid's orders, in the order offered, by name (`browse::album_sorts`): `newest`, `artistRows`, ...
    pub fn album_sorts(&self) -> String {
        Value::from(nori_core::browse::album_sorts().iter().map(|s| s.name()).collect::<Vec<_>>()).to_string()
    }

    /// The album grid's order as it was left, by name (`browse::album_sort_saved`).
    pub fn album_sort(&self) -> String {
        let prefs = self.app.settings.current().unwrap_or_default();
        nori_core::browse::album_sort_saved(prefs.list_prefs).name().into()
    }

    /// Keeps `name` as the album grid's order (`browse::album_sort_kept`).
    pub fn keep_album_sort(&self, name: String) -> Result<(), JsValue> {
        let sort = nori_core::browse::AlbumSort::of_name(&name).ok_or_else(|| error(format!("no album order {name}")))?;
        let kept = nori_core::browse::album_sort_kept(sort);
        let mut prefs = self.app.settings.current().unwrap_or_default();
        prefs.list_prefs.insert(kept.key, kept.value);
        self.app.settings.put(prefs);
        Ok(())
    }

    /// Where each artist's shelf starts among `albums` (JSON, in the Artist order: `browse::artist_rows`).
    pub fn artist_rows(&self, albums: String) -> Result<String, JsValue> {
        let albums: Vec<nori_core::Album> = serde_json::from_str(&albums).map_err(error)?;
        Ok(Value::from(nori_core::browse::artist_rows(albums)).to_string())
    }

    /// The rows of `rows` (JSON, each a list of fields) that `filter` keeps, by position (`pages::TextIndex::view`).
    pub fn filter_rows(&self, rows: String, filter: String) -> Result<String, JsValue> {
        let rows: Vec<Vec<String>> = serde_json::from_str(&rows).map_err(error)?;
        Ok(Value::from(nori_core::pages::TextIndex::new(rows).view(filter).rows).to_string())
    }

    /// Home's rows that a browser draws, in the order kept (`browse::home_rows_shown`, `home_shelves`):
    /// `[{ row, kind: "albums", sort, size } | { row, kind: "playlists", take }]`, `row` the `HomeRow`'s
    /// name. The songs shelf reads the offline index and the pinned playlists are pinned on a phone,
    /// neither of which a browser keeps.
    pub fn home_rows(&self) -> String {
        use nori_core::browse::{home_rows_shown, home_shelves, HomeShelf};
        let prefs = self.app.settings.current().unwrap_or_default();
        let rows = home_rows_shown(prefs.home_rows.clone(), self.rules().account);
        let shelves = home_shelves(rows.clone());
        let out: Vec<Value> = rows
            .iter()
            .zip(shelves)
            .filter_map(|(row, shelf)| match shelf {
                HomeShelf::Albums { sort, size, .. } => Some(json!({ "row": format!("{row:?}"), "kind": "albums", "sort": sort.name(), "size": size })),
                HomeShelf::Playlists { take } => Some(json!({ "row": format!("{row:?}"), "kind": "playlists", "take": take })),
                HomeShelf::Songs { .. } | HomeShelf::Pinned => None,
            })
            .collect();
        Value::from(out).to_string()
    }

    /// The Library page's sections a browser has, in the order kept (`browse::library_sections_shown`), by
    /// name: those it shows as pills, and `Favourites` where it is one of its places.
    pub fn library_sections(&self) -> String {
        use nori_core::settings::LibrarySection::*;
        let prefs = self.app.settings.current().unwrap_or_default();
        let shown = nori_core::browse::library_sections_shown(prefs.library_sections, self.rules().sections);
        let drawn: Vec<String> = shown.into_iter().filter(|s| matches!(s, Albums | Artists | Songs | Playlists | Genres | Favourites)).map(|s| format!("{s:?}")).collect();
        Value::from(drawn).to_string()
    }

    /// What the head of a page's song list offers, in order (`menus::page_actions`), by name, for `kind`
    /// (`Album`, `Artist`, `Playlist`) with `songs` songs. A browser keeps no downloads.
    pub fn page_actions(&self, kind: String, songs: u32) -> Result<String, JsValue> {
        use nori_core::menus::{page_actions, PageKind};
        let kind = match kind.as_str() {
            "Album" => PageKind::Album,
            "Artist" => PageKind::Artist,
            "Playlist" => PageKind::Playlist,
            _ => return Err(error(format!("no page {kind}"))),
        };
        Ok(Value::from(page_actions(kind, songs, 0, false).iter().map(|a| format!("{a:?}")).collect::<Vec<_>>()).to_string())
    }

    /// The settings `names` (comma separated) as `{ name: value }`.
    pub fn settings(&self, names: String) -> String {
        let prefs = self.app.settings.current().unwrap_or_default();
        let v: serde_json::Map<String, Value> = names.split(',').filter_map(|n| Some((n.to_string(), Value::String(nori_core::settings_model::value_of(&prefs, n)?)))).collect();
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
            eq_status: nori_core::settings_model::eq_status(&prefs, false),
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

    /// Song `song`'s menu (JSON, as a page read it), most used first (`menus::song_menu`): `[{ action, enabled,
    /// more, on, id, name, named }]`. `player`: opened from the player, showing `lyrics`.
    pub fn menu(&self, song: String, player: bool, lyrics: bool) -> Result<String, JsValue> {
        use nori_core::menus::{song_menu, SongAction, SongDownload};
        let song: nori_core::Song = serde_json::from_str(&song).map_err(error)?;
        let rules = self.rules();
        let items: Vec<Value> = song_menu(song.clone(), song.starred, SongDownload::None, player, lyrics, rules.stars, rules.account, false, rules.radio, self.jam_offer())
            .into_iter()
            .map(|m| {
                let mut v = json!({ "more": m.more, "enabled": m.enabled });
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
                    a => return json!({ "action": format!("{a:?}"), "more": m.more, "enabled": m.enabled }),
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
