//! Remote control and jams through the core's public calls, against a relay kept in memory that answers
//! as octo-fiesta's hub does (rooms, held polls, jam invites, guests acting with the host's rights): two
//! devices of one account controlling each other, and a jam with a host, an admin and a guest.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::task::{Poll, Waker};
use std::future::Future;
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

use nori_core::client::{Client, NetProfile, Starrable};
use nori_core::library::StarsShown;
use nori_core::remote::{jam_join, Controls, JamJoin, JamStart, Discovery, Follower, JamControls, Lead, Listening, Playing, Reach, RelaySupport, Remote, RemoteMe, RemotePlayer, RemoteShown, Sight};
use nori_core::transport::{Exchange, FailureKind, Transport, TransportError, TransportResponse};
use nori_player::playlist::Hand;
use nori_core::{Core, ServerConfig, Song};
use nori_remote::clock;
use nori_remote::wire::{Answer, Body, DeviceKind, DeviceState, Event, Member, Op, Outgoing, Refusal, Role, Room};
use nori_tasks::{Busy, Native, Tasks};
use parking_lot::Mutex;

const SERVER: &str = "http://octo:5274";

/// Who a request is from, as the relay's checks find it.
#[derive(Clone)]
enum Caller {
    Account(String),
    /// A jam member, by room and member id.
    Guest(String, String),
    /// Someone holding an invite, not yet a member.
    Invited(String),
}

#[derive(Default)]
struct RoomState {
    jam: bool,
    /// The host's user and device, for a jam.
    host: Option<(String, String)>,
    members: Vec<Member>,
    /// (seq, from, to, body).
    events: Vec<(u64, String, Option<String>, Body)>,
    /// The seq of its last change: a held poll wakes only for a room it listens to.
    touched: u64,
}

/// Marks room `room` changed at `seq`.
fn room_touched(hub: &mut Hub, room: &str, seq: u64) {
    if let Some(r) = hub.rooms.get_mut(room) {
        r.touched = seq;
    }
}

/// The rooms a poll by `caller` from device `dev` listens to: its account's and the jams it hosts, or a
/// guest's jam.
fn listened(hub: &Hub, caller: &Caller, dev: &str) -> Vec<String> {
    match caller {
        Caller::Account(user) => {
            let hosted = hub.rooms.iter().filter(|(_, r)| r.host.as_ref().is_some_and(|(u, d)| u == user && d == dev)).map(|(id, _)| id.clone());
            std::iter::once(format!("u:{user}")).chain(hosted).collect()
        }
        Caller::Guest(room, _) => vec![room.clone()],
        Caller::Invited(_) => Vec::new(),
    }
}

#[derive(Default)]
struct Hub {
    seq: u64,
    rooms: HashMap<String, RoomState>,
    /// Invite keys and member keys: (room, member id; None for an invite).
    keys: HashMap<String, (String, Option<String>)>,
    /// Every endpoint asked, with its id parameter.
    asked: Vec<String>,
    closed: bool,
    /// The devices whose held polls wait for news, once for each.
    parked: Vec<String>,
}

/// Device `dev`'s held poll, while it waits for news.
struct Parked<'a>(&'a Relay, String);

impl Drop for Parked<'_> {
    fn drop(&mut self) {
        let mut hub = self.0.hub.lock();
        let at = hub.parked.iter().position(|d| *d == self.1).unwrap();
        hub.parked.swap_remove(at);
    }
}

/// When a test starts on its simulated clock, µs.
const START_US: i64 = 1_000_000_000_000;

/// The relay in front of a server whose library is a few songs.
struct Relay {
    /// Itself, for what reaches it after the device gave up on the request.
    me: Weak<Relay>,
    /// Idle connections to each door by port, kept alive as a platform's HTTP client keeps them (a
    /// connection each would leave the machine's ports waiting to close).
    lan: Mutex<HashMap<u16, Vec<BufReader<std::net::TcpStream>>>>,
    /// Where the relay's and every device's remote work runs, on a clock the test moves.
    sim: Arc<Native>,
    devices: Mutex<Vec<Weak<Remote>>>,
    hub: Mutex<Hub>,
    /// Held polls waiting for news; dropping one gives it up, as a cancelled request.
    waiting: Mutex<Vec<Waker>>,
    next: AtomicU64,
    /// A plain Navidrome: no `noriRemote.*` at all.
    absent: std::sync::atomic::AtomicBool,
    /// A device whose clock reads this much ahead (µs), and whose sends reach the relay late: see
    /// [`Relay::lagging`].
    lagging: Mutex<Option<(String, i64, bool)>>,
    /// Sends of the lagging device so far.
    lagged: AtomicU64,
    /// A door's poll answers come back 250 ms late.
    door_late: std::sync::atomic::AtomicBool,
    /// A device's poll saying it no longer serves reaches the relay 250 ms late.
    leave_late: std::sync::atomic::AtomicBool,
    /// The server cannot be reached; held polls fail once woken.
    down: std::sync::atomic::AtomicBool,
    /// Lets jam guests stream the host's queue (listening along).
    along: std::sync::atomic::AtomicBool,
    /// Tells its time at `nori/time`; its clock reads [`SERVER_SKEW_US`] ahead.
    time: std::sync::atomic::AtomicBool,
    /// The next relay clock answer arrives this late.
    first_time_delay_ms: AtomicU64,
    /// The lagging device's answers to time exchanges reach the relay this late, ms (0: as its other sends).
    answers_late: AtomicU64,
    /// Closing a jam goes unanswered: the request is lost on the way.
    lose_closes: std::sync::atomic::AtomicBool,
    /// Opening a jam takes a while, each one asked a little less than the one before, so their answers
    /// come back in the reverse order.
    opens_late: std::sync::atomic::AtomicBool,
    opens: AtomicU64,
    /// Polls with no `since` are not answered: no device has its first answer.
    first_answers_held: std::sync::atomic::AtomicBool,
    /// The next poll of a device saying it serves stops on its way, sent: see [`Relay::late_poll`].
    late_poll: Mutex<Option<LatePoll>>,
    /// A device that hears nothing the relay holds for it, as one gone without leaving.
    deaf: Mutex<Option<String>>,
    /// Requests to doors are refused, as by a device gone from the network; those under way still answer.
    doors_refuse: std::sync::atomic::AtomicBool,
}

struct LatePoll {
    dev: String,
    caught: Sender<()>,
    /// The test's word to let the poll arrive, holding the clock still until it has.
    go: Receiver<Busy>,
    arrived: Sender<()>,
}

const SERVER_SKEW_US: i64 = 9_000_000;

