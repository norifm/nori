//! This session's star changes, laid over favourites the server sent earlier: an unstarred item leaves at
//! once; a newly starred one arrives with the next answer (the old one lacks its details). Each source's
//! core keeps its own ([`Marks`]); a profile shows them all by key ([`StarMarks`]).

use std::collections::{HashMap, HashSet};

use nori_model::{Album, Artist, ItemKey, Song};
use nori_net::requests::Starrable;

use crate::pages::Starred;

/// This session's marks in one source's core, one id -> starred map per kind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marks {
    songs: HashMap<String, bool>,
    albums: HashMap<String, bool>,
    artists: HashMap<String, bool>,
}

impl Marks {
    fn of(&self, kind: Starrable) -> &HashMap<String, bool> {
        match kind {
            Starrable::Song => &self.songs,
            Starrable::Album => &self.albums,
            Starrable::Artist => &self.artists,
        }
    }

    fn of_mut(&mut self, kind: Starrable) -> &mut HashMap<String, bool> {
        match kind {
            Starrable::Song => &mut self.songs,
            Starrable::Album => &mut self.albums,
            Starrable::Artist => &mut self.artists,
        }
    }

    /// Marks item `id` of `kind`: a press, at once, before the server is asked, or a mark received from
    /// another device. The mark before it, for [`Marks::restore`].
    pub fn insert(&mut self, kind: Starrable, id: String, on: bool) -> Option<bool> {
        self.of_mut(kind).insert(id, on)
    }

    /// This session's mark of item `id` of `kind`, if any.
    pub fn get(&self, kind: Starrable, id: &str) -> Option<bool> {
        self.of(kind).get(id).copied()
    }

    /// The server refused the press that marked `pressed`: the mark from before comes back, unless a
    /// later press changed it.
    pub fn restore(&mut self, kind: Starrable, id: String, pressed: bool, previous: Option<bool>) {
        let marks = self.of_mut(kind);
        if marks.get(&id) == Some(&pressed) {
            match previous {
                Some(on) => marks.insert(id, on),
                None => marks.remove(&id),
            };
        }
    }

    /// Whether the item shows starred: this session's mark, else what its record says (`listed`).
    pub fn starred(&self, kind: Starrable, id: &str, listed: bool) -> bool {
        self.get(kind, id).unwrap_or(listed)
    }

    /// False only when this session unstarred the item; an item without a mark keeps whatever the list says.
    pub fn kept(&self, kind: Starrable, id: &str) -> bool {
        self.get(kind, id) != Some(false)
    }

    /// The songs this session starred, by id.
    pub fn songs_starred(&self) -> impl Iterator<Item = &String> {
        self.songs.iter().filter(|(_, on)| **on).map(|(id, _)| id)
    }

    /// The favourites without what this session unstarred.
    pub fn overlay(&self, starred: Starred) -> Starred {
        let artists: Vec<Artist> = starred.artists.into_iter().filter(|a| self.kept(Starrable::Artist, &a.id)).collect();
        let songs: Vec<Song> = starred.songs.into_iter().filter(|s| self.kept(Starrable::Song, &s.id)).collect();
        Starred::new(artists, self.overlay_albums(starred.albums), songs)
    }

    /// Albums without those this session unstarred (the home page's favourites shelf).
    pub fn overlay_albums(&self, albums: Vec<Album>) -> Vec<Album> {
        albums.into_iter().filter(|a| self.kept(Starrable::Album, &a.id)).collect()
    }
}

/// The marks a profile shows: this session's marks in each of its sources, each item by its key, so one
/// id in two sources is two marks. A client keeps the last it was handed and looks items up in it.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StarMarks {
    pub songs: HashMap<ItemKey, bool>,
    pub albums: HashMap<ItemKey, bool>,
    pub artists: HashMap<ItemKey, bool>,
}

impl StarMarks {
    fn of_mut(&mut self, kind: Starrable) -> &mut HashMap<ItemKey, bool> {
        match kind {
            Starrable::Song => &mut self.songs,
            Starrable::Album => &mut self.albums,
            Starrable::Artist => &mut self.artists,
        }
    }

    /// These and `marks`, the marks of `source` (None: the profile's own).
    pub fn with(mut self, source: Option<&str>, marks: &Marks) -> StarMarks {
        for kind in [Starrable::Song, Starrable::Album, Starrable::Artist] {
            let key = |id: &String| ItemKey { source: source.map(String::from), id: id.clone() };
            self.of_mut(kind).extend(marks.of(kind).iter().map(|(id, on)| (key(id), *on)));
        }
        self
    }

