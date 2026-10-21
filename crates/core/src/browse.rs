//! Browse-screen reads of the index and history. Layouts are nori-library's.

use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult};
use crate::{db, history, Core, Result};

pub use nori_library::browse::*;

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
    pub fn songs_page(&self, sort: String, starred_only: bool, year_from: u32, year_to: u32, offset: u32) -> Result<SongsPage> {
        let (key, descending) = SONG_SORTS.iter().find(|s| s.0 == sort).map_or(("", false), |s| (s.1, s.2));
        let songs = self.browse_songs(key.into(), descending, starred_only, year_from, year_to, offset, SONG_PAGE)?;
        Ok(SongsPage { exhausted: (songs.len() as u32) < SONG_PAGE, songs })
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
    pub async fn songs_listed(&self, sort: String, starred_only: bool, year_from: u32, year_to: u32, offset: u32) -> NetResult<SongsPage> {
        if self.core.rules().indexed {
            return Ok(self.core.songs_page(sort, starred_only, year_from, year_to, offset)?);
        }
        let songs = match self.read_now(Read::SongPage { offset: offset as i32, count: SONG_PAGE as i32 }).await? {
            Page::Found { v } => v.songs,
            _ => Vec::new(),
        };
        Ok(SongsPage { exhausted: (songs.len() as u32) < SONG_PAGE, songs })
    }
}

/// [`Core::browse_songs`]' query: offset ?1, limit ?2, and with `years` the range ?3..=?4.
pub(crate) fn songs_sql(sort: &str, descending: bool, starred_only: bool, years: bool) -> String {
    let key = match sort {
        "title" | "artist" | "album" => format!("json_extract(json, '$.{sort}') COLLATE NOCASE"),
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
    fn pages() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let all: Vec<Song> = (0..250).map(|i| song(&format!("s{i:03}"), &format!("T{:03}", 249 - i), "A", "B", "", 1990 + (i % 20) as u32)).collect();
        db::index(&mut core.db.lock(), &[], &[], &all).unwrap();
        let first = core.songs_page("TITLE".into(), false, 0, 0, 0).unwrap();
        assert_eq!((first.songs.len(), first.exhausted), (200, false));
        assert_eq!(first.songs[0].title, "T000");
        let last = core.songs_page("TITLE".into(), false, 0, 0, 200).unwrap();
        assert_eq!((last.songs.len(), last.exhausted), (50, true));
        let years = core.songs_page("YEAR".into(), false, 2000, 2009, 0).unwrap();
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

}
