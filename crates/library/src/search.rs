//! Search results ranked and split once per answer into everything, the library's and the providers'
//! (`isExternal`), and the search screen's state.

use std::cmp::Reverse;
use std::collections::HashSet;

use nori_model::{is_provider_id, Album, SearchResult, Song};

#[derive(Debug, Clone, Default)]
pub struct SearchSplit {
    pub everything: SearchResult,
    /// Only the library's items; None when that is everything (there are no provider items).
    pub library: Option<SearchResult>,
    /// Only the providers' items; None when there are none.
    pub providers: Option<SearchResult>,
}

pub fn split(r: SearchResult) -> SearchSplit {
    let has_providers = r.songs.iter().any(|s| s.is_external) || r.albums.iter().any(|a| a.is_external) || r.artists.iter().any(|a| a.is_external) || r.playlists.iter().any(|p| is_provider_id(&p.id));
    if !has_providers {
        return SearchSplit { everything: r, library: None, providers: None };
    }
    let part = |external: bool| SearchResult {
        artists: r.artists.iter().filter(|a| a.is_external == external).cloned().collect(),
        albums: r.albums.iter().filter(|a| a.is_external == external).cloned().collect(),
        songs: r.songs.iter().filter(|s| s.is_external == external).cloned().collect(),
        playlists: r.playlists.iter().filter(|p| is_provider_id(&p.id) == external).cloned().collect(),
    };
    let (library, providers) = (part(false), part(true));
    SearchSplit { everything: r, library: Some(library), providers: Some(providers) }
}

/// The first of each id.
pub fn distinct<T>(list: impl IntoIterator<Item = T>, id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|x| seen.insert(id(x).to_string())).collect()
}

/// The server's answer to `query` ranked and split, the first of each repeated id kept (merged provider
/// results repeat ids).
pub fn search_split(result: SearchResult, query: &str) -> SearchSplit {
    split(ranked(
        SearchResult {
            artists: distinct(result.artists, |a| &a.id),
            albums: distinct(result.albums, |a| &a.id),
            songs: distinct(result.songs, |s| &s.id),
            playlists: distinct(result.playlists, |p| &p.id),
        },
        query,
    ))
}

/// The answer ordered around its top song, as a music app's search reads: songs whose title and artist
/// answer every word of `query` first, the most played of them first; then the top song's artists and
/// album lead the others (the album added when the answer lacks it: a search for a song's title finds
/// singles named so before the record it is on), that artist's other albums next, then albums named as
/// asked. Ties keep the source's order, which a provider such as YouTube ranks by popularity itself.
pub fn ranked(mut r: SearchResult, query: &str) -> SearchResult {
    let q = words(query);
    let answers = |text: &str| {
        let w = words(text);
        q.iter().all(|q| w.iter().any(|w| w.starts_with(q.as_str())))
    };
    r.songs.sort_by_cached_key(|s| (!answers(&format!("{} {}", s.title, s.artist)), Reverse(s.play_count)));
    let top = r.songs.first();
    let top_album = top.and_then(|s| s.album_id.clone());
    let top_artists: Vec<String> = top.map(|s| s.artists.iter().map(|a| a.id.clone()).chain(s.artist_id.clone()).filter(|id| !id.is_empty()).collect()).unwrap_or_default();
    if let Some(album) = top.and_then(album_of).filter(|a| !r.albums.iter().any(|b| b.id == a.id)) {
        r.albums.insert(0, album);
    }
    let by_top = |id: &Option<String>| id.as_ref().is_some_and(|id| top_artists.contains(id));
    r.albums.sort_by_cached_key(|a| {
        let place = if top_album.as_ref() == Some(&a.id) {
            0
        } else if by_top(&a.artist_id) {
            1
        } else if answers(&a.name) {
            2
        } else {
            3
        };
        (place, Reverse(a.play_count))
    });
    r.artists.sort_by_cached_key(|a| (!top_artists.contains(&a.id), !answers(&a.name)));
    r.playlists.sort_by_cached_key(|p| !answers(&p.name));
    r
}

/// The album `song` is on, as far as the song tells it.
fn album_of(song: &Song) -> Option<Album> {
    let id = song.album_id.clone().filter(|id| !id.is_empty() && !song.album.is_empty())?;
    Some(Album {
        id,
        name: song.album.clone(),
        artist: song.artist.clone(),
        artist_id: song.artist_id.clone(),
        cover_art: song.cover_art.clone(),
        year: song.year,
        is_external: song.is_external,
        subtitle: nori_model::lines::album_subtitle(&song.artist, song.year),
        ..Default::default()
    })
}

