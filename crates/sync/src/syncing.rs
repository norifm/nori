//! A device's life with its sync server, for every client: the paired device's session in its group's
//! room, pairing a new device on either side, and when the device is connected (presence.rs). The client
//! hands in its WebSockets ([`Sockets`]), what its sockets hear, whether nori is open and playing, and
//! the audio's wake-ups; it gets back what sync did ([`SyncClient`]). Profiles sync brings are kept in
//! the settings at once, as a form keeps them, and the client is told what to open again.
//!
//! Nothing runs on a device that is not paired, or has sync off: no executor, no socket, no timer. While
//! it runs, one task sleeps until the next thing due: a reconnect, a keep-alive on a computer or a phone
//! in the foreground, a paused phone letting go. Music playing with the screen off arms none of them.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use futures_util::future::{select, Either};
use nori_settings::settings::{SavedServer, StoredPrefs};
use nori_settings::settings_store::Settings;
use nori_tasks::{Notify, Tasks};

use crate::book::MergeOffer;
use crate::group::{paired, ResetNotice, Seen};
use crate::link::{Ended, Link, RoomState, SocketId, Sockets};
use crate::pairing::{AdmitStep, Admitting, Came, Confirmation, JoinStep, Joining, PairingRefused};
use crate::presence::{Pings, Presence, Reach};
use crate::profile::{Here, NeededPlugin, SignInCheck};
use crate::keys::RoomName;
use crate::nearby::{self, Nearby, NearbyAnnouncement, NearbyDevice, NearbyTxt};
use crate::room::{Heard, Refusal, Relay};
use crate::session::{Elsewhere, Open, Outcome, RemoteHeard, Session, SyncEvent};
use crate::sync_server;

/// What a client does with what sync brings. Calls come on any thread, never while sync holds its
/// state, so they may call back.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait SyncClient: Send + Sync {
    /// The profiles this device plays, whose sign-ins it lets go, and its installed plugins.
    fn here(&self) -> Here;
    /// Sync changed the saved profiles, which the settings now keep: the plugins they need may have
    /// changed too ([`Syncing::needed_plugins`]).
    fn profiles(&self, synced: ProfilesSynced);
    fn event(&self, event: SyncEvent);
    /// [`Syncing::state`], [`Syncing::pairing`] or [`Syncing::devices`] changed (a device came, went, or
    /// came online or left).
    fn changed(&self);
}

/// Remote control of the profile open on this device (the core's remote), in its group's room. Calls
/// come on any thread, in order, never while sync holds its state, so they may call back.
pub trait RemoteRoom: Send + Sync {
    /// Where the device stands in its group's room for the profile.
    fn in_room(&self, now: InRoom);
    /// [`Syncing::lan_secret`] changed for the profile.
    fn keyed(&self);
    /// The profiles changed here: the group's devices reached directly hear of it
    /// ([`Syncing::direct_hello`]).
    fn changed(&self);
    /// Device `device` (its group id) with the same synced profile open said `message`.
    fn heard(&self, device: String, message: serde_json::Value);
    /// Device `device` left the room.
    fn left(&self, device: String);
    /// A device of the group opened another profile for remote control, or none: those with this
    /// profile and another open ([`Syncing::remote_elsewhere`]) may have changed.
    fn elsewhere(&self);
}

/// Where a device stands in its group's room, as remote control is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InRoom {
    /// In the room afresh: what was heard there before is gone.
    Entered,
    /// Still in it, but the profile's sync identity changed (two profiles found to be one): the
    /// devices heard stay, and they hear this one again once they changed too.
    Renamed,
    /// Out of it: what was heard there is gone.
    Left,
}

/// The time requests a device sends when it enters its group room for remote control: the shortest
/// answer gives the server's clock (each request is a billed message).
const TIME_ASKS: usize = 3;

/// The remote attached, the profile it is for, and what it was last told.
struct Attached {
    /// The profile's id on this device.
    local: String,
    room: Arc<dyn RemoteRoom>,
    /// Who the remote says it is, for the group's devices with another profile open.
    card: serde_json::Value,
    /// The profile's sync identity and the group room it is in, while the room is open.
    in_room: Option<(String, RoomName)>,
    /// The sync identity it was in the room with before it left: its last word still goes as it.
    left_as: Option<String>,
    lan_secret: Option<String>,
}

/// What remote control is told.
enum ToRemote {
    InRoom(InRoom),
    Keyed,
    Changed,
    Heard { device: String, message: serde_json::Value },
    Left { device: String },
    Elsewhere,
}

/// What a change sync brought means for the client.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProfilesSynced {
    /// Profiles removed here: their rows in the database go, as on removing one by hand.
    pub removed: Vec<String>,
    pub active: ActiveProfile,
}

/// What became of the profile in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ActiveProfile {
    Same,
    /// Changed: its connection reads it again.
    Edited,
    /// Another one is in use now, or none (the one in use was removed, or the first one came).
    Switched,
}

/// `servers`, as sync left them, kept in `prefs`: the profile in use stays if it is there, otherwise the
/// first one left is.
fn synced(prefs: &StoredPrefs, servers: Vec<SavedServer>) -> (StoredPrefs, ProfilesSynced) {
    let removed = prefs.servers.iter().filter(|s| !servers.iter().any(|n| n.id == s.id)).map(|s| s.id.clone()).collect();
    let kept = servers.iter().any(|s| s.id == prefs.active_server_id);
    let active_server_id = if kept { prefs.active_server_id.clone() } else { servers.first().map(|s| s.id.clone()).unwrap_or_default() };
    let after = StoredPrefs { servers, active_server_id, ..prefs.clone() };
    let active = match (prefs.active_server(), after.active_server()) {
        (Some(b), Some(a)) if b.id == a.id && b == a => ActiveProfile::Same,
        (Some(b), Some(a)) if b.id == a.id => ActiveProfile::Edited,
        (None, None) => ActiveProfile::Same,
        _ => ActiveProfile::Switched,
    };
    (after, ProfilesSynced { removed, active })
}

