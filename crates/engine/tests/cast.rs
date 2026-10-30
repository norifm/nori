//! A deep cast sink end to end on the virtual clock: the engine plays into `cast::deep`, and a fake
//! device speaking nori-sink-protocol reads it always, asks for more when low, and plays what it holds
//! in time. Tests read what the device received and played.

use crate::common::{self, Stepper, Virtual};

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use nori_engine::cast::{DeepOutput, Pump, Pumped, Shown, Shows};
use nori_engine::{AudioOutput, Config, Engine, Event, Feed, Library, Located, OutputFormat, Settings, Source};
use nori_player::playlist::Playlist;
use nori_player::sim::{self, prefs_off};
use nori_player::transitions::{TransitionPrefs, WindowSong};
use nori_plugins::deep::{Caps, Deep};
use nori_sink_protocol::{Codec, Device as Said, Format, Frames, Host as Frame, VERSION};
use parking_lot::Mutex;

/// The bytes the host writes to the device.
#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Wire {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Something the device holds, at its frame in the stream.
#[derive(Debug, Clone)]
enum Item {
    Format(Format),
    Now { title: String, cover: Option<u32> },
    Gain(f32),
    /// PCM samples, interleaved, of so many channels.
    Pcm(Vec<i32>, usize),
    /// One packet and its frames.
    Packet(Vec<u8>, u64),
}

/// What the device did, at the stream frame it had played to.
#[derive(Debug, Clone, PartialEq)]
pub enum Did {
    Format(Format),
    Now(String, Option<u32>),
    Gain(f32),
    Cover(u32),
    Flush(Option<u64>),
    Pause,
    Resume,
    Volume(f32),
}

/// The fake device.
pub struct Fake {
    pump: Arc<Mutex<Option<Pump>>>,
    deep: Arc<Deep>,
    wire: Wire,
    frames: Frames,
    depth_ms: u32,
    /// Held: each item and the stream frame it starts at.
    held: VecDeque<(u64, Item)>,
    /// Frames received (the stream's end as held), and played, since Hello.
    received: u64,
    pub played: u64,
    /// Incoming audio before this frame is dropped (a flush that came after the device played past it).
    skip_to: u64,
    format: Option<Format>,
    playing: bool,
    /// Credit asked for and not received yet.
    owed: i64,
    due_ns: i64,
    last_ns: i64,
    /// Every frame played (PCM, left channel at 16 bits), what the device did, and the packets played.
    pub heard: Vec<i32>,
    pub did: Vec<(u64, Did)>,
    pub packets: Vec<u8>,
    pub wants: u64,
    /// The most it ever held, ms.
    pub most_held_ms: u64,
    /// Credit overshot, frames, at most.
    pub overshoot: i64,
    /// Said by the test: sent at the next tick.
    pub say: Vec<Said>,
    /// Ticks it played with nothing held: an underrun.
    pub dry: u64,
}

const TICK_NS: i64 = 10_000_000;

impl Fake {
    fn rate(&self) -> u32 {
        self.format.map_or(44_100, |f| f.rate)
    }

    fn held_frames(&self) -> u64 {
        self.received - self.played
    }

    fn said(&mut self, s: Said) {
        self.deep.heard(s);
    }

    fn status(&mut self) {
        let s = Said::Status { played: self.played, held: self.held_frames() as u32, delay_us: 0 };
        self.said(s);
    }

