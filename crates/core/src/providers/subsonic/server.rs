//! The server under a Subsonic profile: its signed URLs, address choice and fallback, music folder
//! scoping, writes and their offline queue, and the library walk.

use std::sync::atomic::Ordering;

use rusqlite::params;

use super::wire;
use crate::client::{blank, pairs, Client, NetResult, Starrable, Write};
use crate::transport::{self, FailureKind, NetError, Outcome, Transport};
use crate::{api, browse, settings, Core, IngestStats, Result, ServerConfig, ServerInfo};

/// Ping timeout for the first address before falling back to the second.
const ADDRESS_PROBE_MS: u32 = 2_500;

/// Tries the server puts off before a queued write is dropped, counted [`TRY_SPACING_MS`] apart.
const MOST_TRIES: i64 = 10;
const TRY_SPACING_MS: i64 = 3_600_000;
/// Age at which a queued write the server puts off is dropped: Last.fm refuses scrobbles older than this.
const OLDEST_MS: i64 = 14 * 86_400_000;

/// The endpoints that take `musicFolderId`.
const FOLDERED: [&str; 7] = ["getAlbumList2", "getArtists", "search3", "getRandomSongs", "getStarred2", "getSongsByGenre", "getIndexes"];

impl ServerConfig {
    /// The server these settings sign requests for.
    pub(super) fn server(&self) -> api::Server {
        let auth = match (&self.api_key, self.legacy_auth) {
            (Some(k), _) if !k.is_empty() => api::Auth::ApiKey(k),
            (_, true) => api::Auth::Legacy { user: &self.user, password: &self.password },
            _ => api::Auth::Token { user: &self.user, password: &self.password },
        };
        api::Server::with(&self.url, auth)
    }
}

/// The endpoint and parameters of a write.
pub(super) fn request(w: Write) -> (&'static str, Vec<(String, String)>) {
    let one = |k: &str, v: String| vec![(k.to_string(), v)];
    let many = |k: &str, ids: Vec<String>| ids.into_iter().map(|v| (k.to_string(), v)).collect::<Vec<_>>();
    match w {
        Write::Star { kind, id, on } => {
            let param = match kind {
                Starrable::Song => "id",
                Starrable::Album => "albumId",
                Starrable::Artist => "artistId",
            };
            (if on { "star" } else { "unstar" }, one(param, id))
        }
        Write::CreatePlaylist { name, song_ids } => {
            let mut p = one("name", name);
            p.extend(many("songId", song_ids));
            ("createPlaylist", p)
        }
        Write::AddToPlaylist { id, song_ids } => {
            let mut p = one("playlistId", id);
            p.extend(many("songIdToAdd", song_ids));
            ("updatePlaylist", p)
        }
        Write::RemoveFromPlaylist { id, index } => ("updatePlaylist", pairs(&[("playlistId", id), ("songIndexToRemove", index.to_string())])),
        Write::RenamePlaylist { id, name } => ("updatePlaylist", pairs(&[("playlistId", id), ("name", name)])),
        // `createPlaylist` with a playlist's id replaces its songs.
        Write::ReplacePlaylist { id, song_ids } => {
            let mut p = one("playlistId", id);
            p.extend(many("songId", song_ids));
            ("createPlaylist", p)
        }
        Write::DeletePlaylist { id } => ("deletePlaylist", one("id", id)),
        Write::CreateRadio { name, stream_url } => ("createInternetRadioStation", pairs(&[("name", name), ("streamUrl", stream_url)])),
        Write::DeleteRadio { id } => ("deleteInternetRadioStation", one("id", id)),
        Write::Scrobble { id, submission, time_ms } => {
            let mut p = pairs(&[("id", id), ("submission", submission.to_string())]);
            if let Some(t) = time_ms {
                p.push(("time".into(), t.to_string()));
            }
            ("scrobble", p)
        }
        Write::SaveQueue { ids, current, position_ms } => {
            let mut p = many("id", ids);
            if let Some(c) = current {
                p.push(("current".into(), c));
            }
            p.push(("position".into(), position_ms.to_string()));
            ("savePlayQueue", p)
        }
    }
}

