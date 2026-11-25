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
use nori_core::remote::{jam_join, Controls, JamJoin, JamStart, Discovery, Follower, JamControls, Lead, Listening, Playing, Reach, Remote, RemoteMe, RemotePlayer, RemoteShown, Sight};
use nori_core::transport::{Exchange, FailureKind, Transport, TransportError, TransportResponse};
use nori_player::playlist::Hand;
use nori_core::race::{LyricsPick, LyricsShown};
use nori_core::settings::{SavedServer, StoredPrefs};
use nori_core::sync::{Came, Here, NeededPlugin, PairingView, Plays, ProfilesSynced, Reach as SyncReach, RemoteRoom, SyncClient, SyncEvent, SyncState, Syncing};
use nori_sync::hub::HubSockets;
use nori_core::{Core, ServerConfig, Song};
use nori_model::LyricsTiming;
use nori_remote::wire::{Answer, Body, DeviceKind, DeviceState, Event, GroupSaid, Member, Op, Outgoing, Refusal, Role, Room};
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
    /// Requests to doors are refused, as by a device gone from the network; those under way still answer.
    doors_refuse: std::sync::atomic::AtomicBool,
    /// The devices' network is metered (mobile data).
    metered: std::sync::atomic::AtomicBool,
    /// LRCLIB's answer for any song.
    lrclib: Mutex<Option<String>>,
    /// The sync server my devices meet at.
    sync: nori_sync::hub::Hub,
    /// Each account's devices' syncs, the first made first: one group each.
    groups: Mutex<HashMap<String, Vec<Arc<Syncing>>>>,
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
            hub: Mutex::default(), waiting: Mutex::default(), next: AtomicU64::new(1), absent: absent.into(), lagging: Mutex::new(None), lagged: AtomicU64::new(0), door_late: Default::default(), down: Default::default(), along: std::sync::atomic::AtomicBool::new(true), time: Default::default(), first_time_delay_ms: Default::default(), answers_late: AtomicU64::new(0), lose_closes: Default::default(), opens_late: Default::default(), opens: AtomicU64::new(0), first_answers_held: Default::default(), doors_refuse: Default::default(), metered: Default::default(), lrclib: Default::default(), sync: nori_sync::hub::Hub::new(START_US / 1000), groups: Mutex::default() })
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
                    events: mine(&hub),
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
        if let Some(body) = self.lrclib.lock().clone().filter(|_| request.url.starts_with("https://lrclib.net/api/get")) {
            return Ok(TransportResponse { status: 200, body: body.into_bytes(), headers: Vec::new() });
        }
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
            self.arrived(&params);
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
        if self.metered.load(Ordering::Relaxed) {
            nori_core::transport::Network::Metered
        } else {
            nori_core::transport::Network::Unmetered
        }
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
    /// Its sync and its sockets to the sync server, in its account's group; none for a jam's guest.
    group: Option<(Arc<Syncing>, Arc<HubSockets>)>,
    _dir: Option<nori_testdir::TempDir>,
}

/// A device of ann's group whose remote control the test speaks by hand, as `id`: what it hears, and
/// what it says.
struct Puppet {
    id: String,
    /// Its profile's id on the puppet.
    profile: String,
    syncing: Arc<Syncing>,
    net: Arc<HubSockets>,
    heard: Arc<Mutex<Vec<GroupSaid>>>,
    _room: Arc<dyn RemoteRoom>,
    _dir: nori_testdir::TempDir,
}

/// What a puppet's remote control hears.
struct PuppetRoom(Arc<Mutex<Vec<GroupSaid>>>);

impl RemoteRoom for PuppetRoom {
    fn in_room(&self, _: nori_core::sync::InRoom) {}
    fn keyed(&self) {}
    fn changed(&self) {}
    fn heard(&self, _: String, message: serde_json::Value) {
        self.0.lock().push(serde_json::from_value(message).unwrap());
    }
    fn left(&self, _: String) {}
}

impl Puppet {
    fn new(relay: &Arc<Relay>, id: &str) -> Puppet {
        let dir = nori_testdir::TempDir::new("remote-puppet");
        let settings = nori_core::settings_store::Settings::new();
        let prefs = settings.open(&dir.path().join("app.db").to_string_lossy()).unwrap();
        let profile = profile_of(&ann());
        settings.put(StoredPrefs { active_server_id: profile.id.clone(), servers: vec![profile.clone()], sync: true, ..prefs });
        let (syncing, net) = relay.grouped(&settings, &ann(), id);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let room: Arc<dyn RemoteRoom> = Arc::new(PuppetRoom(heard.clone()));
        syncing.remote_open(profile.id.clone(), room.clone());
        let puppet = Puppet { id: id.into(), profile: profile.id, syncing, net, heard, _room: room, _dir: dir };
        // As a remote does once in the room: who it is, asking the others. The group's first device is in
        // no room yet.
        let hello = GroupSaid::Here { member: Box::new(Member { id: id.into(), name: id.into(), kind: DeviceKind::Phone, state: None }), ask: true, instance: None };
        puppet.syncing.remote_say(&puppet.profile, None, serde_json::to_value(hello).unwrap());
        puppet
    }

    /// Says `said` to `to`'s device, or every device of the group.
    fn say(&self, to: Option<&Device>, said: GroupSaid) {
        assert!(self.syncing.remote_say(&self.profile, to.map(Device::group_id), serde_json::to_value(said).unwrap()), "said");
    }

    /// Says it serves, playing `state`.
    fn here(&self, state: DeviceState) {
        self.say(None, GroupSaid::Here { member: Box::new(Member { id: self.id.clone(), name: self.id.clone(), kind: DeviceKind::Phone, state: Some(state) }), ask: false, instance: None });
    }

    /// The commands it heard, with their ids.
    fn commands(&self) -> Vec<(String, u64, Op)> {
        self.heard.lock().iter().filter_map(|s| match s {
            GroupSaid::Event { from, to, body: Body::Command { id, op } } if *to == self.id => Some((from.clone(), *id, (**op).clone())),
            _ => None,
        }).collect()
    }

    /// The answers it heard.
    fn answers(&self) -> Vec<(u64, Option<Refusal>)> {
        self.heard.lock().iter().filter_map(|s| match s {
            GroupSaid::Event { to, body: Body::Ack { id, refusal }, .. } if *to == self.id => Some((*id, *refusal)),
            _ => None,
        }).collect()
    }
}

/// What sync tells a test's device: nothing it acts on.
struct Synced;

impl SyncClient for Synced {
    fn here(&self) -> Here {
        Here { plays: Plays::ServersAndPlugins, copied: vec!["subsonic".into()], plugins: vec![] }
    }
    fn profiles(&self, _: ProfilesSynced) {}
    fn install(&self, _: Vec<NeededPlugin>) {}
    fn event(&self, _: SyncEvent) {}
    fn changed(&self) {}
}

/// The profile an account's device keeps for `config`.
fn profile_of(config: &ServerConfig) -> SavedServer {
    SavedServer { id: format!("{}-{}", config.user, nori_core::settings::new_server_id()), name: "Home".into(), url: config.url.clone(), user: config.user.clone(), password: config.password.clone(), ..Default::default() }
}

