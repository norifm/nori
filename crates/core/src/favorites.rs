//! Favorites kept on this device, for a profile whose provider's account can't keep them
//! (`ProfileRules::stars_here`): a seal keeps the item's record in the profile's database
//! (`nori_db::favorites`), and the favorites list them after the source's. They are never indexed or
//! synced, and a seal asks nothing of the source when this session showed the item.

use crate::cache_policy::{records, Page, Read};
use crate::client::{Client, NetResult, Starrable};
use crate::db::favorites::{self, Favorite};
use crate::db::ItemKind;
use crate::stars::Ids;
use crate::transport::NetError;
use crate::{Album, Artist, Core, Result, Song, Starred};

fn item_kind(kind: Starrable) -> ItemKind {
    match kind {
        Starrable::Song => ItemKind::Song,
        Starrable::Album => ItemKind::Album,
        Starrable::Artist => ItemKind::Artist,
    }
}

/// An item's record, as a favorite kept here holds it.
pub(crate) enum Record {
    Song(Box<Song>),
    Album(Album),
    Artist(Artist),
}

impl Record {
    /// The record of item `id` of `kind` that `page` shows.
    pub(crate) fn in_page(page: &mut Page, kind: Starrable, id: &str) -> Option<Record> {
        let (songs, albums, artists) = records(page);
        match kind {
            Starrable::Song => songs.into_iter().find(|s| s.id == id).map(|s| Record::Song(Box::new(s.clone()))),
            Starrable::Album => albums.into_iter().find(|a| a.id == id).map(|a| Record::Album(a.clone())),
            Starrable::Artist => artists.into_iter().find(|a| a.id == id).map(|a| Record::Artist(a.clone())),
        }
    }

    fn json(&self) -> String {
        match self {
            Record::Song(s) => serde_json::to_string(s),
            Record::Album(a) => serde_json::to_string(a),
            Record::Artist(a) => serde_json::to_string(a),
        }
        .expect("records serialize")
    }
}

/// Where the source has the record of an item of `kind`.
fn lookup(kind: Starrable, id: String) -> Read {
    match kind {
        Starrable::Song => Read::SongById { id },
        Starrable::Album => Read::AlbumById { id },
        Starrable::Artist => Read::ArtistById { id },
    }
}

impl Core {
    /// Reads which items the profile keeps here, when its provider keeps any.
    pub(crate) fn load_favorites_here(&self) -> Result<()> {
        let kept = if self.provider().favorites_here() { self.favorites_here()? } else { Starred::default() };
        *self.here.lock() = Ids::of(&kept);
        Ok(())
    }

    /// The favorites kept here, the newest sealed first, each marked starred.
    fn favorites_here(&self) -> Result<Starred> {
        let rows = favorites::favorites(&self.db.lock())?;
        let mut kept = Starred::default();
        for f in rows {
            match f.kind {
                ItemKind::Song => kept.songs.extend(serde_json::from_str(&f.record).ok().map(|s| Song { starred: true, ..s })),
                ItemKind::Album => kept.albums.extend(serde_json::from_str(&f.record).ok().map(|a| Album { starred: true, ..a })),
                ItemKind::Artist => kept.artists.extend(serde_json::from_str(&f.record).ok().map(|a| Artist { starred: true, ..a })),
            }
        }
        Ok(kept)
    }

    /// The source's favourites with those kept here after them, when the provider keeps any; the items
    /// kept here are read anew (a restore may have added some).
    pub(crate) fn with_favorites_here(&self, source: Starred) -> Starred {
        if !self.provider().favorites_here() {
            return source;
        }
        match self.favorites_here() {
            Ok(kept) => {
                *self.here.lock() = Ids::of(&kept);
                crate::stars::with_here(source, kept)
            }
            Err(e) => {
                crate::alog::info(&format!("favorites kept here: {e}"));
                source
            }
        }
    }
}

impl Client {
    /// Seals (`on`) or unseals item `id` of `kind` on this device. A seal keeps the record this session
    /// showed of the item, else the one the source has for it (a read of its page, never a stream).
    pub(crate) async fn star_here(&self, kind: Starrable, id: &str, on: bool) -> NetResult<()> {
        if !on {
            favorites::drop(&self.core.db.lock(), item_kind(kind), id)?;
            self.core.here.lock().set(kind, id, false);
            return Ok(());
        }
        let Some(record) = self.record_of(kind, id).await? else { return Err(NetError::Http { status: 404 }) };
        let kept = Favorite { kind: item_kind(kind), id: id.to_string(), record: record.json(), sealed_ms: crate::db::now_ms() };
        favorites::keep(&self.core.db.lock(), &kept)?;
        self.core.here.lock().set(kind, id, true);
        Ok(())
    }

    /// The record of item `id` of `kind`: the queue's or a download's song, else one a page kept this
    /// session shows, else the source's page of it.
    async fn record_of(&self, kind: Starrable, id: &str) -> NetResult<Option<Record>> {
        if let Some(s) = (kind == Starrable::Song).then(|| self.stored_song_of(id)).flatten() {
            return Ok(Some(Record::Song(Box::new(s))));
        }
        if let Some(r) = self.kept_record(kind, id) {
            return Ok(Some(r));
        }
        Ok(self.cached_or_fetched(lookup(kind, id.to_string())).await?.and_then(|mut page| Record::in_page(&mut page, kind, id)))
    }
}
