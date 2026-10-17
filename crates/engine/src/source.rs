//! A song's bytes over the network, fetched in bursts through the client's [`ByteSource`]. One loader
//! thread per song fills a window ahead of the demuxer up to the `load_control` high mark, closes the
//! connection, and sleeps until the reader comes within the low mark. Most songs fit the memory cap and
//! arrive in one burst. The cap is shared: a song fetched ahead gets what the playing one leaves
//! (`Loader::limit`). With a stream cache entry, each burst is written to it; once the entry is whole
//! the loader drops its memory copy and reads the file.
//!
//! A live stream keeps one connection open and a window that moves with the reader ([`Loader::live`]);
//! ICY `StreamTitle`s are taken out of the bytes and said once the reader passes them
//! ([`Loader::announced`]).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Context, Poll, Waker};
use std::thread::Thread;
use nori_time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;

use crate::arriving::Listening;
use crate::store::Writer;

/// Makes the stream cache entry a loader writes into, on the loader's thread (it may wait for the
/// fetching ahead to hand the song over, `Store::writer_for_player`). A partly written entry is continued.
pub type Keep = Box<dyn FnOnce() -> Option<Writer> + Send>;

/// A response body: its offset in the resource (0 when the server ignores ranges), the resource's
/// length when known, and the bytes.
pub struct Body {
    pub start: u64,
    pub len: Option<u64>,
    pub reader: Box<dyn Read + Send>,
}

/// Why [`ByteSource::open`] gave no body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// Nothing at or after `from` (HTTP 416, or a rangeless answer shorter than `from`); `len` from
    /// `Content-Range: bytes */N` if given. Not a failure: how a transcode's real end is learned when
    /// its promised length was an estimate (Navidrome's `estimateContentLength`).
    PastEnd { len: Option<u64> },
    /// The server refused with this status (retried; not the network's failure).
    Status(u16),
    /// Unreachable now (retried).
    Failed(String),
    /// No answer or next byte within [`Waits::stall_ms`]. Fails a song whose first bytes never come;
    /// a body that stalls half way is asked again from where it stopped.
    TimedOut,
}

impl From<String> for OpenError {
    fn from(why: String) -> OpenError {
        OpenError::Failed(why)
    }
}

impl From<&str> for OpenError {
    fn from(why: &str) -> OpenError {
        OpenError::Failed(why.to_string())
    }
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::PastEnd { len: Some(l) } => write!(f, "past the end, which is at byte {l}"),
            OpenError::PastEnd { len: None } => f.write_str("past the end"),
            OpenError::Status(status) => write!(f, "HTTP {status}"),
            OpenError::Failed(why) => f.write_str(why),
            OpenError::TimedOut => f.write_str("no answer in time"),
        }
    }
}

// ---- requests called off ----

/// Real-time limits of a song's fetch ([`crate::Clock::waits`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waits {
    /// A request with no answer, or no next byte, for this long is called off ([`OpenError::TimedOut`]).
    /// Well under [`READ_TIMEOUT`], so a server that never answers fails as the network's failure.
    pub stall_ms: u64,
    /// A dropped connection is tried again after twice this, doubled for each retry after it.
    pub retry_ms: u64,
}

impl Default for Waits {
    fn default() -> Self {
        Waits { stall_ms: 20_000, retry_ms: 500 }
    }
}

/// Cancels a running request from another thread: its song was let go, it stalled, or newer requests
/// crowded it out ([`MAX_ASKING`]). The client's HTTP stack registers [`Cancel::on_cancel`] or polls
/// [`Cancel::cancelled`]. One per loader, reset for each request ([`Cancel::begin`]).
#[derive(Clone)]
pub struct Cancel(Arc<Mutex<CallState>>);

/// Why a request was called off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Off {
    Asked,
    Stalled,
}

struct CallState {
    /// The song was let go: every request of it is off.
    closed: bool,
    /// This request is off, and why.
    off: Option<Off>,
    /// How the client cancels the running request.
    hook: Option<Box<dyn FnOnce() + Send>>,
    /// Called off at this moment unless it moves first.
    deadline: Option<Instant>,
    watched: bool,
    stall_ms: u64,
    /// Whose watch calls it off when it stalls, and counts it among the requests asking; none for a
    /// request only cancelled by hand.
    fetching: Weak<Fetching>,
}

impl Default for Cancel {
    fn default() -> Self {
        Cancel::of(Weak::new(), Waits::default().stall_ms)
    }
}

impl Cancel {
    /// A request only cancelled by hand.
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// A request of `fetching`'s, called off after `ms` without an answer or a byte.
    pub(crate) fn stalling_after(fetching: &Arc<Fetching>, ms: u64) -> Cancel {
        Cancel::of(Arc::downgrade(fetching), ms)
    }

    fn of(fetching: Weak<Fetching>, stall_ms: u64) -> Cancel {
        let s = CallState { closed: false, off: None, hook: None, deadline: None, watched: false, stall_ms, fetching };
        Cancel(Arc::new(Mutex::new(s)))
    }

    fn fetching(&self) -> Option<Arc<Fetching>> {
        self.0.lock().fetching.upgrade()
    }

    /// Whether the request is off (for an HTTP stack that polls).
    pub fn cancelled(&self) -> bool {
        let s = self.0.lock();
        s.closed || s.off.is_some()
    }

    /// Whether it was called off for stalling.
    pub fn timed_out(&self) -> bool {
        self.0.lock().off == Some(Off::Stalled)
    }

    /// Registers how to cancel the running request; runs it at once if already off. Replaces the last.
    pub fn on_cancel(&self, off: impl FnOnce() + Send + 'static) {
        let mut s = self.0.lock();
        if s.closed || s.off.is_some() {
            drop(s);
            off();
            return;
        }
        s.hook = Some(Box::new(off));
    }

    /// A new request begins: clears the last one's cancellation (not a closed song) and arms the stall.
    pub(crate) fn begin(&self) {
        self.stalls(true);
    }

    /// The request moved (an answer, a byte): re-arms the stall.
    pub(crate) fn moved(&self) {
        self.stalls(false);
    }

    /// Arms the stall deadline. `anew`: a new request, cleared under the same lock so the last one's
    /// cancellation cannot land on it.
    fn stalls(&self, anew: bool) {
        let mut s = self.0.lock();
        if anew {
            s.off = None;
            s.hook = None;
        }
        s.deadline = Some(Instant::now() + Duration::from_millis(s.stall_ms));
        if !s.watched {
            let Some(fetching) = s.fetching.upgrade() else { return };
            s.watched = true;
            drop(s);
            fetching.watch(Arc::downgrade(&self.0));
        }
    }

    /// The request is over (its body dropped).
    pub(crate) fn end(&self) {
        let mut s = self.0.lock();
        s.hook = None;
        s.deadline = None;
    }

    /// Cancels the running request.
    pub(crate) fn call_off(&self, stalled: bool) {
        let hook = {
            let mut s = self.0.lock();
            if stalled || s.off.is_none() {
                s.off = Some(if stalled { Off::Stalled } else { Off::Asked });
            }
            s.deadline = None;
            s.hook.take()
        };
        if let Some(h) = hook {
            h();
        }
    }

    /// The song was let go: cancels this and every later request.
    pub(crate) fn close(&self) {
        self.0.lock().closed = true;
        self.call_off(false);
    }

    pub(crate) fn closed(&self) -> bool {
        self.0.lock().closed
    }
}

/// One owner's song fetches (an engine's loaders, a fetch-ahead): the watch that calls off stalled
/// requests, on one sleeping thread alive only while a request has a deadline; the requests waiting for
/// an answer, by source ([`MAX_ASKING`]); and the loaders alive, for the memory report ([`Fetching::held`]).
#[derive(Default)]
pub struct Fetching {
    watch: Mutex<Watch>,
    watch_cv: Condvar,
    /// The requests waiting for an answer, oldest first, by the source they go to.
    asking: Mutex<Vec<(usize, Cancel)>>,
    alive: Mutex<Vec<Weak<Loaded>>>,
}

/// Requests with a stall deadline, and whether the thread that cancels stalled ones runs.
#[derive(Default)]
struct Watch {
    calls: Vec<Weak<Mutex<CallState>>>,
    running: bool,
}

/// Requests to one [`ByteSource`] waiting for an answer at once. A server that never answers must not
/// take every request the client's HTTP stack lets run to it (OkHttp's per-host cap): past this many the
/// oldest still waiting is called off, the newest being the one wanted. Each source keeps under its own
/// cap (the fetch-ahead asks once at a time, so the player's and its stay within OkHttp's five per host).
const MAX_ASKING: usize = 4;

/// What the loaders hold, for the perf report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Held {
    /// Loaders alive.
    pub songs: u32,
    /// Bytes allocated for them.
    pub bytes: u64,
    /// Of them, those reading from disk.
    pub on_disk: u32,
}

impl Fetching {
    fn watch(self: &Arc<Self>, call: Weak<Mutex<CallState>>) {
        let mut w = self.watch.lock();
        w.calls.push(call);
        if w.running {
            self.watch_cv.notify_all();
            return;
        }
        w.running = true;
        drop(w);
        let fetching = self.clone();
        if std::thread::Builder::new().name("nori-stall".into()).spawn(move || fetching.watching()).is_err() {
            self.watch.lock().running = false;
        }
    }

    fn watching(&self) {
        let mut w = self.watch.lock();
        loop {
            let now = Instant::now();
            let mut due = Vec::new();
            let mut next: Option<Instant> = None;
            w.calls.retain(|c| {
                let Some(c) = c.upgrade() else { return false };
                let mut s = c.lock();
                match s.deadline {
                    Some(at) if at <= now => {
                        due.push(c.clone());
                        s.watched = false;
                        false
                    }
                    Some(at) => {
                        next = Some(next.map_or(at, |n| n.min(at)));
                        true
                    }
                    None => {
                        s.watched = false;
                        false
                    }
                }
            });
            if !due.is_empty() {
                drop(w);
                for c in due {
                    Cancel(c).call_off(true);
                }
                w = self.watch.lock();
                continue;
            }
            match next {
                Some(at) => {
                    self.watch_cv.wait_for(&mut w, at.saturating_duration_since(Instant::now()));
                }
                None => {
                    w.running = false;
                    return;
                }
            }
        }
    }

    /// `cancel`'s request to `source` is about to be made: it counts until [`Fetching::answered`], and the
    /// oldest waiting on the same source is called off when there are too many.
    fn asking(&self, source: usize, cancel: &Cancel) {
        let crowded = {
            let mut a = self.asking.lock();
            a.retain(|(_, c)| !c.cancelled());
            a.push((source, cancel.clone()));
            let same = a.iter().filter(|(s, _)| *s == source).count();
            (same > MAX_ASKING).then(|| a.iter().position(|(s, _)| *s == source).map(|k| a.remove(k).1)).flatten()
        };
        if let Some(c) = crowded {
            c.call_off(false);
        }
    }

