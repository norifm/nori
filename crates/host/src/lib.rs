//! What the terminal, desktop and iOS clients share around an open session: the queue saved as it
//! changes, a collection's songs, the offline index, cover colours and the output volume in dB. Media
//! controls (`desktop` feature) sit over the engine. Each client words what happens itself.

pub mod player;
pub mod remote;
pub mod session;

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use nori_time::{Duration, Instant};

use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::remote::JamPass;
use nori_core::settings::SavedServer;
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, NetError};
use nori_core::{Core, OriginKind, PageOrigin, Song};
use nori_tasks::Tasks;
use nori_covers::memory::Image;
#[cfg(feature = "desktop")]
use nori_engine::{State, Status};
use nori_engine::core::OutputVolume;
use nori_engine::Engine;
use nori_look::cover::CoverColours;

/// The database shared by all profiles and the settings.
pub fn db_path(data: &Path) -> String {
    data.join(nori_core::db::DB_FILE).to_string_lossy().into_owned()
}

pub use nori_core::{net_profile as net, server_config as config};

/// This computer's name, as the account's other devices list it (remote control). A browser has none.
#[cfg(unix)]
pub fn device_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is writable for its whole length; gethostname writes a NUL-terminated name into it.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let name = if ok { String::from_utf8_lossy(buf.split(|b| *b == 0).next().unwrap_or_default()).into_owned() } else { String::new() };
    let name = name.trim_end_matches(".local").to_string();
    if name.is_empty() { "nori".into() } else { name }
}

/// The profile open before a jam was joined, opened again on leaving it (an app value).
const BEFORE_JAM: &str = "host.beforeJam";

/// Why a jam was not joined.
#[derive(Debug)]
pub enum JoinError {
    /// The invite is to the jam this device hosts.
    Own,
    /// The jam has ended.
    Ended,
    Failed(NetError),
}

/// Joins the jam `link` invites to (blocking), named as the active profile's user, else `device`; never
/// one `remote` (this device's) opened ([`nori_core::remote::jam_join`]).
pub fn jam_join(transport: Arc<dyn nori_core::transport::Transport>, settings: &Arc<Settings>, link: String, device: &str, remote: Option<Arc<nori_core::remote::Remote>>) -> Result<JamPass, JoinError> {
    use nori_core::remote::JamJoin;
    let user = settings.prefs(|p| p.servers.iter().find(|s| s.id == p.active_server_id).map(|s| s.user.clone()).filter(|u| !u.is_empty()));
    match block_on(nori_core::remote::jam_join(transport, settings.clone(), remote, link, user.unwrap_or_else(|| device.into()))) {
        Ok(JamJoin::Joined { pass }) => Ok(pass),
        Ok(JamJoin::Own) => Err(JoinError::Own),
        Ok(JamJoin::Ended) => Err(JoinError::Ended),
        Err(e) => Err(JoinError::Failed(e)),
    }
}

/// The guest profile `pass` signs in with, called `name`, made the active one in place of any guest
/// profile before it. The account's profile active until now is opened again by [`jam_left`].
pub fn jam_joined(settings: &Settings, pass: JamPass, name: &str) -> SavedServer {
    let mut prefs = settings.current().unwrap_or_default();
    let active = prefs.servers.iter().find(|s| s.id == prefs.active_server_id);
    if active.is_some_and(|s| !nori_remote::is_guest_key(&s.api_key)) {
        settings.keep_app_value(BEFORE_JAM, prefs.active_server_id.clone());
    }
    let guest = SavedServer { id: nori_core::settings::new_server_id(), name: name.into(), url: pass.url, api_key: pass.api_key, ..Default::default() };
    prefs.servers.retain(|s| !nori_remote::is_guest_key(&s.api_key));
    prefs.servers.push(guest.clone());
    prefs.active_server_id = guest.id.clone();
    settings.put(prefs);
    guest
}

/// Drops the jam's guest profile; the profile to open now: the one open before the jam, else the one
/// used last (the list is in the order they were opened), else none (the login).
pub fn jam_left(settings: &Settings) -> Option<SavedServer> {
    let mut prefs = settings.current().unwrap_or_default();
    prefs.servers.retain(|s| !nori_remote::is_guest_key(&s.api_key));
    let before = settings.app_value(BEFORE_JAM).and_then(|id| prefs.servers.iter().find(|s| s.id == id));
    let back = before.or(prefs.servers.last()).cloned();
    prefs.active_server_id = back.as_ref().map(|s| s.id.clone()).unwrap_or_default();
    settings.put(prefs);
    back
}

