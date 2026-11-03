//! The library index's rows: a column per field of the songs, albums and artists it keeps (each kind
//! fills its own and leaves the rest NULL, which costs a byte), read back into the records by column.
//! Nested fields are packed: replay gain as six little-endian f32 (NaN for none), lists as text joined
//! by unit and record separators.

use nori_model::{Album, Artist, ArtistRef, ExplicitStatus, ReplayGain, Song};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row};

use crate::ItemKind;

/// The `items` table and the indexes the library's queries use.
pub(crate) const TABLE: &str = "
CREATE TABLE IF NOT EXISTS items(rowid INTEGER PRIMARY KEY, server TEXT NOT NULL, kind INTEGER NOT NULL, id TEXT NOT NULL,
  title TEXT, album TEXT, artist TEXT, album_id TEXT, artist_id TEXT, cover_art TEXT, duration INTEGER, track INTEGER, disc_number INTEGER,
  year INTEGER, genre TEXT, suffix TEXT, content_type TEXT, bit_rate INTEGER, size INTEGER, sampling_rate INTEGER, bit_depth INTEGER,
  user_rating INTEGER, starred INTEGER, is_external INTEGER, replay_gain BLOB, artists TEXT, created TEXT, play_count INTEGER, played TEXT,
  path TEXT, explicit_status INTEGER, channel_count INTEGER, music_brainz_id TEXT, bpm INTEGER, comment TEXT, name TEXT, song_count INTEGER,
  release_types TEXT, is_compilation INTEGER, artist_image_url TEXT, album_count INTEGER,
  UNIQUE(server, kind, id));
CREATE INDEX IF NOT EXISTS items_genre ON items(server, genre COLLATE NOCASE) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_artist ON items(server, artist_id) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_year ON items(server, year) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_duration ON items(server, duration) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_created ON items(server, created) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_plays ON items(server, play_count) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_starred ON items(server, starred) WHERE kind=2 AND starred=1;
CREATE INDEX IF NOT EXISTS items_rated ON items(server, user_rating) WHERE kind=2 AND user_rating>=4;
CREATE INDEX IF NOT EXISTS items_title_natural ON items(server, title COLLATE natural_order) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_artist_natural ON items(server, artist COLLATE natural_order) WHERE kind=2;
CREATE INDEX IF NOT EXISTS items_album_natural ON items(server, album COLLATE natural_order) WHERE kind=2;
";

/// Downloaded songs: their columns as in `items`, and when each was queued and whether it finished.
pub(crate) const DOWNLOADS: &str = "
CREATE TABLE IF NOT EXISTS downloads(server TEXT NOT NULL, id TEXT NOT NULL, ts INTEGER NOT NULL, done INTEGER NOT NULL DEFAULT 0,
  title TEXT, album TEXT, artist TEXT, album_id TEXT, artist_id TEXT, cover_art TEXT, duration INTEGER, track INTEGER, disc_number INTEGER,
  year INTEGER, genre TEXT, suffix TEXT, content_type TEXT, bit_rate INTEGER, size INTEGER, sampling_rate INTEGER, bit_depth INTEGER,
  user_rating INTEGER, starred INTEGER, is_external INTEGER, replay_gain BLOB, artists TEXT, created TEXT, play_count INTEGER, played TEXT,
  path TEXT, explicit_status INTEGER, channel_count INTEGER, music_brainz_id TEXT, bpm INTEGER, comment TEXT,
  PRIMARY KEY(server, id)) WITHOUT ROWID;
";

/// The statement that adds a download: id ?1, queued at ?2, then the song's columns.
pub fn add_download_sql() -> String {
    let marks = (0..Song::COLUMNS.len()).map(|k| format!("?{}", k + 3)).collect::<Vec<_>>().join(", ");
    format!("INSERT INTO downloads(server, id, ts, {}) VALUES(sid(), ?1, ?2, {marks})", Song::COLUMNS.join(", "))
}

/// [`add_download_sql`]'s parameters for `song` queued at `ts`.
pub fn download_params(song: &Song, ts: i64) -> Vec<Value> {
    let mut p = vec![Value::Text(song.id.clone()), Value::Integer(ts)];
    p.extend(song.values());
    p
}