/// Where this device's sync stands, for a client to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SyncState {
    /// Sync is off or the device is not paired: nothing runs.
    Idle,
    Connecting,
    Connected,
    /// The connection dropped or could not be made; tries again at `retry_ms` (Unix ms).
    Offline { retry_ms: i64 },
    /// The server refused; tries again at `retry_ms` (Unix ms).
    Refused { refused: Refusal, retry_ms: i64 },
    /// Refused until nori starts again: a protocol version the server does not speak, or a bug.
    Stopped { refused: Refusal },
    /// Let go: paused in the background for a while. It connects again when nori is opened or plays.
    Resting,
}

/// Where pairing stands, for its screen.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PairingView {
    /// No pairing.
    Idle,
    /// Reaching the server.
    Starting,
    /// The new device shows this code; `link` is also the QR code's text. The words show beside it.
    Code { link: String, words: Vec<String> },
    /// Ask the user this on the device with the profiles; [`Syncing::pair_confirm`] follows a yes.
    Confirm { confirmation: Confirmation },
    /// The new device is in the group.
    Paired,
    /// The key went to the new device.
    Sent,
    Refused { reason: PairingRefused },
}

/// Which side of a pairing a pairing screen takes: the new device shows its code, the device with the
/// profiles takes another's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PairSide {
    Show,
    Take,
}

/// The side a pairing screen opens on while sync shows `view`, on a device `paired` or not with `profiles`
/// saved profiles: a pairing under way is taken up on its side; a device in a group with profiles to give
/// takes a code; any other shows its own.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn pair_side(view: PairingView, paired: bool, profiles: u32) -> PairSide {
    match view {
        PairingView::Code { .. } => PairSide::Show,
        PairingView::Confirm { .. } => PairSide::Take,
        _ if paired && profiles > 0 => PairSide::Take,
        _ => PairSide::Show,
    }
}

/// The paired device's session and its connection.
struct Paired {
    session: Session,
    link: Link,
    /// Let go while paused in the background.
    resting: bool,
}

/// A pairing under way and its connection to the meeting's server.
enum Pair {
    Joining(Joining, Link),
    Admitting(Admitting, Link),
}

impl Pair {
    fn link(&self) -> &Link {
        match self {
            Pair::Joining(_, l) | Pair::Admitting(_, l) => l,
        }
    }
}

/// Sync while it runs.
struct Running {
    tasks: Arc<dyn Tasks>,
    /// Unix µs less the executor's clock.
    unix_offset: i64,
    paired: Option<Paired>,
    pair: Option<Pair>,
    /// Tells the timer task to end.
    stop: Arc<AtomicBool>,
}

impl Running {
    fn links(&self) -> impl Iterator<Item = &Link> {
        self.paired.iter().map(|p| &p.link).chain(self.pair.iter().map(Pair::link))
    }
}

struct Inner {
    presence: Presence,
    running: Option<Running>,
    pairing: PairingView,
    remote: Option<Attached>,
    /// The server's clock less the executor's (µs), from the answer that came back fastest since the
    /// group room last opened: (the way there and back, the offset).
    clock: Option<(i64, i64)>,
    nearby: NearbyState,
}

/// The nearby way a code travels (nearby.rs), as this device takes part in it.
#[derive(Default)]
struct NearbyState {
    /// The platform's mDNS; none where there is no local network (a browser).
    hook: Option<Arc<dyn Nearby>>,
    announced: Option<NearbyAnnouncement>,
    /// A pairing screen is open here.
    looking: bool,
    found: Vec<NearbyDevice>,
}

/// What to tell the client once the state is let go.
enum Tell {
    Profiles(ProfilesSynced),
    Event(SyncEvent),
    Changed,
    Remote(Arc<dyn RemoteRoom>, ToRemote),
    Announce(Arc<dyn Nearby>, Option<NearbyAnnouncement>),
    Browse(Arc<dyn Nearby>, bool),
}

/// A device's sync. See the module.
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Syncing {
    settings: Arc<Settings>,
    sockets: Arc<dyn Sockets>,
    client: Arc<dyn SyncClient>,
    /// Makes the executor sync runs on, when it starts.
    executor: Box<dyn Fn() -> Arc<dyn Tasks> + Send + Sync>,
    inner: Mutex<Inner>,
    /// Socket ids, unique across the links.
    sockets_made: Arc<AtomicU64>,
    /// Wakes the timer task: what it waits for may have changed.
    wake: Arc<Notify>,
    /// The saved profiles changed: the group hears of it at the next turn.
    profiles_changed: AtomicBool,
    /// When a keep-alive is due while they ride on the audio's wake-ups (executor clock); i64::MAX when
    /// they do not.
    woken_ping_at: AtomicI64,
    me: Weak<Syncing>,
}

