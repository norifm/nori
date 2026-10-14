//! Play history and taste model reads over the core's database, and the server account's play history
//! fetched from Navidrome's own API. The model is nori-library's.

use std::collections::HashMap;

use rusqlite::params;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::browse::{HistoryAfter, ServerHistory};
use crate::client::{Client, NetResult};
use crate::transport::{Exchange, TransportResponse};
use crate::{model::*, Core, Result};

pub use nori_library::history::*;

/// Server plays asked for per request.
const SERVER_PAGE: usize = if cfg!(test) { 2 } else { 1000 };

#[derive(Deserialize)]
struct Login {
    token: String,
}

/// One entry of Navidrome's `/api/scrobble`; `submissionTime` is in seconds.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scrobble {
    id: i64,
    media_file_id: String,
    submission_time: i64,
}

/// A successful answer's JSON; None for an error status or anything else.
fn answer<T: DeserializeOwned>(r: &TransportResponse) -> Option<T> {
    (200..300).contains(&r.status).then(|| serde_json::from_slice(&r.body).ok()).flatten()
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Fetches the server account's plays submitted since the last fetch into the database, for
    /// [`Core::stats_page`]. Asks nothing with the setting off or without a password.
    pub async fn server_history_sync(&self) -> ServerHistory {
        match self.fetch_server_plays().await {
            Ok(true) => ServerHistory::Current,
            Ok(false) => ServerHistory::Unavailable,
            Err(e) if e.is_io() => ServerHistory::Unreached,
            Err(_) => ServerHistory::Unavailable,
        }
    }
}

impl Client {
    /// Logs in to Navidrome's own API and keeps the plays with ids above the newest kept, walking the
    /// history newest first; ids, not play times, so plays submitted late are not missed. Kept only once
    /// all arrived, so an interrupted walk leaves no gap. False when the server has no history to give.
    async fn fetch_server_plays(&self) -> NetResult<bool> {
        let login = self.core.native_login.read().clone().filter(|_| self.settings().prefs(|p| p.server_history));
        let Some((user, password)) = login else { return Ok(false) };
        let base = self.core.server.read().base.clone();
        let json = serde_json::json!({ "username": user, "password": password }).to_string();
        let r = self.transport.send(Exchange { url: format!("{base}/auth/login"), json: Some(json), ..Default::default() }).await?;
        let Some(Login { token }) = answer(&r) else { return Ok(false) };
        let headers = HashMap::from([("x-nd-authorization".to_string(), format!("Bearer {token}"))]);
        let mut newest = server_plays_newest(&self.core.db.lock())?;
        let mut replace = false;
        let mut fresh = Vec::new();
        for start in (0..).step_by(SERVER_PAGE) {
            let url = format!("{base}/api/scrobble?_sort=id&_order=DESC&_start={start}&_end={}", start + SERVER_PAGE);
            let r = self.transport.send(Exchange { url, headers: headers.clone(), ..Default::default() }).await?;
            let Some(page) = answer::<Vec<Scrobble>>(&r) else { return Ok(false) };
            // The server's newest play is older than one kept: its database was reset.
            if start == 0 && newest > page.first().map(|s| s.id) {
                (newest, replace) = (None, true);
            }
            let kept = |id: i64| newest.is_some_and(|n| id <= n);
            let last = page.len() < SERVER_PAGE || page.iter().any(|s| kept(s.id));
            fresh.extend(page.into_iter().filter(|s| !kept(s.id)).map(|s| ServerPlay { id: s.id, song_id: s.media_file_id, at_ms: s.submission_time * 1000 }));
            if last {
                break;
            }
        }
        server_plays_keep(&mut self.core.db.lock(), &fresh, replace)?;
        Ok(true)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Deletes all listens and song stats (not mix exclusions or smart playlists).
    pub fn history_clear(&self) -> Result<()> {
        self.db.lock().execute_batch("DELETE FROM plays WHERE server=sid(); DELETE FROM song_stats WHERE server=sid();")?;
        Ok(())
    }
}

impl Core {
    /// Up to `limit` listens older than `after` (None: the newest), newest first, omitting songs no longer
    /// indexed, and where the next page starts (None after the last).
    pub fn history_recent(&self, limit: u32, after: Option<HistoryAfter>, include_skipped: bool) -> Result<(Vec<HistoryEntry>, Option<HistoryAfter>)> {
        let after = after.unwrap_or(HistoryAfter { started_ms: i64::MAX, row: i64::MAX });
        let c = self.db.lock();
        let mut st = c.prepare_cached(
            "SELECT i.json, p.started_ms, p.heard_ms, p.completed, p.skipped, p.rowid FROM plays p JOIN items i ON i.server=sid() AND i.kind=2 AND i.id=p.song_id
             WHERE p.server=sid() AND p.skipped<=?1 AND (p.started_ms, p.rowid) < (?3, ?4) ORDER BY p.started_ms DESC, p.rowid DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![include_skipped, limit, after.started_ms, after.row], |r| {
            Ok((entry(r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?), HistoryAfter { started_ms: r.get(1)?, row: r.get(5)? }))
        })?;
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let next = rows.last().filter(|_| rows.len() == limit as usize).map(|(_, at)| *at);
        Ok((rows.into_iter().filter_map(|(e, _)| e).collect(), next))
    }

}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db;

