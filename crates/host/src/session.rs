//! One open server profile: core, client, the engine on the client's output, store, downloader, cover
//! loader and search. Work that waits on the network runs among the session's tasks and reports through
//! the client's [`Out`], as facts the client words.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nori_core::bridge::BridgeTake;
use nori_core::browse::ProfileRules;
use nori_core::cache_policy::{Page, Read};
use nori_core::client::{Client, Starrable};
use nori_core::covers::CoverNet;
use nori_core::library::StarsShown;
use nori_core::profiles::{CurveNotice, DeviceCurve, DeviceEffect};
use nori_core::playlist::{Hand, QueueEdit};
use nori_core::race::{LyricsPick, LyricsShown};
use nori_core::rules::{queue_keep, BridgeStep, QueueMoment};
use nori_core::search::{SearchSession, SearchView};
use nori_core::settings::{SavedServer, SettingChange, StoredPrefs};
use nori_core::settings_store::{APPLY_AUDIO, APPLY_GAIN, CACHE_LIMIT, PLAYER, REPLAN, SOUND};
use nori_core::transport::{block_on, Exchange, FailureKind, NetError, Transport, TransportError, TransportResponse};
use nori_tasks::Tasks;
use nori_core::{Core, CoreError, IngestStats, PageOrigin, Song};
use nori_covers::loader::{Config as CoverConfig, Loader, Ticket};
use nori_covers::memory::Image;
use nori_engine::core::{resolving, settings, Analyses, CoreApp, CoreLibrary, CoreQueue, Downloader, Measurer};
use nori_engine::{AudioOutput, Body, ByteSource, Cancel, Config, Engine, Event, Filling, OpenError, State, Store};
use nori_look::cover::CoverColours;

use nori_core::remote::{Press, Reach};
use crate::{db_path, derive, net, save, Fetch, Keeper, Level};
use nori_remote::wire::Op;

/// What a session reports from any thread.
pub enum Said {
    Engine(Event),
    /// One of possibly several lyrics answers for `song`, each better than the last.
    Lyrics { song: String, pick: LyricsPick },
    Search(SearchView),
    Note(Note),
    /// The reachability check made when a session opens.
    Reachable(Result<(), NetError>),
    /// The other devices or the jam changed (remote control): read them again.
    Remote,
    /// The jam this guest is in ended (`host`: its host's name, if seen): the client leaves it as on
    /// Leave, and says so.
    JamEnded { host: Option<String> },
    /// Another device set the volume (0 to 1).
    Volume(f32),
    /// The AutoEQ curve of the device music moved to was offered or applied; the client says so, and
    /// answers through [`Handle::curve_answer`].
    Curve(CurveNotice),
    /// A heart changed (pressed here, or by another device): hearts are read again ([`Handle::starred`]), or drawn from the core's marks
    /// carried here.
    Starred(nori_core::stars::StarMarks),
}

/// Something done or failed, for the status line.
pub enum Note {
    Queued { next: bool, songs: usize },
    NothingToPlay,
    SongsFailed(NetError),
    NothingToPutBack,
    Downloading(usize),
    DownloadFailed(CoreError),
    StarFailed(NetError),
    Indexing,
    Indexed(IngestStats),
    IndexStopped(NetError),
    Done(Chore),
    /// Forgot this many songs' measurements.
    Forgot(u32),
    /// An AutoEQ curve could not be fetched.
    CurveFailed(NetError),
}

/// Maintenance a settings page runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chore {
    SyncLibrary,
    DownloadLibrary,
    MeasureAgain,
    ClearStream,
    ClearCovers,
    ClearLyrics,
}

/// Where a session's reports go.
pub type Out = Arc<dyn Fn(Said) + Send + Sync>;

/// The engine's events, as the session hands them on.
pub type Events = Box<dyn FnMut(Event) + Send>;

/// The engine of [`Session::open_driven`], turned by its caller, and the songs measured as they arrive
/// on its turns.
pub struct Driven {
    engine: nori_engine::Driven<CoreLibrary, CoreApp, CoreQueue, Events, nori_engine::Monotonic>,
    player: Arc<Engine>,
    analyses: Arc<Analyses>,
    /// Songs measured as they arrived, as last seen: a new one replans the transition.
    measured: u64,
}

impl Driven {
    /// [`nori_engine::Driven::turn`], then a slice of measuring if one is due: ms until the next turn is
    /// due (`None`: when woken), or `None` once the engine stopped.
    pub fn turn(&mut self) -> Option<Option<i64>> {
        let next = self.engine.turn()?;
        let measuring = self.analyses.turn();
        let measured = self.analyses.measured_as_they_came();
        if measured != self.measured {
            self.measured = measured;
            self.player.replan();
        }
        Some([next, measuring].into_iter().flatten().min())
    }
}

/// The platform's HTTP: the core's API and covers, and the engine's audio, over one connection pool
/// (nori-http's ureq agent natively).
pub trait Net: Transport + ByteSource {}

impl<T: Transport + ByteSource> Net for T {}

/// Audio bytes over HTTP: refused when offline, requests counted.
pub struct Audio {
    http: Arc<dyn Net>,
    offline: bool,
    pub requests: AtomicU64,
}

impl Audio {
    pub fn new(http: Arc<dyn Net>, offline: bool) -> Audio {
        Audio { http, offline, requests: AtomicU64::new(0) }
    }
}

impl ByteSource for Audio {
    fn open(&self, url: &str, from: u64) -> Result<Body, OpenError> {
        self.open_cancellable(url, None, from, &Cancel::new())
    }

    fn open_cancellable(&self, url: &str, key: Option<&str>, from: u64, cancel: &Cancel) -> Result<Body, OpenError> {
        if self.offline {
            return Err("offline".into());
        }
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.http.open_cancellable(url, key, from, cancel)
    }

    fn open_part(&self, url: &str, key: Option<&str>, from: u64, end: u64, cancel: &Cancel) -> Result<Body, OpenError> {
        if self.offline {
            return Err("offline".into());
        }
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.http.open_part(url, key, from, end, cancel)
    }

    fn open_live(&self, url: &str) -> Result<(Body, Option<usize>), String> {
        if self.offline {
            return Err("offline".into());
        }
        self.http.open_live(url)
    }

    /// The platform's HTTP fetches on its own events where nothing may block (the web).
    fn push(&self, fill: Filling) -> Option<Filling> {
        if self.offline {
            return Some(fill);
        }
        let given = self.http.push(fill);
        if given.is_none() {
            self.requests.fetch_add(1, Ordering::Relaxed);
        }
        given
    }
}

/// The offline transport: every request fails as unreachable, so the core shows what is stored and
/// queues writes for later.
pub(crate) struct Offline;

fn offline() -> TransportError {
    TransportError::Failed { kind: FailureKind::NoRoute, detail: Some("offline".into()) }
}

#[async_trait::async_trait]
impl Transport for Offline {
    async fn get(&self, _: String, _: u32) -> Result<TransportResponse, TransportError> {
        Err(offline())
    }

    async fn send(&self, _: Exchange) -> Result<TransportResponse, TransportError> {
        Err(offline())
    }

    fn address_changed(&self) {}

    fn network(&self) -> nori_core::transport::Network {
        nori_core::transport::Network::Unmetered
    }
}

struct Shown {
    song: String,
    out: Out,
}

impl LyricsShown for Shown {
    fn show(&self, pick: LyricsPick) {
        (self.out)(Said::Lyrics { song: self.song.clone(), pick });
    }
}

/// The core's star marks moved: the client reads its hearts again, and other devices see the queue's.
struct Hearts {
    out: Out,
    remotes: Arc<crate::remote::Remotes>,
    engine: Arc<Engine>,
}

impl StarsShown for Hearts {
    fn marks(&self, marks: nori_core::stars::StarMarks) {
        self.remotes.played(&self.engine);
        (self.out)(Said::Starred(marks));
    }
}

