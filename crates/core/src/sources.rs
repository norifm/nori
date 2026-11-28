//! A profile's other sources ([`crate::settings::SavedServer::sources`]), browsed with its own as one
//! library. Each is the client of its saved profile, over a core of its own: its provider, account,
//! kept pages, favorite marks, queued writes and rows stay that source's. A list read asks every source
//! and merges their pages ([`nori_library::merged`]); an item's own reads, writes and stream go to the
//! source it came from, which its `source` names.

use std::sync::Arc;

use nori_library::merged::{album_order, marked, merged, merged_found, ListMerge, OneItem, SourceKind, Sourced};

use crate::{ItemKey, Keyed};
use crate::cache_policy::{kept_as, records, Page, Read};
use crate::client::{Client, NetResult, Starrable};
use crate::providers::ResolveStream;
use crate::settings::SavedServer;
use crate::transport::Transport;
use crate::{Core, Genre, Starred};

/// The artists a source is asked for to find one of the same name.
const ARTISTS_ASKED: i32 = 5;

/// Another source of the profile: its saved profile's id and client.
pub(crate) struct Source {
    pub(crate) id: String,
    pub(crate) client: Arc<Client>,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Opens saved profile `source` as the next of this profile's other sources, over `transport`: a core
    /// of its own on the database at `db_path`, with the kinds this one opens. Its client.
    pub fn open_source(&self, db_path: String, source: SavedServer, transport: Arc<dyn Transport>, covers: Arc<crate::covers::CoverNet>) -> nori_model::Result<Arc<Client>> {
        let queue = Arc::new(nori_queue::Session::new(self.session.settings.clone()));
        let core = Core::new(db_path, crate::settings::server_db_id(&source.id), queue)?;
        *core.kinds.write() = self.kinds.read().clone();
        core.configure_profile(source.clone())?;
        let client = Client::new(core, transport, covers);
        client.set_profile(crate::net_profile(&source));
        self.source_added(source.id, client.clone())?;
        Ok(client)
    }
}

impl Core {
    /// `client` is the profile's next other source, `id`: its songs' plays go to its rows, and the
    /// profile reads those rows with its own, and its mixes draw from them where the source's rules have
    /// mixes.
    pub(crate) fn source_added(&self, id: String, client: Arc<Client>) -> crate::Result<()> {
        crate::db::source_read(&self.db.lock(), &id, client.core.rules().mixes)?;
        self.session.db.source_opened(id.clone(), &client.core.session.db);
        self.sources.write().push(Source { id, client });
        Ok(())
    }

    /// What the profile's own source holds.
    pub(crate) fn holds(&self) -> SourceKind {
        self.provider().holds()
    }

    /// What may be done with an item of `source`: that source's rules; None for a source the profile does
    /// not have (any more).
    pub fn rules_of(&self, source: &Option<String>) -> Option<crate::browse::ProfileRules> {
        match source {
            None => Some(self.rules()),
            Some(id) => self.source(id).map(|c| c.core.rules()),
        }
    }

    /// The client of other source `id`; None for one the profile does not have (any more).
    pub(crate) fn source(&self, id: &str) -> Option<Arc<Client>> {
        self.sources.read().iter().find(|s| s.id == id).map(|s| s.client.clone())
    }

    /// The profile's other sources, in order.
    pub(crate) fn others(&self) -> Vec<(String, Arc<Client>)> {
        self.sources.read().iter().map(|s| (s.id.clone(), s.client.clone())).collect()
    }

    /// Whether the profile or any of its other sources keeps an offline index that has not been filled yet.
    pub fn any_empty_index(&self) -> bool {
        let empty = |c: &Core| {
            let r = c.rules();
            r.indexed && !r.rescans && c.index_size().map_or(true, |s| s.songs == 0)
        };
        empty(self) || self.others().into_iter().any(|(_, c)| empty(&c.core))
    }

    /// This session's marks in each of the profile's sources, each item by its key: what a client shows
    /// a seal by.
    pub(crate) fn star_marks(&self) -> crate::stars::StarMarks {
        let own = crate::stars::StarMarks::default().with(None, &self.stars.lock());
        self.others().into_iter().fold(own, |marks, (id, c)| marks.with(Some(&id), &c.core.stars.lock()))
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Whether the profile or any of its other sources rescans files on open and resume.
    pub fn any_rescans(&self) -> bool {
        self.rules().rescans || self.others().into_iter().any(|(_, c)| c.core.rules().rescans)
    }

    /// [`Core::starred`] for an item of `source`: that source's mark over its record. An item of a source
    /// the profile no longer has shows as its record says.
    pub fn starred_in(&self, source: Option<String>, kind: Starrable, id: &str, listed: bool) -> bool {
        match source {
            None => self.starred(kind, id, listed),
            Some(source) => self.source(&source).map_or(listed, |c| c.core.starred(kind, id, listed)),
        }
    }
}

/// Whether `read` lists the library, so it asks every source; the rest is one source's.
fn lists(read: &Read) -> bool {
    matches!(
        read,
        Read::AlbumList { .. }
            | Read::AlbumsByYear { .. }
            | Read::FavouriteAlbums { .. }
            | Read::ArtistIndex
            | Read::PlaylistList
            | Read::StarredItems
            | Read::GenreList
            | Read::Search { .. }
            | Read::SongPage { .. }
            | Read::RandomSongs { .. }
    )
}

/// `page` as the profile shows it from `source`: the profile's own (None) as it is.
fn stamped_in(page: Page, source: Option<&str>) -> Page {
    match source {
        Some(id) => stamped(page, id),
        None => page,
    }
}

/// `page` as the profile shows it from other source `source`: each item is that source's
/// ([`Keyed::of_source`]).
pub(crate) fn stamped(mut page: Page, source: &str) -> Page {
    let (songs, albums, artists) = records(&mut page);
    songs.into_iter().for_each(|s| s.of_source(source));
    albums.into_iter().for_each(|a| a.of_source(source));
    artists.into_iter().for_each(|a| a.of_source(source));
    match &mut page {
        Page::Playlists { v } => v.iter_mut().for_each(|p| p.of_source(source)),
        Page::Found { v } => v.playlists.iter_mut().for_each(|p| p.of_source(source)),
        Page::Feed { v } => v.iter_mut().flat_map(|f| &mut f.playlists).for_each(|p| p.of_source(source)),
        Page::PlaylistPage { v } => v.playlist.of_source(source),
        _ => {}
    }
    page
}

/// The sources' pages of one list read as one page; None when none came. Artists are sorted by name,
/// the rest keep each source's order, the profile's own first. Album lists read a page at a time are
/// [`Client::albums_merged`]'s.
fn merged_pages(pages: Vec<Sourced<Page>>) -> Option<Page> {
    fn each<T>(pages: Vec<Sourced<Page>>, take: impl Fn(Page) -> Vec<T>) -> Vec<Sourced<Vec<T>>> {
        pages.into_iter().map(|p| Sourced { kind: p.kind, items: take(p.items) }).collect()
    }
    let first = pages.first()?;
    Some(match &first.items {
        Page::Albums { .. } => Page::Albums { v: merged(each(pages, Page::albums)) },
        Page::Artists { .. } => {
            let mut v = merged(each(pages, |p| if let Page::Artists { v } = p { v } else { Vec::new() }));
            v.sort_by(|a, b| crate::db::natural_cmp(&a.name, &b.name));
            Page::Artists { v }
        }
        Page::Songs { .. } => Page::Songs { v: merged(each(pages, Page::songs)) },
        Page::Playlists { .. } => Page::Playlists { v: marked(pages.into_iter().flat_map(|p| if let Page::Playlists { v } = p.items { v } else { Vec::new() }).collect()) },
        Page::StarredPage { .. } => {
            let parts: Vec<Sourced<Starred>> = pages.into_iter().filter_map(|p| if let Page::StarredPage { v } = p.items { Some(Sourced { kind: p.kind, items: v }) } else { None }).collect();
            let artists = merged(parts.iter().map(|p| Sourced { kind: p.kind, items: p.items.artists.clone() }).collect());
            let albums = merged(parts.iter().map(|p| Sourced { kind: p.kind, items: p.items.albums.clone() }).collect());
            let songs = merged(parts.into_iter().map(|p| Sourced { kind: p.kind, items: p.items.songs }).collect());
            Page::StarredPage { v: Starred::new(artists, albums, songs) }
        }
        Page::Genres { .. } => {
            let mut v: Vec<Genre> = Vec::new();
            for g in pages.into_iter().flat_map(|p| if let Page::Genres { v } = p.items { v } else { Vec::new() }) {
                match v.iter_mut().find(|k| k.name.eq_ignore_ascii_case(&g.name)) {
                    Some(k) => (k.song_count, k.album_count) = (k.song_count + g.song_count, k.album_count + g.album_count),
                    None => v.push(g),
                }
            }
            Page::Genres { v }
        }
        Page::Found { .. } => Page::Found { v: merged_found(pages.into_iter().filter_map(|p| if let Page::Found { v } = p.items { Some(Sourced { kind: p.kind, items: v }) } else { None }).collect()) },
        _ => return pages.into_iter().next().map(|p| p.items),
    })
}

/// `read` as a list of albums read a page at a time: its first page, the page asked, and its size.
fn album_pages(read: &Read) -> Option<(Read, usize, usize)> {
    let at = |offset: i32, size: i32| (offset.max(0) as usize, size.max(0) as usize);
    match read.clone() {
        Read::AlbumList { kind, size, offset, genre } => Some((Read::AlbumList { kind, size, offset: 0, genre }, at(offset, size).0, at(offset, size).1)),
        Read::AlbumsByYear { from, to, size, offset } => Some((Read::AlbumsByYear { from, to, size, offset: 0 }, at(offset, size).0, at(offset, size).1)),
        _ => None,
    }
}

/// How a list of albums read across the sources is ordered ([`album_order`]); a year's albums by year.
fn list_order(read: &Read) -> Option<fn(&crate::Album, &crate::Album) -> std::cmp::Ordering> {
    match read {
        Read::AlbumList { kind, .. } => album_order(*kind),
        _ => album_order(crate::browse::AlbumSort::ByYear),
    }
}

/// `first`, a list's first page, read from `offset`.
fn from_offset(first: &Read, offset: u32) -> Read {
    match first.clone() {
        Read::AlbumList { kind, size, genre, .. } => Read::AlbumList { kind, size, offset: offset as i32, genre },
        Read::AlbumsByYear { from, to, size, .. } => Read::AlbumsByYear { from, to, size, offset: offset as i32 },
        other => other,
    }
}

impl Client {
    /// The sources' pages of one list read as one page ([`merged_pages`]); the playlists of nori's own
    /// first among the playlists.
    fn merged_page(&self, pages: Vec<Sourced<Page>>) -> Option<Page> {
        match merged_pages(pages)? {
            Page::Playlists { v } => Some(Page::Playlists { v: marked(self.core.mixed_playlists().unwrap_or_default().into_iter().chain(v).collect()) }),
            page => Some(page),
        }
    }

