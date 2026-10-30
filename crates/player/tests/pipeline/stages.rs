//! Speed, pitch and silence skipping through the output clock.

use std::sync::{Arc, Mutex};

use nori_player::pcm::Format;
use nori_player::sim::Player;
use nori_player::stage::{Inputs, Preparing, Slot, SoundStage, StageMaker, StageSpec, Tap};

use crate::common::*;

/// The player's position `ms` of the virtual clock apart, after `settle_ms` for the change to reach
/// the ear through what the output already holds.
fn pace(p: &mut Player, settle_ms: i64, ms: i64) -> i64 {
    p.run_for(settle_ms);
    let a = p.position_ms();
    p.run_for(ms);
    p.position_ms() - a
}

#[test]
fn speed_and_pitch() {
    let song = centred(440.0, 0.4, 60.0);
    let mut p = Player::new(vec![track("a", &song)]);
    p.play_from(0);
    p.run_for(2_000);
    let heard_before = p.sink.heard_frames as usize;
    p.set_speed(1.5, 1.0);
    let moved = pace(&mut p, 12_000, 6_000);
    assert!((8_900..=9_100).contains(&moved), "6 s of the clock moved the song {moved} ms");
    assert!(p.run_to_end(60_000));
    assert!(p.sink.gaps.is_empty(), "no hole when the speed changed: {:?}", p.sink.gaps);
    // Everything after the change played in two thirds of its time, give or take the stretcher's edges.
    let rest = p.sink.heard_frames as usize - heard_before;
    let want = (frames(60.0) - heard_before) as f64;
    assert!((rest as f64 - want).abs() < want / 3.0 + frames(0.5) as f64, "{rest} frames for {want}");
    let heard = left(&p.sink.heard_samples());
    let hz = pitch_hz(&heard[heard.len() - frames(6.0)..heard.len() - frames(1.0)], RATE as f64);
    assert!((hz - 440.0).abs() < 2.0, "the pitch stays: {hz:.1} Hz");

    // Pitch keeps pace.
    let song = centred(440.0, 0.4, 40.0);
    let mut p = Player::new(vec![track("a", &song)]);
    p.set_speed(1.0, 1.1);
    p.play_from(0);
    let moved = pace(&mut p, 2_000, 6_000);
    assert!((5_950..=6_050).contains(&moved), "6 s of the clock moved the song {moved} ms");
    assert!(p.run_to_end(60_000));
    assert!((p.sink.heard_frames as i64 - frames(40.0) as i64).abs() < frames(0.1) as i64, "as long as the song: {}", p.sink.heard_frames);
    let heard = left(&p.sink.heard_samples());
    let hz = pitch_hz(&heard[frames(5.0)..frames(30.0)], RATE as f64);
    assert!((hz - 484.0).abs() < 2.0, "a tenth higher: {hz:.1} Hz");
}

#[test]
fn silence_skipping() {
    // Three seconds of music, four of silence, three of music.
    let (a, b) = (music(3.0, 31), music(3.0, 32));
    let song: Vec<i16> = [a.clone(), vec![0; frames(4.0) * 2], b.clone()].concat();
    let mut p = Player::new(vec![track("a", &song)]);
    p.set_skip_silence(true);
    p.play_from(0);
    assert!(p.run_to_end(30_000));
    let heard = p.sink.heard_samples();
    let secs = heard.len() as f64 / 2.0 / RATE as f64;
    // The pause is cut to a fifth of itself: 0.8 s instead of 4.
    assert!((6.7..7.0).contains(&secs), "{secs:.2} s heard of 10");
    assert!(heard[..a.len()] == a[..], "the music before the pause is untouched");
    // After it, from its first note above the silence threshold: the quiet samples before that are the
    // end of the pause, and fade in with it.
    let loud = b.iter().position(|v| v.unsigned_abs() > 1024).unwrap() / 2 * 2;
    assert!(heard[heard.len() - (b.len() - loud)..] == b[loud..], "and so is the music after it");
    assert!(p.sink.gaps.is_empty());
    // What was skipped still counts as played: the player ends at the end of the song.
    assert!((p.position_ms() - 10_000).abs() <= 25, "the position is the song's: {}", p.position_ms());

    // Silence skipping keeps short rest.
    let song: Vec<i16> = [music(2.0, 33), vec![0; frames(0.05) * 2], music(2.0, 34)].concat();
    let mut p = Player::new(vec![track("a", &song)]);
    p.set_skip_silence(true);
    p.play_from(0);
    assert!(p.run_to_end(30_000));
    assert!(p.sink.heard_samples() == song, "50 ms of quiet is music");
}


