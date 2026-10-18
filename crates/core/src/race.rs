//! Lyrics calls: the server and enabled lyrics services together, through the client's transport
//! and the response cache. Service logic and ranking are nori-lyrics'.

use std::sync::Arc;

use crate::client::{Client, NetResult};
use crate::settings::StoredPrefs;
use crate::{Core, Song};

pub use nori_lyrics::race::*;
use nori_settings::lyrics_sources::{lyrics_lookup, LyricsLookup};

impl LyricsCache for Core {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.cache_get(key.to_string()).ok().flatten()
    }

    fn fresh(&self, key: &str, max_age_ms: i64) -> bool {
        self.cache_fresh(key.to_string(), max_age_ms).unwrap_or(false)
    }

    fn put(&self, key: &str, body: Vec<u8>) {
        let _ = self.cache_put(key.to_string(), body);
    }

    fn voice(&self, song: &Song) -> Option<nori_player::automix::vocal::VocalCurve> {
        self.analysis_voice(&song.id).ok().flatten()
    }
}

#[derive(Default)]
pub(crate) struct Prefetch {
    /// The queue entries asked for, in order.
    entries: Vec<u64>,
    /// Those of them already warmed.
    warmed: Vec<u64>,
    cancel: Option<futures_util::future::AbortHandle>,
}

// A cancelled caller releases its entries so a later visit can warm them again.
struct PrefetchRun<'a> {
    work: &'a parking_lot::Mutex<Prefetch>,
    entries: Vec<u64>,
}

impl PrefetchRun<'_> {
    fn warmed(&self, entry: u64) {
        let mut work = self.work.lock();
        if work.entries == self.entries { work.warmed.push(entry); }
    }

    fn finish(self) {
        let mut work = self.work.lock();
        if work.entries == self.entries { work.cancel = None; }
    }
}

impl Drop for PrefetchRun<'_> {
    fn drop(&mut self) {
        let mut work = self.work.lock();
        if work.entries == self.entries && work.cancel.take().is_some() {
            work.entries.clear();
        }
    }
}

/// Passes on only picks that replace the last one shown ([`lyrics_replaces`]), each with the timing the
/// listener gave those words of song `song_id`, if they did.
struct Screen {
    to: Arc<dyn LyricsShown>,
    last: parking_lot::Mutex<Option<LyricsPick>>,
    core: Arc<Core>,
    song_id: String,
}

