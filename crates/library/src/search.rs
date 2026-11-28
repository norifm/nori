//! Search results ranked and split once per answer into everything, the library's and the providers'
//! (`isExternal`), and the search screen's state.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::hash::Hash;

use nori_model::{is_provider_id, Album, ItemKey, Keyed, SearchResult, Song};

#[derive(Debug, Clone, Default)]
pub struct SearchSplit {
    pub everything: SearchResult,
    /// Only the library's items; None when that is everything (there are no provider items).
    pub library: Option<SearchResult>,
    /// Only the providers' items; None when there are none.
    pub providers: Option<SearchResult>,
}

/// No song, album, artist or playlist.
pub fn found_nothing(r: &SearchResult) -> bool {
    r.songs.is_empty() && r.albums.is_empty() && r.artists.is_empty() && r.playlists.is_empty()
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

/// The first of each item, told apart by `key`.
pub fn distinct<T, K: Hash + Eq>(list: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> Vec<T> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|x| seen.insert(key(x))).collect()
}

/// The server's answer to `query` ranked and split, the first of each repeated id kept (merged provider
/// results repeat ids).
pub fn search_split(result: SearchResult, query: &str) -> SearchSplit {
    split(ranked(
        SearchResult {
            artists: distinct(result.artists, Keyed::key),
            albums: distinct(result.albums, Keyed::key),
            songs: distinct(result.songs, Keyed::key),
            playlists: distinct(result.playlists, Keyed::key),
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
    let top_source = top.and_then(|s| s.source.clone());
    let top_album = top.and_then(|s| Some(ItemKey { source: s.source.clone(), id: s.album_id.clone()? }));
    let top_artists: Vec<String> = top.map(|s| s.artists.iter().map(|a| a.id.clone()).chain(s.artist_id.clone()).filter(|id| !id.is_empty()).collect()).unwrap_or_default();
    if let Some(album) = top.and_then(album_of).filter(|a| !r.albums.iter().any(|b| b.key() == a.key())) {
        r.albums.insert(0, album);
    }
    let by_top = |source: &Option<String>, id: &Option<String>| *source == top_source && id.as_ref().is_some_and(|id| top_artists.contains(id));
    r.albums.sort_by_cached_key(|a| {
        let place = if top_album == Some(a.key()) {
            0
        } else if by_top(&a.source, &a.artist_id) {
            1
        } else if answers(&a.name) {
            2
        } else {
            3
        };
        (place, Reverse(a.play_count))
    });
    r.artists.sort_by_cached_key(|a| (!by_top(&a.source, &Some(a.id.clone())), !answers(&a.name)));
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
        source: song.source.clone(),
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

/// The scope chips, in their order after the kinds' (everything is none of them lit); the client names each.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn search_scopes() -> Vec<SearchScope> {
    vec![SearchScope::Library, SearchScope::Providers]
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
    pub answer: SearchShown,
    pub scope: SearchScope,
    /// The scope chips are offered: there are provider items, or a scope other than everything is on.
    pub scopes_offered: bool,
    /// The kind chips after All, in their order: the kinds the answer shown has.
    pub kinds: Vec<SearchKind>,
    /// The kind chip lit; None for All.
    pub kind: Option<SearchKind>,
    /// How the results page lays out the answer shown ([`search_layout`]); empty while there is none.
    pub layout: SearchLayout,
}

/// Where the answer on the search screen stands, narrowed to the scope.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SearchShown {
    /// No query: the recent searches show instead.
    #[default]
    Blank,
    /// The server is being asked; the offline index's answer shows meanwhile, if it has one.
    Asking { index: Option<SearchResult> },
    /// The server could not be asked; the index's answer stays.
    Failed { index: Option<SearchResult>, fallback: SearchFallback },
    /// The server's answer.
    Found { result: SearchResult },
    /// The server answered with nothing.
    NothingFound,
}

impl SearchShown {
    /// The results on screen.
    pub fn result(&self) -> Option<&SearchResult> {
        match self {
            SearchShown::Asking { index } | SearchShown::Failed { index, .. } => index.as_ref(),
            SearchShown::Found { result } => Some(result),
            SearchShown::Blank | SearchShown::NothingFound => None,
        }
    }

    /// [`SearchShown::result`], taken.
    pub fn into_result(self) -> Option<SearchResult> {
        match self {
            SearchShown::Asking { index } | SearchShown::Failed { index, .. } => index,
            SearchShown::Found { result } => Some(result),
            SearchShown::Blank | SearchShown::NothingFound => None,
        }
    }
}

/// Where the answer to the field's query stands.
#[derive(Default)]
pub enum Answer {
    /// The field is blank: the recent searches show instead.
    #[default]
    Blank,
    /// The server is being asked; the index's answer shows meanwhile (the previous query's until it has one).
    Asking(Option<SearchSplit>),
    /// The server could not be asked; the index's answer stays.
    Failed { index: Option<SearchSplit>, fallback: SearchFallback },
    /// The server's answer, which nothing else replaces for the same query.
    Server(SearchSplit),
}

impl Answer {
    fn split(&self) -> Option<&SearchSplit> {
        match self {
            Answer::Blank => None,
            Answer::Asking(split) | Answer::Failed { index: split, .. } => split.as_ref(),
            Answer::Server(split) => Some(split),
        }
    }

    /// The answer shown, taken out, leaving [`Answer::Blank`].
    pub fn take_split(&mut self) -> Option<SearchSplit> {
        match std::mem::take(self) {
            Answer::Blank => None,
            Answer::Asking(split) | Answer::Failed { index: split, .. } => split,
            Answer::Server(split) => Some(split),
        }
    }
}

/// What the search field holds and what answers it has had: the state of the core's `SearchSession`.
#[derive(Default)]
pub struct Session {
    pub text: String,
    pub answer: Answer,
    pub scope: SearchScope,
    /// The kind picked for the query; None for all of them.
    pub kind: Option<SearchKind>,
}

impl Session {
    /// The kind chip `kind` picked (None: All); the chip lit picked again is All.
    pub fn pick(&mut self, kind: Option<SearchKind>) {
        self.kind = if kind == self.kind { None } else { kind };
    }

    /// The scope chip `scope` tapped: it narrows the answer to it, or back to everything if it already did.
    pub fn narrow(&mut self, scope: SearchScope) {
        self.scope = if scope == self.scope { SearchScope::Everything } else { scope };
    }

    /// The query the field holds: its text without the spaces around it.
    pub fn query(&self) -> &str {
        self.text.trim()
    }

    /// What the screen shows now.
    pub fn view(&self) -> SearchView {
        let split = self.answer.split();
        let has_providers = split.is_some_and(|s| s.providers.is_some());
        let shown = |s: &Option<SearchSplit>| s.as_ref().map(|s| s.shown(self.scope));
        let answer = match &self.answer {
            Answer::Blank => SearchShown::Blank,
            Answer::Asking(index) => SearchShown::Asking { index: shown(index) },
            Answer::Failed { index, fallback } => SearchShown::Failed { index: shown(index), fallback: fallback.clone() },
            Answer::Server(split) => {
                let result = split.shown(self.scope);
                if found_nothing(&result) {
                    SearchShown::NothingFound
                } else {
                    SearchShown::Found { result }
                }
            }
        };
        let kinds = answer.result().map(kinds_found).unwrap_or_default();
        // A kind picked that the answer lacks (a scope narrowed it away) shows all.
        let kind = self.kind.filter(|k| kinds.contains(k));
        let layout = answer.result().map(|r| kind.map_or_else(|| search_layout(r, self.query()), |k| whole_kind(r, k))).unwrap_or_default();
        SearchView {
            text: self.text.clone(),
            query: self.query().to_string(),
            answer,
            scope: self.scope,
            scopes_offered: has_providers || self.scope != SearchScope::Everything,
            kinds,
            kind,
            layout,
        }
    }
}

/// A kind of result: a section of the results page, a group of the field's preview. The client names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum SearchKind {
    Songs,
    Artists,
    Albums,
    Playlists,
}

/// The kinds in the order of their chips and of the results page's sections.
pub const SEARCH_KINDS: [SearchKind; 4] = [SearchKind::Songs, SearchKind::Albums, SearchKind::Artists, SearchKind::Playlists];

/// How many of `kind` answer `r` has.
fn count_of(r: &SearchResult, kind: SearchKind) -> usize {
    match kind {
        SearchKind::Songs => r.songs.len(),
        SearchKind::Albums => r.albums.len(),
        SearchKind::Artists => r.artists.len(),
        SearchKind::Playlists => r.playlists.len(),
    }
}

/// The kinds `r` has, in their chips' order.
pub fn kinds_found(r: &SearchResult) -> Vec<SearchKind> {
    SEARCH_KINDS.into_iter().filter(|&k| count_of(r, k) > 0).collect()
}

/// `kind`'s results in `r` whole: the page with that chip picked.
fn whole_kind(r: &SearchResult, kind: SearchKind) -> SearchLayout {
    SearchLayout { top: None, sections: vec![SearchSection { kind, items: (0..count_of(r, kind) as u32).collect() }] }
}

/// One result by its kind and its place in that kind's list of the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SearchItem {
    pub kind: SearchKind,
    pub at: u32,
}

