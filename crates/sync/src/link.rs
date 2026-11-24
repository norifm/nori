//! A [`Relay`] to a real sync server: one WebSocket per room, which the platform opens, reads and writes
//! ([`Sockets`]). The link turns the frames into [`Heard`], waits for a room's welcome before it sends
//! there, comes back quietly after a drop, after a random short wait that grows while connecting fails,
//! and holds a refusal as a state with the time to try again. Nothing here runs by itself: the owner
//! hands in what the sockets say and calls [`Link::tick`] at [`Link::due_at`], and sends keep-alives
//! when [`Link::ping_at`] says so.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::keys::{random, RoomName};
use crate::room::{self, Heard, Member, Refusal, Relay, RoomKind, To};
use crate::wire::{room_url, Asked, Code, Down, Up};

/// A socket the link asked the platform to open; a room gets a new one each time it connects.
pub type SocketId = u64;

/// The platform's WebSockets. Every call returns at once and never calls back into sync before it
/// returns; the platform answers through [`crate::Syncing::frame`] and [`crate::Syncing::ended`], in the
/// order each socket's events happened.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait Sockets: Send + Sync {
    /// Opens a WebSocket to `url` as `id`.
    fn open(&self, id: SocketId, url: String);
    /// Sends `frame` as a binary frame.
    fn send(&self, id: SocketId, frame: Vec<u8>);
    /// A keep-alive: a WebSocket ping, or the text frame `ping` where only text can be sent (a browser).
    fn ping(&self, id: SocketId);
    /// Closes the socket normally; nothing more is told of it.
    fn close(&self, id: SocketId);
}

/// How a socket ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Ended {
    /// The server closed it with this close code.
    Closed { code: u16 },
    /// The network dropped it or it never opened: no close frame came.
    Dropped,
    /// The server answered the handshake with this HTTP status instead: it cannot run now.
    Status { status: u16 },
}

/// Where a room's connection stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomState {
    /// Opening, before the welcome.
    Opening,
    Open,
    /// Waiting until `until` (the link's clock, µs) to connect again; `refused` says why when the server
    /// refused rather than the network dropping.
    Waiting { until: i64, refused: Option<Refusal> },
    /// Refused for good: not tried again until the room is joined again.
    Stopped(Refusal),
}

/// A keep-alive goes out after this long without traffic on a connection.
pub(crate) const PING_AFTER_US: i64 = 50 * 1_000_000;
/// The first wait before connecting again is random between these.
const RETRY_US: (i64, i64) = (1_000_000, 5_000_000);
/// The longest wait while connecting keeps failing.
const RETRY_MAX_US: i64 = 5 * 60 * 1_000_000;
/// The wait after too many rooms were opened from this network address.
const ROOMS_WAIT_US: i64 = 60 * 1_000_000;
/// How much past midnight UTC a device tries again after the daily wall, at most: spread out, so the
/// devices do not all come at once.
const WALL_SPREAD_US: i64 = 30 * 60 * 1_000_000;
const DAY_US: i64 = 24 * 60 * 60 * 1_000_000;

struct Room {
    kind: RoomKind,
    socket: SocketId,
    state: RoomState,
    /// Frames asked for while the room is not open, sent after its welcome.
    queued: Vec<Queued>,
    /// Connections in a row that failed, which the wait before the next grows with.
    failures: u32,
    members: BTreeSet<u16>,
    /// The last traffic either way, on the link's clock.
    active_us: i64,
}

struct State {
    rooms: BTreeMap<RoomName, Room>,
    /// Where socket ids come from, unique among the links that share it.
    ids: Arc<AtomicU64>,
    heard: Vec<Heard>,
}

/// A frame waiting for a room's welcome.
enum Queued {
    Frame(Vec<u8>),
    /// A time request, stamped as it leaves: the server's clock is told from the way there and back.
    Time,
}

/// What a call asks of the sockets, done once the state is let go, so a platform answering at once
/// does not find it held.
enum Do {
    Open(SocketId, String),
    Send(SocketId, Vec<u8>),
    Ping(SocketId),
    Close(SocketId),
}

