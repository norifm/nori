//! Remote control on the terminal and desktop clients: the session's player as the core's
//! [`RemotePlayer`], the active device while it is another one ([`Elsewhere`]), which the session's
//! controls then act on, and mDNS through mdns-sd for nearby devices (`desktop` feature). A [`Remote`] is
//! made only while remote control or jams are switched on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(feature = "desktop")]
use std::sync::Weak;

use nori_core::remote::{JamControls, Mirror, MirrorRow, Playing, Reach, Remote, RemotePlayer, RemoteShown};
#[cfg(feature = "desktop")]
use nori_core::remote::{Announcement, Discovery};
use nori_core::playlist::PlaylistView;
use nori_core::Song;
use nori_engine::{Engine, Status};
use nori_remote::wire::Op;
use parking_lot::Mutex;

use crate::session::{Handle, Out, Said};
use crate::Level;

/// The session's remote, while there is one.
pub struct Remotes {
    slot: Mutex<Option<Arc<Remote>>>,
    level: Arc<Level>,
    /// The engine waits for a song's bytes with nothing left to play.
    buffering: AtomicBool,
    /// Another device plays, as last seen.
    mirroring: AtomicBool,
}

impl Remotes {
    pub(crate) fn new(level: Arc<Level>) -> Remotes {
        Remotes { slot: Mutex::default(), level, buffering: AtomicBool::new(false), mirroring: AtomicBool::new(false) }
    }

    /// The active device while it is another one: the player shows and controls it instead of this one.
    pub fn elsewhere(&self) -> Option<Elsewhere> {
        let remote = self.get()?;
        let mirror = remote.active()?;
        Some(Elsewhere { mirror, remote })
    }

    /// The jam this device is a guest in, as [`crate::session::Handle::jam_playing`] shows it.
    pub(crate) fn jam_playing(&self) -> Option<Elsewhere> {
        let remote = self.get()?;
        let mirror = remote.jam_playing()?;
        Some(Elsewhere { mirror, remote })
    }

    /// Notes whether another device plays now; true when it just started to.
    fn mirroring(&self, on: bool) -> bool {
        let was = self.mirroring.swap(on, Ordering::Relaxed);
        on && !was
    }

    pub(crate) fn buffering(&self, on: bool) {
        self.buffering.store(on, Ordering::Relaxed);
    }

    pub fn get(&self) -> Option<Arc<Remote>> {
        self.slot.lock().clone()
    }

    pub(crate) fn set(&self, remote: Option<Arc<Remote>>) {
        // Out of the slot first: going, it tells [`Shown`], which reads the slot.
        let old = std::mem::replace(&mut *self.slot.lock(), remote);
        if let Some(old) = old {
            old.stop();
        }
    }

    /// Tells the remote (if any) where the engine is now.
    pub fn played(&self, engine: &Engine) {
        let Some(r) = self.get() else { return };
        r.played(self.playing(&engine.status()));
    }

    /// The playback other devices see, from the engine's status `s`.
    fn playing(&self, s: &Status) -> Playing {
        nori_engine::core::playing(s, self.buffering.load(Ordering::Relaxed), self.level.percent())
    }
}

/// The session's engine playing along with the jam this guest listens along to.
pub(crate) struct Along {
    pub(crate) engine: Arc<Engine>,
    pub(crate) session: Arc<nori_core::queue::Session>,
}

impl nori_core::remote::Follower for Along {
    fn lead(&self, lead: Option<nori_core::remote::Lead>) {
        nori_engine::core::follow(&self.engine, &self.session, lead);
    }
}

/// What another device asks of this one, done to the session's queue and engine as its own keys would.
pub(crate) struct HostPlayer(pub(crate) Handle);