    /// Takes what the host wrote.
    fn read(&mut self) {
        let bytes = std::mem::take(&mut *self.wire.0.lock());
        let mut frames = std::mem::take(&mut self.frames);
        frames.push(&bytes);
        let mut replies = Vec::new();
        while let Some(f) = frames.next_host() {
            let f = f.expect("a frame the protocol knows");
            let at = self.received;
            match f {
                Frame::Hello { .. } => {}
                Frame::Depth { ms } => {
                    self.depth_ms = ms;
                    replies.push(None);
                }
                Frame::Format(f) => self.held.push_back((at, Item::Format(f))),
                Frame::Audio(b) => {
                    let f = self.held.iter().rev().find_map(|(_, i)| match i {
                        Item::Format(f) => Some(*f),
                        _ => None,
                    });
                    let f = f.or(self.format).expect("a format before audio");
                    assert_eq!(f.codec, Codec::Pcm, "PCM only after a PCM format");
                    let width = f.bits as usize / 8;
                    let samples: Vec<i32> = b.chunks_exact(width).map(|s| if width == 2 { i16::from_le_bytes([s[0], s[1]]) as i32 } else { i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8 }).collect();
                    let frames = (samples.len() / f.channels as usize) as u64;
                    self.take_audio(Item::Pcm(samples, f.channels as usize), frames);
                }
                Frame::Encoded(b) => {
                    let frames = packet_frames(b);
                    self.take_audio(Item::Packet(b.to_vec(), frames), frames);
                }
                Frame::Gain { db } => self.held.push_back((at, Item::Gain(db))),
                Frame::Now { title, cover, .. } => self.held.push_back((at, Item::Now { title: title.to_string(), cover })),
                Frame::Cover { id, .. } => self.did.push((self.played, Did::Cover(id))),
                Frame::Flush { seq, keep } => {
                    self.did.push((self.played, Did::Flush(keep)));
                    let keep = keep.filter(|&k| k >= self.played);
                    let to = keep.unwrap_or(self.played);
                    // Audio from `to` on goes; the last Now and covers stay.
                    let mut kept = VecDeque::new();
                    for (a, item) in self.held.drain(..) {
                        match item {
                            Item::Pcm(s, ch) if a < to => {
                                let n = ((to - a) as usize * ch).min(s.len());
                                kept.push_back((a, Item::Pcm(s[..n].to_vec(), ch)));
                            }
                            Item::Packet(..) if a < to => kept.push_back((a, item)),
                            Item::Format(_) | Item::Gain(_) | Item::Now { .. } if a < to => kept.push_back((a, item)),
                            _ => {}
                        }
                    }
                    self.held = kept;
                    self.skip_to = self.played;
                    self.received = to;
                    replies.push(Some(Said::Flushed { seq, played: self.played }));
                }
                Frame::Pause => {
                    self.playing = false;
                    self.did.push((self.played, Did::Pause));
                    replies.push(None);
                }
                Frame::Resume => {
                    self.playing = true;
                    self.did.push((self.played, Did::Resume));
                    replies.push(None);
                }
                Frame::Ramp { .. } => {}
                Frame::Volume { gain } => self.did.push((self.played, Did::Volume(gain))),
            }
        }
        self.frames = frames;
        for r in replies {
            if let Some(r) = r {
                self.said(r);
            }
            self.status();
        }
    }

    /// Audio of `frames` arrived: held, unless the device already played past it (after a flush).
    fn take_audio(&mut self, item: Item, frames: u64) {
        let at = self.received;
        self.received += frames;
        self.owed -= frames as i64;
        self.overshoot = self.overshoot.max(-self.owed);
        if frames > 0 && self.received <= self.skip_to {
            return;
        }
        let item = match item {
            Item::Pcm(s, ch) if at < self.skip_to => Item::Pcm(s[(self.skip_to - at) as usize * ch..].to_vec(), ch),
            i => i,
        };
        self.held.push_back((at.max(self.skip_to), item));
    }

    /// Plays `n` frames of what it holds.
    fn play(&mut self, mut n: u64) {
        while n > 0 {
            let Some((at, item)) = self.held.pop_front() else { return };
            match item {
                Item::Format(f) => {
                    self.format = Some(f);
                    self.did.push((self.played, Did::Format(f)));
                }
                Item::Now { title, cover } => self.did.push((self.played, Did::Now(title, cover))),
                Item::Gain(db) => self.did.push((self.played, Did::Gain(db))),
                Item::Pcm(s, ch) => {
                    let frames = (s.len() / ch) as u64;
                    let take = n.min(frames);
                    let shift = if self.format.is_some_and(|f| f.bits == 24) { 8 } else { 0 };
                    self.heard.extend(s[..take as usize * ch].iter().step_by(ch).map(|v| v >> shift));
                    self.played += take;
                    n -= take;
                    if take < frames {
                        self.held.push_front((at + take, Item::Pcm(s[take as usize * ch..].to_vec(), ch)));
                    }
                }
                Item::Packet(b, frames) => {
                    self.packets.extend_from_slice(&b);
                    self.played += frames;
                    n = n.saturating_sub(frames);
                }
            }
        }
        // Marks at the place reached are done before the next frame.
        while let Some((_, item)) = self.held.front().filter(|(_, i)| !matches!(i, Item::Pcm(..) | Item::Packet(..))) {
            match item.clone() {
                Item::Format(f) => {
                    self.format = Some(f);
                    self.did.push((self.played, Did::Format(f)));
                }
                Item::Now { title, cover } => self.did.push((self.played, Did::Now(title, cover))),
                Item::Gain(db) => self.did.push((self.played, Did::Gain(db))),
                _ => unreachable!(),
            }
            self.held.pop_front();
        }
    }