pub struct Open<'a> {
    /// The app's queue session, over its settings.
    pub queue: Arc<nori_core::queue::Session>,
    pub data: &'a Path,
    pub http: Arc<dyn Net>,
    pub profile: SavedServer,
    /// The sound card. The client built it and owns its device volume.
    pub output: Box<dyn AudioOutput>,
    /// The volume, set through [`Handle::set_volume`] (or followed with [`Handle::volume_followed`] where
    /// the system keeps it).
    pub volume: Arc<Level>,
    /// Bytes the engine may hold for songs ahead. 256 on the desktop, less on a phone.
    pub memory_mb: u32,
    pub covers: Option<CoverConfig>,
    pub offline: bool,
    /// The process's media controls, driven by this session while it is open. Always a field, so a
    /// client built without `desktop` (where nothing can fill it) compiles either way.
    pub mpris: Option<MediaControls>,
    /// This device as the account's other devices list it (remote control).
    pub device: nori_core::remote::RemoteMe,
    /// The platform's mDNS for remote control (Bonjour on iOS). None: with `desktop`, the session's own
    /// (mdns-sd); without, no nearby devices.
    pub discovery: Option<Arc<dyn nori_core::remote::Discovery>>,
    /// Where work that waits on the network runs ([`crate::tasks`] natively).
    pub tasks: Arc<dyn Tasks>,
    /// Songs are kept as files under `data`: the stream cache and downloads. A browser keeps none.
    pub keeps_music: bool,
    /// The platform's own decoders, preferred to those here.
    pub decoders: nori_engine::Platform,
    pub out: Out,
    /// The kinds of profile it opens: nori's own, and its plugins' where the client has them.
    pub kinds: Arc<nori_core::providers::Kinds>,
}

#[cfg(feature = "desktop")]
pub type MediaControls = Arc<nori_mpris::Mpris>;
/// No media controls without `desktop`: only `None` fits.
#[cfg(not(feature = "desktop"))]
pub enum MediaControls {}

/// One open server profile. Its controls are [`Handle`]'s; closing it saves the queue and stops the
/// engine.
pub struct Session {
    h: Handle,
}

impl std::ops::Deref for Session {
    type Target = Handle;

    fn deref(&self) -> &Handle {
        &self.h
    }
}

/// An open session's parts, shared by its tasks, the remote control and the media keys: cloning it is
/// one reference.
#[derive(Clone)]
pub struct Handle(Arc<Parts>);

impl std::ops::Deref for Handle {
    type Target = Parts;

    fn deref(&self) -> &Parts {
        &self.0
    }
}

pub struct Parts {
    pub core: Arc<Core>,
    pub client: Arc<Client>,
    pub engine: Arc<Engine>,
    /// The stream cache and downloads, where the platform keeps music files.
    pub store: Option<Arc<Store>>,
    downloader: Option<Arc<Downloader>>,
    pub covers: Option<Arc<Loader>>,
    level: Arc<Level>,
    search: Arc<SearchSession>,
    pub offline: bool,
    /// The process's media controls while this session drives them.
    #[cfg(feature = "desktop")]
    pub(crate) media: Option<crate::remote::Media>,
    keeper: Arc<Keeper>,
    /// The database file, for its size.
    pub db: PathBuf,
    /// Remote control and jams, while switched on.
    pub(crate) remotes: Arc<crate::remote::Remotes>,
    device: nori_core::remote::RemoteMe,
    /// The profile is a jam guest's: songs picked are asked of the jam's host, and nothing else plays.
    pub guest: bool,
    /// The rules the client was last told of ([`Parts::account_changed`]).
    rules_shown: parking_lot::Mutex<ProfileRules>,
    discovery: Option<Arc<dyn nori_core::remote::Discovery>>,
    /// The AutoEQ notice said last, waiting for an answer.
    curve: parking_lot::Mutex<Option<CurveNotice>>,
    tasks: Arc<dyn Tasks>,
    out: Out,
}

impl Session {
    /// Opens the profile without touching the network, so an unreachable server still opens with what
    /// is stored; [`Handle::check`] reports reachability.
    pub fn open(o: Open) -> Result<Session, String> {
        Self::open_with(o, false, |library, app, queue, output, config, events| Engine::start(library, app, queue, output, None, config, events))
    }

    /// [`Session::open`] with the engine on the caller's thread, for a platform without threads (the web):
    /// the caller turns the [`Driven`] whenever `wake` is called and once the time it asks for has passed.
    pub fn open_driven(o: Open, wake: impl Fn() + Send + Sync + Clone + 'static) -> Result<(Session, Driven), String> {
        let mut driven = None;
        let s = Self::open_with(o, true, |library, app, queue, output, config, events| {
            let analyses = library.analyses.clone();
            let (engine, d) = Engine::driven(library, app, queue, output, config, nori_engine::Monotonic::new(), wake, events);
            driven = Some((d, analyses));
            engine
        })?;
        let (engine, analyses) = driven.expect("the engine was made");
        let player = s.engine.clone();
        Ok((s, Driven { engine, player, analyses, measured: 0 }))
    }

    #[cfg(test)]
    fn open_on<C: nori_engine::Clock + Sync>(o: Open, clock: C) -> Result<Session, String> {
        Self::open_with(o, false, |library, app, queue, output, config, events| Engine::start_on(library, app, queue, output, None, config, clock, events))
    }

    /// `turns`: the caller drives the engine, so songs are measured on its turns, not threads.
    fn open_with(o: Open, turns: bool, start: impl FnOnce(CoreLibrary, CoreApp, CoreQueue, Box<dyn AudioOutput>, Config, Events) -> Engine) -> Result<Session, String> {
        let db = db_path(o.data);
        let core = Core::new(db.clone(), nori_core::settings::server_db_id(&o.profile.id), o.queue.clone()).map_err(|e| format!("the database: {e}"))?;
        core.use_kinds(o.kinds.clone());
        let transport: Arc<dyn Transport> = if o.offline { Arc::new(Offline) } else { o.http.clone() };
        o.kinds.set_transport(transport.clone());
        core.configure_profile(o.profile.clone()).map_err(|e| format!("the server: {e}"))?;
        let cover_net = CoverNet::over(o.http.clone());
        let client = Client::new(core.clone(), transport, cover_net.clone());
        client.set_profile(net(&o.profile));
        let prefs = core.session.settings.current().unwrap_or_default();
        // Set by the client before the engine starts, so the first chain has the right compensation.
        let level = o.volume;
        let output = o.output;
        let store = match o.keeps_music {
            true => Some(Store::open(o.data.join("music"), prefs.cache_mb.max(0) as u64 * 1024 * 1024).map_err(|e| format!("the music directory: {e}"))?),
            false => None,
        };
        let audio = resolving(&client, Arc::new(Audio::new(o.http.clone(), o.offline)));
        let analyses = if turns { Analyses::on_turns(client.clone()) } else { Analyses::of(client.clone()) };
        let downloader = store.as_ref().map(|store| Downloader::new(client.clone(), audio.clone(), store.clone(), analyses.clone()));
        // `bridging`: a song the network cannot bring raises `Event::Bridge` (see `Handle::bridge`).
        let guest = nori_remote::is_guest_key(&o.profile.api_key);
        let rules = core.rules();
        let app = CoreApp::new(core.session.clone()).per_device(core.clone()).bridging();
        // A guest plays its host's transitions.
        let app = match &store {
            Some(store) if rules.measures => app.measuring(Measurer::new(analyses.clone(), store.clone())),
            _ => app,
        };
        let library = CoreLibrary { client: client.clone(), bytes: audio, store: store.clone(), analyses };
        let config = Config { memory_mb: o.memory_mb, settings: settings(&prefs, core.session.settings.layer()), platform: o.decoders, ..Config::default() };
        // Each event is the session's first (among its tasks, off the engine's thread), then the client's.
        let heard: Arc<std::sync::OnceLock<std::sync::Weak<Parts>>> = Arc::default();
        let (to, out, tasks) = (heard.clone(), o.out.clone(), o.tasks.clone());
        let events: Events = Box::new(move |e| {
            if let Some(parts) = to.get().and_then(std::sync::Weak::upgrade) {
                let e = e.clone();
                tasks.spawn(Box::pin(async move { Handle(parts).heard(&e) }));
            }
            out(Said::Engine(e));
        });
        let engine = Arc::new(start(library, app, CoreQueue(core.session.clone()), output, config, events));
        let covers = o.covers.map(|config| Arc::new(Loader::new(config, cover_net)));
        let remotes = Arc::new(crate::remote::Remotes::new(level.clone()));
        let (saved, playing) = (core.clone(), engine.clone());
        let keeper = Keeper::start(move || save(&saved, &playing), o.tasks.clone());
        // A jam guest is no device of the account's: the jam lists it as a guest.
        let device = if guest { nori_core::remote::RemoteMe { kind: nori_remote::wire::DeviceKind::Guest, ..o.device } } else { o.device };
        #[cfg(feature = "desktop")]
        let media = o.mpris.map(|m| crate::remote::Media::new(m, covers.clone(), core.clone()));
        let parts = Parts {
            core,
            client,
            engine,
            store,
            downloader,
            covers,
            level,
            search: SearchSession::new(),
            offline: o.offline,
            #[cfg(feature = "desktop")]
            media,
            keeper,
            db: PathBuf::from(db),
            remotes,
            device,
            guest,
            rules_shown: parking_lot::Mutex::new(rules),
            discovery: o.discovery,
            curve: Default::default(),
            tasks: o.tasks,
            out: o.out,
        };
        let h = Handle(Arc::new(parts));
        let _ = heard.set(Arc::downgrade(&h.0));
        #[cfg(feature = "desktop")]
        if let Some(m) = &h.media {
            m.mpris.serve(Some(Arc::new(h.clone())));
        }
        h.restore();
        h.follow_remote();
        if !h.offline && h.rules().downloads && h.core.download_counts().pending > 0 {
            h.start_downloads();
        }
        Ok(Session { h })
    }