/// A song's columns, in [`song`]'s order; the query names the table `i`.
pub const SONG: &str = "i.id, i.title, i.album, i.artist, i.album_id, i.artist_id, i.cover_art, i.duration, i.track, i.disc_number, i.year, i.genre, i.suffix, i.content_type, i.bit_rate, i.size, i.sampling_rate, i.bit_depth, i.user_rating, i.starred, i.is_external, i.replay_gain, i.artists, i.created, i.play_count, i.played, i.path, i.explicit_status, i.channel_count, i.music_brainz_id, i.bpm, i.comment";
/// How many columns [`SONG`] is, for the ones a query selects after it.
pub const SONG_LEN: usize = 32;
/// An album's columns, in [`album`]'s order.
pub const ALBUM: &str = "i.id, i.name, i.artist, i.artist_id, i.cover_art, i.song_count, i.duration, i.year, i.genre, i.starred, i.is_external, i.release_types, i.is_compilation, i.user_rating, i.play_count, i.created, i.explicit_status, i.music_brainz_id";
/// An artist's columns, in [`artist`]'s order.
pub const ARTIST: &str = "i.id, i.name, i.cover_art, i.artist_image_url, i.album_count, i.starred, i.is_external";

const UNIT: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// The song whose [`SONG`] columns start at `at`.
pub fn song(r: &Row, at: usize) -> rusqlite::Result<Song> {
    let g = |k: usize| at + k;
    Ok(Song {
        id: r.get(g(0))?,
        title: text(r, g(1))?,
        album: text(r, g(2))?,
        artist: text(r, g(3))?,
        album_id: r.get(g(4))?,
        artist_id: r.get(g(5))?,
        cover_art: r.get(g(6))?,
        duration: number(r, g(7))?,
        track: number(r, g(8))?,
        disc_number: number(r, g(9))?,
        year: number(r, g(10))?,
        genre: r.get(g(11))?,
        suffix: text(r, g(12))?,
        content_type: text(r, g(13))?,
        bit_rate: number(r, g(14))?,
        size: r.get::<_, Option<i64>>(g(15))?.unwrap_or(0) as u64,
        sampling_rate: number(r, g(16))?,
        bit_depth: number(r, g(17))?,
        user_rating: number(r, g(18))? as u8,
        starred: flag(r, g(19))?,
        is_external: flag(r, g(20))?,
        replay_gain: r.get::<_, Option<Vec<u8>>>(g(21))?.map(|b| unpack_gain(&b)),
        artists: r.get::<_, Option<String>>(g(22))?.map(|t| unpack_artists(&t)).unwrap_or_default(),
        created: r.get(g(23))?,
        play_count: number(r, g(24))?,
        played: r.get(g(25))?,
        path: r.get(g(26))?,
        explicit_status: explicit(number(r, g(27))?),
        channel_count: number(r, g(28))?,
        music_brainz_id: r.get(g(29))?,
        bpm: number(r, g(30))?,
        comment: r.get(g(31))?,
        line: String::new(),
    }
    .dressed())
}

/// The album whose [`ALBUM`] columns start at `at`.
pub fn album(r: &Row, at: usize) -> rusqlite::Result<Album> {
    let g = |k: usize| at + k;
    Ok(Album {
        id: r.get(g(0))?,
        name: text(r, g(1))?,
        artist: text(r, g(2))?,
        artist_id: r.get(g(3))?,
        cover_art: r.get(g(4))?,
        song_count: number(r, g(5))?,
        duration: number(r, g(6))?,
        year: number(r, g(7))?,
        genre: r.get(g(8))?,
        starred: flag(r, g(9))?,
        is_external: flag(r, g(10))?,
        release_types: r.get::<_, Option<String>>(g(11))?.map(|t| t.split(RECORD).map(str::to_string).collect()).unwrap_or_default(),
        is_compilation: flag(r, g(12))?,
        user_rating: number(r, g(13))? as u8,
        play_count: number(r, g(14))?,
        created: r.get(g(15))?,
        explicit_status: explicit(number(r, g(16))?),
        music_brainz_id: r.get(g(17))?,
        subtitle: String::new(),
    }
    .dressed())
}

/// The artist whose [`ARTIST`] columns start at `at`.
pub fn artist(r: &Row, at: usize) -> rusqlite::Result<Artist> {
    let g = |k: usize| at + k;
    Ok(Artist {
        id: r.get(g(0))?,
        name: text(r, g(1))?,
        cover_art: r.get(g(2))?,
        artist_image_url: r.get(g(3))?,
        album_count: r.get::<_, Option<i64>>(g(4))?.map(|n| n as u32),
        starred: flag(r, g(5))?,
        is_external: flag(r, g(6))?,
    })
}

fn text(r: &Row, at: usize) -> rusqlite::Result<String> {
    Ok(r.get::<_, Option<String>>(at)?.unwrap_or_default())
}

fn number(r: &Row, at: usize) -> rusqlite::Result<u32> {
    Ok(r.get::<_, Option<i64>>(at)?.unwrap_or(0) as u32)
}

fn flag(r: &Row, at: usize) -> rusqlite::Result<bool> {
    Ok(r.get::<_, Option<i64>>(at)?.unwrap_or(0) != 0)
}

