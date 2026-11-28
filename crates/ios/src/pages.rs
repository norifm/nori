//! The coarse path: one page per call, answered as JSON on a callback, cached copy first and the
//! server's after it when it differs. Lists of songs a page showed are kept by the request's token so a
//! tap plays from them without sending songs back across.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nori_core::browse::{album_sort_direction, album_sort_kept, album_sort_saved, album_sorts, artist_rows, favorites_in_order, home_pinned, home_row_more, home_row_page, next_album_page, shelf_albums_read, song_sort_direction, song_sort_kept, song_sort_saved, song_sorts, AlbumSort, HomeShelf, SortDirection};
use nori_core::cache_policy::{Page, Read};
use nori_core::home::HomePlace;
use nori_core::numbers::{self, POINT};
use nori_core::client::Starrable;
use nori_core::menus::{download_entries, page_actions, playlist_edits, DownloadAct, PageAction, PageKind, PlaylistEdit, SongDownload};
use nori_core::pages::release_groups;
use nori_core::mixes::board::{MixLookup, MixTile, FAVOURITES_MIX};
use nori_core::stage::QueueRows;
use nori_core::{Album, Artist, Lyrics, OriginKind, PageOrigin, Playlist, Song};
use nori_core::lyrics_follow::LyricsNow;
use nori_core::search::SearchShown;
use nori_covers::loader::Ticket;
use nori_engine::State;
use nori_host::session::{read_pages, read_pages_in, Chore};
use nori_host::pages::{HomePart, ShelfItems};
use nori_host::Fetch;
use nori_core::settings::HomeRow;
use nori_core::setup::{setup_answers, setup_covers, setup_settings, SetupAnswer, SetupStep};
use serde_json::{json, Value};

use crate::paging;
use crate::session::{c_text, server, with_session};

pub const PAGE_HOME: i32 = 1;
pub const PAGE_ALBUMS: i32 = 2;
/// [`PAGE_ALBUMS`]' `arg` for the favorite albums only, all on one page.
const ALBUMS_FAVORITES: &str = "favorites";
pub const PAGE_ARTISTS: i32 = 3;
pub const PAGE_PLAYLISTS: i32 = 4;
pub const PAGE_SONGS: i32 = 5;
pub const PAGE_GENRES: i32 = 6;
pub const PAGE_DOWNLOADS: i32 = 7;
pub const PAGE_ALBUM: i32 = 8;
pub const PAGE_ARTIST: i32 = 9;
pub const PAGE_PLAYLIST: i32 = 10;
pub const PAGE_GENRE: i32 = 11;
pub const PAGE_QUEUE: i32 = 12;
pub const PAGE_SEARCH: i32 = 13;
pub const PAGE_STARRED: i32 = 14;

/// Albums asked per list page.
const ALBUM_PAGE: i32 = 500;
/// Song lists kept for taps; older pages fall off.
const KEPT_LISTS: usize = 24;

pub const PAGE_MIX: i32 = 15;
/// The smart playlists list (built-ins and saved).
pub const PAGE_SMARTS: i32 = 16;
/// One smart playlist by id: its songs.
pub const PAGE_SMART: i32 = 17;
pub const PAGE_DOWNLOADED_ALBUM: i32 = 18;
/// A Home row in full: `arg` is its shelf's key, then a newline and the offset of a page past the first.
pub const PAGE_HOME_ROW: i32 = 19;
/// This iPod's listening history: `arg` empty for its newest page.
pub const PAGE_HISTORY: i32 = 20;
/// The listening stats: `arg` as [`stats`] reads it.
pub const PAGE_STATS: i32 = 21;

pub type PageFn = unsafe extern "C" fn(u64, *const c_char);
/// `(token, width, height, rgba, len, owner, stand_in)`: the pixels stay valid until `owner` is given to
/// [`nori_ios_cover_release`], so the app draws from them without a copy; `stand_in`: a picture that
/// shows while the cover's own comes, which comes after it.
pub type CoverFn = unsafe extern "C" fn(u64, u32, u32, *const u8, usize, *mut std::ffi::c_void, bool);

/// What keeps a handed-out picture's pixels alive.
type Keep = Box<dyn std::any::Any + Send>;

/// Hands `keep`'s pixels (`pixels` of it) to the cover callback, as a stand-in with `stand_in`; without a
/// callback they are dropped.
fn hand<T: Send + 'static>(token: u64, width: u32, height: u32, keep: T, pixels: fn(&T) -> &[u8], stand_in: bool) {
    let Some(cb) = *lock(&COVER_HOOK) else { return };
    let (at, len) = {
        let p = pixels(&keep);
        (p.as_ptr(), p.len())
    };
    // Moving `keep` into the box does not move the heap buffer `at` points into.
    let owner = Box::into_raw(Box::new(Box::new(keep) as Keep)).cast::<std::ffi::c_void>();
    unsafe { cb(token, width, height, at, len, owner, stand_in) };
}

/// Lets go of a picture the cover callback was handed.
///
/// # Safety
/// `owner` came with a cover callback and is released once.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_cover_release(owner: *mut std::ffi::c_void) {
    if !owner.is_null() {
        // SAFETY: made by `Box::into_raw` in `hand` (the caller's promise).
        drop(unsafe { Box::from_raw(owner.cast::<Keep>()) });
    }
}

// ABI tests share callbacks and retained lists.
#[cfg(test)]
pub(crate) static ABI_IN_USE: Mutex<()> = Mutex::new(());

static PAGE_HOOK: Mutex<Option<PageFn>> = Mutex::new(None);
static COVER_HOOK: Mutex<Option<CoverFn>> = Mutex::new(None);

struct List {
    token: u64,
    songs: Vec<Song>,
    origin: Option<PageOrigin>,
    /// The playlist it is, when the profile may change it (the core's `playlist_editable_in`).
    editable: Option<Playlist>,
}

static LISTS: Mutex<VecDeque<List>> = Mutex::new(VecDeque::new());
static TICKETS: Mutex<Vec<(u64, Ticket)>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn keep_list(token: u64, songs: Vec<Song>, origin: Option<PageOrigin>) {
    keep(List { token, songs, origin, editable: None });
}

fn keep(list: List) {
    let mut lists = lock(&LISTS);
    lists.retain(|l| l.token != list.token);
    lists.push_back(list);
    while lists.len() > KEPT_LISTS {
        lists.pop_front();
    }
}

/// What row `index` of list `token` offers to change, when it is a playlist the account may change.
pub(crate) fn list_edits(token: u64, index: u32) -> Vec<PlaylistEdit> {
    lock(&LISTS).iter().find(|l| l.token == token).map_or_else(Vec::new, |l| playlist_edits(index, l.songs.len() as u32, l.editable.is_some()))
}

/// The change `edit` of row `index` of list `token` makes, when it is a playlist the profile may change.
pub(crate) fn list_edit(token: u64, index: u32, edit: PlaylistEdit) -> Option<(Playlist, nori_host::session::PlaylistChange)> {
    let lists = lock(&LISTS);
    let l = lists.iter().find(|l| l.token == token)?;
    let songs = l.songs.iter().map(|s| s.id.clone()).collect();
    Some((l.editable.clone()?, nori_host::session::PlaylistChange::Edit { songs, index, edit }))
}

/// The session closed: its lists name another server's songs.
pub(crate) fn forget_lists() {
    lock(&LISTS).clear();
}

pub(crate) fn list(token: u64) -> Option<(Vec<Song>, Option<PageOrigin>)> {
    lock(&LISTS)
        .iter()
        .find(|l| l.token == token)
        .map(|l| (l.songs.clone(), l.origin.clone()))
}

/// The page list `token` was read for.
pub(crate) fn list_origin(token: u64) -> Option<PageOrigin> {
    lock(&LISTS).iter().find(|l| l.token == token)?.origin.clone()
}

/// Song `i` of list `token`, copying only that song.
pub(crate) fn list_song(token: u64, i: usize) -> Option<Song> {
    lock(&LISTS).iter().find(|l| l.token == token)?.songs.get(i).cloned()
}

/// Appends a page to list `token`, now the newest list. Its length before, or None when it is not kept.
pub(crate) fn extend_list(token: u64, more: &[Song]) -> Option<usize> {
    let mut lists = lock(&LISTS);
    let at = lists.iter().position(|l| l.token == token)?;
    let mut l = lists.remove(at)?;
    let from = l.songs.len();
    l.songs.extend_from_slice(more);
    lists.push_back(l);
    Some(from)
}

pub(crate) fn send(token: u64, v: &Value) {
    let Some(cb) = *lock(&PAGE_HOOK) else { return };
    let marked = v.get("sections").and_then(|_| with_session(|s| with_source_marks(s, with_download_marks(s, v.clone()))));
    let text = CString::new(marked.as_ref().unwrap_or(v).to_string().replace('\0', " ")).unwrap_or_default();
    unsafe { cb(token, text.as_ptr()) };
}

/// `v`'s song rows with their download marks, `dl` (menus.rs's `download_glyphs`): 1 on its way, 2
/// downloaded, none without a mark. A collection's page has its head say when every song is downloaded.
fn with_download_marks(s: &nori_host::session::Session, mut v: Value) -> Value {
    use nori_core::menus::DownloadGlyph;
    let page = matches!(v["head"]["k"].as_str(), Some("album" | "downloadedAlbum" | "playlist" | "mix" | "smart"));
    let Some(sections) = v["sections"].as_array_mut() else { return v };
    let mut rows: Vec<&mut Value> = sections.iter_mut().filter_map(|x| x["items"].as_array_mut()).flatten().filter(|r| r["k"] == "song").collect();
    let ids: Vec<nori_core::ItemKey> = rows.iter().map(|r| row_key(r)).collect();
    let marks = s.download_glyphs(&ids, page);
    for (row, mark) in rows.iter_mut().zip(marks) {
        match mark {
            DownloadGlyph::Done => row["dl"] = json!(2),
            DownloadGlyph::Ring | DownloadGlyph::Failed => row["dl"] = json!(1),
            DownloadGlyph::None => {}
        }
    }
    v
}

/// `v`'s items the core marks (`marked`) with their source's circle, `mark`: `{icon, letter}` as the
/// profile lists show it, of the source `src` names (absent: the profile's own).
fn with_source_marks(s: &nori_host::session::Session, mut v: Value) -> Value {
    let Some(sections) = v["sections"].as_array_mut() else { return v };
    let prefs = s.core.session.settings.current().unwrap_or_default();
    for row in sections.iter_mut().filter_map(|x| x["items"].as_array_mut()).flatten().filter(|r| r["marked"] == true) {
        let source = row["src"].as_str().map(String::from);
        if let Some(sv) = nori_core::settings::source_profile(prefs.servers.clone(), prefs.active_server_id.clone(), source) {
            let letter = nori_core::settings::profile_letter(sv.clone(), nori_core::settings::label(&sv.name, &sv.url));
            row["mark"] = json!({ "icon": sv.icon.id(), "letter": letter });
        }
    }
    v
}

fn send_error(token: u64, e: &nori_core::transport::NetError) {
    send(token, &failure(e));
}

/// A failure as a page's answer says it: its code and detail.
fn failure(e: &nori_core::transport::NetError) -> Value {
    let (code, detail) = crate::account::fail(e.clone());
    json!({ "error": code, "detail": detail.unwrap_or_default() })
}

/// A failed server search, kept as its fallback's reason in the page's own words ([`failure`]).
fn search_failure(e: &nori_core::transport::NetError) -> String {
    failure(e).to_string()
}

fn cover(c: &Option<String>) -> &str {
    c.as_deref().unwrap_or("")
}

/// An album, artist, playlist or song as the app hands it back: its id, and where it is kept when that
/// is not the profile's own source: another source, or this device for a playlist of nori's own.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ItemRef {
    pub(crate) id: String,
    pub(crate) source: Option<String>,
    pub(crate) mixed: bool,
}

impl ItemRef {
    /// What the app is given for the item: the bare id for one of the profile's own source, else this as
    /// JSON, so a profile of one source crosses as before.
    pub(crate) fn sent(id: &str, source: &Option<String>, mixed: bool) -> String {
        match (source, mixed) {
            (None, false) => id.to_string(),
            _ => json!({ "id": id, "source": source, "mixed": mixed }).to_string(),
        }
    }

    /// What the app handed back, as [`ItemRef::sent`] gave it: JSON, or a bare id of the profile's own.
    pub(crate) fn taken(arg: &str) -> ItemRef {
        match serde_json::from_str::<Value>(arg) {
            Ok(v) if v.is_object() => ItemRef {
                id: v["id"].as_str().unwrap_or_default().to_string(),
                source: v["source"].as_str().map(String::from),
                mixed: v["mixed"].as_bool().unwrap_or(false),
            },
            _ => ItemRef { id: arg.to_string(), ..ItemRef::default() },
        }
    }

    /// The playlist it names, as far as changing it needs.
    pub(crate) fn playlist(self) -> Playlist {
        Playlist { id: self.id, source: self.source, mixed: self.mixed, ..Playlist::default() }
    }

    fn at(&self) -> nori_core::sources::PlaylistAt {
        nori_core::sources::playlist_at(&self.clone().playlist())
    }
}

/// An item's link to album or artist `id` of `source` ([`ItemRef::sent`]); empty for none.
fn link(id: &Option<String>, source: &Option<String>) -> String {
    id.as_deref().map(|id| ItemRef::sent(id, source, false)).unwrap_or_default()
}

/// A song row's key, from its `src` and `id`.
fn row_key(row: &Value) -> nori_core::ItemKey {
    nori_core::ItemKey { source: row["src"].as_str().map(String::from), id: row["id"].as_str().unwrap_or_default().to_string() }
}

/// A song as the app lists it; `key` is what tells it apart in the queue and the player (`ItemKey::text`).
pub(crate) fn song_json(s: &Song, i: usize) -> Value {
    json!({
        "k": "song", "id": s.id, "key": nori_core::Keyed::key(s).text(), "ref": ItemRef::sent(&s.id, &s.source, false), "t": s.title, "s": s.artist, "a": s.album,
        "albumId": link(&s.album_id, &s.source), "artistId": link(&s.artist_id, &s.source),
        "c": cover(&s.cover_art), "d": s.duration, "i": i, "x": s.is_external, "n": s.track,
        "disc": s.disc_number, "st": s.starred, "marked": s.source_marked, "src": s.source,
    })
}

