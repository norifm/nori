//! nori music core: the `Core` a platform opens per server profile (database, server address, response
//! parsing into the index) and the domain calls over it. Re-exports the domain crates.

pub mod automix;
pub mod transfers;
pub mod queue;
pub mod bridge;
pub mod history;
pub mod m3u;
pub mod mixes;
pub mod profiles;
pub mod smart;
pub mod actions;
pub mod browse;
pub mod covers;
pub mod search;
pub mod shown;
pub mod client;
pub mod cache_policy;
pub mod race;
pub mod stream;
pub mod autofill;
pub mod car;
pub mod playlist;
pub mod library;
pub mod stage;
pub mod update;
pub mod remote;
pub mod providers;
#[cfg(feature = "neural-beats")]
pub mod model_download;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

pub use nori_db::{self as db, background};
pub use nori_model::{alog, deciders, heap, lines, model, numbers, CoreError, SmartProblem};
pub use nori_automix::beat_model;
pub use nori_devices::{autoeq, outputs};
pub use nori_library::{menus, pages, rows, stars};
pub use nori_lyrics::{formats, look, lrclib, lyrics, services};
pub use nori_net::{api, transport};
pub use nori_perf::perf_log;
pub use nori_queue::{heard, rules, scrobble};
pub use nori_queue::deciders::{button_act, focus_act};
pub use nori_settings::{credits, dsp, lyrics_sources, overrides, settings, settings_model, settings_pages, settings_store};
pub use nori_settings::settings::parse_eq_preset;
pub use nori_model::model::*;
pub use nori_library::pages::{AlbumDetail, ArtistDetail, PlaylistDetail, Starred};

use nori_model::Result;

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ServerConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    /// OpenSubsonic API key; when set it is used instead of user and password.
    pub api_key: Option<String>,
    pub legacy_auth: bool,
}

/// The sign-in a saved profile keeps.
pub fn server_config(p: &settings::SavedServer) -> ServerConfig {
    ServerConfig { url: p.url.clone(), user: p.user.clone(), password: p.password.clone(), api_key: (!p.api_key.is_empty()).then(|| p.api_key.clone()), legacy_auth: p.legacy_auth }
}

