//! The coarse path: one page per call, answered as JSON on a callback, cached copy first and the
//! server's after it when it differs. Lists of songs a page showed are kept by the request's token so a
//! tap plays from them without sending songs back across.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nori_core::browse::{album_sort_direction, album_sort_kept, album_sort_saved, album_sorts, artist_rows, favorites_in_order, home_pinned, home_row_more, home_row_page, next_album_page, shelf_albums_read, song_sort_direction, song_sort_kept, song_sort_saved, song_sorts, AlbumSort, HomeShelf, SortDirection};
use nori_core::cache_policy::{Page, Read};
use nori_core::numbers::{self, POINT};
use nori_core::client::Starrable;
use nori_core::menus::{download_entries, page_actions, DownloadAct, PageAction, PageKind, SongDownload};
use nori_core::pages::release_groups;
use nori_core::mixes::board::{MixLookup, MixTile, FAVOURITES_MIX};
use nori_core::stage::QueueRows;
use nori_core::{Album, Artist, Genre, Lyrics, OriginKind, PageOrigin, Playlist, SearchResult, Song};
use nori_covers::loader::Ticket;
use nori_engine::State;
use nori_host::session::{read_pages, Chore};
use nori_host::pages::{HomePart, ShelfItems};
use nori_host::Fetch;
use nori_core::settings::HomeRow;
use serde_json::{json, Value};

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

pub type PageFn = unsafe extern "C" fn(u64, *const c_char);
/// `(token, width, height, rgba, len, owner)`: the pixels stay valid until `owner` is given to
/// [`nori_ios_cover_release`], so the app draws from them without a copy.
pub type CoverFn = unsafe extern "C" fn(u64, u32, u32, *const u8, usize, *mut std::ffi::c_void);

/// What keeps a handed-out picture's pixels alive.
type Keep = Box<dyn std::any::Any + Send>;

/// Hands `keep`'s pixels (`pixels` of it) to the cover callback; without a callback they are dropped.
fn hand<T: Send + 'static>(token: u64, width: u32, height: u32, keep: T, pixels: fn(&T) -> &[u8]) {
    let Some(cb) = *lock(&COVER_HOOK) else { return };
    let (at, len) = {
        let p = pixels(&keep);
        (p.as_ptr(), p.len())
    };
    // Moving `keep` into the box does not move the heap buffer `at` points into.
    let owner = Box::into_raw(Box::new(Box::new(keep) as Keep)).cast::<std::ffi::c_void>();
    unsafe { cb(token, width, height, at, len, owner) };
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
}

static LISTS: Mutex<VecDeque<List>> = Mutex::new(VecDeque::new());
static TICKETS: Mutex<Vec<(u64, Ticket)>> = Mutex::new(Vec::new());
static LYRICS: Mutex<Option<(String, Lyrics)>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn keep_list(token: u64, songs: Vec<Song>, origin: Option<PageOrigin>) {
    let mut lists = lock(&LISTS);
    lists.retain(|l| l.token != token);
    lists.push_back(List {
        token,
        songs,
        origin,
    });
    while lists.len() > KEPT_LISTS {
        lists.pop_front();
    }
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
    let text = CString::new(v.to_string().replace('\0', " ")).unwrap_or_default();
    unsafe { cb(token, text.as_ptr()) };
}

fn send_error(token: u64, e: &nori_core::transport::NetError) {
    let (code, detail) = crate::account::fail(e.clone());
    send(token, &json!({ "error": code, "detail": detail.unwrap_or_default() }));
}

fn cover(c: &Option<String>) -> &str {
    c.as_deref().unwrap_or("")
}

pub(crate) fn song_json(s: &Song, i: usize) -> Value {
    json!({
        "k": "song", "id": s.id, "t": s.title, "s": s.artist, "a": s.album,
        "albumId": s.album_id.as_deref().unwrap_or(""), "artistId": s.artist_id.as_deref().unwrap_or(""),
        "c": cover(&s.cover_art), "d": s.duration, "i": i, "x": s.is_external, "n": s.track,
        "disc": s.disc_number, "st": s.starred,
    })
}

