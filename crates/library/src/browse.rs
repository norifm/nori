//! The browsing screens' rules: home shelves, list paging and orders, library sections, the stats page.

use nori_model::{Album, FeedShelf, HistoryEntry, ListeningStats, Playlist, Song};
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

/// The home rows a profile shows of the ones kept: without the `account`'s things (a jam guest's,
/// [`ProfileRules::account`]) the server's album shelves, not its favourites, playlists or indexed songs.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn home_rows_shown(rows: Vec<HomeRow>, account: bool) -> Vec<HomeRow> {
    rows.into_iter().filter(|r| account || matches!(shelf(*r), HomeShelf::Albums { follows_stars: false, .. })).collect()
}

/// A source's own feed (`ProfileRules::feed`) as shelves of albums, for the Home that draws only albums
/// (the terminal's): a shelf's songs stand for their albums, each once; its playlists and artists are
/// left out, and a shelf left with nothing goes.
pub fn feed_album_shelves(feed: Vec<FeedShelf>) -> Vec<(String, Vec<Album>)> {
    feed.into_iter()
        .filter_map(|f| {
            let mut albums = f.albums;
            for s in f.songs {
                let Some(id) = s.album_id.filter(|id| !albums.iter().any(|a| &a.id == id)) else { continue };
                let subtitle = nori_model::lines::album_subtitle(&s.artist, 0);
                albums.push(Album { id, name: s.album, artist: s.artist, artist_id: s.artist_id, cover_art: s.cover_art, subtitle, ..Default::default() });
            }
            (!albums.is_empty()).then_some((f.title, albums))
        })
        .collect()
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

/// The album grid's sort menu, in its order; the client names each.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn album_sorts() -> Vec<AlbumSort> {
    vec![
        AlbumSort::ByName,
        AlbumSort::ByArtist,
        AlbumSort::ArtistRows,
        AlbumSort::Newest,
        AlbumSort::Recent,
        AlbumSort::Frequent,
        AlbumSort::Starred,
        AlbumSort::ByYear,
        AlbumSort::Random,
    ]
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

/// The album grid's order as it was left (A–Z the first time), kept by [`AlbumSort::name`].
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn album_sort_saved(prefs: std::collections::HashMap<String, String>) -> AlbumSort {
    prefs.get(ALBUMS_SORT_KEY).and_then(|n| AlbumSort::of_name(n)).unwrap_or(AlbumSort::ByName)
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
    ProfileRules { asks: guest, account: !guest, downloads: !guest, radio: !guest, indexed: !guest, sections, settings, remote: true, measures: !guest, stars: !guest, stars_after_sign_in: false, scrobbles: !guest, feed: false }
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

    /// Songs stand for their albums once; playlists are left out, and a shelf of only them goes.
    #[test]
    fn a_feed_as_album_shelves() {
        let song = |id: &str, album: Option<&str>| Song { id: id.into(), album: album.unwrap_or_default().into(), album_id: album.map(String::from), artist: "A".into(), ..Default::default() };
        let feed = vec![
            FeedShelf { title: "Quick picks".into(), songs: vec![song("s1", Some("al1")), song("s2", Some("al1")), song("s3", None), song("s4", Some("al2"))], ..Default::default() },
            FeedShelf { title: "All hits".into(), playlists: vec![Playlist::default()], ..Default::default() },
            FeedShelf { title: "New releases".into(), albums: vec![Album { id: "al3".into(), ..Default::default() }], ..Default::default() },
        ];
        let shelves = feed_album_shelves(feed);
        let said: Vec<(&str, Vec<&str>)> = shelves.iter().map(|(t, a)| (t.as_str(), a.iter().map(|a| a.id.as_str()).collect())).collect();
        assert_eq!(said, [("Quick picks", vec!["al1", "al2"]), ("New releases", vec!["al3"])]);
        assert_eq!((shelves[0].1[0].name.as_str(), shelves[0].1[0].subtitle.as_str()), ("al1", "A"));
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
