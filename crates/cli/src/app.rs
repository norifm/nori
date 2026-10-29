//! UI state and input handling, free of I/O: [`Msg`]s in, [`Cmd`]s out for the runner, so it runs in
//! tests without a server, sound card or terminal. The window has tabs along the top, a page and a player
//! bar; the full player replaces the page with its song and a panel, and [`Focus`] says which has the keys.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nori_core::browse::{AlbumSort, HistoryAfter, LibrarySection, ProfileRules};
use nori_core::library::QueueOffer;
use nori_core::menus::{DownloadAct, PageAction, PageKind, SongDownload};
use nori_core::mixes::board::{MixSheet, MixTile};
use nori_core::model::HistoryEntry;
use nori_core::settings::HomeRow;
use nori_core::client::Starrable;
use nori_core::playlist::PlaylistView;
use nori_core::remote::wire::Pending;
use nori_core::remote::{Controls, JamControls, JamView, Listening, Reach, RemoteDevice};
use nori_core::search::SearchView;
use nori_core::settings::{EqLevel, SavedServer, SoundBand, StoredPrefs, TapAction};
use nori_core::settings_store::SoundTool;
use nori_core::stars::StarMarks;
use nori_core::{Album, AlbumDetail, Artist, ArtistDetail, OriginKind, PageOrigin, Playlist, PlaylistDetail, Song};
use nori_engine::{Event, State};
use nori_core::rules::sound_tuning;
use nori_look::cover::CoverColours;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::art::{Colors, Theme};
use crate::backend::{Data, Downloads, Fetch, Msg, Req, ALBUM_PAGE};
use crate::keys::{action, Action, Scope};
use crate::lyrics::SongLyrics;
use crate::settings_view::{Chore, Opened, SettingsView, Switch};

/// The page's root view, under any opened album, artist or playlist pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Home,
    Search,
    Albums,
    Artists,
    Songs,
    Playlists,
    Favorites,
    Downloads,
    History,
    Equalizer,
    Settings,
    /// Full-window login form.
    Login,
}

/// Number key targets, in order: the tabs, then the rest.
pub const GO: [View; 9] = [View::Home, View::Albums, View::Artists, View::Songs, View::Playlists, View::Favorites, View::Downloads, View::Equalizer, View::Settings];

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::Search => "Search",
            View::Home | View::Login => "Home",
            View::Albums => "Albums",
            View::Artists => "Artists",
            View::Songs => "Songs",
            View::Playlists => "Playlists",
            View::Favorites => "Favorites",
            View::History => "History",
            View::Downloads => "Downloads",
            View::Equalizer => "Equalizer",
            View::Settings => "Settings",
        }
    }
}

/// The part of the window that has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Main,
    /// The full player's panel.
    Panel,
}

/// The full player's panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Panel {
    /// The song alone, in the middle.
    Playing,
    #[default]
    Lyrics,
    Queue,
    /// Where the music plays: this computer or another device (remote control), and the jam.
    Devices,
}

pub const PANELS: [(Panel, &str); 4] = [(Panel::Playing, "Playing"), (Panel::Queue, "Queue"), (Panel::Lyrics, "Lyrics"), (Panel::Devices, "Devices")];

/// Work for the runner.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Load(Req),
    /// `from`: the page the songs are the list of (`playlist_set`'s origin).
    Play { songs: Vec<Song>, start: usize, shuffle: bool, from: Option<PageOrigin> },
    PlayFetch(Fetch, bool),
    Enqueue(Vec<Song>, bool),
    EnqueueFetch(Fetch, bool),
    Toggle,
    Next,
    Previous,
    Seek(i64),
    Volume(f32),
    Jump(usize),
    Remove(usize),
    /// Undo the last removal (`playlist_restore`).
    Restore(String),
    Move(usize, usize),
    Shuffle(bool),
    /// The next repeat mode after the one shown.
    CycleRepeat,
    Download(Vec<Song>),
    /// Keeps the song out of the mixes.
    ExcludeFromMixes(String),
    /// Adds songs to the playlist `id`, or makes one called `name` of them.
    AddToPlaylist { id: String, songs: Vec<String> },
    NewPlaylist { name: String, songs: Vec<String> },
    /// Reads which of these songs are downloaded.
    ReadDownloads(Vec<String>),
    /// Draws the mix again.
    RedrawMix(String),
    /// Keeps a mix as a playlist, its name made from `title` by the core.
    SaveMix { title: String, songs: Vec<Song> },
    /// Asks for the queue another device left, for a player `playing` or not and showing the song `shown`.
    LookForOffer { playing: bool, shown: Option<String> },
    /// Plays the queue offered where it was left, and does not offer it again.
    TakeOffer(QueueOffer),
    DismissOffer(QueueOffer),
    /// Keeps an order of a list for the next time.
    Keep(nori_core::browse::ListPref),
    /// Plays the library shuffled: random albums whole, or every song.
    ShuffleLibrary { albums: bool },
    DownloadFetch(Fetch),
    DownloadRemove(String),
    Star(Starrable, String, bool),
    Setting(String, String),
    Level(EqLevel, f32),
    Band(u32, SoundBand),
    /// A graphic equalizer band's gain in dB.
    Graphic(u32, f32),
    Sound(SoundToolCmd),
    Action(Chore),
    /// The answer to the last AutoEQ note (`Session::curve_answer`).
    Curve,
    Mouse(bool),
    Images(bool),
    Paint(bool),
    /// Output device for the next start; empty for the system default.
    Device(String),
    Tuning(bool),
    SearchTyped(String),
    SearchServer(String),
    Lyrics(String),
    /// The playing song cover by id, and the colors the player wears from it.
    Cover(String),
    Login(Box<SavedServer>),
    SwitchServer(String),
    /// Moves the music to this device, or here (None).
    Pick(Option<String>),
    JamStart,
    JamEnd,
    /// Accepts (true) or refuses a jam request.
    JamDecide(u64, bool),
    /// Joins the jam this invite link is to.
    JoinJam(String),
    /// The account sheet, as the session has the account.
    OfferSignIn,
    /// The provider's sign-in page in a window, or the pasted cookies.
    SignInWindow,
    SignIn(String),
    SignOut,
    /// No reminder at start until the account is signed in, and its session ends, again.
    StopReminding,
    Pair(String),
    ResetPairing,
    LeaveJam,
    /// A jam guest listens along (plays the host's music here, in step), or only watches.
    Listen(bool),
    JamAlong(bool),
    JamRemoteSongs(bool),
    Plugin(PluginCmd),
    Quit,
}

/// What the runner does with the plugins (Settings › Plugins); each reads them again after.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginCmd {
    Read,
    /// Shows the `.noriplugin` or plugin folder at this path before it is installed.
    Install(String),
    /// Installs what the sheet showed.
    InstallPreview(Box<nori_plugins::PluginPreview>),
    Turn(String, bool),
    /// Moves the plugin to this place in the order.
    Place(String, u32),
    Uninstall(String),
    /// A row of a plugin, its new value.
    Set(String, String, nori_plugins::Value),
    /// Runs a plugin's action row.
    Act(String, String),
    /// Asks the repos what they offer.
    Look,
    /// Shows a version (newest first) of the offer at this place in those last read, before it is
    /// installed.
    Get(u32, u32),
    /// Runs a plugin's menu action on these items.
    Menu(nori_plugins::MenuEntry, nori_plugins::Items),
    /// Asks the plugins for this song's info lines and the player's status.
    Ask(Box<Song>),
    /// Keeps a plugin at its version, or lets it update.
    Hold(String, bool),
    AddRepo(String),
    RemoveRepo(String),
}

impl Cmd {
    /// A short description for the debug log, without passwords.
    pub fn brief(&self) -> String {
        match self {
            Cmd::Load(r) => format!("load {r:?}"),
            Cmd::Play { songs, start, shuffle, .. } => format!("play {} songs from {start} shuffle={shuffle}", songs.len()),
            Cmd::Enqueue(songs, next) => format!("enqueue {} songs next={next}", songs.len()),
            Cmd::Download(songs) => format!("download {} songs", songs.len()),
            Cmd::Login(p) => format!("log in to {}", p.url),
            Cmd::Cover(art) => format!("cover {art}"),
            Cmd::Lyrics(id) => format!("lyrics {id}"),
            Cmd::Toggle => "toggle".into(),
            Cmd::Next => "next".into(),
            Cmd::Previous => "previous".into(),
            Cmd::Seek(ms) => format!("seek {ms}"),
            Cmd::Volume(v) => format!("volume {v}"),
            Cmd::Jump(i) => format!("jump {i}"),
            Cmd::Shuffle(on) => format!("shuffle {on}"),
            Cmd::CycleRepeat => "repeat".into(),
            Cmd::Setting(k, v) => format!("setting {k}={v}"),
            Cmd::Action(a) => format!("action {a:?}"),
            Cmd::Curve => "curve".into(),
            Cmd::Tuning(on) => format!("tuning {on}"),
            Cmd::Mouse(on) => format!("mouse {on}"),
            Cmd::Images(on) => format!("images {on}"),
            Cmd::Device(d) => format!("device {d}"),
            Cmd::Pick(d) => format!("play on {d:?}"),
            Cmd::JamStart => "start a jam".into(),
            Cmd::JamEnd => "end the jam".into(),
            Cmd::JamDecide(r, yes) => format!("jam request {r} accepted={yes}"),
            Cmd::JoinJam(_) => "join a jam".into(),
            Cmd::OfferSignIn => "offer to sign in".into(),
            Cmd::SignInWindow => "sign-in window".into(),
            Cmd::SignIn(_) => "sign in with pasted cookies".into(),
            Cmd::SignOut => "sign out".into(),
            Cmd::StopReminding => "stop the sign-in reminder".into(),
            Cmd::Pair(_) => "pair devices".into(),
            Cmd::ResetPairing => "forget paired devices".into(),
            Cmd::LeaveJam => "leave the jam".into(),
            Cmd::JamAlong(on) => format!("room listening along {on}"),
            Cmd::JamRemoteSongs(on) => format!("room remote songs {on}"),
            Cmd::Listen(on) => format!("listen along {on}"),
            Cmd::Quit => "quit".into(),
            Cmd::Plugin(PluginCmd::Set(id, row, _)) => format!("plugin {id} row {row}"),
            Cmd::Plugin(c) => format!("plugin {c:?}"),
            _ => "an edit".into(),
        }
    }
}

/// `SoundTool` as a comparable command.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundToolCmd {
    Preset(usize),
    AutoPreamp(bool),
    /// At a frequency and gain.
    AddBand(f32, f32),
    RemoveBand(u32),
    ResetBands,
}

impl SoundToolCmd {
    pub fn tool(&self) -> Option<SoundTool> {
        Some(match self {
            SoundToolCmd::Preset(i) => SoundTool::Preset { preset: nori_core::dsp::eq_presets().get(*i)?.clone() },
            SoundToolCmd::AutoPreamp(a) => SoundTool::AutoPreamp { automatic: *a },
            SoundToolCmd::AddBand(freq, gain_db) => SoundTool::AddBand { freq: *freq, gain_db: *gain_db },
            SoundToolCmd::RemoveBand(i) => SoundTool::RemoveBand { index: *i },
            SoundToolCmd::ResetBands => SoundTool::ResetBands,
        })
    }
}

/// A list's selection and scroll.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Sel {
    pub at: usize,
    pub top: usize,
}

impl Sel {
    pub fn by(&mut self, d: isize, len: usize) {
        if len == 0 {
            self.at = 0;
            return;
        }
        self.at = (self.at as isize + d).clamp(0, len as isize - 1) as usize;
    }

    pub fn to(&mut self, i: usize, len: usize) {
        self.at = i.min(len.saturating_sub(1));
    }

    /// Scrolls so the selection shows in `height` rows.
    pub fn fit(&mut self, height: usize, len: usize) {
        if height == 0 {
            return;
        }
        self.at = self.at.min(len.saturating_sub(1));
        if self.at < self.top {
            self.top = self.at;
        } else if self.at >= self.top + height {
            self.top = self.at + 1 - height;
        }
        self.top = self.top.min(len.saturating_sub(height.min(len)));
    }
}

/// Loading state of fetched data.
#[derive(Debug, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }
}

/// A page opened over the view.
pub enum Page {
    Album { id: String, detail: Load<Box<AlbumDetail>>, sel: Sel },
    DownloadedAlbum { id: String, detail: Load<Box<AlbumDetail>>, sel: Sel },
    Artist { id: String, detail: Load<Box<ArtistDetail>>, sel: Sel },
    Playlist { id: String, detail: Load<Box<PlaylistDetail>>, sel: Sel },
    /// One of the core's mixes, or the favorites.
    Mix { id: String, detail: Load<Box<MixSheet>>, sel: Sel },
    /// All of a Home shelf's entries: a library row's read a page at a time (`more` while there may be more),
    /// a feed's or For you's as Home has them.
    Row { title: String, items: Vec<Item>, row: Option<HomeRow>, more: bool, sel: Sel },
}

impl Page {
    /// What the page reads; None for one made of what is already here.
    pub fn req(&self) -> Option<Req> {
        Some(match self {
            Page::Album { id, .. } => Req::Album(id.clone()),
            Page::DownloadedAlbum { id, .. } => Req::DownloadedAlbum(id.clone()),
            Page::Artist { id, .. } => Req::Artist(id.clone()),
            Page::Playlist { id, .. } => Req::Playlist(id.clone()),
            Page::Mix { id, .. } => Req::Mix(id.clone()),
            Page::Row { row, .. } => Req::Row { row: (*row)?, offset: 0 },
        })
    }

    fn sel(&mut self) -> &mut Sel {
        match self {
            Page::Album { sel, .. } | Page::DownloadedAlbum { sel, .. } | Page::Artist { sel, .. } | Page::Playlist { sel, .. } | Page::Mix { sel, .. } | Page::Row { sel, .. } => sel,
        }
    }

    fn len(&self) -> usize {
        match self {
            Page::Album { detail, .. } | Page::DownloadedAlbum { detail, .. } => detail.ready().map_or(0, |d| d.songs.len()),
            Page::Artist { detail, .. } => detail.ready().map_or(0, |d| if d.albums.is_empty() { d.songs.len() } else { artist_lines(d).len() }),
            Page::Playlist { detail, .. } => detail.ready().map_or(0, |d| d.songs.len()),
            Page::Mix { detail, .. } => detail.ready().map_or(0, |d| d.songs.len()),
            Page::Row { items, .. } => items.len(),
        }
    }

    /// The page's songs: an album's, a playlist's, a mix's, and an artist's that lists no albums (a provider's
    /// songs); an artist with albums lists those.
    pub fn songs(&self) -> Option<&[Song]> {
        match self {
            Page::Album { detail, .. } | Page::DownloadedAlbum { detail, .. } => detail.ready().map(|d| d.songs.as_slice()),
            Page::Playlist { detail, .. } => detail.ready().map(|d| d.songs.as_slice()),
            Page::Mix { detail, .. } => detail.ready().map(|d| d.songs.as_slice()),
            Page::Artist { detail, .. } => detail.ready().filter(|d| d.albums.is_empty()).map(|d| d.songs.as_slice()),
            Page::Row { .. } => None,
        }
    }
}

/// A line of an artist's page that lists albums: a shelf's name (index into its release groups), or an
/// album under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistLine {
    Shelf(usize),
    Album(usize),
}

/// An artist's albums under their shelves (`pages::release_groups`); none for an artist listing songs.
pub fn artist_lines(d: &ArtistDetail) -> Vec<ArtistLine> {
    if d.albums.is_empty() {
        return Vec::new();
    }
    d.groups.iter().enumerate().flat_map(|(k, g)| std::iter::once(ArtistLine::Shelf(k)).chain(g.albums.iter().map(|i| ArtistLine::Album(*i as usize)))).collect()
}

/// The selected thing that item actions (open, enqueue, star...) apply to.
#[derive(Debug, Clone)]
pub enum Item {
    Song(Vec<Song>, usize),
    Album(Album),
    DownloadedAlbum(Album),
    Artist(Artist),
    Playlist(Playlist),
    Mix(MixTile),
}

/// One of Home's shelves: its title, its entries, and the library's row it is (none for a source's own).
#[derive(Debug, Clone)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Item>,
    pub row: Option<HomeRow>,
}

/// The home page: the queue another device left, For you, then its shelves, each a heading and its first
/// entries.
#[derive(Default)]
pub struct Home {
    /// Indexed like the session's Home rows, or a source's feed; None until loaded.
    pub rows: Vec<Option<Shelf>>,
    /// For you: the favorites, then the mixes.
    pub mixes: Vec<MixTile>,
    /// The queue left on the server, offered to carry on here (`Client::queue_offer`).
    pub offer: Option<QueueOffer>,
    pub error: Option<String>,
    pub asked: bool,
    /// Selection over [`Home::lines`].
    pub sel: Sel,
}

/// How many of a shelf's entries Home lists; its heading opens all of them.
pub const HOME_ENTRIES: usize = 5;

/// Which of Home's shelves: For you, or a row by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShelfAt {
    ForYou,
    Row(usize),
}

/// A line of Home: the queue offered, a shelf's heading, one of its entries, or the gap before the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeLine {
    Offer,
    Shelf(ShelfAt),
    Entry(ShelfAt, usize),
    Gap,
}

impl Home {
    /// A shelf's title and entries; For you's are the mixes.
    pub fn shelf(&self, at: ShelfAt) -> Option<(String, Vec<Item>)> {
        match at {
            ShelfAt::ForYou => Some((crate::text::FOR_YOU.to_string(), self.mixes.iter().cloned().map(Item::Mix).collect())),
            ShelfAt::Row(i) => self.rows.get(i)?.as_ref().map(|s| (s.title.clone(), s.items.clone())),
        }
    }

    /// How many entries a shelf has.
    pub fn count(&self, at: ShelfAt) -> usize {
        match at {
            ShelfAt::ForYou => self.mixes.len(),
            ShelfAt::Row(i) => self.rows.get(i).and_then(|r| r.as_ref()).map_or(0, |s| s.items.len()),
        }
    }