    /// Item `key` of `kind` marked at once, as a press shows it before the core's marks come.
    pub fn mark(&mut self, kind: Starrable, key: ItemKey, on: bool) {
        self.of_mut(kind).insert(key, on);
    }

    /// Whether item `key` of `kind` shows starred: this session's mark in its source, else what its
    /// record says (`listed`).
    pub fn starred(&self, kind: Starrable, key: &ItemKey, listed: bool) -> bool {
        let marks = match kind {
            Starrable::Song => &self.songs,
            Starrable::Album => &self.albums,
            Starrable::Artist => &self.artists,
        };
        marks.get(key).copied().unwrap_or(listed)
    }
}

/// Items by kind, each set of ids apart: the favourites a source listed last (an item among them shows
/// starred where its own record does not say so: a YouTube Music song played from an album, a search or
/// a radio), or those kept on this device.
#[derive(Debug, Default)]
pub struct Ids {
    songs: HashSet<String>,
    albums: HashSet<String>,
    artists: HashSet<String>,
}

impl Ids {
    /// The items of `starred`.
    pub fn of(starred: &Starred) -> Ids {
        Ids {
            songs: starred.songs.iter().map(|s| s.id.clone()).collect(),
            albums: starred.albums.iter().map(|a| a.id.clone()).collect(),
            artists: starred.artists.iter().map(|a| a.id.clone()).collect(),
        }
    }

    fn of_kind(&mut self, kind: Starrable) -> &mut HashSet<String> {
        match kind {
            Starrable::Song => &mut self.songs,
            Starrable::Album => &mut self.albums,
            Starrable::Artist => &mut self.artists,
        }
    }

    /// Whether item `id` of `kind` is among them.
    pub fn has(&self, kind: Starrable, id: &str) -> bool {
        match kind {
            Starrable::Song => self.songs.contains(id),
            Starrable::Album => self.albums.contains(id),
            Starrable::Artist => self.artists.contains(id),
        }
    }

    /// Adds item `id` of `kind` (`on`) or takes it out.
    pub fn set(&mut self, kind: Starrable, id: &str, on: bool) {
        let ids = self.of_kind(kind);
        if on {
            ids.insert(id.to_string());
        } else {
            ids.remove(id);
        }
    }
}