impl RemotePlayer for HostPlayer {
    fn apply(&self, op: Op) {
        let h = &self.0;
        match op {
            Op::Play => {
                h.engine.play();
            }
            Op::Pause => {
                h.engine.pause();
            }
            Op::Seek { ms } => {
                h.engine.seek(ms);
            }
            Op::Next => {
                h.engine.next();
            }
            Op::Previous => {
                h.engine.previous();
            }
            Op::Jump { index, .. } => {
                h.engine.play_at(index as usize, 0);
            }
            Op::Remove { index, .. } => h.remove_here(index as usize),
            Op::Move { from, to, .. } => h.move_here(from as usize, to as usize),
            Op::Restore { song, index } => h.put_back_from_afar(song, index as usize),
            Op::Add { songs, next } => h.enqueue_here(songs, next),
            Op::Replace { songs, index, position_ms, play, order, shuffle, repeat, .. } => h.replace(songs, index as usize, position_ms, play, order, shuffle, repeat),
            Op::Shuffle { on } => h.shuffle_here(on),
            Op::Repeat { mode } => h.repeat_here(mode),
            Op::Volume { percent } => return h.volume_from_afar(percent as f32 / 100.0),
            // The devices see it once the core has marked it.
            Op::Star { id, on } => return h.star_from_afar(id, on),
            // The core keeps transfers, pages, time exchanges and jam ops to itself.
            Op::Transfer { .. } | Op::Clear | Op::LibraryMarks { .. } | Op::Page { .. } | Op::Clock { .. } | Op::Request { .. } | Op::Decide { .. } | Op::Promote { .. } | Op::Kick { .. } => {}
        }
        h.remotes.played(&h.engine);
    }
}

/// Tells the client to read the devices or the jam again. Once another device plays, this one's engine
/// stops and lets its output go.
pub(crate) struct Shown {
    pub(crate) out: Out,
    pub(crate) remotes: Arc<Remotes>,
    pub(crate) engine: Arc<Engine>,
}

impl RemoteShown for Shown {
    fn marks_changed(&self) {
        if let Some(remote) = self.remotes.get() {
            (self.out)(Said::Starred(remote.star_marks()));
        }
    }

    fn changed(&self) {
        let elsewhere = self.remotes.get().is_some_and(|r| r.active().is_some());
        if self.remotes.mirroring(elsewhere) {
            self.engine.release_now();
        }
        (self.out)(Said::Remote);
    }

    fn jam_ended(&self, host: Option<String>) {
        (self.out)(Said::JamEnded { host });
    }
}

/// The account's active device while it is another one, as read at one moment: what the player shows
/// instead of this device's own playback, and where its controls go.
#[derive(Clone)]
pub struct Elsewhere {
    pub mirror: Mirror,
    remote: Arc<Remote>,
}

impl Elsewhere {
    /// The song playing there.
    pub fn song(&self) -> Option<&Song> {
        self.row().map(|r| &r.song)
    }

    /// Whether the song `id` is starred as the device shows it; None when it is not in its queue.
    pub(crate) fn starred(&self, id: &str) -> Option<bool> {
        self.mirror.rows.iter().find(|r| r.song.id == id).map(|r| r.song.starred)
    }

    /// Stars song `id` there when its queue has it ([`Remote::star_where_playing`]); false otherwise.
    pub(crate) fn star(&self, id: &str, on: bool) -> bool {
        self.remote.clone().star_where_playing(id.to_string(), on)
    }

    pub(crate) fn row(&self) -> Option<&MirrorRow> {
        self.mirror.at.and_then(|a| self.mirror.rows.get(a as usize))
    }

    /// Where the song is now: what the device's listener hears at this moment ([`Mirror::position_now`]).
    pub fn position_ms(&self) -> i64 {
        self.mirror.position_now()
    }

    /// The device's queue as this client reads its own: each song at its list index there (left default
    /// where it is not known here), in play order, the one playing current.
    pub fn view(&self) -> PlaylistView {
        let m = &self.mirror;
        let mut songs = vec![Song::default(); m.rows.iter().map(|r| r.index as usize + 1).max().unwrap_or(0)];
        for r in &m.rows {
            songs[r.index as usize] = r.song.clone();
        }
        PlaylistView {
            songs,
            len: m.rows.len() as u32,
            list_rev: m.rev,
            order: m.rows.iter().map(|r| r.index).collect(),
            queued: Vec::new(),
            index: self.row().map_or(-1, |r| r.index as i32),
            shuffle: m.shuffle,
            repeat: m.repeat,
            bridging: false,
            rev: m.rev,
        }
    }

    /// The device's volume, 0 to 1, when it can be set.
    pub fn volume(&self) -> Option<f32> {
        self.mirror.volume.map(|v| v as f32 / 100.0)
    }

    /// `songs` from `start` (or shuffled) as the device's queue, playing, its repeat kept.
    fn replace(&self, songs: Vec<Song>, start: usize, shuffle: bool) -> Op {
        Op::Replace { songs, index: start as u32, position_ms: 0, play: true, order: None, shuffle, repeat: self.mirror.repeat, transfer: None, listen: None }
    }

    pub(crate) fn play(&self, songs: Vec<Song>, start: usize, shuffle: bool) {
        self.send(self.replace(songs, start, shuffle));
    }