impl Relay {
    /// A sync for the device whose settings `settings` keep, in the group of `config`'s account: the
    /// first device makes it when the second pairs. It is a phone playing in the background, whose
    /// keep-alives ride on the audio: sync arms no timer of its own.
    fn grouped(&self, settings: &Arc<nori_core::settings_store::Settings>, config: &ServerConfig, name: &str) -> (Arc<Syncing>, Arc<HubSockets>) {
        let net = HubSockets::new(&self.sync, Some(self.sim.clone()));
        let sim = self.sim.clone();
        let syncing = Syncing::new(settings.clone(), net.clone(), Arc::new(Synced), SyncReach::Phone, Box::new(move || sim.clone() as Arc<dyn Tasks>));
        net.tell(&syncing);
        syncing.playing(true);
        let key = format!("{}:{}", config.user, config.password);
        let first = self.groups.lock().get(&key).and_then(|g| g.first().cloned());
        if let Some(old) = first.filter(|_| !nori_sync::paired(settings)) {
            syncing.pair_show(name.into());
            let link = self.sim.until("a code", || match syncing.pairing() { PairingView::Code { link, .. } => Some(link), _ => None });
            assert_eq!(old.pair_take(link, Came::Camera), None);
            self.sim.until("what to confirm", || matches!(old.pairing(), PairingView::Confirm { .. }).then_some(()));
            old.pair_confirm("First".into(), false);
            self.sim.until("paired", || (syncing.pairing() == PairingView::Paired).then_some(()));
            syncing.pair_cancel();
            old.pair_cancel();
            let local = settings.with_prefs(|p| p.active_server_id.clone()).unwrap();
            self.sim.until("the profile synced", || (syncing.state() == SyncState::Connected && syncing.synced(local.clone())).then_some(()));
        }
        self.groups.lock().entry(key).or_default().push(syncing.clone());
        (syncing, net)
    }
}

impl Device {
    fn new(relay: &Arc<Relay>, config: ServerConfig, kind: DeviceKind, name: &str) -> Device {
        Device::found(relay, config, kind, name, None)
    }

    fn found(relay: &Arc<Relay>, config: ServerConfig, kind: DeviceKind, name: &str, discovery: Option<Arc<dyn Discovery>>) -> Device {
        Device::with(relay, config, kind, name, discovery, Default::default())
    }