    fn answered(&self, cancel: &Cancel) {
        self.asking.lock().retain(|(_, c)| !Arc::ptr_eq(&c.0, &cancel.0));
    }

    fn alive(&self, loaded: &Arc<Loaded>) {
        let mut all = self.alive.lock();
        all.retain(|w| w.strong_count() > 0);
        all.push(Arc::downgrade(loaded));
    }

    /// What every loader alive holds now.
    pub fn held(&self) -> Held {
        let all: Vec<Arc<Loaded>> = self.alive.lock().iter().filter_map(Weak::upgrade).collect();
        let mut h = Held::default();
        for l in all {
            let s = l.state.lock();
            h.songs += 1;
            h.bytes += s.data.capacity() as u64;
            h.on_disk += s.disk.is_some() as u32;
        }
        h
    }
}

/// Opens `url` from `from` through `source` (cache `key` if it keeps what it reads) as `cancel`'s
/// request, watched for stalls and crowding while it opens and while its body is read.
pub(crate) fn open_watched(source: &dyn ByteSource, url: &str, key: Option<&str>, from: u64, cancel: &Cancel) -> Result<Body, OpenError> {
    if cancel.closed() {
        return Err("the song is no longer wanted".into());
    }
    cancel.begin();
    let fetching = cancel.fetching();
    if let Some(f) = &fetching {
        f.asking(source as *const dyn ByteSource as *const () as usize, cancel);
    }
    let opened = source.open_cancellable(url, key, from, cancel);
    if let Some(f) = &fetching {
        f.answered(cancel);
    }
    let stalled = cancel.timed_out();
    match opened {
        Ok(mut b) if !cancel.cancelled() => {
            cancel.moved();
            b.reader = Box::new(Watched { inner: b.reader, cancel: cancel.clone() });
            Ok(b)
        }
        Ok(_) if stalled => Err(OpenError::TimedOut),
        Ok(_) => Err("called off".into()),
        // Cancelled for stalling, whatever error the client made of it.
        Err(OpenError::Failed(_)) if stalled => Err(OpenError::TimedOut),
        Err(e) => {
            cancel.end();
            Err(e)
        }
    }
}

/// A body under watch: each read re-arms the stall; the end disarms it.
struct Watched {
    inner: Box<dyn Read + Send>,
    cancel: Cancel,
}

impl Read for Watched {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let got = self.inner.read(buf);
        match &got {
            Ok(0) => self.cancel.end(),
            Ok(_) => self.cancel.moved(),
            Err(_) if self.cancel.timed_out() => return Err(io::Error::new(io::ErrorKind::TimedOut, OpenError::TimedOut.to_string())),
            Err(_) => {}
        }
        got
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.cancel.end();
    }
}

/// The client's HTTP for audio: GET `url` from byte `from` (a `Range` request), blocking on the loader's
/// thread. A range at or past the end is [`OpenError::PastEnd`].
pub trait ByteSource: Send + Sync {
    fn open(&self, url: &str, from: u64) -> Result<Body, OpenError>;

    /// [`ByteSource::open`] cached under `key` by a client whose HTTP stack caches (media3 on Android).
    fn open_keyed(&self, url: &str, _key: &str, from: u64) -> Result<Body, OpenError> {
        self.open(url, from)
    }

    /// [`ByteSource::open`] (or `open_keyed`) that `cancel` can abort: the client should hook
    /// [`Cancel::on_cancel`] so a hung request frees its connection and dispatcher slot. By default the
    /// request is not cancellable.
    fn open_cancellable(&self, url: &str, key: Option<&str>, from: u64, _cancel: &Cancel) -> Result<Body, OpenError> {
        match key {
            Some(k) => self.open_keyed(url, k, from),
            None => self.open(url, from),
        }
    }

    /// A live stream with `Icy-MetaData: 1`: the body and the answer's `icy-metaint`, if any.
    fn open_live(&self, url: &str) -> Result<(Body, Option<usize>), String> {
        self.open(url, 0).map(|b| (b, None)).map_err(|e| e.to_string())
    }

    /// Drops the copy of `url` the client's own cache holds (media3's on Android); true if one went.
    fn forget(&self, _url: &str) -> bool {
        false
    }

    /// Takes a loader's fetching to run on the platform's own events, where nothing may block (the
    /// web's fetch). Given back, the loader reads [`ByteSource::open`]'s bodies on a thread of its own.
    fn push(&self, fill: Filling) -> Option<Filling> {
        Some(fill)
    }
}

/// Fill up to `high` bytes ahead of the reader, start again below `low`, hold at most `cap`
/// (from `nori_player::transport::load_control`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub low: u64,
    pub high: u64,
    pub cap: u64,
}

/// Bytes per ms assumed when unknown (320 kbps, erring towards fetching more).
const BYTES_PER_MS_GUESS: u64 = 40;
/// Read size: as much as makes a song ready ([`READY`]), so a burst is a few large reads.
const CHUNK: usize = 256 * 1024;
/// A reader this far past what is loaded has seeked: fetch from there.
const FAR: u64 = 1024 * 1024;
/// Bytes ahead that make a song ready to read without waiting.
pub(crate) const READY: u64 = 256 * 1024;
/// A song read as it arrives ([`ByteSource::push`]) is ready sooner when the rate it arrives at keeps
/// ahead of the music ([`arrives_in_time`]): with this much of it at hand, and at least [`START_BYTES`]
/// for its header and first packets.
const START_MS: f64 = 1_000.0;
const START_BYTES: u64 = 16 * 1024;
/// The rate is known once a burst has brought bytes for this long.
const RATE_MS: f64 = 250.0;
/// The rest is counted as arriving this much slower than measured.
const RATE_MARGIN: f64 = 1.25;

/// Whether a song read as it arrives can be played from now without running dry: `ahead` bytes at
/// hand (at least [`START_MS`] of music and [`START_BYTES`]), and the music at hand and still to come
/// (`rest` bytes) lasts as long as the rest takes to arrive at `rate`, with [`RATE_MARGIN`]. `per_ms`:
/// the song's bytes per ms of music; `rate`: bytes per ms arriving.
fn arrives_in_time(ahead: u64, rest: u64, per_ms: f64, rate: f64) -> bool {
    if per_ms <= 0.0 || rate <= 0.0 || ahead < ((per_ms * START_MS) as u64).max(START_BYTES) {
        return false;
    }
    (ahead + rest) as f64 / per_ms >= rest as f64 / rate * RATE_MARGIN
}
/// A dropped connection is tried again this many times before the song counts as failed.
const RETRIES: u32 = 3;
/// How long a read waits for bytes before failing.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A live stream is ready with this much: two seconds at 128 kbps.
const LIVE_READY: u64 = 32 * 1024;
/// Live read size, below [`LIVE_READY`]: a reader that fills the whole buffer (Android's) returns as
/// the station sends, in real time.
const LIVE_CHUNK: usize = 16 * 1024;
/// A live stream keeps this much behind the reader (for the demuxer) and at most this much ahead
/// (reached only while paused; the connection then waits in the socket).
const LIVE_BEHIND: u64 = 256 * 1024;
const LIVE_AHEAD: u64 = 2 * 1024 * 1024;

impl Window {
    /// The window for a song of `duration_ms` and `len` bytes (either may be unknown), from
    /// `load_control` (min ms, max ms, .., .., byte cap).
    pub fn for_song(load: [i64; 5], duration_ms: Option<i64>, len: Option<u64>) -> Window {
        let per_ms = match (duration_ms, len) {
            (Some(d), Some(l)) if d > 0 => (l / d as u64).max(1),
            _ => BYTES_PER_MS_GUESS,
        };
        let cap = load[4].max(1) as u64;
        Window { low: (load[0] as u64 * per_ms).min(cap / 2), high: (load[1] as u64 * per_ms).min(cap), cap }
    }
}

#[derive(Default)]
struct State {
    /// Bytes `base..base + data.len()` of the resource; what the reader left behind goes from the front
    /// without moving the rest.
    base: u64,
    data: VecDeque<u8>,
    /// The whole song's cache entry, read instead of `data` (then empty).
    disk: Option<Arc<File>>,
    len: Option<u64>,
    /// Loaded to the end.
    done: bool,
    error: Option<String>,
    /// The server's error status, if it answered.
    answered: Option<u16>,
    /// Where the demuxer reads.
    reader_at: u64,
    /// The reader wants bytes from here, outside what is kept.
    restart: Option<u64>,
    closed: bool,
    /// The engine thread to wake when ready.
    waiter: Option<Thread>,
    /// Readers blocked in a read. A count: one giving up must not leave another unwoken.
    blocked: u32,
    /// Connections opened: one per burst.
    bursts: u32,
    window: Option<Window>,
    /// Bytes held at most, below the window's cap (`Loader::limit`).
    budget: Option<u64>,
    /// A live stream.
    live: bool,
    /// ICY titles not yet passed by the reader (with their byte), and the last one passed.
    titles: VecDeque<(u64, String)>,
    announced: Option<String>,
    /// Times the length turned out shorter than promised ([`Loader::shortened`]).
    shortened: u32,
    /// The song ends before this byte ([`State::not_at`]): a promised length at or past it is ignored.
    before: Option<u64>,
    /// Fetched by the platform ([`ByteSource::push`]): readers never block, and it is woken through
    /// `pusher` when the next step may have changed.
    pushed: bool,
    pusher: Option<Waker>,
    /// A pushed loader's earlier windows, kept while a song opens on it and until the reader reads the
    /// window again: an opening tried again finds what the last try read, wherever it was.
    pieces: Vec<(u64, VecDeque<u8>)>,
    /// Openings under way on a pushed loader ([`Loader::opening`]).
    opening: u32,
    /// Where the reader reads an earlier window; `reader_at` is where it last read the window.
    in_piece: Option<u64>,
    /// The song's bytes per ms of music, from its length and tagged duration.
    per_ms: Option<f64>,
    /// The running burst's first bytes: when they came, and the end then (its rate is measured from
    /// there).
    burst_from: Option<(Instant, u64)>,
}

impl State {
    /// The resource ends at `at`, shortening a promised length.
    fn ends_at(&mut self, at: u64) {
        match self.len {
            Some(l) if l <= at => {}
            Some(_) => {
                self.len = Some(at);
                self.shortened += 1;
            }
            None => self.len = Some(at),
        }
    }

    /// The resource ends somewhere before `at`: drops a promised length at or past it.
    fn not_at(&mut self, at: u64) {
        self.before = Some(self.before.map_or(at, |b| b.min(at)));
        if self.len.is_some_and(|l| l >= at) {
            self.len = None;
            self.shortened += 1;
        }
    }

