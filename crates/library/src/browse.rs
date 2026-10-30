//! The browsing screens' rules: home shelves, list paging and orders, library sections, the stats page.

use nori_model::{Album, HistoryEntry, ListeningStats, Playlist, Song};
use nori_settings::settings::HomeRow;
pub use nori_settings::settings::LibrarySection;

/// A day, in milliseconds.
pub const DAY_MS: i64 = 86_400_000;

/// Albums per page of the album grid (a server request each).
const ALBUM_PAGE: u32 = 60;
/// Songs per page of the "all songs" list (the offline index).
pub const SONG_PAGE: u32 = 200;
/// Listens per page of the history.
pub const HISTORY_PAGE: u32 = 100;
/// Albums on an album shelf of the home page, and how many playlists its playlist shelf shows.
const SHELF: u32 = 20;
/// Entries per page of a home row's full view; its whole-list rows take [`WHOLE`] at once.
const FULL_VIEW_PAGE: u32 = 60;
/// More playlists than any account holds, as `take` of a whole list (it crosses to Kotlin as an `Int`).
const WHOLE: u32 = i32::MAX as u32;
/// How many entries each top list of the listening stats holds.
pub const STATS_TOP: u32 = 10;

// ---- the home page ----------------------------------------------------------

/// What one row of the home page shows and where it comes from.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum HomeShelf {
    /// `getAlbumList2` of type `sort`, `size` albums; `follows_stars`: asked again on every star, with
    /// this session's marks laid over (the favourites shelf).
    Albums { sort: AlbumSort, size: u32, follows_stars: bool },
    /// The first `take` playlists, newest first.
    Playlists { take: u32 },
    /// A page of the offline index's songs.
    Songs { sort: String, descending: bool, limit: u32 },
    /// The pinned playlists ([`home_pinned`]).
    Pinned,
}

fn shelf(row: HomeRow) -> HomeShelf {
    let albums = |sort, follows_stars| HomeShelf::Albums { sort, size: SHELF, follows_stars };
    match row {
        HomeRow::Recent => albums(AlbumSort::Recent, false),
        HomeRow::Newest => albums(AlbumSort::Newest, false),
        HomeRow::Frequent => albums(AlbumSort::Frequent, false),
        HomeRow::Random => albums(AlbumSort::Random, false),
        HomeRow::Starred => albums(AlbumSort::Starred, true),
        HomeRow::Playlists => HomeShelf::Playlists { take: SHELF },
        HomeRow::TopSongs => HomeShelf::Songs { sort: "playCount".into(), descending: true, limit: SHELF },
        HomeRow::Pinned => HomeShelf::Pinned,
    }
}

/// The shelf of each home row the user kept.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_shelves(rows: Vec<HomeRow>) -> Vec<HomeShelf> {
    rows.into_iter().map(shelf).collect()
}

/// What page `offset` entries into a home row's full view ("see all") reads, in the same shapes as the
/// shelf: `size` and `limit` are the page, and a page shorter than that is the last. A favourites page past
/// the first is read from the starred list at `offset` (the first, from the shelf's read, carries this
/// session's marks). None once the row has nothing past `offset`: whole lists (playlists, pinned) are one
/// page, and a random draw is the shelf's own (kept until a refresh), so its full view shows the same albums.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_row_page(row: HomeRow, offset: u32) -> Option<HomeShelf> {
    let first = offset == 0;
    match shelf(row) {
        random @ HomeShelf::Albums { sort: AlbumSort::Random, .. } => first.then_some(random),
        HomeShelf::Albums { sort, follows_stars, .. } => Some(HomeShelf::Albums { sort, size: FULL_VIEW_PAGE, follows_stars }),
        HomeShelf::Songs { sort, descending, .. } => Some(HomeShelf::Songs { sort, descending, limit: FULL_VIEW_PAGE }),
        HomeShelf::Playlists { .. } => first.then_some(HomeShelf::Playlists { take: WHOLE }),
        HomeShelf::Pinned => first.then_some(HomeShelf::Pinned),
    }
}

/// Whether a home row's full view has more after a page of `got` entries read for `shelf`
/// ([`home_row_page`]): a page shorter than asked for is the last, and a whole list is one page.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_row_more(shelf: HomeShelf, got: u32) -> bool {
    match shelf {
        HomeShelf::Albums { size: page, .. } | HomeShelf::Songs { limit: page, .. } => got >= page,
        HomeShelf::Playlists { .. } | HomeShelf::Pinned => false,
    }
}

/// The home rows a profile shows of the ones kept: without the `account`'s things (a jam guest's,
/// [`ProfileRules::account`]) the server's album shelves, not its favourites, playlists or indexed songs.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_rows_shown(rows: Vec<HomeRow>, account: bool) -> Vec<HomeRow> {
    rows.into_iter().filter(|r| account || matches!(shelf(*r), HomeShelf::Albums { follows_stars: false, .. })).collect()
}

