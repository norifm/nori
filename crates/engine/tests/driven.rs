//! The engine run by its caller on one thread, as the web runs it: no thread of its own, its songs'
//! bytes pushed as the platform's requests answer, the caller turning it when its timer is due and
//! pulling the card.

use crate::common;

use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use common::card::Card;
use common::{Device, Virtual};
use nori_engine::{Body, ByteSource, Config, Engine, Filling, Library, Located, Next, OpenError, SharedQueue, Source};
use nori_player::sim;
use nori_player::transitions::WindowSong;
use parking_lot::Mutex;

const RATE: u32 = 44_100;
const SECS: f64 = 3.0;

/// Takes every loader's fetching, for the test to answer as its network.
#[derive(Default)]
struct Pushes(Mutex<Vec<Filling>>);

impl ByteSource for Pushes {
    fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
        unreachable!("nothing blocks where the engine is driven")
    }

    fn push(&self, fill: Filling) -> Option<Filling> {
        self.0.lock().push(fill);
        None
    }
}

struct Songs {
    bytes: Arc<Pushes>,
}

impl Library for Songs {
    fn locate(&mut self, id: &str) -> Result<Located, String> {
        Ok(Located { source: Source::Url { url: id.into(), bytes: self.bytes.clone() }, hint: Some("wav".into()), duration_ms: Some((SECS * 1000.0) as i64), estimated: false, from_second: None })
    }

    fn about(&self, id: &str) -> WindowSong {
        WindowSong { id: id.into(), title: id.into(), duration_ms: (SECS * 1000.0) as i64, ..Default::default() }
    }
}

/// Answers each request with `songs[url]`, `chunk` bytes per step; drops fetchings that stopped.
fn answer(fills: &mut Vec<Filling>, songs: &[(&str, &[u8])], chunk: usize) {
    fills.retain_mut(|f| {
        let song = songs.iter().find(|(id, _)| *id == f.url()).expect("a song of the test's").1;
        match f.poll_next(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Next::Open(from)) => {
                f.opened(from, Ok((from, Some(song.len() as u64))));
                true
            }
            Poll::Ready(Next::Read(from)) => {
                let end = (from as usize + chunk).min(song.len());
                f.took(from, Ok(&song[from as usize..end]));
                true
            }
            Poll::Ready(Next::Stop) => false,
            Poll::Ready(Next::Wait) | Poll::Pending => true,
        }
    });
}

#[test]
fn a_driven_engine_plays_songs_as_their_bytes_come() {
    let (a, b) = (common::sine(RATE, 440.0, SECS, 8000.0), common::sine(RATE, 660.0, SECS, 8000.0));
    let (wav_a, wav_b) = (common::wav(RATE, &a), common::wav(RATE, &b));
    let songs: [(&str, &[u8]); 2] = [("a", &wav_a), ("b", &wav_b)];
    let queue = SharedQueue::default();
    queue.0.lock().set(vec!["a".into(), "b".into()], Some(0), false, 0);
    let (clock, card, bytes) = (Virtual::default(), Card::new(), Arc::new(Pushes::default()));
    let (engine, mut driven) = Engine::driven(Songs { bytes: bytes.clone() }, sim::App::new(), queue, Box::new(card.clone()), Config::default(), clock.clone(), || {}, |_| {});
    engine.play_at(0, 0);
    let mut fills = Vec::new();
    let mut turns = 0;
    while card.secs() < 2.0 * SECS && clock.now_ns() < 20_000_000_000 {
        fills.append(&mut bytes.0.lock());
        // A 64 kB answer each turn: a few turns per second of music.
        answer(&mut fills, &songs, 64 * 1024);
        let wake = driven.turn().expect("the engine runs");
        turns += 1;
        let pull = card.pull.lock().due_ns();
        let at = wake.map_or(pull, |ms| (clock.now_ns() + ms * 1_000_000).min(pull));
        clock.move_to(at);
        if at >= pull {
            card.pull.lock().tick(at);
        }
    }
    let heard: Vec<f32> = card.heard.lock().clone();
    let want: Vec<f32> = a.iter().chain(&b).map(|&v| v as f32 / 32768.0).collect();
    assert_eq!(heard.len(), want.len(), "both songs whole, after {turns} turns");
    assert!(heard.iter().zip(&want).all(|(h, w)| (h - w).abs() < 1e-4), "the songs as they are, gaplessly");
    assert_eq!(card.pull.lock().dry, 0, "the card never ran dry");
}
