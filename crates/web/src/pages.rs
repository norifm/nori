//! What the page asks to read, as the core's reads, and their answers as JSON for it to draw.

use nori_core::browse::AlbumSort;
use nori_core::cache_policy::{Page, Read};
use nori_core::client::{Client, Starrable};
use nori_core::{Album, Artist, Core, Song};
use serde::Deserialize;
use serde_json::{json, Value};

/// A page's read, as the page names it.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Ask {
    /// `sort` as the server calls it (`newest`, `alphabeticalByName`, ...).
    Albums { sort: String, size: i32, offset: i32 },
    Album { id: String },
    Artists,
    Artist { id: String },
    Playlists,
    Playlist { id: String },
    Genres,
    Genre { name: String },
    Songs { offset: i32, count: i32 },
    Starred,
    Search { query: String },
}

impl Ask {
    fn read(self) -> Result<Read, String> {
        Ok(match self {
            Ask::Albums { sort, size, offset } => Read::AlbumList { kind: AlbumSort::of_name(&sort).ok_or(format!("no album order {sort}"))?, size, offset, genre: None },
            Ask::Album { id } => Read::AlbumById { id },
            Ask::Artists => Read::ArtistIndex,
            Ask::Artist { id } => Read::ArtistById { id },
            Ask::Playlists => Read::PlaylistList,
            Ask::Playlist { id } => Read::PlaylistById { id },
            Ask::Genres => Read::GenreList,
            Ask::Genre { name } => Read::SongsByGenre { genre: name, count: 500 },
            Ask::Songs { offset, count } => Read::SongPage { offset, count },
            Ask::Starred => Read::StarredItems,
            Ask::Search { query } => Read::Search { query, songs: 40, albums: 20, artists: 20 },
        })
    }
}

/// A change to the library, as the page names it.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Change {
    #[serde(rename_all = "camelCase")]
    CreatePlaylist { name: String, song_ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    AddToPlaylist { id: String, song_ids: Vec<String> },
    /// The song at list index `index` of playlist `id`.
    RemoveFromPlaylist { id: String, index: i32 },
    RenamePlaylist { id: String, name: String },
    /// The playlist's songs in their new order.
    #[serde(rename_all = "camelCase")]
    ReorderPlaylist { id: String, song_ids: Vec<String> },
    DeletePlaylist { id: String },
    /// A heart on a song, album or artist (`what`), or off.
    Star { what: String, id: String, on: bool },
}

/// The page's hearts need no word when the marks move: it reads its pages again.
struct Unshown;

impl nori_core::library::StarsShown for Unshown {
    fn marks(&self, _: nori_core::stars::StarMarks) {}
}

/// Asks the server for `change` (JSON); kept for later when offline.
pub async fn change(client: &Client, change: &str) -> Result<(), String> {
    use nori_core::client::{Starrable, Write};
    let w = match serde_json::from_str::<Change>(change).map_err(|e| e.to_string())? {
        Change::CreatePlaylist { name, song_ids } => Write::CreatePlaylist { name, song_ids },
        Change::AddToPlaylist { id, song_ids } => Write::AddToPlaylist { id, song_ids },
        Change::RemoveFromPlaylist { id, index } => Write::RemoveFromPlaylist { id, index },
        Change::RenamePlaylist { id, name } => Write::RenamePlaylist { id, name },
        Change::ReorderPlaylist { id, song_ids } => Write::ReplacePlaylist { id, song_ids },
        Change::DeletePlaylist { id } => Write::DeletePlaylist { id },
        Change::Star { what, id, on } => {
            let kind = match what.as_str() {
                "song" => Starrable::Song,
                "album" => Starrable::Album,
                "artist" => Starrable::Artist,
                _ => return Err(format!("no heart for {what}")),
            };
            return client.star(kind, id, on, std::sync::Arc::new(Unshown)).await.map_err(|e| e.to_string());
        }
    };
    client.write(w).await.map_err(|e| e.to_string())
}

