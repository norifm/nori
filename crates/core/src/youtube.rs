//! A YouTube Music profile: innertube-music's answers as nori's records under `yt-` ids, its reads as
//! the pages' `Page`s, its likes as hearts, its stream addresses and the URLs they open at. Best effort
//! and server-less; signed in or not (account.rs) (docs/research/youtube-music.md).

mod account;

pub use account::{youtube_sign_in_done, youtube_sign_in_page, YouTubeAccount, YouTubeSignIn};

use std::collections::HashMap;
use std::time::{Duration, Instant};

use innertube_music as yt;

use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Starrable, Write};
use crate::stream::StreamTarget;
use crate::transport::{block_on, Exchange, NetError, Transport};
use crate::{Album, AlbumDetail, Artist, ArtistDetail, ArtistInfo, ArtistRef, FeedShelf, Playlist, PlaylistDetail, SearchResult, Song, Starred};

/// What a YouTube Music profile's library is kept under (a server's is its address and user).
pub(crate) const IDENT: &str = "youtube-music";

/// Prefix of every YouTube id in nori (songs, albums, artists, playlists): the stream store and cover
/// cache are shared by all profiles.
const ID_PREFIX: &str = "yt-";

/// Scheme of a song's stream address until it is resolved.
const ADDRESS: &str = "youtube:";

/// Songs fetched ahead of the one playing: each costs a player request and a whole download.
pub(crate) const AHEAD: usize = 1;

/// nori's id for a YouTube id.
fn id(raw: &str) -> String {
    format!("{ID_PREFIX}{raw}")
}

/// YouTube's id inside nori's `id`; None when it is not a YouTube id.
fn raw_id(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX).filter(|r| !r.is_empty())
}

/// The YouTube id inside nori's `id`, or a parse error naming it.
fn raw(id: &str) -> NetResult<&str> {
    raw_id(id).ok_or_else(|| NetError::Parse { reason: format!("{id} is no YouTube id") })
}

/// The video a stream address names; None for any other URL.
fn address_video(address: &str) -> Option<&str> {
    address.strip_prefix(ADDRESS).filter(|v| !v.is_empty())
}

/// Every Subsonic call of a YouTube Music profile: there is no server to send it to. Not an I/O failure,
/// so writes are dropped, not queued.
pub(crate) fn no_subsonic(endpoint: &str) -> NetError {
    NetError::Api { code: 0, reason: format!("YouTube Music has no {endpoint}") }
}

/// Where song `id` streams from and the key its bytes are kept under. The quality settings do not
/// apply: YouTube's file is played as it is.
pub(crate) fn stream_target(id: String) -> StreamTarget {
    let url = raw_id(&id).map(|v| format!("{ADDRESS}{v}")).unwrap_or_default();
    StreamTarget { url, key: format!("{id}:0") }
}

/// The URL of thumbnail `cover` at `size` px.
pub(crate) fn cover_address(cover: &str, size: u32) -> String {
    yt::thumbnail_url(cover, size)
}

/// The platform's transport as innertube-music's HTTP client.
struct Wire<'a>(&'a dyn Transport);

impl yt::Http for Wire<'_> {
    type Error = NetError;

    fn send(&self, r: yt::Request) -> impl std::future::Future<Output = Result<yt::Response, NetError>> + Send {
        let exchange = Exchange { url: r.url, headers: r.headers.into_iter().collect(), json: r.body, timeout_ms: 0 };
        async move {
            let a = self.0.send(exchange).await?;
            Ok(yt::Response { status: a.status, body: a.body, set_cookies: a.set_cookies })
        }
    }
}

fn net_error(e: yt::Error<NetError>) -> NetError {
    match e {
        yt::Error::Transport(e) => e,
        yt::Error::Status(status) => NetError::Http { status },
        yt::Error::Parse(reason) => NetError::Parse { reason },
        yt::Error::Unplayable(reason) => NetError::Api { code: 0, reason: format!("YouTube will not play it: {reason}") },
        yt::Error::SignedOut => NetError::Api { code: 0, reason: "signed out of YouTube Music".into() },
    }
}

fn artist_refs(links: &[yt::ArtistLink]) -> Vec<ArtistRef> {
    links.iter().map(|a| ArtistRef { id: id(&a.id), name: a.name.clone() }).collect()
}

fn song(s: yt::Song) -> Song {
    let artists = artist_refs(&s.artists);
    let (album, album_id) = s.album.map_or((String::new(), None), |a| (a.name, Some(id(&a.id))));
    Song {
        id: id(&s.video_id),
        title: s.title,
        artist: s.artist,
        artist_id: artists.first().map(|a| a.id.clone()),
        artists,
        album,
        album_id,
        duration: s.duration,
        track: s.track,
        year: s.year,
        cover_art: s.thumbnail,
        ..Default::default()
    }
    .dressed()
}