fn decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&v[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn json(v: impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}

fn subsonic_error(message: &str) -> Vec<u8> {
    format!(r#"{{"subsonic-response":{{"status":"failed","error":{{"code":40,"message":"{message}"}}}}}}"#).into_bytes()
}

/// What a jam guest's key may ask: reading the library through the host's account, and the jam.
const GUEST_ENDPOINTS: &[&str] = &["ping", "search3", "getCoverArt", "getSong", "getAlbum", "getArtist", "noriRemote.poll", "noriRemote.send", "noriRemote.leave"];

impl Relay {
    fn new() -> Arc<Relay> {
        Relay::made(false)
    }

    fn absent() -> Arc<Relay> {
        Relay::made(true)
    }

    fn made(absent: bool) -> Arc<Relay> {
        Arc::new_cyclic(|me| Relay {
            me: me.clone(),
            lan: Mutex::default(),
            sim: Arc::new(Native::simulated("nori-remote-test", START_US).unwrap()),
            devices: Mutex::default(),
            hub: Mutex::default(), waiting: Mutex::default(), next: AtomicU64::new(1), absent: absent.into(), lagging: Mutex::new(None), lagged: AtomicU64::new(0), door_late: Default::default(), leave_late: Default::default(), down: Default::default(), along: std::sync::atomic::AtomicBool::new(true), time: Default::default(), first_time_delay_ms: Default::default(), answers_late: AtomicU64::new(0), lose_closes: Default::default(), opens_late: Default::default(), opens: AtomicU64::new(0), first_answers_held: Default::default(), late_poll: Mutex::new(None), deaf: Mutex::new(None), doors_refuse: Default::default() })
    }

    /// Device `dev`'s times read `skew_us` ahead as the relay passes them on. Its states reach the relay
    /// 250 ms late; its other sends every other one at once, the rest 60 to 170 ms late.
    fn lag(&self, dev: &str, skew_us: i64) {
        *self.lagging.lock() = Some((dev.to_string(), skew_us, true));
    }

    /// Device `dev`'s times read `skew_us` ahead as the relay passes them on; its sends go at once.
    fn skew(&self, dev: &str, skew_us: i64) {
        *self.lagging.lock() = Some((dev.to_string(), skew_us, false));
    }

    /// The lagging device's send `body`, as it reaches the relay (late) and with its clock read ahead.
    async fn lagged(&self, dev: &str, body: Option<String>) -> Option<String> {
        let Some((skew, slow)) = self.lagging.lock().as_ref().filter(|(d, ..)| d == dev).map(|(_, s, l)| (*s, *l)) else { return body };
        let mut out: Outgoing = serde_json::from_str(body.as_deref()?).unwrap();
        let k = self.lagged.fetch_add(1, Ordering::Relaxed);
        let answers = self.answers_late.load(Ordering::Relaxed);
        let late = match &out.body {
            _ if out.state.is_some() => 250,
            Some(Body::Clock { .. }) if answers > 0 => answers,
            _ if k % 2 == 1 => 0,
            _ => 60 + k * 53 % 110,
        };
        if slow {
            self.sim.sleep(Duration::from_millis(late)).await;
        }
        if let Some(at) = out.state.as_mut().and_then(|s| s.at_us.as_mut()) {
            *at += skew;
        }
        // The relay's clock less the device's, as its own clock reads.
        if let Some(server) = out.state.as_mut().and_then(|s| s.server_us.as_mut()) {
            *server -= skew;
        }
        if let Some(Body::Clock { t2, t3, .. }) = &mut out.body {
            *t2 += skew;
            *t3 += skew;
        }
        Some(serde_json::to_string(&out).unwrap())
    }

    fn key(&self) -> String {
        format!("k{}", self.next.fetch_add(1, Ordering::Relaxed))
    }

    fn asked(&self) -> Vec<String> {
        self.hub.lock().asked.clone()
    }

    /// Whether device `dev` has a poll waiting for news.
    fn parked(&self, dev: &str) -> bool {
        self.hub.lock().parked.iter().any(|d| d == dev)
    }

    /// How many times `endpoint` was asked.
    fn asked_for(&self, endpoint: &str) -> usize {
        self.hub.lock().asked.iter().filter(|a| a.split(' ').next() == Some(endpoint)).count()
    }

    /// The jams it keeps open.
    fn jams(&self) -> usize {
        self.hub.lock().rooms.values().filter(|r| r.jam).count()
    }

    fn close(&self) {
        self.down.store(true, Ordering::Relaxed);
        {
            let mut hub = self.hub.lock();
            hub.closed = true;
            self.wake();
        }
        let devices = std::mem::take(&mut *self.devices.lock());
        for device in devices.into_iter().filter_map(|d| d.upgrade()) {
            device.stop();
        }
    }

    /// Lists device `id` in ann's room with `state`; None as the relay lists a device that polled and has
    /// said nothing since.
    fn list(&self, id: &str, state: Option<DeviceState>) {
        let mut hub = self.hub.lock();
        let room = hub.rooms.entry("u:ann".into()).or_default();
        match room.members.iter_mut().find(|m| m.id == id) {
            Some(m) => m.state = state,
            None => room.members.push(Member { id: id.into(), name: id.into(), kind: DeviceKind::Phone, state }),
        }
        hub.seq += 1;
        let seq = hub.seq;
        room_touched(&mut hub, "u:ann", seq);
        self.wake();
    }

    /// Starts afresh, as after a restart: every room and key is gone, and held polls answer.
    fn restart(&self) {
        let mut hub = self.hub.lock();
        hub.rooms.clear();
        hub.keys.clear();
        self.wake();
    }

    /// The server cannot be reached (`down`), or can again.
    fn go_down(&self, down: bool) {
        let _hub = self.hub.lock();
        self.down.store(down, Ordering::Relaxed);
        self.wake();
    }

    /// Device `dev`'s next poll saying it serves stops on its way here, sent and past recall: told on
    /// the first channel; it goes on once the second is sent to, and has arrived when the third tells.
    fn late_poll(&self, dev: &str) -> (Receiver<()>, Sender<Busy>, Receiver<()>) {
        let ((caught, on_way), (go, wait), (arrived, here)) = (channel(), channel(), channel());
        *self.late_poll.lock() = Some(LatePoll { dev: dev.to_string(), caught, go: wait, arrived });
        (on_way, go, here)
    }

    /// Answers the polls with no `since` held until now, and those to come.
    fn release_first_answers(&self) {
        let _hub = self.hub.lock();
        self.first_answers_held.store(false, Ordering::Relaxed);
        self.wake();
    }

    /// Until first answers are no longer held, or the relay closed or went down.
    async fn first_answer(&self) {
        std::future::poll_fn(|cx| {
            let hub = self.hub.lock();
            if !self.first_answers_held.load(Ordering::Relaxed) || hub.closed || self.down.load(Ordering::Relaxed) {
                return Poll::Ready(());
            }
            self.waiting.lock().push(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    /// Wakes the held polls; called with the hub locked.
    fn wake(&self) {
        self.waiting.lock().drain(..).for_each(Waker::wake);
    }

    /// An account device's poll says whether it serves: it joins or leaves the account's room as the poll
    /// arrives, before any hold.
    fn arrived(&self, p: &HashMap<String, String>) {
        let (Some(user), Some(dev), false) = (p.get("u"), p.get("dev"), self.absent.load(Ordering::Relaxed)) else { return };
        let mut hub = self.hub.lock();
        let room = hub.rooms.entry(format!("u:{user}")).or_default();
        let listed = room.members.iter().position(|m| m.id == *dev);
        match (listed, p.get("serve").map(String::as_str) == Some("1")) {
            (None, true) => {
                let kind = serde_json::from_value(serde_json::Value::String(p["kind"].clone())).unwrap_or_default();
                room.members.push(Member { id: dev.clone(), name: p["name"].clone(), kind, state: None });
            }
            (Some(at), false) => {
                room.members.remove(at);
            }
            _ => return,
        }
        hub.seq += 1;
        let seq = hub.seq;
        room_touched(&mut hub, &format!("u:{user}"), seq);
        self.wake();
    }

    /// A held poll's wait: until there is news after `since` in one of `rooms`, or the relay closed or
    /// went down.
    async fn news(&self, since: u64, rooms: &[String], dev: &str) {
        let mut parked = None;
        std::future::poll_fn(|cx| {
            let mut hub = self.hub.lock();
            if rooms.iter().any(|r| hub.rooms.get(r).is_none_or(|s| s.touched > since)) || hub.closed || self.down.load(Ordering::Relaxed) {
                return Poll::Ready(());
            }
            if parked.is_none() {
                hub.parked.push(dev.to_string());
                parked = Some(Parked(self, dev.to_string()));
            }
            self.waiting.lock().push(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    fn caller(&self, endpoint: &str, p: &HashMap<String, String>) -> Result<Caller, Vec<u8>> {
        if let Some(user) = p.get("u") {
            return Ok(Caller::Account(user.clone()));
        }
        let key = p.get("apiKey").and_then(|k| k.strip_prefix("nori-jam-")).ok_or_else(|| subsonic_error("no credentials"))?;
        match self.hub.lock().keys.get(key) {
            Some((room, None)) if endpoint == "noriRemote.join" => Ok(Caller::Invited(room.clone())),
            Some((room, Some(member))) if GUEST_ENDPOINTS.contains(&endpoint) => Ok(Caller::Guest(room.clone(), member.clone())),
            _ => Err(subsonic_error("not for a jam guest")),
        }
    }

    fn answer(&self, endpoint: &str, p: &HashMap<String, String>, body: Option<&str>) -> Vec<u8> {
        if self.absent.load(Ordering::Relaxed) && endpoint.starts_with("noriRemote.") {
            return subsonic_error("not here");
        }
        let caller = match self.caller(endpoint, p) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let dev = p.get("dev").cloned().unwrap_or_default();
        let mut hub = self.hub.lock();
        let bump = |hub: &mut Hub, room: &str| {
            hub.seq += 1;
            let seq = hub.seq;
            room_touched(hub, room, seq);
            self.wake();
        };
        match (endpoint, &caller) {
            ("noriRemote.poll", _) => {
                let me = match &caller {
                    Caller::Account(_) => dev.clone(),
                    Caller::Guest(_, member) => member.clone(),
                    Caller::Invited(_) => unreachable!(),
                };
                let rooms = listened(&hub, &caller, &dev);
                let since = p.get("since").and_then(|s| s.parse::<u64>().ok());
                let mine = |hub: &Hub| -> Vec<Event> {
                    rooms
                        .iter()
                        .filter_map(|r| hub.rooms.get(r).map(|state| (r, state)))
                        .flat_map(|(r, state)| state.events.iter().map(move |e| (r, e)))
                        .filter(|(_, (seq, from, to, _))| since.is_some_and(|s| *seq > s) && *from != me && to.as_ref().is_none_or(|t| *t == me))
                        .map(|(r, (seq, from, _, body))| Event { seq: *seq, room: r.clone(), from: from.clone(), body: body.clone() })
                        .collect()
                };
                let answer = Answer {
                    seq: hub.seq,
                    you: me.clone(),
                    rooms: rooms.iter().filter_map(|r| hub.rooms.get(r).map(|s| Room { room: r.clone(), jam: s.jam, members: s.members.clone() })).collect(),
                    events: if self.deaf.lock().as_ref() == Some(&me) { Vec::new() } else { mine(&hub) },
                    along: self.along.load(Ordering::Relaxed),
                    time: self.time.load(Ordering::Relaxed),
                };
                json(answer)
            }
            ("noriRemote.send", _) => {
                let out: Outgoing = serde_json::from_str(body.unwrap_or("{}")).unwrap();
                let (room, from) = match &caller {
                    Caller::Account(user) => (out.room.clone().unwrap_or(format!("u:{user}")), dev.clone()),
                    Caller::Guest(room, member) => (room.clone(), member.clone()),
                    Caller::Invited(_) => unreachable!(),
                };
                let seq = hub.seq + 1;
                let state = hub.rooms.entry(room.clone()).or_default();
                if let Some(s) = out.state {
                    // A device publishing in its account's room serves, polled or not yet.
                    if !state.jam && !state.members.iter().any(|m| m.id == from) {
                        let kind = serde_json::from_value(serde_json::Value::String(p["kind"].clone())).unwrap_or_default();
                        state.members.push(Member { id: from.clone(), name: p["name"].clone(), kind, state: None });
                    }
                    match state.members.iter_mut().find(|m| m.id == from) {
                        Some(m) => m.state = Some(s),
                        None => return subsonic_error("not a member"),
                    }
                }
                if let Some(b) = out.body {
                    state.events.push((seq, from, out.to, b));
                }
                bump(&mut hub, &room);
                json(serde_json::json!({ "seq": seq }))
            }
            ("noriRemote.open", Caller::Account(user)) => {
                let (room, invite) = (format!("j{}", self.key()), self.key());
                let host = Member { id: dev.clone(), name: p["name"].clone(), kind: DeviceKind::Phone, state: None };
                hub.rooms.insert(room.clone(), RoomState { jam: true, host: Some((user.clone(), dev)), members: vec![host], ..Default::default() });
                hub.keys.insert(invite.clone(), (room.clone(), None));
                bump(&mut hub, &room);
                json(serde_json::json!({ "room": room, "invite": invite }))
            }
            ("noriRemote.join", Caller::Invited(room)) => {
                let (member, key) = (format!("m{}", self.key()), self.key());
                hub.keys.insert(key.clone(), (room.clone(), Some(member.clone())));
                hub.rooms.get_mut(room).unwrap().members.push(Member { id: member.clone(), name: p["name"].clone(), kind: DeviceKind::Guest, state: None });
                bump(&mut hub, room);
                json(serde_json::json!({ "room": room, "member": member, "key": key }))
            }
            ("noriRemote.close", Caller::Account(_)) => {
                let room = p["room"].clone();
                hub.rooms.remove(&room);
                hub.keys.retain(|_, (r, _)| *r != room);
                bump(&mut hub, &room);
                json(serde_json::json!({}))
            }
            ("noriRemote.leave", Caller::Guest(room, member)) => {
                let (room, member) = (room.clone(), member.clone());
                hub.keys.retain(|_, (r, m)| !(*r == room && m.as_deref() == Some(member.as_str())));
                hub.rooms.get_mut(&room).unwrap().members.retain(|m| m.id != member);
                bump(&mut hub, &room);
                json(serde_json::json!({}))
            }
            ("noriRemote.kick", Caller::Account(_)) => {
                let (room, member) = (p["room"].clone(), p["member"].clone());
                hub.keys.retain(|_, (r, m)| !(*r == room && m.as_deref() == Some(member.as_str())));
                hub.rooms.get_mut(&room).unwrap().members.retain(|m| m.id != member);
                bump(&mut hub, &room);
                json(serde_json::json!({}))
            }
            ("ping" | "star" | "unstar", _) => br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#.to_vec(),
            _ => subsonic_error("not here"),
        }
    }
}

#[async_trait::async_trait]
impl Transport for Relay {
    async fn get(&self, url: String, _timeout_ms: u32) -> Result<TransportResponse, TransportError> {
        self.send(Exchange { url, ..Default::default() }).await
    }

    async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
        if request.url.starts_with("http://127.0.0.1:") {
            self.hub.lock().asked.push(format!("lan {}", request.url));
            if self.doors_refuse.load(Ordering::Relaxed) {
                return Err(TransportError::Failed { kind: FailureKind::Connect, detail: Some("refused".into()) });
            }
            let late = self.door_late.load(Ordering::Relaxed) && request.url.contains("noriRemote.poll");
            let relay = self.me.upgrade().unwrap();
            let answer = off_thread(Some(self.sim.busy()), move || relay.lan_exchange(&request)).await;
            if late {
                self.sim.sleep(Duration::from_millis(250)).await;
            }
            return answer;
        }
        let unreachable = || Err(TransportError::Failed { kind: FailureKind::Connect, detail: Some("unreachable".into()) });
        if self.down.load(Ordering::Relaxed) {
            return unreachable();
        }
        if let Some(t1) = request.url.strip_prefix(&format!("{SERVER}/nori/time?t1=")).filter(|_| self.time.load(Ordering::Relaxed)) {
            self.hub.lock().asked.push("nori/time".into());
            let now = self.sim.now_us() + SERVER_SKEW_US;
            let body = Body::Clock { t1: t1.parse().unwrap(), t2: now, t3: now };
            let delay = self.first_time_delay_ms.swap(0, Ordering::Relaxed);
            if delay > 0 {
                self.sim.sleep(Duration::from_millis(delay)).await;
            }
            return Ok(TransportResponse { status: 200, body: json(body), headers: Vec::new() });
        }
        let Some(rest) = request.url.strip_prefix(&format!("{SERVER}/rest/")).filter(|_| !self.down.load(Ordering::Relaxed)) else {
            return unreachable();
        };
        let (endpoint, query) = rest.split_once('?').unwrap_or((rest, ""));
        let params: HashMap<String, String> = query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), decode(v))).collect();
        if endpoint == "noriRemote.close" && self.lose_closes.load(Ordering::Relaxed) {
            self.hub.lock().asked.push("lost close".into());
            return unreachable();
        }
        if endpoint == "noriRemote.open" && self.opens_late.load(Ordering::Relaxed) {
            let k = self.opens.fetch_add(1, Ordering::Relaxed);
            self.hub.lock().asked.push("open waits".into());
            self.sim.sleep(Duration::from_millis(300 - 50 * k.min(5))).await;
        }
        self.hub.lock().asked.push(format!("{endpoint} {} {}", params.get("id").map_or("", String::as_str), params.get("dev").map_or("", String::as_str)));
        if endpoint == "noriRemote.poll" {
            if self.leave_late.load(Ordering::Relaxed) && params.get("serve").map(String::as_str) == Some("0") {
                self.sim.sleep(Duration::from_millis(250)).await;
            }
            let late = params.get("serve").filter(|s| *s == "1").and_then(|_| {
                let mut late = self.late_poll.lock();
                late.take_if(|l| params.get("dev") == Some(&l.dev))
            });
            match late {
                Some(l) => {
                    l.caught.send(()).unwrap();
                    let (relay, sent) = (self.me.upgrade().unwrap(), params.clone());
                    // Sent, it arrives even if the device gives up waiting for the answer.
                    let busy = off_thread(None, move || {
                        let busy = l.go.recv().unwrap();
                        relay.arrived(&sent);
                        l.arrived.send(()).unwrap();
                        busy
                    })
                    .await;
                    drop(busy);
                }
                None => self.arrived(&params),
            }
            if !params.contains_key("since") {
                self.first_answer().await;
                if self.down.load(Ordering::Relaxed) {
                    return unreachable();
                }
            }
            if let (Some(since), Some("1")) = (params.get("since").and_then(|s| s.parse().ok()), params.get("hold").map(String::as_str)) {
                // The rooms it listens to as it arrives: a jam opened meanwhile is not one of them. A key
                // no longer known is refused at once.
                if let Ok(c) = self.caller(endpoint, &params) {
                    let dev = params.get("dev").map_or("", String::as_str);
                    let rooms = listened(&self.hub.lock(), &c, dev);
                    self.news(since, &rooms, dev).await;
                }
                if self.down.load(Ordering::Relaxed) {
                    return unreachable();
                }
            }
        }
        let json = match endpoint {
            "noriRemote.send" => self.lagged(params.get("dev").map_or("", String::as_str), request.json).await,
            _ => request.json,
        };
        Ok(TransportResponse { status: 200, body: self.answer(endpoint, &params, json.as_deref()), headers: Vec::new() })
    }

    fn address_changed(&self) {}

    fn network(&self) -> nori_core::transport::Network {
        nori_core::transport::Network::Unmetered
    }
}

/// `f` on a thread of its own, awaited as a request is: dropping the future gives it up. While `busy`,
/// the simulated clock waits for it.
async fn off_thread<T: Send + 'static>(busy: Option<Busy>, f: impl FnOnce() -> T + Send + 'static) -> T {
    let slot = Arc::new(Mutex::new((None, None::<Waker>)));
    let done = slot.clone();
    std::thread::spawn(move || {
        let v = f();
        let mut d = done.lock();
        d.0 = Some(v);
        if let Some(w) = d.1.take() {
            w.wake();
        }
        drop(d);
        drop(busy);
    });
    std::future::poll_fn(|cx| {
        let mut s = slot.lock();
        match s.0.take() {
            Some(v) => Poll::Ready(v),
            None => {
                s.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    })
    .await
}

/// One HTTP/1.1 exchange with a door on this machine; refused where no door listens, failed where the
/// door closed meanwhile.
impl Relay {
    /// `request` to a door over a kept connection, or a new one.
    fn lan_exchange(&self, request: &Exchange) -> Result<TransportResponse, TransportError> {
        let rest = request.url.strip_prefix("http://127.0.0.1:").unwrap();
        let (port, target) = rest.split_at(rest.find('/').unwrap());
        let port: u16 = port.parse().unwrap();
        let failed = |kind| move |e: std::io::Error| TransportError::Failed { kind, detail: Some(e.to_string()) };
        let kept = std::iter::from_fn(|| self.lan.lock().get_mut(&port).and_then(Vec::pop)).find(open);
        let mut r = match kept {
            Some(r) => r,
            None => BufReader::new(std::net::TcpStream::connect(("127.0.0.1", port)).map_err(failed(FailureKind::Connect))?),
        };
        let (method, body) = request.json.as_deref().map_or(("GET", ""), |b| ("POST", b));
        let read = failed(FailureKind::Interrupted);
        write!(r.get_mut(), "{method} {target} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()).map_err(read)?;
        let mut line = String::new();
        r.read_line(&mut line).map_err(read)?;
        let Some(status) = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()) else { return Err(read(std::io::ErrorKind::UnexpectedEof.into())) };
        let mut length = 0;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).map_err(read)?;
            if h.trim().is_empty() {
                break;
            }
            if let Some(v) = h.strip_prefix("Content-Length: ") {
                length = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        r.read_exact(&mut body).map_err(read)?;
        self.lan.lock().entry(port).or_default().push(r);
        Ok(TransportResponse { status, body, headers: Vec::new() })
    }
}

/// Whether a kept connection is still open: nothing to read yet, and not ended by the door.
fn open(r: &BufReader<std::net::TcpStream>) -> bool {
    let c = r.get_ref();
    let _ = c.set_nonblocking(true);
    let waiting = matches!(c.peek(&mut [0]), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock);
    let _ = c.set_nonblocking(false);
    waiting && r.buffer().is_empty()
}

/// What a device's discovery was asked to announce.
#[derive(Default)]
struct Announced(Mutex<Option<nori_core::remote::Announcement>>, std::sync::atomic::AtomicBool);

impl Discovery for Announced {
    fn announce(&self, door: Option<nori_core::remote::Announcement>) {
        *self.0.lock() = door;
    }

    fn browse(&self, on: bool) { self.1.store(on, Ordering::Relaxed); }
}

struct NoMarks;

impl StarsShown for NoMarks {
    fn marks(&self, _: nori_core::stars::StarMarks) {}
}

/// The ops a device's player was told to carry out.
struct Player(Mutex<Sender<Op>>);

impl RemotePlayer for Player {
    fn apply(&self, op: Op) {
        let _ = self.0.lock().send(op);
    }
}

/// Keeps each jam end the remote was told of (the host's name).
struct Shown(Arc<Mutex<Vec<Option<String>>>>);

impl RemoteShown for Shown {
    fn marks_changed(&self) {}

    fn changed(&self) {}

    fn jam_ended(&self, host: Option<String>) {
        self.0.lock().push(host);
    }
}

fn ann() -> ServerConfig {
    ServerConfig { url: SERVER.into(), user: "ann".into(), password: "pw".into(), ..Default::default() }
}

/// Opens a jam on `d`; its invite.
fn opened(d: &Device) -> String {
    d.sim.run("a jam opened", d.remote.clone().jam_open()).unwrap().expect("a jam opened")
}

/// Joins the jam `link` invites to from a device of no account, as `name`; its pass.
fn joined(relay: &Arc<Relay>, link: String, name: String) -> nori_core::remote::JamPass {
    let settings = nori_core::settings_store::Settings::new();
    match relay.sim.run("joined", jam_join(relay.clone(), settings, None, link, name)).unwrap() {
        JamJoin::Joined { pass } => pass,
        other => panic!("not joined: {other:?}"),
    }
}

/// Moves the test's clock until `ready`, for what does not tell a device's change notices (the relay's
/// own state).
fn eventually(relay: &Relay, what: &str, ready: impl Fn() -> bool) {
    relay.sim.until(what, || ready().then_some(()));
}

struct Device {
    sim: Arc<Native>,
    core: Arc<Core>,
    client: Arc<Client>,
    remote: Arc<Remote>,
    ops: Receiver<Op>,
    ended: Arc<Mutex<Vec<Option<String>>>>,
}

impl Device {
    fn new(relay: &Arc<Relay>, config: ServerConfig, kind: DeviceKind, name: &str) -> Device {
        Device::found(relay, config, kind, name, None)
    }

    fn found(relay: &Arc<Relay>, config: ServerConfig, kind: DeviceKind, name: &str, discovery: Option<Arc<dyn Discovery>>) -> Device {
        Device::with(relay, config, kind, name, discovery, Default::default())
    }

    fn with(relay: &Arc<Relay>, config: ServerConfig, kind: DeviceKind, name: &str, discovery: Option<Arc<dyn Discovery>>, session: Arc<nori_queue::Session>) -> Device {
        let core = Core::new(String::new(), "remote".into(), session).unwrap();
        core.configure(config).unwrap();
        let client = Client::new(core.clone(), relay.clone(), Default::default());
        client.set_profile(NetProfile { url: SERVER.into(), ..Default::default() });
        let (ops_to, ops) = channel();
        let ended = Arc::new(Mutex::new(Vec::new()));
        let (me, player, shown) = (RemoteMe { name: name.into(), kind }, Arc::new(Player(Mutex::new(ops_to))), Arc::new(Shown(ended.clone())));
        let remote = Remote::with_tasks(client.clone(), me, player, shown, discovery, relay.sim.clone());

        relay.devices.lock().push(Arc::downgrade(&remote));
        Device { sim: relay.sim.clone(), core, client, remote, ops, ended }
    }

    fn account(relay: &Arc<Relay>, kind: DeviceKind, name: &str) -> Device {
        Device::new(relay, ann(), kind, name)
    }

    /// An account's device whose app keeps its settings in `dir`, jams on; made again from the same
    /// `dir`, it is the same device after the app started again.
    fn kept(relay: &Arc<Relay>, dir: &nori_testdir::TempDir, kind: DeviceKind, name: &str) -> Device {
        let settings = nori_core::settings_store::Settings::new();
        let mut prefs = settings.open(&dir.path().join("app.db").to_string_lossy()).unwrap();
        prefs.remote_control = true;
        settings.put(prefs);
        Device::with(relay, ann(), kind, name, None, Arc::new(nori_queue::Session::new(settings)))
    }

    /// Moves the test's clock until `ready` gives a value.
    fn until<T>(&self, what: &str, mut ready: impl FnMut(&Remote) -> Option<T>) -> T {
        self.sim.until(what, || ready(&self.remote))
    }

    fn told(&self) -> Op {
        self.sim.until("an op for the player", || self.ops.try_recv().ok())
    }

    fn playing(&self, ids: &[&str], start: u32) {
        let songs: Vec<Song> = ids.iter().map(|id| Song { id: id.to_string(), title: id.to_uppercase(), duration: 200, ..Default::default() }).collect();
        self.core.session.register(songs);
        self.core.session.set(ids.iter().map(|s| s.to_string()).collect(), Some(start), false, None);
        self.remote.clone().played(Playing { playing: true, position_ms: 5_000, index: None, volume: Some(40), ..Default::default() });
    }

}

#[test]
fn returning_here_does_not_wait_for_a_dead_device() {
    for action in ["wait", "play", "edit"] {
        let relay = Relay::new();
        let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
        phone.playing(&["s1", "s2"], 0);
        phone.remote.clone().serve(true);
        let ghost = phone.until("the phone published", |_| relay.hub.lock().rooms.values().flat_map(|r| &r.members).find(|m| m.id == phone.remote.id() && m.state.as_ref().is_some_and(|s| s.playing)).cloned());
        phone.remote.clone().stop();
        relay.hub.lock().rooms.entry("u:ann".into()).or_default().members.push(ghost);
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        desk.queued(&["local"]);
        desk.remote.clone().serve(true);
        desk.remote.clone().watch(true);
        desk.until("the stale relay entry", |r| r.devices().into_iter().find(|d| d.id == phone.remote.id() && d.state.as_ref().is_some_and(|s| s.playing)));
        desk.remote.clone().watch(false);
        desk.remote.clone().pick(Some(phone.remote.id()));
        assert!(desk.remote.active().is_some());
        desk.remote.clone().pick(None);
        let picked_at = relay.sim.now_us();
        desk.until("the controls here", |r| r.active().is_none().then_some(()));
        assert!(relay.sim.now_us() - picked_at <= 3_000_000, "a dead device holds the controls no longer than a command shows");
        assert_eq!(desk.core.session.playlist(|p| p.ids().to_vec()), ["local"], "the stored local queue remains usable");
        let request = || relay.hub.lock().rooms["u:ann"].events.iter().find_map(|(_, from, _, body)| match body {
            Body::Command { id, op } if from == &desk.remote.id() && matches!(op.as_ref(), Op::Transfer { to, .. } if to == &desk.remote.id()) => Some(*id),
            _ => None,
        });
        eventually(&relay, "the queue request", || request().is_some());
        let transfer = request();
        match action {
            "play" => desk.playing(&["local"], 0),
            "edit" => desk.queued(&["edited"]),
            _ => {},
        }
        // A delayed response from the previous app instance must respect what happened here.
        let outgoing = Outgoing {
            to: Some(desk.remote.id()),
            body: Some(Body::Command { id: 999, op: Box::new(Op::Replace {
                songs: vec![Song::only_id("remote".into())], index: 0, position_ms: 5_000,
                play: true, order: None, shuffle: false, repeat: 0, transfer, listen: None, mix: None,
            }) }),
            ..Default::default()
        };
        relay.sim.run("a request", relay.send(Exchange {
            url: format!("{SERVER}/rest/noriRemote.send?u=ann&dev={}", phone.remote.id()),
            json: Some(serde_json::to_string(&outgoing).unwrap()), ..Default::default()
        })).unwrap();
        if action == "wait" {
            assert!(matches!(desk.told(), Op::Replace { songs, .. } if songs[0].id == "remote"));
        } else {
            eventually(&relay, "the late queue refused", || relay.hub.lock().rooms["u:ann"].events.iter().any(|(_, from, _, body)| {
                from == &desk.remote.id() && matches!(body, Body::Ack { id: 999, refusal: Some(Refusal::Stale) })
            }));
            assert!(desk.ops.try_recv().is_err(), "a late handoff cannot replace local playback or edits");
        }
        relay.close();
    }
}

#[test]
fn active_discovery_follows_the_playing_device() {
    let relay = Relay::new();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.playing(&["s1", "s2"], 1);
    desk.remote.clone().serve(true);
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    phone.queued(&["old"]);
    phone.remote.clone().discover_active(true);
    let mirror = phone.until("the desktop already playing", |r| r.active());
    assert_eq!((mirror.id, mirror.at, mirror.playing), (desk.remote.id(), Some(1), true));
    assert!(desk.ops.try_recv().is_err(), "discovering does not replace the desktop's queue");
    assert!(phone.ops.try_recv().is_err(), "discovering does not play the saved queue locally");
    relay.close();
}

#[test]
fn only_the_device_playing_shows_a_song() {
    let relay = Relay::new();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let term = Device::account(&relay, DeviceKind::Terminal, "Term");
    // The phone and the terminal played before and keep their queues; the desk plays now.
    phone.queued(&["c"]);
    term.queued(&["b"]);
    desk.playing(&["a"], 0);
    for d in [&desk, &phone, &term] {
        d.remote.clone().serve(true);
        d.remote.clone().watch(true);
    }
    phone.remote.clone().discover_active(true);
    phone.until("the desk followed", |r| r.active());
    let now = |d: &Device, name: &str| d.until(name, |r| r.devices().into_iter().find(|x| x.name == name && x.state.is_some())).now.map(|e| e.id);
    assert_eq!(now(&phone, "Desk").as_deref(), Some("a"), "the device playing");
    assert_eq!(now(&phone, "Term"), None, "the terminal plays nothing");
    assert_eq!(now(&desk, "Phone"), None, "the phone plays nothing");
    assert_eq!(now(&desk, "Term"), None);
    assert_eq!(now(&term, "Phone"), None);
    relay.close();
}

#[test]
fn a_like_where_the_music_plays_shows_on_every_page() {
    let relay = Relay::new();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    desk.playing(&["a", "b"], 0);
    desk.remote.clone().serve(true);
    phone.remote.clone().watch(true);
    phone.remote.clone().discover_active(true);
    phone.until("the desk followed", |r| r.active());
    let marked = |d: &Device, id: &str| d.remote.star_marks().songs.get(id).copied();
    // Liked from the phone's player, starred on the desk: the phone's pages show it as well.
    assert!(phone.remote.clone().star_where_playing("a".into(), true));
    assert_eq!(marked(&phone, "a"), Some(true));
    // Liked on the desk: the phone hears it in the desk's queue.
    relay.sim.run("a request", desk.client.star(Starrable::Song, "b".into(), true, Arc::new(NoMarks))).unwrap();
    phone.until("the desk's like", |r| r.star_marks().songs.get("b").copied().filter(|on| *on));
    relay.close();
}

#[test]
fn active_discovery_respects_local_controls_and_visibility() {
    for case in ["hidden", "picked here", "playing here", "paused peer"] {
        let relay = Relay::new();
        let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
        phone.remote.clone().discover_active(true);
        match case {
            "hidden" => phone.remote.clone().sight(Sight::Nothing),
            "picked here" => phone.remote.clone().pick(None),
            "playing here" => phone.playing(&["own"], 0),
            _ => {}
        }
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        if case == "paused peer" { desk.queued(&["s1"]); } else { desk.playing(&["s1"], 0); }
        desk.remote.clone().serve(true);
        phone.remote.clone().watch(true);
        phone.until(case, |r| r.devices().into_iter().find(|d| d.name == "Desk").and_then(|d| d.state));
        assert!(phone.remote.active().is_none(), "{case}");
        relay.close();
    }
}

#[test]
fn active_discovery_uses_lan_without_a_relay() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let desk = Device::found(&relay, ann(), DeviceKind::Desktop, "Desk", Some(announced.clone()));
    desk.playing(&["s1", "s2"], 1);
    desk.remote.clone().serve(true);
    let door = announced.0.lock().clone().unwrap();
    let browsing = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(browsing.clone()));
    phone.remote.clone().discover_active(true);
    assert!(browsing.1.load(Ordering::Relaxed));
    phone.remote.clone().lan_found(door.name, "127.0.0.1".into(), door.port, door.txt);
    let mirror = phone.until("the playing LAN device", |r| r.active());
    assert_eq!((mirror.id, mirror.at), (desk.remote.id(), Some(1)));
    assert!(!browsing.1.load(Ordering::Relaxed), "after discovery only the active device is followed");
    relay.close();
}

#[test]
fn a_restarted_device_publishes_a_newer_instance() {
    let relay = Relay::new();
    let dir = nori_testdir::TempDir::new("remote-restart");
    let first = Device::kept(&relay, &dir, DeviceKind::Desktop, "Desk");
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    first.playing(&["s1", "s2"], 0);
    first.remote.clone().serve(true);
    phone.remote.clone().watch(true);
    let before = phone.until("the first process", |r| r.devices().into_iter().find(|d| d.name == "Desk").and_then(|d| d.state));
    let id = first.remote.id();
    eventually(&relay, "the device id is stored", || first.core.session.settings.app_value("remoteDeviceId.desktop.0").as_deref() == Some(&id));
    first.remote.clone().stop();
    let second = Device::kept(&relay, &dir, DeviceKind::Desktop, "Desk");
    assert_eq!(second.remote.id(), id);
    second.playing(&["s1", "s2"], 1);
    second.remote.clone().serve(true);
    let after = phone.until("the restarted process", |r| r.devices().into_iter().find(|d| d.id == id).and_then(|d| d.state).filter(|s| s.index == Some(1)));
    assert!(after.instance.unwrap() > before.instance.unwrap());
    relay.close();
}

#[test]
fn clients_over_one_app_database_are_devices_of_their_own() {
    let relay = Relay::new();
    let dir = nori_testdir::TempDir::new("remote-shared-db");
    let old = {
        let settings = nori_core::settings_store::Settings::new();
        settings.open(&dir.path().join("app.db").to_string_lossy()).unwrap();
        settings.app_db().unwrap().lock().execute("INSERT INTO app_kv(key, value) VALUES('remoteDeviceId', 'old')", []).unwrap();
        "old".to_string()
    };
    let desk = Device::kept(&relay, &dir, DeviceKind::Desktop, "Mac");
    let terminal = Device::kept(&relay, &dir, DeviceKind::Terminal, "Mac");
    let second = Device::kept(&relay, &dir, DeviceKind::Desktop, "Mac");
    let ids = [desk.remote.id(), terminal.remote.id(), second.remote.id()];
    assert_eq!(ids[0], old, "the first client keeps the id the app had");
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2], "{ids:?}");

    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    for d in [&desk, &terminal, &second] {
        d.remote.clone().serve(true);
    }
    phone.remote.clone().watch(true);
    for id in &ids {
        relay.list(id, Some(DeviceState { playing: false, ..Default::default() }));
    }
    phone.until("the three", |r| (r.devices().len() == 3).then_some(()));
    phone.remote.clone().send(terminal.remote.id(), Op::Pause);
    assert_eq!(terminal.told(), Op::Pause);
    phone.remote.clone().send(desk.remote.id(), Op::Next);
    assert_eq!(desk.told(), Op::Next);
    assert!(terminal.ops.try_recv().is_err() && second.ops.try_recv().is_err(), "only the device asked carries it out");

    // Stopped, its id is free for the next copy of its kind.
    second.remote.clone().stop();
    let again = Device::kept(&relay, &dir, DeviceKind::Desktop, "Mac");
    assert_eq!(again.remote.id(), ids[2]);
    relay.close();
}

#[test]
fn a_handoff_counts_one_listen() {
    use nori_core::scrobble::TrackChange;

    for (heard, pull) in [(120_000, true), (300_000, true), (120_000, false), (300_000, false)] {
        let relay = Relay::new();
        let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        phone.playing(&["long"], 0);
        let song = Song { id: "long".into(), duration: 600, ..Default::default() };
        phone.core.session.register(vec![song]);
        let now = relay.sim.now_us() / 1_000;
        let started = nori_core::db::now_ms() - heard;
        phone.core.session.scrobble_track(Some("long".into()), TrackChange::Moved, true, now - heard, started, 0);
        phone.remote.clone().serve(true);
        desk.remote.clone().watch(true);
        desk.until("the phone", |r| r.devices().into_iter().find(|d| d.name == "Phone"));
        if pull {
            desk.remote.clone().send(phone.remote.id(), Op::Transfer { to: desk.remote.id(), listen_handoff: true });
        } else {
            desk.playing(&["other"], 0);
            desk.remote.clone().serve(true);
            phone.remote.clone().watch(true);
            phone.until("the desk", |r| r.devices().into_iter().find(|d| d.name == "Desk"));
            phone.remote.clone().hand_over(desk.remote.id());
        }
        let Op::Replace { songs, .. } = desk.told() else { panic!("handed queue") };
        assert_eq!(phone.told(), Op::Pause);
        desk.core.session.register(songs);
        let now = relay.sim.now_us() / 1_000;
        phone.core.session.scrobble_playing(false, now);
        desk.core.session.scrobble_track(Some("long".into()), TrackChange::Moved, true, now, nori_core::db::now_ms(), 0);
        let donor = phone.core.session.scrobble_track(None, TrackChange::Ended, false, now, nori_core::db::now_ms(), 0);
        let receiver = desk.core.session.scrobble_track(None, TrackChange::Ended, false, now + heard, nori_core::db::now_ms() + heard, 0);
        assert_eq!(donor.submit_id, None, "the old device no longer owns the listen");
        assert_eq!(receiver.submit_id.as_deref(), Some("long"), "time heard on both devices qualifies");
        assert_eq!(receiver.submit_at, started, "the original start belongs to the same listen");
        nori_core::background::flush();
        assert!(phone.core.history_recent(10, None, true).unwrap().0.is_empty(), "the donor does not record the listen again");
        let history = desk.core.history_recent(10, None, true).unwrap().0;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].started_ms, started);
        assert!(history[0].heard_ms >= 2 * heard);
        relay.close();
    }
}

#[test]
fn a_handoff_during_a_mix_carries_it_on() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let session = &phone.core.session;
    session.settings.open("").unwrap();
    let mut prefs = session.settings.current().unwrap();
    prefs.crossfade_sec = 6;
    session.settings.put(prefs);
    phone.playing(&["s1", "s2", "s3"], 1);
    session.planner.transition_window(
        ["s1", "s2", "s3"].into_iter().map(|id| nori_player::transitions::WindowSong { id: id.into(), duration_ms: 200_000, ..Default::default() }).collect(),
        false,
    );
    let plan = session.planner.plan_for("s1").expect("a crossfade out of s1");
    // Three seconds into the crossfade: s2 is the song shown, s1's ending still sounds.
    phone.remote.clone().played(Playing {
        playing: true,
        mixing: true,
        mix_from: Some(0),
        mix_position_ms: 197_000,
        mix_rate: 1.0,
        position_ms: 3_000,
        index: Some(1),
        volume: Some(40),
        ..Default::default()
    });
    phone.remote.clone().serve(true);
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.name == "Phone"));

    desk.remote.clone().send(phone.remote.id(), Op::Transfer { to: desk.remote.id(), listen_handoff: true });
    match desk.told() {
        Op::Replace { index, position_ms, mix, .. } => {
            assert_eq!(index, 0, "the song mixed out of goes on");
            assert!((197_000..198_000).contains(&position_ms), "where it was heard, at {position_ms}");
            assert_eq!(mix.map(|m| (m.from, m.into)), Some((0, "s2".to_string())));
        }
        op => panic!("{op:?}"),
    }
    assert_eq!(desk.core.session.planner.plan_for("s1"), Some(plan), "the desk mixes on by the phone's plan");
    relay.close();
}