async fn ping(transport: &dyn Transport, server: &api::Server, timeout_ms: u32) -> NetResult<()> {
    wire::parse(&transport::get(transport, server.url("ping", &[]), timeout_ms).await?)?;
    Ok(())
}

/// Verifies a profile before it is kept, opening nothing: pings its address, else `alt_url`. On error 41
/// (no token auth) tries legacy auth and returns true if that worked.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub async fn login_check(transport: std::sync::Arc<dyn Transport>, config: ServerConfig, alt_url: String) -> NetResult<bool> {
    let legacy_possible = !config.legacy_auth && config.api_key.as_deref().unwrap_or("").is_empty();
    match login_attempt(&*transport, &config, &alt_url).await {
        Ok(()) => Ok(false),
        Err(NetError::Api { code: 41, .. }) if legacy_possible => {
            login_attempt(&*transport, &ServerConfig { legacy_auth: true, ..config }, &alt_url).await?;
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

async fn login_attempt(transport: &dyn Transport, config: &ServerConfig, alt_url: &str) -> NetResult<()> {
    let server = config.server();
    let first = ping(transport, &server, 0).await;
    if first.is_err() && !blank(alt_url) {
        ping(transport, &server.rebased(alt_url), 0).await
    } else {
        first
    }
}

/// A write queued while the server was unreachable.
pub(crate) struct PendingCall {
    pub row_id: i64,
    pub endpoint: String,
    pub params: Vec<(String, String)>,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Sets the server; returns the normalised base url. Clears the library when the profile now points
    /// at another server or user.
    pub fn configure(&self, config: ServerConfig) -> Result<String> {
        let s = config.server();
        self.serves(&format!("{}|{}", s.base, config.user))?;
        let base = s.base.clone();
        *self.server.write() = s;
        self.install_kind(settings::ServerKind::Subsonic)?;
        self.starts_transcodes.store(false, Ordering::Relaxed);
        let key = config.api_key.unwrap_or_default();
        self.set_rules(browse::profile_rules(nori_remote::is_guest_key(&key)));
        *self.native_login.write() = key.is_empty().then(|| (config.user.clone(), config.password.clone()));
        *self.account.write() = match (key.is_empty(), nori_remote::is_guest_key(&key)) {
            (true, _) => Some((config.user, config.password)),
            (false, false) => Some((config.user, key)),
            (false, true) => None,
        };
        Ok(base)
    }
}

impl Core {
    /// Signed url of `endpoint` without parameters, for callers appending `&id=..` per row.
    pub(crate) fn url_prefix(&self, endpoint: &str) -> String {
        self.server.read().url(endpoint, &[])
    }

    /// `max_bit_rate` 0 and empty `format` mean the original file. A transcode comes without a length:
    /// with `estimateContentLength`, Navidrome cuts one that comes out larger than its estimate.
    pub(super) fn stream_url(&self, id: String, max_bit_rate: u32, format: String) -> String {
        let mut p = vec![("id".to_string(), id)];
        if max_bit_rate > 0 {
            p.push(("maxBitRate".into(), max_bit_rate.to_string()));
        }
        if !format.is_empty() {
            p.push(("format".into(), format));
        }
        self.server.read().url("stream", &p)
    }

    /// Points requests at another address of the same server (LAN vs WAN) without touching the index.
    fn use_address(&self, url: String) {
        let next = self.server.read().rebased(&url);
        *self.server.write() = next;
    }

    /// Validates any response; used for ping and for calls with no payload.
    pub(crate) fn parse_status(&self, body: Vec<u8>) -> Result<ServerInfo> {
        Ok(wire::parse(&body)?.into())
    }

    /// Library sync: indexes a search3 page; how many of each it lists, and the index rows it filled.
    pub(crate) fn ingest_search(&self, body: Vec<u8>) -> Result<(IngestStats, Vec<i64>)> {
        let f = wire::found(&body)?;
        let rows = crate::db::index(&mut self.db.lock(), &f.artists, &f.albums, &f.songs)?;
        Ok((IngestStats { artists: f.artists.len() as u32, albums: f.albums.len() as u32, songs: f.songs.len() as u32 }, rows))
    }

    pub(crate) fn pending_add(&self, endpoint: &str, params: &[(String, String)]) -> Result<()> {
        let json = serde_json::to_string(params).unwrap_or_default();
        self.db.lock().execute("INSERT INTO pending(server, endpoint, params, queued_ms) VALUES(sid(), ?1, ?2, ?3)", params![endpoint, json, crate::db::now_ms()])?;
        Ok(())
    }

    /// The server put off a queued write at `now`: counts a try (one an hour at most, so an outage behind
    /// a proxy that answers every request with 502 costs one) and drops the write once it is spent.
    pub(crate) fn pending_put_off(&self, row_id: i64, now: i64) -> Result<()> {
        let c = self.db.lock();
        c.execute("UPDATE pending SET tries=tries+1, tried_ms=?2 WHERE server=sid() AND rowid=?1 AND ?2-tried_ms>=?3", params![row_id, now, TRY_SPACING_MS])?;
        c.execute("DELETE FROM pending WHERE server=sid() AND rowid=?1 AND (tries>=?2 OR ?3-queued_ms>=?4)", params![row_id, MOST_TRIES, now, OLDEST_MS])?;
        Ok(())
    }

    pub(crate) fn pending_any(&self) -> Result<bool> {
        Ok(self.db.lock().query_row("SELECT EXISTS(SELECT 1 FROM pending WHERE server=sid())", [], |r| r.get(0))?)
    }

    pub(crate) fn pending_list(&self) -> Result<Vec<PendingCall>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("SELECT rowid, endpoint, params FROM pending WHERE server=sid() ORDER BY rowid LIMIT 200")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(row_id, endpoint, json)| {
                let pairs: Vec<(String, String)> = serde_json::from_str(&json).unwrap_or_default();
                PendingCall { row_id, endpoint, params: pairs }
            })
            .collect())
    }

    pub(crate) fn pending_done(&self, row_id: i64) -> Result<()> {
        self.db.lock().execute("DELETE FROM pending WHERE server=sid() AND rowid=?1", [row_id])?;
        Ok(())
    }
}