    /// A page of a list of albums across the sources ([`ListMerge`]): each source read on from where the
    /// list's earlier pages left it, in the list's order. The first page starts the list anew. An error
    /// only when the page is empty and a source failed.
    /// Boxed: a source's read may come back here.
    fn albums_merged(&self, first: Read, offset: usize, size: usize, others: Vec<(String, Arc<Client>)>) -> futures_util::future::BoxFuture<'_, NetResult<Page>> {
        Box::pin(self.albums_merged_now(first, offset, size, others))
    }

    async fn albums_merged_now(&self, first: Read, offset: usize, size: usize, others: Vec<(String, Arc<Client>)>) -> NetResult<Page> {
        let key = kept_as(&first).map(|(k, _)| k).unwrap_or_default();
        let order = list_order(&first);
        let lanes: Vec<(Option<&str>, &Client)> = std::iter::once((None, self)).chain(others.iter().map(|(id, c)| (Some(id.as_str()), &**c))).collect();
        let kept = if offset == 0 { None } else { self.core.album_lists.lock().remove(&key) };
        let mut list = kept.unwrap_or_else(|| ListMerge::new(lanes.iter().map(|(_, c)| c.core.holds()).collect(), order));
        let mut failed = None;
        while let Some((at, from)) = list.wants(offset + size) {
            let (id, c) = lanes[at];
            let albums = match c.cached_or_fetched(from_offset(&first, from)).await {
                Ok(page) => page.map(|p| match id {
                    Some(id) => stamped(p, id),
                    None => p,
                }),
                Err(e) => {
                    failed = Some(e);
                    None
                }
            };
            list.got(at, albums.map(Page::albums).unwrap_or_default(), size as u32);
        }
        let page = list.page(offset, size);
        self.core.album_lists.lock().insert(key, list);
        match failed {
            Some(e) if page.is_empty() => Err(e),
            _ => Ok(Page::Albums { v: page }),
        }
    }

    /// [`Client::read_each`] over every source of a list read: the pages kept, merged, then the merged
    /// pages once a source answered anew. An error only when no source has anything to show.
    pub(crate) async fn read_each_merged(&self, read: Read, others: Vec<(String, Arc<Client>)>, mut each: impl FnMut(Page)) -> NetResult<()> {
        if let Some((first, offset, size)) = album_pages(&read) {
            each(self.albums_merged(first, offset, size, others).await?);
            return Ok(());
        }
        let mut latest: Vec<Option<Sourced<Page>>> = Vec::new();
        let mut fresh = true;
        let own = self.read_stored(read.clone())?;
        fresh &= own.fresh;
        let mut digests = vec![own.digest];
        latest.push(own.page.map(|p| Sourced { kind: self.core.holds(), items: p }));
        for (id, c) in &others {
            let stored = c.read_stored(read.clone())?;
            fresh &= stored.fresh;
            digests.push(stored.digest);
            latest.push(stored.page.map(|p| Sourced { kind: c.core.holds(), items: stamped(p, id) }));
        }
        let shown = |latest: &[Option<Sourced<Page>>]| self.merged_page(latest.iter().flatten().map(|p| Sourced { kind: p.kind, items: p.items.clone() }).collect());
        if let Some(p) = shown(&latest) {
            each(p);
        }
        if fresh {
            return Ok(());
        }
        let mut changed = false;
        let mut failed = None;
        let sources = std::iter::once((None, self)).chain(others.iter().map(|(id, c)| (Some(id.as_str()), &**c)));
        for (at, (id, c)) in sources.enumerate() {
            match c.read_refresh(read.clone(), digests[at]).await {
                Ok(Some(p)) => {
                    latest[at] = Some(Sourced { kind: c.core.holds(), items: match id { Some(id) => stamped(p, id), None => p } });
                    changed = true;
                }
                Ok(None) => {}
                Err(e) => failed = Some(e),
            }
        }
        match (changed, failed) {
            (true, _) => shown(&latest).into_iter().for_each(each),
            (false, Some(e)) if latest.iter().all(Option::is_none) => return Err(e),
            _ => {}
        }
        Ok(())
    }

    /// [`Client::read_now`] over every source of a list read, merged; an error only when every source
    /// failed.
    pub(crate) async fn read_now_merged(&self, read: Read, others: Vec<(String, Arc<Client>)>) -> NetResult<Page> {
        if let Some((first, offset, size)) = album_pages(&read) {
            return self.albums_merged(first, offset, size, others).await;
        }
        let mut pages = Vec::new();
        let mut failed = None;
        match self.read_now_own(read.clone()).await {
            Ok(p) => pages.push(Sourced { kind: self.core.holds(), items: p }),
            Err(e) => failed = Some(e),
        }
        for (id, c) in others {
            match c.read_now_own(read.clone()).await {
                Ok(p) => pages.push(Sourced { kind: c.core.holds(), items: stamped(p, &id) }),
                Err(e) => failed = failed.or(Some(e)),
            }
        }
        match (self.merged_page(pages), failed) {
            (Some(p), _) => Ok(p),
            (None, Some(e)) => Err(e),
            (None, None) => crate::cache_policy::empty(&read),
        }
    }

    /// What this device keeps of `read`: of a list read, every source's kept page merged.
    pub(crate) fn read_kept(&self, read: Read) -> NetResult<Option<Page>> {
        let others = self.others_for(&read);
        let own = self.read_stored(read.clone())?.page;
        if others.is_empty() {
            return Ok(own);
        }
        let mut pages = vec![own.map(|p| Sourced { kind: self.core.holds(), items: p })];
        for (id, c) in &others {
            pages.push(c.read_stored(read.clone())?.page.map(|p| Sourced { kind: c.core.holds(), items: stamped(p, id) }));
        }
        if let Some((_, _, size)) = album_pages(&read) {
            let order = list_order(&read);
            let kinds = std::iter::once(self.core.holds()).chain(others.iter().map(|(_, c)| c.core.holds())).collect();
            let mut list = ListMerge::new(kinds, order);
            // Each source's kept page is all it has to give.
            for (at, p) in pages.into_iter().enumerate() {
                list.got(at, p.map(|p| p.items.albums()).unwrap_or_default(), u32::MAX);
            }
            list.wants(size);
            return Ok(Some(Page::Albums { v: list.page(0, size) }));
        }
        Ok(self.merged_page(pages.into_iter().flatten().collect()))
    }