    /// Saves the queue and stops the engine.
    pub fn close(&self) {
        #[cfg(feature = "desktop")]
        if let Some(m) = &self.media {
            m.mpris.serve(None);
        }
        self.keep(QueueMoment::Closing);
        self.remotes.set(None);
        self.keeper.stop();
        self.engine.stop();
    }
}

impl Handle {
    fn note(&self, n: Note) {
        (self.out)(Said::Note(n));
    }

    /// Runs `work` among the session's tasks.
    fn run(&self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        self.tasks.spawn(Box::pin(work));
    }

    /// An engine event, as the session takes it before the client does.
    fn heard(&self, e: &Event) {
        #[cfg(feature = "desktop")]
        self.mpris_heard(e);
        self.followed(e);
    }

    /// Tells the media controls the song or state changed.
    #[cfg(feature = "desktop")]
    pub fn mpris_changed(&self) {
        if let Some(m) = &self.media {
            m.mpris.changed();
        }
    }

    /// Tells the media controls of engine event `e`: the song or state changed, or the place jumped (a
    /// seek, a restart, a song coming in past its start).
    #[cfg(feature = "desktop")]
    fn mpris_heard(&self, e: &Event) {
        let Some(m) = &self.media else { return };
        if matches!(e, Event::Song { .. } | Event::State(_) | Event::Looped { .. } | Event::Bridge { .. }) {
            m.mpris.changed();
        }
        let jumped = self.engine.status_with(|s| m.seen.lock().jumped(s));
        if jumped && self.remotes.elsewhere().is_none() {
            m.mpris.seeked();
        }
    }

    /// Pings the server in the background, fills an empty offline index and flushes pending writes.
    pub fn check(&self) {
        if self.offline {
            return;
        }
        let me = self.clone();
        self.run(async move {
            let r = me.client.read_now(Read::Ping).await.map(|_| ());
            let ok = r.is_ok();
            (me.out)(Said::Reachable(r));
            // Search and the songs list read the offline index, where the profile keeps one.
            if ok && me.rules().indexed && me.core.index_size().map_or(true, |s| s.songs == 0) {
                me.sync();
            }
            let _ = me.client.flush_pending().await;
        });
    }

    /// Fills the offline index from the server, on a thread of its own: its pages are written between
    /// requests, which would hold up the session's other tasks.
    pub fn sync(&self) {
        let (client, out) = (self.client.clone(), self.out.clone());
        crate::spawn("nori-sync", move || {
            out(Said::Note(Note::Indexing));
            out(Said::Note(match block_on(client.sync_library()) {
                Ok(total) => Note::Indexed(total),
                Err(e) => Note::IndexStopped(e),
            }));
        });
    }

    /// Restores the saved queue, paused at its position; with none saved the queue is empty (the one
    /// left by the profile open before, a jam's host's songs, is no queue of this one's).
    fn restore(&self) {
        let Some(q) = self.core.load_queue().ok().filter(|q| !q.songs.is_empty()) else {
            self.core.session.set(Vec::new(), None, false, None);
            return self.engine.queue_changed();
        };
        let index = q.index as usize;
        self.core.session.set(q.songs.iter().map(|s| s.id.clone()).collect(), Some(index as u32), false, q.origin);
        self.engine.queue_changed();
        self.engine.go_to(index, q.position_ms as i64);
    }

    /// Saves the queue and pushes it to the server as `rules::queue_keep` says for `moment`.
    pub fn keep(&self, moment: QueueMoment) {
        let k = queue_keep(moment);
        if k.save_after_ms > 0 {
            self.keeper.later(k.save_after_ms);
        } else {
            save(&self.core, &self.engine);
        }
        if k.push && self.rules().account {
            let (client, st) = (self.client.clone(), self.engine.status());
            self.run(async move {
                let _ = client.playlist_push(st.id.clone(), st.position_now().max(0)).await;
            });
        }
    }

    /// Plays `songs` from `start` (with `shuffle`, from wherever shuffle starts), on the device playing.
    /// Provider songs are dropped unless picked: the server downloads whatever is requested. `from` is the
    /// page the songs are the list of (`playlist_set`).
    pub fn play(&self, songs: Vec<Song>, start: usize, shuffle: bool, from: Option<PageOrigin>) {
        self.play_kept(songs, start, shuffle, from, false);
    }

    /// Starts a jam around `songs[start]`: they play here, then the jam opens. Its invite link (None when
    /// one is open or opening already); nothing while remote control is off.
    pub async fn start_jam(&self, songs: Vec<Song>, start: usize, from: Option<PageOrigin>) -> Option<Result<Option<String>, NetError>> {
        let remote = self.remote()?;
        self.play(songs, start, false, from);
        Some(remote.jam_open().await)
    }

    /// `songs` as the queue around the song playing, `songs[at]`: it goes on where it is, playing or
    /// paused, rather than starting again (a tap on it in a list). Any other song plays as [`Handle::play`]
    /// would. True when it went on.
    pub fn keep_playing(&self, songs: Vec<Song>, at: usize, from: Option<PageOrigin>) -> bool {
        let playing = self.engine.status_with(|s| s.id.clone());
        let keep = playing.is_some() && songs.get(at).map(|s| &s.id) == playing.as_ref() && self.elsewhere().is_none();
        self.play_kept(songs, at, false, from, keep);
        keep
    }

    /// Fetches `what`'s songs among the session's tasks and hands them to `then`; says so when that fails.
    fn fetched(&self, what: Fetch, then: impl FnOnce(&Handle, Vec<Song>) + Send + 'static) {
        let me = self.clone();
        self.run(async move {
            match what.songs(&me.client).await {
                Ok(songs) => then(&me, songs),
                Err(e) => me.note(Note::SongsFailed(e)),
            }
        });
    }

    /// Plays an album, playlist or artist once its songs are fetched; a radio from the song playing
    /// goes on with it.
    pub fn play_later(&self, what: Fetch, shuffle: bool) {
        let origin = what.origin();
        let radio = matches!(what, Fetch::Radio(_));
        self.fetched(what, move |me, songs| match songs.is_empty() {
            false if radio => _ = me.keep_playing(songs, 0, Some(origin)),
            false => me.play(songs, 0, shuffle, Some(origin)),
            true => me.note(Note::NothingToPlay),
        });
    }

    /// Adds songs after the current one (`next`) or at the end, on the device playing.
    pub fn enqueue(&self, songs: Vec<Song>, next: bool) {
        self.add(songs, next);
    }

    /// Enqueues an album, playlist or artist once fetched.
    pub fn enqueue_later(&self, what: Fetch, next: bool) {
        self.fetched(what, move |me, songs| me.add(songs, next));
    }

    /// The account's active device while it is another one, which the player shows and every control
    /// here acts on.
    pub fn elsewhere(&self) -> Option<crate::remote::Elsewhere> {
        self.remotes.elsewhere()
    }

    /// The jam this profile is a guest in, as the player shows it in place of this device's own playback:
    /// the host's song, its queue around it and the playhead. A guest's controls reach no device.
    pub fn jam_playing(&self) -> Option<crate::remote::Elsewhere> {
        self.remotes.jam_playing()
    }

    /// Plays or pauses; a queue restored but never started starts where it was.
    pub fn toggle(&self) {
        if self.there(Press::Toggle) {
            return;
        }
        match self.restored() {
            Some((at, ms)) => {
                self.engine.play_at(at, ms);
            }
            None => self.engine.toggle(),
        }
    }

    /// Plays; a queue restored but never started starts where it was.
    #[cfg(feature = "desktop")]
    pub(crate) fn resume(&self) {
        if self.there(Press::Play) {
            return;
        }
        match self.restored() {
            Some((at, ms)) => self.engine.play_at(at, ms),
            None => self.engine.play(),
        };
    }