/// The pinned playlists, in the order the server lists them.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_pinned(playlists: Vec<Playlist>, pins: Vec<String>) -> Vec<Playlist> {
    playlists.into_iter().filter(|p| pins.contains(&p.id)).collect()
}

/// The home rows with the one at `from` moved to `to`; unchanged when either is not a row.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_rows_moved(rows: Vec<HomeRow>, from: u32, to: u32) -> Vec<HomeRow> {
    moved(rows, from, to)
}

fn moved<T>(mut rows: Vec<T>, from: u32, to: u32) -> Vec<T> {
    let (from, to) = (from as usize, to as usize);
    if from < rows.len() && to < rows.len() {
        let row = rows.remove(from);
        rows.insert(to, row);
    }
    rows
}

/// The home rows with `row` switched on (at the end) or off.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_rows_toggled(rows: Vec<HomeRow>, row: HomeRow, on: bool) -> Vec<HomeRow> {
    toggled(rows, row, on)
}

fn toggled<T: PartialEq>(mut rows: Vec<T>, row: T, on: bool) -> Vec<T> {
    rows.retain(|r| *r != row);
    if on {
        rows.push(row);
    }
    rows
}

/// The rows that are not shown, in the order a new install shows them.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_rows_hidden(shown: Vec<HomeRow>) -> Vec<HomeRow> {
    HomeRow::every().iter().copied().filter(|r| !shown.contains(r)).collect()
}

/// The Library page's sections: those `chosen`, in their order, that the profile `offered`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn library_sections_shown(chosen: Vec<LibrarySection>, offered: Vec<LibrarySection>) -> Vec<LibrarySection> {
    chosen.into_iter().filter(|s| offered.contains(s) && LibrarySection::LISTED.contains(s)).collect()
}

/// The sections the Library page could list that are not `chosen`, in the order a new install lists them.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn library_sections_hidden(chosen: Vec<LibrarySection>) -> Vec<LibrarySection> {
    LibrarySection::LISTED.into_iter().filter(|s| !chosen.contains(s)).collect()
}

/// The chosen sections with the one at `from` moved to `to`; unchanged when either is not one.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn library_sections_moved(chosen: Vec<LibrarySection>, from: u32, to: u32) -> Vec<LibrarySection> {
    moved(chosen, from, to)
}

/// The chosen sections with `section` switched on (at the end) or off.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn library_sections_toggled(chosen: Vec<LibrarySection>, section: LibrarySection, on: bool) -> Vec<LibrarySection> {
    toggled(chosen, section, on)
}

/// The pinned playlists (kept on the device: the server cannot star one) with `id` pinned or not.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn pins_toggled(pins: Vec<String>, id: String, on: bool) -> Vec<String> {
    toggled(pins, id, on)
}

/// The cached answers a manual refresh drops, so it reaches the server.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_refresh_drops() -> Vec<String> {
    ["page:albums/", "page:playlists", "page:starred", "page:home"].map(String::from).to_vec()
}

// ---- long lists ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Paging {
    pub albums: u32,
    pub songs: u32,
    pub history: u32,
}

/// A page of the album grid that came back shorter than asked for is the last one.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn albums_exhausted(got: u32) -> bool {
    got < ALBUM_PAGE
}

/// Where the album list's next page starts, after a page from `offset` asked `asked` albums long brought
/// `got`: none once a page comes short, and a random order or the favorites (read whole) are one page.
pub fn next_album_page(sort: AlbumSort, favorites: bool, offset: u32, asked: u32, got: u32) -> Option<u32> {
    (got >= asked && sort != AlbumSort::Random && !favorites).then_some(offset + got)
}

/// Whether `page`, read after `listed`, starts with an album already listed: the server ignored the
/// offset, and reading on would bring the same page without end.
pub fn album_page_repeats(listed: &[Album], page: &[Album]) -> bool {
    page.first().is_some_and(|first| listed.iter().any(|a| a.id == first.id))
}

// ---- album orders -------------------------------------------------------------

/// The album grid's orders: those `getAlbumList2` knows, and ByArtist drawn as a row per artist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AlbumSort {
    Newest,
    Recent,
    Frequent,
    Random,
    ByName,
    ByArtist,
    /// ByArtist with each artist's albums in one row under their name ([`artist_rows`]).
    ArtistRows,
    Starred,
    ByGenre,
    ByYear,
}

impl AlbumSort {
    const ALL: [AlbumSort; 10] = [
        AlbumSort::Newest, AlbumSort::Recent, AlbumSort::Frequent, AlbumSort::Random, AlbumSort::ByName, AlbumSort::ByArtist,
        AlbumSort::ArtistRows, AlbumSort::Starred, AlbumSort::ByGenre, AlbumSort::ByYear,
    ];