/// Runs `f` on a named thread.
pub fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(f);
}

/// Volume (0 to 1) in dB, floored at -96.
pub fn volume_db(v: f32) -> f64 {
    if v > 0.0 { 20.0 * (v as f64).log10() } else { -96.0 }
}

/// The client's volume, 0 to 1, and the loudness compensation it is. The account's other devices read
/// and set it too (remote control).
pub struct Level {
    value: AtomicU32,
    /// Sets the sound card's level; None where this client cannot set the volume, which is then followed
    /// but not offered to other devices.
    card: Option<Box<dyn Fn(f32) + Send + Sync>>,
    /// The volume in dB, for loudness compensation.
    pub loudness: Arc<OutputVolume>,
}

impl Level {
    pub fn new(v: f32, card: Option<Box<dyn Fn(f32) + Send + Sync>>) -> Arc<Level> {
        let level = Level { value: AtomicU32::new(0), card, loudness: Arc::default() };
        level.set(v);
        Arc::new(level)
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.value.load(Ordering::Relaxed))
    }

    /// Sets the volume; true when that moved loudness compensation audibly.
    pub fn set(&self, v: f32) -> bool {
        let v = v.clamp(0.0, 1.0);
        self.value.store(v.to_bits(), Ordering::Relaxed);
        if let Some(card) = &self.card {
            card(v);
        }
        self.loudness.set(volume_db(v))
    }

    /// As another device shows and sets it, 0 to 100; None when this client cannot set it.
    pub fn percent(&self) -> Option<u8> {
        self.card.as_ref().map(|_| (self.get() * 100.0).round() as u8)
    }
}

/// Page colours from a cover, as Android's `CoverLoader.colours`: RGBA converted to ARGB.
pub fn derive(image: &Image) -> CoverColours {
    let px: Vec<u32> = image.pixels.as_chunks::<4>().0.iter().map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]])).collect();
    nori_look::cover::derive(&px, image.width as usize, image.height as usize, false)
}

/// A collection whose songs are fetched on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    Album(String),
    DownloadedAlbum(String),
    Playlist(String),
    Artist(String),
    /// One of the core's mixes (or the favourites), as its page lists it.
    Mix(String),
}

impl Fetch {
    /// The page these songs are the list of.
    pub fn origin(&self) -> PageOrigin {
        match self {
            Fetch::Album(id) => PageOrigin::new(OriginKind::Album, id.as_str()),
            Fetch::DownloadedAlbum(id) => PageOrigin::new(OriginKind::Downloads, id.as_str()),
            Fetch::Playlist(id) => PageOrigin::new(OriginKind::Playlist, id.as_str()),
            Fetch::Artist(id) => PageOrigin::new(OriginKind::Artist, id.as_str()),
            Fetch::Mix(id) => PageOrigin::new(OriginKind::Mix, id.as_str()),
        }
    }

    /// The collection's songs, from the server.
    pub async fn songs(self, client: &Client) -> Result<Vec<Song>, NetError> {
        Ok(match self {
            Fetch::DownloadedAlbum(id) => client.core().downloaded_album(id)?.map(|a| a.songs).unwrap_or_default(),
            Fetch::Album(id) => match client.read_now(Read::AlbumSongs { id }).await? {
                Page::Songs { v } => v,
                Page::AlbumPage { v } => v.songs,
                _ => Vec::new(),
            },
            Fetch::Playlist(id) => match client.read_now(Read::PlaylistSongs { id }).await? {
                Page::Songs { v } => v,
                Page::PlaylistPage { v } => v.songs,
                _ => Vec::new(),
            },
            Fetch::Artist(id) => match client.read_now(Read::ArtistById { id }).await? {
                Page::ArtistPage { v } => client.artist_songs(v.albums).await,
                _ => Vec::new(),
            },
            Fetch::Mix(id) => client.mix_songs(id).await?,
        })
    }
}

/// Where a session's work that waits on the network runs natively: one [`nori_tasks::Native`] thread,
/// parked while nothing is ready. Requests answer from nori-http's own threads.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn tasks() -> Arc<dyn Tasks> {
    Arc::new(nori_tasks::Native::new("nori-tasks").expect("a thread for the session's tasks"))
}

/// Saves the queue and playback position.
pub fn save(core: &Core, engine: &Engine) {
    let _ = core.playlist_save(engine.status_with(|s| s.position_now()).max(0) as u64);
}