    /// Pauses, on the device playing.
    #[cfg(feature = "desktop")]
    pub(crate) fn halt(&self) {
        if !self.there(Press::Pause) {
            self.engine.pause();
        }
    }

    /// The song of a queue restored but never started, and where it was.
    fn restored(&self) -> Option<(usize, i64)> {
        let st = self.engine.status();
        self.core.session.playlist(|p| p.current()).filter(|_| st.state == State::Idle).map(|at| (at, st.position_now()))
    }

    pub fn previous(&self) {
        if !self.there(Press::Previous) {
            self.engine.previous();
        }
    }

    pub fn seek(&self, ms: i64) {
        if !self.there(Press::Seek { ms }) {
            self.engine.seek(ms);
        }
    }

    /// Plays the song at list index `index`.
    pub fn jump(&self, index: usize) {
        if !self.there(Press::Jump { index: index as u32 }) {
            self.engine.play_at(index, 0);
        }
    }

    /// Sends `press` to the active device while it is another one; false while this one plays. A jam
    /// guest's go by its role ([`nori_core::remote::Remote::jam_press`]): to the host, carried out here
    /// (false), or nowhere; its volume is its own.
    fn there(&self, press: Press) -> bool {
        if self.guest {
            let jam = self.jam_playing();
            return match jam.as_ref().and_then(|j| Some((j, j.op(press)?))) {
                Some((j, op)) => j.jam_press(op) != Reach::Here,
                None => !matches!(press, Press::Volume { .. }),
            };
        }
        let Some(e) = self.elsewhere() else { return false };
        if let Some(op) = e.op(press) {
            e.send(op);
        }
        true
    }

    /// The remote control and jams, while switched on: other devices to control, the jam hosted.
    pub fn remote(&self) -> Option<Arc<nori_core::remote::Remote>> {
        self.remotes.get()
    }

    /// The app came into sight or left it: a mirrored device's clock and the search for a playing
    /// device follow only while it is seen.
    pub fn in_sight(&self, seen: bool) {
        let Some(remote) = self.remote() else { return };
        remote.clone().sight(if seen { nori_core::remote::Sight::Screen } else { nori_core::remote::Sight::Nothing });
        if seen {
            remote.discover_active(true);
        }
    }

    /// Makes or drops the remote as the settings say, and serves while remote control is on.
    fn follow_remote(&self) {
        let prefs = self.core.session.settings.current().unwrap_or_default();
        if self.offline || !self.rules().remote || !(prefs.remote_control || self.guest) {
            return self.remotes.set(None);
        }
        let remote = self.remotes.get().unwrap_or_else(|| {
            #[cfg(feature = "desktop")]
            let mdns = self.discovery.is_none().then(crate::remote::Mdns::start).flatten();
            #[cfg(feature = "desktop")]
            let discovery = self.discovery.clone().or_else(|| mdns.clone().map(|m| m as Arc<dyn nori_core::remote::Discovery>));
            #[cfg(not(feature = "desktop"))]
            let discovery = self.discovery.clone();
            let player = Arc::new(crate::remote::HostPlayer(self.clone()));
            let shown = crate::remote::Shown { out: self.out.clone(), remotes: self.remotes.clone(), engine: self.engine.clone() };
            let r = nori_core::remote::Remote::with_tasks(self.client.clone(), self.device.clone(), player, Arc::new(shown), discovery, self.tasks.clone());
            #[cfg(feature = "desktop")]
            if let Some(m) = &mdns {
                m.serve(&r);
            }
            // A guest listening along plays its jam's music here.
            if self.guest {
                r.follow_with(Some(Arc::new(crate::remote::Along { engine: self.engine.clone(), session: self.core.session.clone() })));
            }
            self.remotes.set(Some(r.clone()));
            self.remotes.played(&self.engine);
            if prefs.remote_control && !self.guest {
                r.clone().discover_active(true);
            }
            r
        });
        // A guest is no device of the account's; it follows its jam.
        remote.clone().serve(prefs.remote_control && !self.guest);
        if self.guest {
            remote.clone().watch(true);
        }
        self.remotes.played(&self.engine);
    }

    /// Removes the song at list index `index` on the device playing; if it was playing, the next one
    /// takes its place.
    pub fn remove(&self, index: usize) {
        if !self.there(Press::Remove { index: index as u32 }) {
            self.remove_here(index);
        }
    }

    /// Removes everything after the current song, on the device playing.
    pub fn clear_upcoming(&self) {
        if self.there(Press::Clear) {
            return;
        }
        for i in self.core.session.playlist(|p| p.after_current()) {
            self.core.session.remove(i as u32, i as u32 + 1);
        }
        self.edited();
    }

    /// Undoes the removal of `id` where it was removed (here, or on the device playing); the playing
    /// song is unchanged.
    pub fn put_back(&self, id: &str) {
        let back = match self.elsewhere() {
            Some(e) => e.put_back(id),
            None => self.restore_here(id),
        };
        if !back {
            self.note(Note::NothingToPutBack);
        }
    }

    /// Moves the song at list index `from` to `to`, on the device playing.
    pub fn move_song(&self, from: usize, to: usize) {
        if !self.there(Press::Move { from: from as u32, to: to as u32 }) {
            self.move_here(from, to);
        }
    }

    pub fn shuffle(&self, on: bool) {
        if !self.there(Press::Shuffle { on }) {
            self.shuffle_here(on);
        }
    }

    /// A page's Shuffle (`pages::hero_buttons`): while the queue is the one `page` started, turns its
    /// shuffle off or on, the song playing going on. False when it is not: the page's songs start shuffled.
    pub fn shuffle_page(&self, page: &PageOrigin) -> bool {
        let q = &self.core.session;
        let here = q.origin().as_ref() == Some(page);
        match nori_core::pages::hero_buttons(here, q.shuffle_shown(), false, false, true, true).shuffle_press {
            nori_core::pages::HeroPress::ShuffleOff => self.shuffle(false),
            nori_core::pages::HeroPress::ShuffleOn => self.shuffle(true),
            nori_core::pages::HeroPress::Start | nori_core::pages::HeroPress::Toggle => return false,
        }
        true
    }

    /// A page's Shuffle and Play as they stand (`pages::hero_buttons`): for the queue `page` started while it
    /// is the one in play, `playing` or `buffering`.
    pub fn page_buttons(&self, page: &PageOrigin, playing: bool, buffering: bool) -> nori_core::pages::HeroButtons {
        let q = &self.core.session;
        nori_core::pages::hero_buttons(q.origin().as_ref() == Some(page), q.shuffle_shown(), playing, buffering, true, true)
    }

    pub fn repeat(&self, mode: u8) {
        if !self.there(Press::Repeat { mode }) {
            self.repeat_here(mode);
        }
    }

    /// The next repeat mode after the one shown (`nori_core::rules::queue_next_repeat`), on the device playing.
    pub fn cycle_repeat(&self) {
        let now = match self.playback() {
            crate::player::Playback::Here => self.core.session.playlist(|p| p.repeat()),
            crate::player::Playback::There(e) | crate::player::Playback::Jam(Some(e)) => e.mirror.repeat,
            crate::player::Playback::Jam(None) => return,
        };
        self.repeat(nori_core::rules::queue_next_repeat(now));
    }

    fn start_downloads(&self) {
        if let Some(d) = &self.downloader {
            d.start(nori_core::transfers::DOWNLOADS_AT_ONCE);
        }
    }

    /// Queues `songs` for download (their covers fetched to disk too) and starts the downloader.
    pub fn download(&self, songs: Vec<Song>) {
        self.note(queue_downloads(&self.core, self.covers.as_ref(), songs));
        self.start_downloads();
    }

    /// [`Handle::download`] of what `what` names, once fetched.
    pub fn download_later(&self, what: Fetch) {
        self.fetched(what, |me, songs| me.download(songs));
    }

    /// Where song `id`'s download stands.
    pub fn download_state(&self, id: &str) -> nori_core::menus::SongDownload {
        self.download_states(&[id])[0]
    }

    /// Where each song's download stands, the queue read once for them all.
    pub fn download_states(&self, ids: &[&str]) -> Vec<nori_core::menus::SongDownload> {
        use nori_core::menus::SongDownload;
        let waiting = self.core.download_waiting().unwrap_or_default();
        ids.iter()
            .map(|id| if self.core.download_song(id).is_some() { SongDownload::Done } else if waiting.contains(*id) { SongDownload::Pending } else { SongDownload::None })
            .collect()
    }