    pub fn lines(&self) -> Vec<HomeLine> {
        let mut out = Vec::new();
        if self.offer.is_some() {
            out.push(HomeLine::Offer);
        }
        let shelves = std::iter::once(ShelfAt::ForYou).chain((0..self.rows.len()).map(ShelfAt::Row));
        for at in shelves.filter(|at| self.count(*at) > 0) {
            if !out.is_empty() {
                out.push(HomeLine::Gap);
            }
            out.push(HomeLine::Shelf(at));
            out.extend((0..self.count(at).min(HOME_ENTRIES)).map(|n| HomeLine::Entry(at, n)));
        }
        out
    }

    /// The selected line.
    pub fn line(&self) -> Option<HomeLine> {
        self.lines().get(self.sel.at).copied()
    }

    /// The entry selected.
    pub fn item(&self) -> Option<Item> {
        match self.line()? {
            HomeLine::Entry(ShelfAt::ForYou, n) => self.mixes.get(n).cloned().map(Item::Mix),
            HomeLine::Entry(ShelfAt::Row(i), n) => self.rows.get(i)?.as_ref()?.items.get(n).cloned(),
            _ => None,
        }
    }

    /// Moves the selection off a gap in direction `down`.
    fn settle(&mut self, down: bool) {
        let lines = self.lines();
        if lines.get(self.sel.at) == Some(&HomeLine::Gap) {
            self.sel.at = if down && self.sel.at + 1 < lines.len() { self.sel.at + 1 } else { self.sel.at.saturating_sub(1) };
        }
    }

    /// The offered song, playing here or the player showing it, takes the offer away (`offer_stands`).
    fn offer_still(&mut self, here: Option<String>, playing: bool) {
        let gone = self.offer.as_ref().is_some_and(|o| !o.songs.get(o.index as usize).is_some_and(|s| nori_core::library::offer_stands(&s.id, here.clone(), playing)));
        if gone {
            self.offer = None;
        }
    }
}

pub struct Library {
    pub albums: Load<Vec<Album>>,
    pub albums_more: bool,
    pub albums_sel: Sel,
    /// The albums' order, whether only the favorites are listed, and in Artist order where each artist's
    /// shelf starts (`browse::artist_rows`).
    pub album_sort: AlbumSort,
    pub albums_favorites: bool,
    pub album_shelves: Vec<u32>,
    pub artists: Load<Vec<Artist>>,
    pub artists_sel: Sel,
    pub playlists: Load<Vec<Playlist>>,
    pub songs: Load<Vec<Song>>,
    pub songs_more: bool,
    pub songs_sel: Sel,
    /// The songs' order, one of `browse::song_sorts`, and whether only the favorites are listed.
    pub song_sort: String,
    pub songs_favorites: bool,
    pub playlists_sel: Sel,
    /// The starred songs.
    pub favorites: Load<Vec<Song>>,
    pub favorites_sel: Sel,
    /// What this device played, newest first, and where the next page starts.
    pub history: Load<Vec<HistoryEntry>>,
    pub history_next: Option<HistoryAfter>,
    pub history_sel: Sel,
    /// What the downloads weigh, once weighed.
    pub downloads_bytes: Option<i64>,
}

impl Default for Library {
    fn default() -> Self {
        Library {
            albums: Load::Idle,
            albums_more: false,
            albums_sel: Sel::default(),
            album_sort: nori_core::browse::album_sort_saved(Default::default()),
            albums_favorites: false,
            album_shelves: Vec::new(),
            artists: Load::Idle,
            artists_sel: Sel::default(),
            playlists: Load::Idle,
            songs: Load::Idle,
            songs_more: false,
            songs_sel: Sel::default(),
            song_sort: nori_core::browse::song_sort_saved(Default::default()),
            songs_favorites: false,
            playlists_sel: Sel::default(),
            favorites: Load::Idle,
            favorites_sel: Sel::default(),
            history: Load::Idle,
            history_next: None,
            history_sel: Sel::default(),
            downloads_bytes: None,
        }
    }
}

/// A place the library keeps, listed at the top of each of its sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Favorites,
    Downloads,
    History,
    /// The shown list shuffled: random albums, or every song.
    Shuffle,
}

/// A line of a library section: a place, the gap after them, an artist's shelf in Artist order (by the
/// index of its first album), or an entry of the section by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibLine {
    Place(Place),
    Gap,
    Shelf(usize),
    Entry(usize),
}

#[derive(Default)]
pub struct Search {
    pub text: String,
    pub editing: bool,
    pub view: Option<SearchView>,
    /// Selection over `rows()`, titles included.
    pub sel: Sel,
    /// When to query the server, once typing pauses.
    pub ask_at: Option<Instant>,
}

/// A search result row: a section title (with its count) or an item.
pub enum SearchRow<'a> {
    Title(&'static str, usize),
    Song(&'a Song),
    Album(&'a Album),
    Artist(&'a Artist),
    Playlist(&'a Playlist),
}

impl Search {
    /// The results as one list: songs, albums, artists, playlists, each under a title.
    pub fn rows(&self) -> Vec<SearchRow<'_>> {
        let mut out = Vec::new();
        let Some(r) = self.view.as_ref().and_then(|v| v.shown.as_ref()) else { return out };
        if !r.songs.is_empty() {
            out.push(SearchRow::Title("Songs", r.songs.len()));
            out.extend(r.songs.iter().map(SearchRow::Song));
        }
        if !r.albums.is_empty() {
            out.push(SearchRow::Title("Albums", r.albums.len()));
            out.extend(r.albums.iter().map(SearchRow::Album));
        }
        if !r.artists.is_empty() {
            out.push(SearchRow::Title("Artists", r.artists.len()));
            out.extend(r.artists.iter().map(SearchRow::Artist));
        }
        if !r.playlists.is_empty() {
            out.push(SearchRow::Title("Playlists", r.playlists.len()));
            out.extend(r.playlists.iter().map(SearchRow::Playlist));
        }
        out
    }

    /// Moves the selection off a title in direction `down`.
    fn settle(&mut self, down: bool) {
        let rows = self.rows();
        let title = |i: usize| matches!(rows.get(i), Some(SearchRow::Title(..)));
        let mut at = self.sel.at;
        if title(at) {
            at = if (down || at == 0) && at + 1 < rows.len() { at + 1 } else { at.saturating_sub(1) };
        }
        self.sel.at = at;
    }
}

/// The login form.
#[derive(Default)]
pub struct Login {
    /// Name, address, user, password.
    pub fields: [String; 4],
    pub focus: usize,
    pub error: Option<String>,
    pub busy: bool,
    /// Selection in the saved servers list.
    pub sel: Sel,
    /// Whether the saved servers list has the keys.
    pub on_list: bool,
    /// The kind of profile added, of those offered ([`App::login_kinds`]).
    pub kind: usize,
}

pub const LOGIN_FIELDS: [&str; 4] = ["Name (optional)", "Server address", "User", "Password"];
/// What each of `LOGIN_FIELDS` fills in.
const LOGIN_ASKS: [nori_core::providers::FormField; 4] = {
    use nori_core::providers::FormField::*;
    [Name, Address, User, Password]
};

/// Playback state for drawing; position extrapolated from `at`.
#[derive(Debug, Clone, Copy)]
pub struct Now {
    pub state: State,
    pub position_ms: i64,
    pub at: Instant,
    pub speed: f32,
    pub mixing: bool,
    pub buffering: bool,
}

impl Default for Now {
    fn default() -> Self {
        Now { state: State::Idle, position_ms: 0, at: Instant::now(), speed: 1.0, mixing: false, buffering: false }
    }
}

impl Now {
    pub fn position(&self, now: Instant) -> i64 {
        match self.state {
            State::Playing if !self.buffering => self.position_ms + (now.saturating_duration_since(self.at).as_secs_f64() * 1000.0 * self.speed as f64) as i64,
            _ => self.position_ms,
        }
    }
}

/// Remote control and jams as the runner last read them from the session; empty while both are off.
#[derive(Default)]
pub struct Devices {
    /// Remote control or jams are switched on.
    pub on: bool,
    /// A jam can be started: jams are on and the server has not said it cannot relay.
    pub jams: bool,
    /// Jams are on but the server cannot relay them.
    pub jams_unsupported: bool,
    pub pairing: Option<String>,
    /// The account's other devices.
    pub list: Vec<RemoteDevice>,
    pub sel: Sel,
    /// The device playing while it is another one: its id and name.
    pub active: Option<(String, String)>,
    /// Hearts as the device playing shows them, by song id, while it is another one.
    pub hearts: HashMap<String, bool>,
    /// The jam this computer hosts, or is a guest in.
    pub jam: Option<JamView>,
    /// Who asked for each song that came in through the jam, by song id.
    pub added: HashMap<String, String>,
    /// A jam guest's controls by its role: what its keys reach, and what the player says.
    pub controls: Option<JamControls>,
    /// The plugins whose rows show here too.
    pub plugins: Vec<nori_plugins::PluginCard>,
}

/// The sheet before installing what `p` shows: what it is, what it may reach, where it comes from.
fn preview_sheet(p: nori_plugins::PluginPreview) -> Overlay {
    let c = &p.card;
    let mut lines: Vec<String> = c.about.iter().cloned().collect();
    lines.extend([crate::text::plugin_is(&c.points, !c.overrides.is_empty()), crate::text::plugin_reach(&c.net, &c.overrides)].into_iter().filter(|l| !l.is_empty()));
    lines.extend(p.repo.as_deref().map(crate::text::from_repo));
    lines.push(crate::text::INSTALLED_OFF.into());
    Overlay::Confirm { title: crate::text::install_version(&c.name, &c.version), lines, yes: crate::text::INSTALL.into(), cmd: Box::new(Cmd::Plugin(PluginCmd::InstallPreview(Box::new(p)))) }
}

/// What the enabled plugins add to the screens, as last read.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Slots {
    /// Some plugin fills a place: the song's info and the player's status are asked for.
    pub asked: bool,
    /// The actions plugins add to the menus, each with the kind of item it is for.
    pub menu: Vec<(nori_plugins::ItemKind, nori_plugins::MenuEntry)>,
    /// The playing song's id and the plugins' lines in its info.
    pub info: Option<(String, Vec<nori_plugins::InfoGroup>)>,
    /// The plugins' lines under the output row.
    pub status: Vec<String>,
}

impl Slots {
    /// The menu actions plugins add for items of `kind`.
    pub fn menu_of(&self, kind: nori_plugins::ItemKind) -> impl Iterator<Item = &nori_plugins::MenuEntry> {
        self.menu.iter().filter(move |(k, _)| *k == kind).map(|(_, e)| e)
    }

    /// The playing song's info lines, read for `song`.
    pub fn info_of(&self, song: &str) -> &[nori_plugins::InfoGroup] {
        match &self.info {
            Some((id, groups)) if id == song => groups,
            _ => &[],
        }
    }
}

/// A row of the devices panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRow {
    Here,
    /// Index into [`Devices::list`]: its name, then what it plays.
    Device(usize),
    Playing(usize),
    /// Starts a jam, or shows the one hosted.
    Jam,
    /// Joins someone's jam.
    Join,
    Pair,
    ResetPairing,
    /// A plugin's name over its rows: index into [`Devices::plugins`].
    PluginName(usize),
    /// Row `.1` of plugin `.0`, then its hint.
    Plugin(usize, usize),
    PluginHint(usize, usize),
}

impl Devices {
    /// The panel's rows, the plugins' last.
    pub fn rows(&self) -> Vec<DeviceRow> {
        let mut rows = if self.on { vec![DeviceRow::Here] } else { Vec::new() };
        if self.on {
            rows.extend((0..self.list.len()).flat_map(|i| [DeviceRow::Device(i), DeviceRow::Playing(i)]));
            if self.jams || self.jam.is_some() {
                rows.push(DeviceRow::Jam);
            }
            rows.push(DeviceRow::Join);
            if self.pairing.is_some() {
                rows.extend([DeviceRow::Pair, DeviceRow::ResetPairing]);
            }
        }
        for (c, card) in self.plugins.iter().enumerate() {
            rows.push(DeviceRow::PluginName(c));
            for (r, row) in card.rows.iter().enumerate() {
                rows.push(DeviceRow::Plugin(c, r));
                if row.hint.is_some() {
                    rows.push(DeviceRow::PluginHint(c, r));
                }
            }
        }
        rows
    }

    /// Moves the selection off a line that only says something (what a device plays, a plugin's name
    /// or hint), in direction `down`.
    fn settle(&mut self, down: bool) {
        let rows = self.rows();
        let picks = |at: &usize| !matches!(rows[*at], DeviceRow::Playing(_) | DeviceRow::PluginName(_) | DeviceRow::PluginHint(..));
        if self.sel.at >= rows.len() || picks(&self.sel.at) {
            return;
        }
        let after = (self.sel.at + 1..rows.len()).find(picks);
        let before = (0..self.sel.at).rev().find(picks);
        if let Some(at) = if down { after.or(before) } else { before.or(after) } {
            self.sel.at = at;
        }
    }

    /// Row `r` of plugin `c` as the settings draw it.
    pub fn plugin_row(&self, c: usize, r: usize) -> Option<crate::settings_view::Row> {
        let card = self.plugins.get(c)?;
        Some(crate::settings_view::plugin_row(&card.id, card.rows.get(r)?))
    }

    /// The panel shows: other devices or jams are on, or a plugin has rows here.
    pub fn shown(&self) -> bool {
        self.on || !self.plugins.is_empty()
    }

    /// The jam's guests, the host left out.
    pub fn listeners(&self) -> Vec<&nori_core::remote::wire::JamMember> {
        self.jam.as_ref().map_or_else(Vec::new, |j| j.listeners().collect())
    }

    /// The jam's host's name.
    pub fn host(&self) -> &str {
        self.jam.as_ref().map_or("", JamView::host)
    }

    /// The jam as the player bar and the queue say it: "Jam · 2 listening", a guest's with its host.
    pub fn jam_strip(&self) -> Option<String> {
        let j = self.jam.as_ref()?;
        let n = self.listeners().len();
        if self.controls.is_some_and(|c| c.paused_here) {
            return Some(crate::text::JAM_PAUSED_HERE.to_string());
        }
        Some(if j.hosting { crate::text::jam_strip(n) } else { crate::text::jam_guest_strip(self.host(), n) })
    }

    /// The requests the queue lists ([`JamView::asks`]).
    pub fn asks(&self) -> Vec<&Pending> {
        self.jam.as_ref().map_or_else(Vec::new, |j| j.asks().collect())
    }

    /// The songs this guest asked for that wait for the host, by id.
    pub fn asked(&self) -> HashSet<&str> {
        let guest = self.jam.as_ref().is_some_and(|j| !j.hosting);
        self.asks().into_iter().filter(|_| guest).map(|p| p.song.id.as_str()).collect()
    }

    /// How this guest listens along; Watching while hosting or in no jam.
    pub fn listening(&self) -> Listening {
        self.jam.as_ref().filter(|j| !j.hosting).map_or(Listening::Watching, |j| j.listening)
    }
}

/// A row of the queue panel: the hosted jam's header and requests, then the songs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueRow {
    /// How many listen; opens the invite, as the line of their names under it does.
    Jam,
    Listeners,
    /// A guest's switch for listening along.
    Listen,
    /// A line of why a guest who asked to listen along does not.
    AlongNote(usize),
    /// A guest's heading over its requests.
    Asking,
    /// A request, by index into [`Devices::asks`].
    Ask(usize),
    /// A line of the note under a provider's song asked for: request, line.
    Downloads(usize, usize),
    End,
    /// A song by list index.
    Song(usize),
}

impl QueueRow {
    /// The request a row is about.
    pub fn ask(self) -> Option<usize> {
        match self {
            QueueRow::Ask(k) | QueueRow::Downloads(k, _) => Some(k),
            _ => None,
        }
    }
}

/// A popup over the screen.
pub enum Overlay {
    Help { scroll: usize },
    /// A choice list; `target` receives the value picked.
    Picker { title: String, options: Vec<(String, String)>, sel: Sel, target: Target },
    /// A text field whose text goes to `target`.
    Input { title: String, text: String, secret: bool, target: Target },
    /// A question before `cmd` is carried out: what it is about, and the word for going ahead.
    Confirm { title: String, lines: Vec<String>, yes: String, cmd: Box<Cmd> },
    /// What the selection's menu offers, the song's less used actions `folded` under More until it opens.
    Menu { title: String, item: Box<Item>, lines: Vec<crate::menu::MenuLine>, folded: Vec<crate::menu::MenuLine>, sel: Sel },
    /// A new playlist's name, for `songs`.
    NewPlaylist { text: String, songs: Vec<String> },
    /// The jam's invite: its link and QR code.
    Invite { link: String },
    JamSettings,
    /// An invite link pasted to join someone's jam, and why joining failed.
    Join { text: String, error: Option<String>, busy: bool },
    /// The profile's account with a provider: who is signed in (empty: no one), whether the provider
    /// ended the session, cookies being pasted, why signing in failed, and a sign-in under way.
    Account { kind: nori_core::settings::ServerKind, user: String, ended: bool, pasting: bool, text: String, error: Option<String>, busy: bool },
}

/// What a picker sets.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Setting(String),
    Device,
    Preset,
    /// The playlist these songs go to (empty: a new one).
    Playlist(Vec<String>),
    /// A sound level the equalizer's tools set.
    Level(EqLevel),
    /// A row of plugin `plugin`, of `kind`.
    Plugin { plugin: String, row: String, kind: nori_plugins::RowKind },
    /// The plugin file or folder to install.
    Install,
    /// The repo to add, once the user trusts it.
    AddRepo,
}

impl Target {
    /// The command for picking `value`; None for a non-numeric preset.
    pub fn cmd(self, value: String) -> Option<Cmd> {
        match self {
            Target::Setting(name) => Some(Cmd::Setting(name, value)),
            Target::Device => Some(Cmd::Device(value)),
            Target::Preset => value.parse().ok().map(|i| Cmd::Sound(SoundToolCmd::Preset(i))),
            Target::Playlist(songs) => Some(Cmd::AddToPlaylist { id: value, songs }),
            Target::Level(l) => value.parse().ok().map(|v| Cmd::Level(l, v)),
            Target::Plugin { plugin, row, kind } => {
                use nori_plugins::{RowKind, Value};
                let v = match kind {
                    RowKind::Switch => Value::Switch(value.parse().ok()?),
                    RowKind::Choice => Value::Choice(value.parse().ok()?),
                    RowKind::Level => Value::Level(value.parse().ok()?),
                    RowKind::Text | RowKind::Secret => Value::Text(value),
                    RowKind::Action | RowKind::Status | RowKind::Section | RowKind::Note | RowKind::Link => return None,
                };
                Some(Cmd::Plugin(PluginCmd::Set(plugin, row, v)))
            }
            Target::Install => Some(Cmd::Plugin(PluginCmd::Install(value.trim().to_string()))),
            Target::AddRepo => Some(Cmd::Plugin(PluginCmd::AddRepo(value.trim().to_string()))),
        }
    }
}

