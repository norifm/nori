//! Browse-screen reads of the index and history. Layouts are nori-library's.

use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult};
use crate::{db, history, Core, Result};
use rusqlite::params;

pub use nori_library::browse::*;

/// The server read page `offset` of a home album shelf is ([`home_shelves`], [`home_row_page`]): the
/// favourites' first page is the starred albums with this session's marks laid over them, every other page
/// the album list.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn shelf_albums_read(sort: AlbumSort, size: u32, follows_stars: bool, offset: u32) -> Read {
    if follows_stars && offset == 0 {
        Read::FavouriteAlbums { size: size as i32 }
    } else {
        Read::AlbumList { kind: sort, size: size as i32, offset: offset as i32, genre: None }
    }
}

/// Receives a profile's rules whenever they change: its source ended the account's session, or took it
/// up again.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait RulesShown: Send + Sync {
    fn rules(&self, rules: ProfileRules);
}

impl Core {
    /// Sets what the app offers over this profile; its songs are measured only where the rules say.
    pub(crate) fn set_rules(&self, rules: ProfileRules) {
        self.session.planner.set_measures(rules.measures);
        let changed = std::mem::replace(&mut *self.rules.write(), rules.clone()) != rules;
        let shown = self.rules_shown.read().clone();
        if let Some(shown) = shown.filter(|_| changed) {
            shown.rules(rules);
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// What the app offers over this profile, as configured: a jam guest's or the account's.
    pub fn rules(&self) -> ProfileRules {
        self.rules.read().clone()
    }

    /// `shown` is told the rules whenever they change from now on; None stops it.
    pub fn watch_rules(&self, shown: Option<std::sync::Arc<dyn RulesShown>>) {
        *self.rules_shown.write() = shown;
    }

    /// A page of the local song list at `offset`, sorted by the [song_sorts] entry `sort` (unknown: index
    /// order), optionally starred only and within `year_from..=year_to` (when `year_to` > 0).
    pub fn songs_page(&self, sort: String, reversed: bool, starred_only: bool, year_from: u32, year_to: u32, offset: u32) -> Result<SongsPage> {
        let (key, descending) = SONG_SORTS.iter().find(|s| s.0 == sort).map_or(("", false), |s| (s.1, s.2));
        let songs = self.browse_songs(key.into(), descending != reversed, starred_only, year_from, year_to, offset, SONG_PAGE)?;
        Ok(SongsPage { exhausted: (songs.len() as u32) < SONG_PAGE, songs })
    }

    /// A page of `limit` indexed albums at `offset` in `sort`'s order, `reversed` turning it around: the
    /// album grid turned against the server's way, which `getAlbumList2` cannot list.
    pub fn albums_indexed(&self, sort: AlbumSort, reversed: bool, offset: u32, limit: u32) -> Result<Vec<crate::Album>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached(&albums_sql(sort, reversed))?;
        let rows: Vec<String> = st.query_map(params![offset, limit], |r| r.get::<_, String>(0))?.filter_map(|r| r.ok()).collect();
        Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    /// A page of the listening history from `after` (None: the newest), newest first, without skips.
    pub fn history_page(&self, after: Option<HistoryAfter>) -> Result<HistoryPage> {
        let (entries, next) = self.history_recent(HISTORY_PAGE, after, false)?;
        Ok(HistoryPage { entries, next })
    }

    /// The listening stats page of `source`'s plays in the last `days` days (0: all). The server's plays
    /// are charted at `tz_offset_ms`; this device's at the offset each was played at.
    pub fn stats_page(&self, days: u32, source: StatsSource, tz_offset_ms: i32) -> Result<StatsPage> {
        let now = db::now_ms();
        let from = if days == 0 { 0 } else { now - days as i64 * DAY_MS };
        let c = self.db.lock();
        let stats = match source {
            StatsSource::Device => history::summary(&c, from, now, STATS_TOP)?,
            StatsSource::ServerAccount => history::server_summary(&c, from, now, tz_offset_ms, STATS_TOP)?,
        };
        Ok(StatsPage::new(source, stats))
    }

    /// Decades with indexed songs, newest first, with song counts.
    pub fn browse_decades(&self) -> Result<Vec<Decade>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached(&decades_sql())?;
        let rows = st.query_map([], |r| {
            let start: u32 = r.get(0)?;
            Ok(Decade { start, song_count: r.get(1)? })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Albums offered by the provider's Home feed, or a fresh random library page.
    pub async fn featured_albums(&self, limit: u32) -> NetResult<Vec<crate::Album>> {
        if !self.core.rules().feed {
            return Ok(self.read_now(Read::AlbumList { kind: AlbumSort::Random, size: limit.min(i32::MAX as u32) as i32, offset: 0, genre: None }).await?.albums());
        }
        let mut albums = Vec::new();
        if let Some(Page::Feed { v }) = self.cached_or_fetched(Read::HomeFeed).await? {
            for album in v.into_iter().flat_map(|s| s.albums) {
                if !albums.iter().any(|a: &crate::Album| a.id == album.id) { albums.push(album); }
            }
        }
        albums.truncate(limit as usize);
        Ok(albums)
    }

    /// The provider's featured playlists, without loading their songs or audio.
    pub async fn featured_playlists(&self) -> NetResult<Vec<crate::Playlist>> {
        let mut playlists = Vec::new();
        if !self.core.rules().feed { return Ok(playlists); }
        if let Some(Page::Feed { v }) = self.cached_or_fetched(Read::HomeFeed).await? {
            for playlist in v.into_iter().flat_map(|s| s.playlists) {
                if !playlists.iter().any(|p: &crate::Playlist| p.id == playlist.id) { playlists.push(playlist); }
            }
        }
        Ok(playlists)
    }

    /// A page of the songs list at `offset`: the offline index's, as [`Core::songs_page`] has it, or for a
    /// profile that keeps none (a jam guest's) the server's, in its own order whatever the sort.
    pub async fn songs_listed(&self, sort: String, reversed: bool, starred_only: bool, year_from: u32, year_to: u32, offset: u32) -> NetResult<SongsPage> {
        if self.core.rules().indexed {
            return Ok(self.core.songs_page(sort, reversed, starred_only, year_from, year_to, offset)?);
        }
        let songs = match self.read_now(Read::SongPage { offset: offset as i32, count: SONG_PAGE as i32 }).await? {
            Page::Found { v } => v.songs,
            _ => Vec::new(),
        };
        Ok(SongsPage { exhausted: (songs.len() as u32) < SONG_PAGE, songs })
    }
}

/// The favorite albums, read whole (`Read::StarredItems`), in `sort`'s order as the server orders the
/// album grid: A–Z by name, by artist (each artist's together, by year), newest year first, last added
/// first, most played first, or shuffled; `reversed` turns an order around.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn favorites_in_order(albums: Vec<crate::Album>, sort: AlbumSort, reversed: bool) -> Vec<crate::Album> {
    let mut albums = in_order(albums, sort);
    if reversed && sort != AlbumSort::Random {
        albums.reverse();
    }
    albums
}

fn in_order(mut albums: Vec<crate::Album>, sort: AlbumSort) -> Vec<crate::Album> {
    let name = |a: &crate::Album, b: &crate::Album| db::natural_cmp(&a.name, &b.name);
    match sort {
        AlbumSort::ByName => albums.sort_by(|a, b| name(a, b).then_with(|| db::natural_cmp(&a.artist, &b.artist))),
        AlbumSort::ByArtist | AlbumSort::ArtistRows => {
            albums.sort_by(|a, b| db::natural_cmp(&a.artist, &b.artist).then_with(|| a.artist_id.cmp(&b.artist_id)).then(a.year.cmp(&b.year)).then_with(|| name(a, b)))
        }
        AlbumSort::ByYear => albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| name(a, b))),
        AlbumSort::Newest => albums.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| name(a, b))),
        AlbumSort::Frequent => albums.sort_by(|a, b| b.play_count.cmp(&a.play_count).then_with(|| name(a, b))),
        AlbumSort::Random => return crate::autofill::shuffled(albums),
        // Not offered on the grid (`album_sorts`): as read.
        AlbumSort::Recent | AlbumSort::ByGenre | AlbumSort::Starred => {}
    }
    albums
}

