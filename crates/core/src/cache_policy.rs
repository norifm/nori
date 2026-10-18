//! Server reads and their response cache. [`Client::read_stored`] returns the cached page at once (and
//! whether it is fresh enough to skip the server); [`Client::read_fetch`] asks the server and returns a
//! page only if it changed. Writes evict what they change (client.rs).

use std::hash::{Hash, Hasher};

use rusqlite::OptionalExtension;

use crate::client::{pairs, Client, NetResult};
use crate::{
    Album, AlbumDetail, Artist, ArtistDetail, ArtistInfo, Directory, FeedShelf, Genre, Lyrics, MusicFolder, PlayQueue, Playlist, PlaylistDetail, RadioStation, SearchResult,
    ServerInfo, Song, Starred,
};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
/// Freshness window of most reads.
const BROWSE: i64 = 2 * MINUTE;

/// A server read. Variants up to `FolderById`, and `HomeFeed`, are cached; the rest always ask.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Read {
    /// getAlbumList2 in `kind`'s order. `ByYear` means this year; `Random` is never cached.
    AlbumList { kind: crate::browse::AlbumSort, size: i32, offset: i32, genre: Option<String> },
    AlbumsByYear { from: i32, to: i32, size: i32, offset: i32 },
    /// The starred `AlbumList` (same cache entry) with this session's star changes applied.
    FavouriteAlbums { size: i32 },
    ArtistIndex,
    AlbumById { id: String },
    ArtistById { id: String },
    ArtistAbout { id: String },
    TopSongs { artist: String },
    PlaylistList,
    PlaylistById { id: String },
    StarredItems,
    GenreList,
    RadioList,
    /// Asks with `enhanced=true` for word cues and translations (OpenSubsonic songLyrics v2).
    LyricsBySong { song_id: String },
    /// The folder tree, for libraries organised by directory.
    FolderIndex,
    FolderById { id: String },
    // Never cached:
    MusicFolders,
    Ping,
    /// Uncached so octo-fiesta's provider results show; one big page since the proxy repeats them per offset.
    Search { query: String, songs: i32, albums: i32, artists: i32 },
    /// Every song on the server, `count` from `offset`, in the server's order: the songs list of a profile
    /// that keeps no offline index (a jam guest's).
    SongPage { offset: i32, count: i32 },
    RandomSongs { size: i32, genre: Option<String> },
    SongsByGenre { genre: String, count: i32 },
    SimilarSongs { id: String, count: i32 },
    SongById { id: String },
    AlbumSongs { id: String },
    PlaylistSongs { id: String },
    /// Not queued offline: stale by the time it could be replayed.
    NowPlaying { id: String },
    /// A public share link (needs sharing enabled on the server).
    ShareLink { id: String },
    PullQueue,
    /// The source's own home feed (`ProfileRules::feed`), kept for the session; a Subsonic server has none.
    HomeFeed,
}

/// A parsed answer; the variant follows the read.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Page {
    Albums { v: Vec<Album> },
    Artists { v: Vec<Artist> },
    AlbumPage { v: AlbumDetail },
    ArtistPage { v: ArtistDetail },
    About { v: ArtistInfo },
    Songs { v: Vec<Song> },
    OneSong { v: Option<Song> },
    Playlists { v: Vec<Playlist> },
    PlaylistPage { v: PlaylistDetail },
    StarredPage { v: Starred },
    Genres { v: Vec<Genre> },
    Stations { v: Vec<RadioStation> },
    LyricsPage { v: Lyrics },
    DirectoryPage { v: Directory },
    Found { v: SearchResult },
    Folders { v: Vec<MusicFolder> },
    Status { v: ServerInfo },
    ShareUrl { v: String },
    /// The queue saved on the server, and the app that saved it (`by`).
    Queue { v: PlayQueue, by: String },
    Feed { v: Vec<FeedShelf> },
}

impl Page {
    /// Its songs; none for a page without.
    pub fn songs(self) -> Vec<Song> {
        match self {
            Page::Songs { v } => v,
            Page::AlbumPage { v } => v.songs,
            Page::PlaylistPage { v } => v.songs,
            Page::StarredPage { v } => v.songs,
            _ => Vec::new(),
        }
    }

    /// Its albums; none for a page without.
    pub fn albums(self) -> Vec<Album> {
        match self {
            Page::Albums { v } => v,
            Page::ArtistPage { v } => v.albums,
            _ => Vec::new(),
        }
    }
}

/// The cached answer for a read. `digest` (None: nothing cached) is passed back to [`Client::read_fetch`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Stored {
    pub page: Option<Page>,
    pub digest: Option<u64>,
    /// Parsed and within its freshness window: skip the server.
    pub fresh: bool,
}