fn album_json(a: &Album) -> Value {
    let sub = if a.subtitle.is_empty() {
        &a.artist
    } else {
        &a.subtitle
    };
    json!({
        "k": "album", "id": a.id, "t": a.name, "s": sub, "c": cover(&a.cover_art), "y": a.year,
        "x": a.is_external, "artistId": a.artist_id.as_deref().unwrap_or(""),
    })
}

/// An artist's page: a shelf per kind of release (`pages::release_groups`; `kind` its `ReleaseKind` and
/// `tag` the server's for a tagged one, for the app to word), then the songs its provider lists, kept for
/// `token` to play from a tap.
fn artist_page(token: u64, v: &nori_core::ArtistDetail) -> Value {
    let a = &v.artist;
    keep_list(token, v.songs.clone(), Some(Fetch::Artist(a.id.clone()).origin()));
    let mut sections: Vec<Value> = release_groups(&v.albums)
        .into_iter()
        .map(|g| {
            let mut s = section("release", true, g.albums.iter().filter_map(|&i| v.albums.get(i as usize)).map(album_json).collect());
            s["kind"] = json!(g.kind as u8);
            s["tag"] = json!(g.tag);
            s
        })
        .collect();
    if !v.songs.is_empty() {
        sections.push(songs_section("songs", &v.songs, 0));
    }
    json!({
        "head": {
            "k": "artist", "id": a.id, "t": a.name, "c": cover(&a.cover_art), "n": v.albums.len(), "ns": v.songs.len(), "st": a.starred,
            "albumSongs": v.albums.iter().map(|al| al.song_count).sum::<u32>(),
        },
        "sections": sections,
    })
}

fn album_page(v: &nori_core::AlbumDetail) -> Value {
    let a = &v.album;
    json!({
        "head": {
            "k": "album", "id": a.id, "t": a.name, "s": a.artist,
            "artistId": a.artist_id.as_deref().unwrap_or(""), "c": cover(&a.cover_art),
            "y": a.year, "g": a.genre.as_deref().unwrap_or(""), "sec": v.seconds,
            "n": v.songs.len(), "st": a.starred,
        },
        "sections": [songs_section("songs", &v.songs, 0)],
    })
}

fn artist_json(a: &Artist) -> Value {
    json!({ "k": "artist", "id": a.id, "t": a.name, "n": a.album_count, "c": cover(&a.cover_art), "x": a.is_external })
}

/// A playlist row; `s` is its own line (a provider's, who made it), for one not counted until opened.
fn playlist_json(p: &Playlist) -> Value {
    let mut v = json!({ "k": "playlist", "id": p.id, "t": p.name, "n": p.song_count, "d": p.duration, "c": cover(&p.cover_art) });
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

fn genre_json(g: &Genre) -> Value {
    json!({ "k": "genre", "id": g.name, "t": g.name, "n": g.album_count, "m": g.song_count })
}

fn section(key: &str, grid: bool, items: Vec<Value>) -> Value {
    json!({ "key": key, "grid": grid, "items": items })
}

/// A feed's shelves as titled sections (key `feed`); their songs are kept as one list for `token`.
fn feed_sections(token: u64, feed: &[nori_core::FeedShelf]) -> Vec<Value> {
    let mut songs = Vec::new();
    let mut out = Vec::new();
    for f in feed {
        let titled = |mut v: Value| {
            v["title"] = json!(f.title);
            v
        };
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
    }
    keep_list(token, songs, None);
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

/// A search answer as four sections, its songs kept for `token`.
fn found(token: u64, r: &SearchResult, server: bool) -> Value {
    keep_list(token, r.songs.clone(), None);
    json!({
        "server": server,
        "sections": [
            section("artists", false, r.artists.iter().map(artist_json).collect()),
            section("albums", false, r.albums.iter().map(album_json).collect()),
            section("playlists", false, r.playlists.iter().map(playlist_json).collect()),
            songs_section("songs", &r.songs, 0),
        ],
    })
}

/// The first letter a long list is indexed by: A to Z, `#` for the rest.
fn letter(s: &str) -> char {
    match s.chars().next().map(|c| c.to_ascii_uppercase()) {
        Some(c) if c.is_ascii_uppercase() => c,
        _ => '#',
    }
}

/// `items`, each with the letter of the field the list is in the order of (`by`), when it is one.
fn lettered(mut items: Vec<Value>, by: Option<&str>) -> Vec<Value> {
    if let Some(by) = by {
        for v in &mut items {
            let l = letter(v[by].as_str().unwrap_or_default());
            v["l"] = json!(l.to_string());
        }
    }
    items
}

/// Which field of a row an album order runs by, for the A–Z index.
fn album_letters(sort: AlbumSort) -> Option<&'static str> {
    match sort {
        AlbumSort::ByName => Some("t"),
        AlbumSort::ByArtist => Some("s"),
        _ => None,
    }
}

/// Which field of a row a [`song_sorts`] order runs by, for the A–Z index.
fn song_letters(sort: &str) -> Option<&'static str> {
    match sort {
        "TITLE" => Some("t"),
        "ARTIST" => Some("s"),
        "ALBUM" => Some("a"),
        _ => None,
    }
}