    /// The `type` the server is asked for.
    pub fn api(self) -> &'static str {
        match self {
            AlbumSort::Newest => "newest",
            AlbumSort::Recent => "recent",
            AlbumSort::Frequent => "frequent",
            AlbumSort::Random => "random",
            AlbumSort::ByName => "alphabeticalByName",
            AlbumSort::ByArtist | AlbumSort::ArtistRows => "alphabeticalByArtist",
            AlbumSort::Starred => "starred",
            AlbumSort::ByGenre => "byGenre",
            AlbumSort::ByYear => "byYear",
        }
    }

    /// The name it is kept and offered under: the server's `type`, except for orders the server
    /// has no `type` of their own for.
    pub fn name(self) -> &'static str {
        match self {
            AlbumSort::ArtistRows => "artistRows",
            s => s.api(),
        }
    }

    /// The order called `name` (a stored, car folder's or client's name for it).
    pub fn of_name(name: &str) -> Option<AlbumSort> {
        AlbumSort::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The album grid's orders offered, in their order; the client names each. Those that mean the same on the
/// songs list (title, artist, year, added, most played) come in its order. Artist is
/// [`AlbumSort::ArtistRows`]: a plain A–Z by artist is no longer offered. Favorites are a filter over
/// any of these, as on the songs list.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn album_sorts() -> Vec<AlbumSort> {
    vec![
        AlbumSort::ByName,
        AlbumSort::ArtistRows,
        AlbumSort::ByYear,
        AlbumSort::Newest,
        AlbumSort::Frequent,
        AlbumSort::Random,
    ]
}

/// Which way a list runs in one of its orders, for the client to mark beside the order's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SortDirection {
    Ascending,
    Descending,
    Shuffled,
}

/// The way the album grid runs in `sort`: names from A, counts and dates from the most.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn album_sort_direction(sort: AlbumSort) -> SortDirection {
    match sort {
        AlbumSort::ByName | AlbumSort::ByArtist | AlbumSort::ArtistRows | AlbumSort::ByGenre => SortDirection::Ascending,
        AlbumSort::Newest | AlbumSort::Recent | AlbumSort::Frequent | AlbumSort::Starred | AlbumSort::ByYear => SortDirection::Descending,
        AlbumSort::Random => SortDirection::Shuffled,
    }
}

/// One entry of the list settings (`listPrefs`): which order a long list was left in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ListPref {
    pub key: String,
    pub value: String,
}

const ALBUMS_SORT_KEY: &str = "albums.sort";
const SONGS_SORT_KEY: &str = "songs.sort";
const ALBUMS_REVERSED_KEY: &str = "albums.reversed";
const SONGS_REVERSED_KEY: &str = "songs.reversed";

/// A long list whose order can be chosen and turned around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SortedList {
    Albums,
    Songs,
}

/// The order a list is in, by its place among the orders offered, and whether it runs against its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SortPick {
    pub order: u32,
    pub reversed: bool,
}

/// What a tap on order `picked` does to a list in order `now`: the order it is in turns around (when it
/// has a way to turn, `reversible`), another starts in its own way, so a first tap always gives what its
/// name says (titles from A, the newest first).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sort_picked(now: SortPick, picked: u32, reversible: bool) -> SortPick {
    if picked == now.order {
        SortPick { order: picked, reversed: now.reversed != reversible }
    } else {
        SortPick { order: picked, reversed: false }
    }
}

/// The way a list runs: its order's own (`natural`), turned around while `reversed`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sort_way(natural: SortDirection, reversed: bool) -> SortDirection {
    match (natural, reversed) {
        (SortDirection::Ascending, true) => SortDirection::Descending,
        (SortDirection::Descending, true) => SortDirection::Ascending,
        (way, _) => way,
    }
}

/// Whether `list` was left turned around (the list settings, `listPrefs`).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sort_reversed_saved(prefs: std::collections::HashMap<String, String>, list: SortedList) -> bool {
    prefs.get(reversed_key(list)).is_some_and(|v| v == "1")
}

/// The list setting that remembers whether `list` is turned around.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sort_reversed_kept(list: SortedList, reversed: bool) -> ListPref {
    ListPref { key: reversed_key(list).into(), value: if reversed { "1" } else { "0" }.into() }
}

fn reversed_key(list: SortedList) -> &'static str {
    match list {
        SortedList::Albums => ALBUMS_REVERSED_KEY,
        SortedList::Songs => SONGS_REVERSED_KEY,
    }
}

/// The album grid's order as it was left (A–Z the first time), kept by [`AlbumSort::name`]. A plain A–Z
/// by artist comes back as [`AlbumSort::ArtistRows`]; any other order no longer offered (favorites) as A–Z.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn album_sort_saved(prefs: std::collections::HashMap<String, String>) -> AlbumSort {
    match prefs.get(ALBUMS_SORT_KEY).and_then(|n| AlbumSort::of_name(n)) {
        Some(AlbumSort::ByArtist) => AlbumSort::ArtistRows,
        Some(kept) if album_sorts().contains(&kept) => kept,
        _ => AlbumSort::ByName,
    }
}