/// [`Core::browse_songs`]' query: offset ?1, limit ?2, and with `years` the range ?3..=?4.
pub(crate) fn songs_sql(sort: &str, descending: bool, starred_only: bool, years: bool) -> String {
    let key = match sort {
        "title" | "artist" | "album" => format!("json_extract(json, '$.{sort}') COLLATE natural_order"),
        "year" | "duration" | "created" | "playCount" | "userRating" => format!("json_extract(json, '$.{sort}')"),
        _ => "rowid".to_string(),
    };
    // The song kind written out, so the songs' sort indexes (db.rs) serve the order a page at a time.
    // Starred songs are few: sorting them beats walking a sort index past all the others.
    let items = if starred_only { "items INDEXED BY items_starred" } else { "items" };
    let mut sql = format!("SELECT json FROM {items} WHERE server=sid() AND kind={}", db::ItemKind::Song as i64);
    if starred_only {
        sql.push_str(" AND json_extract(json, '$.starred') = 1");
    }
    if years {
        sql.push_str(" AND json_extract(json, '$.year') BETWEEN ?3 AND ?4");
    }
    sql.push_str(&format!(" ORDER BY {key} {} LIMIT ?2 OFFSET ?1", if descending { "DESC" } else { "ASC" }));
    sql
}

/// [`Core::albums_indexed`]' query: offset ?1, limit ?2. Each order as [`favorites_in_order`] has it, every
/// key's way turned when `reversed` except the year inside an artist's albums and the name breaking ties.
fn albums_sql(sort: AlbumSort, reversed: bool) -> String {
    let (up, down) = if reversed { ("DESC", "ASC") } else { ("ASC", "DESC") };
    let name = "json_extract(json, '$.name') COLLATE natural_order";
    let artist = "json_extract(json, '$.artist') COLLATE natural_order";
    let order = match sort {
        AlbumSort::ByName => format!("{name} {up}, {artist} {up}"),
        AlbumSort::ByArtist | AlbumSort::ArtistRows => format!("{artist} {up}, json_extract(json, '$.artistId') {up}, json_extract(json, '$.year'), {name}"),
        AlbumSort::ByYear => format!("json_extract(json, '$.year') {down}, {name}"),
        AlbumSort::Newest => format!("json_extract(json, '$.created') {down}, {name}"),
        AlbumSort::Frequent => format!("coalesce(json_extract(json, '$.playCount'), 0) {down}, {name}"),
        AlbumSort::Random | AlbumSort::Recent | AlbumSort::ByGenre | AlbumSort::Starred => "rowid".into(),
    };
    format!("SELECT json FROM items WHERE server=sid() AND kind={} ORDER BY {order} LIMIT ?2 OFFSET ?1", db::ItemKind::Album as i64)
}

