//! A local profile's pages, read from the offline index its sync fills: what a Subsonic server answers
//! from its own database, answered here the same way. Folders are the folders the files are in.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Params};

use super::scan::{Playlist as Found, PLAYLISTS};
use crate::browse::AlbumSort;
use crate::cache_policy::{Page, Read};
use crate::client::Starrable;
use crate::db::items::{self, ALBUM, ARTIST, SONG};
use crate::db::{self, ItemKind};
use crate::{Album, AlbumDetail, Artist, ArtistDetail, Core, Directory, Genre, Playlist, PlaylistDetail, Result, SearchResult, Song, Starred};

/// What a folder's id starts with; the rest is its path.
const FOLDER: &str = "dir:";

fn songs(c: &Connection, tail: &str, p: impl Params) -> rusqlite::Result<Vec<Song>> {
    let sql = format!("SELECT {SONG} FROM items i WHERE server=sid() AND kind={} {tail}", ItemKind::Song as i64);
    c.prepare_cached(&sql)?.query_map(p, |r| items::song(r, 0))?.collect()
}

fn albums(c: &Connection, tail: &str, p: impl Params) -> rusqlite::Result<Vec<Album>> {
    let sql = format!("SELECT {ALBUM} FROM items i WHERE server=sid() AND kind={} {tail}", ItemKind::Album as i64);
    c.prepare_cached(&sql)?.query_map(p, |r| items::album(r, 0))?.collect()
}

fn artists(c: &Connection, tail: &str, p: impl Params) -> rusqlite::Result<Vec<Artist>> {
    let sql = format!("SELECT {ARTIST} FROM items i WHERE server=sid() AND kind={} {tail}", ItemKind::Artist as i64);
    c.prepare_cached(&sql)?.query_map(p, |r| items::artist(r, 0))?.collect()
}

/// Albums with the plays of their songs on this device: when one was last heard, and how often.
const PLAYED: &str = "JOIN (SELECT so.album_id AS a, max(s.last_played_ms) AS last, sum(s.plays) AS plays FROM song_stats s
  JOIN items so ON so.server=s.server AND so.kind=2 AND so.id=s.song_id WHERE s.server=sid() GROUP BY so.album_id) p ON p.a=i.id";