    pub(crate) const NOW: i64 = 1_788_000_000_000; // 2026-08-29

    pub(crate) const DAY: i64 = 86_400_000;

    pub(crate) fn song(id: &str, title: &str, artist: &str, album: &str, genre: &str, year: u32) -> Song {
        Song {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            artist_id: (!artist.is_empty()).then(|| format!("ar-{}", artist.to_lowercase())),
            album_id: (!album.is_empty()).then(|| format!("al-{}", album.to_lowercase())),
            cover_art: Some(format!("cv-{id}")),
            genre: (!genre.is_empty()).then(|| genre.to_string()),
            year,
            duration: 200,
            suffix: "flac".into(),
            ..Default::default()
        }
        .dressed()
    }

    /// A full listen at `at`.
    pub(crate) fn listen(core: &Core, s: &Song, at: i64) {
        assert!(record(&mut core.db.lock(), s, at, s.duration as i64 * 1000, 0, NOW).unwrap());
    }

    pub(crate) fn skip(core: &Core, s: &Song, at: i64) {
        assert!(record(&mut core.db.lock(), s, at, 5_000, 0, NOW).unwrap());
    }

    /// Records a listen; false when nothing was recorded.
    fn rec(core: &Core, s: Song, started_ms: i64, heard_ms: i64) -> bool {
        record(&mut core.db.lock(), &s, started_ms, heard_ms, 0, NOW).unwrap()
    }

