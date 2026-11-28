//! Home as a start shows it: everything this device keeps for the page, read at once and in one piece, so
//! the page is whole in its first frame; the server's answers follow ([`Client::read_refreshed`]).

use crate::browse::{home_pinned, home_rows_shown, home_shelves, shelf_albums_read, HomeShelf};
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult};
use crate::library::{local_epoch_day, QueueOffer};
use crate::mixes::board::MixTile;
use crate::settings::HomeRow;
use crate::{Album, FeedShelf, Playlist, Song};

/// What one Home shelf holds.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ShelfItems {
    Albums { v: Vec<Album> },
    Playlists { v: Vec<Playlist> },
    Songs { v: Vec<Song> },
}

/// Home from what this device keeps.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HomeKept {
    /// "For you": the favourites' tile, then the mixes; empty when the page has none.
    pub mixes: Vec<MixTile>,
    /// The sources' own Homes (`ProfileRules::feed`) as last shown, one after another; None for a
    /// library's Home or none kept.
    pub feed: Option<Vec<FeedShelf>>,
    /// The shelves of the rows asked for, in their order: what is kept, None where nothing is yet. None
    /// when no source is a library: a service's Home is its feed alone.
    pub shelves: Vec<Option<ShelfItems>>,
    /// The order the shelves and the feed's shelves show in ([`home_woven`]).
    pub places: Vec<HomePlace>,
    /// The queue left on the server as it last answered, when it is worth offering.
    pub offer: Option<QueueOffer>,
}

/// One of Home's shelves in the order they show: row `at` of the library's, or shelf `at` of the
/// sources' own feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum HomePlace {
    Row { at: u32 },
    Feed { at: u32 },
}

/// What a shelf shows, as it is told apart from another's: each item's kind and plain name and artist.
fn looks(shelf: Either<'_>) -> std::collections::HashSet<(u8, String, String)> {
    use nori_library::merged::plain;
    let songs = |v: &[Song]| v.iter().map(|s| (0, plain(&s.title), plain(&s.artist))).collect::<Vec<_>>();
    let albums = |v: &[Album]| v.iter().map(|a| (1, plain(&a.name), plain(&a.artist))).collect::<Vec<_>>();
    let playlists = |v: &[Playlist]| v.iter().map(|p| (2, plain(&p.name), String::new())).collect::<Vec<_>>();
    match shelf {
        Either::Row(Some(ShelfItems::Songs { v })) => songs(v).into_iter().collect(),
        Either::Row(Some(ShelfItems::Albums { v })) => albums(v).into_iter().collect(),
        Either::Row(Some(ShelfItems::Playlists { v })) => playlists(v).into_iter().collect(),
        Either::Row(None) => Default::default(),
        Either::Feed(f) => songs(&f.songs).into_iter().chain(albums(&f.albums)).chain(playlists(&f.playlists)).chain(f.artists.iter().map(|a| (3, nori_library::merged::plain(&a.name), String::new()))).collect(),
    }
}

enum Either<'a> {
    Row(&'a Option<ShelfItems>),
    Feed(&'a FeedShelf),
}

/// Home's shelves in the order they show: the library's rows (`rows`, across every source) and the
/// sources' own feed shelves (`feed`) in turns, a row first, the rest of the longer after. A shelf
/// near-identical to the one shown before it (at least half of the smaller's items in the other) is
/// left out: a service's feed often holds the library's own shelf again.
pub fn home_woven(rows: &[Option<ShelfItems>], feed: &[FeedShelf]) -> Vec<HomePlace> {
    let mut out = Vec::new();
    let mut before = std::collections::HashSet::new();
    let (mut r, mut f) = (0, 0);
    while r < rows.len() || f < feed.len() {
        let row_turn = r < rows.len() && (f >= feed.len() || r <= f);
        let (place, shown) = if row_turn {
            r += 1;
            (HomePlace::Row { at: r as u32 - 1 }, looks(Either::Row(&rows[r - 1])))
        } else {
            f += 1;
            (HomePlace::Feed { at: f as u32 - 1 }, looks(Either::Feed(&feed[f - 1])))
        };
        let smaller = shown.len().min(before.len());
        let shared = shown.intersection(&before).count();
        if smaller > 0 && shared * 2 >= smaller {
            continue;
        }
        out.push(place);
        before = shown;
    }
    out
}

/// [`home_woven`] for a client that holds the shelves as lists.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_places(rows: Vec<Option<ShelfItems>>, feed: Vec<FeedShelf>) -> Vec<HomePlace> {
    home_woven(&rows, &feed)
}