impl LyricsShown for Screen {
    fn show(&self, mut pick: LyricsPick) {
        pick.lyrics.timing = nori_lyrics::timing::with_yours(pick.lyrics.timing, self.core.lyrics_yours(&self.song_id, &pick));
        let mut last = self.last.lock();
        if last.as_ref().is_some_and(|p| !p.lyrics.lines.is_empty() && pick.lyrics.lines.is_empty()) {
            return;
        }
        if lyrics_replaces(last.as_ref(), &pick) {
            *last = Some(pick.clone());
            drop(last);
            self.to.show(pick);
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Warms the lyrics of the songs prepared ahead ([`nori_queue::Session::lyrics_ahead`]) one after
    /// another, without publishing them to a screen.
    pub async fn lyrics_prefetch(&self) {
        use futures_util::future::{AbortHandle, Abortable};
        let next = self.core.session.lyrics_ahead(self.metered());
        let entries: Vec<u64> = next.iter().map(|(entry, _)| *entry).collect();
        let (old, todo, registration) = {
            let mut work = self.prefetch.lock();
            if work.entries == entries { return; }
            work.warmed.retain(|e| entries.contains(e));
            let todo: Vec<(u64, String)> = next.into_iter().filter(|(e, _)| !work.warmed.contains(e)).collect();
            work.entries = entries.clone();
            let old = work.cancel.take();
            let registration = (!todo.is_empty()).then(|| {
                let (cancel, registration) = AbortHandle::new_pair();
                work.cancel = Some(cancel);
                registration
            });
            (old, todo, registration)
        };
        if let Some(old) = old { old.abort(); }
        let Some(registration) = registration else { return };
        let run = PrefetchRun { work: &self.prefetch, entries };
        let warm = async {
            let mut all = true;
            for (entry, id) in todo {
                if self.lyrics_for(id.clone(), Arc::new(Unseen)).await.is_ok() {
                    crate::alog::info(&format!("lyrics: prefetched {id}"));
                    run.warmed(entry);
                } else {
                    all = false;
                }
            }
            all
        };
        // A failed song is tried again at the next visit.
        if Abortable::new(warm, registration).await == Ok(true) {
            run.finish();
        }
    }

    /// Streams song `id`'s lyrics to `shown`: the server's (cached, then changed), then the services'
    /// when those are not timed, and an empty server answer last if nobody had any. No pick is shown
    /// twice. Dropping the future cancels its requests.
    pub async fn lyrics_for(&self, id: String, shown: Arc<dyn LyricsShown>) -> NetResult<()> {
        let asked = self.settings().with_prefs(lyrics_lookup).unwrap_or_else(|| lyrics_lookup(&StoredPrefs::default()));
        self.lyrics_for_with(id, &asked, shown).await
    }
}

impl Client {
    /// [`Client::lyrics_for`] with the services given.
    async fn lyrics_for_with(&self, id: String, asked: &LyricsLookup, shown: Arc<dyn LyricsShown>) -> NetResult<()> {
        use crate::cache_policy::{Page, Read};
        let screen = Arc::new(Screen { to: shown, last: parking_lot::Mutex::new(None), core: self.core.clone(), song_id: id.clone() });
        let read = || Read::LyricsBySong { song_id: id.clone() };
        let show_server = |v: &crate::Lyrics| {
            let timed = screen.last.lock().as_ref().is_some_and(|pick| pick.lyrics.synced);
            if !v.lines.is_empty() && (!timed || v.synced) {
                screen.show(LyricsPick { lyrics: v.clone(), origin: nori_settings::lyrics_sources::LyricsOrigin::Server });
            }
        };
        let stored = self.read_stored(read()).ok();
        let digest = stored.as_ref().and_then(|s| s.digest);
        let server: Option<crate::Lyrics> = match stored.as_ref().and_then(|s| s.page.clone()) {
            Some(Page::LyricsPage { v }) => Some(v),
            _ => None,
        };
        if let Some(v) = &server {
            show_server(v);
        }
        let song = self.stored_song_of(&id);
        if let Some(song) = &song {
            if let Some(pick) = cached_pick(&*self.core, song,
                server.as_ref().is_some_and(|v| !v.lines.is_empty()),
                server.as_ref().is_some_and(|v| v.synced), asked) {
                screen.show(pick);
            }
        }
        // A downloaded song's lyrics were looked up at download: serve them from the cache without
        // requests; the server is re-asked only after a week, after showing them.
        let fresh = stored.as_ref().is_some_and(|s| s.fresh);
        let downloaded = server.is_some() && !fresh && self.core.download_song(&id).is_some();
        let refresh_after = downloaded && self.read_stored_within(read(), DOWNLOADED_SERVER_KEPT_MS).is_ok_and(|s| !s.fresh);
        let has_lines = server.as_ref().is_some_and(|l| !l.lines.is_empty());
        let synced = server.as_ref().is_some_and(|l| l.synced);
        let refresh = async {
            if !fresh && !downloaded {
                if let Ok(Some(Page::LyricsPage { v })) = self.read_refresh(read(), digest).await {
                    show_server(&v);
                    return v.synced;
                }
            }
            synced
        };
        let services = async {
            let song = match song { Some(song) => song, None => self.song_of(id.clone()).await? };
            if let Some(line) = lookup(&*self.transport, &*self.core, &song, has_lines, synced, asked, &*screen, &self.lyrics).await {
                nori_perf::perf_log::note_core("lyrics", line.trim_start_matches("lyrics: "));
            }
            Ok::<(), crate::transport::NetError>(())
        };
        // Timed server lyrics finish the lookup; otherwise both sources can publish as they arrive.
        use futures_util::future::{select, Either};
        match select(Box::pin(refresh), Box::pin(services)).await {
            Either::Left((timed, services)) => { if !timed { services.await?; } }
            Either::Right((result, refresh)) => { result?; refresh.await; }
        }
        if refresh_after {
            // Newly timed server lyrics win; anything else is used next time.
            if let Ok(Some(Page::LyricsPage { v })) = self.read_refresh(read(), digest).await {
                if v.synced && !synced {
                    show_server(&v);
                }
            }
        }
        Ok(())
    }
}

/// Freshness of a downloaded song's cached server lyrics.
pub(crate) const DOWNLOADED_SERVER_KEPT_MS: i64 = MISS_KEPT_MS;

impl Core {
    /// The offset the listener gave `pick`'s words of song `song_id`, if they did.
    fn lyrics_yours(&self, song_id: &str, pick: &LyricsPick) -> Option<i64> {
        let key = nori_lyrics::timing::key(song_id, pick.origin, &pick.lyrics);
        crate::db::kv_get(&self.db.lock(), &key).ok().flatten().and_then(|v| v.parse().ok())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// `pick`, song `song_id`'s lyrics, a step sooner (`dir` > 0) or later (< 0), or back to the sync
    /// check's timing (0); the listener's timing is kept for these words and given them whenever they
    /// are shown again.
    pub fn lyrics_nudge(&self, song_id: String, mut pick: LyricsPick, dir: i32) -> LyricsPick {
        pick.lyrics.timing = nori_lyrics::timing::nudged(pick.lyrics.timing, dir);
        let key = nori_lyrics::timing::key(&song_id, pick.origin, &pick.lyrics);
        let c = self.db.lock();
        let kept = match pick.lyrics.timing {
            nori_model::LyricsTiming::Yours { ms, .. } => crate::db::kv_put(&c, &key, &ms.to_string()),
            _ => crate::db::kv_delete(&c, &key),
        };
        if let Err(e) = kept {
            crate::alog::info(&format!("lyrics: timing not kept: {e}"));
        }
        pick
    }

    /// Bytes of cached service lyrics and picks.
    pub fn lyrics_cache_bytes(&self) -> i64 {
        let c = self.db.lock();
        c.query_row(
            "SELECT COALESCE(SUM(length(key) + length(body)), 0) FROM cache WHERE server=sid() AND key >= ?1 AND key < ?1 || x'ff'",
            [CACHE_PREFIX],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    /// Clears cached service lyrics, keeping the picks of downloaded songs (and the server's lyrics).
    pub fn lyrics_cache_clear(&self) {
        let keep: Vec<String> = self.downloads(true).unwrap_or_default().iter().map(best_key).collect();
        let keep = serde_json::to_string(&keep).unwrap_or_default();
        let _ = self.db.lock().execute(
            "DELETE FROM cache WHERE server=sid() AND key >= ?1 AND key < ?1 || x'ff' AND key NOT IN (SELECT value FROM json_each(?2))",
            rusqlite::params![CACHE_PREFIX, keep],
        );
    }
}

/// Discards picks (background lookups).
struct Unseen;

impl LyricsShown for Unseen {
    fn show(&self, _: LyricsPick) {}
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Looks up lyrics for downloaded songs one by one so they work offline, marking each song's lyrics
    /// work done when its lookup ends. Provider songs are skipped.
    pub async fn lyrics_for_downloads(&self, ids: Vec<String>) {
        for id in ids.into_iter().filter(|id| !crate::is_provider_id(id)) {
            let downloads = &self.core.downloads;
            downloads.with(|t| t.working(&id, crate::transfers::Work::Lyrics));
            let _ = self.lyrics_for(id.clone(), Arc::new(Unseen)).await;
            downloads.with(|t| t.work_done(&id, crate::transfers::Work::Lyrics));
        }
    }

}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::tests::{block, client, Fake};
    use crate::client::NetProfile;
    use crate::transport::FailureKind;
    use nori_settings::lyrics_sources::LyricsService;
    use nori_settings::lyrics_sources::LyricsOrigin;
    use nori_model::LyricsTiming;
    use parking_lot::Mutex;

    fn song() -> Song {
        Song { id: "1".into(), title: "Dogs (Remastered)".into(), artist: "Pink Floyd".into(), album: "Animals".into(), duration: 1024, ..Default::default() }
    }

    fn setup() -> (Arc<Client>, Arc<Fake>) {
        client(NetProfile { url: "h".into(), ..Default::default() })
    }

    fn lrclib() -> LyricsLookup {
        LyricsLookup { services: vec![LyricsService::Lrclib], prefer_words: true, paxsenix_key: String::new(), better_lyrics_key: String::new() }
    }

    #[derive(Default)]
    struct Screen(Mutex<Vec<LyricsPick>>);

    impl LyricsShown for Screen {
        fn show(&self, pick: LyricsPick) {
            self.0.lock().push(pick);
        }
    }

    fn run(c: &Client, s: &Song, has_lines: bool, synced: bool, asked: &LyricsLookup) -> Vec<LyricsPick> {
        let screen = Screen::default();
        block(lookup(&*c.transport, &*c.core, s, has_lines, synced, asked, &screen, &c.lyrics));
        screen.0.into_inner()
    }

    const SYNCED: &str = r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[{"synced":true,"line":[{"start":1500,"value":"timed"}]}]}}}"#;
    const NONE: &str = r#"{"subsonic-response":{"status":"ok","lyricsList":{}}}"#;

    #[test]
    fn services_show_while_the_server_is_pending() {
        use crate::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
        use std::future::Future;
        use std::task::{Context, Waker};

        struct SlowServer(Arc<Fake>);
        #[async_trait::async_trait]
        impl Transport for SlowServer {
            async fn get(&self, url: String, timeout: u32) -> Result<TransportResponse, TransportError> {
                if url.contains("getLyricsBySongId") { std::future::pending().await } else { self.0.get(url, timeout).await }
            }
            async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> { self.0.send(request).await }
            fn address_changed(&self) {}
            fn network(&self) -> Network { Network::Unmetered }
        }
        let (original, fake) = setup();
        let c = Client::new(original.core.clone(), Arc::new(SlowServer(fake.clone())), Default::default());
        c.set_profile(NetProfile { url: "h".into(), ..Default::default() });
        let s = song();
        c.core.session.register(vec![s.clone()]);
        fake.answer(r#"{"statusCode":404}"#);
        fake.answer(r#"[{"duration":1024,"syncedLyrics":"[00:02.00]line one\n[01:00.00]line two here\n[02:00.00]a third line\n[03:00.00]and the fourth"}]"#);
        let screen = Arc::new(Screen::default());
        let asked = lrclib();
        let mut lookup = Box::pin(c.lyrics_for_with(s.id, &asked, screen.clone()));
        assert!(lookup.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
        assert_eq!(screen.0.lock().last().map(|p| p.origin), Some(LyricsOrigin::Lrclib));
    }

    #[test]
    fn prefetch_warms_the_library_songs_prepared_ahead() {
        let stored = |c: &Client, id: &str| matches!(c.read_stored(crate::cache_policy::Read::LyricsBySong { song_id: id.into() }).ok().and_then(|s| s.page),
            Some(crate::cache_policy::Page::LyricsPage { v }) if !v.lines.is_empty());
        // Mobile data: the next song only, by default.
        for next in ["ahead", "ext-ahead"] {
            let (c, fake) = setup();
            *fake.metered.lock() = true;
            let ahead = Song { id: next.into(), ..song() };
            c.core.session.register(vec![Song { id: "current".into(), ..song() }, ahead]);
            c.core.session.set(vec!["current".into(), next.into(), "later".into()], Some(0), false, None);
            fake.answer(SYNCED);
            block(c.lyrics_prefetch());
            if next == "ahead" {
                assert!(stored(&c, next), "prefetch must store the next song's words before it is played");
                let shown = Arc::new(Screen::default());
                block(c.lyrics_for(next.into(), shown.clone())).unwrap();
                assert_eq!(shown.0.lock()[0].lyrics.lines[0].text, "timed");
                assert_eq!(fake.asked().len(), 1, "opening the prefetched song needs no request");
                assert!(fake.asked()[0].contains("id=ahead"));
                assert!(c.core.cache_get("getLyricsBySongId&id=later&enhanced=true".into()).unwrap().is_none());
            } else {
                assert!(fake.asked().is_empty(), "a provider's: no prefetch requests");
            }
        }
        // Wi-Fi: as many as the songs prepared ahead there, three by default.
        let (c, fake) = setup();
        let queue = ["current", "s1", "s2", "s3", "s4"];
        c.core.session.register(queue.iter().map(|id| Song { id: id.to_string(), ..song() }).collect());
        c.core.session.set(queue.iter().map(|id| id.to_string()).collect(), Some(0), false, None);
        (0..3).for_each(|_| fake.answer(SYNCED));
        block(c.lyrics_prefetch());
        assert_eq!(queue.map(|id| stored(&c, id)), [false, true, true, true, false]);
        // The next song moves on: only the newly prepared one is asked for.
        c.core.session.moved_to(1);
        fake.answer(SYNCED);
        block(c.lyrics_prefetch());
        assert!(stored(&c, "s4"));
        assert_eq!(fake.asked().len(), 4);
    }

    #[test]
    fn a_new_prefetch_cancels_the_obsolete_request() {
        use crate::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
        use std::future::Future;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::{Context, Waker};

        struct Pending(Arc<AtomicUsize>);
        impl Drop for Pending {
            fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); }
        }
        struct Gate { fake: Arc<Fake>, active: Arc<AtomicUsize> }
        #[async_trait::async_trait]
        impl Transport for Gate {
            async fn get(&self, url: String, timeout: u32) -> Result<TransportResponse, TransportError> {
                if url.contains("id=first") {
                    self.active.fetch_add(1, Ordering::SeqCst);
                    let _pending = Pending(self.active.clone());
                    std::future::pending().await
                } else { self.fake.get(url, timeout).await }
            }
            async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> { self.fake.send(request).await }
            fn address_changed(&self) {}
            fn network(&self) -> Network { Network::Unmetered }
        }
        let (original, fake) = setup();
        let active = Arc::new(AtomicUsize::new(0));
        let c = Client::new(original.core.clone(), Arc::new(Gate { fake: fake.clone(), active: active.clone() }), Default::default());
        c.set_profile(NetProfile { url: "h".into(), ..Default::default() });
        let dir = nori_testdir::TempDir::new("lyrics-prefetch-cancel");
        c.settings().open(&dir.join("settings.db").to_string_lossy()).unwrap();
        c.settings().edit_by_name("lyricsOnline", "false");
        c.core.session.register(vec![Song { id: "first".into(), ..song() }, Song { id: "second".into(), ..song() }]);
        c.core.session.set(vec!["now".into(), "first".into()], Some(0), false, None);
        let mut first = Box::pin(c.lyrics_prefetch());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert_eq!(active.load(Ordering::SeqCst), 1);
        // Asking again for the same entry starts no second request.
        block(c.lyrics_prefetch());
        assert_eq!(active.load(Ordering::SeqCst), 1);
        c.core.session.set(vec!["now".into(), "second".into()], Some(0), false, None);
        fake.answer(SYNCED);
        block(c.lyrics_prefetch());
        assert!(first.as_mut().poll(&mut cx).is_ready());
        assert_eq!(active.load(Ordering::SeqCst), 0, "the obsolete network future was dropped");
        let shown = Arc::new(Screen::default());
        block(c.lyrics_for("second".into(), shown.clone())).unwrap();
        assert_eq!(shown.0.lock()[0].lyrics.lines[0].text, "timed");
        assert_eq!(fake.asked().len(), 1);
        c.core.session.set(vec!["now".into(), "first".into()], Some(0), false, None);
        let mut cancelled = Box::pin(c.lyrics_prefetch());
        assert!(cancelled.as_mut().poll(&mut cx).is_pending());
        drop(cancelled);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        let mut retry = Box::pin(c.lyrics_prefetch());
        assert!(retry.as_mut().poll(&mut cx).is_pending(), "cancelling a caller allows this entry to be warmed again");
        assert_eq!(active.load(Ordering::SeqCst), 1);
        drop(retry);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        let mut removed = Box::pin(c.lyrics_prefetch());
        assert!(removed.as_mut().poll(&mut cx).is_pending());
        c.core.session.set(vec!["now".into()], Some(0), false, None);
        block(c.lyrics_prefetch());
        assert!(removed.as_mut().poll(&mut cx).is_ready());
        assert_eq!(active.load(Ordering::SeqCst), 0, "removing the upcoming entry cancels its request");
        assert_eq!(fake.asked().len(), 1);
    }

    #[test]
    fn server_and_services() {
        let (c, fake) = setup();
        // Unique titles: a failing service rests per song across tests.
        c.core.session.register(vec![Song { id: "lf1".into(), title: "In Order One".into(), ..song() }, Song { id: "lf2".into(), title: "In Order Two".into(), ..song() }]);
        fake.answer(SYNCED);
        let page = Arc::new(Screen::default());
        block(c.lyrics_for("lf1".into(), page.clone())).unwrap();
        let got = page.0.lock().clone();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].origin, got[0].lyrics.lines[0].text.as_str()), (LyricsOrigin::Server, "timed"));
        assert_eq!(fake.asked().len(), 1, "timed server lyrics: no service asked");
        // Stale, unchanged answer: not shown again.
        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        fake.answer(SYNCED);
        let again = Arc::new(Screen::default());
        block(c.lyrics_for("lf1".into(), again.clone())).unwrap();
        assert_eq!(again.0.lock().len(), 1);
        // No server lyrics: shown once, at the end.
        fake.answer(NONE);
        let none = Arc::new(Screen::default());
        block(c.lyrics_for("lf2".into(), none.clone())).unwrap();
        let got = none.0.lock().clone();
        assert_eq!(got.len(), 1);
        assert!(got[0].lyrics.lines.is_empty() && got[0].origin == LyricsOrigin::Server);