/// The source's favourites with those kept on this device after them, each item once.
pub fn with_here(source: Starred, here: Starred) -> Starred {
    fn join<T>(mut first: Vec<T>, then: Vec<T>, id: impl Fn(&T) -> &str) -> Vec<T> {
        let seen: HashSet<String> = first.iter().map(|x| id(x).to_string()).collect();
        first.extend(then.into_iter().filter(|x| !seen.contains(id(x))));
        first
    }
    Starred::new(join(source.artists, here.artists, |a| &a.id), join(source.albums, here.albums, |a| &a.id), join(source.songs, here.songs, |s| &s.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids are kept by kind: a song among them is not the album of its id; the set from a listing is
    /// that listing's alone.
    #[test]
    fn ids_by_kind() {
        let s = |id: &str| Song { id: id.into(), ..Default::default() };
        let mut ids = Ids::of(&Starred::new(vec![], vec![], vec![s("rap-god")]));
        assert!(ids.has(Starrable::Song, "rap-god"));
        assert!(!ids.has(Starrable::Album, "rap-god"));
        ids.set(Starrable::Album, "rap-god", true);
        ids.set(Starrable::Song, "rap-god", false);
        assert_eq!((ids.has(Starrable::Song, "rap-god"), ids.has(Starrable::Album, "rap-god")), (false, true));
        assert!(!Ids::of(&Starred::new(vec![], vec![], vec![])).has(Starrable::Album, "rap-god"), "a new listing");
    }

    /// Those kept here follow the source's, and an item the source lists too is shown once, as the
    /// source has it.
    #[test]
    fn kept_here_follow_the_source() {
        let s = |id: &str, title: &str| Song { id: id.into(), title: title.into(), ..Default::default() };
        let a = |id: &str| Album { id: id.into(), ..Default::default() };
        let source = Starred::new(vec![], vec![a("l1")], vec![s("1", "account"), s("2", "account")]);
        let here = Starred::new(vec![Artist { id: "r1".into(), ..Default::default() }], vec![a("l2"), a("l1")], vec![s("3", "here"), s("2", "here")]);
        let got = with_here(source, here);
        assert_eq!(got.songs.iter().map(|s| (s.id.as_str(), s.title.as_str())).collect::<Vec<_>>(), [("1", "account"), ("2", "account"), ("3", "here")]);
        assert_eq!(got.albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["l1", "l2"]);
        assert_eq!((got.artists.len(), got.library_songs), (1, 3));
    }

    #[test]
    fn star_marks() {
        let s = |id: &str| Song { id: id.into(), ..Default::default() };
        let a = |id: &str| Album { id: id.into(), ..Default::default() };
        let r = |id: &str| Artist { id: id.into(), ..Default::default() };
        let starred = Starred::new(vec![r("1"), r("2")], vec![a("1"), a("2")], vec![s("1"), s("2"), s("3")]);
        // The same id under another kind does not count: an album "1" unstarred is not song "1".
        let mut m = Marks::default();
        m.insert(Starrable::Song, "2".into(), false);
        m.insert(Starrable::Song, "3".into(), true);
        m.insert(Starrable::Album, "1".into(), false);
        m.insert(Starrable::Artist, "9".into(), false);
        let out = m.overlay(starred);
        assert_eq!(out.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        assert_eq!(out.albums.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["2"]);
        assert_eq!(out.artists.len(), 2);
        assert_eq!(out.library_songs, 2, "counted after the overlay");
        assert_eq!(m.overlay_albums(vec![a("1"), a("2")]).len(), 1);
        assert_eq!(Marks::default().overlay_albums(vec![a("1")]).len(), 1);

        // Refused star restores mark.
        let mut m = Marks::default();
        let first = m.insert(Starrable::Album, "1".into(), true);
        assert_eq!(first, None);
        let second = m.insert(Starrable::Album, "1".into(), false);
        assert_eq!(second, Some(true));
        m.restore(Starrable::Album, "1".into(), true, first);
        assert_eq!(m.get(Starrable::Album, "1"), Some(false), "a later press stays");
        m.restore(Starrable::Album, "1".into(), false, second);
        assert_eq!(m.get(Starrable::Album, "1"), Some(true));
        m.restore(Starrable::Album, "1".into(), true, first);
        assert_eq!(m.get(Starrable::Album, "1"), None);
        // Split by kind, keyed by the id alone: a song and an album may share an id.
        m.insert(Starrable::Song, "1".into(), true);
        assert_eq!((m.get(Starrable::Song, "1"), m.get(Starrable::Album, "1"), m.get(Starrable::Artist, "1")), (Some(true), None, None));
        m.insert(Starrable::Album, "2".into(), false);
        assert!(m.overlay_albums(vec![Album { id: "2".into(), ..Default::default() }]).is_empty());
        // A heart shows the mark over the record, either way.
        let shown = (m.starred(Starrable::Song, "1", false), m.starred(Starrable::Album, "2", true), m.starred(Starrable::Song, "7", true));
        assert_eq!(shown, (true, false, true));
    }

    /// A profile shows each source's marks on that source's items alone: one id in two sources is two
    /// marks, and an item with no mark shows its record.
    #[test]
    fn a_profile_shows_each_sources_marks_on_its_items() {
        let (mut own, mut other) = (Marks::default(), Marks::default());
        own.insert(Starrable::Song, "1".into(), true);
        other.insert(Starrable::Song, "1".into(), false);
        other.insert(Starrable::Album, "2".into(), true);
        let shown = StarMarks::default().with(None, &own).with(Some("acct2"), &other);
        let key = |source: Option<&str>, id: &str| ItemKey { source: source.map(String::from), id: id.into() };
        for (kind, item, listed, starred) in [
            (Starrable::Song, key(None, "1"), false, true),
            (Starrable::Song, key(Some("acct2"), "1"), true, false),
            (Starrable::Album, key(Some("acct2"), "2"), false, true),
            (Starrable::Album, key(None, "2"), false, false),
            (Starrable::Song, key(Some("acct3"), "1"), true, true),
        ] {
            assert_eq!(shown.starred(kind, &item, listed), starred, "{kind:?} {item}");
        }
        // A press marks its item alone.
        let mut pressed = shown.clone();
        pressed.mark(Starrable::Song, key(Some("acct2"), "1"), true);
        assert_eq!((pressed.starred(Starrable::Song, &key(Some("acct2"), "1"), false), pressed.starred(Starrable::Song, &key(None, "1"), false)), (true, true));
        pressed.mark(Starrable::Song, key(None, "1"), false);
        assert_eq!((pressed.starred(Starrable::Song, &key(Some("acct2"), "1"), false), pressed.starred(Starrable::Song, &key(None, "1"), true)), (true, false));
    }

}