/// A clickable area's meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A tab of the top outline (or a footer link).
    Tab(View),
    /// A list row or card by index.
    Row(ListRef, usize),
    /// A list's whole area, for the wheel.
    List(ListRef),
    Seek,
    Volume,
    Button(Button),
    SearchField,
    LoginField(usize),
    /// Index into `settings_view::GROUPS`.
    Group(usize),
    /// The equalizer's curve, off its points.
    Curve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListRef {
    Home,
    Albums,
    Artists,
    Songs,
    Playlists,
    Favorites,
    History,
    Search,
    Page,
    Downloads,
    Settings,
    Eq,
    Queue,
    Lyrics,
    Devices,
    Profiles,
    Picker,
    Menu,
    Help,
}

impl ListRef {
    /// The window part the list is in.
    fn focus(self) -> Focus {
        match self {
            ListRef::Queue | ListRef::Lyrics | ListRef::Devices => Focus::Panel,
            _ => Focus::Main,
        }
    }

    /// Whether a single click opens: lists of things with pages, not of songs.
    fn opens_on_click(self) -> bool {
        matches!(self, ListRef::Albums | ListRef::Artists | ListRef::Playlists)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Previous,
    Toggle,
    Next,
    Shuffle,
    Repeat,
    Panel(Panel),
    Full,
    PlayAll,
    ShuffleAll,
    /// Star the page's album or artist.
    Star,
    /// One of the page's actions (`menus::page_actions`).
    Page(PageAction),
    /// The mix shown, kept as a playlist.
    SaveMix,
    /// Star the playing song.
    StarSong,
    Help,
    Connect,
    /// The login's way to Settings › Plugins.
    LoginPlugins,
    /// Plays the queue offered on Home, or turns it down.
    Resume,
    Dismiss,
}

/// Layout facts from the last frame: the full player's panel width, and whether a clock shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    /// The panel's width inside its margins.
    pub panel_w: u16,
    /// A seek clock is on screen, so playback needs a redraw each second.
    pub clock: bool,
}

impl Default for Shown {
    fn default() -> Self {
        Shown { panel_w: 37, clock: true }
    }
}

/// Columns before the note under a provider's song asked for, in the queue panel.
pub const QUEUE_NOTE_INDENT: usize = 4;

/// A mouse drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    /// Seek bar position as a fraction of the song.
    Seek(f32),
    Volume,
    /// Index into `eq_rows`.
    Band(usize),
}

#[derive(Default)]
pub struct App {
    pub view: View,
    /// Pages opened over the view, topmost last.
    pub pages: Vec<Page>,
    pub focus: Focus,
    pub home: Home,
    pub library: Library,
    pub search: Search,
    /// The full player's panel.
    pub panel: Panel,
    /// Full-window player.
    pub full: bool,
    pub shown: Shown,
    pub queue: Option<PlaylistView>,
    pub queue_sel: Sel,
    /// Id of the song last removed from the queue, for undo.
    pub taken: Option<String>,
    pub downloads: Load<Box<Downloads>>,
    pub downloads_sel: Sel,
    pub downloads_at: Option<Instant>,
    pub settings: SettingsView,
    pub eq_sel: Sel,
    pub login: Login,
    pub lyrics: Option<SongLyrics>,
    pub lyrics_for: Option<String>,
    pub lyrics_sel: Option<usize>,
    pub lyrics_wake: Option<Instant>,
    pub overlay: Option<Overlay>,
    pub now: Now,
    /// The audible song (during a mix, the louder one).
    pub song: Option<Song>,
    /// Songs' download states as last read, by id.
    pub downloaded: HashMap<String, SongDownload>,
    /// This session's star marks, drawn over the records' flags.
    pub marks: StarMarks,
    pub devices: Devices,
    /// What the plugins add to the screens.
    pub slots: Slots,
    /// What the open profile offers (a jam guest's: the host's library, its picks asked of the host).
    pub rules: ProfileRules,
    /// The planned transition out of `song`.
    pub transition: Option<nori_core::automix::planner::TransitionNote>,
    /// The transition that brought `song` in, while still mixing.
    pub mixed_in: Option<nori_core::automix::planner::TransitionNote>,
    /// Cover id of `song`.
    pub cover_art: Option<String>,
    pub colours: Option<Box<CoverColours>>,
    /// The season's colors, for the chrome and the pages.
    pub look: Theme,
    /// The interface is dark, as the appearance setting and the terminal have it.
    pub dark: bool,
    /// The full player's colors: the cover's when it wears them, else the season's.
    pub theme: Theme,
    /// Paint the page in the season's colors.
    pub paint: bool,
    /// How many colors the terminal shows.
    pub colors: Colors,
    /// No stamp on the seal ($NORI_REDUCE_MOTION).
    pub reduced_motion: bool,
    /// The transport button or the seal just pressed, until its flash ends.
    pub flash: Option<(Button, Instant)>,
    pub prefs: StoredPrefs,
    pub volume: f32,
    pub mouse: bool,
    pub images: bool,
    /// The library's section last open, which its tab opens.
    pub section: View,
    /// Image protocol name, for the settings page.
    pub protocol: &'static str,
    pub server: String,
    /// The output this computer plays through, as the engine last named it; empty for the default.
    pub output: String,
    pub offline: bool,
    pub unreachable: Option<String>,
    pub note: Option<(String, bool, Instant)>,
    pub hits: Vec<(Rect, Hit)>,
    pub cmds: Vec<Cmd>,
    pub dirty: bool,
    /// The terminal has focus; true until it says otherwise, as terminals without focus events never do.
    pub in_sight: bool,
    pub quit: bool,
    /// Whether the engine holds the equalizer's low-latency buffer ([`App::sound_edited`]); released
    /// when the equalizer closes or is switched off.
    pub tuning: bool,
    /// The sound was changed on the equalizer since it was opened.
    pub touched: bool,
    pub drag: Option<Drag>,
    /// Where the equalizer's curve was drawn, for the mouse.
    pub eq_plot: Rect,
    pub seek_rect: Rect,
    pub volume_rect: Rect,
    /// The last left click, to detect a second click on the same target.
    last_click: Option<(Hit, Instant)>,
}

impl App {
    pub fn new(prefs: StoredPrefs) -> App {
        App {
            paint: true,
            prefs,
            volume: 1.0,
            mouse: true,
            images: true,
            section: View::Albums,
            protocol: "none",
            dirty: true,
            in_sight: true,
            ..App::default()
        }
        .themed()
    }

    /// Takes the color settings (what the terminal shows, whether to paint) and dresses the screens.
    pub fn themed(mut self) -> App {
        self.retheme();
        self
    }

    /// What the terminal calls profile kind `kind`.
    pub fn provider_words(&self, kind: &nori_core::settings::ServerKind) -> crate::text::ProviderWords {
        crate::text::provider_of(&self.settings.own.kinds, kind)
    }

    pub fn say(&mut self, text: impl Into<String>, error: bool) {
        self.note = Some((text.into(), error, Instant::now()));
        self.dirty = true;
    }

    /// Takes the stored settings and updates what depends on them.
    pub fn prefs_changed(&mut self, prefs: StoredPrefs) {
        self.prefs = prefs;
        self.retheme();
        self.settings.invalidate();
        self.dirty = true;
    }

    pub fn retheme(&mut self) {
        let day = crate::art::Day::now();
        let dark = crate::art::terminal_dark(std::env::var("COLORFGBG").ok().as_deref());
        self.dark = self.prefs.theme.is_dark(dark);
        self.look = Theme::season(&self.prefs, day, dark, self.paint, self.colors);
        self.theme = match &self.colours {
            Some(c) if self.prefs.theme.covers_tint_pages() && self.images => Theme::dyed(c.edge, &self.prefs, day, self.dark, self.paint, self.colors),
            _ => self.look,
        };
    }

    /// Whether lyrics are visible (the full player's panel).
    pub fn lyrics_shown(&self) -> bool {
        self.full && self.panel == Panel::Lyrics
    }

    /// Whether the transition note is visible.
    pub fn transition_shown(&self) -> bool {
        self.full
    }

    // ---- going places ----

    /// The library's sections kept in the order chosen, of those the profile has and this client lists.
    pub fn sections(&self) -> Vec<View> {
        let shown = nori_core::browse::library_sections_shown(self.prefs.library_sections.clone(), self.rules.sections.clone());
        shown
            .into_iter()
            .filter_map(|s| match s {
                LibrarySection::Albums => Some(View::Albums),
                LibrarySection::Artists => Some(View::Artists),
                LibrarySection::Songs => Some(View::Songs),
                LibrarySection::Playlists => Some(View::Playlists),
                _ => None,
            })
            .collect()
    }

    /// The places at the top of the library's section `view`, as the phone's library has them: the
    /// favorites where they are kept, downloads and history where the profile keeps them, and shuffling the
    /// list where it can be (an account's albums or songs).
    pub fn places(&self, view: View) -> Vec<Place> {
        let kept = nori_core::browse::library_sections_shown(self.prefs.library_sections.clone(), self.rules.sections.clone());
        let mut out = Vec::new();
        if kept.contains(&LibrarySection::Favourites) {
            out.push(Place::Favorites);
        }
        for (s, place) in [(LibrarySection::Downloads, Place::Downloads), (LibrarySection::History, Place::History)] {
            if self.rules.sections.contains(&s) {
                out.push(place);
            }
        }
        if self.rules.account && matches!(view, View::Albums | View::Songs) {
            out.push(Place::Shuffle);
        }
        out
    }

    /// The lines of the library's section open: its places, then its entries; in Artist order the
    /// albums under a line for each artist.
    pub fn library_lines(&self) -> Vec<LibLine> {
        let l = &self.library;
        let entries = match self.view {
            View::Albums => l.albums.ready().map_or(0, Vec::len),
            View::Artists => l.artists.ready().map_or(0, Vec::len),
            View::Songs => l.songs.ready().map_or(0, Vec::len),
            View::Playlists => l.playlists.ready().map_or(0, Vec::len),
            _ => 0,
        };
        let mut out: Vec<LibLine> = self.places(self.view).into_iter().map(LibLine::Place).collect();
        if !out.is_empty() && entries > 0 {
            out.push(LibLine::Gap);
        }
        if self.view == View::Albums && l.album_sort == AlbumSort::ArtistRows {
            let starts: Vec<usize> = l.album_shelves.iter().map(|s| *s as usize).filter(|s| *s < entries).collect();
            for (k, &start) in starts.iter().enumerate() {
                let end = starts.get(k + 1).copied().unwrap_or(entries);
                out.push(LibLine::Shelf(start));
                out.extend((start..end).map(LibLine::Entry));
            }
        } else {
            out.extend((0..entries).map(LibLine::Entry));
        }
        out
    }

    /// Selects the first entry of the library's section `view` once its first page came, past its places,
    /// unless the selection is already further on.
    fn onto_entries(&mut self, view: View) {
        let first = self.places(view).len() + 1;
        let l = &mut self.library;
        let (sel, has) = match view {
            View::Albums => (&mut l.albums_sel, l.albums.ready().is_some_and(|v| !v.is_empty())),
            View::Artists => (&mut l.artists_sel, l.artists.ready().is_some_and(|v| !v.is_empty())),
            View::Songs => (&mut l.songs_sel, l.songs.ready().is_some_and(|v| !v.is_empty())),
            View::Playlists => (&mut l.playlists_sel, l.playlists.ready().is_some_and(|v| !v.is_empty())),
            _ => return,
        };
        if has && first > 1 && sel.at < first {
            sel.at = first;
        }
    }

    /// The library section's selection and its line.
    fn library_line(&self) -> Option<LibLine> {
        let l = &self.library;
        let sel = match self.view {
            View::Albums => l.albums_sel,
            View::Artists => l.artists_sel,
            View::Songs => l.songs_sel,
            View::Playlists => l.playlists_sel,
            _ => return None,
        };
        self.library_lines().get(sel.at).copied()
    }

    /// Whether the profile has `view`.
    pub fn reaches(&self, view: View) -> bool {
        match view {
            View::Downloads => self.rules.sections.contains(&LibrarySection::Downloads),
            View::History => self.rules.sections.contains(&LibrarySection::History),
            View::Favorites => self.rules.stars,
            View::Home | View::Search | View::Equalizer | View::Settings | View::Login => true,
            v => self.sections().contains(&v),
        }
    }

    /// The library's tab: the section last open, or the first the profile has.
    pub fn library(&self) -> View {
        if self.reaches(self.section) { self.section } else { self.sections().first().copied().unwrap_or(View::Favorites) }
    }

    /// A jam guest's: its picks are asked of the host, and the player shows the jam, which it controls by
    /// its role ([`App::offers`]).
    pub fn guest(&self) -> bool {
        self.rules.asks
    }

    /// Whether the player offers `control`: a jam guest's by its role (the core's jam controls), this
    /// computer's own player all of them.
    pub fn offers(&self, control: impl Fn(&Controls) -> Reach) -> bool {
        !self.guest() || self.devices.controls.is_some_and(|c| control(&c.controls) != Reach::Nowhere)
    }

    /// Opens a root view, closing any opened pages, and requests its data if needed.
    pub fn go(&mut self, view: View) {
        if view != View::Equalizer {
            self.touched = false;
        }
        self.view = view;
        if crate::chrome::tab_name(view) == crate::chrome::tab_name(View::Albums) {
            self.section = view;
        }
        self.pages.clear();
        self.full = false;
        self.focus = Focus::Main;
        self.tune();
        self.dirty = true;
        // What the downloads weigh, beside their place.
        if self.sections().contains(&view) && self.places(view).contains(&Place::Downloads) {
            self.cmds.push(Cmd::Load(Req::DownloadsSize));
        }
        let l = &mut self.library;
        match view {
            View::Home => {
                if !self.home.asked {
                    self.home.asked = true;
                    (self.home.rows, self.home.mixes) = (Vec::new(), Vec::new());
                    self.home.sel = Sel::default();
                    self.cmds.push(Cmd::Load(Req::Home));
                }
                // Each time Home comes into sight, the queue another device left; a jam guest's is the host's.
                if self.rules.account {
                    let (playing, shown) = (self.now.state == State::Playing, self.song.as_ref().map(|s| s.id.clone()));
                    self.cmds.push(Cmd::LookForOffer { playing, shown });
                }
            }
            View::Albums if matches!(l.albums, Load::Idle) => {
                l.albums = Load::Loading;
                // The order it was left in (`album_sort_saved`).
                l.album_sort = nori_core::browse::album_sort_saved(self.prefs.list_prefs.clone());
                self.cmds.push(Cmd::Load(Req::Albums { sort: l.album_sort, favorites: l.albums_favorites, offset: 0 }));
            }
            View::Artists if matches!(l.artists, Load::Idle) => {
                l.artists = Load::Loading;
                self.cmds.push(Cmd::Load(Req::Artists));
            }
            View::Songs if matches!(l.songs, Load::Idle) => {
                l.songs = Load::Loading;
                l.song_sort = nori_core::browse::song_sort_saved(self.prefs.list_prefs.clone());
                self.cmds.push(Cmd::Load(Req::Songs { sort: l.song_sort.clone(), starred: l.songs_favorites, offset: 0 }));
            }
            View::History if matches!(l.history, Load::Idle) => {
                l.history = Load::Loading;
                self.cmds.push(Cmd::Load(Req::History(None)));
            }
            View::Playlists if matches!(l.playlists, Load::Idle) => {
                l.playlists = Load::Loading;
                self.cmds.push(Cmd::Load(Req::Playlists));
            }
            View::Favorites if matches!(l.favorites, Load::Idle) => {
                l.favorites = Load::Loading;
                self.cmds.push(Cmd::Load(Req::Favorites));
            }
            View::Downloads => self.cmds.push(Cmd::Load(Req::Downloads)),
            View::Settings if !self.settings.facts_asked => {
                self.settings.facts_asked = true;
                self.cmds.push(Cmd::Load(Req::Facts));
                self.cmds.push(Cmd::Plugin(PluginCmd::Read));
            }
            View::Search if self.search.view.is_none() => self.search.editing = true,
            _ => {}
        }
    }

    pub(crate) fn open_page(&mut self, page: Page) {
        self.cmds.extend(page.req().map(Cmd::Load));
        self.pages.push(page);
        self.focus = Focus::Main;
        self.full = false;
        self.dirty = true;
    }

    pub fn open_album(&mut self, id: String) {
        self.open_page(Page::Album { id, detail: Load::Loading, sel: Sel::default() });
    }

    pub fn open_artist(&mut self, id: String) {
        self.open_page(Page::Artist { id, detail: Load::Loading, sel: Sel::default() });
    }

    pub(crate) fn open_playlist(&mut self, id: String) {
        self.open_page(Page::Playlist { id, detail: Load::Loading, sel: Sel::default() });
    }

    /// The topmost opened page.
    pub fn page(&self) -> Option<&Page> {
        self.pages.last()
    }

    fn page_mut(&mut self) -> Option<&mut Page> {
        self.pages.last_mut()
    }

    /// Opens the full player on panel `p`; the same again closes it. The queue, lyrics and devices take
    /// the keys.
    pub fn set_panel(&mut self, p: Panel) {
        if self.full && self.panel == p {
            return self.close_full();
        }
        if p == Panel::Devices && !self.devices.shown() {
            return self.say(crate::text::REMOTE_OFF, false);
        }
        self.panel = p;
        self.open_full();
        match p {
            // Select the playing song.
            Panel::Queue => {
                let current = self.queue.as_ref().map_or(-1, |q| q.index);
                if let Some(row) = self.queue_rows().iter().position(|r| *r == QueueRow::Song(current as usize)) {
                    self.queue_sel.at = row;
                }
            }
            // Select the device playing.
            Panel::Devices => {
                let d = &mut self.devices;
                let active = d.active.as_ref().and_then(|(id, _)| d.list.iter().position(|x| &x.id == id));
                d.sel.at = active.map_or(0, |i| d.rows().iter().position(|r| *r == DeviceRow::Device(i)).unwrap_or(0));
                d.settle(true);
            }
            Panel::Lyrics => self.want_lyrics(),
            Panel::Playing => {}
        }
    }