/// `text` lowercased as words, as the index splits it.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Which of the answer the search screen shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SearchScope {
    #[default]
    Everything,
    /// Only what is in the library already; everything when there are no provider items.
    Library,
    /// Only what the providers offer; nothing when there is none.
    Providers,
}

/// The scope chips, in their order; the client names each.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn search_scopes() -> Vec<SearchScope> {
    vec![SearchScope::Everything, SearchScope::Library, SearchScope::Providers]
}

/// The server could not be asked and the offline answer stays; `reason` as `SearchSession::failed` gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SearchFallback {
    pub reason: Option<String>,
}

impl SearchSplit {
    /// The answer narrowed to `scope`.
    fn shown(&self, scope: SearchScope) -> SearchResult {
        match scope {
            SearchScope::Everything => self.everything.clone(),
            SearchScope::Library => self.library.clone().unwrap_or_else(|| self.everything.clone()),
            SearchScope::Providers => self.providers.clone().unwrap_or_default(),
        }
    }
}

/// What the search screen shows now.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SearchView {
    /// What is in the field, as typed.
    pub text: String,
    /// The query that is asked: the text trimmed. Empty for none.
    pub query: String,
    /// The answer narrowed to the scope; None with no query (the recent searches show instead).
    pub shown: Option<SearchResult>,
    /// The answer is the server's rather than the offline index's.
    pub from_server: bool,
    /// The server is being asked.
    pub searching: bool,
    /// The server could not be asked; the offline answer stays on screen.
    pub error: Option<SearchFallback>,
    pub scope: SearchScope,
    /// The scope chips are offered: there are provider items, or a scope other than everything is on.
    pub scopes_offered: bool,
    /// The server answered with nothing at all.
    pub nothing_found: bool,
}

/// What the search field holds and what answers it has had: the state of the core's `SearchSession`.
#[derive(Default)]
pub struct Session {
    pub text: String,
    pub split: Option<SearchSplit>,
    pub from_server: bool,
    pub searching: bool,
    pub error: Option<SearchFallback>,
    pub scope: SearchScope,
}

impl Session {
    /// The query the field holds: its text without the spaces around it.
    pub fn query(&self) -> &str {
        self.text.trim()
    }

    /// What the screen shows now.
    pub fn view(&self) -> SearchView {
        let shown = self.split.as_ref().map(|s| s.shown(self.scope));
        let has_providers = self.split.as_ref().is_some_and(|s| s.providers.is_some());
        let empty = shown.as_ref().is_some_and(|r| r.songs.is_empty() && r.albums.is_empty() && r.artists.is_empty() && r.playlists.is_empty());
        SearchView {
            text: self.text.clone(),
            query: self.query().to_string(),
            nothing_found: self.from_server && empty,
            shown,
            from_server: self.from_server,
            searching: self.searching,
            error: self.error.clone(),
            scope: self.scope,
            scopes_offered: has_providers || self.scope != SearchScope::Everything,
        }
    }
}

/// Shorter than this, a query is a keystroke on the way to one, not one worth remembering.
pub const REMEMBER_MIN_UTF16: usize = 2;

/// How long live search waits after a keystroke before asking the server, ms.
pub const LIVE_SEARCH_DELAY_MS: i64 = 150;

/// [`LIVE_SEARCH_DELAY_MS`], for the platform.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn live_search_delay_ms() -> i64 {
    LIVE_SEARCH_DELAY_MS
}

/// Live search's wait after a keystroke: none for a blank field, else `delay_ms`
/// ([`LIVE_SEARCH_DELAY_MS`]). Twin of Android's `SearchViewModel` debounce.
pub fn live_delay_ms(query: &str, delay_ms: i64) -> i64 {
    if query.chars().all(kotlin_whitespace) { 0 } else { delay_ms }
}