#[test]
fn two_devices_control_each_other_through_the_relay() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2", "s3"], 1);
    phone.remote.clone().serve(true);
    desk.remote.clone().watch(true);

    let state = desk.until("the phone's state", |r| r.devices().into_iter().find(|d| d.name == "Phone").and_then(|d| d.state));
    assert_eq!((state.index, state.playing, state.volume), (Some(1), true, Some(40)));
    assert_eq!(state.entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["S1", "S2", "S3"]);
    let phone_id = phone.remote.id();

    desk.remote.clone().send(phone_id.clone(), Op::Next);
    assert_eq!(phone.told(), Op::Next);
    // The song the player says it arrived on is the one shown, before the queue's own current moves.
    phone.remote.clone().played(Playing { playing: true, position_ms: 0, index: Some(2), volume: Some(40), ..Default::default() });
    let moved = desk.until("the next song", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.index == Some(2)));
    assert_eq!(moved.entries.iter().find(|e| Some(e.index) == moved.index).map(|e| e.title.as_str()), Some("S3"));

    // An edit made against a queue that changed since is refused, and the controller is told.
    let stale = state.rev;
    phone.core.session.remove(2, 3);
    phone.remote.clone().played(Playing { playing: true, position_ms: 6_000, index: None, volume: Some(40), ..Default::default() });
    desk.remote.clone().send(phone_id.clone(), Op::Remove { index: 0, rev: stale });
    let refused = desk.until("the refusal", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.refused));
    assert_eq!(refused, Refusal::Stale);
    assert!(phone.ops.try_recv().is_err(), "nothing done");
    let fresh = desk.until("the new queue", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.rev != stale));
    desk.remote.clone().send(phone_id.clone(), Op::Remove { index: 0, rev: fresh.rev });
    assert_eq!(phone.told(), Op::Remove { index: 0, rev: fresh.rev });

    // Playing here: the phone hands over its queue and position, then pauses.
    desk.remote.clone().send(phone_id.clone(), Op::Transfer { to: desk.remote.id(), listen_handoff: true });
    match desk.told() {
        Op::Replace { songs, index, position_ms, play, .. } => {
            assert_eq!(songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
            assert_eq!(index, 1);
            assert!(play && position_ms >= 6_000, "at {position_ms}");
        }
        op => panic!("{op:?}"),
    }
    assert_eq!(phone.told(), Op::Pause);

    // And back: the desk sends what it plays to the phone.
    desk.playing(&["s4"], 0);
    desk.remote.clone().hand_over(phone_id.clone());
    assert!(matches!(phone.told(), Op::Replace { songs, index: 0, play: true, .. } if songs[0].id == "s4"));
    assert_eq!(desk.told(), Op::Pause);

    // Not serving any more: the phone leaves the list.
    phone.remote.clone().serve(false);
    desk.until("the phone gone", |r| r.devices().iter().all(|d| d.id != phone_id).then_some(()));
    relay.close();
}

