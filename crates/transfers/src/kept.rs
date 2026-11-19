//! Playlists kept downloaded: the songs each held when last seen, and which downloads keeping one
//! brought. A song leaving a kept playlist goes from the downloads only when keeping brought it and no
//! kept playlist holds it any more; one downloaded by itself stays.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::transfers::DownloadQueued;

/// What downloading a kept playlist, or bringing the kept ones up to date, did: the downloads queued,
/// and the songs that left them to go from the downloads.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct KeptChange {
    pub queued: DownloadQueued,
    pub gone: Vec<String>,
}

impl KeptChange {
    /// Takes in `other`, a further playlist's change.
    pub fn add(&mut self, other: KeptChange) {
        self.queued.fresh.extend(other.queued.fresh);
        self.queued.again.extend(other.queued.again);
        self.gone.extend(other.gone);
    }
}

/// Why the kept playlists are looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum KeptCheck {
    /// A session opened: looked at.
    Opened,
    /// The app came back in sight: looked at unless that was done in the last [`RECHECK_MS`].
    Shown,
}

/// How long after a look at the kept playlists coming back in sight looks again.
pub const RECHECK_MS: i64 = 30 * 60 * 1000;

/// Whether `check` looks at the kept playlists, last looked at `last_ms` (None: not yet), at `now_ms`.
pub fn check_due(check: KeptCheck, last_ms: Option<i64>, now_ms: i64) -> bool {
    check == KeptCheck::Opened || last_ms.is_none_or(|t| now_ms < t || now_ms - t >= RECHECK_MS)
}

/// Keeps playlist `id` downloaded, holding `songs`, of which keeping brought `brought` into the
/// downloads. The songs it held before and holds no more that are to go are returned.
pub fn keep(c: &mut Connection, id: &str, songs: &[String], brought: &[String]) -> rusqlite::Result<Vec<String>> {
    let tx = c.transaction()?;
    tx.execute("INSERT OR IGNORE INTO kept_playlists(server, id) VALUES(sid(), ?1)", [id])?;
    let gone = follow(&tx, id, songs, brought)?;
    tx.commit()?;
    Ok(gone)
}

/// Playlist `id` is no more: it is let go, and the songs keeping it brought that no other kept playlist
/// holds are returned, to go.
pub fn vanished(c: &mut Connection, id: &str) -> rusqlite::Result<Vec<String>> {
    let tx = c.transaction()?;
    let gone = follow(&tx, id, &[], &[])?;
    let_go_in(&tx, id)?;
    tx.commit()?;
    Ok(gone)
}

/// The playlists kept downloaded. One holding songs none of which is downloaded or queued any more (its
/// downloads were removed) is let go first.
pub fn kept(c: &mut Connection) -> rusqlite::Result<Vec<String>> {
    let tx = c.transaction()?;
    let removed: Vec<String> = tx
        .prepare_cached(
            "SELECT p.id FROM kept_playlists p WHERE p.server=sid()
               AND EXISTS (SELECT 1 FROM kept_songs k WHERE k.server=sid() AND k.playlist=p.id)
               AND NOT EXISTS (SELECT 1 FROM kept_songs k JOIN downloads d ON d.server=sid() AND d.id=k.song WHERE k.server=sid() AND k.playlist=p.id)",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &removed {
        let_go_in(&tx, id)?;
    }
    let ids = tx.prepare_cached("SELECT id FROM kept_playlists WHERE server=sid() ORDER BY id")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    tx.commit()?;
    Ok(ids)
}

/// Lets playlist `id` go; what keeping brought that no kept playlist holds now is a download of its own.
fn let_go_in(c: &Connection, id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM kept_songs WHERE server=sid() AND playlist=?1", [id])?;
    c.execute("DELETE FROM kept_brought WHERE server=sid() AND song NOT IN (SELECT song FROM kept_songs WHERE server=sid())", [])?;
    c.execute("DELETE FROM kept_playlists WHERE server=sid() AND id=?1", [id])?;
    Ok(())
}