    /// The song's window, within the budget.
    fn window(&self, load: [i64; 5], duration_ms: Option<i64>) -> Window {
        let w = self.window.unwrap_or_else(|| Window::for_song(load, duration_ms, self.len));
        match self.budget {
            Some(b) if b < w.cap => Window { low: w.low.min(b / 2), high: w.high.min(b), cap: b },
            _ => w,
        }
    }

    fn end(&self) -> u64 {
        match (&self.disk, self.len) {
            (Some(_), Some(len)) => len,
            _ => self.base + self.data.len() as u64,
        }
    }

    /// Bytes at hand from the reader on: in the window, or in the earlier window it reads (and the window
    /// right after it).
    fn ahead(&self) -> u64 {
        self.end().saturating_sub(self.reader_at)
    }

    /// Bytes at hand from `at` on, without a gap.
    fn held_from(&self, at: u64) -> u64 {
        if self.in_window(at) {
            return self.end() - at;
        }
        let Some((b, d)) = self.piece_at(at) else { return 0 };
        let end = b + d.len() as u64;
        end - at + if self.base == end { self.data.len() as u64 } else { 0 }
    }

    /// Whether a read from `at` would find its bytes: enough of them (or of a pushed song arriving in
    /// time), all up to the end, or no more to come.
    fn ready_from(&self, at: u64) -> bool {
        let held = self.held_from(at);
        self.error.is_some() || self.closed || held >= READY || self.len.is_some_and(|l| at + held >= l) || (self.done && held > 0 && at + held == self.end()) || (self.in_window(at) && self.in_time(held, Instant::now()))
    }

    fn in_window(&self, at: u64) -> bool {
        at >= self.base && at < self.end()
    }

    /// The earlier window holding byte `at`.
    fn piece_at(&self, at: u64) -> Option<(u64, &VecDeque<u8>)> {
        self.pieces.iter().find(|(b, d)| at >= *b && at < b + d.len() as u64).map(|(b, d)| (*b, d))
    }

    /// The end of the earlier window the reader reads, once within `low` of it with nothing after it at
    /// hand: where the window goes on.
    fn piece_running_out(&self, low: u64) -> Option<u64> {
        let at = self.in_piece?;
        let (b, d) = self.piece_at(at)?;
        let end = b + d.len() as u64;
        let held = self.in_window(end) || self.base == end || self.piece_at(end).is_some();
        (!held && end - at < low.max(READY)).then_some(end)
    }

    /// Room for the burst and `n` more bytes: what is kept behind the reader (at least [`FAR`], what a
    /// song over its cap keeps) and `high` ahead of it, up to the song's end. Reserved exactly: doubling
    /// copied the bytes and held up to twice them.
    fn reserve_burst(&mut self, high: u64, n: usize) {
        if let Some(len) = self.len {
            let burst = self.reader_at.saturating_sub(self.base).max(FAR) + high + CHUNK as u64;
            let want = (len.saturating_sub(self.base).min(burst) as usize).max(self.data.len() + n);
            self.data.reserve_exact(want - self.data.len());
        }
    }

    fn at_end(&self) -> bool {
        self.done || self.len.is_some_and(|l| self.end() >= l)
    }

    /// Whether the whole song is in memory: None while it may yet be; false when it never will be
    /// (failed, or larger than one burst).
    fn whole(&self) -> Option<bool> {
        if self.error.is_some() || self.closed {
            return Some(false);
        }
        if self.base == 0 && self.at_end() {
            return Some(true);
        }
        if self.window.is_some_and(|w| self.len.is_some_and(|len| len > w.high)) {
            return Some(false);
        }
        None
    }

    fn ready(&self) -> bool {
        self.ready_at(Instant::now())
    }

    /// [`State::ready`] with the clock at `now`.
    fn ready_at(&self, now: Instant) -> bool {
        if let Some(at) = self.in_piece {
            return self.ready_from(at);
        }
        self.at_end() || self.error.is_some() || self.ahead() >= if self.live { LIVE_READY } else { READY } || self.in_time(self.ahead(), now)
    }

    /// A pushed song's bytes arrive fast enough to play from what is at hand ([`arrives_in_time`]).
    fn in_time(&self, ahead: u64, now: Instant) -> bool {
        let (true, Some(per_ms), Some((since, from)), Some(len)) = (self.pushed && !self.live, self.per_ms, self.burst_from, self.len) else { return false };
        let ms = now.saturating_duration_since(since).as_secs_f64() * 1000.0;
        if ms < RATE_MS {
            return false;
        }
        let rate = self.end().saturating_sub(from) as f64 / ms;
        arrives_in_time(ahead, len.saturating_sub(self.end()), per_ms, rate)
    }

    /// Takes the titles the reader passed; keeps the last.
    fn passed(&mut self) {
        while self.titles.front().is_some_and(|(at, _)| *at <= self.reader_at) {
            self.announced = self.titles.pop_front().map(|(_, t)| t);
        }
    }
}

/// A loader's next step ([`Filling::poll_next`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Ask for the song from this byte.
    Open(u64),
    /// Read the open body: its next bytes are the song's from this byte.
    Read(u64),
    /// Nothing until the reader moves; the open body, if any, is closed ([`Filling::open`]).
    Wait,
    /// The song was let go: its request is dropped.
    Stop,
}

/// What a loader's fetching keeps between its steps.
struct Run {
    load: [i64; 5],
    duration_ms: Option<i64>,
    /// A body is open.
    open: bool,
    /// Failures in a row.
    failures: u32,
    /// The open body brought bytes.
    brought: bool,
    /// An entry the fetching ahead began: its start is read from disk and the rest fetched at once.
    go_on: bool,
    keep: Option<Writer>,
    taker: Option<Listening>,
}

/// A rangeless answer to a request from `asked`, read from `at` up to it.
struct Skip {
    asked: u64,
    at: u64,
    len: Option<u64>,
}

/// A loader's fetching apart from its I/O: what to ask for next ([`Filling::poll_next`]) and what came of
/// it ([`Filling::opened`], [`Filling::took`]). Natively the loader's own thread runs it over
/// [`ByteSource::open`]'s blocking bodies; a platform where nothing may block (the web) takes it in
/// [`ByteSource::push`] and runs it on its own events.
pub struct Filling {
    loaded: Arc<Loaded>,
    run: Run,
    url: String,
    /// The cache entry and the taker, made at the first step.
    start: Option<(Option<Keep>, Option<Listening>)>,
    skipping: Option<Skip>,
}

impl Filling {
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The body is still wanted; once not, the platform drops it.
    pub fn open(&self) -> bool {
        self.run.open || self.skipping.is_some()
    }

    /// The next step; pending until it may have changed, when `cx`'s waker is woken.
    pub fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Next> {
        self.begin();
        let loaded = self.loaded.clone();
        let mut s = loaded.state.lock();
        match self.next_locked(&mut s) {
            Next::Wait => {
                s.pusher = Some(cx.waker().clone());
                Poll::Pending
            }
            next => Poll::Ready(next),
        }
    }

    /// The answer to [`Next::Open`] from `from`: where its body starts and the song's length, or why there
    /// is none. Rest this long before the next step, if any.
    pub fn opened(&mut self, from: u64, answer: Result<(u64, Option<u64>), OpenError>) -> Option<Duration> {
        match answer {
            // A rangeless answer: read forward to `from`.
            Ok((start, len)) if start < from => {
                self.skipping = Some(Skip { asked: from, at: start, len });
                None
            }
            Ok((_, len)) => self.loaded.opened(&mut self.run, from, Ok(len)),
            Err(e) => self.loaded.opened(&mut self.run, from, Err(e)),
        }
    }

    /// What reading the body at [`Next::Read`]'s `from` brought: bytes, nothing at its end, or why it
    /// broke. Rest this long before the next step, if any.
    pub fn took(&mut self, from: u64, got: Result<&[u8], String>) -> Option<Duration> {
        let Some(k) = &mut self.skipping else { return self.loaded.took(&mut self.run, from, got) };
        let asked = k.asked;
        match got {
            // Ended before the byte asked for.
            Ok([]) => {
                let len = k.at;
                self.skipping = None;
                self.loaded.opened(&mut self.run, asked, Err(OpenError::PastEnd { len: Some(len) }))
            }
            Ok(bytes) => {
                let n = bytes.len().min((asked - k.at) as usize);
                k.at += n as u64;
                if k.at < asked {
                    return None;
                }
                let len = k.len.filter(|&l| l >= asked);
                self.skipping = None;
                let rest = self.loaded.opened(&mut self.run, asked, Ok(len));
                match &bytes[n..] {
                    [] => rest,
                    more => self.loaded.took(&mut self.run, asked, Ok(more)),
                }
            }
            Err(e) => {
                self.skipping = None;
                let e = if self.loaded.cancel.timed_out() { OpenError::TimedOut } else { OpenError::Failed(e) };
                self.loaded.opened(&mut self.run, asked, Err(e))
            }
        }
    }

    /// The cancellation of the running request: a platform hooks it to abort its request
    /// ([`Cancel::on_cancel`]).
    pub fn cancel(&self) -> &Cancel {
        &self.loaded.cancel
    }

    /// Makes the cache entry and the taker; an entry the fetching ahead began is read back from disk.
    fn begin(&mut self) {
        let Some((keep, taker)) = self.start.take() else { return };
        self.run.taker = taker;
        self.run.keep = keep.and_then(|k| k());
        let Some(k) = self.run.keep.as_mut().filter(|k| k.written() > 0) else { return };
        match k.read_back() {
            Ok(bytes) => {
                if let Some(t) = self.run.taker.as_mut() {
                    t.take(&bytes);
                }
                let mut s = self.loaded.state.lock();
                s.data = bytes.into();
                self.run.go_on = true;
                self.loaded.wake(&mut s);
            }
            Err(_) => self.run.keep = None,
        }
    }

    fn next_locked(&mut self, s: &mut State) -> Next {
        match &self.skipping {
            Some(_) if s.closed => Next::Stop,
            Some(k) => Next::Read(k.at),
            None => self.loaded.next(s, &mut self.run),
        }
    }
}

/// One song's bytes, shared by its loader thread and readers.
struct Loaded {
    state: Mutex<State>,
    cv: Condvar,
    /// Its running request, cancelled when the song is let go.
    cancel: Cancel,
    retry_ms: u64,
    /// Whose watch its requests are under.
    _fetching: Arc<Fetching>,
}

/// A song being loaded from `url` of a source; dropping the last handle stops the loader and frees its
/// memory.
pub struct Loader(Arc<Loaded>, Arc<dyn ByteSource>, String);