/// Debounced queue saves (`rules::queue_keep`): a task that sleeps until a save is due, alive while one
/// is.
pub struct Keeper {
    save: Box<dyn Fn() + Send + Sync>,
    tasks: Arc<dyn Tasks>,
    due: parking_lot::Mutex<Due>,
}

#[derive(Default)]
struct Due {
    at: Option<Instant>,
    /// The task that saves runs.
    running: bool,
    stopped: bool,
}

impl Keeper {
    /// Saves with `save` when due, waiting among `tasks`.
    pub fn start(save: impl Fn() + Send + Sync + 'static, tasks: Arc<dyn Tasks>) -> Arc<Keeper> {
        Arc::new(Keeper { save: Box::new(save), tasks, due: parking_lot::Mutex::default() })
    }

    /// Schedules a save `ms` from now, replacing any pending one.
    pub fn later(self: &Arc<Self>, ms: i64) {
        let mut due = self.due.lock();
        if due.stopped {
            return;
        }
        due.at = Some(Instant::now() + Duration::from_millis(ms.max(0) as u64));
        if std::mem::replace(&mut due.running, true) {
            return;
        }
        let me = self.clone();
        self.tasks.spawn(Box::pin(async move {
            loop {
                let wait = {
                    let mut due = me.due.lock();
                    match due.at {
                        Some(at) if !due.stopped && Instant::now() < at => at.saturating_duration_since(Instant::now()),
                        Some(_) if !due.stopped => {
                            due.at = None;
                            Duration::ZERO
                        }
                        _ => {
                            due.running = false;
                            return;
                        }
                    }
                };
                if wait.is_zero() {
                    (me.save)();
                } else {
                    me.tasks.sleep(wait).await;
                }
            }
        }));
    }

    pub fn stop(&self) {
        self.due.lock().stopped = true;
    }
}

/// The place heard at the last look, for telling a jump (MPRIS's Seeked) from the music moving on.
#[cfg(feature = "desktop")]
#[derive(Default)]
pub(crate) struct PlaceSeen(Option<Seen>);

#[cfg(feature = "desktop")]
struct Seen {
    index: usize,
    id: Option<String>,
    ms: i64,
    at: Instant,
    pace: f32,
    playing: bool,
}

/// A place this far from where the last look ran on to has jumped.
#[cfg(feature = "desktop")]
const JUMP_MS: i64 = 500;

#[cfg(feature = "desktop")]
impl PlaceSeen {
    /// Looks at `s`: whether its place jumped since the last look, within the song away from where that
    /// look ran on to, or into another song anywhere but its start.
    pub(crate) fn jumped(&mut self, s: &Status) -> bool {
        let Some(index) = s.index else {
            self.0 = None;
            return false;
        };
        let seen = Seen { index, id: s.id.clone(), ms: s.position_ms, at: s.at, pace: s.pace, playing: s.state == State::Playing };
        let ran_to = match self.0.replace(seen) {
            Some(p) if p.index == index && p.id == s.id && p.playing => p.ms + (s.at.saturating_duration_since(p.at).as_secs_f64() * 1000.0 * p.pace as f64) as i64,
            Some(p) if p.index == index && p.id == s.id => p.ms,
            _ => 0,
        };
        (s.position_ms - ran_to).abs() > JUMP_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saves are put off until the last one asked for is due; none after the stop.
    #[test]
    fn queue_saves_wait_for_the_last_one_asked() {
        let saves = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (seen, started) = (saves.clone(), Instant::now());
        let keeper = Keeper::start(move || seen.lock().push(started.elapsed()), tasks());
        keeper.later(150);
        std::thread::sleep(Duration::from_millis(50));
        keeper.later(150);
        let until = Instant::now() + Duration::from_secs(10);
        while saves.lock().is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(300));
        let at = saves.lock().clone();
        assert_eq!(at.len(), 1, "one save for both: {at:?}");
        assert!(at[0] >= Duration::from_millis(200), "150 ms after the second ask: {:?}", at[0]);
        keeper.stop();
        keeper.later(0);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(saves.lock().len(), 1, "nothing saved once stopped");
    }

