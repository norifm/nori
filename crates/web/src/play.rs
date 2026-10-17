//! The player build: the session every client shares (`nori_host::session`) over the worker's `fetch`,
//! its engine driven by the worker (no threads), its sound pulled by the AudioWorklet through the page's
//! messages, its reports handed to the page as JSON.

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

use nori_core::look::clock_on;
use nori_core::settings::SavedServer;
use nori_core::{PageOrigin, Song};
use nori_engine::{AudioOutput, Body, ByteSource, Feed, Filling, OpenError, OutputFormat, State};
use nori_host::session::{Driven, Note, Open, Said, Session};
use nori_look::lyrics::LyricClock;
use nori_tasks::{Local, OneThread};
use parking_lot::Mutex;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use crate::fetch::Fetch;
use crate::{error, Nori};

/// How far ahead of the ear the worklet is kept fed, ms.
const AHEAD_MS: u64 = 500;

/// What the worklet is told to do, in order ([`Nori::orders`]).
enum Order {
    /// Stop where it is, keeping what it holds.
    Pause,
    Resume,
    /// Drop what it holds, answered with the frames dropped ([`Nori::flushed`]).
    Flush,
    /// Fade to `to` over `ms`, from `from` or where it is.
    Ramp { from: Option<f32>, to: f32, ms: i64 },
}

/// The AudioWorklet's side: what the worker pulls for it, and what it holds.
#[derive(Default)]
struct Card {
    feed: Option<Feed>,
    playing: bool,
    /// The AudioContext's rate.
    rate: u32,
    /// From the AudioContext to the ear, µs.
    ear_us: u64,
    /// Frames pulled for the worklet, and those it had played at its last report, made at `reported_ms`.
    sent: u64,
    played: u64,
    reported_ms: f64,
    orders: Vec<Order>,
    /// Flushes the worklet has not answered: nothing is pulled until it has given back what it dropped.
    flushing: u32,
}

impl Card {
    /// Frames the worklet holds now: those sent less those played by its last report and since.
    fn holds(&self) -> u64 {
        let held = self.sent.saturating_sub(self.played);
        if !self.playing {
            return held;
        }
        let since = (js_sys::Date::now() - self.reported_ms).max(0.0) * self.rate as f64 / 1000.0;
        held.saturating_sub(since as u64)
    }
}

/// The engine's output: the worklet, told at once what the engine does, so a pause, seek or skip is
/// heard now rather than after the music it holds.
struct Output(Arc<Mutex<Card>>);

impl AudioOutput for Output {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        let rate = self.0.lock().rate;
        Ok(OutputFormat { rate, channels: want.channels.min(2), bits: 0 })
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        self.0.lock().feed = Some(feed);
        Ok(())
    }

    fn pause(&mut self) {
        let mut card = self.0.lock();
        card.playing = false;
        card.orders.push(Order::Pause);
    }

    fn resume(&mut self) {
        let mut card = self.0.lock();
        card.playing = true;
        card.orders.push(Order::Resume);
    }

    fn latency_us(&self) -> u64 {
        let card = self.0.lock();
        card.holds() * 1_000_000 / card.rate.max(1) as u64 + card.ear_us
    }

    fn takes_float(&mut self) -> bool {
        true
    }

    fn flush(&mut self) {
        let mut card = self.0.lock();
        card.flushing += 1;
        card.orders.push(Order::Flush);
    }

    /// On the worklet's own level, heard at once.
    fn ramp(&mut self, from: Option<f32>, target: f32, ms: i64) -> bool {
        self.0.lock().orders.push(Order::Ramp { from, to: target, ms });
        true
    }

    fn holding(&self) -> bool {
        let card = self.0.lock();
        card.sent > card.played
    }

    fn close(&mut self) {
        self.0.lock().feed = None;
    }
}

/// A function of the page's, called from the module's one thread.
#[derive(Clone)]
pub struct Js(OneThread<js_sys::Function>);

impl Js {
    fn call(&self) {
        let _ = self.0.call0(&JsValue::NULL);
    }

    fn say(&self, v: &Value) {
        let _ = self.0.call1(&JsValue::NULL, &JsValue::from_str(&v.to_string()));
    }
}

/// The player's songs are streamed into their loaders.
impl ByteSource for Fetch<Js> {
    fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
        Err("nothing blocks in a browser: the bytes are pushed".into())
    }

    fn push(&self, fill: Filling) -> Option<Filling> {
        let wake = self.0.clone();
        crate::stream::push(fill, move || wake.call());
        None
    }
}

