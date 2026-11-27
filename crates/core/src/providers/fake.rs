//! A provider of one album and an account behind a password, standing in for a third kind: the core's
//! generic paths (search, pages, play, sign-in) work through it and name no provider.

use std::sync::Arc;

use parking_lot::Mutex;

use super::{AccountState, Answer, FieldSpec, FormField, Provider, ProviderInfo, ResolveStream, SignIn, SignInResult};
use crate::browse::{profile_rules, ProfileRules};
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Starrable, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::Transport;
use crate::{Album, AlbumDetail, Core, SearchResult, Song};

#[derive(Default)]
struct Fake {
    signed_in: Mutex<Option<String>>,
    hearts: Mutex<Vec<String>>,
    /// Home feeds read; each answers with another shelf, as a service's feed does.
    feeds: Mutex<u32>,
    /// The source ended the session; and whether the client was told to stop reminding of it.
    ended: Mutex<(bool, bool)>,
    /// Album pages read.
    albums_read: Mutex<u32>,
    /// Its album lists its songs the other way round, as a newer answer might.
    turned: Mutex<bool>,
    /// Nothing reaches it.
    offline: Mutex<bool>,
    /// Its long playlist's reads, and what holds each after its first part until the test lets it go.
    long_reads: Mutex<u32>,
    long_held: Mutex<Option<futures_channel::oneshot::Receiver<()>>>,
}

/// Signed out, its favorites are kept on this device.
fn rules(signed_in: bool) -> ProfileRules {
    ProfileRules { account: false, playlists: signed_in, indexed: false, remote: false, stars: true, stars_here: !signed_in, scrobbles: signed_in, feed: true, mixes: signed_in, ..profile_rules(false) }
}

fn song(id: &str, title: &str) -> Song {
    Song { id: id.into(), title: title.into(), album: "Fakes".into(), album_id: Some("f-al".into()), duration: 60, ..Default::default() }.dressed()
}

fn album() -> AlbumDetail {
    AlbumDetail::new(Album { id: "f-al".into(), name: "Fakes".into(), song_count: 2, ..Default::default() }, vec![song("f-1", "One"), song("f-2", "Two")], Vec::new())
}

/// Its long playlist as far as `songs` of it.
fn long(songs: usize) -> Page {
    let all = [song("f-1", "One"), song("f-2", "Two"), song("f-3", "Three")];
    Page::PlaylistPage { v: crate::PlaylistDetail::new(crate::Playlist { id: "f-long".into(), song_count: 3, ..Default::default() }, all[..songs].to_vec()) }
}

