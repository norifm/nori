//! A sync server in memory, for tests: rooms, members, the kept copy and the server's clock, which the
//! test moves. Each device's link keeps what it heard until the test takes it, so devices run in
//! turns, with no thread and no sleeping. [`HubSockets`] is a device's WebSockets to it, speaking the
//! protocol, for a whole [`crate::Syncing`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use nori_tasks::Tasks;

use crate::keys::RoomName;
use crate::link::{Ended, SocketId, Sockets};
use crate::room::{Asked, Heard, Member, Refusal, Relay, RoomKind, To};
use crate::wire::{read_up, Code, Down, Up, Welcome};
use crate::Syncing;

/// How long a meeting room stays open, and how long it then refuses connections.
pub const MEETING_MS: i64 = 5 * 60 * 1000;
const MEETING_CLOSED_MS: i64 = 60 * 60 * 1000;
/// The largest message the server passes.
const MESSAGE_BYTES: usize = 64 * 1024;

struct Room {
    kind: RoomKind,
    opened_ms: i64,
    /// Each member and the link it is.
    members: Vec<(Member, usize)>,
}

/// The server's clock.
enum Clock {
    /// Unix ms, moved by the test ([`Hub::advance`]).
    Moved(i64),
    /// The device's own, for a test that runs in real time.
    Real,
}

impl Default for Clock {
    fn default() -> Self {
        Clock::Moved(0)
    }
}

#[derive(Default)]
struct State {
    clock: Clock,
    rooms: BTreeMap<RoomName, Room>,
    /// Meeting rooms whose time ran out, and when.
    closed: BTreeMap<RoomName, i64>,
    copies: BTreeMap<RoomName, ([u8; 32], Vec<u8>)>,
    inboxes: Vec<VecDeque<Heard>>,
    next_member: u32,
    wall: bool,
    /// The devices whose sockets hand on what they hear by themselves.
    running: Vec<Weak<HubSockets>>,
}

impl State {
    fn now_ms(&self) -> i64 {
        match self.clock {
            Clock::Moved(ms) => ms,
            Clock::Real => nori_time::unix_ms(),
        }
    }

    fn tell(&mut self, link: usize, heard: Heard) {
        self.inboxes[link].push_back(heard);
    }

    fn refuse(&mut self, link: usize, room: &RoomName, asked: Asked, reason: Refusal) {
        self.tell(link, Heard::Refused { room: room.clone(), asked, reason });
    }

    /// `link` leaves `room`; the others hear it, and an empty room closes.
    fn leave(&mut self, room: &RoomName, link: usize) {
        let Some(r) = self.rooms.get_mut(room) else { return };
        let Some(at) = r.members.iter().position(|(_, l)| *l == link) else { return };
        let (member, _) = r.members.remove(at);
        let others: Vec<usize> = r.members.iter().map(|(_, l)| *l).collect();
        if others.is_empty() {
            self.rooms.remove(room);
        }
        for other in others {
            self.tell(other, Heard::Left { room: room.clone(), member });
        }
    }
}

/// The server.
#[derive(Clone, Default)]
pub struct Hub(Arc<Mutex<State>>);

/// One device's connection to the [`Hub`].
pub struct HubLink {
    hub: Arc<Mutex<State>>,
    link: usize,
}

impl Hub {
    /// A server whose clock reads `now_ms` until the test moves it.
    pub fn new(now_ms: i64) -> Hub {
        Hub(Arc::new(Mutex::new(State { clock: Clock::Moved(now_ms), ..State::default() })))
    }

    /// A server whose clock is the device's, for a test that runs in real time.
    pub fn real_time() -> Hub {
        Hub(Arc::new(Mutex::new(State { clock: Clock::Real, ..State::default() })))
    }

    pub fn link(&self) -> HubLink {
        let mut s = self.0.lock().unwrap();
        s.inboxes.push(VecDeque::new());
        HubLink { hub: self.0.clone(), link: s.inboxes.len() - 1 }
    }

