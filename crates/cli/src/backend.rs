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
    /// A plugin's cast started, changed or ended (None).
    Cast(Option<nori_plugins::Cast>),
    /// Read again what `Req` reads, where it is on screen.
    Reload(Req),
    /// The queue another device left on the server, to offer here.
    Offer(Option<nori_core::library::QueueOffer>),
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
    /// How signing the account in went.
    SignedIn(nori_core::providers::SignInResult),
    /// A plugin installed (its id), or why not.
    Installed(Result<String, nori_plugins::Refusal>),
    /// A plugin to install, as its sheet shows it, or why it can't be.
    Preview(Result<nori_plugins::PluginPreview, nori_plugins::Refusal>),
    /// The plugins' lines in `song`'s info and under the player's output.
    Slots { song: String, info: Vec<nori_plugins::InfoGroup>, status: Vec<String> },
    /// What the plugin repos offer.
    Offers(nori_plugins::Offers),
    /// Plugins updated on their own: each one's id and new version.
    Updated(Vec<(String, String)>),
    /// The devices plugins control, or what they report, changed: read them again.
    Controlled,
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
            Msg::Cast(c) => format!("cast {c:?}"),
            Msg::Offer(o) => format!("queue offered: {}", o.is_some()),
            Msg::Reload(r) => format!("read {r:?} again"),
            Msg::LoggedIn(r) => format!("logged in: {}", r.is_ok()),
            Msg::Reachable(r) => format!("reachable: {}", r.is_ok()),
            Msg::Volume(v) => format!("volume {v}"),
            Msg::Starred(_) => "star marks".into(),
            Msg::Remote => "remote".into(),
            Msg::Jam(r) => format!("jam opened: {}", r.is_ok()),
            Msg::Joined(r) => format!("jam joined: {}", r.is_ok()),
            Msg::Left(said) => format!("jam left: {said}"),
            Msg::SignedIn(r) => format!("signed in: {}", matches!(r, nori_core::providers::SignInResult::SignedIn { .. })),
            Msg::Installed(r) => format!("plugin installed: {r:?}"),
            Msg::Preview(r) => format!("plugin preview: {:?}", r.as_ref().map(|p| &p.card.id)),
            Msg::Slots { song, info, status } => format!("plugins on {song}: {} info groups, {} status lines", info.len(), status.len()),
            Msg::Updated(u) => format!("{} plugins updated", u.len()),
            Msg::Controlled => "controlled devices".into(),
            Msg::Offers(o) => format!("{} plugins offered, {} repos unread", o.offers.len(), o.failed.len()),
            Msg::From(id, m) => format!("session {id}: {}", m.brief()),
        }
    }
}

/// A screen's data request.
#[derive(Debug, Clone, PartialEq)]
pub enum Req {
    Home,
    /// The albums in a `browse::album_sorts` order, `favorites` only the starred.
    Albums { sort: nori_core::browse::AlbumSort, favorites: bool, offset: u32 },
    Artists,
    Playlists,
    /// The songs in a `browse::song_sorts` order, `starred` only the favorites.
    Songs { sort: String, starred: bool, offset: u32 },
    /// This device's listening history from where a page left off.
    History(Option<nori_core::browse::HistoryAfter>),
    /// What the downloads weigh.
    DownloadsSize,
    Album(String),
    DownloadedAlbum(String),
    Artist(String),
    Playlist(String),
    Mix(String),
    /// A Home row in full, a page from `offset`.
    Row { row: nori_core::settings::HomeRow, offset: u32 },
    Favorites,
    Downloads,
    /// The downloads screen's queue sections only, for the refresh while downloads run.
    DownloadProgress,
    Facts,
}