#[async_trait::async_trait]
impl Provider for Fake {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { kind: ServerKind::default(), fields: vec![FieldSpec { what: FormField::Name, advanced: false }], sign_in: vec![SignIn::Paste], identities: None }
    }

    fn ready(&self, _: &SavedServer) -> bool {
        true
    }

    async fn check(&self, _: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
        Ok(profile)
    }

    fn rules(&self, profile: &SavedServer, _: bool) -> ProfileRules {
        rules(!profile.user.is_empty())
    }

    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()> {
        core.set_rules(rules(!profile.user.is_empty()));
        Ok(())
    }

    fn favorites_here(&self) -> bool {
        true
    }

    /// Its account has hearts for songs alone.
    fn keeps_favorites(&self, kind: Starrable) -> bool {
        kind == Starrable::Song
    }

    async fn read(&self, _: &Client, read: Read) -> NetResult<Option<Answer>> {
        if *self.offline.lock() {
            return Err(crate::transport::NetError::Transport { kind: crate::transport::FailureKind::Connect, detail: None });
        }
        Ok(match read {
            Read::StarredItems => {
                let hearts = self.hearts.lock();
                Some(Page::StarredPage { v: crate::Starred::new(Vec::new(), Vec::new(), album().songs.into_iter().filter(|s| hearts.contains(&s.id)).collect()) })
            }
            Read::Search { query, .. } => Some(Page::Found { v: SearchResult { songs: album().songs.into_iter().filter(|s| s.title.to_lowercase().contains(&query)).collect(), ..Default::default() } }),
            Read::AlbumById { id } if id == "f-al" => {
                *self.albums_read.lock() += 1;
                let mut v = album();
                if *self.turned.lock() {
                    v = AlbumDetail::new(v.album, v.songs.into_iter().rev().collect(), Vec::new());
                }
                Some(Page::AlbumPage { v })
            }
            Read::PlaylistById { id } if id == "f-long" => {
                *self.long_reads.lock() += 1;
                Some(long(3))
            }
            // Its own playlist names no owner; one it follows names who made it.
            Read::PlaylistList => {
                let list = |id: &str, owner: Option<&str>| crate::Playlist { id: id.into(), name: id.into(), owner: owner.map(str::to_string), ..Default::default() };
                Some(Page::Playlists { v: vec![list("f-own", None), list("f-followed", Some("someone"))] })
            }
            Read::PlaylistById { id } if id == "f-own" => Some(Page::PlaylistPage { v: crate::PlaylistDetail::new(crate::Playlist { id, ..Default::default() }, vec![album().songs.remove(0)]) }),
            Read::HomeFeed => {
                Moment::default().await;
                let mut n = self.feeds.lock();
                *n += 1;
                // As a service's may, it lists a song twice, and part of its shelf again under the same title.
                let [one, two] = <[Song; 2]>::try_from(album().songs).expect("two songs");
                let shelf = |songs: Vec<Song>| crate::FeedShelf { title: format!("feed {n}"), songs, ..Default::default() };
                Some(Page::Feed { v: vec![shelf(vec![one.clone(), one]), shelf(vec![two])] })
            }
            _ => None,
        }
        .map(|p| Answer::Page(Box::new(p))))
    }

    /// Its long playlist shows its first song, then waits for the test before the whole.
    async fn read_in_parts(&self, client: &Client, read: Read, parts: crate::cache_policy::Parts) -> NetResult<Option<Answer>> {
        if matches!(&read, Read::PlaylistById { id } if id == "f-long") {
            let _ = parts.unbounded_send(long(1));
            let held = self.long_held.lock().take();
            if let Some(held) = held {
                let _ = held.await;
            }
        }
        self.read(client, read).await
    }

    async fn write(&self, _: &Client, w: Write) -> NetResult<()> {
        match w {
            Write::Star { kind: Starrable::Song, id, on } if self.signed_in.lock().is_some() => {
                let mut hearts = self.hearts.lock();
                hearts.retain(|h| *h != id);
                if on {
                    hearts.push(id);
                }
                Ok(())
            }
            _ => Err(super::nowhere("write")),
        }
    }

    fn stream(&self, _: &Client, id: String, _: StreamQuality) -> StreamTarget {
        StreamTarget { url: format!("fake:{id}"), key: format!("{id}:0") }
    }

    fn ahead(&self) -> Option<usize> {
        Some(0)
    }

    fn cover_address(&self, _: &Core, cover: &str, size: u32) -> String {
        format!("https://fake.example/cover/{cover}/{size}")
    }

    fn cover_sizes(&self) -> &[u32] {
        &[226, 544, 1200]
    }

    fn resolver(&self, _: &Arc<Client>) -> Option<Arc<dyn ResolveStream>> {
        Some(Arc::new(Opens))
    }

    async fn sign_in(&self, client: &Client, answer: String) -> SignInResult {
        if answer != "let me in" {
            return SignInResult::Refused;
        }
        *self.signed_in.lock() = Some("Fay".into());
        client.core.set_rules(rules(true));
        SignInResult::SignedIn { name: "Fay".into() }
    }

    fn account(&self, _: &Client) -> AccountState {
        if self.ended.lock().0 {
            return AccountState::Ended;
        }
        if self.signed_in.lock().is_some() { AccountState::SignedIn } else { AccountState::SignedOut }
    }

    fn reminds_to_sign_in(&self, _: &Client) -> bool {
        *self.ended.lock() == (true, false)
    }

    fn stop_reminding(&self, _: &Client) {
        self.ended.lock().1 = true;
    }
}

/// Ready when polled again: a source's answer takes a moment, so reads asked together are on their way together.
#[derive(Default)]
struct Moment(bool);

impl std::future::Future for Moment {
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        if std::mem::replace(&mut self.0, true) {
            return std::task::Poll::Ready(());
        }
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    }
}

struct Opens;

impl ResolveStream for Opens {
    fn resolve(&self, address: &str) -> Option<NetResult<String>> {
        address.strip_prefix("fake:").map(|id| Ok(format!("https://fake.example/{id}")))
    }

    fn refused(&self, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, Fake as Wire};

    /// A client over the fake provider, its network a scripted transport no test answers.
    fn fake() -> (Arc<Client>, Arc<Fake>) {
        fake_at(String::new())
    }