/// Kotlin's `Char.isWhitespace`, which differs from Rust's (U+001C-U+001F count, U+0085 does not).
pub fn kotlin_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t'..='\r' | '\u{1c}'..='\u{20}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_model::{Artist, Playlist};

    fn result() -> SearchResult {
        let s = |id: &str, ext: bool| Song { id: id.into(), is_external: ext, ..Default::default() };
        let a = |id: &str, ext: bool| Album { id: id.into(), is_external: ext, ..Default::default() };
        let p = |id: &str| Playlist { id: id.into(), ..Default::default() };
        SearchResult {
            artists: vec![Artist { id: "ar".into(), ..Default::default() }],
            albums: vec![a("al", false), a("ext-al", true), a("ext-al", true)],
            songs: vec![s("1", false), s("ext-2", true), s("1", false), s("3", false)],
            playlists: vec![p("yt-VLa"), p("pl-b"), p("yt-VLa")],
        }
    }

    fn ids<T>(l: &[T], id: impl Fn(&T) -> &str) -> Vec<&str> {
        l.iter().map(id).collect()
    }

    #[test]
    fn server_answers() {
        let r = search_split(result(), "");
        assert!(r.providers.is_some());
        assert_eq!(ids(&r.everything.songs, |s| &s.id), ["1", "ext-2", "3"]);
        assert_eq!(ids(&r.everything.albums, |a| &a.id), ["al", "ext-al"]);
        assert_eq!(ids(&r.everything.playlists, |p| &p.id), ["yt-VLa", "pl-b"]);
        let (lib, prov) = (r.library.unwrap(), r.providers.unwrap());
        assert_eq!(ids(&lib.songs, |s| &s.id), ["1", "3"]);
        assert_eq!(lib.artists.len(), 1);
        assert_eq!(ids(&prov.songs, |s| &s.id), ["ext-2"]);
        assert_eq!(ids(&prov.albums, |a| &a.id), ["ext-al"]);
        assert!(prov.artists.is_empty());
        assert_eq!((ids(&lib.playlists, |p| &p.id), ids(&prov.playlists, |p| &p.id)), (vec!["yt-VLa"], vec!["pl-b"]));

        // A library only answer is not copied.
        let mut r = result();
        r.songs.retain(|s| !s.is_external);
        r.albums.retain(|a| !a.is_external);
        r.playlists.retain(|p| !is_provider_id(&p.id));
        let s = search_split(r, "");
        assert!(s.library.is_none() && s.providers.is_none());
    }

    #[test]
    fn ranked_around_the_top_song() {
        let song = |id: &str, title: &str, artist: (&str, &str), album: &str, plays: u32| Song {
            id: id.into(),
            title: title.into(),
            artist: artist.1.into(),
            artist_id: Some(artist.0.into()),
            album_id: Some(album.into()),
            play_count: plays,
            ..Default::default()
        };
        let album = |id: &str, name: &str, artist: &str, plays: u32| Album { id: id.into(), name: name.into(), artist_id: Some(artist.into()), play_count: plays, ..Default::default() };
        let artist = |id: &str, name: &str| Artist { id: id.into(), name: name.into(), ..Default::default() };
        let eminem = ("em", "Eminem");
        let r = SearchResult {
            songs: vec![
                song("by-album", "Berzerk", eminem, "mmlp2", 90),
                song("cover", "Rap God (Cover)", ("nobody", "Nobody"), "cover-single", 0),
                song("real", "Rap God", eminem, "mmlp2", 3),
            ],
            albums: vec![
                album("cover-single", "Rap God", "nobody", 0),
                album("other", "Something", "x", 50),
                album("recovery", "Recovery", "em", 0),
                album("mmlp2", "The Marshall Mathers LP2", "em", 0),
            ],
            artists: vec![artist("profile", "Rap God"), artist("x", "X"), artist("em", "Eminem")],
            ..Default::default()
        };
        let r = ranked(r, "rap god");
        assert_eq!(ids(&r.songs, |s| &s.id), ["real", "cover", "by-album"], "the most played answer leads");
        assert_eq!(ids(&r.albums, |a| &a.id), ["mmlp2", "recovery", "cover-single", "other"]);
        assert_eq!(ids(&r.artists, |a| &a.id), ["em", "profile", "x"]);

        // The top song's album leads even where the answer lacks it.
        let mut real = song("real", "Rap God", eminem, "mmlp2", 0);
        real.album = "The Marshall Mathers LP2".into();
        let r = ranked(SearchResult { songs: vec![real], albums: vec![album("cover-single", "Rap God", "nobody", 0)], ..Default::default() }, "rap god");
        assert_eq!(r.albums.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect::<Vec<_>>(), [("mmlp2", "The Marshall Mathers LP2"), ("cover-single", "Rap God")]);
        assert_eq!(r.albums[0].artist_id.as_deref(), Some("em"));

        // A query naming the artist and the song finds it by both.
        let r = ranked(SearchResult { songs: vec![song("cover", "Eminem Rap God karaoke", ("k", "Karaoke"), "k", 0), song("real", "Rap God", eminem, "mmlp2", 0)], ..Default::default() }, "eminem rap god");
        assert_eq!(ids(&r.songs, |s| &s.id), ["cover", "real"], "equally answered: the source's order");
    }
}
