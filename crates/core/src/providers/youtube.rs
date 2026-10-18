//! A YouTube Music profile: innertube-music's answers as nori's records under `yt-` ids, its reads as
//! the pages' `Page`s, its likes as hearts, its stream addresses and the URLs they open at. Best effort
//! and server-less; signed in or not (account.rs) (docs/research/youtube-music.md).

mod account;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use innertube_music as yt;

use super::{AccountState, Answer, FieldSpec, FormField, Provider, ProviderInfo, ResolveStream, SignIn, SignInResult};
use crate::browse::{LibrarySection, ProfileRules, SettingsPart};
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Starrable, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::{block_on, Exchange, NetError, Transport};
use crate::{Album, AlbumDetail, Artist, ArtistDetail, ArtistInfo, ArtistRef, Core, FeedShelf, Playlist, PlaylistDetail, SearchResult, Song, Starred};

/// What a YouTube Music profile's library is kept under (a server's is its address and user).
const IDENT: &str = "youtube-music";

/// Prefix of every YouTube id in nori (songs, albums, artists, playlists): the stream store and cover
/// cache are shared by all profiles.
const ID_PREFIX: &str = "yt-";

/// Scheme of a song's stream address until it is resolved.
const ADDRESS: &str = "youtube:";

/// Songs fetched ahead of the one playing: each costs a player request and a whole download.
const AHEAD: usize = 1;

/// nori's id for a YouTube id.
fn id(raw: &str) -> String {
    format!("{ID_PREFIX}{raw}")
}

/// YouTube's id inside nori's `id` (a live stream's radio id too); None when it is not a YouTube id.
fn raw_id(id: &str) -> Option<&str> {
    crate::unradio(id).strip_prefix(ID_PREFIX).filter(|r| !r.is_empty())
}

/// The YouTube id inside nori's `id`, or a parse error naming it.
fn raw(id: &str) -> NetResult<&str> {
    raw_id(id).ok_or_else(|| NetError::Parse { reason: format!("{id} is no YouTube id") })
}

/// The video a stream address names; None for any other URL.
fn address_video(address: &str) -> Option<&str> {
    address.strip_prefix(ADDRESS).filter(|v| !v.is_empty())
}

/// Where song `id` streams from and the key its bytes are kept under. The quality settings do not
/// apply: YouTube's file is played as it is.
fn stream_target(id: String) -> StreamTarget {
    let url = raw_id(&id).map(|v| format!("{ADDRESS}{v}")).unwrap_or_default();
    StreamTarget { url, key: format!("{id}:0") }
}

/// The URL of thumbnail `cover` at `size` px.
fn cover_address(cover: &str, size: u32) -> String {
    yt::thumbnail_url(cover, size)
}