impl Syncing {
    /// Sync for a device of `reach` over `settings`: it starts now if the device is paired and sync is
    /// on, otherwise when that changes ([`Syncing::settings_changed`]) or pairing starts. `executor`
    /// makes the executor it then runs on.
    pub fn new(settings: Arc<Settings>, sockets: Arc<dyn Sockets>, client: Arc<dyn SyncClient>, reach: Reach, executor: Box<dyn Fn() -> Arc<dyn Tasks> + Send + Sync>) -> Arc<Syncing> {
        let syncing = Arc::new_cyclic(|me| Syncing {
            settings,
            sockets,
            client,
            executor,
            inner: Mutex::new(Inner { presence: Presence::new(reach), running: None, pairing: PairingView::Idle, remote: None, clock: None, nearby: NearbyState::default() }),
            sockets_made: Arc::default(),
            wake: Arc::default(),
            profiles_changed: AtomicBool::new(false),
            woken_ping_at: AtomicI64::new(i64::MAX),
            me: me.clone(),
        });
        // The client is told nothing yet: it has no sync to read until this returns.
        if syncing.sync_on() && paired(&syncing.settings) {
            syncing.start(&mut syncing.inner());
        }
        syncing
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sync_on(&self) -> bool {
        self.settings.with_prefs(|p| p.sync).unwrap_or(false)
    }

    /// The executor's clock while sync runs, the device's otherwise (the same clock natively).
    fn now(inner: &Inner) -> i64 {
        inner.running.as_ref().map_or_else(nori_time::device_us, |r| r.tasks.now_us())
    }

    /// Runs `f` over the state, then catches up with what the links heard and tells the client what
    /// came of it.
    fn with<R>(&self, f: impl FnOnce(&Syncing, &mut Inner, i64) -> R) -> R {
        let mut inner = self.inner();
        let now = Syncing::now(&inner);
        let shown = self.shown(&inner, now);
        for link in inner.running.iter().flat_map(Running::links) {
            link.at(now);
        }
        let due = self.next_due(&inner);
        let r = f(self, &mut inner, now);
        let mut tells = Vec::new();
        // `f` may have started sync on an executor of its own.
        let now = Syncing::now(&inner);
        self.pump(&mut inner, now, &mut tells);
        if inner.running.as_ref().is_some_and(|r| r.paired.is_none() && r.pair.is_none()) {
            self.stop(&mut inner);
        }
        Syncing::tell_remote(&mut inner, &mut tells);
        self.tell_nearby(&mut inner, &mut tells);
        if self.shown(&inner, now) != shown {
            tells.push(Tell::Changed);
        }
        self.woken_ping_at.store(self.woken_ping(&inner), Ordering::Relaxed);
        let woke = self.next_due(&inner) != due;
        drop(inner);
        if woke {
            self.wake.notify();
        }
        self.tell(tells);
        r
    }

    /// Starts sync on an executor of its own, whose clock it reads from then on.
    fn start(&self, inner: &mut Inner) {
        let tasks = (self.executor)();
        let now = tasks.now_us();
        let running = Running { unix_offset: nori_time::unix_ms() * 1000 - tasks.now_us(), tasks: tasks.clone(), paired: None, pair: None, stop: Arc::default() };
        let (me, wake, stop) = (self.me.clone(), self.wake.clone(), running.stop.clone());
        tasks.spawn(Box::pin(timers(me, tasks.clone(), wake, stop)));
        let me = self.me.clone();
        self.settings.watch_profiles(Some(Arc::new(move || {
            if let Some(s) = me.upgrade() {
                s.profiles_changed.store(true, Ordering::Relaxed);
                s.wake.notify();
            }
        })));
        inner.running = Some(running);
        self.open_session(inner, now);
    }

    /// Closes every connection; the executor ends with its task. A pairing under way ends as refused.
    fn stop(&self, inner: &mut Inner) {
        self.settings.watch_profiles(None);
        let Some(r) = inner.running.take() else { return };
        r.links().for_each(Link::leave_all);
        r.stop.store(true, Ordering::Relaxed);
        // The timer task may wait for nothing but a wake: it ends, and its executor with it.
        self.wake.notify();
        if r.pair.is_some() {
            inner.pairing = PairingView::Refused { reason: PairingRefused::SyncOff };
        }
    }

    /// The session the settings keep, connected if the device holds its connection now.
    fn open_session(&self, inner: &mut Inner, now: i64) {
        let Some(running) = inner.running.as_mut() else { return };
        let Some(session) = Session::open(self.settings.clone()) else { return };
        running.paired = Some(self.paired(session, running, &inner.presence, now));
    }

    fn paired(&self, session: Session, running: &Running, presence: &Presence, now: i64) -> Paired {
        let link = Link::new(session.server().to_string(), self.sockets.clone(), self.sockets_made.clone(), now, now + running.unix_offset);
        let resting = !presence.wants(now);
        if !resting {
            session.connect(&link);
        }
        Paired { session, link, resting }
    }

    /// The next moment the timer task acts at, on the executor's clock.
    fn next_due(&self, inner: &Inner) -> Option<i64> {
        let r = inner.running.as_ref()?;
        let p = &inner.presence;
        let pings = p.pings() == Pings::Timer;
        let link_due = |l: &Link| [l.due_at(), pings.then(|| l.ping_at()).flatten()].into_iter().flatten().min();
        let session = r.paired.as_ref().filter(|p| !p.resting).and_then(|paired| [link_due(&paired.link), p.let_go_at()].into_iter().flatten().min());
        [session, r.pair.as_ref().and_then(|pair| link_due(pair.link()))].into_iter().flatten().min()
    }

    fn woken_ping(&self, inner: &Inner) -> i64 {
        let Some(paired) = inner.running.as_ref().and_then(|r| r.paired.as_ref()).filter(|p| !p.resting) else { return i64::MAX };
        if inner.presence.pings() != Pings::Woken {
            return i64::MAX;
        }
        paired.link.ping_at().unwrap_or(i64::MAX)
    }

    /// What is due now: connecting again, keep-alives on a timer, letting go, telling the group of
    /// profiles changed here.
    fn tick(&self) {
        let profiles = self.profiles_changed.swap(false, Ordering::Relaxed);
        let mut tells = Vec::new();
        self.with(|s, inner, now| {
            let Some(running) = inner.running.as_mut() else { return };
            let p = &inner.presence;
            for link in running.links() {
                link.tick();
                if p.pings() == Pings::Timer {
                    link.keep_alive();
                }
            }
            let unix_ms = (now + running.unix_offset) / 1000;
            if let Some(paired) = running.paired.as_mut() {
                if !paired.resting && !p.wants(now) {
                    paired.link.leave_all();
                    paired.resting = true;
                }
                if profiles {
                    let servers = s.servers();
                    let here = s.client.here();
                    paired.session.changed(&paired.link, &servers, &here, unix_ms);
                    tells.extend(inner.remote.as_ref().map(|a| Tell::Remote(a.room.clone(), ToRemote::Changed)));
                }
            }
        });
        self.tell(tells);
    }

    fn servers(&self) -> Vec<SavedServer> {
        self.settings.with_prefs(|p| p.servers.clone()).unwrap_or_default()
    }

    /// Keeps `servers` in the settings, as sync left them, and tells the client.
    fn keep(&self, servers: Vec<SavedServer>, tells: &mut Vec<Tell>) {
        let mut told = None;
        self.settings.change(|p| {
            let (after, synced) = synced(p, servers);
            told = Some(synced);
            after
        });
        tells.extend(told.map(Tell::Profiles));
    }

    fn outcome(&self, out: Outcome, tells: &mut Vec<Tell>) {
        if let Some(servers) = out.servers {
            self.keep(servers, tells);
        }
        if let Some(local) = out.switch {
            self.open(local, tells);
        }
        tells.extend(out.events.into_iter().map(Tell::Event));
    }

    /// Opens profile `local` here, as a device of the group asked: the client opens the profile in use
    /// again, as when sync changed it.
    fn open(&self, local: String, tells: &mut Vec<Tell>) {
        let mut switched = false;
        self.settings.change(|p| {
            switched = p.active_server_id != local && p.servers.iter().any(|s| s.id == local);
            if switched { StoredPrefs { active_server_id: local.clone(), ..p.clone() } } else { p.clone() }
        });
        if switched {
            tells.push(Tell::Profiles(ProfilesSynced { removed: Vec::new(), active: ActiveProfile::Switched }));
        }
    }

    /// Hands remote control what the group room said for the profile it is in the room for.
    fn remote_heard(remote: Option<&Attached>, heard: Vec<RemoteHeard>, tells: &mut Vec<Tell>) {
        let Some((a, (ours, _))) = remote.and_then(|a| Some((a, a.in_room.as_ref()?))) else { return };
        for h in heard {
            let told = match h {
                RemoteHeard::Said { profile, device, message } if profile == *ours => ToRemote::Heard { device, message },
                RemoteHeard::Said { .. } => continue,
                RemoteHeard::Left { device } => ToRemote::Left { device },
                RemoteHeard::Opened => ToRemote::Elsewhere,
            };
            tells.push(Tell::Remote(a.room.clone(), told));
        }
    }

    /// Tells remote control where it stands when that changed: in the group room for its profile's
    /// sync identity or no longer, and the LAN key. Entering, it learns the server's clock.
    fn tell_remote(inner: &mut Inner, tells: &mut Vec<Tell>) {
        if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
            let open = inner.remote.as_ref().and_then(|a| Some(Open { profile: p.session.sync_id(&a.local)?.to_string(), remote: a.card.clone() }));
            let in_room = !p.resting && p.link.room(&p.session.room()) == Some(RoomState::Open);
            p.session.opened(&p.link, open, in_room);
        }
        let paired = inner.running.as_ref().and_then(|r| r.paired.as_ref());
        let open = paired.filter(|p| !p.resting && p.link.room(&p.session.room()) == Some(RoomState::Open));
        if open.is_none() {
            inner.clock = None;
        }
        let Some(a) = inner.remote.as_mut() else { return };
        let in_room = open.and_then(|p| Some((p.session.sync_id(&a.local)?.to_string(), p.session.room())));
        let lan_secret = paired.and_then(|p| Some(p.session.key().lan_secret(p.session.sync_id(&a.local)?)));
        let now = match (&a.in_room, &in_room) {
            (None, Some(_)) => Some(InRoom::Entered),
            (Some((_, was)), Some((_, room))) if was != room => Some(InRoom::Entered),
            (Some((was, _)), Some((id, _))) if was != id => Some(InRoom::Renamed),
            (Some(_), None) => Some(InRoom::Left),
            _ => None,
        };
        if let Some(now) = now {
            if let (Some(p), InRoom::Entered, None) = (open, now, inner.clock) {
                (0..TIME_ASKS).for_each(|_| p.link.ask_time(&p.session.room()));
            }
            tells.push(Tell::Remote(a.room.clone(), ToRemote::InRoom(now)));
            a.left_as = if now == InRoom::Left { a.in_room.take().map(|(id, _)| id) } else { None };
            a.in_room = in_room;
        }
        if lan_secret != a.lan_secret {
            a.lan_secret = lan_secret;
            tells.push(Tell::Remote(a.room.clone(), ToRemote::Keyed));
        }
    }

