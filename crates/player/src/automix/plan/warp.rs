//! The incoming song locked to the outgoing one beat by beat (docs/research/automix-novel.md, section 1): a live band
//! drifts, so one constant ratio leaves the beats apart within bars; a rate per beat follows each one. The beats are
//! each end's as heard ([`TrackAnalysis::outro_beats`], [`TrackAnalysis::intro_beats`]).

use super::*;
use crate::types::{Beat, Rate};

/// How far a paired beat may sound from its partner, ms.
const LOCK_SURE_MS: f64 = 20.0;
/// The most the incoming speed changes from one beat to the next: 0.4 % a beat, at the edge of what is heard as a
/// tempo change.
const SPEED_STEP: f64 = 0.004;
/// Beat intervals of the two songs this far from their usual ratio mean a beat missed or added: no pairing.
const PAIR_SLACK: f64 = 0.15;
/// The fewest paired beats worth following.
const MIN_PAIRS: usize = 8;

/// One beat of each song meant to sound together: the outgoing one's in transition time, the incoming one's in its
/// song time past `in_start_ms`, and whether both sit on attacks heard.
#[derive(Clone, Copy, Debug)]
struct Pair {
    at: f64,
    song: f64,
    sure: bool,
}

/// The two songs' beats over `p` paired outward from the window's landing, where the outgoing song's beat at
/// `out_ms` (its song time) meets the incoming one's at `in_ms`: one to one, or every other beat of the song whose
/// beats fall twice as often (half and double time). None without beats at both ends, or where the pairing does not
/// hold (a beat missed or added on one side).
fn pairs(p: &TransitionPlan, a: &[Beat], b: &[Beat], out_ms: f64, in_ms: f64) -> Option<Vec<Pair>> {
    let nearest = |beats: &[Beat], t: f64| beats.iter().enumerate().min_by(|x, y| (x.1.at_ms as f64 - t).abs().total_cmp(&(y.1.at_ms as f64 - t).abs())).map(|(i, _)| i);
    let (ia, ib) = (nearest(a, out_ms)?, nearest(b, in_ms)?);
    let gap = |beats: &[Beat], i: usize| beats.get(i + 1).map(|n| (n.at_ms - beats[i].at_ms) as f64);
    // Each anchor within a quarter of a beat of where the window put it.
    if (a[ia].at_ms as f64 - out_ms).abs() > 0.25 * gap(a, ia)? || (b[ib].at_ms as f64 - in_ms).abs() > 0.25 * gap(b, ib)? {
        return None;
    }
    // Beats per beat on each side: the incoming song's beat against the outgoing one's, at the speed it plays.
    let level = ((gap(b, ib)? / p.tempo_ratio) / gap(a, ia)?).log2().round() as i32;
    let (step_a, step_b) = match level {
        0 => (1, 1),
        -1 => (1, 2),
        1 => (2, 1),
        _ => return None,
    };
    let mut out = Vec::new();
    for k in -(a.len() as i64)..=(a.len() as i64) {
        let (i, j) = (ia as i64 + step_a * k, ib as i64 + step_b * k);
        let (Some(x), Some(y)) = (usize::try_from(i).ok().and_then(|i| a.get(i)), usize::try_from(j).ok().and_then(|j| b.get(j))) else { continue };
        let at = x.at_ms as f64 - p.out_start_ms as f64;
        let song = y.at_ms as f64 - p.in_start_ms as f64;
        if (0.0..=p.duration_ms as f64).contains(&at) && song >= 0.0 {
            out.push(Pair { at, song, sure: x.sure && y.sure });
        }
    }
    // One to one: each pair's two beat gaps in their usual ratio.
    let mut ratios: Vec<f64> = out.windows(2).map(|w| (w[1].song - w[0].song) / (w[1].at - w[0].at)).collect();
    if out.len() < MIN_PAIRS || ratios.iter().any(|r| !r.is_finite() || *r <= 0.0) {
        return None;
    }
    ratios.sort_by(f64::total_cmp);
    let usual = ratios[ratios.len() / 2];
    let held = out.windows(2).all(|w| (((w[1].song - w[0].song) / (w[1].at - w[0].at)) / usual - 1.0).abs() <= PAIR_SLACK);
    held.then_some(out)
}

/// Whether, under `p`'s incoming tempo, every pair of beats on attacks sounds within the lock: a beat no attack was
/// heard on (a quiet or beatless stretch) has nothing to clash.
fn locked(p: &TransitionPlan, pairs: &[Pair]) -> bool {
    pairs.iter().filter(|x| x.sure).all(|x| (p.wall_of_in_song(x.song) - x.at).abs() <= LOCK_SURE_MS)
}