/// Local playback, downloads and history are available signed out. Signing in adds the account's
/// library and likes. Remote control and jams use paired LAN devices without a server.
fn rules(signed_in: bool) -> ProfileRules {
    use SettingsPart::*;
    let settings = vec![Profiles, ServerOptions, Look, Playback, Sound, Lyrics, Downloads, Storage, About];
    let mut sections = if signed_in { vec![LibrarySection::Albums, LibrarySection::Favourites, LibrarySection::Playlists] } else { Vec::new() };
    sections.push(LibrarySection::Downloads);
    sections.push(LibrarySection::History);
    ProfileRules { asks: false, account: false, downloads: true, radio: true, indexed: false, sections, settings, remote: true, measures: true, stars: signed_in, feed: true }
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
    // A live stream plays as a radio station.
    let song_id = if s.live { crate::radio_id(&id(&s.video_id)) } else { id(&s.video_id) };
    Song {
        id: song_id,
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
struct State {
    api: yt::InnerTube,
    session: account::Session,
    streams: Streams,
}

/// The YouTube Music provider, over the profile's [`State`].
#[derive(Default)]
pub(crate) struct YouTube(Arc<State>);

impl State {
    /// `read` answered by YouTube Music; None for the reads it has nothing for. Signed in, the
    /// library's reads are the account's: its saved albums (one page), its playlists and its liked songs
    /// as the favorites.
    async fn read(&self, c: &Client, read: Read) -> NetResult<Option<Page>> {
        if let Read::SongById { id } = &read {
            if let Some(song) = c.stored_song_of(id) {
                return Ok(Some(Page::OneSong { v: Some(song) }));
            }
        }
        let (api, t) = (&self.api, &Wire(&*c.transport));
        let n = |count: i32| count.max(0) as usize;
        self.fresh_session(c, account::unix_now()).await;
        let signed_in = self.signed_in(c);
        Ok(Some(match read {
            Read::SimilarSongs { id, count } => Page::Songs { v: self.after(c, api.radio(t, raw(&id)?).await)?.into_iter().take(n(count)).map(song).collect() },
            Read::SongById { id } => {
                let video = raw(&id)?;
                let found = self.after(c, api.radio(t, video).await)?.into_iter().find(|s| s.video_id == video);
                Page::OneSong { v: found.map(song) }
            },
            Read::Search { query, songs, albums, artists } => Page::Found { v: found(self.after(c, api.search(t, &query).await)?, n(songs), n(albums), n(artists)) },
            Read::AlbumById { id } => Page::AlbumPage { v: album_detail(self.after(c, api.album(t, raw(&id)?).await)?) },
            Read::AlbumSongs { id } => Page::Songs { v: album_detail(self.after(c, api.album(t, raw(&id)?).await)?).songs },
            Read::ArtistById { id } => Page::ArtistPage { v: artist_detail(self.after(c, api.artist(t, raw(&id)?).await)?).0 },
            Read::ArtistAbout { id } => Page::About { v: artist_detail(self.after(c, api.artist(t, raw(&id)?).await)?).1 },
            Read::PlaylistById { id } => Page::PlaylistPage { v: playlist_detail(self.after(c, api.playlist(t, raw(&id)?).await)?) },
            Read::PlaylistSongs { id } => Page::Songs { v: playlist_detail(self.after(c, api.playlist(t, raw(&id)?).await)?).songs },
            Read::HomeFeed => Page::Feed { v: self.after(c, api.home(t).await)?.into_iter().map(shelf).collect() },
            Read::PlaylistList if signed_in => Page::Playlists { v: self.after(c, api.library_playlists(t).await)?.into_iter().map(|p| playlist(p, &[])).collect() },
            Read::AlbumList { offset: 0, .. } if signed_in => Page::Albums { v: self.after(c, api.library_albums(t).await)?.into_iter().map(album).collect() },
            Read::StarredItems if signed_in => {
                let liked = self.after(c, api.liked_songs(t).await)?.into_iter().map(|s| Song { starred: true, ..song(s) }).collect();
                Page::StarredPage { v: Starred::new(Vec::new(), Vec::new(), liked) }
            }
            _ => return Ok(None),
        }))
    }

    /// `w` for YouTube Music: a song's heart is the account's like; nothing else has anywhere to go.
    /// Never queued: a like that fails is taken back.
    async fn write(&self, c: &Client, w: Write) -> NetResult<()> {
        match w {
            Write::Star { kind: Starrable::Song, id, on } if self.signed_in(c) => {
                self.fresh_session(c, account::unix_now()).await;
                self.after(c, self.api.like(&Wire(&*c.transport), raw(&id)?, on).await)
            }
            _ => Err(super::nowhere("write")),
        }
    }
}

#[async_trait::async_trait]
impl Provider for YouTube {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            kind: ServerKind::YouTubeMusic,
            fields: vec![FieldSpec { what: FormField::Name, advanced: false }],
            sign_in: vec![SignIn::WebPage { start_url: yt::auth::SIGN_IN_PAGE.to_string() }, SignIn::Paste],
        }
    }

    fn ready(&self, _: &SavedServer) -> bool {
        true
    }

    /// No server to check.
    async fn check(&self, _: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
        Ok(profile)
    }

    /// Signed in when the profile names its account (`user`, kept at sign-in); never a jam guest's.
    fn rules(&self, profile: &SavedServer, guest: bool) -> ProfileRules {
        let mut shown = rules(!profile.user.is_empty() && !guest);
        if guest {
            shown.asks = true;
            shown.downloads = false;
            shown.radio = false;
            shown.measures = false;
            shown.stars = false;
            shown.sections.clear();
            shown.settings.retain(|p| !matches!(p, SettingsPart::ServerOptions | SettingsPart::Downloads));
        }
        shown
    }

    /// No server and no Subsonic account. Clears the library when the profile was a server's.
    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()> {
        core.serves(IDENT)?;
        *core.server.write() = crate::api::Server::default();
        core.set_rules(self.rules(profile, nori_remote::is_guest_key(&profile.api_key)));
        *core.native_login.write() = None;
        *core.account.write() = None;
        Ok(())
    }

    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>> {
        Ok(self.0.read(client, read).await?.map(|p| Answer::Page(Box::new(p))))
    }

    async fn write(&self, client: &Client, w: Write) -> NetResult<()> {
        self.0.write(client, w).await
    }

    fn stream(&self, _: &Client, id: String, _: StreamQuality) -> StreamTarget {
        stream_target(id)
    }

    fn ahead(&self) -> Option<usize> {
        Some(AHEAD)
    }

    fn cover_address(&self, _: &Core, cover: &str, size: u32) -> String {
        cover_address(cover, size)
    }

    fn resolver(&self, client: &Arc<Client>) -> Option<Arc<dyn ResolveStream>> {
        Some(Arc::new(Resolver { state: self.0.clone(), transport: client.transport.clone() }))
    }

    fn sign_in_done(&self, address: &str, cookies: &str) -> bool {
        yt::auth::signed_in_at(address, cookies)
    }

    async fn sign_in(&self, client: &Client, answer: String) -> SignInResult {
        self.0.sign_in(client, answer).await
    }

    fn sign_out(&self, client: &Client) {
        self.0.sign_out(client)
    }

    fn account(&self, client: &Client) -> AccountState {
        self.0.account(client)
    }
}

/// A song's stream address opens at the URL YouTube serves it at now, asked of YouTube unless one is
/// kept; a refused one is asked again.
#[derive(Clone)]
struct Resolver {
    state: Arc<State>,
    transport: Arc<dyn Transport>,
}

impl Resolver {
    /// The stream `video` plays from, asked of YouTube unless one is kept. Blocks the caller.
    fn stream(&self, video: &str) -> NetResult<yt::Stream> {
        let now = Instant::now();
        let streams = &self.state.streams;
        if let Some(s) = streams.get(video, now) {
            return Ok(s);
        }
        let s = block_on(self.state.api.player(&Wire(&*self.transport), video)).map_err(net_error)?;
        crate::alog::info(&format!("YouTube stream of {video}: itag {}, {}", s.format.itag, s.format.mime));
        streams.keep(video, s.clone(), now);
        Ok(s)
    }
}

impl ResolveStream for Resolver {
    /// Blocks the caller while YouTube is asked.
    fn resolve(&self, address: &str, from: u64) -> Option<NetResult<String>> {
        let video = address_video(address)?;
        Some(self.stream(video).map(|s| s.url_from(from)))
    }

    fn refused(&self, address: &str) {
        if let Some(video) = address_video(address) {
            self.state.streams.forget(video);
        }
    }

    fn live(&self, address: &str) -> Option<Box<dyn std::io::Read + Send>> {
        let video = address_video(address)?.to_string();
        Some(Box::new(LiveAudio { resolver: self.clone(), video, next: None, bytes: Vec::new(), read: 0 }))
    }
}

/// Pieces a live stream starts behind the newest: the next is made only as the last plays out, so
/// starting at the newest would leave nothing in hand while it is.
const LIVE_BEHIND: u64 = 2;

/// A live video's audio as one endless ADTS stream ([`yt::live`]), asked of YouTube a piece at a time on
/// the reading thread.
struct LiveAudio {
    resolver: Resolver,
    video: String,
    /// The next piece's number; None until the newest one's is known.
    next: Option<u64>,
    bytes: Vec<u8>,
    read: usize,
}