    /// Steps the pump until it rests.
    fn pump(&mut self) -> bool {
        let mut woke = false;
        // What the offload path wrote on the engine's thread.
        self.read();
        loop {
            let mut slot = self.pump.lock();
            let Some(p) = slot.as_mut() else { break };
            let waits = p.engine_waits();
            let step = p.step();
            // A pump that found the device gone woke the engine to say so.
            woke |= (waits && !p.engine_waits()) || step == Pumped::Done;
            drop(slot);
            self.read();
            if step != Pumped::More {
                break;
            }
        }
        woke
    }
}

/// Frames in one packet: a FLAC frame's block size from its header, else an MP3 frame's 1152.
fn packet_frames(b: &[u8]) -> u64 {
    if b.len() > 6 && b[0] == 0xff && b[1] & 0xfe == 0xf8 {
        let code = b[2] >> 4;
        // Past the frame number (UTF-8 coded).
        let lead = b[4];
        let n = if lead < 0x80 { 1 } else { lead.leading_ones() as usize };
        let at = 4 + n;
        return match code {
            1 => 192,
            2..=5 => 576 << (code - 2),
            6 => b[at] as u64 + 1,
            7 => u16::from_be_bytes([b[at], b[at + 1]]) as u64 + 1,
            8..=15 => 256 << (code - 8),
            _ => 0,
        };
    }
    if b.starts_with(b"fLaC") {
        return 0;
    }
    1152
}

impl common::Device for Fake {
    fn due_ns(&self) -> i64 {
        self.due_ns
    }

    fn tick(&mut self, now_ns: i64) -> bool {
        let mut woke = self.pump();
        for s in std::mem::take(&mut self.say) {
            self.said(s);
        }
        if self.playing {
            let n = ((now_ns - self.last_ns).max(0) as u128 * self.rate() as u128 / 1_000_000_000) as u64;
            if self.held_frames() < n && self.received > 0 {
                self.dry += 1;
            }
            self.play(n);
        }
        self.last_ns = now_ns;
        self.due_ns = now_ns + TICK_NS;
        let held_ms = self.held_frames() * 1000 / self.rate() as u64;
        self.most_held_ms = self.most_held_ms.max(held_ms);
        // Low, with what it asked for in: asks for the depth.
        if self.depth_ms > 0 && held_ms * 2 < self.depth_ms as u64 && self.owed <= 0 {
            let frames = (self.depth_ms as u64 * self.rate() as u64 / 1000).saturating_sub(self.held_frames());
            self.owed = frames as i64;
            self.wants += 1;
            self.status();
            self.said(Said::Want { frames: frames as u32 });
            woke |= true;
        }
        woke |= self.pump();
        woke
    }
}

/// An output that never plays: the engine's own until the cast takes the music.
struct Quiet;

impl AudioOutput for Quiet {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        Ok(want)
    }
    fn start(&mut self, _feed: Feed) -> Result<(), String> {
        Ok(())
    }
    fn pause(&mut self) {}
    fn resume(&mut self) {}
    fn latency_us(&self) -> u64 {
        0
    }
    fn close(&mut self) {}
}

struct Files {
    dir: Arc<nori_testdir::TempDir>,
    songs: Vec<(String, i64, &'static str)>,
}

impl Library for Files {
    fn locate(&mut self, id: &str) -> Result<Located, String> {
        let (_, ms, hint) = self.songs.iter().find(|s| s.0 == id).ok_or("no such song")?;
        Ok(Located { source: Source::File(vec![self.dir.join(id)]), hint: Some((*hint).into()), duration_ms: Some(*ms), estimated: false, from_second: None })
    }