    /// An account's device (a guest's has no group) keeps its settings in a directory of its own
    /// unless `session` keeps them already, with its account's profile in use.
    fn with(relay: &Arc<Relay>, config: ServerConfig, kind: DeviceKind, name: &str, discovery: Option<Arc<dyn Discovery>>, session: Arc<nori_queue::Session>) -> Device {
        let account = kind != DeviceKind::Guest;
        let (session, dir) = match account && session.settings.app_db().is_none() {
            true => {
                let dir = nori_testdir::TempDir::new("remote-device");
                let settings = nori_core::settings_store::Settings::new();
                let prefs = settings.open(&dir.path().join("app.db").to_string_lossy()).unwrap();
                settings.put(prefs);
                (Arc::new(nori_queue::Session::new(settings)), Some(dir))
            }
            false => (session, None),
        };
        let settings = session.settings.clone();
        if account && settings.with_prefs(|p| p.servers.is_empty()).unwrap_or(true) {
            let profile = profile_of(&config);
            settings.change(|p| StoredPrefs { active_server_id: profile.id.clone(), servers: vec![profile.clone()], sync: true, remote_control: true, ..p.clone() });
        }
        let group = account.then(|| relay.grouped(&settings, &config, name));
        let core = Core::new(String::new(), "remote".into(), session).unwrap();
        core.configure(config).unwrap();
        let client = Client::new(core.clone(), relay.clone(), Default::default());
        client.set_profile(NetProfile { url: SERVER.into(), ..Default::default() });
        let (ops_to, ops) = channel();
        let ended = Arc::new(Mutex::new(Vec::new()));
        let (me, player, shown) = (RemoteMe { name: name.into(), kind }, Arc::new(Player(Mutex::new(ops_to))), Arc::new(Shown(ended.clone())));
        let remote = Remote::with_tasks(client.clone(), me, player, shown, discovery, group.as_ref().map(|(s, _)| s.clone()), relay.sim.clone());

        relay.devices.lock().push(Arc::downgrade(&remote));
        Device { sim: relay.sim.clone(), core, client, remote, ops, ended, group, _dir: dir }
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

    /// Its id in its group.
    fn group_id(&self) -> String {
        self.group.as_ref().and_then(|(s, _)| s.device()).expect("in a group")
    }

    /// The frames it sent to the sync server so far: what a server on Cloudflare bills.
    fn billed(&self) -> usize {
        self.group.as_ref().map_or(0, |(_, net)| net.sent.load(Ordering::Relaxed))
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
        // Listed in the group's room as playing, it answers nothing.
        let ghost = Puppet::new(&relay, "ghost");
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        desk.queued(&["local"]);
        desk.remote.clone().serve(true);
        desk.remote.clone().watch(true);
        ghost.here(DeviceState { playing: true, index: Some(0), len: 1, ..Default::default() });
        desk.until("the ghost playing", |r| r.devices().into_iter().find(|d| d.id == "ghost" && d.state.as_ref().is_some_and(|s| s.playing)));
        desk.remote.clone().watch(false);
        desk.remote.clone().pick(Some("ghost".into()));
        assert!(desk.remote.active().is_some());
        desk.remote.clone().pick(None);
        let picked_at = relay.sim.now_us();
        desk.until("the controls here", |r| r.active().is_none().then_some(()));
        assert!(relay.sim.now_us() - picked_at <= 3_000_000, "a dead device holds the controls no longer than a command shows");
        assert_eq!(desk.core.session.playlist(|p| p.ids().to_vec()), ["local"], "the stored local queue remains usable");
        let request = || ghost.commands().into_iter().find_map(|(from, id, op)| (from == desk.remote.id() && matches!(op, Op::Transfer { ref to, .. } if *to == desk.remote.id())).then_some(id));
        eventually(&relay, "the queue request", || request().is_some());
        let transfer = request();
        match action {
            "play" => desk.playing(&["local"], 0),
            "edit" => desk.queued(&["edited"]),
            _ => {},
        }
        // A late answer, from a device that came back, must respect what happened here.
        let late = Body::Command { id: 999, op: Box::new(Op::Replace {
            songs: vec![Song::only_id("remote".into())], index: 0, position_ms: 5_000,
            play: true, order: None, shuffle: false, repeat: 0, transfer, listen: None, mix: None,
        }) };
        ghost.say(Some(&desk), GroupSaid::Event { from: "ghost".into(), to: desk.remote.id(), body: late });
        if action == "wait" {
            assert!(matches!(desk.told(), Op::Replace { songs, .. } if songs[0].id == "remote"));
        } else {
            eventually(&relay, "the late queue refused", || ghost.answers().contains(&(999, Some(Refusal::Stale))));
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

/// Where the music plays, one answer for every client: another device mirrored outranks a plugin's
/// cast, which outranks this device; a jam guest's profile plays its jam; a host plays here.
#[test]
fn the_destination_ranks_mirror_cast_and_here() {
    use nori_core::destination::{destination, Casting, Destination};
    let speaker = || Some(Casting { device: "Speaker".into(), sink: true });
    assert_eq!(destination(None, None), Destination::Here { hosting: false }, "remote control off");
    assert_eq!(destination(None, speaker()), Destination::Cast { device: "Speaker".into(), sink: true });
    let relay = Relay::new();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.playing(&["s1", "s2"], 1);
    desk.remote.clone().serve(true);
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    assert_eq!(destination(Some(phone.remote.clone()), speaker()), Destination::Cast { device: "Speaker".into(), sink: true });
    phone.remote.clone().discover_active(true);
    phone.until("the desktop playing", |r| r.active());
    assert_eq!(destination(Some(phone.remote.clone()), speaker()), Destination::Mirroring { device: "Desk".into() });
    let link = opened(&desk);
    assert_eq!(destination(Some(desk.remote.clone()), None), Destination::Here { hosting: true });
    let pass = joined(&relay, link, "Gus".into());
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    assert_eq!(destination(Some(gus.remote.clone()), None), Destination::Guest);
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

/// The search for a playing device goes on once this device's own music stops, or the device it
/// showed is gone, while the app stays on screen.
#[test]
fn active_discovery_resumes_while_the_app_stays_on_screen() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    phone.remote.clone().discover_active(true);
    phone.playing(&["own"], 0);
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.playing(&["s1"], 0);
    desk.remote.clone().serve(true);
    phone.remote.clone().watch(true);
    phone.until("the desk heard", |r| r.devices().into_iter().find(|d| d.name == "Desk").and_then(|d| d.state));
    assert!(phone.remote.active().is_none(), "music playing here keeps the player on this device");

    phone.remote.clone().played(Playing { playing: false, position_ms: 6_000, ..Default::default() });
    phone.until("the desk followed after the pause", |r| r.active().filter(|m| m.name == "Desk"));

    phone.remote.clone().watch(false);
    desk.remote.clone().stop();
    phone.until("the desk gone", |r| r.active().is_none().then_some(()));
    let term = Device::account(&relay, DeviceKind::Terminal, "Term");
    term.playing(&["s2"], 0);
    term.remote.clone().serve(true);
    phone.until("the next device playing", |r| r.active().filter(|m| m.name == "Term"));
    relay.close();
}

/// A guest leaving at once and again as its session closes says so once.
#[test]
fn leaving_a_jam_twice_tells_the_relay_once() {
    let relay = Relay::new();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.playing(&["s1"], 0);
    desk.remote.clone().serve(true);
    let pass = joined(&relay, opened(&desk), "Gus".into());
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    gus.remote.clone().jam_leave();
    gus.remote.clone().jam_leave();
    eventually(&relay, "the leave heard", || relay.asked_for("noriRemote.leave") > 0);
    relay.sim.pass(Duration::from_secs(5));
    assert_eq!(relay.asked_for("noriRemote.leave"), 1);
    relay.close();
}

#[test]
fn active_discovery_uses_lan_without_a_relay() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let desk = Device::found(&relay, ann(), DeviceKind::Desktop, "Desk", Some(announced.clone()));
    desk.playing(&["s1", "s2"], 1);
    desk.remote.clone().serve(true);
    let browsing = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(browsing.clone()));
    // Announced as the group's devices sign now, with the phone in it.
    let door = announced.0.lock().clone().unwrap();
    phone.remote.clone().discover_active(true);
    assert!(browsing.1.load(Ordering::Relaxed));
    phone.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
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
        assert_eq!(donor.counted, None, "the old device no longer owns the listen");
        assert_eq!(receiver.counted.map(|s| s.id).as_deref(), Some("long"), "time heard on both devices qualifies");
        assert_eq!(receiver.counted_at, started, "the original start belongs to the same listen");
        desk.core.session.settings.writer.flush();
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
fn two_devices_control_each_other_through_their_group() {
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
    assert_eq!(m.rows.iter().map(|r| r.song.id.as_str()).collect::<Vec<_>>(), ["s1", "s2", "s5"], "in play order: what was asked for plays after what the host played");
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
fn a_host_that_stops_serving_still_shows_its_jams_queue() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    host.remote.clone().serve(true);
    host.playing(&["s1", "s2"], 0);
    opened(&host);
    host.until("the queue", |r| r.jam_view()?.queue);
    host.remote.clone().serve(false);
    let queue = host.remote.jam_view().and_then(|v| v.queue).map(|q| q.entries.len());
    assert_eq!(queue, Some(2));

    // Serving again, the group hears its state anew though nothing but time moved.
    let watcher = Puppet::new(&relay, "watcher");
    host.remote.clone().serve(true);
    eventually(&relay, "the host's state said again", || watcher.heard.lock().iter().any(|s| matches!(s, GroupSaid::Here { member, .. } if member.id == host.remote.id() && member.state.is_some())));
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

/// Paused, with nothing on its screen, on a metered network, a device says nothing and still hears what
/// another device of its group asks of it at once.
#[test]
fn out_of_sight_on_mobile_data_a_paused_device_hears_its_group() {
    let relay = Relay::new();
    relay.metered.store(true, Ordering::Relaxed);
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().played(Playing { playing: false, position_ms: 5_000, volume: Some(40), ..Default::default() });
    phone.remote.clone().serve(true);
    desk.remote.clone().serve(true);
    let id = phone.remote.id();
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == id));
    phone.remote.clone().sight(Sight::Nothing);
    let said = phone.billed();
    relay.sim.pass(Duration::from_secs(400));
    assert_eq!(phone.billed(), said, "nothing said while it waits");
    desk.remote.clone().send(id, Op::Play);
    relay.sim.pass(Duration::from_secs(1));
    assert_eq!(phone.ops.try_recv().ok(), Some(Op::Play), "asked to play, it hears within a second");
    phone.remote.stop();
    desk.remote.stop();
    relay.close();
}

#[test]
fn library_discovery_follows_the_foreground_while_playing() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
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
    phone.remote.clone().serve(true);
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
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
fn nearby_devices_need_no_server() {
    let relay = Relay::absent();
    let ann = |password: &str| ServerConfig { url: SERVER.into(), user: "ann".into(), password: password.into(), ..Default::default() };
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann("pw"), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    let txt = || door.txt.clone();

    // A device of another group on the network does not list the door.
    let other = Device::new(&relay, ServerConfig { user: "bob".into(), ..ann("pw") }, DeviceKind::Desktop, "Bob");
    other.remote.clone().watch(true);
    other.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, txt());
    assert!(other.remote.devices().is_empty());

    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, txt());
    let seen = desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.state.is_some()));
    assert!(seen.nearby && seen.name == "Phone");
    let said = desk.billed();
    desk.remote.clone().send(seen.id.clone(), Op::Pause);
    assert_eq!(phone.told(), Op::Pause);
    assert_eq!(desk.billed(), said, "the command went to the door, not the sync server");

    // Mirrored with the picker closed: its door is still followed, and its whole queue read through it.
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(seen.id.clone()));
    phone.remote.clone().played(Playing { playing: false, position_ms: 9_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone's newer word", |r| r.active().filter(|m| m.position_ms == 9_000 && m.rows.len() == 2));

    // The door seen again (mDNS reports it once per network interface) is still followed by the one poller.
    let first_polls = || relay.asked().iter().filter(|a| a.starts_with("lan ") && a.contains("noriRemote.poll") && !a.contains("since=")).count();
    let before = first_polls();
    for _ in 0..3 {
        desk.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, txt());
    }
    phone.remote.clone().played(Playing { playing: true, position_ms: 1_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone playing", |r| r.active().filter(|m| m.playing));
    assert_eq!(first_polls(), before, "no poller started over");

    // Seen at an address of this machine's no door answers at (another network interface): the phone is
    // still followed where it answers.
    let nowhere = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    desk.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], nowhere, txt());
    phone.remote.clone().played(Playing { playing: false, position_ms: 3_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone paused", |r| r.active().filter(|m| !m.playing));
    phone.remote.clone().played(Playing { playing: true, position_ms: 4_000, index: None, volume: None, ..Default::default() });
    desk.until("the phone playing again", |r| r.active().filter(|m| m.playing));

    // Its word comes late through the door; the sync server's clock, which both know, says when it was
    // heard.
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

    // A device of the same account in another group is not shown the door.
    let stranger = Device::new(&relay, ann("old"), DeviceKind::Desktop, "Old");
    stranger.remote.clone().watch(true);
    stranger.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, txt());
    assert!(stranger.remote.devices().is_empty(), "the announced group tag differs");
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
    let phone_id = phone.remote.id();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, door.txt.clone());
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
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, door.txt.clone());
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.state.is_some()));

    desk.remote.clone().lan_lost(door.name.clone());
    assert!(desk.remote.devices().iter().all(|d| d.nearby), "nearby while the open picker follows it");
    desk.remote.clone().watch(false);
    assert!(desk.remote.devices().iter().all(|d| !d.nearby), "no longer nearby once nothing follows it, listed through the group");
    relay.close();
}