impl LiveAudio {
    /// The next piece into `bytes`. Its URL refused (served URLs expire), the stream is asked anew once.
    fn fetch(&mut self) -> NetResult<()> {
        self.bytes.clear();
        self.read = 0;
        let mut fresh = false;
        loop {
            let stream = self.resolver.stream(&self.video)?;
            match block_on(self.resolver.state.api.live_piece(&Wire(&*self.resolver.transport), &stream, self.next, &mut self.bytes)) {
                Ok(n) if self.next.is_none() => {
                    self.bytes.clear();
                    self.next = Some(n.saturating_sub(LIVE_BEHIND));
                }
                Ok(n) => {
                    self.next = Some(n + 1);
                    return Ok(());
                }
                Err(yt::Error::Status(403)) if !fresh => {
                    fresh = true;
                    self.resolver.state.streams.forget(&self.video);
                }
                Err(e) => return Err(net_error(e)),
            }
        }
    }
}

impl std::io::Read for LiveAudio {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.read == self.bytes.len() {
            self.fetch().map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        let n = buf.len().min(self.bytes.len() - self.read);
        buf[..n].copy_from_slice(&self.bytes[self.read..self.read + n]);
        self.read += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, client};
    use crate::client::NetProfile;
    use crate::transport::{FailureKind, Network, TransportError, TransportResponse};

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
        core.configure_profile(SavedServer { kind: ServerKind::YouTubeMusic, ..Default::default() }).unwrap();
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

    fn resolver(c: &Arc<Client>) -> Arc<dyn ResolveStream> {
        c.stream_resolver().expect("a YouTube Music profile resolves its addresses")
    }

    const ROTATE: &str = "https://accounts.youtube.com/RotateCookies";
    const BROWSE: &str = "https://music.youtube.com/youtubei/v1/browse";

    #[test]
    fn a_queued_song_is_read_without_a_request() {
        let (c, t) = youtube(&[]);
        let expected = Song { id: "yt-queued".into(), title: "Queued song".into(), duration: 200, ..Default::default() };
        c.core.session.register(vec![expected.clone()]);
        let Page::OneSong { v } = block(c.read_now(Read::SongById { id: expected.id.clone() })).unwrap() else { panic!() };
        assert_eq!(v, Some(expected));
        assert!(t.asked.lock().is_empty());
    }

    #[test]
    fn radio_reads_youtubes_watch_queue() {
        let (c, t) = youtube(&[&fixture("radio")]);
        let songs = block(c.songs(Read::SimilarSongs { id: "yt-dQw4w9WgXcQ".into(), count: 2 })).unwrap();
        assert_eq!(songs.iter().map(|s| (s.id.as_str(), s.title.as_str(), s.duration)).collect::<Vec<_>>(), [
            ("yt-dQw4w9WgXcQ", "Never Gonna Give You Up", 212),
            ("yt-djV11Xbc914", "Take On Me", 212),
        ]);
        assert!(t.asked.lock()[0].url.contains("/next?"));
    }

    #[test]
    fn radio_reuses_the_watch_queues_seed() {
        let (c, t) = youtube(&[&fixture("radio")]);
        let songs = block(c.radio("yt-dQw4w9WgXcQ".into())).unwrap();
        assert_eq!(songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["yt-dQw4w9WgXcQ", "yt-djV11Xbc914"]);
        assert_eq!(t.asked.lock().len(), 1);
    }

    /// A Home row says no length; the radio's own copy of its seed does.
    #[test]
    fn a_radio_seed_from_home_takes_the_watch_queues_length() {
        let (c, _) = youtube(&[&fixture("radio")]);
        c.core.session.register(vec![Song { id: "yt-dQw4w9WgXcQ".into(), title: "From Home".into(), ..Default::default() }]);
        let songs = block(c.radio("yt-dQw4w9WgXcQ".into())).unwrap();
        assert_eq!((songs[0].title.as_str(), songs[0].duration), ("From Home", 212));
    }

    #[test]
    fn a_song_not_kept_locally_uses_watch_metadata() {
        let (c, t) = youtube(&[&fixture("radio")]);
        let Page::OneSong { v: Some(s) } = block(c.read_now(Read::SongById { id: "yt-djV11Xbc914".into() })).unwrap() else { panic!() };
        assert_eq!((s.id.as_str(), s.title.as_str(), s.duration), ("yt-djV11Xbc914", "Take On Me", 212));
        assert_eq!(t.asked.lock().len(), 1);
    }

    #[test]
    fn the_car_lists_the_home_feed_and_available_library() {
        let (c, _) = youtube(&[&fixture("home"), &fixture("home_more")]);
        let home = block(c.browse_children("home".into(), 0, 100));
        assert!(!home.failed);
        assert_eq!(home.songs[0].id, "yt-hHZ4ej144rc");
        assert!(!home.folders.is_empty());
        let library = block(c.browse_children("library".into(), 0, 100));
        assert!(library.folders.is_empty(), "a signed-out profile has no account library");
        let root = c.car_root(4, false, 0, 100);
        assert_eq!(root.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["home", "downloads"]);
    }

    #[test]
    fn featured_collections_come_from_the_feed() {
        let (c, t) = youtube(&[&fixture("home"), &fixture("home_more"), &fixture("queue")]);
        let albums = block(c.featured_albums(1)).unwrap();
        assert_eq!(albums.len(), 1);
        assert!(albums[0].id.starts_with("yt-MPREb_"));
        let playlists = block(c.featured_playlists()).unwrap();
        assert_eq!(playlists.len(), 4);
        assert_eq!(playlists[0].id, "yt-VLRDCLAK5uy_nHSqCJjDrW9HBhCNdF6tWPdnOMngOv0wA");
        assert_eq!(t.asked.lock().len(), 3, "the feed is shared and no playlist or audio is opened");
    }

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

    /// The visitor id YouTube gave one session goes with the next session's first request, so Home is
    /// the same visitor's feed.
    #[test]
    fn the_visitor_id_is_kept_between_sessions() {
        let dir = nori_testdir::TempDir::new("yt-visitor");
        let path = dir.path().join("library.db").to_string_lossy().into_owned();
        let session = || {
            let core = crate::Core::new(path.clone(), "yt".into(), Default::default()).unwrap();
            core.configure_profile(SavedServer { id: "yt".into(), kind: ServerKind::YouTubeMusic, ..Default::default() }).unwrap();
            let t = Arc::new(InnerTube { answers: parking_lot::Mutex::new(vec![answer(200, "home_more", &[]), answer(200, "home", &[])]), ..Default::default() });
            block(Client::new(core, t.clone(), Default::default()).read_now(Read::HomeFeed)).unwrap();
            let visitor = t.asked.lock()[0].headers.get("X-Goog-Visitor-Id").cloned();
            visitor
        };
        assert_eq!(session(), None);
        assert_eq!(session().as_deref(), Some("CgtGSVhUVVJFX0lEKAA%3D"));
    }

