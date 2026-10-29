//! The open server profile (nori-host's session), its reports worded as [`Msg`]s for the UI thread, and
//! the pages' reads.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use nori_core::cache_policy::{Page, Read};
use nori_core::race::LyricsPick;
use nori_core::remote::wire::DeviceKind;
use nori_core::search::SearchView;
use nori_core::settings::{HomeRow, SavedServer};
use nori_covers::loader::Ticket;
use nori_covers::memory::Image;
use nori_engine::Event;
use nori_host::pages::{HomePart, PageAsk, PageFailed, PageGot};
use nori_host::session::{Chore, Note, Said};
pub use nori_host::desktop::app;
pub use nori_host::Fetch;
use nori_http::Http;
use nori_look::cover::CoverColours;
use nori_output_cpal::CpalOutput;

use crate::words::{net_error, songs};
use crate::AppWindow;

pub use nori_core::transport::block_on;

/// Cover draw size. Large and hero covers also get their page colours derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverSize {
    Card,
    Large,
    Hero,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoverKey {
    pub id: String,
    pub size: CoverSize,
}

/// Messages from other threads to the UI thread.
pub enum Msg {
    Engine(Event),
    Data(Req, Result<Data, String>),
    /// A decoded cover, with page colours for large sizes.
    Cover { key: CoverKey, image: Arc<Image>, colours: Option<Box<CoverColours>> },
    Search(SearchView),
    Lyrics { song: String, pick: LyricsPick },
    Facts(Box<crate::settings::Facts>),
    Note { text: String, error: bool },
    /// A playlist write the server took, and what to say: the playlists are read again.
    Playlist { text: String, change: PlaylistChange },
    /// A note with a button, answered with [`nori_host::session::Handle::curve_answer`].
    Ask { text: String, action: String },
    LoggedIn(Result<SavedServer, String>),
    /// The account's sign-in answered.
    SignedIn(nori_core::providers::SignInResult),
    /// The sign-in window (its process id) came back on a signed-in page, and the core's answer.
    SignInWindowDone(u32, nori_core::providers::SignInResult),
    /// A headphone's AutoEQ curve as preset text (None: AutoEQ has none for it), by the headphone's name.
    AutoEqCurve(String, Result<Option<String>, String>),
    /// The AutoEQ list was fetched again, or could not be.
    AutoEqList(Result<(), String>),
    Reachable(Result<(), String>),
    /// The other devices or the jam changed (remote control, jams).
    Remote,
    /// The jam asked for opened, or why not.
    Jam(Result<(), String>),
    /// Someone's jam joined (what the guest profile signs in with), or why not.
    Joined(Result<nori_core::remote::JamPass, String>),
    /// The jam this guest is in ended (its host's name, if seen).
    JamEnded(Option<String>),
    /// Another device set the volume (0 to 1).
    Volume(f32),
    /// A heart changed, here or on another device.
    Starred,
    /// A link to a song from the server, to copy; None when it made none.
    Shared(Option<String>),
    /// A queue another device or app left on the server, worth offering here, or none.
    Offer(Option<nori_core::library::QueueOffer>),
    /// A plugin installed (its id), or why not.
    PluginInstalled(Result<String, nori_plugins::Refusal>),
    /// A plugin's install sheet, or why it can't be installed.
    PluginPreview(Result<nori_plugins::PluginPreview, nori_plugins::Refusal>),
    /// The plugins' lines in song `song`'s details.
    SongInfo { song: String, groups: Vec<nori_plugins::InfoGroup> },
    /// The plugins' lines under the player's output, asked for song `song`.
    PlayerStatus { song: Option<String>, lines: Vec<String> },
    /// A plugin's still sleeve's address for song `song`, to fetch.
    Artwork { song: String, url: String },
    /// A plugin's still sleeve for song `song`, drawn as its cover in Now Playing.
    Sleeve { song: String, image: Arc<Image> },
    /// What the plugin repos offer.
    PluginOffers(nori_plugins::Offers),
    /// Plugins updated on their own: each one's id and new version.
    PluginsUpdated(Vec<(String, String)>),
    /// The devices plugins control, or what they report, changed.
    DevicesControlled,
    /// A message from the session with this id; dropped once another session is open.
    From(u64, Box<Msg>),
}

/// What a playlist write did, for the pages that show it.
pub enum PlaylistChange {
    Made,
    /// Its songs or its name, by id.
    Changed(String),
    Deleted(String),
}

/// Sends [`Msg`]s to the UI thread: queued in the app's inbox, then the window is woken to drain it.
#[derive(Clone)]
pub struct Tx {
    inbox: mpsc::Sender<Msg>,
    ui: slint::Weak<AppWindow>,
}