#[test]
fn a_server_without_the_relay_is_asked_again_only_by_a_picker() {
    let relay = Relay::absent();
    let dir = nori_testdir::TempDir::new("remote-no-relay");
    let phone = Device::kept(&relay, &dir, DeviceKind::Phone, "Phone");
    phone.until("the answer", |r| (r.jam_start() == JamStart::Unsupported).then_some(()));
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
    phone.until("the answer", |r| (r.jam_start() == JamStart::Unsupported).then_some(()));
    let asked = || relay.asked().iter().filter(|a| a.starts_with("noriRemote.")).count();

    phone.remote.clone().watch(true);
    phone.until("asked again", |_| (asked() == 2).then_some(()));
    phone.remote.clone().watch(false);
    assert_eq!(phone.remote.jam_start(), JamStart::Unsupported);

    // The server gained the relay: the next opening finds it, and the picker follows the account's devices.
    relay.absent.store(false, Ordering::Relaxed);
    phone.remote.clone().watch(true);
    phone.until("the relay", |r| (r.jam_start() == JamStart::Offered).then_some(()));
    assert_eq!(phone.remote.jam_start(), JamStart::Offered);
    phone.until("polling", |_| (phone.polls(&relay) >= 3).then_some(()));
    relay.close();
}

/// A picker opened again while the last opening's answer is still on its way asks once more after it.
#[test]
fn a_picker_opened_during_an_answer_asks_again() {
    let relay = Relay::absent();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    phone.until("the answer", |r| (r.jam_start() == JamStart::Unsupported).then_some(()));
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

/// A mirrored device's playhead runs on by the sync server's clock, which both devices know: nothing is
/// said for it, on screen or off.
#[test]
fn a_mirrored_playhead_runs_on_without_a_message() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored whole, playing", |r| r.active().filter(|m| m.playing && m.rows.len() == 2));
    let said = (desk.billed(), phone.billed());
    for sight in [Sight::Screen, Sight::Notification, Sight::Screen] {
        desk.remote.clone().sight(sight);
        relay.sim.pass(Duration::from_secs(60));
    }
    assert_eq!((desk.billed(), phone.billed()), said, "a minute each, on screen and off");
    let now = relay.sim.now_us();
    let shown = desk.remote.active().unwrap().position_at(now);
    assert!((185_000..=186_000).contains(&shown), "three minutes on from 5 s: {shown}");
    relay.close();
}

/// Nothing shows the mirrored device: what it says still arrives, and nothing is asked of it.
#[test]
fn a_mirrored_device_out_of_sight_is_asked_nothing() {
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
    desk.until("the phone mirrored whole, paused", |r| r.active().filter(|m| !m.playing && m.rows.len() == 2));

    desk.remote.clone().sight(Sight::Nothing);
    let said = desk.billed();
    phone.remote.clone().played(Playing { playing: true, position_ms: 5_000, ..Default::default() });
    desk.until("the phone playing", |r| r.active().filter(|m| m.playing));
    relay.sim.pass(Duration::from_secs(60));
    assert_eq!(desk.billed(), said, "the desk said nothing");
    desk.remote.clone().sight(Sight::Screen);
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

    // Gone: no longer followed, and another device's news asks nothing of the desk.
    phone.remote.clone().serve(false);
    desk.until("the phone gone", |r| r.active().is_none().then_some(()));
    assert!(!desk.remote.mirroring());
    let other = Device::account(&relay, DeviceKind::Phone, "Other");
    other.playing(&["s1"], 0);
    other.remote.clone().serve(true);
    desk.until("the other device", |r| r.devices().into_iter().find(|d| d.name == "Other"));
    let said = desk.billed();
    other.remote.clone().played(Playing { playing: false, position_ms: 9_000, ..Default::default() });
    desk.until("its news", |r| r.devices().into_iter().find(|d| d.name == "Other").and_then(|d| d.state).filter(|s| !s.playing));
    assert_eq!(desk.billed(), said, "the desk says nothing with nothing to follow");
    relay.close();
}

#[test]
fn a_page_past_the_end_of_a_queue_that_shrank_is_empty() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Puppet::new(&relay, "desk");
    phone.playing(&["s1", "s2", "s3"], 0);
    phone.remote.clone().serve(true);
    let asked = Body::Command { id: 1, op: Box::new(Op::Page { from: 10, count: 50 }) };
    desk.say(Some(&phone), GroupSaid::Event { from: "desk".into(), to: phone.remote.id(), body: asked });
    eventually(&relay, "an empty page", || desk.heard.lock().iter().any(|s| matches!(s, GroupSaid::Event { body: Body::Page { from: 10, entries, .. }, .. } if entries.is_empty())));
    relay.close();
}