fn album_json(a: &Album) -> Value {
    let sub = if a.subtitle.is_empty() {
        &a.artist
    } else {
        &a.subtitle
    };
    json!({
        "k": "album", "id": a.id, "ref": ItemRef::sent(&a.id, &a.source, false), "t": a.name, "s": sub, "c": cover(&a.cover_art), "y": a.year,
        "x": a.is_external, "artistId": link(&a.artist_id, &a.source), "marked": a.source_marked, "src": a.source,
    })
}

/// An artist's page: a shelf per kind of release (`pages::release_groups`; `kind` its `ReleaseKind` and
/// `tag` the server's for a tagged one, for the app to word), then the songs its provider lists, kept for
/// `token` to play from a tap.
/// An artist's page, with its about part once read: the biography and the pages to read on first (key
/// `about`: a `bio` item, then a `link` item each, its address the id and which page its `t`), the similar
/// artists last (key `similar`).
fn artist_page(token: u64, v: &nori_core::ArtistDetail, about: Option<&nori_core::pages::ArtistAbout>) -> Value {
    let a = &v.artist;
    keep_list(token, v.songs.clone(), Some(Fetch::Artist(a.id.clone(), a.source.clone()).origin()));
    let mut sections: Vec<Value> = Vec::new();
    if let Some(about) = about {
        let bio = about.biography.iter().map(|b| json!({ "k": "bio", "id": "", "t": b }));
        let links = [("lastFm", &about.last_fm), ("musicBrainz", &about.musicbrainz)].into_iter().filter_map(|(n, url)| Some(json!({ "k": "link", "id": url.as_deref()?, "t": n })));
        sections.push(section("about", false, bio.chain(links).collect()));
    }
    sections.extend(release_groups(&v.albums)
        .into_iter()
        .map(|g| {
            let mut s = section("release", true, g.albums.iter().filter_map(|&i| v.albums.get(i as usize)).map(album_json).collect());
            s["kind"] = json!(g.kind as u8);
            s["tag"] = json!(g.tag);
            s
        }));
    if !v.songs.is_empty() {
        sections.push(songs_section("songs", &v.songs, 0));
    }
    if let Some(about) = about.filter(|a| !a.similar.is_empty()) {
        sections.push(section("similar", true, about.similar.iter().map(artist_json).collect()));
    }
    json!({
        "head": {
            "k": "artist", "id": a.id, "ref": ItemRef::sent(&a.id, &a.source, false), "t": a.name, "c": cover(&a.cover_art), "n": v.albums.len(), "ns": v.songs.len(), "st": a.starred,
            "albumSongs": v.release_songs.unwrap_or_default(),
        },
        "sections": sections,
    })
}

fn album_page(v: &nori_core::AlbumDetail) -> Value {
    let a = &v.album;
    json!({
        "head": {
            "k": "album", "id": a.id, "ref": ItemRef::sent(&a.id, &a.source, false), "t": a.name, "s": a.artist,
            "artistId": link(&a.artist_id, &a.source), "c": cover(&a.cover_art),
            "y": a.year, "g": a.genre.as_deref().unwrap_or(""), "sec": v.seconds,
            "n": v.songs.len(), "st": a.starred,
        },
        "sections": [songs_section("songs", &v.songs, 0)],
    })
}

fn artist_json(a: &Artist) -> Value {
    json!({ "k": "artist", "id": a.id, "ref": ItemRef::sent(&a.id, &a.source, false), "t": a.name, "n": a.album_count, "c": cover(&a.cover_art), "x": a.is_external, "marked": a.source_marked, "src": a.source })
}

/// A playlist row; `s` is its own line (a provider's, who made it), for one not counted until opened.
fn playlist_json(p: &Playlist) -> Value {
    let mut v = json!({ "k": "playlist", "id": p.id, "ref": ItemRef::sent(&p.id, &p.source, p.mixed), "t": p.name, "n": p.song_count, "d": p.duration, "c": cover(&p.cover_art), "marked": p.source_marked, "src": p.source });
    if let Some(line) = &p.comment {
        v["s"] = json!(line);
    }
    v
}

/// A smart playlist row: `n` is the built-in's [`SmartBuiltin`] for the client to word, or absent when named.
fn smart_json(p: &nori_core::SmartPlaylist) -> Value {
    let mut v = json!({ "k": "smart", "id": p.id, "t": p.name });
    if let Some(b) = p.builtin {
        v["n"] = json!(b as u8);
    }
    v
}

/// The definition of smart playlist `id`: a saved one, or a built-in.
fn smart_of(core: &nori_core::Core, id: &str) -> Option<nori_core::SmartPlaylist> {
    if let Ok(saved) = core.smart_list() {
        if let Some(p) = saved.into_iter().find(|p| p.id == id) {
            return Some(p);
        }
    }
    nori_core::smart::smart_defaults()
        .into_iter()
        .find(|p| p.id == id)
}

/// A "For you" tile: `n` is the mix's [`MixName`] for the client to word.
fn mix_json(t: &MixTile) -> Value {
    json!({
        "k": "mix", "id": t.id, "n": t.name as u8, "c": t.covers.first().map_or("", String::as_str),
        "covers": t.covers,
    })
}

fn genre_json(g: &nori_core::browse::GenreTile) -> Value {
    json!({ "k": "genre", "id": g.name, "t": g.name, "n": g.song_count, "c": g.cover_art })
}

/// The period, the source and whether to ask the server's new plays first, from [`stats`]' `arg`.
fn stats_asked(arg: &str) -> (nori_core::browse::StatsPeriod, nori_core::browse::StatsSource, bool) {
    use nori_core::browse::{StatsPeriod, StatsSource};
    let mut words = arg.split('\n');
    let period = words.next().and_then(|n| StatsPeriod::ALL.into_iter().find(|p| format!("{p:?}") == n)).unwrap_or(StatsPeriod::OPENED);
    let source = words.next().and_then(|n| StatsSource::ALL.into_iter().find(|s| format!("{s:?}") == n)).unwrap_or(StatsSource::Device);
    (period, source, words.next() == Some("1"))
}

/// The listening stats for `arg` (`period\nsource\nsync`, each by its core name; empty: the ones the page
/// opens on): the choices and the one taken, how the server account's plays stood when asked for (with
/// `sync`), the plays, the numbers under them, each hour's share of the busiest and the busiest hour and
/// weekday, and the top lists.
fn stats(client: &nori_core::client::Client, core: &nori_core::Core, arg: &str) -> Value {
    use nori_core::browse::{StatsPeriod, StatsSource};
    let (period, source, sync) = stats_asked(arg);
    let server = sync.then(|| format!("{:?}", nori_core::transport::block_on(client.server_history_sync())));
    let now = nori_core::db::now_ms();
    let tz = (nori_core::library::local_offset_s(now / 1000) * 1000) as i32;
    let p = match core.stats_page(period, source, tz) {
        Ok(p) => p,
        Err(e) => return json!({ "error": crate::account::LOGIN_DATABASE, "detail": e.to_string() }),
    };
    let entries = |v: &[nori_core::TopEntry]| v.iter().map(|e| json!({ "t": e.name, "n": e.plays, "ms": e.listened_ms, "c": e.cover_art })).collect::<Vec<_>>();
    let st = &p.stats;
    json!({
        "periods": StatsPeriod::ALL.map(|p| format!("{p:?}")),
        "sources": StatsSource::ALL.map(|s| format!("{s:?}")),
        "period": format!("{period:?}"),
        "source": format!("{source:?}"),
        "server": server,
        "plays": st.plays,
        "ms": st.listened_ms,
        "tiles": p.tiles.iter().map(|t| json!({ "k": format!("{:?}", t.kind), "n": t.value })).collect::<Vec<_>>(),
        "hours": p.hours,
        "hour": p.busiest_hour,
        "weekday": p.busiest_weekday,
        "songs": st.top_songs.iter().map(|t| json!({ "t": t.song.title, "s": t.song.artist, "n": t.plays, "c": t.song.cover_art })).collect::<Vec<_>>(),
        "artists": entries(&st.top_artists),
        "albums": entries(&st.top_albums),
        "genres": entries(&st.top_genres),
    })
}

fn section(key: &str, grid: bool, items: Vec<Value>) -> Value {
    json!({ "key": key, "grid": grid, "items": items })
}

/// A feed's shelves as titled sections (key `feed`); their songs are kept as one list for `token`.
/// A source's own Home shelf as sections, titled as it titles it: its songs (after `songs`, the list taps
/// play from), its albums and its playlists.
fn feed_shelf(f: &nori_core::FeedShelf, songs: &mut Vec<Song>) -> Vec<Value> {
    let titled = |mut v: Value| {
        v["title"] = json!(f.title);
        v
    };
    let mut out = Vec::new();
    if !f.songs.is_empty() {
        out.push(titled(songs_section("feed", &f.songs, songs.len())));
        songs.extend(f.songs.iter().cloned());
    }
    if !f.albums.is_empty() {
        out.push(titled(section("feed", true, f.albums.iter().map(album_json).collect())));
    }
    if !f.playlists.is_empty() {
        out.push(titled(section("feed", true, f.playlists.iter().map(playlist_json).collect())));
    }
    out
}

fn songs_section(key: &str, songs: &[Song], from: usize) -> Value {
    section(
        key,
        false,
        songs
            .iter()
            .enumerate()
            .map(|(i, s)| song_json(s, from + i))
            .collect(),
    )
}

/// The search's kinds as the page names its sections and chips.
fn kind_key(kind: nori_core::search::SearchKind) -> &'static str {
    use nori_core::search::SearchKind;
    match kind {
        SearchKind::Songs => "songs",
        SearchKind::Albums => "albums",
        SearchKind::Artists => "artists",
        SearchKind::Playlists => "playlists",
    }
}

/// A search's view as the page draws it: the core's layout as sections (the top result's keyed `top`,
/// then a kind's each), the kind chips after All (`kinds`) and the one lit (`kind`, null for All). Its
/// songs are kept for `token` as the search's, so a tap plays the one tapped alone.
fn found(token: u64, v: &nori_core::search::SearchView, server: bool) -> Value {
    use nori_core::search::SearchKind;
    let r = v.answer.result().cloned().unwrap_or_default();
    keep_list(token, r.songs.clone(), Some(PageOrigin::new(OriginKind::Search, v.query.as_str())));
    let item = |kind: SearchKind, at: u32| {
        let at = at as usize;
        match kind {
            SearchKind::Songs => song_json(&r.songs[at], at),
            SearchKind::Albums => album_json(&r.albums[at]),
            SearchKind::Artists => artist_json(&r.artists[at]),
            SearchKind::Playlists => playlist_json(&r.playlists[at]),
        }
    };
    let top = v.layout.top.map(|t| section("top", false, vec![item(t.kind, t.at)]));
    let sections: Vec<Value> = top.into_iter().chain(v.layout.sections.iter().map(|s| section(kind_key(s.kind), false, s.items.iter().map(|&at| item(s.kind, at)).collect()))).collect();
    json!({
        "server": server,
        "sections": sections,
        "kinds": v.kinds.iter().map(|&k| kind_key(k)).collect::<Vec<_>>(),
        "kind": v.kind.map(kind_key),
    })
}

/// A page of an album grid from `offset`, or with `favorites` the starred albums whole in `sort`'s order:
/// more follow while pages come full (`browse::next_album_page`).
fn album_grid(token: u64, client: &nori_core::client::Client, sort: AlbumSort, favorites: bool, offset: i32) {
    let at = paging::From::Offset(offset as u32);
    if offset == 0 {
        paging::first(token, at);
    }
    let read = if favorites {
        Read::StarredItems
    } else {
        Read::AlbumList { kind: sort, size: ALBUM_PAGE, offset, genre: None }
    };
    let read = pages_sent(client, token, read, &|p| {
        let v = match p {
            Page::Albums { v } => v,
            Page::StarredPage { v } => favorites_in_order(v.albums, sort, false),
            _ => return None,
        };
        let sections = if sort == AlbumSort::ArtistRows {
            artist_shelves(&v)
        } else {
            vec![section("albums", true, v.iter().map(album_json).collect())]
        };
        Some(json!({
            "more": paging::answered(token, at, next_album_page(sort, favorites, offset as u32, ALBUM_PAGE as u32, v.len() as u32).map(paging::From::Offset)),
            "from": offset,
            "sections": sections,
        }))
    });
    if !read {
        paging::failed(token, at);
    }
}

/// Albums in [`AlbumSort::ArtistRows`] order as a shelf per artist (`browse::artist_rows`), each titled
/// with its artist (`artistId` to open them) and counted in `n`. A page that goes on with the last
/// page's artist starts with that artist's shelf again, for the app to join.
fn artist_shelves(albums: &[Album]) -> Vec<Value> {
    let starts = artist_rows(albums.to_vec());
    starts
        .iter()
        .enumerate()
        .map(|(n, &from)| {
            let to = starts.get(n + 1).map_or(albums.len(), |&t| t as usize);
            let row = &albums[from as usize..to];
            let mut s = section("artistRow", true, row.iter().map(album_json).collect());
            s["title"] = json!(row[0].artist);
            s["artistId"] = json!(link(&row[0].artist_id, &row[0].source));
            s
        })
        .collect()
}

/// Hands the server's starred songs to the favourites mix when the stored copy (already handed over) is
/// stale or missing. True when they changed.
fn refresh_favourites(
    client: &nori_core::client::Client,
    stored: nori_core::client::NetResult<nori_core::library::FavouritesHanded>,
) {
    let digest = match stored {
        Ok(f) if f.fresh => return,
        Ok(f) => f.digest,
        Err(_) => None,
    };
    let _ = nori_core::transport::block_on(client.mix_favourites_refresh(digest));
}

/// Each Home row with its shelf's key, which the app words and opens the row's full view by.
const SHELVES: [(HomeRow, &str); 8] = [
    (HomeRow::Pinned, "pinned"),
    (HomeRow::Playlists, "playlists"),
    (HomeRow::Recent, "recent"),
    (HomeRow::Newest, "newest"),
    (HomeRow::Frequent, "frequent"),
    (HomeRow::TopSongs, "topSongs"),
    (HomeRow::Random, "random"),
    (HomeRow::Starred, "starred"),
];

fn shelf_key(row: HomeRow) -> &'static str {
    SHELVES.iter().find(|(r, _)| *r == row).map_or("", |(_, k)| k)
}