    /// Announces this device's code while its pairing screen shows one, for the device's server
    /// accounts; withdraws it otherwise.
    fn tell_nearby(&self, inner: &mut Inner, tells: &mut Vec<Tell>) {
        let Some(hook) = inner.nearby.hook.clone() else { return };
        let wanted = match &inner.pairing {
            PairingView::Code { link, .. } => nearby::announcement(link, &nearby::account_tags(&self.servers())),
            _ => None,
        };
        if wanted != inner.nearby.announced {
            inner.nearby.announced = wanted.clone();
            tells.push(Tell::Announce(hook, wanted));
        }
    }

    /// Hands the session and the pairing what their links heard, until nothing more comes.
    fn pump(&self, inner: &mut Inner, now: i64, tells: &mut Vec<Tell>) {
        let Inner { presence, running, pairing, remote, clock, .. } = inner;
        let Some(running) = running.as_mut() else { return };
        let unix_ms = (now + running.unix_offset) / 1000;
        let mut here = None;
        loop {
            let mut any = false;
            if let Some(paired) = running.paired.as_mut() {
                for h in paired.link.heard() {
                    any = true;
                    if let Heard::Time { room, ms, rtt_us } = &h {
                        if *room == paired.session.room() && clock.is_none_or(|(best, _)| *rtt_us < best) {
                            *clock = Some((*rtt_us, ms * 1000 - now));
                        }
                        continue;
                    }
                    let here = here.get_or_insert_with(|| self.client.here());
                    let mut out = paired.session.heard(&paired.link, h, &self.servers(), here, unix_ms);
                    Syncing::remote_heard(remote.as_ref(), std::mem::take(&mut out.remote), tells);
                    self.outcome(out, tells);
                }
                // A phone paused in the background lets go of a connection that dropped.
                let dropped = matches!(paired.link.room(&paired.session.room()), Some(RoomState::Waiting { .. }));
                if !paired.resting && dropped && !presence.reconnects() {
                    paired.link.leave_all();
                    paired.resting = true;
                }
            }
            let heard = running.pair.as_ref().map(|p| p.link().heard()).unwrap_or_default();
            for h in heard {
                any = true;
                self.pairing_heard(running, presence, pairing, h, now);
            }
            if !any {
                break;
            }
        }
    }