pub enum Data {
    /// Home shelf: index (into the session's Home rows, or the service's feed), the library's row it is,
    /// title, entries.
    HomeRow(usize, Option<nori_core::settings::HomeRow>, String, Vec<crate::app::Item>),
    /// For you's tiles.
    Mixes(Vec<nori_core::mixes::board::MixTile>),
    /// A page of a Home row in full, and whether it was the last.
    Row(Vec<crate::app::Item>, bool),
    Mix(Box<nori_core::mixes::board::MixSheet>),
    Albums(Vec<nori_core::Album>),
    Artists(Vec<nori_core::Artist>),
    Playlists(Vec<nori_core::Playlist>),
    Songs(Vec<Song>, bool),
    History(nori_core::browse::HistoryPage),
    DownloadsSize(i64),
    /// The starred songs.
    Favorites(Vec<Song>),
    Album(Box<AlbumDetail>),
    Artist(Box<ArtistDetail>),
    Playlist(Box<PlaylistDetail>),
    Downloads(Box<Downloads>),
    DownloadProgress(Box<nori_core::transfers::DownloadSections>),
    Facts(Box<Facts>),
}

/// The downloads screen's sections.
#[derive(Default)]
pub struct Downloads {
    pub active: Vec<Song>,
    pub queued: Vec<Song>,
    pub failed: Vec<Song>,
    pub stored: nori_core::library::DownloadedLibrary,
    /// How many songs this session had finished when `stored` was read; more means read it again.
    pub finished: usize,
}


/// Logs `draft` in and keeps it as the active profile (blocking): the core checks it with its provider
/// (a server is tried through its second address, then with legacy auth, which the profile then keeps).
pub fn check_login(data: &Path, http: Arc<Http>, draft: SavedServer) -> Result<SavedServer, String> {
    nori_host::desktop::log_in(data, http, draft).map_err(|e| match net_error(&e) {
        e if e.is_empty() => "the server did not answer".to_string(),
        e => e,
    })
}