fn shelf_row(key: &str) -> Option<HomeRow> {
    SHELVES.iter().find(|(_, k)| *k == key).map(|(r, _)| *r)
}

/// A page of Home row `row`'s full view from `offset` (`browse::home_row_page`): its albums or playlists
/// as a grid, its songs as a list kept for `token`; `more` while a page came full.
fn home_row(token: u64, client: &nori_core::client::Client, core: &nori_core::Core, row: HomeRow, offset: u32) {
    let Some(shelf) = home_row_page(row, offset) else {
        return send(token, &json!({ "sections": [], "from": offset }));
    };
    let key = shelf_key(row);
    let at = paging::From::Offset(offset);
    if offset == 0 {
        paging::first(token, at);
    }
    // Whether more may follow a page from `offset` that brought `got`.
    let more = |got: usize| paging::answered(token, at, home_row_more(shelf.clone(), got as u32).then_some(paging::From::Offset(offset + got as u32)));
    match shelf.clone() {
        HomeShelf::Albums { sort, size, follows_stars } => {
            let read = pages_sent(client, token, shelf_albums_read(sort, size, follows_stars, offset), &|p| match p {
                Page::Albums { v } => Some(json!({
                    "more": more(v.len()), "from": offset,
                    "sections": [section(key, true, v.iter().map(album_json).collect())],
                })),
                _ => None,
            });
            if !read {
                paging::failed(token, at);
            }
        }
        HomeShelf::Songs { sort, descending, limit } => {
            let songs = core.browse_songs(sort, descending, false, 0, 0, offset, limit).unwrap_or_default();
            let from = if offset == 0 { None } else { extend_list(token, &songs) }.unwrap_or(0);
            let v = json!({
                "more": more(songs.len()), "from": offset,
                "sections": [songs_section(key, &songs, from)],
            });
            if offset == 0 {
                keep_list(token, songs, Some(PageOrigin::new(OriginKind::Shelf, key)));
            }
            send(token, &v);
        }
        HomeShelf::Playlists { take } => pages(client, token, Read::PlaylistList, &|p| match p {
            Page::Playlists { v } => Some(json!({ "sections": [section(key, true, v.iter().take(take as usize).map(playlist_json).collect())] })),
            _ => None,
        }),
        HomeShelf::Pinned => {
            let pins = core.session.settings.prefs(|p| p.pinned_playlists.clone());
            pages(client, token, Read::PlaylistList, &|p| match p {
                Page::Playlists { v } => Some(json!({ "sections": [section(key, true, home_pinned(v, pins.clone()).iter().map(playlist_json).collect())] })),
                _ => None,
            })
        }
    }
}

/// A shelf of covers; songs are a list.
fn shelf_grid(row: HomeRow) -> bool {
    row != HomeRow::TopSongs
}

/// The running downloads' facts on their rows: `pct` whole percent (-1 unknown), `bps` bytes a second
/// (0 unknown), `eta` seconds left (-1 unknown). A song saved and still processing has none.
fn with_progress(core: &nori_core::Core, rows: &mut Value) {
    let Some(items) = rows["items"].as_array_mut() else { return };
    core.transfers().with(|t| {
        for v in items {
            if let (_, Some(f)) = t.row(&row_key(v)) {
                v["pct"] = json!(f.percent);
                v["bps"] = json!(f.speed_bps);
                v["eta"] = json!(f.eta_s);
            }
        }
    });
}

/// What the downloads are doing, for the Downloads page's card: `stage` "download" with the batch's facts
/// (`notice` 0 waiting for a network, 1 one song named, 2 one song, 3 several), or "work" with the saved
/// songs still waiting for each step; `stages` and `at` lay out its bar (the bytes, then the work after).
fn work_json(p: &nori_core::transfers::Progress) -> Value {
    use nori_core::transfers::{NoticeKind, Progress};
    match p {
        Progress::Fetching(n) => {
            let notice = match n.kind {
                NoticeKind::Waiting => 0,
                NoticeKind::OneNamed => 1,
                NoticeKind::One => 2,
                NoticeKind::Many => 3,
            };
            json!({ "stage": "download", "notice": notice, "position": n.position, "total": n.total, "song": n.current, "artist": n.artist, "album": n.label,
                    "permille": n.permille, "eta": n.eta_s, "stages": 1, "at": 0 })
        }
        Progress::Processing { work: w, after_fetching } => {
            let at = u8::from(*after_fetching);
            json!({ "stage": "work", "lyrics": w.lyrics, "analysing": w.analysing, "beats": w.beats, "permille": w.permille, "eta": w.eta_s, "stages": 1 + at, "at": at })
        }
    }
}

fn list_prefs() -> HashMap<String, String> {
    with_session(|s| s.core.session.settings.prefs(|p| p.list_prefs.clone())).unwrap_or_default()
}

/// Album `it`'s page from its source: the stored copy, then the source's.
fn album(client: &Arc<nori_core::client::Client>, token: u64, it: ItemRef) {
    pages_from(token, &mut |got| nori_host::session::read_pages_in(client, it.source.clone(), Read::AlbumById { id: it.id.clone() }, got), &|p| match p {
        Page::AlbumPage { v } => {
            keep_list(token, v.songs.clone(), Some(Fetch::Album(v.album.id.clone(), v.album.source.clone()).origin()));
            Some(album_page(&v))
        }
        _ => None,
    });
}

/// Reads `read`, sending each page `make` turns into an answer; an error only when nothing came.
fn pages(
    client: &nori_core::client::Client,
    token: u64,
    read: Read,
    make: &dyn Fn(Page) -> Option<Value>,
) {
    pages_sent(client, token, read, make);
}

/// Sends each page `read` brings as `make` draws it; false when it failed (the failure is sent).
fn pages_sent(
    client: &nori_core::client::Client,
    token: u64,
    read: Read,
    make: &dyn Fn(Page) -> Option<Value>,
) -> bool {
    pages_from(token, &mut |got| read_pages(client, read.clone(), got), make)
}

/// [`pages_sent`] of what `reading` reads, handing each page to the function it is given.
fn pages_from(
    token: u64,
    reading: &mut nori_host::pages::PageReader<'_>,
    make: &dyn Fn(Page) -> Option<Value>,
) -> bool {
    let mut any = false;
    let r = reading(&mut |p| {
        if let Some(v) = make(p) {
            any = true;
            send(token, &v);
        }
    });
    match r {
        Ok(()) if !any => send(token, &json!({ "sections": [] })),
        Ok(()) => {}
        Err(e) => {
            send_error(token, &e);
            return false;
        }
    }
    true
}

pub(crate) fn read(token: u64, kind: i32, arg: String) {
    let Some((client, core)) = server() else {
        send(token, &json!({ "closed": true }));
        return;
    };
    match kind {
        // "For you", then the library's shelves and the sources' own (their feeds, titled as they title
        // them) in the order woven; all their songs one list for taps. A jam guest's mixes stay empty.
        PAGE_HOME => {
            let Some(h) = with_session(|s| nori_host::session::Handle::clone(s)) else { return };
            let rows = h.home_rows();
            let home = Mutex::new((Vec::new(), vec![None; rows.len()], Vec::new(), Vec::new()));
            h.read_home(rows.clone(), true, move |r| {
                let mut home = lock(&home);
                let (mixes, shelves, feed, places) = &mut *home;
                match r {
                    Ok(HomePart::Feed(v)) => *feed = v,
                    Ok(HomePart::Mixes(tiles)) => *mixes = tiles.iter().map(mix_json).collect(),
                    Ok(HomePart::Shelf { at, items }) => shelves[at] = Some(items),
                    Ok(HomePart::Places(v)) => *places = v,
                    Err(e) => return send_error(token, &e),
                }
                let mut songs = Vec::new();
                let mut sections = vec![section("mixes", true, mixes.clone())];
                for place in places.iter() {
                    match *place {
                        HomePlace::Row { at } => {
                            let (row, key) = (rows[at as usize], shelf_key(rows[at as usize]));
                            sections.push(match &shelves[at as usize] {
                                None => section(key, shelf_grid(row), Vec::new()),
                                Some(ShelfItems::Albums { v }) => section(key, true, v.iter().map(album_json).collect()),
                                Some(ShelfItems::Playlists { v }) => section(key, true, v.iter().map(playlist_json).collect()),
                                Some(ShelfItems::Songs { v }) => {
                                    let shelf = songs_section(key, v, songs.len());
                                    songs.extend(v.iter().cloned());
                                    shelf
                                }
                            });
                        }
                        HomePlace::Feed { at } => {
                            let Some(f) = feed.get(at as usize) else { continue };
                            sections.extend(feed_shelf(f, &mut songs));
                        }
                    }
                }
                keep_list(token, songs, None);
                send(token, &json!({ "sections": sections }));
            });
        }
        PAGE_HOME_ROW => {
            let (key, offset) = arg.split_once('\n').unwrap_or((arg.as_str(), "0"));
            match shelf_row(key) {
                Some(row) => home_row(token, &client, &core, row, offset.parse().unwrap_or(0)),
                None => send(token, &json!({ "sections": [] })),
            }
        }
        PAGE_MIX => {
            if arg == FAVOURITES_MIX {
                refresh_favourites(&client, client.mix_favourites_stored());
            } else {
                nori_core::transport::block_on(client.mix_ensure(arg.clone(), false));
            }
            match core.mix_page(arg.clone()) {
                MixLookup::Ready { sheet } => {
                    keep_list(
                        token,
                        sheet.songs.clone(),
                        Some(PageOrigin::new(OriginKind::Mix, sheet.id.as_str())),
                    );
                    send(
                        token,
                        &json!({
                            "head": {
                                "k": "mix", "id": sheet.id, "n": sheet.name as u8, "covers": sheet.covers,
                                "sec": sheet.seconds, "count": sheet.songs.len(), "refreshable": sheet.refreshable,
                            },
                            "sections": [songs_section("songs", &sheet.songs, 0)],
                        }),
                    );
                }
                _ => send(token, &json!({ "sections": [] })),
            }
        }
        PAGE_SMARTS => {
            let mut items: Vec<Value> = nori_core::smart::smart_defaults()
                .iter()
                .map(smart_json)
                .collect();
            match core.smart_list() {
                Ok(saved) => items.extend(saved.iter().map(smart_json)),
                Err(e) if items.is_empty() => {
                    return send(token, &json!({ "error": crate::account::LOGIN_DATABASE, "detail": e.to_string() }));
                }
                Err(_) => {}
            }
            send(token, &json!({ "sections": [section("smart", false, items)] }));
        }
        PAGE_SMART => {
            let Some(pl) = smart_of(&core, &arg) else {
                return send(token, &json!({ "sections": [] }));
            };
            let limit = nori_core::browse::library_sizes().smart_songs;
            match core.smart_page(pl.json.clone(), limit) {
                Ok(page) => {
                    keep_list(
                        token,
                        page.songs.clone(),
                        Some(PageOrigin::new(OriginKind::Smart, pl.id.as_str())),
                    );
                    let n = pl.builtin.map(|b| b as u8);
                    send(
                        token,
                        &json!({
                            "head": {
                                "k": "smart", "id": pl.id, "t": pl.name, "n": n,
                                "sec": page.seconds, "count": page.songs.len(),
                            },
                            "sections": [songs_section("songs", &page.songs, 0)],
                        }),
                    );
                }
                Err(e) => send(token, &json!({ "error": crate::account::LOGIN_DATABASE, "detail": e.to_string() })),
            }
        }
                PAGE_ALBUMS => album_grid(
            token,
            &client,
            album_sort_saved(list_prefs()),
            arg == ALBUMS_FAVORITES,
            arg.parse().unwrap_or(0),
        ),
        PAGE_ARTISTS => pages(&client, token, Read::ArtistIndex, &|p| match p {
            Page::Artists { v } => Some(json!({
                "sections": [section("artists", false, v.iter().map(artist_json).collect())]
            })),
            _ => None,
        }),
        // In a profile of several sources, each playlist names the source it is kept in (`on`).
        PAGE_PLAYLISTS => {
            let prefs = core.session.settings.current().unwrap_or_default();
            let several = prefs.active_server().is_some_and(|p| !p.sources.is_empty());
            let on = |p: &Playlist| nori_core::settings::source_profile(prefs.servers.clone(), prefs.active_server_id.clone(), p.source.clone()).map(|sv| crate::account::label(&sv)).unwrap_or_default();
            pages(&client, token, Read::PlaylistList, &|page| match page {
                Page::Playlists { v } => Some(json!({
                    "sections": [section("playlists", false, v.iter().map(|p| {
                        let mut row = playlist_json(p);
                        if several && !p.mixed {
                            row["on"] = json!(on(p));
                        }
                        row
                    }).collect())]
                })),
                _ => None,
            })
        }
        // The genres with songs, by name (`Core::genres_listed`).
        PAGE_GENRES => pages(&client, token, Read::GenreList, &|p| match p {
            Page::Genres { v } => Some(json!({
                "sections": [section("genres", false, core.genres_listed(v).iter().map(genre_json).collect())]
            })),
            _ => None,
        }),
        // A genre's songs under its hero, as a mix's page: a cover of them, Play and Shuffle.
        PAGE_GENRE => match nori_core::transport::block_on(client.genre_songs(arg.clone())) {
            Ok(songs) => {
                let seconds: u64 = songs.iter().map(|s| s.duration as u64).sum();
                let cover = core.genre_cover(arg.clone()).ok().flatten();
                let v = json!({
                    "head": { "k": "genre", "id": arg, "t": arg, "c": cover, "n": songs.len(), "count": songs.len(), "sec": seconds },
                    "sections": [songs_section("songs", &songs, 0)],
                });
                keep_list(token, songs, Some(PageOrigin::new(OriginKind::Genre, arg.as_str())));
                send(token, &v);
            }
            Err(e) => send_error(token, &e),
        },
        // What this iPod played, newest first, a page from the place the last one ended.
        PAGE_HISTORY => {
            let after = paging::From::history(&arg);
            let at = paging::From::History(after);
            if after.is_none() {
                paging::first(token, at);
            }
            match core.history_page(after) {
                Ok(page) => {
                    let songs: Vec<Song> = page.entries.into_iter().map(|e| e.song).collect();
                    let kept = if after.is_none() { None } else { extend_list(token, &songs) };
                    let from = kept.unwrap_or(0);
                    let more = paging::answered(token, at, page.next.map(|n| paging::From::History(Some(n))));
                    let v = json!({ "more": more, "from": from, "sections": [songs_section("songs", &songs, from)] });
                    if kept.is_none() {
                        keep_list(token, songs, None);
                    }
                    send(token, &v);
                }
                Err(e) => {
                    paging::failed(token, at);
                    send(token, &json!({ "error": crate::account::LOGIN_DATABASE, "detail": e.to_string() }));
                }
            }
        }
        PAGE_STATS => send(token, &stats(&client, &core, &arg)),
        PAGE_STARRED => pages(&client, token, Read::StarredItems, &|p| match p {
            Page::StarredPage { v } => {
                keep_list(token, v.songs.clone(), None);
                Some(json!({ "sections": [
                    section("albums", true, v.albums.iter().map(album_json).collect()),
                    section("artists", false, v.artists.iter().map(artist_json).collect()),
                    songs_section("songs", &v.songs, 0),
                ]}))
            }
            _ => None,
        }),
        PAGE_SONGS => {
            let offset = arg.parse().unwrap_or(0);
            let at = paging::From::Offset(offset);
            if offset == 0 {
                paging::first(token, at);
            }
            let sort = song_sort_saved(list_prefs());
            match nori_core::transport::block_on(client.songs_listed(sort.clone(), false, false, 0, 0, offset)) {
                Ok(p) => {
                    let kept = if offset == 0 { None } else { extend_list(token, &p.songs) };
                    let from = kept.unwrap_or(0);
                    let rows = songs_section("songs", &p.songs, from);
                    let more = paging::answered(token, at, (!p.exhausted).then_some(paging::From::Offset(offset + p.songs.len() as u32)));
                    let v = json!({
                        "more": more, "from": from, "sections": [rows],
                    });
                    if kept.is_none() {
                        keep_list(token, p.songs, None);
                    }
                    send(token, &v);
                }
                Err(e) => {
                    paging::failed(token, at);
                    send_error(token, &e);
                }
            }
        }
        PAGE_DOWNLOADS => {
            let library = core.downloaded_library().unwrap_or_default();
            let stored = library.songs;
            let (active, queued, failed) = match core.download_sections() {
                Ok(s) => (s.active, s.queued, s.failed),
                Err(_) => Default::default(),
            };
            let mut all = Vec::new();
            // What is kept, by album, above the lists.
            let albums: Vec<Value> = library.albums.iter().map(|g| {
                let mut card = album_json(g);
                card["k"] = json!("downloadedAlbum");
                card
            }).collect();
            let mut out = if albums.is_empty() { Vec::new() } else { vec![section("albums", true, albums)] };
            for (key, songs) in [
                ("active", &active),
                ("queued", &queued),
                ("failed", &failed),
                ("stored", &stored),
            ] {
                let mut rows = songs_section(key, songs, all.len());
                if key == "active" {
                    with_progress(&core, &mut rows);
                }
                out.push(rows);
                all.extend(songs.iter().cloned());
            }
            keep_list(token, all, None);
            let work = with_session(|s| s.progress()).flatten();
            send(token, &json!({ "running": !active.is_empty() || work.is_some(), "work": work.as_ref().map(work_json), "sections": out }));
        }
        PAGE_DOWNLOADED_ALBUM => match core.downloaded_album(arg.clone()) {
            Ok(Some(v)) => {
                keep_list(token, v.songs.clone(), Some(v.queue.origin()));
                let mut value = album_page(&v);
                value["head"]["k"] = json!("downloadedAlbum");
                send(token, &value);
            }
            // None left: the album's own page, so removing them leaves it open.
            Ok(None) => album(&client, token, ItemRef::taken(&arg)),
            Err(e) => send_error(token, &e.into()),
        },
        PAGE_ALBUM => album(&client, token, ItemRef::taken(&arg)),
        PAGE_ARTIST => {
            let it = ItemRef::taken(&arg);
            let last = Mutex::new(None);
            pages_from(token, &mut |got| nori_core::transport::block_on(client.clone().read_each_artist(it.source.clone(), it.id.clone(), got)), &|p| match p {
                Page::ArtistPage { v } => {
                    let page = artist_page(token, &v, None);
                    *lock(&last) = Some(v);
                    Some(page)
                }
                _ => None,
            });
            // After the page, which comes again with it: a provider answers both from one read of the
            // artist. A page without it is still the page.
            let Some(v) = last.into_inner().unwrap_or_else(|e| e.into_inner()) else { return };
            let mut about = None;
            let _ = read_pages_in(&client, it.source.clone(), Read::ArtistAbout { id: it.id.clone() }, |p| {
                if let Page::About { v } = p {
                    about = Some(nori_core::pages::ArtistAbout::of(v));
                }
            });
            if let Some(a) = about.filter(|a| !a.is_empty()) {
                send(token, &artist_page(token, &v, Some(&a)));
            }
        }
        PAGE_PLAYLIST => {
            let it = ItemRef::taken(&arg);
            pages_from(token, &mut |got| nori_core::transport::block_on(client.clone().read_each_playlist(it.id.clone(), it.at(), got)), &|p| match p {
            Page::PlaylistPage { v } => {
                let pl = &v.playlist;
                let editable = with_session(|s| {
                    let prefs = s.core.session.settings.current().unwrap_or_default();
                    let user = nori_core::settings::source_profile(prefs.servers, prefs.active_server_id, pl.source.clone()).map(|sv| sv.user).unwrap_or_default();
                    s.client.playlist_editable_in(pl, &user)
                });
                let editable = editable.unwrap_or(false).then(|| pl.clone());
                keep(List { token, songs: v.songs.clone(), origin: Some(Fetch::Playlist(pl.id.clone(), nori_core::sources::playlist_at(pl)).origin()), editable });
                Some(json!({
                    "head": {
                        "k": "playlist", "id": pl.id, "ref": ItemRef::sent(&pl.id, &pl.source, pl.mixed), "t": pl.name, "s": pl.owner.as_deref().unwrap_or(""),
                        "c": cover(&pl.cover_art), "sec": v.seconds, "n": v.songs.len(),
                        "note": pl.comment.as_deref().unwrap_or(""),
                    },
                    "sections": [songs_section("songs", &v.songs, 0)],
                }))
            }
            _ => None,
            });
        }
        PAGE_QUEUE => {
            let Some(v) = with_session(queue_json) else { return };
            send(token, &v);
        }
        PAGE_SEARCH => {
            let local = with_session(|s| s.search_typed(&arg));
            let Some(view) = local else { return };
            if view.query.is_empty() {
                let recent = core.search_history().unwrap_or_default();
                return send(token, &json!({ "recent": recent, "sections": [] }));
            }
            if view.answer.result().is_some() {
                send(token, &found(token, &view, false));
            }
            // The server already answered this query (a chip picked asks it again): its answer stands.
            if matches!(view.answer, SearchShown::Found { .. } | SearchShown::NothingFound) {
                return send(token, &found(token, &view, true));
            }
            let Some(answered) = with_session(|s| s.search_server_now(view.query.clone(), search_failure)).flatten() else { return };
            match answered.answer {
                SearchShown::Found { .. } | SearchShown::NothingFound => send(token, &found(token, &answered, true)),
                // The index found nothing either: the page says why, as `send_error` would.
                SearchShown::Failed { index: None, fallback } => send(token, &serde_json::from_str(&fallback.reason.unwrap_or_default()).unwrap_or(Value::Null)),
                SearchShown::Failed { .. } | SearchShown::Asking { .. } | SearchShown::Blank => {}
            }
        }
        _ => send(token, &json!({ "sections": [] })),
    }
}