    /// [`fake`] over the database at `db` (empty: one in memory).
    fn fake_at(db: String) -> (Arc<Client>, Arc<Fake>) {
        let core = Core::new(db, "fake".into(), Default::default()).unwrap();
        let provider = Arc::new(Fake::default());
        core.install(ServerKind::default(), provider.clone());
        core.configure_profile(SavedServer::default()).unwrap();
        (Client::new(core, Arc::new(Wire::default()), Default::default()), provider)
    }

    #[test]
    fn signing_in_is_offered_again_once_the_session_ended_until_told_to_stop() {
        use crate::providers::flow::SignInEvent;
        let (c, fake) = fake();
        assert_eq!(c.sign_in_due(None), None);
        fake.ended.lock().0 = true;
        assert_eq!(c.sign_in_due(None), Some(SignInEvent::Offer { ended: true }));
        c.stop_reminding();
        assert_eq!(c.sign_in_due(None), None);
        assert_eq!(c.sign_in_due(Some(super::super::AfterAdding::Remind)), None, "it signs in by pasting, not on a page: nothing follows adding");
    }

    /// Search and an album's page come from the provider and are indexed; what it has nothing for is
    /// the empty page, and nothing goes to a Subsonic server.
    #[test]
    fn search_and_pages_come_from_the_provider() {
        let (c, _) = fake();
        let Page::Found { v } = block(c.read_now(Read::Search { query: "two".into(), songs: 10, albums: 10, artists: 10 })).unwrap() else { panic!() };
        assert_eq!(v.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["f-2"]);
        let Some(Page::AlbumPage { v }) = block(c.cached_or_fetched(Read::AlbumById { id: "f-al".into() })).unwrap() else { panic!() };
        assert_eq!(v.songs.len(), 2);
        assert_eq!(c.core.local_search("one".into(), 10).unwrap().songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["f-1"], "indexed");
        assert!(matches!(block(c.read_now(Read::GenreList)), Ok(Page::Genres { v }) if v.is_empty()));
        assert!(block(c.sync_library()).is_err(), "no server to sync");
    }

    /// Home's feed is read once and kept for the whole session, however long ago; Home's refresh asks
    /// anew and adds what is new after what was shown, as for every provider.
    #[test]
    fn home_feed_is_kept_for_the_session_and_merged_on_refresh() {
        let (c, fake) = fake();
        let shown = || {
            let mut pages = Vec::new();
            block(c.read_each(Read::HomeFeed, |p| if let Page::Feed { v } = p { pages.push(v.iter().map(|s| s.title.clone()).collect::<Vec<_>>()) })).unwrap();
            pages
        };
        assert_eq!(shown(), [["feed 1"]]);
        crate::providers::harness::age(&c);
        assert_eq!(shown(), [["feed 1"]], "kept: not asked again");
        assert_eq!(*fake.feeds.lock(), 1);
        c.drop_cached(crate::browse::home_refresh_drops()).unwrap();
        assert_eq!(shown(), [vec!["feed 1"], vec!["feed 1", "feed 2"]], "a refresh asks again and keeps what was shown");
    }

    /// A feed that lists an item twice, or a shelf's title twice, shows one shelf of each title with each
    /// item once.
    #[test]
    fn home_shows_each_shelf_and_item_once() {
        let (c, _) = fake();
        let Ok(Some(Page::Feed { v })) = block(c.cached_or_fetched(Read::HomeFeed)) else { panic!("a feed") };
        let shown: Vec<(&str, Vec<&str>)> = v.iter().map(|s| (s.title.as_str(), s.songs.iter().map(|s| s.id.as_str()).collect())).collect();
        assert_eq!(shown, [("feed 1", vec!["f-1", "f-2"])]);
    }

    /// The last session's Home shows at once in the next, and the feed asked again replaces it.
    #[test]
    fn home_shows_the_last_sessions_feed_until_asked_again() {
        let dir = nori_testdir::TempDir::new("fake-home");
        let db = dir.join("nori.db").to_str().unwrap().to_string();
        let titles = |c: &Client| {
            let mut pages = Vec::new();
            block(c.read_each(Read::HomeFeed, |p| if let Page::Feed { v } = p { pages.push(v.iter().map(|s| s.title.clone()).collect::<Vec<_>>()) })).unwrap();
            pages
        };
        let (c, _) = fake_at(db.clone());
        assert_eq!(titles(&c), [["feed 1"]]);
        drop(c);
        let (c, fake) = fake_at(db);
        *fake.feeds.lock() = 5;
        assert_eq!(titles(&c), [["feed 1"], ["feed 6"]]);
        assert_eq!(titles(&c), [["feed 6"]], "this session's, kept");
    }