    fn open_full(&mut self) {
        if !self.full {
            self.ask_slots();
        }
        self.full = true;
        self.focus = Focus::Panel;
        if self.panel == Panel::Lyrics {
            self.want_lyrics();
        }
    }

    fn close_full(&mut self) {
        self.full = false;
        self.focus = Focus::Main;
    }

    fn toggle_full(&mut self) {
        if self.full {
            self.close_full();
        } else {
            self.set_panel(self.panel);
        }
    }

    /// The next (or previous) panel of the full player.
    fn cycle(&mut self, forward: bool) {
        if !self.full {
            return self.dirty = false;
        }
        let shown: Vec<Panel> = PANELS.iter().map(|p| p.0).filter(|p| *p != Panel::Devices || self.devices.shown()).collect();
        let at = shown.iter().position(|p| *p == self.panel).unwrap_or(0);
        let n = shown.len();
        let to = shown[if forward { (at + 1) % n } else { (at + n - 1) % n }];
        self.panel = to;
        self.open_full();
    }

    fn want_lyrics(&mut self) {
        let Some(id) = self.song.as_ref().map(|s| s.id.clone()) else { return };
        if self.lyrics_for.as_deref() != Some(&id) {
            self.lyrics_for = Some(id.clone());
            self.lyrics = None;
            self.lyrics_sel = None;
            self.cmds.push(Cmd::Lyrics(id));
        }
        self.lyrics_wake = Some(Instant::now());
    }

    // ---- what comes in ----

    pub fn handle(&mut self, msg: Msg) {
        // Handlers clear `dirty` when nothing changed; an earlier message in the batch stays dirty.
        let was = std::mem::replace(&mut self.dirty, true);
        self.take(msg);
        self.dirty |= was;
    }

    fn take(&mut self, msg: Msg) {
        match msg {
            Msg::Key(k) => self.key(k),
            Msg::Mouse(m) => self.mouse_event(m),
            Msg::Paste(text) => self.paste(&text),
            Msg::Resize => {}
            // Regaining focus redraws; the runner decides whether fully.
            Msg::Focus(on) => {
                self.in_sight = on;
                self.dirty = on;
            }
            Msg::Engine(e) => self.engine(e),
            Msg::Data(req, r) => self.data(req, r),
            Msg::Cover { art, colours, .. } => {
                if let Some(c) = colours {
                    if self.cover_art.as_deref() == Some(&art) {
                        self.colours = Some(c);
                        self.retheme();
                    }
                }
            }
            Msg::Lyrics { song, pick } => {
                if self.lyrics_for.as_deref() == Some(&song) && self.lyrics.as_ref().is_none_or(|l| l.replaced_by(&pick)) {
                    let pos = self.now.position(Instant::now());
                    self.lyrics = Some(SongLyrics::new(pick, pos));
                    self.lyrics_wake = Some(Instant::now());
                }
            }
            Msg::Search(v) => {
                if v.query == self.search.text.trim() {
                    self.search.view = Some(v);
                    self.search.settle(true);
                }
            }
            Msg::Note { text, error } => self.say(text, error),
            // A page or list on screen, read again.
            Msg::Reload(req) => {
                let playlists = req == Req::Playlists;
                if playlists && self.library.playlists.ready().is_some() {
                    self.cmds.push(Cmd::Load(Req::Playlists));
                }
                // Home's playlist rows are read with Home.
                let rows = |s: &Option<Shelf>| s.as_ref().is_some_and(|s| matches!(s.row, Some(HomeRow::Playlists | HomeRow::Pinned)));
                if playlists && self.home.rows.iter().any(rows) {
                    self.cmds.push(Cmd::Load(Req::Home));
                }
                if self.pages.iter().any(|p| p.req().as_ref() == Some(&req)) {
                    self.cmds.push(Cmd::Load(req));
                }
            }
            Msg::Offer(offer) => {
                self.home.offer = offer;
                let (here, playing) = (self.song.as_ref().map(|s| s.id.clone()), self.now.state == State::Playing);
                self.home.offer_still(here, playing);
            }
            Msg::LoggedIn(r) => {
                self.login.busy = false;
                if let Err(e) = r {
                    self.login.error = Some(e);
                }
            }
            Msg::Reachable(r) => self.unreachable = r.err(),
            // Another device playing shows its own volume.
            Msg::Volume(v) if self.devices.active.is_none() => self.volume = v,
            Msg::Volume(_) => self.dirty = false,
            Msg::Starred(marks) => self.marks = marks,
            // The runner read the devices into `devices`.
            Msg::Remote => {}
            Msg::Jam(Ok(())) => {
                self.panel = Panel::Queue;
                self.invite();
            }
            Msg::Jam(Err(e)) => self.say(format!("{} ({e})", crate::text::JAM_FAILED), true),
            Msg::Joined(Err(e)) => match &mut self.overlay {
                Some(Overlay::Join { error, busy, .. }) => {
                    *error = Some(e.clone());
                    *busy = false;
                }
                _ => self.say(e.clone(), true),
            },
            // The runner opens these, and the profiles a jam is joined and left with.
            Msg::SignedIn(nori_core::providers::SignInResult::SignedIn { .. }) => self.overlay = None,
            Msg::SignedIn(r) => {
                let kind = self.prefs.active_server().map(|p| p.kind.clone()).unwrap_or_default();
                let said = crate::text::sign_in(&self.provider_words(&kind).name, &r);
                match &mut self.overlay {
                    Some(Overlay::Account { error, busy, .. }) => {
                        *error = Some(said);
                        *busy = false;
                    }
                    _ => self.say(said, true),
                }
            }
            Msg::Preview(Ok(p)) => self.overlay = Some(preview_sheet(p)),
            Msg::Installed(Ok(_)) => self.say(crate::text::PLUGIN_INSTALLED, false),
            Msg::Installed(Err(e)) | Msg::Preview(Err(e)) => self.say(crate::text::plugin_refused(&e), true),
            Msg::Slots { song, info, status } => {
                if self.song.as_ref().is_some_and(|s| s.id == song) {
                    self.slots.info = Some((song, info));
                    self.slots.status = status;
                }
            }
            Msg::Updated(updated) => {
                let named: Vec<(String, String)> = updated.into_iter().map(|(id, to)| (self.settings.own.plugins.iter().find(|c| c.id == id).map_or(id, |c| c.name.clone()), to)).collect();
                self.say(crate::text::plugins_updated(&named), false);
                self.cmds.push(Cmd::Plugin(PluginCmd::Read));
            }
            Msg::From(..) | Msg::Joined(Ok(_)) | Msg::Left(_) | Msg::Offers(_) => {}
        }
    }

    fn engine(&mut self, e: Event) {
        match e {
            Event::State(s) => {
                // Restart the clock from now: the engine's status may still predate this event.
                let t = Instant::now();
                self.now.position_ms = self.now.position(t);
                self.now.at = t;
                self.now.state = s;
                if s == State::Playing {
                    self.lyrics_wake = Some(Instant::now());
                }
                // Music started: Home lets the offer go.
                let here = self.song.as_ref().map(|s| s.id.clone());
                self.home.offer_still(here, s == State::Playing);
            }
            Event::Song { .. } | Event::Looped { .. } => {}
            Event::Error { id, message } => {
                let what = crate::backend::app().song(&id).map_or_else(|| "The output".to_string(), |s| format!("“{}”", s.title));
                self.say(format!("{what} would not play: {message}"), true);
            }
            Event::Buffering(on) => self.now.buffering = on,
            Event::Stopped { .. } => self.say("Playback stopped: too many songs in a row would not play", true),
            Event::Output { name } => {
                self.say(format!("Playing on {name}"), false);
                self.output = name;
            }
            Event::Title(t) => self.say(format!("On air: {t}"), false),
            Event::Bridge { .. } => self.say("The network is gone", true),
            Event::Waiting(true) => self.say("The server could not be reached: trying again", true),
            Event::Waiting(false) => {}
            Event::Mixing(on) => self.now.mixing = on,
            // How a jam's host is followed is the core's jam controls' to show.
            Event::Position { .. } | Event::Placed { .. } | Event::Awake(_) | Event::Following(_) => {}
        }
    }

    /// Asks the plugins that fill a place for the playing song's info and the player's status.
    fn ask_slots(&mut self) {
        if let Some(song) = self.song.clone().filter(|_| self.slots.asked) {
            self.cmds.push(Cmd::Plugin(PluginCmd::Ask(Box::new(song))));
        }
    }

    /// The audible song, as read from the engine; on a change, cover and lyrics follow.
    pub fn heard(&mut self, song: Option<Song>) {
        let changed = self.song.as_ref().map(|s| &s.id) != song.as_ref().map(|s| &s.id);
        self.song = song;
        // The player came to show the song offered.
        self.home.offer_still(self.song.as_ref().map(|s| s.id.clone()), self.now.state == State::Playing);
        if !changed {
            return;
        }
        self.transition = None;
        self.ask_slots();
        let art = self.song.as_ref().and_then(|s| s.cover_art.clone());
        if art != self.cover_art {
            self.colours = None;
            self.retheme();
        }
        self.cover_art = art.clone();
        if let Some(art) = art {
            self.cmds.push(Cmd::Cover(art));
        }
        if self.lyrics_shown() {
            self.want_lyrics();
        } else {
            self.lyrics = None;
            self.lyrics_for = None;
        }
    }

    fn data(&mut self, req: Req, r: Result<Data, String>) {
        match (req, r) {
            (Req::Home, Ok(Data::HomeRow(i, row, title, items))) => {
                // A feed has as many shelves as the service sends.
                if i >= self.home.rows.len() {
                    self.home.rows.resize(i + 1, None);
                }
                if let Some(slot) = self.home.rows.get_mut(i) {
                    *slot = Some(Shelf { title, items, row });
                }
                self.home.error = None;
                self.home.settle(true);
            }
            (Req::Home, Ok(Data::Mixes(tiles))) => {
                self.home.mixes = tiles;
                self.home.settle(true);
            }
            (Req::Home, Err(e)) => self.home.error = Some(e),
            // An order left since is not shown.
            (Req::Albums { sort, favorites, .. }, _) if (sort, favorites) != (self.library.album_sort, self.library.albums_favorites) => self.dirty = false,
            (Req::Albums { sort, favorites, offset }, Ok(Data::Albums(v))) => {
                let l = &mut self.library;
                l.albums_more = nori_core::browse::next_album_page(sort, favorites, offset, ALBUM_PAGE, v.len() as u32).is_some();
                match &mut l.albums {
                    Load::Ready(have) if offset > 0 => {
                        have.truncate(offset as usize);
                        have.extend(v);
                    }
                    slot => *slot = Load::Ready(v),
                }
                if sort == AlbumSort::ArtistRows {
                    l.album_shelves = nori_core::browse::artist_rows(l.albums.ready().cloned().unwrap_or_default());
                }
                self.onto_entries(View::Albums);
            }
            (Req::Albums { .. }, Err(e)) => self.library.albums = Load::Failed(e),
            (Req::Artists, Ok(Data::Artists(v))) => {
                self.library.artists = Load::Ready(v);
                self.onto_entries(View::Artists);
            }
            (Req::Artists, Err(e)) => self.library.artists = Load::Failed(e),
            (Req::Playlists, Ok(Data::Playlists(v))) => {
                self.library.playlists = Load::Ready(v);
                self.onto_entries(View::Playlists);
            }
            (Req::Playlists, Err(e)) => self.library.playlists = Load::Failed(e),
            (Req::Favorites, Ok(Data::Favorites(v))) => self.library.favorites = Load::Ready(v),
            (Req::Favorites, Err(e)) => self.library.favorites = Load::Failed(e),
            (Req::Songs { ref sort, starred, .. }, _) if (sort, starred) != (&self.library.song_sort, self.library.songs_favorites) => self.dirty = false,
            (Req::Songs { offset, .. }, Ok(Data::Songs(v, exhausted))) => {
                self.library.songs_more = !exhausted;
                match &mut self.library.songs {
                    Load::Ready(have) if offset > 0 => {
                        have.truncate(offset as usize);
                        have.extend(v);
                    }
                    slot => *slot = Load::Ready(v),
                }
                self.onto_entries(View::Songs);
            }
            (Req::Songs { .. }, Err(e)) => self.library.songs = Load::Failed(e),
            (Req::History(after), Ok(Data::History(page))) => {
                let l = &mut self.library;
                l.history_next = page.next;
                let songs = page.entries;
                match &mut l.history {
                    Load::Ready(have) if after.is_some() => have.extend(songs),
                    slot => *slot = Load::Ready(songs),
                }
            }
            (Req::History(_), Err(e)) => self.library.history = Load::Failed(e),
            (Req::DownloadsSize, Ok(Data::DownloadsSize(bytes))) => self.library.downloads_bytes = Some(bytes),
            (Req::DownloadsSize, Err(_)) => self.dirty = false,
            (Req::Downloads, Ok(Data::Downloads(d))) => {
                self.downloads_at = d.refresh_at();
                self.downloads = Load::Ready(d);
            }
            (Req::DownloadProgress, Ok(Data::DownloadProgress(s))) => match &mut self.downloads {
                Load::Ready(d) if d.finished == s.finished.len() => {
                    let s = *s;
                    (d.active, d.queued, d.failed) = (s.active, s.queued, s.failed);
                    self.downloads_at = d.refresh_at();
                }
                _ => self.cmds.push(Cmd::Load(Req::Downloads)),
            },
            (Req::Downloads | Req::DownloadProgress, Err(e)) => self.downloads = Load::Failed(e),
            (Req::Facts, Ok(Data::Facts(f))) => self.settings.set_facts(*f),
            (Req::Facts, Err(_)) => {}
            (Req::Row { row, offset }, r) => {
                let Some(Page::Row { items, more, .. }) = self.pages.iter_mut().rev().find(|p| matches!(p, Page::Row { row: Some(at), .. } if *at == row)) else { return self.dirty = false };
                match r {
                    Ok(Data::Row(v, done)) => {
                        if offset == 0 {
                            *items = v;
                        } else {
                            items.truncate(offset as usize);
                            items.extend(v);
                        }
                        *more = !done;
                    }
                    _ => *more = false,
                }
            }
            (req, r) => {
                for page in self.pages.iter_mut().rev() {
                    if page.req().as_ref() != Some(&req) {
                        continue;
                    }
                    // What of its songs is downloaded, for its head's download entry.
                    if let Ok(Data::Album(d)) = &r {
                        self.cmds.push(Cmd::ReadDownloads(d.songs.iter().map(|s| s.id.clone()).collect()));
                    }
                    if let Ok(Data::Playlist(d)) = &r {
                        self.cmds.push(Cmd::ReadDownloads(d.songs.iter().map(|s| s.id.clone()).collect()));
                    }
                    if let Ok(Data::Mix(d)) = &r {
                        self.cmds.push(Cmd::ReadDownloads(d.songs.iter().map(|s| s.id.clone()).collect()));
                    }
                    match (page, r) {
                        (Page::Album { detail, .. } | Page::DownloadedAlbum { detail, .. }, Ok(Data::Album(d))) => *detail = Load::Ready(d),
                        (Page::Artist { detail, sel, .. }, Ok(Data::Artist(d))) => {
                            // The first album, under its shelf's name.
                            if sel.at == 0 && !d.albums.is_empty() {
                                sel.at = 1;
                            }
                            *detail = Load::Ready(d);
                        }
                        (Page::Playlist { detail, .. }, Ok(Data::Playlist(d))) => *detail = Load::Ready(d),
                        (Page::Mix { detail, .. }, Ok(Data::Mix(d))) => *detail = Load::Ready(d),
                        (Page::Mix { detail, .. }, Err(e)) => *detail = Load::Failed(e),
                        (Page::Album { detail, .. } | Page::DownloadedAlbum { detail, .. }, Err(e)) => *detail = Load::Failed(e),
                        (Page::Artist { detail, .. }, Err(e)) => *detail = Load::Failed(e),
                        (Page::Playlist { detail, .. }, Err(e)) => *detail = Load::Failed(e),
                        _ => {}
                    }
                    break;
                }
            }
        }
    }

    // ---- time ----