/// A page of an album grid from `offset`, or with `favorites` the starred albums whole in `sort`'s order:
/// more follow while pages come full (`browse::next_album_page`).
fn album_grid(token: u64, client: &nori_core::client::Client, sort: AlbumSort, favorites: bool, genre: Option<String>, offset: i32) {
    let read = if favorites {
        Read::StarredItems
    } else {
        Read::AlbumList { kind: sort, size: ALBUM_PAGE, offset, genre }
    };
    pages(client, token, read, &|p| {
        let v = match p {
            Page::Albums { v } => v,
            Page::StarredPage { v } => favorites_in_order(v.albums, sort, false),
            _ => return None,
        };
        let sections = if sort == AlbumSort::ArtistRows {
            artist_shelves(&v)
        } else {
            vec![section("albums", true, lettered(v.iter().map(album_json).collect(), album_letters(sort)))]
        };
        Some(json!({
            "more": next_album_page(sort, favorites, offset as u32, ALBUM_PAGE as u32, v.len() as u32).is_some(),
            "next": offset + v.len() as i32, "from": offset,
            "letters": album_letters(sort).is_some(),
            "sections": sections,
        }))
    });
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
            s["artistId"] = json!(row[0].artist_id.as_deref().unwrap_or(""));
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
    let more = |got: usize| home_row_more(shelf.clone(), got as u32);
    match shelf.clone() {
        HomeShelf::Albums { sort, size, follows_stars } => {
            pages(client, token, shelf_albums_read(sort, size, follows_stars, offset), &|p| match p {
                Page::Albums { v } => Some(json!({
                    "more": more(v.len()), "next": offset + v.len() as u32, "from": offset,
                    "sections": [section(key, true, v.iter().map(album_json).collect())],
                })),
                _ => None,
            });
        }
        HomeShelf::Songs { sort, descending, limit } => {
            let songs = core.browse_songs(sort, descending, false, 0, 0, offset, limit).unwrap_or_default();
            let from = if offset == 0 { None } else { extend_list(token, &songs) }.unwrap_or(0);
            let v = json!({
                "more": more(songs.len()), "next": offset + songs.len() as u32, "from": offset,
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
            let id = v["id"].as_str().unwrap_or_default().to_string();
            if let (_, Some(f)) = t.row(&id) {
                v["pct"] = json!(f.percent);
                v["bps"] = json!(f.speed_bps);
                v["eta"] = json!(f.eta_s);
            }
        }
    });
}

fn list_prefs() -> HashMap<String, String> {
    with_session(|s| s.core.session.settings.prefs(|p| p.list_prefs.clone())).unwrap_or_default()
}

/// Reads `read`, sending each page `make` turns into an answer; an error only when nothing came.
fn pages(
    client: &nori_core::client::Client,
    token: u64,
    read: Read,
    make: &dyn Fn(Page) -> Option<Value>,
) {
    let mut any = false;
    let r = read_pages(client, read, |p| {
        if let Some(v) = make(p) {
            any = true;
            send(token, &v);
        }
    });
    match r {
        Ok(()) if !any => send(token, &json!({ "sections": [] })),
        Ok(()) => {}
        Err(e) => send_error(token, &e),
    }
}

fn read(token: u64, kind: i32, arg: String) {
    let Some((client, core)) = server() else {
        send(token, &json!({ "closed": true }));
        return;
    };
    match kind {
        // A source's own Home (`ProfileRules::feed`): its shelves, titled as it titles them; their songs one
        // list for taps.
        PAGE_HOME if core.rules().feed => pages(&client, token, Read::HomeFeed, &|p| {
            let Page::Feed { v } = p else { return None };
            Some(json!({ "sections": feed_sections(token, &v) }))
        }),
        PAGE_HOME => {
            let Some(h) = with_session(|s| nori_host::session::Handle::clone(s)) else { return };
            let rows = h.home_rows();
            // "For you", then the shelves in the order kept; a jam guest's mixes stay empty.
            let shown: Vec<Value> = std::iter::once(section("mixes", true, Vec::new())).chain(rows.iter().map(|&r| section(shelf_key(r), shelf_grid(r), Vec::new()))).collect();
            let shown = std::sync::Mutex::new(shown);
            h.read_home(rows, true, move |r| {
                let mut v = lock(&shown);
                match r {
                    Ok(HomePart::Feed(feed)) => return send(token, &json!({ "sections": feed_sections(token, &feed) })),
                    Ok(HomePart::Mixes(tiles)) => v[0] = section("mixes", true, tiles.iter().map(mix_json).collect()),
                    Ok(HomePart::Shelf { at, items }) => {
                        let key = v[at + 1]["key"].as_str().unwrap_or_default().to_string();
                        v[at + 1] = match items {
                            ShelfItems::Albums(a) => section(&key, true, a.iter().map(album_json).collect()),
                            ShelfItems::Playlists(p) => section(&key, true, p.iter().map(playlist_json).collect()),
                            ShelfItems::Songs(songs) => {
                                let shelf = songs_section(&key, &songs, 0);
                                keep_list(token, songs, None);
                                shelf
                            }
                        };
                    }
                    Err(e) => return send_error(token, &e),
                }
                send(token, &json!({ "sections": *v }));
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
            None,
            arg.parse().unwrap_or(0),
        ),
        PAGE_ARTISTS => pages(&client, token, Read::ArtistIndex, &|p| match p {
            Page::Artists { v } => Some(json!({
                "sections": [section("artists", false, v.iter().map(artist_json).collect())]
            })),
            _ => None,
        }),
        PAGE_PLAYLISTS => pages(&client, token, Read::PlaylistList, &|p| match p {
            Page::Playlists { v } => Some(json!({
                "sections": [section("playlists", false, v.iter().map(playlist_json).collect())]
            })),
            _ => None,
        }),
        PAGE_GENRES => pages(&client, token, Read::GenreList, &|p| match p {
            Page::Genres { v } => Some(json!({
                "sections": [section("genres", false, v.iter().map(genre_json).collect())]
            })),
            _ => None,
        }),
        PAGE_GENRE => {
            let (genre, offset) = arg.split_once('\n').unwrap_or((arg.as_str(), "0"));
            album_grid(token, &client, AlbumSort::ByGenre, false, Some(genre.into()), offset.parse().unwrap_or(0))
        }
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
            let sort = song_sort_saved(list_prefs());
            match nori_core::transport::block_on(client.songs_listed(sort.clone(), false, false, 0, 0, offset)) {
                Ok(p) => {
                    let kept = if offset == 0 { None } else { extend_list(token, &p.songs) };
                    let from = kept.unwrap_or(0);
                    let mut rows = songs_section("songs", &p.songs, from);
                    rows["items"] = json!(lettered(
                        rows["items"].as_array().cloned().unwrap_or_default(),
                        song_letters(&sort),
                    ));
                    let v = json!({
                        "more": !p.exhausted, "next": offset as usize + p.songs.len(),
                        "from": from, "letters": song_letters(&sort).is_some(), "sections": [rows],
                    });
                    if kept.is_none() {
                        keep_list(token, p.songs, None);
                    }
                    send(token, &v);
                }
                Err(e) => send_error(token, &e),
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
            send(token, &json!({ "running": !active.is_empty(), "sections": out }));
        }
        PAGE_DOWNLOADED_ALBUM => match core.downloaded_album(arg.clone()) {
            Ok(Some(v)) => {
                keep_list(token, v.songs.clone(), Some(v.queue.origin()));
                let mut value = album_page(&v);
                value["head"]["k"] = json!("downloadedAlbum");
                send(token, &value);
            }
            Ok(None) => send(token, &json!({ "sections": [] })),
            Err(e) => send_error(token, &e.into()),
        },
        PAGE_ALBUM => pages(&client, token, Read::AlbumById { id: arg.clone() }, &|p| match p {
            Page::AlbumPage { v } => {
                keep_list(token, v.songs.clone(), Some(Fetch::Album(v.album.id.clone()).origin()));
                Some(album_page(&v))
            }
            _ => None,
        }),
        PAGE_ARTIST => pages(&client, token, Read::ArtistById { id: arg.clone() }, &|p| match p {
            Page::ArtistPage { v } => Some(artist_page(token, &v)),
            _ => None,
        }),
        PAGE_PLAYLIST => pages(&client, token, Read::PlaylistById { id: arg.clone() }, &|p| match p {
            Page::PlaylistPage { v } => {
                keep_list(token, v.songs.clone(), Some(Fetch::Playlist(v.playlist.id.clone()).origin()));
                let pl = &v.playlist;
                Some(json!({
                    "head": {
                        "k": "playlist", "id": pl.id, "t": pl.name, "s": pl.owner.as_deref().unwrap_or(""),
                        "c": cover(&pl.cover_art), "sec": v.seconds, "n": v.songs.len(),
                        "note": pl.comment.as_deref().unwrap_or(""),
                    },
                    "sections": [songs_section("songs", &v.songs, 0)],
                }))
            }
            _ => None,
        }),
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
            if let Some(r) = &view.shown {
                send(token, &found(token, r, false));
            }
            let read = Read::Search {
                query: view.query.clone(),
                songs: 40,
                albums: 20,
                artists: 10,
            };
            match nori_core::transport::block_on(client.read_now(read)) {
                Ok(Page::Found { v }) => send(token, &found(token, &v, true)),
                Ok(_) => {}
                Err(e) if view.shown.is_none() => send_error(token, &e),
                Err(_) => {}
            }
        }
        _ => send(token, &json!({ "sections": [] })),
    }
}

/// The queue the player shows (`Handle::queue_shown`): this iPod's, the device's it mirrors (`remote`:
/// read only but for a tap, which plays that song there), or a jam guest's host's (`jam`), each song
/// with who asked for it in the jam (`by`) where someone did.
fn queue_json(s: &nori_host::session::Session) -> Value {
    let q = s.queue_shown(None);
    let shown = q.heard().map_or(-1, |i| i as i32);
    let mut v = queue_sections(&q.view.songs, &q.rows, q.view.repeat, q.view.shuffle, shown);
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
            "kept": rows.kept,
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
/// controls reach another device, or remote control is on here.
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
        "state": state, "index": n.index.map_or(-1, |i| i as i64), "ms": n.position_now().max(0),
        "pace": n.pace, "repeat": n.repeat, "shuffle": n.shuffle, "len": n.len, "mixing": n.mixing,
        "eq": here && s.core.session.settings.prefs(|p| p.eq_enabled), "remote": !n.jam && (!here || s.remote().is_some()),
        "buffering": n.buffering, "volume": n.volume.map(|v| (v * 100.0).round() as i64), "jam": n.jam,
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
    let Some((songs, origin)) = list(token) else { return 0 };
    with_session(|s| s.keep_playing(songs, index.max(0) as usize, origin) as i32).unwrap_or(0)
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
    let ids: Vec<&str> = songs.iter().map(|song| song.id.as_str()).collect();
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

/// Runs download entry `act` (as [`entries_of`] codes it) on `songs`: the songs downloaded, or removed
/// for 2.
fn act_on(s: &nori_host::session::Session, songs: Vec<Song>, act: i32) -> i32 {
    let states = download_states(s, &songs);
    let picked = |want: fn(&(bool, bool)) -> bool| -> Vec<Song> {
        songs.iter().zip(&states).filter(|(_, st)| want(st)).map(|(song, _)| song.clone()).collect()
    };
    let to_get = match act {
        0 => songs.clone(),
        1 => picked(|(here, _)| !here),
        2 => {
            let done = picked(|(_, done)| *done);
            done.iter().for_each(|song| s.download_remove(&song.id));
            return done.len() as i32;
        }
        _ => return 0,
    };
    let n = to_get.len() as i32;
    s.download(to_get);
    n
}

/// The songs of album, playlist or artist `id` (`kind` as in [`nori_ios_read`]): the stored copy, then
/// the server's; offline the stored one alone.
fn collection_songs(core: &nori_core::Core, client: &nori_core::client::Client, kind: i32, id: String) -> Vec<Song> {
    let mut songs = Vec::new();
    match kind {
        PAGE_DOWNLOADED_ALBUM => return core.downloaded_album(id).ok().flatten().map(|a| a.songs).unwrap_or_default(),
        PAGE_ARTIST => return nori_core::transport::block_on(client.artist_songs_of(id)).unwrap_or_default(),
        PAGE_ALBUM => {
            let _ = read_pages(client, Read::AlbumById { id }, |p| {
                if let Page::AlbumPage { v } = p {
                    songs = v.songs;
                }
            });
        }
        PAGE_PLAYLIST => {
            let _ = read_pages(client, Read::PlaylistById { id }, |p| {
                if let Page::PlaylistPage { v } = p {
                    songs = v.songs;
                }
            });
        }
        _ => {}
    }
    songs
}

/// List `token`'s download entries ([`entries_of`]) as JSON to free.
#[no_mangle]
pub extern "C" fn nori_ios_download_entries(token: u64) -> *mut c_char {
    let Some((songs, _)) = list(token) else { return std::ptr::null_mut() };
    with_session(|s| owned(&entries_of(s, &songs))).unwrap_or(std::ptr::null_mut())
}

/// What a page's list head offers for list `token` (`menus::page_actions`), as JSON to free: `[{a, act, n}]`,
/// `a` 0 download (`act` as [`entries_of`] codes it, `n` the songs it takes), 1 download everything by the
/// artist, 2 add to the queue, 3 a new draw of the mix, 4 edit. `kind` as in [`nori_ios_read`];
/// `refreshable`: a mix that can be drawn again.
#[no_mangle]
pub extern "C" fn nori_ios_page_actions(token: u64, kind: i32, refreshable: i32) -> *mut c_char {
    let kind = match kind {
        PAGE_ALBUM | PAGE_DOWNLOADED_ALBUM => PageKind::Album,
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
    let (songs, kind) = if albums != 0 {
        (nori_core::transport::block_on(client.shuffle_albums()), OriginKind::ShuffleAlbums)
    } else {
        (nori_core::transport::block_on(client.shuffle_all()), OriginKind::ShuffleSongs)
    };
    match songs {
        Ok(songs) if !songs.is_empty() => with_session(|s| s.play(songs, 0, false, Some(PageOrigin::new(kind, "")))).map_or(0, |()| 1),
        _ => 0,
    }
}

/// Runs download entry `act` on list `token` ([`act_on`]).
#[no_mangle]
pub extern "C" fn nori_ios_download_act(token: u64, act: i32) -> i32 {
    let Some((songs, _)) = list(token) else { return 0 };
    with_session(|s| act_on(s, songs, act)).unwrap_or(0)
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
    let songs = collection_songs(&core, &client, kind, id);
    with_session(|s| act_on(s, songs, act)).unwrap_or(0)
}

fn fetch(kind: i32, id: String) -> Option<Fetch> {
    match kind {
        PAGE_DOWNLOADED_ALBUM => Some(Fetch::DownloadedAlbum(id)),
        PAGE_ALBUM => Some(Fetch::Album(id)),
        PAGE_ARTIST => Some(Fetch::Artist(id)),
        PAGE_PLAYLIST => Some(Fetch::Playlist(id)),
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

/// Stars (`on` 1) or unstars a song (1), album (2) or artist (3).
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_star(kind: i32, id: *const c_char, on: i32) {
    let Some(kind) = starrable(kind) else { return };
    let id = c_text(id);
    with_session(|s| s.star(kind, id, on != 0));
}

/// Whether a song (1), album (2) or artist (3) shows starred (1), its record saying `listed`: a song as
/// the device playing elsewhere shows it while it is in that queue, else this session's mark over the
/// record (`Handle::starred`).
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_starred(kind: i32, id: *const c_char, listed: i32) -> i32 {
    let (Some(kind), id, listed) = (starrable(kind), c_text(id), listed != 0) else { return listed };
    let shown = with_session(|s| match kind {
        Starrable::Song => s.song_starred(&id, listed),
        _ => s.core.starred(kind, &id, listed),
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
/// memory cache, disk, or server. A failure has zero dimensions and no owner; cancellation has no answer.
///
/// # Safety
/// `id` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_cover(token: u64, id: *const c_char, px: u32) {
    request(token, c_text(id), px, move |image| {
        // The decoded image's own buffer, shared with the loader's memory cache.
        if let Some(image) = image {
            let (w, h) = (image.width, image.height);
            hand(token, w, h, image, |i| &i.pixels);
        } else if let Some(cb) = *lock(&COVER_HOOK) {
            unsafe { cb(token, 0, 0, std::ptr::null(), 0, std::ptr::null_mut()) };
        }
    });
}

/// Asks for cover `id` at `px`, kept under `token` until it comes or is let go.
pub(crate) fn request(token: u64, id: String, px: u32, deliver: impl FnOnce(Option<Arc<nori_covers::Image>>) + Send + 'static) {
    if id.is_empty() || px == 0 {
        return;
    }
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let ticket = with_session(|s| {
        s.cover(id, px, false, move |image, _| {
            finished.store(true, Ordering::Release);
            lock(&TICKETS).retain(|(t, _)| *t != token);
            deliver(image.ok());
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
    for (id, size) in current.chain(around.wants.into_iter().map(|w| (w.id, w.size))) {
        loader.warm(&s.core.cover_address(id, size));
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

/// Keeps the newest lyrics answer for `song`, for [`nori_ios_lyrics_page`].
pub(crate) fn lyrics_arrived(song: &str, lyrics: &Lyrics) {
    *lock(&LYRICS) = Some((song.to_string(), lyrics.clone()));
}

/// The lyrics held for `song`, if they are the last that came.
pub(crate) fn held_lyrics(song: &str) -> Option<Lyrics> {
    lock(&LYRICS).as_ref().filter(|(id, _)| id == song).map(|(_, l)| l.clone())
}

/// Asks for `song`'s lyrics; each better answer is a lyrics report.
///
/// # Safety
/// `song` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_lyrics(song: *const c_char) {
    let song = c_text(song);
    if song.is_empty() {
        return;
    }
    with_session(|s| s.lyrics(song));
}

/// The lyrics last found for `song`, as JSON to free; NULL when none came yet.
///
/// # Safety
/// `song` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_lyrics_page(song: *const c_char) -> *mut c_char {
    let song = c_text(song);
    let held: std::sync::MutexGuard<'_, Option<(String, Lyrics)>> = lock(&LYRICS);
    let Some((id, l)) = held.as_ref().filter(|(id, _)| *id == song) else {
        return std::ptr::null_mut();
    };
    let lines: Vec<Value> = l
        .lines
        .iter()
        .map(|line| json!({ "s": line.start_ms, "e": line.end_ms, "t": line.text }))
        .collect();
    owned(&json!({ "id": id, "synced": l.synced, "offset": l.timing.offset_ms(), "lines": lines }))
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
        "core": list(nori_core::credits::core_credits()),
        "data": list(nori_core::credits::data_credits()),
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

/// The equalizer screen is in sight (`in_sight`) and was touched: the output's short buffer follows
/// the core's rule.
#[no_mangle]
pub extern "C" fn nori_ios_tuning(in_sight: i32, touched: i32) {
    with_session(|s| s.engine.set_shallow(nori_core::rules::sound_tuning(in_sight != 0, touched != 0)));
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

    fn song(id: &str) -> Song {
        Song {
            id: id.into(),
            title: id.into(),
            ..Song::default()
        }
    }

    #[test]
    fn a_handed_picture_lives_until_the_app_lets_go() {
        let _turn = lock(&ABI_IN_USE);
        static HELD: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        unsafe extern "C" fn keep(_: u64, _: u32, _: u32, _: *const u8, _: usize, owner: *mut std::ffi::c_void) {
            lock(&HELD).push(owner as usize);
        }
        let hook = *lock(&COVER_HOOK);
        *lock(&COVER_HOOK) = Some(keep);
        let image = Arc::new(vec![1u8; 4]);
        hand(9, 1, 1, image.clone(), |v| v.as_slice());
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
        assert_eq!(page_origin(PAGE_GENRE, "g"), None);
    }

    #[test]
    fn the_queue_runs_in_play_order_split_at_the_song_playing() {
        let songs: Vec<Song> = ["a", "b", "c", "d"].map(song).to_vec();
        let rows = QueueRows { order: vec![2, 0, 3, 1], reorderable: false, kept: vec![0], now: 1 };
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

    #[test]
    fn a_search_answer_keeps_its_songs_for_a_tap() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        let r = SearchResult {
            songs: vec![song("a"), song("b")],
            ..SearchResult::default()
        };
        let v = found(77, &r, true);
        assert_eq!(v["sections"][3]["items"][1]["i"], 1);
        let kept: Vec<String> = list(77).unwrap().0.into_iter().map(|s| s.id).collect();
        assert_eq!(kept, ["a", "b"]);
    }

    #[test]
    fn an_artists_songs_follow_its_albums_and_play_as_the_artist() {
        let _turn = lock(&ABI_IN_USE);
        forget_lists();
        let artist = Artist { id: "yt-UC1".into(), ..Artist::default() };
        let v = artist_page(78, &nori_core::ArtistDetail::new(artist.clone(), vec![], vec![song("v1"), song("v2")]));
        assert_eq!((v["head"]["n"].as_u64(), v["head"]["ns"].as_u64()), (Some(0), Some(2)));
        assert_eq!(v["sections"][0]["key"], "songs");
        assert_eq!(v["sections"][0]["items"][1]["i"], 1);
        let (kept, origin) = list(78).unwrap();
        assert_eq!((kept.len(), origin), (2, Some(PageOrigin::new(OriginKind::Artist, "yt-UC1"))));
        let server = artist_page(79, &nori_core::ArtistDetail::new(artist, vec![], vec![]));
        assert_eq!(server["sections"].as_array().map(Vec::len), Some(0), "no songs, no section");
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
        let v = artist_page(80, &nori_core::ArtistDetail::new(Artist { id: "ar".into(), ..Artist::default() }, albums, vec![]));
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
    fn rows_carry_the_letter_of_the_field_their_order_runs_by() {
        let rows = || vec![json!({ "t": "blue", "s": "Joni" }), json!({ "t": "4 Way", "s": "Émile" })];
        let by_name = lettered(rows(), album_letters(AlbumSort::ByName));
        assert_eq!((by_name[0]["l"].as_str(), by_name[1]["l"].as_str()), (Some("B"), Some("#")));
        let by_artist = lettered(rows(), album_letters(AlbumSort::ByArtist));
        assert_eq!((by_artist[0]["l"].as_str(), by_artist[1]["l"].as_str()), (Some("J"), Some("#")));
        assert!(lettered(rows(), album_letters(AlbumSort::Newest))[0].get("l").is_none());
        assert_eq!(lettered(rows(), song_letters("ARTIST"))[0]["l"], "J");
        assert!(lettered(rows(), song_letters("ADDED"))[0].get("l").is_none());
    }

    #[test]
    fn a_running_download_carries_its_progress_and_a_waiting_one_none() {
        let core = nori_core::Core::new(String::new(), "t".into(), Default::default()).unwrap();
        core.transfers().with(|t| {
            let slot = t.open("a", 0);
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
}