impl Loader {
    /// Starts loading `url` on a thread of its own, writing into `keep` if given.
    pub fn start(source: Arc<dyn ByteSource>, url: String, load: [i64; 5], duration_ms: Option<i64>, keep: Option<Writer>) -> Arc<Loader> {
        Loader::start_within(&Arc::default(), source, url, load, duration_ms, keep.map(|w| Box::new(move || Some(w)) as Keep), None, None, Waits::default())
    }

    /// [`Loader::start`] among `fetching`'s, with a `budget` (`Loader::limit`), a lazily made cache entry,
    /// and a `taker` fed every byte in order from the first (AutoMix's measuring, `crate::arriving`); a
    /// seek drops it.
    #[allow(clippy::too_many_arguments)]
    pub fn start_within(fetching: &Arc<Fetching>, source: Arc<dyn ByteSource>, url: String, load: [i64; 5], duration_ms: Option<i64>, keep: Option<Keep>, budget: Option<u64>, taker: Option<Listening>, waits: Waits) -> Arc<Loader> {
        let loaded = Arc::new(Loaded::new(fetching, State { budget, pushed: true, ..State::default() }, waits));
        fetching.alive(&loaded);
        let run = Run { load, duration_ms, open: false, failures: 0, brought: false, go_on: false, keep: None, taker: None };
        let fill = Filling { loaded: loaded.clone(), run, url: url.clone(), start: Some((keep, taker)), skipping: None };
        if let Some(fill) = source.push(fill) {
            loaded.state.lock().pushed = false;
            let (l, from) = (loaded.clone(), source.clone());
            std::thread::Builder::new()
                .name("nori-load".into())
                .spawn(move || {
                    // A panicking loader fails its song rather than leaving readers waiting.
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.run(fill, &*from))).is_err() {
                        l.gave_up("the song's loader failed");
                    }
                })
                .expect("a thread for loading");
        }
        Arc::new(Loader(loaded, source, url))
    }

    /// Bytes are still on their way: wanted, not failed, not complete.
    pub fn fetching(&self) -> bool {
        let s = self.0.state.lock();
        !s.closed && s.error.is_none() && !s.at_end()
    }

    /// A live stream at `url` among `fetching`'s, loaded while held.
    pub fn live(fetching: &Arc<Fetching>, source: Arc<dyn ByteSource>, url: String, waits: Waits) -> Arc<Loader> {
        let loaded = Arc::new(Loaded::new(fetching, State { live: true, ..State::default() }, waits));
        fetching.alive(&loaded);
        let (l, from, at) = (loaded.clone(), source.clone(), url.clone());
        std::thread::Builder::new().name("nori-live".into()).spawn(move || l.run_live(&*from, &at)).expect("a thread for loading");
        Arc::new(Loader(loaded, source, url))
    }

    /// The latest ICY title the reader reached, once.
    pub fn announced(&self) -> Option<String> {
        let mut s = self.0.state.lock();
        s.passed();
        s.announced.take()
    }

    /// Its state in words, for a stall report.
    pub fn words(&self) -> String {
        let s = self.0.state.lock();
        let len = s.len.map_or("?".to_string(), |l| l.to_string());
        let mut w = format!("{}..{} of {len} bytes, reader at {}", s.base, s.end(), s.reader_at);
        if s.disk.is_some() {
            w.push_str(", read from the disk");
        }
        if s.done {
            w.push_str(", loaded to its end");
        }
        if let Some(at) = s.restart {
            w.push_str(&format!(", wanted from {at}"));
        }
        if s.blocked > 0 {
            w.push_str(&format!(", {} readers blocked", s.blocked));
        }
        if s.waiter.is_some() {
            w.push_str(", the engine waits on it");
        }
        if !s.ready() {
            w.push_str(", not ready");
        }
        if let Some(e) = &s.error {
            w.push_str(&format!(", gave up: {e}"));
        }
        if s.closed {
            w.push_str(", closed");
        }
        w.push_str(&format!(", {} bursts", s.bursts));
        w
    }

    /// Connections opened: one per burst.
    pub fn bursts(&self) -> u32 {
        self.0.state.lock().bursts
    }

    /// Bytes held in memory.
    pub fn held(&self) -> usize {
        self.0.state.lock().data.len()
    }

    /// Bytes it will hold once its burst is in (capped); 0 once read from disk.
    pub fn holding(&self) -> u64 {
        let s = self.0.state.lock();
        if s.disk.is_some() {
            return 0;
        }
        let cap = s.window.map_or(u64::MAX, |w| w.cap).min(s.budget.unwrap_or(u64::MAX));
        s.len.map_or(s.data.len() as u64, |l| l.saturating_sub(s.base)).min(cap)
    }

    /// Holds at most `bytes` (None: its window's cap). A song fetched ahead gets what the playing one
    /// leaves; opened to play, it gets the whole cap.
    pub fn limit(&self, bytes: Option<u64>) {
        let mut s = self.0.state.lock();
        if s.budget != bytes {
            s.budget = bytes;
            self.0.stir(&mut s);
        }
    }

    /// The whole song is in memory.
    pub fn complete(&self) -> bool {
        let s = self.0.state.lock();
        s.base == 0 && s.at_end() && s.error.is_none()
    }

    /// Blocks until the whole song is in memory. False when it never will be (failed, or larger than
    /// one burst).
    pub(crate) fn wait_whole(&self) -> bool {
        let l = &*self.0;
        let mut s = l.state.lock();
        loop {
            if let Some(whole) = s.whole() {
                return whole;
            }
            s.blocked += 1;
            l.cv.wait(&mut s);
            s.blocked -= 1;
        }
    }

    /// [`Loader::wait_whole`] without waiting: None until known, when `engine` is woken.
    pub(crate) fn whole_or_wake(&self, engine: &Thread) -> Option<bool> {
        let mut s = self.0.state.lock();
        let whole = s.whole();
        if whole.is_none() {
            s.waiter = Some(engine.clone());
        }
        whole
    }

    /// Whether a read from `at` would find its bytes; if not, `engine` is woken when the loader is ready.
    pub(crate) fn ready_from_or_wake(&self, at: u64, engine: &Thread) -> bool {
        let mut s = self.0.state.lock();
        if s.ready_from(at) {
            return true;
        }
        s.waiter = Some(engine.clone());
        false
    }


    /// A song opening on a pushed loader waits to read from `at`: the window goes on where the bytes
    /// held from there stop, whatever its tries read since.
    pub(crate) fn want_from(&self, at: u64) {
        let mut s = self.0.state.lock();
        if s.ready_from(at) {
            return;
        }
        let gap = at + s.held_from(at);
        if gap < s.base || gap > s.end() + FAR {
            s.restart = Some(gap);
        }
        (s.reader_at, s.in_piece) = (gap, None);
        self.0.stir(&mut s);
    }

    /// The platform fetches it ([`ByteSource::push`]): nothing reading it may block.
    pub(crate) fn pushed(&self) -> bool {
        self.0.state.lock().pushed
    }

    /// A song opens on a pushed loader (`on`), or is open or let go: while any opens, what it read is
    /// kept wherever the window moves.
    pub(crate) fn opening(&self, on: bool) {
        let mut s = self.0.state.lock();
        if on {
            s.opening += 1;
        } else {
            s.opening = s.opening.saturating_sub(1);
        }
    }

    /// The source's cache held a copy cut short: it drops it ([`ByteSource::forget`]) and the song is
    /// fetched again from its start. False when the source had no copy to drop.
    pub(crate) fn refetch(&self) -> bool {
        if !self.1.forget(&self.2) {
            return false;
        }
        let mut s = self.0.state.lock();
        (s.restart, s.len, s.disk, s.window, s.before) = (Some(0), None, None, None, None);
        self.0.stir(&mut s);
        true
    }

    /// Times the length turned out shorter than promised: a demuxer re-reads the end when this moves.
    pub fn shortened(&self) -> u32 {
        self.0.state.lock().shortened
    }

    /// The length in bytes, if known.
    pub fn length(&self) -> Option<u64> {
        self.0.state.lock().len
    }

    /// Why the loader gave up, if it did.
    pub fn error(&self) -> Option<String> {
        self.0.state.lock().error.clone()
    }

    /// The server's error status when the loader gave up (the song's failure, not the network's).
    pub fn answered(&self) -> Option<u16> {
        let s = self.0.state.lock();
        s.error.as_ref().and(s.answered)
    }

    /// Whether a read would not block; if not, `engine` is woken when it would not.
    pub(crate) fn ready_or_wake(&self, engine: &Thread) -> bool {
        let mut s = self.0.state.lock();
        if s.ready() {
            return true;
        }
        s.waiter = Some(engine.clone());
        false
    }

    pub fn reader(self: &Arc<Self>) -> LoadedReader {
        LoadedReader { loader: self.clone(), pos: 0, stop: None, trying: None }
    }

    /// A reader for a try at opening a song on a pushed loader: its reads fail from the first that finds
    /// no bytes, noted in `tried`, until it is [`Tried::Open`].
    pub(crate) fn trying_reader(self: &Arc<Self>, tried: Arc<Mutex<Tried>>) -> LoadedReader {
        LoadedReader { loader: self.clone(), pos: 0, stop: None, trying: Some(tried) }
    }

    /// A reader that fails once `stop` is set, for an opening nobody wants any more (it would otherwise
    /// keep moving the fetch against the wanted reader).
    pub(crate) fn reader_until(self: &Arc<Self>, stop: Arc<AtomicBool>) -> LoadedReader {
        LoadedReader { loader: self.clone(), pos: 0, stop: Some(stop), trying: None }
    }

    /// Wakes blocked readers to check `stop`.
    pub(crate) fn nudge(&self) {
        self.0.cv.notify_all();
    }

    /// Blocks until `n` readers wait for bytes (a reader that blocks signals the loader's condvar).
    #[cfg(test)]
    pub(crate) fn wait_blocked(&self, n: u32) {
        let mut s = self.0.state.lock();
        while s.blocked < n {
            self.0.cv.wait(&mut s);
        }
    }
}

impl Drop for Loader {
    /// Stops the loader and cancels its request at once.
    fn drop(&mut self) {
        let mut s = self.0.state.lock();
        s.closed = true;
        self.0.stir(&mut s);
        drop(s);
        self.0.cancel.close();
    }
}

impl Loaded {
    fn new(fetching: &Arc<Fetching>, state: State, waits: Waits) -> Loaded {
        Loaded { state: Mutex::new(state), cv: Condvar::new(), cancel: Cancel::stalling_after(fetching, waits.stall_ms), retry_ms: waits.retry_ms, _fetching: fetching.clone() }
    }

    /// The rest before the retry after `failures` failures in a row: 2, 4, 8 times `retry_ms`.
    fn resting(&self, failures: u32) -> Duration {
        Duration::from_millis(self.retry_ms << failures)
    }

    /// Waits `d` before a retry; false once the song is let go.
    fn rest(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        let mut s = self.state.lock();
        while !s.closed && !self.cv.wait_for(&mut s, until.saturating_duration_since(Instant::now())).timed_out() {}
        !s.closed
    }