    /// What a client shows of sync: the state, the pairing, which of the group's devices are online, and
    /// the devices waiting nearby.
    fn shown(&self, inner: &Inner, now: i64) -> (SyncState, PairingView, Vec<(String, bool)>, Vec<NearbyDevice>) {
        let devices = inner.running.as_ref().and_then(|r| r.paired.as_ref()).map(|p| p.session.devices().into_iter().map(|d| (d.device, d.online)).collect()).unwrap_or_default();
        (self.state_of(inner, now), inner.pairing.clone(), devices, inner.nearby.found.clone())
    }

    /// One thing the pairing's link heard.
    fn pairing_heard(&self, running: &mut Running, presence: &Presence, view: &mut PairingView, heard: Heard, now: i64) {
        let servers = self.servers();
        let Running { paired, pair, .. } = &mut *running;
        let step = match pair.as_mut() {
            Some(Pair::Joining(joining, link)) => match joining.heard(link, heard, Duration::from_micros(now.max(0) as u64)) {
                Some(step) => step,
                None => return,
            },
            Some(Pair::Admitting(admitting, link)) => {
                match admitting.heard(link, heard, paired.as_ref().map(|p| &p.session), &servers) {
                    Some(AdmitStep::Confirm(confirmation)) => *view = PairingView::Confirm { confirmation },
                    Some(AdmitStep::Sent) => *view = PairingView::Sent,
                    Some(AdmitStep::Refused(reason)) => {
                        *view = PairingView::Refused { reason };
                        *pair = None;
                    }
                    None => {}
                }
                return;
            }
            None => return,
        };
        match step {
            JoinStep::Code { link, words } => *view = PairingView::Code { link, words },
            JoinStep::Paired(welcomed) => {
                *pair = None;
                let session = match paired.take() {
                    Some(Paired { session, link, .. }) => welcomed.join(Some((session, &link)), self.settings.clone()),
                    None => welcomed.join(None, self.settings.clone()),
                };
                running.paired = Some(self.paired(session, running, presence, now));
                *view = PairingView::Paired;
            }
            JoinStep::Refused(reason) => {
                *view = PairingView::Refused { reason };
                if !matches!(reason, PairingRefused::Tampered | PairingRefused::Spent) {
                    pair.take().iter().for_each(|p| p.link().leave_all());
                }
            }
        }
    }

    fn state_of(&self, inner: &Inner, now: i64) -> SyncState {
        let Some((running, paired)) = inner.running.as_ref().and_then(|r| Some((r, r.paired.as_ref()?))) else { return SyncState::Idle };
        if paired.resting {
            return SyncState::Resting;
        }
        match paired.link.room(&paired.session.room()) {
            Some(RoomState::Open) => SyncState::Connected,
            Some(RoomState::Waiting { until, refused }) => {
                let retry_ms = (until.max(now) + running.unix_offset) / 1000;
                match refused {
                    Some(refused) => SyncState::Refused { refused, retry_ms },
                    None => SyncState::Offline { retry_ms },
                }
            }
            Some(RoomState::Stopped(refused)) => SyncState::Stopped { refused },
            Some(RoomState::Opening) | None => SyncState::Connecting,
        }
    }

    /// The Unix time in ms while sync runs.
    fn unix_ms(inner: &Inner, now: i64) -> i64 {
        inner.running.as_ref().map_or_else(nori_time::unix_ms, |r| (now + r.unix_offset) / 1000)
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
impl Syncing {
    /// Sync for a native client, on a thread of its own made only when sync starts.
    pub fn native(settings: Arc<Settings>, sockets: Arc<dyn Sockets>, client: Arc<dyn SyncClient>, reach: Reach) -> Arc<Syncing> {
        Syncing::new(settings, sockets, client, reach, Box::new(|| Arc::new(nori_tasks::Native::new("nori-sync").expect("a thread for sync")) as Arc<dyn Tasks>))
    }
}

#[cfg(feature = "ffi")]
#[uniffi::export]
impl Syncing {
    /// A phone's sync (Android).
    #[uniffi::constructor]
    pub fn on_phone(settings: Arc<Settings>, sockets: Arc<dyn Sockets>, client: Arc<dyn SyncClient>) -> Arc<Syncing> {
        Syncing::native(settings, sockets, client, Reach::Phone)
    }
}

/// The calls a client makes.
#[cfg_attr(feature = "ffi", uniffi::export)]
impl Syncing {
    /// The sync and sync server settings, or whether the device is paired, may have changed: sync starts
    /// or stops to match. Off closes every connection and forgets nothing.
    pub fn settings_changed(&self) {
        self.with(|s, inner, _| {
            let wanted = s.sync_on() && (paired(&s.settings) || inner.running.as_ref().is_some_and(|r| r.pair.is_some()));
            match (wanted, inner.running.is_some()) {
                (true, false) => s.start(inner),
                (false, true) => s.stop(inner),
                _ => {}
            }
        });
    }

    /// A frame from socket `id`.
    pub fn frame(&self, id: SocketId, frame: Vec<u8>) {
        self.with(|_, inner, _| inner.running.iter().flat_map(Running::links).for_each(|l| l.frame(id, &frame)));
    }

    /// Socket `id` ended.
    pub fn ended(&self, id: SocketId, how: Ended) {
        self.with(|_, inner, _| inner.running.iter().flat_map(Running::links).for_each(|l| l.ended(id, how)));
    }

    /// nori is on screen, or no longer.
    pub fn foreground(&self, on: bool) {
        self.with(|s, inner, now| {
            inner.presence.foreground(on, now);
            s.come_back(inner, now);
        });
    }

    /// nori plays, or no longer.
    pub fn playing(&self, on: bool) {
        self.with(|s, inner, now| {
            inner.presence.playing(on, now);
            s.come_back(inner, now);
        });
    }

    /// The audio woke the device: a keep-alive goes out if one is due while they ride on these wakes.
    /// Cheap when none is due, as it is called on every wake of the audio.
    pub fn woke(&self) {
        let at = self.woken_ping_at.load(Ordering::Relaxed);
        if at == i64::MAX {
            return;
        }
        self.with(|_, inner, now| {
            if now >= at {
                inner.running.iter().flat_map(|r| r.paired.as_ref()).for_each(|p| p.link.keep_alive());
            }
        });
    }

    pub fn state(&self) -> SyncState {
        let inner = self.inner();
        self.state_of(&inner, Syncing::now(&inner))
    }

    /// The group's other devices, with when each was last seen; none while not paired.
    pub fn devices(&self) -> Vec<Seen> {
        self.inner().running.as_ref().and_then(|r| r.paired.as_ref()).map(|p| p.session.devices()).unwrap_or_default()
    }

    /// Whether the group keeps an encrypted copy on the server.
    pub fn keeps_copy(&self) -> bool {
        self.inner().running.as_ref().and_then(|r| r.paired.as_ref()).is_some_and(|p| p.session.keeps_copy())
    }

    /// Turns the group's kept copy on or off; off deletes it from the server.
    pub fn keep_copy(&self, on: bool) {
        self.with(|_, inner, now| {
            let unix_ms = Syncing::unix_ms(inner, now);
            if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
                p.session.keep_copy(&p.link, on, unix_ms);
            }
        });
    }

    /// What Reset pairing does, for its confirmation; None while not paired.
    pub fn reset_notice(&self) -> Option<ResetNotice> {
        self.inner().running.as_ref().and_then(|r| r.paired.as_ref()).map(|p| p.session.reset_notice())
    }

    /// Reset pairing: this device leaves its group for one of its own, with a new key.
    pub fn reset(&self) {
        self.with(|_, inner, _| {
            if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
                p.session.reset(&p.link);
            }
        });
    }