    /// Moves the clock on by `ms`; meeting rooms whose time ran out close.
    pub fn advance(&self, ms: i64) {
        self.advanced(ms);
        self.stir();
    }

    /// Has every device whose sockets run by themselves take what it heard, as a task of its own.
    fn stir(&self) {
        let running: Vec<Arc<HubSockets>> = self.0.lock().unwrap().running.iter().filter_map(Weak::upgrade).collect();
        running.iter().for_each(|n| n.due());
    }

    fn advanced(&self, ms: i64) {
        let mut s = self.0.lock().unwrap();
        if let Clock::Moved(now) = &mut s.clock {
            *now += ms;
        }
        let now = s.now_ms();
        let over: Vec<RoomName> = s.rooms.iter().filter(|(_, r)| r.kind == RoomKind::Meeting && now - r.opened_ms >= MEETING_MS).map(|(n, _)| n.clone()).collect();
        s.closed.retain(|_, at| now - *at < MEETING_CLOSED_MS);
        for name in over {
            s.closed.insert(name.clone(), now);
            let room = s.rooms.remove(&name).expect("listed");
            for (_, link) in room.members {
                s.tell(link, Heard::Closed { room: name.clone() });
            }
        }
    }

    /// The daily wall: while up, every request is refused.
    pub fn wall(&self, up: bool) {
        self.0.lock().unwrap().wall = up;
    }

    /// The kept copy of `room`, as stored.
    pub fn copy(&self, room: &RoomName) -> Option<Vec<u8>> {
        self.0.lock().unwrap().copies.get(room).map(|(_, blob)| blob.clone())
    }

    /// How many members `room` has.
    pub fn members(&self, room: &RoomName) -> usize {
        self.0.lock().unwrap().rooms.get(room).map_or(0, |r| r.members.len())
    }
}

impl HubLink {
    /// What this device heard since it last asked.
    pub fn heard(&self) -> Vec<Heard> {
        self.hub.lock().unwrap().inboxes[self.link].drain(..).collect()
    }

    /// The connection drops: it leaves every room.
    pub fn drop_connection(&self) {
        let mut s = self.hub.lock().unwrap();
        let rooms: Vec<RoomName> = s.rooms.iter().filter(|(_, r)| r.members.iter().any(|(_, l)| *l == self.link)).map(|(n, _)| n.clone()).collect();
        for room in rooms {
            s.leave(&room, self.link);
        }
    }

    /// The state, unless the wall refuses `asked` in `room`.
    fn open(&self, room: &RoomName, asked: Asked) -> Option<std::sync::MutexGuard<'_, State>> {
        let mut s = self.hub.lock().unwrap();
        if s.wall {
            s.refuse(self.link, room, asked, Refusal::DailyWall);
            return None;
        }
        Some(s)
    }
}

impl Relay for HubLink {
    fn join(&self, room: &RoomName, kind: RoomKind) {
        let Some(mut s) = self.open(room, Asked::Join) else { return };
        if s.closed.contains_key(room) {
            return s.tell(self.link, Heard::Closed { room: room.clone() });
        }
        let now = s.now_ms();
        let member = Member(s.next_member);
        let r = s.rooms.entry(room.clone()).or_insert(Room { kind, opened_ms: now, members: Vec::new() });
        if r.kind == RoomKind::Meeting && r.members.len() >= 2 {
            return s.refuse(self.link, room, Asked::Join, Refusal::Limit);
        }
        let others = r.members.clone();
        r.members.push((member, self.link));
        s.next_member += 1;
        s.tell(self.link, Heard::Joined { room: room.clone(), me: member, members: others.iter().map(|(m, _)| *m).collect() });
        for (_, link) in others {
            s.tell(link, Heard::Arrived { room: room.clone(), member });
        }
    }

    fn leave(&self, room: &RoomName) {
        self.hub.lock().unwrap().leave(room, self.link);
    }