/// The list setting that remembers the album grid's order.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn album_sort_kept(sort: AlbumSort) -> ListPref {
    ListPref { key: ALBUMS_SORT_KEY.into(), value: sort.name().into() }
}

/// Where each artist's row starts among `albums` (in [`AlbumSort::ArtistRows`] order): an album
/// joins the row before it when both are by the same artist, by id where both have one, else by name.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn artist_rows(albums: Vec<Album>) -> Vec<u32> {
    let same = |a: &Album, b: &Album| match (&a.artist_id, &b.artist_id) {
        (Some(x), Some(y)) => x == y,
        _ => a.artist == b.artist,
    };
    (0..albums.len()).filter(|&i| i == 0 || !same(&albums[i - 1], &albums[i])).map(|i| i as u32).collect()
}

/// The songs list's order as it was left (by title the first time), by [`song_sorts`] name.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn song_sort_saved(prefs: std::collections::HashMap<String, String>) -> String {
    prefs.get(SONGS_SORT_KEY).filter(|n| SONG_SORTS.iter().any(|s| s.0 == n.as_str())).cloned().unwrap_or_else(|| SONG_SORTS[0].0.into())
}

/// The list setting that remembers the songs list's order.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn song_sort_kept(name: String) -> ListPref {
    ListPref { key: SONGS_SORT_KEY.into(), value: name }
}

// ---- the library ----------------------------------------------------------------

pub use nori_settings::settings_pages::SettingsPart;

/// What the app offers over a profile. A jam guest's is the normal app over the host's library, read
/// only: what it plays or queues is asked of the host, and nothing in it is the account's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProfileRules {
    /// Playing or queueing songs asks the jam's host for them: a tap or Play asks for the song that
    /// would start, Play next and Add to queue for each song.
    pub asks: bool,
    /// Subsonic account work: ratings, playlist edits, sharing, library mixes and their settings,
    /// scrobbles, now playing and queue pushes. Local history does not require an account.
    pub account: bool,
    /// Songs, albums and playlists can be kept for offline playback on this device.
    pub downloads: bool,
    /// A song can start a radio queue from the provider's related tracks.
    pub radio: bool,
    /// The library is kept in the offline index (synced); without it the songs list reads the server a
    /// page at a time.
    pub indexed: bool,
    /// The library's sections, in the order their pills run.
    pub sections: Vec<LibrarySection>,
    /// The parts of the settings it opens: a guest's are this device's own, nothing that acts on the
    /// host's account or server.
    pub settings: Vec<SettingsPart>,
    /// Remote control and jams are offered (a guest is in one).
    pub remote: bool,
    /// AutoMix measures the songs ahead here (a guest plays its host's transitions).
    pub measures: bool,
    /// Songs carry hearts, and a heart is sent (an account's, or a provider account's likes).
    pub stars: bool,
    /// Songs show a seal that asks to sign in: a provider's account, signed out, would carry them.
    pub stars_after_sign_in: bool,
    /// Songs heard are scrobbled: the account keeps plays (a server's, a provider's history).
    pub scrobbles: bool,
    /// Home is the service's own feed (`Read::HomeFeed`), not the library's shelves.
    pub feed: bool,
    /// Home has "For you": nori's mixes, from what plays here and the account's favourites.
    pub mixes: bool,
}

/// An account's.
impl Default for ProfileRules {
    fn default() -> Self {
        profile_rules(false)
    }
}

const SETTINGS_PARTS: [SettingsPart; 11] = {
    use SettingsPart::*;
    [Profiles, ServerOptions, Look, Playback, Sound, Lyrics, Library, Streaming, Downloads, Storage, About]
};

/// The rules of a jam `guest`'s profile, or of an account's.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn profile_rules(guest: bool) -> ProfileRules {
    use LibrarySection::*;
    let sections = if guest { vec![Albums, Artists, Songs, Genres] } else { vec![Albums, Favourites, Artists, Songs, Playlists, Smart, History, Genres, Decades, Folders, Radio, Downloads] };
    let account_only = [SettingsPart::ServerOptions, SettingsPart::Library, SettingsPart::Downloads];
    let settings = SETTINGS_PARTS.into_iter().filter(|p| !guest || !account_only.contains(p)).collect();
    ProfileRules { asks: guest, account: !guest, downloads: !guest, radio: !guest, indexed: !guest, sections, settings, remote: true, measures: !guest, stars: !guest, stars_after_sign_in: false, scrobbles: !guest, feed: false, mixes: !guest }
}