/// A random wait between `low` and `high`.
fn between(low: i64, high: i64) -> i64 {
    low + (u32::from_be_bytes(random()) as i64 % (high - low).max(1))
}

/// One sync server, as a [`Relay`]: its rooms, each on a socket of its own.
pub(crate) struct Link {
    server: String,
    sockets: Arc<dyn Sockets>,
    /// The link's clock (µs, monotonic) less the Unix time: the server's clock is told in Unix time.
    unix_offset_us: i64,
    /// The link's clock now, as its owner last said.
    now: Mutex<i64>,
    state: Mutex<State>,
}

impl Link {
    /// A link to `server` whose sockets take their ids from `ids`; `now_us` is the owner's clock and
    /// `unix_us` the Unix time at that moment.
    pub fn new(server: String, sockets: Arc<dyn Sockets>, ids: Arc<AtomicU64>, now_us: i64, unix_us: i64) -> Link {
        let state = State { rooms: BTreeMap::new(), ids, heard: Vec::new() };
        Link { server, sockets, unix_offset_us: unix_us - now_us, now: Mutex::new(now_us), state: Mutex::new(state) }
    }

    /// The clock moved on to `now_us`: what the link does next is timed from there.
    pub fn at(&self, now_us: i64) {
        *self.now.lock().expect("not poisoned") = now_us;
    }

    fn now(&self) -> i64 {
        *self.now.lock().expect("not poisoned")
    }

    fn run(&self, f: impl FnOnce(&mut State, i64, &mut Vec<Do>)) {
        let now = self.now();
        let mut todo = Vec::new();
        f(&mut self.state.lock().expect("not poisoned"), now, &mut todo);
        for d in todo {
            match d {
                Do::Open(id, url) => self.sockets.open(id, url),
                Do::Send(id, frame) => self.sockets.send(id, frame),
                Do::Ping(id) => self.sockets.ping(id),
                Do::Close(id) => self.sockets.close(id),
            }
        }
    }

    /// What the server said since the owner last asked, in order.
    pub fn heard(&self) -> Vec<Heard> {
        std::mem::take(&mut self.state.lock().expect("not poisoned").heard)
    }

    /// Where `room`'s connection stands; None when it was not joined.
    pub fn room(&self, room: &RoomName) -> Option<RoomState> {
        self.state.lock().expect("not poisoned").rooms.get(room).map(|r| r.state)
    }

    /// When a room waiting to connect again is due; [`Link::tick`] then.
    pub fn due_at(&self) -> Option<i64> {
        let s = self.state.lock().expect("not poisoned");
        s.rooms.values().filter_map(|r| if let RoomState::Waiting { until, .. } = r.state { Some(until) } else { None }).min()
    }

    /// Connects again the rooms whose wait is over.
    pub fn tick(&self) {
        let server = self.server.clone();
        self.run(|s, now, todo| {
            let due: Vec<RoomName> = s.rooms.iter().filter(|(_, r)| matches!(r.state, RoomState::Waiting { until, .. } if until <= now)).map(|(n, _)| n.clone()).collect();
            for name in due {
                s.connect(&server, &name, now, todo);
            }
        });
    }

    /// When an open connection has been quiet long enough for a keep-alive.
    pub fn ping_at(&self) -> Option<i64> {
        let s = self.state.lock().expect("not poisoned");
        s.rooms.values().filter(|r| r.state == RoomState::Open).map(|r| r.active_us + PING_AFTER_US).min()
    }

    /// Sends a keep-alive on every open connection quiet since [`PING_AFTER_US`].
    pub fn keep_alive(&self) {
        self.run(|s, now, todo| {
            for r in s.rooms.values_mut().filter(|r| r.state == RoomState::Open && now - r.active_us >= PING_AFTER_US) {
                r.active_us = now;
                todo.push(Do::Ping(r.socket));
            }
        });
    }

    /// Leaves every room, as on [`Relay::leave`]; their members are told as having left.
    pub fn leave_all(&self) {
        self.run(|s, _, todo| {
            for (name, r) in std::mem::take(&mut s.rooms) {
                todo.push(Do::Close(r.socket));
                s.lost(&name, r.members, None);
            }
        });
    }