fn album(a: yt::Album) -> Album {
    Album {
        id: id(&a.id),
        subtitle: nori_model::lines::album_subtitle(&a.artist, a.year),
        artist_id: a.artists.first().map(|r| id(&r.id)),
        name: a.title,
        artist: a.artist,
        year: a.year,
        release_types: a.release_type.into_iter().collect(),
        cover_art: a.thumbnail,
        ..Default::default()
    }
}

fn artist(a: yt::Artist) -> Artist {
    Artist { id: id(&a.id), name: a.name, cover_art: a.thumbnail, ..Default::default() }
}

fn found(r: yt::SearchResults, songs: usize, albums: usize, artists: usize) -> SearchResult {
    SearchResult {
        songs: r.songs.into_iter().take(songs).map(song).collect(),
        albums: r.albums.into_iter().take(albums).map(album).collect(),
        artists: r.artists.into_iter().take(artists).map(artist).collect(),
    }
}

fn album_detail(p: yt::AlbumPage) -> AlbumDetail {
    let songs: Vec<Song> = p.songs.into_iter().map(song).collect();
    let mut a = album(p.album);
    a.song_count = songs.len() as u32;
    a.duration = songs.iter().map(|s| s.duration).sum();
    AlbumDetail::new(a, songs, Vec::new())
}

/// The artist's albums and the page's about part.
fn artist_detail(p: yt::ArtistPage) -> (ArtistDetail, ArtistInfo) {
    let about = ArtistInfo {
        biography: p.description,
        image_url: p.artist.thumbnail.as_deref().map(|c| cover_address(c, 800)),
        similar: p.similar.into_iter().map(artist).collect(),
        ..Default::default()
    };
    let albums: Vec<Album> = p.albums.into_iter().map(album).collect();
    let mut a = artist(p.artist);
    a.album_count = albums.len() as u32;
    (ArtistDetail::new(a, albums), about)
}

/// Playlist `l`, of which `songs` are shown.
fn playlist(l: yt::Playlist, songs: &[Song]) -> Playlist {
    Playlist {
        id: id(&l.id),
        name: l.title,
        comment: l.description,
        owner: l.author,
        public: true,
        song_count: l.song_count.unwrap_or(songs.len() as u32),
        duration: songs.iter().map(|s| s.duration).sum(),
        cover_art: l.thumbnail,
    }
}

fn playlist_detail(p: yt::PlaylistPage) -> PlaylistDetail {
    let songs: Vec<Song> = p.songs.into_iter().map(song).collect();
    PlaylistDetail::new(playlist(p.playlist, &songs), songs)
}

/// A feed shelf, its items sorted by kind.
fn shelf(s: yt::Shelf) -> FeedShelf {
    let mut f = FeedShelf { title: s.title, ..Default::default() };
    for item in s.items {
        match item {
            yt::Item::Song(s) => f.songs.push(song(s)),
            yt::Item::Album(a) => f.albums.push(album(a)),
            yt::Item::Artist(a) => f.artists.push(artist(a)),
            yt::Item::Playlist(p) => f.playlists.push(playlist(p, &[])),
        }
    }
    f
}

/// A stream is asked again this long before YouTube stops serving it, so a song opened just before
/// still loads whole.
const MARGIN: Duration = Duration::from_secs(5 * 60);

/// The streams resolved so far, by video, until they expire.
#[derive(Default)]
struct Streams(parking_lot::Mutex<HashMap<String, (yt::Stream, Instant)>>);

impl Streams {
    /// `video`'s stream while it is still served at `now`.
    fn get(&self, video: &str, now: Instant) -> Option<yt::Stream> {
        self.0.lock().get(video).filter(|(_, until)| now < *until).map(|(s, _)| s.clone())
    }

    /// Keeps `stream` for `video`, resolved at `now`; drops those expired.
    fn keep(&self, video: &str, stream: yt::Stream, now: Instant) {
        let until = now + stream.expires_in.saturating_sub(MARGIN);
        let mut m = self.0.lock();
        m.retain(|_, (_, u)| now < *u);
        m.insert(video.to_string(), (stream, until));
    }

    /// YouTube refused `video`'s URL: it is asked again on the next open.
    fn forget(&self, video: &str) {
        self.0.lock().remove(video);
    }
}

/// What a profile's requests share: innertube-music's session (the visitor id, the account's cookies),
/// the session as kept, and the streams resolved so far.
#[derive(Default)]
pub(crate) struct YouTube {
    api: yt::InnerTube,
    session: account::Session,
    streams: Streams,
}