    fn album_pages(c: &Client) -> Vec<Vec<String>> {
        let mut pages = Vec::new();
        let _ = block(c.read_each(Read::AlbumById { id: "f-al".into() }, |p| if let Page::AlbumPage { v } = p { pages.push(v.songs.into_iter().map(|s| s.id).collect()) }));
        pages
    }

    /// A provider's page saved in one session shows at once in the next, and its source is asked again:
    /// the same answer shows nothing new, a changed one replaces it.
    #[test]
    fn a_page_saved_in_one_session_shows_at_once_in_the_next() {
        let dir = nori_testdir::TempDir::new("fake-saved");
        let db = dir.join("nori.db").to_str().unwrap().to_string();
        let (c, _) = fake_at(db.clone());
        assert_eq!(album_pages(&c), [["f-1", "f-2"]]);
        drop(c);

        let (c, fake) = fake_at(db.clone());
        assert_eq!(album_pages(&c), [["f-1", "f-2"]], "the saved copy; the same answer shows nothing more");
        assert_eq!(*fake.albums_read.lock(), 1, "asked again");
        assert_eq!(album_pages(&c), [["f-1", "f-2"]]);
        assert_eq!(*fake.albums_read.lock(), 1, "then kept for the session as before");
        drop(c);

        let (c, fake) = fake_at(db);
        *fake.turned.lock() = true;
        assert_eq!(album_pages(&c), [["f-1", "f-2"], ["f-2", "f-1"]], "the saved copy, then the newer answer");
    }

    /// A saved page is only ever a page's last answer: shown offline, it adds nothing to the index; and
    /// signing out drops it.
    #[test]
    fn a_saved_page_is_never_indexed_and_goes_on_sign_out() {
        let dir = nori_testdir::TempDir::new("fake-saved-index");
        let db = dir.join("nori.db").to_str().unwrap().to_string();
        let (c, _) = fake_at(db.clone());
        album_pages(&c);
        c.core.db.lock().execute_batch("DELETE FROM fts; DELETE FROM items;").unwrap();
        drop(c);

        let (c, fake) = fake_at(db.clone());
        *fake.offline.lock() = true;
        assert_eq!(album_pages(&c), [["f-1", "f-2"]], "offline, the saved copy shows");
        assert!(c.core.local_search("one".into(), 10).unwrap().songs.is_empty(), "nothing of it indexed");
        c.sign_out();
        drop(c);

        let (c, fake) = fake_at(db);
        *fake.offline.lock() = true;
        assert!(album_pages(&c).is_empty(), "signed out, nothing saved shows");
    }

    /// A playlist shown in part is marked as filling; a queue started from that part takes the rest once
    /// the read ends, waiting for it rather than asking again.
    #[test]
    fn a_queue_from_a_page_shown_in_part_takes_the_rest_from_its_read() {
        let (c, fake) = fake();
        let (go, held) = futures_channel::oneshot::channel();
        *fake.long_held.lock() = Some(held);
        let (shown, pages) = std::sync::mpsc::channel();
        let reading = {
            let c = c.clone();
            std::thread::spawn(move || block(c.read_each(Read::PlaylistById { id: "f-long".into() }, |p| if let Page::PlaylistPage { v } = p { shown.send(v).unwrap() })))
        };
        let wait = std::time::Duration::from_secs(10);
        let part = pages.recv_timeout(wait).unwrap();
        assert!(part.filling && part.songs.len() == 1, "the first part, marked");
        c.session().register(part.songs.clone());
        let started = c.session().set(part.songs.iter().map(|s| s.id.clone()).collect(), Some(0), false, Some(part.queue.origin()));
        assert!(started.follows);
        let following = {
            let c = c.clone();
            std::thread::spawn(move || block(c.queue_follow()))
        };
        go.send(()).unwrap();
        reading.join().unwrap().unwrap();
        let whole = pages.recv_timeout(wait).unwrap();
        assert!(!whole.filling && whole.songs.len() == 3);
        let followed = following.join().unwrap().expect("the rest joined");
        assert_eq!(followed.edit.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["f-2", "f-3"]);
        assert_eq!(c.session().playlist(|p| p.ids().to_vec()), ["f-1", "f-2", "f-3"]);
        assert_eq!(*fake.long_reads.lock(), 1, "the page's own read, not another");
    }