#[derive(Clone, Copy)]
enum Parser {
    AlbumList,
    FavouriteAlbums,
    Artists,
    Album,
    AlbumSongs,
    Artist,
    ArtistInfo,
    Songs,
    OneSong,
    Playlists,
    Playlist,
    PlaylistSongs,
    Starred,
    Genres,
    Radio,
    Lyrics,
    Indexes,
    Directory,
    Search,
    Folders,
    Status,
    Share,
    Queue,
    Feed,
}

struct Spec {
    endpoint: &'static str,
    params: Vec<(String, String)>,
    /// None: never cached.
    fresh_ms: Option<i64>,
    parser: Parser,
}

/// The current local year.
fn this_year() -> i32 {
    let now = crate::db::now_ms();
    nori_library::smart::civil_from_ms(now + nori_library::library::local_offset_s(now / 1000) * 1000).0 as i32
}

fn by_year(from: i32, to: i32, size: i32, offset: i32) -> Spec {
    let p = pairs(&[("type", "byYear".into()), ("fromYear", from.to_string()), ("toYear", to.to_string()), ("size", size.to_string()), ("offset", offset.to_string())]);
    Spec { endpoint: "getAlbumList2", params: p, fresh_ms: Some(BROWSE), parser: Parser::AlbumList }
}

fn spec(read: Read) -> Spec {
    let s = |endpoint, params, fresh_ms, parser| Spec { endpoint, params, fresh_ms, parser };
    let id = |id: String| pairs(&[("id", id)]);
    match read {
        Read::AlbumList { kind, size, offset, genre } => {
            use crate::browse::AlbumSort;
            if kind == AlbumSort::ByYear {
                return by_year(this_year(), 0, size, offset);
            }
            let fresh = (kind != AlbumSort::Random).then_some(BROWSE);
            let mut p = pairs(&[("type", kind.api().into()), ("size", size.to_string()), ("offset", offset.to_string())]);
            if let Some(g) = genre {
                p.push(("genre".into(), g));
            }
            s("getAlbumList2", p, fresh, Parser::AlbumList)
        }
        Read::AlbumsByYear { from, to, size, offset } => by_year(from, to, size, offset),
        Read::FavouriteAlbums { size } => {
            let p = pairs(&[("type", "starred".into()), ("size", size.to_string()), ("offset", "0".into())]);
            s("getAlbumList2", p, Some(BROWSE), Parser::FavouriteAlbums)
        }
        Read::ArtistIndex => s("getArtists", vec![], Some(BROWSE), Parser::Artists),
        Read::AlbumById { id: i } => s("getAlbum", id(i), Some(BROWSE), Parser::Album),
        Read::ArtistById { id: i } => s("getArtist", id(i), Some(BROWSE), Parser::Artist),
        Read::ArtistAbout { id: i } => s("getArtistInfo2", pairs(&[("id", i), ("count", "10".into())]), Some(DAY), Parser::ArtistInfo),
        Read::TopSongs { artist } => s("getTopSongs", pairs(&[("artist", artist), ("count", "20".into())]), Some(DAY), Parser::Songs),
        Read::PlaylistList => s("getPlaylists", vec![], Some(BROWSE), Parser::Playlists),
        Read::PlaylistById { id: i } => s("getPlaylist", id(i), Some(BROWSE), Parser::Playlist),
        Read::StarredItems => s("getStarred2", vec![], Some(BROWSE), Parser::Starred),
        Read::GenreList => s("getGenres", vec![], Some(HOUR), Parser::Genres),
        Read::RadioList => s("getInternetRadioStations", vec![], Some(HOUR), Parser::Radio),
        Read::LyricsBySong { song_id } => s("getLyricsBySongId", pairs(&[("id", song_id), ("enhanced", "true".into())]), Some(HOUR), Parser::Lyrics),
        Read::FolderIndex => s("getIndexes", vec![], Some(HOUR), Parser::Indexes),
        Read::FolderById { id: i } => s("getMusicDirectory", id(i), Some(BROWSE), Parser::Directory),
        Read::MusicFolders => s("getMusicFolders", vec![], None, Parser::Folders),
        Read::Ping => s("ping", vec![], None, Parser::Status),
        Read::Search { query, songs, albums, artists } => s(
            "search3",
            pairs(&[("query", query), ("songCount", songs.to_string()), ("albumCount", albums.to_string()), ("artistCount", artists.to_string())]),
            None,
            Parser::Search,
        ),
        Read::SongPage { offset, count } => s(
            "search3",
            pairs(&[("query", String::new()), ("songCount", count.to_string()), ("songOffset", offset.to_string()), ("albumCount", "0".into()), ("artistCount", "0".into())]),
            None,
            Parser::Search,
        ),
        Read::RandomSongs { size, genre } => {
            let mut p = pairs(&[("size", size.to_string())]);
            if let Some(g) = genre {
                p.push(("genre".into(), g));
            }
            s("getRandomSongs", p, None, Parser::Songs)
        }
        Read::SongsByGenre { genre, count } => s("getSongsByGenre", pairs(&[("genre", genre), ("count", count.to_string())]), None, Parser::Songs),
        Read::SimilarSongs { id: i, count } => s("getSimilarSongs2", pairs(&[("id", i), ("count", count.to_string())]), None, Parser::Songs),
        Read::SongById { id: i } => s("getSong", id(i), None, Parser::OneSong),
        Read::AlbumSongs { id: i } => s("getAlbum", id(i), None, Parser::AlbumSongs),
        Read::PlaylistSongs { id: i } => s("getPlaylist", id(i), None, Parser::PlaylistSongs),
        Read::NowPlaying { id: i } => s("scrobble", pairs(&[("id", i), ("submission", "false".into())]), None, Parser::Status),
        Read::ShareLink { id: i } => s("createShare", id(i), None, Parser::Share),
        Read::PullQueue => s("getPlayQueue", vec![], None, Parser::Queue),
        Read::HomeFeed => s("homeFeed", vec![], Some(BROWSE), Parser::Feed),
    }
}