    /// Puts song `id`, removed here, back where it was in the device's queue; false when it was not.
    pub(crate) fn put_back(&self, id: &str) -> bool {
        self.remote.clone().put_back(self.mirror.id.clone(), id.to_string())
    }

    /// This jam guest's control `op`, by its role ([`Remote::jam_press`]).
    pub(crate) fn jam_press(&self, op: Op) -> Reach {
        self.remote.clone().jam_press(op)
    }

    /// What this jam guest's controls reach, and what its play button shows; None outside a jam.
    pub fn jam_controls(&self) -> Option<JamControls> {
        self.remote.jam_controls()
    }

    pub(crate) fn send(&self, op: Op) {
        self.remote.clone().send(self.mirror.id.clone(), op);
    }
}

/// The process's media controls while a session drives them: what they last saw, and the cover they
/// show.
#[cfg(feature = "desktop")]
pub(crate) struct Media {
    pub(crate) mpris: Arc<nori_mpris::Mpris>,
    pub(crate) seen: Mutex<crate::PlaceSeen>,
    pub(crate) cover: NowCover,
}

#[cfg(feature = "desktop")]
impl Media {
    pub(crate) fn new(mpris: Arc<nori_mpris::Mpris>, covers: Option<Arc<nori_covers::loader::Loader>>, core: Arc<nori_core::Core>) -> Media {
        let cover = NowCover::new(covers, core, Arc::downgrade(&mpris));
        Media { mpris, seen: Mutex::default(), cover }
    }
}

/// The cover of the song the media controls show: its file for macOS's Now Playing to draw, and for MPRIS
/// where the cover cache keeps it, never the signed server address.
#[cfg(feature = "desktop")]
pub(crate) struct NowCover {
    covers: Option<Arc<nori_covers::loader::Loader>>,
    core: Arc<nori_core::Core>,
    /// Told when the file is read.
    media: Weak<nori_mpris::Mpris>,
    shown: Arc<Mutex<CoverShown>>,
}

/// The cover the media controls show.
#[cfg(feature = "desktop")]
#[derive(Default)]
struct CoverShown {
    id: String,
    /// Its file, once read.
    file: Option<CoverFile>,
}

/// A cover read for the media controls.
#[cfg(feature = "desktop")]
#[derive(Clone)]
pub(crate) struct CoverFile {
    /// Its bytes, kept only where Now Playing draws them (macOS).
    pub(crate) bytes: Option<Arc<[u8]>>,
    /// Its file in the cover cache; none for a cover the cache does not keep (a provider's).
    pub(crate) path: Option<std::path::PathBuf>,
}

#[cfg(feature = "desktop")]
impl NowCover {
    fn new(covers: Option<Arc<nori_covers::loader::Loader>>, core: Arc<nori_core::Core>, media: Weak<nori_mpris::Mpris>) -> NowCover {
        NowCover { covers, core, media, shown: Arc::default() }
    }

    /// Cover `id`'s file once read; asked for another cover, it is read on a thread of its own and the
    /// media controls are told when it is in.
    pub(crate) fn of(&self, id: &str) -> Option<CoverFile> {
        let covers = self.covers.clone()?;
        let mut shown = self.shown.lock();
        if shown.id == id {
            return shown.file.clone();
        }
        *shown = CoverShown { id: id.to_string(), file: None };
        let url = self.core.cover_address(id.to_string(), nori_core::covers::cover_rendition(NOW_COVER_PX));
        let (slot, media, id) = (self.shown.clone(), self.media.clone(), id.to_string());
        crate::spawn("nori-now-cover", move || {
            let Some(file) = read_cover(&covers, &url) else { return };
            {
                let mut shown = slot.lock();
                if shown.id != id {
                    return;
                }
                shown.file = Some(file);
            }
            if let Some(m) = media.upgrade() {
                m.changed();
            }
        });
        None
    }
}

/// Reads the cover at `url` on this thread, fetching it into the cover cache if it is not there.
#[cfg(feature = "desktop")]
fn read_cover(covers: &nori_covers::loader::Loader, url: &str) -> Option<CoverFile> {
    let mut file = Vec::new();
    covers.read(url, &mut file).ok()?;
    let key = covers.key(url);
    let path = covers.disk().filter(|d| d.contains(key)).map(|d| d.path(key));
    Some(CoverFile { bytes: cfg!(target_os = "macos").then(|| file.into()), path })
}

/// Now Playing's cover size, in pixels.
#[cfg(feature = "desktop")]
const NOW_COVER_PX: u32 = 600;

