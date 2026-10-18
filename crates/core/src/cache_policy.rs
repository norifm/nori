//! The pages every provider's reads make, and how they are kept: the store all providers share.
//! [`Client::read_stored`] returns the kept page at once (and whether it is fresh enough not to ask the
//! source); [`Client::read_fetch`] asks the source and returns a page only if it changed. A page is kept
//! under nori's own key for its read, on disk when the source answers raw ([`Answer::Raw`], parsed again
//! on every read) and in memory for the session otherwise. The session's marks lay over every page as it
//! is read; a write makes the pages of its kind stale ([`stale`]); Home stays put for the session and a
//! refresh merges into what was shown.

use std::hash::{Hash, Hasher};

use rusqlite::OptionalExtension;

use crate::browse::AlbumSort;
use crate::client::{Client, NetResult, Write};
use crate::providers::Answer;
use crate::{
    Album, AlbumDetail, Artist, ArtistDetail, ArtistInfo, Directory, FeedShelf, Genre, Lyrics, MusicFolder, PlayQueue, Playlist, PlaylistDetail, RadioStation, SearchResult,
    ServerInfo, Song, Starred,
};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
/// Freshness window of most reads.
const BROWSE: i64 = 2 * MINUTE;

/// A read of the profile's source. Variants up to `FolderById`, and `HomeFeed`, are kept; the rest
/// always ask.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Read {
    /// Albums in `kind`'s order. `ByYear` means this year; `Random` is never kept.
    AlbumList { kind: AlbumSort, size: i32, offset: i32, genre: Option<String> },
    AlbumsByYear { from: i32, to: i32, size: i32, offset: i32 },
    /// The starred `AlbumList` (the same kept page) with this session's star changes applied.
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
    /// With word cues and translations where the source has them.
    LyricsBySong { song_id: String },
    /// The folder tree, for libraries organised by directory.
    FolderIndex,
    FolderById { id: String },
    // Never kept:
    MusicFolders,
    Ping,
    /// Never kept, so octo-fiesta's provider results show; one big page since the proxy repeats them per offset.
    Search { query: String, songs: i32, albums: i32, artists: i32 },
    /// Every song of the source, `count` from `offset`, in its order: the songs list of a profile that
    /// keeps no offline index (a jam guest's).
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

/// The current local year.
pub(crate) fn this_year() -> i32 {
    let now = crate::db::now_ms();
    nori_library::smart::civil_from_ms(now + nori_library::library::local_offset_s(now / 1000) * 1000).0 as i32
}