impl Client {
    /// `read` answered by YouTube Music; reads it has nothing for get the empty page. Signed in, the
    /// library's reads are the account's: its saved albums (one page), its playlists and its liked songs
    /// as the favorites.
    pub(crate) async fn youtube_read(&self, read: Read) -> NetResult<Page> {
        let (api, t) = (&self.innertube.api, &Wire(&*self.transport));
        let n = |count: i32| count.max(0) as usize;
        self.youtube_fresh_session(account::unix_now()).await;
        let signed_in = self.youtube_signed_in();
        Ok(match read {
            Read::Search { query, songs, albums, artists } => Page::Found { v: found(self.youtube_after(api.search(t, &query).await)?, n(songs), n(albums), n(artists)) },
            Read::AlbumById { id } => Page::AlbumPage { v: album_detail(self.youtube_after(api.album(t, raw(&id)?).await)?) },
            Read::AlbumSongs { id } => Page::Songs { v: album_detail(self.youtube_after(api.album(t, raw(&id)?).await)?).songs },
            Read::ArtistById { id } => Page::ArtistPage { v: artist_detail(self.youtube_after(api.artist(t, raw(&id)?).await)?).0 },
            Read::ArtistAbout { id } => Page::About { v: artist_detail(self.youtube_after(api.artist(t, raw(&id)?).await)?).1 },
            Read::PlaylistById { id } => Page::PlaylistPage { v: playlist_detail(self.youtube_after(api.playlist(t, raw(&id)?).await)?) },
            Read::PlaylistSongs { id } => Page::Songs { v: playlist_detail(self.youtube_after(api.playlist(t, raw(&id)?).await)?).songs },
            Read::HomeFeed => Page::Feed { v: self.youtube_after(api.home(t).await)?.into_iter().map(shelf).collect() },
            Read::PlaylistList if signed_in => Page::Playlists { v: self.youtube_after(api.library_playlists(t).await)?.into_iter().map(|p| playlist(p, &[])).collect() },
            Read::AlbumList { offset: 0, .. } if signed_in => Page::Albums { v: self.youtube_after(api.library_albums(t).await)?.into_iter().map(album).collect() },
            Read::StarredItems if signed_in => {
                let liked = self.youtube_after(api.liked_songs(t).await)?.into_iter().map(|s| Song { starred: true, ..song(s) }).collect();
                Page::StarredPage { v: self.core.stars.lock().overlay(Starred::new(Vec::new(), Vec::new(), liked)) }
            }
            other => return self.nothing(other),
        })
    }

    /// `w` for YouTube Music: a song's heart is the account's like; nothing else has anywhere to go.
    /// Never queued: a like that fails is taken back.
    pub(crate) async fn youtube_write(&self, w: Write) -> NetResult<()> {
        match w {
            Write::Star { kind: Starrable::Song, id, on } if self.youtube_signed_in() => {
                self.youtube_fresh_session(account::unix_now()).await;
                self.youtube_after(self.innertube.api.like(&Wire(&*self.transport), raw(&id)?, on).await)
            }
            _ => Err(no_subsonic("write")),
        }
    }

    /// The URL to open stream `address` at from byte `from`; None when it is no YouTube song's (a URL
    /// already). Asks YouTube for the stream unless one is kept, blocking the caller (a loader's thread,
    /// never the engine's).
    pub fn playable(&self, address: &str, from: u64) -> Option<NetResult<String>> {
        let video = address_video(address).filter(|_| self.youtube())?;
        let now = Instant::now();
        let streams = &self.innertube.streams;
        let stream = match streams.get(video, now) {
            Some(s) => s,
            None => match block_on(self.innertube.api.player(&Wire(&*self.transport), video)) {
                Ok(s) => {
                    crate::alog::info(&format!("YouTube stream of {video}: itag {}, {}", s.format.itag, s.format.mime));
                    streams.keep(video, s.clone(), now);
                    s
                }
                Err(e) => return Some(Err(net_error(e))),
            },
        };
        Some(Ok(stream.url_from(from)))
    }