/// A profile's session, open.
struct Opened {
    host: Session,
    driven: RefCell<Driven>,
}

/// The player's state in the worker.
pub struct Live {
    tell: Js,
    wake: Js,
    card: Arc<Mutex<Card>>,
    /// What the session said since the last turn.
    said: Arc<Mutex<Vec<Said>>>,
    open: RefCell<Option<Opened>>,
    /// The lyrics shown, and their clock.
    lyrics: RefCell<Option<(String, LyricClock)>>,
}

impl Live {
    pub fn new(tell: js_sys::Function, wake: js_sys::Function) -> Live {
        Live { tell: Js(OneThread(tell)), wake: Js(OneThread(wake)), card: Arc::new(Mutex::new(Card { rate: 48_000, ..Card::default() })), said: Arc::default(), open: RefCell::default(), lyrics: RefCell::default() }
    }
}

impl Nori {
    pub(crate) fn enter(&self, profile: SavedServer) -> Result<(), JsValue> {
        self.leave();
        let live = &self.live;
        let (said, wake) = (live.said.clone(), live.wake.clone());
        let out: nori_host::session::Out = Arc::new(move |s| {
            said.lock().push(s);
            wake.call();
        });
        let open = Open {
            queue: self.app.clone(),
            data: Path::new(""),
            http: Arc::new(Fetch(live.wake.clone())),
            profile,
            output: Box::new(Output(live.card.clone())),
            volume: nori_host::Level::new(1.0, None),
            memory_mb: 128,
            covers: None,
            offline: false,
            mpris: None,
            device: nori_core::remote::RemoteMe { name: "Browser".into(), kind: nori_core::remote::wire::DeviceKind::Desktop },
            discovery: None,
            tasks: Arc::new(Local),
            keeps_music: false,
            decoders: nori_engine::Platform { aac: None, later: Some(crate::later::decoder) },
            finds_active: false,
            out,
        };
        let wake = live.wake.clone();
        let (host, driven) = Session::open_driven(open, move || wake.call()).map_err(error)?;
        host.check();
        *live.open.borrow_mut() = Some(Opened { host, driven: RefCell::new(driven) });
        Ok(())
    }

    /// Saves the queue and closes the session.
    pub(crate) fn leave(&self) {
        if let Some(o) = self.live.open.borrow_mut().take() {
            o.host.close();
        }
    }

    pub(crate) fn client(&self) -> Option<Arc<nori_core::client::Client>> {
        self.live.open.borrow().as_ref().map(|o| o.host.client.clone())
    }

    fn with<R>(&self, f: impl FnOnce(&Session) -> R) -> Option<R> {
        self.live.open.borrow().as_ref().map(|o| f(&o.host))
    }

    /// Hands what the session said to the page, and the engine's events to the session.
    fn drain(&self) {
        let said = std::mem::take(&mut *self.live.said.lock());
        for s in said {
            if let Said::Lyrics { song, pick } = &s {
                let at = self.with(|h| h.engine.status().position_now()).unwrap_or(0);
                *self.live.lyrics.borrow_mut() = Some((song.clone(), clock_on(&pick.lyrics, at)));
            }
            if let Some(v) = told(&s) {
                self.live.tell.say(&v);
            }
        }
    }
}

/// The player.
#[wasm_bindgen]
impl Nori {
    /// The AudioContext's rate and its latency to the ear, before the first Play.
    pub fn set_card(&self, rate: u32, ear_ms: f64) {
        let mut card = self.live.card.lock();
        card.rate = rate;
        card.ear_us = (ear_ms.max(0.0) * 1000.0) as u64;
    }

    /// Plays `songs` (JSON, as a page read them) from `start`, shuffled with `shuffle`; `kind` and `id`
    /// name the page they are the list of (an album, a playlist), or are empty.
    pub fn play(&self, songs: String, start: usize, shuffle: bool, kind: String, id: String) -> Result<(), JsValue> {
        let songs: Vec<Song> = serde_json::from_str(&songs).map_err(error)?;
        let from = serde_json::from_value::<nori_core::OriginKind>(Value::String(kind)).ok().map(|k| PageOrigin::new(k, id.as_str()));
        self.with(|h| h.play(songs, start, shuffle, from)).ok_or_else(|| error("not signed in"))
    }