/// The terminal client's own settings, stored in the core's `app_kv`.
pub mod own {
    pub use nori_host::desktop::own::{flag, keep, number, text};
    pub const MOUSE: &str = "tui.mouse";
    pub const IMAGES: &str = "tui.images";
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
            Req::Albums { sort, favorites, offset } => Some(PageAsk::Albums { sort, offset, favorites }),
            Req::Artists => Some(PageAsk::Artists),
            Req::Playlists => Some(PageAsk::Playlists),
            Req::Album(id) => Some(PageAsk::Album(id)),
            Req::DownloadedAlbum(id) => Some(PageAsk::DownloadedAlbum(id)),
            Req::Artist(id) => Some(PageAsk::Artist(id)),
            Req::Playlist(id) => Some(PageAsk::Playlist(id)),
            Req::Mix(id) => Some(PageAsk::Mix(id)),
            Req::Favorites => Some(PageAsk::Favorites),
            Req::Row { .. } | Req::Home | Req::Songs { .. } | Req::History(_) | Req::DownloadsSize | Req::Downloads | Req::DownloadProgress | Req::Facts => None,
        };
        if let Some(ask) = ask {
            return self.host.read_page(ask, move |r| send(r.map(data_of).map_err(|e| page_failed(&e))));
        }
        match req {
            Req::Row { row, offset } => {
                use crate::app::Item;
                let page = nori_core::browse::home_row_page(row, offset);
                self.host.read_page(PageAsk::HomeRow { row, offset }, move |r| {
                    let last = |got: usize| !page.clone().is_some_and(|s| nori_core::browse::home_row_more(s, got as u32));
                    send(match r {
                        Ok(PageGot::Albums(v)) => Ok(Data::Row(v.iter().cloned().map(Item::Album).collect(), last(v.len()))),
                        Ok(PageGot::Playlists(v)) => Ok(Data::Row(v.into_iter().map(Item::Playlist).collect(), true)),
                        Ok(PageGot::Songs { songs, exhausted }) => Ok(Data::Row((0..songs.len()).map(|n| Item::Song(songs.clone(), n)).collect(), exhausted)),
                        Ok(_) | Err(PageFailed::Nothing) => Ok(Data::Row(Vec::new(), true)),
                        Err(e) => Err(page_failed(&e)),
                    })
                });
            }
            Req::Home => {
                let rows = self.host.home_rows();
                self.host.read_home(rows.clone(), true, move |r| match r {
                    // A source's own Home: its feed, each shelf's songs first (a song plays the shelf's songs
                    // from it), then its albums, playlists and artists, as the desktop's.
                    Ok(HomePart::Feed(feed)) => {
                        let shown = feed.into_iter().filter(|f| !(f.songs.is_empty() && f.albums.is_empty() && f.playlists.is_empty() && f.artists.is_empty()));
                        for (i, f) in shown.enumerate() {
                            use crate::app::Item;
                            let songs = (0..f.songs.len()).map(|n| Item::Song(f.songs.clone(), n));
                            let rest = f.albums.into_iter().map(Item::Album).chain(f.playlists.into_iter().map(Item::Playlist)).chain(f.artists.into_iter().map(Item::Artist));
                            send(Ok(Data::HomeRow(i, None, f.title, songs.chain(rest).collect())));
                        }
                    }
                    Ok(HomePart::Shelf { at, items }) => {
                        use crate::app::Item;
                        let items = match items {
                            ShelfItems::Albums(v) => v.into_iter().map(Item::Album).collect(),
                            ShelfItems::Playlists(v) => v.into_iter().map(Item::Playlist).collect(),
                            ShelfItems::Songs(v) => (0..v.len()).map(|n| Item::Song(v.clone(), n)).collect(),
                        };
                        send(Ok(Data::HomeRow(at, Some(rows[at]), crate::text::home_row(rows[at]).into(), items)))
                    }
                    Ok(HomePart::Mixes(tiles)) => send(Ok(Data::Mixes(tiles))),
                    Err(e) => send(Err(unreachable(&e))),
                });
            }
            Req::Downloads => {
                let core = self.core.clone();
                nori_host::spawn("nori-read", move || {
                    let stored = core.downloaded_library();
                    send(stored.and_then(|stored| core.download_sections().map(|s| Data::Downloads(Box::new(Downloads { finished: s.finished.len(), active: s.active, queued: s.queued, failed: s.failed, stored })))).map_err(|e| e.to_string()));
                });
            }
            // The index's songs in the order asked for (the server's own for a profile that keeps none).
            Req::Songs { sort, starred, offset } => {
                let client = self.client.clone();
                nori_host::spawn("nori-read", move || send(block_on(client.songs_listed(sort, starred, 0, 0, offset)).map(|p| Data::Songs(p.songs, p.exhausted)).map_err(|e| unreachable(&e))));
            }
            Req::History(after) => {
                let core = self.core.clone();
                nori_host::spawn("nori-read", move || send(core.history_page(after).map(Data::History).map_err(|e| e.to_string())));
            }
            Req::DownloadsSize => {
                let core = self.core.clone();
                nori_host::spawn("nori-read", move || send(core.downloads(true).map(|v| Data::DownloadsSize(v.iter().map(|s| s.size as i64).sum())).map_err(|e| e.to_string())));
            }
            Req::DownloadProgress => {
                let core = self.core.clone();
                nori_host::spawn("nori-read", move || send(core.download_sections().map(|s| Data::DownloadProgress(Box::new(s))).map_err(|e| e.to_string())));
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
    pub fn cover(&self, art: String, size: u32) -> Option<nori_covers::loader::Ticket> {
        let (tx, me) = (self.tx.clone(), self.id);
        self.host.cover(art.clone(), size, true, move |image, colours| {
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
        PageGot::Mix(m) => Data::Mix(m),
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
    // Every profile's: they share this computer's download folder.
    let listed = core.downloads_by_profile().unwrap_or_default();
    let all_download_songs = listed.iter().map(|p| p.ids.len() as u32).sum();
    let download_profiles: Vec<(String, i64)> = listed.into_iter().map(|p| { let b = store.map_or(0, |s| s.download_bytes_of(&p.ids)) as i64; (p.profile, b) }).collect();
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
            downloads: download_profiles.iter().map(|p| p.1).sum(),
            all_download_songs,
            download_profiles,
            download_songs: downloads.len() as u32,
            database: std::fs::metadata(db).map(|m| m.len() as i64).unwrap_or(0),
        },
        folders,
        devices: CpalOutput::devices(),
        unsent: client.unsent().map_or((0, 0), |u| (u.waiting, u.dropped)),
        rules: core.rules(),
    }
}