    /// YouTube refused the URL `address` was opened at: the next open asks for the stream again.
    pub fn playable_refused(&self, address: &str) {
        if let Some(video) = address_video(address) {
            self.innertube.streams.forget(video);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, client};
    use crate::client::NetProfile;
    use crate::transport::{FailureKind, Network, TransportError, TransportResponse};
    use std::sync::Arc;

    /// Answers InnerTube calls with queued answers and keeps what was asked; with none left the network
    /// is down. A GET (Subsonic) is a test failure.
    #[derive(Default)]
    struct InnerTube {
        answers: parking_lot::Mutex<Vec<TransportResponse>>,
        asked: parking_lot::Mutex<Vec<Exchange>>,
    }

    impl InnerTube {
        fn urls(&self) -> Vec<String> {
            self.asked.lock().iter().map(|e| e.url.split('?').next().unwrap_or_default().to_string()).collect()
        }
    }

    #[async_trait::async_trait]
    impl Transport for InnerTube {
        async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
            panic!("a Subsonic request in a YouTube Music profile: {url}");
        }
        async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
            self.asked.lock().push(request);
            self.answers.lock().pop().ok_or(TransportError::Failed { kind: FailureKind::Connect, detail: None })
        }
        fn address_changed(&self) {}
        fn network(&self) -> Network {
            Network::Unmetered
        }
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/../youtube/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// An answer of `status` with fixture `body` (or `{}`) and `set_cookies`.
    fn answer(status: u16, body: &str, set_cookies: &[&str]) -> TransportResponse {
        let body = if body.is_empty() { "{}".to_string() } else { fixture(body) };
        TransportResponse { status, body: body.into_bytes(), set_cookies: set_cookies.iter().map(|c| c.to_string()).collect() }
    }

    fn youtube_answering(answers: Vec<TransportResponse>) -> (Arc<Client>, Arc<InnerTube>) {
        let core = crate::Core::new(String::new(), "yt".into(), Default::default()).unwrap();
        core.configure_youtube_music(false).unwrap();
        let t = Arc::new(InnerTube { answers: parking_lot::Mutex::new(answers.into_iter().rev().collect()), ..Default::default() });
        (Client::new(core, t.clone(), Default::default()), t)
    }

    fn youtube(answers: &[&str]) -> (Arc<Client>, Arc<InnerTube>) {
        youtube_answering(answers.iter().map(|a| TransportResponse { status: 200, body: a.as_bytes().to_vec(), set_cookies: Vec::new() }).collect())
    }

    const SESSION: &str = "SAPISID=sapi; __Secure-3PAPISID=sapi3; __Secure-1PSIDTS=ts1; SID=sid";

    /// A profile signed in with [`SESSION`], last rotated `ago` seconds before now.
    fn signed_in(ago: u64, answers: Vec<TransportResponse>) -> (Arc<Client>, Arc<InnerTube>) {
        let (c, t) = youtube_answering(answers);
        account::tests::store(&c, &account::tests::session(SESSION, ago));
        (c, t)
    }

    const ROTATE: &str = "https://accounts.youtube.com/RotateCookies";
    const BROWSE: &str = "https://music.youtube.com/youtubei/v1/browse";

    #[test]
    fn ids_and_addresses() {
        assert_eq!(id("dQw4w9WgXcQ"), "yt-dQw4w9WgXcQ");
        assert_eq!(raw_id("yt-MPREb_x"), Some("MPREb_x"));
        assert_eq!(raw_id("ext-deezer-1"), None);
        assert_eq!(raw_id("yt-"), None);
        assert_eq!(address_video(&stream_target("yt-dQw4w9WgXcQ".into()).url), Some("dQw4w9WgXcQ"));
        assert_eq!(stream_target("al-1".into()).url, "");
        assert_eq!(address_video("https://nas/rest/stream?id=1"), None);
    }

    /// YouTube's records become nori's under `yt-` ids, with the subtitle, counts and lengths nori
    /// derives; a song is no provider's, so it is indexed.
    #[test]
    fn records_are_noris_under_yt_ids() {
        let s = song(yt::Song { video_id: "v".into(), title: "T".into(), artist: "A & B".into(), artists: vec![yt::ArtistLink { id: "UCa".into(), name: "A".into() }, yt::ArtistLink { id: "UCb".into(), name: "B".into() }], album: Some(yt::AlbumLink { id: "MPREb_x".into(), name: "X".into() }), duration: 10, ..Default::default() });
        assert_eq!((s.id.as_str(), s.artist_id.as_deref(), s.album_id.as_deref(), s.album.as_str()), ("yt-v", Some("yt-UCa"), Some("yt-MPREb_x"), "X"));
        assert_eq!(s.artists.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["yt-UCa", "yt-UCb"]);
        assert!(!s.is_provider() && !s.line.is_empty());
        let a = album(yt::Album { id: "MPREb_x".into(), title: "X".into(), artist: "A".into(), artists: vec![yt::ArtistLink { id: "UCa".into(), name: "A".into() }], year: 2013, release_type: Some("Album".into()), thumbnail: None });
        assert_eq!((a.id.as_str(), a.artist_id.as_deref(), a.subtitle.as_str(), a.release_types.as_slice()), ("yt-MPREb_x", Some("yt-UCa"), "A · 2013", &["Album".to_string()][..]));
        let d = playlist_detail(yt::PlaylistPage { playlist: yt::Playlist { id: "VLp".into(), ..Default::default() }, songs: vec![yt::Song { video_id: "v".into(), duration: 7, ..Default::default() }] });
        assert_eq!((d.playlist.id.as_str(), d.playlist.song_count, d.playlist.duration, d.playlist.public), ("yt-VLp", 1, 7, true));
    }

