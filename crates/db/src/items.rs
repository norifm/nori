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
pub(crate) trait Item {
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
pub(crate) fn upsert<T: Item>(c: &Connection, st: &Statements, item: &T) -> rusqlite::Result<Option<i64>> {
    let id = item.id();
    // Provider items are not library rows.
    if id.is_empty() || nori_model::is_provider_id(id) {
        return Ok(None);
    }
    let values = item.values();
    let n = values.len();
    let mut find = c.prepare_cached(&st.find)?;
    let old = find
        .query_row(rusqlite::params![T::KIND, id], |r| Ok((r.get::<_, i64>(0)?, (1..=n).map(|k| r.get::<_, Value>(k)).collect::<rusqlite::Result<Vec<Value>>>()?)))
        .optional()?;
    let fts = |rowid: i64, sql: &str| c.prepare_cached(sql)?.execute(rusqlite::params![rowid, item.text()]).map(|_| ());
    match old {
        Some((rowid, was)) if was == values => Ok(Some(rowid)),
        Some((rowid, _)) => {
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

/// Moves an index kept as JSON (one `json` column, before 0.8) into the columns, rowids kept so the
/// search index still points at its rows. Remove once no install predates 0.8.
pub(crate) fn migrate_from_json(c: &Connection) -> rusqlite::Result<()> {
    let has_json = c.prepare("PRAGMA table_info(items)")?.query_map([], |r| r.get::<_, String>(1))?.any(|n| n.is_ok_and(|n| n == "json"));
    if !has_json {
        return Ok(());
    }
    // `o.json`: json_each has a hidden column of that name.
    let j = |path: &str| format!("json_extract(o.json, '$.{path}')");
    // The JSON names of each column, as the records were serialized (camelCase).
    let camel = |c: &str| {
        let mut out = String::new();
        let mut up = false;
        for ch in c.chars() {
            if ch == '_' {
                up = true;
            } else if up {
                out.extend(ch.to_uppercase());
                up = false;
            } else {
                out.push(ch);
            }
        }
        out
    };
    let value = |col: &str| match col {
        // Written by nori's own serializer: booleans.
        "starred" | "is_external" | "is_compilation" => format!("coalesce({}, 0)", j(&camel(col))),
        "explicit_status" => format!("CASE {} WHEN 'clean' THEN 1 WHEN 'explicit' THEN 2 ELSE 0 END", j("explicitStatus")),
        "replay_gain" => {
            let f = |k: &str| format!("coalesce({}, 'NaN')", j(&format!("replayGain.{k}")));
            // Packed by the gain function registered for the move.
            format!("CASE WHEN {} IS NULL THEN NULL ELSE pack_gain({}, {}, {}, {}, {}, {}) END", j("replayGain"), f("trackGain"), f("albumGain"), f("trackPeak"), f("albumPeak"), f("baseGain"), f("fallbackGain"))
        }
        "artists" => "(SELECT group_concat(json_extract(value, '$.id') || char(31) || json_extract(value, '$.name'), char(30)) FROM json_each(o.json, '$.artists'))".to_string(),
        "release_types" => "(SELECT group_concat(value, char(30)) FROM json_each(o.json, '$.releaseTypes'))".to_string(),
        _ => j(&camel(col)),
    };
    c.create_scalar_function("pack_gain", 6, rusqlite::functions::FunctionFlags::SQLITE_UTF8, |ctx| {
        let mut out = Vec::with_capacity(24);
        for k in 0..6 {
            let v = match ctx.get::<Value>(k)? {
                Value::Real(f) => f as f32,
                Value::Integer(n) => n as f32,
                _ => f32::NAN,
            };
            out.extend(v.to_le_bytes());
        }
        Ok(out)
    })?;
    let copy = |kind: ItemKind, cols: &[&str]| format!(
        "INSERT INTO items(rowid, server, kind, id, {}) SELECT o.rowid, o.server, o.kind, o.id, {} FROM items_json o WHERE o.kind={};",
        cols.join(", "),
        cols.iter().map(|c| value(c)).collect::<Vec<_>>().join(", "),
        kind as i64
    );
    let tx = c.unchecked_transaction()?;
    tx.execute_batch("ALTER TABLE items RENAME TO items_json;")?;
    for index in ["items_genre", "items_artist", "items_year", "items_duration", "items_created", "items_plays", "items_starred", "items_rated", "items_title_natural", "items_artist_natural", "items_album_natural"] {
        tx.execute_batch(&format!("DROP INDEX IF EXISTS {index};"))?;
    }
    tx.execute_batch(TABLE)?;
    tx.execute_batch(&copy(ItemKind::Song, Song::COLUMNS))?;
    tx.execute_batch(&copy(ItemKind::Album, Album::COLUMNS))?;
    tx.execute_batch(&copy(ItemKind::Artist, Artist::COLUMNS))?;
    tx.execute_batch("DROP TABLE items_json;")?;
    tx.commit()
}