/// The key `read`'s page is kept under, before the source's scope ([`crate::providers::Provider::scope`]),
/// and how long it is fresh; None for a read never kept. Writes and refreshes drop pages by key prefix
/// ([`stale`], [`crate::browse::home_refresh_drops`]).
pub(crate) fn kept_as(read: &Read) -> Option<(String, i64)> {
    let by_year = |from: i32, to: i32, size: i32, offset: i32| format!("page:albums/byYear/{from}/{to}/{size}/{offset}");
    Some(match read {
        Read::AlbumList { kind: AlbumSort::Random, .. } => return None,
        Read::AlbumList { kind: AlbumSort::ByYear, size, offset, .. } => (by_year(this_year(), 0, *size, *offset), BROWSE),
        Read::AlbumList { kind, size, offset, genre } => {
            (format!("page:albums/{}/{size}/{offset}{}", kind.api(), genre.as_ref().map(|g| format!("/{g}")).unwrap_or_default()), BROWSE)
        }
        Read::AlbumsByYear { from, to, size, offset } => (by_year(*from, *to, *size, *offset), BROWSE),
        Read::FavouriteAlbums { size } => (format!("page:albums/{}/{size}/0", AlbumSort::Starred.api()), BROWSE),
        Read::ArtistIndex => ("page:artists".into(), BROWSE),
        Read::AlbumById { id } => (format!("page:album/{id}"), BROWSE),
        Read::ArtistById { id } => (format!("page:artist/{id}"), BROWSE),
        Read::ArtistAbout { id } => (format!("page:about/{id}"), DAY),
        Read::TopSongs { artist } => (format!("page:top/{artist}"), DAY),
        Read::PlaylistList => ("page:playlists".into(), BROWSE),
        Read::PlaylistById { id } => (format!("page:playlist/{id}"), BROWSE),
        Read::StarredItems => ("page:starred".into(), BROWSE),
        Read::GenreList => ("page:genres".into(), HOUR),
        Read::RadioList => ("page:radio".into(), HOUR),
        Read::LyricsBySong { song_id } => (format!("page:lyrics/{song_id}"), HOUR),
        Read::FolderIndex => ("page:folders".into(), HOUR),
        Read::FolderById { id } => (format!("page:folder/{id}"), BROWSE),
        Read::HomeFeed => ("page:home".into(), BROWSE),
        Read::MusicFolders
        | Read::Ping
        | Read::Search { .. }
        | Read::SongPage { .. }
        | Read::RandomSongs { .. }
        | Read::SongsByGenre { .. }
        | Read::SimilarSongs { .. }
        | Read::SongById { .. }
        | Read::AlbumSongs { .. }
        | Read::PlaylistSongs { .. }
        | Read::NowPlaying { .. }
        | Read::ShareLink { .. }
        | Read::PullQueue => return None,
    })
}

/// The pages a write makes stale, as key prefixes, for every provider.
pub(crate) fn stale(w: &Write) -> &'static [&'static str] {
    match w {
        // What lists hearts, and the pages whose items show them.
        Write::Star { .. } => &["page:starred", "page:album/", "page:artist/", "page:playlist/", "page:albums/starred/"],
        Write::CreatePlaylist { .. } => &["page:playlists"],
        Write::AddToPlaylist { .. } | Write::RemoveFromPlaylist { .. } | Write::RenamePlaylist { .. } | Write::ReplacePlaylist { .. } | Write::DeletePlaylist { .. } => {
            &["page:playlist/", "page:playlists"]
        }
        Write::CreateRadio { .. } | Write::DeleteRadio { .. } => &["page:radio"],
        Write::Scrobble { .. } | Write::SaveQueue { .. } => &[],
    }
}

/// The page `read` makes of a source that has nothing for it.
fn empty(read: &Read) -> NetResult<Page> {
    Ok(match read {
        Read::AlbumList { .. } | Read::AlbumsByYear { .. } | Read::FavouriteAlbums { .. } => Page::Albums { v: Vec::new() },
        Read::ArtistIndex | Read::FolderIndex => Page::Artists { v: Vec::new() },
        Read::AlbumById { .. } => Page::AlbumPage { v: AlbumDetail::new(Album::default(), Vec::new(), Vec::new()) },
        Read::ArtistById { .. } => Page::ArtistPage { v: ArtistDetail::new(Artist::default(), Vec::new()) },
        Read::ArtistAbout { .. } => Page::About { v: ArtistInfo::default() },
        Read::TopSongs { .. } | Read::RandomSongs { .. } | Read::SongsByGenre { .. } | Read::SimilarSongs { .. } | Read::AlbumSongs { .. } | Read::PlaylistSongs { .. } => {
            Page::Songs { v: Vec::new() }
        }
        Read::SongById { .. } => Page::OneSong { v: None },
        Read::PlaylistList => Page::Playlists { v: Vec::new() },
        Read::PlaylistById { .. } => Page::PlaylistPage { v: PlaylistDetail::new(Playlist::default(), Vec::new()) },
        Read::StarredItems => Page::StarredPage { v: Starred::new(Vec::new(), Vec::new(), Vec::new()) },
        Read::GenreList => Page::Genres { v: Vec::<Genre>::new() },
        Read::RadioList => Page::Stations { v: Vec::<RadioStation>::new() },
        Read::LyricsBySong { .. } => Page::LyricsPage { v: crate::lyrics::build(Vec::new()) },
        Read::FolderById { .. } => Page::DirectoryPage { v: Directory::default() },
        Read::Search { .. } | Read::SongPage { .. } => Page::Found { v: SearchResult::default() },
        Read::MusicFolders => Page::Folders { v: Vec::<MusicFolder>::new() },
        Read::Ping | Read::NowPlaying { .. } => Page::Status { v: ServerInfo::default() },
        Read::ShareLink { .. } => return Err(crate::CoreError::Parse { reason: "no share in response".into() }.into()),
        Read::PullQueue => Page::Queue { v: PlayQueue::default(), by: String::new() },
        Read::HomeFeed => Page::Feed { v: Vec::new() },
    })
}