#[test]
fn a_mirrored_queue_is_cleared_and_put_back_in_its_place() {
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

    // Clear: the phone's player clears its queue, and the desk's undo has it put back what it took.
    assert!(!desk.remote.clone().put_back_cleared(phone_id.clone()), "only a clear from here");
    desk.remote.clone().send(phone_id.clone(), Op::Clear);
    assert_eq!(phone.told(), Op::Clear);
    assert!(desk.remote.clone().put_back_cleared(phone_id.clone()));
    assert_eq!(phone.told(), Op::RestoreCleared);
    assert!(!desk.remote.clone().put_back_cleared(phone_id.clone()), "put back once");
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

#[test]
fn hand_offs_in_a_circle_are_followed_once() {
    let relay = Relay::new();
    let (phone, laptop) = (Puppet::new(&relay, "phone"), Puppet::new(&relay, "laptop"));
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    let entries = vec![nori_remote::wire::Entry { index: 0, id: "s1".into(), duration: 200, ..Default::default() }];
    let handed = |to: &str| DeviceState { instance: Some(1), seq: 2, index: Some(0), len: 1, entries: entries.clone(), handed_to: Some(to.into()), ..Default::default() };
    phone.here(handed("laptop"));
    laptop.here(handed("phone"));
    desk.until("both", |r| (r.devices().iter().filter(|d| d.state.is_some()).count() == 2).then_some(()));
    desk.remote.clone().pick(Some("phone".into()));
    assert_eq!(desk.remote.active().map(|m| m.id), Some("laptop".to_string()));
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
    // The phone's state is on its way as the desk sends.
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
    let phone_id = phone.remote.id();

    // The desk mirrors the phone through the group and its door.
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, door.txt.clone());
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.nearby));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored", |r| r.active());
    let desk_id = desk.remote.id();
    let asked = || relay.asked().into_iter().filter(|a| (a.starts_with("noriRemote.poll") && a.ends_with(&desk_id)) || (a.starts_with("lan ") && a.contains(&format!("dev={desk_id}")) && !a.contains("noriRemote.send"))).collect::<Vec<_>>();
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
    assert_eq!(asked(), before, "no poll after the stop");
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

    // A client quitting right after the stop has told the group.
    let said = phone.billed();
    phone.remote.clone().stop();
    phone.remote.sent();
    assert!(phone.billed() > said, "the group was told before the client may quit");
    desk.until("the phone gone", |r| r.devices().is_empty().then_some(()));
    relay.close();
}

/// A device's remote made again at once, as switching to another profile of the same account does,
/// stays listed though the one before said it is gone after.
#[test]
fn a_device_made_again_at_once_stays_listed() {
    let relay = Relay::new();
    let dir = nori_testdir::TempDir::new("remote-made-again");
    let phone = Device::kept(&relay, &dir, DeviceKind::Phone, "Phone");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    desk.remote.clone().watch(true);
    let listed = |r: &Remote| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state);
    let first = desk.until("the phone", listed);

    phone.remote.clone().stop();
    let again = Device::kept(&relay, &dir, DeviceKind::Phone, "Phone");
    assert_eq!(again.remote.id(), phone_id, "the same device");
    again.playing(&["s1"], 0);
    again.remote.clone().serve(true);
    again.until("the new remote's state said", |_| listed(&desk.remote).filter(|s| s.instance > first.instance));
    // The remote before says it is gone only now, after the new one said it is here.
    let gone = GroupSaid::Gone { id: phone_id.clone(), instance: first.instance };
    phone.group.as_ref().unwrap().0.remote_say(&phone.core.session.settings.with_prefs(|p| p.active_server_id.clone()).unwrap(), None, serde_json::to_value(gone).unwrap());
    desk.sim.pass(Duration::from_secs(5));
    assert!(listed(&desk.remote).is_some(), "the desk lost the phone");
    relay.close();
}

/// A device whose connection to the sync server dropped leaves the lists, and is listed again once it
/// is back.
#[test]
fn far_devices_are_hidden_while_their_connection_is_down() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    let listed = |r: &Remote| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state);
    desk.until("the phone", listed);

    desk.group.as_ref().unwrap().1.drop_all();
    desk.until("the phone hidden", |r| r.devices().is_empty().then_some(()));
    desk.until("the phone again", listed);
    phone.group.as_ref().unwrap().1.drop_all();
    desk.until("the phone hidden while its connection is down", |r| r.devices().is_empty().then_some(()));
    desk.until("and again once it is back", listed);
    relay.close();
}

/// A mirrored device that leaves the lists is no longer shown as the one played on: it stops serving,
/// its connection drops, or this device's does.
#[test]
fn a_mirrored_device_that_leaves_the_lists_is_no_longer_mirrored() {
    for how in ["stops serving", "drops", "drops here"] {
        let relay = Relay::new();
        let phone = Puppet::new(&relay, "phone");
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        phone.here(DeviceState { playing: true, index: Some(0), len: 1, ..Default::default() });
        desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == "phone").and_then(|d| d.state));
        desk.remote.clone().pick(Some("phone".into()));
        desk.until("the phone mirrored", |r| r.active());

        match how {
            "stops serving" => phone.say(None, GroupSaid::Here { member: Box::new(Member { id: "phone".into(), name: "phone".into(), kind: DeviceKind::Phone, state: None }), ask: false, instance: None }),
            "drops" => phone.net.drop_all(),
            _ => desk.group.as_ref().unwrap().1.drop_all(),
        }
        desk.until("the phone hidden", |r| r.devices().is_empty().then_some(()));
        desk.until(&format!("the phone no longer mirrored once it {how}"), |r| r.active().is_none().then_some(()));
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

/// What a lyrics view was shown, the newest last.
#[derive(Default)]
struct LyricsView(Mutex<Vec<LyricsPick>>);

impl LyricsShown for LyricsView {
    fn show(&self, pick: LyricsPick) {
        self.0.lock().push(pick);
    }
}

impl LyricsView {
    fn timing(&self) -> Option<LyricsTiming> {
        self.0.lock().last().map(|p| p.lyrics.timing)
    }
}

/// A sung song measured into an analyser, and LRCLIB's answer for it: a line per sung phrase, 1.2 s late.
fn sung() -> (Song, nori_player::automix::analysis::Analyzer, String) {
    use nori_player::automix::analysis::Analyzer;
    use nori_player::automix::eval::{Song as Synthetic, Style, FULL, SUNG};
    let synthetic = Synthetic { sections: vec![(4, FULL), (12, SUNG), (6, FULL), (12, SUNG), (4, FULL)], ..Synthetic::new("offset", Style::Backbeat, 112.0, 2, false) };
    let (x, truth) = synthetic.render();
    let mut a = Analyzer::new(synthetic.rate, 0);
    a.feed(&x);
    let mut lrc = String::new();
    let mut last = f64::MIN;
    for (i, (start, end)) in truth.voice.iter().enumerate() {
        if start - last > 0.3 {
            let t = start - 0.2 + 1.2;
            lrc.push_str(&format!("[{:02}:{:05.2}]lomira teshvan korupel sumidah {i} netori falquen\n", (t / 60.0) as u32, t % 60.0));
        }
        last = *end;
    }
    let secs = (x.len() / synthetic.rate as usize) as u32;
    let song = Song { id: "yt-venom".into(), title: "Offset".into(), artist: "Pink Floyd".into(), duration: secs, ..Default::default() };
    let answer = serde_json::json!({"syncedLyrics": lrc, "duration": secs, "trackName": "Offset", "artistName": "Pink Floyd"}).to_string();
    (song, a, answer)
}