    fn send(&self, room: &RoomName, to: To, payload: Vec<u8>) {
        let Some(mut s) = self.open(room, Asked::Send) else { return };
        if payload.len() > MESSAGE_BYTES {
            return s.refuse(self.link, room, Asked::Send, Refusal::Limit);
        }
        let Some(r) = s.rooms.get(room) else { return };
        let Some(from) = r.members.iter().find(|(_, l)| *l == self.link).map(|(m, _)| *m) else { return };
        let targets: Vec<usize> = r.members.iter().filter(|(m, l)| *l != self.link && (to == To::Everyone || to == To::Member(*m))).map(|(_, l)| *l).collect();
        for link in targets {
            s.tell(link, Heard::Message { room: room.clone(), from, payload: payload.clone() });
        }
    }

    fn ask_time(&self, room: &RoomName) {
        let Some(mut s) = self.open(room, Asked::Time) else { return };
        let ms = s.now_ms();
        s.tell(self.link, Heard::Time { room: room.clone(), ms, rtt_us: 0 });
    }

    fn put_copy(&self, room: &RoomName, secret: [u8; 32], blob: Vec<u8>) {
        let Some(mut s) = self.open(room, Asked::PutCopy) else { return };
        if s.rooms.get(room).is_some_and(|r| r.kind != RoomKind::Group) || blob.len() > MESSAGE_BYTES {
            return s.refuse(self.link, room, Asked::PutCopy, Refusal::Limit);
        }
        if s.copies.get(room).is_some_and(|(set, _)| *set != secret) {
            return s.refuse(self.link, room, Asked::PutCopy, Refusal::WrongSecret);
        }
        s.copies.insert(room.clone(), (secret, blob));
    }

    fn get_copy(&self, room: &RoomName) {
        let Some(mut s) = self.open(room, Asked::GetCopy) else { return };
        let blob = s.copies.get(room).map(|(_, blob)| blob.clone());
        s.tell(self.link, Heard::Copy { room: room.clone(), blob });
    }

    fn delete_copy(&self, room: &RoomName, secret: [u8; 32]) {
        let Some(mut s) = self.open(room, Asked::DeleteCopy) else { return };
        match s.copies.get(room) {
            Some((set, _)) if *set != secret => s.refuse(self.link, room, Asked::DeleteCopy, Refusal::WrongSecret),
            _ => {
                s.copies.remove(room);
            }
        }
    }
}

/// One socket of the stand-in server: a hub link in one room.
struct Socket {
    link: HubLink,
    room: RoomName,
    /// The time requests waiting for their answer, as sent.
    times: VecDeque<i64>,
}

/// Where a device's deliveries run by themselves, and to whose sync.
struct Runs {
    tasks: Arc<dyn Tasks>,
    to: Mutex<Weak<Syncing>>,
}

/// A device's WebSockets to the [`Hub`], speaking the protocol. Made with an executor, what the server
/// says reaches the device's sync by itself, as tasks there; made without, the test hands it on with
/// [`HubSockets::deliver`].
pub struct HubSockets {
    hub: Hub,
    sockets: Mutex<HashMap<SocketId, Socket>>,
    /// Sockets that ended on the server's side or the network's, to be told.
    ended: Mutex<Vec<(SocketId, Ended)>>,
    /// Sockets opened so far.
    pub opened: AtomicUsize,
    /// Keep-alives sent so far.
    pub pings: AtomicUsize,
    /// Binary frames sent so far: what a server on Cloudflare bills.
    pub sent: AtomicUsize,
    runs: Option<Runs>,
    /// A delivery is on its way.
    pending: AtomicBool,
    me: Weak<HubSockets>,
}