impl Tx {
    pub fn new(ui: slint::Weak<AppWindow>) -> (Tx, mpsc::Receiver<Msg>) {
        let (inbox, rx) = mpsc::channel();
        (Tx { inbox, ui }, rx)
    }

    pub fn send(&self, m: Msg) {
        if self.inbox.send(m).is_ok() {
            let _ = self.ui.upgrade_in_event_loop(|ui| ui.invoke_messages_arrived());
        }
    }
}

/// A page read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    /// Home's "For you" and the shelves of these rows.
    Home(Vec<HomeRow>),
    Page(PageAsk),
}

/// A page read's answer.
pub enum Data {
    Home(HomePart),
    Page(PageGot),
}

/// Desktop-only settings, stored as app values in the database.
pub mod own {
    pub use nori_host::desktop::own::{keep, number, text};
    pub const VOLUME: &str = "desktop.volume";
    /// Output device name; empty for the system default.
    pub const DEVICE: &str = "desktop.device";
}

/// Logs `draft` in and keeps it as the active profile (blocking).
pub fn check_login(data: &std::path::Path, http: Arc<Http>, draft: SavedServer) -> Result<SavedServer, String> {
    nori_host::desktop::log_in(data, http, draft).map_err(|e| net_error(&e))
}

/// One open server profile; its messages come tagged with its id.
pub struct Session {
    pub id: u64,
    host: nori_host::session::Session,
    tx: Tx,
}

impl std::ops::Deref for Session {
    type Target = nori_host::session::Session;

    fn deref(&self) -> &Self::Target {
        &self.host
    }
}

impl Session {
    /// Opens the profile without touching the network, so an unreachable server still opens.
    /// `mpris`: the process's media controls, driven by this session while it is open.
    pub fn open(data: &Path, http: Arc<Http>, profile: SavedServer, tx: Tx, mpris: Option<Arc<nori_mpris::Mpris>>) -> Result<Session, String> {
        static IDS: AtomicU64 = AtomicU64::new(0);
        let id = IDS.fetch_add(1, Ordering::Relaxed);
        let to = tx.clone();
        let out = Arc::new(move |s: Said| to.send(Msg::From(id, Box::new(worded(s)))));
        let o = nori_host::desktop::DesktopOpen {
            data,
            http,
            profile,
            device: own::text(own::DEVICE),
            volume: own::number(own::VOLUME, 1.0),
            covers: Some(nori_covers::Config { memory_bytes: 0, ..nori_covers::Config::new(data.join("covers")) }),
            offline: false,
            mpris,
            kind: DeviceKind::Desktop,
            out,
        };
        Ok(Session { id, host: nori_host::desktop::open(o)?, tx })
    }