    /// Adds `songs` (JSON) after the current one (`next`) or at the end.
    pub fn enqueue(&self, songs: String, next: bool) -> Result<(), JsValue> {
        let songs: Vec<Song> = serde_json::from_str(&songs).map_err(error)?;
        self.with(|h| h.enqueue(songs, next)).ok_or_else(|| error("not signed in"))
    }

    pub fn toggle(&self) {
        self.with(|h| h.toggle());
    }

    pub fn next(&self) {
        self.with(|h| h.next());
    }

    pub fn previous(&self) {
        self.with(|h| h.previous());
    }

    pub fn seek(&self, ms: f64) {
        self.with(|h| h.seek(ms as i64));
    }

    /// Plays the queue's song at `index`.
    pub fn jump(&self, index: usize) {
        self.with(|h| h.jump(index));
    }

    pub fn remove(&self, index: usize) {
        self.with(|h| h.remove(index));
    }

    pub fn move_song(&self, from: usize, to: usize) {
        self.with(|h| h.move_song(from, to));
    }

    pub fn shuffle(&self, on: bool) {
        self.with(|h| h.shuffle(on));
    }

    /// The next repeat mode after the one shown: off, all, one.
    pub fn cycle_repeat(&self) {
        self.with(|h| h.cycle_repeat());
    }

    /// Sets a setting by name and applies it; false if there is no such setting.
    pub fn setting(&self, name: String, value: String) -> bool {
        self.with(|h| h.setting(&name, &value).is_some()).unwrap_or_else(|| self.app.settings.edit_by_name(&name, &value).is_some())
    }

    /// Asks for `song`'s lyrics; they come as a `lyrics` report.
    pub fn lyrics(&self, song: String) {
        self.with(|h| h.lyrics(song));
    }

    /// The lyrics' state at the position now: `{ song, active, glide, sung, wait }` (`wait`: ms until it
    /// changes, 0 never), or null without lyrics.
    pub fn lyrics_now(&self) -> String {
        let lyrics = self.live.lyrics.borrow();
        let Some((song, clock)) = lyrics.as_ref() else { return "null".into() };
        let at = self.with(|h| h.engine.status().position_now()).unwrap_or(0);
        let step = clock.advance(at, false, false, true);
        json!({ "song": song, "active": step.frame.active, "glide": step.frame.glide_ms, "sung": step.frame.sung, "wait": step.wait }).to_string()
    }

    /// What plays: `{ state, index, id, positionMs, mixing, song, queue, current, shuffle, repeat,
    /// underruns, transition }` (`transition`: the plan out of the song, `{ kind, into, reason }`); for a
    /// jam guest, the host's (`jam: true`).
    pub fn now(&self) -> String {
        self.with(|h| {
            if let Some(there) = h.jam_playing() {
                let m = &there.mirror;
                let song = there.song().map(|s| crate::pages::song(h.core.as_ref(), s));
                let queue: Vec<Value> = m.rows.iter().map(|r| crate::pages::song(h.core.as_ref(), &r.song)).collect();
                return json!({
                    "jam": true,
                    "state": if m.playing { "Playing" } else { "Paused" },
                    "index": m.at,
                    "id": there.song().map(|s| s.id.clone()),
                    "positionMs": there.position_ms(),
                    "pace": if m.playing { m.rate } else { 0.0 },
                    "mixing": m.mixing,
                    "song": song,
                    "queue": queue,
                    "current": m.at,
                    "shuffle": m.shuffle,
                    "repeat": 0,
                    "underruns": 0,
                })
                .to_string();
            }
            let s = h.engine.status();
            let q = &h.core.session;
            let (ids, current, shuffle, repeat) = q.playlist(|p| (p.ids().to_vec(), p.current(), p.shuffling(), p.repeat()));
            let queue: Vec<Value> = ids.iter().map(|id| q.song(id).map_or(json!({ "id": id }), |s| crate::pages::song(h.core.as_ref(), &s))).collect();
            json!({
                "state": format!("{:?}", s.state),
                "index": s.index,
                "id": s.id,
                "positionMs": s.position_now(),
                "pace": s.pace,
                "mixing": s.mixing,
                "song": s.id.as_deref().and_then(|id| q.song(id)).map(|song| crate::pages::song(h.core.as_ref(), &song)),
                "queue": queue,
                "current": current,
                "shuffle": shuffle,
                "repeat": repeat,
                "underruns": s.underruns,
                "transition": s.id.as_deref().and_then(|id| q.planner.transition_note(id)).map(|t| json!({ "kind": format!("{:?}", t.kind), "into": t.incoming_id, "reason": t.reason })),
            })
            .to_string()
        })
        .unwrap_or_else(|| "null".into())
    }