/// mDNS for nearby doors, over mdns-sd's own thread; made with the remote, so nothing runs while it is off.
#[cfg(feature = "desktop")]
pub(crate) struct Mdns {
    daemon: mdns_sd::ServiceDaemon,
    remote: std::sync::OnceLock<Weak<Remote>>,
    announced: Mutex<Option<String>>,
}

#[cfg(feature = "desktop")]
const SERVICE_TYPE: &str = "_nori._tcp.local.";

#[cfg(feature = "desktop")]
impl Mdns {
    pub(crate) fn start() -> Option<Arc<Mdns>> {
        let daemon = mdns_sd::ServiceDaemon::new().ok()?;
        Some(Arc::new(Mdns { daemon, remote: Default::default(), announced: Mutex::new(None) }))
    }

    pub(crate) fn serve(&self, remote: &Arc<Remote>) {
        let _ = self.remote.set(Arc::downgrade(remote));
    }
}

#[cfg(feature = "desktop")]
impl Drop for Mdns {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

#[cfg(feature = "desktop")]
impl Discovery for Mdns {
    fn announce(&self, door: Option<Announcement>) {
        if let Some(old) = self.announced.lock().take() {
            let _ = self.daemon.unregister(&old);
        }
        let Some(door) = door else { return };
        let host = format!("{}.local.", door.name);
        let txt: std::collections::HashMap<String, String> = door.txt.into_iter().map(|p| (p.key, p.value)).collect();
        let Ok(info) = mdns_sd::ServiceInfo::new(SERVICE_TYPE, &door.name, &host, "", door.port, txt) else { return };
        let info = info.enable_addr_auto();
        let name = info.get_fullname().to_string();
        if self.daemon.register(info).is_ok() {
            *self.announced.lock() = Some(name);
        }
    }

