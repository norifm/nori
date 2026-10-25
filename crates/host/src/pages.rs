//! The library's pages as the clients read them, each on a thread of its own: the stored copy first,
//! then the server's when it differs. Home's shelves come in the core's order (`browse::home_shelves`).

use nori_core::browse::{favorites_in_order, home_pinned, home_row_more, home_row_page, home_rows_shown, home_shelves, shelf_albums_read, AlbumSort, HomeShelf};
use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::mixes::board::{MixLookup, MixSheet, MixTile};
use nori_core::settings::HomeRow;
use nori_core::transport::{block_on, NetError};
use nori_core::{Album, AlbumDetail, Artist, ArtistDetail, CoreError, FeedShelf, Playlist, PlaylistDetail, Song, Starred};

use crate::session::{read_pages, Handle};

/// A page a client reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageAsk {
    /// The albums in `sort`'s order, a page from `offset` (more follow while pages come full); with
    /// `favorites`, the starred ones only, all on one page (`browse::next_album_page`).
    Albums { sort: AlbumSort, offset: u32, favorites: bool },
    Artists,
    Playlists,
    /// What the account has starred.
    Favorites,
    /// The offline index's songs by title, a page from `offset`.
    Songs { offset: u32 },
    Album(String),
    DownloadedAlbum(String),
    Artist(String),
    Playlist(String),
    Mix(String),
    /// A Home row in full ("see all"): its page from `offset` (`browse::home_row_page`).
    HomeRow { row: HomeRow, offset: u32 },
}

/// What a page read brings.
pub enum PageGot {
    Albums(Vec<Album>),
    Artists(Vec<Artist>),
    Playlists(Vec<Playlist>),
    Favorites(Box<Starred>),
    Songs { songs: Vec<Song>, exhausted: bool },
    Album(Box<AlbumDetail>),
    Artist(Box<ArtistDetail>),
    Playlist(Box<PlaylistDetail>),
    Mix(Box<MixSheet>),
}

/// Why a page has nothing to show.
#[derive(Debug)]
pub enum PageFailed {
    /// The server answered with nothing for it.
    Nothing,
    Net(NetError),
    /// The album is not downloaded any more.
    NotDownloaded,
    Stored(CoreError),
    /// The mix has no songs yet.
    EmptyMix,
}

/// Albums asked for a page of the album list.
pub const ALBUM_PAGE: u32 = 500;

/// What Home shows, as it comes.
pub enum HomePart {
    /// "For you": the favourites' tile, then the mixes; empty without an account and on a source's
    /// own Home. Sent first, and again
    /// once the mixes are drawn, when they changed.
    Mixes(Vec<MixTile>),
    /// A source's own Home (`ProfileRules::feed`): its shelves.
    Feed(Vec<FeedShelf>),
    /// Shelf `at` of the rows asked for, with what it holds; a shelf with nothing in it comes too.
    Shelf { at: usize, items: ShelfItems },
}

/// What a Home shelf holds.
pub enum ShelfItems {
    Albums(Vec<Album>),
    Playlists(Vec<Playlist>),
    Songs(Vec<Song>),
}

impl Handle {
    /// Reads `ask` on a thread of its own; `each` gets every answer, or why there is none.
    pub fn read_page(&self, ask: PageAsk, each: impl Fn(Result<PageGot, PageFailed>) + Send + 'static) {
        let me = self.clone();
        crate::spawn("nori-read", move || me.page(ask, &each));
    }

