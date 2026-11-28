//! One library over a profile's sources: when two sources' items are one (the exact match), which copy
//! shows, and their lists merged. A wrong merge hides a song the listener has, so the match is strict:
//! anything less shows both copies, each marked with its source.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

use nori_model::{Album, Artist, Playlist, SearchResult, Song};

use crate::browse::AlbumSort;

/// What a source holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SourceKind {
    /// The listener's own files: a Subsonic server's library, the files on a device.
    Library,
    /// A streaming service's catalog.
    Service,
}

/// One source's answer, and what the source holds.
pub struct Sourced<T> {
    pub kind: SourceKind,
    pub items: T,
}

/// Songs further apart than this in length are two recordings.
const SAME_LENGTH_S: u32 = 2;

/// An item two sources can both have.
pub trait OneItem: Clone {
    /// What this item is, from a source that holds `source`: a service's item a server only fetches
    /// (an octo-fiesta `ext-` id) is the service's.
    fn held(&self, source: SourceKind) -> SourceKind;

    /// Whether `self` and `other` are one item: the exact match.
    fn same(&self, other: &Self) -> bool;

    /// What two items that are [`OneItem::same`] have alike, so a long list finds the candidates at once;
    /// two items alike but not the same are look-alikes ([`source_marks`]).
    fn likeness(&self) -> String;

    /// The source it comes from (None: the profile's own).
    fn source(&self) -> &Option<String>;

    /// Shows its source's mark, or not ([`marked`]).
    fn set_source_marked(&mut self, on: bool);
}

/// `text` as matches compare it: lowercase, its spaces one each and none at the ends, a typographic
/// apostrophe a plain one. Nothing more: "Song (Live)" and "Song" stay two.
pub fn plain(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase().replace('\u{2019}', "'")
}

/// Both texts say the same once [`plain`], and say something.
fn alike(a: &str, b: &str) -> bool {
    let a = plain(a);
    !a.is_empty() && a == plain(b)
}

/// Both MusicBrainz ids where both items have one: their match settles it.
fn brainz(a: &Option<String>, b: &Option<String>) -> Option<bool> {
    let id = |i: &Option<String>| i.as_deref().map(str::trim).filter(|i| !i.is_empty()).map(str::to_ascii_lowercase);
    Some(id(a)? == id(b)?)
}

impl OneItem for Song {
    fn held(&self, source: SourceKind) -> SourceKind {
        if self.is_provider() { SourceKind::Service } else { source }
    }

    /// The same MusicBrainz recording; without one on both, the same title, artist and album, each
    /// given, with lengths known and at most [`SAME_LENGTH_S`] apart.
    fn same(&self, other: &Song) -> bool {
        brainz(&self.music_brainz_id, &other.music_brainz_id).unwrap_or_else(|| {
            alike(&self.title, &other.title)
                && alike(&self.artist, &other.artist)
                && alike(&self.album, &other.album)
                && self.duration > 0
                && other.duration > 0
                && self.duration.abs_diff(other.duration) <= SAME_LENGTH_S
        })
    }

    fn likeness(&self) -> String {
        format!("{}\0{}", plain(&self.title), plain(&self.artist))
    }

    fn source(&self) -> &Option<String> {
        &self.source
    }

    fn set_source_marked(&mut self, on: bool) {
        self.source_marked = on;
    }
}

impl OneItem for Album {
    fn held(&self, source: SourceKind) -> SourceKind {
        if self.is_provider() { SourceKind::Service } else { source }
    }

    /// The same MusicBrainz release; without one on both, the same name and artist, and the same year
    /// and number of songs where both say them.
    fn same(&self, other: &Album) -> bool {
        let agree = |a: u32, b: u32| a == 0 || b == 0 || a == b;
        brainz(&self.music_brainz_id, &other.music_brainz_id).unwrap_or_else(|| {
            alike(&self.name, &other.name) && alike(&self.artist, &other.artist) && agree(self.year, other.year) && agree(self.song_count, other.song_count)
        })
    }

    fn likeness(&self) -> String {
        format!("{}\0{}", plain(&self.name), plain(&self.artist))
    }

    fn source(&self) -> &Option<String> {
        &self.source
    }