    /// Deletes a download from disk.
    pub fn download_remove(&self, id: &str) {
        let _ = self.core.download_remove(id.to_string());
        if let Some(store) = &self.store {
            let _ = std::fs::remove_file(store.download_path(id));
        }
    }

    /// Sets a setting by name and applies its effects, as Android's player does. None if no such setting.
    pub fn setting(&self, name: &str, value: &str) -> Option<SettingChange> {
        let change = self.core.session.settings.edit_by_name(name, value)?;
        self.apply(change.effect, &change.prefs);
        if matches!(name, "remoteControl" | "jam") {
            self.follow_remote();
        }
        if let Some(store) = self.store.as_ref().filter(|_| change.effect & CACHE_LIMIT != 0) {
            store.set_limit(change.prefs.cache_mb.max(0) as u64 * 1024 * 1024);
        }
        if change.server {
            // Per-server settings (music folder, second address bitrate) live on the profile.
            if let Some(s) = change.prefs.active_server() {
                self.client.set_profile(net(s));
            }
        }
        Some(change)
    }

    /// The system moved the volume by itself (its keys, where it keeps the volume), 0 to 1: the
    /// account's other devices see it.
    pub fn volume_followed(&self, v: f32) {
        self.set_level(v);
    }

    /// The volume, 0 to 1.
    pub fn volume(&self) -> f32 {
        self.level.get()
    }

    /// Sets the volume of the device playing, as the client's own control does; the account's other
    /// devices see it.
    pub fn set_volume(&self, v: f32) {
        if !self.there(Press::Volume { level: v }) {
            self.set_level(v);
        }
    }

    /// Applies a settings change's `effect` bits to the engine.
    pub fn apply(&self, effect: u32, prefs: &StoredPrefs) {
        if effect & (APPLY_AUDIO | SOUND | PLAYER) != 0 {
            self.engine.set_settings(settings(prefs, self.core.session.settings.layer()));
        }
        if effect & APPLY_GAIN != 0 {
            self.engine.gain_changed();
        }
        if effect & REPLAN != 0 {
            self.engine.replan();
        }
    }

    /// Does what a device step said (`DeviceEffect`): loads the sound it gives and, when it asks, has the
    /// device arrive again.
    pub fn device_effect(&self, output: &str, effect: DeviceEffect) {
        let settings = &self.core.session.settings;
        if let Some(prefs) = effect.apply.and_then(|s| Some(settings.current()?.with_sound(s))) {
            let bits = settings.put(prefs.clone());
            self.apply(bits, &prefs);
        }
        if effect.arrive {
            self.device_effect(output, self.core.device_arrive(output.to_string()));
            self.device_curve(output.to_string());
        }
    }

    /// The AutoEQ step of `output`'s arrival (`Client::device_curve`), among the session's tasks.
    fn device_curve(&self, output: String) {
        let me = self.clone();
        // Unmetered: the AutoEQ list waits for no network these clients can tell apart.
        self.run(async move {
            let c = me.client.device_curve(output.clone(), false).await;
            me.device_curve_done(&output, c);
        });
    }

    /// Performs an AutoEQ step's effect and says its notice, kept for [`Handle::curve_answer`].
    fn device_curve_done(&self, output: &str, c: DeviceCurve) {
        self.device_effect(output, c.effect);
        *self.curve.lock() = c.notice.clone();
        if let Some(n) = c.notice {
            (self.out)(Said::Curve(n));
        }
    }

    /// The answer to the last [`Said::Curve`]: an offered curve applied, or one applied without asking
    /// undone.
    pub fn curve_answer(&self) {
        let Some(notice) = self.curve.lock().take() else { return };
        match notice {
            CurveNotice::Offer { output, entry } => {
                let me = self.clone();
                self.run(async move {
                    match me.client.device_accept(output.clone(), entry).await {
                        Ok(c) => me.device_curve_done(&output, c),
                        Err(e) => me.note(Note::CurveFailed(e)),
                    }
                });
            }
            CurveNotice::Applied { output, curve, before, created } => {
                let effect = self.core.device_undo(output.clone(), curve, created, before);
                self.device_effect(&output, effect);
            }
        }
    }

    /// Applies `effect` with the stored settings (after an in-place edit such as an equalizer band).
    pub fn applied(&self, effect: u32) {
        if let Some(p) = self.core.session.settings.current() {
            self.apply(effect, &p);
        }
    }

    /// A lyrics view opened or closed: the core warms the songs ahead's lyrics only while one is open.
    pub fn lyrics_view(&self, open: bool) {
        self.client.lyrics_view(open);
        if open {
            let client = self.client.clone();
            self.run(async move { client.lyrics_prefetch().await });
        }
    }

    /// Fetches lyrics for `song`; each better answer arrives as [`Said::Lyrics`].
    pub fn lyrics(&self, song: String) {
        let (client, out) = (self.client.clone(), self.out.clone());
        self.run(async move {
            let _ = client.lyrics_for(song.clone(), Arc::new(Shown { song, out })).await;
        });
    }

    /// Requests cover `id` at `px` square: the core's rendition for that size is fetched
    /// ([`nori_core::covers::cover_rendition`]) and decoded to `px`. `done` gets the image or failure and,
    /// when `colours`, its page colors, derived off the caller's thread (a memory hit answers at once). None
    /// without a cover loader.
    pub fn cover(&self, id: String, px: u32, colours: bool, done: impl FnOnce(Result<Arc<Image>, nori_covers::Error>, Option<Box<CoverColours>>) + Send + 'static) -> Option<Ticket> {
        let url = self.core.cover_address(id, nori_core::covers::cover_rendition(px));
        Some(self.covers.as_ref()?.request(&url, px, px, move |r| match r {
            Ok(image) if colours => crate::spawn("nori-colours", move || {
                let colours = Box::new(derive(&image));
                done(Ok(image), Some(colours));
            }),
            r => done(r, None),
        }))
    }

    /// Searches the offline index for the typed text.
    pub fn search_typed(&self, text: &str) -> SearchView {
        let view = self.search.typed(text.to_string());
        if view.query.is_empty() {
            return view;
        }
        let limit = nori_core::browse::library_sizes().local_search;
        self.search.local(self.core.clone(), view.query.clone(), limit).ok().flatten().unwrap_or(view)
    }

    /// Searches the server in the background. `error` words a failure for the results.
    pub fn search_server(&self, query: String, error: fn(&NetError) -> String) {
        if self.offline || query.trim().is_empty() {
            return;
        }
        let _ = self.core.search_remember_recent(query.clone());
        let (search, client, out) = (self.search.clone(), self.client.clone(), self.out.clone());
        self.run(async move {
            match search.ask(client, query.clone()).await {
                Ok(Some(v)) => out(Said::Search(v)),
                Ok(None) => {}
                Err(e) => {
                    if let Some(v) = search.failed(query, Some(error(&e))) {
                        out(Said::Search(v));
                    }
                }
            }
        });
    }

    /// Signs the profile's account in with `answer` (cookies pasted from a browser); signed in, the
    /// profile keeps the account's name, which a session opened on it again reads as signed in.
    pub fn sign_in(&self, answer: String) -> nori_core::providers::SignInResult {
        let r = block_on(self.client.sign_in(answer));
        if let nori_core::providers::SignInResult::SignedIn { name } = &r {
            self.keep_account(name);
        }
        r
    }

    /// What the app offers over the profile (a guest's: the host's library, read only), as the core
    /// has it now.
    pub fn rules(&self) -> ProfileRules {
        self.core.rules()
    }

    /// The account as it is, once its rules changed since the client was last told (a source ended the
    /// account's session, or took it up again); None while they stand.
    pub fn account_changed(&self) -> Option<nori_core::providers::AccountState> {
        let now = self.core.rules();
        let mut shown = self.rules_shown.lock();
        (now != *shown).then(|| {
            *shown = now;
            self.client.account()
        })
    }

    /// Signs the profile's account out here; the profile forgets the account's name.
    pub fn sign_out(&self) {
        self.client.sign_out();
        self.keep_account("");
    }

    fn keep_account(&self, name: &str) {
        let settings = &self.core.session.settings;
        let Some(mut prefs) = settings.current() else { return };
        let id = prefs.active_server_id.clone();
        if let Some(s) = prefs.servers.iter_mut().find(|s| s.id == id) {
            s.user = name.to_string();
            settings.put(prefs);
        }
    }