fn digest(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// What tells two kept pages apart.
fn page_digest(page: &Page) -> u64 {
    digest(format!("{page:?}").as_bytes())
}

/// `shown` with what `fresh` adds: items new to a shelf at its end, new shelves at the end, so nothing
/// shown moves or leaves.
fn merged(mut shown: Vec<FeedShelf>, fresh: Vec<FeedShelf>) -> Vec<FeedShelf> {
    fn add<T>(to: &mut Vec<T>, from: Vec<T>, id: impl Fn(&T) -> &str) {
        let new: Vec<T> = from.into_iter().filter(|f| !to.iter().any(|t| id(t) == id(f))).collect();
        to.extend(new);
    }
    for f in fresh {
        match shown.iter_mut().find(|s| s.title == f.title) {
            Some(s) => {
                add(&mut s.songs, f.songs, |x| &x.id);
                add(&mut s.albums, f.albums, |x| &x.id);
                add(&mut s.playlists, f.playlists, |x| &x.id);
                add(&mut s.artists, f.artists, |x| &x.id);
            }
            None => shown.push(f),
        }
    }
    shown
}

/// A page kept in memory for the session, when it came and its digest.
#[derive(Clone)]
pub(crate) struct Kept {
    page: Page,
    pub(crate) at: i64,
    digest: u64,
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

impl Client {
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

    /// `page` as `read` shows it this session: the favourites with this session's marks laid over them.
    fn overlaid(&self, read: &Read, page: Page) -> Page {
        match (read, page) {
            (_, Page::StarredPage { v }) => Page::StarredPage { v: self.marked_favourites(v) },
            (Read::FavouriteAlbums { .. }, Page::Albums { v }) => Page::Albums { v: self.core.stars.lock().overlay_albums(v) },
            (_, page) => page,
        }
    }

    /// A source's favourites with this session's marks laid over them: unstarred items leave, and a
    /// song starred that they lack yet (a server that has not had the write yet, YouTube listing a like
    /// seconds later) leads them when its record is known.
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

    /// The key `read`'s page is kept under, the source's scope included, and how long it is fresh.
    fn kept_key(&self, read: &Read) -> Option<(String, i64)> {
        kept_as(read).map(|(key, window)| (key + &self.core.provider().scope(self, read), window))
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Asks the profile's source, bypassing the kept pages; what it lists is indexed.
    pub async fn read_now(&self, read: Read) -> NetResult<Page> {
        let provider = self.core.provider();
        let page = match provider.read(self, read.clone()).await? {
            Some(Answer::Page(page)) => *page,
            Some(Answer::Raw(raw)) => provider.parse(self, &read, &raw)?,
            None => empty(&read)?,
        };
        self.index(&page, matches!(read, Read::FolderIndex))?;
        Ok(self.overlaid(&read, page))
    }
}

impl Client {
    /// The kept page for `read` (None if absent or unparseable) and whether it is fresh.
    pub(crate) fn read_stored(&self, read: Read) -> NetResult<Stored> {
        if matches!(read, Read::HomeFeed) {
            return self.home_stored();
        }
        self.stored(read, None)
    }

    /// [`Client::read_stored`] with a custom freshness window (for kept reads only).
    pub(crate) fn read_stored_within(&self, read: Read, fresh_ms: i64) -> NetResult<Stored> {
        self.stored(read, Some(fresh_ms))
    }

    /// The page kept for `read`, in memory or on disk, fresh within `within_ms` (None: the read's own
    /// window).
    fn stored(&self, read: Read, within_ms: Option<i64>) -> NetResult<Stored> {
        let Some((k, window)) = self.kept_key(&read) else { return Ok(Stored { page: None, digest: None, fresh: false }) };
        let window = within_ms.unwrap_or(window);
        let kept = self.core.kept.lock().get(&k).cloned();
        let (page, digest, at) = match kept {
            Some(kept) => (Some(kept.page), kept.digest, kept.at),
            None => {
                let row: Option<(Vec<u8>, i64)> = {
                    let c = self.core.db.lock();
                    let mut st = c.prepare_cached("SELECT body, ts FROM cache WHERE server=sid() AND key=?1")?;
                    st.query_row([k], |r| Ok((r.get(0)?, r.get(1)?))).optional()?
                };
                let Some((body, ts)) = row else { return Ok(Stored { page: None, digest: None, fresh: false }) };
                (self.core.provider().parse(self, &read, &body).ok(), digest(&body), ts)
            }
        };
        let fresh = page.as_ref().is_some_and(|p| !lists_provider_items(p)) && crate::db::now_ms() - at < window;
        Ok(Stored { page: page.map(|p| self.overlaid(&read, p)), digest: Some(digest), fresh })
    }

    /// Home stays as first shown for the whole session; after a refresh dropped it, the last one shows
    /// until the merged one comes.
    fn home_stored(&self) -> NetResult<Stored> {
        let kept = self.stored(Read::HomeFeed, Some(i64::MAX))?;
        if kept.page.is_some() {
            return Ok(kept);
        }
        Ok(Stored { page: self.core.home.lock().clone().map(|v| Page::Feed { v }), digest: None, fresh: false })
    }

    /// A source may reshuffle Home on every request: a refresh keeps what was shown and adds what is new.
    async fn home_fetch(&self, digest: Option<u64>) -> NetResult<Option<Page>> {
        let Page::Feed { v } = self.read_now(Read::HomeFeed).await? else { return Ok(None) };
        let shown = self.core.home.lock().take();
        let v = match shown {
            Some(shown) => merged(shown, v),
            None => v,
        };
        *self.core.home.lock() = Some(v.clone());
        let page = Page::Feed { v };
        let kept = page_digest(&page);
        if let Some((key, _)) = self.kept_key(&Read::HomeFeed) {
            self.core.kept.lock().insert(key, Kept { page: page.clone(), at: crate::db::now_ms(), digest: kept });
        }
        Ok((Some(kept) != digest).then_some(page))
    }

    /// The kept page while fresh, else the source's, kept; the stale one when it can't be reached.
    pub(crate) async fn cached_or_fetched(&self, read: Read) -> NetResult<Option<Page>> {
        let stored = self.read_stored(read.clone())?;
        if stored.fresh {
            return Ok(stored.page);
        }
        Ok(self.read_refresh(read, stored.digest).await?.or(stored.page))
    }

    /// Asks the source; returns the page only if it differs from the kept one of `stored_digest`, and
    /// keeps it either way (restarting the window). Reads never kept always return their page. A write
    /// meanwhile makes the answer stale: it is not kept then.
    pub(crate) async fn read_fetch(&self, read: Read, stored_digest: Option<u64>) -> NetResult<Option<Page>> {
        if matches!(read, Read::HomeFeed) {
            return self.home_fetch(stored_digest).await;
        }
        let Some((k, _)) = self.kept_key(&read) else { return self.read_now(read).await.map(Some) };
        let revision = self.core.cache_revision.load(std::sync::atomic::Ordering::Relaxed);
        let provider = self.core.provider();
        let folders = matches!(read, Read::FolderIndex);
        let page = match provider.read(self, read.clone()).await? {
            Some(Answer::Raw(body)) => {
                let page = if stored_digest != Some(digest(&body)) {
                    match provider.parse(self, &read, &body) {
                        Err(e) if provider.gone(&e) => {
                            self.core.cache_drop(&k)?;
                            return Err(e);
                        }
                        page => {
                            let page = page?;
                            self.index(&page, folders)?;
                            Some(page)
                        }
                    }
                } else {
                    None
                };
                self.core.cache_put_checked(k, body, Some(revision))?;
                page
            }
            answer => {
                let page = match answer {
                    Some(Answer::Page(page)) => *page,
                    _ => empty(&read)?,
                };
                self.index(&page, folders)?;
                let digest = page_digest(&page);
                self.core.keep_checked(k, Kept { page: page.clone(), at: crate::db::now_ms(), digest }, revision);
                (Some(digest) != stored_digest).then_some(page)
            }
        };
        Ok(page.map(|p| self.overlaid(&read, p)))
    }
}

impl crate::Core {
    /// Keeps `kept` under `key` unless a write since `revision` made what it shows stale.
    fn keep_checked(&self, key: String, kept: Kept, revision: u64) {
        let mut pages = self.kept.lock();
        if revision == self.cache_revision.load(std::sync::atomic::Ordering::Relaxed) {
            pages.insert(key, kept);
        }
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
        c.core.cache_put("page:genres".into(), GENRES.into()).unwrap();
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
        c.core.cache_put("page:genres".into(), b"garbage".to_vec()).unwrap();
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
        assert!(c.core.cache_get("page:albums/starred/20/0&musicFolderId=7".into()).unwrap().is_some());
        assert!(fake.asked()[0].ends_with("&type=starred&size=20&offset=0&musicFolderId=7"));
        fake.answer(r#"{"subsonic-response":{"status":"ok","lyricsList":{}}}"#);
        block(c.read_fetch(Read::LyricsBySong { song_id: "s 1".into() }, None)).unwrap();
        assert!(c.core.cache_get("page:lyrics/s 1".into()).unwrap().is_some());
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

    /// A heart queued offline leaves the favourites and the album's page readable offline, stale: the
    /// server is asked for them once it answers.
    #[test]
    fn an_offline_heart_keeps_its_pages_readable() {
        let (c, fake) = setup();
        fake.answer(r#"{"subsonic-response":{"status":"ok","starred2":{"song":[{"id":"s1","title":"One"}]}}}"#);
        block(c.read_fetch(Read::StarredItems, None)).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","album":{"id":"al1","name":"A","song":[{"id":"s2","title":"Two"}]}}}"#);
        block(c.read_fetch(Read::AlbumById { id: "al1".into() }, None)).unwrap();
        fake.fail(FailureKind::Connect);
        block(c.star(crate::client::Starrable::Song, "s2".into(), true, Arc::new(crate::providers::harness::Marked))).unwrap();
        assert!(c.core.pending_any().unwrap(), "queued");
        for read in [Read::StarredItems, Read::AlbumById { id: "al1".into() }] {
            let stored = c.read_stored(read.clone()).unwrap();
            assert!(stored.page.is_some() && !stored.fresh, "{read:?}");
            assert!(block(c.cached_or_fetched(read.clone())).unwrap().is_some(), "{read:?} offline");
        }
    }

    /// A server has no home feed: Home is the empty feed, and nothing is asked.
    #[test]
    fn a_server_is_not_asked_for_a_feed() {
        let (c, fake) = setup();
        assert!(matches!(block(c.cached_or_fetched(Read::HomeFeed)), Ok(Some(Page::Feed { v })) if v.is_empty()));
        assert!(fake.asked().is_empty());
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