    /// Home's fixture as YouTube reshuffles it: its two shelves swapped, and Quick picks without its
    /// first song.
    fn reshuffled_home() -> String {
        let mut home: serde_json::Value = serde_json::from_str(&fixture("home")).unwrap();
        let shelves = home.pointer_mut("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").unwrap().as_array_mut().unwrap();
        shelves.swap(0, 1);
        shelves[1].pointer_mut("/musicCarouselShelfRenderer/contents").unwrap().as_array_mut().unwrap().remove(0);
        home.to_string()
    }

    fn feed(c: &Client) -> Vec<(String, Vec<String>)> {
        let mut shown = Vec::new();
        block(c.read_each(Read::HomeFeed, |p| if let Page::Feed { v } = p { shown = v.iter().map(|s| (s.title.clone(), s.songs.iter().map(|x| x.id.clone()).collect())).collect() })).unwrap();
        shown
    }

    /// Home read again in the session is the page first shown, however long ago, without a request.
    #[test]
    fn home_stays_for_the_session() {
        let (c, t) = youtube(&[&fixture("home"), &fixture("home_more"), &fixture("queue"), &reshuffled_home(), &fixture("home_more")]);
        let first = feed(&c);
        c.core.kept.lock().values_mut().for_each(|k| k.at = 0);
        assert_eq!(feed(&c), first);
        assert_eq!(t.asked.lock().len(), 3);
    }

    /// The app started again takes YouTube's feed as it is now.
    #[test]
    fn a_new_session_takes_a_fresh_home() {
        let dir = nori_testdir::TempDir::new("yt-home-session");
        let path = dir.path().join("library.db").to_string_lossy().into_owned();
        let session = |home: String| {
            let core = crate::Core::new(path.clone(), "yt".into(), Default::default()).unwrap();
            core.configure_profile(SavedServer { id: "yt".into(), kind: ServerKind::YouTubeMusic, ..Default::default() }).unwrap();
            let answers = [home, fixture("home_more")].map(|b| TransportResponse { status: 200, body: b.into_bytes(), set_cookies: Vec::new() });
            let t = Arc::new(InnerTube { answers: parking_lot::Mutex::new(answers.into_iter().rev().collect()), ..Default::default() });
            feed(&Client::new(core, t, Default::default()))[0].0.clone()
        };
        assert_eq!(session(fixture("home")), "Quick picks");
        assert_eq!(session(reshuffled_home()), "All hits");
    }

    /// A refresh asks YouTube again but keeps every shelf and item where it was; only new ones are
    /// added, at the end.
    #[test]
    fn a_refresh_adds_new_picks_without_reshuffling() {
        let (c, t) = youtube(&[&reshuffled_home(), &fixture("home_more"), &fixture("queue"), &fixture("home"), &fixture("home_more"), &fixture("queue"), &reshuffled_home(), &fixture("home_more"), &fixture("queue")]);
        let first = feed(&c);
        assert_eq!(first[0].0, "All hits");
        let picks = first[1].1.clone();
        c.drop_cached(crate::browse::home_refresh_drops()).unwrap();
        let merged = feed(&c);
        assert_eq!(t.asked.lock().len(), 6);
        assert_eq!(merged.iter().map(|s| &s.0).collect::<Vec<_>>(), first.iter().map(|s| &s.0).collect::<Vec<_>>(), "no shelf moves");
        assert_eq!(merged[1].1[..picks.len()], picks[..], "shown picks keep their places");
        assert_eq!(merged[1].1[picks.len()..], ["yt-hHZ4ej144rc".to_string()], "the new pick goes last");
        c.drop_cached(crate::browse::home_refresh_drops()).unwrap();
        assert_eq!(feed(&c), merged, "a feed only rearranged changes nothing");
    }

    #[test]
    fn a_lan_guest_never_uses_the_owners_cookies() {
        let dir = nori_testdir::TempDir::new("yt-jam-cookie-isolation");
        let path = dir.path().join("library.db").to_string_lossy().into_owned();
        let settings = crate::settings_store::Settings::new();
        settings.open(&path).unwrap();
        let session = Arc::new(crate::queue::Session::new(settings));
        let own = crate::Core::new(path.clone(), "owner".into(), session.clone()).unwrap();
        own.configure_profile(SavedServer { id: "owner".into(), kind: ServerKind::YouTubeMusic, user: "private".into(), ..Default::default() }).unwrap();
        let transport = Arc::new(InnerTube { answers: parking_lot::Mutex::new(vec![answer(200, "search_artists", &[]), answer(200, "search_albums", &[]), answer(200, "search_songs", &[])]), ..Default::default() });
        let owner = Client::new(own, transport.clone(), Default::default());
        account::tests::store(&owner, &account::tests::session(SESSION, 30));
        let profile = crate::remote::jam_profile(crate::remote::JamPass { url: "http://192.168.1.2:1234".into(), api_key: nori_remote::guest_key("lan-guest") }, "guest".into(), "Jam".into());
        let guest = crate::Core::new(path, profile.id.clone(), session).unwrap();
        guest.configure_profile(profile).unwrap();
        let guest = Client::new(guest, transport.clone(), Default::default());
        let Page::Found { v } = block(guest.read_now(Read::Search { query: "daft punk".into(), songs: 1, albums: 0, artists: 0 })).unwrap() else { panic!() };
        assert!(!v.songs.is_empty());
        assert!(transport.asked.lock().iter().all(|request| !request.headers.contains_key("Cookie")));
        assert_eq!(account::tests::stored(&owner).unwrap().cookies(), SESSION);
        assert!(account::tests::stored(&guest).is_none());
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
        assert!(matches!(block(c.sync_library()), Err(NetError::Api { .. })), "no server to sync");
        assert!(t.asked.lock().is_empty());
        let rules = c.core.rules();
        assert!(!rules.account && rules.remote && rules.measures);
    }