        // Synced server lyrics win and failures are not cached.
        let (c, fake) = setup();
        assert!(run(&c, &song(), true, true, &lrclib()).is_empty());
        assert!(fake.asked().is_empty());
        let s = Song { title: "Offline".into(), ..song() };
        fake.fail(FailureKind::UnknownHost);
        assert_eq!(run(&c, &s, false, false, &lrclib())[0].origin, LyricsOrigin::Server);
        assert_eq!(c.core.lyrics_cache_bytes(), 0);

        // Plain service lyrics do not replace server plain and miss is cached.
        let (c, fake) = setup();
        let s = Song { title: "Plain".into(), ..song() };
        fake.answer(r#"{"plainLyrics":"just words"}"#);
        fake.answer("[]");
        assert!(run(&c, &s, true, false, &lrclib()).is_empty());
        let (c, fake) = setup();
        fake.answer(r#"{"statusCode":404}"#);
        fake.answer("[]");
        let none = run(&c, &s, false, false, &lrclib());
        assert!(none[0].lyrics.lines.is_empty() && none[0].origin == LyricsOrigin::Server);
        run(&c, &s, false, false, &lrclib());
        assert_eq!(fake.asked().len(), 2, "miss cached: {:?}", fake.asked());
    }

    #[test]
    fn cached_service_words_arrive_before_the_server_refresh() {
        struct Immediate { fake: Arc<Fake>, shown: Mutex<Vec<(LyricsPick, usize)>> }
        impl LyricsShown for Immediate {
            fn show(&self, pick: LyricsPick) { self.shown.lock().push((pick, self.fake.asked().len())); }
        }
        let (c, fake) = setup();
        let s = Song { id: "cached-before-refresh".into(), title: "Before the Rain".into(), ..song() };
        c.core.session.register(vec![s.clone()]);
        fake.answer(r#"{"statusCode":404}"#);
        fake.answer(r#"[{"duration":1022,"syncedLyrics":"[00:02.00]line one\n[01:00.00]line two here\n[02:00.00]a third line\n[03:00.00]and the fourth"}]"#);
        let found = run(&c, &s, false, false, &lrclib());
        assert_eq!(found[0].origin, LyricsOrigin::Lrclib);
        c.core.cache_put(format!("getLyricsBySongId&id={}&enhanced=true", s.id), NONE.as_bytes().to_vec()).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts = 0 WHERE key LIKE 'getLyricsBySongId%'", []).unwrap();
        let before = fake.asked().len();
        fake.fail(FailureKind::Timeout);
        let screen = Arc::new(Immediate { fake: fake.clone(), shown: Mutex::new(Vec::new()) });
        block(c.lyrics_for_with(s.id, &lrclib(), screen.clone())).unwrap();
        let shown = screen.shown.lock();
        assert_eq!(shown[0].0.origin, LyricsOrigin::Lrclib);
        assert_eq!(shown[0].1, before, "cached words must arrive before attempting the slow server request");
    }

    #[test]
    fn your_timing_stays_with_those_words() {
        let (c, fake) = setup();
        let s = Song { id: "nudged".into(), title: "Before the Rain".into(), ..song() };
        c.core.session.register(vec![s.clone()]);
        fake.answer(r#"{"statusCode":404}"#);
        fake.answer(r#"[{"duration":1022,"syncedLyrics":"[00:02.00]line one\n[01:00.00]line two here\n[02:00.00]a third line\n[03:00.00]and the fourth"}]"#);
        run(&c, &s, false, false, &lrclib());
        c.core.cache_put(format!("getLyricsBySongId&id={}&enhanced=true", s.id), NONE.as_bytes().to_vec()).unwrap();
        let shown = || {
            let screen = Arc::new(Screen::default());
            block(c.lyrics_for_with(s.id.clone(), &lrclib(), screen.clone())).unwrap();
            let first = screen.0.lock()[0].clone();
            assert_eq!(first.origin, LyricsOrigin::Lrclib);
            first
        };
        let pick = shown();
        assert_eq!(pick.lyrics.timing, LyricsTiming::NotChecked);
        let once = c.core.lyrics_nudge(s.id.clone(), pick.clone(), 1);
        let twice = c.core.lyrics_nudge(s.id.clone(), once, 1);
        assert_eq!(twice.lyrics.timing, LyricsTiming::Yours { ms: 2 * nori_look::lyrics::NUDGE_STEP_MS, automatic: None });
        assert_eq!(shown().lyrics.timing, twice.lyrics.timing, "shown again, still yours");
        assert_eq!(c.core.lyrics_yours(&s.id, &LyricsPick { origin: LyricsOrigin::Netease, ..pick.clone() }), None, "another source's words");
        assert_eq!(c.core.lyrics_yours("other", &pick), None, "another song");
        assert_eq!(c.core.lyrics_nudge(s.id.clone(), shown(), 0).lyrics.timing, LyricsTiming::NotChecked);
        assert_eq!(shown().lyrics.timing, LyricsTiming::NotChecked, "reset is forgotten for good");
    }

    #[test]
    fn service_hit_cached_and_ranked() {
        let (c, fake) = setup();
        fake.answer(r#"{"statusCode":404,"message":"not found"}"#);
        fake.answer(r#"[{"duration":1000,"syncedLyrics":"[00:01.00]far"},{"duration":1026,"plainLyrics":"close plain"},{"duration":1022,"syncedLyrics":"[00:02.00]close synced\n[01:00.00]you gotta be crazy\n[02:00.00]you gotta have a real need\n[03:00.00]you gotta sleep on your toes"}]"#);
        let got = run(&c, &song(), true, false, &lrclib());
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].origin, got[0].lyrics.lines[0].text.as_str()), (LyricsOrigin::Lrclib, "close synced"));
        let asked = fake.asked();
        assert!(asked[0].contains("track_name=Dogs&"));
        assert!(asked[1].starts_with("https://lrclib.net/api/search?track_name=Dogs&artist_name=Pink+Floyd"));
        // Cached: no request the second time.
        let again = run(&c, &song(), false, false, &lrclib());
        assert_eq!(again[0].lyrics.lines[0].text, "close synced");
        assert_eq!(fake.asked().len(), 2);
    }

    #[test]
    fn lyrics_cache_size_and_clear() {
        let (c, fake) = setup();
        fake.answer(r#"{"statusCode":404}"#);
        fake.answer(r#"[{"duration":1022,"syncedLyrics":"[00:02.00]line one\n[01:00.00]line two here\n[02:00.00]a third line\n[03:00.00]and the fourth"}]"#);
        assert_eq!(c.core.lyrics_cache_bytes(), 0);
        c.core.cache_put("getAlbum|1".into(), b"{}".to_vec()).unwrap();
        run(&c, &song(), false, false, &lrclib());
        let bytes = c.core.lyrics_cache_bytes();
        assert!(bytes > 100, "{bytes}");
        c.core.lyrics_cache_clear();
        assert_eq!(c.core.lyrics_cache_bytes(), 0);
        assert!(c.core.cache_get("getAlbum|1".into()).unwrap().is_some(), "other cache entries stay");
    }

    /// Lyrics of an analysed song get the offset of their times against its vocal curve.
    #[test]
    fn analysed_lyrics_get_sync_offset() {
        use nori_player::automix::analysis::Analyzer;
        use nori_player::automix::eval::{Song as Synthetic, Style, FULL, SUNG};
        let (c, fake) = setup();
        let synthetic = Synthetic { sections: vec![(4, FULL), (12, SUNG), (6, FULL), (12, SUNG), (4, FULL)], ..Synthetic::new("offset", Style::Backbeat, 112.0, 2, false) };
        let (x, truth) = synthetic.render();
        let mut a = Analyzer::new(synthetic.rate, 0);
        a.feed(&x);
        nori_automix::store::put_voice(&c.core.db.lock(), "sync1", &a.take_features().voice_curve()).unwrap();
        // A line per sung phrase, 1.2 s late (plus the 0.2 s lead sync.rs expects).
        let mut lrc = String::new();
        let mut last = f64::MIN;
        for (i, (start, _)) in truth.voice.iter().enumerate() {
            if start - last > 0.3 {
                let t = start - 0.2 + 1.2;
                lrc.push_str(&format!("[{:02}:{:05.2}]lomira teshvan korupel sumidah {i} netori falquen\n", (t / 60.0) as u32, t % 60.0));
            }
            last = truth.voice[i].1;
        }
        let secs = (x.len() / synthetic.rate as usize) as u32;
        fake.answer(&serde_json::json!({"syncedLyrics": lrc, "duration": secs, "trackName": "Offset", "artistName": "Pink Floyd"}).to_string());
        let s = Song { id: "sync1".into(), title: "Offset".into(), duration: secs, ..song() };
        let got = run(&c, &s, false, false, &lrclib());
        let l = &got.last().expect("lyrics").lyrics;
        assert!(matches!(l.timing, LyricsTiming::Automatic { ms } if (ms - 1200).abs() < 150), "{:?}", l.timing);
        assert_eq!(l.lines[0].start_ms, ((truth.voice[0].0 - 0.2 + 1.2) * 1000.0 / 10.0).round() as i64 * 10, "times unchanged");
    }

    #[test]
    fn downloaded_lyrics() {
        let (c, fake) = setup();
        let kept = Song { id: "dl-lyr".into(), title: "Paper Lanterns".into(), artist: "The Invented".into(), album: "Nowhere".into(), duration: 200, ..Default::default() };
        let other = Song { id: "st-lyr".into(), title: "Streamed Only".into(), ..kept.clone() };
        c.core.download_queue(vec![kept.clone()]).unwrap();
        c.core.download_done("dl-lyr".into()).unwrap();
        for s in [&kept, &other] {
            fake.answer(r#"{"statusCode":404}"#);
            fake.answer(r#"[{"duration":200,"syncedLyrics":"[00:02.00]a line made up\n[01:00.00]another made up line\n[02:00.00]a third invented one\n[03:00.00]and the last of them"}]"#);
            assert_eq!(run(&c, s, false, false, &lrclib())[0].origin, LyricsOrigin::Lrclib);
        }
        let kept_at = |c: &Client| c.core.db.lock().query_row("SELECT ts FROM cache WHERE key = ?1", [best_key(&kept)], |r| r.get::<_, i64>(0)).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts = ts - 86400000", []).unwrap();
        let aged = kept_at(&c);
        c.core.lyrics_cache_clear();
        assert_eq!(kept_at(&c), aged, "a kept pick keeps its age, which times its lookup again");
        // Offline from here.
        let asked = fake.asked().len();
        let offline = run(&c, &kept, false, false, &lrclib());
        assert_eq!((offline[0].origin, offline[0].lyrics.lines[0].text.as_str()), (LyricsOrigin::Lrclib, "a line made up"));
        assert_eq!(fake.asked().len(), asked);
        assert!(run(&c, &other, false, false, &lrclib())[0].lyrics.lines.is_empty(), "streamed song's cleared");
        c.core.download_remove("dl-lyr".into()).unwrap();
        c.core.lyrics_cache_clear();
        assert_eq!(c.core.lyrics_cache_bytes(), 0, "download deleted");

        // A downloaded song's lyrics open from the cache with no request, online or off; after a week the
        // server is re-asked, after showing them.
        let (c, fake) = setup();
        let s = Song { id: "dl-open".into(), title: "Harbour Of Tin".into(), artist: "The Invented".into(), album: "Nowhere".into(), duration: 200, ..Default::default() };
        let asked = LyricsLookup { services: vec![LyricsService::Lrclib, LyricsService::LyricsPlus], ..lrclib() };
        c.core.session.register(vec![s.clone()]);
        c.core.download_queue(vec![s.clone()]).unwrap();
        c.core.download_done(s.id.clone()).unwrap();
        // At download: none on the server, line-timed LRCLIB, LyricsPlus unreachable.
        fake.answer(NONE);
        fake.answer(r#"{"statusCode":404}"#);
        fake.answer(r#"[{"duration":200,"syncedLyrics":"[00:02.00]tin boats in the harbour\n[01:00.00]a lantern made of paper\n[02:00.00]nobody sings this line\n[03:00.00]and the tide goes out"}]"#);
        let first = open(&c, &fake, &s.id, &asked);
        assert_eq!(first.last().unwrap().1.origin, LyricsOrigin::Lrclib);
        let kept = |picks: &[(usize, LyricsPick)]| picks.first().is_some_and(|(_, p)| p.origin == LyricsOrigin::Lrclib && p.lyrics.lines[0].text == "tin boats in the harbour");
        let age = |ms: i64| c.core.db.lock().execute("UPDATE cache SET ts = ts - ?1", [ms]).unwrap();
        // Two hours on, online: not asked.
        age(2 * 3_600_000);
        let before = fake.asked().len();
        fake.answer(NONE);
        let online = open(&c, &fake, &s.id, &asked);
        assert!(kept(&online), "{online:?}");
        assert_eq!(fake.asked().len(), before, "opened online: {:?}", &fake.asked()[before..]);
        assert_eq!(online.len(), 1);
        fake.answers.lock().clear();
        let offline = open(&c, &fake, &s.id, &asked);
        assert!(kept(&offline), "{offline:?}");
        assert_eq!(fake.asked().len(), before, "opened offline: {:?}", &fake.asked()[before..]);
        // A week on: shown first, then the server is re-asked once.
        age(8 * DAY);
        let late = open(&c, &fake, &s.id, &asked);
        assert!(kept(&late) && late[0].0 == before, "{late:?}");
        let asked_after = &fake.asked()[before..];
        assert!(matches!(asked_after, [u] if u.contains("getLyricsBySongId")), "{asked_after:?}");

        // Song of reads downloads without server.
        let (c, fake) = setup();
        let s = Song { id: "dl-known".into(), title: "Kept Here".into(), artist: "The Invented".into(), duration: 180, ..Default::default() };
        c.core.download_queue(vec![s.clone()]).unwrap();
        c.core.download_done(s.id.clone()).unwrap();
        let got = block(c.song_of(s.id.clone())).unwrap();
        assert_eq!((got.title.as_str(), got.duration), ("Kept Here", 180));
        assert!(fake.asked().is_empty(), "{:?}", fake.asked());
    }

    /// Picks shown, each with the request count at that moment.
    struct Seen {
        fake: Arc<Fake>,
        picks: Mutex<Vec<(usize, LyricsPick)>>,
    }

    impl LyricsShown for Seen {
        fn show(&self, pick: LyricsPick) {
            self.picks.lock().push((self.fake.asked().len(), pick));
        }
    }

    fn open(c: &Client, fake: &Arc<Fake>, id: &str, asked: &LyricsLookup) -> Vec<(usize, LyricsPick)> {
        let seen = Arc::new(Seen { fake: fake.clone(), picks: Mutex::new(Vec::new()) });
        block(c.lyrics_for_with(id.into(), asked, seen.clone())).unwrap();
        let picks = seen.picks.lock().clone();
        picks
    }

    const DAY: i64 = 24 * 3_600_000;

    #[test]
    fn download_lyrics_in_order() {
        let (c, fake) = setup();
        fake.answer(SYNCED);
        fake.answer(SYNCED);
        block(c.lyrics_for_downloads(vec!["ext-deezer-7".into(), "dla-1".into(), "dla-2".into()]));
        let lyrics: Vec<String> = fake.asked().into_iter().filter(|u| u.contains("getLyricsBySongId")).collect();
        assert_eq!(lyrics.len(), 2, "{lyrics:?}");
        assert!(lyrics[0].contains("id=dla-1&") && lyrics[1].contains("id=dla-2&"));
        assert!(fake.asked().iter().all(|u| !u.contains("ext-")));
    }

}