/// A rate for each pair that lands its incoming beat on the outgoing one: from the transition's start, each beat's
/// speed is the one that closes the gap to the next pair's beat, changing at most [`SPEED_STEP`] a beat; after the
/// last, an ease back to the song's own tempo at [`moves::EASE_PCT_PER_BEAT`]. `first` is the speed the window was
/// laid out at; `beat_ms` the incoming song's own beat.
fn rates(pairs: &[Pair], first: f64, beat_ms: f64) -> Vec<Rate> {
    let mut knots = vec![Rate { at_ms: 0.0, speed: first }];
    let (mut t, mut played, mut speed) = (0.0, 0.0, first);
    // Before the first pair the song plays at the first speed, from its start.
    if let Some(x) = pairs.first() {
        (t, played) = (x.at - x.song / first, 0.0);
    }
    for x in pairs {
        let dt = x.at - t;
        if dt <= 0.0 {
            continue;
        }
        // The speed whose average over the gap reaches the beat: linear from the last one, so twice the mean less it,
        // halved toward the last to damp the swing.
        let mean = (x.song - played) / dt;
        let next = (speed + (2.0 * mean - speed - speed) * 0.5).clamp(speed * (1.0 - SPEED_STEP), speed * (1.0 + SPEED_STEP));
        played += dt * (speed + next) / 2.0;
        (t, speed) = (x.at, next);
        knots.push(Rate { at_ms: t, speed });
    }
    let beats_back = ((speed - 1.0).abs() * 100.0 / moves::EASE_PCT_PER_BEAT).ceil();
    if beats_back > 0.0 {
        knots.push(Rate { at_ms: t + beats_back * beat_ms / speed, speed: 1.0 });
    }
    knots
}

/// Locks `p`'s incoming song to the outgoing one beat by beat where both ends have their beats: the rates that land
/// each pair together, when they keep every pair within the lock. `swap` (transition ms) is where the window put the
/// two songs' meeting downbeats. Err when the beats are known and drift apart even so: the window does not lock.
pub(super) fn lock(p: &mut TransitionPlan, a: &TrackAnalysis, b: &TrackAnalysis, swap: f64) -> Result<bool, String> {
    // An outgoing loop repeats bars picked as steady; a bend or glide moves the outgoing beats themselves.
    if a.outro_beats.is_empty() || b.intro_beats.is_empty() || p.out_loop_ms.is_some() || p.bend.is_some() || p.out_glide.is_some() {
        return Ok(false);
    }
    let out_ms = p.out_start_ms as f64 + swap;
    let in_ms = p.in_start_ms as f64 + p.in_song_ms(swap);
    let Some(all) = pairs(p, &a.outro_beats, &b.intro_beats, out_ms, in_ms) else { return Ok(false) };
    // Only beats on attacks steer and are checked.
    let mut pairs: Vec<Pair> = all.into_iter().filter(|x| x.sure).collect();
    if pairs.len() < MIN_PAIRS {
        return Ok(false);
    }
    // An incoming loop's repeats keep the speed they were played at: what comes after them must hold at one ratio.
    if let Some(l) = p.in_loop {
        let after = p.in_delay_ms as f64 + (l.ms * (l.repeats as i64 + 1)) as f64;
        pairs.retain(|x| x.at >= after);
        return if pairs.len() < MIN_PAIRS || locked(p, &pairs) { Ok(false) } else { Err("the beats drift apart under the loop".into()) };
    }
    if locked(p, &pairs) {
        return Ok(false);
    }
    // The incoming song starts where its first paired beat lands on its partner: its start is the warp's other freedom.
    let mut warped = p.clone();
    let lead = pairs[0].song - p.tempo_ratio * (pairs[0].at - p.in_delay_ms as f64).max(0.0);
    let start = p.in_start_ms as f64 + lead;
    if start < 0.0 || start - b.silence_start_ms as f64 > MAX_SKIP_MS as f64 {
        return Err("the beats drift apart".into());
    }
    warped.in_start_ms = start.round() as i64;
    let shift = warped.in_start_ms as f64 - p.in_start_ms as f64;
    pairs.iter_mut().for_each(|x| x.song -= shift);
    warped.in_rates = rates(&pairs, p.tempo_ratio, 60_000.0 / b.bpm.max(1.0));
    (warped.tempo_ramp_beats, warped.tempo_ramp_ms) = (0, 0);
    if !locked(&warped, &pairs) {
        return Err("the beats drift apart".into());
    }
    *p = warped;
    Ok(true)
}
