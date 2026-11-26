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
    /// A source's own Home (`ProfileRules::feed`) as last shown; None for a library's Home or none kept.
    pub feed: Option<Vec<FeedShelf>>,
    /// The shelves of the rows asked for, in their order: what is kept, None where nothing is yet.
    pub shelves: Vec<Option<ShelfItems>>,
    /// The queue left on the server as it last answered, when it is worth offering.
    pub offer: Option<QueueOffer>,
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
    /// period's missing mixes drawn from the index, so it all comes at once.
    pub fn home_kept(&self, rows: Vec<HomeRow>, mixes: bool) -> NetResult<HomeKept> {
        let rules = self.core.rules();
        let mixes = if mixes && rules.mixes {
            self.mix_favourites_stored()?;
            self.core.mix_warm(local_epoch_day());
            self.core.mix_cards(self.core.session.settings.in_effect(|p| p.taste_model))
        } else {
            Vec::new()
        };
        let feed = if rules.feed {
            match self.read_stored(Read::HomeFeed)?.page {
                Some(Page::Feed { v }) => Some(v),
                _ => None,
            }
        } else {
            None
        };
        let pins = self.core.session.settings.prefs(|p| p.pinned_playlists.clone());
        let rows = if rules.feed { Vec::new() } else { rows };
        let shelves = home_shelves(rows)
            .iter()
            .map(|shelf| match (shelf, shelf_read(shelf)) {
                (HomeShelf::Songs { sort, descending, limit }, _) => Ok(Some(ShelfItems::Songs { v: self.core.browse_songs(sort.clone(), *descending, false, 0, 0, 0, *limit)? })),
                (_, Some(read)) => Ok(self.read_stored(read)?.page.and_then(|p| shelf_items(shelf, p, &pins))),
                (_, None) => Ok(None),
            })
            .collect::<NetResult<Vec<_>>>()?;
        Ok(HomeKept { mixes, feed, shelves, offer: self.queue_offer_kept(false, None)? })
    }

    /// The server's answer to `read` when it differs from the page kept for it; None when it is the same,
    /// the kept one is fresh, or the server can't be reached while one is kept.
    pub async fn read_refreshed(&self, read: Read) -> NetResult<Option<Page>> {
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
        block(c.read_fetch(first_page.clone(), None)).unwrap();
        fake.answer(&list(&["c", "a", "b"]));
        block(c.read_fetch(shelf_read(&home_shelves(vec![HomeRow::Newest])[0]).unwrap(), None)).unwrap();

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