    /// Home and the widget's shelves asked at once ask the source once, and all show its feed.
    #[test]
    fn home_asked_together_is_asked_of_the_source_once() {
        let (c, fake) = fake();
        let mut shown = Vec::new();
        let home = c.read_each(Read::HomeFeed, |p| if let Page::Feed { v } = p { shown.extend(v.into_iter().map(|s| s.title)) });
        let (h, albums, playlists) = block(futures_util::future::join3(home, c.featured_albums(5), c.featured_playlists()));
        h.unwrap();
        albums.unwrap();
        playlists.unwrap();
        assert_eq!(*fake.feeds.lock(), 1);
        assert_eq!(shown, ["feed 1"]);
    }

    /// A song plays from the provider's address, opened at the URL it resolves to; covers are its own.
    #[test]
    fn songs_play_from_the_providers_addresses() {
        let (c, _) = fake();
        let t = c.resolve("f-1".into(), false, false);
        assert_eq!((t.url.as_str(), t.key.as_str()), ("fake:f-1", "f-1:0"));
        let opens = c.stream_resolver().unwrap();
        assert_eq!(opens.resolve(&t.url).unwrap().unwrap(), "https://fake.example/f-1");
        c.session().register(album().songs);
        c.session().set(vec!["f-1".into(), "f-2".into()], Some(0), false, None);
        assert!(c.precache_targets(false).is_empty(), "nothing past the next song");
        assert_eq!(c.core.cover_address("f-al".into(), 300), "https://fake.example/cover/f-al/300");
    }

    /// A cover that is an address already is filled with the smallest size its image server keeps ready
    /// at least each size nori fetches: a tile's, a page head's and its stand-in, and their downloads alike.
    #[test]
    fn covers_take_the_sizes_the_image_server_keeps_ready() {
        let (c, _) = fake();
        let art = "https://img.example/a=w{size}-h{size}";
        let at = |px: u32| format!("https://img.example/a=w{px}-h{px}");
        assert_eq!(c.core.cover_addresses(art, 160), (at(544), Vec::new()));
        assert_eq!(c.core.cover_addresses(art, 1080), (at(1200), vec![at(544)]));
        assert_eq!(c.core.download_cover_urls(vec![art.into()]), [at(544), at(1200)]);
    }

    /// Signing in is the provider's to judge; signed in, its rules give hearts, and a heart goes to it.
    #[test]
    fn signing_in_goes_through_the_provider() {
        let (c, provider) = fake();
        let heart = Write::Star { kind: Starrable::Song, id: "f-1".into(), on: true };
        assert!(block(c.write(heart.clone())).is_err(), "signed out: nowhere to go");
        assert_eq!(block(c.sign_in("guess".into())), SignInResult::Refused);
        assert_eq!(c.account(), AccountState::SignedOut);
        assert_eq!(block(c.sign_in("let me in".into())), SignInResult::SignedIn { name: "Fay".into() });
        assert_eq!(c.account(), AccountState::SignedIn);
        assert!(c.core.rules().stars);
        block(c.write(heart)).unwrap();
        assert_eq!(*provider.hearts.lock(), ["f-1"]);
        assert!(!c.core.pending_any().unwrap(), "nothing queued for a server");
    }

    fn star(c: &Client, kind: Starrable, id: &str, on: bool) -> NetResult<()> {
        block(c.star(kind, id.into(), on, Arc::new(crate::providers::harness::Marked)))
    }

    fn favorites(c: &Client) -> Vec<String> {
        let Ok(Page::StarredPage { v }) = block(c.read_now(Read::StarredItems)) else { panic!("favorites") };
        v.albums.iter().map(|a| format!("album {}", a.id)).chain(v.songs.iter().map(|s| format!("{} {}", s.id, s.title))).collect()
    }