/// A lyrics view of song `id` open on `d`.
fn lyrics_view(d: &Device, id: &str) -> Arc<LyricsView> {
    let view = Arc::new(LyricsView::default());
    let (client, id, shown) = (d.client.clone(), id.to_string(), view.clone());
    d.sim.spawn(Box::pin(async move { let _ = client.lyrics_for(id, shown).await; }));
    view
}

/// A jam's guest shows the host's lyrics as the host times them, looking nothing up; moving them is the
/// host's and its admins', as seeking is.
#[test]
fn a_jam_guest_shows_the_hosts_lyrics() {
    let relay = Relay::new();
    let host = Device::account(&relay, DeviceKind::Phone, "Host");
    let (song, measured, answer) = sung();
    *relay.lrclib.lock() = Some(answer);
    host.core.session.register(vec![song.clone()]);
    host.core.session.set(vec![song.id.clone()], Some(0), false, None);
    host.core.analysis_finish(&song.id, measured).unwrap().expect("measured");
    host.remote.clone().played(Playing { playing: true, position_ms: 5_000, ..Default::default() });
    let link = opened(&host);
    let pass = joined(&relay, link, "Gus".into());
    let gus = Device::new(&relay, ServerConfig { url: pass.url, api_key: Some(pass.api_key), ..Default::default() }, DeviceKind::Guest, "Gus");
    gus.remote.clone().watch(true);
    gus.until("the host's song", |r| r.jam_playing());

    let at_host = lyrics_view(&host, &song.id);
    let checked = host.until("the host's lyrics checked", |_| at_host.timing().filter(|t| matches!(t, LyricsTiming::Automatic { .. })));
    let at_gus = lyrics_view(&gus, &song.id);
    gus.until("the host's lyrics at Gus", |_| (at_gus.timing() == Some(checked)).then_some(()));
    assert_eq!(*at_gus.0.lock(), at_host.0.lock().last().cloned().into_iter().collect::<Vec<_>>(), "only the host's, nothing of Gus's own");

    let shown = at_gus.0.lock().last().cloned().unwrap();
    assert_eq!(gus.core.lyrics_nudge(song.id.clone(), shown, 1).lyrics.timing, checked, "not a guest's to move");
    relay.close();
}

/// A device that shows another's playback shows the lyrics that device shows, timed as it times them:
/// only the device playing hears the song to check them against. A nudge on either reaches both.
#[test]
fn lyrics_show_as_the_device_playing_them_times_them() {
    let relay = Relay::new();
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let (song, measured, answer) = sung();
    *relay.lrclib.lock() = Some(answer);
    // The phone queued the song before but never heard it; the desk plays it and measured it.
    for d in [&desk, &phone] {
        d.core.session.register(vec![song.clone()]);
        d.core.session.set(vec![song.id.clone()], Some(0), false, None);
    }
    desk.core.analysis_finish(&song.id, measured).unwrap().expect("measured");
    desk.remote.clone().played(Playing { playing: true, position_ms: 5_000, ..Default::default() });
    phone.remote.clone().played(Playing { playing: false, position_ms: 0, ..Default::default() });
    for d in [&desk, &phone] {
        d.remote.clone().serve(true);
        d.remote.clone().watch(true);
    }
    phone.remote.clone().discover_active(true);
    phone.until("the desk followed", |r| r.active());

    let at_desk = lyrics_view(&desk, &song.id);
    let checked = desk.until("the desk's lyrics checked", |_| at_desk.timing().filter(|t| matches!(t, LyricsTiming::Automatic { .. })));
    assert!(matches!(checked, LyricsTiming::Automatic { ms } if (ms - 1200).abs() < 150), "{checked:?}");
    let on_phone = lyrics_view(&phone, &song.id);
    phone.until("the phone's lyrics timed as the desk's", |_| (on_phone.timing() == Some(checked)).then_some(()));
    assert_eq!(on_phone.0.lock().last(), at_desk.0.lock().last(), "the same words");

    // A step sooner on the phone moves the desk's too, and one later on the desk the phone's.
    let shown = on_phone.0.lock().last().cloned().unwrap();
    let sooner = phone.core.lyrics_nudge(song.id.clone(), shown, 1).lyrics.timing;
    assert!(matches!(sooner, LyricsTiming::Yours { .. }), "{sooner:?}");
    desk.until("the phone's nudge on the desk", |_| (at_desk.timing() == Some(sooner)).then_some(()));
    let shown = at_desk.0.lock().last().cloned().unwrap();
    let later = desk.core.lyrics_nudge(song.id.clone(), shown, -1).lyrics.timing;
    assert_ne!(later, sooner);
    phone.until("the desk's nudge on the phone", |_| (on_phone.timing() == Some(later)).then_some(()));
    relay.close();
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

/// A speaker another device reaches takes the music here, whatever that device last played; only what
/// plays there now goes on.
#[test]
fn casting_through_a_device_sends_this_ones_music() {
    use Did::*;
    type Steps = &'static [(bool, Did)];
    let cases: &[(&str, Steps, Picked)] = &[
        ("the phone played last", &[(true, Played), (true, Paused), (false, Played), (false, Paused)], Picked::Transfers),
        ("the desk plays", &[(false, Played), (false, Paused), (true, Played)], Picked::Transfers),
        ("the phone plays", &[(true, Played), (true, Paused), (false, Played)], Picked::Mirrors),
    ];
    for (case, steps, want) in cases {
        let relay = Relay::new();
        let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
        let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
        phone.remote.clone().serve(true);
        desk.remote.clone().serve(true);
        let phone_id = phone.remote.id();
        for &(on_desk, did) in steps.iter() {
            let (d, ids) = if on_desk { (&desk, ["d1", "d2"]) } else { (&phone, ["p1", "p2"]) };
            match did {
                Played => d.playing(&ids, 0),
                Paused => d.remote.clone().played(Playing { playing: false, position_ms: 6_000, ..Default::default() }),
                Queued => d.queued(&ids),
            }
            relay.sim.pass(Duration::from_secs(1));
            if !on_desk {
                let playing = matches!(did, Played);
                desk.until(&format!("{case}: the phone's state"), |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.playing == playing && s.index.is_some()));
            }
        }
        let output = nori_remote::wire::CastOutput { plugin: "p".into(), id: "kitchen".into(), name: "Kitchen".into(), look: Default::default() };
        desk.remote.clone().cast_on(nori_core::remote::ReachedOutput { device: phone_id.clone(), via: "Phone".into(), output: output.clone() });
        let mut replaced = None;
        let mut cast = false;
        relay.sim.pass(Duration::from_millis(300));
        while let Ok(op) = phone.ops.try_recv() {
            match op {
                Op::Replace { songs, .. } => replaced = Some(songs[0].id.clone()),
                Op::CastTo { output: o } => cast = o == output,
                _ => {}
            }
        }
        assert!(cast, "{case}: the phone casts");
        assert_eq!(replaced.as_deref(), matches!(want, Picked::Transfers).then_some("d1"), "{case}: the music cast");
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
    host.playing(&["s1"], 0);
    host.remote.clone().watch(true);
    opened(&host);
    relay.lose_closes.store(true, Ordering::Relaxed);
    host.remote.clone().jam_close();
    eventually(&relay, "the close lost", || relay.asked().iter().any(|a| a == "lost close"));
    relay.lose_closes.store(false, Ordering::Relaxed);

    // News at the relay: the host's held poll answers, the jam still listed in it.
    relay.list("other", None);
    eventually(&relay, "the jam closed at the relay", || relay.jams() == 0);
    assert!(host.remote.jam_view().is_none(), "no jam shown");
    relay.close();
}

/// Start pressed again and again while the relay is slow to open: one jam is asked for, and once it is
/// ended none comes back.
#[test]
fn a_jam_is_started_once_and_stays_ended() {
    let dir = nori_testdir::TempDir::new("remote-starts");
    let relay = Relay::new();
    let host = Device::kept(&relay, &dir, DeviceKind::Phone, "Host");
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
    relay.list("other", None);
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
    // Its state changes, and would have reached the jam's room by now.
    host.remote.clone().played(Playing { playing: false, position_ms: 7_000, ..Default::default() });
    relay.sim.pass(Duration::from_secs(1));
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

/// A jam guest's pick by its role (`jam_play`, `jam_add`): a plain guest's is asked of the host, the
/// song it would start; an admin's goes straight into the jam's queue.
#[test]
fn a_jam_pick_is_asked_or_sent_by_role() {
    use nori_core::jam_pick::Picked;
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
        d
    };
    let (gus, dee) = (guest_of("Gus"), guest_of("Dee"));
    host.remote.clone().played(Playing { playing: true, position_ms: 30_000, index: Some(0), ..Default::default() });
    let dee_id = dee.until("the jam", |r| r.jam_view().filter(|v| v.members.len() == 3)).you;
    host.remote.clone().jam_act(Op::Promote { member: dee_id, admin: true });
    dee.until("Dee's role", |r| r.jam_controls().filter(|c| !c.asks));
    gus.until("Gus's controls", |r| r.jam_controls());
    let song = |id: &str| Song { id: id.into(), title: id.into(), ..Default::default() };

    assert_eq!(gus.remote.clone().jam_play(vec![song("s7"), song("s8")], 1, false), Picked::Asked { title: "s8".into() });
    host.until("Gus's request", |r| r.jam_view().filter(|v| v.pending.len() == 1 && v.pending[0].song.id == "s8"));
    assert!(host.ops.try_recv().is_err(), "asked, not played");

    assert_eq!(dee.remote.clone().jam_add(vec![song("s9")], true), Picked::Sent);
    assert_eq!(host.told(), Op::Add { songs: vec![song("s9")], next: true });
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
    d.remote.sent();
    assert_eq!(d.sim.now_us(), at, "stopping waited, timers {:?}", d.sim.waits());
}

/// `f` started on the test's executor; its output once it ends.
fn spawned<T: Send + 'static>(sim: &Native, f: impl Future<Output = T> + Send + 'static) -> Arc<Mutex<Option<T>>> {
    let out = Arc::new(Mutex::new(None));
    let to = out.clone();
    sim.spawn(Box::pin(async move { *to.lock() = Some(f.await) }));
    out
}