impl HubSockets {
    /// Sockets to `hub`; with `runs`, delivering by themselves on it once [`HubSockets::tell`] names
    /// their sync.
    pub fn new(hub: &Hub, runs: Option<Arc<dyn Tasks>>) -> Arc<HubSockets> {
        let net = Arc::new_cyclic(|me| HubSockets { hub: hub.clone(), sockets: Mutex::default(), ended: Mutex::default(), opened: AtomicUsize::new(0), pings: AtomicUsize::new(0), sent: AtomicUsize::new(0), runs: runs.map(|tasks| Runs { tasks, to: Mutex::new(Weak::new()) }), pending: AtomicBool::new(false), me: me.clone() });
        if net.runs.is_some() {
            hub.0.lock().unwrap().running.push(Arc::downgrade(&net));
        }
        net
    }

    /// The sync these sockets deliver to by themselves.
    pub fn tell(&self, syncing: &Arc<Syncing>) {
        if let Some(runs) = &self.runs {
            *runs.to.lock().unwrap() = Arc::downgrade(syncing);
        }
    }

    /// A delivery runs soon, unless one is on its way.
    fn due(&self) {
        let Some(Runs { tasks, .. }) = &self.runs else { return };
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.me.clone();
        tasks.spawn(Box::pin(async move {
            let Some(net) = me.upgrade() else { return };
            net.pending.store(false, Ordering::Release);
            let to = net.runs.as_ref().and_then(|r| r.to.lock().unwrap().upgrade());
            if let Some(syncing) = to {
                net.deliver(&syncing);
            }
        }));
    }

    pub fn open_sockets(&self) -> usize {
        self.sockets.lock().unwrap().len()
    }

    /// The network drops every socket.
    pub fn drop_all(&self) {
        let gone: Vec<(SocketId, Socket)> = self.sockets.lock().unwrap().drain().collect();
        for (id, s) in gone {
            s.link.drop_connection();
            self.ended.lock().unwrap().push((id, Ended::Dropped));
        }
        self.hub.stir();
    }

    /// Hands `syncing` what its sockets heard; true when anything.
    pub fn deliver(&self, syncing: &Syncing) -> bool {
        let mut out: Vec<(SocketId, Result<Vec<u8>, Ended>)> = self.ended.lock().unwrap().drain(..).map(|(id, e)| (id, Err(e))).collect();
        let mut sockets = self.sockets.lock().unwrap();
        let mut closed = Vec::new();
        for (&id, s) in sockets.iter_mut() {
            for h in s.link.heard() {
                let said = match h {
                    Heard::Joined { me, members, .. } => {
                        let w = Welcome { you: me.0 as u16, max_frame: 65_536, messages_per_minute: 240, bytes_per_minute: 4 << 20, max_members: 20, closes_at: None, members: members.iter().map(|m| m.0 as u16).collect() };
                        Ok(Down::Welcome(w))
                    }
                    Heard::Arrived { member, .. } => Ok(Down::Joined(member.0 as u16)),
                    Heard::Left { member, .. } => Ok(Down::Left(member.0 as u16)),
                    Heard::Message { from, payload, .. } => Ok(Down::Message { from: from.0 as u16, alone: false, payload }),
                    Heard::Time { ms, .. } => Ok(Down::Time { t1: s.times.pop_front().expect("asked"), t2: ms * 1000, t3: ms * 1000 }),
                    Heard::Copy { blob, .. } => Ok(Down::Kept(blob)),
                    Heard::Closed { .. } => Err(Ended::Closed { code: Code::RoomClosed.number() }),
                    Heard::Refused { asked: crate::room::Asked::Join, reason, .. } => Err(Ended::Closed { code: refused(reason).number() }),
                    Heard::Refused { reason, .. } => Ok(Down::Refused { asked: crate::wire::Asked::KeptPut, code: refused(reason), retry_after_ms: 0 }),
                };
                if said.is_err() {
                    closed.push(id);
                }
                out.push((id, said.map(|d| d.encode())));
            }
        }
        for id in closed {
            sockets.remove(&id);
        }
        drop(sockets);
        let any = !out.is_empty();
        for (id, said) in out {
            match said {
                Ok(frame) => syncing.frame(id, frame),
                Err(how) => syncing.ended(id, how),
            }
        }
        any
    }
}