/// A plugin's stage stand-in: a tremolo whose phase is all its state, and its input the depth.
struct Tremolo {
    rate: f64,
    depth: f64,
    phase: f64,
}

impl nori_player::stage::SoundStage for Tremolo {
    fn inputs(&mut self, inputs: &Inputs) {
        self.depth = inputs.rows[0];
    }
    fn process(&mut self, planar: &mut [f32]) {
        let frames = planar.len() / 2;
        for k in 0..frames {
            let g = (1.0 - self.depth * (0.5 + 0.5 * (self.phase * std::f64::consts::TAU).sin())) as f32;
            planar[k] *= g;
            planar[frames + k] *= g;
            self.phase = (self.phase + 3.0 / self.rate).fract();
        }
    }
    fn reset(&mut self) {
        self.phase = 0.0;
    }
    fn save(&mut self, into: &mut Vec<u8>) {
        into.clear();
        into.extend_from_slice(&self.phase.to_le_bytes());
        into.extend_from_slice(&self.depth.to_le_bytes());
    }
    fn restore(&mut self, from: &[u8]) {
        self.phase = f64::from_le_bytes(from[..8].try_into().unwrap());
        self.depth = f64::from_le_bytes(from[8..16].try_into().unwrap());
    }
}

struct MakesTremolo;

impl nori_player::stage::StageMaker for MakesTremolo {
    fn make(&self, rate: u32, _channels: usize, inputs: &Inputs) -> Option<Box<dyn nori_player::stage::SoundStage>> {
        Some(Box::new(Tremolo { rate: rate as f64, depth: inputs.rows[0], phase: 0.0 }))
    }
}

fn tremolo(maker: &std::sync::Arc<MakesTremolo>, depth: f64) -> StageSpec {
    StageSpec { maker: maker.clone(), slot: Slot::Master, rows: vec![depth] }
}

/// What a song sounds like through a tremolo stage, made again from what the output still holds at
/// `remade_ms` if given.
fn through_tremolo(song: &[i16], remade_ms: Option<i64>) -> Vec<i16> {
    let maker = std::sync::Arc::new(MakesTremolo);
    let mut p = Player::new(vec![track("a", song)]);
    p.set_stages(vec![tremolo(&maker, 0.8)]);
    p.play_from(0);
    if let Some(ms) = remade_ms {
        p.run_for(ms);
        p.remake();
    }
    assert!(p.run_to_end(60_000));
    p.sink.heard_samples()
}

#[test]
fn a_stage_made_again_mid_song_sounds_the_same() {
    let song = centred(440.0, 0.4, 20.0);
    let straight = through_tremolo(&song, None);
    let heard = left(&straight);
    let (quiet, loud) = (rms(&heard[frames(5.0)..frames(5.0) + 400]), rms(&heard));
    assert!(quiet != loud, "the stage changes the sound");
    assert_eq!(through_tremolo(&song, Some(7_300)), straight, "its state is kept where the output is made again");
}

