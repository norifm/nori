//! Android Auto's calls: folders read as a screen reads them (stored answer first, so they open offline),
//! the queue a picked row plays, search and spoken requests. The tree itself is nori-library's.

use std::collections::VecDeque;

use crate::cache_policy::{Page, Read};
use crate::client::Client;
use crate::mixes::board::{MixLookup, FAVOURITES_MIX};
use crate::Song;

pub use nori_library::car::*;

/// The folders last listed, newest last: their later pages and the list a picked row plays come from
/// the one read, so a random folder draws once.
#[derive(Default)]
pub struct Shown(VecDeque<(String, BrowsePage)>);

impl Shown {
    /// How many folders are remembered: a car lists a few levels deep at a time.
    const KEPT: usize = 8;

    fn keep(&mut self, parent: &str, page: &BrowsePage) {
        self.0.retain(|(p, _)| p != parent);
        self.0.push_back((parent.into(), page.clone()));
        while self.0.len() > Self::KEPT {
            self.0.pop_front();
        }
    }

    fn get(&self, parent: &str) -> Option<BrowsePage> {
        self.0.iter().find(|(p, _)| p == parent).map(|(_, s)| s.clone())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// The root folder of the tree.
    pub fn browse_root(&self) -> BrowseFolder {
        folder(ROOT, CarFolder::Root)
    }

    /// Page `page` of `page_size` rows of the root's tabs, `limit` at most, the downloads first while
    /// `offline` (nori-library's `root`).
    pub fn car_root(&self, limit: u32, offline: bool, page: u32, page_size: u32) -> BrowsePage {
        let rules = self.core.rules();
        let folders = root(4, offline).into_iter().filter(|f| match f.kind {
            Some(CarFolder::Favourites) => rules.stars,
            Some(CarFolder::Library) => rules.account || rules.sections.iter().any(|s| matches!(s, crate::browse::LibrarySection::Albums | crate::browse::LibrarySection::Playlists)),
            Some(CarFolder::Downloads) => rules.downloads,
            _ => true,
        }).take(limit.max(1) as usize).collect();
        page_of(&BrowsePage { folders, ..Default::default() }, page, page_size)
    }

    /// Page `page` of `page_size` rows of folder `parent`, a long one in runs (nori-library's `in_runs`):
    /// nothing for one that is not known, `failed` for one that cannot be read now. Pages after the
    /// first come from the first's read, a run from its folder's, and a search's results from
    /// [`Client::car_search`].
    pub async fn browse_children(&self, parent: String, page: u32, page_size: u32) -> BrowsePage {
        let shown = match run_part(&parent) {
            Some((n, whole)) => run_of(&self.whole(whole, true).await, n),
            None => in_runs(&parent, self.whole(&parent, page > 0 || parent.starts_with("search:")).await),
        };
        page_of(&shown, page, page_size)
    }

    /// What a picked row plays: a song of a folder plays the folder from it, Play and Shuffle the whole
    /// of it. None for an id that is not one of the tree's rows.
    pub async fn car_queue(&self, row: String) -> Option<CarQueue> {
        let mut row = car_row(row)?;
        // A song of a run plays its whole folder from it.
        if let Some((n, whole)) = run_part(&row.parent) {
            let whole = whole.to_string();
            let all = self.whole(&whole, true).await;
            let from = library_songs(all.songs[..songs_before_run(&all, n)].to_vec()).len();
            row.parent = whole;
            return Some(queue_for(&row, library_songs(all.songs), from));
        }
        let (kind, arg) = row.parent.split_once(':').unwrap_or((row.parent.as_str(), ""));
        // Played whole, an artist is all their songs and a genre its songs, not the albums listed.
        let songs = match kind {
            "downloads" if row.song.is_none() => self.core.downloads(true).unwrap_or_default(),
            "artist" if row.song.is_none() => self.artist_songs_of(arg.into()).await.unwrap_or_default(),
            "genre" if row.song.is_none() => self.songs(Read::SongsByGenre { genre: arg.into(), count: 100, offset: 0 }).await.unwrap_or_default(),
            _ => {
                let shown = self.car.lock().get(&row.parent).map(|p| p.songs);
                match shown {
                    Some(s) => s,
                    None => self.folder_songs(&row.parent).await.unwrap_or_default(),
                }
            }
        };
        Some(queue_for(&row, library_songs(songs), 0))
    }

    /// The car's search: the artists, albums and songs the server finds for `query`, under their headings.
    pub async fn car_search(&self, query: String) -> BrowsePage {
        let parent = format!("search:{query}");
        in_runs(&parent, self.listed(&parent).await)
    }

    /// What a spoken request plays (nori-library's `voice_pick`): an artist's songs and a genre's shuffled,
    /// an album or a playlist from its start as its page's queue, the songs found from the first. Empty
    /// when nothing matches or the server cannot be asked.
    pub async fn car_voice(&self, ask: VoiceAsk) -> CarQueue {
        let term = voice_term(&ask);
        let queue = |songs: Vec<Song>, kind: Option<nori_model::OriginKind>, id: String, shuffle: bool| CarQueue {
            songs: library_songs(songs),
            index: 0,
            origin: kind.map(|kind| nori_model::PageOrigin { kind, id }),
            shuffle,
        };
        if ask.focus == VoiceFocus::Genre {
            let songs = self.songs(Read::SongsByGenre { genre: term.clone(), count: 100, offset: 0 }).await.unwrap_or_default();
            return queue(songs, None, term, true);
        }
        let sizes = crate::browse::library_sizes();
        let found = match self.read_now(Read::Search { query: term, songs: sizes.search_songs, albums: sizes.search_albums, artists: sizes.search_artists }).await {
            Ok(Page::Found { v }) => v,
            _ => Default::default(),
        };
        let playlists = match ask.focus {
            VoiceFocus::Any | VoiceFocus::Playlist => match self.cached_or_fetched(Read::PlaylistList).await {
                Ok(Some(Page::Playlists { v })) => v,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        use nori_model::OriginKind as K;
        match voice_pick(&ask, &found, &playlists) {
            VoicePick::Artist(id) => queue(self.artist_songs_of(id.clone()).await.unwrap_or_default(), Some(K::Artist), id, true),
            VoicePick::Album(id) => queue(self.folder_songs(&format!("album:{id}")).await.unwrap_or_default(), Some(K::Album), id, false),
            VoicePick::Playlist(id) => queue(self.folder_songs(&format!("playlist:{id}")).await.unwrap_or_default(), Some(K::Playlist), id, false),
            VoicePick::Genre(g) => queue(self.songs(Read::SongsByGenre { genre: g.clone(), count: 100, offset: 0 }).await.unwrap_or_default(), None, g, true),
            VoicePick::Songs(songs) => queue(songs, None, String::new(), false),
            VoicePick::Nothing => CarQueue::default(),
        }
    }
}

/// The most albums a Subsonic server lists at once.
const ALBUMS_PER_READ: usize = 500;

/// A folder that could not be read.
fn failed() -> BrowsePage {
    BrowsePage { failed: true, ..Default::default() }
}

/// Play and Shuffle, for a folder that plays whole.
fn whole() -> Vec<CarAction> {
    vec![CarAction::Play, CarAction::Shuffle]
}

impl Client {
    /// Everything folder `parent` holds: as it was listed last when `kept` and it was, else read now.
    async fn whole(&self, parent: &str, kept: bool) -> BrowsePage {
        let shown = if kept { self.car.lock().get(parent) } else { None };
        match shown {
            Some(all) => all,
            None => self.listed(parent).await,
        }
    }

    /// Everything folder `parent` holds, kept for its later pages and picks unless it could not be read.
    async fn listed(&self, parent: &str) -> BrowsePage {
        let (kind, arg) = parent.split_once(':').unwrap_or((parent, ""));
        let page = match kind {
            ROOT => self.car_root(4, false, 0, u32::MAX),
            "home" => self.car_home().await,
            "library" => {
                let rules = self.core.rules();
                let folders = if rules.feed {
                    library().into_iter().filter(|f| match f.kind {
                        Some(CarFolder::Albums) => rules.sections.contains(&crate::browse::LibrarySection::Albums),
                        Some(CarFolder::Playlists) => rules.sections.contains(&crate::browse::LibrarySection::Playlists),
                        _ => false,
                    }).collect()
                } else { library() };
                BrowsePage { folders, ..Default::default() }
            },
            "albums" => match crate::browse::AlbumSort::of_name(arg) {
                Some(crate::browse::AlbumSort::ByName) => self.a_to_z(parent, AzPart::Whole).await,
                Some(sort) => match self.cached_or_fetched(Read::AlbumList { kind: sort, size: 100, offset: 0, genre: None }).await {
                    Ok(Some(Page::Albums { v })) => BrowsePage { folders: album_folders(&v, None, 100), ..Default::default() },
                    _ => failed(),
                },
                None => BrowsePage::default(),
            },
            "artists" => self.a_to_z(parent, AzPart::Whole).await,
            "az" => match az_part(arg) {
                Some((list, part)) => self.a_to_z(list, part).await,
                None => BrowsePage::default(),
            },
            "artist" => match self.cached_or_fetched(Read::ArtistById { id: arg.into() }).await {
                Ok(Some(Page::ArtistPage { v })) => BrowsePage { folders: album_folders(&v.albums, None, usize::MAX), actions: whole(), ..Default::default() },
                _ => failed(),
            },
            "playlists" => match self.cached_or_fetched(Read::PlaylistList).await {
                Ok(Some(Page::Playlists { v })) => BrowsePage { folders: v.iter().map(playlist_folder).collect(), ..Default::default() },
                _ => failed(),
            },
            "genres" => match self.cached_or_fetched(Read::GenreList).await {
                Ok(Some(Page::Genres { v })) => BrowsePage { folders: v.iter().filter(|g| g.song_count > 0).map(genre_folder).collect(), ..Default::default() },
                _ => failed(),
            },
            "genre" => match self.a_to_z(parent, AzPart::Whole).await {
                p if p.failed => p,
                p => BrowsePage { actions: whole(), ..p },
            },
            "starred" => match self.cached_or_fetched(Read::StarredItems).await {
                Ok(Some(Page::StarredPage { v })) => {
                    let mut p = self.songs_page(library_songs(v.songs), true);
                    p.folders = album_folders(&v.albums, Some(CarGroup::Albums), usize::MAX);
                    p.folders.extend(v.artists.iter().filter(|a| !a.is_external).map(|a| artist_folder(a, Some(CarGroup::Artists))));
                    if !p.folders.is_empty() {
                        p.songs_group = Some(CarGroup::Songs);
                    }
                    p
                }
                _ => failed(),
            },
            "downloads" => match self.core.downloaded_library() {
                Ok(library) => {
                    let mut p = self.songs_page(library.songs, false);
                    p.folders = library.albums.iter().map(|g| {
                        let mut f = album_folder(g, None);
                        f.id = format!("downloaded:{}", g.id);
                        f
                    }).collect();
                    if !p.folders.is_empty() || !p.songs.is_empty() {
                        p.actions = whole();
                    }
                    p
                }
                Err(_) => failed(),
            },
            "search" => self.search_page(arg).await,
            _ => match self.folder_songs(parent).await {
                Some(songs) => self.songs_page(songs, kind != "random"),
                None if matches!(kind, "album" | "playlist" | "mix" | "random" | "downloaded") => failed(),
                None => BrowsePage::default(),
            },
        };
        if !page.failed {
            self.car.lock().keep(parent, &page);
        }
        page
    }

    /// Part `part` of A to Z list `list`: the artists, the albums by name, or a genre's albums.
    async fn a_to_z(&self, list: &str, part: AzPart) -> BrowsePage {
        let albums = match list.split_once(':') {
            Some(("genre", genre)) => Some((crate::browse::AlbumSort::ByGenre, Some(genre))),
            _ if list == "albums:alphabeticalByName" => Some((crate::browse::AlbumSort::ByName, None)),
            _ => None,
        };
        let rows = match albums {
            Some((sort, genre)) => match self.every_album(sort, genre).await {
                Some(v) => album_folders(&v, None, usize::MAX),
                None => return failed(),
            },
            None if list == "artists" => match self.cached_or_fetched(Read::ArtistIndex).await {
                Ok(Some(Page::Artists { v })) => v.iter().filter(|a| !a.is_external).map(|a| artist_folder(a, None)).collect(),
                _ => return failed(),
            },
            None => return BrowsePage::default(),
        };
        a_to_z(list, rows, part)
    }

    /// Every album in `kind`'s order (of `genre`, if given), a server's longest list at a time; what
    /// could be read if a later list cannot be, none if the first cannot.
    async fn every_album(&self, kind: crate::browse::AlbumSort, genre: Option<&str>) -> Option<Vec<crate::Album>> {
        let mut all = Vec::new();
        loop {
            let read = Read::AlbumList { kind, size: ALBUMS_PER_READ as i32, offset: all.len() as i32, genre: genre.map(Into::into) };
            match self.cached_or_fetched(read).await {
                Ok(Some(Page::Albums { v })) if crate::browse::album_page_repeats(&all, &v) => return Some(all),
                Ok(Some(Page::Albums { v })) => {
                    let more = v.len() == ALBUMS_PER_READ;
                    all.extend(v);
                    if !more {
                        return Some(all);
                    }
                }
                _ if all.is_empty() => return None,
                _ => return Some(all),
            }
        }
    }

    async fn search_page(&self, query: &str) -> BrowsePage {
        let sizes = crate::browse::library_sizes();
        let read = Read::Search { query: query.into(), songs: sizes.search_songs, albums: sizes.search_albums, artists: sizes.search_artists };
        let Ok(Page::Found { v }) = self.read_now(read).await else { return failed() };
        let mut p = self.songs_page(library_songs(v.songs), false);
        p.folders = v.artists.iter().filter(|a| !a.is_external).map(|a| artist_folder(a, Some(CarGroup::Artists))).collect();
        p.folders.extend(album_folders(&v.albums, Some(CarGroup::Albums), usize::MAX));
        p.songs_group = Some(CarGroup::Songs);
        p
    }

    /// Home: favourites and today's mixes as on the phone's Home, then what was played and added lately.
    async fn car_home(&self) -> BrowsePage {
        if self.core.rules().feed {
            let Ok(Some(Page::Feed { v })) = self.cached_or_fetched(Read::HomeFeed).await else { return failed() };
            let mut songs = Vec::new();
            let mut folders = Vec::new();
            for shelf in v {
                for song in shelf.songs {
                    if !songs.iter().any(|s: &Song| s.id == song.id) { songs.push(song); }
                }
                folders.extend(album_folders(&shelf.albums, Some(CarGroup::Albums), usize::MAX));
                folders.extend(shelf.playlists.iter().map(playlist_folder));
                folders.extend(shelf.artists.iter().map(|a| artist_folder(a, Some(CarGroup::Artists))));
            }
            let mut page = self.songs_page(songs, false);
            page.folders = folders;
            page.songs_group = Some(CarGroup::Songs);
            return page;
        }
        let taste = self.settings().in_effect(|p| p.taste_model);
        // The starred songs handed to the mixes, as the Home row hands them before it is drawn.
        if let Ok(h) = self.mix_favourites_stored() {
            if !h.fresh {
                let _ = self.mix_favourites_refresh(h.digest).await;
            }
        }
        if taste {
            self.mix_warm_all().await;
        }
        let mut folders: Vec<BrowseFolder> = self.core.mix_cards(taste).iter().filter(|t| !t.covers.is_empty()).map(mix_folder).collect();
        let mut read = false;
        for (kind, group) in [(crate::browse::AlbumSort::Recent, CarGroup::RecentlyPlayed), (crate::browse::AlbumSort::Newest, CarGroup::RecentlyAdded)] {
            if let Ok(Some(Page::Albums { v })) = self.cached_or_fetched(Read::AlbumList { kind, size: SHELF as i32, offset: 0, genre: None }).await {
                read = true;
                folders.extend(album_folders(&v, Some(group), SHELF));
            }
        }
        BrowsePage { failed: folders.is_empty() && !read, folders, ..Default::default() }
    }

    /// The songs folder `parent` lists and plays, as its page has them; none when it cannot be read or
    /// is not a folder of songs.
    async fn folder_songs(&self, parent: &str) -> Option<Vec<Song>> {
        let (kind, arg) = parent.split_once(':').unwrap_or((parent, ""));
        let songs = match kind {
            "album" => self.cached_or_fetched(Read::AlbumById { id: arg.into() }).await.ok()??.songs(),
            "playlist" => self.cached_or_fetched(Read::PlaylistById { id: arg.into() }).await.ok()??.songs(),
            "mix" => {
                if arg != FAVOURITES_MIX {
                    self.mix_ensure(arg.into(), false).await;
                }
                match self.core.mix_page(arg.into()) {
                    MixLookup::Ready { sheet } => sheet.songs,
                    _ => return None,
                }
            }
            "starred" => self.cached_or_fetched(Read::StarredItems).await.ok()??.songs(),
            "random" => self.songs(Read::RandomSongs { size: 50, genre: None }).await.ok()?,
            "downloads" => self.core.downloads(true).ok()?,
            "downloaded" => self.core.downloaded_album(arg.into()).ok()??.songs,
            "search" => {
                let sizes = crate::browse::library_sizes();
                match self.read_now(Read::Search { query: arg.into(), songs: sizes.search_songs, albums: 0, artists: 0 }).await.ok()? {
                    Page::Found { v } => v.songs,
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(library_songs(songs))
    }

    /// `songs` as a folder's rows, each marked downloaded or not, with Play and Shuffle above them when
    /// `actions`.
    fn songs_page(&self, songs: Vec<Song>, actions: bool) -> BrowsePage {
        let downloaded = {
            let held = self.core.transfers().held();
            songs.iter().map(|s| held.state(&crate::Keyed::key(s)) == crate::transfers::HeldState::Done).collect()
        };
        BrowsePage {
            downloaded,
            actions: if actions && !songs.is_empty() { whole() } else { Vec::new() },
            songs,
            ..Default::default()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ItemKey;
    use crate::client::tests::{block, client};
    use crate::client::NetProfile;

    #[test]
    fn folders_list() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let p = block(c.browse_children(ROOT.into(), 0, 100));
        assert_eq!(p.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["home", "library", "starred", "downloads"]);
        assert_eq!(c.car_root(3, true, 0, 100).folders[0].id, "downloads");
        assert!(block(c.browse_children("library".into(), 0, 100)).folders.iter().any(|f| f.id == "artists"));
        assert!(fake.asked.lock().is_empty(), "the root and the library's lists ask nothing of the server");

        // Playlists folder lists song counts.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"p1","name":"Evening","songCount":12}]}}}"#);
        let p = block(c.browse_children("playlists".into(), 0, 100));
        assert_eq!((p.folders[0].id.as_str(), p.folders[0].title.as_str(), p.folders[0].songs, p.folders[0].playable), ("playlist:p1", "Evening", Some(12), true));
        assert!(block(c.browse_children("nonsense".into(), 0, 100)).folders.is_empty());
    }

    const ALBUM: &str = r#"{"subsonic-response":{"status":"ok","album":{"id":"a1","name":"Monster","artist":"Future","song":[
        {"id":"s1","title":"One","album":"Monster"},{"id":"s2","title":"Two","album":"Monster"},{"id":"x","title":"Theirs","isExternal":true}]}}}"#;

    #[test]
    fn albums_play_as_queue() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.download_queue(["s1", "s2"].map(|id| Song { id: id.into(), ..Default::default() }).to_vec()).unwrap();
        c.core.download_settle(vec![ItemKey::own("s2")], vec![true]).unwrap();
        fake.answer(ALBUM);
        let p = block(c.browse_children("album:a1".into(), 0, 100));
        assert_eq!(p.actions, [CarAction::Play, CarAction::Shuffle]);
        assert_eq!(p.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s1", "s2"], "a provider's song is left out");
        assert_eq!(p.downloaded, [false, true], "only a finished download is marked");
        let asked = fake.asked().len();
        let q = block(c.car_queue(car_song_row("album:a1".into(), "s2".into()))).unwrap();
        assert_eq!((q.songs.len(), q.index, q.shuffle), (2, 1, false));
        assert_eq!(q.origin.unwrap().id, "a1");
        assert_eq!(fake.asked().len(), asked, "the list shown is played, not asked for again");
        assert!(block(c.car_queue("s1".into())).is_none(), "a bare song id is not a row");

        // A spoken album plays as its queue.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"album":[{"id":"a1","name":"Monster","artist":"Future"}]}}}"#);
        fake.answer(ALBUM);
        let q = block(c.car_voice(VoiceAsk { query: "monster".into(), focus: VoiceFocus::Album, album: Some("Monster".into()), ..Default::default() }));
        assert_eq!(q.songs.len(), 2);
        assert_eq!(q.origin, Some(nori_model::PageOrigin { kind: nori_model::OriginKind::Album, id: "a1".into() }));
        assert!(!q.shuffle);
    }

    #[test]
    fn an_unreadable_folder_says_it_failed() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.fail(crate::transport::FailureKind::Connect);
        assert!(block(c.browse_children("album:a1".into(), 0, 100)).failed);
        fake.fail(crate::transport::FailureKind::Connect);
        assert!(block(c.browse_children("albums:newest".into(), 0, 100)).failed);
    }

    #[test]
    fn folder_read_once_per_paging() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[{"id":"x","isDir":false},{"id":"y","isDir":false},{"id":"z","isDir":false}]}}}"#);
        let ids = |p: BrowsePage| p.songs.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(block(c.browse_children("random".into(), 0, 2))), ["x", "y"]);
        assert_eq!(ids(block(c.browse_children("random".into(), 1, 2))), ["z"]);
        let q = block(c.car_queue(car_song_row("random".into(), "z".into()))).unwrap();
        assert_eq!((q.songs.len(), q.index), (3, 2));
        assert_eq!(fake.asked().len(), 1, "later pages and the pick are of the same draw");
        let root = c.car_root(4, false, 1, 3);
        assert_eq!(root.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["downloads"]);
    }

    #[test]
    fn search_results_listed_from_the_search() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","searchResult3":{"song":[{"id":"s1","isDir":false},{"id":"s2","isDir":false}]}}}"#);
        let found = block(c.car_search("q".into()));
        let listed = block(c.browse_children("search:q".into(), 0, 50));
        assert_eq!(listed.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
        assert_eq!(listed.songs.len(), found.songs.len());
        assert_eq!(fake.asked().len(), 1, "the car's listing of the results asks the server again");
    }

    #[test]
    fn a_server_ignoring_the_offset_ends_the_list() {
        let page = (1..=500).map(|i| format!(r#"{{"id":"a{i}","name":"A{i}"}}"#)).collect::<Vec<_>>().join(",");
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        for _ in 0..3 {
            fake.answer(&format!(r#"{{"subsonic-response":{{"status":"ok","albumList2":{{"album":[{page}]}}}}}}"#));
        }
        let (mut rows, mut initials) = (Vec::new(), Vec::new());
        reach(&c, "genre:Rock", &mut rows, &mut initials);
        assert_eq!((rows.len(), fake.asked().len()), (500, 2));
    }

    /// Every row a car can reach from folder `parent`, asked whole as a car asks, in the order it lists
    /// them: folders, then songs as the rows that play them. Each list it is sent fits what media3 passes
    /// on; `seen` sees each.
    fn walk(c: &Client, parent: &str, rows: &mut Vec<String>, seen: &mut dyn FnMut(&str, &BrowsePage)) {
        let page = block(c.browse_children(parent.into(), 0, u32::MAX));
        assert!(page.folders.len() + page.songs.len() <= CAR_LIST_ROWS, "{parent} lists {} rows", page.folders.len() + page.songs.len());
        seen(parent, &page);
        for f in page.folders {
            if f.media == CarMedia::Part {
                walk(c, &f.id, rows, seen);
            } else {
                rows.push(f.id);
            }
        }
        rows.extend(page.songs.iter().map(|s| car_song_row(parent.into(), s.id.clone())));
    }

    /// [`walk`] over an A to Z list: each list goes by initial; `initials` are its initials' folders.
    fn reach(c: &Client, parent: &str, rows: &mut Vec<String>, initials: &mut Vec<String>) {
        let letter = |t: &str| match t.chars().next() {
            Some('é') => 'E',
            Some('Ø') => 'O',
            Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
            _ => '#',
        };
        walk(c, parent, rows, &mut |parent, page| {
            let letters: Vec<char> = page.folders.iter().filter(|f| f.media != CarMedia::Part).map(|f| letter(&f.title)).collect();
            assert!(letters.is_sorted(), "{parent} goes by initial: {letters:?}");
            initials.extend(page.folders.iter().filter(|f| f.media == CarMedia::Part && f.subtitle.is_none()).map(|f| f.title.clone()));
        });
    }

    #[test]
    fn a_long_folder_reaches_the_car_in_order() {
        let songs = |n: usize| (0..n).map(|i| format!(r#"{{"id":"s{}","title":"Song {i}"}}"#, if i == 450 { 10 } else { i })).collect::<Vec<_>>().join(",");
        let playlist = format!(r#"{{"subsonic-response":{{"status":"ok","playlist":{{"id":"p1","name":"Long","entry":[{}]}}}}}}"#, songs(520));
        let albums: Vec<String> = (0..12).map(|i| format!(r#"{{"id":"l{i}","name":"Album {i}"}}"#)).collect();
        let artists: Vec<String> = (0..3).map(|i| format!(r#"{{"id":"r{i}","name":"Artist {i}"}}"#)).collect();
        let starred = format!(r#"{{"subsonic-response":{{"status":"ok","starred2":{{"song":[{}],"album":[{}],"artist":[{}]}}}}}}"#, songs(450), albums.join(","), artists.join(","));
        let many: Vec<String> = (0..260).map(|i| format!(r#"{{"id":"l{i}","name":"Album {i}"}}"#)).collect();
        let artist = format!(r#"{{"subsonic-response":{{"status":"ok","artist":{{"id":"r1","name":"Prolific","album":[{}]}}}}}}"#, many.join(","));
        let in_order = |parent: &str, n: usize| (0..n).map(|i| car_song_row(parent.into(), format!("s{}", if i == 450 { 10 } else { i }))).collect::<Vec<_>>();
        let folders = |prefix: &str, n: usize| (0..n).map(move |i| format!("{prefix}{i}")).collect::<Vec<_>>();
        let starred_rows: Vec<String> = folders("album:l", 12).into_iter().chain(folders("artist:r", 3)).chain(in_order("starred", 450)).collect();
        for (parent, answer, want) in [
            ("playlist:p1", playlist, in_order("playlist:p1", 520)),
            ("starred", starred, starred_rows),
            ("artist:r1", artist, folders("album:l", 260)),
        ] {
            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            fake.answer(&answer);
            let mut rows = Vec::new();
            walk(&c, parent, &mut rows, &mut |_, _| {});
            // A song's row names the run it was picked in; the run is part of the folder it plays.
            let whole: Vec<String> = rows.iter().map(|r| match car_row(r.clone()) {
                Some(CarRow { parent: p, song: Some(s), .. }) => car_song_row(run_part(&p).map_or(p.as_str(), |(_, w)| w).into(), s),
                _ => r.clone(),
            }).collect();
            assert_eq!(whole, want, "{parent}: every row once, in order");
            assert_eq!(fake.asked().len(), 1, "{parent}: its runs are of the one read");
        }

        // A song picked in a run plays the whole playlist from where it was picked.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(&format!(r#"{{"subsonic-response":{{"status":"ok","playlist":{{"id":"p1","name":"Long","entry":[{}]}}}}}}"#, songs(520)));
        let top = block(c.browse_children("playlist:p1".into(), 0, u32::MAX));
        assert_eq!(top.actions, [CarAction::Play, CarAction::Shuffle]);
        let run = &top.folders[2].id;
        block(c.browse_children(run.clone(), 0, u32::MAX));
        for (song, index) in [("s10", 450), ("s519", 519)] {
            let q = block(c.car_queue(car_song_row(run.clone(), song.into()))).unwrap();
            assert_eq!((q.songs.len(), q.index), (520, index), "{song}");
            assert_eq!(q.origin, Some(nori_model::PageOrigin::new(nori_model::OriginKind::Playlist, "p1")));
        }
        let q = block(c.car_queue(car_action_row("playlist:p1".into(), CarAction::Play))).unwrap();
        assert_eq!((q.songs.len(), q.index), (520, 0));
        assert_eq!(fake.asked().len(), 1);
    }

    #[test]
    fn every_initial_reaches_the_car() {
        // 520 names in the server's order, not the car's: "The" and accents where a server's sort name
        // puts them, more under S than one car list holds.
        let names: Vec<String> = (0..520).map(|i| match i % 13 {
            0 => format!("The Band {i}"),
            1 => format!("émile {i}"),
            2 => format!("{i} Degrees"),
            3..=8 => format!("Sound {i}"),
            9 => format!("Øystein {i}"),
            10 => format!("Ana {i}"),
            11 => format!("zeal {i}"),
            _ => format!("Kai {i}"),
        }).rev().collect();
        let artists: Vec<String> = names.iter().enumerate().map(|(i, n)| format!(r#"{{"id":"r{i}","name":"{n}"}}"#)).collect();
        let albums = |from: usize, to: usize| {
            let v: Vec<String> = (from..to).map(|i| format!(r#"{{"id":"l{i}","name":"{}"}}"#, names[i])).collect();
            format!(r#"{{"subsonic-response":{{"status":"ok","albumList2":{{"album":[{}]}}}}}}"#, v.join(","))
        };
        let artists = format!(r#"{{"subsonic-response":{{"status":"ok","artists":{{"index":[{{"name":"A","artist":[{}]}}]}}}}}}"#, artists.join(","));
        let lists = [("artists", "artist:r", vec![artists]), ("albums:alphabeticalByName", "album:l", vec![albums(0, 500), albums(500, 520)]), ("genre:Rock", "album:l", vec![albums(0, 500), albums(500, 520)])];
        for (list, prefix, answers) in lists {
            let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
            answers.iter().for_each(|a| fake.answer(a));
            let (mut rows, mut initials) = (Vec::new(), Vec::new());
            reach(&c, list, &mut rows, &mut initials);
            let mut want: Vec<String> = (0..520).map(|i| format!("{prefix}{i}")).collect();
            rows.sort();
            want.sort();
            assert_eq!(rows, want, "{list}: every row once");
            assert_eq!(initials, ["#", "A", "E", "K", "O", "S", "T", "Z"], "{list}");
            let top = block(c.browse_children(list.into(), 0, u32::MAX));
            assert_eq!(top.actions.is_empty(), !list.starts_with("genre:"), "{list}: a genre plays whole");
        }
    }

    #[test]
    fn album_opens_offline_from_cache() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.cache_put("page:album/a".into(), br#"{"subsonic-response":{"status":"ok","album":{"id":"a","name":"A","song":[{"id":"s","isDir":false}]}}}"#.to_vec()).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts=0", []).unwrap();
        fake.fail(crate::transport::FailureKind::Connect);
        assert_eq!(block(c.browse_children("album:a".into(), 0, 10)).songs.len(), 1);
    }

    #[test]
    fn stale_favorites_refresh_in_the_car() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.cache_put("page:starred".into(), br#"{"subsonic-response":{"status":"ok","starred2":{}}}"#.to_vec()).unwrap();
        c.core.db.lock().execute("UPDATE cache SET ts=0", []).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","starred2":{"song":[{"id":"favorite","title":"Favorite"}]}}}"#);
        let page = block(c.browse_children("starred".into(), 0, 100));
        assert_eq!(page.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["favorite"]);
        assert_eq!(fake.asked().len(), 1);
    }

    #[test]
    fn downloads_browse_and_play_only_finished_album_tracks() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let songs = ["two", "one", "pending", "loose"].map(|id| Song {
            id: id.into(), album_id: (id != "loose").then(|| "album".into()),
            track: if id == "one" { 1 } else { 2 }, ..Default::default()
        }).to_vec();
        c.core.download_queue(songs).unwrap();
        c.core.download_settle(vec![ItemKey::own("one"), ItemKey::own("two"), ItemKey::own("loose")], vec![true; 3]).unwrap();
        let root = block(c.browse_children("downloads".into(), 0, 100));
        assert_eq!(root.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["downloaded:album"]);
        assert_eq!(root.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["loose"]);
        let album = block(c.browse_children(root.folders[0].id.clone(), 0, 100));
        assert_eq!(album.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(album.downloaded, [true, true]);
        let queue = block(c.car_queue(car_action_row("downloaded:album".into(), CarAction::Play))).unwrap();
        assert_eq!(queue.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(queue.origin, Some(nori_model::PageOrigin::new(nori_model::OriginKind::Downloads, "album")));
        let all = block(c.car_queue(car_action_row("downloads".into(), CarAction::Play))).unwrap();
        assert_eq!(all.songs.len(), 3);
        assert!(fake.asked().is_empty(), "browsing and playing downloads works offline");
    }
}