    /// A song streams from its YouTube address under a key that ignores the quality settings; nothing
    /// past the next song is fetched ahead.
    #[test]
    fn downloads_use_the_provider() {
        let (c, _) = youtube(&[]);
        let target = c.download_target("yt-dQw4w9WgXcQ".into(), StreamQuality { bit_rate: 192, format: "mp3".into() });
        assert_eq!(target.url, "youtube:dQw4w9WgXcQ");
        assert_eq!(target.key, "dl:yt-dQw4w9WgXcQ");
        let thumbnail = "https://yt3.googleusercontent.com/abc";
        assert_eq!(c.core.download_cover_urls(vec![thumbnail.into()]), [format!("{thumbnail}=w320-h320-l90-rj"), format!("{thumbnail}=w800-h800-l90-rj")], "its covers are YouTube's");
    }

    #[test]
    fn songs_stream_from_youtube_addresses() {
        let (c, _) = youtube(&[]);
        let t = c.resolve("yt-dQw4w9WgXcQ".into(), false, false);
        assert_eq!((t.url.as_str(), t.key.as_str()), ("youtube:dQw4w9WgXcQ", "yt-dQw4w9WgXcQ:0"));
        let quality = StreamQuality { bit_rate: 192, format: "opus".into() };
        assert_eq!(c.stream_key("yt-dQw4w9WgXcQ".into(), true, quality.clone(), quality), t.key, "the measurer finds the same bytes as playback on either network");
        let ids: Vec<String> = ["yt-a", "yt-b", "yt-c", "yt-d"].map(String::from).to_vec();
        let songs = ids.iter().map(|id| crate::Song { id: id.clone(), ..Default::default() }).collect();
        c.session().register(songs);
        c.session().set(ids, Some(0), false, None);
        assert_eq!(c.session().precache(false), ["yt-c", "yt-d"], "the songs after next would be fetched");
        assert!(c.precache_targets(false).is_empty());
        assert_eq!(c.core.cover_address("https://yt3.googleusercontent.com/abc".into(), 320), "https://yt3.googleusercontent.com/abc=w320-h320-l90-rj");
        assert_eq!(c.core.cover_prefix(), None, "each cover's address is its own");
    }

    /// A stream is asked once and kept; refused, it is asked again. The first open asks the whole file
    /// as a range, later ones come plain (with the byte source's `Range`).
    #[test]
    fn a_stream_is_resolved_once_until_refused() {
        let player = fixture("player");
        let (c, t) = youtube(&[&player, &player]);
        let r = resolver(&c);
        let first = r.resolve("youtube:dQw4w9WgXcQ", 0).unwrap().unwrap();
        assert!(first.contains("itag=251") && first.ends_with("&range=0-3433754"), "{first}");
        let later = r.resolve("youtube:dQw4w9WgXcQ", 2_000_000).unwrap().unwrap();
        assert!(!later.contains("&range="));
        assert_eq!(t.asked.lock().len(), 1);
        r.refused("youtube:dQw4w9WgXcQ");
        assert!(r.resolve("youtube:dQw4w9WgXcQ", 0).unwrap().is_ok());
        assert_eq!(t.asked.lock().len(), 2);
        assert!(r.resolve("https://nas/rest/stream?id=1", 0).is_none(), "a server's URL is opened as it is");
        assert!(r.resolve("youtube:gone", 0).unwrap().is_err(), "a failed request fails the open");
    }

    /// A live video is a radio station that streams from its YouTube address.
    #[test]
    fn a_live_video_plays_as_a_radio_station() {
        let s = song(yt::Song { video_id: "v1".into(), live: true, ..Default::default() });
        assert_eq!(s.id, "radio:yt-v1");
        assert_eq!(raw_id(&s.id), Some("v1"));
        let (c, _) = youtube(&[]);
        assert_eq!(c.resolve(s.id, false, false).url, "youtube:v1");
    }