#[test]
fn a_jam_takes_requests_through_its_host() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    assert!(link.starts_with("http://octo:5274/nori/jam#s=http%3A%2F%2Focto%3A5274&k="), "{link}");

    let guest_of = |name: &str| {
        let pass = joined(&relay, link.clone(), name.into());
        assert_eq!(pass.url, SERVER);
        let d = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, name);
        d.remote.clone().watch(true);
        d
    };
    let gus = guest_of("Gus");
    let dee = guest_of("Dee");

    let view = gus.until("the host's queue", |r| r.jam_view().filter(|v| v.queue.is_some() && v.members.len() == 3));
    assert!(!view.hosting);
    assert_eq!(view.queue.unwrap().entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
    let roles: Vec<(String, Role)> = view.members.iter().map(|m| (m.name.clone(), m.role)).collect();
    assert_eq!(roles, [("Host".to_string(), Role::Host), ("Gus".into(), Role::Guest), ("Dee".into(), Role::Guest)]);
    let dee_id = view.members.iter().find(|m| m.name == "Dee").unwrap().id.clone();

    // The host makes Dee an admin.
    host.remote.clone().jam_act(Op::Promote { member: dee_id, admin: true });
    dee.until("Dee's role", |r| r.jam_view().filter(|v| v.members.iter().any(|m| m.name == "Dee" && m.role == Role::Admin)));

    assert!(!view.remote_songs);
    host.remote.clone().jam_remote_songs(true);
    gus.until("provider permission", |r| r.jam_view().filter(|v| v.remote_songs));

    // A guest's provider song waits, untouched, until an admin accepts it.
    let wish = Song { id: "ext-deezer-song-9".into(), title: "Wish".into(), is_external: true, ..Default::default() };
    gus.remote.clone().jam_act(Op::Request { song: wish.clone() });
    let pending = dee.until("the request", |r| r.jam_view().and_then(|v| v.pending.first().cloned()));
    assert!(pending.provider && pending.from_name == "Gus");
    assert!(host.ops.try_recv().is_err(), "not queued yet");

    // Dee's own request goes straight in.
    let blue = Song { id: "s5".into(), title: "Blue".into(), ..Default::default() };
    dee.remote.clone().jam_act(Op::Request { song: blue.clone() });
    assert_eq!(host.told(), Op::Add { songs: vec![blue.clone()], next: false });

    // What the host plays, as a guest's player shows it: the song, the queue around it, who added each.
    host.core.session.register(vec![blue]);
    host.core.session.take(2, vec!["s5".into()], vec![Hand::Last]);
    let said_before = relay.sim.now_us();
    host.remote.clone().played(Playing { playing: true, position_ms: 9_000, index: Some(0), ..Default::default() });
    let m = gus.until("the host's queue in Gus's player", |r| r.jam_playing().filter(|m| m.rows.len() == 3));
    assert_eq!(m.rows.iter().map(|r| r.song.id.as_str()).collect::<Vec<_>>(), ["s1", "s5", "s2"], "in play order: what was asked for plays next");
    assert_eq!((m.name.as_str(), m.at, m.playing), ("Host", Some(0), true));
    assert_eq!((m.next, m.previous), (None, None), "a guest's skips reach nowhere");
    let admin = dee.until("the host's queue in Dee's player", |r| r.jam_playing().filter(|m| m.rows.len() == 3));
    assert_eq!((admin.next, admin.previous), (Some(1), None), "an admin's skip the host's queue");
    let latest = 9_000 + (relay.sim.now_us() - said_before) / 1_000;
    assert!((9_000..=latest).contains(&m.position_ms), "the published position advances from the report: {} outside 9000..={latest}", m.position_ms);
    assert_eq!(gus.remote.jam_added().get("s5").map(String::as_str), Some("Dee"));
    assert!(host.remote.jam_playing().is_none(), "the host plays it itself");

    dee.remote.clone().jam_act(Op::Decide { request: pending.request, accept: true });
    assert_eq!(host.told(), Op::Add { songs: vec![wish], next: false });
    gus.until("the request gone", |r| r.jam_view().filter(|v| v.pending.is_empty()));
    let added = host.remote.jam_added();
    assert_eq!(added.get("ext-deezer-song-9").map(String::as_str), Some("Gus"), "added by who asked, not who accepted");
    assert_eq!(added.get("s5").map(String::as_str), Some("Dee"));
    assert_eq!(added.len(), 2, "the host's own songs are nobody's: {added:?}");

    // A guest cannot accept, and is told.
    gus.remote.clone().jam_act(Op::Request { song: Song { id: "s4".into(), ..Default::default() } });
    let pending = gus.until("Gus's second request", |r| r.jam_view().and_then(|v| v.pending.first().cloned()));
    gus.remote.clone().jam_act(Op::Decide { request: pending.request, accept: true });
    assert_eq!(gus.until("the refusal", |r| r.jam_view().and_then(|v| v.refused)), Refusal::NotAllowed);

    host.remote.clone().jam_close();
    assert!(host.remote.jam_added().is_empty(), "no jam, no names");
    assert!(host.remote.jam_view().is_none(), "the host is in no jam once it ended it");

    let provider_asked: Vec<String> = relay.asked().into_iter().filter(|a| a.contains("ext-")).collect();
    assert!(provider_asked.is_empty(), "the relay never looked the provider song up: {provider_asked:?}");
    relay.close();
}

#[test]
fn a_jam_opened_while_a_poll_is_held_hears_its_first_request() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1"], 0);
    // The devices sheet is open: a poll of the account's room is held when the jam opens from it.
    host.remote.clone().watch(true);
    eventually(&relay, "the held poll", || !relay.waiting.lock().is_empty());
    let link = opened(&host);

    // A guest asks at once, before anything else happens in the account's room.
    let pass = joined(&relay, link, "Gus".into());
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    gus.remote.clone().watch(true);
    gus.until("the host's state", |r| r.jam_view().filter(|v| v.queue.is_some()));
    gus.remote.clone().jam_act(Op::Request { song: Song { id: "s2".into(), title: "S2".into(), ..Default::default() } });
    let asked = host.until("Gus's request", |r| r.jam_view().and_then(|v| v.pending.first().cloned()));
    assert_eq!((asked.from_name.as_str(), asked.song.id.as_str()), ("Gus", "s2"));
    relay.close();
}

#[test]
fn library_discovery_follows_the_foreground_while_playing() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.core.session.settings.open("").unwrap();
    phone.remote.clone().serve(true);
    phone.playing(&["s1"], 0);
    assert!(announced.1.load(Ordering::Relaxed), "a playing client can find peers for favorite updates");
    phone.remote.clone().sight(Sight::Nothing);
    assert!(!announced.1.load(Ordering::Relaxed), "background playback adds no discovery listener");
    phone.remote.clone().sight(Sight::Screen);
    assert!(announced.1.load(Ordering::Relaxed));
    phone.remote.stop();
    relay.close();
}

#[test]
fn nearby_favorites_update_without_playback_commands() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.core.session.settings.open("").unwrap();
    phone.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the door announced");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.core.session.settings.open("").unwrap();
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name, "127.0.0.1".into(), door.port, door.txt);
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.state.is_some()));
    for kind in [Starrable::Song, Starrable::Album, Starrable::Artist] {
        for on in [true, false] {
            relay.sim.run("a request", desk.client.star(kind, "s1".into(), on, Arc::new(NoMarks))).unwrap();
            phone.until("the favorite change", |_| {
                (phone.core.starred(kind, "s1", !on) == on).then_some(())
            });
            assert!(phone.ops.try_recv().is_err(), "library changes do not reach the player");
        }
    }
    assert_eq!(relay.asked_for("star"), 3, "only the three local writes reach the server");
    assert_eq!(relay.asked_for("unstar"), 3, "received marks do not repeat server writes");
    let mut prefs = phone.core.session.settings.current().unwrap();
    prefs.remote_control = false;
    phone.core.session.settings.put(prefs);
    desk.remote.clone().send(phone.remote.id(), Op::LibraryMarks { marks: vec![nori_remote::wire::LibraryMark { kind: Starrable::Song, id: "blocked".into(), on: true }] });
    desk.until("disabled favorites refused", |r| r.devices().into_iter().find(|d| d.id == phone.remote.id() && d.refused == Some(Refusal::NotAllowed)));
    assert!(!phone.core.starred(Starrable::Song, "blocked", false));
    desk.remote.stop();
    phone.remote.stop();
    relay.close();
}

#[test]
fn nearby_devices_need_no_relay() {
    let relay = Relay::absent();
    let ann = |password: &str| ServerConfig { url: SERVER.into(), user: "ann".into(), password: password.into(), ..Default::default() };
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann("pw"), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the door announced");
    let txt = || door.txt.clone();

    // Another account's device on the network does not list the door.
    let other = Device::new(&relay, ServerConfig { user: "bob".into(), ..ann("pw") }, DeviceKind::Desktop, "Bob");
    other.remote.clone().watch(true);
    other.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, txt());
    assert!(other.remote.devices().is_empty());

    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, txt());
    let seen = desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.state.is_some()));
    assert!(seen.nearby && seen.name == "Phone");
    desk.remote.clone().send(seen.id.clone(), Op::Pause);
    assert_eq!(phone.told(), Op::Pause);
    let through_relay = format!("noriRemote.send  {}", desk.remote.id());
    assert!(!relay.asked().contains(&through_relay), "the command went to the door, not the relay");

    // Mirrored with the picker closed: its door is still followed, and its whole queue read through it.
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(seen.id.clone()));
    phone.remote.clone().played(Playing { playing: false, position_ms: 9_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone's newer word", |r| r.active().filter(|m| m.position_ms == 9_000 && m.rows.len() == 2));

    // The door seen again (mDNS reports it once per network interface) is still followed by the one poller.
    let first_polls = || relay.asked().iter().filter(|a| a.starts_with("lan ") && a.contains("noriRemote.poll") && !a.contains("since=")).count();
    let before = first_polls();
    for _ in 0..3 {
        desk.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, txt());
    }
    phone.remote.clone().played(Playing { playing: true, position_ms: 1_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone playing", |r| r.active().filter(|m| m.playing));
    assert_eq!(first_polls(), before, "no poller started over");

    // Seen at an address of this machine's no door answers at (another network interface): the phone is
    // still followed where it answers.
    let nowhere = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    desk.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), nowhere, txt());
    phone.remote.clone().played(Playing { playing: false, position_ms: 3_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone paused", |r| r.active().filter(|m| !m.playing));
    phone.remote.clone().played(Playing { playing: true, position_ms: 4_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone playing again", |r| r.active().filter(|m| m.playing));

    // Its word comes late through the door; its clock, learned there at once, says when it was heard.
    relay.door_late.store(true, Ordering::Relaxed);
    let said_before = relay.sim.now_us();
    phone.remote.clone().played(Playing { playing: true, position_ms: 12_000, index: None, volume: None, ..Default::default() });
    let said_after = relay.sim.now_us();
    let m = desk.until("the phone's place", |r| r.active().filter(|m| m.playing && m.position_ms >= 12_000));
    let now = relay.sim.now_us();
    let earliest = heard_at(12_000, said_after, now) - 5;
    let latest = heard_at(12_000, said_before, now) + 5;
    assert!((earliest..=latest).contains(&m.position_at(now)), "the desk shows {} ms outside the heard interval {earliest}..={latest}", m.position_at(now));
    relay.door_late.store(false, Ordering::Relaxed);

    // A device of the same user with an old password is not shown the door.
    let stranger = Device::new(&relay, ann("old"), DeviceKind::Desktop, "Old");
    stranger.remote.clone().watch(true);
    stranger.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, txt());
    assert!(stranger.remote.devices().is_empty(), "the announced account tag differs");
}

/// mDNS says a door went away of a device still there (an announcement missed while the network slept):
/// the device mirrored through it stays mirrored while its door answers, and goes once it does not.
#[test]
fn a_mirrored_door_whose_announcement_went_stays_while_it_answers() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the door announced");
    let phone_id = phone.remote.id();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, door.txt.clone());
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.state.is_some()));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active().filter(|m| m.playing));

    desk.remote.clone().lan_lost(door.name.clone());
    assert_eq!(desk.remote.active().map(|m| m.id), Some(phone_id.clone()), "still mirrored");
    phone.remote.clone().played(Playing { playing: false, position_ms: 9_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone's newer word", |r| r.active().filter(|m| !m.playing && m.position_ms == 9_000));

    stop(&phone);
    desk.until("the phone gone", |r| r.active().is_none().then_some(()));
    assert!(desk.remote.devices().is_empty());
    relay.close();
}

/// A door whose announcement went that nothing follows leaves the devices at once.
#[test]
fn a_door_whose_announcement_went_leaves_the_picker_once_closed() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the door announced");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, door.txt.clone());
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.state.is_some()));

    desk.remote.clone().lan_lost(door.name.clone());
    assert_eq!(desk.remote.devices().len(), 1, "listed while the open picker follows it");
    desk.remote.clone().watch(false);
    assert!(desk.remote.devices().is_empty(), "gone once nothing follows it");
    relay.close();
}

#[test]
fn a_server_without_the_relay_is_asked_again_only_by_a_picker() {
    let relay = Relay::absent();
    let dir = nori_testdir::TempDir::new("remote-no-relay");
    let phone = Device::kept(&relay, &dir, DeviceKind::Phone, "Phone");
    phone.until("the answer", |r| (r.relay() == RelaySupport::Unsupported).then_some(()));
    let asked = || relay.asked().iter().filter(|a| a.starts_with("noriRemote.")).count();
    assert_eq!(asked(), 1, "one probe");
    assert_eq!(phone.remote.jam_start(), JamStart::Unsupported, "every screen says the server lacks the relay");

    // Serving, watching and playing go on without the relay; a jam is refused before anything is asked.
    phone.playing(&["s1"], 0);
    phone.remote.clone().watch(true);
    phone.until("the picker's probe", |_| (asked() == 2).then_some(()));
    phone.remote.clone().serve(true);
    phone.remote.clone().watch(false);
    phone.remote.clone().serve(false);
    phone.remote.clone().send("elsewhere".into(), Op::Pause);
    assert!(relay.sim.run("a request", phone.remote.clone().jam_open()).is_err());
    assert_eq!(asked(), 2, "nothing more asked of a server that has no relay");
    assert!(phone.remote.jam_view().is_none());
}

#[test]
fn a_picker_opened_asks_again_whether_the_server_relays() {
    let relay = Relay::absent();
    let dir = nori_testdir::TempDir::new("remote-no-relay");
    let phone = Device::kept(&relay, &dir, DeviceKind::Phone, "Phone");
    phone.until("the answer", |r| (r.relay() == RelaySupport::Unsupported).then_some(()));
    let asked = || relay.asked().iter().filter(|a| a.starts_with("noriRemote.")).count();

    phone.remote.clone().watch(true);
    phone.until("asked again", |_| (asked() == 2).then_some(()));
    phone.remote.clone().watch(false);
    assert_eq!(phone.remote.relay(), RelaySupport::Unsupported);

    // The server gained the relay: the next opening finds it, and the picker follows the account's devices.
    relay.absent.store(false, Ordering::Relaxed);
    phone.remote.clone().watch(true);
    phone.until("the relay", |r| (r.relay() == RelaySupport::Supported).then_some(()));
    assert_eq!(phone.remote.jam_start(), JamStart::Offered);
    phone.until("polling", |_| (phone.polls(&relay) >= 3).then_some(()));
    relay.close();
}

/// A picker opened again while the last opening's answer is still on its way asks once more after it.
#[test]
fn a_picker_opened_during_an_answer_asks_again() {
    let relay = Relay::absent();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    phone.until("the answer", |r| (r.relay() == RelaySupport::Unsupported).then_some(()));
    let asked = || relay.asked().iter().filter(|a| a.starts_with("noriRemote.")).count();

    relay.first_answers_held.store(true, Ordering::Relaxed);
    phone.remote.clone().watch(true);
    eventually(&relay, "asked again", || asked() == 2);
    phone.remote.clone().watch(false);
    phone.remote.clone().watch(true);
    relay.release_first_answers();
    eventually(&relay, "asked for the second opening", || asked() == 3);
    relay.close();
}

impl Relay {
    /// Time exchanges sent through the relay so far.
    fn clocks(&self) -> usize {
        let hub = self.hub.lock();
        hub.rooms.values().flat_map(|r| &r.events).filter(|(_, _, _, b)| matches!(b, Body::Command { op, .. } if matches!(**op, Op::Clock { .. }))).count()
    }
}