/// The queue the player shows (`Handle::queue_shown`): this iPod's, the device's it mirrors (`remote`:
/// read only but for a tap, which plays that song there), or a jam guest's host's (`jam`), each song
/// with who asked for it in the jam (`by`) where someone did. `offers`: what its head offers for the whole
/// queue, by name (`menus::queue_actions`).
fn queue_json(s: &nori_host::session::Session) -> Value {
    let q = s.queue_shown(None);
    let shown = q.heard().map_or(-1, |i| i as i32);
    let mut v = queue_sections(&q.songs, &q.rows, q.view.repeat, q.view.shuffle, shown);
    let offers = nori_core::menus::queue_actions(q.rows.order.len() as u32, q.upcoming().count() as u32, s.rules().playlists, s.guest);
    v["head"]["offers"] = offers.iter().map(|a| format!("{a:?}")).collect();
    if q.mirrored {
        v["head"]["remote"] = json!(true);
    }
    if q.jam {
        v["head"]["jam"] = json!(true);
    }
    for section in v["sections"].as_array_mut().into_iter().flatten() {
        for row in section["items"].as_array_mut().into_iter().flatten() {
            if let Some(by) = row["id"].as_str().and_then(|id| q.by.get(id)) {
                row["by"] = json!(by);
            }
        }
    }
    v
}

/// The queue in play order (`Core::queue_rows`): what has played, the song playing, what plays next.
/// Each row's `i` is its place in the list, which the controls take.
fn queue_sections(songs: &[Song], rows: &QueueRows, repeat: u8, shuffle: bool, shown: i32) -> Value {
    let items = |at: &[u32]| -> Vec<Value> {
        at.iter()
            .filter_map(|&i| songs.get(i as usize).map(|s| song_json(s, i as usize)))
            .collect()
    };
    let (history, now, next): (&[u32], &[u32], &[u32]) = match usize::try_from(rows.now) {
        Ok(n) if n < rows.order.len() => (&rows.order[..n], &rows.order[n..=n], &rows.order[n + 1..]),
        _ => (&[], &[], &rows.order),
    };
    json!({
        "head": {
            "cur": shown, "repeat": repeat, "shuffle": shuffle, "reorderable": rows.reorderable,
            "kept": rows.kept, "from": rows.from,
        },
        "sections": [
            section("history", false, items(history)),
            section("now", false, items(now)),
            section("next", false, items(next)),
        ],
    })
}

/// What plays now (`Handle::now`), for the mini player, the card and the lock screen: this iPod's, the
/// device's it mirrors (`device` its name), or for a jam guest the jam's (`jam`). `remote`: the
/// controls reach another device, or remote control is on here. `system`: the lock screen shows it
/// (`Playback::shown_to_system`).
fn now_json(s: &nori_host::session::Session) -> Value {
    let n = s.now();
    let state = match n.state {
        State::Idle => 0,
        State::Playing => 1,
        State::Paused => 2,
        State::Ended => 3,
    };
    let here = n.device.is_none() && !n.jam;
    let mut v = json!({
        "state": state, "index": n.index.map_or(-1, |i| i as i64), "ms": n.position_now().max(0), "fadingMs": n.fading_ms(),
        "pace": n.pace, "repeat": n.repeat, "shuffle": n.shuffle, "len": n.len, "mixing": n.mixing,
        "eq": here && s.core.session.settings.in_effect(|p| p.eq_enabled), "remote": !n.jam && (!here || s.remote().is_some()),
        "buffering": n.buffering, "volume": n.volume.map(|v| (v * 100.0).round() as i64), "jam": n.jam,
        "system": s.playback().shown_to_system(),
    });
    if let Some(device) = &n.device {
        v["device"] = json!(device);
        // The device's kind (`DeviceKind`), for its glyph.
        if let Some(e) = s.elsewhere() {
            v["kind"] = json!(e.mirror.kind as u8);
        }
    }
    if let Some(song) = &n.song {
        v["song"] = song_json(song, n.index.unwrap_or(0));
        v["suffix"] = json!(song.suffix);
        v["kbps"] = json!(song.bit_rate);
        v["hz"] = json!(song.sampling_rate);
        v["bits"] = json!(song.bit_depth);
        v["album"] = json!(song.album);
    }
    v
}

pub(crate) fn owned(v: &Value) -> *mut c_char {
    CString::new(v.to_string().replace('\0', " "))
        .map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Where page answers go: `(token, json)`, the string valid for the call only. NULL clears it.
///
/// # Safety
/// `cb`, when set, stays valid while set.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_on_page(cb: Option<PageFn>) {
    *lock(&PAGE_HOOK) = cb;
}

/// Reads page `kind` (an id, a genre name, a query or an offset in `arg`) on a thread of its own.
/// Answers come to the page callback with `token`, the stored copy first.
///
/// # Safety
/// `arg` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_read(token: u64, kind: i32, arg: *const c_char) {
    let arg = c_text(arg);
    nori_host::spawn("nori-ios-page", move || read(token, kind, arg));
}

/// Plays the song list page `token` showed from `index` (with `shuffle`, from wherever it starts).
#[no_mangle]
pub extern "C" fn nori_ios_play_list(token: u64, index: i32, shuffle: i32) {
    let Some((songs, origin)) = list(token) else { return };
    with_session(|s| s.play(songs, index.max(0) as usize, shuffle != 0, origin));
}

/// A tap on song `index` of list `token`: the list plays from it, or, when it is the song playing, becomes
/// the queue around it and the song goes on. 1 when it went on (the app opens the player).
#[no_mangle]
pub extern "C" fn nori_ios_tap_song(token: u64, index: i32) -> i32 {
    let Some((mut songs, origin)) = list(token) else { return 0 };
    let mut index = index.max(0) as usize;
    // A search's song plays alone: its results are not an album, and queueing the rest would have a
    // provider fetch every one of them.
    if origin.as_ref().is_some_and(|o| o.kind == OriginKind::Search) {
        let Some(one) = songs.get(index).cloned() else { return 0 };
        (songs, index) = (vec![one], 0);
    }
    with_session(|s| s.keep_playing(songs, index, origin) as i32).unwrap_or(0)
}

/// The search's kind chip picked: -1 All, 0 songs, 1 albums, 2 artists, 3 playlists. The page reads the
/// search again to show it.
#[no_mangle]
pub extern "C" fn nori_ios_search_pick(kind: i32) {
    use nori_core::search::SearchKind;
    let kind = usize::try_from(kind).ok().and_then(|k| [SearchKind::Songs, SearchKind::Albums, SearchKind::Artists, SearchKind::Playlists].get(k).copied());
    with_session(|s| s.search_pick(kind));
}

/// Adds song `index` of list `token` (or all of them for -1) next or at the end.
#[no_mangle]
pub extern "C" fn nori_ios_enqueue_list(token: u64, index: i32, next: i32) {
    let songs = match usize::try_from(index) {
        Ok(i) => list_song(token, i).map(|s| vec![s]),
        Err(_) => list(token).map(|l| l.0),
    };
    let Some(songs) = songs else { return };
    with_session(|s| s.enqueue(songs, next != 0));
}