    /// [`Client::read_refreshed`] of a list read across the sources: the merged page once a source
    /// answered; None when none could be reached while one is kept.
    pub(crate) async fn read_refreshed_merged(&self, read: Read) -> NetResult<Option<Page>> {
        let kept = self.read_kept(read.clone())?;
        match self.read_now(read).await {
            Ok(page) => Ok(Some(page)),
            Err(_) if kept.is_some() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The profile's sources whose rules have a Home of their own (a service's feed), its own first, each
    /// with the source it is.
    pub fn home_feeds(self: &Arc<Self>) -> Vec<(Option<String>, Arc<Client>)> {
        std::iter::once((None, self.clone())).chain(self.core.others().into_iter().map(|(id, c)| (Some(id), c))).filter(|(_, c)| c.core.rules().feed).collect()
    }

    /// The other sources a list read asks besides the profile's own; none for a profile of one source,
    /// or a read of one source's.
    pub(crate) fn others_for(&self, read: &Read) -> Vec<(String, Arc<Client>)> {
        if !lists(read) {
            return Vec::new();
        }
        self.core.others()
    }

    /// The client of the source an item of `source` comes from: this one for the profile's own; an error
    /// for a source the profile does not have (any more).
    pub fn in_source(self: &Arc<Self>, source: &Option<String>) -> NetResult<Arc<Client>> {
        match source {
            None => Ok(self.clone()),
            Some(id) => self.core.source(id).ok_or_else(|| crate::providers::nowhere("such source")),
        }
    }

    /// `read` of item `key` asked of its source, each item of the page saying where it comes from.
    pub(crate) async fn first_in_source(&self, key: &ItemKey, read: Read) -> NetResult<Page> {
        match (self.other_source(key)?, &key.source) {
            (Some(c), Some(source)) => Ok(stamped(c.first(read).await?, source)),
            _ => self.first(read).await,
        }
    }

    /// Every source's resolver: each turns only its own addresses into URLs.
    pub(crate) fn resolvers(self: &Arc<Self>) -> Option<Arc<dyn ResolveStream>> {
        let own = self.core.provider().resolver(self);
        let all: Vec<Arc<dyn ResolveStream>> = own.into_iter().chain(self.core.others().iter().filter_map(|(_, c)| c.core.provider().resolver(c))).collect();
        match all.len() {
            0 => None,
            1 => all.into_iter().next(),
            _ => Some(Arc::new(Resolvers(all))),
        }
    }

    /// The page of artist `id` of `source` as one artist across the profile's sources: its source's page
    /// as it comes, then, once the others answered, one page with the albums and songs of the same artist
    /// ([`OneItem::same`]) in every source, an item two of them have showing once.
    pub async fn read_each_artist(self: Arc<Self>, source: Option<String>, id: String, mut each: impl FnMut(Page)) -> NetResult<()> {
        let mut opened = None;
        self.clone().read_each_in(source.clone(), Read::ArtistById { id }, |p| {
            if let Page::ArtistPage { v } = &p {
                opened = Some(v.clone());
            }
            each(p)
        })
        .await?;
        let Some(opened) = opened else { return Ok(()) };
        let sources = std::iter::once((None, self.clone())).chain(self.core.others().into_iter().map(|(id, c)| (Some(id), c)));
        let mut parts = Vec::new();
        for (at, c) in sources {
            if at == source {
                parts.push(Sourced { kind: c.core.holds(), items: opened.clone() });
                continue;
            }
            let Ok(Page::Found { v }) = c.read_now_own(Read::Search { query: opened.artist.name.clone(), songs: 0, albums: 0, artists: ARTISTS_ASKED }).await else { continue };
            let Some(same) = v.artists.into_iter().find(|a| a.same(&opened.artist)) else { continue };
            if let Ok(Some(Page::ArtistPage { v })) = c.cached_or_fetched(Read::ArtistById { id: same.id }).await.map(|p| p.map(|p| stamped_in(p, at.as_deref()))) {
                parts.push(Sourced { kind: c.core.holds(), items: v });
            }
        }
        if parts.len() > 1 {
            let albums = merged(parts.iter().map(|p| Sourced { kind: p.kind, items: p.items.albums.clone() }).collect());
            let songs = merged(parts.into_iter().map(|p| Sourced { kind: p.kind, items: p.items.songs }).collect());
            each(Page::ArtistPage { v: crate::ArtistDetail::new(opened.artist, albums, songs) });
        }
        Ok(())
    }

    /// [`Client::read_each`] of an item's read in the source it comes from; its pages say that source.
    pub async fn read_each_in(self: Arc<Self>, source: Option<String>, read: Read, mut each: impl FnMut(Page)) -> NetResult<()> {
        let client = self.in_source(&source)?;
        match source {
            None => client.read_each(read, each).await,
            Some(id) => client.read_each(read, |p| each(stamped(p, &id))).await,
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Whether Home has the library's rows: some source of the profile is not a service with a Home of its own.
    pub fn library_home(&self) -> bool {
        !self.core.rules().feed || self.core.others().iter().any(|(_, c)| !c.core.rules().feed)
    }

    /// Whether Home has "For you": some source of the profile has mixes.
    pub fn mixes_anywhere(&self) -> bool {
        self.core.rules().mixes || self.core.others().iter().any(|(_, c)| c.core.rules().mixes)
    }

    /// The sources' own Homes as kept, one after another; None when none is kept.
    pub fn home_feed_kept(&self) -> NetResult<Option<Vec<crate::FeedShelf>>> {
        let mut feed: Option<Vec<crate::FeedShelf>> = None;
        let own = std::iter::once((None, self)).filter(|(_, c)| c.core.rules().feed);
        let others = self.core.others();
        for (id, c) in own.chain(others.iter().filter(|(_, c)| c.core.rules().feed).map(|(id, c)| (Some(id.as_str()), &**c))) {
            if let Some(Page::Feed { v }) = c.read_stored(Read::HomeFeed)?.page.map(|p| stamped_in(p, id)) {
                feed.get_or_insert_with(Vec::new).extend(v);
            }
        }
        Ok(feed)
    }

    /// The sources' own Homes, each asked anew: all of them once one changed; None when none did. An
    /// error only when none could be reached and none is kept.
    pub async fn home_feed_refreshed(self: Arc<Self>) -> NetResult<Option<Vec<crate::FeedShelf>>> {
        let (mut changed, mut failed, mut all) = (false, None, Vec::new());
        for (id, c) in self.home_feeds() {
            let fresh = match c.read_refreshed(Read::HomeFeed).await {
                Ok(page) => page,
                Err(e) => {
                    failed = Some(e);
                    None
                }
            };
            changed |= fresh.is_some();
            let page = match fresh {
                Some(p) => Some(p),
                None => c.read_stored(Read::HomeFeed)?.page,
            };
            if let Some(Page::Feed { v }) = page.map(|p| stamped_in(p, id.as_deref())) {
                all.extend(v);
            }
        }
        match (changed, failed) {
            (true, _) => Ok(Some(all)),
            (false, Some(e)) if all.is_empty() => Err(e),
            _ => Ok(None),
        }
    }

}

/// The source every one of `songs` comes from; None for songs of several sources, or none.
pub(crate) fn one_source(songs: &[crate::Song]) -> Option<Option<String>> {
    let first = &songs.first()?.source;
    songs.iter().all(|s| s.source == *first).then(|| first.clone())
}

/// Where a new playlist of some songs is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum NewPlaylist {
    /// In the songs' one source (None: the profile's own), as that source's playlist.
    InSource { source: Option<String> },
    /// As one of nori's own, on this device ([`Core::mixed_playlist_create`]).
    Mixed,
}

/// Where a playlist is kept.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PlaylistAt {
    /// In a source of the profile (None: its own).
    Source { source: Option<String> },
    /// On this device, as one of nori's own ([`crate::Playlist::mixed`]).
    Device,
}

/// Where `playlist` is kept.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn playlist_at(playlist: &crate::Playlist) -> PlaylistAt {
    if playlist.mixed { PlaylistAt::Device } else { PlaylistAt::Source { source: playlist.source.clone() } }
}

impl Client {
    /// [`Client::read_each`] of playlist `id`'s page where it is kept: nori's own from this device, a
    /// source's from that source; a playlist of nori's own that is gone is an error.
    pub async fn read_each_playlist(self: Arc<Self>, id: String, at: PlaylistAt, mut each: impl FnMut(Page)) -> NetResult<()> {
        match at {
            PlaylistAt::Source { source } => self.read_each_in(source, Read::PlaylistById { id }, each).await,
            PlaylistAt::Device => {
                let page = self.core.mixed_playlist(id)?.ok_or_else(|| crate::providers::nowhere("such playlist"))?;
                each(Page::PlaylistPage { v: page });
                Ok(())
            }
        }
    }
}

/// A change to a playlist ([`Client::playlist_change`]).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PlaylistChange {
    /// These songs added at its end.
    Add { songs: Vec<crate::Song> },
    /// Row `index` of its songs (`songs`, by id) moved or taken out.
    Edit { songs: Vec<String>, index: u32, edit: crate::menus::PlaylistEdit },
    Rename { name: String },
    Delete,
}