fn explicit(n: u32) -> ExplicitStatus {
    match n {
        1 => ExplicitStatus::Clean,
        2 => ExplicitStatus::Explicit,
        _ => ExplicitStatus::Unmarked,
    }
}

fn explicit_number(e: ExplicitStatus) -> i64 {
    match e {
        ExplicitStatus::Unmarked => 0,
        ExplicitStatus::Clean => 1,
        ExplicitStatus::Explicit => 2,
    }
}

fn pack_gain(g: &ReplayGain) -> Vec<u8> {
    [g.track_gain, g.album_gain, g.track_peak, g.album_peak, g.base_gain, g.fallback_gain].iter().flat_map(|v| v.unwrap_or(f32::NAN).to_le_bytes()).collect()
}

fn unpack_gain(b: &[u8]) -> ReplayGain {
    let v = |k: usize| b.get(k * 4..k * 4 + 4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).filter(|f| !f.is_nan());
    ReplayGain { track_gain: v(0), album_gain: v(1), track_peak: v(2), album_peak: v(3), base_gain: v(4), fallback_gain: v(5) }
}

fn pack_artists(a: &[ArtistRef]) -> Option<String> {
    (!a.is_empty()).then(|| a.iter().map(|a| format!("{}{UNIT}{}", a.id, a.name)).collect::<Vec<_>>().join(&RECORD.to_string()))
}

fn unpack_artists(t: &str) -> Vec<ArtistRef> {
    t.split(RECORD).map(|a| a.split_once(UNIT).map_or_else(|| ArtistRef { id: a.to_string(), name: String::new() }, |(id, name)| ArtistRef { id: id.into(), name: name.into() })).collect()
}

fn opt(s: &Option<String>) -> Value {
    s.clone().map_or(Value::Null, Value::Text)
}

fn int(n: impl Into<i64>) -> Value {
    Value::Integer(n.into())
}

/// A record the index keeps: its kind, id, searched text and columns.
pub trait Item {
    const KIND: ItemKind;
    /// The columns [`Item::values`] fills, in order.
    const COLUMNS: &'static [&'static str];
    fn id(&self) -> &str;
    fn text(&self) -> String;
    fn values(&self) -> Vec<Value>;
}

impl Item for Song {
    const KIND: ItemKind = ItemKind::Song;
    const COLUMNS: &'static [&'static str] = &[
        "title", "album", "artist", "album_id", "artist_id", "cover_art", "duration", "track", "disc_number", "year", "genre", "suffix", "content_type", "bit_rate", "size",
        "sampling_rate", "bit_depth", "user_rating", "starred", "is_external", "replay_gain", "artists", "created", "play_count", "played", "path", "explicit_status",
        "channel_count", "music_brainz_id", "bpm", "comment",
    ];
    fn id(&self) -> &str {
        &self.id
    }
    fn text(&self) -> String {
        format!("{} {} {}", self.title, self.artist, self.album)
    }
    fn values(&self) -> Vec<Value> {
        vec![
            Value::Text(self.title.clone()),
            Value::Text(self.album.clone()),
            Value::Text(self.artist.clone()),
            opt(&self.album_id),
            opt(&self.artist_id),
            opt(&self.cover_art),
            int(self.duration),
            int(self.track),
            int(self.disc_number),
            int(self.year),
            opt(&self.genre),
            Value::Text(self.suffix.clone()),
            Value::Text(self.content_type.clone()),
            int(self.bit_rate),
            int(self.size as i64),
            int(self.sampling_rate),
            int(self.bit_depth),
            int(self.user_rating),
            int(self.starred),
            int(self.is_external),
            self.replay_gain.as_ref().map_or(Value::Null, |g| Value::Blob(pack_gain(g))),
            pack_artists(&self.artists).map_or(Value::Null, Value::Text),
            opt(&self.created),
            int(self.play_count),
            opt(&self.played),
            opt(&self.path),
            int(explicit_number(self.explicit_status)),
            int(self.channel_count),
            opt(&self.music_brainz_id),
            int(self.bpm),
            opt(&self.comment),
        ]
    }
}

