//! A controllable device's door: the listener and its connections, native only.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use nori_tasks::Tasks;
use nori_time::{Duration, Instant};

use parking_lot::{Mutex, MutexGuard};

use super::{signature, ROOM};
use crate::wire::{Answer, Body, DeviceState, Event, HostPage, HostRead, HostSource, Member, Outgoing, Room, HOLD_MS};

/// How far a request's time may be from the door's: a captured request cannot be sent again later.
const SKEW_MS: i64 = 60_000;

/// Connections a door serves at once; later ones are closed unread. Held polls from a few controllers and jam guests fit.
const MAX_CONNECTIONS: usize = 32;

/// Bytes of a request's line and headers; a longer head is not a command.
const HEAD_MOST: u64 = 16 << 10;

/// Replies a door keeps for controllers that have not polled them yet.
const KEPT_EVENTS: usize = 64;

/// Whether `target` (path and query, `sig` last) was signed with `secret` within [`SKEW_MS`] of `now_ms`.
fn verified(secret: &str, method: &str, target: &str, body: &[u8], now_ms: i64) -> bool {
    let Some((signed, sig)) = target.rsplit_once("&sig=") else { return false };
    let fresh = query(signed, "ts").and_then(|t| t.parse::<i64>().ok()).is_some_and(|ts| now_ms.abs_diff(ts) <= SKEW_MS as u64);
    let expected = signature(secret, method, signed, body);
    fresh && expected.len() == sig.len() && expected.bytes().zip(sig.bytes()).fold(0, |difference, (a, b)| difference | (a ^ b)) == 0
}

/// Parameter `key` of a request target, undecoded (doors are only sent unreserved characters).
fn query<'a>(target: &'a str, key: &str) -> Option<&'a str> {
    target.split_once('?')?.1.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
}

fn now_ms() -> i64 {
    nori_time::unix_ms()
}

/// What a door does with a command that came through it: (the sending device's id, the body).
pub type Handler = Box<dyn Fn(&str, Body) + Send + Sync>;

/// What a jam's guest asks of its host's provider through the door.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    Read(HostRead),
    /// Where song `id` streams from at most `kbps` (0: as it is); `anew` when the last URL was refused.
    Source { id: String, kbps: u32, anew: bool },
}

/// The host's answer to an [`Ask`].
#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    Page(Box<HostPage>),
    Source(HostSource),
}

/// Why the host did not answer an [`Ask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unanswered {
    /// A guest may not ask it (a song the jam does not play).
    NotAllowed,
    /// The host's provider could not answer.
    Failed,
}

/// What a door answers a jam guest's [`Ask`] with; called on the asking connection's thread, so it may
/// block.
pub type Asker = Box<dyn Fn(Ask) -> Result<Told, Unanswered> + Send + Sync>;

struct Inside {
    seq: u64,
    me: Member,
    /// Replies by sequence, each to one controller.
    events: VecDeque<(u64, String, Body)>,
    /// The bells of the polls held now, by hold number, each with its [`Tasks::waiting`] guard; the
    /// next change rings and drops them.
    holds: HashMap<u64, (Bell, Option<Box<dyn Send>>)>,
    holds_opened: u64,
}

impl Inside {
    /// Wakes every held poll.
    fn ring(&mut self) {
        for (_, (bell, waiting)) in self.holds.drain() {
            drop(waiting);
            ring(&bell);
        }
    }
}

struct Shared {
    secret: Mutex<String>,
    closed: AtomicBool,
    /// Connections being served, at most [`MAX_CONNECTIONS`].
    live: AtomicUsize,
    inside: Mutex<Inside>,
    handler: Handler,
    jam: Mutex<Option<LanJam>>,
    /// Keeps this device's clock ([`crate::clock::now_us`] but where a test simulates it).
    tasks: Arc<dyn Tasks>,
}

struct LanJam {
    invite: String,
    revoked: HashSet<String>,
    members: Vec<(String, Member)>,
    handler: Arc<Handler>,
    changed: Arc<dyn Fn(Vec<Member>) + Send + Sync>,
    asker: Arc<Asker>,
}

/// A controllable device's listener. Its thread blocks in `accept` (no wakeups); each connection gets a
/// thread of its own while it lasts, up to [`MAX_CONNECTIONS`].
pub struct Door {
    port: u16,
    shared: Arc<Shared>,
}

