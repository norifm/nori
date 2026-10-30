//! A deep cast sink (`nori_plugins::deep`): the device holds tens of seconds and takes nori's audio in
//! the bursts it asks for, as Android's AudioTrack takes the ring's ([`AudioOutput::bursts`]).
//!
//! The device reads its socket always and, when low, sends one `Want { frames }`: the pump writes that
//! much and sleeps until the next, woken only by the device, the engine (a command, or music written
//! into the ring at the end of its turn) or the socket failing; never by a timer. Control frames go
//! out at once, never behind unasked audio. The place heard is the last `Status` run on by the clock;
//! a flush keeps what the device holds before the music made again ([`Kept`]), so a sound change
//! carries on seamlessly and a seek lands where it should. Each ring's format goes first in-band, so a
//! new song's format reaches the device where the engine reopens, after what it holds of the last
//! ([`AudioOutput::takes_formats`]). Each song start goes as `Now` (and its cover once) at its frame.
//!
//! While nothing touches the samples and the device decodes the song's codec, the engine's offload path
//! sends the file's own packets instead ([`CastOffload`], the same connection and counts).

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::{JoinHandle, Thread};

use nori_plugins::deep::{Codec, Deep, Device, Format, Frame};
use parking_lot::{Condvar, Mutex};

use crate::offload::{Coded, CodedSong, Coding, OffloadOutput, Support};
use crate::output::{AudioOutput, Feed, Kept, OutputFormat};

/// Depth when the plugin prefers none, and the bounds of one it prefers, ms.
const DEPTH_MS: u32 = 60_000;
const DEPTH_MIN_MS: u32 = 5_000;
const DEPTH_MAX_MS: u32 = 120_000;
/// Depth while a sound page is tuned: a change is heard about this soon.
const SHALLOW_MS: u32 = 250;
/// Of the device's buffer, what the depth may fill (its own bookkeeping, a packet's overshoot).
const BUFFER_SHARE: f64 = 0.9;
/// Frames per audio frame sent.
const CHUNK: usize = 4096;
/// Covers the device keeps by id, as last sent; older ones are sent again if a song needs them.
const COVERS: usize = 32;

/// The clock the counts run on, µs.
pub type NowUs = Arc<dyn Fn() -> i64 + Send + Sync>;

/// What a device shows of the songs, from the client's library.
pub trait Shows: Send + Sync {
    fn now(&self, id: &str) -> Option<Shown>;
    /// Song `id`'s cover at `size` px square, as JPEG, and what tells it from other songs' (an album's
    /// share one). Called on the pump's thread, or the engine's while offloaded: from the cover cache.
    fn cover(&self, id: &str, size: u32) -> Option<(String, Vec<u8>)>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shown {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
}

/// The device's place: frames played at `at_us`, running on from there while it plays.
#[derive(Debug, Clone, Copy, Default)]
struct Anchor {
    at_us: i64,
    played: u64,
    delay_us: u32,
    running: bool,
}

/// Rates of the stream from each frame on (the formats sent), oldest first.
#[derive(Debug, Default)]
struct Rates(VecDeque<(u64, u32)>);

impl Rates {
    fn rate_at(&self, frame: u64) -> u32 {
        self.0.iter().rev().find(|(from, _)| *from <= frame).or(self.0.front()).map_or(48_000, |r| r.1).max(1)
    }

    /// µs from frame `a` to `b`.
    fn us(&self, a: u64, b: u64) -> u64 {
        if b <= a {
            return 0;
        }
        let mut us = 0;
        let mut at = a;
        while at < b {
            let rate = self.rate_at(at) as u64;
            let end = self.0.iter().map(|r| r.0).find(|&f| f > at).unwrap_or(u64::MAX).min(b);
            us += (end - at) * 1_000_000 / rate;
            at = end;
        }
        us
    }

    /// The frame `us` after frame `a`, no further than `most`.
    fn after(&self, a: u64, us: u64, most: u64) -> u64 {
        let (mut at, mut left) = (a, us);
        while at < most {
            let rate = self.rate_at(at) as u64;
            let end = self.0.iter().map(|r| r.0).find(|&f| f > at).unwrap_or(u64::MAX).min(most);
            let span = (end - at) * 1_000_000 / rate;
            if left < span {
                return at + left * rate / 1_000_000;
            }
            left -= span;
            at = end;
        }
        most
    }

