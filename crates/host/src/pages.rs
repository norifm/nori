//! The library's pages as the clients read them, each on a thread of its own: the stored copy first,
//! then the server's when it differs. Home's shelves come in the core's order (`browse::home_shelves`),
//! all that is stored at once (`home::home_kept`), then the server's answers.

use nori_core::browse::{favorites_in_order, home_pinned, home_row_more, home_row_page, home_shelves, shelf_albums_read, AlbumSort, HomeShelf};
pub use nori_core::home::ShelfItems;
use nori_core::home::{shelf_items, shelf_read};
use nori_core::cache_policy::{Page, Read};
use nori_core::browse::{GenreTile, HistoryAfter, HistoryPage, ServerHistory, StatsPage, StatsPeriod, StatsSource};
use nori_core::library::{DownloadedLibrary, QueueOffer};
use nori_core::transfers::DownloadSections;
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
    /// The songs list in order `sort` (a `browse::song_sorts` name), turned around when `reversed`, only the
    /// favorites with `favorites`, a page from `offset` (`Client::songs_listed`).
    Songs { sort: String, reversed: bool, favorites: bool, offset: u32 },
    /// The genres with songs (`Core::genres_listed`).
    Genres,
    /// A genre's songs, by name.
    Genre(String),
    /// The listening history, a page from `after` (None: the newest).
    History(Option<HistoryAfter>),
    /// The listening stats of `source`'s plays in `period`; with `sync`, the server account's new plays are
    /// asked for first (`Client::server_history_sync`).
    Stats { period: StatsPeriod, source: StatsSource, sync: bool },
    /// What is downloaded, and what is on its way.
    Downloads,
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
    Genres(Vec<GenreTile>),
    /// A genre's songs and a cover of them.
    Genre { songs: Vec<Song>, cover: Option<String> },
    History(HistoryPage),
    /// The stats, and how the server account's plays stood when asked for (with `sync`).
    Stats { page: Box<StatsPage>, server: Option<ServerHistory> },
    Downloads(Box<Downloads>),
    Album(Box<AlbumDetail>),
    Artist(Box<ArtistDetail>),
    Playlist(Box<PlaylistDetail>),
    Mix(Box<MixSheet>),
}

/// The Downloads page: the songs on their way, by section, and what is kept.
pub struct Downloads {
    pub sections: DownloadSections,
    pub stored: DownloadedLibrary,
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

impl Handle {
    /// The queue another device or app left on the server, worth offering against what the player shows
    /// (`Client::queue_offer`); none when it cannot be asked. Blocks.
    pub fn queue_offer(&self) -> Option<QueueOffer> {
        let now = self.now();
        block_on(self.client.queue_offer(now.playing, now.song.map(|s| s.id))).ok().flatten()
    }

    /// [`Handle::queue_offer`] as the server last answered, at once: shown before it is asked again.
    pub fn queue_offer_kept(&self) -> Option<QueueOffer> {
        let now = self.now();
        self.client.queue_offer_kept(now.playing, now.song.map(|s| s.id)).ok().flatten()
    }

    /// The queue Home offered (`offer::Offered::close`) was taken up (`take`: it plays from its song and
    /// place) or turned down; either way it is not offered again.
    pub fn queue_offer_closed(&self, offer: QueueOffer, take: bool) {
        self.client.queue_offer_done(offer.clone());
        if take {
            let position = offer.position_ms as i64;
            self.play(offer.songs, offer.index as usize, false, None);
            self.seek(position);
        }
    }

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
                pages(Read::StarredItems, &|p| if let Page::StarredPage { v } = p { Some(PageGot::Albums(favorites_in_order(v.albums, sort, false))) } else { None });
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
            PageAsk::Songs { sort, reversed, favorites, offset } => each(match block_on(self.client.songs_listed(sort, reversed, favorites, 0, 0, offset)) {
                Ok(p) => Ok(PageGot::Songs { songs: p.songs, exhausted: p.exhausted }),
                Err(e) => Err(PageFailed::Net(e)),
            }),
            PageAsk::Genres => pages(Read::GenreList, &|p| if let Page::Genres { v } = p { Some(PageGot::Genres(self.core.genres_listed(v))) } else { None }),
            PageAsk::Genre(name) => each(match block_on(self.client.genre_songs(name.clone())) {
                Ok(songs) => Ok(PageGot::Genre { songs, cover: self.core.genre_cover(name).ok().flatten() }),
                Err(e) => Err(PageFailed::Net(e)),
            }),
            PageAsk::History(after) => each(self.core.history_page(after).map(PageGot::History).map_err(PageFailed::Stored)),
            PageAsk::Stats { period, source, sync } => {
                let server = sync.then(|| block_on(self.client.server_history_sync()));
                let now = nori_core::db::now_ms();
                let tz = (nori_core::library::local_offset_s(now / 1000) * 1000) as i32;
                each(self.core.stats_page(period, source, tz).map(|page| PageGot::Stats { page: Box::new(page), server }).map_err(PageFailed::Stored));
            }
            PageAsk::Downloads => each(
                self.core
                    .downloaded_library()
                    .and_then(|stored| Ok(Downloads { sections: self.core.download_sections()?, stored }))
                    .map(|d| PageGot::Downloads(Box::new(d)))
                    .map_err(PageFailed::Stored),
            ),
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
        self.client.home_rows()
    }