/// Reads what `ask` (JSON) names: the stored copy, then the server's if it differs; the last as JSON.
pub async fn read(client: &Client, ask: &str) -> Result<String, String> {
    let read = serde_json::from_str::<Ask>(ask).map_err(|e| e.to_string())?.read()?;
    let mut last = None;
    client.read_each(read, |p| last = Some(p)).await.map_err(|e| e.to_string())?;
    let core = client.core();
    Ok(last.map_or(Value::Null, |p| page(core, p)).to_string())
}

/// A cover's address for `px` square.
pub fn cover(core: &Core, id: &str, px: u32) -> String {
    core.cover_address(id.to_string(), nori_core::covers::cover_rendition(px))
}

/// `item` as JSON with its cover's address, and its heart as this session marked it (`starred`).
fn with_cover<T: serde::Serialize>(core: &Core, item: &T, art: Option<&String>, heart: (Starrable, &str, bool)) -> Value {
    let mut v = serde_json::to_value(item).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        if let Some(art) = art {
            m.insert("cover".into(), Value::String(cover(core, art, 300)));
        }
        m.insert("starred".into(), Value::Bool(core.starred(heart.0, heart.1, heart.2)));
    }
    v
}

fn album(core: &Core, a: &Album) -> Value {
    with_cover(core, a, a.cover_art.as_ref(), (Starrable::Album, &a.id, a.starred))
}

fn artist(core: &Core, a: &Artist) -> Value {
    with_cover(core, a, a.cover_art.as_ref(), (Starrable::Artist, &a.id, a.starred))
}

fn albums(core: &Core, v: &[Album]) -> Value {
    v.iter().map(|a| album(core, a)).collect()
}

fn artists(core: &Core, v: &[Artist]) -> Value {
    v.iter().map(|a| artist(core, a)).collect()
}

/// A song as the web draws it.
pub fn song(core: &Core, s: &Song) -> Value {
    with_cover(core, s, s.cover_art.as_ref(), (Starrable::Song, &s.id, s.starred))
}

fn songs(core: &Core, v: &[Song]) -> Value {
    v.iter().map(|s| song(core, s)).collect()
}

fn playlist(core: &Core, p: &nori_core::Playlist) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    if let (Value::Object(m), Some(art)) = (&mut v, p.cover_art.as_ref()) {
        m.insert("cover".into(), Value::String(cover(core, art, 300)));
    }
    v
}

/// How many of `v` show until Show all is pressed.
fn few(v: &[Song]) -> u32 {
    nori_core::browse::songs_shown(v.len() as u32, false)
}

/// A page as the web draws it.
fn page(core: &Core, p: Page) -> Value {
    match p {
        Page::Albums { v } => json!({ "albums": albums(core, &v) }),
        Page::Artists { v } => json!({ "artists": artists(core, &v) }),
        Page::AlbumPage { v } => json!({ "album": album(core, &v.album), "songs": songs(core, &v.songs), "seconds": v.seconds }),
        Page::ArtistPage { v } => json!({ "artist": artist(core, &v.artist), "albums": albums(core, &v.albums), "songs": songs(core, &v.songs), "few": few(&v.songs) }),
        Page::Songs { v } => json!({ "songs": songs(core, &v) }),
        Page::Playlists { v } => json!({ "playlists": v.iter().map(|p| playlist(core, p)).collect::<Value>() }),
        Page::PlaylistPage { v } => json!({ "playlist": playlist(core, &v.playlist), "songs": songs(core, &v.songs), "seconds": v.seconds }),
        Page::StarredPage { v } => json!({ "artists": artists(core, &v.artists), "albums": albums(core, &v.albums), "songs": songs(core, &v.songs) }),
        Page::Genres { v } => json!({ "genres": v }),
        Page::Found { v } => json!({ "artists": artists(core, &v.artists), "albums": albums(core, &v.albums), "songs": songs(core, &v.songs), "few": few(&v.songs) }),
        _ => Value::Null,
    }
}