impl Item for Album {
    const KIND: ItemKind = ItemKind::Album;
    const COLUMNS: &'static [&'static str] = &[
        "name", "artist", "artist_id", "cover_art", "song_count", "duration", "year", "genre", "starred", "is_external", "release_types", "is_compilation", "user_rating",
        "play_count", "created", "explicit_status", "music_brainz_id",
    ];
    fn id(&self) -> &str {
        &self.id
    }
    fn text(&self) -> String {
        format!("{} {}", self.name, self.artist)
    }
    fn values(&self) -> Vec<Value> {
        vec![
            Value::Text(self.name.clone()),
            Value::Text(self.artist.clone()),
            opt(&self.artist_id),
            opt(&self.cover_art),
            int(self.song_count),
            int(self.duration),
            int(self.year),
            opt(&self.genre),
            int(self.starred),
            int(self.is_external),
            if self.release_types.is_empty() { Value::Null } else { Value::Text(self.release_types.join(&RECORD.to_string())) },
            int(self.is_compilation),
            int(self.user_rating),
            int(self.play_count),
            opt(&self.created),
            int(explicit_number(self.explicit_status)),
            opt(&self.music_brainz_id),
        ]
    }
}

impl Item for Artist {
    const KIND: ItemKind = ItemKind::Artist;
    const COLUMNS: &'static [&'static str] = &["name", "cover_art", "artist_image_url", "album_count", "starred", "is_external"];
    fn id(&self) -> &str {
        &self.id
    }
    fn text(&self) -> String {
        self.name.clone()
    }
    fn values(&self) -> Vec<Value> {
        vec![
            Value::Text(self.name.clone()),
            opt(&self.cover_art),
            opt(&self.artist_image_url),
            self.album_count.map_or(Value::Null, int),
            int(self.starred),
            int(self.is_external),
        ]
    }
}

/// One kind's statements, made once per [`crate::index`].
pub(crate) struct Statements {
    find: String,
    update: String,
    insert: String,
}

impl Statements {
    pub(crate) fn of<T: Item>() -> Statements {
        let cols = T::COLUMNS.join(", ");
        let marks = (0..T::COLUMNS.len()).map(|k| format!("?{}", k + 3)).collect::<Vec<_>>().join(", ");
        let sets = T::COLUMNS.iter().enumerate().map(|(k, c)| format!("{c}=?{}", k + 2)).collect::<Vec<_>>().join(", ");
        Statements {
            find: format!("SELECT rowid, {cols} FROM items WHERE server=sid() AND kind=?1 AND id=?2"),
            update: format!("UPDATE items SET {sets} WHERE rowid=?1"),
            insert: format!("INSERT INTO items(server, kind, id, {cols}) VALUES(sid(), ?1, ?2, {marks})"),
        }
    }
}

/// Stores `item`; its rowid, none for an item the index does not keep.
/// Whether the column read holds `value`.
fn holds(read: rusqlite::types::ValueRef, value: &Value) -> bool {
    use rusqlite::types::ValueRef as R;
    match (read, value) {
        (R::Null, Value::Null) => true,
        (R::Integer(a), Value::Integer(b)) => a == *b,
        (R::Real(a), Value::Real(b)) => a == *b,
        (R::Text(a), Value::Text(b)) => a == b.as_bytes(),
        (R::Blob(a), Value::Blob(b)) => a == b.as_slice(),
        _ => false,
    }
}

pub(crate) fn upsert<T: Item>(c: &Connection, st: &Statements, item: &T) -> rusqlite::Result<Option<i64>> {
    let id = item.id();
    // Provider items are not library rows.
    if id.is_empty() || nori_model::is_provider_id(id) {
        return Ok(None);
    }
    let values = item.values();
    let n = values.len();
    let mut find = c.prepare_cached(&st.find)?;
    // Each column compared where it is read: an unchanged row allocates nothing.
    let same = |r: &rusqlite::Row| -> rusqlite::Result<bool> { Ok((1..=n).all(|k| r.get_ref(k).is_ok_and(|v| holds(v, &values[k - 1])))) };
    let old = find.query_row(rusqlite::params![T::KIND, id], |r| Ok((r.get::<_, i64>(0)?, same(r)?))).optional()?;
    let fts = |rowid: i64, sql: &str| c.prepare_cached(sql)?.execute(rusqlite::params![rowid, item.text()]).map(|_| ());
    match old {
        Some((rowid, true)) => Ok(Some(rowid)),
        Some((rowid, false)) => {
            let mut params = vec![Value::Integer(rowid)];
            params.extend(values);
            c.prepare_cached(&st.update)?.execute(rusqlite::params_from_iter(params))?;
            fts(rowid, "INSERT OR REPLACE INTO fts(rowid, text) VALUES(?1, ?2)")?;
            Ok(Some(rowid))
        }
        None => {
            let mut params = vec![Value::Integer(T::KIND as i64), Value::Text(id.to_string())];
            params.extend(values);
            c.prepare_cached(&st.insert)?.execute(rusqlite::params_from_iter(params))?;
            let rowid = c.last_insert_rowid();
            fts(rowid, "INSERT INTO fts(rowid, text) VALUES(?1, ?2)")?;
            Ok(Some(rowid))
        }
    }
}