    /// Whether saved profile `local` syncs: removing it removes it from every paired device.
    pub fn synced(&self, local: String) -> bool {
        self.inner().running.as_ref().and_then(|r| r.paired.as_ref()).is_some_and(|p| p.session.synced(&local))
    }

    /// Sync this profile off for `local`; with `elsewhere`, it is removed from the other devices too.
    /// The client then keeps the profile's `sync_off`.
    pub fn sync_off(&self, local: String, elsewhere: bool) {
        self.with(|_, inner, now| {
            let unix_ms = Syncing::unix_ms(inner, now);
            if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
                p.session.sync_off(&p.link, &local, elsewhere, unix_ms);
            }
        });
    }

    /// Profiles that may be one, for the user to merge.
    pub fn offers(&self) -> Vec<MergeOffer> {
        let servers = self.servers();
        self.inner().running.as_ref().and_then(|r| r.paired.as_ref()).map(|p| p.session.offers(&servers)).unwrap_or_default()
    }

    /// Makes the two profiles of `offer` one on every device: the one in use stays, else the first.
    pub fn merge(&self, offer: MergeOffer) {
        let active = self.settings.with_prefs(|p| p.active_server_id.clone()).unwrap_or_default();
        let (keep, drop) = offer.kept(&active);
        let mut tells = Vec::new();
        self.with(|s, inner, now| {
            let unix_ms = Syncing::unix_ms(inner, now);
            if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
                let servers = p.session.merge(&p.link, keep, drop, &s.servers(), unix_ms);
                s.keep(servers, &mut tells);
            }
        });
        self.tell(tells);
    }

    /// Profiles `a` and `b` are not one: they are offered for merging no more.
    pub fn keep_apart(&self, a: String, b: String) {
        self.with(|_, inner, _| {
            if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
                p.session.keep_apart(&a, &b);
            }
        });
    }

    /// A sign-in here to `account`, called `name` by its provider, for profile `local`: accepted only
    /// for the account the profile syncs with.
    pub fn signed_in(&self, local: String, account: Option<String>, name: Option<String>) -> SignInCheck {
        self.with(|_, inner, now| {
            let unix_ms = Syncing::unix_ms(inner, now);
            match inner.running.as_mut().and_then(|r| r.paired.as_mut()) {
                Some(p) => p.session.signed_in(&p.link, &local, account, name, unix_ms),
                None => SignInCheck::Accepted,
            }
        })
    }

    /// The plugins the synced profiles play from that this device runs plugins for and lacks: Install
    /// brings them all, each from its repo.
    pub fn needed_plugins(&self) -> Vec<NeededPlugin> {
        let here = self.client.here();
        let inner = self.inner();
        inner.running.as_ref().and_then(|r| r.paired.as_ref()).map(|p| p.session.needed(&here)).unwrap_or_default()
    }

    pub fn pairing(&self) -> PairingView {
        self.inner().pairing.clone()
    }

    /// Shows a code on this device, `name`, to pair it into a group on the sync server the settings
    /// name. It leaves the group it is in once paired.
    pub fn pair_show(&self, name: String) {
        self.pair_begin(|s, running, now| {
            let server = sync_server(s.sync_on(), s.settings.with_prefs(|p| p.sync_server.clone()).flatten())?;
            let link = Link::new(server.clone(), s.sockets.clone(), s.sockets_made.clone(), now, now + running.unix_offset);
            let plays = s.client.here().plays;
            let joining = Joining::start(&link, server, name, plays, running.paired.as_ref().map(|p| &p.session), Duration::from_micros(now.max(0) as u64));
            Some(Pair::Joining(joining, link))
        });
    }

    /// Takes a code another device shows: scanned, pasted, or from a nearby device. A code that does
    /// not read is refused at once; otherwise [`Syncing::pairing`] comes to what to confirm.
    pub fn pair_take(&self, text: String, came: Came) -> Option<PairingRefused> {
        let mut refused = None;
        self.pair_begin(|s, running, now| {
            let own = running.paired.as_ref().map(|p| &p.session);
            let admitting = match Admitting::read(&text, came, own) {
                Ok(a) => a,
                Err(r) => {
                    refused = Some(r);
                    return None;
                }
            };
            let link = Link::new(admitting.server().to_string(), s.sockets.clone(), s.sockets_made.clone(), now, now + running.unix_offset);
            admitting.check(&link);
            Some(Pair::Admitting(admitting, link))
        });
        if let Some(reason) = refused {
            self.with(|_, inner, _| inner.pairing = PairingView::Refused { reason });
        }
        refused
    }

    /// The user confirmed on the device with the profiles; `name` is this device's, `keep_copy` the
    /// answer to the first pairing's question. Once the key went, the client hears that the new device
    /// was paired, once: its arrival in the group's room is no news here.
    pub fn pair_confirm(&self, name: String, keep_copy: bool) {
        let mut tells = Vec::new();
        self.with(|s, inner, now| {
            let Inner { presence, running, pairing, .. } = inner;
            let Some(running) = running.as_mut() else { return };
            let unix_ms = (now + running.unix_offset) / 1000;
            let Some(Pair::Admitting(admitting, link)) = running.pair.as_mut() else { return };
            let new = admitting.name().to_string();
            let step = match running.paired.as_mut() {
                Some(own) => admitting.admit(link, &mut own.session, unix_ms),
                None => match admitting.admit_first(link, name, keep_copy, s.settings.clone(), unix_ms) {
                    Ok(session) => {
                        running.paired = Some(s.paired(session, running, presence, now));
                        Some(AdmitStep::Sent)
                    }
                    Err(reason) => Some(AdmitStep::Refused(reason)),
                },
            };
            *pairing = match step {
                Some(AdmitStep::Refused(reason)) => PairingView::Refused { reason },
                Some(_) => {
                    tells.push(Tell::Event(SyncEvent::NewDevice { name: new }));
                    PairingView::Sent
                }
                None => return,
            };
            running.pair = None;
        });
        self.tell(tells);
    }

    /// Where the nearby way of pairing reaches the local network: the platform's mDNS.
    pub fn nearby_with(&self, nearby: Arc<dyn Nearby>) {
        self.with(|_, inner, _| inner.nearby.hook = Some(nearby));
    }

    /// A pairing screen opened here (`on`) or closed: while open, it lists the devices nearby that wait to
    /// be paired and have a server account of this device's ([`Syncing::nearby`]).
    pub fn pair_look(&self, on: bool) {
        let mut tells = Vec::new();
        self.with(|_, inner, _| {
            let n = &mut inner.nearby;
            if n.looking == on {
                return;
            }
            n.looking = on;
            if !on {
                n.found.clear();
            }
            tells.extend(n.hook.clone().map(|hook| Tell::Browse(hook, on)));
        });
        self.tell(tells);
    }

    /// The platform's mDNS found `service`, a device waiting to be paired, with TXT `txt`.
    pub fn nearby_found(&self, service: String, txt: Vec<NearbyTxt>) {
        let ours = nearby::account_tags(&self.servers());
        self.with(|_, inner, _| {
            let n = &mut inner.nearby;
            let Some((link, name)) = nearby::code(&txt, &ours).and_then(|link| Some((link.clone(), crate::pairing::code_name(&link)?))).filter(|_| n.looking) else { return };
            n.found.retain(|d| d.service != service);
            n.found.push(NearbyDevice { service, name, link });
        });
    }

    /// The platform's mDNS saw `service` go.
    pub fn nearby_lost(&self, service: String) {
        self.with(|_, inner, _| inner.nearby.found.retain(|d| d.service != service));
    }

    /// The devices waiting to be paired that a pairing screen lists: take one with
    /// [`Syncing::pair_take`], came [`Came::Nearby`].
    pub fn nearby(&self) -> Vec<NearbyDevice> {
        self.inner().nearby.found.clone()
    }

    /// Stops the pairing under way, or forgets how the last one ended.
    pub fn pair_cancel(&self) {
        self.with(|_, inner, _| {
            if let Some(pair) = inner.running.as_mut().and_then(|r| r.pair.take()) {
                pair.link().leave_all();
            }
            inner.pairing = PairingView::Idle;
        });
    }
}