    /// (plays, skips, last played, heard ms, stored taste) of song `id`, if played.
    fn stat(core: &Core, id: &str) -> Option<(u32, u32, i64, i64, f64)> {
        let c = core.db.lock();
        let row = c.query_row("SELECT plays, skips, last_played_ms, heard_ms_total, taste FROM song_stats WHERE server=sid() AND song_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, f64>(4)?)));
        row.ok()
    }

    fn stats(core: &Core, from_ms: i64, to_ms: i64, top: u32) -> ListeningStats {
        summary(&core.db.lock(), from_ms, to_ms, top).unwrap()
    }

    #[test]
    fn records_listens() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = song("s1", "Dogs", "Pink Floyd", "Animals", "Rock", 1977);
        assert!(rec(&core, s.clone(), NOW - DAY, 200_000));
        assert!(rec(&core, s.clone(), NOW - DAY + 1, 5_000));
        assert!(!rec(&core, s.clone(), NOW, 1_500), "under 2 s");
        // Indexed by the listen alone.
        assert_eq!(core.index_size().unwrap().songs, 1);
        assert_eq!(core.local_search("dogs".into(), 5).unwrap().songs, vec![s.clone()]);

        let (plays, skips, last, heard, _) = stat(&core, "s1").unwrap();
        assert_eq!((plays, skips, last, heard), (1, 1, NOW - DAY, 205_000));
        assert!(stat(&core, "missing").is_none());

        let h = core.history_recent(10, None, true).unwrap().0;
        assert_eq!(h.len(), 2);
        assert!(h[0].skipped && !h[0].completed && h[1].completed);
        assert_eq!(h[1].song, s);
        assert_eq!(core.history_recent(10, None, false).unwrap().0.len(), 1);
        let (first, next) = core.history_recent(1, None, true).unwrap();
        assert!(first[0].skipped);
        assert!(!core.history_recent(1, next, true).unwrap().0[0].skipped);
        assert!(core.history_recent(0, None, true).unwrap().0.is_empty());

        core.history_clear().unwrap();
        assert!(core.history_recent(10, None, true).unwrap().0.is_empty());
        assert!(stat(&core, "s1").is_none());

        // Record keeps newer index entry.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let mut s = song("s1", "Dogs", "Pink Floyd", "Animals", "Rock", 1977);
        s.starred = true;
        db::index(&mut core.db.lock(), &[], &[], std::slice::from_ref(&s)).unwrap();
        let stale = Song { starred: false, ..s.clone() };
        rec(&core, stale, NOW, 200_000);
        assert!(core.history_recent(1, None, true).unwrap().0[0].song.starred);

        // Provider tracks are not recorded.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        for id in ["ext-deezer-song-7", "pl-deezer-9", ""] {
            assert!(!rec(&core, Song { id: id.into(), duration: 100, ..Default::default() }, NOW, 100_000));
        }
        assert!(!rec(&core, Song { id: "x".into(), is_external: true, ..Default::default() }, NOW, 100_000));
        assert_eq!(core.index_size().unwrap().songs, 0);
        assert!(core.history_recent(10, None, true).unwrap().0.is_empty());
    }

    #[test]
    fn taste_stays_sane() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let (fresh, old, skipped) = (song("a", "A", "X", "", "", 0), song("b", "B", "X", "", "", 0), song("c", "C", "X", "", "", 0));
        listen(&core, &fresh, NOW);
        listen(&core, &old, NOW - 30 * DAY);
        skip(&core, &skipped, NOW);
        let stored = |id: &str| stat(&core, id).unwrap().4;
        assert!((decayed(stored("a"), NOW) - 1.0).abs() < 1e-9);
        assert!((decayed(stored("b"), NOW) - 0.5).abs() < 1e-9);
        assert!((decayed(stored("c"), NOW) + 0.6).abs() < 1e-9);
        // Stored values already sort like today's.
        assert!(stored("a") > stored("b") && stored("b") > stored("c"));
        // Recording out of order gives the same result.
        listen(&core, &old, NOW - 60 * DAY);
        assert!((decayed(stored("b"), NOW) - 0.75).abs() < 1e-9);

        let plain = Song::default();
        assert_eq!(taste(&Song { starred: true, user_rating: 5, ..plain.clone() }, 0.0, NOW), 3.5);
        assert_eq!(taste(&Song { user_rating: 1, ..plain.clone() }, 0.0, NOW), -3.0);
        assert_eq!(taste(&Song { user_rating: 3, ..plain }, 0.0, NOW), 0.0);

        // Absurd timestamps keep taste finite.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = song("a", "A", "X", "", "", 0);
        assert!(record(&mut core.db.lock(), &s, i64::MAX / 2, 200_000, 0, NOW).unwrap());
        assert!(record(&mut core.db.lock(), &s, -5, 200_000, 0, NOW).unwrap());
        let t = taste(&s, stat(&core, "a").unwrap().4, NOW);
        assert!(t.is_finite() && t < 3.0, "{t}");
    }

    #[test]
    fn summaries() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = stats(&core, 0, i64::MAX, 10);
        assert_eq!((s.plays, s.listened_ms, s.longest_streak_days), (0, 0, 0));
        assert_eq!((s.plays_per_hour.len(), s.plays_per_weekday.len()), (24, 7));
        assert!(s.first_play.is_none() && s.top_songs.is_empty());

        // Summary counts tops and charts.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let dogs = song("s1", "Dogs", "Pink Floyd", "Animals", "Rock", 1977);
        let pigs = song("s2", "Pigs", "Pink Floyd", "Animals", "Rock", 1977);
        let bjork = song("s3", "Jóga", "Björk", "Homogenic", "Electronic", 1997);
        // 2026-06-01 00:00 UTC, a Monday.
        let monday = 1_780_272_000_000;
        let at = |day: i64, hour: i64| monday + day * DAY + hour * 3_600_000;
        listen(&core, &dogs, at(0, 8));
        listen(&core, &dogs, at(1, 8));
        listen(&core, &pigs, at(2, 23));
        skip(&core, &pigs, at(2, 23) + 1);
        listen(&core, &bjork, at(4, 8));
        // Outside the period.
        listen(&core, &bjork, at(40, 8));

        let s = stats(&core, monday, monday + 7 * DAY, 2);
        assert_eq!((s.plays, s.skips, s.listened_ms), (4, 1, 805_000));
        assert_eq!((s.distinct_songs, s.distinct_artists, s.distinct_albums), (3, 2, 2));
        assert_eq!(s.top_songs.len(), 2);
        assert_eq!((s.top_songs[0].song.id.as_str(), s.top_songs[0].plays, s.top_songs[0].listened_ms), ("s1", 2, 400_000));
        assert_eq!((s.top_artists[0].name.as_str(), s.top_artists[0].plays, s.top_artists[0].id.as_str()), ("Pink Floyd", 3, "ar-pink floyd"));
        assert_eq!(s.top_artists[0].cover_art.as_deref(), Some("cv-s1"));
        assert_eq!(s.top_artists[1].name, "Björk");
        assert_eq!((s.top_albums[0].name.as_str(), s.top_albums[0].listened_ms), ("Animals", 605_000));
        assert_eq!((s.top_genres[0].name.as_str(), s.top_genres[0].plays), ("Rock", 3));
        assert_eq!((s.plays_per_hour[8], s.plays_per_hour[23], s.plays_per_hour.iter().sum::<u32>()), (3, 1, 4));
        assert_eq!(s.plays_per_weekday, vec![1, 1, 1, 0, 1, 0, 0]);
        assert_eq!((s.active_days, s.longest_streak_days), (4, 3));
        assert_eq!(s.first_play.unwrap().song, dogs);

        // Charts use local time.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = song("s1", "A", "X", "", "", 0);
        // Sunday 23:30 UTC is Monday 01:30 at UTC+2.
        let sunday_late = 1_780_272_000_000 - 30 * 60_000;
        record(&mut core.db.lock(), &s, sunday_late, 200_000, 2 * 3_600_000, NOW).unwrap();
        let st = stats(&core, 0, i64::MAX, 1);
        assert_eq!((st.plays_per_hour[1], st.plays_per_weekday[0]), (1, 1));
    }

    mod server {
        use super::*;
        use crate::browse::StatsSource;
        use crate::client::tests::{block, client, Fake};
        use crate::client::NetProfile;
        use crate::transport::FailureKind;
        use crate::ServerConfig;

        const TOKEN: &str = r#"{"id":"u1","name":"U","username":"u","token":"tok"}"#;

        /// Navidrome's answer for plays `ids`, each at `id` hours after 2026-06-01 00:00 UTC.
        fn page(ids: &[i64]) -> String {
            let rows: Vec<String> = ids.iter().map(|id| format!(r#"{{"id":{id},"mediaFileId":"s{}","submissionTime":{}}}"#, id % 2, 1_780_272_000 + id * 3600)).collect();
            format!("[{}]", rows.join(","))
        }

        fn sync(c: &Client, fake: &Fake, answers: &[&str]) -> ServerHistory {
            for a in answers {
                fake.answer(a);
            }
            block(c.server_history_sync())
        }

        fn server_plays(c: &Client) -> u32 {
            c.core.stats_page(0, StatsSource::ServerAccount, 0).unwrap().stats.plays
        }

        #[test]
        fn fetches_only_new_plays() {
            let (c, fake) = client(NetProfile { url: "http://nd:4533".into(), ..Default::default() });
            assert_eq!(sync(&c, &fake, &[TOKEN, &page(&[3, 2]), &page(&[1])]), ServerHistory::Current);
            let sent = fake.sent.lock().clone();
            assert_eq!(sent[0].url, "http://nd:4533/auth/login");
            let login: serde_json::Value = serde_json::from_str(sent[0].json.as_deref().unwrap()).unwrap();
            assert_eq!(login, serde_json::json!({ "username": "u", "password": "p" }));
            assert_eq!(sent[1].url, "http://nd:4533/api/scrobble?_sort=id&_order=DESC&_start=0&_end=2");
            assert_eq!(sent[1].headers.get("x-nd-authorization").map(String::as_str), Some("Bearer tok"));
            assert_eq!((sent[1].json.as_deref(), sent[2].url.as_str()), (None, "http://nd:4533/api/scrobble?_sort=id&_order=DESC&_start=2&_end=4"));
            assert_eq!(server_plays(&c), 3);

            // The walk stops at the first page reaching a kept play; a late submission counts once.
            fake.sent.lock().clear();
            assert_eq!(sync(&c, &fake, &[TOKEN, &page(&[5, 4]), &page(&[3, 2])]), ServerHistory::Current);
            assert_eq!((fake.sent.lock().len(), server_plays(&c)), (3, 5));

            // An interrupted walk keeps nothing, and the next one fetches it all.
            fake.answer(TOKEN);
            fake.answer(&page(&[7, 6]));
            fake.fail(FailureKind::Timeout);
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unreached);
            assert_eq!(server_plays(&c), 5);
            assert_eq!(sync(&c, &fake, &[TOKEN, &page(&[7, 6]), &page(&[5, 4])]), ServerHistory::Current);
            assert_eq!(server_plays(&c), 7);

            // Another account's plays go with the library.
            c.core.configure(ServerConfig { url: "http://nd:4533".into(), user: "v".into(), password: "p".into(), ..Default::default() }).unwrap();
            assert_eq!(server_plays(&c), 0);
        }

        #[test]
        fn a_reset_server_history_replaces_the_kept_plays() {
            let (c, fake) = client(NetProfile { url: "http://nd".into(), ..Default::default() });
            assert_eq!(sync(&c, &fake, &[TOKEN, &page(&[5, 4]), &page(&[3, 2]), &page(&[1])]), ServerHistory::Current);
            assert_eq!(sync(&c, &fake, &[TOKEN, &page(&[2, 1]), "[]"]), ServerHistory::Current);
            assert_eq!(server_plays(&c), 2);
            assert_eq!(sync(&c, &fake, &[TOKEN, "[]"]), ServerHistory::Current);
            assert_eq!(server_plays(&c), 0);
        }

        #[test]
        fn a_server_without_history_is_unavailable() {
            let (c, fake) = client(NetProfile { url: "http://nd".into(), ..Default::default() });
            fake.answers.lock().push_back(Ok((401, br#"{"error":"Invalid username or password"}"#.to_vec())));
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unavailable);
            fake.answers.lock().push_back(Ok((200, b"<html>octo-fiesta</html>".to_vec())));
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unavailable);
            fake.answer(TOKEN);
            fake.answers.lock().push_back(Ok((404, vec![])));
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unavailable);
            fake.fail(FailureKind::UnknownHost);
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unreached);
            assert_eq!(fake.sent.lock().len(), 5);

            // Nothing is asked with an API key, or with the setting off.
            c.core.configure(ServerConfig { url: "http://nd".into(), user: "u".into(), api_key: Some("k".into()), ..Default::default() }).unwrap();
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unavailable);
            assert_eq!(fake.sent.lock().len(), 5);
            let (c, fake) = client(NetProfile { url: "http://nd".into(), ..Default::default() });
            c.settings().open("").unwrap();
            c.settings().edit_by_name("serverHistory", "false").unwrap();
            assert_eq!(block(c.server_history_sync()), ServerHistory::Unavailable);
            assert!(fake.sent.lock().is_empty() && fake.asked().is_empty());
        }

        #[test]
        fn device_and_server_plays_are_counted_apart() {
            let (c, fake) = client(NetProfile { url: "http://nd".into(), ..Default::default() });
            let (s0, s1) = (song("s0", "A", "X", "Al", "Rock", 0), song("s1", "B", "Y", "Bl", "Jazz", 0));
            db::index(&mut c.core.db.lock(), &[], &[], &[s0.clone(), s1]).unwrap();
            listen(&c.core, &s0, db::now_ms() - DAY);
            skip(&c.core, &s0, db::now_ms() - DAY + 1);
            // Plays 1 to 3: Monday 01:00 to 03:00 UTC; s1, s0, s1.
            assert_eq!(sync(&c, &fake, &[TOKEN, &page(&[3, 2]), &page(&[1])]), ServerHistory::Current);

            let device = c.core.stats_page(0, StatsSource::Device, 0).unwrap();
            assert_eq!((device.source, device.stats.plays, device.stats.skips, device.stats.listened_ms), (StatsSource::Device, 1, 1, 205_000));
            let server = c.core.stats_page(0, StatsSource::ServerAccount, 0).unwrap();
            let st = &server.stats;
            assert_eq!((server.source, st.plays, st.skips, st.listened_ms, st.distinct_songs), (StatsSource::ServerAccount, 3, 0, 0, 2));
            assert_eq!((st.top_songs[0].song.id.as_str(), st.top_songs[0].plays), ("s1", 2));
            assert_eq!((st.top_artists[0].name.as_str(), st.top_genres[0].name.as_str()), ("Y", "Jazz"));
            assert_eq!((st.plays_per_hour[1], st.plays_per_hour[3], st.plays_per_weekday[0]), (1, 1, 3));
            assert_eq!(st.first_play.as_ref().unwrap().started_ms, 1_780_275_600_000);

            // Charted at the offset asked: Monday 01:00 to 03:00 UTC is Sunday 22:00 to Monday 00:00 at UTC-3.
            let west = c.core.stats_page(0, StatsSource::ServerAccount, -3 * 3_600_000).unwrap().stats;
            assert_eq!((west.plays_per_hour[22], west.plays_per_hour[0], west.plays_per_hour[1]), (1, 1, 0));
            assert_eq!((west.plays_per_weekday[6], west.plays_per_weekday[0]), (2, 1));
            assert_eq!((west.active_days, west.longest_streak_days), (2, 2));
            assert_eq!(c.core.stats_page(7, StatsSource::ServerAccount, 0).unwrap().stats.plays, 0, "older than a week");
        }
    }

    #[test]
    fn listens_survive_cleared_index() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        listen(&core, &song("s1", "A", "X", "", "", 0), NOW);
        db::clear_library(&core.db.lock()).unwrap();
        assert!(core.history_recent(10, None, true).unwrap().0.is_empty());
        let s = stats(&core, 0, i64::MAX, 5);
        assert_eq!((s.plays, s.distinct_songs, s.top_songs.len()), (1, 1, 0));
        assert_eq!(stat(&core, "s1").unwrap().0, 1);
    }
}