/// Downloads song `index` of list `token`, or all of them for -1.
#[no_mangle]
pub extern "C" fn nori_ios_download_list(token: u64, index: i32) {
    let songs = match usize::try_from(index) {
        Ok(i) => list_song(token, i).map(|s| vec![s]),
        Err(_) => list(token).map(|l| l.0),
    };
    let Some(songs) = songs else { return };
    with_session(|s| s.download(songs));
}

fn act_code(a: DownloadAct) -> i32 {
    match a {
        DownloadAct::All => 0,
        DownloadAct::Missing => 1,
        DownloadAct::Remove => 2,
    }
}

/// Each song of `songs` with whether it is downloaded or on its way, and whether it is downloaded.
fn download_states(s: &nori_host::session::Session, songs: &[Song]) -> Vec<(bool, bool)> {
    let ids: Vec<nori_core::ItemKey> = songs.iter().map(nori_core::Keyed::key).collect();
    s.download_states(&ids)
        .into_iter()
        .map(|state| match state {
            SongDownload::Done => (true, true),
            SongDownload::Pending => (true, false),
            SongDownload::None => (false, false),
        })
        .collect()
}

/// `songs`' download entries (`menus::download_entries`): `[{act, n}]`, `act` 0 download all, 1 download
/// the `n` missing, 2 remove the `n` downloaded. None where downloads are not the profile's (a jam guest's).
fn entries_of(s: &nori_host::session::Session, songs: &[Song]) -> Value {
    if !s.rules().downloads {
        return json!([]);
    }
    let states = download_states(s, songs);
    let missing = states.iter().filter(|(here, _)| !here).count();
    let done = states.iter().filter(|(_, done)| *done).count();
    let entries: Vec<Value> = download_entries(songs.len() as u32, missing as u32)
        .into_iter()
        .map(|a| {
            let n = match a {
                DownloadAct::All => songs.len(),
                DownloadAct::Missing => missing,
                DownloadAct::Remove => done,
            };
            json!({ "act": act_code(a), "n": n })
        })
        .collect();
    Value::from(entries)
}

/// Runs download entry `act` (as [`entries_of`] codes it) on `songs`, those of `playlist` when they are
/// one's: the songs downloaded, or removed for 2. A playlist is downloaded whole, so it is kept
/// downloaded as it changes.
fn act_on(s: &nori_host::session::Session, songs: Vec<Song>, act: i32, playlist: Option<String>) -> i32 {
    let states = download_states(s, &songs);
    let picked = |want: fn(&(bool, bool)) -> bool| -> Vec<Song> {
        songs.iter().zip(&states).filter(|(_, st)| want(st)).map(|(song, _)| song.clone()).collect()
    };
    let to_get = match act {
        0 => songs.clone(),
        1 => picked(|(here, _)| !here),
        2 => {
            let done = picked(|(_, done)| *done);
            done.iter().for_each(|song| s.download_remove(&nori_core::Keyed::key(song)));
            return done.len() as i32;
        }
        _ => return 0,
    };
    let n = to_get.len() as i32;
    match playlist {
        Some(id) => s.download_playlist(id, songs),
        None => s.download(to_get),
    }
    n
}

/// The songs of album, playlist or artist `id` (its item's `ref`; `kind` as in [`nori_ios_read`]), each
/// saying its source: the stored copy, then the source's; offline the stored one alone.
fn collection_songs(core: &nori_core::Core, client: &Arc<nori_core::client::Client>, kind: i32, id: String) -> Vec<Song> {
    if kind == PAGE_DOWNLOADED_ALBUM {
        return core.downloaded_album(id).ok().flatten().map(|a| a.songs).unwrap_or_default();
    }
    let it = ItemRef::taken(&id);
    let mut songs = Vec::new();
    let mut take = |p: Page| match p {
        Page::AlbumPage { v } => songs = v.songs,
        Page::PlaylistPage { v } => songs = v.songs,
        _ => {}
    };
    let _ = match kind {
        PAGE_ARTIST => return fetch(kind, id).map(|f| nori_core::transport::block_on(f.songs(client)).unwrap_or_default()).unwrap_or_default(),
        PAGE_ALBUM => nori_host::session::read_pages_in(client, it.source.clone(), Read::AlbumById { id: it.id }, &mut take),
        PAGE_PLAYLIST => nori_core::transport::block_on(client.clone().read_each_playlist(it.id.clone(), it.at(), &mut take)),
        _ => Ok(()),
    };
    songs
}

/// List `token`'s download entries ([`entries_of`]) as JSON to free.
#[no_mangle]
pub extern "C" fn nori_ios_download_entries(token: u64) -> *mut c_char {
    let Some((songs, _)) = list(token) else { return std::ptr::null_mut() };
    with_session(|s| owned(&entries_of(s, &songs))).unwrap_or(std::ptr::null_mut())
}

/// Forgets what this iPod played (`Core::history_clear`); 1 when it did.
#[no_mangle]
pub extern "C" fn nori_ios_history_clear() -> i32 {
    server().is_some_and(|(_, core)| core.history_clear().is_ok()) as i32
}

/// What page `kind` (as in [`nori_ios_read`]) stands in place of its first answer (`sketch::sketch`, with
/// its head unknown), as JSON to free: `{after, pass, ease, parts: [{p, a}]}`, the core's quick-load time,
/// shimmer pass and fade in ms; `p` 0 a head, 1 a shelf, 2 a grid, 3 rows; `a` 0 a square, 1 a round
/// picture, 2 a song's number, 3 words alone. Null for a page with no sketch.
#[no_mangle]
pub extern "C" fn nori_ios_sketch(kind: i32) -> *mut c_char {
    use nori_core::sketch::{sketch, Art, Reading, Sketch};
    let reading = match kind {
        PAGE_HOME => Reading::Home,
        PAGE_ALBUMS | PAGE_HOME_ROW => Reading::Albums,
        PAGE_ARTISTS => Reading::Artists,
        PAGE_PLAYLISTS | PAGE_SMARTS => Reading::Playlists,
        PAGE_SONGS => Reading::Songs,
        PAGE_GENRES => Reading::Genres,
        PAGE_STARRED => Reading::Favorites,
        PAGE_SEARCH => Reading::Search,
        PAGE_ALBUM | PAGE_DOWNLOADED_ALBUM => Reading::Album,
        PAGE_ARTIST => Reading::Artist,
        PAGE_PLAYLIST => Reading::Playlist,
        PAGE_MIX | PAGE_SMART | PAGE_GENRE => Reading::Mix,
        _ => return std::ptr::null_mut(),
    };
    let part = |s: Sketch| {
        let (p, art) = match s {
            Sketch::Head { art } => (0, art),
            Sketch::Shelf { art } => (1, art),
            Sketch::Grid { art } => (2, art),
            Sketch::Rows { art } => (3, art),
        };
        let a = match art {
            Art::Square => 0,
            Art::Round => 1,
            Art::Number => 2,
            Art::Bare => 3,
        };
        json!({ "p": p, "a": a })
    };
    let stage = nori_core::stage::stage();
    owned(&json!({
        "after": stage.quick_load_ms, "pass": stage.sheen_ms, "ease": stage.fade_ms,
        "parts": sketch(reading, false).into_iter().map(part).collect::<Vec<_>>(),
    }))
}

/// What a page's list head offers for list `token` (`menus::page_actions`), as JSON to free: `[{a, act, n}]`,
/// `a` 0 download (`act` as [`entries_of`] codes it, `n` the songs it takes), 1 download everything by the
/// artist, 2 add to the queue, 3 a new draw of the mix, 4 edit. `kind` as in [`nori_ios_read`];
/// `refreshable`: a mix that can be drawn again.
#[no_mangle]
pub extern "C" fn nori_ios_page_actions(token: u64, kind: i32, refreshable: i32) -> *mut c_char {
    let kind = match kind {
        // A genre's songs offer what an album's do, as Android's genre page.
        PAGE_ALBUM | PAGE_DOWNLOADED_ALBUM | PAGE_GENRE => PageKind::Album,
        PAGE_ARTIST => PageKind::Artist,
        PAGE_PLAYLIST => PageKind::Playlist,
        PAGE_MIX => PageKind::Mix { refreshable: refreshable != 0 },
        PAGE_SMART => PageKind::Smart,
        _ => return std::ptr::null_mut(),
    };
    let songs = list(token).map(|(songs, _)| songs).unwrap_or_default();
    with_session(|s| {
        let states = download_states(s, &songs);
        let missing = states.iter().filter(|(here, _)| !here).count();
        let done = states.iter().filter(|(_, done)| *done).count();
        owned(&Value::from(head_actions(kind, songs.len(), missing, done, s.rules().downloads)))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// [`nori_ios_page_actions`]' lines for `songs` songs, `missing` not downloaded and `done` downloaded.
fn head_actions(kind: PageKind, songs: usize, missing: usize, done: usize, downloads: bool) -> Vec<Value> {
    page_actions(kind, songs as u32, missing as u32, downloads)
        .into_iter()
        .map(|a| match a {
            PageAction::Download { act } => {
                let n = match act {
                    DownloadAct::All => songs,
                    DownloadAct::Missing => missing,
                    DownloadAct::Remove => done,
                };
                json!({ "a": 0, "act": act_code(act), "n": n })
            }
            PageAction::DownloadEverything => json!({ "a": 1 }),
            PageAction::AddToQueue => json!({ "a": 2 }),
            PageAction::Refresh => json!({ "a": 3 }),
            PageAction::Edit => json!({ "a": 4 }),
        })
        .collect()
}

/// Draws mix `id` again; its page reads again after. Blocks.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_mix_refresh(id: *const c_char) {
    let id = c_text(id);
    let Some((client, _)) = server() else { return };
    nori_core::transport::block_on(client.mix_ensure(id, true));
}

/// Plays the library shuffled: random albums each whole (`albums` 1), or random songs. Blocks; 1 when it
/// plays.
#[no_mangle]
pub extern "C" fn nori_ios_shuffle_library(albums: i32) -> i32 {
    let Some((client, _)) = server() else { return 0 };
    match nori_core::transport::block_on(client.shuffle_library(albums != 0)) {
        Ok((songs, from)) if !songs.is_empty() => with_session(|s| s.play(songs, 0, false, Some(from))).map_or(0, |()| 1),
        _ => 0,
    }
}

/// Runs download entry `act` on list `token` ([`act_on`]).
#[no_mangle]
pub extern "C" fn nori_ios_download_act(token: u64, act: i32) -> i32 {
    let Some((songs, origin)) = list(token) else { return 0 };
    let playlist = origin.filter(|o| o.kind == OriginKind::Playlist).map(|o| o.id);
    with_session(|s| act_on(s, songs, act, playlist)).unwrap_or(0)
}

/// [`nori_ios_download_entries`] for an album, playlist or artist by id, as a card's menu has it. Blocks.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_collection_download_entries(kind: i32, id: *const c_char) -> *mut c_char {
    let id = c_text(id);
    let Some((client, core)) = server() else { return std::ptr::null_mut() };
    let songs = collection_songs(&core, &client, kind, id);
    with_session(|s| owned(&entries_of(s, &songs))).unwrap_or(std::ptr::null_mut())
}

/// [`nori_ios_download_act`] for an album, playlist or artist by id. Blocks.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_collection_download_act(kind: i32, id: *const c_char, act: i32) -> i32 {
    let id = c_text(id);
    let Some((client, core)) = server() else { return 0 };
    let songs = collection_songs(&core, &client, kind, id.clone());
    // A playlist of the profile's own source is kept downloaded as it changes.
    let kept = Some(ItemRef::taken(&id)).filter(|it| kind == PAGE_PLAYLIST && it.at() == nori_core::sources::PlaylistAt::Source { source: None }).map(|it| it.id);
    with_session(|s| act_on(s, songs, act, kept)).unwrap_or(0)
}

/// What album, artist or playlist `id` (its item's `ref`) plays, `kind` as in [`nori_ios_read`].
fn fetch(kind: i32, id: String) -> Option<Fetch> {
    if kind == PAGE_DOWNLOADED_ALBUM {
        return Some(Fetch::DownloadedAlbum(id));
    }
    let it = ItemRef::taken(&id);
    match kind {
        PAGE_ALBUM => Some(Fetch::Album(it.id, it.source)),
        PAGE_ARTIST => Some(Fetch::Artist(it.id, it.source)),
        PAGE_PLAYLIST => Some(Fetch::Playlist(it.id.clone(), it.at())),
        _ => None,
    }
}

/// Plays an album, artist or playlist (`kind` as in [`nori_ios_read`]) once its songs are fetched.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_play_collection(kind: i32, id: *const c_char, shuffle: i32) {
    let Some(what) = fetch(kind, c_text(id)) else { return };
    with_session(|s| s.play_later(what, shuffle != 0));
}

/// Enqueues an album, artist or playlist next (`next` 1) or last.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_enqueue_collection(kind: i32, id: *const c_char, next: i32) {
    let Some(what) = fetch(kind, c_text(id)) else { return };
    with_session(|s| s.enqueue_later(what, next != 0));
}

/// A song (1), album (2) or artist (3) by its code.
fn starrable(kind: i32) -> Option<Starrable> {
    match kind {
        1 => Some(Starrable::Song),
        2 => Some(Starrable::Album),
        3 => Some(Starrable::Artist),
        _ => None,
    }
}

/// Stars (`on` 1) or unstars a song (1), album (2) or artist (3), `id` its item's `ref`, in its source.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_star(kind: i32, id: *const c_char, on: i32) {
    let Some(kind) = starrable(kind) else { return };
    let it = ItemRef::taken(&c_text(id));
    with_session(|s| match kind {
        Starrable::Song => s.star_song(nori_core::ItemKey { source: it.source, id: it.id }, on != 0, s.elsewhere().as_ref()),
        _ => s.star(it.source, kind, it.id, on != 0),
    });
}

/// Whether a song (1), album (2) or artist (3) shows starred (1), its record saying `listed`: a song as
/// the device playing elsewhere shows it while it is in that queue, else this session's mark over the
/// record (`Handle::starred`).
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_starred(kind: i32, id: *const c_char, listed: i32) -> i32 {
    let (Some(kind), it, listed) = (starrable(kind), ItemRef::taken(&c_text(id)), listed != 0) else { return listed };
    let shown = with_session(|s| match kind {
        Starrable::Song => s.song_starred(&nori_core::ItemKey { source: it.source.clone(), id: it.id.clone() }, listed),
        _ => s.core.starred_in(it.source.clone(), kind, &it.id, listed),
    });
    i32::from(shown.unwrap_or(listed))
}