#[test]
fn a_mirrored_device_is_timed_only_while_it_plays_on_screen() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().played(Playing { playing: false, position_ms: 5_000, ..Default::default() });
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored, paused", |r| r.active().filter(|m| !m.playing));
    // A whole burst's time.
    relay.sim.pass(Duration::from_millis(2_500));
    assert_eq!(relay.clocks(), 0, "nothing wakes for a playhead standing still");

    // Playing, with only the notification showing it (the screen off).
    desk.remote.clone().sight(Sight::Notification);
    phone.remote.clone().played(Playing { playing: true, position_ms: 5_000, ..Default::default() });
    desk.until("the phone playing", |r| r.active().filter(|m| m.playing));
    relay.sim.pass(Duration::from_millis(2_500));
    assert_eq!(relay.clocks(), 0, "nor for a playhead nothing shows");

    desk.remote.clone().sight(Sight::Screen);
    desk.until("timed once it plays on screen", |_| (relay.clocks() > 0).then_some(()));
    relay.close();
}

/// A mirrored device gone quiet without leaving the relay shows paused once two time exchanges went
/// unanswered, and plays again once it answers.
#[test]
fn a_mirrored_device_gone_quiet_stops_playing() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.state.is_some()));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored, playing", |r| r.active().filter(|m| m.playing));
    relay.sim.pass(Duration::from_millis(2_500));

    *relay.deaf.lock() = Some(phone_id.clone());
    let sent = relay.clocks();
    eventually(&relay, "two exchanges unanswered", || relay.clocks() == sent + 2);
    assert!(desk.remote.active().unwrap().playing, "still playing until the next exchange falls due");
    let quiet = desk.until("shown paused", |r| r.active().filter(|m| !m.playing));
    assert_eq!(relay.clocks(), sent + 3, "paused as the third falls due");
    let at = quiet.position_at(relay.sim.now_us());
    assert!(at > 30_000, "paused where it was last shown, not where it was last heard: {at}");
    relay.sim.pass(Duration::from_secs(5));
    assert_eq!(desk.remote.active().unwrap().position_at(relay.sim.now_us()), at, "the playhead stands still");

    *relay.deaf.lock() = None;
    desk.until("playing again once it answers", |r| r.active().filter(|m| m.playing));
    relay.close();
}

/// A mirrored device whose door refuses a time exchange shows paused at once when nothing else reaches
/// it; with the relay up, the relay's exchanges decide.
#[test]
fn a_mirrored_device_whose_door_refuses_stops_playing() {
    for relayed in [false, true] {
        let relay = if relayed { Relay::new() } else { Relay::absent() };
        let announced = Arc::new(Announced::default());
        let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
        phone.playing(&["s1", "s2"], 0);
        phone.remote.clone().serve(true);
        let door = announced.0.lock().clone().unwrap();
        let phone_id = phone.remote.id();
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        desk.remote.clone().watch(true);
        desk.remote.clone().lan_found(door.name, "127.0.0.1".into(), door.port, door.txt);
        desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.nearby));
        desk.remote.clone().watch(false);
        desk.remote.clone().pick(Some(phone_id.clone()));
        desk.until("the phone mirrored, playing", |r| r.active().filter(|m| m.playing));
        relay.sim.pass(Duration::from_millis(2_500));

        let times = || relay.asked().iter().filter(|a| a.starts_with("lan ") && a.contains("noriRemote.time")).count();
        let (before, refused_at) = (times(), relay.sim.now_us());
        relay.doors_refuse.store(true, Ordering::Relaxed);
        if relayed {
            for _ in 0..45 {
                relay.sim.pass(Duration::from_secs(1));
                assert!(desk.remote.active().unwrap().playing, "still playing while the relay reaches it");
            }
            assert!(times() > before, "its door was asked and refused");
        } else {
            desk.until("shown paused", |r| r.active().filter(|m| !m.playing));
            assert!(relay.sim.now_us() - refused_at < clock::EVERY_US, "at once");
            assert_eq!(times(), before + 1, "on the refused time exchange");
        }
        relay.close();
    }
}

/// A device that stops serving while a poll saying it serves is on its way leaves again after it.
#[test]
fn a_device_leaves_after_its_last_poll() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let id = phone.remote.id();
    let member = || relay.hub.lock().rooms.values().flat_map(|r| &r.members).any(|m| m.id == id);
    eventually(&relay, "the phone listed", || relay.hub.lock().rooms.values().flat_map(|r| &r.members).any(|m| m.id == id && m.state.is_some()));
    let (caught, go, arrived) = relay.late_poll(&id);
    // Its held poll answers, and the next one sets out.
    {
        let mut hub = relay.hub.lock();
        hub.seq += 1;
        let seq = hub.seq;
        room_touched(&mut hub, "u:ann", seq);
        relay.wake();
    }
    relay.sim.until("the next poll on its way", || caught.try_recv().ok());
    phone.remote.clone().serve(false);
    eventually(&relay, "the leave at the relay", || !member());
    go.send(relay.sim.busy()).unwrap();
    relay.sim.until("the poll arrived", || arrived.try_recv().ok());
    eventually(&relay, "left again after it", || !member());
    relay.close();
}

/// A poll from before the device served, reaching the relay late, takes it off the list: it says its
/// state again.
#[test]
fn a_serving_device_the_relay_dropped_is_listed_again() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let id = phone.remote.id();
    let listed = || relay.hub.lock().rooms.values().flat_map(|r| &r.members).any(|m| m.id == id && m.state.is_some());
    eventually(&relay, "the phone listed", listed);
    let late: HashMap<String, String> = [("u", "ann"), ("dev", id.as_str()), ("serve", "0")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
    relay.arrived(&late);
    eventually(&relay, "the phone listed again", listed);
    relay.close();
}

#[test]
fn a_paused_device_out_of_sight_is_not_followed() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().played(Playing { playing: false, position_ms: 5_000, ..Default::default() });
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored, paused", |r| r.active().filter(|m| !m.playing));

    // Nothing shows the phone on the desk any more: its held poll ends, and the phone playing is not heard.
    desk.remote.clone().sight(Sight::Nothing);
    phone.remote.clone().played(Playing { playing: true, position_ms: 5_000, ..Default::default() });
    let plays = || relay.hub.lock().rooms.values().flat_map(|r| &r.members).any(|m| m.id == phone_id && m.state.as_ref().is_some_and(|s| s.playing));
    phone.until("the relay has the phone playing", |_| plays().then_some(()));
    relay.sim.pass(Duration::from_millis(300));
    assert!(desk.remote.active().is_some_and(|m| !m.playing), "the desk did not hear it");
    // A poll on its way as the sight changed may still arrive, and is answered; none waits for more.
    assert!(!relay.parked(&desk.remote.id()), "nor asks");

    desk.remote.clone().sight(Sight::Screen);
    desk.until("followed again, playing", |r| r.active().filter(|m| m.playing));
    relay.close();
}

impl Device {
    /// Polls this device has asked of the relay.
    fn polls(&self, relay: &Relay) -> usize {
        relay.asked().iter().filter(|a| a.starts_with("noriRemote.poll") && a.ends_with(&self.remote.id())).count()
    }
}

#[test]
fn a_device_playing_elsewhere_is_mirrored_whole() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let ids: Vec<String> = (0..130).map(|n| format!("s{n}")).collect();
    phone.playing(&ids.iter().map(String::as_str).collect::<Vec<_>>(), 60);
    phone.core.session.shuffle(true);
    phone.remote.clone().played(Playing { playing: true, position_ms: 5_000, index: None, volume: Some(40), ..Default::default() });
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();

    // Nothing queued on the desk: picking the phone only follows it, without a picker open.
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().watch(false);
    assert!(!desk.remote.mirroring());
    desk.remote.clone().pick(Some(phone_id.clone()));
    let order: Vec<u32> = phone.core.session.playlist(|p| p.play_order().map(|i| i as u32).collect());
    let m = desk.until("the whole queue", |r| r.active().filter(|m| m.rows.len() == 130));
    assert!(desk.remote.mirroring());
    assert_eq!(m.rows.iter().map(|r| r.index).collect::<Vec<_>>(), order, "in play order, by list index");
    assert_eq!(m.at.map(|a| m.rows[a as usize].song.id.clone()), Some("s60".to_string()));
    assert!(m.shuffle && m.playing && m.len == 130);
    assert!(phone.ops.try_recv().is_err(), "pages are answered by the core, not the player");

    // A command shows at once, before the phone's own word.
    desk.remote.clone().send(phone_id.clone(), Op::Pause);
    let m = desk.remote.active().unwrap();
    assert!(!m.playing);
    assert_eq!(phone.told(), Op::Pause);
    desk.remote.clone().send(phone_id.clone(), Op::Volume { percent: 15 });
    assert!(desk.remote.clone().star_where_playing("s60".into(), true), "a song of its queue is starred there");
    assert!(!desk.remote.clone().star_where_playing("elsewhere".into(), true), "any other here");
    let m = desk.remote.active().unwrap();
    assert_eq!(m.volume, Some(15));
    assert!(m.rows[m.at.unwrap() as usize].song.starred);
    assert_eq!((phone.told(), phone.told()), (Op::Volume { percent: 15 }, Op::Star { id: "s60".into(), on: true }));
    // The phone's player stars it, as its own heart would: once, on the server.
    relay.sim.run("a request", phone.client.star(Starrable::Song, "s60".into(), true, Arc::new(NoMarks))).unwrap();
    assert_eq!(relay.asked().iter().filter(|a| a.starts_with("star ")).count(), 1);

    // The phone's next word corrects what was foreseen, and keeps the heart its record does not have yet.
    phone.remote.clone().played(Playing { playing: false, position_ms: 7_000, index: None, volume: Some(30), ..Default::default() });
    let m = desk.until("the phone's volume", |r| r.active().filter(|m| m.volume == Some(30) && m.position_ms == 7_000));
    assert!(m.rows[m.at.unwrap() as usize].song.starred);
    // Its own keys move it too.
    phone.remote.clone().volume_changed(Some(22));
    desk.until("the phone's keys", |r| r.active().filter(|m| m.volume == Some(22) && m.position_ms == 7_000));

    // Gone: no longer followed, and nothing more is asked of the relay for it.
    phone.remote.clone().serve(false);
    desk.until("the phone gone", |r| r.active().is_none().then_some(()));
    assert!(!desk.remote.mirroring());
    let asked = desk.polls(&relay);
    let other = Device::account(&relay, DeviceKind::Phone, "Other");
    other.playing(&["s1"], 0);
    other.remote.clone().serve(true);
    other.until("serving", |_| (relay.asked().iter().filter(|a| a.ends_with(&other.remote.id())).count() >= 2).then_some(()));
    assert_eq!(desk.polls(&relay), asked, "the desk stopped polling with nothing to follow");
    relay.close();
}

#[test]
fn a_page_past_the_end_of_a_queue_that_shrank_is_empty() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2", "s3"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));

    desk.remote.clone().send(phone_id, Op::Page { from: 10, count: 50 });
    eventually(&relay, "an empty page", || {
        let hub = relay.hub.lock();
        hub.rooms.values().flat_map(|r| &r.events).any(|(_, _, _, b)| matches!(b, Body::Page { from: 10, entries, .. } if entries.is_empty()))
    });
    relay.close();
}

#[test]
fn a_mirrored_queue_is_cleared_and_a_removed_song_put_back_in_its_place() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2", "s3", "s4", "s5"], 1);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().pick(Some(phone_id.clone()));
    let m = desk.until("the phone mirrored", |r| r.active().filter(|m| m.rows.len() == 5));

    // The third song taken out and put back: it goes back where it was, not after the song playing.
    desk.remote.clone().send(phone_id.clone(), Op::Remove { index: 2, rev: m.rev });
    assert_eq!(phone.told(), Op::Remove { index: 2, rev: m.rev });
    phone.core.session.remove(2, 3);
    assert!(!desk.remote.clone().put_back(phone_id.clone(), "s4".into()), "only a song taken out from here");
    assert!(desk.remote.clone().put_back(phone_id.clone(), "s3".into()));
    match phone.told() {
        Op::Restore { song, index } => {
            assert_eq!((song.id.as_str(), song.title.as_str(), index), ("s3", "S3", 2));
            assert!(phone.core.session.restore(song.id).at.is_some(), "the phone's own undo knows its place");
        }
        op => panic!("{op:?}"),
    }
    assert_eq!(phone.core.session.playlist(|p| p.ids().to_vec()), ["s1", "s2", "s3", "s4", "s5"]);
    assert!(!desk.remote.clone().put_back(phone_id.clone(), "s3".into()), "put back once");

    // Clear: the phone's player removes what plays after the current song, from the end.
    desk.remote.clone().send(phone_id.clone(), Op::Clear);
    let removed: Vec<u32> = (0..3).map(|_| match phone.told() {
        Op::Remove { index, .. } => index,
        op => panic!("{op:?}"),
    }).collect();
    assert_eq!(removed, [4, 3, 2]);
    assert!({ relay.sim.pass(Duration::from_millis(300)); phone.ops.try_recv().is_err() }, "the song playing stays");
    relay.close();
}

#[test]
fn a_transfer_keeps_the_play_order_shuffle_and_repeat() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2", "s3", "s4", "s5"], 2);
    phone.core.session.shuffle(true);
    phone.core.session.repeat(2);
    phone.remote.clone().serve(true);
    desk.remote.clone().serve(true);
    let (phone_id, desk_id) = (phone.remote.id(), desk.remote.id());
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    // The phone follows only a device it lists.
    phone.until("the desk", |r| r.devices().into_iter().find(|d| d.id == desk_id).and_then(|d| d.state));
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active());

    // "This device": the phone hands its queue over as it plays, and follows the desk.
    desk.remote.clone().pick(None);
    let order: Vec<u32> = phone.core.session.playlist(|p| p.play_order().map(|i| i as u32).collect());
    match desk.told() {
        Op::Replace { songs, index, play, order: sent, shuffle, repeat, .. } => {
            assert_eq!(songs.len(), 5);
            assert_eq!((index, play, shuffle, repeat), (2, true, true, 2));
            assert_eq!(sent, Some(order));
        }
        op => panic!("{op:?}"),
    }
    assert_eq!(phone.told(), Op::Pause);
    desk.until("playing here", |r| r.active().is_none().then_some(()));
    let m = phone.until("the desk mirrored", |r| r.active());
    assert_eq!(m.id, desk_id);
    relay.close();
}

#[test]
fn controllers_follow_playback_to_where_it_was_handed() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let tablet = Device::account(&relay, DeviceKind::Phone, "Tablet");
    phone.playing(&["s1", "s2"], 0);
    for d in [&phone, &desk] {
        d.remote.clone().serve(true);
    }
    let (phone_id, desk_id) = (phone.remote.id(), desk.remote.id());
    tablet.remote.clone().watch(true);
    tablet.until("both", |r| (r.devices().iter().filter(|d| d.state.is_some()).count() == 2).then_some(()));
    // A device hears commands from its first answer on: the desk has had one once it lists the phone.
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    tablet.remote.clone().pick(Some(phone_id.clone()));
    tablet.until("the phone mirrored", |r| r.active().filter(|m| m.id == phone_id));

    // Moved on from the tablet: the phone hands over to the desk, and the tablet follows the desk.
    tablet.remote.clone().pick(Some(desk_id.clone()));
    assert_eq!(phone.told(), Op::Pause);
    assert!(matches!(desk.told(), Op::Replace { play: true, .. }));
    assert_eq!(tablet.until("the desk mirrored", |r| r.active()).id, desk_id);

    // The desk handing its playback on by itself is followed too.
    let elsewhere = Device::account(&relay, DeviceKind::Desktop, "Elsewhere");
    elsewhere.remote.clone().serve(true);
    let elsewhere_id = elsewhere.remote.id();
    desk.remote.clone().watch(true);
    desk.until("elsewhere", |r| r.devices().into_iter().find(|d| d.id == elsewhere_id && d.state.is_some()));
    desk.playing(&["s1", "s2"], 1);
    desk.remote.clone().hand_over(elsewhere_id.clone());
    assert_eq!(tablet.until("followed on", |r| r.active().filter(|m| m.id == elsewhere_id)).name, "Elsewhere");
    relay.close();
}

/// The device playback was handed to says it got the queue before its player has it (the player takes a
/// moment): the controller goes on showing the song it handed, never the device's song from before.
#[test]
fn a_handed_device_shows_the_handed_song_until_its_player_has_it() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["p1", "p2"], 1);
    desk.queued(&["d1", "d2"]);
    for d in [&phone, &desk] {
        d.remote.clone().serve(true);
    }
    let desk_id = desk.remote.id();
    phone.remote.clone().watch(true);
    phone.until("the desk", |r| r.devices().into_iter().find(|d| d.id == desk_id && d.state.is_some()));
    phone.remote.clone().watch(false);
    let song = |r: &Remote| r.active().and_then(|m| m.at.and_then(|a| m.rows.get(a as usize)).map(|row| row.song.id.clone()));

    phone.remote.clone().hand_over(desk_id.clone());
    assert_eq!(song(&phone.remote).as_deref(), Some("p2"));
    assert!(matches!(desk.told(), Op::Replace { .. }));
    // Its next state answers the command, its player still on the song from before.
    desk.remote.clone().volume_changed(Some(33));
    phone.until("the desk's answer", |r| r.devices().into_iter().find(|d| d.id == desk_id).and_then(|d| d.state).filter(|s| s.volume == Some(33)));
    assert_eq!(song(&phone.remote).as_deref(), Some("p2"), "still the handed song");

    desk.playing(&["p1", "p2"], 1);
    desk.remote.clone().volume_changed(Some(34));
    phone.until("the desk playing it", |r| r.active().filter(|m| m.volume == Some(34)));
    assert_eq!(song(&phone.remote).as_deref(), Some("p2"));
    relay.close();
}

