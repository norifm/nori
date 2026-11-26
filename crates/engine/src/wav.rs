//! An output clocked like a sound card, `pace` times faster than real time, so the engine behaves as on
//! one and a queue renders in seconds. It writes what a device would have played (underruns as silence)
//! from the first music to the end to a WAV file, as 16-bit or float samples, or, silent, drops it: a
//! player that must make no sound (a device test's peer) plays at real time and opens no sound card.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use nori_time::{Duration, Instant};

use crate::output::{AudioOutput, Feed, OutputFormat};

/// Frames per pull (a sound card's period).
const PERIOD: usize = 1024;

struct Shared {
    playing: AtomicBool,
    closed: AtomicBool,
    frames: AtomicU64,
}

pub struct WavOutput {
    /// None: silent.
    path: Option<PathBuf>,
    pace: f64,
    float: bool,
    format: Option<OutputFormat>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl WavOutput {
    /// Writes to `path`, playing `pace` times faster than real time (1 for real time).
    pub fn new(path: impl Into<PathBuf>, pace: f64) -> WavOutput {
        WavOutput::to(Some(path.into()), pace)
    }

    /// Plays at real time into nothing.
    pub fn silent() -> WavOutput {
        WavOutput::to(None, 1.0)
    }

    fn to(path: Option<PathBuf>, pace: f64) -> WavOutput {
        let shared = Arc::new(Shared { playing: AtomicBool::new(false), closed: AtomicBool::new(false), frames: AtomicU64::new(0) });
        WavOutput { path, pace: pace.max(0.01), float: false, format: None, shared, thread: None }
    }

    /// Writes float samples, as a float device would take.
    pub fn in_float(mut self) -> WavOutput {
        self.float = true;
        self
    }

    /// Frames played so far.
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }
}

fn header(f: OutputFormat, frames: u64, float: bool) -> Vec<u8> {
    let width = if float { 4 } else { 2 };
    let data = frames * f.channels as u64 * width;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    // 1 is integer PCM, 3 IEEE float.
    h.extend_from_slice(&(if float { 3u16 } else { 1u16 }).to_le_bytes());
    h.extend_from_slice(&(f.channels as u16).to_le_bytes());
    h.extend_from_slice(&f.rate.to_le_bytes());
    h.extend_from_slice(&(f.rate * f.channels as u32 * width as u32).to_le_bytes());
    h.extend_from_slice(&(f.channels as u16 * width as u16).to_le_bytes());
    h.extend_from_slice(&(width as u16 * 8).to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data as u32).to_le_bytes());
    h
}

/// What a [`Pacer::pull`] did.
pub struct Pulled {
    /// The pull woke the engine (it crossed the ring's low mark).
    pub woke_engine: bool,
    /// The music is over and all pulled.
    pub finished: bool,
}

/// The device side of a [`WavOutput`]: a period of music every period of its clock, written to a file or
/// dropped.
pub struct Pacer {
    feed: Feed,
    file: Option<BufWriter<File>>,
    float: bool,
    period_ns: i64,
    due_ns: i64,
    /// Silence before the first music is skipped.
    started: bool,
    frames: u64,
    block: Vec<i16>,
    floats: Vec<f32>,
    bytes: Vec<u8>,
}

impl Pacer {
    /// Pulls from `feed` into `file` (None drops the music), `pace` times faster than real time.
    pub fn new(feed: Feed, file: Option<File>, float: bool, pace: f64) -> io::Result<Pacer> {
        let f = feed.format();
        let mut file = file.map(BufWriter::new);
        if let Some(w) = &mut file {
            w.write_all(&header(f, 0, float))?;
        }
        let period_ns = (PERIOD as f64 * 1e9 / f.rate as f64 / pace).round() as i64;
        let (block, floats) = if float { (Vec::new(), vec![0f32; PERIOD * f.channels]) } else { (vec![0i16; PERIOD * f.channels], Vec::new()) };
        Ok(Pacer { feed, file, float, period_ns, due_ns: 0, started: false, frames: 0, block, floats, bytes: Vec::new() })
    }

    /// When the next period is due, ns on the pacer's clock.
    pub fn due_ns(&self) -> i64 {
        self.due_ns
    }

    /// Plays on from `now_ns`, after a pause.
    pub fn resume(&mut self, now_ns: i64) {
        self.due_ns = now_ns;
    }

    /// Takes the period due at [`Pacer::due_ns`].
    pub fn pull(&mut self) -> io::Result<Pulled> {
        self.due_ns += self.period_ns;
        let waits = self.feed.engine_waits();
        if !self.started && self.feed.available() < PERIOD {
            return Ok(Pulled { woke_engine: false, finished: false });
        }
        self.started = true;
        let channels = self.feed.format().channels;
        let got = if self.float { self.feed.pull(&mut self.floats) } else { self.feed.pull_i16(&mut self.block) };
        let finished = self.feed.finished();
        let n = if finished && got < PERIOD { got } else { PERIOD };
        if let Some(w) = &mut self.file {
            self.bytes.clear();
            if self.float {
                self.bytes.extend(self.floats[..n * channels].iter().flat_map(|v| v.to_le_bytes()));
            } else {
                self.bytes.extend(self.block[..n * channels].iter().flat_map(|v| v.to_le_bytes()));
            }
            w.write_all(&self.bytes)?;
        }
        self.frames += n as u64;
        Ok(Pulled { woke_engine: waits && !self.feed.engine_waits(), finished })
    }

    /// Frames played so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Writes the file's header with the frames played.
    fn finish(self) -> io::Result<()> {
        let Some(mut w) = self.file else { return Ok(()) };
        w.seek(SeekFrom::Start(0))?;
        w.write_all(&header(self.feed.format(), self.frames, self.float))?;
        w.flush()
    }
}

/// The device's thread: a period each period of real time while playing.
fn play(path: Option<PathBuf>, pace: f64, float: bool, feed: Feed, shared: Arc<Shared>) -> io::Result<()> {
    let file = path.map(File::create).transpose()?;
    let mut pacer = Pacer::new(feed, file, float, pace)?;
    let start = Instant::now();
    let now_ns = || start.elapsed().as_nanos() as i64;
    while !shared.closed.load(Ordering::Acquire) {
        if !shared.playing.load(Ordering::Acquire) {
            std::thread::park();
            pacer.resume(now_ns());
            continue;
        }
        let wait = pacer.due_ns() - now_ns();
        if wait > 0 {
            std::thread::sleep(Duration::from_nanos(wait as u64));
        }
        if pacer.pull()?.finished {
            shared.playing.store(false, Ordering::Release);
        }
        shared.frames.store(pacer.frames(), Ordering::Relaxed);
    }
    pacer.finish()
}

impl AudioOutput for WavOutput {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        self.format = Some(want);
        Ok(want)
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        let (path, pace, float, shared) = (self.path.clone(), self.pace, self.float, self.shared.clone());
        let t = std::thread::Builder::new().name("nori-wav".into()).spawn(move || play(path, pace, float, feed, shared).map_err(|e| e.to_string())).map_err(|e| e.to_string())?;
        self.thread = Some(t);
        Ok(())
    }

    fn pause(&mut self) {
        self.shared.playing.store(false, Ordering::Release);
    }

    fn resume(&mut self) {
        self.shared.playing.store(true, Ordering::Release);
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    fn latency_us(&self) -> u64 {
        0
    }

    fn takes_float(&mut self) -> bool {
        self.float
    }

    fn close(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            if let Ok(Err(e)) = t.join() {
                let _ = writeln!(std::io::stderr(), "nori: the WAV file could not be written: {e}");
            }
        }
    }
}