    /// Runs the engine and hands on what was said: ms until it wants to run again, -1 for when the page
    /// next pulls or asks.
    pub fn turn(&self) -> f64 {
        let next = self.live.open.borrow().as_ref().and_then(|o| o.driven.borrow_mut().turn());
        self.drain();
        match next {
            Some(Some(ms)) => ms as f64,
            _ => -1.0,
        }
    }

    /// Fills `out` with interleaved samples for the worklet, as many as the engine has made up to
    /// [`AHEAD_MS`] ahead of what it played; returns the frames written. Nothing while paused or while a
    /// flush is unanswered.
    pub fn pull(&self, out: &mut [f32]) -> usize {
        let mut card = self.live.card.lock();
        let room = (card.rate as u64 * AHEAD_MS / 1000).saturating_sub(card.sent.saturating_sub(card.played)) as usize;
        if !card.playing || card.flushing > 0 {
            return 0;
        }
        let Some(f) = card.feed.as_mut() else { return 0 };
        let ch = f.format().channels.max(1);
        let n = (out.len() / ch).min(f.available()).min(room);
        if n == 0 {
            return 0;
        }
        let got = f.pull(&mut out[..n * ch]);
        card.sent += got as u64;
        got
    }

    /// What the worklet is to do, in order: a JSON list of its messages, empty for none.
    pub fn orders(&self) -> String {
        let orders = std::mem::take(&mut self.live.card.lock().orders);
        if orders.is_empty() {
            return String::new();
        }
        let told: Vec<Value> = orders
            .into_iter()
            .map(|o| match o {
                Order::Pause => json!({ "do": "pause" }),
                Order::Resume => json!({ "do": "resume" }),
                Order::Flush => json!({ "do": "flush" }),
                Order::Ramp { from, to, ms } => json!({ "do": "ramp", "from": from, "to": to, "ms": ms }),
            })
            .collect();
        Value::Array(told).to_string()
    }

    /// The worklet's report: `played` frames in all.
    pub fn played(&self, played: f64) {
        let mut card = self.live.card.lock();
        card.played = played as u64;
        card.reported_ms = js_sys::Date::now();
    }

    /// The worklet answered `flushes` flushes, dropping `dropped` frames it held: they are taken back
    /// from the ring, as far as the flush kept them, so the place heard stays exact.
    pub fn flushed(&self, played: f64, dropped: f64, flushes: u32) {
        self.played(played);
        let mut card = self.live.card.lock();
        let dropped = dropped as u64;
        card.sent = card.sent.saturating_sub(dropped);
        card.flushing = card.flushing.saturating_sub(flushes);
        if let Some(f) = card.feed.as_mut() {
            f.rewind(dropped);
        }
    }

    /// The output's channels once open, else 0.
    pub fn channels(&self) -> usize {
        self.live.card.lock().feed.as_ref().map_or(0, |f| f.format().channels)
    }

    /// Whether music plays (the worklet running dry is then a fault).
    pub fn playing(&self) -> bool {
        self.with(|h| h.engine.status().state == State::Playing).unwrap_or(false)
    }
}

/// Jams: hosting one on the server's relay, or a guest of someone's (Spotify's Jam, as on Android).
#[wasm_bindgen]
impl Nori {
    /// The jam as the page shows it: `{ start, hosting, guest, link, host, you, members: [{ id, name,
    /// role }], pending: [{ request, from, title, artist }], along, listening, asks }`.
    pub fn jam(&self) -> String {
        self.with(|h| {
            let r = h.remote();
            let start = r.as_ref().map_or(nori_core::remote::JamStart::Unavailable, |r| r.jam_start());
            let view = r.as_ref().and_then(|r| r.jam_view());
            let controls = r.as_ref().and_then(|r| r.jam_controls());
            json!({
                "start": format!("{start:?}"),
                "guest": h.guest,
                "hosting": view.as_ref().is_some_and(|v| v.hosting),
                "link": view.as_ref().and_then(|v| v.link.clone()),
                "host": view.as_ref().map(|v| v.host().to_string()),
                "you": view.as_ref().map(|v| v.you.clone()),
                "members": view.as_ref().map_or(Vec::new(), |v| v.members.iter().map(|m| json!({ "id": m.id, "name": m.name, "role": format!("{:?}", m.role) })).collect()),
                "pending": view.as_ref().map_or(Vec::new(), |v| v.pending.iter().map(|p| json!({ "request": p.request, "from": p.from_name, "title": p.song.title, "artist": p.song.artist })).collect()),
                "along": view.as_ref().is_some_and(|v| v.along),
                "listening": view.as_ref().map(|v| format!("{:?}", v.listening)),
                "asks": controls.is_some_and(|c| c.asks),
            })
            .to_string()
        })
        .unwrap_or_else(|| "null".into())
    }