/// Playback taken back here goes on showing the device it comes from until its queue is here, never
/// this device's own queue from before.
#[test]
fn playback_taken_back_shows_where_it_was_until_the_queue_is_here() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["p1", "p2"], 1);
    desk.queued(&["d1", "d2"]);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.state.is_some()));
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active().filter(|m| m.at.is_some()));

    desk.remote.clone().pick(None);
    let song = desk.remote.active().and_then(|m| m.at.and_then(|a| m.rows.get(a as usize)).map(|row| row.song.id.clone()));
    assert_eq!(song.as_deref(), Some("p2"), "the phone's song until its queue is here");
    assert!(matches!(desk.told(), Op::Replace { .. }));
    assert!(desk.remote.active().is_none(), "here once the queue is");
    relay.close();
}

/// A controller moving playback from one device to another shows the second playing what the first
/// handed it, never its song from before.
#[test]
fn a_device_picked_from_another_shows_what_it_is_handed() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let tablet = Device::account(&relay, DeviceKind::Phone, "Tablet");
    phone.playing(&["p1", "p2"], 1);
    desk.queued(&["d1", "d2"]);
    for d in [&phone, &desk] {
        d.remote.clone().serve(true);
    }
    let (phone_id, desk_id) = (phone.remote.id(), desk.remote.id());
    tablet.remote.clone().watch(true);
    tablet.until("both", |r| (r.devices().iter().filter(|d| d.state.is_some()).count() == 2).then_some(()));
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    tablet.remote.clone().pick(Some(phone_id.clone()));
    tablet.until("the phone mirrored", |r| r.active().filter(|m| m.at.is_some()));

    tablet.remote.clone().pick(Some(desk_id.clone()));
    let song = |r: &Remote| r.active().and_then(|m| m.at.and_then(|a| m.rows.get(a as usize)).map(|row| row.song.id.clone()));
    assert_eq!(song(&tablet.remote).as_deref(), Some("p2"));
    // The desk's word from before it has the queue.
    desk.remote.clone().volume_changed(Some(33));
    tablet.until("the desk's word", |r| r.devices().into_iter().find(|d| d.id == desk_id).and_then(|d| d.state).filter(|s| s.volume == Some(33)));
    assert_eq!(song(&tablet.remote).as_deref(), Some("p2"), "still what it is handed");
    relay.close();
}

/// Playback handed back to a device that has not played since: it no longer says it handed it on, so
/// a controller picking it stays there.
#[test]
fn a_device_handed_back_its_playback_is_followed_there() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let laptop = Device::account(&relay, DeviceKind::Desktop, "Laptop");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    for d in [&phone, &laptop] {
        d.remote.clone().serve(true);
    }
    let (phone_id, laptop_id) = (phone.remote.id(), laptop.remote.id());
    desk.remote.clone().watch(true);
    for (d, other) in [(&phone, &laptop_id), (&laptop, &phone_id)] {
        d.until("the other", |r| r.devices().into_iter().find(|d| d.id == *other && d.state.is_some()));
    }

    phone.remote.clone().hand_over(laptop_id.clone());
    assert_eq!(phone.told(), Op::Pause);
    assert!(matches!(laptop.told(), Op::Replace { .. }));
    laptop.queued(&["s1", "s2"]);
    laptop.remote.clone().hand_over(phone_id.clone());
    assert!(matches!(phone.told(), Op::Replace { play: false, .. }));
    phone.queued(&["s1", "s2"]);
    let state = |id: &str| desk.remote.devices().into_iter().find(|d| d.id == id).and_then(|d| d.state);
    desk.until("each said where it stands", |_| {
        (state(&laptop_id)?.handed_to.as_ref() == Some(&phone_id) && state(&phone_id)?.handed_to.is_none()).then_some(())
    });

    desk.remote.clone().pick(Some(phone_id.clone()));
    assert_eq!(desk.remote.active().map(|m| m.id), Some(phone_id));
    relay.close();
}

/// Two devices each last said they handed their playback to the other (one of them heard before it
/// was handed back): a controller follows the hand-off once and stays.
#[test]
fn hand_offs_in_a_circle_are_followed_once() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let laptop = Device::account(&relay, DeviceKind::Desktop, "Laptop");
    phone.playing(&["s1", "s2"], 0);
    laptop.playing(&["s1", "s2"], 0);
    for d in [&phone, &laptop] {
        d.remote.clone().serve(true);
    }
    let (phone_id, laptop_id) = (phone.remote.id(), laptop.remote.id());
    let published = |id: &str| relay.hub.lock().rooms.values().flat_map(|r| &r.members).find(|m| m.id == id && m.state.is_some()).cloned();
    let mut members = phone.until("both published", |_| Some([published(&phone_id)?, published(&laptop_id)?]));
    phone.remote.clone().stop();
    laptop.remote.clone().stop();
    for (m, to) in members.iter_mut().zip([&laptop_id, &phone_id]) {
        let state = m.state.as_mut().unwrap();
        state.playing = false;
        state.handed_to = Some(to.clone());
    }
    relay.hub.lock().rooms.entry("u:ann".into()).or_default().members.extend(members);

    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    desk.until("both", |r| (r.devices().iter().filter(|d| d.state.is_some()).count() == 2).then_some(()));
    desk.remote.clone().pick(Some(phone_id));
    assert_eq!(desk.remote.active().map(|m| m.id), Some(laptop_id));
    relay.close();
}

/// A state the device published before it carried out a command, arriving after the command was sent,
/// does not undo what the controller shows; the device's own word does, or time when it says nothing.
#[test]
fn a_command_shows_until_the_device_says_it_carried_it_out() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active().filter(|m| m.playing));
    // The phone's states reach the relay late.
    relay.lag(&phone_id, 0);

    phone.remote.clone().played(Playing { playing: true, position_ms: 8_000, index: None, volume: Some(40), ..Default::default() });
    desk.remote.clone().send(phone_id.clone(), Op::Pause);
    desk.remote.clone().send(phone_id.clone(), Op::Volume { percent: 10 });
    desk.until("the state from before", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.position_ms >= 8_000));
    let m = desk.remote.active().unwrap();
    assert_eq!((m.playing, m.volume), (false, Some(10)), "still as commanded");

    assert_eq!((phone.told(), phone.told()), (Op::Pause, Op::Volume { percent: 10 }));
    phone.remote.clone().played(Playing { playing: false, position_ms: 8_400, index: None, volume: Some(12), ..Default::default() });
    desk.until("the phone's word", |r| r.active().filter(|m| !m.playing && m.position_ms == 8_400 && m.volume == Some(12)));

    // A command the phone never carries out shows only for a while.
    desk.remote.clone().send(phone_id.clone(), Op::Play);
    assert!(desk.remote.active().unwrap().playing);
    assert_eq!(phone.told(), Op::Play);
    desk.until("the phone as it is", |r| r.active().filter(|m| !m.playing));
    relay.close();
}

/// The device says it carried a pause out in a state published before its player has paused (the player
/// takes a moment): the controller goes on showing the pause until the player's own word.
#[test]
fn a_pause_stays_shown_through_the_devices_word_that_it_got_it() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active().filter(|m| m.playing));

    desk.remote.clone().send(phone_id.clone(), Op::Pause);
    assert_eq!(phone.told(), Op::Pause);
    // The phone's next state carries the answer, its player still playing.
    phone.remote.clone().volume_changed(Some(41));
    let m = desk.until("the phone's answer", |r| r.active().filter(|m| m.volume == Some(41)));
    assert!(!m.playing, "the pause is still shown");
    phone.remote.clone().played(Playing { playing: false, position_ms: 5_400, index: None, volume: Some(41), ..Default::default() });
    desk.until("the phone paused", |r| r.active().filter(|m| !m.playing && m.position_ms == 5_400));
    relay.close();
}

/// Where the device's listener is at `now_us`, playing on from `position_ms` said at `said_us`.
fn heard_at(position_ms: i64, said_us: i64, now_us: i64) -> i64 {
    position_ms + (now_us - said_us) / 1000
}

#[test]
fn a_mirrored_playhead_is_where_the_device_is_heard() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    // The phone's clock reads seven seconds ahead of the desk's, and its word reaches the relay late.
    relay.lag(&phone.remote.id(), 7_000_000);
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active());

    let said = relay.sim.now_us();
    phone.remote.clone().played(Playing { playing: true, position_ms: 30_000, index: None, volume: Some(40), ..Default::default() });
    desk.until("the phone's new place", |r| r.active().filter(|m| m.position_ms >= 30_000));
    // The burst of time exchanges is over within three seconds.
    relay.sim.pass(Duration::from_secs(3));
    let m = desk.remote.active().expect("still mirrored");
    let now = relay.sim.now_us();
    let off = m.position_at(now) - heard_at(30_000, said, now);
    assert!(off.abs() <= 10, "the desk shows the phone {off} ms off what is heard there");
    relay.close();
}

#[test]
fn a_stopped_remote_lets_go_of_everything() {
    let relay = Relay::new();
    let announced = Arc::new(Announced::default());
    let ann = ServerConfig { url: SERVER.into(), user: "ann".into(), password: "pw".into(), ..Default::default() };
    let phone = Device::found(&relay, ann, DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the door announced");
    let phone_id = phone.remote.id();

    // The desk mirrors the phone through the relay and its door, learning its clock there.
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().lan_found(door.name.clone(), "127.0.0.1".into(), door.port, door.txt.clone());
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.nearby));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active());
    let desk_id = desk.remote.id();
    let asked = || relay.asked().into_iter().filter(|a| (a.starts_with("noriRemote.poll") && a.ends_with(&desk_id)) || (a.starts_with("lan ") && a.contains(&format!("dev={desk_id}")) && !a.contains("noriRemote.send"))).collect::<Vec<_>>();
    desk.until("a time exchange through the door", |_| asked().iter().any(|a| a.contains("noriRemote.time")).then_some(()));
    desk.until("a held poll at the door", |_| asked().iter().any(|a| a.contains("since=")).then_some(()));

    // Its held polls are given up at once, not waited out.
    stop(&desk);
    assert!(desk.remote.active().is_none(), "the player plays here again");
    assert_eq!(Arc::strong_count(&desk.remote), 1, "no thread of the remote runs");

    // News from the phone reaches another device, and nothing more is asked for the desk.
    let before = asked();
    let tablet = Device::account(&relay, DeviceKind::Phone, "Tablet");
    tablet.remote.clone().watch(true);
    phone.remote.clone().played(Playing { playing: false, position_ms: 20_000, index: None, volume: None, ..Default::default() });
    tablet.until("the phone's news", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.position_ms == 20_000));
    assert_eq!(asked(), before, "no poll or time exchange after the stop");
    relay.close();
}

#[test]
fn a_stopped_device_leaves_the_lists_at_once() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));

    // Its leaving reaches the relay late: stopping waits for it, as a client quits right after.
    relay.leave_late.store(true, Ordering::Relaxed);
    phone.remote.clone().stop();
    let listed = relay.hub.lock().rooms["u:ann"].members.iter().any(|m| m.id == phone_id);
    assert!(!listed, "the relay was told before the stop returned");
    desk.until("the phone gone", |r| r.devices().is_empty().then_some(()));
    relay.close();
}

#[test]
fn far_devices_are_hidden_while_the_relay_cannot_be_reached() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    let listed = |r: &Remote| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state);
    desk.until("the phone", listed);

    // What the relay last said is not shown once it no longer answers: the phone may have gone since.
    relay.go_down(true);
    desk.until("the phone hidden", |r| r.devices().is_empty().then_some(()));

    // Back as soon as the relay answers, without waiting for news.
    relay.go_down(false);
    desk.remote.clone().watch(false);
    desk.remote.clone().watch(true);
    desk.until("the phone again", listed);
    relay.close();
}

/// A mirrored device that leaves the lists is no longer shown as the one played on: the relay no longer
/// answers, or lists it without a state.
#[test]
fn a_mirrored_device_that_leaves_the_lists_is_no_longer_mirrored() {
    for relay_down in [true, false] {
        let relay = Relay::new();
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        relay.list("phone", Some(DeviceState { playing: true, index: Some(0), len: 1, ..Default::default() }));
        desk.remote.clone().watch(true);
        desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == "phone").and_then(|d| d.state));
        desk.remote.clone().pick(Some("phone".into()));
        desk.until("the phone mirrored", |r| r.active());

        if relay_down {
            relay.go_down(true);
        } else {
            relay.list("phone", None);
        }
        desk.until("the phone hidden", |r| r.devices().is_empty().then_some(()));
        desk.until(&format!("the phone no longer mirrored, relay down: {relay_down}"), |r| r.active().is_none().then_some(()));
        relay.close();
    }
}

#[derive(Clone, Copy, Debug)]
enum Did {
    /// Plays a queue of its own.
    Played,
    Paused,
    /// Has a queue it has not played.
    Queued,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Picked {
    /// The desk hands its queue over and pauses.
    Transfers,
    /// The desk only follows the phone.
    Mirrors,
}

impl Device {
    /// Has songs `ids` queued, paused at the first.
    fn queued(&self, ids: &[&str]) {
        self.core.session.register(ids.iter().map(|id| Song { id: id.to_string(), duration: 200, ..Default::default() }).collect());
        self.core.session.set(ids.iter().map(|s| s.to_string()).collect(), Some(0), false, None);
        self.remote.clone().played(Playing { playing: false, position_ms: 6_000, ..Default::default() });
    }
}

/// Picking the phone from the desk moves the active device's playback there: the desk's when the desk
/// is the active device, none when the phone is.
#[test]
fn picking_a_device_moves_the_active_ones_playback() {
    use Did::*;
    /// (on the desk, what it did), in order.
    type Steps = &'static [(bool, Did)];
    let cases: &[(&str, Steps, Picked)] = &[
        ("the phone plays", &[(true, Played), (true, Paused), (false, Played)], Picked::Mirrors),
        ("the desk plays", &[(false, Played), (false, Paused), (true, Played)], Picked::Transfers),
        ("the desk played last", &[(false, Played), (false, Paused), (true, Played), (true, Paused)], Picked::Transfers),
        ("the phone played last", &[(true, Played), (true, Paused), (false, Played), (false, Paused)], Picked::Mirrors),
        ("the phone is idle", &[(true, Played), (true, Paused)], Picked::Transfers),
        ("neither played", &[(true, Queued), (false, Queued)], Picked::Mirrors),
    ];
    for (case, steps, want) in cases {
        let relay = Relay::new();
        let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        phone.remote.clone().serve(true);
        desk.remote.clone().serve(true);
        let phone_id = phone.remote.id();
        let heard = |what: &str, ok: &dyn Fn(&nori_remote::wire::DeviceState) -> bool| {
            desk.until(&format!("{case}: {what}"), |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| ok(s)));
        };
        for &(on_desk, did) in steps.iter() {
            let (d, ids) = if on_desk { (&desk, ["d1", "d2"]) } else { (&phone, ["p1", "p2"]) };
            match did {
                Played => d.playing(&ids, 0),
                Paused => d.remote.clone().played(Playing { playing: false, position_ms: 6_000, ..Default::default() }),
                Queued => d.queued(&ids),
            }
            // A moment between a person's steps, so which came last is told apart.
            relay.sim.pass(Duration::from_secs(1));
            if !on_desk {
                // The desk hears each of the phone's states.
                let playing = matches!(did, Played);
                heard("the phone's state", &|s| s.playing == playing && s.index.is_some());
            }
        }
        heard("the phone", &|_| true);
        desk.remote.clone().pick(Some(phone_id.clone()));
        let picked = if desk.ops.try_recv() == Ok(Op::Pause) { Picked::Transfers } else { Picked::Mirrors };
        assert_eq!(picked, *want, "{case}");
        assert_eq!(desk.until(case, |r| r.active()).id, phone_id, "{case}: followed");
        match want {
            Picked::Transfers => {
                assert!(matches!(phone.told(), Op::Replace { songs, .. } if songs[0].id == "d1"), "{case}: the desk's queue goes");
                phone.queued(&["d1", "d2"]);
                // Waited for in the mirror the pick goes by, not in the device list read before it.
                desk.until(&format!("{case}: the queue there"), |r| r.active().filter(|m| m.at.is_some() && m.rows.first().is_some_and(|e| e.song.id == "d1")));
                // Picked back, the queue comes here again.
                desk.remote.clone().pick(None);
                assert!(matches!(desk.told(), Op::Replace { songs, .. } if songs[0].id == "d1"), "{case}: pulled back");
            }
            Picked::Mirrors => assert!({ relay.sim.pass(Duration::from_millis(300)); phone.ops.try_recv().is_err() }, "{case}: the phone's queue stays"),
        }
        relay.close();
    }
}