    /// A guest profile takes the active one's place while in a jam; leaving drops it and opens the
    /// account's again, also after a second jam joined from the first.
    #[test]
    fn a_jam_guest_profile_comes_and_goes() {
        let dir = nori_testdir::TempDir::new("host-jam");
        let settings = Settings::new();
        let mut prefs = settings.open(&db_path(dir.path())).unwrap();
        let own = |id: &str| SavedServer { id: id.into(), url: format!("http://{id}"), user: "ann".into(), password: "pw".into(), ..Default::default() };
        prefs.servers = vec![own("home"), own("work")];
        prefs.active_server_id = "work".into();
        settings.put(prefs);
        let pass = |n: u8| JamPass { url: "http://friend".into(), api_key: nori_remote::guest_key(&format!("key{n}")) };

        let first = jam_joined(&settings, pass(1), "Jam");
        nori_core::background::flush();
        let second = jam_joined(&settings, pass(2), "Jam");
        nori_core::background::flush();
        let p = settings.current().unwrap();
        let ids: Vec<&str> = p.servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["home", "work", second.id.as_str()], "one guest profile, the latest");
        assert_ne!(first.id, second.id);
        assert_eq!((p.active_server_id.as_str(), second.name.as_str()), (second.id.as_str(), "Jam"));

        assert_eq!(jam_left(&settings).map(|s| s.id), Some("work".into()), "the profile open before the jams");
        let p = settings.current().unwrap();
        assert_eq!((p.servers.len(), p.active_server_id.as_str()), (2, "work"));
    }

    /// With the profile open before the jam gone, leaving opens the one used last, else the login.
    #[test]
    fn leaving_a_jam_falls_back_to_the_profile_used_last() {
        let dir = nori_testdir::TempDir::new("host-jam-back");
        let settings = Settings::new();
        let mut prefs = settings.open(&db_path(dir.path())).unwrap();
        let own = |id: &str| SavedServer { id: id.into(), url: format!("http://{id}"), user: "ann".into(), ..Default::default() };
        // In the order they were opened: "work" last.
        prefs.servers = vec![own("home"), own("work")];
        prefs.active_server_id = "gone".into();
        settings.put(prefs);
        let pass = JamPass { url: "http://friend".into(), api_key: nori_remote::guest_key("key") };
        jam_joined(&settings, pass.clone(), "Jam");
        assert_eq!(jam_left(&settings).map(|s| s.id), Some("work".into()));

        let mut prefs = settings.current().unwrap();
        prefs.servers.clear();
        settings.put(prefs);
        jam_joined(&settings, pass, "Jam");
        assert_eq!(jam_left(&settings).map(|s| s.id), None, "the login");
        assert!(settings.current().unwrap().servers.is_empty());
    }

    /// Seeked's rule: a jump within the song, or a song coming in past its start, but not the music
    /// moving on at its pace, a pause, or a resume.
    #[cfg(feature = "desktop")]
    #[test]
    fn a_place_jumps_only_off_its_run() {
        let start = Instant::now();
        let mut seen = PlaceSeen::default();
        // (seconds since start, index, id, ms, playing, pace, jumped)
        let looks = [
            (0.0, Some(0), "a", 0, true, 1.0, false),
            (10.0, Some(0), "a", 10_000, true, 1.0, false),
            (11.0, Some(0), "a", 11_040, true, 1.0, false),
            (12.0, Some(0), "a", 60_000, true, 1.0, true),
            (13.0, Some(0), "a", 61_000, false, 1.0, false),
            (40.0, Some(0), "a", 61_000, false, 1.0, false),
            (41.0, Some(0), "a", 20_000, false, 1.0, true),
            (90.0, Some(0), "a", 20_000, true, 1.0, false),
            (93.0, Some(0), "a", 0, true, 1.0, true),
            (94.0, Some(1), "b", 120, true, 1.0, false),
            (100.0, Some(2), "c", 15_000, true, 1.5, true),
            (102.0, Some(2), "c", 18_000, true, 1.0, false),
            (104.0, Some(2), "c", 0, true, 1.0, true),
            (105.0, Some(2), "d", 30_000, true, 1.0, true),
            (106.0, None, "", 0, false, 1.0, false),
            (107.0, Some(2), "d", 0, true, 1.0, false),
        ];
        for (n, (t, index, id, ms, playing, pace, jumped)) in looks.into_iter().enumerate() {
            let state = if playing { State::Playing } else { State::Paused };
            let at = start + Duration::from_secs_f64(t);
            let status = Status { state, index, id: index.map(|_| id.to_string()), position_ms: ms, at, pace, ..Status::default() };
            assert_eq!(seen.jumped(&status), jumped, "look {n}");
        }
    }
}