/// A decade's years, first and last, from its first year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct YearSpan {
    pub from: u32,
    pub to: u32,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn decade_years(start_year: u32) -> YearSpan {
    YearSpan { from: start_year, to: start_year + 9 }
}

/// Whether a new radio station can be added: it has a name and its stream is a web address.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn radio_can_add(name: String, url: String) -> bool {
    !name.trim().is_empty() && url.starts_with("http")
}

/// How many items the library's reads ask for at a time.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LibrarySizes {
    /// A server search, in one page: octo-fiesta repeats provider results on every offset.
    pub search_songs: i32,
    pub search_albums: i32,
    pub search_artists: i32,
    /// The offline index's answer to a keystroke.
    pub local_search: u32,
    /// Songs a smart playlist is evaluated to.
    pub smart_songs: u32,
    /// The server's random songs, and a genre's songs.
    pub random_songs: i32,
    pub genre_songs: i32,
    /// Songs per request while the index is synced.
    pub sync_page: u32,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn library_sizes() -> LibrarySizes {
    LibrarySizes { search_songs: 40, search_albums: 20, search_artists: 10, local_search: 30, smart_songs: 500, random_songs: 100, genre_songs: 200, sync_page: 500 }
}

/// How many of a page's `total` songs show (an artist's, search's): a few until Show all is pressed
/// (`all`), as Spotify's Popular does; a list barely past that shows whole, with no Show all.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn songs_shown(total: u32, all: bool) -> u32 {
    const FEW: u32 = 5;
    if all || total <= FEW + 2 { total } else { FEW }
}

/// Page sizes of the long lists. A page shorter than its size is the last one.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn browse_paging() -> Paging {
    Paging { albums: ALBUM_PAGE, songs: SONG_PAGE, history: HISTORY_PAGE }
}

/// An order of the "all songs" list: `name` is stored and passed back, `key` is the field sorted on.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SongSortOption {
    pub name: String,
    pub key: String,
    pub descending: bool,
}

/// Newest, longest and most played read from the top, the rest from A.
pub const SONG_SORTS: [(&str, &str, bool); 7] = [
    ("TITLE", "title", false),
    ("ARTIST", "artist", false),
    ("ALBUM", "album", false),
    ("YEAR", "year", true),
    ("ADDED", "created", true),
    ("PLAYS", "playCount", true),
    ("LONGEST", "duration", true),
];

pub fn song_sorts() -> Vec<SongSortOption> {
    SONG_SORTS.iter().map(|(name, key, descending)| SongSortOption { name: (*name).into(), key: (*key).into(), descending: *descending }).collect()
}

/// The way the songs list runs in the order kept as `name`; an order it does not know runs from A.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn song_sort_direction(name: String) -> SortDirection {
    match SONG_SORTS.iter().find(|(n, _, _)| *n == name) {
        Some((_, _, true)) => SortDirection::Descending,
        _ => SortDirection::Ascending,
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SongsPage {
    pub songs: Vec<Song>,
    /// True when this was the last page.
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    /// Where the next page starts; None after the last.
    pub next: Option<HistoryAfter>,
}

/// A place in the history, newest first: listens after it are older. Listens recorded meanwhile come
/// before it, so later pages neither repeat nor skip any.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HistoryAfter {
    pub started_ms: i64,
    pub row: i64,
}

/// Whose plays the listening stats count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum StatsSource {
    /// The plays this installation recorded, with the time heard and skips.
    Device,
    /// The server account's plays from every device (Navidrome 0.59 on, from when the server was
    /// updated): counted plays only, so no time heard and no skips.
    ServerAccount,
}

/// How the server account's play history stands after asking the server for what is new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ServerHistory {
    /// Up to date.
    Current,
    /// The server did not answer; the plays kept from earlier are counted.
    Unreached,
    /// The server keeps no play history this profile can read (not Navidrome 0.59 or later, an API key
    /// instead of a password, or a login refused).
    Unavailable,
}

/// The listening stats page.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StatsPage {
    pub source: StatsSource,
    pub stats: ListeningStats,
    /// The busiest hour, 0-23 (the first of equals); none when nothing was played.
    pub busiest_hour: Option<u32>,
    /// The busiest weekday, 0 Monday to 6 Sunday; only with a busiest hour.
    pub busiest_weekday: Option<u32>,
    /// Each hour's bar as a fraction of the busiest hour's.
    pub hours: Vec<f32>,
}

/// The first index of the largest value (Kotlin's `maxByOrNull`).
fn busiest(v: &[u32]) -> Option<usize> {
    v.iter()
        .enumerate()
        .fold(None, |best: Option<(usize, u32)>, (i, &x)| match best {
            Some((_, b)) if b >= x => best,
            _ => Some((i, x)),
        })
        .map(|(i, _)| i)
}