fn refused(reason: Refusal) -> Code {
    match reason {
        Refusal::DailyWall => Code::DailyLimit,
        Refusal::Limit => Code::RoomFull,
        Refusal::Version => Code::VersionUnsupported,
        Refusal::WrongSecret => Code::WrongSecret,
        Refusal::Unavailable => Code::Unavailable,
        Refusal::Invalid => Code::BadRequest,
    }
}

impl Sockets for HubSockets {
    fn open(&self, id: SocketId, url: String) {
        self.opened.fetch_add(1, Ordering::Relaxed);
        let (path, query) = url.split_once("/room/").expect("a room's address").1.split_once('?').expect("a query");
        let kind = if query.contains("kind=meeting") { RoomKind::Meeting } else { RoomKind::Group };
        let room = RoomName::parse(path).expect("a room's name");
        let link = self.hub.link();
        link.join(&room, kind);
        self.sockets.lock().unwrap().insert(id, Socket { link, room, times: VecDeque::new() });
        self.hub.stir();
    }

    fn send(&self, id: SocketId, frame: Vec<u8>) {
        self.sent.fetch_add(1, Ordering::Relaxed);
        let mut sockets = self.sockets.lock().unwrap();
        let Some(s) = sockets.get_mut(&id) else { return };
        match read_up(&frame) {
            Up::SendAll(p) => s.link.send(&s.room, To::Everyone, p.to_vec()),
            Up::SendTo(m, p) => s.link.send(&s.room, To::Member(Member(m.into())), p.to_vec()),
            Up::Time(t1) => {
                s.times.push_back(t1);
                s.link.ask_time(&s.room);
            }
            Up::KeptGet => s.link.get_copy(&s.room),
            Up::KeptPut(secret, blob) => s.link.put_copy(&s.room, *secret, blob.to_vec()),
            Up::KeptDelete(secret) => s.link.delete_copy(&s.room, *secret),
        }
        drop(sockets);
        self.hub.stir();
    }

    fn ping(&self, _: SocketId) {
        self.pings.fetch_add(1, Ordering::Relaxed);
    }

    fn close(&self, id: SocketId) {
        let gone = self.sockets.lock().unwrap().remove(&id);
        if let Some(s) = gone {
            s.link.leave(&s.room);
        }
        self.hub.stir();
    }
}

/// What sync tells a test's device: nothing it acts on. It plays servers' and plugins' profiles.
struct Untold;

impl crate::SyncClient for Untold {
    fn here(&self) -> crate::Here {
        crate::Here { plays: crate::Plays::ServersAndPlugins, copied: vec!["subsonic".into()], plugins: Vec::new() }
    }
    fn profiles(&self, _: crate::ProfilesSynced) {}
    fn event(&self, _: crate::SyncEvent) {}
    fn changed(&self) {}
}