    /// Stars or unstars on the server (queued when offline); says so when it fails.
    pub fn star(&self, kind: Starrable, id: String, on: bool) {
        if !self.rules().stars {
            return;
        }
        let (client, out, hearts) = (self.client.clone(), self.out.clone(), self.hearts());
        self.run(async move {
            if let Err(e) = client.star(kind, id, on, hearts).await {
                out(Said::Note(Note::StarFailed(e)));
            }
        });
    }

    /// Whether `song` shows starred: as `there` (the device playing, while it is another one) shows it
    /// when the song is in its queue, else this session's mark over the song's record.
    pub fn starred(&self, song: &Song, there: Option<&crate::remote::Elsewhere>) -> bool {
        there.and_then(|e| e.starred(&song.id)).unwrap_or_else(|| self.core.starred(Starrable::Song, &song.id, song.starred))
    }

    /// Whether song `id` shows starred, its record saying `listed`, as [`Handle::starred`] does with the
    /// device playing elsewhere, without copying that device's queue.
    pub fn song_starred(&self, id: &str, listed: bool) -> bool {
        self.remotes.get().and_then(|r| r.mirrored_starred(id)).unwrap_or_else(|| self.core.starred(Starrable::Song, id, listed))
    }

    /// Stars or unstars a song once: through `there` while the song is in its queue (that device tells
    /// the server and shows it), else from here.
    pub fn star_song(&self, id: String, on: bool, there: Option<&crate::remote::Elsewhere>) {
        if !there.is_some_and(|e| e.star(&id, on)) {
            self.star(Starrable::Song, id, on);
        }
    }

    /// Runs a settings page's maintenance button.
    pub fn action(&self, chore: Chore) {
        match chore {
            Chore::SyncLibrary => return self.sync(),
            Chore::DownloadLibrary => {
                let r = self.core.download_queue_library();
                self.start_downloads();
                return match r {
                    Ok(q) => self.note(Note::Downloading(q.fresh.len() + q.again.len())),
                    Err(e) => self.note(Note::DownloadFailed(e)),
                };
            }
            Chore::ClearStream => {
                if let Some(store) = &self.store {
                    store.clear_cache();
                }
            }
            Chore::ClearLyrics => self.core.lyrics_cache_clear(),
            Chore::ClearCovers => {
                if let Some(d) = self.covers.as_ref().and_then(|l| l.disk()) {
                    d.clear();
                }
            }
            Chore::MeasureAgain => {
                let Ok(m) = self.core.measure_again() else { return };
                self.applied(m.effect);
                return self.note(Note::Forgot(m.forgot));
            }
        }
        self.note(Note::Done(chore));
    }

    /// Feeds an engine event to the core: scrobbling, queue saves and refills, the offline bridge.
    fn followed(&self, e: &Event) {
        use nori_core::scrobble::TrackChange;
        let q = &self.core.session;
        let (now, wall) = (monotonic_ms(), nori_core::db::now_ms());
        let tz = (nori_core::library::local_offset_s(wall / 1000) * 1000) as i32;
        let playing = self.engine.status_with(|s| s.state == State::Playing);
        let send = match e {
            Event::Song { id, .. } => Some(q.scrobble_track(Some(id.clone()), TrackChange::Moved, playing, now, wall, tz)),
            Event::Looped { id, .. } => Some(q.scrobble_track(Some(id.clone()), TrackChange::Looped, playing, now, wall, tz)),
            Event::State(State::Ended) => Some(q.scrobble_track(None, TrackChange::Ended, false, now, wall, tz)),
            Event::State(s) => {
                q.scrobble_playing(*s == State::Playing, now);
                None
            }
            _ => None,
        };
        if let Some(send) = send.filter(|s| self.rules().scrobbles && (s.submit_id.is_some() || s.now_playing_id.is_some())) {
            let client = self.client.clone();
            self.run(async move {
                // Offline, writes wait in the pending queue with their original time.
                if let Some(id) = send.submit_id {
                    let _ = client.write(nori_core::client::Write::Scrobble { id, submission: true, time_ms: Some(send.submit_at) }).await;
                }
                if let Some(id) = send.now_playing_id {
                    let _ = client.write(nori_core::client::Write::Scrobble { id, submission: false, time_ms: None }).await;
                }
            });
        }
        match e {
            Event::Song { .. } => self.arrived(),
            Event::State(State::Ended) => self.ended(),
            Event::State(State::Paused) => self.keep(QueueMoment::Paused),
            Event::Bridge { .. } => self.bridge(),
            Event::Output { name } => self.device_curve(name.clone()),
            _ => {}
        }
        if matches!(e, Event::Song { .. } | Event::Looped { .. } | Event::State(_) | Event::Position { .. } | Event::Placed { .. } | Event::Mixing(_)
                | Event::Buffering(_)) {
            self.remotes.played(&self.engine);
        }
    }

    /// Runs the core's steps for a new song (`nori_queue::Session::song_arrived`).
    fn arrived(&self) {
        let steps = self.core.session.song_arrived();
        self.keeper.later(steps.save_after_ms);
        let client = self.client.clone();
        self.run(async move { client.lyrics_prefetch().await });
        if steps.fill {
            self.refill();
        }
        if matches!(steps.bridge, BridgeStep::Bridging | BridgeStep::Parked | BridgeStep::Left) {
            // Each asks whether the server answers. Bridging: the bridge's songs ahead go. Parked: back
            // to the parked song, else bridged further. Left: the bridge's songs go.
            let me = self.clone();
            let step = steps.bridge;
            self.run(async move {
                let edit = match step {
                    BridgeStep::Bridging => me.client.bridge_ahead().await,
                    BridgeStep::Parked => me.client.bridge_parked().await,
                    _ => me.client.bridge_left().await,
                };
                if let Some(edit) = edit {
                    me.apply_edit(&edit);
                }
            });
        }
        if steps.pause_at_end {
            self.engine.pause_at_end(true);
        }
    }

    /// Hands an unreachable song to the core's offline bridge and applies its decision.
    fn bridge(&self) {
        match self.core.bridge_take() {
            BridgeTake::Jump { index } => {
                self.engine.play_at(index as usize, 0);
            }
            BridgeTake::Bridged { edit } => self.apply_edit(&edit),
            BridgeTake::Skip => {
                self.engine.next();
                self.engine.play();
            }
            BridgeTake::Stop => {}
        }
    }

    /// Appends autofill songs; a next pressed while fetching is applied when they land.
    fn refill(&self) {
        let me = self.clone();
        self.run(async move {
            let fresh = me.client.autofill().await;
            me.filled(fresh);
        });
    }

    /// Next; at the queue's end with autofill on, fetches songs first.
    pub fn next(&self) {
        if self.there(Press::Next) {
            return;
        }
        if self.core.session.playlist(|p| p.next().is_some()) {
            self.engine.next();
            return;
        }
        match self.core.session.autofill_next() {
            nori_core::autofill::FillNext::Skip => {
                self.engine.next();
            }
            nori_core::autofill::FillNext::Fetch => self.refill(),
            nori_core::autofill::FillNext::Wait => {}
        }
    }

    /// The queue played out: a plugin may say what plays now (`Client::queue_ended`).
    fn ended(&self) {
        if self.core.session.deciders().is_none() {
            return;
        }
        let me = self.clone();
        self.run(async move {
            let fresh = me.client.queue_ended().await;
            let ids: Vec<String> = fresh.songs.iter().map(|s| s.id.clone()).collect();
            let q = &me.core.session;
            let (len, end) = q.playlist(|p| (p.len(), p.play_order().last().map(|at| p.seqs()[at])));
            // A queue started meanwhile plays instead.
            if ids.is_empty() || end != fresh.after {
                return;
            }
            let n = ids.len();
            q.take(len as u32, ids, vec![Hand::No; n]);
            me.edited();
            if let Some(end) = end {
                me.engine.resume_after_refill(end);
            }
        });
    }

    /// A media key or button pressed: what it does is the core's (`Session::media_button`).
    pub fn pressed(&self, press: nori_core::deciders::Press) {
        use nori_core::deciders::ButtonAct;
        match self.core.session.media_button(press) {
            ButtonAct::PlayPause => self.toggle(),
            ButtonAct::Next => self.next(),
            ButtonAct::Previous => self.previous(),
            ButtonAct::Nothing => {}
        }
    }