    /// Something that moves the fetching changed: wakes the loader's thread, or the platform pushing.
    fn stir(&self, s: &mut State) {
        self.cv.notify_all();
        if let Some(w) = s.pusher.take() {
            w.wake();
        }
    }

    /// The fetching's next step, with the lock held. Closes `run`'s body at the high mark and the end.
    fn next(&self, s: &mut State, run: &mut Run) -> Next {
        if s.closed {
            return Next::Stop;
        }
        let low = s.window(run.load, run.duration_ms).low;
        if s.restart.is_none() {
            s.restart = s.piece_running_out(low);
        }
        if let Some(at) = s.restart.take() {
            run.open = false;
            s.burst_from = None;
            if s.opening > 0 && !s.data.is_empty() {
                let data = std::mem::take(&mut s.data);
                let base = s.base;
                s.pieces.push((base, data));
            }
            // Wanted from there, whatever an opening's try read since.
            if s.pushed {
                s.reader_at = at;
            }
            s.base = at;
            s.data.clear();
            s.done = false;
            s.error = None;
            // A gap: neither the cache entry nor the taker can continue.
            run.keep = None;
            run.taker = None;
        }
        let w = s.window(run.load, run.duration_ms);
        if s.at_end() {
            run.open = false;
            s.burst_from = None;
            s.done = true;
            if let (Some(k), Some(len), None) = (run.keep.take(), s.len, &s.error) {
                // Whole in the entry: read from there and drop the memory copy.
                let whole = s.base == 0 && s.data.len() as u64 == len;
                if let Some(file) = k.finish_open(len).filter(|_| whole) {
                    s.data = VecDeque::new();
                    s.disk = Some(Arc::new(file));
                }
            }
            if let Some(t) = run.taker.take() {
                t.end(s.error.is_none());
            }
            self.wake(s);
            return Next::Wait;
        }
        if (run.open && s.ahead() < w.high) || (!run.open && (run.go_on || s.ahead() < w.low.max(1))) {
            // Over the cap: drop what the reader left behind.
            let keep_from = s.reader_at.saturating_sub(FAR);
            if s.data.len() as u64 > w.cap && keep_from > s.base {
                let drop = (keep_from - s.base) as usize;
                s.data.drain(..drop);
                s.base = keep_from;
            }
            return if run.open { Next::Read(s.end()) } else { Next::Open(s.end()) };
        }
        // At the high mark: closed until the reader nears the low mark.
        run.open = false;
        s.burst_from = None;
        Next::Wait
    }

    /// A request from `from` answered: its body's length, or why there is none. The rest to take before
    /// the next step, if any.
    fn opened(&self, run: &mut Run, from: u64, answer: Result<Option<u64>, OpenError>) -> Option<Duration> {
        match answer {
            Ok(len) => {
                run.go_on = false;
                run.brought = false;
                let mut s = self.state.lock();
                let promised = len.filter(|&l| s.before.is_none_or(|b| l < b));
                match (s.len, promised) {
                    (None, l) => s.len = l,
                    // An earlier end than first promised: that was an estimate.
                    (Some(had), Some(l)) if l < had && l >= s.end() => s.ends_at(l),
                    _ => {}
                }
                s.window = Some(Window::for_song(run.load, run.duration_ms, s.len));
                s.per_ms = s.len.zip(run.duration_ms.filter(|&d| d > 0)).map(|(l, d)| l as f64 / d as f64);
                s.burst_from = None;
                s.bursts += 1;
                let high = s.window(run.load, run.duration_ms).high;
                s.reserve_burst(high, 0);
                run.open = true;
                None
            }
            // Past the end (a demuxer probing an estimated length): the real end, not a failure.
            Err(OpenError::PastEnd { len }) => {
                let mut s = self.state.lock();
                if s.end() == from && s.restart.is_none() {
                    match len.filter(|&l| l <= from) {
                        // Bytes held up to `from`: it ends right there.
                        _ if !s.data.is_empty() => s.ends_at(from),
                        Some(l) => s.ends_at(l),
                        // Somewhere before `from`: unknown until the bytes run out.
                        None => s.not_at(from),
                    }
                    s.done = true;
                    run.failures = 0;
                    run.go_on = false;
                }
                self.wake(&mut s);
                None
            }
            Err(_) if self.cancel.closed() => None,
            Err(e) => {
                run.failures += 1;
                // A timeout fails at once: retrying a server that never answers (octo-fiesta fetching an
                // unobtainable song) only keeps the player waiting.
                if run.failures > RETRIES || e == OpenError::TimedOut {
                    let mut s = self.state.lock();
                    s.answered = if let OpenError::Status(status) = e { Some(status) } else { None };
                    s.error = Some(e.to_string());
                    s.done = true;
                    self.wake(&mut s);
                    return None;
                }
                Some(self.resting(run.failures))
            }
        }
    }

    /// The open body, read from `from`, brought `got` (empty at its end). The rest to take before the
    /// next step, if any.
    fn took(&self, run: &mut Run, from: u64, got: Result<&[u8], String>) -> Option<Duration> {
        let mut s = self.state.lock();
        let mut kept: &[u8] = &[];
        match got {
            // A clean end is the real end, whatever was promised.
            Ok([]) => {
                run.open = false;
                s.burst_from = None;
                if s.end() == from && s.restart.is_none() {
                    s.done = true;
                    s.ends_at(from);
                }
            }
            Ok(bytes) => {
                run.failures = 0;
                run.brought = true;
                if s.end() == from && s.restart.is_none() {
                    if s.data.capacity() - s.data.len() < bytes.len() {
                        let high = s.window(run.load, run.duration_ms).high;
                        s.reserve_burst(high, bytes.len());
                    }
                    s.data.extend(bytes);
                    kept = bytes;
                    if s.burst_from.is_none() {
                        s.burst_from = Some((Instant::now(), s.end()));
                    }
                }
            }
            // Broken after bytes came: fetched again from there at once. Broken before any, or stalled,
            // it is a failure, so a server that answers and then breaks or never sends gives up after a
            // few.
            Err(e) => {
                run.open = false;
                s.burst_from = None;
                let stalled = self.cancel.timed_out();
                if stalled || !run.brought {
                    run.failures += 1;
                    if run.failures > RETRIES {
                        s.error = Some(if stalled { OpenError::TimedOut.to_string() } else { e });
                        s.done = true;
                    } else if !stalled {
                        return Some(self.resting(run.failures));
                    }
                }
            }
        }
        self.wake(&mut s);
        drop(s);
        if !kept.is_empty() {
            // Written without the lock, so the demuxer reads on.
            if run.keep.as_mut().is_some_and(|k| !k.write(from, kept)) {
                run.keep = None;
            }
            if let Some(t) = run.taker.as_mut() {
                t.take(kept);
            }
        }
        None
    }

    /// The loader's thread: [`Filling`]'s steps over the source's blocking bodies.
    fn run(&self, mut fill: Filling, source: &dyn ByteSource) {
        fill.begin();
        let mut body: Option<Box<dyn Read + Send>> = None;
        let mut chunk = vec![0u8; CHUNK];
        loop {
            let next = {
                let mut s = self.state.lock();
                loop {
                    let next = fill.next_locked(&mut s);
                    if !fill.open() {
                        body = None;
                    }
                    if next != Next::Wait {
                        break next;
                    }
                    self.cv.wait(&mut s);
                }
            };
            let rest = match next {
                Next::Open(from) => match open_watched(source, &fill.url, None, from, &self.cancel) {
                    Ok(b) => {
                        body = Some(b.reader);
                        fill.opened(from, Ok((b.start, b.len)))
                    }
                    Err(e) => fill.opened(from, Err(e)),
                },
                Next::Read(from) => {
                    let got = body.as_mut().expect("a body is open").read(&mut chunk);
                    fill.took(from, got.as_ref().map(|&n| &chunk[..n]).map_err(|e| e.to_string()))
                }
                Next::Wait | Next::Stop => return,
            };
            if !fill.open() {
                body = None;
            }
            if rest.is_some_and(|d| !self.rest(d)) {
                return;
            }
        }
    }

    /// Reads a live stream's connection, strips ICY titles and drops what the reader passed. A dropped
    /// connection is reopened (the stream resumes where the station is); one that cannot be is the end.
    fn run_live(&self, source: &dyn ByteSource, url: &str) {
        let mut body: Option<Icy> = None;
        let mut chunk = vec![0u8; LIVE_CHUNK];
        let mut failures = 0;
        loop {
            {
                let mut s = self.state.lock();
                loop {
                    if s.closed {
                        return;
                    }
                    if body.is_none() || s.ahead() < LIVE_AHEAD {
                        break;
                    }
                    // The reader stopped: the connection waits in the socket.
                    self.cv.wait(&mut s);
                }
                let keep_from = s.reader_at.saturating_sub(LIVE_BEHIND);
                if keep_from > s.base {
                    let drop = ((keep_from - s.base) as usize).min(s.data.len());
                    s.data.drain(..drop);
                    s.base += drop as u64;
                }
            }
            if body.is_none() {
                match source.open_live(url) {
                    Ok((b, every)) => {
                        self.state.lock().bursts += 1;
                        body = Some(Icy::new(b.reader, every));
                    }
                    Err(e) => {
                        failures += 1;
                        if failures > RETRIES {
                            let mut s = self.state.lock();
                            s.error = Some(e);
                            s.done = true;
                            self.wake(&mut s);
                            return;
                        }
                        if !self.rest(self.resting(failures)) {
                            return;
                        }
                        continue;
                    }
                }
            }
            let icy = body.as_mut().expect("a body is open");
            let got = icy.read(&mut chunk);
            let title = icy.title.take();
            let mut s = self.state.lock();
            match got {
                Ok(n) if n > 0 => {
                    failures = 0;
                    s.data.extend(&chunk[..n]);
                }
                // Closed or broken: reconnect.
                _ => {
                    body = None;
                    failures += 1;
                }
            }
            if let Some(t) = title {
                let at = s.end();
                s.titles.push_back((at, t));
            }
            self.wake(&mut s);
        }
    }

    /// The loader thread died (panicked): readers are told the song failed and the engine is woken.
    fn gave_up(&self, why: &str) {
        let mut s = self.state.lock();
        if s.error.is_none() && !s.at_end() {
            s.error = Some(why.to_string());
        }
        s.done = true;
        s.restart = None;
        self.cv.notify_all();
        self.wake(&mut s);
    }

    /// Wakes blocked readers, and the waiting engine once ready.
    fn wake(&self, s: &mut State) {
        if s.blocked > 0 {
            self.cv.notify_all();
        }
        if s.ready() {
            if let Some(t) = s.waiter.take() {
                t.unpark();
            }
        }
    }
}