/// A section of the results page: its kind's items as the page shows them, by their places in the answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SearchSection {
    pub kind: SearchKind,
    pub items: Vec<u32>,
}

/// The results page of an answer. On All: the one result that answers the query best, first and largest,
/// then a section per kind in [`SEARCH_KINDS`]' order, the top result left out of its own: the best few
/// songs beside it, then a row of each other kind. With a kind picked: that kind's results whole, alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SearchLayout {
    pub top: Option<SearchItem>,
    pub sections: Vec<SearchSection>,
}

/// How many songs stand beside the top result, as Spotify's: the Songs chip shows them all.
pub const SONGS_BESIDE_TOP: usize = 4;

/// The most cards a row of All holds: its kind's chip shows them all.
pub const SHELF_MOST: usize = 10;

/// How well `name` answers the query's words `q`: 3 the same words, 2 every word of the query starts
/// one of its words, 0 neither.
fn answering(name: &str, q: &[String]) -> u8 {
    let w = words(name);
    if !q.is_empty() && w == q {
        3
    } else if !q.is_empty() && q.iter().all(|q| w.iter().any(|w| w.starts_with(q.as_str()))) {
        2
    } else {
        0
    }
}

/// The results page of `r` (ranked already, [`ranked`]) for `query`. The top result is the top song,
/// unless an artist, album or playlist answers the query better by its name than the song does by its
/// title (a song that answers only with its artist's name counts 1): then the first that answers best,
/// artists before albums before playlists. With nothing answering it is still the top song, or the first
/// of the other kinds.
pub fn search_layout(r: &SearchResult, query: &str) -> SearchLayout {
    let q = words(query);
    let best = |names: &mut dyn Iterator<Item = &str>| names.enumerate().map(|(i, n)| (answering(n, &q), i)).max_by_key(|&(score, i)| (score, Reverse(i)));
    let song = r.songs.first().map(|s| {
        let by_title = answering(&s.title, &q);
        let score = if by_title > 0 { by_title } else if answering(&format!("{} {}", s.title, s.artist), &q) > 0 { 1 } else { 0 };
        (score, SearchItem { kind: SearchKind::Songs, at: 0 })
    });
    let others = [
        (SearchKind::Artists, best(&mut r.artists.iter().map(|a| a.name.as_str()))),
        (SearchKind::Albums, best(&mut r.albums.iter().map(|a| a.name.as_str()))),
        (SearchKind::Playlists, best(&mut r.playlists.iter().map(|p| p.name.as_str()))),
    ];
    let better = others
        .iter()
        .filter_map(|&(kind, found)| found.map(|(score, at)| (score, SearchItem { kind, at: at as u32 })))
        .filter(|&(score, _)| score > song.map_or(0, |(s, _)| s))
        .fold(None::<(u8, SearchItem)>, |held, next| if held.is_some_and(|h| h.0 >= next.0) { held } else { Some(next) });
    let first_of = || others.iter().find_map(|&(kind, found)| found.map(|_| SearchItem { kind, at: 0 }));
    let top = better.or(song).map(|(_, item)| item).or_else(first_of);
    let sections = SEARCH_KINDS
        .iter()
        .filter_map(|&kind| {
            let most = if kind == SearchKind::Songs { SONGS_BESIDE_TOP } else { SHELF_MOST };
            let items: Vec<u32> = (0..count_of(r, kind) as u32).filter(|&at| top != Some(SearchItem { kind, at })).take(most).collect();
            (!items.is_empty()).then_some(SearchSection { kind, items })
        })
        .collect();
    SearchLayout { top, sections }
}