    fn browse(&self, on: bool) {
        if !on {
            let _ = self.daemon.stop_browse(SERVICE_TYPE);
            return;
        }
        let (Ok(events), Some(remote)) = (self.daemon.browse(SERVICE_TYPE), self.remote.get().cloned()) else { return };
        crate::spawn("nori-mdns", move || {
            while let Ok(e) = events.recv() {
                let Some(r) = remote.upgrade() else { return };
                match e {
                    mdns_sd::ServiceEvent::ServiceResolved(s) => {
                        let Some(ip) = s.addresses.iter().map(|a| a.to_ip_addr()).find(|a| a.is_ipv4()).or_else(|| s.addresses.iter().next().map(|a| a.to_ip_addr())) else { continue };
                        let txt = s.txt_properties.iter().map(|p| nori_core::Param { key: p.key().to_string(), value: p.val_str().to_string() }).collect();
                        r.lan_found(s.fullname.clone(), ip.to_string(), s.port, txt);
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, name) => r.lan_lost(name),
                    mdns_sd::ServiceEvent::SearchStopped(_) => return,
                    _ => {}
                }
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nori_engine::State;

    /// A server whose every answer is one small PNG.
    #[cfg(feature = "desktop")]
    struct Png;

    #[cfg(feature = "desktop")]
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot much more";

    #[cfg(feature = "desktop")]
    #[async_trait::async_trait]
    impl nori_core::transport::Transport for Png {
        async fn get(&self, _: String, _: u32) -> Result<nori_core::transport::TransportResponse, nori_core::transport::TransportError> {
            Ok(nori_core::transport::TransportResponse { status: 200, body: PNG.to_vec(), set_cookies: Vec::new() })
        }
        async fn send(&self, e: nori_core::transport::Exchange) -> Result<nori_core::transport::TransportResponse, nori_core::transport::TransportError> {
            self.get(e.url, e.timeout_ms).await
        }
        fn address_changed(&self) {}
        fn network(&self) -> nori_core::transport::Network {
            nori_core::transport::Network::Unmetered
        }
    }

    /// The media controls get the cover's file in the cache, named without the signed address's
    /// credentials; a provider's cover, which the cache does not keep, gets none.
    #[cfg(feature = "desktop")]
    #[test]
    fn a_cover_read_points_at_its_cached_file() {
        let dir = nori_testdir::TempDir::new("host-now-cover");
        let covers = nori_covers::loader::Loader::new(nori_covers::loader::Config::new(dir.path()), nori_core::covers::CoverNet::over(Arc::new(Png)));
        let url = "http://music.example/rest/getCoverArt?u=ann&t=token123&s=salt456&id=al-1&size=600";
        let path = read_cover(&covers, url).unwrap().path.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
        let name = path.to_string_lossy();
        assert!(!name.contains("token123") && !name.contains("salt456") && !name.contains("ann"), "{name}");
        assert!(path.starts_with(dir.path()));

        let provider = "http://music.example/rest/getCoverArt?u=ann&t=token123&s=salt456&id=ext-deezer-song-1&size=600";
        assert!(read_cover(&covers, provider).unwrap().path.is_none());
    }

    #[test]
    fn other_devices_see_and_set_the_volume() {
        let card = Arc::new(Mutex::new(0.0f32));
        let to_card = card.clone();
        let remotes = Remotes::new(Level::new(0.4, Some(Box::new(move |v| *to_card.lock() = v))));
        let status = Status { state: State::Playing, ..Default::default() };
        assert_eq!(remotes.playing(&status).volume, Some(40));
        assert!(remotes.level.set(0.25), "loudness moves with it");
        assert_eq!((*card.lock(), remotes.playing(&status).volume), (0.25, Some(25)));
        remotes.buffering(true);
        assert!(remotes.playing(&status).buffering);

        // A volume this client cannot set is followed, not offered.
        let system = Remotes::new(Level::new(0.4, None));
        assert_eq!(system.playing(&status).volume, None);
    }

    struct Nothing;

    impl RemotePlayer for Nothing {
        fn apply(&self, _: Op) {}
    }

    impl RemoteShown for Nothing {
    fn marks_changed(&self) {}

        fn changed(&self) {}
        fn jam_ended(&self, _: Option<String>) {}
    }

    /// The desk mirrored playing the second of three songs (200 s each) 10 s in, at volume 40.
    pub(crate) fn desk() -> Elsewhere {
        let core = nori_core::Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let client = nori_core::client::Client::new(core, Arc::new(crate::session::Offline), Default::default());
        let me = nori_core::remote::RemoteMe { name: "Mac".into(), kind: nori_remote::wire::DeviceKind::Desktop };
        let remote = Remote::new(client, me, Arc::new(Nothing), Arc::new(Nothing), None);
        let row = |index: u32, id: &str| MirrorRow { index, song: Song { id: id.into(), duration: 200, ..Default::default() } };
        let mirror = Mirror {
            id: "desk".into(),
            name: "Desk".into(),
            kind: nori_remote::wire::DeviceKind::Desktop,
            rows: vec![row(2, "a"), row(0, "b"), row(1, "c")],
            at: Some(1),
            len: 3,
            rev: 7,
            playing: true,
            buffering: false,

            mixing: false,
            position_ms: 10_000,
            rate: 1.0,
            at_us: 0,
            shuffle: true,
            repeat: 0,
            volume: Some(40),
            refused: None,
        };
        Elsewhere { mirror, remote }
    }

    #[test]
    fn a_device_elsewhere_runs_on_within_its_song() {
        let mut e = desk();
        assert_eq!(e.song().map(|s| s.id.as_str()), Some("b"));
        assert_eq!((e.mirror.position_at(2_500_000), e.mirror.position_at(500_000_000)), (12_500, 200_000), "held at the song's end");
        e.mirror.playing = false;
        assert_eq!(e.mirror.position_at(2_500_000), 10_000, "paused");
        let v = e.view();
        let ids: Vec<&str> = v.songs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!((ids, v.order.clone(), v.index), (vec!["b", "c", "a"], vec![2, 0, 1], 0), "each song at its list index, in play order");
        let mut known = e.clone();
        known.mirror.rows.remove(0);
        assert_eq!(known.view().songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b", "c"], "as far as the rows are known");
        e.mirror.at = None;
        assert_eq!(e.view().index, -1);
        assert_eq!(e.volume(), Some(0.4));
        e.mirror.rows[2].song.starred = true;
        assert_eq!((e.starred("c"), e.starred("a"), e.starred("x")), (Some(true), Some(false), None), "hearts of its queue only");
    }

    #[test]
    fn a_replace_keeps_the_devices_repeat() {
        let songs = vec![Song { id: "x".into(), ..Default::default() }];
        assert!(matches!(desk().replace(songs, 0, false), Op::Replace { play: true, position_ms: 0, shuffle: false, repeat: 0, .. }));
    }

    #[test]
    fn the_engine_lets_go_once_each_time_another_device_starts_playing() {
        let remotes = Remotes::new(Level::new(1.0, None));
        let seen: Vec<bool> = [false, true, true, false, true].into_iter().map(|on| remotes.mirroring(on)).collect();
        assert_eq!(seen, [false, true, false, false, true]);
    }
}