    fn set_source_marked(&mut self, on: bool) {
        self.source_marked = on;
    }
}

impl OneItem for Artist {
    fn held(&self, source: SourceKind) -> SourceKind {
        if self.is_external { SourceKind::Service } else { source }
    }

    /// The same name.
    fn same(&self, other: &Artist) -> bool {
        alike(&self.name, &other.name)
    }

    fn likeness(&self) -> String {
        plain(&self.name)
    }

    fn source(&self) -> &Option<String> {
        &self.source
    }

    fn set_source_marked(&mut self, on: bool) {
        self.source_marked = on;
    }
}

impl OneItem for Playlist {
    fn held(&self, source: SourceKind) -> SourceKind {
        source
    }

    /// Never: a source's playlists stay its own.
    fn same(&self, _: &Playlist) -> bool {
        false
    }

    fn likeness(&self) -> String {
        plain(&self.name)
    }

    fn source(&self) -> &Option<String> {
        &self.source
    }

    fn set_source_marked(&mut self, on: bool) {
        self.source_marked = on;
    }
}

/// Which of `items`, a list shown, show their source's mark: those with a look-alike of another source
/// in the list (the same name and artist, but not one item by the exact match, so both show), to tell
/// the two apart. Nothing else shows a source.
fn source_marks<T: OneItem>(items: &[T]) -> Vec<bool> {
    let mut sources: HashMap<String, Vec<&Option<String>>> = HashMap::new();
    for item in items {
        sources.entry(item.likeness()).or_default().push(item.source());
    }
    items.iter().map(|item| sources[&item.likeness()].iter().any(|s| *s != item.source())).collect()
}

/// `items`, a list shown, each saying whether it shows its source's mark ([`source_marks`]).
pub fn marked<T: OneItem>(mut items: Vec<T>) -> Vec<T> {
    let marks = source_marks(&items);
    items.iter_mut().zip(marks).for_each(|(item, on)| item.set_source_marked(on));
    items
}

/// The sources' items as one list, in the sources' order: an item another source has already listed
/// shows once, as the copy of the listener's own library over a service's, else the first listed, in
/// the place the first took.
pub fn merged<T: OneItem>(lists: Vec<Sourced<Vec<T>>>) -> Vec<T> {
    let mut shown: Vec<(SourceKind, T)> = Vec::new();
    for list in lists {
        for item in list.items {
            let held = item.held(list.kind);
            match shown.iter_mut().find(|(_, s)| s.same(&item)) {
                Some(kept) if held < kept.0 => *kept = (held, item),
                Some(_) => {}
                None => shown.push((held, item)),
            }
        }
    }
    marked(shown.into_iter().map(|(_, item)| item).collect())
}

/// How a list of albums in `sort`'s order compares two albums of different sources: None where only
/// each source knows its order (shuffled, recently played, favorites), so their lists take turns.
pub fn album_order(sort: AlbumSort) -> Option<fn(&Album, &Album) -> Ordering> {
    fn by_name(a: &Album, b: &Album) -> Ordering {
        nori_db::natural_cmp(&a.name, &b.name)
    }
    fn by_artist(a: &Album, b: &Album) -> Ordering {
        nori_db::natural_cmp(&a.artist, &b.artist).then_with(|| by_name(a, b))
    }
    fn newest(a: &Album, b: &Album) -> Ordering {
        b.created.cmp(&a.created)
    }
    fn most_played(a: &Album, b: &Album) -> Ordering {
        b.play_count.cmp(&a.play_count)
    }
    fn by_year(a: &Album, b: &Album) -> Ordering {
        b.year.cmp(&a.year).then_with(|| by_name(a, b))
    }
    match sort {
        AlbumSort::ByName | AlbumSort::ByGenre => Some(by_name),
        AlbumSort::ByArtist | AlbumSort::ArtistRows => Some(by_artist),
        AlbumSort::Newest => Some(newest),
        AlbumSort::Frequent => Some(most_played),
        AlbumSort::ByYear => Some(by_year),
        AlbumSort::Recent | AlbumSort::Random | AlbumSort::Starred => None,
    }
}