    fn about(&self, id: &str) -> WindowSong {
        let ms = self.songs.iter().find(|s| s.0 == id).map_or(0, |s| s.1);
        WindowSong { id: id.into(), title: id.into(), duration_ms: ms, album_id: Some("album".into()), ..Default::default() }
    }
}

struct Queue(Arc<Mutex<Playlist>>);

impl nori_engine::Queue for Queue {
    fn read<R>(&self, f: impl FnOnce(&Playlist) -> R) -> R {
        f(&self.0.lock())
    }
    fn moved_to(&mut self, index: usize) {
        self.0.lock().moved_to(index);
    }
    fn set_repeat(&mut self, mode: u8) {
        self.0.lock().set_repeat(mode);
    }
    fn skips(&self, _: &Playlist, _: usize) -> bool {
        false
    }
}

/// What the device shows: each song's id as its title, and a cover per album (the id's first letter).
pub struct Covers;

impl Shows for Covers {
    fn now(&self, id: &str) -> Option<Shown> {
        Some(Shown { title: id.into(), ..Shown::default() })
    }
    fn cover(&self, id: &str, _size: u32) -> Option<(String, Vec<u8>)> {
        Some((id[..1].to_string(), vec![0xff, 0xd8, 0xff, 0xd9]))
    }
}

pub struct Song {
    pub id: &'static str,
    pub file: Vec<u8>,
    pub ms: i64,
    pub hint: &'static str,
}

/// A WAV song of `samples` (stereo) at `rate`.
pub fn wav(id: &'static str, rate: u32, samples: &[i16]) -> Song {
    Song { id, file: common::wav(rate, samples), ms: (samples.len() / 2) as i64 * 1000 / rate as i64, hint: "wav" }
}

pub struct Rig {
    pub engine: Arc<Engine>,
    pub time: Stepper<Fake>,
    pub dev: Arc<Mutex<Fake>>,
    pub events: Arc<Mutex<Vec<Event>>>,
    _dir: Arc<nori_testdir::TempDir>,
}

pub fn caps() -> Caps {
    Caps { version: VERSION, buffer_bytes: 16 << 20, max_rate: 96_000, bits: 1 | 2, decodes: Codec::Flac.bit() | Codec::Mp3.bit(), cover: None }
}

impl Rig {
    pub fn new(songs: Vec<Song>, prefs: TransitionPrefs, settings: Settings, caps: Caps, shows: Option<Arc<dyn Shows>>) -> Rig {
        Rig::with_app(songs, { let mut a = sim::App::new(); a.prefs = prefs; a }, settings, caps, shows)
    }

    pub fn with_app(songs: Vec<Song>, app: sim::App, settings: Settings, caps: Caps, shows: Option<Arc<dyn Shows>>) -> Rig {
        let dir = Arc::new(nori_testdir::TempDir::new("cast-deep"));
        for s in &songs {
            std::fs::write(dir.join(s.id), &s.file).unwrap();
        }
        let mut list = Playlist::default();
        list.set(songs.iter().map(|s| s.id.to_string()).collect(), Some(0), false, 0);
        let list = Arc::new(Mutex::new(list));
        let library = Files { dir: dir.clone(), songs: songs.iter().map(|s| (s.id.to_string(), s.ms, s.hint)).collect() };
        let clock = Virtual::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let config = Config { settings, ..Config::default() };
        let engine = Arc::new(Engine::start_on(library, app, Queue(list), Box::new(Quiet), None, config, clock.clone(), move |e| seen.lock().push(e)));
        let wire = Wire::default();
        let deep = Deep::new(caps, None, Box::new(wire.clone()));
        let at = clock.clone();
        let (out, offload) = DeepOutput::new(deep.clone(), None, None, shows, Arc::new(move || at.now_ns() / 1000));
        let (out, pump) = out.by_hand();
        let dev = Arc::new(Mutex::new(Fake {
            pump,
            deep,
            wire: wire.clone(),
            frames: Frames::new(),
            depth_ms: 0,
            held: VecDeque::new(),
            received: 0,
            played: 0,
            skip_to: 0,
            format: None,
            playing: false,
            owed: 0,
            due_ns: 0,
            last_ns: 0,
            heard: Vec::new(),
            did: Vec::new(),
            packets: Vec::new(),
            wants: 0,
            most_held_ms: 0,
            overshoot: 0,
            say: Vec::new(),
            dry: 0,
        }));
        engine.set_card(Box::new(out), Some(Box::new(offload)), true);
        let time = Stepper::new(clock, dev.clone());
        Rig { engine, time, dev, events, _dir: dir }
    }

    pub fn run(&self, ms: u64) {
        self.time.run(Duration::from_millis(ms));
    }