impl StatsPage {
    /// The page for `stats`, counted from `source`.
    pub fn new(source: StatsSource, stats: ListeningStats) -> Self {
        let busiest_hour = busiest(&stats.plays_per_hour).filter(|&h| stats.plays_per_hour[h] > 0);
        let busiest_weekday = busiest_hour.and(busiest(&stats.plays_per_weekday)).filter(|&d| d < 7);
        let peak = stats.plays_per_hour.iter().copied().max().unwrap_or(0).max(1) as f32;
        let hours = stats.plays_per_hour.iter().map(|&p| p as f32 / peak).collect();
        StatsPage { source, busiest_hour: busiest_hour.map(|h| h as u32), busiest_weekday: busiest_weekday.map(|d| d as u32), hours, stats }
    }
}

/// A decade that has songs in the index: its first year and how many songs.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Decade {
    pub start: u32,
    pub song_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_the_order_again_turns_it_around() {
        let pick = |order, reversed| SortPick { order, reversed };
        for (now, picked, reversible, want) in [
            (pick(0, false), 0, true, pick(0, true)),
            (pick(0, true), 0, true, pick(0, false)),
            (pick(0, true), 2, true, pick(2, false)),
            (pick(5, false), 5, false, pick(5, false)),
        ] {
            assert_eq!(sort_picked(now, picked, reversible), want, "{now:?} tapped {picked}");
        }
        assert_eq!(sort_way(SortDirection::Ascending, true), SortDirection::Descending);
        assert_eq!(sort_way(SortDirection::Shuffled, true), SortDirection::Shuffled);
        let kept = sort_reversed_kept(SortedList::Songs, true);
        let prefs = std::collections::HashMap::from([(kept.key, kept.value)]);
        assert!(sort_reversed_saved(prefs.clone(), SortedList::Songs) && !sort_reversed_saved(prefs, SortedList::Albums));
    }

    #[test]
    fn album_orders_run_the_way_the_songs_list_does() {
        for (album, song) in [(AlbumSort::ByName, "TITLE"), (AlbumSort::ArtistRows, "ARTIST"), (AlbumSort::ByYear, "YEAR"), (AlbumSort::Newest, "ADDED"), (AlbumSort::Frequent, "PLAYS")] {
            assert_eq!(album_sort_direction(album), song_sort_direction(song.into()), "{album:?}");
        }
    }

    #[test]
    fn album_pages_go_on_while_full() {
        let next = |sort, favorites, got| next_album_page(sort, favorites, 1000, 500, got);
        assert_eq!(
            [next(AlbumSort::Newest, false, 500), next(AlbumSort::Newest, false, 100), next(AlbumSort::Newest, false, 0), next(AlbumSort::Random, false, 500), next(AlbumSort::ByName, true, 500)],
            [Some(1500), None, None, None, None]
        );
    }

    #[test]
    fn a_page_starting_with_a_listed_album_repeats() {
        let albums = |ids: &[&str]| ids.iter().map(|id| Album { id: id.to_string(), ..Default::default() }).collect::<Vec<_>>();
        let listed = albums(&["a", "b"]);
        assert_eq!([&albums(&["a", "b"]), &albums(&["c", "a"]), &albums(&[])].map(|page| album_page_repeats(&listed, page)), [true, false, false]);
    }

    #[test]
    fn long_song_lists_show_a_few() {
        assert_eq!([songs_shown(40, false), songs_shown(40, true), songs_shown(7, false), songs_shown(8, false), songs_shown(0, false)], [5, 40, 7, 5, 0]);
    }

    #[test]
    fn a_guest_home_shows_the_hosts_album_shelves_only() {
        let every = HomeRow::every().to_vec();
        assert_eq!(home_rows_shown(every.clone(), true), every);
        assert_eq!(home_rows_shown(every, false), [HomeRow::Recent, HomeRow::Newest, HomeRow::Frequent, HomeRow::Random]);
    }

    /// A guest opens this device's settings, and the profiles to switch back with, but nothing of the
    /// host's account or server.
    #[test]
    fn a_guest_opens_only_this_devices_settings() {
        use SettingsPart::*;
        assert_eq!(profile_rules(false).settings, SETTINGS_PARTS);
        assert_eq!(profile_rules(true).settings, [Profiles, Look, Playback, Sound, Lyrics, Streaming, Storage, About]);
    }

    /// An account measures ahead and offers remote control; a guest is in a jam but plays its host's
    /// transitions. Neither's Home is a feed; an account's songs carry hearts, a guest's none.
    #[test]
    fn who_measures_and_who_is_remote() {
        let rules = |r: ProfileRules| (r.remote, r.measures, r.account, r.indexed, r.sections.is_empty(), r.stars, r.feed);
        assert_eq!(rules(profile_rules(false)), (true, true, true, true, false, true, false));
        assert_eq!(rules(profile_rules(true)), (true, false, false, false, false, false, false));
    }

    #[test]
    fn listening_page_reads_numbers() {
        use nori_model::TopEntry;
        let mut hours = vec![0u32; 24];
        hours[9] = 5;
        hours[21] = 10;
        hours[22] = 10;
        let stats = ListeningStats {
            plays_per_hour: hours,
            plays_per_weekday: vec![1, 0, 0, 0, 4, 4, 0],
            top_artists: vec![TopEntry { listened_ms: 187_900, ..Default::default() }],
            ..Default::default()
        };
        let p = StatsPage::new(StatsSource::Device, stats);
        // The first of equal peaks, as `maxByOrNull` picks.
        assert_eq!((p.busiest_hour, p.busiest_weekday), (Some(21), Some(4)));
        assert_eq!((p.hours[9], p.hours[21], p.hours[0]), (0.5, 1.0, 0.0));
        // Nothing played: no chart and nothing to say about it, and no dividing by nothing.
        let none = StatsPage::new(StatsSource::Device, ListeningStats { plays_per_hour: vec![0; 24], plays_per_weekday: vec![0; 7], ..Default::default() });
        assert_eq!((none.busiest_hour, none.busiest_weekday, none.hours[3]), (None, None, 0.0));
    }

    #[test]
    fn row_pages() {
        let albums = |sort, follows_stars| Some(HomeShelf::Albums { sort, size: 60, follows_stars });
        let songs = Some(HomeShelf::Songs { sort: "playCount".into(), descending: true, limit: 60 });
        let whole = Some(HomeShelf::Playlists { take: i32::MAX as u32 });
        // Row, offset, page.
        let cases = [
            (HomeRow::Recent, 0, albums(AlbumSort::Recent, false)),
            (HomeRow::Recent, 120, albums(AlbumSort::Recent, false)),
            (HomeRow::Starred, 0, albums(AlbumSort::Starred, true)),
            (HomeRow::Starred, 60, albums(AlbumSort::Starred, true)),
            (HomeRow::Random, 0, Some(HomeShelf::Albums { sort: AlbumSort::Random, size: 20, follows_stars: false })),
            (HomeRow::Random, 20, None),
            (HomeRow::TopSongs, 0, songs.clone()),
            (HomeRow::TopSongs, 60, songs),
            (HomeRow::Playlists, 0, whole),
            (HomeRow::Playlists, 60, None),
            (HomeRow::Pinned, 0, Some(HomeShelf::Pinned)),
            (HomeRow::Pinned, 1, None),
        ];
        for (row, offset, want) in cases {
            assert_eq!(home_row_page(row, offset), want, "{row:?} at {offset}");
        }
        // A page as long as asked for may have more after it; a shorter one, or a whole list, is the last.
        for (shelf, got, more) in [
            (albums(AlbumSort::Recent, false), 60, true),
            (albums(AlbumSort::Recent, false), 59, false),
            (Some(HomeShelf::Songs { sort: "title".into(), descending: false, limit: 60 }), 60, true),
            (Some(HomeShelf::Songs { sort: "title".into(), descending: false, limit: 60 }), 3, false),
            (Some(HomeShelf::Playlists { take: 20 }), 20, false),
            (Some(HomeShelf::Pinned), 4, false),
        ] {
            assert_eq!(home_row_more(shelf.clone().unwrap(), got), more, "{shelf:?} after {got}");
        }
        // The random row's full view is the shelf's own draw, not another.
        assert_eq!(home_row_page(HomeRow::Random, 0).as_ref(), home_shelves(vec![HomeRow::Random]).first());
    }

    #[test]
    fn shelves_and_rows() {
        let s = home_shelves(vec![HomeRow::Pinned, HomeRow::Starred, HomeRow::Recent, HomeRow::TopSongs, HomeRow::Playlists]);
        assert_eq!(s[0], HomeShelf::Pinned);
        assert_eq!(s[1], HomeShelf::Albums { sort: AlbumSort::Starred, size: 20, follows_stars: true });
        assert_eq!(s[2], HomeShelf::Albums { sort: AlbumSort::Recent, size: 20, follows_stars: false });
        assert_eq!(s[3], HomeShelf::Songs { sort: "playCount".into(), descending: true, limit: 20 });
        assert_eq!(s[4], HomeShelf::Playlists { take: 20 });
        let p = |id: &str| Playlist { id: id.into(), ..Default::default() };
        let pinned = home_pinned(vec![p("a"), p("b"), p("c")], vec!["c".into(), "a".into(), "z".into()]);
        assert_eq!(pinned.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "c"]);

        // Home rows and pins edit.
        use HomeRow::{Frequent as C, Newest as B, Recent as A};
        let rows = vec![A, B, C];
        assert_eq!(home_rows_toggled(rows.clone(), B, false), [A, C]);
        assert_eq!(home_rows_toggled(vec![A, C], B, true), [A, C, B]);
        assert_eq!(home_rows_toggled(rows.clone(), A, true), [B, C, A], "never twice");
        let shown = [HomeRow::Pinned, HomeRow::Playlists, HomeRow::Recent, HomeRow::Frequent, HomeRow::TopSongs, HomeRow::Random];
        assert_eq!(home_rows_hidden(shown.to_vec()), [HomeRow::Newest, HomeRow::Starred]);
        assert_eq!(pins_toggled(vec!["p".into()], "q".into(), true), ["p", "q"]);
        assert_eq!(pins_toggled(vec!["p".into(), "q".into()], "p".into(), false), ["q"]);
        assert_eq!(home_rows_moved(rows.clone(), 0, 2), [B, C, A]);
        assert_eq!(home_rows_moved(rows.clone(), 2, 0), [C, A, B]);
        assert_eq!(home_rows_moved(rows.clone(), 1, 3), rows, "out of range: unchanged");
    }

    #[test]
    fn library_sections_listed_and_edited() {
        use LibrarySection::*;
        let account = profile_rules(false).sections;
        assert_eq!(library_sections_shown(LibrarySection::LISTED.to_vec(), account.clone()), LibrarySection::LISTED, "a new install lists them all");
        assert_eq!(library_sections_shown(vec![Smart, Radio, History, Downloads, Albums], account), [Radio, Albums], "smart, history and downloads are not rows");
        assert_eq!(library_sections_shown(vec![Radio, Songs, Albums], profile_rules(true).sections), [Songs, Albums], "only what a guest is offered");
        assert_eq!(library_sections_hidden(vec![Albums, Songs, Radio]), [Favourites, Artists, Playlists, Genres, Decades, Folders]);
        assert_eq!(library_sections_toggled(vec![Albums, Songs], Radio, true), [Albums, Songs, Radio]);
        assert_eq!(library_sections_toggled(vec![Albums, Songs], Albums, false), [Songs]);
        assert_eq!(library_sections_moved(vec![Albums, Songs, Radio], 2, 0), [Radio, Albums, Songs]);
    }

    #[test]
    fn album_orders_keep_server_names() {
        use std::collections::HashMap;
        assert_eq!(album_sort_saved(HashMap::new()), AlbumSort::ByName);
        let kept = album_sort_kept(AlbumSort::Frequent);
        assert_eq!((kept.key.as_str(), kept.value.as_str()), ("albums.sort", "frequent"));
        assert_eq!(album_sort_saved(HashMap::from([(kept.key, kept.value)])), AlbumSort::Frequent);
        assert_eq!(album_sort_saved(HashMap::from([("albums.sort".to_string(), "byYear".to_string())])), AlbumSort::ByYear);
        let rows = album_sort_kept(AlbumSort::ArtistRows);
        assert_eq!(album_sort_saved(HashMap::from([(rows.key, rows.value)])), AlbumSort::ArtistRows, "kept apart from ByArtist");
        let plain = album_sort_kept(AlbumSort::ByArtist);
        assert_eq!(album_sort_saved(HashMap::from([(plain.key, plain.value)])), AlbumSort::ArtistRows, "a plain A-Z by artist is shown as shelves");
        let favorites = album_sort_kept(AlbumSort::Starred);
        assert_eq!(album_sort_saved(HashMap::from([(favorites.key, favorites.value)])), AlbumSort::ByName, "favorites are a filter now");
        assert_eq!(AlbumSort::ArtistRows.api(), "alphabeticalByArtist");
        assert_eq!(song_sort_saved(HashMap::new()), "TITLE");
        assert_eq!(song_sort_saved(HashMap::from([("songs.sort".to_string(), "PLAYS".to_string())])), "PLAYS");
        assert_eq!(song_sort_saved(HashMap::from([("songs.sort".to_string(), "GONE".to_string())])), "TITLE");
        assert_eq!(song_sort_kept("YEAR".into()), ListPref { key: "songs.sort".into(), value: "YEAR".into() });
    }

    #[test]
    fn artist_rows_start_where_the_artist_changes() {
        let album = |artist: &str, id: Option<&str>| Album { artist: artist.into(), artist_id: id.map(String::from), ..Default::default() };
        let albums = vec![
            album("A", Some("1")),
            album("A", Some("1")),
            album("A", Some("2")),
            album("B", None),
            album("B", None),
            album("B", Some("3")),
            album("C", None),
        ];
        assert_eq!(artist_rows(albums), [0, 2, 3, 6], "same name, another id: another row; one id missing: by name");
        assert!(artist_rows(Vec::new()).is_empty());
    }

    #[test]
    fn decades_and_radio_urls() {
        assert_eq!(decade_years(1990), YearSpan { from: 1990, to: 1999 });
        assert!(radio_can_add("FIP".into(), "https://x".into()));
        assert!(!radio_can_add(" ".into(), "https://x".into()));
        assert!(!radio_can_add("FIP".into(), "ftp://x".into()));
    }
}