/// One source's part of a [`ListMerge`]: its items read but not shown yet, how many it was asked for,
/// and whether it has no more.
struct Lane<T> {
    kind: SourceKind,
    waiting: VecDeque<T>,
    asked: u32,
    ended: bool,
}

/// A list read across sources a page at a time, in the list's order: each source is read on from where
/// it stopped, and the next item shown is the first in `order` among the sources' next ones (ties to
/// the earlier source), or with no order the next source's in turn. An item one shown already is
/// ([`merged`]'s rule) shows once.
pub struct ListMerge<T> {
    lanes: Vec<Lane<T>>,
    order: Option<fn(&T, &T) -> Ordering>,
    shown: Vec<(SourceKind, T)>,
    by_likeness: HashMap<String, Vec<usize>>,
    turn: usize,
}

impl<T: OneItem> ListMerge<T> {
    /// A list over sources holding `kinds`, in their order.
    pub fn new(kinds: Vec<SourceKind>, order: Option<fn(&T, &T) -> Ordering>) -> Self {
        let lanes = kinds.into_iter().map(|kind| Lane { kind, waiting: VecDeque::new(), asked: 0, ended: false }).collect();
        ListMerge { lanes, order, shown: Vec::new(), by_likeness: HashMap::new(), turn: 0 }
    }

    /// The source to read before items up to `upto` can show, and the offset to read it from; None once
    /// they show or every source has ended.
    pub fn wants(&mut self, upto: usize) -> Option<(usize, u32)> {
        loop {
            if self.shown.len() >= upto {
                return None;
            }
            if let Some(at) = self.lanes.iter().position(|l| l.waiting.is_empty() && !l.ended) {
                return Some((at, self.lanes[at].asked));
            }
            let at = self.next()?;
            let item = self.lanes[at].waiting.pop_front().expect("next has an item");
            let held = item.held(self.lanes[at].kind);
            self.show(held, item);
        }
    }

    /// Source `at`'s items from where it was read up to, `size` asked: fewer means it has no more. A
    /// source that could not be read is `ended` with none.
    pub fn got(&mut self, at: usize, items: Vec<T>, size: u32) {
        let lane = &mut self.lanes[at];
        lane.ended = (items.len() as u32) < size;
        lane.asked += items.len() as u32;
        lane.waiting.extend(items);
    }

    /// Items `from..from + n`, as far as they show, each marked as that page shows it ([`marked`]).
    pub fn page(&self, from: usize, n: usize) -> Vec<T> {
        marked(self.shown.iter().skip(from).take(n).map(|(_, i)| i.clone()).collect())
    }

    /// The source whose next item shows next; None when none has one.
    fn next(&mut self) -> Option<usize> {
        let ready = |l: &Lane<T>| !l.waiting.is_empty();
        let at = match self.order {
            Some(order) => (0..self.lanes.len()).filter(|&i| ready(&self.lanes[i])).min_by(|&a, &b| order(&self.lanes[a].waiting[0], &self.lanes[b].waiting[0]).then(a.cmp(&b)))?,
            None => {
                let n = self.lanes.len();
                let at = (0..n).map(|k| (self.turn + k) % n).find(|&i| ready(&self.lanes[i]))?;
                self.turn = (at + 1) % n;
                at
            }
        };
        Some(at)
    }

    /// `item` shown after the others, unless one shown is the same: then the listener's own copy stays.
    fn show(&mut self, held: SourceKind, item: T) {
        let key = item.likeness();
        let same = self.by_likeness.get(&key).and_then(|at| at.iter().copied().find(|&i| self.shown[i].1.same(&item)));
        match same {
            Some(i) if held < self.shown[i].0 => self.shown[i] = (held, item),
            Some(_) => {}
            None => {
                self.by_likeness.entry(key).or_default().push(self.shown.len());
                self.shown.push((held, item));
            }
        }
    }
}