    /// A live stream starts two pieces behind the newest and goes on by number; its URL refused (an
    /// expired one), the stream is asked again and the same piece asked anew.
    #[test]
    fn a_live_stream_is_read_piece_by_piece() {
        let piece = std::fs::read(format!("{}/../youtube/tests/fixtures/live_piece.mp4", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let numbered = |n: &str| {
            let at = piece.windows(19).position(|w| w == b"Sequence-Number: 41").unwrap() + 17;
            let mut p = piece.clone();
            p[at..at + 2].copy_from_slice(n.as_bytes());
            TransportResponse { status: 200, body: p, set_cookies: Vec::new() }
        };
        let player = || answer(200, "player", &[]);
        let (c, t) = youtube_answering(vec![player(), numbered("41"), numbered("39"), answer(403, "", &[]), player(), numbered("40")]);
        let mut frames = Vec::new();
        assert_eq!(yt::live::adts(&piece, &mut frames), Some(41));
        let mut live = resolver(&c).live("youtube:dQw4w9WgXcQ").unwrap();
        let mut heard = vec![0; 2 * frames.len()];
        std::io::Read::read_exact(&mut live, &mut heard).unwrap();
        assert_eq!(heard, [&frames[..], &frames[..]].concat(), "39, then 40");
        let asked: Vec<String> = t.asked.lock().iter().map(|e| e.url.rsplit_once('&').map_or("player", |(_, q)| if q.starts_with("sq=") { q } else { "newest" }).to_string()).collect();
        assert_eq!(asked, ["player", "newest", "sq=39", "sq=40", "player", "sq=40"]);
        assert!(resolver(&c).live("https://nas/rest/stream?id=1").is_none());
    }

    /// An unplayable video's error carries YouTube's reason.
    #[test]
    fn an_unplayable_video_says_why() {
        let (c, _) = youtube(&[&fixture("unplayable")]);
        match resolver(&c).resolve("youtube:x", 0) {
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
        assert!(c.stream_resolver().is_none());
        assert!(c.resolve("yt-x".into(), false, false).url.contains("/rest/stream?"));
    }

    /// Signing in asks YouTube for the account's name and keeps the session, rotated cookies included;
    /// hearts and the library follow.
    #[test]
    fn signing_in_keeps_the_session() {
        let (c, t) = youtube_answering(vec![answer(200, "account_menu", &["SIDCC=cc; Domain=.youtube.com"])]);
        assert_eq!(c.account(), AccountState::SignedOut);
        assert_eq!(block(c.sign_in(format!("Cookie: {SESSION}"))), SignInResult::SignedIn { name: "Ann Example".into() });
        assert_eq!(t.asked.lock()[0].headers["Cookie"], SESSION);
        let kept = account::tests::stored(&c).unwrap();
        assert_eq!(kept.cookies(), format!("{SESSION}; SIDCC=cc"));
        assert!(kept.rotated_ago() < 5, "fresh from the browser");
        assert_eq!(c.account(), AccountState::SignedIn);
        assert!(c.core.rules().stars && !c.core.rules().sections.is_empty());
        c.sign_out();
        assert_eq!((c.account(), account::tests::stored(&c)), (AccountState::SignedOut, None));
        assert!(!c.core.rules().stars);
    }

    /// Text without a session's cookies asks nothing; cookies YouTube does not take change nothing.
    #[test]
    fn a_failed_sign_in_changes_nothing() {
        let (c, t) = youtube_answering(vec![answer(200, "signed_out", &[]), answer(429, "", &[]), answer(200, "signed_out", &[])]);
        assert_eq!(block(c.sign_in("VISITOR_INFO1_LIVE=v".into())), SignInResult::NoSession);
        assert!(t.asked.lock().is_empty());
        assert_eq!(block(c.sign_in(SESSION.into())), SignInResult::Refused);
        assert_eq!(t.urls()[1], ROTATE, "asked again after a rotation");
        assert!(matches!(block(c.sign_in(SESSION.into())), SignInResult::Unreachable { .. }), "the network is down");
        assert_eq!((c.account(), account::tests::stored(&c)), (AccountState::SignedOut, None));
    }

    /// Google's page is done once it is back on YouTube Music holding a session.
    #[test]
    fn the_sign_in_page_is_googles_and_done_on_youtube_music() {
        let info = YouTube::default().info();
        assert!(matches!(&info.sign_in[0], SignIn::WebPage { start_url } if start_url.starts_with("https://accounts.google.com/")));
        assert!(super::super::sign_in_done(ServerKind::YouTubeMusic, "https://music.youtube.com/".into(), SESSION.into()));
        assert!(!super::super::sign_in_done(ServerKind::YouTubeMusic, "https://accounts.google.com/x".into(), SESSION.into()));
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
        assert_eq!(c.account(), AccountState::SignedIn);

        let (c, t) = signed_in(30, vec![answer(401, "", &[]), answer(200, "", &["__Secure-1PSIDTS=ts2"]), answer(200, "library_playlists", &[])]);
        assert!(matches!(block(c.read_now(Read::PlaylistList)), Ok(Page::Playlists { v }) if v.len() == 2));
        assert_eq!(t.urls(), [BROWSE, ROTATE, BROWSE], "one refusal: rotated and asked again");
        assert_eq!(c.account(), AccountState::SignedIn);
        assert!(account::tests::stored(&c).unwrap().cookies().contains("ts2"));
    }

    /// A rotation YouTube refuses ends the session: it says so, keeps the cookies for a look, and the
    /// profile goes on as a visitor's until signed in again.
    #[test]
    fn a_refused_session_ends_but_is_not_wiped() {
        let (c, t) = signed_in(601, vec![answer(401, "", &[]), answer(200, "home", &[]), answer(200, "home_more", &[]), answer(200, "queue", &[])]);
        let Page::Feed { v } = block(c.read_now(Read::HomeFeed)).unwrap() else { panic!() };
        assert_eq!(v.len(), 4, "everyone's feed");
        assert_eq!(c.account(), AccountState::Ended);
        let kept = account::tests::stored(&c).unwrap();
        assert!(kept.ended() && kept.cookies() == SESSION);
        assert!(!t.asked.lock()[1].headers.contains_key("Cookie"), "asked as a visitor");
        assert!(matches!(block(c.read_now(Read::PlaylistList)), Ok(Page::Playlists { v }) if v.is_empty()));
        assert_eq!(t.asked.lock().len(), 4, "the library is not asked");
    }

    /// A refusal that a rotation does not set right ends the session too.
    #[test]
    fn refused_twice_the_session_ends() {
        let (c, _) = signed_in(30, vec![answer(200, "signed_out", &[]), answer(200, "", &[]), answer(200, "signed_out", &[])]);
        assert!(block(c.read_now(Read::PlaylistList)).is_err());
        assert_eq!(c.account(), AccountState::Ended);
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

    struct Shown;
    impl crate::library::StarsShown for Shown {
        fn marks(&self, _: crate::stars::StarMarks) {}
    }

    /// A heart shows in the favorites at once, in the kept list and in YouTube's next answer that does
    /// not list the like yet; an unhearted song leaves both.
    #[test]
    fn hearts_show_in_the_favorites_at_once() {
        let (c, _) = signed_in(30, vec![answer(200, "playlist", &[]), answer(200, "album", &[]), answer(200, "", &[]), answer(200, "", &[]), answer(200, "playlist", &[])]);
        let ids = |p: Option<Page>| match p {
            Some(Page::StarredPage { v }) => v.songs.into_iter().map(|s| (s.id, s.starred)).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(ids(block(c.cached_or_fetched(Read::StarredItems)).unwrap()).len(), 3);
        block(c.read_now(Read::AlbumById { id: "yt-MPREb_K8qWMWVqXGi".into() })).unwrap();
        block(c.star(Starrable::Song, "yt-zhl-Cs1-sG4".into(), true, Arc::new(Shown))).unwrap();
        block(c.star(Starrable::Song, "yt-khnokW3Mw24".into(), false, Arc::new(Shown))).unwrap();
        let shown = ids(c.read_stored(Read::StarredItems).unwrap().page);
        assert_eq!(shown[0], ("yt-zhl-Cs1-sG4".to_string(), true), "{shown:?}");
        assert!(shown.len() == 3 && !shown.iter().any(|(id, _)| id == "yt-khnokW3Mw24"), "{shown:?}");
        assert_eq!(ids(block(c.read_now(Read::StarredItems)).ok()), shown, "YouTube lags; the marks do not");
    }

    /// Answers InnerTube by what is asked, as YouTube would, and keeps each request's path and target.
    #[derive(Default)]
    struct Answers {
        asked: parking_lot::Mutex<Vec<String>>,
    }

    impl Answers {
        fn taken(&self) -> Vec<String> {
            std::mem::take(&mut *self.asked.lock())
        }
    }

    #[async_trait::async_trait]
    impl Transport for Answers {
        async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
            panic!("a Subsonic request in a YouTube Music profile: {url}");
        }
        async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
            let path = request.url.split('?').next().unwrap_or_default().rsplit("/v1/").next().unwrap_or_default().to_string();
            let body: serde_json::Value = request.json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            let at = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let target = ["browseId", "continuation", "videoId", "query"].iter().map(|k| at(k)).find(|v| !v.is_empty()).unwrap_or_default();
            self.asked.lock().push(format!("{path} {target}"));
            let browse = at("browseId");
            let name = match path.as_str() {
                "browse" if !at("continuation").is_empty() => "home_more",
                "browse" if browse == "FEmusic_home" => "home",
                "browse" if browse == "FEmusic_liked_albums" => "library_albums",
                "browse" if browse == "FEmusic_liked_playlists" => "library_playlists",
                "browse" if browse.starts_with("MPREb") => "album",
                "browse" if browse.starts_with("UC") => "artist",
                "browse" if browse.starts_with("VL") => "playlist",
                "next" => "radio",
                "music/get_queue" => "queue",
                "player" => "player",
                "search" => match at("params").as_str() {
                    "EgWKAQIIAWoKEAkQBRAKEAMQBA==" => "search_songs",
                    "EgWKAQIYAWoKEAkQBRAKEAMQBA==" => "search_albums",
                    _ => "search_artists",
                },
                _ => "",
            };
            Ok(answer(200, name, &[]))
        }
        fn address_changed(&self) {}
        fn network(&self) -> Network {
            Network::Unmetered
        }
    }

    /// The differential harness's YouTube Music session ([`crate::providers::harness`]): signed in,
    /// its pages read, kept and refreshed, hearts given and taken back.
    #[test]
    fn a_youtube_session() {
        use crate::providers::harness::{age, each, kept, now, Marked, Transcript};
        let mut t = Transcript::new("providers-youtube");
        let core = crate::Core::new(String::new(), "yt".into(), Default::default()).unwrap();
        core.configure_profile(SavedServer { kind: ServerKind::YouTubeMusic, user: "Ann".into(), ..Default::default() }).unwrap();
        let yt = Arc::new(Answers::default());
        let c = Client::new(core, yt.clone(), Default::default());
        account::tests::store(&c, &account::tests::session(SESSION, 30));
        let asked = |t: &mut Transcript| yt.taken().into_iter().for_each(|a| t.line(format!("asked {a}")));
        let reads = [
            Read::HomeFeed,
            Read::AlbumById { id: "yt-MPREb_K8qWMWVqXGi".into() },
            Read::ArtistById { id: "yt-UCRr1xG_2WIDs18a6cIiCxeA".into() },
            Read::ArtistAbout { id: "yt-UCRr1xG_2WIDs18a6cIiCxeA".into() },
            Read::PlaylistById { id: "yt-VLRDCLAK5uy_nHSqCJjDrW9HBhCNdF6tWPdnOMngOv0wA".into() },
            Read::PlaylistList,
            Read::AlbumList { kind: crate::browse::AlbumSort::ByName, size: 60, offset: 0, genre: None },
            Read::AlbumList { kind: crate::browse::AlbumSort::ByName, size: 60, offset: 60, genre: None },
            Read::StarredItems,
            Read::FavouriteAlbums { size: 20 },
            Read::GenreList,
            Read::LyricsBySong { song_id: "yt-x".into() },
        ];

        t.step("first reads");
        for r in &reads {
            each(&mut t, &c, r.clone());
            asked(&mut t);
        }
        t.step("stored");
        for r in &reads {
            t.stored(r, &c.read_stored(r.clone()));
        }
        t.step("read again while fresh");
        for r in &reads {
            each(&mut t, &c, r.clone());
        }
        asked(&mut t);
        t.step("aged");
        age(&c);
        for r in &reads {
            each(&mut t, &c, r.clone());
        }
        asked(&mut t);

        t.step("reads always asked");
        for r in [
            Read::Search { query: "daft punk".into(), songs: 2, albums: 2, artists: 1 },
            Read::SimilarSongs { id: "yt-dQw4w9WgXcQ".into(), count: 3 },
            Read::SongById { id: "yt-djV11Xbc914".into() },
            Read::AlbumSongs { id: "yt-MPREb_K8qWMWVqXGi".into() },
            Read::PlaylistSongs { id: "yt-VLRDCLAK5uy_nHSqCJjDrW9HBhCNdF6tWPdnOMngOv0wA".into() },
            Read::RandomSongs { size: 5, genre: None },
            Read::Ping,
            Read::PullQueue,
        ] {
            now(&mut t, &c, r.clone());
            t.stored(&r, &c.read_stored(r.clone()));
            asked(&mut t);
        }

        t.step("Home's refresh");
        c.drop_cached(crate::browse::home_refresh_drops()).unwrap();
        kept(&mut t, &c, &reads);
        each(&mut t, &c, Read::HomeFeed);
        asked(&mut t);
        kept(&mut t, &c, &reads);

        t.step("hearts");
        for r in &reads {
            each(&mut t, &c, r.clone());
        }
        yt.taken();
        t.result("heart", &block(c.star(Starrable::Song, "yt-zhl-Cs1-sG4".into(), true, Arc::new(Marked))));
        t.result("unheart", &block(c.star(Starrable::Song, "yt-khnokW3Mw24".into(), false, Arc::new(Marked))));
        t.result("album", &block(c.star(Starrable::Album, "yt-MPREb_K8qWMWVqXGi".into(), true, Arc::new(Marked))));
        asked(&mut t);
        kept(&mut t, &c, &reads);
        for r in &reads {
            t.stored(r, &c.read_stored(r.clone()));
        }
        for r in &reads {
            each(&mut t, &c, r.clone());
        }
        asked(&mut t);
        now(&mut t, &c, Read::StarredItems);
        asked(&mut t);

        t.step("writes with nowhere to go");
        t.result("playlist", &block(c.write(Write::CreatePlaylist { name: "x".into(), song_ids: vec!["yt-a".into()] })));
        t.result("scrobble", &block(c.write(Write::Scrobble { id: "yt-a".into(), submission: true, time_ms: None })));
        t.line(format!("pending {}", c.core.pending_any().unwrap()));
        asked(&mut t);

        t.step("streams, downloads and covers");
        let r = c.resolve("yt-dQw4w9WgXcQ".into(), false, false);
        t.line(format!("stream {} {}", r.url, r.key));
        let d = c.download_target("yt-dQw4w9WgXcQ".into(), StreamQuality { bit_rate: 192, format: "mp3".into() });
        t.line(format!("download {} {}", d.url, d.key));
        t.line(format!("cover {}", c.core.cover_address("https://yt3.googleusercontent.com/abc".into(), 320)));
        t.line(format!("download covers {:?}", c.core.download_cover_urls(vec!["https://yt3.googleusercontent.com/abc".into()])));
        t.finish();
    }

    /// A kept page asked anew that YouTube answers the same is not shown again.
    #[test]
    fn an_unchanged_page_is_shown_once() {
        let (c, t) = youtube(&[&fixture("album"), &fixture("album")]);
        let read = || Read::AlbumById { id: "yt-MPREb_K8qWMWVqXGi".into() };
        block(c.read_each(read(), |_| {})).unwrap();
        c.core.kept.lock().values_mut().for_each(|k| k.at = 0);
        let mut shown = 0;
        block(c.read_each(read(), |_| shown += 1)).unwrap();
        assert_eq!((shown, t.asked.lock().len()), (1, 2), "the kept page, and YouTube asked");
    }

    /// After a heart, favourites YouTube lists as before show once: the heart is laid over the kept
    /// page and over YouTube's alike.
    #[test]
    fn favourites_listed_as_before_a_heart_show_once() {
        let (c, _) = signed_in(30, vec![answer(200, "playlist", &[]), answer(200, "", &[]), answer(200, "playlist", &[])]);
        block(c.cached_or_fetched(Read::StarredItems)).unwrap();
        block(c.star(Starrable::Song, "yt-khnokW3Mw24".into(), false, Arc::new(Shown))).unwrap();
        let mut shown = Vec::new();
        block(c.read_each(Read::StarredItems, |p| shown.push(p.songs().len()))).unwrap();
        assert_eq!(shown, [2]);
    }

    /// A heart makes the kept favourites stale, as a server's: YouTube is asked for them on the next read.
    #[test]
    fn a_heart_asks_the_favourites_anew() {
        let (c, t) = signed_in(30, vec![answer(200, "playlist", &[]), answer(200, "", &[]), answer(200, "playlist", &[])]);
        block(c.cached_or_fetched(Read::StarredItems)).unwrap();
        block(c.star(Starrable::Song, "yt-khnokW3Mw24".into(), false, Arc::new(Shown))).unwrap();
        assert!(!c.read_stored(Read::StarredItems).unwrap().fresh);
        block(c.cached_or_fetched(Read::StarredItems)).unwrap();
        assert_eq!(t.urls(), [BROWSE, "https://music.youtube.com/youtubei/v1/like/removelike", BROWSE]);
    }

    /// Signed-in library features and a temporary guest's requests follow the profile rules.
    #[test]
    fn its_rules_follow_the_account() {
        let (out, signed) = (rules(false), rules(true));
        assert!(out.feed && signed.feed && out.measures && out.remote && !out.account && !out.indexed);
        assert_eq!(out.sections, [LibrarySection::Downloads, LibrarySection::History]);
        assert!(out.downloads && !out.stars);
        assert!(!out.settings.iter().any(|p| matches!(p, SettingsPart::Library | SettingsPart::Streaming)));
        assert!(signed.downloads && signed.stars && signed.sections == [LibrarySection::Albums, LibrarySection::Favourites, LibrarySection::Playlists, LibrarySection::Downloads, LibrarySection::History]);
        assert!(!signed.account && signed.remote, "LAN remote is independent of a server account");
        let of = |user: &str, guest| super::super::rules_of(SavedServer { kind: ServerKind::YouTubeMusic, user: user.into(), ..Default::default() }, guest);
        let guest = of("", true);
        assert!(guest.asks && guest.remote && !guest.downloads && !guest.radio && !guest.stars && !guest.measures && guest.sections.is_empty());
        assert_eq!(of("Ann", false), signed, "signed in as Ann");
        assert!(of("", false).settings.contains(&SettingsPart::ServerOptions), "profiles are added from it");
    }

    /// The profile asks only for a name, keeps its kind, and a second one added takes over the first,
    /// still signed in, never a server's.
    #[test]
    fn a_second_profile_is_the_first() {
        use super::super::{server_for_login, server_ready};
        use crate::settings::ServerList;
        let yt = SavedServer { id: "y1".into(), kind: ServerKind::YouTubeMusic, ..SavedServer::default() };
        assert!(server_ready(yt.clone()));
        assert!(!server_ready(SavedServer { kind: ServerKind::Subsonic, ..yt.clone() }));
        let blank_server = SavedServer { id: "s1".into(), ..SavedServer::default() };
        let list = ServerList { servers: vec![blank_server, yt], active_server_id: "s1".into() };
        let again = SavedServer { id: crate::settings::new_server_id(), kind: ServerKind::YouTubeMusic, ..SavedServer::default() };
        assert_eq!(server_for_login(list.clone(), again.clone()).id, "y1");
        let mut signed_in = list.clone();
        signed_in.servers[1].user = "Ann".into();
        assert_eq!(server_for_login(signed_in, again).user, "Ann", "still signed in");
        let server = SavedServer { id: crate::settings::new_server_id(), ..SavedServer::default() };
        assert_eq!(server_for_login(list, server).id, "s1", "the server's, not YouTube's");
    }
}
