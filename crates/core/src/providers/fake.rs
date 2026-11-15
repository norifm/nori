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
}

/// Signed out, its favorites are kept on this device.
fn rules(signed_in: bool) -> ProfileRules {
    ProfileRules { account: false, indexed: false, remote: false, stars: true, stars_here: !signed_in, scrobbles: signed_in, feed: true, mixes: signed_in, ..profile_rules(false) }
}

fn album() -> AlbumDetail {
    let song = |id: &str, title: &str| Song { id: id.into(), title: title.into(), album: "Fakes".into(), album_id: Some("f-al".into()), duration: 60, ..Default::default() }.dressed();
    AlbumDetail::new(Album { id: "f-al".into(), name: "Fakes".into(), song_count: 2, ..Default::default() }, vec![song("f-1", "One"), song("f-2", "Two")], Vec::new())
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
        Ok(match read {
            Read::StarredItems => {
                let hearts = self.hearts.lock();
                Some(Page::StarredPage { v: crate::Starred::new(Vec::new(), Vec::new(), album().songs.into_iter().filter(|s| hearts.contains(&s.id)).collect()) })
            }
            Read::Search { query, .. } => Some(Page::Found { v: SearchResult { songs: album().songs.into_iter().filter(|s| s.title.to_lowercase().contains(&query)).collect(), ..Default::default() } }),
            Read::AlbumById { id } if id == "f-al" => {
                *self.albums_read.lock() += 1;
                Some(Page::AlbumPage { v: album() })
            }
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