    fn sender(&self) -> impl Fn(Msg) + Send + 'static {
        let (tx, id) = (self.tx.clone(), self.id);
        move |m| tx.send(Msg::From(id, Box::new(m)))
    }

    /// Reads a page: the cached answer first, then the server's if it differs.
    pub fn load(&self, req: Req) {
        let send = self.sender();
        match req.clone() {
            Req::Home(rows) => self.host.read_home(rows, true, move |r| send(Msg::Data(req.clone(), r.map(Data::Home).map_err(|e| net_error(&e))))),
            Req::Page(ask) => self.host.read_page(ask, move |r| send(Msg::Data(req.clone(), r.map(Data::Page).map_err(|e| page_failed(&e))))),
        }
    }

    /// Requests a cover at `px` square, with its page colours where `colours`, derived off this thread.
    pub fn cover(&self, key: CoverKey, px: u32, colours: bool) -> Option<Ticket> {
        let send = self.sender();
        self.host.cover(key.id.clone(), px, colours, move |image, colours| {
            if let Ok(image) = image { send(Msg::Cover { key, image, colours }); }
        })
    }

    /// Remote control, while it is on: the other devices and moving the music between them.
    pub fn remote(&self) -> Option<Arc<nori_core::remote::Remote>> {
        self.host.remote()
    }

    /// Starts a jam around `songs[start]` ([`nori_host::session::Handle::start_jam`]); [`Msg::Jam`] says
    /// whether it opened.
    pub fn start_jam(&self, songs: Vec<nori_core::Song>, start: usize) {
        let (host, send) = (nori_host::session::Handle::clone(&self.host), self.sender());
        nori_host::spawn("nori-jam", move || {
            if let Some(opened) = nori_core::transport::block_on(host.start_jam(songs, start, None)) {
                send(Msg::Jam(opened.map(drop).map_err(|e| net_error(&e))));
            }
        });
    }

    /// Starts hosting a jam on the server's relay; [`Msg::Jam`] says whether it opened.
    pub fn jam_open(&self) {
        let (Some(r), send) = (self.remote(), self.sender()) else { return };
        nori_host::spawn("nori-jam", move || send(Msg::Jam(nori_core::transport::block_on(r.jam_open()).map(drop).map_err(|e| net_error(&e)))));
    }

    /// Searches the server (after a typing pause).
    pub fn search_server(&self, query: String) {
        self.host.search_server(query, net_error);
    }

    /// Gathers [`Facts`](crate::settings::Facts) on a worker (asks the server for music folders).
    pub fn facts(&self) {
        let (core, client, store, db, send) = (self.core.clone(), self.client.clone(), self.store.clone(), self.db.clone(), self.sender());
        let cover_bytes = self.covers.as_ref().and_then(|l| l.disk()).map_or(0, |d| d.bytes());
        nori_host::spawn("nori-facts", move || {
            let index = core.index_size().unwrap_or_default();
            let downloads = core.downloads(true).unwrap_or_default();
            // Every profile's: they share this computer's download folder.
            let listed = core.downloads_by_profile().unwrap_or_default();
            let all_download_songs = listed.iter().map(|p| p.ids.len() as u32).sum();
            let profiles: Vec<(String, u64)> =
                listed.into_iter().map(|p| { let b = store.as_ref().map_or(0, |s| s.download_bytes_of(&p.ids)); (p.profile, b) }).collect();
            let folders = match block_on(client.read_now(Read::MusicFolders)) {
                Ok(Page::Folders { v }) => v.into_iter().map(|f| (f.name, f.id)).collect(),
                _ => Vec::new(),
            };
            send(Msg::Facts(Box::new(crate::settings::Facts {
                analysed: core.analysis_count().unwrap_or(0),
                indexed: (index.songs, index.albums, index.artists),
                stream_bytes: store.as_ref().map_or(0, |s| s.cache_bytes()),
                cover_bytes,
                lyrics_bytes: core.lyrics_cache_bytes().max(0) as u64,
                download_bytes: profiles.iter().map(|p| p.1).sum(),
                all_download_songs,
                download_profiles: profiles,
                download_songs: downloads.len() as u32,
                database_bytes: std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0),
                folders,
                devices: CpalOutput::devices(),
                device: own::text(own::DEVICE).unwrap_or_default(),
                syncing: false,
                unsent: client.unsent().map_or((0, 0), |u| (u.waiting, u.dropped)),
                rules: core.rules(),
            })));
        });
    }
}

/// Why a page has nothing, in words.
fn page_failed(e: &PageFailed) -> String {
    match e {
        PageFailed::Nothing => "The server sent nothing for this page".into(),
        PageFailed::Net(e) => net_error(e),
        PageFailed::NotDownloaded => "Album is no longer downloaded".into(),
        PageFailed::Stored(e) => e.to_string(),
        PageFailed::EmptyMix => "This mix has nothing in it yet".into(),
    }
}

/// A session's report as the UI thread takes it.
fn worded(s: Said) -> Msg {
    let note = |text: String, error: bool| Msg::Note { text, error };
    match s {
        Said::Engine(e) => Msg::Engine(e),
        Said::Lyrics { song, pick } => Msg::Lyrics { song, pick },
        Said::Search(v) => Msg::Search(v),
        Said::Reachable(r) => Msg::Reachable(r.map_err(|e| net_error(&e))),
        Said::Remote => Msg::Remote,
        Said::JamEnded { host } => Msg::JamEnded(host),
        Said::Volume(v) => Msg::Volume(v),
        Said::Starred(_) => Msg::Starred,
        Said::Curve(n) => {
            let (text, action) = crate::words::curve_notice(&n);
            Msg::Ask { text, action: action.into() }
        }
        Said::Note(n) => match n {
            Note::Queued { next, songs: n } => note(format!("{}: {}", if next { "Playing next" } else { "Added to the queue" }, songs(n)), false),
            Note::NothingToPlay => note("Nothing to play".into(), false),
            Note::SongsFailed(e) => note(format!("Could not load the songs: {}", net_error(&e)), true),
            Note::NothingToPutBack => note("Nothing to put back".into(), false),
            Note::Downloading(n) => note(format!("Downloading {n} songs"), false),
            Note::DownloadFailed(e) => note(format!("Could not download the library: {e}"), true),
            Note::StarFailed(e) => note(format!("Could not change the favorite: {}", net_error(&e)), true),
            Note::Indexing => note("Filling the offline index…".into(), false),
            Note::Indexed(t) => note(format!("Offline index: {} songs", t.songs), false),
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