/// A line of the search field's preview, the list under the field as it is typed in: a group's head, a
/// result by its place in the answer, or the way to the whole search page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "line", rename_all = "camelCase")]
pub enum PreviewLine {
    /// The top result's head; its line follows.
    Top,
    Head { kind: SearchKind },
    Artist { at: u32 },
    Album { at: u32 },
    Playlist { at: u32 },
    Song { at: u32 },
    All,
}

/// The preview of the results page `layout`: the top result, then the first few of each section in the
/// page's order, then the way to everything; nothing when it found nothing.
pub fn search_preview(layout: &SearchLayout) -> Vec<PreviewLine> {
    let line = |item: SearchItem| match item.kind {
        SearchKind::Songs => PreviewLine::Song { at: item.at },
        SearchKind::Artists => PreviewLine::Artist { at: item.at },
        SearchKind::Albums => PreviewLine::Album { at: item.at },
        SearchKind::Playlists => PreviewLine::Playlist { at: item.at },
    };
    let mut lines = Vec::new();
    if let Some(top) = layout.top {
        lines.extend([PreviewLine::Top, line(top)]);
    }
    for s in &layout.sections {
        let most = if s.kind == SearchKind::Songs { 3 } else { 2 };
        lines.push(PreviewLine::Head { kind: s.kind });
        lines.extend(s.items.iter().take(most).map(|&at| line(SearchItem { kind: s.kind, at })));
    }
    if !lines.is_empty() {
        lines.push(PreviewLine::All);
    }
    lines
}