    /// A frame from socket `id`.
    pub fn frame(&self, id: SocketId, frame: &[u8]) {
        let unix_offset = self.unix_offset_us;
        self.run(|s, now, todo| {
            let Some((name, r)) = s.rooms.iter_mut().find(|(_, r)| r.socket == id) else { return };
            let name = name.clone();
            r.active_us = now;
            let down = match Down::decode(frame) {
                Ok(down) => down,
                Err(e) => {
                    nori_model::alog::info(&format!("sync: the server sent {}, a bug on one side", e.0));
                    todo.push(Do::Close(id));
                    r.state = RoomState::Stopped(Refusal::Invalid);
                    let members = std::mem::take(&mut r.members);
                    return s.lost(&name, members, Some((room::Asked::Join, Refusal::Invalid)));
                }
            };
            let heard = match down {
                Down::Welcome(w) => {
                    r.state = RoomState::Open;
                    r.failures = 0;
                    r.members = w.members.iter().copied().collect();
                    todo.extend(r.queued.drain(..).map(|q| match q {
                        Queued::Frame(f) => Do::Send(id, f),
                        Queued::Time => Do::Send(id, Up::Time(now + unix_offset).encode()),
                    }));
                    Heard::Joined { room: name, me: Member(w.you.into()), members: w.members.into_iter().map(|m| Member(m.into())).collect() }
                }
                Down::Joined(m) => {
                    r.members.insert(m);
                    Heard::Arrived { room: name, member: Member(m.into()) }
                }
                Down::Left(m) => {
                    r.members.remove(&m);
                    Heard::Left { room: name, member: Member(m.into()) }
                }
                Down::Message { from, payload, .. } => Heard::Message { room: name, from: Member(from.into()), payload },
                Down::Time { t1, t2, t3 } => {
                    // As NTP: the server's clock less this one's, from the request's way out and back.
                    let t4 = now + unix_offset;
                    let offset = ((t2 - t1) + (t3 - t4)) / 2;
                    Heard::Time { room: name, ms: (t4 + offset) / 1000 }
                }
                Down::Kept(blob) => Heard::Copy { room: name, blob },
                Down::Done(_) => return,
                Down::Refused { asked, code, .. } => {
                    let Some(reason) = code.refusal() else { return };
                    Heard::Refused { room: name, asked: asked.into(), reason }
                }
            };
            s.heard.push(heard);
        });
    }

    /// Sends `frame` to `room`, or keeps it for the room's welcome.
    fn queue(&self, room: &RoomName, frame: Queued) {
        let unix_offset = self.unix_offset_us;
        self.run(|s, now, todo| s.send(room, frame, now, now + unix_offset, todo));
    }

    /// Socket `id` ended.
    pub fn ended(&self, id: SocketId, how: Ended) {
        let unix_offset = self.unix_offset_us;
        self.run(|s, now, _| {
            let Some((name, r)) = s.rooms.iter_mut().find(|(_, r)| r.socket == id) else { return };
            let name = name.clone();
            let members = std::mem::take(&mut r.members);
            let code = match how {
                Ended::Closed { code } => Code::of(code),
                Ended::Dropped => None,
                Ended::Status { .. } => Some(Code::Unavailable),
            };
            if code == Some(Code::RoomClosed) {
                s.rooms.remove(&name);
                s.lost(&name, members, None);
                return s.heard.push(Heard::Closed { room: name });
            }
            let refused = code.and_then(Code::refusal);
            let wait = match (code, refused) {
                (_, Some(stop @ (Refusal::Version | Refusal::Invalid))) => {
                    r.state = RoomState::Stopped(stop);
                    return s.lost(&name, members, Some((room::Asked::Join, stop)));
                }
                (Some(Code::TooManyRooms), _) => ROOMS_WAIT_US + between(0, RETRY_US.1),
                (_, Some(Refusal::DailyWall)) => {
                    let unix = now + unix_offset;
                    (unix.div_euclid(DAY_US) + 1) * DAY_US - unix + between(0, WALL_SPREAD_US)
                }
                _ => (between(RETRY_US.0, RETRY_US.1) << r.failures.min(8)).min(RETRY_MAX_US),
            };
            r.failures += 1;
            r.state = RoomState::Waiting { until: now + wait, refused };
            s.lost(&name, members, refused.map(|reason| (room::Asked::Join, reason)));
        });
    }
}