/// Remote control's side of the group room, for the core.
impl Syncing {
    /// Attaches `room`, the remote of profile `local` (its id here), which says it is `card`: while the
    /// device is in its group room and the profile syncs, it hears what the group's devices with that
    /// profile open say, and what it says reaches them. The group's devices with another profile open
    /// list it with `card`. Replaces the one attached before.
    pub fn remote_open(&self, local: String, room: Arc<dyn RemoteRoom>, card: serde_json::Value) {
        self.with(|_, inner, _| inner.remote = Some(Attached { local, room, card, in_room: None, left_as: None, lan_secret: None }));
    }

    /// Detaches the remote attached, if it is `room`.
    pub fn remote_close(&self, room: &Arc<dyn RemoteRoom>) {
        self.with(|_, inner, _| {
            if inner.remote.as_ref().is_some_and(|a| Arc::ptr_eq(&a.room, room)) {
                inner.remote = None;
            }
        });
    }

    /// The group's devices in the room that play profile `local` (its id here) but have another one
    /// open for remote control, with who their remote control says it is.
    pub fn remote_elsewhere(&self, local: &str) -> Vec<Elsewhere> {
        let kind = self.settings.with_prefs(|p| p.servers.iter().find(|s| s.id == local).map(|s| s.kind.clone())).flatten();
        let inner = self.inner();
        let Some((p, kind)) = inner.running.as_ref().and_then(|r| r.paired.as_ref()).zip(kind) else { return Vec::new() };
        p.session.sync_id(local).map(|id| p.session.elsewhere(id, &kind)).unwrap_or_default()
    }

    /// Asks device `device` (its group id) to open profile `local` (its id here) for remote control;
    /// false when the profile does not sync or the device is not in the room.
    pub fn remote_switch(&self, local: &str, device: &str) -> bool {
        self.with(|_, inner, _| {
            let Some(p) = inner.running.as_ref().and_then(|r| r.paired.as_ref()).filter(|p| !p.resting) else { return false };
            p.session.sync_id(local).is_some_and(|id| p.session.switch(&p.link, device, id))
        })
    }