    fn page(&self, ask: PageAsk, each: &dyn Fn(Result<PageGot, PageFailed>)) {
        let pages = |read: Read, make: &dyn Fn(Page) -> Option<PageGot>| {
            let mut any = false;
            let r = read_pages(&self.client, read, |p| {
                if let Some(got) = make(p) {
                    any = true;
                    each(Ok(got));
                }
            });
            match r {
                Ok(()) if !any => each(Err(PageFailed::Nothing)),
                Ok(()) => {}
                Err(e) => each(Err(PageFailed::Net(e))),
            }
        };
        match ask {
            PageAsk::Albums { sort, favorites: true, .. } => {
                pages(Read::StarredItems, &|p| if let Page::StarredPage { v } = p { Some(PageGot::Albums(favorites_in_order(v.albums, sort))) } else { None });
            }
            PageAsk::Albums { sort, offset, favorites: false } => {
                let read = Read::AlbumList { kind: sort, size: ALBUM_PAGE as i32, offset: offset as i32, genre: None };
                pages(read, &|p| if let Page::Albums { v } = p { Some(PageGot::Albums(v)) } else { None });
            }
            PageAsk::Artists => pages(Read::ArtistIndex, &|p| if let Page::Artists { v } = p { Some(PageGot::Artists(v)) } else { None }),
            PageAsk::Favorites => pages(Read::StarredItems, &|p| if let Page::StarredPage { v } = p { Some(PageGot::Favorites(Box::new(v))) } else { None }),
            PageAsk::Playlists => pages(Read::PlaylistList, &|p| if let Page::Playlists { v } = p { Some(PageGot::Playlists(v)) } else { None }),
            PageAsk::Album(id) => pages(Read::AlbumById { id }, &|p| if let Page::AlbumPage { v } = p { Some(PageGot::Album(Box::new(v))) } else { None }),
            PageAsk::Artist(id) => pages(Read::ArtistById { id }, &|p| if let Page::ArtistPage { v } = p { Some(PageGot::Artist(Box::new(v))) } else { None }),
            PageAsk::Playlist(id) => pages(Read::PlaylistById { id }, &|p| if let Page::PlaylistPage { v } = p { Some(PageGot::Playlist(Box::new(v))) } else { None }),
            PageAsk::DownloadedAlbum(id) => each(match self.core.downloaded_album(id) {
                Ok(Some(a)) => Ok(PageGot::Album(Box::new(a))),
                Ok(None) => Err(PageFailed::NotDownloaded),
                Err(e) => Err(PageFailed::Stored(e)),
            }),
            PageAsk::Songs { offset } => each(match block_on(self.client.songs_listed("title".into(), false, 0, 0, offset)) {
                Ok(p) => Ok(PageGot::Songs { songs: p.songs, exhausted: p.exhausted }),
                Err(e) => Err(PageFailed::Net(e)),
            }),
            PageAsk::HomeRow { row, offset } => match home_row_page(row, offset).map(|s| (s.clone(), s)) {
                // Nothing past `offset`: the row is all here.
                None => each(Err(PageFailed::Nothing)),
                Some((_, HomeShelf::Albums { sort, size, follows_stars })) => {
                    pages(shelf_albums_read(sort, size, follows_stars, offset), &|p| if let Page::Albums { v } = p { Some(PageGot::Albums(v)) } else { None });
                }
                Some((shelf, HomeShelf::Songs { sort, descending, limit })) => each(match self.core.browse_songs(sort, descending, false, 0, 0, offset, limit) {
                    Ok(songs) => Ok(PageGot::Songs { exhausted: !home_row_more(shelf, songs.len() as u32), songs }),
                    Err(e) => Err(PageFailed::Stored(e)),
                }),
                Some((_, HomeShelf::Playlists { take })) => pages(Read::PlaylistList, &|p| if let Page::Playlists { v } = p { Some(PageGot::Playlists(v.into_iter().take(take as usize).collect())) } else { None }),
                Some((_, HomeShelf::Pinned)) => {
                    let pins = self.core.session.settings.prefs(|p| p.pinned_playlists.clone());
                    pages(Read::PlaylistList, &|p| if let Page::Playlists { v } = p { Some(PageGot::Playlists(home_pinned(v, pins.clone()))) } else { None });
                }
            },
            PageAsk::Mix(id) => each(match block_on(self.client.mix_songs(id.clone())) {
                Ok(_) => match self.core.mix_page(id) {
                    MixLookup::Ready { sheet } => Ok(PageGot::Mix(Box::new(sheet))),
                    MixLookup::NotDrawn | MixLookup::Unknown => Err(PageFailed::EmptyMix),
                },
                Err(e) => Err(PageFailed::Net(e)),
            }),
        }
    }

    /// The Home rows the profile shows, in the order kept: a jam guest's only the server's album shelves
    /// (`browse::home_rows_shown`).
    pub fn home_rows(&self) -> Vec<HomeRow> {
        let kept = self.core.session.settings.prefs(|p| p.home_rows.clone());
        home_rows_shown(kept, self.rules().account)
    }

    /// [`Handle::home_rows`] that are shelves of albums, for a Home that draws only albums.
    pub fn home_album_rows(&self) -> Vec<HomeRow> {
        self.home_rows().into_iter().filter(|&r| matches!(home_shelves(vec![r])[..], [HomeShelf::Albums { .. }])).collect()
    }

