//! The engine run by its caller on one thread, as the web runs it: no thread of its own, songs whole in
//! memory, the caller turning it when its timer is due and pulling the card.

use crate::common;

use std::sync::Arc;

use common::card::Card;
use common::{Device, Virtual};
use nori_engine::{Config, Engine, Library, Located, SharedQueue, Source};
use nori_player::sim;
use nori_player::transitions::WindowSong;

const RATE: u32 = 44_100;
const SECS: f64 = 3.0;

struct Memory(Arc<[u8]>);

impl Library for Memory {
    fn locate(&mut self, _id: &str) -> Result<Located, String> {
        Ok(Located { source: Source::Memory(self.0.clone()), hint: Some("wav".into()), duration_ms: Some((SECS * 1000.0) as i64), estimated: false })
    }

    fn about(&self, id: &str) -> WindowSong {
        WindowSong { id: id.into(), title: id.into(), duration_ms: (SECS * 1000.0) as i64, ..Default::default() }
    }
}

#[test]
fn a_driven_engine_plays_a_song_from_memory_whole() {
    let samples = common::sine(RATE, 440.0, SECS, 8000.0);
    let queue = SharedQueue::default();
    queue.0.lock().set(vec!["a".into()], Some(0), false, 0);
    let (clock, card) = (Virtual::default(), Card::new());
    let (engine, mut driven) = Engine::driven(Memory(common::wav(RATE, &samples).into()), sim::App::new(), queue, Box::new(card.clone()), Config::default(), clock.clone(), || {}, |_| {});
    engine.play_at(0, 0);
    let mut turns = 0;
    while card.secs() < SECS && clock.now_ns() < 10_000_000_000 {
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
    let want: Vec<f32> = samples.iter().map(|&v| v as f32 / 32768.0).collect();
    assert_eq!(heard.len(), want.len(), "the whole song, after {turns} turns");
    assert!(heard.iter().zip(&want).all(|(h, w)| (h - w).abs() < 1e-4), "the song as it is");
}