/// What plays now, as JSON to free with `nori_ios_free`; NULL when nothing is open.
#[no_mangle]
pub extern "C" fn nori_ios_now() -> *mut c_char {
    with_session(|s| owned(&now_json(s))).unwrap_or(std::ptr::null_mut())
}

/// Where decoded covers go: `(token, width, height, rgba, len)`, the pixels valid for the call only.
///
/// # Safety
/// `cb`, when set, stays valid while set.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_on_cover(cb: Option<CoverFn>) {
    *lock(&COVER_HOOK) = cb;
}

/// Asks for cover `id` at `px` square; the answer comes to the cover callback with `token`, from the
/// memory cache, disk, or server. Before it, the picture the core picks to stand in while it comes (the
/// cover kept at another size or rendition) may come, marked `stand_in`; one in memory comes at once, on
/// the caller's thread. A failure has zero dimensions and no owner; cancellation has no answer.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_cover(token: u64, id: *const c_char, px: u32) {
    // The decoded image's own buffer, shared with the loader's memory cache.
    let stand_in = move |image: Arc<nori_covers::Image>| hand(token, image.width, image.height, image, |i| &i.pixels, true);
    request(token, c_text(id), px, stand_in, move |image| match image {
        Some(image) => hand(token, image.width, image.height, image, |i| &i.pixels, false),
        None => hand_none(token),
    });
}

/// Tells the cover callback the picture for `token` will not come.
fn hand_none(token: u64) {
    if let Some(cb) = *lock(&COVER_HOOK) {
        unsafe { cb(token, 0, 0, std::ptr::null(), 0, std::ptr::null_mut(), false) };
    }
}

/// How many covers the first run's AutoMix question plays (`setup::setup_covers`).
#[no_mangle]
pub extern "C" fn nori_ios_setup_covers() -> u32 {
    setup_covers().len() as u32
}

/// Cover `index` of those, decoded `px` square off the caller's thread and handed to the cover callback with
/// `token`; none for an index past them.
#[no_mangle]
pub extern "C" fn nori_ios_setup_cover(token: u64, index: u32, px: u32) {
    nori_host::spawn("nori-ios-setup-cover", move || {
        let side = px as usize;
        let decoded = setup_covers().get(index as usize).and_then(|bytes| nori_covers::Decoder::new().decode(bytes, side, side, nori_covers::Alpha::Straight).ok());
        match decoded {
            Some(pixels) => hand(token, px, px, pixels, Vec::as_slice, false),
            None => hand_none(token),
        }
    });
}

/// Asks for cover `id` at `px`, kept under `token` until it comes or is let go; `stand_in` gets what stands
/// in while it comes.
pub(crate) fn request(token: u64, id: String, px: u32, mut stand_in: impl FnMut(Arc<nori_covers::Image>) + Send + 'static, deliver: impl FnOnce(Option<Arc<nori_covers::Image>>) + Send + 'static) {
    if id.is_empty() || px == 0 {
        return;
    }
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let mut deliver = Some(deliver);
    let ticket = with_session(|s| {
        s.cover(id, px, false, move |image, _, shown| {
            if shown == nori_covers::Shown::StandIn {
                if let Ok(image) = image {
                    stand_in(image);
                }
                return;
            }
            finished.store(true, Ordering::Release);
            lock(&TICKETS).retain(|(t, _)| *t != token);
            if let Some(deliver) = deliver.take() {
                deliver(image.ok());
            }
        })
    })
    .flatten();
    if let Some(ticket) = ticket {
        let mut tickets = lock(&TICKETS);
        if done.load(Ordering::Acquire) {
            ticket.detach();
        } else {
            tickets.push((token, ticket));
        }
    }
}

/// The queue origin a headed page's Play and Shuffle start: its kind (as in [`nori_ios_read`]) and id.
fn page_origin(kind: i32, id: &str) -> Option<PageOrigin> {
    let kind = match kind {
        PAGE_DOWNLOADED_ALBUM => OriginKind::Downloads,
        PAGE_ALBUM => OriginKind::Album,
        PAGE_ARTIST => OriginKind::Artist,
        PAGE_PLAYLIST => OriginKind::Playlist,
        PAGE_MIX => OriginKind::Mix,
        PAGE_SMART => OriginKind::Smart,
        PAGE_GENRE => OriginKind::Genre,
        _ => return None,
    };
    Some(PageOrigin::new(kind, id))
}

/// A headed page's Play and Shuffle as they stand (`pages::hero_buttons`), packed as
/// `HeroButtons::pack` says: while the queue playing was started from this page, Play pauses and
/// resumes it and Shuffle turns its shuffle off or on. `playing` and `buffering` are what the app shows.
fn hero(origin: Option<&PageOrigin>, page: &PageOrigin, shuffled: bool, playing: bool, buffering: bool) -> i32 {
    let here = origin == Some(page);
    nori_core::pages::hero_buttons(here, shuffled, playing, buffering, true, true).pack()
}

/// [`hero`] for page `kind` `id` against the queue now. 0 for a page without those buttons.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_hero(kind: i32, id: *const c_char, playing: i32, buffering: i32) -> i32 {
    let Some(page) = page_origin(kind, &c_text(id)) else { return 0 };
    with_session(|s| {
        let shuffled = s.core.session.playlist(|p| p.lit());
        hero(s.core.session.origin().as_ref(), &page, shuffled, playing != 0, buffering != 0)
    })
    .unwrap_or(0)
}

/// Queue songs either side of the current one whose covers are fetched ahead.
const WARM_AHEAD: i32 = 2;

/// The song at `index` began: its cover and its neighbours' (`Core::covers_around`, both renditions)
/// are fetched into the disk cache, so the player, a skip and the lock screen find them there.
pub(crate) fn warm_around(s: &nori_host::session::Session, index: usize) {
    let Some(loader) = s.covers.as_ref() else { return };
    let Ok(at) = i32::try_from(index) else { return };
    let around = s.core.covers_around(at, at - 1, at + 1, WARM_AHEAD);
    let current = around.near.first().into_iter().flat_map(|id| {
        let rules = nori_core::covers::cover_rules();
        [rules.row, rules.full].map(|size| (id.clone(), size))
    });
    for url in current.chain(around.wants.into_iter().map(|w| (w.id, w.size))).filter_map(|(id, size)| s.core.cover_address(id, size)) {
        loader.warm(&url);
    }
}

/// Lets go of cover request `token`: a cell scrolled away.
#[no_mangle]
pub extern "C" fn nori_ios_cover_cancel(token: u64) {
    let gone: Vec<(u64, Ticket)> = {
        let mut t = lock(&TICKETS);
        let (gone, kept) = t.drain(..).partition(|(k, _)| *k == token);
        *t = kept;
        gone
    };
    drop(gone);
}

/// The words the session's lyrics view shows for `song`, if they are its.
pub(crate) fn held_lyrics(song: &nori_core::ItemKey) -> Option<Lyrics> {
    with_session(|s| s.lyrics_now().words_of(song).map(|p| p.lyrics.clone())).flatten()
}

/// The lyrics view opened on `song`: its lyrics are looked up, each change a lyrics report.
///
/// # Safety
/// `song` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_lyrics(song: *const c_char) {
    let song = c_text(song);
    if song.is_empty() {
        return;
    }
    with_session(|s| {
        s.lyrics_heard(Some(nori_core::ItemKey::from_text(&song)));
        s.lyrics_view(true);
    });
}

/// The lyrics view closed.
#[no_mangle]
pub extern "C" fn nori_ios_lyrics_closed() {
    with_session(|s| s.lyrics_view(false));
}

/// What the lyrics view shows for `song`, as JSON to free (no lines when there are none, or the lookup
/// failed); NULL while they are looked up.
///
/// # Safety
/// `song` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_lyrics_page(song: *const c_char) -> *mut c_char {
    let song = nori_core::ItemKey::from_text(&c_text(song));
    let Some(now) = with_session(|s| s.lyrics_now()).filter(|n| n.song() == Some(&song)) else {
        return std::ptr::null_mut();
    };
    let failed = Lyrics::default();
    let l = match &now {
        LyricsNow::Shown { pick, .. } => &pick.lyrics,
        LyricsNow::Failed { .. } => &failed,
        LyricsNow::NoSong | LyricsNow::Looking { .. } => return std::ptr::null_mut(),
    };
    let lines: Vec<Value> = l
        .lines
        .iter()
        .map(|line| json!({ "s": line.start_ms, "e": line.end_ms, "t": line.text, "b": line.backing }))
        .collect();
    owned(&json!({ "id": song.text(), "synced": l.synced, "offset": l.timing.offset_ms(), "lines": lines }))
}

/// A long list's orders as JSON to free: `now` the one it is in, `all` every one in the menu's order
/// (albums by [`AlbumSort::name`], songs by [`song_sorts`] name, both for the client to word), and `ways`
/// which way each runs (1 up, -1 down, 0 shuffled). Null for a page without orders.
#[no_mangle]
pub extern "C" fn nori_ios_sorts(kind: i32) -> *mut c_char {
    let way = |d| match d {
        SortDirection::Ascending => 1,
        SortDirection::Descending => -1,
        SortDirection::Shuffled => 0,
    };
    let prefs = list_prefs();
    let v = match kind {
        PAGE_ALBUMS => json!({
            "now": album_sort_saved(prefs).name(),
            "all": album_sorts().into_iter().map(|s| s.name()).collect::<Vec<_>>(),
            "ways": album_sorts().into_iter().map(|s| way(album_sort_direction(s))).collect::<Vec<_>>(),
        }),
        PAGE_SONGS => json!({
            "now": song_sort_saved(prefs),
            "all": song_sorts().into_iter().map(|s| s.name).collect::<Vec<_>>(),
            "ways": song_sorts().into_iter().map(|s| way(song_sort_direction(s.name))).collect::<Vec<_>>(),
        }),
        _ => return std::ptr::null_mut(),
    };
    owned(&v)
}

/// Keeps `name` (one of [`nori_ios_sorts`]' `all`) as the order of list `kind`; the page reads again.
///
/// # Safety
/// `name` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_sort(kind: i32, name: *const c_char) {
    let name = c_text(name);
    let kept = match kind {
        PAGE_ALBUMS => match album_sorts().into_iter().find(|s| s.name() == name) {
            Some(s) => album_sort_kept(s),
            None => return,
        },
        PAGE_SONGS if song_sorts().iter().any(|s| s.name == name) => song_sort_kept(name),
        _ => return,
    };
    with_session(|s| {
        let settings = &s.core.session.settings;
        if let Some(mut prefs) = settings.current() {
            prefs.list_prefs.insert(kept.key, kept.value);
            settings.put(prefs);
        }
    });
}

