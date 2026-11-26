//! The silent output's pacer ([`nori_engine::WavOutput::silent`]) as the sound card, on the clock the
//! test moves: it takes a period of music each period of time, so songs play at real time, one after
//! another, and end, with nothing heard.

use crate::common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{Device, Stepper, Virtual};
use nori_engine::wav::Pacer;
use nori_engine::{AudioOutput, Config, Engine, Event, Feed, Library, Located, OutputFormat, SharedQueue, Source, State};
use nori_player::sim;
use nori_player::transitions::WindowSong;
use parking_lot::Mutex;

const RATE: u32 = 44_100;
const SECS: u32 = 6;

/// The pacer, driven by the test's clock as the output's thread drives it by real time.
struct Paced {
    clock: Virtual,
    pacer: Option<Pacer>,
    playing: bool,
    /// When a stopped device looks again.
    idle_ns: i64,
}

impl Device for Paced {
    fn due_ns(&self) -> i64 {
        match &self.pacer {
            Some(p) if self.playing => p.due_ns(),
            _ => self.idle_ns,
        }
    }

    fn tick(&mut self, now_ns: i64) -> bool {
        self.idle_ns = now_ns + 10_000_000;
        let Some(pacer) = self.pacer.as_mut().filter(|_| self.playing) else { return false };
        let pulled = pacer.pull().expect("nothing to write");
        if pulled.finished {
            self.playing = false;
        }
        pulled.woke_engine
    }
}

#[derive(Clone)]
struct Silent(Arc<Mutex<Paced>>);

impl AudioOutput for Silent {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        Ok(want)
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        self.0.lock().pacer = Some(Pacer::new(feed, None, false, 1.0).map_err(|e| e.to_string())?);
        Ok(())
    }

    fn pause(&mut self) {
        self.0.lock().playing = false;
    }

    fn resume(&mut self) {
        let mut p = self.0.lock();
        let now = p.clock.now_ns();
        if let Some(pacer) = p.pacer.as_mut() {
            pacer.resume(now);
        }
        p.playing = true;
    }

    fn latency_us(&self) -> u64 {
        0
    }

    fn close(&mut self) {
        self.0.lock().pacer = None;
    }
}

struct Files(Vec<(String, PathBuf)>);

impl Library for Files {
    fn locate(&mut self, id: &str) -> Result<Located, String> {
        let path = self.0.iter().find(|(i, _)| i == id).map(|(_, p)| p.clone()).ok_or("no such song")?;
        Ok(Located { source: Source::File(vec![path]), hint: Some("wav".into()), duration_ms: Some(SECS as i64 * 1000), estimated: false, from_second: None })
    }

    fn about(&self, id: &str) -> WindowSong {
        WindowSong { id: id.into(), title: id.into(), duration_ms: SECS as i64 * 1000, ..Default::default() }
    }
}

#[test]
fn the_silent_output_plays_songs_at_real_time_to_their_end() {
    let dir = nori_testdir::TempDir::new("paced");
    let songs: Vec<(String, PathBuf)> = ["a", "b"]
        .iter()
        .map(|id| {
            let path = dir.join(format!("{id}.wav"));
            std::fs::write(&path, common::wav(RATE, &common::sine(RATE, 440.0, SECS as f64, 12_000.0))).unwrap();
            (id.to_string(), path)
        })
        .collect();
    let clock = Virtual::default();
    let paced = Arc::new(Mutex::new(Paced { clock: clock.clone(), pacer: None, playing: false, idle_ns: 0 }));
    let queue = SharedQueue::default();
    queue.0.lock().set(vec!["a".into(), "b".into()], Some(0), false, 0);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let mut app = sim::App::new();
    app.prefs = sim::prefs_off();
    let engine = Engine::start_on(Files(songs), app, queue, Box::new(Silent(paced.clone())), None, Config::default(), clock.clone(), move |e| seen.lock().push(e));
    engine.queue_changed();
    engine.position_updates(Some(Duration::from_millis(250)));
    let time = Stepper::new(clock.clone(), paced.clone());
    engine.play_at(0, 0);
    let played = || paced.lock().pacer.as_ref().map_or(0, Pacer::frames);
    assert!(time.until(Duration::from_secs(2), || played() > 0), "the music starts");
    let (from_ns, from_frames) = (clock.now_ns(), played());
    time.run(Duration::from_secs(3));
    let secs = (played() - from_frames) as f64 / RATE as f64;
    let elapsed = (clock.now_ns() - from_ns) as f64 / 1e9;
    assert!((secs - elapsed).abs() < 0.05, "{secs:.3} s played in {elapsed:.3} s");
    let ms = || events.lock().iter().rev().find_map(|e| if let Event::Position { index: 0, ms, .. } = e { Some(*ms) } else { None });
    let at = ms().expect("the place is said");
    assert!((2_800..=3_300).contains(&at), "the place follows the clock: {at} ms");
    let next = || events.lock().iter().any(|e| matches!(e, Event::Song { index: 1, .. }));
    assert!(time.until(Duration::from_secs(SECS as u64), next), "the first song ends and the next plays");
    let stopped = || events.lock().iter().any(|e| matches!(e, Event::State(State::Ended)));
    assert!(time.until(Duration::from_secs(SECS as u64 + 1), stopped), "and the queue ends with the last song");
    let started_ns = from_ns - (from_frames as f64 / RATE as f64 * 1e9) as i64;
    let total = (clock.now_ns() - started_ns) as f64 / 1e9;
    assert!((total - 2.0 * SECS as f64).abs() < 0.5, "two songs of {SECS} s took {total:.2} s");
}