/// The relay a jam screen polls: a poll that failed waits twice as long each time, until the network is
/// back.
#[test]
fn a_failed_poll_waits_longer_each_time_until_the_network_is_back() {
    let relay = Relay::new();
    relay.first_answers_held.store(true, Ordering::Relaxed);
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let sim = &relay.sim;
    phone.playing(&["s1"], 0);
    phone.remote.clone().watch(true);
    let polls = || relay.asked_for("noriRemote.poll");
    // Its first poll fails once everything here is said: nothing changes while it rests.
    eventually(&relay, "the first poll at the relay", || polls() == 1);
    relay.go_down(true);
    waiting(sim, "the retry", Duration::from_secs(30));
    relay.go_down(false);
    relay.release_first_answers();
    sim.pass(Duration::from_secs(29));
    assert_eq!(sim.waits(), [Duration::from_secs(1)], "only the retry waits");
    assert_eq!(polls(), 1, "not asked again before its time");
    sim.pass(Duration::from_secs(1));
    eventually(&relay, "asked again", || polls() > 1);

    // Down for good: each failed poll waits twice as long as the last, up to 5 min, and a change here
    // does not ask early.
    relay.go_down(true);
    for secs in [30, 60, 120, 240, 300, 300] {
        waiting(sim, "the retry", Duration::from_secs(secs));
        phone.remote.clone().played(Playing { playing: true, position_ms: secs as i64 * 1_000, ..Default::default() });
        waiting(sim, "the same retry after a change", Duration::from_secs(secs));
        sim.pass(Duration::from_secs(secs));
    }
    phone.remote.network_back();
    waiting(sim, "the first wait again once the network is back", Duration::from_secs(30));
    relay.go_down(false);
    let asked = polls();
    phone.remote.network_back();
    eventually(&relay, "answered again", || polls() > asked + 1);

    // Down again after an answer: the wait starts over, and the network coming back asks at once.
    relay.go_down(true);
    waiting(sim, "the retry after that", Duration::from_secs(30));
    relay.go_down(false);
    let asked = polls();
    phone.remote.network_back();
    eventually(&relay, "asked again once the network is back", || polls() > asked);
    relay.close();
}

#[test]
fn leaving_gives_up_the_held_poll_and_leaves_nothing_waiting() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let sim = &relay.sim;
    phone.playing(&["s1"], 0);
    phone.remote.clone().watch(true);
    let id = phone.remote.id();
    eventually(&relay, "the held poll", || relay.parked(&id));
    phone.remote.clone().watch(false);
    eventually(&relay, "its poll no longer held", || !relay.parked(&id));
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
    phone.remote.sent();
    assert_eq!(Arc::strong_count(&phone.remote), 1, "no task of the remote runs");
    relay.close();
}

#[test]
fn sends_reach_the_device_in_the_order_made() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    desk.remote.clone().watch(true);
    let phone_id = phone.remote.id();
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    for ms in 1..=12 {
        desk.remote.clone().send(phone_id.clone(), Op::Seek { ms: ms * 1_000 });
    }
    let told: Vec<Op> = (0..12).map(|_| phone.told()).collect();
    assert_eq!(told, (1..=12).map(|ms| Op::Seek { ms: ms * 1_000 }).collect::<Vec<_>>());
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
                d.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
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