    pub fn wait_for(&self, secs: u64, mut done: impl FnMut(&Fake) -> bool) -> bool {
        self.time.until(Duration::from_secs(secs), || done(&self.dev.lock()))
    }
}

/// Music-like samples unique per seed, stereo 16-bit.
pub fn music(rate: u32, secs: f64, seed: u64) -> Vec<i16> {
    let n = (secs * rate as f64) as usize;
    let hz = [110.0, 331.0, 1250.0].map(|h| h * (1.0 + seed as f64 * 0.01));
    let mut out = Vec::with_capacity(n * 2);
    let mut r = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let v: f64 = hz.iter().enumerate().map(|(k, h)| (std::f64::consts::TAU * h * t).sin() * 0.2 / (k + 1) as f64).sum();
        r = r.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let noise = ((r >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.02;
        out.push(((v + noise) * 32767.0) as i16);
        out.push(((v * 0.8 - noise) * 32767.0) as i16);
    }
    out
}

/// The left channel of stereo samples.
pub fn left(s: &[i16]) -> Vec<i32> {
    s.iter().step_by(2).map(|&v| v as i32).collect()
}

/// Where `part` (at least 64 frames) is found in `whole`.
pub fn find(whole: &[i32], part: &[i32]) -> Option<usize> {
    let k = part.len().min(256);
    whole.windows(k).position(|w| w == &part[..k])
}

#[test]
fn a_deep_sink_takes_the_music_in_bursts_it_asks_for() {
    let a = music(44_100, 150.0, 1);
    let rig = Rig::new(vec![wav("a", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(160, |d| d.heard.len() >= a.len() / 2), "the whole song was heard: {} of {}", rig.dev.lock().heard.len(), a.len() / 2);
    let d = rig.dev.lock();
    assert_eq!(d.heard[..a.len() / 2], left(&a)[..], "every frame, as it was");
    assert_eq!(d.overshoot, 0, "never more than the credit");
    assert!(d.most_held_ms <= 60_000 + 50, "within its depth: {} ms", d.most_held_ms);
    // A Want each time it fell to half its depth: about every 30 s.
    assert!((4..=7).contains(&d.wants), "{} asks for 150 s", d.wants);
    let formats: Vec<_> = d.did.iter().filter(|(_, x)| matches!(x, Did::Format(_))).collect();
    assert_eq!(formats.len(), 1, "one format: {formats:?}");
}

#[test]
fn a_seek_flushes_the_device_and_lands_where_asked() {
    let a = music(44_100, 120.0, 2);
    let rig = Rig::new(vec![wav("a", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(20, |d| d.heard.len() > 44_100 * 5));
    rig.engine.seek(70_000);
    let from = rig.dev.lock().heard.len();
    assert!(rig.wait_for(5, |d| d.heard.len() > from + 44_100), "plays on after the seek");
    let d = rig.dev.lock();
    assert!(d.did.iter().any(|(_, x)| *x == Did::Flush(None)), "everything held was dropped: {:?}", d.did);
    let song = left(&a);
    let at = (from..from + 44_100).find_map(|k| find(&song[70 * 44_100 - 2_000..70 * 44_100 + 44_100], &d.heard[k..k + 256]).map(|p| (k, p))).expect("the song after the seek");
    let landed = 70 * 44_100 - 2_000 + at.1;
    assert!(landed.abs_diff(70 * 44_100) < 441 * 2, "at 70 s: {landed}");
    assert!(at.0 - from < 44_100 / 2, "heard within half a second: {} frames", at.0 - from);
}

#[test]
fn a_sound_change_carries_on_from_what_the_device_held() {
    let a = music(44_100, 120.0, 3);
    let rig = Rig::new(vec![wav("a", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(30, |d| d.heard.len() > 44_100 * 20 && d.held_frames() > 44_100 * 25), "deep before the change");
    let asked = rig.dev.lock().heard.len();
    rig.engine.set_settings(Settings { sound: nori_engine::Sound { preamp_db: -12.0, ..Default::default() }, ..Settings::default() });
    assert!(rig.wait_for(40, |d| d.heard.len() > asked + 44_100 * 10));
    let d = rig.dev.lock();
    assert!(d.did.iter().any(|(_, x)| matches!(x, Did::Flush(Some(_)))), "the device kept what it held before the change: {:?}", d.did);
    let song = left(&a);
    // Every frame is the song's, at its place: nothing lost or played twice, louder then quieter.
    let quiet_from = (asked..d.heard.len()).find(|&k| (d.heard[k] - song[k]).abs() > 400).expect("the change is heard");
    assert!(quiet_from - asked < 44_100, "within a second of asking: {} ms", (quiet_from - asked) * 1000 / 44_100);
    for (k, (&got, &was)) in d.heard.iter().zip(&song).enumerate().skip(quiet_from + 2_000) {
        let want = was as f64 * 10f64.powf(-12.0 / 20.0);
        assert!((got as f64 - want).abs() < 300.0, "frame {k}: {got} for {want:.0}");
    }
}

#[test]
fn pause_stops_the_device_at_once_and_it_keeps_what_it_holds() {
    let a = music(44_100, 90.0, 4);
    let rig = Rig::new(vec![wav("a", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(20, |d| d.heard.len() > 44_100 * 10));
    rig.engine.pause();
    assert!(rig.wait_for(2, |d| d.did.iter().any(|(_, x)| *x == Did::Pause)));
    let (heard, held) = { let d = rig.dev.lock(); (d.heard.len(), d.held_frames()) };
    rig.run(10_000);
    assert!(rig.dev.lock().heard.len() - heard < 44_100 / 2, "stopped, but for the pause's fade");
    assert_eq!(rig.dev.lock().held_frames(), held, "and holds what it held");
    rig.engine.play();
    assert!(rig.wait_for(5, |d| d.heard.len() > heard + 44_100 * 3));
    let d = rig.dev.lock();
    assert_eq!(d.heard[..44_100 * 13], left(&a)[..44_100 * 13], "the song plays on where it stopped");
}

#[test]
fn each_song_reaches_the_device_at_its_own_rate_switching_at_the_boundary() {
    let (a, b) = (music(44_100, 20.0, 6), music(48_000, 20.0, 7));
    let rig = Rig::new(vec![wav("a", 44_100, &a), wav("b", 48_000, &b)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(60, |d| d.heard.len() >= (a.len() + b.len()) / 2), "both heard");
    let d = rig.dev.lock();
    let formats: Vec<(u64, u32)> = d.did.iter().filter_map(|(at, x)| match x {
        Did::Format(f) => Some((*at, f.rate)),
        _ => None,
    }).collect();
    let end = (a.len() / 2) as u64;
    assert_eq!(formats, [(0, 44_100), (end, 48_000)], "switched exactly where b starts");
    assert_eq!(d.heard[..end as usize], left(&a)[..], "a as it was");
    assert_eq!(d.heard[end as usize..end as usize + b.len() / 2], left(&b)[..], "b as it was: not resampled");
    assert_eq!(d.dry, 0, "gapless: the device never ran dry");
}

#[test]
fn a_song_longer_than_the_device_holds_goes_into_the_next_without_running_dry() {
    for (name, crossfade_s, auto_mix) in [("gapless", 0, false), ("crossfade", 5, false), ("automix", 0, true)] {
        let (a, b) = (music(44_100, 100.0, 21), music(44_100, 100.0, 22));
        let prefs = TransitionPrefs { crossfade_s, auto_mix, ..prefs_off() };
        let rig = Rig::new(vec![wav("a", 44_100, &a), wav("b", 44_100, &b)], prefs, Settings::default(), caps(), None);
        rig.engine.play_at(0, 0);
        assert!(rig.wait_for(130, |d| d.heard.len() > 44_100 * 115), "{name}: into the next song");
        let d = rig.dev.lock();
        assert_eq!(d.dry, 0, "{name}: never ran dry");
        assert!(!d.did.iter().any(|(_, x)| matches!(x, Did::Flush(_))), "{name}: nothing flushed: {:?}", d.did);
    }
}

#[test]
fn a_song_its_device_could_decode_after_pcm_comes_without_running_dry() {
    let (a, b) = (music(44_100, 100.0, 23), music(44_100, 100.0, 24));
    let rig = Rig::new(vec![wav("a", 44_100, &a), flac("b", 44_100, &b)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(130, |d| d.heard.len() > 44_100 * 115), "into the next song");
    let d = rig.dev.lock();
    assert_eq!(d.dry, 0, "never ran dry");
    assert!(!d.did.iter().any(|(_, x)| matches!(x, Did::Flush(_))), "nothing flushed: {:?}", d.did);
}

#[test]
fn a_mix_between_two_rates_switches_after_it() {
    let (a, b) = (music(44_100, 30.0, 8), music(48_000, 30.0, 9));
    let prefs = TransitionPrefs { crossfade_s: 5, ..prefs_off() };
    let rig = Rig::new(vec![wav("a", 44_100, &a), wav("b", 48_000, &b)], prefs, Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(80, |d| d.played > 44_100 * 50));
    let d = rig.dev.lock();
    let mix_from = 25 * 44_100;
    for (at, x) in &d.did {
        if let Did::Format(f) = x {
            assert!(*at == 0 || *at >= mix_from as u64 + 4 * 44_100, "no switch inside the mix: {f:?} at {at}");
        }
    }
    assert_eq!(d.dry, 0, "never ran dry");
}

#[test]
fn now_playing_comes_at_each_songs_first_frame_with_its_cover_once() {
    let (a1, a2, b1) = (music(44_100, 10.0, 10), music(44_100, 10.0, 11), music(44_100, 40.0, 12));
    let caps = Caps { cover: Some(240), ..caps() };
    let rig = Rig::new(vec![wav("a1", 44_100, &a1), wav("a2", 44_100, &a2), wav("b1", 44_100, &b1)], prefs_off(), Settings::default(), caps, Some(Arc::new(Covers)));
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(30, |d| d.heard.len() > 44_100 * 22));
    let d = rig.dev.lock();
    let nows: Vec<(u64, String, Option<u32>)> = d.did.iter().filter_map(|(at, x)| match x {
        Did::Now(t, c) => Some((*at, t.clone(), *c)),
        _ => None,
    }).collect();
    let covers = d.did.iter().filter(|(_, x)| matches!(x, Did::Cover(_))).count();
    let ten = 441_000;
    assert_eq!(nows.iter().map(|n| (n.0, n.1.as_str())).collect::<Vec<_>>(), [(0, "a1"), (ten, "a2"), (2 * ten, "b1")], "at each song's first frame");
    assert!(nows[0].2.is_some() && nows[0].2 == nows[1].2, "an album's songs share a cover: {nows:?}");
    assert!(nows[2].2.is_some() && nows[2].2 != nows[0].2, "another album's is its own: {nows:?}");
    assert_eq!(covers, 2, "each cover sent once");
    drop(d);

    // A device that shows no covers is sent none.
    let rig = Rig::new(vec![wav("a1", 44_100, &a1)], prefs_off(), Settings::default(), Caps { cover: None, ..caps }, Some(Arc::new(Covers)));
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(10, |d| d.heard.len() > 44_100));
    let d = rig.dev.lock();
    assert!(d.did.iter().any(|(_, x)| *x == Did::Now("a1".into(), None)), "{:?}", d.did);
    assert!(!d.did.iter().any(|(_, x)| matches!(x, Did::Cover(_))));
}

#[test]
fn a_now_after_a_skip_lands_where_the_new_song_starts() {
    let (a, b) = (music(44_100, 60.0, 13), music(44_100, 60.0, 14));
    let rig = Rig::new(vec![wav("a", 44_100, &a), wav("b", 44_100, &b)], prefs_off(), Settings::default(), caps(), Some(Arc::new(Covers)));
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(20, |d| d.heard.len() > 44_100 * 5));
    rig.engine.next();
    assert!(rig.wait_for(10, |d| d.did.iter().any(|(_, x)| *x == Did::Now("b".into(), None))), "{:?}", rig.dev.lock().did);
    rig.run(2_000);
    let d = rig.dev.lock();
    let at = d.did.iter().find(|(_, x)| *x == Did::Now("b".into(), None)).map(|(at, _)| *at as usize).unwrap();
    assert_eq!(find(&left(&b), &d.heard[at..at + 256]), Some(0), "b's first frame comes right after its Now");
}

#[test]
fn a_now_lands_in_a_mix() {
    let (a, b) = (music(44_100, 30.0, 15), music(44_100, 30.0, 16));
    let prefs = TransitionPrefs { crossfade_s: 5, ..prefs_off() };
    let rig = Rig::new(vec![wav("a", 44_100, &a), wav("b", 44_100, &b)], prefs, Settings::default(), caps(), Some(Arc::new(Covers)));
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(60, |d| d.did.iter().any(|(_, x)| *x == Did::Now("b".into(), None))));
    let d = rig.dev.lock();
    let at = d.did.iter().find(|(_, x)| *x == Did::Now("b".into(), None)).map(|(at, _)| *at).unwrap();
    assert!((25 * 44_100..=30 * 44_100).contains(&at), "within the mix: {at}");
}

/// A FLAC song of `samples` (stereo) at `rate`, with a seek point every 10 s.
pub fn flac(id: &'static str, rate: u32, samples: &[i16]) -> Song {
    Song { id, file: nori_engine::testing::flac(rate, samples, Some(10)), ms: (samples.len() / 2) as i64 * 1000 / rate as i64, hint: "flac" }
}

/// The codecs of the formats the device switched to, in order.
fn codecs(d: &Fake) -> Vec<Codec> {
    let mut all: Vec<Codec> = d.did.iter().filter_map(|(_, x)| match x {
        Did::Format(f) => Some(f.codec),
        _ => None,
    }).collect();
    all.dedup();
    all
}

#[test]
fn a_flac_song_nothing_touches_reaches_the_device_as_its_own_bytes() {
    let a = music(44_100, 40.0, 20);
    let song = flac("f", 44_100, &a);
    let file = song.file.clone();
    let rig = Rig::new(vec![song], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(60, |d| d.played > 44_100 * 39));
    let d = rig.dev.lock();
    assert_eq!(codecs(&d), [Codec::Flac], "offloaded throughout");
    assert!(d.packets.starts_with(b"fLaC"), "the stream info first");
    let frames = &d.packets[4 + 4 + 34..];
    let at = file.windows(64).position(|w| w == &frames[..64]).expect("the file's first frame");
    assert_eq!(frames, &file[at..at + frames.len()], "the file's own frames, in order");
    assert!(d.overshoot < 4096, "within a packet of the credit: {}", d.overshoot);
}

#[test]
fn a_codec_the_device_does_not_decode_goes_as_pcm() {
    let a = music(44_100, 20.0, 21);
    let caps = Caps { decodes: Codec::Mp3.bit(), ..caps() };
    let rig = Rig::new(vec![flac("f", 44_100, &a)], prefs_off(), Settings::default(), caps, None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(30, |d| d.heard.len() > 44_100 * 5));
    let d = rig.dev.lock();
    assert_eq!(codecs(&d), [Codec::Pcm]);
    assert_eq!(d.heard[..44_100 * 5], left(&a)[..44_100 * 5]);
}

#[test]
fn the_equalizer_takes_the_song_to_pcm_and_back() {
    let a = music(44_100, 90.0, 22);
    let rig = Rig::new(vec![flac("f", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(20, |d| d.played > 44_100 * 5));
    rig.engine.set_settings(Settings { sound: nori_engine::Sound { preamp_db: -6.0, ..Default::default() }, ..Settings::default() });
    assert!(rig.wait_for(10, |d| codecs(d).last() == Some(&Codec::Pcm)), "PCM once the samples are touched: {:?}", codecs(&rig.dev.lock()));
    let switched = rig.dev.lock().played;
    rig.engine.set_settings(Settings::default());
    assert!(rig.wait_for(20, |d| codecs(d).last() == Some(&Codec::Flac)), "and offloaded again: {:?}", codecs(&rig.dev.lock()));
    let d = rig.dev.lock();
    assert_eq!(codecs(&d), [Codec::Flac, Codec::Pcm, Codec::Flac]);
    assert!(switched < 44_100 * 8, "the equalizer heard within a few seconds: {} ms", switched * 1000 / 44_100);
}

#[test]
fn a_seek_while_offloaded_flushes_and_goes_on_from_its_frame() {
    let a = music(44_100, 60.0, 23);
    let rig = Rig::new(vec![flac("f", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(20, |d| d.played > 44_100 * 3), "{:?} {:?} {:?}", { let d = rig.dev.lock(); (d.depth_ms, d.received, d.wants, d.owed, d.playing, d.held.len()) }, rig.engine.status().pcm_why, rig.engine.status().offloaded);
    let before = rig.dev.lock().packets.len();
    rig.engine.seek(31_000);
    assert!(rig.wait_for(10, |d| d.packets.len() > before + 100_000));
    let d = rig.dev.lock();
    assert!(d.did.iter().any(|(_, x)| *x == Did::Flush(None)), "{:?}", d.did);
    // The first frame after the flush is the one holding 31 s (frames of 4096, numbered).
    let after = &d.packets[before..];
    let head = after.windows(4).position(|w| w == b"fLaC").expect("the stream info again");
    let frame = &after[head + 42..];
    assert_eq!((frame[0], frame[1]), (0xff, 0xf8), "a frame");
    let from = frame_number(&frame[4..]) * 4096;
    assert!((29 * 44_100..=31 * 44_100).contains(&from), "the frame the seek table puts a little before 31 s: {} ms", from * 1000 / 44_100);
}

/// A FLAC frame number, UTF-8 coded.
fn frame_number(b: &[u8]) -> u64 {
    if b[0] < 0x80 {
        return b[0] as u64;
    }
    let n = b[0].leading_ones() as usize;
    let mut v = (b[0] & (0x7f >> n)) as u64;
    for &c in &b[1..n] {
        v = v << 6 | (c & 0x3f) as u64;
    }
    v
}

#[test]
fn an_offloaded_songs_gain_comes_at_its_first_frame() {
    let (a, b) = (music(44_100, 20.0, 24), music(44_100, 20.0, 25));
    let mut app = sim::App::new();
    app.gains.insert("a".into(), 0.5);
    app.gains.insert("b".into(), 1.0);
    let rig = Rig::with_app(vec![flac("a", 44_100, &a), flac("b", 44_100, &b)], app, Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(60, |d| d.played > 44_100 * 25));
    let d = rig.dev.lock();
    let gains: Vec<(u64, i32)> = d.did.iter().filter_map(|(at, x)| match x {
        Did::Gain(db) => Some((*at, (db * 10.0).round() as i32)),
        _ => None,
    }).collect();
    assert_eq!(gains, [(0, -60), (44_100 * 20, 0)], "each song's level, exactly where it starts");
}

#[test]
fn a_device_gone_stops_the_music_as_a_dead_card_would() {
    let a = music(44_100, 60.0, 26);
    let rig = Rig::new(vec![wav("a", 44_100, &a)], prefs_off(), Settings::default(), caps(), None);
    rig.engine.play_at(0, 0);
    assert!(rig.wait_for(10, |d| d.heard.len() > 44_100));
    rig.dev.lock().deep.lost();
    assert!(rig.time.until(Duration::from_secs(5), || rig.events.lock().iter().any(|e| matches!(e, Event::Error { id, .. } if id.is_empty()))), "said: {:?}", rig.events.lock());
}