impl Client {
    /// `params` plus the music folders, for endpoints that take them.
    pub(crate) fn scoped(&self, endpoint: &str, mut params: Vec<(String, String)>) -> Vec<(String, String)> {
        if FOLDERED.contains(&endpoint) {
            params.extend(self.profile.read().music_folder_ids.iter().map(|id| ("musicFolderId".to_string(), id.clone())));
        }
        params
    }

    async fn get(&self, endpoint: &str, params: &[(String, String)]) -> NetResult<Vec<u8>> {
        if !self.core.provider().subsonic_api() {
            return Err(crate::providers::nowhere(endpoint));
        }
        let url = self.core.server.read().url(endpoint, params);
        transport::get(&*self.transport, url, 0).await
    }

    /// One request, retried once through the other address on an I/O failure (not a metered refusal:
    /// the other address is the same server under the same rule).
    pub(crate) async fn fetch(&self, endpoint: &str, params: Vec<(String, String)>) -> NetResult<Vec<u8>> {
        self.send(endpoint, params, true).await
    }

    /// [`Client::fetch`]; one not `repeatable` is retried only when nothing of it was sent.
    async fn send(&self, endpoint: &str, params: Vec<(String, String)>, repeatable: bool) -> NetResult<Vec<u8>> {
        let p = self.scoped(endpoint, params);
        match self.get(endpoint, &p).await {
            Err(e) if e.outcome() != Outcome::Refused && (repeatable || e.nothing_sent()) && !matches!(e, NetError::Transport { kind: FailureKind::Metered, .. }) => {
                if !self.choose_address().await {
                    return Err(e);
                }
                self.get(endpoint, &p).await
            }
            r => r,
        }
    }