    fn from(&mut self, frame: u64, rate: u32) {
        while self.0.back().is_some_and(|r| r.0 >= frame) {
            self.0.pop_back();
        }
        self.0.push_back((frame, rate));
    }

    /// Forgets the rates before `frame` but the one it plays at.
    fn trim(&mut self, frame: u64) {
        while self.0.len() > 1 && self.0[1].0 <= frame {
            self.0.pop_front();
        }
    }
}

/// What the engine, the device and the pump share.
struct State {
    // What the engine asks.
    playing: bool,
    shallow: bool,
    ramps: VecDeque<(Option<f32>, f32, i64)>,
    /// A flush to look for in the ring.
    flush: bool,
    /// The next ring goes on after what the device holds ([`AudioOutput::goes_on`]); otherwise its
    /// first step drops that.
    goes_on: bool,
    drop_held: bool,
    closed: bool,
    float: bool,
    /// The open ring's format, and whether it is still to be said before its first audio.
    format: Option<Format>,
    format_due: bool,
    /// Song starts by ring frame ([`AudioOutput::song_at`]).
    marks: VecDeque<(u64, String)>,
    // What was told the device.
    told_playing: bool,
    told_depth: Option<u32>,
    flushes: u32,
    // What the device said, and the counts.
    credit: i64,
    anchor: Anchor,
    /// Stream frames sent since Hello, as the device counts them.
    sent: u64,
    rates: Rates,
    /// A flush whose answer is awaited: its seq, what it kept, and frames sent since.
    flushing: Option<(u32, Option<u64>, u64)>,
    gone: bool,
    failure_said: bool,
    bursting: bool,
    // Shown.
    covers: VecDeque<(String, u32)>,
    next_id: u32,
    // Offloaded.
    offloaded: Option<Offloaded>,
    /// The engine, woken when the device asks for more while offloaded (not a test's: its device says).
    engine: Option<Thread>,
    asked: bool,
    by_hand: bool,
}

/// The offload path's song as the device plays it.
#[derive(Default)]
struct Offloaded {
    /// The device's count when the track was opened or flushed: the offload path counts from it.
    base: u64,
    /// The next song's start, to send before its first packet: its id, gain and stream info.
    song: Option<(String, f32)>,
    header: Option<Vec<u8>>,
}

struct Shared {
    deep: Arc<Deep>,
    now: NowUs,
    shows: Option<Arc<dyn Shows>>,
    /// The rate and channels the plugin said, if any.
    rate: Option<u32>,
    channels: Option<u8>,
    st: Mutex<State>,
    cv: Condvar,
}

impl Shared {
    fn wake(&self) {
        self.cv.notify_all();
    }

    /// The device's count now, by the last anchor.
    fn played(&self, st: &State) -> u64 {
        let a = st.anchor;
        if !a.running {
            return a.played;
        }
        let run = ((self.now)() - a.at_us).max(0) as u64;
        st.rates.after(a.played, run, st.sent.max(a.played))
    }

    /// How long until everything sent is heard, µs.
    fn latency_us(&self, st: &State) -> u64 {
        let played = self.played(st);
        st.anchor.delay_us as u64 + st.rates.us(played, st.sent)
    }

    /// Anchors the count at `played` now.
    fn anchor(&self, st: &mut State, played: u64, delay_us: Option<u32>) {
        let delay_us = delay_us.unwrap_or(st.anchor.delay_us);
        st.anchor = Anchor { at_us: (self.now)(), played, delay_us, running: st.told_playing };
        st.rates.trim(played);
    }

    /// The depth the device holds now, ms.
    fn depth_ms(&self, st: &State) -> u32 {
        if st.shallow {
            SHALLOW_MS
        } else {
            self.full_depth_ms(st.format)
        }
    }

    /// The preferred depth fitted to the device's buffer at `format`, ms.
    fn full_depth_ms(&self, format: Option<Format>) -> u32 {
        let want = self.deep.depth_ms.unwrap_or(DEPTH_MS).clamp(DEPTH_MIN_MS, DEPTH_MAX_MS);
        let Some(f) = format else { return want };
        let bits = if f.codec == Codec::Pcm { f.bits.max(16) as u64 } else { 16 };
        let per_s = f.rate as u64 * f.channels.max(1) as u64 * bits / 8;
        let fits = (self.deep.caps.buffer_bytes as f64 * BUFFER_SHARE * 1000.0 / per_s.max(1) as f64) as u32;
        want.min(fits).max(SHALLOW_MS)
    }