/// The sources' search results as one: each kind of item [`merged`]; a source's playlists stay its own.
pub fn merged_found(found: Vec<Sourced<SearchResult>>) -> SearchResult {
    let mut parts: (Vec<_>, Vec<_>, Vec<_>) = Default::default();
    let mut playlists = Vec::new();
    for f in found {
        parts.0.push(Sourced { kind: f.kind, items: f.items.artists });
        parts.1.push(Sourced { kind: f.kind, items: f.items.albums });
        parts.2.push(Sourced { kind: f.kind, items: f.items.songs });
        playlists.extend(f.items.playlists);
    }
    SearchResult { artists: merged(parts.0), albums: merged(parts.1), songs: merged(parts.2), playlists: marked(playlists) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SourceKind::*;

    fn song(id: &str, title: &str, artist: &str, album: &str, duration: u32) -> Song {
        Song { id: id.into(), title: title.into(), artist: artist.into(), album: album.into(), duration, ..Default::default() }
    }

    fn ids(songs: &[Song]) -> Vec<&str> {
        songs.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn songs_are_one_only_on_an_exact_match() {
        let base = song("a", "Teardrop", "Massive Attack", "Mezzanine", 330);
        let brainz = |s: &Song, id: &str| Song { music_brainz_id: Some(id.into()), ..s.clone() };
        for (other, same, why) in [
            (song("b", " teardrop ", "MASSIVE  ATTACK", "mezzanine", 331), true, "case and spaces"),
            (song("b", "Teardrop", "Massive Attack", "Mezzanine", 332), true, "two seconds apart"),
            (song("b", "Teardrop", "Massive Attack", "Mezzanine", 333), false, "three seconds apart"),
            (song("b", "Teardrop", "Massive Attack", "Mezzanine", 0), false, "a length not known"),
            (song("b", "Teardrop (Live)", "Massive Attack", "Mezzanine", 330), false, "another version"),
            (song("b", "Teardrop", "Massive Attack feat. Liz Fraser", "Mezzanine", 330), false, "other artists"),
            (song("b", "Teardrop", "Massive Attack", "Mezzanine (Deluxe)", 330), false, "another album"),
            (song("b", "Teardrop", "Massive Attack", "", 330), false, "an album not known"),
        ] {
            assert_eq!(base.same(&other), same, "{why}");
            assert_eq!(other.same(&base), same, "{why}, either way");
        }
        assert!(brainz(&base, "mb-1").same(&brainz(&song("b", "Tear Drop", "Massive Attack", "", 0), "MB-1")), "one recording, whatever the tags");
        assert!(!brainz(&base, "x").same(&brainz(&base, "y")), "two recordings, whatever the tags");
        assert!(brainz(&base, "x").same(&base), "one id alone settles nothing: the tags do");
        assert!(!song("a", "", "", "", 1).same(&song("b", "", "", "", 1)), "nothing said is no match");
    }

    #[test]
    fn albums_and_artists_are_one_on_an_exact_match() {
        let album = |name: &str, year: u32, songs: u32| Album { name: name.into(), artist: "Air".into(), year, song_count: songs, ..Default::default() };
        assert!(album("Moon Safari", 1998, 10).same(&album("moon safari", 0, 10)), "a year not said");
        assert!(!album("Moon Safari", 1998, 10).same(&album("Moon Safari", 2018, 10)), "a reissue");
        assert!(!album("Moon Safari", 1998, 10).same(&album("Moon Safari", 1998, 12)), "more songs");
        let artist = |name: &str| Artist { name: name.into(), ..Default::default() };
        assert!(artist("Air").same(&artist(" air")));
        assert!(!artist("Air").same(&artist("Air France")));
    }

    #[test]
    fn the_listeners_own_copy_shows_where_two_are_one() {
        let own = song("nd", "Teardrop", "Massive Attack", "Mezzanine", 330);
        let theirs = song("yt", "Teardrop", "Massive Attack", "Mezzanine", 331);
        let other = song("yt2", "Angel", "Massive Attack", "Mezzanine", 380);
        let fetched = Song { id: "ext-1".into(), ..own.clone() };
        let list = |kind, songs: &[&Song]| Sourced { kind, items: songs.iter().map(|s| (*s).clone()).collect() };
        assert_eq!(ids(&merged(vec![list(Service, &[&theirs, &other]), list(Library, &[&own])])), ["nd", "yt2"], "the library's, where the service's stood");
        assert_eq!(ids(&merged(vec![list(Library, &[&own]), list(Service, &[&theirs])])), ["nd"]);
        assert_eq!(ids(&merged(vec![list(Service, &[&theirs]), list(Service, &[&own])])), ["yt"], "alike: the first");
        assert_eq!(ids(&merged(vec![list(Service, &[&theirs]), list(Library, &[&fetched])])), ["yt"], "a server's provider song is the service's");
        let near = song("yt3", "Teardrop", "Massive Attack", "Mezzanine", 340);
        assert_eq!(ids(&merged(vec![list(Library, &[&own]), list(Service, &[&near])])), ["nd", "yt3"], "not exact: both");
    }

    /// A list across sources is read a page at a time, each source on from where it stopped: in the list's
    /// order where both know it, else taking turns; an album both have shows once, the library's.
    #[test]
    fn lists_merge_in_order_a_page_at_a_time() {
        let album = |id: &str, name: &str| Album { id: id.into(), name: name.into(), artist: "A".into(), ..Default::default() };
        let library = [album("l1", "Alpha"), album("l2", "Delta"), album("l3", "Echo")];
        let service = [album("s1", "Bravo"), album("s2", "Delta"), album("s3", "Foxtrot")];
        let pages = |order, size: usize, n: usize| {
            let mut m = ListMerge::new(vec![Library, Service], order);
            let mut reads = Vec::new();
            let mut shown = Vec::new();
            for page in 0..n {
                let upto = (page + 1) * size;
                while let Some((at, from)) = m.wants(upto) {
                    reads.push((at, from));
                    let list = if at == 0 { &library } else { &service };
                    m.got(at, list.iter().skip(from as usize).take(size).cloned().collect(), size as u32);
                }
                shown.push(m.page(page * size, size).iter().map(|a| a.id.clone()).collect::<Vec<_>>());
            }
            (shown, reads)
        };
        let (shown, reads) = pages(album_order(AlbumSort::ByName), 2, 3);
        assert_eq!(shown, [vec!["l1", "s1"], vec!["l2", "l3"], vec!["s3"]], "Delta once, the library's");
        assert_eq!(reads, [(0, 0), (1, 0), (0, 2), (1, 2)], "each source read on from where it stopped, until a short page ends it");
        let (shown, _) = pages(album_order(AlbumSort::Random), 3, 2);
        assert_eq!(shown, [vec!["l1", "s1", "l2"], vec!["l3", "s3"]], "no order: turns, the service's Delta its library twin");
    }

    /// Only look-alikes of two sources in one list show their sources' marks.
    #[test]
    fn a_source_shows_only_beside_a_look_alike() {
        let album = |name: &str, source: Option<&str>| Album { name: name.into(), artist: "Daft Punk".into(), source: source.map(String::from), ..Default::default() };
        let list = [album("Random Access Memories", None), album("random access  memories", Some("yt")), album("Discovery", Some("yt")), album("Homework", None), album("Homework", None)];
        let shown = |list: Vec<Album>| marked(list).into_iter().map(|a| a.source_marked).collect::<Vec<_>>();
        assert_eq!(shown(list.to_vec()), [true, true, false, false, false], "two copies of one source show none");
        assert_eq!(shown(vec![album("Discovery", Some("yt"))]), [false], "a source's item alone");
        let merged_list = merged(vec![Sourced { kind: Library, items: vec![album("Homework", None)] }, Sourced { kind: Service, items: vec![Album { year: 1997, song_count: 16, ..album("Homework", Some("yt")) }, album("Alive", Some("yt"))] }]);
        assert_eq!(merged_list.iter().map(|a| a.source_marked).collect::<Vec<_>>(), [false, false], "one item by the exact match shows once, unmarked");
    }

    #[test]
    fn search_results_merge_by_kind_and_keep_each_sources_playlists() {
        let found = |song: Song, playlist: &str| SearchResult { songs: vec![song], playlists: vec![nori_model::Playlist { id: playlist.into(), ..Default::default() }], ..Default::default() };
        let one = merged_found(vec![
            Sourced { kind: Service, items: found(song("yt", "Angel", "Massive Attack", "Mezzanine", 380), "p1") },
            Sourced { kind: Library, items: found(song("nd", "Angel", "Massive Attack", "Mezzanine", 379), "p2") },
        ]);
        assert_eq!(ids(&one.songs), ["nd"]);
        assert_eq!(one.playlists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p1", "p2"]);
    }
}