/// What became of a change to a playlist.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PlaylistChanged {
    /// Made; `kept` is what it changed of a playlist kept downloaded (`Client::kept_playlist_written`).
    Done { kept: crate::transfers::KeptChange },
    /// Not made: the playlist does not take the songs.
    Refused { why: crate::menus::PlaylistRefusal },
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Where a new playlist of `songs` is kept: in their one source when its rules let the account make
    /// playlists (no songs: the profile's own), else as nori's own in a profile of several sources; None
    /// where it can be neither.
    pub fn new_playlist_in(&self, songs: Vec<crate::Song>) -> Option<NewPlaylist> {
        let source = if songs.is_empty() { Some(None) } else { one_source(&songs) };
        match source {
            Some(source) if self.core.rules_of(&source).is_some_and(|r| r.playlists) => Some(NewPlaylist::InSource { source }),
            _ if !self.core.others().is_empty() => Some(NewPlaylist::Mixed),
            _ => None,
        }
    }

    /// A new playlist named `name` of `songs`, kept where [`Client::new_playlist_in`] says.
    pub async fn playlist_create(self: Arc<Self>, name: String, songs: Vec<crate::Song>) -> NetResult<()> {
        match self.new_playlist_in(songs.clone()) {
            Some(NewPlaylist::Mixed) => Ok(self.core.mixed_playlist_create(name, songs).map(drop)?),
            Some(NewPlaylist::InSource { source }) => self.in_source(&source)?.write(crate::client::Write::CreatePlaylist { name, song_ids: songs.into_iter().map(|s| s.id).collect() }).await,
            None => Err(crate::providers::nowhere("playlist")),
        }
    }

    /// Changes `playlist` where it is kept: nori's own on this device, a source's in that source, which
    /// takes only its own songs ([`crate::menus::playlist_refusal`]). A playlist kept downloaded follows.
    pub async fn playlist_change(self: Arc<Self>, playlist: crate::Playlist, change: PlaylistChange) -> NetResult<PlaylistChanged> {
        use crate::client::Write;
        let id = playlist.id.clone();
        if let PlaylistChange::Add { songs } = &change {
            if let Some(why) = crate::menus::playlist_refusal(&playlist, songs) {
                return Ok(PlaylistChanged::Refused { why });
            }
        }
        if playlist.mixed {
            match change {
                PlaylistChange::Add { songs } => self.core.mixed_playlist_add(id, songs),
                PlaylistChange::Edit { index, edit, .. } => self.core.mixed_playlist_edit(id, index, edit),
                PlaylistChange::Rename { name } => self.core.mixed_playlist_rename(id, name),
                PlaylistChange::Delete => self.core.mixed_playlist_delete(id),
            }?;
            return Ok(PlaylistChanged::Done { kept: Default::default() });
        }
        let write = match change {
            PlaylistChange::Add { songs } => Write::AddToPlaylist { id, song_ids: songs.into_iter().map(|s| s.id).collect() },
            PlaylistChange::Edit { songs, index, edit } => match crate::menus::playlist_edit_write(id, songs, index, edit) {
                Some(w) => w,
                None => return Ok(PlaylistChanged::Done { kept: Default::default() }),
            },
            PlaylistChange::Rename { name } => Write::RenamePlaylist { id, name },
            PlaylistChange::Delete => Write::DeletePlaylist { id },
        };
        let c = self.in_source(&playlist.source)?;
        c.write(write.clone()).await?;
        // The write stands; a kept playlist not brought up to date now is at the next look.
        let kept = c.kept_playlist_written(write).await.unwrap_or_default();
        Ok(PlaylistChanged::Done { kept })
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Stars or unstars an item of `source` in that source ([`Client::star_marked`]); `marked` is handed
    /// the profile's marks ([`Core::star_marks`]) each time they move.
    pub async fn star_in(self: Arc<Self>, source: Option<String>, kind: Starrable, id: String, on: bool, marked: Arc<dyn crate::library::StarsShown>) -> NetResult<()> {
        self.in_source(&source)?.star_marked(kind, id, on, || marked.marks(self.core.star_marks())).await
    }

    /// Whether the profile may change `playlist`: nori's own always; a source's as
    /// [`crate::menus::playlist_editable`] says, signed in to that source as `user`, by its rules.
    pub fn playlist_editable_in(&self, playlist: &crate::Playlist, user: &str) -> bool {
        playlist.mixed || crate::menus::playlist_editable(playlist, user, self.core.rules_of(&playlist.source).is_some_and(|r| r.playlists))
    }

    /// [`Client::read_now`] of an item's read in the source it comes from (None: the profile's own), each
    /// item of the page saying where it comes from.
    pub async fn read_now_in(self: Arc<Self>, source: Option<String>, read: Read) -> NetResult<Page> {
        let page = self.in_source(&source)?.read_now(read).await?;
        Ok(stamped_in(page, source.as_deref()))
    }

    /// [`Client::read_cached`] of an item's read in the source it comes from (None: the profile's own).
    pub async fn read_cached_in(self: Arc<Self>, source: Option<String>, read: Read, shown: Arc<dyn crate::library::PageShown>) -> NetResult<()> {
        self.read_each_in(source, read, |p| shown.show(p)).await
    }

    /// What playing playlist `id` plays, from where it is kept, each song saying its source.
    pub async fn playlist_songs_at(self: Arc<Self>, id: String, at: PlaylistAt) -> NetResult<Vec<crate::Song>> {
        match at {
            PlaylistAt::Source { source } => Ok(songs_of_source(self.in_source(&source)?.playlist_songs_of(id).await?, &source)),
            PlaylistAt::Device => Ok(self.core.mixed_playlist(id)?.map(|d| d.songs).unwrap_or_default()),
        }
    }

    /// What playing album `id` of `source` (None: the profile's own) plays, asked of that source now,
    /// each song saying its source.
    pub async fn album_songs_in(self: Arc<Self>, source: Option<String>, id: String) -> NetResult<Vec<crate::Song>> {
        Ok(songs_of_source(self.in_source(&source)?.album_songs_of(id).await?, &source))
    }

    /// What playing artist `id` of `source` plays ([`Client::artist_songs_of`] in that source), each song
    /// saying its source.
    pub async fn artist_songs_in(self: Arc<Self>, source: Option<String>, id: String) -> NetResult<Vec<crate::Song>> {
        Ok(songs_of_source(self.in_source(&source)?.artist_songs_of(id).await?, &source))
    }

    /// The radio of song `id` of `source`, from that source, each song saying its source.
    pub async fn radio_in(self: Arc<Self>, source: Option<String>, id: String) -> NetResult<Vec<crate::Song>> {
        Ok(songs_of_source(self.in_source(&source)?.radio(id).await?, &source))
    }

    /// Playlist `id`'s page where it is kept, for a screen ([`Client::read_each_playlist`]).
    pub async fn read_playlist(self: Arc<Self>, id: String, at: PlaylistAt, shown: Arc<dyn crate::library::PageShown>) -> NetResult<()> {
        self.read_each_playlist(id, at, |p| shown.show(p)).await
    }

    /// The page of artist `id` of `source` as one artist across the profile's sources, for a screen
    /// ([`Client::read_each_artist`]).
    pub async fn read_artist(self: Arc<Self>, source: Option<String>, id: String, shown: Arc<dyn crate::library::PageShown>) -> NetResult<()> {
        self.read_each_artist(source, id, |p| shown.show(p)).await
    }
}

/// Songs of `source` (None: the profile's own) as the profile has them ([`Keyed::of_source`]).
pub fn songs_of_source(songs: Vec<crate::Song>, source: &Option<String>) -> Vec<crate::Song> {
    let Some(source) = source else { return songs };
    songs.into_iter().map(|mut s| {
        s.of_source(source);
        s
    }).collect()
}

/// The resolvers of several sources: an address goes to the one that resolves it.
struct Resolvers(Vec<Arc<dyn ResolveStream>>);

impl ResolveStream for Resolvers {
    fn resolve(&self, address: &str) -> Option<NetResult<String>> {
        self.0.iter().find_map(|r| r.resolve(address))
    }

    fn refused(&self, address: &str) {
        self.0.iter().for_each(|r| r.refused(address));
    }

    fn live(&self, address: &str) -> Option<Box<dyn std::io::Read + Send>> {
        self.0.iter().find_map(|r| r.live(address))
    }

    fn resolve_rest(&self, address: &str, from: u64) -> Option<NetResult<String>> {
        self.0.iter().find_map(|r| r.resolve_rest(address, from))
    }

    fn parts(&self, address: &str) -> Option<crate::providers::StreamParts> {
        self.0.iter().find_map(|r| r.parts(address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ItemKey;
    use crate::client::tests::{block, client, Fake};
    use crate::client::NetProfile;
    use crate::mixes::listen_again;
    use crate::stream::StreamQuality;

    /// A Subsonic profile with a service (the fake provider) as its other source `yt`.
    fn merged() -> (Arc<Client>, Arc<Fake>) {
        let (c, wire) = client(NetProfile { url: "http://nd".into(), ..Default::default() });
        c.core.source_added("yt".into(), crate::providers::fake::source(String::new(), "yt")).unwrap();
        (c, wire)
    }

    fn ids(songs: &[crate::Song]) -> Vec<(&str, Option<&str>)> {
        songs.iter().map(|s| (s.id.as_str(), s.source.as_deref())).collect()
    }

    /// Search asks every source: a song the server has too shows once, as the server's; the service's
    /// other songs show marked with their source.
    #[test]
    fn search_finds_in_every_source_and_shows_one_copy() {
        let (c, wire) = merged();
        wire.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"song":[{"id":"nd-1","title":"One","artist":"Fay","album":"Fakes","duration":61}]}}}"#);
        let Page::Found { v } = block(c.read_now(Read::Search { query: "o".into(), songs: 10, albums: 10, artists: 10 })).unwrap() else { panic!("found") };
        assert_eq!(ids(&v.songs), [("nd-1", None), ("f-2", Some("yt"))]);
        wire.fail(crate::transport::FailureKind::Connect);
        let Page::Found { v } = block(c.read_now(Read::Search { query: "two".into(), songs: 10, albums: 10, artists: 10 })).unwrap() else { panic!("found") };
        assert_eq!(ids(&v.songs), [("f-2", Some("yt"))], "the server unreachable: what the service found");
    }

