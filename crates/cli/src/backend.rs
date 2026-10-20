//! The session behind the screen (nori-host's), its reports worded as [`Msg`]s, and the screens' reads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use nori_core::cache_policy::{Page, Read};
use nori_core::race::LyricsPick;
use nori_core::search::SearchView;
use nori_core::settings::SavedServer;
use nori_core::transport::NetError;
use nori_core::{AlbumDetail, ArtistDetail, PlaylistDetail, Song};
use nori_covers::memory::Image;
pub use nori_host::desktop::app;
use nori_host::pages::{HomePart, PageAsk, PageFailed, PageGot, ShelfItems};
pub use nori_host::pages::ALBUM_PAGE;
pub use nori_host::session::{Audio, Chore};
use nori_host::session::{Note, Said};
pub use nori_host::{db_path, Fetch};
use nori_http::Http;
use nori_look::cover::CoverColours;
use nori_output_cpal::CpalOutput;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};

use crate::settings_view::{Facts, Storage};
use crate::text::{count, net_error};

pub use nori_core::transport::block_on;

/// Everything that wakes the event loop.
pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize,
    /// Terminal (or tmux pane) focus gained or lost.
    Focus(bool),
    Engine(nori_engine::Event),
    Data(Req, Result<Data, String>),
    /// A decoded cover by key, with derived colours when asked for.
    Cover { art: String, image: Arc<Image>, colours: Option<Box<CoverColours>> },
    /// One of possibly several lyrics answers for `song`, each better than the last.
    Lyrics { song: String, pick: LyricsPick },
    Search(SearchView),
    /// A status bar message.
    Note { text: String, error: bool },
    /// The login form's result: the profile to keep, or the error.
    LoggedIn(Result<SavedServer, String>),
    /// Result of the reachability check made when a session opens.
    Reachable(Result<(), String>),
    /// Another device set the volume (0 to 1).
    Volume(f32),
    /// The core's star marks moved (a heart pressed here or on another device).
    Starred(nori_core::stars::StarMarks),
    /// The other devices, the one playing or the jam changed: read them again.
    Remote,
    /// Whether the jam asked for opened, or why not.
    Jam(Result<(), String>),
    /// Someone's jam joined (what the guest profile signs in with), or why not.
    Joined(Result<nori_core::remote::JamPass, String>),
    /// This guest left its jam, or the jam ended: what to say.
    Left(String),
    /// A message from the session with this id; dropped once another session is open.
    From(u64, Box<Msg>),
}

impl Msg {
    /// A short description for the debug log, without lyrics, pixels or passwords.
    pub fn brief(&self) -> String {
        match self {
            Msg::Key(k) => format!("key {:?} {:?} {:?}", k.code, k.modifiers, k.kind),
            Msg::Mouse(m) => format!("mouse {:?} at {},{}", m.kind, m.column, m.row),
            Msg::Paste(p) => format!("paste of {} bytes", p.len()),
            Msg::Resize => "resize".into(),
            Msg::Focus(on) => format!("focus {}", if *on { "gained" } else { "lost" }),
            Msg::Engine(e) => format!("engine {e:?}"),
            Msg::Data(req, r) => format!("data {req:?} {}", if r.is_ok() { "ok" } else { "failed" }),
            Msg::Cover { art, image, colours } => format!("cover {art} {}x{}{}", image.width, image.height, if colours.is_some() { " with colours" } else { "" }),
            Msg::Lyrics { song, .. } => format!("lyrics for {song}"),
            Msg::Search(v) => format!("search results for {:?}", v.query),
            Msg::Note { text, error } => format!("note {text:?} error={error}"),
            Msg::LoggedIn(r) => format!("logged in: {}", r.is_ok()),
            Msg::Reachable(r) => format!("reachable: {}", r.is_ok()),
            Msg::Volume(v) => format!("volume {v}"),
            Msg::Starred(_) => "star marks".into(),
            Msg::Remote => "remote".into(),
            Msg::Jam(r) => format!("jam opened: {}", r.is_ok()),
            Msg::Joined(r) => format!("jam joined: {}", r.is_ok()),
            Msg::Left(said) => format!("jam left: {said}"),
            Msg::From(id, m) => format!("session {id}: {}", m.brief()),
        }
    }
}

/// A screen's data request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    Home,
    Albums { offset: u32 },
    Artists,
    Playlists,
    Songs { offset: u32 },
    Album(String),
    DownloadedAlbum(String),
    Artist(String),
    Playlist(String),
    Favorites,
    Downloads,
    Facts,
}