/// Shorter than this, a query is a keystroke on the way to one, not one worth remembering.
pub const REMEMBER_MIN_UTF16: usize = 2;

/// How many recent searches a profile keeps: the newest, few enough to sit above what an empty search browses.
pub const SEARCHES_KEPT: u32 = 6;

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

    fn named<T>(names: &[&str], make: impl Fn(String, String) -> T) -> Vec<T> {
        names.iter().enumerate().map(|(i, n)| make(format!("{i}"), n.to_string())).collect()
    }

    fn answer(songs: &[(&str, &str)], artists: &[&str], albums: &[&str], playlists: &[&str]) -> SearchResult {
        SearchResult {
            songs: songs.iter().enumerate().map(|(i, (t, a))| Song { id: format!("s{i}"), title: t.to_string(), artist: a.to_string(), ..Default::default() }).collect(),
            artists: named(artists, |id, name| Artist { id, name, ..Default::default() }),
            albums: named(albums, |id, name| Album { id, name, ..Default::default() }),
            playlists: named(playlists, |id, name| Playlist { id, name, ..Default::default() }),
        }
    }

    #[test]
    fn the_top_result_answers_the_query_best() {
        use SearchKind::*;
        let top = |r: &SearchResult, q: &str| search_layout(r, q).top.map(|t| (t.kind, t.at));
        let eminem = answer(&[("Rap God", "Eminem"), ("Lose Yourself", "Eminem")], &["Eminem", "Eminem Tribute"], &["Recovery"], &[]);
        assert_eq!(top(&eminem, "eminem"), Some((Artists, 0)), "the artist's name, where the songs answer only by their artist");
        assert_eq!(top(&eminem, "rap god"), Some((Songs, 0)), "the song's title");
        let rap = answer(&[("Rap God", "Eminem")], &["Rap God"], &["Rap God"], &[]);
        assert_eq!(top(&rap, "rap god"), Some((Songs, 0)), "a title as good as a name: the song");
        let side = answer(&[("Far Song One", "Nori")], &[], &["Near", "Far Side"], &["Far Side Mix"]);
        assert_eq!(top(&side, "far side"), Some((Albums, 1)), "the album named so, wherever it is in its list");
        let mix = answer(&[("Chill", "x")], &[], &[], &["Morning Chill", "Chill Mix"]);
        assert_eq!(top(&mix, "chill mix"), Some((Playlists, 1)));
        let fuzzy = answer(&[], &["Somebody"], &["Something"], &[]);
        assert_eq!(top(&fuzzy, "zq"), Some((Artists, 0)), "nothing answers: the first there is");
        assert_eq!(top(&SearchResult::default(), "x"), None);
    }

    #[test]
    fn all_lays_out_the_top_result_then_a_row_of_each_kind() {
        use SearchKind::*;
        let songs: Vec<(String, String)> = (0..7).map(|i| (format!("Alpha {i}"), "Alpha Waves".to_string())).collect();
        let songs: Vec<(&str, &str)> = songs.iter().map(|(t, a)| (t.as_str(), a.as_str())).collect();
        let albums: Vec<String> = (0..12).map(|i| format!("Album {i}")).collect();
        let albums: Vec<&str> = albums.iter().map(String::as_str).collect();
        let r = answer(&songs, &["Alpha Waves"], &albums, &["Alphas"]);
        let l = search_layout(&r, "alpha waves");
        assert_eq!(l.top, Some(SearchItem { kind: Artists, at: 0 }));
        let shown: Vec<(SearchKind, Vec<u32>)> = l.sections.iter().map(|s| (s.kind, s.items.clone())).collect();
        assert_eq!(
            shown,
            [(Songs, vec![0, 1, 2, 3]), (Albums, (0..SHELF_MOST as u32).collect()), (Playlists, vec![0])],
            "the best few songs, a row of albums; the artist, alone in its kind, leaves no artists' row"
        );
        // A top song is not listed again beside itself.
        let l = search_layout(&answer(&songs[..5], &[], &[], &[]), "alpha 0");
        assert_eq!(l.top, Some(SearchItem { kind: Songs, at: 0 }));
        assert_eq!(l.sections, [SearchSection { kind: Songs, items: vec![1, 2, 3, 4] }]);
    }

    #[test]
    fn a_kind_picked_shows_that_kind_whole() {
        use SearchKind::*;
        let r = answer(&[("One", "A"), ("Two", "A")], &[], &(0..14).map(|_| "Album").collect::<Vec<_>>(), &[]);
        let mut s = Session { text: "a".into(), answer: Answer::Server(search_split(r, "a")), ..Default::default() };
        let v = s.view();
        assert_eq!((v.kinds.as_slice(), v.kind), ([Songs, Albums].as_slice(), None));
        s.pick(Some(Albums));
        let v = s.view();
        assert_eq!(v.kind, Some(Albums));
        assert_eq!(v.layout, SearchLayout { top: None, sections: vec![SearchSection { kind: Albums, items: (0..14).collect() }] });
        s.pick(Some(Albums));
        assert_eq!(s.view().kind, None, "the chip lit, picked again, is All");
        s.pick(Some(Artists));
        assert_eq!((s.view().kind, s.view().layout.top.is_some()), (None, true), "a kind the answer lacks shows all");
    }

    #[test]
    fn the_preview_is_the_page_in_small() {
        let r = answer(&[("Alpha", "A"), ("Alpha 2", "A"), ("Beta", "A"), ("Gamma", "A"), ("Delta", "A")], &["Alphaville", "Alphas", "Alpha Band"], &["Alpha"], &[]);
        use PreviewLine as L;
        assert_eq!(
            search_preview(&search_layout(&r, "alpha")),
            [
                L::Top, L::Song { at: 0 },
                L::Head { kind: SearchKind::Songs }, L::Song { at: 1 }, L::Song { at: 2 }, L::Song { at: 3 },
                L::Head { kind: SearchKind::Albums }, L::Album { at: 0 },
                L::Head { kind: SearchKind::Artists }, L::Artist { at: 0 }, L::Artist { at: 1 },
                L::All,
            ]
        );
        assert!(search_preview(&search_layout(&SearchResult::default(), "x")).is_empty());
    }

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
        let r = ranked(SearchResult { songs: vec![real.clone()], albums: vec![album("cover-single", "Rap God", "nobody", 0)], ..Default::default() }, "rap god");
        assert_eq!(r.albums.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect::<Vec<_>>(), [("mmlp2", "The Marshall Mathers LP2"), ("cover-single", "Rap God")]);
        assert_eq!(r.albums[0].artist_id.as_deref(), Some("em"));

        // The top song's album and artist are those of its own source: another source's of the same id
        // are two others.
        let theirs = |mut s: Song| {
            s.source = Some("yt".into());
            s
        };
        let r = ranked(
            SearchResult {
                songs: vec![theirs(real.clone())],
                albums: vec![album("mmlp2", "Other", "x", 0)],
                artists: vec![artist("em", "Eminem"), Artist { source: Some("yt".into()), ..artist("em", "Eminem") }],
                ..Default::default()
            },
            "rap god",
        );
        assert_eq!(r.albums.iter().map(|a| (a.id.as_str(), a.source.as_deref())).collect::<Vec<_>>(), [("mmlp2", Some("yt")), ("mmlp2", None)]);
        assert_eq!(r.artists.iter().map(|a| a.source.as_deref()).collect::<Vec<_>>(), [Some("yt"), None]);
        let one = search_split(SearchResult { songs: vec![real.clone(), theirs(real.clone()), real.clone()], ..Default::default() }, "");
        assert_eq!(one.everything.songs.len(), 2, "one of each source's");

        // A query naming the artist and the song finds it by both.
        let r = ranked(SearchResult { songs: vec![song("cover", "Eminem Rap God karaoke", ("k", "Karaoke"), "k", 0), song("real", "Rap God", eminem, "mmlp2", 0)], ..Default::default() }, "eminem rap god");
        assert_eq!(ids(&r.songs, |s| &s.id), ["cover", "real"], "equally answered: the source's order");
    }
}