/// The addresses, folders and second address's bitrate a saved profile keeps.
pub fn net_profile(p: &settings::SavedServer) -> client::NetProfile {
    client::NetProfile { url: p.url.clone(), alt_url: p.alt_url.clone(), music_folder_ids: p.music_folder_ids.clone(), alt_max_bit_rate: p.alt_max_bit_rate.max(0) as u32 }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Param {
    pub key: String,
    pub value: String,
}

#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Core {
    /// Also its session's profile database while this is the newest core.
    db: Arc<Mutex<Connection>>,
    server: RwLock<api::Server>,
    /// The downloads table in memory and the platform's download reports (transfers.rs).
    downloads: Arc<transfers::Downloads>,
    /// The "For you" row (mixes/board.rs).
    board: Mutex<mixes::board::Board>,
    /// This session's star changes, overlaid on the server's favourites.
    stars: Mutex<stars::StarMarks>,
    /// The favourites the source listed last, under this session's marks.
    listed: Mutex<stars::Listed>,
    remote: Mutex<std::sync::Weak<remote::Remote>>,
    cache_revision: AtomicU64,
    /// A provider's answers this session, by cache key, with when they came ([`Client::kept_stored`]).
    kept: Mutex<std::collections::HashMap<String, cache_policy::Kept>>,
    /// Home as last shown this session: a refresh drops the kept page, and the new feed is merged into
    /// this one.
    home: Mutex<Option<Vec<FeedShelf>>>,
    /// The queue this core saves and restores and its clients fill.
    pub session: Arc<nori_queue::Session>,
    /// The user and the Subsonic secret (password or API key) the LAN's remote control proves the account
    /// with; None for a jam guest.
    account: RwLock<Option<(String, String)>>,
    /// Endpoint and scoped credential of a temporary LAN jam guest.
    lan_jam: RwLock<Option<(String, String)>>,
    /// The user and password Navidrome's own API logs in with; None with an API key.
    native_login: RwLock<Option<(String, String)>>,
    /// What the app offers over the profile (a jam guest's: the host's library, read only).
    rules: RwLock<browse::ProfileRules>,
    /// Told the rules whenever they change ([`Core::watch_rules`]).
    rules_shown: RwLock<Option<Arc<dyn browse::RulesShown>>>,
    /// What the profile plays from (providers/).
    provider: RwLock<providers::Installed>,
    /// The kinds of profile it opens ([`providers::Kinds`]).
    kinds: RwLock<Arc<providers::Kinds>>,
    /// What the stored queue's songs were saved from (queue.rs).
    saved_queue: Mutex<Option<queue::SavedQueue>>,
    /// The server starts a transcode at the second asked for (Navidrome's `timeOffset`), as its answers
    /// said ([`Core::heard_from_server`]).
    starts_transcodes: std::sync::atomic::AtomicBool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct StoredQueue {
    songs: Vec<Song>,
    index: u32,
    position: u64,
    /// Parsed separately so an unknown origin kind loses only the origin.
    origin: serde_json::Value,
    /// Made with each write of the songs; the place belongs to them only with the same stamp, so two
    /// apps over one database never pair one's place with the other's songs.
    stamp: Option<String>,
}

#[derive(Deserialize)]
struct StoredPlace {
    index: u32,
    position: u64,
    stamp: Option<String>,
}

impl Core {
    /// An answer named the server: Navidrome starts a transcode where `timeOffset` asks.
    pub(crate) fn heard_from_server(&self, kind: &str) {
        if !kind.is_empty() {
            self.starts_transcodes.store(kind.eq_ignore_ascii_case("navidrome"), std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Whether a transcode can be asked for from a whole second on ([`Core::heard_from_server`]).
    pub fn starts_transcodes(&self) -> bool {
        self.starts_transcodes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The queue as saved; nothing saved, or a save that no longer parses, is an empty one.
    fn stored_queue(c: &Connection) -> Result<StoredQueue> {
        Ok(db::kv_get(c, "queue")?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    /// The queue's songs, origin and place, in one transaction; returns their new stamp.
    fn write_queue(&self, queue: PlayQueue) -> Result<String> {
        let stamp = nori_remote::new_id();
        let json = serde_json::json!({ "songs": queue.songs, "index": queue.index, "position": queue.position_ms, "origin": queue.origin, "stamp": stamp });
        let c = self.db.lock();
        let tx = c.unchecked_transaction()?;
        db::kv_put(&tx, "queue", &json.to_string())?;
        db::kv_put(&tx, "queue_place", &Self::place_json(queue.index, queue.position_ms, &stamp))?;
        tx.commit()?;
        Ok(stamp)
    }

    /// The current song's index and position in the stored queue, when its songs are still the ones
    /// written with `stamp`; false when another app has written its own since.
    pub(crate) fn save_place(&self, index: u32, position_ms: u64, stamp: &str) -> Result<bool> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("UPDATE kv SET value=?1 WHERE server=sid() AND key='queue_place' AND json_extract(value, '$.stamp')=?2")?;
        Ok(st.execute(params![Self::place_json(index, position_ms, stamp), stamp])? > 0)
    }

    fn place_json(index: u32, position_ms: u64, stamp: &str) -> String {
        serde_json::json!({ "index": index, "position": position_ms, "stamp": stamp }).to_string()
    }

    /// The downloads table in memory and the progress of their work.
    pub fn transfers(&self) -> &transfers::Downloads {
        &self.downloads
    }

    /// [`Core::transfers`], to hold apart from the core.
    pub fn transfers_arc(&self) -> Arc<transfers::Downloads> {
        self.downloads.clone()
    }

    /// Whether an item shows starred: this session's mark over its record's flag (`listed`).
    pub fn starred(&self, kind: client::Starrable, id: &str, listed: bool) -> bool {
        let listed = self.listed.lock().starred(kind, id, listed);
        self.stars.lock().starred(kind, id, listed)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Opens the database at `db_path` (empty: in memory) for server profile `server`, over the app's
    /// queue `session`.
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new(db_path: String, server: String, session: Arc<nori_queue::Session>) -> Result<Arc<Self>> {
        let db = db::open(&db_path, &server)?;
        session.settings.model.set_home(&db_path);
        let db = Arc::new(Mutex::new(db));
        let core = Arc::new(Core {
            downloads: Arc::new(transfers::Downloads::load(&db)?),
            db,
            server: RwLock::new(api::Server::default()),
            board: Mutex::new(mixes::board::Board::default()),
            stars: Mutex::new(stars::StarMarks::default()),
            listed: Mutex::new(stars::Listed::default()),
            remote: Mutex::default(),
            cache_revision: AtomicU64::new(0),
            kept: Mutex::default(),
            home: Mutex::default(),
            session,
            account: RwLock::new(None), lan_jam: RwLock::new(None),
            native_login: RwLock::new(None),
            rules: RwLock::new(browse::profile_rules(false)),
            rules_shown: RwLock::new(None),
            provider: RwLock::default(),
            kinds: RwLock::new(providers::Kinds::builtin()),
            saved_queue: Mutex::new(None),
            starts_transcodes: std::sync::atomic::AtomicBool::new(false),
        });
        core.session.db.set(&core.db);
        Ok(core)
    }

    pub fn local_search(&self, query: String, limit: u32) -> Result<SearchResult> {
        Ok(db::search(&self.db.lock(), &query, limit)?)
    }

    pub fn index_size(&self) -> Result<IngestStats> {
        let c = self.db.lock();
        Ok(IngestStats { artists: db::count(&c, db::ItemKind::Artist)?, albums: db::count(&c, db::ItemKind::Album)?, songs: db::count(&c, db::ItemKind::Song)? })
    }

    pub fn save_queue(&self, queue: PlayQueue) -> Result<()> {
        let mut saved = self.saved_queue.lock();
        *saved = None;
        self.write_queue(queue).map(drop)
    }

    /// Whether the saved queue has songs: what a play button can resume while the app is not running.
    pub fn queue_saved(&self) -> Result<bool> {
        Ok(!Self::stored_queue(&self.db.lock())?.songs.is_empty())
    }

    pub fn load_queue(&self) -> Result<PlayQueue> {
        let (mut q, place) = {
            let c = self.db.lock();
            (Self::stored_queue(&c)?, db::kv_get(&c, "queue_place")?.and_then(|j| serde_json::from_str::<StoredPlace>(&j).ok()))
        };
        if let Some(p) = place.filter(|p| p.stamp == q.stamp) {
            (q.index, q.position) = (p.index, p.position);
        }
        let index = q.index.min(q.songs.len().saturating_sub(1) as u32);
        self.session.register(q.songs.clone());
        let origin = serde_json::from_value::<Option<crate::PageOrigin>>(q.origin).ok().flatten();
        Ok(PlayQueue { songs: q.songs, index, position_ms: q.position, origin })
    }

    /// A sorted, filtered page of indexed songs. `sort`: title, artist, album, year, duration, created,
    /// playCount or userRating; anything else is index order.
    #[allow(clippy::too_many_arguments, reason = "uniffi export: a query's parts cross the FFI as flat arguments")]
    pub fn browse_songs(&self, sort: String, descending: bool, starred_only: bool, year_from: u32, year_to: u32, offset: u32, limit: u32) -> Result<Vec<Song>> {
        let sql = browse::songs_sql(&sort, descending, starred_only, year_to > 0);
        let c = self.db.lock();
        let mut st = c.prepare_cached(&sql)?;
        let map = |r: &rusqlite::Row| r.get::<_, String>(0);
        let rows: Vec<String> = if year_to > 0 {
            st.query_map(params![offset, limit, year_from, year_to], map)?.filter_map(|r| r.ok()).collect()
        } else {
            st.query_map(params![offset, limit], map)?.filter_map(|r| r.ok()).collect()
        };
        Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    /// The ids of [`Self::downloads`], same order.
    pub fn download_ids(&self, done: bool) -> Result<Vec<String>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("SELECT id FROM downloads WHERE server=sid() AND done=?1 ORDER BY ts DESC")?;
        let rows = st.query_map([done], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|id| id.ok()).collect())
    }

    pub fn downloads(&self, done: bool) -> Result<Vec<Song>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("SELECT json FROM downloads WHERE server=sid() AND done=?1 ORDER BY ts DESC")?;
        let rows = st.query_map([done], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|j| serde_json::from_str(&j.ok()?).ok()).collect())
    }

    pub fn autoeq_search(&self, query: String, limit: u32) -> Result<Vec<AutoEqEntry>> {
        Ok(autoeq::search(&self.db.lock(), &query, limit)?)
    }

    pub fn autoeq_count(&self) -> Result<u32> {
        Ok(autoeq::count(&self.db.lock())?)
    }

    pub fn profiles(&self) -> Result<Vec<SoundProfile>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("SELECT name, json, outputs FROM profiles ORDER BY name")?;
        let rows = st.query_map([], |r| {
            Ok(SoundProfile {
                name: r.get(0)?,
                json: r.get(1)?,
                outputs: r.get::<_, String>(2)?.lines().filter(|l| !l.is_empty()).map(str::to_string).collect(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn profile_delete(&self, name: String) -> Result<()> {
        self.db.lock().execute("DELETE FROM profiles WHERE name=?1", [name])?;
        Ok(())
    }

    pub fn search_history(&self) -> Result<Vec<String>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("SELECT query FROM searches WHERE server=sid() ORDER BY ts DESC")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn search_forget(&self) -> Result<()> {
        self.db.lock().execute("DELETE FROM searches WHERE server=sid()", [])?;
        Ok(())
    }
}

impl Core {
    /// The profile now plays from `ident` (a server and user); the library goes when that changed.
    pub(crate) fn serves(&self, ident: &str) -> Result<()> {
        {
            let db = self.db.lock();
            if db::kv_get(&db, "server")?.as_deref() == Some(ident) {
                return Ok(());
            }
            db::clear_library(&db)?;
            db::kv_put(&db, "server", ident)?;
        }
        // What this session made of the old library goes with it.
        self.kept.lock().clear();
        *self.home.lock() = None;
        *self.board.lock() = Default::default();
        *self.stars.lock() = Default::default();
        Ok(())
    }

    /// Drops the index rows other than `kept`: what a whole sync did not find.
    pub(crate) fn prune_index(&self, kept: &std::collections::HashSet<i64>) -> Result<()> {
        Ok(db::prune(&mut self.db.lock(), kept)?)
    }

    pub fn cache_get(&self, key: String) -> Result<Option<Vec<u8>>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("SELECT body FROM cache WHERE server=sid() AND key=?1")?;
        Ok(st.query_row([key], |r| r.get(0)).optional()?)
    }

    /// Whether `key` was stored less than `max_age_ms` ago.
    pub(crate) fn cache_fresh(&self, key: String, max_age_ms: i64) -> Result<bool> {
        let c = self.db.lock();
        let ts: Option<i64> = c.prepare_cached("SELECT ts FROM cache WHERE server=sid() AND key=?1")?.query_row([key], |r| r.get(0)).optional()?;
        Ok(ts.is_some_and(|t| db::now_ms() - t < max_age_ms))
    }

    pub fn cache_put(&self, key: String, body: Vec<u8>) -> Result<()> {
        self.cache_put_checked(key, body, None)
    }

    fn cache_put_checked(&self, key: String, body: Vec<u8>, revision: Option<u64>) -> Result<()> {
        let c = self.db.lock();
        if revision.is_some_and(|r| r != self.cache_revision.load(Ordering::Relaxed)) {
            return Ok(());
        }
        c.prepare_cached("INSERT OR REPLACE INTO cache(server, key, body, ts) VALUES(sid(), ?1, ?2, ?3)")?.execute(params![key, body, db::now_ms()])?;
        Ok(())
    }

    /// Drops cached responses whose key starts with `prefix`.
    pub(crate) fn cache_evict(&self, prefix: String) -> Result<()> {
        let c = self.db.lock();
        c.execute("DELETE FROM cache WHERE server=sid() AND key >= ?1 AND key < ?1 || x'ff'", [&prefix])?;
        self.kept.lock().retain(|k, _| !k.starts_with(&prefix));
        self.cache_revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Kept answers whose key starts with `prefix` are stale: still shown (offline too) until the
    /// source answers anew.
    pub(crate) fn cache_stale(&self, prefix: &str) -> Result<()> {
        let c = self.db.lock();
        c.execute("UPDATE cache SET ts = 0 WHERE server=sid() AND key >= ?1 AND key < ?1 || x'ff'", [prefix])?;
        self.kept.lock().iter_mut().filter(|(k, _)| k.starts_with(prefix)).for_each(|(_, kept)| kept.at = 0);
        self.cache_revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn cache_drop(&self, key: &str) -> Result<()> {
        self.db.lock().execute("DELETE FROM cache WHERE server=sid() AND key=?1", [key])?;
        Ok(())
    }

    pub fn download_remove(&self, id: String) -> Result<()> {
        self.download_settle(vec![id], vec![false])
    }

    /// AutoEQ curves matching an output device name, best first.
    pub(crate) fn autoeq_for_device(&self, device: String, limit: u32) -> Result<Vec<AutoEqEntry>> {
        Ok(autoeq::matching(&self.db.lock(), &device, limit)?)
    }

    pub fn profile_save(&self, profile: SoundProfile) -> Result<()> {
        let c = self.db.lock();
        c.prepare_cached("INSERT OR REPLACE INTO profiles(name, json, outputs) VALUES(?1, ?2, ?3)")?
            .execute(params![profile.name, profile.json, profile.outputs.join("\n")])?;
        Ok(())
    }

    /// Binds `output` to profile `name` only (None: unbinds it).
    pub(crate) fn profile_bind(&self, output: String, name: Option<String>) -> Result<()> {
        let mut c = self.db.lock();
        let tx = c.transaction()?;
        let rows: Vec<(String, String)> = {
            let mut st = tx.prepare("SELECT name, outputs FROM profiles")?;
            let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            r
        };
        for (profile, outputs) in rows {
            let mut list: Vec<&str> = outputs.lines().filter(|l| !l.is_empty() && *l != output).collect();
            if name.as_deref() == Some(profile.as_str()) {
                list.push(&output);
            }
            let joined = list.join("\n");
            if joined != outputs {
                tx.execute("UPDATE profiles SET outputs=?1 WHERE name=?2", params![joined, profile])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The profile bound to `output`, if any.
    pub(crate) fn profile_for_output(&self, output: String) -> Result<Option<SoundProfile>> {
        Ok(self.profiles()?.into_iter().find(|p| p.outputs.contains(&output)))
    }

    pub fn search_remember(&self, query: String) -> Result<()> {
        let c = self.db.lock();
        c.execute("INSERT OR REPLACE INTO searches(server, query, ts) VALUES(sid(), ?1, ?2)", params![query.trim(), db::now_ms()])?;
        c.execute("DELETE FROM searches WHERE server=sid() AND query NOT IN (SELECT query FROM searches WHERE server=sid() ORDER BY ts DESC LIMIT 20)", [])?;
        Ok(())
    }
}

#[cfg(test)]
impl Core {
    /// Marks one queued download finished.
    pub(crate) fn download_done(&self, id: String) -> Result<()> {
        self.download_settle(vec![id], vec![true])
    }
}

#[cfg(test)]
pub mod tests;