#[test]
fn a_stage_joins_and_leaves_where_the_output_can_change() {
    let maker = std::sync::Arc::new(MakesTremolo);
    let song = centred(440.0, 0.4, 30.0);
    let mut p = Player::new(vec![track("a", &song)]);
    p.play_from(0);
    p.run_for(4_000);
    p.set_stages(vec![tremolo(&maker, 1.0)]);
    p.run_for(14_000);
    p.set_stages(Vec::new());
    assert!(p.run_to_end(60_000));
    assert!(p.sink.gaps.is_empty(), "{:?}", p.sink.gaps);
    let heard = left(&p.sink.heard_samples());
    // The level over a tenth of a second swings with the 3 Hz tremolo while the stage is in.
    let swing = |from: f64| rms(&heard[frames(from)..frames(from + 0.1)]);
    let swings = |from: f64| (0..5).map(|k| swing(from + k as f64 * 0.1)).fold((f64::MAX, 0f64), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let (lo, hi) = swings(1.0);
    assert!(hi - lo < 0.01, "before: steady ({lo:.3}..{hi:.3})");
    let (lo, hi) = swings(10.0);
    assert!(hi - lo > 0.1, "with the stage: it swings ({lo:.3}..{hi:.3})");
    let (lo, hi) = swings(26.0);
    assert!(hi - lo < 0.01, "after: steady again ({lo:.3}..{hi:.3})");
}


/// Plays the first byte of its plugin's prepared data, a hundredth of full scale per unit, in place
/// of the music; the music where there is none. Counts the frames it made since its last reset, its
/// state; counting, it adds them (round a thousand) to what it plays, so a buffer run twice is heard.
struct Marker {
    level: Option<f32>,
    counting: bool,
    frames: u64,
}

impl SoundStage for Marker {
    fn inputs(&mut self, inputs: &Inputs) {
        self.level = inputs.song.prepared_by("marker").map(|d| d[0] as f32 / 100.0);
    }
    fn process(&mut self, planar: &mut [f32]) {
        let n = planar.len() / 2;
        for k in 0..n {
            let count = if self.counting { ((self.frames + k as u64) % 1000) as f32 / 1e4 } else { 0.0 };
            if let Some(l) = self.level {
                planar[k] = l + count;
                planar[n + k] = l + count;
            }
        }
        self.frames += n as u64;
    }
    fn reset(&mut self) {
        self.frames = 0;
    }
    fn save(&mut self, into: &mut Vec<u8>) {
        into.clear();
        into.extend_from_slice(&self.frames.to_le_bytes());
    }
    fn restore(&mut self, from: &[u8]) {
        self.frames = u64::from_le_bytes(from[..8].try_into().unwrap());
    }
}

struct MakesMarker {
    counting: bool,
}

impl StageMaker for MakesMarker {
    fn make(&self, _rate: u32, _channels: usize, inputs: &Inputs) -> Option<Box<dyn SoundStage>> {
        let mut m = Marker { level: None, counting: self.counting, frames: 0 };
        m.inputs(inputs);
        Some(Box::new(m))
    }
}

/// Prepared data by song, set by the test, and the frames each tap heard (its song, frames, ended).
#[derive(Default)]
struct Prepared {
    data: Mutex<Vec<(String, u8)>>,
    heard: Arc<Mutex<Vec<(String, u64, bool)>>>,
}

struct Heard(Arc<Mutex<Vec<(String, u64, bool)>>>, usize);

impl Tap for Heard {
    fn samples(&mut self, interleaved: &[f32]) {
        self.0.lock().unwrap()[self.1].1 += interleaved.len() as u64 / 2;
    }
    fn end(&mut self) {
        self.0.lock().unwrap()[self.1].2 = true;
    }
}

impl Preparing for Prepared {
    fn tap(&self, id: &str, _format: Format, from: u64) -> Option<Box<dyn Tap>> {
        let mut h = self.heard.lock().unwrap();
        h.push((id.to_string(), from, false));
        Some(Box::new(Heard(self.heard.clone(), h.len() - 1)))
    }
    fn prepared(&self, id: &str) -> Vec<(String, Arc<[u8]>)> {
        self.data.lock().unwrap().iter().filter(|(s, _)| s == id).map(|(_, v)| ("marker".to_string(), Arc::from(&[*v][..]))).collect()
    }
    fn on_prepared(&self, _wake: Arc<dyn Fn() + Send + Sync>) {}
}

fn marker(slot: Slot, counting: bool) -> StageSpec {
    StageSpec { maker: Arc::new(MakesMarker { counting }), slot, rows: Vec::new() }
}

/// The left channel's level where it is held steady from `secs` on.
fn level_at(heard: &[f64], secs: f64) -> f64 {
    heard[frames(secs)]
}

#[test]
fn a_stage_reads_each_songs_facts_from_its_first_frame() {
    let (a, b) = (centred(440.0, 0.4, 6.0), centred(440.0, 0.4, 6.0));
    let mut p = Player::new(vec![track("a", &a), track("b", &b)]);
    let prepared = Arc::new(Prepared::default());
    *prepared.data.lock().unwrap() = vec![("a".into(), 10), ("b".into(), 20)];
    p.set_preparing(Some(prepared.clone()));
    p.set_stages(vec![marker(Slot::Master, false)]);
    p.play_from(0);
    assert!(p.run_to_end(30_000));
    let heard = left(&p.sink.heard_samples());
    let edge = frames(6.0);
    assert!((heard[edge - 1] - 0.1).abs() < 1e-3 && (heard[edge] - 0.2).abs() < 1e-3, "the second song's data from its first frame: {} {}", heard[edge - 1], heard[edge]);
    let taps = prepared.heard.lock().unwrap().clone();
    assert_eq!(taps, vec![("a".to_string(), frames(6.0) as u64, true), ("b".to_string(), frames(6.0) as u64, true)], "each song heard whole as it was read, from frame 0");
}

#[test]
fn data_that_comes_while_its_song_plays_is_spliced_in() {
    let song = centred(440.0, 0.4, 20.0);
    let mut p = Player::new(vec![track("a", &song)]);
    let prepared = Arc::new(Prepared::default());
    p.set_preparing(Some(prepared.clone()));
    p.set_stages(vec![marker(Slot::Master, false)]);
    p.play_from(0);
    p.run_for(5_000);
    let at = p.position_ms();
    *prepared.data.lock().unwrap() = vec![("a".into(), 30)];
    p.facts_changed();
    assert!(p.run_to_end(60_000));
    assert!(p.sink.gaps.is_empty(), "{:?}", p.sink.gaps);
    let heard = left(&p.sink.heard_samples());
    assert!((level_at(&heard, 2.0) - 0.3).abs() > 0.01, "before: the song as it is");
    let from = heard.iter().position(|v| (v - 0.3).abs() < 1e-3).expect("the data is heard") as f64 / RATE as f64;
    assert!(from * 1000.0 < (at + 200) as f64, "heard at once, {:.0} ms after it came at {at} ms", from * 1000.0 - at as f64);
    assert!((level_at(&heard, 19.0) - 0.3).abs() < 1e-3, "and on to the end");
}

#[test]
fn a_song_slot_stage_runs_once_on_each_song_as_it_is_read() {
    let (a, b) = (centred(440.0, 0.4, 4.0), centred(440.0, 0.4, 4.0));
    let mut p = Player::new(vec![track("a", &a), track("b", &b)]);
    let prepared = Arc::new(Prepared::default());
    *prepared.data.lock().unwrap() = vec![("a".into(), 10), ("b".into(), 20)];
    p.set_preparing(Some(prepared));
    p.set_song_stages(vec![marker(Slot::Song, true)]);
    p.play_from(0);
    assert!(p.run_to_end(30_000));
    let heard = left(&p.sink.heard_samples());
    let edge = frames(4.0);
    // Each song's own count, from 0 at its start: a frame made twice would skip ahead.
    let count = |k: usize| (k % 1000) as f64 / 1e4;
    for (k, want) in [(0, 0.1), (edge - 1, 0.1 + count(edge - 1)), (edge, 0.2), (edge + 1500, 0.2 + count(1500))] {
        assert!((heard[k] - want).abs() < 5e-5, "frame {k}: {} for {want}", heard[k]);
    }
}

#[test]
fn a_splice_runs_the_last_songs_tail_with_its_own_facts() {
    let (a, b) = (centred(440.0, 0.4, 6.0), centred(440.0, 0.4, 6.0));
    let mut p = Player::new(vec![track("a", &a), track("b", &b)]);
    let prepared = Arc::new(Prepared::default());
    *prepared.data.lock().unwrap() = vec![("a".into(), 10), ("b".into(), 20)];
    p.set_preparing(Some(prepared.clone()));
    p.set_stages(vec![marker(Slot::Master, false)]);
    p.play_from(0);
    p.run_for(5_500);
    assert!(prepared.heard.lock().unwrap().iter().any(|(id, ..)| id == "b"), "the next song is read while the first's tail is still to be heard");
    // The next song's data changes: what is spliced again of the first keeps the first's facts.
    *prepared.data.lock().unwrap() = vec![("a".into(), 10), ("b".into(), 30)];
    p.facts_changed();
    assert!(p.run_to_end(30_000));
    let heard = left(&p.sink.heard_samples());
    let edge = frames(6.0);
    for k in [frames(5.6), edge - 1] {
        assert!((heard[k] - 0.1).abs() < 1e-3, "frame {k} of the first song: {}", heard[k]);
    }
    assert!((heard[edge] - 0.3).abs() < 1e-3, "the next song's new data from its first frame: {}", heard[edge]);
}