    /// What the device said.
    fn heard(&self, said: Option<Device>) {
        let mut st = self.st.lock();
        match said {
            None => st.gone = true,
            Some(Device::Want { frames }) => {
                st.credit = frames as i64;
                st.asked = true;
                if let Some(t) = st.engine.as_ref().filter(|_| st.offloaded.is_some() && !st.by_hand) {
                    t.unpark();
                }
            }
            Some(Device::Status { played, delay_us, .. }) => self.anchor(&mut st, played, Some(delay_us)),
            Some(Device::Flushed { seq, played }) => {
                if let Some((_, keep, since)) = st.flushing.filter(|f| f.0 == seq) {
                    st.flushing = None;
                    st.sent = keep.unwrap_or(played) + since;
                    if let Some(o) = st.offloaded.as_mut() {
                        o.base = played;
                    }
                }
                self.anchor(&mut st, played, None);
            }
            Some(_) => {}
        }
        drop(st);
        self.wake();
    }

    /// Encodes a flush onto `out`: the device keeps what it holds before stream frame `keep` (none:
    /// nothing).
    fn flush(&self, st: &mut State, keep: Option<u64>, out: &mut Vec<u8>) {
        st.flushes += 1;
        let seq = st.flushes;
        Frame::Flush { seq, keep }.encode(out);
        match keep {
            Some(k) => st.sent = k.min(st.sent),
            // Until it answers: where it is now, all dropped.
            None => st.sent = self.played(st),
        }
        st.flushing = Some((seq, keep, 0));
        st.format_due = true;
        if let Some(f) = st.format {
            st.rates.from(st.sent, f.rate);
        }
    }

    /// Frames sent: counted.
    fn sent(&self, st: &mut State, frames: u64) {
        st.sent += frames;
        if let Some(f) = st.flushing.as_mut() {
            f.2 += frames;
        }
        st.credit -= frames as i64;
    }

    /// The controls asked for and not told yet, onto `out`.
    fn controls(&self, st: &mut State, out: &mut Vec<u8>) {
        let depth = self.depth_ms(st);
        if st.told_depth != Some(depth) {
            st.told_depth = Some(depth);
            Frame::Depth { ms: depth }.encode(out);
        }
        if st.told_playing != st.playing {
            let played = self.played(st);
            st.told_playing = st.playing;
            self.anchor(st, played, None);
            if st.playing { Frame::Resume } else { Frame::Pause }.encode(out);
        }
        while let Some((from, target, ms)) = st.ramps.pop_front() {
            if let Some(v) = from {
                Frame::Ramp { target: v, ms: 0 }.encode(out);
            }
            Frame::Ramp { target, ms: ms.clamp(0, u32::MAX as i64) as u32 }.encode(out);
        }
    }

    /// The format, before the first audio after it changed or a flush.
    fn format(&self, st: &mut State, out: &mut Vec<u8>) {
        if !st.format_due {
            return;
        }
        let Some(f) = st.format else { return };
        st.format_due = false;
        Frame::Format(f).encode(out);
        st.rates.from(st.sent, f.rate);
        // The depth fits the format.
        let depth = self.depth_ms(st);
        if st.told_depth != Some(depth) {
            st.told_depth = Some(depth);
            Frame::Depth { ms: depth }.encode(out);
        }
    }

    /// Song `id`'s start, onto `out`: its cover once (if the device shows covers), then `Now`. Called
    /// without the state's lock.
    fn now_playing(&self, id: &str, out: &mut Vec<u8>) {
        let Some(shows) = &self.shows else { return };
        let shown = shows.now(id).unwrap_or_default();
        let cover = self.deep.caps.cover.and_then(|size| shows.cover(id, size));
        let mut st = self.st.lock();
        let cover = cover.map(|(key, jpeg)| match st.covers.iter().find(|c| c.0 == key) {
            Some(c) => c.1,
            None => {
                st.next_id += 1;
                let cid = st.next_id;
                Frame::Cover { id: cid, jpeg: &jpeg }.encode(out);
                st.covers.push_back((key, cid));
                if st.covers.len() > COVERS {
                    st.covers.pop_front();
                }
                cid
            }
        });
        st.next_id += 1;
        Frame::Now { id: st.next_id, title: &shown.title, artist: &shown.artist, album: &shown.album, duration_ms: shown.duration_ms, cover }.encode(out);
    }