impl Door {
    /// Listens on every interface, on a port the system picks; stamps time exchanges on `tasks`' clock.
    pub fn open(me: Member, secret: String, handler: Handler, tasks: Arc<dyn Tasks>) -> std::io::Result<Door> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared { secret: Mutex::new(secret), closed: AtomicBool::new(false), live: AtomicUsize::new(0), inside: Mutex::new(Inside { seq: 1, me, events: VecDeque::new(), holds: HashMap::new(), holds_opened: 0 }), handler, jam: Mutex::new(None), tasks });
        let s = shared.clone();
        std::thread::Builder::new().name("nori-door".into()).spawn(move || accept_each(&s, listener.incoming()))?;
        Ok(Door { port, shared })
    }

    /// Rotates paired-device access without changing the independent jam credentials.
    pub fn pair_secret(&self, secret: String) {
        *self.shared.secret.lock() = secret;
        self.change(|_| {});
    }

    /// Opens a room whose invite and member credentials cannot control paired devices; its members'
    /// asks of the host's provider go to `asker`.
    pub fn open_jam(&self, invite: String, handler: Handler, changed: Box<dyn Fn(Vec<Member>) + Send + Sync>, asker: Asker) {
        *self.shared.jam.lock() = Some(LanJam { invite, revoked: HashSet::new(), members: Vec::new(), handler: Arc::new(handler), changed: Arc::from(changed), asker: Arc::new(asker) });
        self.change(|_| {});
    }

    pub fn close_jam(&self) {
        *self.shared.jam.lock() = None;
        self.change(|_| {});
    }

    pub fn kick(&self, member: &str) {
        let mut jam = self.shared.jam.lock();
        if let Some(jam) = jam.as_mut() {
            let revoked: Vec<_> = jam.members.iter().filter(|(_, m)| m.id == member).map(|(key, _)| key.clone()).collect();
            jam.revoked.extend(revoked);
            jam.members.retain(|(_, m)| m.id != member);
        }
        drop(jam);
        self.change(|_| {});
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn change(&self, f: impl FnOnce(&mut Inside)) {
        let mut i = self.shared.inside.lock();
        i.seq += 1;
        f(&mut i);
        i.ring();
    }

    /// This device's state, as polls answer it from now on.
    pub fn publish(&self, state: DeviceState) {
        self.change(|i| i.me.state = Some(state));
    }

    /// Sends `body` to controller `to` with its next poll.
    pub fn reply(&self, to: &str, body: Body) {
        self.change(|i| {
            let seq = i.seq;
            i.events.push_back((seq, to.to_string(), body));
            if i.events.len() > KEPT_EVENTS {
                i.events.pop_front();
            }
        });
    }
}