    fn filled(&self, fresh: nori_core::autofill::Refill) {
        let ids: Vec<String> = fresh.songs.iter().map(|s| s.id.clone()).collect();
        if !self.client.autofill_arrived(fresh) || ids.is_empty() {
            return;
        }
        let q = &self.core.session;
        let (len, end) = q.playlist(|p| (p.len(), p.play_order().last().map(|at| p.seqs()[at])));
        let n = ids.len();
        q.take(len as u32, ids, vec![Hand::No; n]);
        self.edited();
        if q.autofill_landed() {
            self.engine.next();
        } else if let Some(end) = end {
            self.engine.resume_after_refill(end);
        }
    }

    /// Sets the volume here; the account's other devices see it.
    fn set_level(&self, v: f32) {
        self.level.set(v);
        self.remotes.played(&self.engine);
    }

    /// Another device set the volume: as [`Handle::set_level`], and the client is told.
    pub(crate) fn volume_from_afar(&self, v: f32) {
        self.set_level(v);
        (self.out)(Said::Volume(self.level.get()));
    }

    /// The marks moving, as this session shows them.
    fn hearts(&self) -> Arc<Hearts> {
        Arc::new(Hearts { out: self.out.clone(), remotes: self.remotes.clone(), engine: self.engine.clone() })
    }

    /// Another device favourited a song (or not): the server is told, and the core's marks follow; the
    /// devices see it once marked.
    pub(crate) fn star_from_afar(&self, id: String, on: bool) {
        let (client, hearts) = (self.client.clone(), self.hearts());
        self.run(async move {
            if let Err(e) = client.star(Starrable::Song, id, on, hearts).await {
                nori_core::alog::info(&format!("remote: star not kept: {e}"));
            }
        });
    }

    /// After a queue edit: tells the engine and schedules a save.
    fn edited(&self) {
        self.engine.queue_changed();
        self.keeper.later(queue_keep(QueueMoment::Edited).save_after_ms);
        let client = self.client.clone();
        self.run(async move { client.lyrics_prefetch().await });
        self.remotes.played(&self.engine);
    }

    /// Removes the song at list index `index` here; if it was playing, the next one takes its place.
    pub(crate) fn remove_here(&self, index: usize) {
        let (current, playing) = self.engine.status_with(|s| (s.index, s.state == State::Playing));
        let change = self.core.session.remove(index as u32, index as u32 + 1);
        self.edited();
        if let (true, Some(at)) = (current == Some(index), change.at) {
            if playing {
                self.engine.play_at(at as usize, 0);
            } else {
                self.engine.go_to(at as usize, 0);
            }
        }
    }

    /// Puts back `id` where it was taken out; false when it was not the last song removed.
    fn restore_here(&self, id: &str) -> bool {
        let q = &self.core.session;
        let was_empty = q.playlist(|p| p.is_empty());
        if q.restore(id.to_string()).at.is_none() {
            return false;
        }
        self.edited();
        if was_empty {
            self.engine.go_to(0, 0);
        }
        true
    }

    /// Another device's undo: `song` back where it was taken out, else at list index `index`.
    pub(crate) fn put_back_from_afar(&self, song: Song, index: usize) {
        if self.restore_here(&song.id) {
            return;
        }
        let q = &self.core.session;
        let (len, id) = (q.playlist(|p| p.len()), song.id.clone());
        q.register(vec![song]);
        q.take(index.min(len) as u32, vec![id], vec![Hand::No]);
        self.edited();
        if len == 0 {
            self.engine.go_to(0, 0);
        }
    }

    pub(crate) fn move_here(&self, from: usize, to: usize) {
        self.core.session.move_range(from as u32, from as u32 + 1, to as u32);
        self.edited();
    }

    pub(crate) fn shuffle_here(&self, on: bool) {
        self.core.session.show_shuffle(on);
        self.core.session.shuffle(on);
        self.edited();
    }

    pub(crate) fn repeat_here(&self, mode: u8) {
        // Set on the queue first so the screen shows it at once.
        self.core.session.repeat(mode);
        self.engine.set_repeat(mode);
    }

    /// A queue handed over from another device: `songs` from `index` at `position_ms`, playing or not,
    /// shuffled into `order` under `shuffle`, repeating as `repeat` says.
    #[allow(clippy::too_many_arguments, reason = "the transfer's own fields, as they came")]
    pub(crate) fn replace(&self, songs: Vec<Song>, index: usize, position_ms: i64, play: bool, order: Option<Vec<u32>>, shuffle: bool, repeat: u8) {
        if songs.is_empty() {
            return;
        }
        let q = &self.core.session;
        q.register(songs.clone());
        let change = q.handed(songs.iter().map(|s| s.id.clone()).collect(), index.min(songs.len() - 1) as u32, order, shuffle, repeat);
        self.engine.set_repeat(repeat);
        self.edited();
        let at = change.at.unwrap_or(0) as usize;
        if play {
            self.engine.play_at(at, position_ms);
        } else {
            self.engine.go_to(at, position_ms);
        }
    }

    /// Applies a queue edit the core made itself (the offline bridge).
    fn apply_edit(&self, e: &QueueEdit) {
        self.edited();
        if let Some(seek) = e.seek {
            self.engine.play_at(seek as usize, 0);
        }
    }

    /// [`Handle::play`]; `keep`: the start is the song playing, which goes on with no jump (the queue's
    /// `set` keeps its entry). Sent to the active device while it is another one.
    fn play_kept(&self, songs: Vec<Song>, start: usize, shuffle: bool, from: Option<PageOrigin>, keep: bool) {
        let picked = songs.get(start).map(|s| s.id.clone());
        let songs: Vec<Song> = songs.into_iter().filter(|s| !s.is_provider() || Some(&s.id) == picked.as_ref()).collect();
        if songs.is_empty() {
            return;
        }
        let start = picked.and_then(|id| songs.iter().position(|s| s.id == id)).unwrap_or(0);
        if self.guest {
            let Some(r) = self.remotes.get() else { return };
            if r.jam_controls().is_some_and(|c| !c.asks) {
                let repeat = r.jam_playing().map_or(0, |m| m.repeat);
                r.jam_press(Op::Replace { songs, index: start as u32, position_ms: 0, play: true, order: None, shuffle, repeat, transfer: None, listen: None, mix: None });
            } else {
                self.ask(songs.into_iter().skip(start).take(1).collect());
            }
            return;
        }
        if let Some(e) = self.remotes.elsewhere() {
            return e.play(songs, start, shuffle);
        }
        let q = &self.core.session;
        q.register(songs.clone());
        let change = q.set(songs.iter().map(|s| s.id.clone()).collect(), (!shuffle).then_some(start as u32), shuffle, from);
        self.edited();
        if !keep {
            self.engine.play_at(change.at.unwrap_or(0) as usize, 0);
        }
    }

    /// [`Handle::enqueue_here`] on the device playing: the active device while it is another one.
    fn add(&self, songs: Vec<Song>, next: bool) {
        if self.guest {
            let Some(r) = self.remotes.get() else { return };
            let songs = queueable(songs);
            if r.jam_controls().is_some_and(|c| !c.asks) {
                r.jam_press(Op::Add { songs, next });
            } else {
                self.ask(songs);
            }
            return;
        }
        let Some(e) = self.remotes.elsewhere() else { return self.enqueue_here(songs, next) };
        let songs = queueable(songs);
        if !songs.is_empty() {
            e.send(Op::Add { songs, next });
        }
    }

    /// A jam guest asks the host for `songs`, each a request of its own.
    fn ask(&self, songs: Vec<Song>) {
        let Some(r) = self.remotes.get() else { return };
        for song in songs {
            r.clone().jam_act(Op::Request { song });
        }
    }

    pub(crate) fn enqueue_here(&self, songs: Vec<Song>, next: bool) {
        let songs = queueable(songs);
        if songs.is_empty() {
            return;
        }
        let n = songs.len();
        let q = &self.core.session;
        q.register(songs.clone());
        let (len, current) = q.playlist(|p| (p.len(), p.current()));
        let at = if next { current.map_or(len, |c| c + 1) } else { len };
        let hand = if next { Hand::Next } else { Hand::Last };
        q.take(at as u32, songs.iter().map(|s| s.id.clone()).collect(), vec![hand; n]);
        self.edited();
        if len == 0 {
            self.engine.go_to(0, 0);
        }
        self.note(Note::Queued { next, songs: n });
    }
}


/// `songs` as they may be added to a queue: a single picked song may be a provider's; lists never include
/// them (the server downloads whatever is requested).
fn queueable(songs: Vec<Song>) -> Vec<Song> {
    if songs.len() == 1 { songs } else { songs.into_iter().filter(|s| !s.is_provider()).collect() }
}