/// The sync server bills each message it is sent, so nothing goes on a timer: an hour of listening, with
/// another device showing the music on screen, costs one message a song.
#[test]
fn an_hour_of_listening_watched_costs_a_message_a_song() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let ids: Vec<String> = (0..20).map(|n| format!("s{n}")).collect();
    let songs: Vec<Song> = ids.iter().map(|id| Song { id: id.clone(), title: id.to_uppercase(), duration: 180, ..Default::default() }).collect();
    phone.core.session.register(songs);
    phone.core.session.set(ids.clone(), Some(0), false, None);
    phone.remote.clone().played(Playing { playing: true, position_ms: 0, index: Some(0), volume: Some(40), ..Default::default() });
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state));
    desk.remote.clone().watch(false);
    desk.remote.clone().pick(Some(phone_id.clone()));
    desk.until("the phone mirrored whole", |r| r.active().filter(|m| m.rows.len() == 20));
    let said = phone.billed() + desk.billed();
    for song in 1..20u32 {
        relay.sim.pass(Duration::from_secs(180));
        phone.remote.clone().played(Playing { playing: true, position_ms: 0, index: Some(song), volume: Some(40), ..Default::default() });
        desk.until("the next song shown", |r| r.active().filter(|m| m.at.is_some_and(|at| m.rows[at as usize].song.id == format!("s{song}"))));
    }
    relay.sim.pass(Duration::from_secs(180));
    assert_eq!(phone.billed() + desk.billed() - said, 19, "an hour, 19 songs after the first");
    relay.close();
}

/// A command the nearby door does not take reaches the device through the group's room.
#[test]
fn a_command_the_door_does_not_take_goes_through_the_group() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
    let phone_id = phone.remote.id();
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.nearby && d.state.is_some()));
    relay.doors_refuse.store(true, Ordering::Relaxed);
    let said = desk.billed();
    desk.remote.clone().send(phone_id, Op::Pause);
    assert_eq!(phone.told(), Op::Pause);
    assert_eq!(desk.billed(), said + 1, "once, through the sync server");
    relay.close();
}

/// A profile that does not sync is no profile of my devices: neither of them is offered the other.
#[test]
fn a_profile_that_does_not_sync_offers_no_device() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1"], 0);
    phone.remote.clone().serve(true);
    desk.remote.clone().serve(true);
    desk.remote.clone().watch(true);
    desk.until("the phone", |r| r.devices().into_iter().find(|d| d.name == "Phone"));
    phone.until("the desk", |r| r.devices().into_iter().find(|d| d.name == "Desk"));
    let (syncing, _) = desk.group.as_ref().unwrap();
    let local = desk.core.session.settings.with_prefs(|p| p.active_server_id.clone()).unwrap();
    syncing.sync_off(local.clone(), false);
    desk.core.session.settings.change(|p| StoredPrefs { servers: p.servers.iter().map(|s| SavedServer { sync_off: s.id == local, ..s.clone() }).collect(), ..p.clone() });
    desk.until("the phone no longer offered", |r| r.devices().is_empty().then_some(()));
    phone.remote.clone().played(Playing { playing: false, position_ms: 9_000, ..Default::default() });
    relay.sim.pass(Duration::from_secs(5));
    assert!(desk.remote.devices().is_empty(), "nor heard again");
    phone.remote.clone().watch(true);
    phone.until("the desk gone from the phone", |r| r.devices().is_empty().then_some(()));
    relay.close();
}

/// A state goes through the sync server only to my devices with the profile open that this one does
/// not reach on the network: with none open, or the one watching reading it at the door, it costs nothing.
#[test]
fn a_state_costs_nothing_when_no_device_needs_the_server_for_it() {
    let relay = Relay::new();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let phone_id = phone.remote.id();
    let at = |ms: i64| Playing { playing: true, position_ms: ms, volume: Some(40), ..Default::default() };

    // The desk reads the phone's state at its door.
    let door = announced.0.lock().clone().expect("the door announced");
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.nearby && d.state.is_some()));
    let said = phone.billed();
    phone.remote.clone().played(at(60_000));
    desk.until("the phone's new place", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.position_ms == 60_000));
    assert_eq!(phone.billed(), said, "heard at the door");

    // No other device has the profile open.
    stop(&desk);
    relay.sim.pass(Duration::from_secs(1));
    let said = phone.billed();
    phone.remote.clone().played(at(90_000));
    relay.sim.pass(Duration::from_secs(1));
    assert_eq!(phone.billed(), said, "said to nobody");
    relay.close();
}

/// Two of my devices on one network sync their profiles through the door while the sync server
/// refuses them.
#[test]
fn profiles_sync_through_the_door_when_the_server_refuses() {
    let relay = Relay::absent();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    phone.remote.clone().serve(true);
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    let door = announced.0.lock().clone().expect("the door announced");
    relay.sync.wall(true);
    for d in [&phone, &desk] {
        d.group.as_ref().unwrap().1.drop_all();
    }
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.nearby));
    let names = |d: &Device| d.core.session.settings.with_prefs(|p| p.servers.iter().map(|s| s.name.clone()).collect::<Vec<_>>()).unwrap();
    desk.core.session.settings.change(|p| StoredPrefs { servers: p.servers.iter().map(|s| SavedServer { name: "Music".into(), ..s.clone() }).collect(), ..p.clone() });
    phone.until("the rename through the door", |_| (names(&phone) == ["Music"]).then_some(()));
    relay.close();
}

/// A device whose door this one reaches may not reach this one's (a phone on an emulator's network): its
/// state still goes to it through the sync server.
#[test]
fn a_state_reaches_a_device_that_cannot_reach_this_ones_door() {
    let relay = Relay::new();
    let phone = Device::account(&relay, DeviceKind::Phone, "Phone");
    let announced = Arc::new(Announced::default());
    let desk = Device::found(&relay, ann(), DeviceKind::Desktop, "Desk", Some(announced.clone()));
    desk.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the desk's door announced");
    phone.remote.clone().watch(true);
    phone.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
    phone.until("the desk nearby", |r| r.devices().into_iter().find(|d| d.nearby));
    phone.remote.clone().watch(false);
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    desk.remote.clone().watch(true);
    let phone_id = phone.remote.id();
    desk.until("the phone playing, through the server", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.playing));
    relay.close();
}

/// A device that stopped reading another's door hears that device through the group's room again: what
/// changes there after it let go of the door reaches it, and is shown over what the door last said.
#[test]
fn a_device_that_lets_go_of_a_door_hears_through_the_group() {
    let relay = Relay::new();
    let announced = Arc::new(Announced::default());
    let phone = Device::found(&relay, ann(), DeviceKind::Phone, "Phone", Some(announced.clone()));
    let desk = Device::account(&relay, DeviceKind::Desktop, "Desk");
    phone.playing(&["s1", "s2"], 0);
    phone.remote.clone().serve(true);
    let door = announced.0.lock().clone().expect("the door announced");
    let phone_id = phone.remote.id();
    desk.remote.clone().watch(true);
    desk.remote.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
    desk.until("the phone nearby", |r| r.devices().into_iter().find(|d| d.id == phone_id && d.nearby && d.state.is_some()));
    desk.remote.clone().watch(false);
    let at = |ms: i64, playing: bool| Playing { playing, position_ms: ms, volume: Some(40), ..Default::default() };
    phone.remote.clone().played(at(9_000, false));
    relay.sim.pass(Duration::from_secs(1));
    phone.remote.clone().played(at(20_000, true));
    desk.until("the phone's latest word", |r| r.devices().into_iter().find(|d| d.id == phone_id).and_then(|d| d.state).filter(|s| s.position_ms == 20_000));
    relay.close();
}