impl Drop for Door {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.inside.lock().ring();
        // Wakes the accepting thread so it sees the door closed.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// How long the door waits after an accept that failed without taking a connection (the process is out of
/// descriptors): accepting again at once would fail the same way.
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

/// Serves each accepted connection on a thread of its own, until the door closes.
fn accept_each(s: &Arc<Shared>, accepted: impl Iterator<Item = std::io::Result<TcpStream>>) {
    use std::io::ErrorKind::{ConnectionAborted, ConnectionReset, Interrupted};
    for conn in accepted {
        if s.closed.load(Ordering::Acquire) {
            break;
        }
        let conn = match conn {
            Ok(conn) => conn,
            // That connection is gone; the next is accepted at once.
            Err(e) if matches!(e.kind(), ConnectionAborted | ConnectionReset | Interrupted) => continue,
            Err(_) => {
                std::thread::sleep(ACCEPT_PAUSE);
                continue;
            }
        };
        if s.live.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            s.live.fetch_sub(1, Ordering::AcqRel);
            continue;
        }
        let s = s.clone();
        let _ = std::thread::Builder::new().name("nori-door-in".into()).spawn(move || {
            serve(&s, conn);
            s.live.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

fn serve(s: &Shared, conn: TcpStream) {
    // An idle kept-alive connection ends rather than holding its thread.
    let _ = conn.set_read_timeout(Some(Duration::from_millis(HOLD_MS as u64 * 2)));
    let Ok(mut out) = conn.try_clone() else { return };
    let mut reader = BufReader::new(conn);
    while !s.closed.load(Ordering::Acquire) {
        let Some((method, target, body)) = read_request(&mut reader) else { break };
        let received_us = s.tasks.now_us();
        // None: the controller hung up during a held poll.
        let Some((status, json)) = answer(s, &out, &method, &target, &body, received_us) else { break };
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", json.len());
        if out.write_all(head.as_bytes()).and_then(|_| out.write_all(json.as_bytes())).is_err() {
            break;
        }
    }
    let _ = out.shutdown(Shutdown::Both);
}

/// Method, target and body of the next request on a connection; None at its end or when its head
/// outgrows [`HEAD_MOST`].
fn read_request(r: &mut impl BufRead) -> Option<(String, String, Vec<u8>)> {
    let mut head = (&mut *r).take(HEAD_MOST);
    let mut line = String::new();
    let mut next_line = |line: &mut String| {
        line.clear();
        // A line the budget cut short has no newline.
        head.read_line(line).ok().filter(|_| line.ends_with('\n')).map(|_| ())
    };
    next_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut length = 0usize;
    loop {
        next_line(&mut line)?;
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().ok()?;
            }
        }
    }
    // A command is small; anything larger is not one.
    if length > 1 << 20 {
        return None;
    }
    let mut body = vec![0; length];
    r.read_exact(&mut body).ok()?;
    Some((method, target, body))
}

/// The answer to a request read at `received_us` (the device's clock) from `conn`; None when the
/// controller hung up while its poll was held.
fn answer(s: &Shared, conn: &TcpStream, method: &str, target: &str, body: &[u8], received_us: i64) -> Option<(&'static str, String)> {
    let path = target.split('?').next().unwrap_or_default();
    if path == "/rest/noriRemote.join" {
        let mut jams = s.jam.lock();
        let Some(jam) = jams.as_mut() else { return Some(("401 Unauthorized", "{}".into())) };
        if !verified(&jam.invite, method, target, body, now_ms()) || jam.members.len() >= 32 {
            return Some(("401 Unauthorized", "{}".into()));
        }
        let Some(nonce) = query(target, "nonce").filter(|nonce| nonce.len() == 64 && nonce.bytes().all(|b| b.is_ascii_hexdigit())) else { return Some(("400 Bad Request", "{}".into())) };
        let secret = super::member_secret(&jam.invite, nonce);
        if jam.revoked.contains(&secret) { return Some(("401 Unauthorized", "{}".into())); }
        if jam.members.iter().any(|(key, _)| key == &secret) { return Some(("200 OK", "{}".into())); }
        let id = super::account_tag("guest", &secret);
        let name = query(target, "name").and_then(crate::decode).unwrap_or_default();
        jam.members.push((secret.clone(), Member { id, name, kind: crate::wire::DeviceKind::Guest, state: None }));
        let changed = jam.changed.clone();
        let members = jam.members.iter().map(|(_, m)| m.clone()).collect();
        drop(jams);
        changed(members);
        return Some(("200 OK", "{}".into()));
    }
    let guest = if verified(&s.secret.lock(), method, target, body, now_ms()) { None } else {
        let jam = s.jam.lock();
        let member = jam.as_ref().and_then(|j| j.members.iter().find(|(key, _)| verified(key, method, target, body, now_ms()))).map(|(_, m)| m.id.clone());
        match member { Some(id) => Some(id), None => return Some(("401 Unauthorized", "{}".into())) }
    };
    let Some(dev) = guest.clone().or_else(|| query(target, "dev").map(str::to_string)) else { return Some(("400 Bad Request", "{}".into())) };
    match path {
        "/rest/noriRemote.poll" => {
            let since = query(target, "since").and_then(|v| v.parse::<u64>().ok());
            let hold = query(target, "hold") == Some("1");
            let mut i = s.inside.lock();
            if let (Some(since), true) = (since, hold) {
                hold_poll(s, &mut i, conn, since)?;
            }
            // A pairing reset or revoked membership can wake a held poll.
            if guest.is_none() && !verified(&s.secret.lock(), method, target, body, now_ms()) {
                return Some(("401 Unauthorized", "{}".into()));
            }
            let events = i
                .events
                .iter()
                .filter(|(seq, to, _)| since.is_some_and(|since| *seq > since) && *to == dev)
                .map(|(seq, _, body)| Event { seq: *seq, room: if guest.is_some() { "lan-jam".into() } else { ROOM.into() }, from: i.me.id.clone(), body: body.clone() })
                .collect();
            let (room, members) = if guest.is_some() {
                let jam = s.jam.lock();
                let Some(jam) = jam.as_ref() else { return Some(("401 Unauthorized", "{}".into())) };
                if !jam.members.iter().any(|(key, member)| member.id == dev && verified(key, method, target, body, now_ms())) {
                    return Some(("401 Unauthorized", "{}".into()));
                }
                ("lan-jam", std::iter::once(i.me.clone()).chain(jam.members.iter().map(|(_, m)| m.clone())).collect())
            } else { (ROOM, vec![i.me.clone()]) };
            let a = Answer { seq: i.seq, you: dev.clone(), rooms: vec![Room { room: room.into(), jam: guest.is_some(), members }], events, along: guest.is_some(), ..Default::default() };
            Some(("200 OK", serde_json::to_string(&a).unwrap_or_default()))
        }
        "/rest/noriRemote.leave" if guest.is_some() => {
            let mut jams = s.jam.lock();
            if let Some(jam) = jams.as_mut() {
                jam.members.retain(|(_, m)| m.id != dev);
                let changed = jam.changed.clone();
                let members = jam.members.iter().map(|(_, m)| m.clone()).collect();
                drop(jams);
                changed(members);
            }
            Some(("200 OK", "{}".into()))
        }
        // A time exchange (`crate::clock`), answered at once, stamped as near the socket as this gets.
        "/rest/noriRemote.time" => {
            let Some(t1) = query(target, "t1").and_then(|v| v.parse().ok()) else { return Some(("400 Bad Request", "{}".into())) };
            let times = Body::Clock { t1, t2: received_us, t3: s.tasks.now_us() };
            Some(("200 OK", serde_json::to_string(&times).unwrap_or_default()))
        }
        "/rest/noriRemote.send" if method == "POST" => {
            let Ok(out) = serde_json::from_slice::<Outgoing>(body) else { return Some(("400 Bad Request", "{}".into())) };
            if let Some(b) = out.body {
                if guest.is_some() {
                    let handler = s.jam.lock().as_ref().map(|jam| jam.handler.clone());
                    if let Some(handler) = handler { handler(&dev, b); }
                } else { (s.handler)(&dev, b); }
            }
            let seq = s.inside.lock().seq;
            Some(("200 OK", format!(r#"{{"seq":{seq}}}"#)))
        }
        "/rest/noriRemote.read" if guest.is_some() && method == "POST" => {
            let Ok(read) = serde_json::from_slice::<HostRead>(body) else { return Some(("400 Bad Request", "{}".into())) };
            Some(asked(s, Ask::Read(read)))
        }
        "/rest/noriRemote.source" if guest.is_some() => {
            let Some(id) = query(target, "id").and_then(crate::decode) else { return Some(("400 Bad Request", "{}".into())) };
            let kbps = query(target, "kbps").and_then(|v| v.parse().ok()).unwrap_or(0);
            Some(asked(s, Ask::Source { id, kbps, anew: query(target, "anew") == Some("1") }))
        }
        _ => Some(("404 Not Found", "{}".into())),
    }
}

/// The jam's answer to a guest's `ask`, as the door sends it.
fn asked(s: &Shared, ask: Ask) -> (&'static str, String) {
    let asker = s.jam.lock().as_ref().map(|jam| jam.asker.clone());
    let Some(asker) = asker else { return ("401 Unauthorized", "{}".into()) };
    match asker(ask) {
        Ok(Told::Page(page)) => ("200 OK", serde_json::to_string(&page).unwrap_or_default()),
        Ok(Told::Source(source)) => ("200 OK", serde_json::to_string(&source).unwrap_or_default()),
        Err(Unanswered::NotAllowed) => ("403 Forbidden", "{}".into()),
        Err(Unanswered::Failed) => ("502 Bad Gateway", "{}".into()),
    }
}

/// Holds a poll while the door has nothing past `since`, until it changes, closes, [`HOLD_MS`] pass or
/// the controller speaks again; None when it hung up.
fn hold_poll(s: &Shared, i: &mut MutexGuard<Inside>, conn: &TcpStream, since: u64) -> Option<()> {
    // Checked and held under one lock, so a change or close after this rings the hold.
    if i.seq > since || s.closed.load(Ordering::Acquire) {
        return Some(());
    }
    let until = Instant::now() + Duration::from_millis(HOLD_MS as u64);
    let Ok((bell, ear)) = alarm() else { return Some(()) };
    let hold = i.holds_opened;
    i.holds_opened += 1;
    i.holds.insert(hold, (bell, s.tasks.waiting()));
    let mut woke = Woke::Interrupted;
    while woke == Woke::Interrupted {
        woke = MutexGuard::unlocked(i, || wait(conn, &ear, until));
    }
    i.holds.remove(&hold);
    (woke != Woke::HungUp).then_some(())
}

/// Why a held poll woke.
#[derive(PartialEq)]
enum Woke {
    /// The door changed or closed: a ring drops the hold's bell, so it is heard once.
    Rung,
    /// A signal cut the wait short: waited again.
    #[cfg_attr(not(unix), allow(dead_code))]
    Interrupted,
    TimedOut,
    /// The controller sent more: answered now. Heard only where a hold polls the socket (unix).
    #[cfg_attr(not(unix), allow(dead_code))]
    Spoke,
    HungUp,
}

/// Rings a held poll's ear: the near end of a socket pair, written without blocking (a full buffer has
/// already rung).
#[cfg(unix)]
type Bell = std::os::unix::net::UnixStream;
#[cfg(unix)]
type Ear = std::os::unix::net::UnixStream;

#[cfg(unix)]
fn alarm() -> std::io::Result<(Bell, Ear)> {
    let (bell, ear) = std::os::unix::net::UnixStream::pair()?;
    bell.set_nonblocking(true)?;
    ear.set_nonblocking(true)?;
    Ok((bell, ear))
}

#[cfg(unix)]
fn ring(mut bell: &Bell) {
    let _ = bell.write(&[1]);
}

/// Waits for the ear to ring or `conn` to become readable (more sent, or hung up), until `until`.
#[cfg(unix)]
fn wait(conn: &TcpStream, mut ear: &Ear, until: Instant) -> Woke {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    let mut fds = [conn.as_raw_fd(), ear.as_raw_fd()].map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
    let ms = until.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: `fds` is a valid array of two pollfds for the call's length.
    match unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) } {
        0 => Woke::TimedOut,
        n if n < 0 => Woke::Interrupted,
        _ if fds[0].revents != 0 => match conn.peek(&mut [0]) {
            Ok(n) if n > 0 => Woke::Spoke,
            _ => Woke::HungUp,
        },
        _ => {
            while ear.read(&mut [0; 16]).is_ok_and(|n| n > 0) {}
            Woke::Rung
        }
    }
}

/// Elsewhere a hold hears only the door: a controller hanging up is noticed when the hold ends.
#[cfg(not(unix))]
type Bell = std::sync::mpsc::Sender<()>;
#[cfg(not(unix))]
type Ear = std::sync::mpsc::Receiver<()>;

#[cfg(not(unix))]
fn alarm() -> std::io::Result<(Bell, Ear)> {
    Ok(std::sync::mpsc::channel())
}

#[cfg(not(unix))]
fn ring(bell: &Bell) {
    let _ = bell.send(());
}

#[cfg(not(unix))]
fn wait(_: &TcpStream, ear: &Ear, until: Instant) -> Woke {
    match ear.recv_timeout(until.saturating_duration_since(Instant::now())) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Woke::TimedOut,
        _ => Woke::Rung,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{account_tag, signed};
    use crate::wire::Op;
    use std::io::Read;

    #[test]
    fn requests_need_the_account_secret_and_a_fresh_time() {
        let t = signed("sesame", "POST", "/rest/noriRemote.send?dev=a", b"{}", 1_000_000);
        assert!(verified("sesame", "POST", &t, b"{}", 1_000_000 + SKEW_MS));
        assert!(!verified("other", "POST", &t, b"{}", 1_000_000), "another secret");
        assert!(!verified("sesame", "POST", &t, b"{\"x\":1}", 1_000_000), "another body");
        assert!(!verified("sesame", "GET", &t, b"{}", 1_000_000), "another method");
        assert!(!verified("sesame", "POST", &t.replace("dev=a", "dev=b"), b"{}", 1_000_000), "another device");
        assert!(!verified("sesame", "POST", &t, b"{}", 1_000_001 + SKEW_MS), "too old");
        assert_eq!(account_tag("ann", "pw"), account_tag("ann", "pw"));
        assert_ne!(account_tag("ann", "pw"), account_tag("ann", "pw2"));
        assert_ne!(account_tag("ann", "pw"), account_tag("bob", "pw"));
    }

    fn request(port: u16, secret: &str, method: &str, path_query: &str, body: &str) -> (u16, String) {
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let target = signed(secret, method, path_query, body.as_bytes(), now_ms());
        write!(c, "{method} {target} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut r = BufReader::new(c);
        let mut status = String::new();
        r.read_line(&mut status).unwrap();
        let mut length = 0;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).unwrap();
            if h.trim().is_empty() {
                break;
            }
            if let Some(v) = h.strip_prefix("Content-Length: ") {
                length = v.trim().parse().unwrap();
            }
        }
        let mut b = vec![0; length];
        r.read_exact(&mut b).unwrap();
        (status.split_whitespace().nth(1).unwrap().parse().unwrap(), String::from_utf8(b).unwrap())
    }

    #[test]
    fn a_request_head_past_its_budget_is_refused() {
        let mut huge = b"GET /rest/noriRemote.poll HTTP/1.1\r\nX-Pad: ".to_vec();
        huge.extend(std::iter::repeat_n(b'a', 1 << 20));
        huge.extend_from_slice(b"\r\n\r\n");
        assert!(read_request(&mut std::io::Cursor::new(huge)).is_none());
        let small = b"POST /x?a=b HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi".to_vec();
        assert_eq!(read_request(&mut std::io::Cursor::new(small)), Some(("POST".into(), "/x?a=b".into(), b"hi".to_vec())));
    }

    #[test]
    fn connections_past_the_cap_are_closed_at_once() {
        let door = Door::open(Member { id: "host".into(), ..Default::default() }, "pw".into(), Box::new(|_, _| {}), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
        let held: Vec<_> = (0..MAX_CONNECTIONS).map(|_| TcpStream::connect(("127.0.0.1", door.port())).unwrap()).collect();
        let mut over = TcpStream::connect(("127.0.0.1", door.port())).unwrap();
        over.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(over.read(&mut [0; 1]).map_err(|e| e.kind()), Ok(0));
        drop(held);
    }

    #[test]
    fn held_polls_recheck_access() {
        for guest in [false, true] {
            let door = Door::open(Member { id: "host".into(), ..Default::default() }, "paired-secret".into(), Box::new(|_, _| {}), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
            let secret = if guest {
                door.open_jam("invite-secret".into(), Box::new(|_, _| {}), Box::new(|_| {}), Box::new(|_| Err(Unanswered::Failed)));
                let nonce = "a".repeat(64);
                let join = format!("/rest/noriRemote.join?name=Guest&nonce={nonce}");
                assert_eq!(request(door.port(), "invite-secret", "GET", &join, "").0, 200);
                super::super::member_secret("invite-secret", &nonce)
            } else { "paired-secret".into() };
            let (_, body) = request(door.port(), &secret, "GET", "/rest/noriRemote.poll?dev=own", "");
            let first: Answer = serde_json::from_str(&body).unwrap();
            let port = door.port();
            let held = std::thread::spawn(move || request(port, &secret, "GET", &format!("/rest/noriRemote.poll?dev=own&since={}&hold=1", first.seq), ""));
            let until = Instant::now() + Duration::from_secs(5);
            while door.shared.inside.lock().holds.is_empty() {
                assert!(Instant::now() < until, "poll reached its hold");
                std::thread::yield_now();
            }
            if guest { door.kick(&first.you); } else { door.pair_secret("new-paired-secret".into()); }
            assert_eq!(held.join().unwrap().0, 401, "held poll is revoked, guest={guest}");
        }
    }

    #[test]
    fn failing_accepts_wait_and_end_with_the_door() {
        let door = Door::open(Member::default(), "pw".into(), Box::new(|_, _| {}), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
        let shared = door.shared.clone();
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let out_of_files = std::iter::from_fn(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Some(Err(std::io::Error::other("too many open files")))
        });
        let accepting = std::thread::spawn(move || accept_each(&shared, out_of_files));
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(door);
        accepting.join().unwrap();
        assert!(tries.load(Ordering::Relaxed) <= 5, "{} accepts", tries.load(Ordering::Relaxed));
    }

    #[test]
    fn a_held_poll_ends_at_the_next_change() {
        let door = Door::open(Member { id: "desk".into(), ..Default::default() }, "pw".into(), Box::new(|_, _| {}), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
        let port = door.port();
        // A cursor past the door's own: no change reaches it.
        let (answered, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || answered.send(request(port, "pw", "GET", &format!("/rest/noriRemote.poll?dev=phone&since={}&hold=1", u64::MAX), "").0));
        let until = Instant::now() + Duration::from_secs(5);
        while door.shared.inside.lock().holds.is_empty() {
            assert!(Instant::now() < until, "poll reached its hold");
            std::thread::yield_now();
        }
        door.publish(DeviceState { playing: true, ..Default::default() });
        assert_eq!(answer.recv_timeout(Duration::from_secs(5)), Ok(200), "answered at the change");
    }

    #[test]
    fn jam_credentials_are_scoped_and_revoked() {
        let device_commands = Arc::new(Mutex::new(Vec::new()));
        let jam_commands = Arc::new(Mutex::new(Vec::new()));
        let members = Arc::new(Mutex::new(Vec::new()));
        let got = device_commands.clone();
        let door = Door::open(Member { id: "host".into(), ..Default::default() }, "paired-secret".into(), Box::new(move |from, body| got.lock().push((from.to_owned(), body))), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
        let got = jam_commands.clone();
        let listed = members.clone();
        door.open_jam("invite-secret".into(), Box::new(move |from, body| got.lock().push((from.to_owned(), body))), Box::new(move |m| *listed.lock() = m), Box::new(|_| Err(Unanswered::Failed)));
        let nonce = "a".repeat(64);
        let join = format!("/rest/noriRemote.join?name=Guest&nonce={nonce}");
        assert_eq!(request(door.port(), "wrong", "GET", &join, "").0, 401);
        let (status, body) = request(door.port(), "invite-secret", "GET", &join, "");
        assert_eq!(status, 200);
        let secret = super::super::member_secret("invite-secret", &nonce);
        assert!(!body.contains(&secret), "member credentials are derived, never sent");
        assert_eq!(request(door.port(), "invite-secret", "GET", "/rest/noriRemote.poll?dev=pretender", "").0, 401, "invite cannot control devices or poll");
        let (_, body) = request(door.port(), &secret, "GET", "/rest/noriRemote.poll?dev=host", "");
        let answer: Answer = serde_json::from_str(&body).unwrap();
        assert!(answer.rooms[0].jam && answer.along);
        assert_ne!(answer.you, "host", "identity comes from the credential");
        assert_eq!(members.lock().len(), 1);
        let command = serde_json::to_string(&Outgoing { body: Some(Body::Command { id: 1, op: Box::new(Op::Pause) }), ..Default::default() }).unwrap();
        assert_eq!(request(door.port(), &secret, "POST", "/rest/noriRemote.send?dev=host", &command).0, 200);
        assert!(device_commands.lock().is_empty(), "guests only reach the jam permission boundary");
        assert_eq!(jam_commands.lock()[0].0, answer.you);
        door.pair_secret("new-paired-secret".into());
        assert_eq!(request(door.port(), "paired-secret", "GET", "/rest/noriRemote.poll?dev=own", "").0, 401);
        assert_eq!(request(door.port(), &secret, "GET", "/rest/noriRemote.poll?dev=guest", "").0, 200, "pair rotation preserves the separate jam");
        door.kick(&answer.you);
        assert_eq!(request(door.port(), &secret, "GET", "/rest/noriRemote.poll?dev=guest", "").0, 401);
        assert_eq!(request(door.port(), "invite-secret", "GET", &join, "").0, 401, "replaying the join cannot revive a revoked member");
        door.close_jam();
        assert_eq!(request(door.port(), &secret, "GET", "/rest/noriRemote.poll?dev=guest", "").0, 401);
        assert_eq!(request(door.port(), "invite-secret", "GET", &join, "").0, 401);
        assert_eq!(request(door.port(), "new-paired-secret", "GET", "/rest/noriRemote.poll?dev=own", "").0, 200);
    }

    #[test]
    fn only_jam_members_ask_the_hosts_provider() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = asked.clone();
        let door = Door::open(Member { id: "host".into(), ..Default::default() }, "paired-secret".into(), Box::new(|_, _| {}), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
        let source = HostSource { url: "https://cdn.example/a".into(), parts: Some(crate::wire::SourceParts { len: 9, most: 4 }) };
        let told = source.clone();
        door.open_jam("invite-secret".into(), Box::new(|_, _| {}), Box::new(|_| {}), Box::new(move |ask| {
            heard.lock().push(ask.clone());
            match ask {
                Ask::Read(_) => Ok(Told::Page(Box::new(HostPage::Song { song: None }))),
                Ask::Source { id, .. } if id == "a b" => Ok(Told::Source(told.clone())),
                Ask::Source { .. } => Err(Unanswered::NotAllowed),
            }
        }));
        let port = door.port();
        let nonce = "b".repeat(64);
        assert_eq!(request(port, "invite-secret", "GET", &format!("/rest/noriRemote.join?name=Guest&nonce={nonce}"), "").0, 200);
        let member = super::super::member_secret("invite-secret", &nonce);
        let read = serde_json::to_string(&HostRead::Song { id: "s".into() }).unwrap();
        let of = |id: &str| super::super::target("noriRemote.source", &[("id".into(), id.into()), ("kbps".into(), "160".into())]);
        for outsider in ["invite-secret", "paired-secret", "wrong"] {
            assert_ne!(request(port, outsider, "POST", "/rest/noriRemote.read?dev=host", &read).0, 200, "{outsider} reads");
            assert_ne!(request(port, outsider, "GET", &format!("{}&dev=host", of("a b")), "").0, 200, "{outsider} asks for a source");
        }
        assert!(asked.lock().is_empty(), "only members reach the host's provider");
        let (status, page) = request(port, &member, "POST", "/rest/noriRemote.read", &read);
        assert_eq!((status, serde_json::from_str::<HostPage>(&page).unwrap()), (200, HostPage::Song { song: None }));
        let (status, got) = request(port, &member, "GET", &of("a b"), "");
        assert_eq!((status, serde_json::from_str::<HostSource>(&got).unwrap()), (200, source));
        assert_eq!(request(port, &member, "GET", &of("elsewhere"), "").0, 403, "the host's refusal");
        assert_eq!(asked.lock()[..2], [Ask::Read(HostRead::Song { id: "s".into() }), Ask::Source { id: "a b".into(), kbps: 160, anew: false }]);
        door.close_jam();
        assert_eq!(request(port, &member, "POST", "/rest/noriRemote.read", &read).0, 401, "the jam is over");
    }

    #[test]
    fn a_door_takes_commands_and_answers_polls() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let me = Member { id: "desk".into(), name: "Desk".into(), ..Default::default() };
        let door = Door::open(me, "pw".into(), Box::new(move |from, body| g.lock().push((from.to_string(), body))), Arc::new(nori_tasks::Native::new("door-test").unwrap())).unwrap();
        door.publish(DeviceState { playing: true, rev: 3, ..Default::default() });
        door.reply("tablet", Body::Ack { id: 9, refusal: None });

        let send = serde_json::to_string(&Outgoing { body: Some(Body::Command { id: 1, op: Box::new(Op::Pause) }), ..Default::default() }).unwrap();
        assert_eq!(request(door.port(), "wrong", "POST", "/rest/noriRemote.send?dev=phone", &send).0, 401);
        assert!(got.lock().is_empty());
        assert_eq!(request(door.port(), "pw", "POST", "/rest/noriRemote.send?dev=phone", &send).0, 200);
        assert_eq!(*got.lock(), [("phone".to_string(), Body::Command { id: 1, op: Box::new(Op::Pause) })]);

        let (_, first) = request(door.port(), "pw", "GET", "/rest/noriRemote.poll?dev=phone", "");
        let first: Answer = serde_json::from_str(&first).unwrap();
        assert_eq!(first.rooms[0].members[0].state.as_ref().map(|s| s.rev), Some(3));

        // A held poll comes back with the reply as soon as there is one.
        let port = door.port();
        let since = first.seq;
        let held = std::thread::spawn(move || request(port, "pw", "GET", &format!("/rest/noriRemote.poll?dev=phone&since={since}&hold=1"), "").1);
        door.reply("phone", Body::Ack { id: 1, refusal: None });
        let a: Answer = serde_json::from_str(&held.join().unwrap()).unwrap();
        let bodies: Vec<&Body> = a.events.iter().map(|e| &e.body).collect();
        assert_eq!(bodies, [&Body::Ack { id: 1, refusal: None }], "only its own replies");

        // A controller hanging up ends its held poll at once: the door closes its end without answering.
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let target = signed("pw", "GET", &format!("/rest/noriRemote.poll?dev=phone&since={}&hold=1", a.seq), b"", now_ms());
        write!(c, "GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        c.shutdown(Shutdown::Write).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut rest = Vec::new();
        assert_eq!(c.read_to_end(&mut rest).map_err(|e| e.kind()), Ok(0), "closed unanswered, before the hold's end");

        // A time exchange is stamped on this machine's clock, between the request and its answer.
        let t1 = crate::clock::now_us();
        let (status, times) = request(door.port(), "pw", "GET", &format!("/rest/noriRemote.time?dev=phone&t1={t1}"), "");
        let t4 = crate::clock::now_us();
        match serde_json::from_str::<Body>(&times).unwrap() {
            Body::Clock { t1: echoed, t2, t3 } => assert!(status == 200 && echoed == t1 && t1 <= t2 && t2 <= t3 && t3 <= t4, "{t1} {t2} {t3} {t4}"),
            b => panic!("{b:?}"),
        }
    }
}