/// Pairs the devices whose settings `devices` keep (sync on) into one group through `hub`, the first
/// making it, each a computer whose sync runs on `sim`; once every device holds every profile. A device
/// that brought none takes the others' as its own.
pub fn grouped(hub: &Hub, sim: &Arc<nori_tasks::Native>, devices: &[&Arc<nori_settings::settings_store::Settings>]) -> Vec<Arc<Syncing>> {
    let syncs: Vec<Arc<Syncing>> = devices.iter().map(|settings| {
        let net = HubSockets::new(hub, Some(sim.clone()));
        let runs = sim.clone();
        let syncing = Syncing::new((*settings).clone(), net.clone(), Arc::new(Untold), crate::Reach::Computer, Box::new(move || runs.clone() as Arc<dyn Tasks>));
        net.tell(&syncing);
        syncing
    }).collect();
    for new in &syncs[1..] {
        new.pair_show("New".into());
        let link = sim.until("a code", || match new.pairing() { crate::PairingView::Code { link, .. } => Some(link), _ => None });
        assert_eq!(syncs[0].pair_take(link, crate::Came::Camera), None);
        sim.until("what to confirm", || matches!(syncs[0].pairing(), crate::PairingView::Confirm { .. }).then_some(()));
        syncs[0].pair_confirm("First".into(), false);
        sim.until("paired", || (new.pairing() == crate::PairingView::Paired).then_some(()));
        new.pair_cancel();
        syncs[0].pair_cancel();
    }
    let all = |s: &Arc<nori_settings::settings_store::Settings>| s.with_prefs(|p| p.servers.iter().map(|s| s.id.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let count = devices.iter().map(|s| all(s).len()).max().unwrap_or(0);
    sim.until("every profile on every device", || devices.iter().zip(&syncs).all(|(s, sync)| all(s).len() == count && all(s).into_iter().all(|id| sync.synced(id))).then_some(()));
    syncs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_hear_each_other_and_a_meeting_room_holds_two_for_five_minutes() {
        let hub = Hub::new(0);
        let (a, b, c) = (hub.link(), hub.link(), hub.link());
        let room = RoomName::random();
        a.join(&room, RoomKind::Meeting);
        b.join(&room, RoomKind::Meeting);
        c.join(&room, RoomKind::Meeting);
        assert_eq!(a.heard(), [Heard::Joined { room: room.clone(), me: Member(0), members: vec![] }, Heard::Arrived { room: room.clone(), member: Member(1) }]);
        assert_eq!(b.heard(), [Heard::Joined { room: room.clone(), me: Member(1), members: vec![Member(0)] }]);
        assert_eq!(c.heard(), [Heard::Refused { room: room.clone(), asked: Asked::Join, reason: Refusal::Limit }]);
        b.send(&room, To::Everyone, b"hi".to_vec());
        assert_eq!(a.heard(), [Heard::Message { room: room.clone(), from: Member(1), payload: b"hi".to_vec() }]);
        hub.advance(MEETING_MS);
        assert_eq!(a.heard(), [Heard::Closed { room: room.clone() }]);
        assert_eq!(hub.members(&room), 0);
        c.join(&room, RoomKind::Meeting);
        assert_eq!(c.heard(), [Heard::Closed { room: room.clone() }], "a closed meeting room refuses for an hour");
        hub.advance(MEETING_CLOSED_MS);
        c.join(&room, RoomKind::Meeting);
        assert!(matches!(c.heard()[..], [Heard::Joined { .. }]), "then its name is free again");
    }

    #[test]
    fn the_first_put_sets_the_copy_secret_and_the_wall_refuses_everything() {
        let hub = Hub::new(7);
        let a = hub.link();
        let room = RoomName::random();
        a.join(&room, RoomKind::Group);
        a.put_copy(&room, [1; 32], b"one".to_vec());
        a.put_copy(&room, [2; 32], b"two".to_vec());
        a.delete_copy(&room, [2; 32]);
        a.get_copy(&room);
        let heard = a.heard();
        assert_eq!(heard[1..], [
            Heard::Refused { room: room.clone(), asked: Asked::PutCopy, reason: Refusal::WrongSecret },
            Heard::Refused { room: room.clone(), asked: Asked::DeleteCopy, reason: Refusal::WrongSecret },
            Heard::Copy { room: room.clone(), blob: Some(b"one".to_vec()) },
        ]);
        hub.wall(true);
        a.ask_time(&room);
        assert_eq!(a.heard(), [Heard::Refused { room: room.clone(), asked: Asked::Time, reason: Refusal::DailyWall }]);
        hub.wall(false);
        a.ask_time(&room);
        a.delete_copy(&room, [1; 32]);
        a.get_copy(&room);
        assert_eq!(a.heard(), [Heard::Time { room: room.clone(), ms: 7, rtt_us: 0 }, Heard::Copy { room, blob: None }]);
    }
}
