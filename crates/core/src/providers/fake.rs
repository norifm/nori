//! A provider of one album and an account behind a password, standing in for a third kind: the core's
//! generic paths (search, pages, play, sign-in) work through it and name no provider.

use std::sync::Arc;

use parking_lot::Mutex;

use super::{AccountState, FieldSpec, FormField, Provider, ProviderInfo, ResolveStream, SignIn, SignInResult};
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
}

fn rules(signed_in: bool) -> ProfileRules {
    ProfileRules { account: false, indexed: false, remote: false, stars: signed_in, feed: true, ..profile_rules(false) }
}

fn album() -> AlbumDetail {
    let song = |id: &str, title: &str| Song { id: id.into(), title: title.into(), album: "Fakes".into(), album_id: Some("f-al".into()), duration: 60, ..Default::default() }.dressed();
    AlbumDetail::new(Album { id: "f-al".into(), name: "Fakes".into(), song_count: 2, ..Default::default() }, vec![song("f-1", "One"), song("f-2", "Two")], Vec::new())
}

#[async_trait::async_trait]
impl Provider for Fake {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { kind: ServerKind::default(), fields: vec![FieldSpec { what: FormField::Name, advanced: false }], sign_in: vec![SignIn::Paste] }
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

    async fn read(&self, _: &Client, read: Read) -> NetResult<Option<Page>> {
        Ok(match read {
            Read::Search { query, .. } => Some(Page::Found { v: SearchResult { songs: album().songs.into_iter().filter(|s| s.title.to_lowercase().contains(&query)).collect(), ..Default::default() } }),
            Read::AlbumById { id } if id == "f-al" => Some(Page::AlbumPage { v: album() }),
            _ => None,
        })
    }

    async fn write(&self, _: &Client, w: Write) -> NetResult<()> {
        match w {
            Write::Star { kind: Starrable::Song, id, on: true } if self.signed_in.lock().is_some() => {
                self.hearts.lock().push(id);
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
        if self.signed_in.lock().is_some() { AccountState::SignedIn } else { AccountState::SignedOut }
    }
}

struct Opens;

impl ResolveStream for Opens {
    fn resolve(&self, address: &str, from: u64) -> Option<NetResult<String>> {
        address.strip_prefix("fake:").map(|id| Ok(format!("https://fake.example/{id}?from={from}")))
    }

    fn refused(&self, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, Fake as Wire};

    /// A client over the fake provider, its network a scripted transport no test answers.
    fn fake() -> (Arc<Client>, Arc<Fake>) {
        let core = Core::new(String::new(), "fake".into(), Default::default()).unwrap();
        let provider = Arc::new(Fake::default());
        core.install(ServerKind::default(), provider.clone());
        core.provider().configure(&core, &SavedServer::default()).unwrap();
        (Client::new(core, Arc::new(Wire::default()), Default::default()), provider)
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
        assert!(c.core.rules().feed && !c.core.rules().remote);
    }

    /// A song plays from the provider's address, opened at the URL it resolves to; covers are its own.
    #[test]
    fn songs_play_from_the_providers_addresses() {
        let (c, _) = fake();
        let t = c.resolve("f-1".into(), false, false);
        assert_eq!((t.url.as_str(), t.key.as_str()), ("fake:f-1", "f-1:0"));
        let opens = c.stream_resolver().unwrap();
        assert_eq!(opens.resolve(&t.url, 7).unwrap().unwrap(), "https://fake.example/f-1?from=7");
        c.session().register(album().songs);
        c.session().set(vec!["f-1".into(), "f-2".into()], Some(0), false, None);
        assert!(c.precache_targets(false).is_empty(), "nothing past the next song");
        assert_eq!(c.core.cover_address("f-al".into(), 300), "https://fake.example/cover/f-al/300");
        assert_eq!(c.core.cover_prefix(), None);
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
}