    /// When the loop must next wake on its own: the next second of playback (while a clock is drawn) and a
    /// lyrics change (both only in sight), a pending search, a note expiring, a downloads refresh (in sight). None when idle.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        let mut at: Option<Instant> = None;
        let mut sooner = |t: Instant| at = Some(at.map_or(t, |a| a.min(t)));
        if self.in_sight && self.now.state == State::Playing && !self.now.buffering {
            if self.shown.clock {
                let pos = self.now.position(now).max(0);
                let left = 1000 - pos % 1000;
                sooner(now + Duration::from_millis((left as f32 / self.now.speed.max(0.1)) as u64 + 5));
            }
            if self.lyrics_shown() {
                if let Some(t) = self.lyrics_wake {
                    sooner(t);
                }
            }
        }
        if let Some(t) = self.search.ask_at {
            sooner(t);
        }
        if let Some((_, _, since)) = &self.note {
            sooner(*since + NOTE_FOR);
        }
        if let Some(t) = self.downloads_at.filter(|_| self.in_sight) {
            sooner(t);
        }
        if let Some((_, since)) = self.flash {
            sooner(since + FLASH);
        }
        at
    }

    /// Runs whatever is due at `now`.
    pub fn tick(&mut self, now: Instant) {
        self.dirty = true;
        if self.search.ask_at.is_some_and(|t| t <= now) {
            self.search.ask_at = None;
            self.cmds.push(Cmd::SearchServer(self.search.text.trim().to_string()));
        }
        if self.note.as_ref().is_some_and(|(_, _, since)| *since + NOTE_FOR <= now) {
            self.note = None;
        }
        if self.flash.is_some_and(|(_, since)| since + FLASH <= now) {
            self.flash = None;
        }
        if self.downloads_at.is_some_and(|t| t <= now) {
            self.downloads_at = None;
            if self.view == View::Downloads {
                self.cmds.push(Cmd::Load(Req::DownloadProgress));
            }
        }
    }

    // ---- keys ----

    /// Whether lyrics take the list keys.
    fn lyrics_focused(&self) -> bool {
        self.full && self.panel == Panel::Lyrics
    }

    fn scopes(&self) -> &'static [Scope] {
        if self.lyrics_focused() {
            return &[Scope::Lyrics, Scope::List, Scope::Global];
        }
        match self.focus {
            Focus::Panel if self.panel == Panel::Queue => &[Scope::Edit, Scope::List, Scope::Global],
            // A plugin's row changes its value as a setting does.
            Focus::Panel if self.plugin_device_row().is_some() => &[Scope::Values, Scope::List, Scope::Global],
            Focus::Panel => &[Scope::List, Scope::Global],
            Focus::Main if self.page().is_some() => &[Scope::List, Scope::Global],
            Focus::Main => match self.view {
                View::Downloads | View::Home => &[Scope::Edit, Scope::List, Scope::Global],
                View::Equalizer => &[Scope::Eq, Scope::List, Scope::Global],
                View::Settings => &[Scope::Values, Scope::List, Scope::Global],
                _ => &[Scope::List, Scope::Global],
            },
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            self.dirty = false;
            return;
        }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.do_action(Action::Quit);
            return;
        }
        if self.overlay.is_some() {
            self.overlay_key(k);
            return;
        }
        if self.view == View::Login {
            self.login_key(k);
            return;
        }
        if self.view == View::Search && self.search.editing && !self.full {
            self.search_key(k);
            return;
        }
        if let Some(a) = action(&k, self.scopes()) {
            self.do_action(a);
        } else {
            self.dirty = false;
        }
    }

    fn paste(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if let Some(Overlay::Input { text: t, .. } | Overlay::NewPlaylist { text: t, .. } | Overlay::Join { text: t, .. } | Overlay::Account { text: t, pasting: true, busy: false, .. }) = &mut self.overlay {
            t.push_str(&text);
        } else if self.view == View::Login {
            self.login.fields[self.login.focus].push_str(&text);
        } else if nori_core::remote::is_invite(&text) || nori_core::remote::pairing_valid(text.clone()) {
            // An invite pasted anywhere offers to join its jam.
            self.overlay = Some(Overlay::Join { text, error: None, busy: false });
        } else {
            if self.view != View::Search {
                self.go(View::Search);
            }
            self.focus = Focus::Main;
            self.search.editing = true;
            self.search.text.push_str(&text);
            self.typed();
        }
    }

    fn typed(&mut self) {
        self.cmds.push(Cmd::SearchTyped(self.search.text.clone()));
        let delay = nori_core::search::live_delay_ms(&self.search.text, nori_core::search::LIVE_SEARCH_DELAY_MS);
        self.search.ask_at = (delay > 0).then(|| Instant::now() + Duration::from_millis(delay as u64));
        self.search.sel = Sel::default();
        self.search.settle(true);
    }

    fn search_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.search.editing = false,
            KeyCode::Enter | KeyCode::Down | KeyCode::Tab => {
                self.search.editing = false;
                self.focus = Focus::Main;
                // Enter queries the server now.
                if k.code == KeyCode::Enter && !self.search.text.trim().is_empty() {
                    self.search.ask_at = None;
                    self.cmds.push(Cmd::SearchServer(self.search.text.trim().to_string()));
                }
                self.search.sel = Sel::default();
                self.search.settle(true);
            }
            KeyCode::Backspace => {
                self.search.text.pop();
                self.typed();
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.text.clear();
                self.typed();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.text.push(c);
                self.typed();
            }
            _ => self.dirty = false,
        }
    }

    /// The kinds of profile the login adds: a Subsonic server's, and the enabled plugins' sources.
    pub fn login_kinds(&self) -> Vec<nori_core::providers::ProviderInfo> {
        self.settings.own.kinds.providers()
    }

    /// Which of `LOGIN_FIELDS` the kind chosen asks.
    pub fn login_fields(&self) -> Vec<usize> {
        let kinds = self.login_kinds();
        let Some(info) = kinds.get(self.login.kind).or(kinds.first()) else { return (0..4).collect() };
        (0..4).filter(|&i| info.fields.iter().any(|f| f.what == LOGIN_ASKS[i])).collect()
    }

    /// Settings › Plugins on its own, from the login: no profile is needed to add one.
    fn plugins_alone(&mut self) {
        self.settings.alone = true;
        self.settings.row = Sel::default();
        self.settings.invalidate();
        self.view = View::Settings;
        self.focus = Focus::Main;
        self.cmds.push(Cmd::Plugin(PluginCmd::Read));
    }

    fn login_key(&mut self, k: KeyEvent) {
        if k.code == KeyCode::Char('p') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return self.plugins_alone();
        }
        let kinds = self.login_kinds().len();
        // Another kind, where there is more than one: its fields from the first.
        if matches!(k.code, KeyCode::Left | KeyCode::Right) && kinds > 1 && !self.login.on_list {
            self.login.kind = (self.login.kind + if k.code == KeyCode::Right { 1 } else { kinds - 1 }) % kinds;
            self.login.focus = self.login_fields().first().copied().unwrap_or(0);
            self.login.error = None;
            return;
        }
        let fields = self.login_fields();
        let l = &mut self.login;
        let profiles = self.prefs.servers.len();
        if l.on_list {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => l.sel.by(-1, profiles),
                KeyCode::Down | KeyCode::Char('j') => l.sel.by(1, profiles),
                KeyCode::Tab | KeyCode::Esc => l.on_list = false,
                KeyCode::Enter => {
                    if let Some(s) = self.prefs.servers.get(l.sel.at) {
                        self.cmds.push(Cmd::SwitchServer(s.id.clone()));
                    }
                }
                KeyCode::Char('q') => self.quit = true,
                _ => {}
            }
            return;
        }
        let at = fields.iter().position(|&f| f == l.focus);
        let last = at.is_none_or(|i| i + 1 == fields.len());
        match k.code {
            KeyCode::Tab | KeyCode::Down => {
                if last && profiles > 0 && k.code == KeyCode::Tab {
                    l.on_list = true;
                } else if let Some(i) = at {
                    l.focus = fields[(i + 1) % fields.len()];
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(i) = at {
                    l.focus = fields[(i + fields.len() - 1) % fields.len()];
                }
            }
            KeyCode::Enter if !last => l.focus = fields[at.map_or(0, |i| i + 1)],
            KeyCode::Enter => self.connect(),
            KeyCode::Esc => {
                if !self.prefs.servers.is_empty() {
                    self.view = View::Settings;
                } else {
                    self.quit = true;
                }
            }
            KeyCode::Backspace if at.is_some() => {
                l.fields[l.focus].pop();
            }
            KeyCode::Char(c) if at.is_some() && !k.modifiers.contains(KeyModifiers::CONTROL) => l.fields[l.focus].push(c),
            _ => {}
        }
    }

    fn connect(&mut self) {
        let kinds = self.login_kinds();
        let Some(info) = kinds.get(self.login.kind).or(kinds.first()) else { return };
        let asked = self.login_fields();
        let l = &mut self.login;
        let typed = |i: usize| if asked.contains(&i) { l.fields[i].clone() } else { String::new() };
        let url = typed(1).trim().trim_end_matches('/').to_string();
        let url = if url.is_empty() || url.contains("://") { url } else { format!("https://{url}") };
        let draft = SavedServer { id: nori_core::settings::new_server_id(), kind: info.kind.clone(), name: typed(0).trim().to_string(), url, user: typed(2).trim().to_string(), password: typed(3), ..Default::default() };
        if !self.settings.own.kinds.server_ready(draft.clone()) {
            l.error = Some(crate::text::LOGIN_MISSING.into());
            return;
        }
        l.error = None;
        l.busy = true;
        self.cmds.push(Cmd::Login(Box::new(draft)));
    }

    fn overlay_key(&mut self, k: KeyEvent) {
        let Some(o) = &mut self.overlay else { return };
        match o {
            Overlay::Help { scroll } => match k.code {
                KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => *scroll += 10,
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                _ => self.overlay = None,
            },
            Overlay::Picker { options, sel, target, .. } => match k.code {
                KeyCode::Down | KeyCode::Char('j') => sel.by(1, options.len()),
                KeyCode::Up | KeyCode::Char('k') => sel.by(-1, options.len()),
                KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char(' ') => {
                    if let Some((_, value)) = options.get(sel.at) {
                        let (target, value) = (target.clone(), value.clone());
                        self.overlay = match target {
                            // A new playlist asks for its name.
                            Target::Playlist(songs) if value.is_empty() => Some(Overlay::NewPlaylist { text: String::new(), songs }),
                            target => {
                                self.cmds.extend(target.cmd(value));
                                None
                            }
                        };
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => self.overlay = None,
                _ => {}
            },
            Overlay::Invite { .. } if k.code == KeyCode::Char('s') => self.overlay = Some(Overlay::JamSettings),
            Overlay::Invite { .. } => self.overlay = None,
            Overlay::JamSettings => match k.code {
                KeyCode::Char('l') => {
                    if let Some(j) = &self.devices.jam { self.cmds.push(Cmd::JamAlong(!j.along)); }
                }
                KeyCode::Char('r') => {
                    if let Some(j) = &self.devices.jam { self.cmds.push(Cmd::JamRemoteSongs(!j.remote_songs)); }
                }
                KeyCode::Esc | KeyCode::Char('h') => self.invite(),
                KeyCode::Char('q') => self.overlay = None,
                _ => {}
            },
            Overlay::Account { busy: true, .. } | Overlay::Join { busy: true, .. } if k.code != KeyCode::Esc => self.dirty = false,
            Overlay::Account { pasting: true, text, error, busy, .. } => match k.code {
                KeyCode::Enter if !text.trim().is_empty() => {
                    *busy = true;
                    *error = None;
                    self.cmds.push(Cmd::SignIn(text.trim().to_string()));
                }
                KeyCode::Esc => {
                    if let Some(Overlay::Account { pasting, .. }) = &mut self.overlay {
                        *pasting = false;
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                _ => {}
            },
            Overlay::Account { user, ended, pasting, error, busy, .. } => match k.code {
                KeyCode::Enter if !user.is_empty() => {
                    self.cmds.push(Cmd::SignOut);
                    self.overlay = None;
                }
                KeyCode::Enter => {
                    *busy = true;
                    *error = None;
                    self.cmds.push(Cmd::SignInWindow);
                }
                KeyCode::Char('p') if user.is_empty() => *pasting = true,
                KeyCode::Char('d') if *ended => {
                    self.cmds.push(Cmd::StopReminding);
                    self.overlay = None;
                }
                KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
                _ => {}
            },
            Overlay::Join { text, error, busy } => match k.code {
                KeyCode::Enter if nori_core::remote::pairing_valid(text.clone()) => {
                    self.cmds.push(Cmd::Pair(text.clone()));
                    self.overlay = None;
                }
                KeyCode::Enter if nori_core::remote::is_invite(text) => {
                    *busy = true;
                    *error = None;
                    let link = text.trim().to_string();
                    self.cmds.push(Cmd::JoinJam(link));
                }
                KeyCode::Enter => *error = Some(crate::text::NOT_AN_INVITE.into()),
                KeyCode::Esc => self.overlay = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                _ => {}
            },
            Overlay::Menu { lines, folded, sel, item, .. } => match k.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    sel.by(1, lines.len());
                    if matches!(lines.get(sel.at), Some(crate::menu::MenuLine::Heading(_))) {
                        sel.by(1, lines.len());
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    sel.by(-1, lines.len());
                    if matches!(lines.get(sel.at), Some(crate::menu::MenuLine::Heading(_))) {
                        sel.by(-1, lines.len());
                    }
                }
                KeyCode::Enter | KeyCode::Char('l') => match lines.get(sel.at).cloned() {
                    // More opens in its place.
                    Some(crate::menu::MenuLine::More) => {
                        let rest = std::mem::take(folded);
                        lines.splice(sel.at..=sel.at, rest);
                    }
                    Some(crate::menu::MenuLine::Act(act)) => {
                        let item = (**item).clone();
                        self.menu_act(act, item);
                    }
                    _ => {}
                },
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Char('m') => self.overlay = None,
                _ => {}
            },
            Overlay::NewPlaylist { text, songs } => match k.code {
                KeyCode::Enter if !text.trim().is_empty() => {
                    let (name, songs) = (text.trim().to_string(), std::mem::take(songs));
                    self.overlay = None;
                    self.cmds.push(Cmd::NewPlaylist { name, songs });
                }
                KeyCode::Esc => self.overlay = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                _ => {}
            },
            Overlay::Confirm { cmd, .. } => match k.code {
                KeyCode::Enter => {
                    self.cmds.push((**cmd).clone());
                    self.overlay = None;
                }
                KeyCode::Esc => self.overlay = None,
                _ => self.dirty = false,
            },
            Overlay::Input { text, target, .. } => match k.code {
                KeyCode::Enter => {
                    let (target, typed) = (target.clone(), text.clone());
                    self.overlay = None;
                    match target {
                        // A repo that isn't nori's is added only once the user says they trust it.
                        Target::AddRepo if !typed.trim().is_empty() => self.overlay = Some(crate::settings_view::trust_repo(typed.trim())),
                        Target::AddRepo => {}
                        t => self.cmds.extend(t.cmd(typed)),
                    }
                }
                KeyCode::Esc => self.overlay = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                _ => {}
            },
        }
    }

    pub fn do_action(&mut self, a: Action) {
        let now = Instant::now();
        match a {
            Action::Quit => {
                self.quit = true;
                self.cmds.push(Cmd::Quit);
            }
            Action::Help => self.overlay = Some(Overlay::Help { scroll: 0 }),
            Action::Join => self.join(),
            // A jam guest's keys reach what its role does (Spotify's Jam): an admin's the host's playback, a
            // guest's play and pause its own listening; shuffle, repeat and removals are the host's.
            Action::TogglePlay if !self.offers(|c| c.play_pause) => self.dirty = false,
            Action::Next | Action::Previous if !self.offers(|c| c.skip) => self.dirty = false,
            Action::SeekBack | Action::SeekForward | Action::SeekBackLong | Action::SeekForwardLong if !self.offers(|c| c.seek) => self.dirty = false,
            Action::Shuffle | Action::Repeat if self.guest() => self.dirty = false,
            Action::MoveUp | Action::MoveDown if self.queue_in_focus() && !self.offers(|c| c.reorder) => self.dirty = false,
            Action::Remove | Action::Undo if self.guest() && self.queue_in_focus() => self.dirty = false,
            Action::TogglePlay => {
                self.pressed(Button::Toggle);
                self.cmds.push(Cmd::Toggle);
            }
            Action::Next => {
                self.pressed(Button::Next);
                self.cmds.push(Cmd::Next);
            }
            Action::Previous => {
                self.pressed(Button::Previous);
                self.cmds.push(Cmd::Previous);
            }
            Action::SeekBack => self.seek_by(-5_000, now),
            Action::SeekForward => self.seek_by(5_000, now),
            Action::SeekBackLong => self.seek_by(-30_000, now),
            Action::SeekForwardLong => self.seek_by(30_000, now),
            Action::VolumeUp => self.set_volume(self.volume + 0.05),
            Action::VolumeDown => self.set_volume(self.volume - 0.05),
            Action::Search => {
                self.go(View::Search);
                self.search.editing = true;
            }
            Action::Curve => self.cmds.push(Cmd::Curve),
            Action::Mouse => {
                self.mouse = !self.mouse;
                self.cmds.push(Cmd::Mouse(self.mouse));
                self.say(if self.mouse { "Mouse on" } else { "Mouse off: the terminal selects text" }, false);
            }
            Action::Images => {
                self.images = !self.images;
                self.retheme();
                self.cmds.push(Cmd::Images(self.images));
            }
            Action::Shuffle => {
                self.pressed(Button::Shuffle);
                let on = !self.queue_shuffled();
                self.cmds.push(Cmd::Shuffle(on));
                self.say(if on { "Shuffle on" } else { "Shuffle off" }, false);
            }
            Action::Repeat => {
                self.pressed(Button::Repeat);
                self.cmds.push(Cmd::CycleRepeat);
            }
            Action::Go(n) => {
                if let Some(v) = GO.get(n as usize).filter(|v| self.reaches(**v)) {
                    self.go(*v);
                }
            }
            Action::Panel(p) => self.set_panel(p),
            Action::Full => self.toggle_full(),
            Action::Sort => self.sort_next(),
            Action::Favorites => self.favorites_toggle(),
            Action::Radio => {
                if let Some(s) = self.song.as_ref().filter(|_| self.rules.radio) {
                    self.cmds.push(Cmd::PlayFetch(Fetch::Radio(s.id.clone()), false));
                }
            }
            Action::NextPane | Action::PreviousPane => self.cycle(a == Action::NextPane),
            Action::Back => self.back(),
            Action::Refresh => self.refresh(),
            Action::Sooner | Action::Later | Action::Unnudge => {
                if let Some(l) = &self.lyrics {
                    let dir = match a {
                        Action::Sooner => 1,
                        Action::Later => -1,
                        _ => 0,
                    };
                    let ms = l.clock.nudge(dir);
                    self.lyrics_wake = Some(now);
                    self.say(if ms == 0 { "Lyrics on their own timing".to_string() } else { format!("Lyrics {}", crate::text::nudge(ms)) }, false);
                }
            }
            Action::GroupBack | Action::GroupOn if self.view == View::Settings && self.page().is_none() => {
                let g = self.settings.group_at() as isize + if a == Action::GroupOn { 1 } else { -1 };
                let last = crate::settings_view::GROUPS.len() as isize - 1;
                self.settings.jump(g.clamp(0, last) as usize);
            }
            Action::GroupBack | Action::GroupOn => {
                // The library's section before or after, from one of them.
                let sections = self.sections();
                match sections.iter().position(|v| *v == self.view).filter(|_| self.page().is_none() && !self.full) {
                    Some(at) => {
                        let to = if a == Action::GroupOn { (at + 1).min(sections.len() - 1) } else { at.saturating_sub(1) };
                        self.go(sections[to]);
                    }
                    None => self.dirty = false,
                }
            }
            Action::Key => match self.settings.key().filter(|_| self.view == View::Settings) {
                Some(o) => self.overlay = Some(o),
                None => self.dirty = false,
            },
            Action::Decrease | Action::Increase => {
                let up = a == Action::Increase;
                if let Some(row) = self.plugin_device_row() {
                    return self.cmds.extend(crate::settings_view::step_row(row, up));
                }
                match self.view {
                    View::Equalizer => self.eq_step(up),
                    View::Settings => {
                        let cmds = self.settings.step(up);
                        self.cmds.extend(cmds);
                    }
                    _ => {}
                }
            }
            Action::Remove if self.focus == Focus::Main && self.view == View::Equalizer => {
                if let Some(crate::settings_view::EqRow::Band(i)) = crate::settings_view::eq_rows(&self.prefs).get(self.eq_sel.at) {
                    self.cmds.push(Cmd::Sound(SoundToolCmd::RemoveBand(*i as u32)));
                }
            }
            Action::FreqDown | Action::FreqUp | Action::Narrower | Action::Wider | Action::BandKind | Action::BandSide => {
                let rows = crate::settings_view::eq_rows(&self.prefs);
                let cmd = rows.get(self.eq_sel.at).filter(|_| self.view == View::Equalizer).and_then(|row| match a {
                    Action::FreqDown | Action::FreqUp => row.shift(&self.prefs, a == Action::FreqUp),
                    Action::Narrower | Action::Wider => row.width(&self.prefs, a == Action::Narrower),
                    _ => row.cycle(&self.prefs, a == Action::BandSide),
                });
                match cmd {
                    Some(c) => self.cmds.push(c),
                    None => self.dirty = false,
                }
            }
            Action::Remove if self.focus == Focus::Main && self.view == View::Settings => match self.settings.plugin_key(a) {
                Some(o) => self.overlay = Some(o),
                None => self.dirty = false,
            },
            Action::Remove if self.focus == Focus::Main && self.view == View::Downloads => {
                if let Some(Item::Song(songs, i)) = self.selected() {
                    self.cmds.push(Cmd::DownloadRemove(songs[i].id.clone()));
                }
            }
            Action::Remove if self.queue_in_focus() => match self.queue_rows().get(self.queue_sel.at).copied() {
                // The song playing stays: the rows a remove leaves are the core's (`stage::QueueRows::kept`).
                Some(QueueRow::Song(i)) if self.queue.as_ref().is_some_and(|q| nori_core::stage::mirrored_queue_rows(q.len, false, q.index).kept.contains(&(i as u32))) => {
                    self.dirty = false
                }
                Some(QueueRow::Song(i)) => {
                    // Another device's queue has its own undo.
                    self.taken = self.queue.as_ref().filter(|_| self.devices.active.is_none()).and_then(|q| q.songs.get(i)).map(|s| s.id.clone());
                    self.cmds.push(Cmd::Remove(i));
                }
                Some(row) => match row.ask().and_then(|k| self.ask_request(k)) {
                    Some(request) => self.cmds.push(Cmd::JamDecide(request, false)),
                    None => self.dirty = false,
                },
                None => self.dirty = false,
            },
            Action::Remove if self.home_line() == Some(HomeLine::Offer) => self.take_offer(false),
            Action::Remove => self.dirty = false,
            Action::Jam => {
                let item = if self.focus == Focus::Main { self.selected() } else { None };
                self.jam_with(item);
            }
            Action::Menu => self.open_menu(),
            Action::Undo if self.queue_in_focus() => match self.taken.take() {
                Some(id) => self.cmds.push(Cmd::Restore(id)),
                None => self.say("Nothing to put back", false),
            },
            Action::Undo => self.dirty = false,
            Action::MoveUp | Action::MoveDown if self.view == View::Settings && !self.queue_in_focus() => match self.settings.plugin_move(a == Action::MoveUp) {
                Some(c) => self.cmds.push(c),
                None => self.dirty = false,
            },
            Action::MoveUp | Action::MoveDown if !self.queue_in_focus() => self.dirty = false,
            Action::MoveUp | Action::MoveDown => self.queue_move(a == Action::MoveDown),
            _ => self.list_action(a),
        }
    }

    /// Shows `b` reversed for [`FLASH`]: the press feedback of a click or a key (none with reduced motion).
    fn pressed(&mut self, b: Button) {
        if !self.reduced_motion {
            self.flash = Some((b, Instant::now()));
        }
    }

    fn seek_by(&mut self, delta: i64, now: Instant) {
        let Some(song) = &self.song else { return };
        let len = song.duration as i64 * 1000;
        let to = (self.now.position(now) + delta).clamp(0, (len - 1000).max(0));
        self.sought(to);
    }

    /// Seeks to `to`, shown at once (the engine's status says it from the seek on).
    fn sought(&mut self, to: i64) {
        self.now.position_ms = to;
        self.now.at = Instant::now();
        self.cmds.push(Cmd::Seek(to));
    }

    /// Takes the engine's status.
    pub fn follow_now(&mut self, n: Now) {
        self.now = n;
    }

    /// The session closed: nothing plays any more, and signing in is what is left.
    pub fn session_ended(&mut self) {
        (self.now, self.lyrics_wake) = (Now::default(), None);
        self.go(View::Login);
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = (v * 20.0).round().clamp(0.0, 20.0) / 20.0;
        self.cmds.push(Cmd::Volume(self.volume));
    }

    pub fn queue_shuffled(&self) -> bool {
        self.queue.as_ref().is_some_and(|q| q.shuffle)
    }

    pub fn repeat(&self) -> u8 {
        self.queue.as_ref().map_or(0, |q| q.repeat)
    }

    /// Back: out of the full player, then out of a page.
    fn back(&mut self) {
        if self.full {
            if self.lyrics_sel.is_some() && self.panel == Panel::Lyrics {
                self.lyrics_sel = None;
            } else {
                self.close_full();
            }
            return;
        }
        if self.view == View::Settings && self.settings.alone {
            self.settings.alone = false;
            self.settings.invalidate();
            self.view = View::Login;
            return;
        }
        if self.pages.pop().is_none() {
            self.dirty = false;
        }
    }

    fn refresh(&mut self) {
        if self.lyrics_focused() {
            self.lyrics_for = None;
            return self.want_lyrics();
        }
        if let Some(p) = self.page() {
            self.cmds.extend(p.req().map(Cmd::Load));
            return;
        }
        let l = &mut self.library;
        match self.view {
            View::Home => self.home.asked = false,
            View::Albums => l.albums = Load::Idle,
            View::Artists => l.artists = Load::Idle,
            View::Songs => l.songs = Load::Idle,
            View::Playlists => l.playlists = Load::Idle,
            View::Favorites => l.favorites = Load::Idle,
            View::History => l.history = Load::Idle,
            View::Settings => self.settings.facts_asked = false,
            _ => {}
        }
        self.go(self.view);
    }

    // ---- lists ----

    /// The focused list's selection and length; None for home shelves and lyrics.
    fn list(&mut self) -> Option<(&mut Sel, usize)> {
        if self.focus == Focus::Panel {
            return match self.panel {
                Panel::Queue => {
                    let len = self.queue_rows().len();
                    Some((&mut self.queue_sel, len))
                }
                Panel::Devices => {
                    let len = self.devices.rows().len();
                    Some((&mut self.devices.sel, len))
                }
                Panel::Lyrics | Panel::Playing => None,
            };
        }
        if self.page().is_some() {
            let p = self.page_mut()?;
            let len = p.len();
            return Some((p.sel(), len));
        }
        let lines = self.library_lines().len();
        let l = &self.library;
        let (favorites, history) = (l.favorites.ready().map_or(0, Vec::len), l.history.ready().map_or(0, Vec::len));
        Some(match self.view {
            View::Albums => (&mut self.library.albums_sel, lines),
            View::Artists => (&mut self.library.artists_sel, lines),
            View::Songs => (&mut self.library.songs_sel, lines),
            View::Playlists => (&mut self.library.playlists_sel, lines),
            View::Favorites => (&mut self.library.favorites_sel, favorites),
            View::History => (&mut self.library.history_sel, history),
            View::Search => {
                let len = self.search.rows().len();
                (&mut self.search.sel, len)
            }
            View::Downloads => {
                let len = self.downloads.ready().map_or(0, |d| d.rows().len());
                (&mut self.downloads_sel, len)
            }
            View::Equalizer => (&mut self.eq_sel, crate::settings_view::eq_rows(&self.prefs).len()),
            View::Settings => {
                self.settings.pages(&self.prefs);
                return Some(self.settings.list());
            }
            View::Home => {
                let len = self.home.lines().len();
                (&mut self.home.sel, len)
            }
            View::Login => return None,
        })
    }

    /// The plugin's row selected in the devices panel, while it has the keys.
    fn plugin_device_row(&self) -> Option<crate::settings_view::Row> {
        if self.focus != Focus::Panel || self.panel != Panel::Devices {
            return None;
        }
        match self.devices.rows().get(self.devices.sel.at) {
            Some(DeviceRow::Plugin(c, r)) => self.devices.plugin_row(*c, *r),
            _ => None,
        }
    }

    fn list_action(&mut self, a: Action) {
        if self.lyrics_focused() {
            return self.lyrics_action(a);
        }
        let Some((sel, len)) = self.list() else { return };
        let page = 10;
        match a {
            Action::Up | Action::Left => sel.by(-1, len),
            Action::Down | Action::Right => sel.by(1, len),
            Action::Top => sel.to(0, len),
            Action::Bottom => sel.to(len.saturating_sub(1), len),
            Action::PageUp => sel.by(-page, len),
            Action::PageDown => sel.by(page, len),
            _ => return self.act_on_selected(a),
        }
        self.settle(!matches!(a, Action::Up | Action::Left | Action::PageUp | Action::Top));
        self.more();
    }

    /// Moves the focused list's selection off a line that is not a row, in direction `down`.
    fn settle(&mut self, down: bool) {
        if self.focus == Focus::Panel && self.panel == Panel::Devices {
            self.devices.settle(down);
        }
        if self.focus == Focus::Main && self.page().is_none() {
            match self.view {
                View::Settings => self.settings.skip_titles(down),
                View::Search => self.search.settle(down),
                View::Home => self.home.settle(down),
                View::Albums | View::Artists | View::Songs | View::Playlists if self.library_line() == Some(LibLine::Gap) => {
                    let len = self.library_lines().len();
                    if let Some((sel, _)) = self.list() {
                        sel.by(if down { 1 } else { -1 }, len);
                    }
                }
                _ => {}
            }
        }
    }

    /// Requests the next page when the selection nears the end of a paged list.
    fn more(&mut self) {
        if self.focus != Focus::Main {
            return;
        }
        // A Home row in full, a page at a time.
        if let Some(Page::Row { items, row: Some(row), more: more @ true, sel, .. }) = self.pages.last_mut() {
            if sel.at + 30 >= items.len() {
                *more = false;
                let (row, offset) = (*row, items.len() as u32);
                self.cmds.push(Cmd::Load(Req::Row { row, offset }));
            }
            return;
        }
        if self.page().is_some() {
            return;
        }
        let l = &mut self.library;
        match self.view {
            View::Albums => {
                let len = l.albums.ready().map_or(0, Vec::len);
                if l.albums_sel.at + 50 >= len && l.albums_more {
                    l.albums_more = false;
                    self.cmds.push(Cmd::Load(Req::Albums { sort: l.album_sort, favorites: l.albums_favorites, offset: len as u32 }));
                }
            }
            View::Songs => {
                let len = l.songs.ready().map_or(0, Vec::len);
                if l.songs_sel.at + 50 >= len && l.songs_more {
                    l.songs_more = false;
                    self.cmds.push(Cmd::Load(Req::Songs { sort: l.song_sort.clone(), starred: l.songs_favorites, offset: len as u32 }));
                }
            }
            View::History => {
                let len = l.history.ready().map_or(0, Vec::len);
                if l.history_sel.at + 30 >= len {
                    if let Some(after) = l.history_next.take() {
                        self.cmds.push(Cmd::Load(Req::History(Some(after))));
                    }
                }
            }
            _ => {}
        }
    }

    /// The selected item in the focused part.
    pub fn selected(&self) -> Option<Item> {
        if self.focus == Focus::Panel {
            let q = self.queue.as_ref()?;
            let i = match self.panel {
                Panel::Queue => self.queue_selected_index()?,
                Panel::Lyrics | Panel::Devices | Panel::Playing => return None,
            };
            return q.songs.get(i).cloned().map(|s| Item::Song(vec![s], 0));
        }
        if let Some(p) = self.page() {
            return match p {
                Page::Album { sel, .. } | Page::DownloadedAlbum { sel, .. } | Page::Playlist { sel, .. } | Page::Mix { sel, .. } => p.songs().filter(|s| sel.at < s.len()).map(|s| Item::Song(s.to_vec(), sel.at)),
                Page::Artist { sel, .. } if p.songs().is_some() => p.songs().filter(|s| sel.at < s.len()).map(|s| Item::Song(s.to_vec(), sel.at)),
                Page::Artist { detail, sel, .. } => match artist_lines(detail.ready()?).get(sel.at)? {
                    ArtistLine::Album(i) => detail.ready()?.albums.get(*i).cloned().map(Item::Album),
                    ArtistLine::Shelf(_) => None,
                },
                Page::Row { items, sel, .. } => items.get(sel.at).cloned(),
            };
        }
        let l = &self.library;
        // An artist's shelf stands for the artist, as its line opens their page.
        let shelf = |start: usize| {
            let a = l.albums.ready()?.get(start)?;
            Some(Item::Artist(Artist { id: a.artist_id.clone()?, name: a.artist.clone(), ..Default::default() }))
        };
        match self.view {
            View::Home => self.home.item(),
            View::Albums | View::Artists | View::Songs | View::Playlists => match self.library_line()? {
                LibLine::Shelf(start) => shelf(start),
                LibLine::Entry(i) => match self.view {
                    View::Albums => l.albums.ready()?.get(i).cloned().map(Item::Album),
                    View::Artists => l.artists.ready()?.get(i).cloned().map(Item::Artist),
                    View::Songs => l.songs.ready().filter(|s| i < s.len()).map(|s| Item::Song(s.clone(), i)),
                    _ => l.playlists.ready()?.get(i).cloned().map(Item::Playlist),
                },
                LibLine::Place(_) | LibLine::Gap => None,
            },
            View::Favorites => l.favorites.ready().filter(|s| l.favorites_sel.at < s.len()).map(|s| Item::Song(s.clone(), l.favorites_sel.at)),
            View::History => l.history.ready().filter(|h| l.history_sel.at < h.len()).map(|h| Item::Song(h.iter().map(|e| e.song.clone()).collect(), l.history_sel.at)),
            View::Search => match self.search.rows().get(self.search.sel.at)? {
                // Search songs play alone: the list is no page's queue and may hold provider songs.
                SearchRow::Song(s) => Some(Item::Song(vec![(*s).clone()], 0)),
                SearchRow::Album(a) => Some(Item::Album((*a).clone())),
                SearchRow::Artist(a) => Some(Item::Artist((*a).clone())),
                SearchRow::Playlist(p) => Some(Item::Playlist((*p).clone())),
                SearchRow::Title(..) => None,
            },
            View::Downloads => {
                let rows = self.downloads.ready()?.rows();
                match rows.get(self.downloads_sel.at)? {
                    DownloadRow::Album(a) => Some(Item::DownloadedAlbum((*a).clone())),
                    DownloadRow::Song(songs, i) => Some(Item::Song(songs.to_vec(), *i)),
                    DownloadRow::Header(_) => None,
                }
            }
            _ => None,
        }
    }

    fn act_on_selected(&mut self, a: Action) {
        match self.focus {
            Focus::Panel if a == Action::Open => {
                let i = match self.panel {
                    Panel::Queue => match self.queue_rows().get(self.queue_sel.at).copied() {
                        Some(QueueRow::Song(_)) if self.guest() => None,
                        Some(QueueRow::Song(i)) => Some(i),
                        Some(QueueRow::Jam | QueueRow::Listeners) => return self.invite(),
                        Some(QueueRow::Listen | QueueRow::AlongNote(_)) => return self.cmds.push(Cmd::Listen(self.devices.listening() == Listening::Watching)),
                        Some(QueueRow::End) if self.guest() => return self.cmds.push(Cmd::LeaveJam),
                        Some(QueueRow::End) => return self.cmds.push(Cmd::JamEnd),
                        Some(row) => {
                            if let Some(request) = row.ask().and_then(|k| self.ask_request(k)) {
                                self.cmds.push(Cmd::JamDecide(request, true));
                            }
                            return;
                        }
                        None => None,
                    },
                    Panel::Devices => return self.device_open(),
                    _ => None,
                };
                if let Some(i) = i {
                    self.cmds.push(Cmd::Jump(i));
                }
                return;
            }
            Focus::Main if self.page().is_none() && self.view == View::Settings => {
                if a == Action::Open {
                    self.settings_open();
                }
                return;
            }
            Focus::Main if self.page().is_none() && self.view == View::Equalizer => {
                if a == Action::Open {
                    self.eq_open();
                }
                return;
            }
            _ => {}
        }
        if matches!(a, Action::PlayAll | Action::ShuffleAll) {
            return self.play_all(a == Action::ShuffleAll);
        }
        match (a, self.home_line()) {
            (Action::Open, Some(HomeLine::Shelf(at))) => return self.open_shelf(at),
            (Action::Open, Some(HomeLine::Offer)) => return self.take_offer(true),
            _ => {}
        }
        if let (Action::Open, Some(LibLine::Place(place))) = (a, self.library_line().filter(|_| self.focus == Focus::Main && self.page().is_none())) {
            return match place {
                Place::Favorites => self.go(View::Favorites),
                Place::Downloads => self.go(View::Downloads),
                Place::History => self.go(View::History),
                Place::Shuffle => self.cmds.push(Cmd::ShuffleLibrary { albums: self.view == View::Albums }),
            };
        }
        let Some(item) = self.selected() else { return };
        match (a, item) {
            (Action::Open, Item::Song(songs, i)) => self.tap(songs, i),
            (Action::Open, Item::Album(al)) => self.open_album(al.id),
            (Action::Open, Item::DownloadedAlbum(a)) => self.open_page(Page::DownloadedAlbum { id: a.id, detail: Load::Loading, sel: Sel::default() }),
            (Action::Open, Item::Artist(ar)) => self.open_artist(ar.id),
            (Action::Open, Item::Playlist(p)) => self.open_playlist(p.id),
            (Action::Open, Item::Mix(m)) => self.open_page(Page::Mix { id: m.id, detail: Load::Loading, sel: Sel::default() }),
            (Action::Enqueue | Action::PlayNext, item) => {
                let next = a == Action::PlayNext;
                match item {
                    Item::Song(songs, i) => self.cmds.push(Cmd::Enqueue(vec![songs[i].clone()], next)),
                    Item::Album(al) => self.cmds.push(Cmd::EnqueueFetch(Fetch::Album(al.id), next)),
                    Item::DownloadedAlbum(a) => self.cmds.push(Cmd::EnqueueFetch(Fetch::DownloadedAlbum(a.id), next)),
                    Item::Artist(ar) => self.cmds.push(Cmd::EnqueueFetch(Fetch::Artist(ar.id), next)),
                    Item::Playlist(p) => self.cmds.push(Cmd::EnqueueFetch(Fetch::Playlist(p.id), next)),
                    Item::Mix(m) => self.cmds.push(Cmd::EnqueueFetch(Fetch::Mix(m.id), next)),
                }
            }
            // Hearts are where songs carry them, downloads the account's.
            (Action::Star, _) if !self.rules.stars && self.rules.stars_after_sign_in => self.cmds.push(Cmd::OfferSignIn),
            (Action::Star, _) if !self.rules.stars => self.dirty = false,
            (Action::Download, _) if !self.rules.downloads => self.dirty = false,
            (Action::Download, item) => match item {
                Item::Song(songs, i) => self.cmds.push(Cmd::Download(vec![songs[i].clone()])),
                Item::Album(al) => self.cmds.push(Cmd::DownloadFetch(Fetch::Album(al.id))),
                Item::DownloadedAlbum(_) => {},
                Item::Artist(ar) => self.cmds.push(Cmd::DownloadFetch(Fetch::Artist(ar.id))),
                Item::Playlist(p) => self.cmds.push(Cmd::DownloadFetch(Fetch::Playlist(p.id))),
                Item::Mix(m) => self.cmds.push(Cmd::DownloadFetch(Fetch::Mix(m.id))),
            },
            (Action::Star, item) => {
                let (kind, id, listed) = match item {
                    Item::Song(songs, i) => (Starrable::Song, songs[i].id.clone(), songs[i].starred),
                    Item::Album(al) => (Starrable::Album, al.id, al.starred),
                    Item::DownloadedAlbum(a) => (Starrable::Album, a.id, a.starred),
                    Item::Artist(ar) => (Starrable::Artist, ar.id, ar.starred),
                    Item::Playlist(_) | Item::Mix(_) => return,
                };
                self.star(kind, id, listed);
            }
            _ => self.dirty = false,
        }
    }

    /// The open section's next order, kept for the next time: the albums' (`browse::album_sorts`), or the
    /// songs' (`browse::song_sorts`).
    fn sort_next(&mut self) {
        if self.page().is_some() || self.full {
            return self.dirty = false;
        }
        let l = &mut self.library;
        match self.view {
            View::Albums => {
                let all = nori_core::browse::album_sorts();
                let at = all.iter().position(|s| *s == l.album_sort).map_or(0, |i| (i + 1) % all.len());
                l.album_sort = all[at];
                self.cmds.push(Cmd::Keep(nori_core::browse::album_sort_kept(l.album_sort)));
                self.reload_list();
            }
            View::Songs => {
                let all = nori_core::browse::song_sorts();
                let at = all.iter().position(|s| s.name == l.song_sort).map_or(0, |i| (i + 1) % all.len());
                l.song_sort = all[at].name.clone();
                self.cmds.push(Cmd::Keep(nori_core::browse::song_sort_kept(l.song_sort.clone())));
                self.reload_list();
            }
            _ => self.dirty = false,
        }
    }

    /// The open section (albums or songs) listing only the favorites, or everything again, in the same order.
    fn favorites_toggle(&mut self) {
        if self.page().is_some() || self.full || !self.rules.stars {
            return self.dirty = false;
        }
        let l = &mut self.library;
        match self.view {
            View::Albums => l.albums_favorites = !l.albums_favorites,
            View::Songs => l.songs_favorites = !l.songs_favorites,
            _ => return self.dirty = false,
        }
        self.reload_list();
    }

    /// The open albums or songs read again from the top, in their order and filter.
    fn reload_list(&mut self) {
        let l = &mut self.library;
        match self.view {
            View::Albums => {
                (l.albums, l.album_shelves, l.albums_sel) = (Load::Loading, Vec::new(), Sel::default());
                self.cmds.push(Cmd::Load(Req::Albums { sort: l.album_sort, favorites: l.albums_favorites, offset: 0 }));
            }
            View::Songs => {
                (l.songs, l.songs_sel) = (Load::Loading, Sel::default());
                self.cmds.push(Cmd::Load(Req::Songs { sort: l.song_sort.clone(), starred: l.songs_favorites, offset: 0 }));
            }
            _ => {}
        }
    }

    /// Plays the queue offered where it was left (`take`), or turns it down; either way it is not offered
    /// again (`queue_offer_done`).
    fn take_offer(&mut self, take: bool) {
        if let Some(offer) = self.home.offer.take() {
            self.cmds.push(if take { Cmd::TakeOffer(offer) } else { Cmd::DismissOffer(offer) });
            self.home.settle(true);
        }
    }

    /// Home's line selected, while Home has the keys.
    fn home_line(&self) -> Option<HomeLine> {
        (self.focus == Focus::Main && self.page().is_none() && self.view == View::Home).then(|| self.home.line()).flatten()
    }

    /// All of Home's shelf `i`, as a page.
    fn open_shelf(&mut self, at: ShelfAt) {
        let Some((title, items)) = self.home.shelf(at) else { return };
        let row = match at {
            ShelfAt::Row(i) => self.home.rows.get(i).and_then(|r| r.as_ref()).and_then(|s| s.row),
            ShelfAt::ForYou => None,
        };
        self.open_page(Page::Row { title, items, row, more: false, sel: Sel::default() });
    }

    /// A song picked from a list, per the `tap_action` setting (not offered here; default PlayList).
    fn tap(&mut self, songs: Vec<Song>, i: usize) {
        match self.prefs.tap_action {
            TapAction::PlayOne => self.cmds.push(Cmd::Play { songs: vec![songs[i].clone()], start: 0, shuffle: false, from: None }),
            TapAction::Queue => self.cmds.push(Cmd::Enqueue(vec![songs[i].clone()], false)),
            TapAction::PlayNext => self.cmds.push(Cmd::Enqueue(vec![songs[i].clone()], true)),
            TapAction::PlayList => {
                let from = self.origin_here();
                self.cmds.push(Cmd::Play { songs, start: i, shuffle: false, from })
            }
        }
    }

    /// The page the focused song list belongs to, as a queue origin.
    fn origin_here(&self) -> Option<PageOrigin> {
        if self.focus != Focus::Main {
            return None;
        }
        if let Some(page) = self.page() {
            return Some(match page {
                Page::Album { id, .. } => PageOrigin::new(OriginKind::Album, id.as_str()),
                Page::DownloadedAlbum { id, .. } => PageOrigin::new(OriginKind::Downloads, id.as_str()),
                Page::Artist { id, .. } => PageOrigin::new(OriginKind::Artist, id.as_str()),
                Page::Playlist { id, .. } => PageOrigin::new(OriginKind::Playlist, id.as_str()),
                Page::Mix { id, .. } => PageOrigin::new(OriginKind::Mix, id.as_str()),
                Page::Row { .. } => return None,
            });
        }
        match self.view {
            View::Songs => Some(PageOrigin::new(OriginKind::Songs, "")),
            View::Search => Some(PageOrigin::new(OriginKind::Search, self.search.text.as_str())),
            View::Downloads => Some(PageOrigin::new(OriginKind::Downloads, "")),
            _ => None,
        }
    }

    /// Plays the page, or shuffled.
    pub(crate) fn play_page(&mut self, shuffle: bool) {
        self.focus = Focus::Main;
        self.play_all(shuffle);
    }

    /// Flips a song's seal.
    pub(crate) fn star_song(&mut self, s: Song) {
        self.star(Starrable::Song, s.id, s.starred);
    }

    fn play_all(&mut self, shuffle: bool) {
        if self.focus == Focus::Main && self.page().is_some() {
            match self.page() {
                Some(Page::Artist { id, .. }) => self.cmds.push(Cmd::PlayFetch(Fetch::Artist(id.clone()), shuffle)),
                Some(p) => {
                    if let Some(songs) = p.songs() {
                        let from = self.origin_here();
                        self.cmds.push(Cmd::Play { songs: songs.to_vec(), start: 0, shuffle, from });
                    }
                }
                None => {}
            }
            return;
        }
        match self.selected() {
            Some(Item::Album(a)) => self.cmds.push(Cmd::PlayFetch(Fetch::Album(a.id), shuffle)),
            Some(Item::DownloadedAlbum(a)) => self.cmds.push(Cmd::PlayFetch(Fetch::DownloadedAlbum(a.id), shuffle)),
            Some(Item::Artist(a)) => self.cmds.push(Cmd::PlayFetch(Fetch::Artist(a.id), shuffle)),
            Some(Item::Playlist(p)) => self.cmds.push(Cmd::PlayFetch(Fetch::Playlist(p.id), shuffle)),
            Some(Item::Mix(m)) => self.cmds.push(Cmd::PlayFetch(Fetch::Mix(m.id), shuffle)),
            Some(Item::Song(songs, _)) => {
                let from = self.origin_here();
                self.cmds.push(Cmd::Play { songs, start: 0, shuffle, from })
            }
            None => {}
        }
    }

    /// Flips an item's heart (`listed`: its record's flag), marked at once; the core's marks follow
    /// ([`Msg::Starred`]).
    fn star(&mut self, kind: Starrable, id: String, listed: bool) {
        let on = !self.starred(kind, &id, listed);
        // A like is stamped on the playing song's seal.
        if on && self.song.as_ref().is_some_and(|s| s.id == id) {
            self.pressed(Button::StarSong);
        }
        match self.devices.hearts.get_mut(&id).filter(|_| kind == Starrable::Song) {
            Some(heart) => *heart = on,
            None => {
                self.marks.mark(kind, id.clone(), on);
            }
        }
        self.cmds.push(Cmd::Star(kind, id, on));
    }

    /// Whether an item shows starred (`listed`: its record's flag): a song in the queue of another
    /// device playing as that device shows it, else as this session marked it.
    pub fn starred(&self, kind: Starrable, id: &str, listed: bool) -> bool {
        match self.devices.hearts.get(id).filter(|_| kind == Starrable::Song) {
            Some(on) => *on,
            None => self.marks.starred(kind, id, listed),
        }
    }

    /// What the page's head offers, in the core's order (`menus::page_actions`); none for a page of a Home
    /// row, and none but downloads' removal for a downloaded album.
    pub fn page_actions(&self) -> Vec<PageAction> {
        let Some(page) = self.page() else { return Vec::new() };
        let kind = match page {
            Page::Album { .. } | Page::DownloadedAlbum { .. } => PageKind::Album,
            Page::Artist { .. } => PageKind::Artist,
            Page::Playlist { .. } => PageKind::Playlist,
            Page::Mix { detail, .. } => PageKind::Mix { refreshable: detail.ready().is_some_and(|m| m.refreshable) },
            Page::Row { .. } => return Vec::new(),
        };
        if matches!(page, Page::Artist { detail: Load::Loading | Load::Idle | Load::Failed(_), .. }) {
            return Vec::new();
        }
        let songs = page.songs().unwrap_or_default();
        let missing = self.page_missing();
        nori_core::menus::page_actions(kind, songs.len() as u32, missing as u32, self.rules.downloads)
    }

    /// How many of the page's songs are not downloaded.
    pub fn page_missing(&self) -> usize {
        let songs = self.page().and_then(Page::songs).unwrap_or_default();
        songs.iter().filter(|s| self.downloaded.get(&s.id) != Some(&SongDownload::Done)).count()
    }

    /// Does one of the page's actions.
    pub(crate) fn page_act(&mut self, a: PageAction) {
        let Some(page) = self.page() else { return };
        let songs = page.songs().map(<[Song]>::to_vec).unwrap_or_default();
        let fetch = match page {
            Page::Album { id, .. } => Some(Fetch::Album(id.clone())),
            Page::DownloadedAlbum { id, .. } => Some(Fetch::DownloadedAlbum(id.clone())),
            Page::Artist { id, .. } => Some(Fetch::Artist(id.clone())),
            Page::Playlist { id, .. } => Some(Fetch::Playlist(id.clone())),
            Page::Mix { id, .. } => Some(Fetch::Mix(id.clone())),
            Page::Row { .. } => None,
        };
        match a {
            PageAction::Download { act: DownloadAct::Remove } => {
                self.cmds.extend(songs.iter().map(|s| Cmd::DownloadRemove(s.id.clone())));
            }
            PageAction::Download { act: DownloadAct::Missing } => {
                let missing: Vec<Song> = songs.into_iter().filter(|s| self.downloaded.get(&s.id) != Some(&SongDownload::Done)).collect();
                self.cmds.push(Cmd::Download(missing));
            }
            PageAction::Download { act: DownloadAct::All } | PageAction::DownloadEverything => self.cmds.extend(fetch.map(Cmd::DownloadFetch)),
            PageAction::AddToQueue => self.cmds.extend(fetch.map(|f| Cmd::EnqueueFetch(f, false))),
            PageAction::Refresh => {
                if let Some(Page::Mix { id, .. }) = self.page() {
                    self.cmds.push(Cmd::RedrawMix(id.clone()));
                }
            }
            // The terminal has no smart playlists to edit.
            PageAction::Edit => self.dirty = false,
        }
    }

    /// Keeps the mix shown as a playlist on the account, named by the core (`saved_mix_name`).
    pub(crate) fn save_mix(&mut self) {
        if let Some(Page::Mix { detail: Load::Ready(m), .. }) = self.page() {
            let (title, songs) = (crate::text::mix_name(m.name).to_string(), m.songs.clone());
            self.cmds.push(Cmd::SaveMix { title, songs });
        }
    }

    /// Takes the download state of songs, as their page and menu show it.
    pub fn knows_downloads(&mut self, ids: Vec<String>, states: Vec<SongDownload>) {
        self.downloaded.extend(ids.into_iter().zip(states));
    }

    /// Flips the star on the page's album or artist.
    fn star_page(&mut self) {
        let (kind, id, listed) = match self.page() {
            Some(Page::Album { detail: Load::Ready(d), .. } | Page::DownloadedAlbum { detail: Load::Ready(d), .. }) => (Starrable::Album, d.album.id.clone(), d.album.starred),
            Some(Page::Artist { detail: Load::Ready(d), .. }) => (Starrable::Artist, d.artist.id.clone(), d.artist.starred),
            _ => return,
        };
        self.star(kind, id, listed);
    }

    // ---- the queue ----

    /// Queue list indexes in play order.
    pub fn queue_order(&self) -> Vec<usize> {
        self.queue.as_ref().map_or_else(Vec::new, |q| q.order.iter().map(|&i| i as usize).collect())
    }

    fn queue_in_focus(&self) -> bool {
        self.focus == Focus::Panel && self.panel == Panel::Queue
    }

    fn queue_selected_index(&self) -> Option<usize> {
        if !self.queue_in_focus() {
            return None;
        }
        match self.queue_rows().get(self.queue_sel.at)? {
            QueueRow::Song(i) => Some(*i),
            _ => None,
        }
    }

    /// The queue panel's rows: the hosted jam's header, requests and end, then the songs in play order.
    pub fn queue_rows(&self) -> Vec<QueueRow> {
        let mut rows = Vec::new();
        if let Some(j) = &self.devices.jam {
            rows.push(QueueRow::Jam);
            if !self.devices.listeners().is_empty() {
                rows.push(QueueRow::Listeners);
            }
            let asks = self.devices.asks();
            if !j.hosting {
                rows.push(QueueRow::Listen);
                rows.extend((0..self.along_note().len()).map(QueueRow::AlongNote));
                if !asks.is_empty() {
                    rows.push(QueueRow::Asking);
                }
            }
            for (k, p) in asks.iter().enumerate() {
                rows.push(QueueRow::Ask(k));
                // The host decides, and its server downloads the song.
                if p.provider && j.hosting {
                    rows.extend((0..self.downloads_note().len()).map(|n| QueueRow::Downloads(k, n)));
                }
            }
            rows.push(QueueRow::End);
        }
        rows.extend(self.queue_order().into_iter().map(QueueRow::Song));
        rows
    }

    /// The note under a provider's song asked for, in the lines the queue panel has room for.
    pub fn downloads_note(&self) -> Vec<String> {
        crate::ui::wrap(crate::text::JAM_DOWNLOADS, (self.shown.panel_w as usize).saturating_sub(QUEUE_NOTE_INDENT + 1).max(12))
    }

    /// Why this guest, having asked to listen along, does not, in the lines the queue panel has room for.
    pub fn along_note(&self) -> Vec<String> {
        let note = crate::text::jam_along(self.devices.listening());
        if note.is_empty() {
            return Vec::new();
        }
        crate::ui::wrap(note, (self.shown.panel_w as usize).saturating_sub(QUEUE_NOTE_INDENT + 1).max(12))
    }

    /// The request id of the jam's request `k` ([`Devices::asks`]) while hosting: only the host decides.
    fn ask_request(&self, k: usize) -> Option<u64> {
        self.devices.jam.as_ref().filter(|j| j.hosting)?;
        self.devices.asks().get(k).map(|p| p.request)
    }

    fn queue_move(&mut self, down: bool) {
        if self.queue_shuffled() {
            self.say("Turn shuffle off to move songs", false);
            return;
        }
        let Some(from) = self.queue_selected_index() else { return };
        let len = self.queue.as_ref().map_or(0, |q| q.len as usize);
        let to = if down { from + 1 } else { from.wrapping_sub(1) };
        if to >= len {
            return;
        }
        self.cmds.push(Cmd::Move(from, to));
        self.queue_sel.at = if down { self.queue_sel.at + 1 } else { self.queue_sel.at - 1 };
    }

    // ---- other devices and the jam ----

    /// Enter on a devices panel row: the music moves there, or the jam starts or shows its invite.
    fn device_open(&mut self) {
        let d = &self.devices;
        match d.rows().get(d.sel.at).copied() {
            Some(DeviceRow::Here) => self.cmds.push(Cmd::Pick(None)),
            Some(DeviceRow::Device(i) | DeviceRow::Playing(i)) => {
                if let Some(id) = d.list.get(i).map(|x| x.id.clone()) {
                    self.cmds.push(Cmd::Pick(Some(id)));
                }
            }
            Some(DeviceRow::Jam) if d.jam.is_some() => self.invite(),
            Some(DeviceRow::Jam) => self.jam_start(),
            Some(DeviceRow::Join) => self.join(),
            Some(DeviceRow::Pair) => { if let Some(link) = d.pairing.clone() { self.overlay = Some(Overlay::Invite { link }); } }
            Some(DeviceRow::ResetPairing) => self.cmds.push(Cmd::ResetPairing),
            Some(DeviceRow::Plugin(c, r)) => match self.devices.plugin_row(c, r).and_then(crate::settings_view::open_row) {
                Some(Opened::Cmds(c)) => self.cmds.extend(c),
                Some(Opened::Overlay(o)) => self.overlay = Some(o),
                _ => self.dirty = false,
            },
            Some(DeviceRow::PluginName(_) | DeviceRow::PluginHint(..)) | None => self.dirty = false,
        }
    }

    /// Shows the hosted jam's invite.
    fn invite(&mut self) {
        if let Some(link) = self.devices.jam.as_ref().and_then(|j| j.link.clone()) {
            self.overlay = Some(Overlay::Invite { link });
        }
    }

    /// Asks for an invite link, to join someone's jam.
    fn join(&mut self) {
        self.overlay = Some(Overlay::Join { text: String::new(), error: None, busy: false });
    }

    /// Starts a jam; the one hosted shows its invite instead, and where jams cannot start, why. A guest
    /// starts none.
    fn jam_start(&mut self) {
        if self.guest() {
            self.dirty = false;
        } else if self.devices.jam.is_some() {
            self.invite();
        } else if self.devices.jams {
            self.cmds.push(Cmd::JamStart);
        } else {
            self.say(if self.devices.jams_unsupported { crate::text::JAM_UNSUPPORTED } else { crate::text::JAMS_OFF }, false);
        }
    }

    /// Starts a jam around a song, album, artist, playlist or mix, which plays (a jam alone with none).
    pub(crate) fn jam_with(&mut self, item: Option<Item>) {
        let play = match item {
            Some(Item::Song(songs, i)) => Some(Cmd::Play { songs, start: i, shuffle: false, from: self.origin_here() }),
            Some(Item::Album(a)) => Some(Cmd::PlayFetch(Fetch::Album(a.id), false)),
            Some(Item::DownloadedAlbum(a)) => Some(Cmd::PlayFetch(Fetch::DownloadedAlbum(a.id), false)),
            Some(Item::Artist(a)) => Some(Cmd::PlayFetch(Fetch::Artist(a.id), false)),
            Some(Item::Playlist(p)) => Some(Cmd::PlayFetch(Fetch::Playlist(p.id), false)),
            Some(Item::Mix(m)) => Some(Cmd::PlayFetch(Fetch::Mix(m.id), false)),
            None => None,
        };
        if self.devices.jam.is_none() && self.devices.jams {
            self.cmds.extend(play);
        }
        self.jam_start();
    }

    // ---- lyrics ----

    fn lyrics_action(&mut self, a: Action) {
        let Some(l) = &self.lyrics else { return };
        let len = l.pick.lyrics.lines.len();
        // `active` is past the end after the last line.
        let active = (l.clock.shown().active.max(0) as usize).min(len.saturating_sub(1));
        let at = self.lyrics_sel.unwrap_or(active);
        match a {
            Action::Up => self.lyrics_sel = Some(at.saturating_sub(1)),
            Action::Down => self.lyrics_sel = Some((at + 1).min(len.saturating_sub(1))),
            Action::Top => self.lyrics_sel = Some(0),
            Action::Bottom => self.lyrics_sel = Some(len.saturating_sub(1)),
            Action::Open if l.pick.lyrics.synced => {
                let to = l.clock.tap(at);
                self.lyrics_sel = None;
                self.sought(to);
                self.lyrics_wake = Some(Instant::now());
            }
            _ => {}
        }
    }

    // ---- the settings and the equalizer ----

    fn settings_open(&mut self) {
        match self.settings.open() {
            Some(Opened::Cmds(c)) => self.cmds.extend(c),
            Some(Opened::Overlay(o)) => self.overlay = Some(o),
            Some(Opened::View(v)) => self.go(v),
            Some(Opened::Own(switch)) => self.own_toggle(switch),
            Some(Opened::Login) => {
                self.login = Login::default();
                self.view = View::Login;
            }
            None => self.dirty = false,
        }
    }

    fn own_toggle(&mut self, switch: Switch) {
        match switch {
            Switch::Mouse => self.do_action(Action::Mouse),
            Switch::Images => self.do_action(Action::Images),
            Switch::Transparent => self.cmds.push(Cmd::Paint(!self.paint)),
            Switch::Setting(_) | Switch::Stores(..) | Switch::Plugin { .. } | Switch::Hold(_) => {}
        }
        self.settings.invalidate();
    }

    /// A sound change was stored. Returns true when the engine should now take the low-latency buffer
    /// (first real edit on the equalizer; `rules::sound_tuning`), so later edits are heard
    /// at once. Merely opening the equalizer does not.
    pub fn sound_edited(&mut self) -> bool {
        self.touched |= sound_tuning(self.view == View::Equalizer, true);
        self.tune()
    }

    /// Updates `tuning` from `rules::sound_tuning`; pushes the release command, returns true on take.
    fn tune(&mut self) -> bool {
        let want = sound_tuning(self.view == View::Equalizer, self.touched);
        if want == self.tuning {
            return false;
        }
        self.tuning = want;
        if !want {
            self.cmds.push(Cmd::Tuning(false));
        }
        want
    }

    fn eq_step(&mut self, up: bool) {
        let rows = crate::settings_view::eq_rows(&self.prefs);
        if let Some(row) = rows.get(self.eq_sel.at) {
            if let Some(c) = row.step(&self.prefs, up) {
                self.cmds.push(c);
            }
        }
    }

    fn eq_open(&mut self) {
        let rows = crate::settings_view::eq_rows(&self.prefs);
        match rows.get(self.eq_sel.at) {
            Some(crate::settings_view::EqRow::Presets) => {
                let options = nori_core::dsp::eq_presets().iter().enumerate().map(|(i, p)| (crate::text::preset(p.kind).to_string(), i.to_string())).collect();
                self.overlay = Some(Overlay::Picker { title: "Presets".into(), options, sel: Sel::default(), target: Target::Preset });
            }
            Some(row) => {
                if let Some(c) = row.open(&self.prefs) {
                    self.cmds.push(c);
                }
            }
            None => {}
        }
    }

    // ---- the mouse ----

    fn hit(&self, col: u16, row: u16) -> Option<Hit> {
        // Last drawn wins: overlays are drawn last.
        self.hits.iter().rev().find(|(r, _)| r.contains(Position { x: col, y: row })).map(|(_, h)| *h)
    }

    fn mouse_event(&mut self, m: MouseEvent) {
        if !self.mouse {
            self.dirty = false;
            return;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(h) = self.hit(m.column, m.row) else {
                    self.dirty = false;
                    return;
                };
                let again = self.last_click.is_some_and(|(last, t)| last == h && t.elapsed() < Duration::from_millis(500));
                self.last_click = Some((h, Instant::now()));
                self.click(h, m.column, m.row, again);
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.drag {
                Some(Drag::Seek(_)) => self.drag = Some(Drag::Seek(share(self.seek_rect, m.column))),
                Some(Drag::Volume) => self.set_volume(share(self.volume_rect, m.column)),
                Some(Drag::Band(i)) => self.band_to(i, m.column, m.row),
                None => self.dirty = false,
            },
            MouseEventKind::Up(MouseButton::Left) => match self.drag.take() {
                Some(Drag::Seek(s)) => self.seek_share(s),
                Some(Drag::Volume | Drag::Band(_)) => {}
                None => self.dirty = false,
            },
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
                match self.hit(m.column, m.row) {
                    // The wheel changes an equalizer control, up for more.
                    Some(Hit::Row(ListRef::Eq, i)) => {
                        self.focus = Focus::Main;
                        self.eq_sel.at = i;
                        self.eq_step(d < 0);
                    }
                    Some(Hit::Row(l, _) | Hit::List(l)) => self.scroll(l, d),
                    Some(Hit::Volume) => self.set_volume(self.volume - d as f32 * 0.05),
                    _ => self.dirty = false,
                }
            }
            _ => self.dirty = false,
        }
    }

    fn seek_share(&mut self, share: f32) {
        let Some(song) = &self.song else { return };
        let to = (song.duration as f32 * 1000.0 * share) as i64;
        self.sought(to);
    }

    fn scroll(&mut self, l: ListRef, d: isize) {
        match l {
            ListRef::Help => {
                if let Some(Overlay::Help { scroll }) = &mut self.overlay {
                    *scroll = (*scroll as isize + d * 3).max(0) as usize;
                }
            }
            ListRef::Lyrics => {
                let len = self.lyrics.as_ref().map_or(0, |l| l.pick.lyrics.lines.len());
                let at = self.lyrics_sel.unwrap_or_else(|| self.lyrics.as_ref().map_or(0, |l| l.clock.shown().active.max(0) as usize));
                self.lyrics_sel = Some((at as isize + d * 3).clamp(0, len.saturating_sub(1) as isize) as usize);
            }
            ListRef::Picker => {
                if let Some(Overlay::Picker { sel, options, .. }) = &mut self.overlay {
                    sel.by(d * 3, options.len());
                }
            }
            ListRef::Menu => {
                if let Some(Overlay::Menu { sel, lines, .. }) = &mut self.overlay {
                    sel.by(d, lines.len());
                }
            }
            ListRef::Profiles => {}
            _ => {
                self.focus = l.focus();
                if let Some((sel, len)) = self.list() {
                    sel.by(d * 3, len);
                }
                self.settle(d > 0);
                self.more();
            }
        }
    }

    fn click(&mut self, h: Hit, col: u16, row: u16, again: bool) {
        match h {
            Hit::Tab(view) => self.go(view),
            Hit::Seek => self.drag = Some(Drag::Seek(share(self.seek_rect, col))),
            Hit::Volume => {
                self.drag = Some(Drag::Volume);
                self.set_volume(share(self.volume_rect, col));
            }
            Hit::SearchField => {
                if self.view != View::Search {
                    self.go(View::Search);
                }
                self.focus = Focus::Main;
                self.search.editing = true;
            }
            Hit::LoginField(i) => {
                self.login.on_list = false;
                self.login.focus = i;
            }
            Hit::Group(g) => {
                self.focus = Focus::Main;
                self.settings.jump(g);
            }
            Hit::Button(b) => self.button(b),
            Hit::List(l) => {
                if !matches!(l, ListRef::Help | ListRef::Picker | ListRef::Menu | ListRef::Profiles) && !self.full {
                    self.focus = l.focus();
                } else {
                    self.dirty = false;
                }
            }
            Hit::Curve => self.add_band_at(col, row),
            Hit::Row(ListRef::Eq, i) => {
                self.focus = Focus::Main;
                self.eq_sel.at = i;
                let rows = crate::settings_view::eq_rows(&self.prefs);
                match rows.get(i) {
                    Some(r) if r.band(&self.prefs).is_some() => {
                        self.drag = Some(Drag::Band(i));
                    }
                    Some(r) if r.clicks() || again => self.eq_open(),
                    _ => {}
                }
            }
            Hit::Row(l, i) => self.click_row(l, i, again),
        }
    }

    /// Where cell `x`, `y` is on the equalizer's curve: across (0..1) and dB.
    fn on_curve(&self, x: u16, y: u16) -> Option<(f32, f32)> {
        let r = self.eq_plot;
        if r.width < 2 || r.height < 2 {
            return None;
        }
        let across = (x.saturating_sub(r.x).min(r.width - 1) as f32 + 0.5) / r.width as f32;
        let down = (y.saturating_sub(r.y).min(r.height - 1) as f32 + 0.5) / r.height as f32;
        Some((across, (1.0 - 2.0 * down) * nori_core::dsp::CURVE_DB))
    }

    /// Band row `i` dragged to cell `x`, `y` on the curve.
    fn band_to(&mut self, i: usize, x: u16, y: u16) {
        let Some((at, db)) = self.on_curve(x, y) else { return };
        let rows = crate::settings_view::eq_rows(&self.prefs);
        match rows.get(i).and_then(|row| row.place(&self.prefs, at, db)) {
            Some(c) => self.cmds.push(c),
            None => self.dirty = false,
        }
    }

    /// A click on the parametric curve off its points: a band added there, and chosen.
    fn add_band_at(&mut self, x: u16, y: u16) {
        let Some((at, db)) = self.on_curve(x, y).filter(|_| self.prefs.eq_mode == nori_core::settings::EqMode::Parametric && self.prefs.eq_enabled) else {
            self.dirty = false;
            return;
        };
        let r = nori_core::settings::EQ_RANGES.gain;
        self.cmds.push(Cmd::Sound(SoundToolCmd::AddBand(nori_core::dsp::band_freq_at(at), ((db * 2.0).round() / 2.0).clamp(r.min, r.max))));
        // The new band's row: after the bands there are, where Balance is now.
        let rows = crate::settings_view::eq_rows(&self.prefs);
        if let Some(at) = rows.iter().position(|r| matches!(r, crate::settings_view::EqRow::Balance)) {
            self.focus = Focus::Main;
            self.eq_sel.at = at;
        }
    }

    fn button(&mut self, b: Button) {
        match b {
            Button::Previous => self.do_action(Action::Previous),
            Button::Toggle => self.do_action(Action::TogglePlay),
            Button::Next => self.do_action(Action::Next),
            Button::Shuffle => self.do_action(Action::Shuffle),
            Button::Repeat => self.do_action(Action::Repeat),
            Button::Panel(p) => self.set_panel(p),
            Button::Full => self.toggle_full(),
            Button::PlayAll => self.play_all(false),
            Button::ShuffleAll => self.play_all(true),
            Button::Star => self.star_page(),
            Button::Page(a) => self.page_act(a),
            Button::SaveMix => self.save_mix(),
            Button::StarSong => {
                if let Some(s) = self.song.as_ref().filter(|_| self.rules.stars) {
                    let (id, listed) = (s.id.clone(), s.starred);
                    self.star(Starrable::Song, id, listed);
                }
            }
            Button::Help => self.do_action(Action::Help),
            Button::Connect => self.connect(),
            Button::LoginPlugins => self.plugins_alone(),
            Button::Resume => self.take_offer(true),
            Button::Dismiss => self.take_offer(false),
        }
    }

    fn click_row(&mut self, l: ListRef, i: usize, again: bool) {
        match l {
            ListRef::Picker => {
                if let Some(Overlay::Picker { sel, options, .. }) = &mut self.overlay {
                    sel.to(i, options.len());
                }
                self.overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                return;
            }
            ListRef::Menu => {
                if let Some(Overlay::Menu { sel, lines, .. }) = &mut self.overlay {
                    sel.to(i, lines.len());
                }
                self.overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                return;
            }
            ListRef::Lyrics => {
                self.lyrics_sel = Some(i);
                if again {
                    self.lyrics_action(Action::Open);
                }
                return;
            }
            ListRef::Profiles => {
                self.login.on_list = true;
                self.login.sel.at = i;
                if again {
                    self.login_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
                return;
            }
            ListRef::Help => return,
            _ => {}
        }
        self.focus = l.focus();
        if let Some((sel, len)) = self.list() {
            sel.to(i, len);
        }
        // What has a page of its own and switches act on one click; songs on a second.
        let page = matches!(l, ListRef::Page | ListRef::Home) && !matches!(self.selected(), Some(Item::Song(..)));
        let settings_now = l == ListRef::Settings && self.settings.clicks_open();
        if again || l.opens_on_click() || page || settings_now {
            self.act_on_selected(Action::Open);
        }
    }
}

/// `col`'s position across `r`, 0 to 1.
fn share(r: Rect, col: u16) -> f32 {
    if r.width == 0 {
        return 0.0;
    }
    (col.saturating_sub(r.x) as f32 / r.width.saturating_sub(1).max(1) as f32).clamp(0.0, 1.0)
}

impl Downloads {
    /// The next refresh: in a second while downloads run.
    fn refresh_at(&self) -> Option<Instant> {
        (!self.active.is_empty() || !self.queued.is_empty()).then(|| Instant::now() + Duration::from_secs(1))
    }

    /// Downloads page rows.
    pub fn rows(&self) -> Vec<DownloadRow<'_>> {
        let mut out = Vec::new();
        for (title, list) in [("Downloading", &self.active), ("Waiting", &self.queued), ("Failed", &self.failed), ("Songs", &self.stored.songs)] {
            if list.is_empty() {
                continue;
            }
            out.push(DownloadRow::Header(format!("{title} ({})", list.len())));
            for i in 0..list.len() {
                out.push(DownloadRow::Song(list.as_slice(), i));
            }
        }
        if !self.stored.albums.is_empty() {
            out.push(DownloadRow::Header(format!("Albums ({})", self.stored.albums.len())));
            out.extend(self.stored.albums.iter().map(DownloadRow::Album));
        }
        out
    }
}

/// A downloads page row: a heading, album, or song in its list.
pub enum DownloadRow<'a> {
    Header(String),
    Album(&'a Album),
    Song(&'a [Song], usize),
}

/// How long a status note shows.
pub const NOTE_FOR: Duration = Duration::from_secs(4);

/// How long a pressed transport button or a stamped seal shows reversed.
pub const FLASH: Duration = Duration::from_millis(230);