    /// Signed out, a seal keeps the item on this device: its record, as the page showed it, is in the
    /// favorites in this session and the next, and nothing is asked of the source, queued or played.
    #[test]
    fn signed_out_a_seal_is_kept_here() {
        let dir = nori_testdir::TempDir::new("fake-favorites-here");
        let db = dir.join("nori.db").to_str().unwrap().to_string();
        let (c, fake) = fake_at(db.clone());
        assert!(c.core.rules().stars && c.core.rules().stars_here);
        block(c.cached_or_fetched(Read::AlbumById { id: "f-al".into() })).unwrap();
        star(&c, Starrable::Song, "f-2", true).unwrap();
        star(&c, Starrable::Album, "f-al", true).unwrap();
        assert_eq!(*fake.albums_read.lock(), 1, "the record shown was kept: nothing asked again");
        assert!(fake.hearts.lock().is_empty() && !c.core.pending_any().unwrap(), "nothing sent or queued for the source");
        assert!(c.session().playlist(|p| p.ids().is_empty()) && c.precache_targets(false).is_empty(), "nothing queued or fetched");
        assert_eq!(favorites(&c), ["album f-al", "f-2 Two"]);
        assert!(c.core.starred(Starrable::Song, "f-2", false));
        drop(c);

        let (c, _) = fake_at(db);
        assert!(c.core.starred(Starrable::Album, "f-al", false), "kept across sessions");
        assert_eq!(favorites(&c), ["album f-al", "f-2 Two"]);
        star(&c, Starrable::Song, "f-2", false).unwrap();
        assert_eq!(favorites(&c), ["album f-al"]);
        assert!(!c.core.starred(Starrable::Song, "f-2", false));
    }

    /// An item this session didn't show is kept as the source's page of it has it; one the source
    /// doesn't have can't be sealed, and its mark goes back.
    #[test]
    fn a_seal_reads_the_record_it_lacks() {
        let (c, fake) = fake();
        star(&c, Starrable::Album, "f-al", true).unwrap();
        assert_eq!((*fake.albums_read.lock(), favorites(&c)), (1, vec!["album f-al".to_string()]));
        assert!(star(&c, Starrable::Song, "nowhere", true).is_err());
        assert!(!c.core.starred(Starrable::Song, "nowhere", false), "the mark went back");
    }

    /// Signed in, a seal goes to the account and the favorites are the account's with those kept here
    /// before after them; one kept here goes from here, and from the account only when it lists it.
    #[test]
    fn signed_in_seals_go_to_the_account_beside_those_kept_here() {
        let (c, fake) = fake();
        block(c.cached_or_fetched(Read::AlbumById { id: "f-al".into() })).unwrap();
        star(&c, Starrable::Song, "f-1", true).unwrap();
        star(&c, Starrable::Song, "f-2", true).unwrap();
        block(c.sign_in("let me in".into()));
        assert!(c.core.rules().stars && !c.core.rules().stars_here);
        star(&c, Starrable::Song, "f-2", true).unwrap();
        assert_eq!(*fake.hearts.lock(), ["f-2"], "the account's");
        assert_eq!(favorites(&c), ["f-2 Two", "f-1 One"], "the account's first, each once");
        star(&c, Starrable::Song, "f-1", false).unwrap();
        star(&c, Starrable::Song, "f-2", false).unwrap();
        assert!(fake.hearts.lock().is_empty(), "the account listed it");
        assert!(favorites(&c).is_empty());
    }

    /// Signed in, songs can be added to the account's own playlists, each knowing which it holds; the
    /// ones it follows, and signed out, none.
    #[test]
    fn songs_go_to_the_accounts_own_playlists() {
        let (c, _) = fake();
        let choices = |c: &Client| -> Vec<(String, Vec<String>, u32)> { block(c.playlist_choices(vec!["f-1".into(), "f-2".into()])).unwrap().into_iter().map(|p| (p.playlist.id, p.adds, p.holds)).collect() };
        assert!(choices(&c).is_empty(), "signed out");
        block(c.sign_in("let me in".into()));
        assert_eq!(choices(&c), [("f-own".to_string(), vec!["f-2".to_string()], 1)]);
    }

    /// Signed in, a seal the account has nowhere for (an album's) is kept on this device, and goes
    /// from here; nothing of it is sent.
    #[test]
    fn signed_in_a_seal_the_account_cant_keep_is_kept_here() {
        let (c, fake) = fake();
        block(c.sign_in("let me in".into()));
        star(&c, Starrable::Song, "f-2", true).unwrap();
        star(&c, Starrable::Album, "f-al", true).unwrap();
        assert_eq!(*fake.hearts.lock(), ["f-2"], "the album went nowhere");
        assert!(c.core.starred(Starrable::Album, "f-al", false));
        assert_eq!(favorites(&c), ["album f-al", "f-2 Two"]);
        star(&c, Starrable::Album, "f-al", false).unwrap();
        assert_eq!(favorites(&c), ["f-2 Two"]);
    }
}