/// `read`'s page from the index of `core`, whose profile reads `folders`; None for a read it has nothing for.
pub(super) fn page(core: &Core, read: Read, folders: &[String]) -> Result<Option<Page>> {
    let c = core.db.lock();
    let c = &*c;
    let page = match read {
        Read::AlbumList { kind, size, offset, genre } => {
            let v = match (kind, genre) {
                (_, Some(g)) => albums(c, "AND genre = ?3 COLLATE NOCASE ORDER BY name_key LIMIT ?1 OFFSET ?2", params![size, offset, g])?,
                (AlbumSort::Recent, None) => played(c, "p.last", size, offset)?,
                (AlbumSort::Frequent, None) => played(c, "p.plays", size, offset)?,
                (AlbumSort::Random, None) => albums(c, "ORDER BY random() LIMIT ?1 OFFSET ?2", params![size, offset])?,
                (AlbumSort::Starred, None) => albums(c, "AND starred=1 ORDER BY name_key LIMIT ?1 OFFSET ?2", params![size, offset])?,
                (AlbumSort::ByGenre, None) => Vec::new(),
                (sort, None) => c.prepare_cached(&crate::browse::albums_sql(sort, false))?.query_map(params![offset, size], |r| items::album(r, 0))?.collect::<rusqlite::Result<_>>()?,
            };
            Page::Albums { v }
        }
        Read::AlbumsByYear { from, to, size, offset } => {
            let order = if from <= to { "ASC" } else { "DESC" };
            let v = albums(c, &format!("AND year BETWEEN ?3 AND ?4 ORDER BY year {order}, name_key LIMIT ?1 OFFSET ?2"), params![size, offset, from.min(to), from.max(to)])?;
            Page::Albums { v }
        }
        Read::FavouriteAlbums { size } => Page::Albums { v: albums(c, "AND starred=1 ORDER BY name_key LIMIT ?1", [size])? },
        Read::ArtistIndex => Page::Artists { v: artists(c, "ORDER BY name_key", [])? },
        Read::AlbumById { id } => {
            let Some(album) = albums(c, "AND id=?1", [&id])?.pop() else { return Ok(None) };
            Page::AlbumPage { v: AlbumDetail::new(album, album_songs(c, &id)?, Vec::new()) }
        }
        Read::AlbumSongs { id } => Page::Songs { v: album_songs(c, &id)? },
        Read::ArtistById { id } => {
            let Some(artist) = artists(c, "AND id=?1", [&id])?.pop() else { return Ok(None) };
            let own = albums(c, "AND artist_id=?1 ORDER BY year, name_key", [&id])?;
            // Songs on others' albums: a compilation's, or as a guest.
            let elsewhere = songs(
                c,
                "AND (artist_id=?1 OR instr(artists, ?1) > 0) AND coalesce(album_id, '') NOT IN (SELECT id FROM items WHERE server=sid() AND kind=1 AND artist_id=?1)
                 ORDER BY album_key, disc_number, track",
                [&id],
            )?;
            Page::ArtistPage { v: ArtistDetail::new(artist, own, elsewhere) }
        }
        Read::StarredItems => {
            let starred = "AND starred=1 ORDER BY name_key";
            Page::StarredPage { v: Starred::new(artists(c, starred, [])?, albums(c, starred, [])?, songs(c, "AND starred=1 ORDER BY title_key", [])?) }
        }
        Read::GenreList => {
            let sql = "SELECT genre, count(*), count(DISTINCT album_id) FROM items WHERE server=sid() AND kind=2 AND genre <> '' GROUP BY genre COLLATE NOCASE ORDER BY genre COLLATE NOCASE";
            let v = c.prepare_cached(sql)?.query_map([], |r| Ok(Genre { name: r.get(0)?, song_count: r.get(1)?, album_count: r.get(2)? }))?.collect::<rusqlite::Result<_>>()?;
            Page::Genres { v }
        }
        Read::Search { query, songs: s, albums: a, artists: r } => {
            let mut v: SearchResult = db::search(c, &query, s.max(a).max(r).max(0) as u32)?;
            v.songs.truncate(s.max(0) as usize);
            v.albums.truncate(a.max(0) as usize);
            v.artists.truncate(r.max(0) as usize);
            Page::Found { v }
        }
        Read::SongPage { offset, count } => Page::Found { v: SearchResult { songs: songs(c, "ORDER BY title_key LIMIT ?1 OFFSET ?2", params![count, offset])?, ..Default::default() } },
        Read::RandomSongs { size, genre: Some(g) } => Page::Songs { v: songs(c, "AND genre = ?2 COLLATE NOCASE ORDER BY random() LIMIT ?1", params![size, g])? },
        Read::RandomSongs { size, genre: None } => Page::Songs { v: songs(c, "ORDER BY random() LIMIT ?1", [size])? },
        Read::SongsByGenre { genre, count, offset } => {
            Page::Songs { v: songs(c, "AND genre = ?1 COLLATE NOCASE ORDER BY artist_key, album_key, disc_number, track LIMIT ?2 OFFSET ?3", params![genre, count, offset])? }
        }
        Read::SongById { id } => Page::OneSong { v: songs(c, "AND id=?1", [&id])?.pop() },
        Read::PlaylistList => Page::Playlists { v: playlists(c)?.into_iter().map(|(list, _)| list).collect() },
        Read::PlaylistById { id } | Read::PlaylistSongs { id } => {
            let Some((list, songs)) = playlists(c)?.into_iter().find(|(l, _)| l.id == id) else { return Ok(None) };
            Page::PlaylistPage { v: PlaylistDetail::new(list, songs) }
        }
        Read::FolderIndex => {
            let roots = if folders.is_empty() { common_folder(c)?.into_iter().collect() } else { folders.to_vec() };
            Page::Artists { v: roots.iter().map(|r| folder(r)).collect() }
        }
        Read::FolderById { id } => {
            let Some(dir) = id.strip_prefix(FOLDER) else { return Ok(None) };
            Page::DirectoryPage { v: directory(c, dir)? }
        }
        _ => return Ok(None),
    };
    Ok(Some(page))
}

fn album_songs(c: &Connection, id: &str) -> rusqlite::Result<Vec<Song>> {
    songs(c, "AND album_id=?1 ORDER BY disc_number, track, title_key", [id])
}