    /// An album list across sources comes a page at a time in its order, each source read on from where
    /// it stopped.
    #[test]
    fn album_lists_merge_in_order_a_page_at_a_time() {
        let (c, wire) = merged();
        let albums = |names: &[(&str, &str)]| {
            let v: Vec<String> = names.iter().map(|(id, name)| format!(r#"{{"id":"{id}","name":"{name}","artist":"Ann"}}"#)).collect();
            format!(r#"{{"subsonic-response":{{"status":"ok","albumList2":{{"album":[{}]}}}}}}"#, v.join(","))
        };
        let page = |offset: i32| {
            let Page::Albums { v } = block(c.read_now(Read::AlbumList { kind: crate::browse::AlbumSort::ByName, size: 2, offset, genre: None })).unwrap() else { panic!("albums") };
            v.into_iter().map(|a| (a.name, a.source)).collect::<Vec<_>>()
        };
        let own = |name: &str| (name.to_string(), None);
        let yt = |name: &str| (name.to_string(), Some("yt".to_string()));
        wire.answer(&albums(&[("a1", "Alpha"), ("a2", "Delta")]));
        assert_eq!(page(0), [own("Alpha"), yt("Bravo")]);
        wire.answer(&albums(&[("a3", "Echo")]));
        assert_eq!(page(2), [own("Delta"), own("Echo")]);
        assert_eq!(page(4), [yt("Fakes")]);
        let asked: Vec<bool> = wire.asked().iter().map(|u| u.contains("offset=2")).collect();
        assert_eq!(asked, [false, true], "the server asked for its first page, then on from where it stopped: {:?}", wire.asked());
    }

    /// A profile of several sources has one Home, from what is kept: its rows across every source, the
    /// feed of a source with a Home of its own woven in, each item saying its source.
    #[test]
    fn home_is_one_across_sources() {
        use crate::home::{shelf_read, HomePlace, ShelfItems};
        let (c, wire) = merged();
        let yt = c.core.source("yt").unwrap();
        yt.core.set_rules(crate::browse::ProfileRules { feed: true, ..yt.core.rules() });
        let recent = shelf_read(&crate::browse::home_shelves(vec![crate::settings::HomeRow::Recent])[0]).unwrap();
        wire.answer(r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"r1","name":"Mine","artist":"Ann"}]}}}"#);
        block(c.cached_or_fetched(recent.clone())).unwrap();
        block(yt.cached_or_fetched(recent)).unwrap();
        block(yt.read_each(Read::HomeFeed, |_| {})).unwrap();
        let asked = wire.asked().len();
        let kept = c.home_kept(vec![crate::settings::HomeRow::Recent], false).unwrap();
        assert_eq!(wire.asked().len(), asked, "nothing asked");
        let Some(ShelfItems::Albums { v }) = &kept.shelves[0] else { panic!("albums") };
        assert_eq!(v.iter().map(|a| (a.id.as_str(), a.source.as_deref())).collect::<Vec<_>>(), [("r1", None), ("f-b", Some("yt")), ("f-al", Some("yt"))], "recently played takes turns");
        let feed = kept.feed.expect("the service's feed");
        assert!(feed.iter().flat_map(|f| &f.songs).all(|s| s.source.as_deref() == Some("yt")));
        assert_eq!(kept.places, [HomePlace::Row { at: 0 }, HomePlace::Feed { at: 0 }], "the feed kept as one shelf");
    }

    /// An artist's page shows the same artist of every source as one: the albums of both, an album both
    /// have once, the library's.
    #[test]
    fn an_artist_is_one_across_sources() {
        let (c, wire) = merged();
        wire.answer(r#"{"subsonic-response":{"status":"ok","artist":{"id":"ar1","name":"Fay","album":[{"id":"a1","name":"Fakes","artist":"Fay"},{"id":"a2","name":"Mine","artist":"Fay"}]}}}"#);
        let mut pages = Vec::new();
        block(c.clone().read_each_artist(None, "ar1".into(), |p| {
            if let Page::ArtistPage { v } = p {
                pages.push(v.albums.iter().map(|a| (a.id.clone(), a.source.clone())).collect::<Vec<_>>());
            }
        }))
        .unwrap();
        let (own, yt) = (|id: &str| (id.to_string(), None), |id: &str| (id.to_string(), Some("yt".to_string())));
        assert_eq!(pages.first().unwrap(), &[own("a1"), own("a2")], "its own source's page first");
        assert_eq!(pages.last().unwrap(), &[own("a1"), own("a2"), yt("f-b")], "then every source's, Fakes once");
    }

    /// Songs go to nori's own playlists in a profile of several sources, and to their one source's own
    /// where its rules have playlists; a new playlist of songs of several sources is nori's own.
    #[test]
    fn songs_of_any_source_go_to_a_playlist() {
        let (c, _) = merged();
        let yt = c.core.source("yt").unwrap();
        block(yt.sign_in("let me in".into()));
        let mine = c.core.mixed_playlist_create("Both".into(), vec![crate::Song { id: "f-1".into(), source: Some("yt".into()), ..Default::default() }]).unwrap();
        let theirs = |id: &str| crate::Song { id: id.into(), source: Some("yt".into()), ..Default::default() };
        let choices: Vec<(String, Option<String>, bool, Vec<String>)> = block(c.clone().playlist_choices(vec![theirs("f-1"), theirs("f-2")])).unwrap().into_iter().map(|p| (p.playlist.id, p.playlist.source, p.playlist.mixed, p.adds.into_iter().map(|s| s.id).collect())).collect();
        assert_eq!(choices, [(mine.id.clone(), None, true, vec!["f-2".to_string()]), ("f-own".to_string(), Some("yt".to_string()), false, vec!["f-2".to_string()])]);
        let both = vec![theirs("f-1"), crate::Song::only_id("s1".into())];
        let taken: Vec<bool> = block(c.clone().playlist_choices(both.clone())).unwrap().into_iter().map(|p| p.refused.is_none()).collect();
        assert_eq!(taken, [true, false], "songs of two sources: nori's own alone takes them");
        assert_eq!(c.new_playlist_in(both), Some(NewPlaylist::Mixed));
        assert_eq!(c.new_playlist_in(vec![crate::Song::only_id("s1".into())]), Some(NewPlaylist::InSource { source: None }));
        assert_eq!(c.new_playlist_in(vec![theirs("f-1")]), Some(NewPlaylist::InSource { source: Some("yt".into()) }), "signed in, the service makes playlists");
    }

    /// A playlist is changed where it is kept: nori's own on this device, with nothing sent; a source's
    /// in that source, which refuses another source's songs as data, sending nothing.
    #[test]
    fn a_playlist_changes_where_it_is_kept() {
        let (c, wire) = merged();
        block(c.core.source("yt").unwrap().sign_in("let me in".into()));
        let theirs = crate::Song { id: "f-1".into(), source: Some("yt".into()), ..Default::default() };
        let mine = c.core.mixed_playlist_create("Both".into(), Vec::new()).unwrap();
        wire.answer(r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"p1","name":"Road"}]}}}"#);
        let choices = block(c.clone().playlist_choices(vec![theirs.clone()])).unwrap();
        let shown: Vec<(&str, Option<&str>, bool)> = choices.iter().map(|ch| (ch.playlist.id.as_str(), ch.playlist.source.as_deref(), ch.refused.is_some())).collect();
        assert_eq!(shown, [(mine.id.as_str(), None, false), ("f-own", Some("yt"), false), ("p1", None, true)], "the server's playlist listed last, refusing");
        let asked = wire.asked().len();
        let add = |at: usize| block(c.clone().playlist_change(choices[at].playlist.clone(), PlaylistChange::Add { songs: choices[at].adds.clone() })).unwrap();
        assert!(matches!(add(0), PlaylistChanged::Done { .. }));
        assert_eq!(c.core.mixed_playlist(mine.id.clone()).unwrap().unwrap().songs.into_iter().map(|s| (s.id, s.source)).collect::<Vec<_>>(), [("f-1".to_string(), Some("yt".to_string()))]);
        assert_eq!(add(2), PlaylistChanged::Refused { why: crate::menus::PlaylistRefusal::OtherSource { source: None } });
        assert_eq!(wire.asked().len(), asked, "nothing sent to the server");
        wire.answer(crate::client::tests::OK);
        let own = crate::Song::only_id("s1".into());
        assert!(matches!(block(c.clone().playlist_change(choices[2].playlist.clone(), PlaylistChange::Add { songs: vec![own] })).unwrap(), PlaylistChanged::Done { .. }));
        assert!(wire.asked().last().is_some_and(|u| u.contains("/rest/updatePlaylist?") && u.contains("songIdToAdd=s1")), "{:?}", wire.asked());
    }

    /// A list read shows every source's items; a source's playlists stay its own.
    #[test]
    fn lists_come_from_every_source() {
        let (c, wire) = merged();
        wire.answer(r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"p1","name":"Mine"}]}}}"#);
        let mut shown = Vec::new();
        block(c.read_each(Read::PlaylistList, |p| if let Page::Playlists { v } = p { shown.push(v.iter().map(|p| (p.id.clone(), p.source.clone())).collect::<Vec<_>>()) })).unwrap();
        let yt = Some("yt".to_string());
        assert_eq!(shown.last().unwrap(), &[("p1".to_string(), None), ("f-own".into(), yt.clone()), ("f-followed".into(), yt.clone())]);
        let mine = c.core.mixed_playlist_create("Both".into(), Vec::new()).unwrap();
        wire.answer(r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"p1","name":"Mine"}]}}}"#);
        let Page::Playlists { v } = block(c.read_now(Read::PlaylistList)).unwrap() else { panic!("playlists") };
        assert_eq!((v[0].id.as_str(), v[0].mixed, v.len()), (mine.id.as_str(), true, 4), "nori's own first");
        let mut playlist = None;
        block(c.clone().read_each_playlist("f-own".into(), PlaylistAt::Source { source: Some("yt".into()) }, |p| playlist = Some(p))).unwrap();
        let Some(Page::PlaylistPage { v }) = playlist else { panic!("a playlist") };
        assert_eq!(v.playlist.source.as_deref(), Some("yt"), "a source's playlist's page says its source, where its changes go");
        let mut album = None;
        block(c.clone().read_each_in(Some("yt".into()), Read::AlbumById { id: "f-al".into() }, |p| album = Some(p))).unwrap();
        let Some(Page::AlbumPage { v }) = album else { panic!("an album") };
        assert_eq!(ids(&v.songs), [("f-1", Some("yt")), ("f-2", Some("yt"))], "an item's page from its source");
        assert!(block(c.clone().read_each_in(Some("gone".into()), Read::AlbumById { id: "f-al".into() }, |_| {})).is_err());
    }

    /// A queue song streams through its own source, its bytes kept apart from another source's song of
    /// the same id; only that source's resolver turns its addresses into URLs.
    #[test]
    fn a_song_streams_through_its_source() {
        let (c, _) = merged();
        c.core.session.register(vec![crate::Song { id: "f-1".into(), source: Some("yt".into()), ..Default::default() }, crate::Song { id: "s1".into(), ..Default::default() }]);
        let q = StreamQuality { bit_rate: 0, format: String::new() };
        let yt = ItemKey { source: Some("yt".into()), id: "f-1".into() };
        let theirs = c.stream_target(&yt, false, q.clone(), q.clone()).unwrap();
        assert_eq!((theirs.url.as_str(), theirs.key.as_str()), ("fake:f-1", "f-1\u{1f}yt:0"));
        let own = c.stream_target(&ItemKey::own("s1"), false, q.clone(), q).unwrap();
        assert!(own.url.starts_with("http://nd/rest/stream?") && own.key == "s1:0", "{own:?}");
        let resolver = c.stream_resolver().expect("the service's");
        assert_eq!(resolver.resolve("fake:f-1").map(Result::unwrap).as_deref(), Some("https://fake.example/f-1"));
        assert!(resolver.resolve(&own.url).is_none());
        let ahead = |k: &ItemKey| c.in_song_source(k, |c| c.core.provider().ahead()).flatten();
        assert_eq!((ahead(&yt), ahead(&ItemKey::own("s1"))), (Some(0), None), "each song as its source says");
    }

    /// A play is kept in the rows of its song's source; the profile's history lists every source's plays,
    /// newest first, each song saying its source, while the source alone lists its own as its own.
    #[test]
    fn history_holds_every_sources_plays() {
        use crate::scrobble::TrackChange;
        let dir = nori_testdir::TempDir::new("sources-history");
        let db = dir.join("nori.db").display().to_string();
        let home = Core::new(db.clone(), "home".into(), Default::default()).unwrap();
        let yt = crate::providers::fake::source(db, "yt");
        home.source_added("yt".into(), yt.clone()).unwrap();
        let song = |id: &str, source: Option<&str>| crate::Song { id: id.into(), title: id.into(), duration: 100, source: source.map(String::from), ..Default::default() };
        let session = &home.session;
        session.register(vec![song("a", None), song("b", Some("yt"))]);
        session.scrobble_track(Some(ItemKey::own("a")), TrackChange::Moved, true, 0, 1_000_000, 0);
        session.scrobble_track(Some(ItemKey { source: Some("yt".into()), id: "b".into() }), TrackChange::Moved, true, 60_000, 1_060_000, 0);
        session.scrobble_track(None, TrackChange::Ended, false, 120_000, 1_120_000, 0);
        session.settings.writer.flush();
        let heard = |core: &Core| core.history_recent(10, None, true).unwrap().0.into_iter().map(|e| (e.song.id, e.song.source)).collect::<Vec<_>>();
        assert_eq!(heard(&home), [("b".to_string(), Some("yt".to_string())), ("a".to_string(), None)]);
        assert_eq!(heard(&yt.core), [("b".to_string(), None)], "kept in the source's own rows");
        home.history_clear().unwrap();
        assert!(heard(&home).is_empty() && heard(&yt.core).is_empty(), "the profile's history is cleared whole");
    }

    /// A client over its own core on the database at `db`, its rows those of `id`, with `rules`.
    fn plain(db: &str, id: &str, rules: crate::browse::ProfileRules) -> Arc<Client> {
        let core = Core::new(db.to_string(), id.into(), Default::default()).unwrap();
        core.set_rules(rules);
        Client::new(core, Arc::new(Fake::default()), Default::default())
    }

    /// The mixes draw from the profile's own songs and those of its sources whose rules have mixes, each
    /// song saying its source; a source without mixes stays out of them.
    #[test]
    fn mixes_draw_from_the_sources_that_have_mixes() {
        use crate::history::tests::{listen, song, DAY, NOW};
        let dir = nori_testdir::TempDir::new("sources-mixes");
        let db = dir.join("nori.db").display().to_string();
        let home = plain(&db, "home", crate::browse::profile_rules(false));
        let nd = plain(&db, "nd2", crate::browse::profile_rules(false));
        let yt = plain(&db, "yt", crate::browse::ProfileRules { mixes: false, ..crate::browse::profile_rules(false) });
        for (c, id) in [(&home, "own"), (&nd, "theirs"), (&yt, "service")] {
            let s = song(id, id, &format!("{id} artist"), &format!("{id} album"), "Rock", 2000);
            crate::db::index(&mut c.core.db.lock(), &[], &[], std::slice::from_ref(&s)).unwrap();
            listen(&c.core, &s, NOW - 2 * DAY);
        }
        let drawn = |home: &Arc<Client>| {
            let mut v: Vec<(String, Option<String>)> = listen_again(&home.core.db.lock(), 10, 1, NOW).unwrap().into_iter().map(|s| (s.id, s.source)).collect();
            v.sort();
            v
        };
        assert_eq!(drawn(&home), [("own".to_string(), None)], "a profile of one source draws from its own");
        home.core.source_added("nd2".into(), nd).unwrap();
        home.core.source_added("yt".into(), yt).unwrap();
        assert_eq!(drawn(&home), [("own".to_string(), None), ("theirs".to_string(), Some("nd2".to_string()))]);
    }

    /// A song of another source downloads where that source keeps downloads: into its rows, from its
    /// address, and the profile lists it with its source; a source that keeps none downloads nothing.
    #[test]
    fn songs_download_from_their_source() {
        let dir = nori_testdir::TempDir::new("sources-downloads");
        let db = dir.join("nori.db").display().to_string();
        let home = plain(&db, "home", crate::browse::profile_rules(false));
        let yt = crate::providers::fake::source(db.clone(), "yt");
        yt.core.set_rules(crate::browse::ProfileRules { downloads: true, ..yt.core.rules() });
        let sc = plain(&db, "sc", crate::browse::ProfileRules { downloads: false, ..crate::browse::profile_rules(false) });
        home.core.source_added("yt".into(), yt.clone()).unwrap();
        home.core.source_added("sc".into(), sc).unwrap();
        let song = |id: &str, source: Option<&str>| crate::Song { id: id.into(), title: id.into(), source: source.map(String::from), ..Default::default() };
        let queued = home.core.download_queue(vec![song("f-1", Some("yt")), song("s-1", Some("sc")), song("own", None)]).unwrap();
        let mut fresh = queued.fresh.clone();
        fresh.sort();
        assert_eq!(fresh, [ItemKey::own("own"), ItemKey { source: Some("yt".into()), id: "f-1".into() }], "the source without downloads keeps none");
        let listed: Vec<(String, Option<String>)> = home.core.downloads(false).unwrap().into_iter().map(|s| (s.id, s.source)).collect();
        assert!(listed.contains(&("f-1".to_string(), Some("yt".to_string()))), "{listed:?}");
        assert_eq!(yt.core.download_ids(false).unwrap(), [ItemKey::own("f-1")], "in the source's own rows");
        let q = StreamQuality { bit_rate: 0, format: String::new() };
        assert_eq!(home.download_target(ItemKey { source: Some("yt".into()), id: "f-1".into() }, q).unwrap().url, "fake:f-1", "from the source's address");
    }

    /// The profile's own account and a second account of the server, both kept in one database: song
    /// `1` is in both.
    fn two_accounts() -> (nori_testdir::TempDir, Arc<Client>, Arc<Fake>, Arc<Client>, Arc<Fake>) {
        accounts_at("http://nd", "http://nd")
    }

    /// The profile's own account at `url` and a second account, `acct2`, at `second_url`, both kept in
    /// one database.
    fn accounts_at(url: &str, second_url: &str) -> (nori_testdir::TempDir, Arc<Client>, Arc<Fake>, Arc<Client>, Arc<Fake>) {
        let dir = nori_testdir::TempDir::new("sources-accounts");
        let db = dir.join("nori.db").display().to_string();
        let account = |id: &str, url: &str, user: &str| {
            let core = Core::new(db.clone(), id.into(), Default::default()).unwrap();
            core.configure(crate::ServerConfig { url: url.into(), user: user.into(), password: "p".into(), ..Default::default() }).unwrap();
            let wire = Arc::new(Fake::default());
            let c = Client::new(core, wire.clone(), Default::default());
            c.set_profile(NetProfile { url: url.into(), ..Default::default() });
            (c, wire)
        };
        let (home, own) = account("home", url, "u");
        let (second, theirs) = account("acct2", second_url, "v");
        home.core.source_added("acct2".into(), second.clone()).unwrap();
        (dir, home, own, second, theirs)
    }

    /// Star marks no screen shows.
    struct Unseen;

    impl crate::library::StarsShown for Unseen {
        fn marks(&self, _: crate::stars::StarMarks) {}
    }

    /// Song `1` of the profile's own account and song `1` of its second account.
    fn both() -> (crate::Song, crate::Song) {
        let own = crate::Song { id: "1".into(), title: "Mine".into(), duration: 100, ..Default::default() };
        let theirs = crate::Song { title: "Theirs".into(), source: Some("acct2".into()), ..own.clone() };
        (own, theirs)
    }

    /// One id in two sources is two songs to the queue: both stay queued, each listed as itself, its play
    /// and now playing told to its own account, its favorite sent there, and the queue saved and opened
    /// again with both; a queue saved before sources were kept loads as the profile's own.
    #[test]
    fn one_id_in_two_accounts_is_two_songs() {
        use crate::scrobble::TrackChange;
        let (_dir, home, own_wire, second, their_wire) = two_accounts();
        let (own, theirs) = both();
        let session = &home.core.session;
        session.register(vec![own.clone(), theirs.clone()]);
        session.set(vec![own.key(), theirs.key()], Some(0), false, None);
        let titles = |songs: &[crate::Song]| songs.iter().map(|s| (s.title.clone(), s.source.clone())).collect::<Vec<_>>();
        let listed = vec![("Mine".to_string(), None), ("Theirs".to_string(), Some("acct2".to_string()))];
        assert_eq!(titles(&session.view(0).songs), listed, "each listed as itself");

        // Each play and now playing goes to the song's own account.
        session.scrobble_track(Some(own.key()), TrackChange::Moved, true, 0, 1_000, 0);
        let sent = session.scrobble_track(Some(theirs.key()), TrackChange::Moved, true, 60_000, 61_000, 0);
        assert_eq!((sent.counted.as_ref().map(|s| s.title.as_str()), sent.playing.as_ref().map(|s| s.title.as_str())), (Some("Mine"), Some("Theirs")));
        own_wire.answer(crate::client::tests::OK);
        their_wire.answer(crate::client::tests::OK);
        block(home.scrobble(sent));
        let told = |wire: &Fake| wire.asked().into_iter().filter(|u| u.contains("/rest/scrobble?")).collect::<Vec<_>>();
        let (to_own, to_theirs) = (told(&own_wire), told(&their_wire));
        assert!(to_own.len() == 1 && to_own[0].contains("u=u&") && to_own[0].contains("submission=true"), "the play of Mine, to its account: {to_own:?}");
        assert!(to_theirs.len() == 1 && to_theirs[0].contains("u=v&") && to_theirs[0].contains("submission=false"), "Theirs now playing, to its account: {to_theirs:?}");

        // A favorite from the player goes to the playing song's account.
        their_wire.answer(crate::client::tests::OK);
        let playing = session.song(&theirs.key()).unwrap();
        block(home.clone().star_in(playing.source.clone(), Starrable::Song, playing.id, true, Arc::new(Unseen))).unwrap();
        assert!(their_wire.asked().iter().any(|u| u.contains("/rest/star?") && u.contains("id=1")) && !own_wire.asked().iter().any(|u| u.contains("/rest/star?")), "starred where Theirs is");

        // Removing one leaves the other; its undo puts back the one taken out.
        session.remove(0, 1);
        assert_eq!(session.playlist(|p| p.ids().to_vec()), [theirs.key()]);
        assert_eq!(session.restore(own.key()).at, Some(0));
        assert_eq!(session.playlist(|p| p.ids().to_vec()), [own.key(), theirs.key()]);

        // Saved and opened again: both, each with its source.
        home.core.playlist_save(0).unwrap();
        let saved = home.core.load_queue().unwrap();
        assert_eq!(titles(&saved.songs), listed);
        assert_eq!(saved.songs.iter().map(crate::Keyed::key).collect::<Vec<_>>(), [own.key(), theirs.key()]);
        let stored = crate::db::kv_get(&home.core.db.lock(), "queue").unwrap().unwrap();
        let json: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert!(json["songs"][0].get("source").is_none() && json["songs"][1]["source"] == "acct2", "a song of the profile's own is stored as before: {stored}");
        drop(second);
    }

    /// The marks handed to a screen, each with how many requests its account had seen by then.
    struct Handed(parking_lot::Mutex<Vec<(crate::stars::StarMarks, usize)>>, Arc<Fake>);

    impl crate::library::StarsShown for Handed {
        fn marks(&self, marks: crate::stars::StarMarks) {
            self.0.lock().push((marks, self.1.asked().len()));
        }
    }

    /// One id in two accounts is two favorites: the profile's own account starred song `1`, the second
    /// did not. Each seal shows its own account's state, a press marks at once and changes only its
    /// account, a refusal puts back that account's mark alone, and each account's kept pages read
    /// offline show its own marks.
    #[test]
    fn one_id_in_two_accounts_is_two_favorites() {
        let (_dir, home, own_wire, second, their_wire) = two_accounts();
        let album = |title: &str, starred: &str| format!(r#"{{"subsonic-response":{{"status":"ok","album":{{"id":"al1","name":"A","artist":"X","song":[{{"id":"1","title":"{title}","isDir":false,"duration":100{starred}}}]}}}}}}"#);
        own_wire.answer(&album("Mine", r#","starred":"2026-01-01T00:00:00Z""#));
        their_wire.answer(&album("Theirs", ""));
        let page = |source: Option<&str>| {
            let mut songs = Vec::new();
            let _ = block(home.clone().read_each_in(source.map(String::from), Read::AlbumById { id: "al1".into() }, |p| if let Page::AlbumPage { v } = p { songs = v.songs }));
            songs.into_iter().next().expect("song 1")
        };
        let (own, theirs) = (page(None), page(Some("acct2")));
        assert_eq!((own.starred, theirs.starred, theirs.key()), (true, false, ItemKey { source: Some("acct2".into()), id: "1".into() }));
        let shown = |marks: &crate::stars::StarMarks| (marks.starred(Starrable::Song, &own.key(), own.starred), marks.starred(Starrable::Song, &theirs.key(), theirs.starred));
        let seals = || (home.core.starred_in(own.source.clone(), Starrable::Song, "1", own.starred), home.core.starred_in(theirs.source.clone(), Starrable::Song, "1", theirs.starred));
        let stars = |wire: &Fake| wire.asked().into_iter().filter(|u| u.contains("/rest/star?") || u.contains("/rest/unstar?")).collect::<Vec<_>>();

        // A press on the second account's copy: marked at once, before its account is asked, and sent
        // there alone.
        their_wire.answer(crate::client::tests::OK);
        let handed = Arc::new(Handed(Default::default(), their_wire.clone()));
        let before = their_wire.asked().len();
        block(home.clone().star_in(theirs.source.clone(), Starrable::Song, "1".into(), true, handed.clone())).unwrap();
        let first = handed.0.lock()[0].clone();
        assert_eq!((shown(&first.0), first.1), ((true, true), before), "marked before the account was asked");
        assert_eq!(seals(), (true, true));
        assert_eq!((stars(&own_wire).len(), stars(&their_wire).len()), (0, 1), "starred in the second account alone");

        // Unfavoriting the profile's own copy leaves the second account's.
        own_wire.answer(crate::client::tests::OK);
        block(home.clone().star_in(None, Starrable::Song, "1".into(), false, handed.clone())).unwrap();
        assert_eq!(shown(&handed.0.lock().last().unwrap().0), (false, true));
        assert_eq!(seals(), (false, true));
        assert!(stars(&own_wire)[0].contains("/rest/unstar?") && stars(&their_wire).len() == 1, "unstarred in the profile's own account alone");

        // The second account refuses an unfavorite: its mark comes back, the own one stays.
        their_wire.answer(r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"no"}}}"#);
        assert!(block(home.clone().star_in(theirs.source.clone(), Starrable::Song, "1".into(), false, handed.clone())).is_err());
        let restored = handed.0.lock().last().unwrap().0.clone();
        assert_eq!((shown(&restored), seals()), ((false, true), (false, true)));

        // Offline, each account's kept page shows its own account's mark.
        own_wire.fail(crate::transport::FailureKind::Connect);
        their_wire.fail(crate::transport::FailureKind::Connect);
        assert_eq!((page(None).starred, page(Some("acct2")).starred), (false, true));
        drop(second);
    }

    /// Two servers that number their covers alike: each item's cover comes from its own server, wherever
    /// its id goes alone, the cover cache keeps the two apart, and the profile's own covers keep their
    /// addresses and cache keys.
    #[test]
    fn one_cover_id_on_two_servers_is_two_covers() {
        let (_dir, home, own_wire, second, their_wire) = accounts_at("http://one", "http://two");
        let album = r#"{"subsonic-response":{"status":"ok","album":{"id":"al1","name":"A","artist":"X","coverArt":"al-1","song":[{"id":"1","title":"T","isDir":false,"coverArt":"al-1"}]}}}"#;
        own_wire.answer(album);
        their_wire.answer(album);
        let page = |source: Option<&str>| {
            let mut detail = None;
            block(home.clone().read_each_in(source.map(String::from), Read::AlbumById { id: "al1".into() }, |p| if let Page::AlbumPage { v } = p { detail = Some(v) })).unwrap();
            detail.expect("the album")
        };
        let (own, theirs) = (page(None), page(Some("acct2")));
        let url = |cover: &Option<String>| home.core.cover_address(cover.clone().expect("a cover"), 320).expect("a source of the profile");
        let (own_url, their_url) = (url(&own.album.cover_art), url(&theirs.album.cover_art));
        assert_eq!(own.album.cover_art.as_deref(), Some("al-1"), "the profile's own cover keeps its id");
        assert_eq!(own_url, format!("{}&id=al-1&size=320", home.core.url_prefix("getCoverArt")), "and its address");
        assert_eq!(their_url, format!("{}&id=al-1&size=320", second.core.url_prefix("getCoverArt")), "from the second server");
        assert_eq!(url(&theirs.songs[0].cover_art), their_url, "a song's cover as its album's");
        assert_eq!(home.core.download_cover_urls(vec![theirs.album.cover_art.clone().unwrap()])[0], their_url);

        let net = crate::covers::CoverNet::new();
        let key = |url: &str| {
            let mut key = Vec::new();
            net.key_parts(url, |p| key.extend_from_slice(p));
            key
        };
        assert_eq!(key(&own_url), b"one/rest/getCoverArt&id=al-1&size=320", "the profile's own key as before");
        assert_eq!(key(&their_url), b"two/rest/getCoverArt&id=al-1&size=320");

        let prefixes: Vec<(Option<String>, bool)> = home.core.cover_prefixes().into_iter().map(|p| (p.source, p.prefix.is_some_and(|p| p.starts_with("http://two/")))).collect();
        assert_eq!(prefixes, [(None, false), (Some("acct2".into()), true)]);
        assert_eq!(home.core.cover_address(format!("al-1{}gone", ItemKey::SOURCE_MARK), 320), None, "a source the profile no longer has");
    }

    /// Leaving the second account's song `1` out of mixes leaves out that song alone, in its account's
    /// rows; the profile's own song `1` stays in.
    #[test]
    fn one_id_in_two_accounts_is_left_out_of_mixes_alone() {
        let (_dir, home, _, second, _) = two_accounts();
        let (own, theirs) = both();
        home.core.mix_excluded_set(theirs.key(), true).unwrap();
        assert_eq!(home.core.mix_excluded_songs().unwrap(), [theirs.key()]);
        let rows: Vec<String> = home.core.db.lock().prepare("SELECT server FROM mix_excluded").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(rows, ["acct2"], "in the second account's rows");
        assert_eq!(second.core.mix_excluded_songs().unwrap(), [ItemKey::own("1")], "the second account's own");
        home.core.mix_excluded_set(own.key(), true).unwrap();
        home.core.mix_excluded_set(theirs.key(), false).unwrap();
        assert_eq!(home.core.mix_excluded_songs().unwrap(), [own.key()]);
    }

    /// A download of each of two accounts' song `1`: two downloads, each kept in its account's rows and
    /// fetched from its account, each settled on its own; the profile's own keeps the keys it had.
    #[test]
    fn one_id_in_two_accounts_downloads_twice() {
        let (_dir, home, _, second, _) = two_accounts();
        second.core.set_rules(crate::browse::ProfileRules { downloads: true, ..second.core.rules() });
        let (own, theirs) = both();
        let queued = home.core.download_queue(vec![own.clone(), theirs.clone()]).unwrap();
        assert_eq!(queued.fresh, [own.key(), theirs.key()], "two downloads, not one");
        let q = StreamQuality { bit_rate: 0, format: String::new() };
        let (mine, their) = (home.download_target(own.key(), q.clone()).unwrap(), home.download_target(theirs.key(), q).unwrap());
        assert_eq!((mine.key.as_str(), their.key.as_str()), ("dl:1", "dl:1\u{1f}acct2"));
        assert!(mine.url.contains("u=u&") && their.url.contains("u=v&"), "each from its account: {} {}", mine.url, their.url);
        home.core.download_settle(vec![theirs.key()], vec![true]).unwrap();
        let held = home.core.transfers().held();
        assert_eq!((held.state(&own.key()), held.state(&theirs.key())), (crate::transfers::HeldState::Pending, crate::transfers::HeldState::Done));
        drop(held);
        assert_eq!(second.core.download_ids(true).unwrap(), [ItemKey::own("1")], "in the second account's rows");
        assert_eq!(home.core.download_ids(false).unwrap(), [own.key()], "the own one still on its way");
    }

    /// A play is told to the song's own source: the server hears of its own songs, never of a service's.
    #[test]
    fn a_play_is_told_to_its_songs_source() {
        let (c, wire) = merged();
        let song = |id: &str, source: Option<&str>| crate::Song { id: id.into(), source: source.map(String::from), ..Default::default() };
        let send = |counted: crate::Song, playing: crate::Song| nori_queue::scrobble::ScrobbleSend { counted: Some(counted), counted_at: 5, playing: Some(playing) };
        block(c.scrobble(send(song("f-1", Some("yt")), song("f-2", Some("gone")))));
        assert!(wire.asked().is_empty(), "nothing to the server: {:?}", wire.asked());
        wire.answer(crate::client::tests::OK);
        wire.answer(crate::client::tests::OK);
        block(c.scrobble(send(song("s1", None), song("f-2", Some("yt")))));
        assert_eq!(wire.asked().len(), 1, "the server's own song alone");
        assert!(wire.asked()[0].contains("/rest/scrobble?") && wire.asked()[0].contains("id=s1"), "{:?}", wire.asked());
    }

    /// A local files profile merged as a source into a Subsonic profile: syncing scans its folder, lists
    /// return albums from both, and the local song resolves to its file on disk.
    #[test]
    fn a_local_source_syncs_lists_and_plays_from_its_file() {
        let dir = nori_testdir::TempDir::new("sources-local");
        let music = dir.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let file = music.join("tone.flac");
        std::fs::write(&file, nori_local::testing::flac(440.0, &["TITLE=Tone", "ARTIST=Ann", "ALBUM=Local Album"], None)).unwrap();

        let db = dir.join("nori.db").to_string_lossy().into_owned();
        let kinds = crate::providers::Kinds::builtin();
        kinds.set_files(Arc::new(nori_local::Disk::folders()));
        let core = Core::new(db.clone(), "home".into(), Arc::default()).unwrap();
        core.use_kinds(kinds);
        let wire = Arc::new(Fake::default());
        let home_sv = SavedServer { id: "home".into(), url: "http://home".into(), user: "u".into(), password: "p".into(), ..Default::default() };
        core.configure_profile(home_sv.clone()).unwrap();
        let local_sv = SavedServer { id: "local".into(), kind: crate::settings::ServerKind::Local, folders: vec![music.to_string_lossy().into()], ..Default::default() };
        core.open_source(db, local_sv, wire.clone(), crate::covers::CoverNet::new()).unwrap();
        let client = Client::new(core, wire.clone(), crate::covers::CoverNet::new());
        client.set_profile(crate::net_profile(&home_sv));

        assert!(client.core.any_rescans());
        // Subsonic fails to sync offline, local still syncs its folder.
        wire.fail(crate::transport::FailureKind::Connect);
        let stats = block(client.sync_library()).unwrap();
        assert_eq!(stats.songs, 1, "the local folder was scanned");

        wire.answer(r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"al1","name":"Server Album","artist":"Bob","songCount":1,"duration":100}]}}}"#);
        let mut albums = Vec::new();
        block(client.clone().read_each(Read::AlbumList { kind: crate::browse::AlbumSort::ByName, size: 20, offset: 0, genre: None }, |p| {
            if let Page::Albums { v } = p {
                albums = v;
            }
        }))
        .unwrap();
        let names: Vec<(&str, Option<&str>)> = albums.iter().map(|a| (a.name.as_str(), a.source.as_deref())).collect();
        assert_eq!(names, [("Local Album", Some("local")), ("Server Album", None)]);

        let local_album = albums.iter().find(|a| a.source.as_deref() == Some("local")).unwrap();
        let mut songs = Vec::new();
        block(client.clone().read_each_in(local_album.source.clone(), Read::AlbumById { id: local_album.id.clone() }, |p| {
            if let Page::AlbumPage { v } = p {
                songs = v.songs;
            }
        }))
        .unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(client.file_of(&songs[0].key()).map(|f| f.path), Some(file.to_string_lossy().into_owned()));
    }
}