    /// Reads Home on a thread of its own: with `mixes`, "For you" first; then a source's own (its feed),
    /// else the shelves of `rows` (some of [`Handle::home_rows`]). `each` gets every part as it comes; an error
    /// only while nothing has come.
    pub fn read_home(&self, rows: Vec<HomeRow>, mixes: bool, each: impl Fn(Result<HomePart, NetError>) + Send + 'static) {
        let me = self.clone();
        crate::spawn("nori-home", move || me.home(rows, mixes, &each));
    }

    fn home(&self, rows: Vec<HomeRow>, mixes: bool, each: &dyn Fn(Result<HomePart, NetError>)) {
        let client = &self.client;
        // The mixes and the favourites are the profile's where it has them (`ProfileRules::mixes`).
        let mixes = mixes.then_some(self.rules().mixes);
        let taste = self.core.session.settings.prefs(|p| p.taste_model);
        // The favourites' tile follows the starred songs: stored at once, the server's after.
        let stored = mixes.filter(|&account| account).map(|_| client.mix_favourites_stored());
        let tiles = |account: bool| if account { self.core.mix_cards(taste) } else { Vec::new() };
        let sent = mixes.map(|account| {
            let t = tiles(account);
            each(Ok(HomePart::Mixes(t.clone())));
            t
        });
        let mut any = sent.is_some();
        // A source's own Home: its feed under "For you", as on Android.
        if self.rules().feed {
            let r = read_pages(client, Read::HomeFeed, |p| {
                if let Page::Feed { v } = p {
                    each(Ok(HomePart::Feed(v)));
                }
            });
            if let Err(e) = r {
                return each(Err(e));
            }
        }
        let pins = self.core.session.settings.prefs(|p| p.pinned_playlists.clone());
        let rows = if self.rules().feed { Vec::new() } else { rows };
        for (at, shelf) in home_shelves(rows).into_iter().enumerate() {
            let r = match shelf {
                HomeShelf::Albums { sort, size, follows_stars } => {
                    shelf_pages(client, shelf_albums_read(sort, size, follows_stars, 0), at, each, |p| if let Page::Albums { v } = p { Some(ShelfItems::Albums(v)) } else { None })
                }
                HomeShelf::Playlists { take } => shelf_pages(client, Read::PlaylistList, at, each, |p| if let Page::Playlists { v } = p { Some(ShelfItems::Playlists(v.into_iter().take(take as usize).collect())) } else { None }),
                HomeShelf::Pinned => shelf_pages(client, Read::PlaylistList, at, each, |p| if let Page::Playlists { v } = p { Some(ShelfItems::Playlists(home_pinned(v, pins.clone()))) } else { None }),
                HomeShelf::Songs { sort, descending, limit } => {
                    let songs = self.core.browse_songs(sort, descending, false, 0, 0, 0, limit).unwrap_or_default();
                    each(Ok(HomePart::Shelf { at, items: ShelfItems::Songs(songs) }));
                    Ok(())
                }
            };
            match r {
                Ok(()) => any = true,
                Err(e) if !any => return each(Err(e)),
                Err(_) => {}
            }
        }
        if let (Some(account), Some(sent)) = (mixes, sent) {
            match stored {
                Some(Ok(f)) if f.fresh => {}
                Some(s) => {
                    let _ = block_on(client.mix_favourites_refresh(s.ok().and_then(|f| f.digest)));
                }
                None => {}
            }
            if account {
                block_on(client.mix_warm_all());
            }
            let now = tiles(account);
            if now != sent {
                each(Ok(HomePart::Mixes(now)));
            }
        }
    }
}

/// Reads one Home shelf's pages; a shelf the server has nothing for comes empty, rather than staying
/// unread.
fn shelf_pages(client: &Client, read: Read, at: usize, each: &dyn Fn(Result<HomePart, NetError>), make: impl Fn(Page) -> Option<ShelfItems>) -> Result<(), NetError> {
    let mut got = false;
    read_pages(client, read, |p| {
        if let Some(items) = make(p) {
            got = true;
            each(Ok(HomePart::Shelf { at, items }));
        }
    })?;
    if !got {
        each(Ok(HomePart::Shelf { at, items: ShelfItems::Albums(Vec::new()) }));
    }
    Ok(())
}