pub enum Data {
    /// Home shelf: index (into the session's `home_album_rows`, or the service's feed), title, albums.
    HomeRow(usize, String, Vec<nori_core::Album>),
    Albums(Vec<nori_core::Album>),
    Artists(Vec<nori_core::Artist>),
    Playlists(Vec<nori_core::Playlist>),
    Songs(Vec<Song>, bool),
    /// The starred songs.
    Favorites(Vec<Song>),
    Album(Box<AlbumDetail>),
    Artist(Box<ArtistDetail>),
    Playlist(Box<PlaylistDetail>),
    Downloads(Box<Downloads>),
    Facts(Box<Facts>),
}

/// The downloads screen's sections.
#[derive(Default)]
pub struct Downloads {
    pub active: Vec<Song>,
    pub queued: Vec<Song>,
    pub failed: Vec<Song>,
    pub stored: nori_core::library::DownloadedLibrary,
}

/// Key prefix of album card covers in [`crate::art::Art`]: `thumb:<cover id>`.
pub const THUMB: &str = "thumb:";

/// Logs `draft` in and keeps it as the active profile (blocking): the core checks it with its provider
/// (a server is tried through its second address, then with legacy auth, which the profile then keeps).
pub fn check_login(http: Arc<Http>, draft: SavedServer) -> Result<SavedServer, String> {
    nori_host::desktop::log_in(http, draft).map_err(|e| match net_error(&e) {
        e if e.is_empty() => "the server did not answer".to_string(),
        e => e,
    })
}

/// The terminal client's own settings, stored in the core's `app_kv`.
pub mod own {
    pub use nori_host::desktop::own::{flag, keep, number, text};
    pub const MOUSE: &str = "tui.mouse";
    pub const IMAGES: &str = "tui.images";
    pub const CARD_COVERS: &str = "tui.cardCovers";
    pub const PAINT: &str = "tui.paint";
    pub const VOLUME: &str = "tui.volume";
    /// Output device name opened at start; empty for the system default.
    pub const DEVICE: &str = "tui.device";
}

pub struct Open<'a> {
    pub data: &'a Path,
    pub http: Arc<Http>,
    pub profile: SavedServer,
    pub device: Option<String>,
    pub images: bool,
    pub offline: bool,
    /// The process's media controls, driven by this session while it is open.
    pub mpris: Option<Arc<nori_mpris::Mpris>>,
    pub tx: Sender<Msg>,
}

/// One open server profile; its messages come tagged with its id.
pub struct Session {
    pub id: u64,
    host: nori_host::session::Session,
    tx: Sender<Msg>,
}

impl std::ops::Deref for Session {
    type Target = nori_host::session::Session;

    fn deref(&self) -> &Self::Target {
        &self.host
    }
}

impl Session {
    pub fn open(o: Open) -> Result<Session, String> {
        static IDS: AtomicU64 = AtomicU64::new(0);
        let id = IDS.fetch_add(1, Ordering::Relaxed);
        let tx = o.tx.clone();
        let out = Arc::new(move |s: Said| {
            let _ = tx.send(Msg::From(id, Box::new(worded(s))));
        });
        let host = nori_host::desktop::open(nori_host::desktop::DesktopOpen {
            data: o.data,
            http: o.http,
            profile: o.profile,
            device: o.device.or_else(|| own::text(own::DEVICE)),
            volume: own::number(own::VOLUME, 1.0),
            covers: o.images.then(|| nori_covers::Config::new(o.data.join("covers"))),
            offline: o.offline,
            mpris: o.mpris,
            kind: nori_core::remote::wire::DeviceKind::Terminal,
            out,
        })?;
        Ok(Session { id, host, tx: o.tx })
    }

