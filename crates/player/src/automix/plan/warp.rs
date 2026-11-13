//! The incoming song locked to the outgoing one beat by beat (docs/research/automix-novel.md, section 1): a live band
//! drifts, so one constant ratio leaves the beats apart within bars; a rate per beat follows each one. The beats are
//! each end's as heard ([`TrackAnalysis::outro_beats`], [`TrackAnalysis::intro_beats`]).

use super::*;
use super::super::curve::cleaned;
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
/// beats fall twice as often (half and double time). Ok(None) where they cannot be paired (no beat near the landing,
/// too few in the window); Err where they can but the pairs do not keep one ratio: the beats drift apart.
fn pairs(p: &TransitionPlan, a: &[Beat], b: &[Beat], out_ms: f64, in_ms: f64) -> Result<Option<Vec<Pair>>, String> {
    pair_up(p, a, b, out_ms, in_ms).map_or(Ok(None), |(out, held)| if held { Ok(Some(out)) } else { Err("the beats drift apart".into()) })
}

/// The pairs, and whether they keep one ratio; None where none can be made.
fn pair_up(p: &TransitionPlan, a: &[Beat], b: &[Beat], out_ms: f64, in_ms: f64) -> Option<(Vec<Pair>, bool)> {
    let nearest = |beats: &[Beat], t: f64| beats.iter().enumerate().min_by(|x, y| (x.1.at_ms as f64 - t).abs().total_cmp(&(y.1.at_ms as f64 - t).abs())).map(|(i, _)| i);
    let (ia, ib) = (nearest(a, out_ms)?, nearest(b, in_ms)?);
    // The beat's interval: to the next one, or from the one before at the end.
    let gap = |beats: &[Beat], i: usize| match beats.get(i + 1) {
        Some(n) => Some((n.at_ms - beats[i].at_ms) as f64),
        None => i.checked_sub(1).map(|j| (beats[i].at_ms - beats[j].at_ms) as f64),
    };
    // Each anchor within a quarter of a beat of where the window put it; inside the beats, one further off is a window
    // off the beats heard.
    let off = |beats: &[Beat], i: usize, t: f64| Some((beats[i].at_ms as f64 - t).abs() > 0.25 * gap(beats, i)?);
    let inside = |beats: &[Beat], t: f64| (beats[0].at_ms as f64..=beats[beats.len() - 1].at_ms as f64).contains(&t);
    if off(a, ia, out_ms)? || off(b, ib, in_ms)? {
        return (inside(a, out_ms) && inside(b, in_ms)).then(|| (Vec::new(), false));
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
    Some((out, held))
}

/// Whether, under `p`'s incoming tempo, every pair of beats on attacks sounds within the lock: a beat no attack was
/// heard on (a quiet or beatless stretch) has nothing to clash.
fn locked(p: &TransitionPlan, pairs: &[Pair]) -> bool {
    pairs.iter().filter(|x| x.sure).all(|x| (p.wall_of_in_song(x.song) - x.at).abs() <= LOCK_SURE_MS)
}

/// The outgoing beats on attacks heard over `p`'s incoming loop, each against the nearest looped incoming beat on an
/// attack: how many met within the lock, None when one did not.
fn under_loop(p: &TransitionPlan, a: &[Beat], b: &[Beat]) -> Option<usize> {
    let Some(l) = p.in_loop else { return Some(0) };
    let (delay, len) = (p.in_delay_ms.max(0) as f64, l.ms.max(1) as f64);
    let end = delay + len * (l.repeats.max(0) + 1) as f64;
    // The looped incoming beats, as heard from the loop's start.
    let looped: Vec<f64> = b
        .iter()
        .filter(|y| y.sure)
        .map(|y| (y.at_ms - p.in_start_ms) as f64 / p.tempo_ratio)
        .filter(|t| (0.0..len).contains(t))
        .collect();
    if looped.is_empty() {
        return Some(0);
    }
    let mut met = 0;
    for x in a.iter().filter(|x| x.sure) {
        let at = (x.at_ms - p.out_start_ms) as f64;
        if !(delay..end).contains(&at) {
            continue;
        }
        let u = (at - delay).rem_euclid(len);
        let off = looped.iter().flat_map(|t| [t - u, t - u - len, t - u + len]).map(f64::abs).fold(f64::MAX, f64::min);
        if off > LOCK_SURE_MS {
            return None;
        }
        met += 1;
    }
    Some(met)
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

/// How a window's beats were found to sound together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Locked {
    /// Not known: an end without its beats, too few paired, or a loop, bend or glide moving them.
    Unchecked,
    /// Within the lock at the plan's own tempo.
    Held,
    /// Within it once the incoming tempo follows them beat by beat.
    Warped,
}

/// Locks `p`'s incoming song to the outgoing one beat by beat where both ends have their beats: the rates that land
/// each pair together, when they keep every pair within the lock. `swap` (transition ms) is where the window put the
/// two songs' meeting downbeats. Err when the beats are known and drift apart even so: the window does not lock.
pub(super) fn lock(p: &mut TransitionPlan, a: &TrackAnalysis, b: &TrackAnalysis, swap: f64) -> Result<Locked, String> {
    // An outgoing loop repeats bars picked as steady; a bend or glide moves the outgoing beats themselves.
    if a.outro_beats.is_empty() || b.intro_beats.is_empty() || p.out_loop_ms.is_some() || p.bend.is_some() || p.out_glide.is_some() {
        return Ok(Locked::Unchecked);
    }
    // Cleaned, so a beat missed does not break the pairing; paired outward from the swap, or from the outgoing song's
    // last beat when the swap is past them.
    let (a_beats, b_beats) = (cleaned(&a.outro_beats), cleaned(&b.intro_beats));
    let last = a_beats[a_beats.len() - 1].at_ms as f64 - p.out_start_ms as f64;
    let anchor = swap.min(last).max(0.0);
    let out_ms = p.out_start_ms as f64 + anchor;
    let in_ms = p.in_start_ms as f64 + p.in_song_ms(anchor);
    let Some(all) = pairs(p, &a_beats, &b_beats, out_ms, in_ms)? else { return Ok(Locked::Unchecked) };
    // Only beats on attacks steer and are checked.
    let mut pairs: Vec<Pair> = all.into_iter().filter(|x| x.sure).collect();
    if pairs.len() < MIN_PAIRS {
        return Ok(Locked::Unchecked);
    }
    // An incoming loop's repeats keep the speed they were played at: the outgoing beats over them, and the pairs
    // after them, must hold at one ratio.
    if let Some(l) = p.in_loop {
        let after = p.in_delay_ms as f64 + (l.ms * (l.repeats as i64 + 1)) as f64;
        pairs.retain(|x| x.at >= after);
        let Some(under) = under_loop(p, &a_beats, &b_beats) else { return Err("the beats drift apart under the loop".into()) };
        return if under + pairs.len() < MIN_PAIRS {
            Ok(Locked::Unchecked)
        } else if locked(p, &pairs) {
            Ok(Locked::Held)
        } else {
            Err("the beats drift apart after the loop".into())
        };
    }
    if locked(p, &pairs) {
        return Ok(Locked::Held);
    }
    // The incoming song starts where its first paired beat lands on its partner: its start is the warp's other freedom.
    let mut warped = p.clone();
    let lead = pairs[0].song - p.tempo_ratio * (pairs[0].at - p.in_delay_ms as f64).max(0.0);
    let start = p.in_start_ms as f64 + lead;
    if start - b.silence_start_ms as f64 > MAX_SKIP_MS as f64 {
        return Err("the beats drift apart".into());
    }
    // Wanted before the song's start: it starts later instead, from its start.
    if start < 0.0 {
        warped.in_delay_ms = p.in_delay_ms + (-start / p.tempo_ratio).round() as i64;
    }
    warped.in_start_ms = start.max(0.0).round() as i64;
    let shift = warped.in_start_ms as f64 - p.in_start_ms as f64;
    pairs.iter_mut().for_each(|x| x.song -= shift);
    warped.in_rates = rates(&pairs, p.tempo_ratio, 60_000.0 / b.bpm.max(1.0));
    (warped.tempo_ramp_beats, warped.tempo_ramp_ms) = (0, 0);
    if !locked(&warped, &pairs) {
        return Err("the beats drift apart".into());
    }
    *p = warped;
    Ok(Locked::Warped)
}

/// How far a warped plan's incoming tempo strays from the speed it starts at, percent at most; 0 unwarped.
pub(super) fn strain_pct(p: &TransitionPlan) -> f64 {
    let first = p.tempo_ratio.max(1e-9);
    p.in_rates.iter().filter(|r| r.at_ms <= p.duration_ms as f64).map(|r| (r.speed / first - 1.0).abs() * 100.0).fold(0.0, f64::max)
}