/// A relay that tells its time: a jam's host and its guests each learn its clock, and a guest plays the
/// host's place by it, however late the host's words and answers reach it (a guest's own exchanges with
/// the host come out as lopsided as those delays).
#[test]
fn jam_guests_listen_along_by_the_relays_clock() {
    let relay = Relay::new();
    relay.time.store(true, Ordering::Relaxed);
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    // The host's clock reads four seconds behind the guest's; its words reach the relay late, and its
    // answers to the guest's time exchanges all a fifth of a second late.
    relay.lag(&host.remote.id(), -4_000_000);
    relay.answers_late.store(200, Ordering::Relaxed);
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    host.remote.clone().jam_along(true);
    let pass = joined(&relay, link, "Gus".into());
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    let leads = Arc::new(Leads::default());
    gus.remote.follow_with(Some(leads.clone()));
    gus.remote.clone().listen(true);

    let said = relay.sim.now_us();
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, rate: 1.25, index: Some(0), volume: None, ..Default::default() });
    gus.until("the host's place", |_| leads.last().flatten().filter(|l| l.ms >= 30_000.0));
    // Both bursts of time exchanges are over within three seconds.
    relay.sim.pass(Duration::from_secs(3));
    let l = leads.last().flatten().expect("a lead");
    let now = relay.sim.now_us();
    let off = l.ms + (now - l.at_us) as f64 / 1000.0 * l.rate - (30_000.0 + (now - said) as f64 / 1000.0 * 1.25);
    assert!(off.abs() <= 5.0, "the guest plays {off:.1} ms off the host: {l:?}, said {said}, now {now}");
    assert!(relay.asked().iter().filter(|a| *a == "nori/time").count() >= 2 * nori_remote::clock::BURST, "each learned the relay's clock");
    relay.close();
}

#[test]
fn a_guest_starts_with_the_settled_clock() {
    let relay = Relay::new();
    relay.time.store(true, Ordering::Relaxed);
    let host = Device::account(&relay, DeviceKind::Desktop, "Host");
    host.playing(&["s1"], 0);
    let link = opened(&host);
    host.remote.clone().jam_along(true);
    host.until("the host's clock burst", |_| (relay.asked().iter().filter(|a| *a == "nori/time").count() >= nori_remote::clock::BURST).then_some(()));
    let said = relay.sim.now_us();
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, rate: 1.0, index: Some(0), ..Default::default() });
    let pass = joined(&relay, link, "Phone".into());
    let guest = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Phone");
    let leads = Arc::new(Leads::default());
    guest.remote.follow_with(Some(leads.clone()));
    relay.first_time_delay_ms.store(160, Ordering::Relaxed);
    guest.remote.clone().listen(true);
    guest.until("the first playback lead", |_| leads.0.lock().iter().flatten().next().cloned());
    let first = leads.0.lock().iter().flatten().next().cloned().expect("the first lead");
    let off = first.ms + (said - first.at_us) as f64 / 1000.0 - 30_000.0;
    assert!(off.abs() < 5.0, "the first lead starts {off:.1} ms off the host");
    relay.close();
}

/// A guest listening along before its host says where it plays waits for the host's word, not on a timer.
#[test]
fn a_guest_waits_for_its_hosts_clock_without_waking() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    let link = opened(&host);
    host.remote.clone().jam_along(true);
    host.playing(&["s1"], 0);
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, rate: 1.0, index: Some(0), volume: None, ..Default::default() });
    let pass = joined(&relay, link, "Gus".into());
    // The guest's first poll goes unanswered: it does not know its host yet.
    relay.first_answers_held.store(true, Ordering::Relaxed);
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    let leads = Arc::new(Leads::default());
    gus.remote.follow_with(Some(leads.clone()));
    gus.remote.clone().listen(true);
    relay.sim.settle();
    assert!(relay.sim.waits().iter().all(|w| *w > Duration::from_secs(1)), "timers wait {:?}", relay.sim.waits());

    relay.release_first_answers();
    gus.until("the host's place", |_| leads.last().flatten());
    relay.close();
}

/// The relay still lists a device from before it restarted, by its own id: it neither lists nor follows
/// itself.
#[test]
fn a_device_never_follows_itself() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let me = phone.remote.id();
    let before = nori_remote::wire::DeviceState { playing: true, index: Some(0), ..Default::default() };
    relay.hub.lock().rooms.entry("u:ann".into()).or_default().members.push(Member { id: me.clone(), name: "Phone".into(), kind: DeviceKind::Phone, state: Some(before) });
    desk.remote.clone().serve(true);
    phone.remote.clone().serve(true);
    phone.remote.clone().watch(true);
    phone.until("the desk", |r| r.devices().into_iter().find(|d| d.name == "Desk"));
    assert!(phone.remote.devices().iter().all(|d| d.id != me), "not listed");
    phone.remote.clone().pick(Some(me));
    assert!(phone.remote.active().is_none(), "not followed");
    relay.close();
}

/// The leads a guest's player was given, newest last.
#[derive(Default)]
struct Leads(Mutex<Vec<Option<Lead>>>);

impl Follower for Leads {
    fn lead(&self, lead: Option<Lead>) {
        self.0.lock().push(lead);
    }
}

impl Leads {
    fn last(&self) -> Option<Option<Lead>> {
        self.0.lock().last().cloned()
    }
}

#[test]
fn jam_guests_listen_along_where_the_host_is_heard() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    // The host's clock reads four seconds behind the guests'. (Its clock learned through late words is
    // nori-remote's clock.rs and the engine's along.rs.)
    relay.skew(&host.remote.id(), -4_000_000);
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    host.remote.clone().jam_along(true);
    let guest_of = |name: &str| {
        let pass = joined(&relay, link.clone(), name.into());
        let d = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, name);
        let leads = Arc::new(Leads::default());
        d.remote.follow_with(Some(leads.clone()));
        d.remote.clone().listen(true);
        (d, leads)
    };
    let (gus, gus_leads) = guest_of("Gus");
    let (dee, dee_leads) = guest_of("Dee");

    let said = relay.sim.now_us();
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, rate: 1.25, index: Some(0), volume: None, ..Default::default() });
    for (g, leads) in [(&gus, &gus_leads), (&dee, &dee_leads)] {
        g.until("the host's place", |_| leads.last().flatten().filter(|l| l.ms >= 30_000.0));
        assert_eq!(g.remote.jam_view().map(|v| v.listening), Some(Listening::Playing));
    }
    // The burst of time exchanges is over within three seconds.
    relay.sim.pass(Duration::from_secs(3));
    for leads in [&gus_leads, &dee_leads] {
        let l = leads.last().flatten().expect("a lead");
        assert_eq!((l.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), l.index, l.rate, l.playing), (vec!["s1", "s2"], 0, 1.25, true));
        let now = relay.sim.now_us();
        let off = l.ms + (now - l.at_us) as f64 / 1000.0 * l.rate - (30_000.0 + (now - said) as f64 / 1000.0 * 1.25);
        assert!(off.abs() <= 10.0, "the guest plays {off:.1} ms off the host");
    }
    // A word reaching them a quarter second late: their pages (the seek bar, the lyrics) show the place
    // heard, not where it was as the word arrived.
    relay.lag(&host.remote.id(), -4_000_000);
    let said = relay.sim.now_us();
    host.remote.clone().played(Playing { playing: true, position_ms: 40_000, rate: 1.25, index: Some(0), volume: None, ..Default::default() });
    for g in [&gus, &dee] {
        let m = g.until("the host's new place", |r| r.jam_playing().filter(|m| m.position_ms >= 40_000));
        let now = relay.sim.now_us();
        let off = m.position_at(now) as f64 - (40_000.0 + (now - said) as f64 / 1000.0 * 1.25);
        assert!(off.abs() <= 10.0, "the guest shows the host's place {off:.1} ms off");
    }
    relay.skew(&host.remote.id(), -4_000_000);

    // The host stops letting its guests listen along: they stop. The view and the follower are told
    // one after the other, so the wait is for both.
    host.remote.clone().jam_along(false);
    for (g, leads) in [(&gus, &gus_leads), (&dee, &dee_leads)] {
        g.until("the host's no, nothing to follow", |r| r.jam_view().filter(|v| v.listening == Listening::HostOff && leads.last() == Some(None)));
    }
    host.remote.clone().jam_along(true);
    gus.until("listening again", |r| r.jam_view().filter(|v| v.listening == Listening::Playing));

    // A server that does not let guests stream: unavailable, the jam itself still there.
    relay.along.store(false, Ordering::Relaxed);
    host.remote.clone().played(Playing { playing: false, position_ms: 31_000, rate: 1.25, index: Some(0), volume: None, ..Default::default() });
    let view = gus.until("the server's no, nothing to follow", |r| r.jam_view().filter(|v| v.listening == Listening::ServerOff && gus_leads.last() == Some(None)));
    assert_eq!(view.members.len(), 3);
    gus.remote.clone().listen(false);
    assert_eq!(gus.remote.jam_view().map(|v| v.listening), Some(Listening::Watching));
    relay.close();
}

#[test]
fn a_guest_leaving_its_jam_stops_playing_along_at_once() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    host.remote.clone().jam_along(true);
    let pass = joined(&relay, link, "Gus".into());
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    let leads = Arc::new(Leads::default());
    gus.remote.follow_with(Some(leads.clone()));
    gus.remote.clone().listen(true);
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, rate: 1.0, index: Some(0), volume: None, ..Default::default() });
    gus.until("the host's place", |_| leads.last().flatten());

    gus.remote.clone().jam_leave();
    assert_eq!(leads.last(), Some(None), "nothing to follow once left");
    let given = leads.0.lock().len();
    host.remote.clone().played(Playing { playing: true, position_ms: 60_000, rate: 1.0, index: Some(1), volume: None, ..Default::default() });
    gus.remote.clone().stop();
    assert_eq!(leads.0.lock().len(), given, "the host is followed no more");
    relay.close();
}

/// Its own invite, to the jam open or one it ended (also after the app started again), changes nothing
/// and asks nothing; anyone's ended jam is told apart from a failure by the relay's answer.
#[test]
fn a_device_knows_its_own_jams_invites_and_ended_ones() {
    let dir = nori_testdir::TempDir::new("remote-invites");
    let relay = Relay::new();
    let host = Device::kept(&relay, &dir, DeviceKind::Phone, "Host");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let join = |d: &Device, link: &str| relay.sim.run("a request", jam_join(relay.clone(), d.core.session.settings.clone(), Some(d.remote.clone()), link.into(), "Me".into())).unwrap();
    let link = opened(&host);
    let app_link = link.replacen("http://octo:5274/nori/jam#", "nori://jam?", 1);
    assert_eq!((join(&host, &link), join(&host, &app_link)), (JamJoin::Own, JamJoin::Own));

    host.remote.clone().jam_close();
    assert_eq!(join(&host, &link), JamJoin::Ended);
    host.remote.clone().stop();
    let again = Device::kept(&relay, &dir, DeviceKind::Phone, "Host");
    assert_eq!(join(&again, &link), JamJoin::Ended, "after the app started again");
    assert_eq!(relay.asked_for("noriRemote.join"), 0, "the relay was not asked");
    assert!(again.remote.jam_view().is_none());

    let desks = opened(&desk);
    assert!(matches!(join(&again, &desks), JamJoin::Joined { .. }), "another device of the account may join");
    desk.remote.clone().jam_close();
    eventually(&relay, "the desk's jam closed", || relay.jams() == 0);
    assert_eq!(join(&again, &desks), JamJoin::Ended);
    relay.close();
}

/// A jam whose close did not reach the relay is still listed in this device's polls: it is closed again,
/// never shown as a jam this device is in.
#[test]
fn a_jam_ended_while_the_relay_missed_it_is_closed_again() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    host.playing(&["s1"], 0);
    host.remote.clone().watch(true);
    opened(&host);
    relay.lose_closes.store(true, Ordering::Relaxed);
    host.remote.clone().jam_close();
    eventually(&relay, "the close lost", || relay.asked().iter().any(|a| a == "lost close"));
    relay.lose_closes.store(false, Ordering::Relaxed);

    // News in the account's room: the host's held poll answers, the jam still listed in it.
    desk.remote.clone().serve(true);
    host.until("the desk", |r| r.devices().into_iter().find(|d| d.name == "Desk"));
    assert!(host.remote.jam_view().is_none(), "no jam shown");
    eventually(&relay, "the jam closed at the relay", || relay.jams() == 0);
    relay.close();
}

/// Start pressed again and again while the relay is slow to open: one jam is asked for, and once it is
/// ended none comes back.
#[test]
fn a_jam_is_started_once_and_stays_ended() {
    let dir = nori_testdir::TempDir::new("remote-starts");
    let relay = Relay::new();
    let host = Device::kept(&relay, &dir, DeviceKind::Phone, "Host");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    host.playing(&["s1"], 0);
    host.remote.clone().watch(true);
    eventually(&relay, "the held poll", || !relay.waiting.lock().is_empty());
    relay.opens_late.store(true, Ordering::Relaxed);
    let starts: Vec<_> = (0..5).map(|_| spawned(&relay.sim, host.remote.clone().jam_open())).collect();
    eventually(&relay, "the start asked", || relay.asked().iter().any(|a| a == "open waits"));
    assert_eq!(host.remote.jam_start(), JamStart::Starting);
    eventually(&relay, "every start answered", || starts.iter().all(|s| s.lock().is_some()));
    let links: Vec<_> = starts.into_iter().map(|s| s.lock().take().unwrap().unwrap()).collect();
    assert_eq!(links.iter().flatten().count(), 1, "{links:?}");
    assert_eq!(relay.asked_for("noriRemote.open"), 1);
    assert_eq!(host.remote.jam_start(), JamStart::Hosting);

    host.remote.clone().jam_close();
    desk.remote.clone().serve(true);
    host.until("the desk", |r| r.devices().into_iter().find(|d| d.name == "Desk"));
    eventually(&relay, "no jam at the relay", || relay.jams() == 0);
    assert!(host.remote.jam_view().is_none());
    assert_eq!(host.remote.jam_start(), JamStart::Offered);
    relay.close();
}

/// Ended while the relay was still opening it: the jam it opens is closed again, and none is hosted.
#[test]
fn ending_a_jam_being_started_cancels_it() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    relay.opens_late.store(true, Ordering::Relaxed);
    let start = spawned(&relay.sim, host.remote.clone().jam_open());
    eventually(&relay, "the start asked", || relay.asked().iter().any(|a| a == "open waits"));
    host.remote.clone().jam_close();
    eventually(&relay, "the start answered", || start.lock().is_some());
    assert_eq!(start.lock().take().unwrap().unwrap(), None);
    assert!(host.remote.jam_view().is_none());
    eventually(&relay, "its jam closed at the relay", || relay.jams() == 0);
    relay.close();
}

/// A jam's host shows in it once it hears commands: a guest's time exchanges sent before would be lost.
#[test]
fn a_jam_host_shows_once_it_hears_its_guests() {
    let relay = Relay::new();
    relay.first_answers_held.store(true, Ordering::Relaxed);
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1"], 0);
    opened(&host);
    let shown = || relay.hub.lock().rooms.values().filter(|r| r.jam).flat_map(|r| &r.members).any(|m| m.id == host.remote.id() && m.state.is_some());
    // Sent after the jam's first state would have been, the same way.
    host.remote.clone().send("nobody".into(), Op::Play);
    eventually(&relay, "the command at the relay", || relay.hub.lock().rooms.values().flat_map(|r| &r.events).any(|e| e.2.as_deref() == Some("nobody")));
    assert!(!shown(), "shown before its first answer");
    relay.release_first_answers();
    eventually(&relay, "the host shown", shown);
    relay.close();
}

/// A guest of `host`'s jam `link`, listening along: its leads.
fn listening_guest(relay: &Arc<Relay>, host: &Device, link: String) -> (Device, Arc<Leads>) {
    host.remote.clone().jam_along(true);
    let pass = joined(relay, link, "Gus".into());
    let gus = Device::new(relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    let leads = Arc::new(Leads::default());
    gus.remote.follow_with(Some(leads.clone()));
    gus.remote.clone().listen(true);
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, rate: 1.0, index: Some(0), volume: None, ..Default::default() });
    gus.until("the host's place", |_| leads.last().flatten());
    (gus, leads)
}

#[test]
fn a_guest_follows_its_host_resuming_after_a_pause() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    let (gus, leads) = listening_guest(&relay, &host, link);
    host.remote.clone().played(Playing { playing: false, position_ms: 31_000, rate: 1.0, index: Some(0), volume: None, ..Default::default() });
    gus.until("the host's pause", |_| leads.last().flatten().filter(|l| !l.playing));
    host.remote.clone().played(Playing { playing: true, position_ms: 31_000, rate: 1.0, index: Some(0), volume: None, ..Default::default() });
    let l = gus.until("the host playing again", |_| leads.last().flatten().filter(|l| l.playing)).clone();
    assert!((l.ms - 31_000.0).abs() < 50.0, "the guest starts where the host resumed, not {}", l.ms);
    relay.close();
}