/// Playlist `id` holds `songs` now; `brought` came into the downloads with it. Returns the songs that
/// left it, keeping brought, and no kept playlist holds.
fn follow(c: &Connection, id: &str, songs: &[String], brought: &[String]) -> rusqlite::Result<Vec<String>> {
    let now: HashSet<&str> = songs.iter().map(String::as_str).collect();
    let held: Vec<String> = c.prepare_cached("SELECT song FROM kept_songs WHERE server=sid() AND playlist=?1")?.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut leave = c.prepare_cached("DELETE FROM kept_songs WHERE server=sid() AND playlist=?1 AND song=?2")?;
    let left: Vec<String> = held.into_iter().filter(|s| !now.contains(s.as_str())).collect();
    for song in &left {
        leave.execute([id, song])?;
    }
    let mut hold = c.prepare_cached("INSERT OR IGNORE INTO kept_songs(server, playlist, song) VALUES(sid(), ?1, ?2)")?;
    for song in songs {
        hold.execute([id, song])?;
    }
    let mut bring = c.prepare_cached("INSERT OR IGNORE INTO kept_brought(server, song) VALUES(sid(), ?1)")?;
    for song in brought {
        bring.execute([song])?;
    }
    let mut held_elsewhere = c.prepare_cached("SELECT 1 FROM kept_songs WHERE server=sid() AND song=?1")?;
    let mut unbring = c.prepare_cached("DELETE FROM kept_brought WHERE server=sid() AND song=?1")?;
    let mut gone = Vec::new();
    for song in left {
        if !held_elsewhere.exists([&song])? && unbring.execute([&song])? > 0 {
            gone.push(song);
        }
    }
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn downloaded(c: &Connection, songs: &[&str]) {
        songs.iter().for_each(|s| _ = c.execute("INSERT INTO downloads(server, id, ts) VALUES(sid(), ?1, 0)", [s]).unwrap());
    }

    #[test]
    fn a_song_leaving_goes_only_if_keeping_brought_it_and_nothing_kept_holds_it() {
        let mut c = nori_db::open("", "s").unwrap();
        downloaded(&c, &["a", "b", "c", "own", "shared"]);
        // "own" was downloaded before the playlists were kept.
        assert_eq!(keep(&mut c, "p", &ids(&["a", "b", "own", "shared"]), &ids(&["a", "b", "shared"])).unwrap(), Vec::<String>::new());
        assert_eq!(keep(&mut c, "q", &ids(&["shared"]), &[]).unwrap(), Vec::<String>::new());
        assert_eq!(kept(&mut c).unwrap(), ["p", "q"]);
        assert_eq!(keep(&mut c, "p", &ids(&["b", "c"]), &ids(&["c"])).unwrap(), ["a"], "own stays, shared is q's too");
        assert_eq!(keep(&mut c, "q", &[], &[]).unwrap(), ["shared"], "brought by p, held by neither now");
        assert_eq!(vanished(&mut c, "p").unwrap(), ["b", "c"]);
        assert_eq!(kept(&mut c).unwrap(), ["q"], "empty, q stays kept");
    }

    #[test]
    fn coming_back_looks_again_at_most_every_half_hour() {
        let now = 10 * RECHECK_MS;
        for (check, last, due) in [
            (KeptCheck::Shown, None, true),
            (KeptCheck::Shown, Some(now - RECHECK_MS + 1), false),
            (KeptCheck::Shown, Some(now - RECHECK_MS), true),
            (KeptCheck::Shown, Some(now + 1), true),
            (KeptCheck::Opened, Some(now), true),
        ] {
            assert_eq!(check_due(check, last, now), due, "{check:?} last {last:?}");
        }
    }

    #[test]
    fn a_playlist_whose_downloads_were_removed_is_let_go() {
        let mut c = nori_db::open("", "s").unwrap();
        downloaded(&c, &["a", "b"]);
        keep(&mut c, "p", &ids(&["a", "b"]), &ids(&["a", "b"])).unwrap();
        c.execute("DELETE FROM downloads WHERE id='a'", []).unwrap();
        assert_eq!(kept(&mut c).unwrap(), ["p"], "one song is still here");
        c.execute("DELETE FROM downloads WHERE id='b'", []).unwrap();
        assert!(kept(&mut c).unwrap().is_empty());
        downloaded(&c, &["a"]);
        keep(&mut c, "q", &ids(&["a"]), &[]).unwrap();
        assert_eq!(keep(&mut c, "q", &[], &[]).unwrap(), Vec::<String>::new(), "a download of its own since p was let go");
    }
}