/// `size` albums from `offset` of those heard on this device, by `order` of their plays, most first.
fn played(c: &Connection, order: &str, size: i32, offset: i32) -> rusqlite::Result<Vec<Album>> {
    let sql = format!("SELECT {ALBUM} FROM items i {PLAYED} WHERE i.server=sid() AND i.kind={} ORDER BY {order} DESC LIMIT ?1 OFFSET ?2", ItemKind::Album as i64);
    c.prepare_cached(&sql)?.query_map(params![size, offset], |r| items::album(r, 0))?.collect()
}

/// The playlists the sync found, with their songs as the index has them.
fn playlists(c: &Connection) -> Result<Vec<(Playlist, Vec<Song>)>> {
    let found: Vec<Found> = db::kv_get(c, PLAYLISTS)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    let mut out = Vec::with_capacity(found.len());
    for f in found {
        let listed: Vec<Song> = f.songs.iter().filter_map(|id| songs(c, "AND id=?1", [id]).ok()?.pop()).collect();
        let list = Playlist {
            id: nori_local::playlist::id(&f.path),
            name: f.name,
            song_count: listed.len() as u32,
            duration: listed.iter().map(|s| s.duration).sum(),
            cover_art: listed.iter().find_map(|s| s.cover_art.clone()),
            ..Default::default()
        };
        out.push((list, listed));
    }
    Ok(out)
}

/// The folder at `path`, as a folder list shows it.
fn folder(path: &str) -> Artist {
    let name = Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    Artist { id: format!("{FOLDER}{path}"), name, ..Default::default() }
}

/// The deepest folder every song is under; None without songs.
fn common_folder(c: &Connection) -> rusqlite::Result<Option<String>> {
    let (Some(first), Some(last)) = (
        c.query_row("SELECT min(path) FROM items WHERE server=sid() AND kind=2", [], |r| r.get::<_, Option<String>>(0))?,
        c.query_row("SELECT max(path) FROM items WHERE server=sid() AND kind=2", [], |r| r.get::<_, Option<String>>(0))?,
    ) else {
        return Ok(None);
    };
    let (a, b) = (Path::new(&first), Path::new(&last));
    let common: std::path::PathBuf = a.components().zip(b.components()).take_while(|(x, y)| x == y).map(|(x, _)| x).collect();
    // A file's own path is no folder: one song's folder is its parent.
    let common = if common == a { a.parent().map(Path::to_path_buf).unwrap_or(common) } else { common };
    Ok(Some(common.to_string_lossy().into_owned()))
}

/// The folder at `dir`: the folders in it that hold music, and its own songs.
fn directory(c: &Connection, dir: &str) -> rusqlite::Result<Directory> {
    let dir = dir.trim_end_matches('/');
    let (from, to) = (format!("{dir}/"), format!("{dir}0"));
    let paths: Vec<String> = c.prepare_cached("SELECT path FROM items WHERE server=sid() AND kind=2 AND path >= ?1 AND path < ?2")?.query_map([&from, &to], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let below: BTreeSet<&str> = paths.iter().filter_map(|p| p[from.len()..].split_once('/').map(|(sub, _)| sub)).collect();
    let folders = below.into_iter().map(|sub| folder(&format!("{from}{sub}"))).collect();
    let mut own = songs(c, "AND path >= ?1 AND path < ?2 AND instr(substr(path, ?3), '/') = 0 ORDER BY disc_number, track, path", params![from, to, from.len() as i64 + 1])?;
    own.sort_by(|a, b| (a.disc_number, a.track, &a.path).cmp(&(b.disc_number, b.track, &b.path)));
    Ok(Directory { id: format!("{FOLDER}{dir}"), name: folder(dir).name, folders, songs: own })
}

/// Puts the seal on item `id` of `kind`, or takes it off.
pub(super) fn star(core: &Core, kind: Starrable, id: &str, on: bool) -> Result<()> {
    let kind = match kind {
        Starrable::Song => ItemKind::Song,
        Starrable::Album => ItemKind::Album,
        Starrable::Artist => ItemKind::Artist,
    };
    core.db.lock().prepare_cached("UPDATE items SET starred=?3 WHERE server=sid() AND kind=?1 AND id=?2")?.execute(params![kind, id, on as i64])?;
    Ok(())
}

/// The path of song `id`'s file; None for a song not in the index.
pub(super) fn path(core: &Core, id: &str) -> Option<String> {
    core.db.lock().prepare_cached("SELECT path FROM items WHERE server=sid() AND kind=2 AND id=?1").ok()?.query_row([id], |r| r.get(0)).optional().ok().flatten().flatten()
}