/// The song kind written out, so the partial year index (db.rs `items_year`) serves the count.
fn decades_sql() -> String {
    format!("SELECT (json_extract(json, '$.year') / 10) * 10 AS d, count(*) FROM items WHERE server=sid() AND kind={} AND json_extract(json, '$.year') > 0 GROUP BY d ORDER BY d DESC", db::ItemKind::Song as i64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::history::tests::song;
    use crate::Song;

    #[test]
    fn shelf_reads() {
        assert!(matches!(shelf_albums_read(AlbumSort::Starred, 60, true, 0), Read::FavouriteAlbums { size: 60 }), "the favourites, marked");
        assert!(matches!(shelf_albums_read(AlbumSort::Starred, 60, true, 60), Read::AlbumList { kind: AlbumSort::Starred, size: 60, offset: 60, genre: None }), "then the starred list");
        assert!(matches!(shelf_albums_read(AlbumSort::Recent, 60, false, 0), Read::AlbumList { kind: AlbumSort::Recent, size: 60, offset: 0, genre: None }));
    }

    #[test]
    fn reversed_lists_turn_around() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let songs: Vec<Song> = ["T1", "T2", "T10"].iter().enumerate().map(|(i, t)| song(&format!("s{i}"), t, "A", "B", "", 1990)).collect();
        let album = |id: &str, name: &str, year: u32| crate::Album { id: id.into(), name: name.into(), artist: "A".into(), year, ..Default::default() };
        let albums = vec![album("a", "LP 2", 2001), album("b", "LP 10", 1999), album("c", "LP 1", 2010)];
        db::index(&mut core.db.lock(), &[], &albums, &songs).unwrap();
        let titles = |reversed| core.songs_page("TITLE".into(), reversed, false, 0, 0, 0).unwrap().songs.into_iter().map(|s| s.title).collect::<Vec<_>>();
        assert_eq!((titles(false), titles(true)), (vec!["T1".to_string(), "T2".into(), "T10".into()], vec!["T10".to_string(), "T2".into(), "T1".into()]));
        let ids = |sort, reversed| core.albums_indexed(sort, reversed, 0, 10).unwrap().into_iter().map(|a| a.id).collect::<String>();
        assert_eq!((ids(AlbumSort::ByName, false), ids(AlbumSort::ByName, true)), ("cab".into(), "bac".into()));
        assert_eq!((ids(AlbumSort::ByYear, false), ids(AlbumSort::ByYear, true)), ("cab".into(), "bac".into()));
        let favorites = |sort, reversed| favorites_in_order(albums.clone(), sort, reversed).into_iter().map(|a| a.id).collect::<String>();
        assert_eq!(favorites(AlbumSort::ByName, true), "bac");
    }

    #[test]
    fn pages() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let all: Vec<Song> = (0..250).map(|i| song(&format!("s{i:03}"), &format!("T{:03}", 249 - i), "A", "B", "", 1990 + (i % 20) as u32)).collect();
        db::index(&mut core.db.lock(), &[], &[], &all).unwrap();
        let first = core.songs_page("TITLE".into(), false, false, 0, 0, 0).unwrap();
        assert_eq!((first.songs.len(), first.exhausted), (200, false));
        assert_eq!(first.songs[0].title, "T000");
        let last = core.songs_page("TITLE".into(), false, false, 0, 0, 200).unwrap();
        assert_eq!((last.songs.len(), last.exhausted), (50, true));
        let years = core.songs_page("YEAR".into(), false, false, 2000, 2009, 0).unwrap();
        // Numbers in titles in their order, not the characters'.
        let numbered: Vec<Song> = ["#10", "#2", "#1"].iter().enumerate().map(|(i, t)| song(&format!("n{i}"), t, "A", "B", "", 1990)).collect();
        let core2 = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        db::index(&mut core2.db.lock(), &[], &[], &numbered).unwrap();
        let titles: Vec<String> = core2.songs_page("TITLE".into(), false, false, 0, 0, 0).unwrap().songs.into_iter().map(|s| s.title).collect();
        assert_eq!(titles, ["#1", "#2", "#10"]);
        assert_eq!(years.songs[0].year, 2009);
        assert!(years.exhausted && years.songs.iter().all(|s| (2000..=2009).contains(&s.year)));

        // Every sort reads its index, not the library; starred songs are read by the starred index.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let c = core.db.lock();
        for (_, key, descending) in SONG_SORTS {
            let plan = |starred| -> String {
                let sql = format!("EXPLAIN QUERY PLAN {}", songs_sql(key, descending, starred, false));
                c.prepare(&sql).unwrap().query_map([0, 200], |r| r.get::<_, String>(3)).unwrap().map(|r| r.unwrap()).collect::<Vec<_>>().join("; ")
            };
            let all = plan(false);
            assert!(all.contains("USING INDEX items_") && !all.contains("TEMP B-TREE"), "{key}: {all}");
            assert!(plan(true).contains("USING INDEX items_starred"), "{key} starred");
        }
        let decades: String = c.prepare(&format!("EXPLAIN QUERY PLAN {}", decades_sql())).unwrap().query_map([], |r| r.get::<_, String>(3)).unwrap().map(|r| r.unwrap()).collect::<Vec<_>>().join("; ");
        assert!(decades.contains("USING INDEX items_year"), "decades: {decades}");
    }

    #[test]
    fn history_pages() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = song("1", "t", "a", "b", "", 0);
        db::index(&mut core.db.lock(), &[], &[], std::slice::from_ref(&s)).unwrap();
        let now = db::now_ms();
        crate::history::record(&mut core.db.lock(), &s, now - 40 * DAY_MS, 200_000, 0, now).unwrap();
        crate::history::record(&mut core.db.lock(), &s, now - DAY_MS, 200_000, 0, now).unwrap();
        let page = core.history_page(None).unwrap();
        assert_eq!((page.entries.len(), page.next), (2, None));
        assert_eq!(core.stats_page(7, StatsSource::Device, 0).unwrap().stats.plays, 1);
        assert_eq!(core.stats_page(0, StatsSource::Device, 0).unwrap().stats.plays, 2);

        // A listen recorded while paging repeats nothing.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = song("1", "t", "a", "b", "", 0);
        db::index(&mut core.db.lock(), &[], &[], std::slice::from_ref(&s)).unwrap();
        let now = db::now_ms();
        for k in 0..HISTORY_PAGE as i64 + 1 {
            crate::history::record(&mut core.db.lock(), &s, now - (k + 2) * 300_000, 200_000, 0, now).unwrap();
        }
        let first = core.history_page(None).unwrap();
        crate::history::record(&mut core.db.lock(), &s, now - 1, 200_000, 0, now).unwrap();
        let second = core.history_page(first.next).unwrap();
        let oldest_first = first.entries.last().unwrap().started_ms;
        assert_eq!(second.entries.len(), 1);
        assert!(second.entries[0].started_ms < oldest_first);
        assert_eq!(second.next, None);
    }

    #[test]
    fn favorites_take_any_order() {
        let album = |id: &str, name: &str, artist: &str, year: u32, created: &str, plays: u32| crate::Album {
            id: id.into(), name: name.into(), artist: artist.into(), artist_id: Some(artist.to_lowercase()), year,
            created: (!created.is_empty()).then(|| created.into()), play_count: plays, starred: true, ..Default::default()
        };
        let read = vec![
            album("a", "beta", "Zed", 2001, "2024-03-01T00:00:00Z", 5),
            album("b", "Alpha", "amy", 1999, "", 9),
            album("c", "Gamma", "Zed", 2010, "2025-01-01T00:00:00Z", 3),
            album("d", "Delta", "Bo", 0, "2023-07-01T00:00:00Z", 1),
        ];
        let order = |sort| favorites_in_order(read.clone(), sort, false).into_iter().map(|a| a.id).collect::<String>();
        for (sort, want) in [
            (AlbumSort::ByName, "badc"),
            (AlbumSort::ArtistRows, "bdac"),
            (AlbumSort::ByYear, "cabd"),
            (AlbumSort::Newest, "cadb"),
            (AlbumSort::Frequent, "bacd"),
        ] {
            assert_eq!(order(sort), want, "{sort:?}");
        }
        let mut shuffled = order(AlbumSort::Random).into_bytes();
        shuffled.sort();
        assert_eq!(shuffled, b"abcd", "a shuffle keeps every favorite once");
    }
}