    /// Sends one change to a Subsonic server, or queues it (reported as success) while the server is
    /// unreachable. Behind queued changes it is queued too, so the server gets them in order. A change
    /// that must not reach the server twice is queued only when the failure shows nothing was sent.
    pub(super) async fn subsonic_write(&self, w: Write) -> NetResult<()> {
        let repeatable = w.repeatable();
        let (endpoint, params) = request(w);
        if self.core.pending_any()? {
            {
                let _replaying = self.replaying.lock();
                self.core.pending_add(endpoint, &params)?;
            }
            self.flush_pending().await?;
        } else {
            match self.send(endpoint, params.clone(), repeatable).await.and_then(|body| Ok(self.core.parse_status(body)?)) {
                Ok(_) => {}
                Err(e) if e.nothing_sent() || (repeatable && e.outcome() != Outcome::Refused) => self.core.pending_add(endpoint, &params)?,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Pings the first address briefly and uses the second if it does not answer. Called on foreground and
    /// after failures. Returns whether the address changed.
    pub async fn choose_address(&self) -> bool {
        let (url, alt) = {
            let p = self.profile.read();
            (p.url.clone(), p.alt_url.clone())
        };
        if blank(&alt) {
            return false;
        }
        let probe = self.core.server.read().rebased(&url);
        let first_answers = ping(&*self.transport, &probe, ADDRESS_PROBE_MS).await.is_ok();
        self.core.use_address(if first_answers { url } else { alt });
        let changed = self.second.swap(!first_answers, Ordering::Relaxed) == first_answers;
        if changed {
            self.transport.address_changed();
        }
        changed
    }

    /// Replays queued writes in order until none are left, stopping at the first that is not
    /// [`Outcome::Refused`]; refused writes are dropped, and so are writes the server keeps putting off
    /// (see `Core::pending_put_off`). While one replay runs, another returns at once: the running one sends
    /// what was queued meanwhile too.
    pub async fn flush_pending(&self) -> NetResult<()> {
        if std::mem::replace(&mut *self.replaying.lock(), true) {
            return Ok(());
        }
        /// Ends the replay however it stops, unless it found nothing left (ended then, under the lock).
        struct Done<'a>(&'a parking_lot::Mutex<bool>, bool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                if self.1 {
                    *self.0.lock() = false;
                }
            }
        }
        let mut done = Done(&self.replaying, true);
        loop {
            let queued = {
                let mut replaying = self.replaying.lock();
                let queued = self.core.pending_list()?;
                if queued.is_empty() {
                    (*replaying, done.1) = (false, false);
                    return Ok(());
                }
                queued
            };
            for p in queued {
                if let Err(e) = self.get(&p.endpoint, &p.params).await.and_then(|body| Ok(self.core.parse_status(body)?)) {
                    match e.outcome() {
                        Outcome::Offline => return Ok(()),
                        Outcome::Later => return Ok(self.core.pending_put_off(p.row_id, crate::db::now_ms())?),
                        Outcome::Refused => {}
                    }
                }
                self.core.pending_done(p.row_id)?;
            }
        }
    }

    /// Walks the whole library into the offline index, a search3 page at a time, then drops what the
    /// server no longer lists; how many of each it listed.
    pub async fn sync_library(&self) -> NetResult<IngestStats> {
        let page = crate::browse::library_sizes().sync_page;
        let n = page.to_string();
        let mut total = IngestStats::default();
        let mut kept = std::collections::HashSet::new();
        let mut offset = 0;
        loop {
            let o = offset.to_string();
            let p = pairs(&[
                ("query", String::new()),
                ("songCount", n.clone()),
                ("songOffset", o.clone()),
                ("albumCount", n.clone()),
                ("albumOffset", o.clone()),
                ("artistCount", n.clone()),
                ("artistOffset", o),
            ]);
            let (seen, rows) = self.core.ingest_search(self.fetch("search3", p).await?)?;
            if seen.songs == 0 && seen.albums == 0 && seen.artists == 0 {
                self.core.prune_index(&kept)?;
                return Ok(total);
            }
            total = IngestStats { artists: total.artists + seen.artists, albums: total.albums + seen.albums, songs: total.songs + seen.songs };
            kept.extend(rows);
            offset += page;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::client::tests::{block, client, two_addresses, Fake, OK};
    use crate::client::NetProfile;

    #[test]
    fn music_folder_where_foldered() {
        let (c, _) = client(NetProfile { url: "h".into(), music_folder_ids: vec!["3".into(), "5".into()], ..Default::default() });
        let folder = |id: &str| ("musicFolderId".to_string(), id.to_string());
        assert_eq!(c.scoped("search3", vec![]), vec![folder("3"), folder("5")]);
        assert!(c.scoped("getAlbum", vec![]).is_empty());
        let (c, _) = client(NetProfile { url: "h".into(), ..Default::default() });
        assert!(c.scoped("search3", vec![]).is_empty());
    }

    #[test]
    fn retries() {
        {
            let (c, fake) = client(two_addresses());
            fake.fail(FailureKind::Connect); // the request
            fake.fail(FailureKind::Timeout); // the ping to the first address
            fake.answer(OK); // the request again, through the second
            assert!(block(c.fetch("getGenres", vec![])).is_ok());
            let asked = fake.asked();
            assert!(asked[0].starts_with("http://lan:4533/rest/getGenres"));
            assert!(asked[1].starts_with("http://lan:4533/rest/ping"));
            assert_eq!(fake.asked.lock()[1].1, 2_500);
            assert!(asked[2].starts_with("https://wan.example/rest/getGenres"));
            assert!(c.on_second_address());
            assert_eq!(*fake.switched.lock(), 1);

            fake.answer(OK);
            assert!(block(c.choose_address()));
            assert!(!c.on_second_address());
            assert_eq!(*fake.switched.lock(), 2);
            fake.answer(OK);
            assert!(!block(c.choose_address()));
        }

        // Metered and other failures are not retried.
        {
            let (c, fake) = client(two_addresses());
            fake.fail(FailureKind::Metered);
            assert!(matches!(block(c.fetch("ping", vec![])), Err(NetError::Transport { kind: FailureKind::Metered, .. })));
            fake.fail(FailureKind::Other);
            assert!(block(c.fetch("ping", vec![])).is_err());
            assert_eq!(fake.asked().len(), 2);

            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            fake.fail(FailureKind::Connect);
            assert!(block(c.fetch("ping", vec![])).is_err());
            assert_eq!(fake.asked().len(), 1);
        }
    }

    #[test]
    fn http_error_status_vs_error_body() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answers.lock().push_back(Ok((502, vec![])));
        assert!(matches!(block(c.fetch("ping", vec![])), Err(NetError::Http { status: 502 })));
        fake.answers.lock().push_back(Ok((401, br#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"no"}}}"#.to_vec())));
        let body = block(c.fetch("ping", vec![])).unwrap();
        assert!(matches!(c.core.parse_status(body), Err(crate::CoreError::Api { code: 40, .. })));
        // A proxy's HTML error page reports its status, not a parse error.
        fake.answers.lock().push_back(Ok((522, b"<html><body>error code: 522</body></html>".to_vec())));
        assert!(matches!(block(c.fetch("ping", vec![])), Err(NetError::Http { status: 522 })));
    }

    #[test]
    fn offline_writes() {
        {
            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            c.core.cache_put("page:playlist/1".into(), b"x".to_vec()).unwrap();
            fake.fail(FailureKind::UnknownHost);
            block(c.write(Write::AddToPlaylist { id: "1".into(), song_ids: vec!["a".into(), "b".into()] })).unwrap();
            fake.fail(FailureKind::UnknownHost);
            block(c.write(Write::Scrobble { id: "s".into(), submission: true, time_ms: Some(5) })).unwrap();
            assert_eq!(c.core.pending_list().unwrap().len(), 2);
            assert!(c.core.cache_get("page:playlist/1".into()).unwrap().is_some() && !c.core.cache_fresh("page:playlist/1".into(), 60_000).unwrap(), "stale even when queued, and still shown");

            // Online: the queue in order, then the new write; the rejected one is dropped.
            fake.answer(r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"gone"}}}"#);
            fake.answer(OK);
            fake.answer(OK);
            block(c.write(Write::Star { kind: Starrable::Album, id: "al".into(), on: true })).unwrap();
            let asked = fake.asked();
            assert!(asked[2].contains("/rest/updatePlaylist?") && asked[2].ends_with("&playlistId=1&songIdToAdd=a&songIdToAdd=b"));
            assert!(asked[3].contains("/rest/scrobble?") && asked[3].ends_with("&id=s&submission=true&time=5"));
            assert!(asked[4].contains("/rest/star?") && asked[4].ends_with("&albumId=al"));
            assert!(c.core.pending_list().unwrap().is_empty());
        }

        // Replay stops at network failure.
        {
            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            c.core.pending_add("star", &[]).unwrap();
            c.core.pending_add("unstar", &[]).unwrap();
            fake.fail(FailureKind::Timeout);
            block(c.flush_pending()).unwrap();
            assert_eq!(c.core.pending_list().unwrap().len(), 2);
            assert_eq!(fake.asked().len(), 1);
        }

        // Replays at once send each write once.
        {
            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            c.core.pending_add("scrobble", &[]).unwrap();
            *fake.pends.lock() = true;
            fake.answer(OK);
            fake.answer(OK);
            let (a, b) = block(futures_util::future::join(c.flush_pending(), c.flush_pending()));
            assert!(a.is_ok() && b.is_ok());
            assert_eq!(fake.asked().len(), 1);
            assert!(c.core.pending_list().unwrap().is_empty());
        }
    }

    #[test]
    fn edits_not_queued() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.fail(FailureKind::Timeout);
        assert!(block(c.write(Write::AddToPlaylist { id: "1".into(), song_ids: vec!["a".into()] })).is_err());
        fake.fail(FailureKind::Timeout);
        block(c.write(Write::Star { kind: Starrable::Song, id: "s".into(), on: true })).unwrap();
        let queued: Vec<String> = c.core.pending_list().unwrap().into_iter().map(|p| p.endpoint).collect();
        assert_eq!(queued, ["star"]);

        // Unsure edit not sent again elsewhere.
        let (c, fake) = client(two_addresses());
        fake.fail(FailureKind::Timeout);
        fake.fail(FailureKind::Timeout);
        fake.answer(OK);
        assert!(block(c.write(Write::AddToPlaylist { id: "1".into(), song_ids: vec!["a".into()] })).is_err());
        assert_eq!(fake.asked().len(), 1, "{:?}", fake.asked());

        // Rejected write errors and is not queued.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.cache_put("page:starred".into(), b"x".to_vec()).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"no"}}}"#);
        assert!(matches!(block(c.write(Write::Star { kind: Starrable::Song, id: "1".into(), on: false })), Err(NetError::Api { code: 70, .. })));
        assert!(c.core.pending_list().unwrap().is_empty());
        assert!(c.core.cache_get("page:starred".into()).unwrap().is_some());
    }

    /// A request a proxy refuses as it stands (too long, not there) is refused again however often it is
    /// sent: it is dropped from the queue rather than holding every later scrobble and star behind it.
    /// Refusals that pass (signed out at the proxy, too many requests) and server errors keep it.
    #[test]
    fn a_refused_write_does_not_hold_the_queue() {
        for (status, kept) in [(414, false), (404, false), (413, false), (401, true), (403, true), (429, true), (502, true)] {
            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            c.core.pending_add("savePlayQueue", &[]).unwrap();
            c.core.pending_add("scrobble", &[]).unwrap();
            fake.answers.lock().push_back(Ok((status, b"<html>refused</html>".to_vec())));
            fake.answer(OK);
            block(c.flush_pending()).unwrap();
            let left: Vec<String> = c.core.pending_list().unwrap().into_iter().map(|p| p.endpoint).collect();
            assert_eq!(left, if kept { vec!["savePlayQueue", "scrobble"] } else { vec![] }, "HTTP {status}");
        }
        // Sent now, a refused write says so and is not queued.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answers.lock().push_back(Ok((414, b"<html>too long</html>".to_vec())));
        assert!(matches!(block(c.write(Write::Star { kind: Starrable::Song, id: "s".into(), on: true })), Err(NetError::Http { status: 414 })));
        assert!(c.core.pending_list().unwrap().is_empty());
    }

    /// A write the server keeps putting off goes after ten tries an hour apart, or once two weeks old, so
    /// the ones behind it are sent. Being offline never counts.
    #[test]
    fn a_write_put_off_for_good_is_dropped() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.pending_add("savePlayQueue", &[]).unwrap();
        c.core.pending_add("scrobble", &[]).unwrap();
        let head = || c.core.pending_list().unwrap().first().map(|p| p.endpoint.clone());
        for _ in 0..20 {
            fake.answers.lock().push_back(Ok((502, vec![])));
            block(c.flush_pending()).unwrap();
            fake.fail(FailureKind::Connect);
            block(c.flush_pending()).unwrap();
        }
        assert_eq!(head().as_deref(), Some("savePlayQueue"), "an outage within the hour is one try");
        let row = c.core.pending_list().unwrap()[0].row_id;
        let now = crate::db::now_ms();
        for hour in 1..9 {
            c.core.pending_put_off(row, now + hour * TRY_SPACING_MS).unwrap();
        }
        assert_eq!(head().as_deref(), Some("savePlayQueue"), "nine tries");
        c.core.pending_put_off(row, now + 9 * TRY_SPACING_MS).unwrap();
        assert_eq!(head().as_deref(), Some("scrobble"), "the tenth drops it");

        let row = c.core.pending_list().unwrap()[0].row_id;
        c.core.pending_put_off(row, now + OLDEST_MS - 1_000).unwrap();
        assert_eq!(head().as_deref(), Some("scrobble"));
        c.core.pending_put_off(row, now + OLDEST_MS).unwrap();
        assert_eq!(head(), None, "two weeks old");
    }

    #[test]
    fn a_portal_page_keeps_the_queue() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.pending_add("star", &[]).unwrap();
        fake.answer("<html>sign in</html>");
        block(c.flush_pending()).unwrap();
        assert_eq!(c.core.pending_list().unwrap().len(), 1);

        // Sent now, a write the portal answers is queued, not lost.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer("<html>sign in</html>");
        block(c.write(Write::Star { kind: Starrable::Song, id: "s".into(), on: true })).unwrap();
        assert_eq!(c.core.pending_list().unwrap().len(), 1);
    }