/// Queues `songs` but providers' for download, their covers fetched to disk too; what to say of it.
fn queue_downloads(core: &Core, covers: Option<&Arc<Loader>>, songs: Vec<Song>) -> Note {
    let songs: Vec<Song> = songs.into_iter().filter(|s| !s.is_provider()).collect();
    warm_covers(core, covers, &songs);
    match core.download_queue(songs) {
        Ok(q) => Note::Downloading(q.fresh.len() + q.again.len()),
        Err(e) => Note::DownloadFailed(e),
    }
}

fn warm_covers(core: &Core, covers: Option<&Arc<Loader>>, songs: &[Song]) {
    let Some(loader) = covers else { return };
    for url in core.download_cover_urls(songs.iter().filter_map(|s| s.cover_art.clone()).collect()) {
        loader.warm(&url);
    }
}

/// Runs `Client::read_each` (the stored copy, then the server's if different; an error only when
/// nothing is stored), handing each page to `each`.
pub fn read_pages(client: &Client, read: Read, each: impl FnMut(Page)) -> Result<(), NetError> {
    block_on(client.read_each(read, each))
}

/// The same monotonic clock remote handoffs use to close a listen.
fn monotonic_ms() -> i64 {
    nori_remote::clock::now_us() / 1_000
}

#[cfg(test)]
#[path = "refill_tests.rs"]
mod refill_tests;

/// A session's opening for a test: online, playing into `output`, reporting nowhere.
#[cfg(test)]
fn test_open<'a>(queue: Arc<nori_core::queue::Session>, data: &'a Path, profile: SavedServer, output: Box<dyn AudioOutput>) -> Open<'a> {
    Open {
        queue,
        data,
        http: nori_http::Http::new(),
        profile,
        kinds: nori_core::providers::Kinds::builtin(),
        output,
        volume: Level::new(1.0, None),
        memory_mb: 16,
        covers: None,
        offline: false,
        mpris: None,
        device: nori_core::remote::RemoteMe { name: "Mac".into(), kind: nori_remote::wire::DeviceKind::Desktop },
        discovery: None,
        tasks: crate::tasks(),
        keeps_music: true,
        decoders: Default::default(),
        out: Arc::new(|_| {}),
    }
}

#[cfg(test)]
use nori_engine::testing::Device;
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../engine/tests/common/card.rs"]
mod test_card;

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::time::Duration;

    use nori_engine::Loader;

    use super::*;

    #[test]
    fn downloads_said_are_the_ones_queued() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let song = |id: &str| Song { id: id.into(), ..Default::default() };
        core.download_queue(vec![song("done")]).unwrap();
        core.download_settle(vec!["done".into()], vec![true]).unwrap();
        let said = queue_downloads(&core, None, vec![song("done"), song("new"), song("ext-1")]);
        assert!(matches!(said, Note::Downloading(1)), "the finished and the provider's song are not downloaded");
    }

    #[test]
    fn a_page_shuffle_turns_its_own_queue_off_and_on() {
        let dir = nori_testdir::TempDir::new("host-page-shuffle");
        let settings = nori_core::settings_store::Settings::new();
        let mut prefs = settings.open(&db_path(dir.path())).unwrap();
        prefs.remote_control = false;
        settings.put(prefs);
        let queue = Arc::new(nori_core::queue::Session::new(settings));
        let profile = SavedServer { id: "t".into(), user: "Ann".into(), ..Default::default() };
        let s = Session::open(Open { offline: true, ..test_open(queue, dir.path(), profile, Box::new(nori_engine::wav::WavOutput::new(dir.join("out.wav"), 1.0))) }).unwrap();
        let (album, other) = (PageOrigin::new(nori_core::OriginKind::Album, "al"), PageOrigin::new(nori_core::OriginKind::Album, "other"));
        let songs: Vec<Song> = ["a", "b", "c", "d"].map(|id| Song { id: id.into(), ..Default::default() }).into();
        s.play(songs, 0, true, Some(album.clone()));
        let playing = || s.core.session.playlist(|p| p.current_id().map(str::to_string));
        let started = playing();
        assert!(!s.shuffle_page(&other), "another page's Shuffle starts its own songs");
        assert!(s.shuffle_page(&album));
        assert!(!s.core.session.shuffle_shown());
        assert_eq!(playing(), started, "the song playing goes on");
        assert_eq!(s.core.session.playlist(|p| p.play_order().collect::<Vec<_>>()), [0, 1, 2, 3]);
        assert!(s.shuffle_page(&album));
        assert!(s.core.session.shuffle_shown());
        assert_eq!(playing(), started);
        s.close();
    }

    /// A profile opens on its own saved queue or none, not on the one the profile open before left (a
    /// jam's host's songs, once the guest left).
    #[test]
    fn a_profile_opens_without_the_last_ones_queue() {
        let dir = nori_testdir::TempDir::new("host-queue");
        let queue = Arc::new(nori_core::queue::Session::new(nori_core::settings_store::Settings::new()));
        queue.register(vec![Song { id: "host-song".into(), duration: 100, ..Default::default() }]);
        queue.set(vec!["host-song".into()], Some(0), false, None);
        let profile = SavedServer { id: "own".into(), url: "http://own".into(), user: "ann".into(), password: "pw".into(), ..Default::default() };
        let s = Session::open(Open { offline: true, ..test_open(queue.clone(), dir.path(), profile, Box::new(nori_engine::wav::WavOutput::new(dir.join("out.wav"), 1.0))) })
        .unwrap();
        assert!(queue.playlist(|p| p.ids().is_empty()), "the host's songs are gone");
        s.close();
    }

    /// The repeat button goes off, all, one, off, as every client shows it.
    #[test]
    fn repeat_cycles_off_all_one() {
        let dir = nori_testdir::TempDir::new("host-repeat");
        let queue = Arc::new(nori_core::queue::Session::new(nori_core::settings_store::Settings::new()));
        let profile = SavedServer { id: "own".into(), url: "http://own".into(), user: "ann".into(), ..Default::default() };
        let s = Session::open(Open { offline: true, ..test_open(queue.clone(), dir.path(), profile, Box::new(nori_engine::wav::WavOutput::new(dir.join("out.wav"), 1.0))) }).unwrap();
        let modes: Vec<u8> = (0..3).map(|_| {
            s.cycle_repeat();
            queue.playlist(|p| p.repeat())
        }).collect();
        assert_eq!(modes, [nori_core::rules::REPEAT_ALL, nori_core::rules::REPEAT_ONE, nori_core::rules::REPEAT_OFF]);
        s.close();
    }

    /// Every provider obeys the shared remote switch when opening a session.
    #[test]
    fn profile_remote_follows_the_switch() {
        let dir = nori_testdir::TempDir::new("host-remote-switch");
        let settings = nori_core::settings_store::Settings::new();
        settings.open(&db_path(dir.path())).unwrap();
        let queue = Arc::new(nori_core::queue::Session::new(settings.clone()));
        let kinds = nori_core::providers::Kinds::builtin();
        for kind in kinds.providers().into_iter().map(|p| p.kind) {
            for on in [false, true] {
                let mut prefs = settings.current().unwrap();
                prefs.remote_control = on;
                settings.put(prefs);
                let profile = SavedServer { id: "own".into(), kind: kind.clone(), url: "http://127.0.0.1:9".into(), user: "ann".into(), ..Default::default() };
                let s = Session::open(test_open(queue.clone(), dir.path(), profile.clone(), Box::new(nori_engine::wav::WavOutput::new(dir.join("out.wav"), 1.0)))).unwrap();
                assert_eq!(s.rules(), kinds.rules_of(profile, false));
                assert_eq!(s.remote().is_some(), on && s.rules().remote);
                s.close();
            }
        }
    }

    /// A song's request the server never answers is dropped with its song: the connection closes.
    #[test]
    fn a_song_let_go_hangs_up_its_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/song", listener.local_addr().unwrap());
        let (asked, closed) = (std::sync::mpsc::channel(), std::sync::mpsc::channel());
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 1024];
            while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
            }
            asked.0.send(()).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            let _ = closed.0.send(s.read(&mut buf).ok());
        });
        let audio = Arc::new(Audio::new(nori_http::Http::new(), false));
        let loader = Loader::start(audio, url, [1_000, 4_000, 0, 0, 1 << 30], None, None);
        asked.1.recv_timeout(Duration::from_secs(10)).expect("the request came");
        drop(loader);
        assert_eq!(closed.1.recv_timeout(Duration::from_secs(5)), Ok(Some(0)), "the client hung up");
    }
}