/// What a shelf's page holds, as the shelf shows it; `pins` the pinned playlists' ids.
pub fn shelf_items(shelf: &HomeShelf, page: Page, pins: &[String]) -> Option<ShelfItems> {
    match (shelf, page) {
        (HomeShelf::Albums { .. }, Page::Albums { v }) => Some(ShelfItems::Albums { v }),
        (HomeShelf::Playlists { take }, Page::Playlists { v }) => Some(ShelfItems::Playlists { v: v.into_iter().take(*take as usize).collect() }),
        (HomeShelf::Pinned, Page::Playlists { v }) => Some(ShelfItems::Playlists { v: home_pinned(v, pins.to_vec()) }),
        _ => None,
    }
}

/// The read behind a shelf; None for one read from the offline index.
pub fn shelf_read(shelf: &HomeShelf) -> Option<Read> {
    match shelf {
        HomeShelf::Albums { sort, size, follows_stars } => Some(shelf_albums_read(*sort, *size, *follows_stars, 0)),
        HomeShelf::Playlists { .. } | HomeShelf::Pinned => Some(Read::PlaylistList),
        HomeShelf::Songs { .. } => None,
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// The Home rows the profile shows, in the order kept: a jam guest's only the server's album shelves.
    pub fn home_rows(&self) -> Vec<HomeRow> {
        let kept = self.core.session.settings.prefs(|p| p.home_rows.clone());
        home_rows_shown(kept, self.core.rules().account)
    }

    /// Home's `rows` (some of [`Client::home_rows`]), with "For you" when `mixes`, from what this device
    /// keeps. Nothing is asked of the server: the stored favourites are handed to the mixes and the
    /// period's missing mixes drawn from the index, so it all comes at once. A profile of several sources
    /// has one Home: its rows across every source, the feeds of those that have their own Home woven in.
    pub fn home_kept(&self, rows: Vec<HomeRow>, mixes: bool) -> NetResult<HomeKept> {
        let mixes = if mixes && self.mixes_anywhere() {
            self.mix_favourites_stored()?;
            self.core.mix_warm(local_epoch_day());
            self.core.mix_cards(self.core.session.settings.in_effect(|p| p.taste_model))
        } else {
            Vec::new()
        };
        let feed = self.home_feed_kept()?;
        let pins = self.core.session.settings.prefs(|p| p.pinned_playlists.clone());
        let rows = if self.library_home() { rows } else { Vec::new() };
        let shelves = home_shelves(rows)
            .iter()
            .map(|shelf| match (shelf, shelf_read(shelf)) {
                (HomeShelf::Songs { sort, descending, limit }, _) => Ok(Some(ShelfItems::Songs { v: self.core.browse_songs(sort.clone(), *descending, false, 0, 0, 0, *limit)? })),
                (_, Some(read)) => Ok(self.read_kept(read)?.and_then(|p| shelf_items(shelf, p, &pins))),
                (_, None) => Ok(None),
            })
            .collect::<NetResult<Vec<_>>>()?;
        let places = home_woven(&shelves, feed.as_deref().unwrap_or_default());
        Ok(HomeKept { mixes, feed, shelves, places, offer: self.queue_offer_kept(false, None)? })
    }

    /// The server's answer to `read` when it differs from the page kept for it; None when it is the same,
    /// the kept one is fresh, or the server can't be reached while one is kept.
    pub async fn read_refreshed(&self, read: Read) -> NetResult<Option<Page>> {
        if !self.others_for(&read).is_empty() {
            return self.read_refreshed_merged(read).await;
        }
        let stored = self.read_stored(read.clone())?;
        if stored.fresh {
            return Ok(None);
        }
        self.read_refresh(read, stored.digest).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, client};
    use crate::client::NetProfile;

    fn albums(v: &Option<ShelfItems>) -> Vec<String> {
        match v {
            Some(ShelfItems::Albums { v }) => v.iter().map(|a| a.id.clone()).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn home_comes_whole_from_what_is_kept_without_asking() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        c.core.set_rules(crate::browse::ProfileRules { account: true, mixes: true, feed: false, ..c.core.rules() });
        c.core.session.settings.edit_by_name("queueOffer", "true").unwrap();
        let rows = vec![HomeRow::Recent, HomeRow::Playlists, HomeRow::Newest, HomeRow::TopSongs];
        fake.answer(r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"r1","name":"R"}]}}}"#);
        block(c.read_each(shelf_read(&home_shelves(vec![HomeRow::Recent])[0]).unwrap(), |_| {})).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"p1","name":"P"},{"id":"p2","name":"Q"}]}}}"#);
        block(c.read_each(Read::PlaylistList, |_| {})).unwrap();
        fake.answer(r#"{"subsonic-response":{"status":"ok","playQueue":{"current":"b","position":1200,"changedBy":"NavidromeUI","entry":[{"id":"a","title":"A","isDir":false},{"id":"b","title":"B","isDir":false}]}}}"#);
        assert!(block(c.queue_offer(false, None)).unwrap().is_some());
        let asked = fake.asked().len();

        let kept = c.home_kept(rows, true).unwrap();
        assert_eq!(fake.asked().len(), asked, "nothing asked of the server");
        assert_eq!(albums(&kept.shelves[0]), ["r1"]);
        assert!(matches!(&kept.shelves[1], Some(ShelfItems::Playlists { v }) if v.len() == 2));
        assert_eq!(kept.shelves[2], None, "nothing kept for it yet");
        assert_eq!(kept.shelves[3], Some(ShelfItems::Songs { v: Vec::new() }), "the offline index, empty");
        assert_eq!(kept.mixes[0].id, crate::mixes::board::FAVOURITES_MIX);
        assert_eq!(kept.offer.map(|o| o.index), Some(1), "the queue the server last offered");
        assert!(c.home_kept(Vec::new(), false).unwrap().mixes.is_empty());
    }

    /// Rows and feed shelves take turns, a row first; a shelf repeating most of the one before it is left
    /// out, and a shelf not read yet repeats nothing.
    #[test]
    fn home_weaves_rows_and_feeds_without_repeating_a_shelf() {
        let album = |name: &str| Album { name: name.into(), artist: "A".into(), ..Default::default() };
        let song = |title: &str| Song { title: title.into(), artist: "A".into(), ..Default::default() };
        let row = |names: &[&str]| Some(ShelfItems::Albums { v: names.iter().map(|n| album(n)).collect() });
        let shelf = |songs: &[&str], albums: &[&str]| FeedShelf { songs: songs.iter().map(|t| song(t)).collect(), albums: albums.iter().map(|n| album(n)).collect(), ..Default::default() };
        use HomePlace::{Feed, Row};
        let rows = [row(&["One", "Two"]), None, row(&["Three"])];
        let feed = [shelf(&[], &["two", "one", "Seven"]), shelf(&["Hey"], &[]), shelf(&["Hey", "Jude"], &[])];
        assert_eq!(home_woven(&rows, &feed), [Row { at: 0 }, Row { at: 1 }, Feed { at: 1 }, Row { at: 2 }, Feed { at: 2 }], "the feed's first repeats row 0; row 1 is not read yet");
        assert_eq!(home_woven(&rows, &[]), [Row { at: 0 }, Row { at: 1 }, Row { at: 2 }], "a library's Home");
        assert_eq!(home_woven(&[], &feed), [Feed { at: 0 }, Feed { at: 1 }], "a service's Home, its repeated shelf once");
    }

    #[test]
    fn a_rows_full_view_opens_on_what_its_shelf_shows() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let list = |ids: &[&str]| {
            let albums: Vec<String> = ids.iter().map(|id| format!(r#"{{"id":"{id}","name":"{id}"}}"#)).collect();
            format!(r#"{{"subsonic-response":{{"status":"ok","albumList2":{{"album":[{}]}}}}}}"#, albums.join(","))
        };
        let first_page = shelf_read(&crate::browse::home_row_page(HomeRow::Newest, 0).unwrap()).unwrap();
        // The full view opened once, then albums were added and Home showed them.
        fake.answer(&list(&["a", "b"]));
        block(c.read_fetch(first_page.clone(), None, None)).unwrap();
        fake.answer(&list(&["c", "a", "b"]));
        block(c.read_fetch(shelf_read(&home_shelves(vec![HomeRow::Newest])[0]).unwrap(), None, None)).unwrap();

        let shelf = albums(&c.home_kept(vec![HomeRow::Newest], false).unwrap().shelves[0]);
        let opened = c.read_stored(first_page).unwrap().page.map(|p| p.albums().into_iter().map(|a| a.id).collect::<Vec<_>>());
        assert_eq!(shelf, ["c", "a", "b"]);
        assert_eq!(opened, Some(shelf), "the full view's first page is the shelf's own");
    }

    #[test]
    fn a_refresh_brings_only_what_changed() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let queue = |at: u32| format!(r#"{{"subsonic-response":{{"status":"ok","playQueue":{{"current":"a","position":{at},"changedBy":"x","entry":[{{"id":"a","title":"A","isDir":false}}]}}}}}}"#);
        fake.answer(&queue(1));
        assert!(block(c.read_refreshed(Read::PullQueue)).unwrap().is_some(), "nothing kept: the server's");
        fake.answer(&queue(1));
        assert!(block(c.read_refreshed(Read::PullQueue)).unwrap().is_none(), "the same again");
        fake.answer(&queue(2));
        assert!(block(c.read_refreshed(Read::PullQueue)).unwrap().is_some(), "changed");
        fake.fail(crate::transport::FailureKind::Connect);
        assert!(block(c.read_refreshed(Read::PullQueue)).unwrap().is_none(), "unreachable, one kept");
    }
}