    #[test]
    fn a_star_leaves_other_lists_cached() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let keys = ["page:album/1", "page:albums/newest/20/0", "page:about/1", "page:artists", "page:playlists", "page:playlist/2"];
        for k in keys {
            c.core.cache_put(k.into(), b"x".to_vec()).unwrap();
        }
        fake.fail(FailureKind::Connect);
        block(c.write(Write::Star { kind: Starrable::Song, id: "s".into(), on: true })).unwrap();
        let kept: Vec<&str> = keys.into_iter().filter(|k| c.core.cache_fresh(k.to_string(), 60_000).unwrap()).collect();
        assert_eq!(kept, ["page:albums/newest/20/0", "page:about/1", "page:artists", "page:playlists"]);
    }

    #[test]
    fn star_playlist_now_playing() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.cache_put("page:starred".into(), b"x".to_vec()).unwrap();
        c.core.cache_put("page:playlists".into(), b"x".to_vec()).unwrap();
        for _ in 0..4 {
            fake.answer(OK);
        }
        block(c.write(Write::Star { kind: Starrable::Song, id: "s1".into(), on: true })).unwrap();
        assert!(!c.core.cache_fresh("page:starred".into(), 60_000).unwrap());
        block(c.write(Write::Star { kind: Starrable::Song, id: "s1".into(), on: false })).unwrap();
        block(c.write(Write::CreatePlaylist { name: "nori check".into(), song_ids: vec!["s1".into(), "s2".into()] })).unwrap();
        assert!(!c.core.cache_fresh("page:playlists".into(), 60_000).unwrap());
        block(c.read_now(crate::cache_policy::Read::NowPlaying { id: "s1".into() })).unwrap();
        let asked = fake.asked();
        assert!(asked[0].contains("/rest/star?") && asked[0].ends_with("&id=s1"), "{}", asked[0]);
        assert!(asked[1].contains("/rest/unstar?") && asked[1].ends_with("&id=s1"), "{}", asked[1]);
        assert!(asked[2].contains("/rest/createPlaylist?") && asked[2].contains("&name=nori") && asked[2].ends_with("&songId=s1&songId=s2"), "{}", asked[2]);
        assert!(asked[3].contains("/rest/scrobble?") && asked[3].ends_with("&id=s1&submission=false"), "{}", asked[3]);
        assert!(c.core.pending_list().unwrap().is_empty());
    }

    #[test]
    fn login_and_probe() {
        let fake = Arc::new(Fake::default());
        let config = ServerConfig { url: "http://lan".into(), user: "u".into(), password: "p".into(), ..Default::default() };
        fake.fail(FailureKind::Connect);
        fake.answer(OK);
        assert!(!block(login_check(fake.clone(), config.clone(), "https://wan".into())).unwrap());
        assert!(fake.asked()[1].starts_with("https://wan/rest/ping"));

        let no_token = r#"{"subsonic-response":{"status":"failed","error":{"code":41,"message":"no token"}}}"#;
        fake.answer(no_token);
        fake.answer(OK);
        assert!(block(login_check(fake.clone(), config.clone(), String::new())).unwrap());
        assert!(fake.asked()[3].contains("&p=enc:70&"));

        fake.answer(no_token);
        let keyed = ServerConfig { api_key: Some("k".into()), ..config };
        assert!(matches!(block(login_check(fake.clone(), keyed, String::new())), Err(NetError::Api { code: 41, .. })));

        // Probe keeps second address.
        let (c, fake) = client(two_addresses());
        fake.fail(FailureKind::Timeout);
        assert!(block(c.choose_address()));
        *fake.pends.lock() = true;
        fake.fail(FailureKind::Timeout);
        let (_, during) = block(futures_util::future::join(c.choose_address(), async { c.core.url_prefix("stream") }));
        assert!(during.starts_with("https://wan.example/"), "{during}");
    }

    const EMPTY_PAGE: &str = r#"{"subsonic-response":{"status":"ok","searchResult3":{}}}"#;

    #[test]
    fn sync_drops_what_the_server_no_longer_has() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let sync = |page: &str| {
            fake.answer(page);
            fake.answer(EMPTY_PAGE);
            block(c.sync_library()).unwrap();
        };
        sync(r#"{"subsonic-response":{"status":"ok","searchResult3":{"album":[{"id":"al1","name":"Blue"},{"id":"al2","name":"Bluer"}],
            "song":[{"id":"a","title":"Blue one"},{"id":"b","title":"Blue two"}]}}}"#);
        assert_eq!(c.core.local_search("blue".into(), 10).unwrap().songs.len(), 2);
        sync(r#"{"subsonic-response":{"status":"ok","searchResult3":{"album":[{"id":"al1","name":"Blue"}],"song":[{"id":"a","title":"Blue one"}]}}}"#);
        let found = c.core.local_search("blue".into(), 10).unwrap();
        assert_eq!(found.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(found.albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["al1"]);
        assert_eq!(c.core.index_size().unwrap().songs, 1);

        // A walk cut short drops nothing.
        fake.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"song":[{"id":"c","title":"Blue three"}]}}}"#);
        fake.fail(FailureKind::Connect);
        assert!(block(c.sync_library()).is_err());
        assert_eq!(c.core.index_size().unwrap().songs, 2);
    }

    #[test]
    fn sync_pages_until_empty() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"artist":[{"id":"ar1","name":"A"}],
            "album":[{"id":"al1","name":"B"},{"id":"al2","name":"C"}],"song":[{"id":"s1","title":"x"},{"id":"s2","title":"y"},{"id":"s3","title":"z"}]}}}"#);
        fake.answer(EMPTY_PAGE);
        let total = block(c.sync_library()).unwrap();
        assert_eq!((total.artists, total.albums, total.songs), (1, 2, 3));
        let asked = fake.asked();
        assert!(asked[0].ends_with("&query=&songCount=500&songOffset=0&albumCount=500&albumOffset=0&artistCount=500&artistOffset=0"), "{}", asked[0]);
        assert!(asked[1].contains("&songOffset=500&"), "{}", asked[1]);
        assert_eq!(asked.len(), 2);
    }
}