    /// Starts hosting a jam; its invite link (null when one is being started already).
    pub async fn jam_open(&self) -> Result<Option<String>, JsValue> {
        let remote = self.with(|h| h.remote()).flatten().ok_or_else(|| error("remote control is off"))?;
        remote.jam_open().await.map_err(error)
    }

    /// Ends the jam this device hosts.
    pub fn jam_close(&self) {
        if let Some(r) = self.with(|h| h.remote()).flatten() {
            r.jam_close();
        }
    }

    /// Lets the jam's guests listen along, or not.
    pub fn jam_along(&self, on: bool) {
        if let Some(r) = self.with(|h| h.remote()).flatten() {
            r.jam_along(on);
        }
    }

    /// Accepts or declines request `request` (host or admin).
    pub fn jam_decide(&self, request: f64, accept: bool) {
        if let Some(r) = self.with(|h| h.remote()).flatten() {
            r.jam_act(nori_core::remote::wire::Op::Decide { request: request as u64, accept });
        }
    }

    /// Listens along with the jam this guest is in, or only shows it.
    pub fn jam_listen(&self, on: bool) {
        if let Some(r) = self.with(|h| h.remote()).flatten() {
            r.listen(on);
        }
    }

    /// Joins the jam `link` invites to as `name` and opens its guest profile: "joined", "own" (this
    /// device's jam) or "ended".
    pub async fn jam_join(&self, link: String, name: String) -> Result<String, JsValue> {
        use nori_core::remote::JamJoin;
        let remote = self.with(|h| h.remote()).flatten();
        let transport: Arc<dyn nori_core::transport::Transport> = Arc::new(Fetch(()));
        let pass = match nori_core::remote::jam_join(transport, self.app.settings.clone(), remote, link, name.clone()).await.map_err(error)? {
            JamJoin::Joined { pass } => pass,
            JamJoin::Own => return Ok("own".into()),
            JamJoin::Ended => return Ok("ended".into()),
        };
        let guest = nori_host::jam_joined(&self.app.settings, pass, &name);
        self.enter(guest)?;
        Ok("joined".into())
    }

    /// Leaves the jam this guest is in and opens the profile used before it: true when there is one
    /// (false: sign in).
    pub fn jam_leave(&self) -> Result<bool, JsValue> {
        if let Some(r) = self.with(|h| h.remote()).flatten() {
            r.jam_leave();
        }
        self.leave();
        match nori_host::jam_left(&self.app.settings) {
            Some(back) => self.enter(back).map(|_| true),
            None => Ok(false),
        }
    }
}

/// A report of the session's as the page takes it.
fn told(s: &Said) -> Option<Value> {
    Some(match s {
        Said::Engine(e) => json!({ "kind": "engine", "event": format!("{e:?}") }),
        Said::Lyrics { song, pick } => json!({ "kind": "lyrics", "song": song, "lyrics": pick.lyrics }),
        Said::Search(_) => return None,
        Said::Note(n) => json!({ "kind": "note", "note": note(n) }),
        Said::Reachable(r) => json!({ "kind": "reachable", "ok": r.is_ok() }),
        Said::Remote => json!({ "kind": "remote" }),
        Said::JamEnded { host } => json!({ "kind": "jamEnded", "host": host }),
        Said::Volume(v) => json!({ "kind": "volume", "volume": v }),
        Said::Curve(_) => return None,
        Said::Starred(_) => json!({ "kind": "starred" }),
    })
}

/// A note's name and number, for the page to word.
fn note(n: &Note) -> Value {
    match n {
        Note::Queued { next, songs } => json!({ "what": "queued", "next": next, "songs": songs }),
        Note::NothingToPlay => json!({ "what": "nothingToPlay" }),
        Note::SongsFailed(e) => json!({ "what": "songsFailed", "why": e.to_string() }),
        Note::NothingToPutBack => json!({ "what": "nothingToPutBack" }),
        Note::StarFailed(e) => json!({ "what": "starFailed", "why": e.to_string() }),
        _ => json!({ "what": "other" }),
    }
}