/// Cache key: the endpoint then `&k=v` for each sent param (folder included), in order, unencoded.
/// Eviction prefixes rely on this shape ("getAlbumList2&type=starred").
fn key(endpoint: &str, params: &[(String, String)]) -> String {
    let mut k = String::with_capacity(endpoint.len() + params.iter().map(|(a, b)| a.len() + b.len() + 2).sum::<usize>());
    k.push_str(endpoint);
    for (a, b) in params {
        k.push('&');
        k.push_str(a);
        k.push('=');
        k.push_str(b);
    }
    k
}

fn digest(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Whether `page` lists a provider item. Such answers change as provider songs get downloaded, so they
/// are never fresh.
fn lists_provider_items(page: &Page) -> bool {
    match page {
        Page::AlbumPage { v } => v.album.is_provider() || v.songs.iter().any(Song::is_provider),
        Page::ArtistPage { v } => v.albums.iter().any(Album::is_provider),
        Page::PlaylistPage { v } => v.songs.iter().any(Song::is_provider),
        Page::Songs { v } => v.iter().any(Song::is_provider),
        Page::Albums { v } => v.iter().any(Album::is_provider),
        _ => false,
    }
}

/// The Subsonic error for an item the server does not have.
const NOT_FOUND: i32 = 70;

impl Client {
    fn parse(&self, parser: Parser, body: Vec<u8>) -> NetResult<Page> {
        let r = crate::parse(&body)?;
        self.core.heard_from_server(&r.kind);
        let stars = || self.core.stars.lock();
        Ok(match parser {
            Parser::AlbumList => Page::Albums { v: r.album_list2.unwrap_or_default().album },
            Parser::FavouriteAlbums => Page::Albums { v: stars().overlay_albums(r.album_list2.unwrap_or_default().album) },
            Parser::Artists => Page::Artists { v: r.artists.unwrap_or_default().artists() },
            Parser::Indexes => Page::Artists { v: r.indexes.unwrap_or_default().artists() },
            Parser::Album => Page::AlbumPage { v: r.album.unwrap_or_default().into() },
            Parser::AlbumSongs => Page::Songs { v: r.album.unwrap_or_default().song },
            Parser::Artist => Page::ArtistPage { v: r.artist.unwrap_or_default().into() },
            Parser::ArtistInfo => Page::About { v: r.artist_info2.unwrap_or_default().into() },
            Parser::Songs => Page::Songs { v: r.songs() },
            Parser::OneSong => Page::OneSong { v: r.songs().into_iter().next() },
            Parser::Playlists => Page::Playlists { v: r.playlists.unwrap_or_default().playlist },
            Parser::Playlist => Page::PlaylistPage { v: r.playlist.unwrap_or_default().into() },
            Parser::PlaylistSongs => Page::Songs { v: r.playlist.unwrap_or_default().entry },
            // This session's star changes apply to cached and fresh answers alike.
            Parser::Starred => Page::StarredPage { v: self.marked_favourites(r.starred2.unwrap_or_default().into()) },
            Parser::Genres => {
                let mut v = r.genres.unwrap_or_default().genre;
                v.sort_by_key(|g| std::cmp::Reverse(g.song_count));
                Page::Genres { v }
            }
            Parser::Radio => Page::Stations { v: r.internet_radio_stations.unwrap_or_default().station },
            Parser::Lyrics => {
                Page::LyricsPage { v: crate::lyrics::build(r.lyrics_list.unwrap_or_default().structured_lyrics) }
            }
            Parser::Directory => Page::DirectoryPage { v: r.directory.unwrap_or_default().into() },
            Parser::Search => Page::Found { v: r.search_result3.unwrap_or_default().into() },
            Parser::Folders => Page::Folders { v: r.music_folders.unwrap_or_default().music_folder },
            Parser::Status => Page::Status { v: r.into() },
            Parser::Share => {
                let url = r.shares.and_then(|s| s.share.into_iter().next()).map(|s| s.url).filter(|u| !u.is_empty());
                Page::ShareUrl { v: url.ok_or(crate::CoreError::Parse { reason: "no share in response".into() })? }
            }
            Parser::Queue => {
                let q = r.play_queue.unwrap_or_default();
                let by = q.changed_by.clone();
                Page::Queue { v: q.into(), by }
            }
            Parser::Feed => Page::Feed { v: Vec::new() },
        })
    }

    /// Parses a server answer and keeps its library items in the index. Cached answers are only parsed:
    /// they were indexed when they came.
    fn parse_fresh(&self, parser: Parser, body: Vec<u8>) -> NetResult<Page> {
        let page = self.parse(parser, body)?;
        self.index(&page, matches!(parser, Parser::Indexes))?;
        Ok(page)
    }

    /// Keeps `page`'s library items in the index; `folders` when it lists folders, not artists.
    fn index(&self, page: &Page, folders: bool) -> NetResult<()> {
        let none: (&[Artist], &[Album], &[Song]) = (&[], &[], &[]);
        let (artists, albums, songs) = match page {
            _ if folders => none,
            Page::Albums { v } => (&[][..], v.as_slice(), &[][..]),
            Page::Artists { v } => (v.as_slice(), &[][..], &[][..]),
            Page::Songs { v } => (&[][..], &[][..], v.as_slice()),
            Page::OneSong { v } => (&[][..], &[][..], v.as_slice()),
            Page::AlbumPage { v } => (&[][..], std::slice::from_ref(&v.album), v.songs.as_slice()),
            Page::ArtistPage { v } => (std::slice::from_ref(&v.artist), v.albums.as_slice(), &[][..]),
            Page::PlaylistPage { v } => (&[][..], &[][..], v.songs.as_slice()),
            Page::StarredPage { v } => (v.artists.as_slice(), v.albums.as_slice(), v.songs.as_slice()),
            Page::Found { v } => (v.artists.as_slice(), v.albums.as_slice(), v.songs.as_slice()),
            _ => none,
        };
        if !(artists.is_empty() && albums.is_empty() && songs.is_empty()) {
            crate::db::index(&mut self.core.db.lock(), artists, albums, songs)?;
        }
        Ok(())
    }

    /// The page `read` answers when there is nothing to answer: what a server gives for an empty answer.
    fn nothing(&self, read: Read) -> NetResult<Page> {
        self.parse(spec(read).parser, br#"{"subsonic-response":{"status":"ok"}}"#.to_vec())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Asks the profile's source, bypassing the cache; what it lists is indexed.
    pub async fn read_now(&self, read: Read) -> NetResult<Page> {
        let folders = matches!(read, Read::FolderIndex);
        let page = match self.core.provider().read(self, read.clone()).await? {
            Some(page) => page,
            None => self.nothing(read)?,
        };
        self.index(&page, folders)?;
        Ok(page)
    }
}

impl Client {
    /// The kept page for `read` (None if absent or unparseable) and whether it is fresh.
    pub(crate) fn read_stored(&self, read: Read) -> NetResult<Stored> {
        self.core.provider().read_stored(self, read, None)
    }

    /// [`Client::read_stored`] with a custom freshness window (for cached reads only).
    pub(crate) fn read_stored_within(&self, read: Read, fresh_ms: i64) -> NetResult<Stored> {
        self.core.provider().read_stored(self, read, Some(fresh_ms))
    }

    /// The cached page while fresh, else the source's, kept; the stale one when it can't be reached.
    pub(crate) async fn cached_or_fetched(&self, read: Read) -> NetResult<Option<Page>> {
        let stored = self.read_stored(read.clone())?;
        if stored.fresh {
            return Ok(stored.page);
        }
        Ok(self.read_refresh(read, stored.digest).await?.or(stored.page))
    }

    /// Asks the source; returns the page only if it differs from `stored_digest`, and keeps it either
    /// way (restarting the window). Uncached reads always return their page.
    pub(crate) async fn read_fetch(&self, read: Read, stored_digest: Option<u64>) -> NetResult<Option<Page>> {
        self.core.provider().read_fetch(self, read, stored_digest).await
    }

    /// `read` asked of the Subsonic server.
    pub(crate) async fn subsonic_read(&self, read: Read) -> NetResult<Page> {
        let sp = spec(read);
        let body = self.fetch(sp.endpoint, sp.params).await?;
        self.parse(sp.parser, body)
    }

    /// The server's cached answer for `read`, fresh within `within_ms` (None: the read's own window).
    pub(crate) fn subsonic_stored(&self, read: Read, within_ms: Option<i64>) -> NetResult<Stored> {
        let sp = spec(read);
        let Some(fresh_ms) = sp.fresh_ms.map(|ms| within_ms.unwrap_or(ms)) else { return Ok(Stored { page: None, digest: None, fresh: false }) };
        let k = key(sp.endpoint, &self.scoped(sp.endpoint, sp.params));
        let row: Option<(Vec<u8>, i64)> = {
            let c = self.core.db.lock();
            let mut st = c.prepare_cached("SELECT body, ts FROM cache WHERE server=sid() AND key=?1")?;
            st.query_row([k], |r| Ok((r.get(0)?, r.get(1)?))).optional()?
        };
        let Some((body, ts)) = row else { return Ok(Stored { page: None, digest: None, fresh: false }) };
        let digest = Some(digest(&body));
        let page = self.parse(sp.parser, body).ok();
        let fresh = page.as_ref().is_some_and(|p| !lists_provider_items(p)) && crate::db::now_ms() - ts < fresh_ms;
        Ok(Stored { page, digest, fresh })
    }

    /// [`Client::read_stored`] of a provider that stores nothing: its answer kept in memory this session,
    /// fresh for the read's window, so a page read again (Home scrolled back to, a page left and
    /// returned to) shows what it showed.
    pub(crate) fn kept_stored(&self, read: Read, within_ms: Option<i64>) -> Stored {
        let sp = spec(read);
        let Some(fresh_ms) = sp.fresh_ms.map(|ms| within_ms.unwrap_or(ms)) else { return Stored { page: None, digest: None, fresh: false } };
        let kept = self.core.kept.lock().get(&key(sp.endpoint, &sp.params)).cloned();
        let fresh = kept.as_ref().is_some_and(|(_, at)| crate::db::now_ms() - at < fresh_ms);
        let page = kept.map(|(p, _)| match p {
            Page::StarredPage { v } => Page::StarredPage { v: self.marked_favourites(v) },
            p => p,
        });
        Stored { page, digest: None, fresh }
    }

    /// A source's favourites with this session's marks laid over them: unstarred items leave, and a
    /// song starred that they lack yet (YouTube lists a like seconds later) leads them when its record
    /// is known.
    pub(crate) fn marked_favourites(&self, starred: Starred) -> Starred {
        let marks = self.core.stars.lock().clone();
        let starred = marks.overlay(starred);
        let mut missing: Vec<&String> = marks.songs.iter().filter(|(id, on)| **on && !starred.songs.iter().any(|s| &s.id == *id)).map(|(id, _)| id).collect();
        missing.sort();
        let mut songs: Vec<Song> =
            missing.into_iter().filter_map(|id| self.stored_song_of(id).or_else(|| self.core.indexed_song(id))).map(|s| Song { starred: true, ..s }).collect();
        if songs.is_empty() {
            return starred;
        }
        songs.extend(starred.songs);
        Starred::new(starred.artists, starred.albums, songs)
    }

    /// [`Client::read_fetch`] of a provider that stores nothing: asked anew, and kept ([`Client::kept_stored`]).
    pub(crate) async fn kept_fetch(&self, read: Read) -> NetResult<Option<Page>> {
        let page = self.read_now(read.clone()).await?;
        self.keep(read, page.clone());
        Ok(Some(page))
    }

    /// Keeps `page` as `read`'s answer this session ([`Client::kept_stored`]); cached reads only.
    pub(crate) fn keep(&self, read: Read, page: Page) {
        let sp = spec(read);
        if sp.fresh_ms.is_some() {
            self.core.kept.lock().insert(key(sp.endpoint, &sp.params), (page, crate::db::now_ms()));
        }
    }

    /// [`Client::read_fetch`] of the Subsonic server, whose answers are cached whole.
    pub(crate) async fn subsonic_fetch(&self, read: Read, stored_digest: Option<u64>) -> NetResult<Option<Page>> {
        let sp = spec(read);
        if sp.fresh_ms.is_none() {
            let body = self.fetch(sp.endpoint, sp.params).await?;
            return Ok(Some(self.parse_fresh(sp.parser, body)?));
        }
        let k = key(sp.endpoint, &self.scoped(sp.endpoint, sp.params.clone()));
        let revision = self.core.cache_revision.load(std::sync::atomic::Ordering::Relaxed);
        let body = self.fetch(sp.endpoint, sp.params).await?;
        let page = if stored_digest != Some(digest(&body)) {
            match self.parse_fresh(sp.parser, body.clone()) {
                Err(e @ crate::transport::NetError::Api { code: NOT_FOUND, .. }) => {
                    self.core.cache_drop(&k)?;
                    return Err(e);
                }
                page => Some(page?),
            }
        } else {
            None
        };
        self.core.cache_put_checked(k, body, Some(revision))?;
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, client, Fake};
    use crate::client::NetProfile;
    use crate::transport::FailureKind;
    use std::sync::Arc;

    const GENRES: &str = r#"{"subsonic-response":{"status":"ok","genres":{"genre":[{"value":"Rock","songCount":3}]}}}"#;
    const GENRES2: &str = r#"{"subsonic-response":{"status":"ok","genres":{"genre":[{"value":"Jazz","songCount":1}]}}}"#;

    #[test]
    fn a_read_before_a_star_cannot_restore_its_evicted_cache() {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};
        use crate::client::{Starrable, Write};
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        *fake.pends.lock() = true;
        let mut old = std::pin::pin!(c.read_fetch(Read::StarredItems, None));
        assert!(matches!(old.as_mut().poll(&mut Context::from_waker(Waker::noop())), Poll::Pending));
        fake.answer(crate::client::tests::OK);
        block(c.write(Write::Star { kind: Starrable::Song, id: "s".into(), on: true })).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","starred2":{}}}"#);
        block(old).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","starred2":{"song":[{"id":"s"}]}}}"#);
        let page = block(c.cached_or_fetched(Read::StarredItems)).unwrap().unwrap();
        assert_eq!(page.songs().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s"]);
    }

    #[test]
    fn a_navidrome_answer_says_transcodes_start_where_asked() {
        let (c, fake) = setup();
        assert!(!c.core.starts_transcodes());
        fake.answer(r#"{"subsonic-response":{"status":"ok","type":"gonic","genres":{"genre":[]}}}"#);
        block(c.read_fetch(Read::GenreList, None)).unwrap();
        assert!(!c.core.starts_transcodes(), "another server: transcodes are read from their start");
        fake.answer(r#"{"subsonic-response":{"status":"ok","type":"navidrome","genres":{"genre":[]}}}"#);
        block(c.read_fetch(Read::GenreList, None)).unwrap();
        assert!(c.core.starts_transcodes());
    }

    fn genres(p: &Option<Page>) -> String {
        match p {
            Some(Page::Genres { v }) => v.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(","),
            other => panic!("{other:?}"),
        }
    }

    fn setup() -> (Arc<Client>, Arc<Fake>) {
        client(NetProfile { url: "h".into(), music_folder_ids: vec!["7".into()], ..Default::default() })
    }

    #[test]
    fn freshness() {
        let (c, fake) = setup();
        let s = c.read_stored(Read::GenreList).unwrap();
        assert!(s.page.is_none() && s.digest.is_none() && !s.fresh);
        fake.answer(GENRES);
        assert_eq!(genres(&block(c.read_fetch(Read::GenreList, s.digest)).unwrap()), "Rock");

        let s = c.read_stored(Read::GenreList).unwrap();
        assert_eq!(genres(&s.page), "Rock");
        assert!(s.fresh);
        assert_eq!(fake.asked().len(), 1);

        // Unchanged answer returns none and refreshes.
        let (c, fake) = setup();
        c.core.cache_put("getGenres".into(), GENRES.into()).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        let s = c.read_stored(Read::GenreList).unwrap();
        assert!(s.page.is_some() && !s.fresh);
        fake.answer(GENRES);
        assert!(block(c.read_fetch(Read::GenreList, s.digest)).unwrap().is_none());
        assert!(c.read_stored(Read::GenreList).unwrap().fresh);

        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        fake.answer(GENRES2);
        assert_eq!(genres(&block(c.read_fetch(Read::GenreList, s.digest)).unwrap()), "Jazz");

        // Deleted page is dropped.
        let (c, fake) = setup();
        let read = || Read::AlbumById { id: "al-1".into() };
        fake.answer(r#"{"subsonic-response":{"status":"ok","album":{"id":"al-1","name":"A","song":[{"id":"s1","title":"t"}]}}}"#);
        block(c.read_fetch(read(), None)).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"not found"}}}"#);
        let mut shown = 0;
        assert!(matches!(block(c.read_each(read(), |_| shown += 1)), Err(crate::transport::NetError::Api { code: 70, .. })));
        assert_eq!(shown, 1, "the cached page, before the server said");
        assert!(c.read_stored(read()).unwrap().page.is_none());
        // Offline, the cached page stands.
        fake.answer(r#"{"subsonic-response":{"status":"ok","album":{"id":"al-1","name":"A","song":[]}}}"#);
        block(c.read_fetch(read(), None)).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        fake.fail(FailureKind::Connect);
        assert!(block(c.read_each(read(), |_| {})).is_ok());

        // Unreadable cache is stale.
        let (c, _) = setup();
        c.core.cache_put("getGenres".into(), b"garbage".to_vec()).unwrap();
        let s = c.read_stored(Read::GenreList).unwrap();
        assert!(s.page.is_none() && s.digest.is_some() && !s.fresh);
    }

    #[test]
    fn uncached_reads() {
        let (c, fake) = setup();
        let song = |ext: bool| format!(r#"{{"id":"s{ext}","title":"t","isExternal":{ext}}}"#);
        let album = |ext: bool| format!(r#"{{"id":"a{ext}","name":"A","isExternal":{ext}}}"#);
        // Each read, its answer around ITEM, and whether the item is a song.
        let pages = [
            (Read::AlbumById { id: "x".into() }, r#""album":{"id":"al","name":"A","song":[ITEM]}"#, true),
            (Read::AlbumById { id: "y".into() }, r#""album":ITEM"#, false),
            (Read::ArtistById { id: "x".into() }, r#""artist":{"id":"ar","name":"A","album":[ITEM]}"#, false),
            (Read::PlaylistById { id: "x".into() }, r#""playlist":{"id":"p","name":"P","entry":[ITEM]}"#, true),
            (Read::TopSongs { artist: "x".into() }, r#""topSongs":{"song":[ITEM]}"#, true),
            (Read::AlbumList { kind: crate::browse::AlbumSort::Newest, size: 5, offset: 0, genre: None }, r#""albumList2":{"album":[ITEM]}"#, false),
        ];
        for (read, answer, songs) in pages {
            for provider in [true, false] {
                let item = if songs { song(provider) } else { album(provider) };
                fake.answer(&format!(r#"{{"subsonic-response":{{"status":"ok",{}}}}}"#, answer.replace("ITEM", &item)));
                block(c.read_fetch(read.clone(), None)).unwrap();
                let s = c.read_stored(read.clone()).unwrap();
                assert!(s.page.is_some() && s.fresh != provider, "{read:?}, a provider item: {provider}");
            }
        }

        // Random is uncached and by year is this year.
        let (c, fake) = setup();
        let random = Read::AlbumList { kind: crate::browse::AlbumSort::Random, size: 5, offset: 0, genre: Some("Rock".into()) };
        assert!(c.read_stored(random.clone()).unwrap().digest.is_none());
        let list = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"a","name":"A"}]}}}"#;
        fake.answer(list);
        assert!(block(c.read_fetch(random.clone(), None)).unwrap().is_some());
        fake.answer(list);
        assert!(block(c.read_fetch(random, None)).unwrap().is_some());
        assert_eq!(c.core.db.lock().query_row("SELECT count(*) FROM cache", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert!(fake.asked()[0].ends_with("&type=random&size=5&offset=0&genre=Rock&musicFolderId=7"));

        fake.answer(list);
        block(c.read_fetch(Read::AlbumList { kind: crate::browse::AlbumSort::ByYear, size: 50, offset: 0, genre: None }, None)).unwrap();
        let year = this_year();
        assert!(fake.asked()[2].ends_with(&format!("&type=byYear&fromYear={year}&toYear=0&size=50&offset=0&musicFolderId=7")));

        // A page of every song, as a profile without the offline index lists them.
        fake.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"song":[{"id":"s9","title":"t"}]}}}"#);
        let page = block(c.read_now(Read::SongPage { offset: 100, count: 50 })).unwrap();
        assert!(matches!(page, Page::Found { v } if v.songs.iter().map(|s| s.id.as_str()).eq(["s9"])));
        assert!(fake.asked()[3].ends_with("&query=&songCount=50&songOffset=100&albumCount=0&artistCount=0&musicFolderId=7"), "{}", fake.asked()[3]);
    }

    #[test]
    fn a_guest_lists_the_servers_songs_and_an_account_its_index() {
        let (c, fake) = setup();
        c.core.configure(crate::ServerConfig { url: "h".into(), api_key: Some(nori_remote::guest_key("k")), ..Default::default() }).unwrap();
        assert!(c.core.rules().asks);
        fake.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"song":[{"id":"s9","title":"t"}]}}}"#);
        let page = block(c.songs_listed("title".into(), false, 0, 0, 0)).unwrap();
        assert!(page.exhausted && page.songs.iter().map(|s| s.id.as_str()).eq(["s9"]));
        assert!(fake.asked()[0].contains("search3?") && fake.asked()[0].contains("&query=&songCount="), "{}", fake.asked()[0]);

        c.core.configure(crate::ServerConfig { url: "h".into(), user: "ann".into(), password: "pw".into(), ..Default::default() }).unwrap();
        assert!(!c.core.rules().asks);
        assert!(block(c.songs_listed("title".into(), false, 0, 0, 0)).unwrap().songs.is_empty(), "nothing indexed");
        assert_eq!(fake.asked().len(), 1, "the index, not the server");
    }

    #[test]
    fn keys_hold_params_and_folder() {
        let (c, fake) = setup();
        fake.answer(r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[]}}}"#);
        block(c.read_fetch(Read::AlbumList { kind: crate::browse::AlbumSort::Starred, size: 20, offset: 0, genre: None }, None)).unwrap();
        assert!(c.core.cache_get("getAlbumList2&type=starred&size=20&offset=0&musicFolderId=7".into()).unwrap().is_some());
        assert!(fake.asked()[0].ends_with("&type=starred&size=20&offset=0&musicFolderId=7"));
        fake.answer(r#"{"subsonic-response":{"status":"ok","lyricsList":{}}}"#);
        block(c.read_fetch(Read::LyricsBySong { song_id: "s 1".into() }, None)).unwrap();
        assert!(c.core.cache_get("getLyricsBySongId&id=s 1&enhanced=true".into()).unwrap().is_some());
    }

    #[test]
    fn favourites_apply_session_stars() {
        let (c, fake) = setup();
        fake.answer(r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"fa-1","name":"A"},{"id":"fa-2","name":"B"}]}}}"#);
        block(c.read_fetch(Read::AlbumList { kind: crate::browse::AlbumSort::Starred, size: 20, offset: 0, genre: None }, None)).unwrap();
        c.core.stars.lock().mark(crate::client::Starrable::Album, "fa-2".into(), false);
        match c.read_stored(Read::FavouriteAlbums { size: 20 }).unwrap().page {
            Some(Page::Albums { v }) => assert_eq!(v.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["fa-1"]),
            other => panic!("{other:?}"),
        }
        match c.read_stored(Read::AlbumList { kind: crate::browse::AlbumSort::Starred, size: 20, offset: 0, genre: None }).unwrap().page {
            Some(Page::Albums { v }) => assert_eq!(v.len(), 2, "plain AlbumList is not overlaid"),
            other => panic!("{other:?}"),
        }
    }

    /// A heart shows in the favourites at once, while the server's answer does not list it yet.
    #[test]
    fn a_heart_shows_in_the_favourites_before_the_server_lists_it() {
        let (c, fake) = setup();
        c.core.session.register(vec![Song { id: "s2".into(), title: "Two".into(), ..Default::default() }]);
        fake.answer(crate::client::tests::OK);
        block(c.star(crate::client::Starrable::Song, "s2".into(), true, Arc::new(crate::providers::harness::Marked))).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","starred2":{"song":[{"id":"s1","title":"One"}]}}}"#);
        let Some(Page::StarredPage { v }) = block(c.cached_or_fetched(Read::StarredItems)).unwrap() else { panic!() };
        assert_eq!(v.songs.iter().map(|s| (s.id.as_str(), s.starred)).collect::<Vec<_>>(), [("s2", true), ("s1", false)]);
    }

    #[test]
    fn failures_reach_the_caller() {
        let (c, fake) = setup();
        fake.fail(FailureKind::Connect);
        assert!(block(c.read_fetch(Read::GenreList, None)).is_err());
        fake.answer(r#"{"subsonic-response":{"status":"ok","song":{"id":"1","title":"t"}}}"#);
        match block(c.read_now(Read::SongById { id: "1".into() })).unwrap() {
            Page::OneSong { v } => assert_eq!(v.unwrap().title, "t"),
            other => panic!("{other:?}"),
        }
    }
}