/// A song's bytes as a seekable stream for the demuxer; reads block until the bytes are there.
pub struct LoadedReader {
    loader: Arc<Loader>,
    pos: u64,
    /// Set once nobody wants this read ([`Loader::reader_until`]).
    stop: Option<Arc<AtomicBool>>,
    trying: Option<Arc<Mutex<Tried>>>,
}

/// A try at opening a song on a pushed loader ([`Loader::trying_reader`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tried {
    Reading,
    /// A read found no bytes here: every read after it fails too, so the try ends there.
    Dry(u64),
    /// Opened: read as any reader.
    Open,
}

impl Read for LoadedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.trying.as_ref().is_some_and(|t| matches!(*t.lock(), Tried::Dry(_))) {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let l = &*self.loader.0;
        let mut s = l.state.lock();
        loop {
            if self.stop.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
                return Err(io::Error::other("the song is no longer wanted"));
            }
            let end = s.end();
            if let Some(file) = s.disk.clone() {
                if self.pos >= end {
                    return Ok(0);
                }
                s.reader_at = self.pos;
                drop(s);
                let want = buf.len().min((end - self.pos) as usize);
                let n = read_at(&file, &mut buf[..want], self.pos)?;
                self.pos += n as u64;
                return Ok(n);
            }
            if self.pos >= s.base && self.pos < end {
                let from = (self.pos - s.base) as usize;
                let n = buf.len().min(s.data.len() - from);
                let (front, back) = s.data.as_slices();
                let a = front.len().saturating_sub(from).min(n);
                buf[..a].copy_from_slice(&front[from.min(front.len())..][..a]);
                let b = (from + a).saturating_sub(front.len());
                buf[a..n].copy_from_slice(&back[b..b + n - a]);
                self.pos += n as u64;
                s.reader_at = self.pos;
                s.in_piece = None;
                // Read on in the window: the earlier ones are behind.
                if s.opening == 0 {
                    s.pieces.clear();
                }
                // Within the low mark: the next burst is due.
                if !s.at_end() && (s.window.is_some_and(|w| s.ahead() < w.low) || (s.live && s.ahead() < LIVE_AHEAD)) {
                    l.stir(&mut s);
                }
                return Ok(n);
            }
            if let Some((b, d)) = s.piece_at(self.pos) {
                let from = (self.pos - b) as usize;
                let n = buf.len().min(d.len() - from);
                for (out, byte) in buf[..n].iter_mut().zip(d.range(from..from + n)) {
                    *out = *byte;
                }
                self.pos += n as u64;
                s.in_piece = Some(self.pos);
                if s.piece_running_out(s.window.map_or(READY, |w| w.low)).is_some() {
                    l.stir(&mut s);
                }
                return Ok(n);
            }
            if s.at_end() && self.pos >= end && s.restart.is_none() {
                if let Some(e) = &s.error {
                    return Err(io::Error::other(e.clone()));
                }
                return Ok(0);
            }
            // At or past the known end: nothing to fetch.
            if !s.live && s.len.is_some_and(|l| self.pos >= l) {
                return Ok(0);
            }
            if s.live && self.pos < s.base {
                return Err(io::Error::other("a live stream cannot be read again from further back"));
            }
            if !s.live && (self.pos < s.base || self.pos > end + FAR) {
                s.restart = Some(self.pos);
            }
            s.reader_at = self.pos;
            s.in_piece = None;
            l.stir(&mut s);
            if s.pushed {
                if let Some(t) = &self.trying {
                    let mut t = t.lock();
                    if *t == Tried::Reading {
                        *t = Tried::Dry(self.pos);
                    }
                }
                return Err(io::ErrorKind::WouldBlock.into());
            }
            s.blocked += 1;
            let timed_out = l.cv.wait_for(&mut s, READ_TIMEOUT).timed_out();
            s.blocked -= 1;
            if timed_out {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "the song's bytes did not come"));
            }
        }
    }
}

/// Reads at `at` without moving a file position: several readers share one entry.
fn read_at(file: &File, buf: &mut [u8], at: u64) -> io::Result<usize> {
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::read_at(file, buf, at);
    #[cfg(windows)]
    return std::os::windows::fs::FileExt::seek_read(file, buf, at);
    #[cfg(not(any(unix, windows)))]
    {
        let mut f = file;
        f.seek(SeekFrom::Start(at))?;
        f.read(buf)
    }
}

/// A live stream body with ICY metadata stripped: every `every` bytes comes a length byte (in units
/// of 16) and that much of `StreamTitle='...';`.
struct Icy {
    inner: Box<dyn Read + Send>,
    every: Option<usize>,
    /// Music bytes before the next metadata block.
    left: usize,
    /// The last title, until the loader takes it.
    title: Option<String>,
}

impl Icy {
    fn new(inner: Box<dyn Read + Send>, every: Option<usize>) -> Icy {
        let every = every.filter(|&n| n > 0);
        Icy { inner, every, left: every.unwrap_or(0), title: None }
    }

    /// Reads one metadata block.
    fn announcement(&mut self) -> io::Result<()> {
        let mut len = [0u8; 1];
        self.inner.read_exact(&mut len)?;
        let n = len[0] as usize * 16;
        if n == 0 {
            return Ok(());
        }
        let mut text = vec![0u8; n];
        self.inner.read_exact(&mut text)?;
        if let Some(t) = stream_title(&text) {
            self.title = Some(t);
        }
        Ok(())
    }
}

impl Read for Icy {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(every) = self.every else { return self.inner.read(buf) };
        if self.left == 0 {
            self.announcement()?;
            self.left = every;
        }
        let want = buf.len().min(self.left);
        let n = self.inner.read(&mut buf[..want])?;
        self.left -= n;
        Ok(n)
    }
}

/// The title in an ICY block (`StreamTitle='Artist - Song';StreamUrl='';`, NUL padded). Non-UTF-8 is
/// read as Latin-1, which stations use as often.
pub fn stream_title(text: &[u8]) -> Option<String> {
    let text = match std::str::from_utf8(text) {
        Ok(t) => t.to_string(),
        Err(_) => text.iter().map(|&b| b as char).collect(),
    };
    let from = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[from..];
    // Ends at the field's closing quote; quotes inside are kept.
    let end = rest.find("';").or_else(|| rest.rfind('\'')).unwrap_or(rest.len());
    let title = rest[..end].trim_matches(char::from(0)).trim();
    (!title.is_empty()).then(|| title.to_string())
}

impl Seek for LoadedReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let len = self.loader.0.state.lock().len;
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before the start"))?,
            SeekFrom::End(d) => len.ok_or_else(|| io::Error::other("length unknown"))?.checked_add_signed(d).ok_or_else(|| io::Error::other("seek before the start"))?,
        };
        Ok(self.pos)
    }
}

impl MediaSource for LoadedReader {
    fn is_seekable(&self) -> bool {
        self.loader.0.state.lock().len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.loader.0.state.lock().len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    /// A count a test waits on.
    #[derive(Default)]
    struct Signal(Mutex<u64>, Condvar);

    impl Signal {
        fn bump(&self) {
            *self.0.lock() += 1;
            self.1.notify_all();
        }

        fn reach(&self, n: u64) {
            let until = Instant::now() + Duration::from_secs(10);
            let mut c = self.0.lock();
            while *c < n {
                assert!(!self.1.wait_until(&mut c, until).timed_out() || *c >= n, "{} of {n}", *c);
            }
        }
    }

    /// Waits as a blocked reader (counted in `blocked`) until `f` holds of the loader.
    fn until(l: &Loader, what: &str, f: impl Fn(&State) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        let mut s = l.0.state.lock();
        s.blocked += 1;
        while !f(&s) {
            assert!(!l.0.cv.wait_until(&mut s, until).timed_out() || f(&s), "{what}");
        }
        s.blocked -= 1;
    }

    /// Bytes served and connections closed.
    #[derive(Default)]
    struct Served {
        bytes: AtomicU64,
        closed: Signal,
    }

    /// Serves `len` made-up bytes (byte `i` is `i as u8`) and counts requests.
    struct Counting {
        len: u64,
        opens: Mutex<Vec<u64>>,
        served: Arc<Served>,
    }

    struct Made {
        at: u64,
        len: u64,
        served: Arc<Served>,
    }

    impl Drop for Made {
        fn drop(&mut self) {
            self.served.closed.bump();
        }
    }

    impl Read for Made {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min((self.len - self.at) as usize);
            for (k, b) in buf[..n].iter_mut().enumerate() {
                *b = (self.at + k as u64) as u8;
            }
            self.at += n as u64;
            self.served.bytes.fetch_add(n as u64, Ordering::Relaxed);
            Ok(n)
        }
    }

    impl ByteSource for Counting {
        fn open(&self, _: &str, from: u64) -> Result<Body, OpenError> {
            self.opens.lock().push(from);
            Ok(Body { start: from, len: Some(self.len), reader: Box::new(Made { at: from, len: self.len, served: self.served.clone() }) })
        }
    }

    fn server(len: u64) -> Arc<Counting> {
        Arc::new(Counting { len, opens: Mutex::new(Vec::new()), served: Arc::default() })
    }

    /// Bytes served once `bursts` connections are closed.
    fn settled(s: &Counting, bursts: u64) -> u64 {
        s.served.closed.reach(bursts);
        s.served.bytes.load(Ordering::Relaxed)
    }