    /// Loads a screen's data in the background: the stored copy first, then the server's if different.
    pub fn load(&self, req: Req) {
        let (id, tx, asked) = (self.id, self.tx.clone(), req.clone());
        let send = move |r: Result<Data, String>| {
            let _ = tx.send(Msg::From(id, Box::new(Msg::Data(asked.clone(), r))));
        };
        let ask = match req.clone() {
            Req::Albums { offset } => Some(PageAsk::Albums { sort: nori_core::browse::AlbumSort::ByName, offset }),
            Req::Artists => Some(PageAsk::Artists),
            Req::Playlists => Some(PageAsk::Playlists),
            Req::Songs { offset } => Some(PageAsk::Songs { offset }),
            Req::Album(id) => Some(PageAsk::Album(id)),
            Req::DownloadedAlbum(id) => Some(PageAsk::DownloadedAlbum(id)),
            Req::Artist(id) => Some(PageAsk::Artist(id)),
            Req::Playlist(id) => Some(PageAsk::Playlist(id)),
            Req::Favorites => Some(PageAsk::Favorites),
            Req::Home | Req::Downloads | Req::Facts => None,
        };
        if let Some(ask) = ask {
            return self.host.read_page(ask, move |r| send(r.map(data_of).map_err(|e| page_failed(&e))));
        }
        match req {
            Req::Home => {
                let rows = self.host.home_album_rows();
                self.host.read_home(rows.clone(), false, move |r| match r {
                    // A source's own Home: its feed, as shelves of albums.
                    Ok(HomePart::Feed(feed)) => {
                        for (i, (title, albums)) in nori_core::browse::feed_album_shelves(feed).into_iter().enumerate() {
                            send(Ok(Data::HomeRow(i, title, albums)));
                        }
                    }
                    Ok(HomePart::Shelf { at, items: ShelfItems::Albums(albums) }) => send(Ok(Data::HomeRow(at, crate::text::home_row(rows[at]).into(), albums))),
                    Ok(_) => {}
                    Err(e) => send(Err(unreachable(&e))),
                });
            }
            Req::Downloads => {
                let core = self.core.clone();
                nori_host::spawn("nori-read", move || {
                    let stored = core.downloaded_library();
                    send(stored.and_then(|stored| core.download_sections().map(|s| Data::Downloads(Box::new(Downloads { active: s.active, queued: s.queued, failed: s.failed, stored })))).map_err(|e| e.to_string()));
                });
            }
            Req::Facts => {
                let (core, client, store, db, offline) = (self.core.clone(), self.client.clone(), self.store.clone(), self.db.clone(), self.offline);
                let covers_bytes = self.covers.as_ref().and_then(|l| l.disk().map(|d| d.bytes())).unwrap_or(0);
                nori_host::spawn("nori-read", move || send(Ok(Data::Facts(Box::new(facts(&core, &client, store.as_deref(), &db, covers_bytes, offline))))));
            }
            _ => {}
        }
    }

    /// Requests cover `art` at `size` px, deriving colours on the loader thread when `colours`.
    pub fn cover(&self, art: String, size: u32, colours: bool) -> Option<nori_covers::loader::Ticket> {
        self.cover_as(art.clone(), art, size, colours)
    }

    /// Requests an album card cover, delivered under the key `thumb:<art>`.
    pub fn thumb(&self, art: String, size: u32) -> Option<nori_covers::loader::Ticket> {
        self.cover_as(art.clone(), format!("{THUMB}{art}"), size, false)
    }

    fn cover_as(&self, id: String, art: String, size: u32, colours: bool) -> Option<nori_covers::loader::Ticket> {
        let (tx, me) = (self.tx.clone(), self.id);
        self.host.cover(id, size, colours, move |image, colours| {
            if let Ok(image) = image {
                let _ = tx.send(Msg::From(me, Box::new(Msg::Cover { art, image, colours })));
            }
        })
    }

    pub fn search_server(&self, query: String) {
        self.host.search_server(query, net_error);
    }

    /// Starts hosting a jam on the server's relay; [`Msg::Jam`] says whether it opened.
    pub fn jam_open(&self) {
        let Some(r) = self.remote() else { return };
        let (tx, me) = (self.tx.clone(), self.id);
        nori_host::spawn("nori-jam", move || {
            let opened = block_on(r.jam_open()).map(drop).map_err(|e| net_error(&e));
            let _ = tx.send(Msg::From(me, Box::new(Msg::Jam(opened))));
        });
    }

    /// Leaves the jam this guest is in, at once ([`Msg::Left`]); the relay is told on the way.
    pub fn jam_leave(&self) {
        if let Some(r) = self.remote() {
            r.jam_leave();
        }
        let _ = self.tx.send(Msg::Left(crate::text::JAM_LEFT.into()));
    }
}