    /// A search keeps at most as many of each kind as asked.
    #[test]
    fn a_search_is_cut_to_the_counts_asked() {
        let (s, al, ar) = (fixture("search_songs"), fixture("search_albums"), fixture("search_artists"));
        let (c, _) = youtube(&[&s, &al, &ar]);
        let Page::Found { v } = block(c.read_now(Read::Search { query: "daft punk".into(), songs: 2, albums: 10, artists: 1 })).unwrap() else { panic!() };
        assert_eq!(v.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Robot Rock (Edit)", "Da Funk"]);
        assert_eq!(v.albums.len(), 3);
        assert_eq!(v.artists.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["yt-UCRr1xG_2WIDs18a6cIiCxeA"]);
    }

    /// Pages come from InnerTube, are indexed for local search, and are never served from the cache.
    #[test]
    fn reads_go_to_innertube_and_are_indexed() {
        let album = fixture("album");
        let (c, t) = youtube(&[&album, &album]);
        let Page::AlbumPage { v } = block(c.read_now(Read::AlbumById { id: "yt-MPREb_K8qWMWVqXGi".into() })).unwrap() else { panic!() };
        assert_eq!((v.album.name.as_str(), v.songs.len(), v.album.song_count, v.album.duration), ("Random Access Memories", 3, 3, 276 + 323 + 545));
        assert_eq!(c.core.local_search("giorgio".into(), 10).unwrap().songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["yt-zhl-Cs1-sG4"]);
        assert!(c.read_stored(Read::AlbumById { id: "yt-MPREb_K8qWMWVqXGi".into() }).unwrap().page.is_none());
        let again = block(c.cached_or_fetched(Read::AlbumById { id: "yt-MPREb_K8qWMWVqXGi".into() })).unwrap();
        assert!(matches!(again, Some(Page::AlbumPage { .. })));
        assert_eq!(t.asked.lock().len(), 2);
        assert!(matches!(block(c.read_now(Read::AlbumById { id: "al-1".into() })), Err(NetError::Parse { .. })), "not a YouTube id: nothing asked");
        assert_eq!(t.asked.lock().len(), 2);
    }

    /// The artist page's about part: its biography, its picture at 800 px and similar artists.
    #[test]
    fn an_artists_about_comes_from_its_page() {
        let (c, _) = youtube(&[&fixture("artist")]);
        let Page::About { v } = block(c.read_now(Read::ArtistAbout { id: "yt-UCRr1xG_2WIDs18a6cIiCxeA".into() })).unwrap() else { panic!() };
        assert!(v.biography.as_deref().is_some_and(|b| b.starts_with("Daft Punk were")));
        assert!(v.image_url.as_deref().is_some_and(|u| u.ends_with("=w800-h800-l90-rj")));
        assert!(v.similar.len() == 3 && v.similar.iter().all(|a| a.id.starts_with("yt-UC")));
    }

    /// Reads YouTube has nothing for answer empty pages without a request; writes fail at once, unqueued.
    #[test]
    fn subsonic_reads_and_writes_go_nowhere() {
        let (c, t) = youtube(&[]);
        assert!(matches!(block(c.read_now(Read::GenreList)), Ok(Page::Genres { v }) if v.is_empty()));
        assert!(matches!(block(c.read_now(Read::RandomSongs { size: 10, genre: None })), Ok(Page::Songs { v }) if v.is_empty()));
        assert!(matches!(block(c.read_now(Read::LyricsBySong { song_id: "yt-x".into() })), Ok(Page::LyricsPage { .. })));
        let star = block(c.write(crate::client::Write::Star { kind: crate::client::Starrable::Song, id: "yt-x".into(), on: true }));
        assert!(matches!(star, Err(NetError::Api { .. })), "{star:?}");
        assert!(!c.core.pending_any().unwrap(), "nothing queued for later");
        assert!(t.asked.lock().is_empty());
        let rules = c.core.rules();
        assert!(!rules.account && !rules.remote && rules.measures);
    }

    /// A song streams from its YouTube address under a key that ignores the quality settings; nothing
    /// past the next song is fetched ahead.
    #[test]
    fn songs_stream_from_youtube_addresses() {
        let (c, _) = youtube(&[]);
        let t = c.resolve("yt-dQw4w9WgXcQ".into(), false, false);
        assert_eq!((t.url.as_str(), t.key.as_str()), ("youtube:dQw4w9WgXcQ", "yt-dQw4w9WgXcQ:0"));
        let ids: Vec<String> = ["yt-a", "yt-b", "yt-c", "yt-d"].map(String::from).to_vec();
        let songs = ids.iter().map(|id| crate::Song { id: id.clone(), ..Default::default() }).collect();
        c.session().register(songs);
        c.session().set(ids, Some(0), false, None);
        assert_eq!(c.session().precache(false), ["yt-c"], "the settings would fetch the song after next");
        assert!(c.precache_targets(false).is_empty());
        assert_eq!(c.core.cover_address("https://yt3.googleusercontent.com/abc".into(), 320), "https://yt3.googleusercontent.com/abc=w320-h320-l90-rj");
    }

    /// A stream is asked once and kept; refused, it is asked again. The first open asks the whole file
    /// as a range, later ones come plain (with the byte source's `Range`).
    #[test]
    fn a_stream_is_resolved_once_until_refused() {
        let player = fixture("player");
        let (c, t) = youtube(&[&player, &player]);
        let first = c.playable("youtube:dQw4w9WgXcQ", 0).unwrap().unwrap();
        assert!(first.contains("itag=251") && first.ends_with("&range=0-3433754"), "{first}");
        let later = c.playable("youtube:dQw4w9WgXcQ", 2_000_000).unwrap().unwrap();
        assert!(!later.contains("&range="));
        assert_eq!(t.asked.lock().len(), 1);
        c.playable_refused("youtube:dQw4w9WgXcQ");
        assert!(c.playable("youtube:dQw4w9WgXcQ", 0).unwrap().is_ok());
        assert_eq!(t.asked.lock().len(), 2);
        assert!(c.playable("https://nas/rest/stream?id=1", 0).is_none(), "a server's URL is opened as it is");
        assert!(c.playable("youtube:gone", 0).unwrap().is_err(), "a failed request fails the open");
    }

    /// An unplayable video's error carries YouTube's reason.
    #[test]
    fn an_unplayable_video_says_why() {
        let (c, _) = youtube(&[&fixture("unplayable")]);
        match c.playable("youtube:x", 0) {
            Some(Err(NetError::Api { reason, .. })) => assert_eq!(reason, "YouTube will not play it: This video is unavailable"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stream_is_kept_until_shortly_before_it_expires() {
        let s = yt::Stream { format: yt::Format { itag: 251, url: "u".into(), mime: "audio/webm".into(), bit_rate: 1, length: None }, expires_in: Duration::from_secs(21_540) };
        let streams = Streams::default();
        let t0 = Instant::now();
        streams.keep("v", s.clone(), t0);
        assert_eq!(streams.get("v", t0 + Duration::from_secs(21_540) - MARGIN - Duration::from_secs(1)), Some(s.clone()));
        assert_eq!(streams.get("v", t0 + Duration::from_secs(21_540) - MARGIN), None);
        assert_eq!(streams.get("w", t0), None);
        streams.forget("v");
        assert_eq!(streams.get("v", t0), None);
        // Expired ones go when another is kept.
        streams.keep("v", s.clone(), t0);
        streams.keep("w", s, t0 + Duration::from_secs(30_000));
        assert_eq!(streams.0.lock().len(), 1);
    }

    /// A Subsonic profile keeps its server: nothing of this applies to it.
    #[test]
    fn a_subsonic_profile_is_untouched() {
        let (c, _) = client(NetProfile { url: "h".into(), ..Default::default() });
        assert!(!c.youtube());
        assert!(c.playable("youtube:x", 0).is_none());
        assert!(c.resolve("yt-x".into(), false, false).url.contains("/rest/stream?"));
    }

    /// Signing in asks YouTube for the account's name and keeps the session, rotated cookies included;
    /// hearts and the library follow.
    #[test]
    fn signing_in_keeps_the_session() {
        let (c, t) = youtube_answering(vec![answer(200, "account_menu", &["SIDCC=cc; Domain=.youtube.com"])]);
        assert_eq!(c.youtube_account(), YouTubeAccount::SignedOut);
        assert_eq!(block(c.youtube_sign_in(format!("Cookie: {SESSION}"))), YouTubeSignIn::SignedIn { name: "Ann Example".into() });
        assert_eq!(t.asked.lock()[0].headers["Cookie"], SESSION);
        let kept = account::tests::stored(&c).unwrap();
        assert_eq!(kept.cookies(), format!("{SESSION}; SIDCC=cc"));
        assert!(kept.rotated_ago() < 5, "fresh from the browser");
        assert_eq!(c.youtube_account(), YouTubeAccount::SignedIn);
        assert!(c.core.rules().stars && !c.core.rules().sections.is_empty());
        c.youtube_sign_out();
        assert_eq!((c.youtube_account(), account::tests::stored(&c)), (YouTubeAccount::SignedOut, None));
        assert!(!c.core.rules().stars);
    }

    /// Text without a session's cookies asks nothing; cookies YouTube does not take change nothing.
    #[test]
    fn a_failed_sign_in_changes_nothing() {
        let (c, t) = youtube_answering(vec![answer(200, "signed_out", &[]), answer(429, "", &[]), answer(200, "signed_out", &[])]);
        assert_eq!(block(c.youtube_sign_in("VISITOR_INFO1_LIVE=v".into())), YouTubeSignIn::NoSession);
        assert!(t.asked.lock().is_empty());
        assert_eq!(block(c.youtube_sign_in(SESSION.into())), YouTubeSignIn::Refused);
        assert_eq!(t.urls()[1], ROTATE, "asked again after a rotation");
        assert!(matches!(block(c.youtube_sign_in(SESSION.into())), YouTubeSignIn::Unreachable { .. }), "the network is down");
        assert_eq!((c.youtube_account(), account::tests::stored(&c)), (YouTubeAccount::SignedOut, None));
    }

    /// A session last rotated ten minutes ago or more is rotated before the next call, and the new
    /// cookies are kept at once; a fresher one is not.
    #[test]
    fn the_session_is_rotated_before_it_goes_stale() {
        let (c, t) = signed_in(601, vec![answer(200, "", &["__Secure-1PSIDTS=ts2; Domain=.youtube.com; Secure"]), answer(200, "library_albums", &[])]);
        let Page::Albums { v } = block(c.read_now(Read::AlbumList { kind: crate::browse::AlbumSort::ByName, size: 60, offset: 0, genre: None })).unwrap() else { panic!() };
        assert_eq!(v.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["yt-MPREb_K8qWMWVqXGi", "yt-MPREb_NP5KiWDRCjf"]);
        assert_eq!(t.urls(), [ROTATE, BROWSE]);
        assert!(t.asked.lock()[1].headers["Cookie"].contains("__Secure-1PSIDTS=ts2"));
        let kept = account::tests::stored(&c).unwrap();
        assert!(kept.cookies().contains("ts2") && kept.rotated_ago() < 5);

        let (c, t) = signed_in(30, vec![answer(200, "library_playlists", &[])]);
        let Page::Playlists { v } = block(c.read_now(Read::PlaylistList)).unwrap() else { panic!() };
        assert_eq!(v.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["yt-VLLM", "yt-VLOLAK5uy_lNVBcjNtiCwq-n95uoxJ-Gd4vsfMkyZNs"]);
        assert_eq!(t.urls(), [BROWSE], "rotated 30 s ago: not again");
    }

    /// Cookies an answer changes are kept the moment it comes, not only a rotation's.
    #[test]
    fn rotated_cookies_from_any_answer_are_kept() {
        let (c, _) = signed_in(30, vec![answer(200, "home", &["__Secure-3PSIDTS=new; Domain=.youtube.com"]), answer(200, "home_more", &[])]);
        let Page::Feed { v } = block(c.read_now(Read::HomeFeed)).unwrap() else { panic!() };
        assert_eq!(v.len(), 4);
        assert!(account::tests::stored(&c).unwrap().cookies().ends_with("; __Secure-3PSIDTS=new"));
    }

    /// The network failing, a rotation, or a refusal set right by one, keeps the session.
    #[test]
    fn a_transient_failure_keeps_the_session() {
        let (c, t) = signed_in(601, vec![]);
        assert!(block(c.read_now(Read::PlaylistList)).is_err());
        assert_eq!(t.urls(), [ROTATE, BROWSE]);
        let kept = account::tests::stored(&c).unwrap();
        assert!(!kept.ended() && kept.rotated_ago() >= 601, "still due: asked again next time");
        assert_eq!(c.youtube_account(), YouTubeAccount::SignedIn);

        let (c, t) = signed_in(30, vec![answer(401, "", &[]), answer(200, "", &["__Secure-1PSIDTS=ts2"]), answer(200, "library_playlists", &[])]);
        assert!(matches!(block(c.read_now(Read::PlaylistList)), Ok(Page::Playlists { v }) if v.len() == 2));
        assert_eq!(t.urls(), [BROWSE, ROTATE, BROWSE], "one refusal: rotated and asked again");
        assert_eq!(c.youtube_account(), YouTubeAccount::SignedIn);
        assert!(account::tests::stored(&c).unwrap().cookies().contains("ts2"));
    }

    /// A rotation YouTube refuses ends the session: it says so, keeps the cookies for a look, and the
    /// profile goes on as a visitor's until signed in again.
    #[test]
    fn a_refused_session_ends_but_is_not_wiped() {
        let (c, t) = signed_in(601, vec![answer(401, "", &[]), answer(200, "home", &[]), answer(200, "home_more", &[])]);
        let Page::Feed { v } = block(c.read_now(Read::HomeFeed)).unwrap() else { panic!() };
        assert_eq!(v.len(), 4, "everyone's feed");
        assert_eq!(c.youtube_account(), YouTubeAccount::Ended);
        let kept = account::tests::stored(&c).unwrap();
        assert!(kept.ended() && kept.cookies() == SESSION);
        assert!(!t.asked.lock()[1].headers.contains_key("Cookie"), "asked as a visitor");
        assert!(matches!(block(c.read_now(Read::PlaylistList)), Ok(Page::Playlists { v }) if v.is_empty()));
        assert_eq!(t.asked.lock().len(), 3, "the library is not asked");
    }

    /// A refusal that a rotation does not set right ends the session too.
    #[test]
    fn refused_twice_the_session_ends() {
        let (c, _) = signed_in(30, vec![answer(200, "signed_out", &[]), answer(200, "", &[]), answer(200, "signed_out", &[])]);
        assert!(block(c.read_now(Read::PlaylistList)).is_err());
        assert_eq!(c.youtube_account(), YouTubeAccount::Ended);
    }

    /// Signed in, a song's heart is the account's like; signed out it has nowhere to go and is not queued.
    #[test]
    fn hearts_are_likes() {
        let (c, t) = signed_in(30, vec![answer(200, "", &[]), answer(200, "", &[])]);
        block(c.write(Write::Star { kind: Starrable::Song, id: "yt-v1".into(), on: true })).unwrap();
        block(c.write(Write::Star { kind: Starrable::Song, id: "yt-v1".into(), on: false })).unwrap();
        assert_eq!(t.urls(), ["https://music.youtube.com/youtubei/v1/like/like", "https://music.youtube.com/youtubei/v1/like/removelike"]);
        assert!(block(c.write(Write::Star { kind: Starrable::Album, id: "yt-MPREb_x".into(), on: true })).is_err(), "albums are not liked");
        let (c, t) = youtube(&[]);
        assert!(block(c.write(Write::Star { kind: Starrable::Song, id: "yt-v1".into(), on: true })).is_err());
        assert!(t.asked.lock().is_empty() && !c.core.pending_any().unwrap());
    }

    /// The favorites are the account's liked songs, hearted; the feed's shelves keep their kinds.
    #[test]
    fn liked_songs_are_the_favorites_and_the_feed_its_shelves() {
        let (c, _) = signed_in(30, vec![answer(200, "playlist", &[])]);
        let Page::StarredPage { v } = block(c.read_now(Read::StarredItems)).unwrap() else { panic!() };
        assert_eq!(v.songs.len(), 3);
        assert!(v.songs.iter().all(|s| s.starred && s.id.starts_with("yt-")));
        let (c, t) = youtube_answering(vec![answer(200, "home", &[]), answer(200, "home_more", &[])]);
        let Page::Feed { v } = block(c.read_now(Read::HomeFeed)).unwrap() else { panic!() };
        assert_eq!((v[0].title.as_str(), v[0].songs.len()), ("Quick picks", 3));
        assert!(v[0].songs[0].id == "yt-hHZ4ej144rc" && v[0].songs[0].album_id.as_deref() == Some("yt-MPREb_belO7bZVmAt"));
        assert_eq!((v[1].title.as_str(), v[1].playlists[0].id.as_str()), ("All hits", "yt-VLRDCLAK5uy_nHSqCJjDrW9HBhCNdF6tWPdnOMngOv0wA"));
        assert!(v[3].albums.iter().all(|a| a.id.starts_with("yt-MPREb_")));
        assert!(t.asked.lock().iter().all(|e| !e.headers.contains_key("Cookie")), "signed out, a visitor's feed");
        assert!(matches!(block(c.read_now(Read::StarredItems)), Ok(Page::StarredPage { v }) if v.songs.is_empty()));
    }
}