    fn read(r: &mut LoadedReader, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n];
        r.read_exact(&mut out).unwrap();
        out
    }

    /// 100 bytes/ms for a 10 s, 1 MB song: a 1-4 s window is 100-400 kB.
    const LOAD: [i64; 5] = [1_000, 4_000, 0, 0, 1 << 30];

    #[test]
    fn fetches_to_high_waits_for_low() {
        let s = server(1_000_000);
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(10_000), None);
        let mut r = l.reader();
        let first = settled(&s, 1);
        assert!((400_000..400_000 + CHUNK as u64).contains(&first), "one burst to the high mark: {first}");
        assert_eq!(*s.opens.lock(), vec![0]);
        // More than the low mark ahead: nothing fetched.
        let got = read(&mut r, 250_000);
        assert!(got.iter().enumerate().all(|(i, &b)| b == i as u8), "the bytes are the song's");
        // Within the low mark: the next burst, from where the last stopped.
        let at = first as usize - 90_000;
        read(&mut r, at - 250_000);
        let second = settled(&s, 2);
        assert_eq!(s.opens.lock().clone(), vec![0, first], "one more request, picking up where the first stopped");
        assert!(second - first >= 300_000, "a whole burst, not a top-up: {}", second - first);
        let rest = read(&mut r, 1_000_000 - at);
        assert!(rest.iter().enumerate().all(|(i, &b)| b == (at + i) as u8));
        assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0, "and then the end");
        assert_eq!(l.bursts() as usize, s.opens.lock().len());
    }

    #[test]
    fn small_song_fetched_in_one_request() {
        let s = server(300_000);
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(3_000), None);
        assert_eq!(settled(&s, 1), 300_000);
        let mut r = l.reader();
        let all = read(&mut r, 300_000);
        assert!(all.iter().enumerate().all(|(i, &b)| b == i as u8));
        assert_eq!(*s.opens.lock(), vec![0]);
    }

    #[test]
    fn start_follows_the_rate() {
        // A 4-minute song of 128 kbit/s (16 bytes per ms): 3.84 MB.
        let (per_ms, len) = (16.0, 3_840_000u64);
        let ready = |ahead: u64, rate: f64| arrives_in_time(ahead, len - ahead, per_ms, rate);
        // 400 kbit/s (50 bytes per ms): faster than the music, so a second of it is enough.
        assert!(ready(16_384, 50.0));
        assert!(!ready(15_000, 50.0), "less than its header and first packets");
        // As fast as the music: the margin is waited for first.
        assert!(!ready(100_000, 16.0));
        let enough = len as f64 * (1.0 - 1.0 / RATE_MARGIN);
        assert!(ready(enough as u64 + 1, 16.0) && !ready(enough as u64 - 16_000, 16.0));
        // A FLAC at the same 400 kbit/s (about 110 bytes per ms) is not ready on rate alone.
        assert!(!arrives_in_time(256 * 1024, 26_000_000, 110.0, 50.0));
        // A second of a song that comes fast: more than the floor of bytes.
        assert!(!arrives_in_time(100_000, 30_000_000, 110.0, 10_000.0) && arrives_in_time(110_000, 30_000_000, 110.0, 10_000.0));
    }

    #[test]
    fn pushed_song_ready_once_its_rate_is_known() {
        let at = Instant::now();
        let s = |pushed: bool, ahead: usize, came_ms: u64| State {
            pushed,
            len: Some(3_840_000),
            per_ms: Some(16.0),
            data: vec![0u8; ahead].into(),
            burst_from: Some((at, 4096)),
            ..State::default()
        }
        .ready_at(at + Duration::from_millis(came_ms));
        assert!(s(true, 32 * 1024, 600), "28 KB in 600 ms: 47 bytes per ms, ahead of the music");
        assert!(!s(true, 32 * 1024, 200), "too soon to know the rate");
        assert!(!s(true, 32 * 1024, 5_000), "6 bytes per ms: behind the music");
        assert!(!s(false, 32 * 1024, 600), "a loader on its own thread waits for READY as ever");
        assert!(s(true, READY as usize, 5_000) && s(false, READY as usize, 5_000), "READY is always enough");
        // A song opening reads from where it ran dry: the same there.
        let opening = State { pushed: true, len: Some(3_840_000), per_ms: Some(16.0), data: vec![0u8; 32 * 1024].into(), burst_from: Some((Instant::now() - Duration::from_millis(600), 4096)), ..State::default() };
        assert!(opening.ready_from(0) && !opening.ready_from(20 * 1024), "the floor from where the opening reads");
    }

    #[test]
    fn ahead_holds_budget_then_rest() {
        let s = server(600_000);
        let l = Loader::start_within(&Arc::default(), s.clone(), "song".into(), LOAD, Some(6_000), None, Some(200_000), None, Waits::default());
        let ahead = settled(&s, 1);
        assert!(ahead <= 200_000 + CHUNK as u64, "no more than its budget while it waits: {ahead}");
        assert!(l.held() >= 100_000, "but its start is at hand: {}", l.held());
        assert_eq!(l.holding(), 200_000);
        // Now playing: the whole cap; the rest comes as the reader nears the end.
        l.limit(None);
        let mut r = l.reader();
        let all = read(&mut r, 600_000);
        assert!(all.iter().enumerate().all(|(i, &b)| b == i as u8), "every byte, in order");
        assert_eq!(*s.opens.lock(), vec![0, ahead], "one more request, from where the budget stopped it");
    }

    #[test]
    fn song_over_its_cap_read_through() {
        // Bursts of 400 kB within 500 kB: what the reader left behind goes as new bytes come.
        let load = [1_000, 4_000, 0, 0, 500_000];
        let s = server(3_000_000);
        let l = Loader::start(s.clone(), "song".into(), load, Some(30_000), None);
        let mut r = l.reader();
        let all = read(&mut r, 3_000_000);
        assert!(all.iter().enumerate().all(|(i, &b)| b == i as u8), "every byte, in order");
        assert!(l.held() <= 500_000 + CHUNK + FAR as usize, "within its cap: {}", l.held());
    }

    #[test]
    fn whole_cached_song_read_from_disk() {
        let d = nori_testdir::TempDir::new("source-disk");
        let store = Arc::new(crate::store::Store::open(d.path(), 1 << 22).unwrap());
        let s = server(300_000);
        let fetching = Arc::new(Fetching::default());
        let keep = store.writer("song:0").map(|w| Box::new(move || Some(w)) as Keep);
        let l = Loader::start_within(&fetching, s.clone(), "song".into(), LOAD, Some(3_000), keep, None, None, Waits::default());
        let mut r = l.reader();
        let all = read(&mut r, 300_000);
        assert!(all.iter().enumerate().all(|(i, &b)| b == i as u8));
        assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0, "the end");
        // Dropped on the loader thread, a moment after the last bytes.
        until(&l, "read from the disk", |s| s.disk.is_some());
        assert_eq!((l.held(), l.holding()), (0, 0), "nothing kept in memory");
        assert!(l.complete());
        // Readable from anywhere.
        let mut again = l.reader();
        again.seek(SeekFrom::Start(123_456)).unwrap();
        assert_eq!(read(&mut again, 10), (123_456..123_466).map(|i| i as u8).collect::<Vec<_>>());
        assert_eq!(*s.opens.lock(), vec![0], "and the network was asked once");
        assert_eq!(fetching.held(), Held { songs: 1, bytes: 0, on_disk: 1 });
    }

    #[test]
    fn bytes_reserved() {
        let s = server(300_000);
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(3_000), None);
        settled(&s, 1);
        assert_eq!(l.0.state.lock().data.capacity(), 300_000, "made once, not grown by doubling");

        // Later burst reserves its bytes.
        // 100 bytes/ms: bursts of 4 MB, the next once 1 MB is left.
        let load = [10_000, 40_000, 0, 0, 1 << 30];
        let s = server(10_000_000);
        let l = Loader::start(s.clone(), "song".into(), load, Some(100_000), None);
        let first = settled(&s, 1);
        read(&mut l.reader(), first as usize - 900_000);
        settled(&s, 2);
        let st = l.0.state.lock();
        assert!(st.data.capacity() - st.data.len() < CHUNK, "{} held in {}: not doubled", st.data.len(), st.data.capacity());
    }

    #[test]
    fn seek_past_loaded_fetches_from_there() {
        let s = server(5_000_000);
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(50_000), None);
        settled(&s, 1);
        let mut r = l.reader();
        r.seek(SeekFrom::Start(4_000_000)).unwrap();
        let got = read(&mut r, 1000);
        assert!(got.iter().enumerate().all(|(i, &b)| b == (4_000_000 + i) as u8));
        assert_eq!(*s.opens.lock(), vec![0, 4_000_000], "not the megabytes in between");
    }

    /// A transcoding server: promises `promised` bytes, sends `real`, answers 416 past `real` (with the
    /// length when `says`). The first body breaks after `cut` bytes.
    struct Estimating {
        real: u64,
        promised: u64,
        says: bool,
        cut: Mutex<Option<u64>>,
        /// The body ends with an error, as OkHttp reads one shorter than its Content-Length.
        breaks_at_end: bool,
        opens: Mutex<Vec<u64>>,
    }

    struct Cut {
        made: Made,
        cut: Option<u64>,
        breaks_at_end: bool,
    }

    impl Read for Cut {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let stop = self.cut.unwrap_or(self.made.len);
            if self.made.at >= stop && (self.cut.is_some() || self.breaks_at_end) {
                return Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
            }
            let n = buf.len().min((stop - self.made.at) as usize);
            self.made.read(&mut buf[..n])
        }
    }

    impl ByteSource for Estimating {
        fn open(&self, _: &str, from: u64) -> Result<Body, OpenError> {
            self.opens.lock().push(from);
            if from >= self.real {
                return Err(OpenError::PastEnd { len: self.says.then_some(self.real) });
            }
            let made = Made { at: from, len: self.real, served: Arc::default() };
            let reader = Cut { made, cut: self.cut.lock().take(), breaks_at_end: self.breaks_at_end };
            Ok(Body { start: from, len: Some(self.promised), reader: Box::new(reader) })
        }
    }

    fn estimating(real: u64, promised: u64, says: bool) -> Estimating {
        Estimating { real, promised, says, cut: Mutex::new(None), breaks_at_end: false, opens: Mutex::new(Vec::new()) }
    }

    #[test]
    fn ends_found() {
        let s = Arc::new(estimating(300_000, 320_000, true));
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(3_000), None);
        let mut r = l.reader();
        let all = read(&mut r, 300_000);
        assert!(all.iter().enumerate().all(|(i, &b)| b == i as u8));
        assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0, "the end");
        assert_eq!(l.length(), Some(300_000), "the real length, not the estimate");
        assert_eq!(l.shortened(), 1);
        assert_eq!(r.seek(SeekFrom::End(0)).unwrap(), 300_000);
        assert!(l.complete() && l.error().is_none());
        assert_eq!(*s.opens.lock(), vec![0], "nothing asked for past the end");

        // Read past real end is end.
        for says in [true, false] {
            let s = Arc::new(estimating(3_000_000, 3_200_000, says));
            let l = Loader::start(s.clone(), "song".into(), LOAD, Some(30_000), None);
            let mut r = l.reader();
            read(&mut r, 1000);
            // Where an Ogg reader looks for the last page.
            let probe = 3_200_000 - 65_307;
            r.seek(SeekFrom::Start(probe)).unwrap();
            assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0, "nothing there: the end (says {says})");
            assert!(l.error().is_none(), "not a failure: {:?}", l.error());
            assert_eq!(l.length(), says.then_some(3_000_000), "the real length, or none known (says {says})");
            assert_eq!(l.shortened(), 1);
            assert_eq!(s.opens.lock().iter().filter(|&&o| o == probe).count(), 1, "asked once: {:?}", s.opens.lock());

            r.seek(SeekFrom::Start(2_999_000)).unwrap();
            let tail = read(&mut r, 1000);
            assert!(tail.iter().enumerate().all(|(i, &b)| b == (2_999_000 + i) as u8));
            assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0);
            assert_eq!(l.length(), Some(3_000_000), "the end found where the bytes stop (says {says})");
        }

        // Broken body at end learns end.
        let mut e = estimating(300_000, 320_000, false);
        e.breaks_at_end = true;
        let s = Arc::new(e);
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(3_000), None);
        let mut r = l.reader();
        read(&mut r, 300_000);
        assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0);
        assert_eq!(l.length(), Some(300_000));
        assert!(l.error().is_none());
        assert_eq!(*s.opens.lock(), vec![0, 300_000], "asked again where it broke, and told that is the end");

        // Rangeless short answer is end.
        struct Whole(u64, Mutex<Vec<u64>>);
        impl ByteSource for Whole {
            fn open(&self, _: &str, from: u64) -> Result<Body, OpenError> {
                self.1.lock().push(from);
                Ok(Body { start: 0, len: Some(self.0 + 50_000), reader: Box::new(Made { at: 0, len: self.0, served: Arc::default() }) })
            }
        }
        let s = Arc::new(Whole(2_000_000, Mutex::new(Vec::new())));
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(20_000), None);
        let mut r = l.reader();
        read(&mut r, 1000);
        r.seek(SeekFrom::Start(2_020_000)).unwrap();
        assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0);
        assert_eq!(l.length(), Some(2_000_000), "where the whole answer ended");
        assert!(l.error().is_none());
    }

    #[test]
    fn dropped_connection_resumes() {
        let e = estimating(600_000, 640_000, true);
        *e.cut.lock() = Some(150_000);
        let s = Arc::new(e);
        let l = Loader::start(s.clone(), "song".into(), LOAD, Some(6_000), None);
        let mut r = l.reader();
        let all = read(&mut r, 600_000);
        assert!(all.iter().enumerate().all(|(i, &b)| b == i as u8), "every byte, in order");
        assert_eq!(r.read(&mut [0u8; 16]).unwrap(), 0);
        assert_eq!(s.opens.lock()[..2], [0, 150_000], "asked again from where it broke");
        assert_eq!(l.length(), Some(600_000), "ended where the bytes did, not where the network dropped");
        assert!(l.error().is_none());
    }

    #[test]
    fn abandoned_readers() {
        // Regression: readers giving up left the wanted one asleep until its timeout (music stopped at a
        // song's end).
        /// Answers only when let, so all readers are waiting.
        struct Late(Arc<Counting>, Arc<Signal>);
        impl ByteSource for Late {
            fn open(&self, url: &str, from: u64) -> Result<Body, OpenError> {
                self.1.reach(1);
                self.0.open(url, from)
            }
        }
        let blocked = |l: &Loader| l.0.state.lock().blocked;
        let answer = Arc::new(Signal::default());
        let l = Loader::start(Arc::new(Late(server(2_000_000), answer.clone())), "song".into(), LOAD, Some(20_000), None);
        let mut wanted = l.reader();
        let player = std::thread::spawn(move || {
            let mut b = [0u8; 16];
            let n = wanted.read(&mut b);
            (n, Instant::now())
        });
        let stop = Arc::new(AtomicBool::new(false));
        let others: Vec<_> = (0..8)
            .map(|_| {
                let mut r = l.reader_until(stop.clone());
                std::thread::spawn(move || r.read(&mut [0u8; 16]).is_err())
            })
            .collect();
        // Ten with this wait.
        until(&l, "the player's reader and eight opened for nothing all wait on the bytes", |s| s.blocked == 10);
        stop.store(true, Ordering::Release);
        l.nudge();
        let gave_up = others.into_iter().map(|o| o.join().unwrap()).filter(|&e| e).count();
        assert_eq!(gave_up, 8, "the readers nobody wants give up");
        assert_eq!(blocked(&l), 1, "the player's reader still waits, and is counted as waiting");
        let answered = Instant::now();
        answer.bump();
        let (n, at) = player.join().unwrap();
        assert_eq!(n.expect("the bytes"), 16);
        let took = at - answered;
        assert!(took < Duration::from_secs(3), "read as the bytes came, not after the {READ_TIMEOUT:?} timeout: {took:?}");

        // Unwanted reader stops waiting.
        /// Answers from past the start only when let.
        struct Slow(Arc<Counting>, Arc<Signal>);
        impl ByteSource for Slow {
            fn open(&self, url: &str, from: u64) -> Result<Body, OpenError> {
                if from > 0 {
                    self.1.reach(1);
                }
                self.0.open(url, from)
            }
        }
        let answer = Arc::new(Signal::default());
        let l = Loader::start(Arc::new(Slow(server(5_000_000), answer.clone())), "song".into(), LOAD, Some(50_000), None);
        let stop = Arc::new(AtomicBool::new(false));
        let mut r = l.reader_until(stop.clone());
        read(&mut r, 1000);
        r.seek(SeekFrom::Start(4_000_000)).unwrap();
        let (l2, stop2) = (l.clone(), stop.clone());
        std::thread::spawn(move || {
            until(&l2, "the reader waits", |s| s.blocked == 2);
            stop2.store(true, Ordering::Release);
            l2.nudge();
        });
        assert!(r.read(&mut [0u8; 16]).is_err(), "given up");
        assert!(r.read(&mut [0u8; 16]).is_err(), "and for good");
        assert_eq!(l.0.state.lock().bursts, 1, "given up before the bytes came");
        answer.bump();
    }

    // ---- requests called off ----

    /// Never answers (or never sends a byte) until cancelled; counts requests and cancellations. A
    /// cancellation is counted as the client's hook runs: a request called off before its body was
    /// read is never read.
    #[derive(Default)]
    struct Silent {
        headers: bool,
        asked: Signal,
        called_off: Arc<Signal>,
    }

    /// A body that blocks until its request is called off.
    struct Nothing(Arc<Signal>);

    impl Read for Nothing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            self.0.reach(1);
            Err(io::Error::other("Canceled"))
        }
    }

    impl ByteSource for Silent {
        fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
            unreachable!("asked as a request that may be called off")
        }

        fn open_cancellable(&self, _: &str, _: Option<&str>, from: u64, cancel: &Cancel) -> Result<Body, OpenError> {
            self.asked.bump();
            let off = Arc::new(Signal::default());
            let (o, counted) = (off.clone(), self.called_off.clone());
            cancel.on_cancel(move || {
                counted.bump();
                o.bump();
            });
            if self.headers {
                off.reach(1);
                return Err("Canceled".into());
            }
            Ok(Body { start: from, len: Some(1_000_000), reader: Box::new(Nothing(off)) })
        }
    }

    #[test]
    fn timeouts() {
        let fetching = Arc::new(Fetching::default());
        let s = Silent { headers: true, ..Silent::default() };
        let cancel = Cancel::stalling_after(&fetching, 100);
        let t = Instant::now();
        let got = open_watched(&s, "ext-1", None, 0, &cancel);
        assert_eq!(got.err(), Some(OpenError::TimedOut));
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        // A body whose bytes stop fails its read as timed out.
        let s = Silent { headers: false, ..Silent::default() };
        let cancel = Cancel::stalling_after(&fetching, 100);
        let Ok(mut b) = open_watched(&s, "ext-1", None, 0, &cancel) else { panic!("the headers came") };
        let e = b.reader.read(&mut [0u8; 16]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);

        // Timeout fails song at once.
        struct Late(Mutex<u32>);
        impl ByteSource for Late {
            fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
                *self.0.lock() += 1;
                Err(OpenError::TimedOut)
            }
        }
        let s = Arc::new(Late(Mutex::new(0)));
        let l = Loader::start(s.clone(), "ext-1".into(), LOAD, Some(3_000), None);
        let mut r = l.reader();
        assert!(r.read(&mut [0u8; 16]).is_err());
        assert_eq!(l.error(), Some(OpenError::TimedOut.to_string()));
        assert_eq!(l.answered(), None, "the network's failure, not the server's");
        assert_eq!(*s.0.lock(), 1, "asked once");
    }

    #[test]
    fn let_go_songs_stop() {
        struct Down(Signal, Arc<Signal>);
        impl ByteSource for Down {
            fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
                self.0.bump();
                Err("down".into())
            }
        }
        impl Drop for Down {
            fn drop(&mut self) {
                self.1.bump();
            }
        }
        let gone = Arc::new(Signal::default());
        let s = Arc::new(Down(Signal::default(), gone.clone()));
        let l = Loader::start_within(&Arc::default(), s.clone(), "song".into(), LOAD, Some(3_000), None, None, None, Waits { stall_ms: 20_000, retry_ms: 60_000 });
        s.0.reach(1);
        drop((s, l));
        gone.reach(1);

        // Dropped song cancels hung request.
        for headers in [true, false] {
            let s = Arc::new(Silent { headers, ..Silent::default() });
            let l = Loader::start(s.clone(), "ext-1".into(), LOAD, Some(3_000), None);
            s.asked.reach(1);
            drop(l);
            s.called_off.reach(1);
            assert_eq!(*s.asked.0.lock(), 1, "and not asked again (headers {headers})");
        }
    }

    /// A server that answers and breaks every body before a byte fails the song after a few tries,
    /// resting between them, rather than being asked again and again at once.
    #[test]
    fn broken_body_fails_after_retries() {
        struct Resets(Mutex<u32>);
        struct Reset;
        impl Read for Reset {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"))
            }
        }
        impl ByteSource for Resets {
            fn open(&self, _: &str, from: u64) -> Result<Body, OpenError> {
                *self.0.lock() += 1;
                Ok(Body { start: from, len: Some(1_000), reader: Box::new(Reset) })
            }
        }
        let s = Arc::new(Resets(Mutex::new(0)));
        let l = Loader::start_within(&Arc::default(), s.clone(), "song".into(), LOAD, Some(3_000), None, None, None, Waits { stall_ms: 20_000, retry_ms: 1 });
        until(&l, "the song fails", |s| s.done);
        assert!(l.error().is_some(), "failed");
        assert_eq!(*s.0.lock(), RETRIES + 1, "asked once and again {RETRIES} times");
    }

    #[test]
    fn crowding_cancels_oldest_per_source() {
        let asking = Fetching::default();
        let off = Arc::new(Mutex::new(Vec::new()));
        let ask = |source: usize, k: usize| {
            let c = Cancel::new();
            let o = off.clone();
            c.on_cancel(move || o.lock().push(k));
            asking.asking(source, &c);
            c
        };
        let calls: Vec<Cancel> = (0..MAX_ASKING + 2).map(|k| ask(1, k)).collect();
        assert_eq!(*off.lock(), [0, 1], "the two oldest called off");
        assert!(calls[2..].iter().all(|c| !c.cancelled()), "the newest go on");
        let other = ask(2, 99);
        assert!(!other.cancelled() && off.lock().len() == 2, "another source has its own cap");
        // An answer makes room.
        asking.answered(&calls[5]);
        ask(1, 100);
        assert_eq!(off.lock().len(), 2);
    }
}