impl State {
    /// Opens a socket for `name`, which was joined.
    fn connect(&mut self, server: &str, name: &RoomName, now: i64, todo: &mut Vec<Do>) {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let r = self.rooms.get_mut(name).expect("joined");
        r.socket = id;
        match room_url(server, name, r.kind) {
            Some(url) => {
                r.state = RoomState::Opening;
                r.active_us = now;
                todo.push(Do::Open(id, url));
            }
            None => {
                r.state = RoomState::Stopped(Refusal::Invalid);
                self.heard.push(Heard::Refused { room: name.clone(), asked: room::Asked::Join, reason: Refusal::Invalid });
            }
        }
    }

    /// `name`'s connection is gone: its members left as this device sees it, and the refusal if any.
    fn lost(&mut self, name: &RoomName, members: BTreeSet<u16>, refused: Option<(room::Asked, Refusal)>) {
        self.heard.extend(members.into_iter().map(|m| Heard::Left { room: name.clone(), member: Member(m.into()) }));
        if let Some((asked, reason)) = refused {
            self.heard.push(Heard::Refused { room: name.clone(), asked, reason });
        }
    }

    /// `frame` for `name`: sent while it is open, kept until its welcome otherwise. `unix_us` stamps a
    /// time request.
    fn send(&mut self, name: &RoomName, frame: Queued, now: i64, unix_us: i64, todo: &mut Vec<Do>) {
        let Some(r) = self.rooms.get_mut(name) else { return };
        if r.state != RoomState::Open {
            return r.queued.push(frame);
        }
        r.active_us = now;
        todo.push(Do::Send(r.socket, match frame {
            Queued::Frame(f) => f,
            Queued::Time => Up::Time(unix_us).encode(),
        }));
    }
}

impl From<Asked> for room::Asked {
    fn from(asked: Asked) -> room::Asked {
        match asked {
            Asked::SendAll | Asked::SendTo => room::Asked::Send,
            Asked::Time => room::Asked::Time,
            Asked::KeptGet => room::Asked::GetCopy,
            Asked::KeptPut => room::Asked::PutCopy,
            Asked::KeptDelete => room::Asked::DeleteCopy,
        }
    }
}

impl Relay for Link {
    fn join(&self, room: &RoomName, kind: RoomKind) {
        let server = self.server.clone();
        self.run(|s, now, todo| {
            if let Some(old) = s.rooms.insert(room.clone(), Room { kind, socket: 0, state: RoomState::Opening, queued: Vec::new(), failures: 0, members: BTreeSet::new(), active_us: now }) {
                todo.push(Do::Close(old.socket));
            }
            s.connect(&server, room, now, todo);
        });
    }

    fn leave(&self, room: &RoomName) {
        self.run(|s, _, todo| {
            if let Some(r) = s.rooms.remove(room) {
                todo.push(Do::Close(r.socket));
            }
        });
    }

    fn send(&self, room: &RoomName, to: To, payload: Vec<u8>) {
        let frame = match to {
            To::Everyone => Up::SendAll(&payload).encode(),
            To::Member(Member(m)) => Up::SendTo(m as u16, &payload).encode(),
        };
        self.queue(room, Queued::Frame(frame));
    }

    fn ask_time(&self, room: &RoomName) {
        self.queue(room, Queued::Time);
    }

    fn put_copy(&self, room: &RoomName, secret: [u8; 32], blob: Vec<u8>) {
        self.queue(room, Queued::Frame(Up::KeptPut(&secret, &blob).encode()));
    }

    fn get_copy(&self, room: &RoomName) {
        self.queue(room, Queued::Frame(Up::KeptGet.encode()));
    }

    fn delete_copy(&self, room: &RoomName, secret: [u8; 32]) {
        self.queue(room, Queued::Frame(Up::KeptDelete(&secret).encode()));
    }
}

#[cfg(test)]
mod tests;