    /// Reads Home on a thread of its own: with `mixes`, "For you" first; then a source's own (its feed),
    /// else the shelves of `rows` (some of [`Handle::home_rows`]). Everything stored comes at once, then the
    /// server's answers where they differ; a shelf with nothing stored comes once the server answers, empty
    /// if it has nothing. `each` gets every part as it comes; an error only while nothing has come.
    pub fn read_home(&self, rows: Vec<HomeRow>, mixes: bool, each: impl Fn(Result<HomePart, NetError>) + Send + 'static) {
        let me = self.clone();
        crate::spawn("nori-home", move || me.home(rows, mixes, &each));
    }

    fn home(&self, rows: Vec<HomeRow>, mixes: bool, each: &dyn Fn(Result<HomePart, NetError>)) {
        let client = &self.client;
        let kept = match client.home_kept(rows.clone(), mixes) {
            Ok(kept) => kept,
            Err(e) => return each(Err(e)),
        };
        if mixes {
            each(Ok(HomePart::Mixes(kept.mixes.clone())));
        }
        if let Some(feed) = &kept.feed {
            each(Ok(HomePart::Feed(feed.clone())));
        }
        for (at, items) in kept.shelves.iter().enumerate() {
            if let Some(items) = items {
                each(Ok(HomePart::Shelf { at, items: items.clone() }));
            }
        }
        let mut any = mixes || kept.feed.is_some() || kept.shelves.iter().any(Option::is_some);
        let mut answered = |r: Result<(), NetError>| match r {
            Ok(()) => {
                any = true;
                true
            }
            Err(e) if !any => {
                each(Err(e));
                false
            }
            Err(_) => true,
        };
        if self.rules().feed {
            let r = block_on(client.read_refreshed(Read::HomeFeed)).map(|page| {
                if let Some(Page::Feed { v }) = page {
                    each(Ok(HomePart::Feed(v)));
                }
            });
            if !answered(r) {
                return;
            }
        }
        let pins = self.core.session.settings.prefs(|p| p.pinned_playlists.clone());
        let rows = if self.rules().feed { Vec::new() } else { rows };
        for (at, shelf) in home_shelves(rows).iter().enumerate() {
            let Some(read) = shelf_read(shelf) else { continue };
            let r = block_on(client.read_refreshed(read)).map(|page| match page.and_then(|p| shelf_items(shelf, p, &pins)) {
                Some(items) => each(Ok(HomePart::Shelf { at, items })),
                None if kept.shelves.get(at).is_none_or(Option::is_none) => each(Ok(HomePart::Shelf { at, items: ShelfItems::Albums { v: Vec::new() } })),
                None => {}
            });
            if !answered(r) {
                return;
            }
        }
        if mixes && self.rules().mixes {
            match client.mix_favourites_stored() {
                Ok(f) if f.fresh => {}
                stored => {
                    let _ = block_on(client.mix_favourites_refresh(stored.ok().and_then(|f| f.digest)));
                }
            }
            block_on(client.mix_warm_all());
            let now = self.core.mix_cards(self.core.session.settings.in_effect(|p| p.taste_model));
            if now != kept.mixes {
                each(Ok(HomePart::Mixes(now)));
            }
        }
    }
}