#[test]
fn a_guest_hears_its_jam_end_and_stops_playing_along() {
    // Ended by the host, this guest sent out by it, or forgotten by the relay (its key no longer signs in).
    for how in ["closed", "kicked", "restarted"] {
        let relay = Relay::new();
        let host = Device::account(&relay, DeviceKind::Phone, "Host");
        host.playing(&["s1", "s2"], 0);
        let link = opened(&host);
        let (gus, leads) = listening_guest(&relay, &host, link);
        match how {
            "closed" => host.remote.clone().jam_close(),
            "kicked" => {
                let you = gus.until("its place in the jam", |r| r.jam_view()).you;
                host.until("its guest", |r| r.jam_view().filter(|v| v.members.iter().any(|m| m.id == you)));
                host.remote.clone().jam_act(Op::Kick { member: you });
            }
            _ => relay.restart(),
        }
        gus.until(&format!("the end, {how}"), |_| (!gus.ended.lock().is_empty()).then_some(()));
        assert_eq!(*gus.ended.lock(), [Some("Host".to_string())], "{how}");
        assert_eq!(leads.last(), Some(None), "nothing to follow");
        assert!(gus.remote.jam_view().is_none());
        gus.remote.clone().stop();
        assert_eq!(gus.ended.lock().len(), 1, "told once");
        assert!(host.ended.lock().is_empty(), "the host was in no one's jam");
        relay.close();
    }
}

#[test]
fn a_guest_profile_opened_after_its_jam_ended_hears_so_at_once() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    let link = opened(&host);
    let pass = joined(&relay, link, "Gus".into());
    host.remote.clone().jam_close();
    eventually(&relay, "the jam closed at the relay", || relay.jams() == 0);
    // The app opens again on the guest profile: its first look at the jam finds it gone.
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    gus.remote.clone().watch(true);
    gus.until("the end", |_| (!gus.ended.lock().is_empty()).then_some(()));
    assert_eq!(*gus.ended.lock(), [None], "no host was ever seen");
    relay.close();
}

#[test]
fn a_guest_leaves_at_once_with_the_relay_down() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    let (gus, leads) = listening_guest(&relay, &host, link);
    relay.go_down(true);
    gus.remote.clone().jam_leave();
    assert_eq!(leads.last(), Some(None), "its music stops here at once");
    gus.remote.clone().stop();
    assert!(gus.ended.lock().is_empty(), "left, not ended by the host");
    relay.close();
}

/// As in Spotify's Jam: an admin's player controls reach the host's playback, every listener's; a plain
/// guest's pause holds only its own listening (the platform's to carry out), and it skips nothing.
#[test]
fn jam_controls_reach_by_role() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.playing(&["s1", "s2"], 0);
    let link = opened(&host);
    host.remote.clone().jam_along(true);
    let guest_of = |name: &str| {
        let pass = joined(&relay, link.clone(), name.into());
        let d = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, name);
        d.remote.follow_with(Some(Arc::new(Leads::default())));
        d.remote.clone().listen(true);
        d.remote.clone().played(Playing { playing: true, position_ms: 30_000, index: Some(0), ..Default::default() });
        d
    };
    let gus = guest_of("Gus");
    let dee = guest_of("Dee");
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, index: Some(0), ..Default::default() });
    let dee_id = dee.until("the jam", |r| r.jam_view().filter(|v| v.members.len() == 3)).you;
    host.remote.clone().jam_act(Op::Promote { member: dee_id, admin: true });
    let admin = dee.until("Dee's role", |r| r.jam_controls().filter(|c| c.controls.skip == Reach::Jam));
    assert_eq!(admin, JamControls { controls: Controls::of(Role::Admin, true), asks: false, playing: true, paused_here: false });

    // The guest's own pause: carried out here, nothing sent; the jam plays on.
    gus.until("Gus's controls", |r| r.jam_controls().filter(|c| c.playing));
    assert_eq!(gus.remote.clone().jam_press(Op::Pause), Reach::Here);
    gus.remote.clone().played(Playing { playing: false, position_ms: 31_000, index: Some(0), ..Default::default() });
    let paused = gus.until("paused here", |r| r.jam_controls().filter(|c| c.paused_here));
    assert_eq!(paused, JamControls { controls: Controls::of(Role::Guest, true), asks: true, playing: false, paused_here: true });
    for op in [Op::Next, Op::Seek { ms: 1_000 }, Op::Move { from: 0, to: 1, rev: 0 }] {
        assert_eq!(gus.remote.clone().jam_press(op), Reach::Nowhere);
    }
    assert_eq!(gus.remote.clone().jam_press(Op::Play), Reach::Here, "play joins the jam again");
    // A guest's control sent all the same (an older client) is refused.
    gus.remote.clone().jam_act(Op::Next);
    assert_eq!(gus.until("the refusal", |r| r.jam_view().and_then(|v| v.refused)), Refusal::NotAllowed);
    assert!(host.ops.try_recv().is_err(), "nothing from the plain guest");

    // The admin's pause and skip are the host's.
    assert_eq!(dee.remote.clone().jam_press(Op::Pause), Reach::Jam);
    assert_eq!(host.told(), Op::Pause);
    assert_eq!(dee.remote.clone().jam_press(Op::Next), Reach::Jam);
    assert_eq!(host.told(), Op::Next);
    let blue = Song {
        id: "s5".into(),
        title: "Blue".into(),
        ..Default::default()
    };
    let replace = Op::Replace {
        songs: vec![blue.clone()],
        index: 0,
        position_ms: 0,
        play: true,
        order: None,
        shuffle: false,
        repeat: 0,
        transfer: None, listen: None, mix: None,
    };
    assert_eq!(dee.remote.clone().jam_press(replace.clone()), Reach::Jam);
    assert_eq!(
        host.told(),
        replace,
        "an admin's tap plays rather than requesting a queue addition"
    );
    host.core.session.register(vec![blue]);
    host.core
        .session
        .set(vec!["s5".into()], Some(0), false, None);
    host.remote.clone().played(Playing {
        playing: true,
        index: Some(0),
        ..Default::default()
    });
    dee.until("the admin's new queue", |r| {
        r.jam_playing()
            .filter(|m| m.rows.len() == 1 && m.rows[0].song.id == "s5")
    });
    let song = Song {
        id: "s4".into(),
        title: "Four".into(),
        ..Default::default()
    };
    let add = Op::Add {
        songs: vec![song.clone()],
        next: false,
    };
    assert_eq!(dee.remote.clone().jam_press(add.clone()), Reach::Jam);
    assert_eq!(host.told(), add);
    host.core.session.register(vec![song]);
    host.core
        .session
        .take(1, vec!["s4".into()], vec![Hand::Last]);
    host.remote.clone().played(Playing {
        playing: true,
        index: Some(0),
        ..Default::default()
    });
    dee.until("the admin's queue addition", |r| {
        r.jam_playing()
            .filter(|m| m.rows.iter().any(|row| row.song.id == "s4"))
    });
    assert!(dee.remote.jam_view().unwrap().pending.is_empty());
    assert_eq!(
        dee.remote.jam_added().get("s4").map(String::as_str),
        Some("Dee")
    );
    relay.close();
}

/// A jam plays here and nowhere else: starting one brings the playback back to this device, and while it
/// runs, the playback cannot be moved to another device.
#[test]
fn a_jam_and_playing_on_another_device_exclude_each_other() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active().filter(|m| m.id == phone_id));

    relay.sim.run("a request", desk.remote.clone().jam_open()).unwrap();
    assert_eq!(phone.told(), Op::Pause);
    assert!(matches!(desk.told(), Op::Replace { play: true, .. }));
    desk.until("playing here", |r| r.active().is_none().then_some(()));

    desk.remote.clone().pick(Some(phone_id.clone()));
    assert!(desk.remote.active().is_none(), "no moving the playback while the jam runs");
    desk.remote.clone().jam_close();
    desk.remote.clone().pick(Some(phone_id.clone()));
    assert!(desk.remote.active().is_some(), "free to move it again once the jam is over");
    relay.close();
}

/// Settles, then checks a timer with `left` to go waits.
fn waiting(sim: &Native, what: &str, left: Duration) {
    sim.settle();
    assert!(sim.waits().contains(&left), "{what}: timers wait {:?}", sim.waits());
}

/// Stops `d`'s remote, which must not wait for time.
fn stop(d: &Device) {
    let at = d.sim.now_us();
    d.remote.clone().stop();
    assert_eq!(d.sim.now_us(), at, "stopping waited, timers {:?}", d.sim.waits());
}

/// `f` started on the test's executor; its output once it ends.
fn spawned<T: Send + 'static>(sim: &Native, f: impl Future<Output = T> + Send + 'static) -> Arc<Mutex<Option<T>>> {
    let out = Arc::new(Mutex::new(None));
    let to = out.clone();
    sim.spawn(Box::pin(async move { *to.lock() = Some(f.await) }));
    out
}

impl Relay {
    /// Whether device `id` is listed with a state.
    fn lists(&self, id: &str) -> bool {
        self.hub.lock().rooms.values().flat_map(|r| &r.members).any(|m| m.id == id && m.state.is_some())
    }
}

#[test]
fn a_failed_poll_waits_longer_each_time_until_the_network_is_back() {
    let relay = Relay::new();
    relay.first_answers_held.store(true, Ordering::Relaxed);
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let sim = &relay.sim;
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let id = phone.remote.id();
    // Its first poll fails once everything here is said: nothing changes while it rests.
    eventually(&relay, "the first poll at the relay", || relay.asked_for("noriRemote.poll") == 1);
    relay.go_down(true);
    waiting(sim, "the retry", Duration::from_secs(30));
    relay.go_down(false);
    relay.release_first_answers();
    sim.pass(Duration::from_secs(29));
    assert_eq!(sim.waits(), [Duration::from_secs(1)], "only the retry waits");
    assert_eq!(relay.asked_for("noriRemote.poll"), 1, "not asked again before its time");
    sim.pass(Duration::from_secs(1));
    eventually(&relay, "listed once asked again", || relay.lists(&id));

    // Down for good: each failed poll waits twice as long as the last, up to 5 min, and a change here
    // does not ask early.
    relay.go_down(true);
    for secs in [30, 60, 120, 240, 300, 300] {
        waiting(sim, "the retry", Duration::from_secs(secs));
        phone.remote.clone().watch(true);
        waiting(sim, "the same retry after a change", Duration::from_secs(secs));
        sim.pass(Duration::from_secs(secs));
    }
    phone.remote.network_back();
    waiting(sim, "the first wait again once the network is back", Duration::from_secs(30));
    relay.go_down(false);
    phone.remote.network_back();
    eventually(&relay, "listed again", || relay.lists(&id));

    // Down again after an answer: the wait starts over, and the network coming back asks at once.
    relay.go_down(true);
    waiting(sim, "the retry after that", Duration::from_secs(30));
    relay.go_down(false);
    let polls = relay.asked_for("noriRemote.poll");
    phone.remote.network_back();
    eventually(&relay, "asked again once the network is back", || relay.asked_for("noriRemote.poll") > polls);
    relay.close();
}

#[test]
fn leaving_gives_up_the_held_poll_and_leaves_nothing_waiting() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let sim = &relay.sim;
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let id = phone.remote.id();
    eventually(&relay, "the held poll", || relay.parked(&id) && relay.lists(&id));
    phone.remote.clone().serve(false);
    eventually(&relay, "left, its poll no longer held", || !relay.parked(&id) && !relay.lists(&id));
    assert!(sim.waits().is_empty(), "no timer waits: {:?}", sim.waits());

    // A jam guest's leave goes, and stopping gives up the rest at once.
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    let pass = joined(&relay, opened(&host), "Bo".into());
    let guest = Device::with(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Bo", None, Default::default());
    guest.remote.clone().watch(true);
    guest.until("the jam", |r| r.jam_view());
    guest.remote.clone().jam_leave();
    stop(&guest);
    assert_eq!(relay.asked_for("noriRemote.leave"), 1);
    assert!(sim.waits().is_empty(), "no timer waits: {:?}", sim.waits());
    assert_eq!(Arc::strong_count(&guest.remote), 1, "no task of the remote runs");
    phone.remote.clone().stop();
    assert_eq!(Arc::strong_count(&phone.remote), 1, "no task of the remote runs");
    relay.close();
}

#[test]
fn sends_reach_the_relay_in_the_order_made() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    desk.remote.clone().watch(true);
    let phone_id = phone.remote.id();
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    // Its sends take from none to 170 ms each on the way.
    relay.lag(&desk.remote.id(), 0);
    for ms in 1..=12 {
        desk.remote.clone().send(phone_id.clone(), Op::Seek { ms: ms * 1_000 });
    }
    let told: Vec<Op> = (0..12).map(|_| phone.told()).collect();
    assert_eq!(told, (1..=12).map(|ms| Op::Seek { ms: ms * 1_000 }).collect::<Vec<_>>());
    relay.close();
}

#[test]
fn a_mirrored_clock_is_learned_in_a_burst_then_every_quarter_minute() {
    let relay = Relay::new();
    let sim = &relay.sim;
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored, playing", |r| r.active().filter(|m| m.playing));
    eventually(&relay, "the first exchange", || relay.clocks() == 1);
    let gap = Duration::from_millis(250);
    for sent in 2..=clock::BURST {
        waiting(sim, "the burst's gap", gap);
        sim.pass(gap);
        eventually(&relay, "the burst's next exchange", || relay.clocks() == sent);
    }
    let every = Duration::from_micros(clock::EVERY_US as u64);
    waiting(sim, "the quarter minute", every);
    sim.pass(every - gap);
    assert_eq!(relay.clocks(), clock::BURST, "none before the quarter minute");
    sim.pass(gap);
    eventually(&relay, "the next exchange", || relay.clocks() == clock::BURST + 1);
    relay.close();
}

/// Playback moved from the phone to the desk, and pulled from the phone by the desk, through the relay
/// and through the doors: the desk is told the queue with no timer run out (the clock never moves).
#[test]
fn a_hand_over_reaches_the_device_without_waiting() {
    for (case, nearby, pull) in [("handed, relay", false, false), ("pulled, relay", false, true), ("handed, door", true, false), ("pulled, door", true, true)] {
        let relay = if nearby { Relay::absent() } else { Relay::new() };
        let made = |kind, name: &str| {
            let announced = Arc::new(Announced::default());
            let d = Device::with(&relay, ann(), kind, name, nearby.then(|| announced.clone() as Arc<dyn Discovery>), Default::default());
            (d, announced)
        };
        let ((phone, phone_door), (desk, desk_door)) = (made(DeviceKind::Phone, "Phone"), made(DeviceKind::Desktop, "Desk"));
        phone.playing(&["s1", "s2"], 1);
        desk.queued(&["d1"]);
        phone.remote.clone().serve(true);
        desk.remote.clone().serve(true);
        phone.remote.clone().watch(true);
        desk.remote.clone().watch(true);
        if nearby {
            for (d, door) in [(&desk, &phone_door), (&phone, &desk_door)] {
                let door = door.0.lock().clone().expect("a door");
                d.remote.clone().lan_found(door.name, "127.0.0.1".into(), door.port, door.txt);
            }
        }
        let (phone_id, desk_id) = (phone.remote.id(), desk.remote.id());
        desk.until(case, |r| r.devices().into_iter().find(|d| d.id == phone_id && d.state.as_ref().is_some_and(|s| s.playing)));
        phone.until(case, |r| r.devices().into_iter().find(|d| d.id == desk_id && d.state.is_some()));
        if pull {
            desk.remote.clone().pick(Some(phone_id.clone()));
            desk.until(case, |r| r.active().filter(|m| m.playing));
        }
        let started = relay.sim.now_us();
        if pull {
            desk.remote.clone().pick(None);
        } else {
            phone.remote.clone().pick(Some(desk_id.clone()));
        }
        let Op::Replace { songs, index: 1, play: true, .. } = desk.told() else { panic!("{case}: the queue handed") };
        let took = relay.sim.now_us() - started;
        assert_eq!(songs.len(), 2, "{case}");
        assert_eq!(phone.told(), Op::Pause, "{case}");
        assert_eq!(took, 0, "{case}: the queue waited for a timer");
        relay.close();
    }
}

#[test]
fn a_device_handed_the_queue_does_not_follow_the_one_that_handed_it() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 1);
    desk.queued(&["d1"]);
    for d in [&phone, &desk] {
        d.remote.clone().serve(true);
        d.remote.clone().watch(true);
    }
    let (phone_id, desk_id) = (phone.remote.id(), desk.remote.id());
    desk.remote.clone().discover_active(true);
    desk.until("the desk follows the phone", |r| r.active().filter(|m| m.id == phone_id));
    phone.until("the phone sees the desk", |r| r.devices().into_iter().find(|d| d.id == desk_id && d.state.is_some()));
    phone.remote.clone().pick(Some(desk_id.clone()));
    assert!(matches!(desk.told(), Op::Replace { play: true, .. }));
    assert_eq!(phone.told(), Op::Pause);
    // The desk's song is still loading, and the phone has said a state since the hand-off but not its
    // pause, when the desk's window comes into sight again.
    phone.remote.clone().played(Playing { playing: true, position_ms: 5_000, volume: Some(41), ..Default::default() });
    desk.until("the desk hears the phone after the hand-off", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.state.as_ref().is_some_and(|s| s.volume == Some(41))));
    desk.remote.clone().discover_active(true);
    relay.sim.pass(Duration::from_secs(5));
    assert!(desk.remote.active().is_none(), "the desk plays the queue handed to it");
    relay.close();
}
