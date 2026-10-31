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
    /// `sort` as the server calls it (`newest`, `alphabeticalByName`, ...); with `favorites` the starred
    /// albums only, all in one answer.
    Albums {
        sort: String,
        size: i32,
        offset: i32,
        #[serde(default)]
        favorites: bool,
    },
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
    /// The offline index's answer to `query`, at once; null with nothing there.
    SearchIndex { query: String },
}

fn album_sort(name: &str) -> Result<AlbumSort, String> {
    AlbumSort::of_name(name).ok_or(format!("no album order {name}"))
}

impl Ask {
    fn read(self) -> Result<Read, String> {
        Ok(match self {
            Ask::Albums { favorites: true, .. } => Read::StarredItems,
            Ask::Albums { sort, size, offset, favorites: false } => Read::AlbumList { kind: album_sort(&sort)?, size, offset, genre: None },
            Ask::Album { id } => Read::AlbumById { id },
            Ask::Artists => Read::ArtistIndex,
            Ask::Artist { id } => Read::ArtistById { id },
            Ask::Playlists => Read::PlaylistList,
            Ask::Playlist { id } => Read::PlaylistById { id },
            Ask::Genres => Read::GenreList,
            Ask::Genre { .. } => return Err("a genre is read whole, by genre_songs".into()),
            Ask::Songs { offset, count } => Read::SongPage { offset, count },
            Ask::Starred => Read::StarredItems,
            Ask::Search { query } => Read::Search { query, songs: 40, albums: 20, artists: 20 },
            Ask::SearchIndex { .. } => return Err("read from the index, not the server".into()),
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

/// Asks the server for `change`; kept for later when offline.
pub async fn change(client: &Client, change: Change) -> Result<(), String> {
    use nori_core::client::{Starrable, Write};
    let w = match change {
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

/// A search's answer `result`, ranked as every client's is, with the field's preview of it.
fn found_page(core: &nori_core::Core, result: nori_core::SearchResult, query: &str, user: Option<&str>) -> Value {
    let found = nori_core::search::search_split(result, query).everything;
    let mut v = page(core, Page::Found { v: found.clone() }, user);
    if let Value::Object(m) = &mut v {
        m.insert("playlists".into(), found.playlists.iter().map(|p| playlist(core, p, user)).collect());
        m.insert("preview".into(), json!(nori_core::search::search_preview(&found)));
    }
    v
}

/// Reads what `ask` (JSON) names: the stored copy, then the server's if it differs; the last as JSON. An
/// album list's answer says where its `next` page starts, or null.
/// `user`: the account signed in, whose own playlists it may change (the core's `playlist_editable`);
/// None for a profile without an account's things.
pub async fn read(client: &Client, ask: &str, user: Option<&str>) -> Result<String, String> {
    let ask = serde_json::from_str::<Ask>(ask).map_err(|e| e.to_string())?;
    if let Ask::SearchIndex { query } = &ask {
        let limit = nori_core::browse::library_sizes().local_search;
        let found = client.core().local_search(query.clone(), limit).map_err(|e| e.to_string())?;
        if found.artists.is_empty() && found.albums.is_empty() && found.songs.is_empty() {
            return Ok(Value::Null.to_string());
        }
        return Ok(found_page(client.core(), found, query, user).to_string());
    }
    if let Ask::Genre { name } = ask {
        let v = client.genre_songs(name).await.map_err(|e| e.to_string())?;
        return Ok(page(client.core(), Page::Songs { v }, user).to_string());
    }
    let paged = match &ask {
        Ask::Albums { sort, size, offset, favorites } => Some((album_sort(sort)?, *favorites, *offset as u32, *size as u32)),
        _ => None,
    };
    let query = match &ask {
        Ask::Search { query } => Some(query.clone()),
        _ => None,
    };
    let mut last = None;
    client.read_each(ask.read()?, |p| last = Some(p)).await.map_err(|e| e.to_string())?;
    let core = client.core();
    let p = match (last, paged) {
        (None, _) => return Ok(Value::Null.to_string()),
        (Some(Page::StarredPage { v }), Some((sort, true, ..))) => Page::Albums { v: nori_core::browse::favorites_in_order(v.albums, sort, false) },
        (Some(p), _) => p,
    };
    // A search's answer ranked as every client's is, with the field's preview of it.
    if let (Some(query), Page::Found { v }) = (query, &p) {
        return Ok(found_page(core, v.clone(), &query, user).to_string());
    }
    let got = match &p {
        Page::Albums { v } => v.len() as u32,
        _ => 0,
    };
    let mut v = page(core, p, user);
    if let (Some((sort, favorites, offset, asked)), Value::Object(m)) = (paged, &mut v) {
        m.insert("next".into(), json!(nori_core::browse::next_album_page(sort, favorites, offset, asked, got)));
    }
    Ok(v.to_string())
}

/// The season's paper the mix cards are dyed on: light, dark, and whether the dark is worn.
pub struct Papers {
    pub light: nori_look::theme::Palette,
    pub dark: nori_look::theme::Palette,
    pub dark_theme: bool,
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

/// Home's "For you", as the desktop's: the mixes' tiles (the core's `mix_cards`), each with its name (the
/// core's `MixName`), its covers' addresses and its card's paper and ink (`mix_card_colours`). `warm`: the
/// server's starred songs and this period's draws are brought up to date first; otherwise the stored
/// favourites are handed over, so it answers at once. Empty where the profile has no mixes.
pub async fn mixes(client: &Client, warm: bool, taste: bool, papers: &Papers) -> String {
    use nori_core::mixes::board::mix_card_colours;
    // A network that does not answer leaves the tiles as they were drawn last.
    if warm {
        let _ = client.mix_hand_favourites().await;
        client.mix_warm_all().await;
    } else {
        let _ = client.mix_favourites_stored();
    }
    let core = client.core();
    let tiles: Vec<Value> = core
        .mix_cards(taste)
        .into_iter()
        .map(|t| {
            let card = mix_card_colours(t.id.clone(), papers.light, papers.dark, papers.dark_theme);
            json!({
                "id": t.id, "name": format!("{:?}", t.name), "favourites": t.favourites,
                "covers": t.covers.iter().map(|c| cover(core, c, 300)).collect::<Vec<_>>(),
                "page": hex(card.page), "ink": hex(card.ink),
            })
        })
        .collect();
    Value::from(tiles).to_string()
}

/// Mix `id`'s page: its name, covers, songs as they play and their length; null for an id no mix has.
pub async fn mix(client: &Client, id: String) -> Result<String, String> {
    use nori_core::mixes::board::MixLookup;
    client.mix_songs(id.clone()).await.map_err(|e| e.to_string())?;
    let core = client.core();
    Ok(match core.mix_page(id) {
        MixLookup::Ready { sheet } => json!({
            "id": sheet.id, "name": format!("{:?}", sheet.name), "favourites": sheet.favourites, "seconds": sheet.seconds,
            "covers": sheet.covers.iter().map(|c| cover(core, c, 300)).collect::<Vec<_>>(), "songs": songs(core, &sheet.songs),
        }),
        MixLookup::NotDrawn | MixLookup::Unknown => Value::Null,
    }
    .to_string())
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

/// Playlist `p`, with whether `user` may change it.
fn playlist(core: &Core, p: &nori_core::Playlist, user: Option<&str>) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        if let Some(art) = p.cover_art.as_ref() {
            m.insert("cover".into(), Value::String(cover(core, art, 300)));
        }
        m.insert("editable".into(), Value::Bool(user.is_some_and(|u| nori_core::menus::playlist_editable(p, u, true))));
    }
    v
}

/// How many of `v` show until Show all is pressed.
fn few(v: &[Song]) -> u32 {
    nori_core::browse::songs_shown(v.len() as u32, false)
}

/// A page as the web draws it.
fn page(core: &Core, p: Page, user: Option<&str>) -> Value {
    match p {
        Page::Albums { v } => json!({ "albums": albums(core, &v) }),
        Page::Artists { v } => json!({ "artists": artists(core, &v) }),
        Page::AlbumPage { v } => json!({
            "album": album(core, &v.album), "songs": songs(core, &v.songs), "seconds": v.seconds,
            "discs": v.discs.iter().map(|d| json!({ "disc": d.disc, "headed": d.headed, "title": d.title, "songs": d.songs, "lines": d.lines })).collect::<Value>(),
            "quality": v.songs.first().and_then(|s| nori_core::numbers::quality(&s.suffix, s.bit_depth, s.sampling_rate, s.bit_rate)),
        }),
        Page::ArtistPage { v } => json!({
            "artist": artist(core, &v.artist), "albums": albums(core, &v.albums), "songs": songs(core, &v.songs), "few": few(&v.songs),
            // The picture across the page's head, at the full size.
            "picture": v.artist.cover_art.as_ref().map(|art| cover(core, art, 800)),
            "groups": v.groups.iter().map(|g| json!({ "kind": format!("{:?}", g.kind), "tag": g.tag, "albums": g.albums })).collect::<Value>(),
        }),
        Page::Songs { v } => json!({ "songs": songs(core, &v) }),
        Page::Playlists { v } => json!({ "playlists": v.iter().map(|p| playlist(core, p, user)).collect::<Value>() }),
        Page::PlaylistPage { v } => json!({ "playlist": playlist(core, &v.playlist, user), "songs": songs(core, &v.songs), "seconds": v.seconds }),
        Page::StarredPage { v } => json!({ "artists": artists(core, &v.artists), "albums": albums(core, &v.albums), "songs": songs(core, &v.songs) }),
        Page::Genres { v } => json!({ "genres": v }),
        Page::Found { v } => json!({ "artists": artists(core, &v.artists), "albums": albums(core, &v.albums), "songs": songs(core, &v.songs), "few": few(&v.songs) }),
        _ => Value::Null,
    }
}