/// A session's report as the event loop takes it.
fn worded(s: Said) -> Msg {
    let note = |text: String, error: bool| Msg::Note { text, error };
    match s {
        Said::Remote => Msg::Remote,
        Said::JamEnded { host } => Msg::Left(crate::text::jam_ended(host.as_deref())),
        Said::Starred(marks) => Msg::Starred(marks),
        Said::Volume(v) => Msg::Volume(v),
        Said::Curve(n) => note(crate::text::curve_notice(&n), false),
        Said::Engine(e) => Msg::Engine(e),
        Said::Lyrics { song, pick } => Msg::Lyrics { song, pick },
        Said::Search(v) => Msg::Search(v),
        Said::Reachable(r) => Msg::Reachable(r.map_err(|e| unreachable(&e))),
        Said::Note(n) => match n {
            Note::Queued { next, songs } => note(format!("{}: {}", if next { "Playing next" } else { "Added to the queue" }, count(songs as u64, "song", "songs")), false),
            Note::NothingToPlay => note("Nothing to play".into(), false),
            Note::SongsFailed(e) => note(format!("Could not load the songs: {}", net_error(&e)), true),
            Note::NothingToPutBack => note("Nothing to put back".into(), false),
            Note::Downloading(n) => note(format!("Downloading {n} songs"), false),
            Note::DownloadFailed(e) => note(format!("Could not download: {e}"), true),
            Note::StarFailed(e) => note(format!("Could not change the favorite: {}", net_error(&e)), true),
            Note::Indexing => note("Filling the offline index…".into(), false),
            Note::Indexed(t) => note(format!("Offline index: {} songs, {} albums, {} artists", t.songs, t.albums, t.artists), false),
            Note::IndexStopped(e) => note(format!("The offline index stopped: {}", net_error(&e)), true),
            Note::Forgot(n) => note(format!("Forgot {n} measured songs"), false),
            Note::CurveFailed(e) => note(format!("Could not fetch the AutoEQ curve: {}", net_error(&e)), true),
            Note::Done(chore) => note(
                match chore {
                    Chore::ClearStream => "Cleared the streamed music",
                    Chore::ClearLyrics => "Cleared the lyrics found online",
                    _ => "Cleared the covers",
                }
                .into(),
                false,
            ),
        },
    }
}

/// A page read's answer as a screen's data.
fn data_of(got: PageGot) -> Data {
    match got {
        PageGot::Albums(v) => Data::Albums(v),
        PageGot::Artists(v) => Data::Artists(v),
        PageGot::Playlists(v) => Data::Playlists(v),
        PageGot::Favorites(s) => Data::Favorites(s.songs),
        PageGot::Songs { songs, exhausted } => Data::Songs(songs, exhausted),
        PageGot::Album(d) => Data::Album(d),
        PageGot::Artist(d) => Data::Artist(d),
        PageGot::Playlist(d) => Data::Playlist(d),
        // No screen of the terminal's reads a mix as a page.
        PageGot::Mix(m) => Data::Songs(m.songs, true),
    }
}

/// Why a page has nothing, in words.
fn page_failed(e: &PageFailed) -> String {
    match e {
        PageFailed::Nothing => "The server sent nothing for this page".into(),
        PageFailed::Net(e) => unreachable(e),
        PageFailed::NotDownloaded => "Album is no longer downloaded".into(),
        PageFailed::Stored(e) => e.to_string(),
        PageFailed::EmptyMix => "This mix has nothing in it yet".into(),
    }
}

/// An unreachable server in words.
fn unreachable(e: &NetError) -> String {
    match net_error(e) {
        e if e.is_empty() => "The server did not answer".into(),
        // Already a full sentence.
        e if e.ends_with(['.', '?']) => e,
        e => format!("The server did not answer: {e}"),
    }
}

/// Library, storage and device facts for the settings page.
fn facts(core: &nori_core::Core, client: &nori_core::client::Client, store: Option<&nori_engine::Store>, db: &PathBuf, cover_bytes: u64, offline: bool) -> Facts {
    let index = core.index_size().unwrap_or_default();
    let downloads = core.downloads(true).unwrap_or_default();
    let folders = match (!offline).then(|| block_on(client.read_now(Read::MusicFolders))) {
        Some(Ok(Page::Folders { v })) => v,
        _ => Vec::new(),
    };
    Facts {
        analysed: core.analysis_count().unwrap_or(0),
        indexed: (index.songs, index.albums, index.artists),
        storage: Storage {
            stream: store.map_or(0, |s| s.cache_bytes()) as i64,
            covers: cover_bytes as i64,
            lyrics: core.lyrics_cache_bytes(),
            downloads: downloads.iter().map(|s| s.size as i64).sum(),
            download_songs: downloads.len() as u32,
            database: std::fs::metadata(db).map(|m| m.len() as i64).unwrap_or(0),
        },
        folders,
        devices: CpalOutput::devices(),
        rules: core.rules(),
    }
}