    fn send(&self, out: &[u8]) {
        if !out.is_empty() && !self.deep.send_raw(out) {
            self.st.lock().gone = true;
        }
    }

    /// The burst (the Wi-Fi lock) runs while credit is left and music is coming.
    fn burst(&self, st: &mut State, on: bool) {
        if st.bursting != on {
            st.bursting = on;
            self.deep.bursting(on);
        }
    }
}

/// The pump: moves the ring to the device as it asks, on a thread of its own (or a test's hand).
pub struct Pump {
    feed: Feed,
    shared: Arc<Shared>,
    samples: Vec<f32>,
    bytes: Vec<u8>,
}

/// What a pump step found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pumped {
    /// It sent or did something: step again.
    More,
    /// Nothing to do until the device, the engine or the ring says something.
    Idle,
    /// The output closed or the device is gone.
    Done,
}

impl Pump {
    /// The engine waits for the ring's low mark ([`Feed::engine_waits`]): for a test's device.
    pub fn engine_waits(&self) -> bool {
        self.feed.engine_waits()
    }

    /// One step.
    pub fn step(&mut self) -> Pumped {
        let sh = self.shared.clone();
        let mut out = std::mem::take(&mut self.bytes);
        out.clear();
        let mut st = sh.st.lock();
        if st.closed {
            return Pumped::Done;
        }
        if st.gone {
            if !std::mem::replace(&mut st.failure_said, true) {
                self.feed.wake_engine();
            }
            return Pumped::Done;
        }
        sh.controls(&mut st, &mut out);
        if std::mem::take(&mut st.drop_held) {
            sh.flush(&mut st, None, &mut out);
        }
        // A flush surfaces on a pull: an empty one finds it without credit.
        st.flush = false;
        self.feed.pull(&mut []);
        self.feed.flushed();
        self.flushed(&mut st, &mut out);
        let at = self.feed.position();
        // A song's start is told before its first frame.
        if st.marks.front().is_some_and(|m| m.0 <= at) && st.credit > 0 && self.feed.available() > 0 {
            let (_, id) = st.marks.pop_front().expect("checked");
            sh.format(&mut st, &mut out);
            drop(st);
            sh.now_playing(&id, &mut out);
            sh.send(&out);
            self.bytes = out;
            return Pumped::More;
        }
        let Some(f) = st.format else {
            drop(st);
            sh.send(&out);
            self.bytes = out;
            return Pumped::Idle;
        };
        let next_mark = st.marks.front().map_or(u64::MAX, |m| m.0);
        let n = (st.credit.max(0) as usize).min(self.feed.available()).min(CHUNK).min(next_mark.saturating_sub(at) as usize);
        let ending = self.feed.ending() && self.feed.available() == 0;
        if n == 0 {
            let on = st.credit > 0 && !ending && st.told_playing;
            sh.burst(&mut st, on);
            drop(st);
            let did = !out.is_empty();
            sh.send(&out);
            self.bytes = out;
            return if did { Pumped::More } else { Pumped::Idle };
        }
        sh.burst(&mut st, true);
        sh.format(&mut st, &mut out);
        let ch = f.channels as usize;
        self.samples.resize(n * ch, 0.0);
        let got = self.feed.pull(&mut self.samples);
        if self.feed.flushed() {
            // A flush between the look and the pull: what the pull took is taken again after it.
            self.feed.rewind_to(self.feed.position().saturating_sub(got as u64));
            self.flushed(&mut st, &mut out);
            drop(st);
            sh.send(&out);
            self.bytes = out;
            return Pumped::More;
        }
        sh.sent(&mut st, got as u64);
        drop(st);
        let samples = &self.samples[..got * ch];
        let at = out.len();
        out.extend_from_slice(&[0; 5]);
        match f.bits {
            24 => {
                for &v in samples {
                    let s = (v * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                    out.extend_from_slice(&s.to_le_bytes()[..3]);
                }
            }
            _ => {
                for &v in samples {
                    out.extend_from_slice(&((v * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
                }
            }
        }
        // An Audio frame in place: its length and kind before the samples.
        let len = (out.len() - at - 4) as u32;
        out[at..at + 4].copy_from_slice(&len.to_le_bytes());
        out[at + 4] = AUDIO_KIND;
        sh.send(&out);
        self.bytes = out;
        Pumped::More
    }

    /// The ring was flushed: the device keeps what it holds before the music made again, or nothing.
    fn flushed(&mut self, st: &mut State, out: &mut Vec<u8>) {
        let sh = &self.shared;
        match self.feed.kept() {
            Kept::Unflushed => {}
            Kept::Nothing => {
                let at = self.feed.position();
                st.marks.retain(|m| m.0 >= at);
                sh.flush(st, None, out);
            }
            Kept::Before(from) => {
                let back = self.feed.position().saturating_sub(from);
                let keep = st.sent.saturating_sub(back);
                self.feed.rewind_to(from);
                st.marks.retain(|m| m.0 < from);
                sh.flush(st, Some(keep), out);
            }
        }
    }

    /// Runs until closed: a step, then sleep until something is said.
    fn run(mut self) {
        loop {
            match self.step() {
                Pumped::More => continue,
                Pumped::Done => return,
                Pumped::Idle => {
                    let sh = self.shared.clone();
                    let mut st = sh.st.lock();
                    while !st.closed && !st.gone && !self.due(&st) {
                        sh.cv.wait(&mut st);
                    }
                }
            }
        }
    }

    /// Whether a step has something to do.
    fn due(&self, st: &State) -> bool {
        let sh = &self.shared;
        st.flush || st.drop_held || st.told_playing != st.playing || !st.ramps.is_empty() || st.told_depth != Some(sh.depth_ms(st)) || (st.credit > 0 && self.feed.available() > 0)
    }
}

/// The Audio frame's kind byte (nori-sink-protocol's).
const AUDIO_KIND: u8 = 0x04;

/// The deep sink as the engine's output.
pub struct DeepOutput {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// A test's: the pump is stepped by hand, not run on a thread of its own.
    by_hand: Option<Arc<Mutex<Option<Pump>>>>,
}

impl DeepOutput {
    /// The output and offload output into `deep`, its counts on the clock `now`.
    pub fn new(deep: Arc<Deep>, rate: Option<u32>, channels: Option<u8>, shows: Option<Arc<dyn Shows>>, now: NowUs) -> (DeepOutput, CastOffload) {
        let st = State {
            playing: false,
            shallow: false,
            ramps: VecDeque::new(),
            flush: false,
            goes_on: false,
            drop_held: false,
            closed: false,
            float: false,
            format: None,
            format_due: false,
            marks: VecDeque::new(),
            told_playing: false,
            told_depth: None,
            flushes: 0,
            credit: 0,
            anchor: Anchor::default(),
            sent: 0,
            rates: Rates::default(),
            flushing: None,
            gone: deep.gone(),
            failure_said: false,
            bursting: false,
            covers: VecDeque::new(),
            next_id: 0,
            offloaded: None,
            engine: None,
            asked: false,
            by_hand: false,
        };
        let shared = Arc::new(Shared { deep: deep.clone(), now, shows, rate, channels, st: Mutex::new(st), cv: Condvar::new() });
        let to = Arc::downgrade(&shared);
        deep.listen(Box::new(move |said| {
            if let Some(s) = to.upgrade() {
                s.heard(said);
            }
        }));
        (DeepOutput { shared: shared.clone(), thread: None, by_hand: None }, CastOffload { shared })
    }

    /// The pump is stepped by the caller ([`DeepOutput::pump`]) instead of a thread: a test's device.
    pub fn by_hand(mut self) -> (DeepOutput, Arc<Mutex<Option<Pump>>>) {
        let slot = Arc::new(Mutex::new(None));
        self.by_hand = Some(slot.clone());
        self.shared.st.lock().by_hand = true;
        (self, slot)
    }

    fn tell(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.shared.st.lock());
        self.shared.wake();
    }
}

impl AudioOutput for DeepOutput {
    /// The song's own format unless the plugin said one; resampled only above the device's highest rate.
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        let caps = self.shared.deep.caps;
        let rate = self.shared.rate.unwrap_or(want.rate);
        let rate = if caps.max_rate > 0 && rate > caps.max_rate { nori_player::policy::capped_rate(rate, caps.max_rate).min(caps.max_rate) } else { rate };
        let channels = self.shared.channels.unwrap_or(want.channels.clamp(1, 8) as u8);
        let mut st = self.shared.st.lock();
        let bits = if (st.float && caps.takes_bits(24)) || !caps.takes_bits(16) { 24 } else { 16 };
        let format = Format { rate, channels, bits, codec: Codec::Pcm };
        st.format_due |= st.format != Some(format);
        st.format = Some(format);
        Ok(OutputFormat { rate, channels: channels as usize, bits: 0 })
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        self.close();
        let mut st = self.shared.st.lock();
        st.closed = false;
        st.marks.clear();
        // A new ring but for a new format is new music: what the device holds goes.
        st.drop_held = !std::mem::take(&mut st.goes_on) && st.sent > 0;
        drop(st);
        let pump = Pump { feed, shared: self.shared.clone(), samples: Vec::new(), bytes: Vec::new() };
        if let Some(slot) = &self.by_hand {
            *slot.lock() = Some(pump);
            return Ok(());
        }
        let t = std::thread::Builder::new().name("nori-cast".into()).spawn(move || pump.run()).map_err(|e| e.to_string())?;
        self.thread = Some(t);
        Ok(())
    }

    fn pause(&mut self) {
        self.tell(|st| st.playing = false);
    }

    fn resume(&mut self) {
        self.tell(|st| st.playing = true);
    }

    /// Until everything sent is heard, by the device's last word.
    fn latency_us(&self) -> u64 {
        let st = self.shared.st.lock();
        self.shared.latency_us(&st)
    }

    fn takes_float(&mut self) -> bool {
        true
    }

    fn float(&mut self, on: bool) {
        self.shared.st.lock().float = on;
    }

    fn flush(&mut self) {
        self.tell(|st| st.flush = true);
    }

    fn fades(&self) -> bool {
        true
    }

    fn ramp(&mut self, from: Option<f32>, target: f32, ms: i64) {
        self.tell(|st| st.ramps.push_back((from, target, ms)));
    }

    fn holding(&self) -> bool {
        let st = self.shared.st.lock();
        self.shared.played(&st) < st.sent
    }

    fn bursts(&self) -> bool {
        true
    }

    fn shallow(&mut self, on: bool) {
        self.tell(|st| st.shallow = on);
    }

    fn failed(&mut self) -> Option<String> {
        self.shared.st.lock().gone.then(|| "the cast's device is gone".to_string())
    }

    fn takes_formats(&self) -> bool {
        true
    }

    fn goes_on(&mut self) {
        self.shared.st.lock().goes_on = true;
    }

    fn depth_us(&self) -> i64 {
        let format = self.shared.st.lock().format;
        self.shared.full_depth_ms(format) as i64 * 1000
    }

    fn wrote(&mut self) {
        self.shared.wake();
    }

    fn marks_songs(&self) -> bool {
        self.shared.shows.is_some()
    }

    fn song_at(&mut self, frame: u64, id: &str) {
        self.tell(|st| {
            st.marks.retain(|m| m.0 < frame);
            st.marks.push_back((frame, id.to_string()));
        });
    }

    fn close(&mut self) {
        self.tell(|st| st.closed = true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(slot) = &self.by_hand {
            slot.lock().take();
        }
    }
}

impl Drop for DeepOutput {
    fn drop(&mut self) {
        self.close();
    }
}

/// The deep sink as the engine's offload output: the song file's own packets, while nothing touches
/// the samples and the device decodes them (`nori_player::policy`, as for a phone's DSP).
pub struct CastOffload {
    shared: Arc<Shared>,
}

impl CastOffload {
    fn lock(&self) -> parking_lot::MutexGuard<'_, State> {
        self.shared.st.lock()
    }

    fn control(&mut self, f: impl FnOnce(&mut State)) {
        let mut st = self.lock();
        f(&mut st);
        let mut out = Vec::new();
        self.shared.controls(&mut st, &mut out);
        drop(st);
        self.shared.send(&out);
    }
}

/// FLAC's stream marker and its stream info as the last metadata block: what a decoder reads first.
fn flac_head(info: &[u8]) -> Vec<u8> {
    let mut h = b"fLaC".to_vec();
    h.extend_from_slice(&[0x80, 0, 0, info.len() as u8]);
    h.extend_from_slice(info);
    h
}

impl OffloadOutput for CastOffload {
    fn supports(&mut self, coded: Coded) -> Support {
        let caps = self.shared.deep.caps;
        if caps.max_rate > 0 && coded.rate > caps.max_rate {
            return Support::No;
        }
        match coded.coding {
            Coding::Flac if caps.decodes(Codec::Flac) => Support::Gapless,
            // The device is not told an MP3's encoder gap.
            Coding::Mp3 if caps.decodes(Codec::Mp3) => Support::Plain,
            _ => Support::No,
        }
    }

    /// Drops what the device holds and says the codec before the first packet.
    fn open(&mut self, coded: Coded, bytes: usize) -> Result<usize, String> {
        let sh = self.shared.clone();
        let mut st = sh.st.lock();
        if st.gone {
            return Err("the cast's device is gone".into());
        }
        st.engine = Some(std::thread::current());
        let mut out = Vec::new();
        sh.flush(&mut st, None, &mut out);
        let codec = if coded.coding == Coding::Flac { Codec::Flac } else { Codec::Mp3 };
        st.format = Some(Format { rate: coded.rate, channels: coded.channels.clamp(1, 8) as u8, bits: 0, codec });
        st.format_due = true;
        let base = st.sent;
        st.offloaded = Some(Offloaded { base, ..Offloaded::default() });
        // The depth at the codec, so the device asks.
        sh.controls(&mut st, &mut out);
        drop(st);
        sh.send(&out);
        Ok(bytes)
    }

    /// One whole packet, once the device asked for more: its credit may be overshot by less than a packet.
    fn write(&mut self, data: &[u8], frames: u64) -> Result<usize, i32> {
        let sh = self.shared.clone();
        let mut st = sh.st.lock();
        if st.gone {
            return Err(-1);
        }
        if st.credit <= 0 {
            return Ok(0);
        }
        let mut out = Vec::new();
        sh.format(&mut st, &mut out);
        let song = st.offloaded.as_mut().and_then(|o| o.song.take().map(|s| (s, o.header.take())));
        drop(st);
        if let Some(((id, level), header)) = song {
            Frame::Gain { db: 20.0 * level.max(1e-6).log10() }.encode(&mut out);
            sh.now_playing(&id, &mut out);
            if let Some(h) = header {
                Frame::Encoded(&h).encode(&mut out);
            }
        }
        Frame::Encoded(data).encode(&mut out);
        let mut st = sh.st.lock();
        sh.sent(&mut st, frames);
        let on = st.told_playing && st.credit > 0;
        sh.burst(&mut st, on);
        drop(st);
        sh.send(&out);
        Ok(data.len())
    }

    fn delay_padding(&mut self, _delay: u32, _padding: u32) {}

    fn end_of_stream(&mut self) -> bool {
        true
    }

    fn play(&mut self) {
        self.control(|st| st.playing = true);
    }

    fn pause(&mut self) {
        let sh = self.shared.clone();
        self.control(|st| st.playing = false);
        sh.burst(&mut sh.st.lock(), false);
    }

    fn flush(&mut self) {
        let sh = self.shared.clone();
        let mut st = sh.st.lock();
        let mut out = Vec::new();
        sh.flush(&mut st, None, &mut out);
        let base = st.sent;
        if let Some(o) = st.offloaded.as_mut() {
            o.base = base;
        }
        drop(st);
        sh.send(&out);
    }

    /// Fades, one step at a time, as the device's ramp; ReplayGain goes in-band ([`OffloadOutput::song`]).
    fn set_volume(&mut self, volume: f32) {
        let mut out = Vec::new();
        Frame::Ramp { target: volume, ms: nori_player::transport::FADE_TICK_MS as u32 }.encode(&mut out);
        self.shared.send(&out);
    }

    fn head(&mut self) -> Option<u64> {
        let st = self.lock();
        let base = st.offloaded.as_ref()?.base;
        Some(self.shared.played(&st).saturating_sub(base))
    }

    fn timestamp(&mut self) -> Option<u64> {
        None
    }

    fn data_requested(&mut self) -> bool {
        std::mem::take(&mut self.lock().asked)
    }

    fn presented(&mut self) -> bool {
        let st = self.lock();
        self.shared.played(&st) >= st.sent
    }

    fn torn_down(&mut self) -> bool {
        self.lock().gone
    }

    fn close(&mut self) {
        let mut st = self.lock();
        st.offloaded = None;
        st.engine = None;
    }

    fn note(&mut self, _what: &str) {}

    fn whole_packets(&self) -> bool {
        true
    }

    fn song(&mut self, song: &CodedSong, id: &str, level: f32) -> bool {
        let mut st = self.lock();
        let Some(o) = st.offloaded.as_mut() else { return true };
        o.song = Some((id.to_string(), level));
        o.header = (song.coded.coding == Coding::Flac).then(|| song.setup.as_deref().map(flac_head)).flatten();
        true
    }
}