/// Home's rows as JSON to free: `{shown, hidden}`, each the rows' names, `shown` in their order.
#[no_mangle]
pub extern "C" fn nori_ios_home_rows() -> *mut c_char {
    with_session(|s| {
        let rows = s.core.session.settings.prefs(|p| p.home_rows.clone());
        let names = |rows: &[HomeRow]| rows.iter().map(|r| format!("{r:?}")).collect::<Vec<_>>();
        owned(&json!({ "shown": names(&rows), "hidden": names(&nori_core::browse::home_rows_hidden(rows.clone())) }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// The kept Home rows changed by `change`.
fn change_home_rows(change: impl FnOnce(Vec<HomeRow>) -> Vec<HomeRow>) {
    with_session(|s| {
        let settings = &s.core.session.settings;
        if let Some(mut prefs) = settings.current() {
            prefs.home_rows = change(prefs.home_rows);
            settings.put(prefs);
        }
    });
}

/// Shows Home row `name` (`on` 1) or leaves it out.
///
/// # Safety
/// `name` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_home_row_set(name: *const c_char, on: i32) {
    let name = c_text(name);
    let Some(row) = HomeRow::every().iter().copied().find(|r| format!("{r:?}") == name) else { return };
    change_home_rows(|rows| nori_core::browse::home_rows_toggled(rows, row, on != 0));
}

/// Moves the Home row shown at `from` to `to`.
#[no_mangle]
pub extern "C" fn nori_ios_home_row_move(from: i32, to: i32) {
    let (Ok(from), Ok(to)) = (u32::try_from(from), u32::try_from(to)) else { return };
    change_home_rows(|rows| nori_core::browse::home_rows_moved(rows, from, to));
}

/// Whether playlist `id` is pinned (1) on this iPod.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_playlist_pinned(id: *const c_char) -> i32 {
    let id = c_text(id);
    with_session(|s| i32::from(nori_core::browse::playlist_pinned(&s.core.session.settings, &id))).unwrap_or(0)
}

/// Pins playlist `id` (`on` 1) or lets it go; the home page's shelf of pinned playlists follows.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_playlist_pin(id: *const c_char, on: i32) {
    let id = c_text(id);
    with_session(|s| nori_core::browse::pin_playlist(&s.core.session.settings, id, on != 0));
}

/// The credits page's lists as JSON to free: `{core, data}`, each `[{name, what, copyright, licence,
/// file}]` (`file` names `licences/<file>.txt` in the bundle, null for none). `data` carries what the app
/// draws with too: Inter, Literata, the seal's and the seasons' Shippori Mincho, and Iconoir.
#[no_mangle]
pub extern "C" fn nori_ios_credits() -> *mut c_char {
    let list = |c: Vec<nori_core::credits::Credit>| -> Vec<Value> {
        c.into_iter()
            .map(|c| json!({ "name": c.name, "what": c.what, "copyright": c.copyright, "licence": c.licence, "file": c.file }))
            .collect()
    };
    owned(&json!({
        "core": list(nori_core::credits::core_credits(nori_core::credits::CreditClient::Ipod)),
        "data": list(nori_core::credits::data_credits(nori_core::credits::CreditClient::Ipod)),
    }))
}

/// The settings this client shows, with their kinds, options and values, as JSON to free.
#[no_mangle]
pub extern "C" fn nori_ios_settings() -> *mut c_char {
    with_session(|s| {
        let prefs = s.core.session.settings.current().unwrap_or_default();
        let rows: Vec<Value> = nori_core::settings_model::specs()
            .into_iter()
            .filter_map(|spec| {
                let value = nori_core::settings_model::value_of(&prefs, &spec.name)?;
                let kind = match spec.kind {
                    nori_core::settings_model::SettingKind::Switch => "switch",
                    nori_core::settings_model::SettingKind::Choice => "choice",
                    nori_core::settings_model::SettingKind::Level => "level",
                    _ => return None,
                };
                Some(json!({
                    "name": spec.name, "kind": kind, "options": spec.options, "min": spec.min,
                    "max": spec.max, "value": value,
                }))
            })
            .collect();
        owned(&json!(rows))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Sets setting `name` to `value` and applies it. 1 when it changed.
///
/// # Safety
/// Both are NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_set(name: *const c_char, value: *const c_char) -> i32 {
    let (name, value) = (c_text(name), c_text(value));
    with_session(|s| i32::from(s.setting(&name, &value).is_some())).unwrap_or(0)
}

/// The active server's music libraries as `{id, name, on, tapped}` rows, when it has more than one;
/// `tapped` is the `musicFolders` value switching that library stores (`folder_switches`).
fn music_folder_rows(shown: Vec<String>, folders: Vec<nori_core::MusicFolder>) -> Vec<Value> {
    if folders.len() <= 1 {
        return Vec::new();
    }
    let switches = nori_core::settings::folder_switches(shown, folders.iter().map(|f| f.id.clone()).collect());
    folders
        .into_iter()
        .zip(switches)
        .map(|(f, s)| json!({ "id": f.id, "name": f.name, "on": s.on, "tapped": s.tapped }))
        .collect()
}

/// The active server's music libraries as JSON to free: `[{id, name, on, tapped}]`, empty when the
/// server has one or fewer, cannot be reached, or this profile does not change server options.
#[no_mangle]
pub extern "C" fn nori_ios_music_folders() -> *mut c_char {
    let rows = with_session(|s| {
        if !s.rules().settings.contains(&nori_core::settings_pages::SettingsPart::ServerOptions) {
            return Vec::new();
        }
        let folders = match nori_core::transport::block_on(s.client.read_now(Read::MusicFolders)) {
            Ok(Page::Folders { v }) => v,
            _ => return Vec::new(),
        };
        let shown = s.core.session.settings.prefs(|p| p.active_server().map(|sv| sv.music_folder_ids.clone()).unwrap_or_default());
        music_folder_rows(shown, folders)
    })
    .unwrap_or_default();
    owned(&json!(rows))
}

/// The first run's questions: the iPod touch has no mobile data, so none about it.
fn setup_steps() -> Vec<SetupStep> {
    nori_core::setup::setup_steps(false)
}

/// The first run's questions as JSON to free, in order: `[{step, answers}]`, each by its name in the core
/// (`Listening`, `Look`; `Albums`, `Blended`, ...).
#[no_mangle]
pub extern "C" fn nori_ios_setup() -> *mut c_char {
    let named = |a: &SetupAnswer| format!("{a:?}");
    let steps: Vec<Value> = setup_steps().into_iter().map(|s| json!({ "step": format!("{s:?}"), "answers": setup_answers(s).iter().map(named).collect::<Vec<_>>() })).collect();
    owned(&json!(steps))
}

/// Answer `answer` of question `step` (`nori_ios_setup`'s indices) given: the settings it stands for set as
/// `nori_ios_set` sets one. 0 for no such answer or no open session.
#[no_mangle]
pub extern "C" fn nori_ios_setup_answer(step: u32, answer: u32) -> i32 {
    let Some(answer) = setup_steps().get(step as usize).and_then(|s| setup_answers(*s).get(answer as usize).copied()) else { return 0 };
    with_session(|s| {
        for pick in setup_settings(answer) {
            s.setting(&pick.name, &pick.value);
        }
        1
    })
    .unwrap_or(0)
}

/// Points across the drawn curve.
const CURVE_SAMPLES: u32 = 97;

/// The equalizer as JSON to free: `{ on, graphic, reach, curve, each, points, marks, bands }`. `curve` is the
/// sound heard in dB at `CURVE_SAMPLES` points across, `each` every band's own, `points` each band's
/// `{x, db}` on it (x 0..1), `marks` the scale's `{label, x}`. A band is `{label, gain, value}` (its
/// frequency and gain as figures), and a parametric one also `{q, width, kind, channel, usesGain}`, kind
/// and channel ordinals for the client to word.
#[no_mangle]
pub extern "C" fn nori_ios_equalizer() -> *mut c_char {
    with_session(|s| {
        let p = s.core.session.settings.current().unwrap_or_default();
        let drawing = nori_core::dsp::eq_drawing(&p, CURVE_SAMPLES);
        let graphic = p.eq_mode == nori_core::settings::EqMode::Graphic;
        let bands: Vec<serde_json::Value> = if graphic {
            let centres = nori_core::dsp::graphic_bands(p.eq_graphic.len() as u32);
            p.eq_graphic.iter().zip(&centres).map(|(g, c)| json!({ "label": numbers::iso_band(c.label_hz, POINT), "gain": g, "value": numbers::signed_db(*g, POINT) })).collect()
        } else {
            p.eq_bands
                .iter()
                .map(|b| {
                    json!({ "label": numbers::hz(b.freq, POINT), "gain": b.gain_db, "value": numbers::signed_db(b.gain_db, POINT), "q": b.q,
                            "width": numbers::fixed(b.q as f64, 2, false, POINT), "kind": b.kind as i32, "channel": b.channel as i32, "usesGain": b.kind.uses_gain() })
                })
                .collect()
        };
        let marks: Vec<_> = [100.0f32, 1000.0, 10_000.0].iter().map(|&hz| json!({ "label": numbers::hz(hz, POINT), "x": nori_core::dsp::band_position(hz) })).collect();
        owned(&json!({
            "on": p.eq_enabled,
            "graphic": graphic,
            "reach": nori_core::dsp::CURVE_DB,
            "curve": drawing.curve,
            "each": drawing.each,
            "points": drawing.points.iter().map(|pt| json!({ "x": pt.x, "db": pt.db })).collect::<Vec<_>>(),
            "marks": marks,
            "bands": bands,
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Band `index` as the curve or its editor left it ([`nori_core::settings_store::BandPoint`]): `pos`
/// across the curve, its gain, Q, and kind and channel ordinals. 1 when the sound changed.
#[no_mangle]
pub extern "C" fn nori_ios_eq_point(index: u32, pos: f32, gain_db: f32, q: f32, kind: i32, channel: i32) -> i32 {
    with_session(|s| match s.core.session.settings.edit_point(index, nori_core::settings_store::BandPoint { pos, gain_db, q, kind, channel }) {
        Some(effect) => {
            s.applied(effect);
            1
        }
        None => 0,
    })
    .unwrap_or(0)
}

/// A parametric band added at `pos` across the curve and `gain_db`, or band `index` removed (`pos`
/// below 0). 1 when the sound changed.
#[no_mangle]
pub extern "C" fn nori_ios_eq_band(index: u32, pos: f32, gain_db: f32) -> i32 {
    use nori_core::settings_store::SoundTool;
    let tool = if pos < 0.0 { SoundTool::RemoveBand { index } } else { SoundTool::AddBand { freq: nori_core::dsp::band_freq_at(pos), gain_db } };
    sound_tool(tool)
}

/// The built-in equalizer curves, in menu order, by their `PresetKind` code for the client to word: a
/// JSON array to free.
#[no_mangle]
pub extern "C" fn nori_ios_presets() -> *mut c_char {
    owned(&json!(nori_core::dsp::eq_presets()
        .iter()
        .map(|p| p.kind as u8)
        .collect::<Vec<_>>()))
}

/// Applies curve `index` of [`nori_ios_presets`]: the equalizer on, its sliders following the curve.
/// 1 when the sound changed.
#[no_mangle]
pub extern "C" fn nori_ios_preset(index: u32) -> i32 {
    let Some(preset) = nori_core::dsp::eq_presets().into_iter().nth(index as usize) else {
        return 0;
    };
    sound_tool(nori_core::settings_store::SoundTool::Preset { preset })
}

/// 1 when `tool` changed the sound, which the engine then hears.
fn sound_tool(tool: nori_core::settings_store::SoundTool) -> i32 {
    with_session(|s| match s.core.session.settings.sound_tool(tool) {
        Ok(Some(change)) => {
            s.applied(change.effect);
            1
        }
        _ => 0,
    })
    .unwrap_or(0)
}

/// Sound page `page` (0 the equalizer, 1 AutoEQ's presets, 2 Settings › Sound) came in sight (`shown`
/// 1) or left it; a change made on one is then heard at once (`nori_core::tuning`).
#[no_mangle]
pub extern "C" fn nori_ios_sound_page(page: i32, shown: i32) {
    use nori_core::tuning::SoundPage;
    let page = match page {
        0 => SoundPage::Equalizer,
        1 => SoundPage::AutoEq,
        _ => SoundPage::Sound,
    };
    with_session(|s| s.sound_page(page, shown != 0));
}

/// Keeps `query` in the recent searches: the user picked a result or pressed search.
///
/// # Safety
/// `query` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_remember(query: *const c_char) {
    let query = c_text(query);
    with_session(|s| {
        let _ = s.core.search_remember_recent(query.trim().to_string());
    });
}

/// Fills the offline index from the server again.
#[no_mangle]
pub extern "C" fn nori_ios_sync() {
    with_session(|s| s.action(Chore::SyncLibrary));
}

/// The index and storage in numbers, as JSON to free.
#[no_mangle]
pub extern "C" fn nori_ios_facts() -> *mut c_char {
    with_session(|s| {
        let index = s.core.index_size().unwrap_or_default();
        let counts = s.core.download_counts();
        owned(&json!({
            "songs": index.songs, "albums": index.albums, "artists": index.artists,
            "downloaded": counts.done, "pending": counts.pending,
            "stream": s.store.as_ref().map_or(0, |st| st.cache_bytes()),
            // This profile's: the store holds every profile's downloads.
            "downloadBytes": s.store.as_ref().map_or(0, |st| st.download_bytes_of(&s.core.download_ids(true).unwrap_or_default())),
            "db": std::fs::metadata(&s.db).map(|m| m.len()).unwrap_or(0),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_core::ItemKey;

    fn song(id: &str) -> Song {
        Song {
            id: id.into(),
            title: id.into(),
            ..Song::default()
        }
    }

    #[test]
    fn the_setup_covers_come_decoded_and_one_past_them_comes_empty() {
        use std::sync::mpsc::{channel, Sender};
        let _turn = lock(&ABI_IN_USE);
        /// A picture handed: its token, width, height and bytes.
        type Handed = (u64, u32, u32, usize);
        static GOT: Mutex<Option<Sender<Handed>>> = Mutex::new(None);
        unsafe extern "C" fn got(token: u64, w: u32, h: u32, _: *const u8, len: usize, owner: *mut std::ffi::c_void, _: bool) {
            if let Some(sent) = &*lock(&GOT) {
                let _ = sent.send((token, w, h, len));
            }
            unsafe { nori_ios_cover_release(owner) };
        }
        let (sent, came) = channel();
        *lock(&GOT) = Some(sent);
        let hook = *lock(&COVER_HOOK);
        *lock(&COVER_HOOK) = Some(got);
        nori_ios_setup_cover(1, 0, 8);
        nori_ios_setup_cover(2, nori_ios_setup_covers(), 8);
        let mut got: Vec<_> = (0..2).filter_map(|_| came.recv_timeout(std::time::Duration::from_secs(10)).ok()).collect();
        *lock(&COVER_HOOK) = hook;
        got.sort();
        assert_eq!(got, [(1, 8, 8, 8 * 8 * 4), (2, 0, 0, 0)]);
    }

    #[test]
    fn a_handed_picture_lives_until_the_app_lets_go() {
        let _turn = lock(&ABI_IN_USE);
        static HELD: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        unsafe extern "C" fn keep(_: u64, _: u32, _: u32, _: *const u8, _: usize, owner: *mut std::ffi::c_void, _: bool) {
            lock(&HELD).push(owner as usize);
        }
        let hook = *lock(&COVER_HOOK);
        *lock(&COVER_HOOK) = Some(keep);
        let image = Arc::new(vec![1u8; 4]);
        hand(9, 1, 1, image.clone(), |v| v.as_slice(), false);
        *lock(&COVER_HOOK) = hook;
        assert_eq!(Arc::strong_count(&image), 2, "held while the app has it");
        let owner = lock(&HELD).pop().unwrap();
        unsafe { nori_ios_cover_release(owner as *mut std::ffi::c_void) };
        assert_eq!(Arc::strong_count(&image), 1);
    }

    #[test]
    fn play_pauses_only_the_queue_its_own_page_started() {
        let album = PageOrigin::new(OriginKind::Album, "a1");
        let other = PageOrigin::new(OriginKind::Album, "a2");
        let pausing = |bits: i32| bits & 4 != 0;
        let play_press = |bits: i32| (bits >> 6) & 3;
        let here = hero(Some(&album), &album, false, true, false);
        assert!(pausing(here));
        assert_eq!(play_press(here), nori_core::pages::HeroPress::Toggle as i32);
        let away = hero(Some(&other), &album, false, true, false);
        assert!(!pausing(away), "another page's queue playing");
        assert_eq!(play_press(away), nori_core::pages::HeroPress::Start as i32);
        assert!(!pausing(hero(Some(&album), &album, false, false, false)), "paused: Play again");
        assert!(pausing(hero(Some(&album), &album, false, false, true)), "waiting for bytes still pauses");
        let downloaded = page_origin(PAGE_DOWNLOADED_ALBUM, "a").unwrap();
        assert!(pausing(hero(Some(&downloaded), &downloaded, false, true, false)));
        assert!(!pausing(hero(Some(&album), &downloaded, false, true, false)));
        assert_eq!(page_origin(PAGE_GENRE, "g"), Some(PageOrigin::new(OriginKind::Genre, "g")));
        assert_eq!(page_origin(PAGE_HISTORY, ""), None);
    }

    #[test]
    fn the_stats_open_on_this_devices_year_and_read_the_choice_asked() {
        use nori_core::browse::{StatsPeriod, StatsSource};
        assert_eq!(stats_asked(""), (StatsPeriod::Year, StatsSource::Device, false));
        assert_eq!(stats_asked("AllTime\nServerAccount\n1"), (StatsPeriod::AllTime, StatsSource::ServerAccount, true));
        assert_eq!(stats_asked("Week\nDevice\n0"), (StatsPeriod::Week, StatsSource::Device, false));
    }

    #[test]
    fn the_queue_runs_in_play_order_split_at_the_song_playing() {
        let songs: Vec<Song> = ["a", "b", "c", "d"].map(song).to_vec();
        let rows = QueueRows { order: vec![2, 0, 3, 1], reorderable: false, kept: vec![0], now: 1, from: None };
        let v = queue_sections(&songs, &rows, 0, true, 0);
        let ids = |s: usize| -> Vec<String> {
            v["sections"][s]["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!((ids(0), ids(1), ids(2)), (vec!["c".to_string()], vec!["a".to_string()], vec!["d".to_string(), "b".to_string()]));
        assert_eq!(v["sections"][2]["items"][0]["i"], 3, "a row keeps its place in the list");
        let none = QueueRows { now: -1, ..rows };
        let v = queue_sections(&songs, &none, 0, true, -1);
        assert_eq!(v["sections"][2]["items"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn a_list_is_kept_by_its_token_and_old_ones_fall_off() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        for t in 0..(KEPT_LISTS as u64 + 2) {
            keep_list(1000 + t, vec![song(&t.to_string())], None);
        }
        assert!(list(1000).is_none());
        assert_eq!(list(1000 + KEPT_LISTS as u64 + 1).unwrap().0[0].id, (KEPT_LISTS + 1).to_string());
        keep_list(1001 + KEPT_LISTS as u64, vec![song("again")], None);
        assert_eq!(list(1001 + KEPT_LISTS as u64).unwrap().0[0].id, "again");
    }

    #[test]
    fn a_page_extends_its_kept_list_in_place() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        keep_list(90, vec![song("a"), song("b")], None);
        keep_list(91, vec![song("other")], None);
        assert_eq!(extend_list(90, &[song("c")]), Some(2));
        assert_eq!(list_song(90, 2).map(|s| s.id), Some("c".into()));
        assert_eq!(list_song(90, 3), None);
        assert_eq!(extend_list(92, &[song("c")]), None, "a list not kept is not made");
        for t in 0..(KEPT_LISTS as u64 - 1) {
            keep_list(2000 + t, vec![], None);
        }
        assert!(list(90).is_some(), "an extended list is the newest");
        assert!(list(91).is_none());
    }

    /// An item of another source, or a playlist of nori's own, crosses to the app as a ref that comes
    /// back whole; one of the profile's own source as its bare id, as before.
    #[test]
    fn an_item_crosses_with_where_it_is_kept() {
        let theirs = Album { id: "al".into(), source: Some("yt".into()), ..Album::default() };
        let sent = album_json(&theirs)["ref"].as_str().unwrap().to_string();
        assert_eq!(ItemRef::taken(&sent), ItemRef { id: "al".into(), source: Some("yt".into()), mixed: false });
        let mine = Playlist { id: "3".into(), mixed: true, ..Playlist::default() };
        assert_eq!(ItemRef::taken(playlist_json(&mine)["ref"].as_str().unwrap()).at(), nori_core::sources::PlaylistAt::Device);
        assert_eq!(album_json(&Album { id: "al".into(), ..Album::default() })["ref"], "al", "the profile's own: its id");
        assert_eq!(ItemRef::taken("al"), ItemRef { id: "al".into(), ..ItemRef::default() });
    }

    #[test]
    fn only_an_own_playlists_rows_move_and_go() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        let origin = Some(PageOrigin::new(OriginKind::Playlist, "p1"));
        keep(List { token: 70, songs: vec![song("a"), song("b")], origin: origin.clone(), editable: Some(Playlist { id: "p".into(), ..Playlist::default() }) });
        keep(List { token: 71, songs: vec![song("a"), song("b")], origin, editable: None });
        assert_eq!(list_edits(70, 0), [PlaylistEdit::MoveDown, PlaylistEdit::Remove]);
        let change = nori_host::session::PlaylistChange::Edit { songs: vec!["a".into(), "b".into()], index: 0, edit: PlaylistEdit::MoveDown };
        assert_eq!(list_edit(70, 0, PlaylistEdit::MoveDown), Some((Playlist { id: "p".into(), ..Playlist::default() }, change)));
        assert_eq!(list_edits(71, 0), []);
        assert_eq!(list_edit(71, 0, PlaylistEdit::Remove), None, "another's playlist changes nothing");
    }

    #[test]
    fn a_search_answer_keeps_its_songs_for_a_tap() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        let r = nori_core::SearchResult {
            songs: vec![song("a"), song("b"), song("c")],
            ..nori_core::SearchResult::default()
        };
        let session = nori_core::search::Session { text: "x".into(), answer: nori_core::search::Answer::Server(nori_core::search::search_split(r, "x")), ..Default::default() };
        let v = found(77, &session.view(), true);
        assert_eq!((v["sections"][0]["key"].as_str(), v["sections"][1]["key"].as_str()), (Some("top"), Some("songs")));
        assert_eq!(v["sections"][1]["items"][1]["i"], 2, "a song's place in the kept list");
        let (kept, origin) = list(77).unwrap();
        assert_eq!(kept.into_iter().map(|s| s.id).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(origin.map(|o| o.kind), Some(OriginKind::Search), "a tap plays the song alone");
    }

    #[test]
    fn an_artists_songs_follow_its_albums_and_play_as_the_artist() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        let artist = Artist { id: "yt-UC1".into(), ..Artist::default() };
        let v = artist_page(78, &nori_core::ArtistDetail::new(artist.clone(), vec![], vec![song("v1"), song("v2")]), None);
        assert_eq!((v["head"]["n"].as_u64(), v["head"]["ns"].as_u64()), (Some(0), Some(2)));
        assert_eq!(v["sections"][0]["key"], "songs");
        assert_eq!(v["sections"][0]["items"][1]["i"], 1);
        let (kept, origin) = list(78).unwrap();
        assert_eq!((kept.len(), origin), (2, Some(PageOrigin::new(OriginKind::Artist, "yt-UC1"))));
        let server = artist_page(79, &nori_core::ArtistDetail::new(artist, vec![], vec![]), None);
        assert_eq!(server["sections"].as_array().map(Vec::len), Some(0), "no songs, no section");
    }

    /// An artist page's about part: the biography and the pages to read on over its releases, the similar
    /// artists after them.
    #[test]
    fn an_artists_about_part_frames_its_releases() {
        let _turn = lock(&ABI_IN_USE);
        let about = nori_core::pages::ArtistAbout {
            biography: Some("Mira Vale writes slow records.".into()),
            last_fm: None,
            musicbrainz: Some("https://musicbrainz.org/artist/mb".into()),
            similar: vec![Artist { id: "ar2".into(), name: "Ferry Songs".into(), ..Artist::default() }],
        };
        let albums = vec![Album { id: "a".into(), ..Album::default() }];
        let v = artist_page(81, &nori_core::ArtistDetail::new(Artist { id: "ar".into(), ..Artist::default() }, albums, vec![]), Some(&about));
        let keys: Vec<&str> = v["sections"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["about", "release", "similar"]);
        let items = &v["sections"][0]["items"];
        assert_eq!((items[0]["k"].as_str(), items[0]["t"].as_str()), (Some("bio"), Some("Mira Vale writes slow records.")));
        assert_eq!((items[1]["k"].as_str(), items[1]["id"].as_str(), items[1]["t"].as_str()), (Some("link"), Some("https://musicbrainz.org/artist/mb"), Some("musicBrainz")));
        assert_eq!(v["sections"][2]["items"][0]["id"], "ar2");
    }

    #[test]
    fn an_artists_releases_stand_on_a_shelf_per_kind() {
        let _turn = lock(&ABI_IN_USE);
        let album = |id: &str, year: u32, kinds: &[&str], songs: u32| Album {
            id: id.into(),
            year,
            release_types: kinds.iter().map(|k| k.to_string()).collect(),
            song_count: songs,
            ..Album::default()
        };
        let albums = vec![album("a", 2001, &[], 10), album("s", 2003, &["Single"], 1), album("b", 2005, &[], 12)];
        let v = artist_page(80, &nori_core::ArtistDetail::new(Artist { id: "ar".into(), ..Artist::default() }, albums, vec![]), None);
        let shelf = |n: usize| {
            let s = &v["sections"][n];
            let ids: Vec<&str> = s["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
            (s["kind"].as_u64().unwrap(), ids)
        };
        let single = nori_core::pages::ReleaseKind::Single as u64;
        assert_eq!((shelf(0), shelf(1)), ((0, vec!["b", "a"]), (single, vec!["s"])), "albums newest first, then the singles");
        assert_eq!(v["head"]["albumSongs"], 23);
    }

    #[test]
    fn albums_by_artist_stand_on_a_shelf_per_artist() {
        let album = |id: &str, artist: &str| Album { id: id.into(), artist: artist.into(), artist_id: Some(artist.to_lowercase()), ..Album::default() };
        let shelves = artist_shelves(&[album("1", "Air"), album("2", "Air"), album("3", "Beck")]);
        let row = |s: &Value| (s["title"].as_str().unwrap().to_string(), s["artistId"].as_str().unwrap().to_string(), s["items"].as_array().unwrap().len());
        assert_eq!(shelves.iter().map(row).collect::<Vec<_>>(), [("Air".into(), "air".into(), 2), ("Beck".into(), "beck".into(), 1)]);
    }

    #[test]
    fn a_pages_head_offers_its_glyphs_with_what_each_takes() {
        let lines = head_actions(PageKind::Album, 8, 3, 5, true);
        assert_eq!(lines, [json!({ "a": 0, "act": 1, "n": 3 }), json!({ "a": 2 })]);
        let mix = head_actions(PageKind::Mix { refreshable: true }, 4, 0, 4, true);
        assert_eq!(mix, [json!({ "a": 3 }), json!({ "a": 0, "act": 2, "n": 4 }), json!({ "a": 2 })]);
        assert_eq!(head_actions(PageKind::Artist, 0, 0, 0, false), [json!({ "a": 2 })]);
    }

    #[test]
    fn every_home_row_has_a_shelf_key_that_opens_it_again() {
        for (row, key) in SHELVES {
            assert_eq!((shelf_key(row), shelf_row(key)), (key, Some(row)));
        }
        assert_eq!(shelf_row("mixes"), None);
    }

    #[test]
    fn a_mix_tile_names_its_mix_by_code_and_leads_with_its_first_cover() {
        let t = MixTile {
            id: "discover".into(),
            name: nori_core::mixes::board::MixName::Discover,
            covers: vec!["a".into(), "b".into()],
            favourites: false,
        };
        let v = mix_json(&t);
        assert_eq!(v["n"], 2);
        assert_eq!(v["c"], "a");
        let bare = mix_json(&MixTile { covers: vec![], ..t });
        assert_eq!(bare["c"], "");
    }

    #[test]
    fn the_work_card_says_the_bytes_then_the_work_after_them() {
        use nori_core::transfers::{Notice, NoticeKind, Processing, Progress};
        let n = Notice { kind: NoticeKind::OneNamed, position: 1, total: 1, current: "Blue".into(), permille: 420, eta_s: 30, ..Default::default() };
        let v = work_json(&Progress::Fetching(n));
        assert_eq!((v["stage"].as_str(), v["notice"].as_i64(), v["song"].as_str(), v["permille"].as_i64()), (Some("download"), Some(1), Some("Blue"), Some(420)));
        let w = work_json(&Progress::Processing { work: Processing { lyrics: 2, analysing: 1, beats: 0, eta_s: 9, permille: 500 }, after_fetching: true });
        assert_eq!((v["stages"].as_i64(), w["stages"].as_i64(), w["at"].as_i64(), w["lyrics"].as_i64()), (Some(1), Some(2), Some(1), Some(2)));
    }

    #[test]
    fn a_running_download_carries_its_progress_and_a_waiting_one_none() {
        let core = nori_core::Core::new(String::new(), "t".into(), Default::default()).unwrap();
        core.transfers().with(|t| {
            let slot = t.open(&ItemKey::own("a"), 0);
            t.note(slot, 1000, 450, 1_000);
        });
        let mut rows = songs_section("active", &[song("a"), song("b")], 0);
        with_progress(&core, &mut rows);
        assert_eq!(rows["items"][0]["pct"], 45);
        assert!(rows["items"][1].get("pct").is_none());
    }

    #[test]
    fn a_smart_row_names_a_builtin_by_code_and_a_saved_one_by_title() {
        let built = nori_core::SmartPlaylist {
            id: "default-most-played".into(),
            name: String::new(),
            json: "{}".into(),
            builtin: Some(nori_core::SmartBuiltin::MostPlayed),
        };
        let v = smart_json(&built);
        assert_eq!(v["k"], "smart");
        assert_eq!(v["n"], 0);
        assert_eq!(v["t"], "");
        let named = nori_core::SmartPlaylist {
            id: "mine".into(),
            name: "Late jazz".into(),
            json: "{}".into(),
            builtin: None,
        };
        let v = smart_json(&named);
        assert_eq!(v["t"], "Late jazz");
        assert!(v.get("n").is_none());
    }

    #[test]
    fn an_album_row_says_its_subtitle_when_read() {
        let a = Album {
            id: "1".into(),
            name: "Blue".into(),
            artist: "Joni".into(),
            subtitle: "Joni · 1971".into(),
            ..Album::default()
        };
        assert_eq!(album_json(&a)["s"], "Joni · 1971");
        let bare = Album {
            subtitle: String::new(),
            ..a
        };
        assert_eq!(album_json(&bare)["s"], "Joni");
    }

    #[test]
    fn music_folder_rows_show_switches_only_for_multiple_folders() {
        let folder = |id: &str, name: &str| nori_core::MusicFolder { id: id.into(), name: name.into() };
        assert!(music_folder_rows(vec![], vec![folder("1", "Music")]).is_empty());
        let rows = music_folder_rows(vec![], vec![folder("1", "Music"), folder("2", "Other")]);
        assert_eq!(rows, [json!({ "id": "1", "name": "Music", "on": true, "tapped": "2" }), json!({ "id": "2", "name": "Other", "on": true, "tapped": "1" })]);
        let one = music_folder_rows(vec!["2".into()], vec![folder("1", "Music"), folder("2", "Other")]);
        assert_eq!(one, [json!({ "id": "1", "name": "Music", "on": false, "tapped": "" }), json!({ "id": "2", "name": "Other", "on": true, "tapped": "2" })]);
    }
}