    /// Says remote control's `message` for profile `local` (its id here) to device `to` (its group id),
    /// or to every device with the same profile open; false when it cannot go: not in the room, the
    /// profile does not sync, or `to` is not there. It goes whether a remote is attached or not, so a
    /// remote's last word goes after it was detached, and after the profile stopped syncing, as the
    /// identity it was in the room with.
    pub fn remote_say(&self, local: &str, to: Option<String>, message: serde_json::Value) -> bool {
        self.with(|_, inner, _| {
            let Some(p) = inner.running.as_ref().and_then(|r| r.paired.as_ref()).filter(|p| !p.resting && p.link.room(&p.session.room()) == Some(RoomState::Open)) else { return false };
            let left_as = inner.remote.as_ref().filter(|a| a.local == local).and_then(|a| a.left_as.clone());
            let Some(profile) = p.session.sync_id(local).map(str::to_string).or(left_as) else { return false };
            p.session.remote_say(&p.link, to.as_deref(), &profile, message)
        })
    }

    /// The sync server's clock less the clock this device's executors read, µs, once the server told it
    /// in the room remote control is in.
    pub fn server_clock(&self) -> Option<i64> {
        self.inner().clock.map(|(_, offset)| offset)
    }

    /// What the group's devices with profile `local` open sign their requests to each other's LAN doors
    /// with and announce them by; None while not paired or the profile does not sync.
    pub fn lan_secret(&self, local: &str) -> Option<String> {
        let inner = self.inner();
        inner.remote.as_ref().filter(|a| a.local == local)?.lan_secret.clone()
    }

    /// What this device says first to a device of its group it reaches on the network, through that
    /// device's LAN door; None while not paired.
    pub fn direct_hello(&self) -> Option<Vec<u8>> {
        let here = self.client.here();
        let inner = self.inner();
        Some(inner.running.as_ref()?.paired.as_ref()?.session.direct_hello(&here))
    }

    /// What a device of the group reached on the network said ([`Syncing::direct_hello`] and what
    /// followed it); what to say back to it. Profile sync takes it as it takes what comes through the
    /// server, so the group's devices on one network sync without the server too.
    pub fn direct_heard(&self, payloads: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        let (servers, here) = (self.servers(), self.client.here());
        let mut tells = Vec::new();
        let answer = self.with(|s, inner, now| {
            let unix_ms = Syncing::unix_ms(inner, now);
            let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()) else { return Vec::new() };
            let (out, answer) = p.session.direct(&p.link, payloads, &servers, &here, unix_ms);
            s.outcome(out, &mut tells);
            answer
        });
        self.tell(tells);
        answer
    }

    /// This device's id in its group; None while not paired.
    pub fn device(&self) -> Option<String> {
        let inner = self.inner();
        Some(inner.running.as_ref()?.paired.as_ref()?.session.group().device.clone())
    }
}

impl Syncing {
    /// Starts a pairing that `begin` sets up, sync starting for it if it does not run.
    fn pair_begin(&self, begin: impl FnOnce(&Syncing, &Running, i64) -> Option<Pair>) {
        self.with(|s, inner, _| {
            if !s.sync_on() {
                inner.pairing = PairingView::Refused { reason: PairingRefused::SyncOff };
                return;
            }
            if inner.running.is_none() {
                s.start(inner);
            }
            let now = Syncing::now(inner);
            let running = inner.running.as_mut().expect("started");
            if let Some(old) = running.pair.take() {
                old.link().leave_all();
            }
            running.pair = begin(s, running, now);
            if running.pair.is_some() {
                inner.pairing = PairingView::Starting;
            }
        });
    }

    /// The device holds its connection again if it let go and wants it now.
    fn come_back(&self, inner: &mut Inner, now: i64) {
        let wants = inner.presence.wants(now);
        if let Some(p) = inner.running.as_mut().and_then(|r| r.paired.as_mut()).filter(|p| p.resting && wants) {
            p.resting = false;
            p.session.connect(&p.link);
        }
    }

    fn tell(&self, tells: Vec<Tell>) {
        for t in tells {
            match t {
                Tell::Profiles(p) => self.client.profiles(p),
                Tell::Event(e) => self.client.event(e),
                Tell::Changed => self.client.changed(),
                Tell::Announce(hook, waiting) => hook.announce(waiting),
                Tell::Browse(hook, on) => hook.browse(on),
                Tell::Remote(room, ToRemote::InRoom(now)) => room.in_room(now),
                Tell::Remote(room, ToRemote::Keyed) => room.keyed(),
                Tell::Remote(room, ToRemote::Changed) => room.changed(),
                Tell::Remote(room, ToRemote::Heard { device, message }) => room.heard(device, message),
                Tell::Remote(room, ToRemote::Left { device }) => room.left(device),
                Tell::Remote(room, ToRemote::Elsewhere) => room.elsewhere(),
            }
        }
    }
}

/// The timer task: sleeps until the next thing due, or until something changes what that is.
async fn timers(me: Weak<Syncing>, tasks: Arc<dyn Tasks>, wake: Arc<Notify>, stop: Arc<AtomicBool>) {
    loop {
        let woken = wake.notified();
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(s) = me.upgrade() else { return };
        let due = s.next_due(&s.inner());
        let profiles = s.profiles_changed.load(Ordering::Relaxed);
        drop(s);
        let now = tasks.now_us();
        match due {
            _ if profiles => {}
            Some(at) if at <= now => {}
            Some(at) => {
                let sleep = tasks.sleep(nori_time::Duration::from_micros((at - now) as u64));
                if let Either::Right(_) = select(sleep, std::pin::pin!(woken)).await {
                    continue;
                }
            }
            None => {
                woken.await;
                continue;
            }
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(s) = me.upgrade() else { return };
        s.tick();
    }
}

#[cfg(test)]
mod tests;
