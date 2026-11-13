//! AutoMix: DJ-style transitions.
//!
//! - `analysis`, `tempo`, `structure`, `loudness`: one streamed pass over a track's PCM gives a `TrackAnalysis`.
//! - `beats` (+ `neural`, `weights`, `checkpoint` with `neural-beats`): the optional Beat This! tracker, whose
//!   grids replace the classical intro and outro grids where it is sure.
//! - `plan`: two analyses and the settings to a `TransitionPlan`; `curve`: an end's beats as a tempo curve for it;
//!   `phrase`: where an end's phrases start.
//! - `mixer`, `stretch`, `resample`: per-buffer rendering of a plan.
//! - `vocal`: the vocal activity curve synced lyrics are checked against.
//! - `synth`, `eval` (tests and `synth`): synthetic songs the analysis and plans are scored on.

pub mod analysis;
pub mod beats;
#[cfg(any(test, feature = "neural-beats"))]
pub mod checkpoint;
mod curve;
pub mod loudness;
pub mod mixer;
mod phrase;
#[cfg(feature = "neural-beats")]
pub mod neural;
pub mod plan;
pub mod resample;
pub mod stretch;
pub mod structure;
pub mod tempo;
pub mod vocal;
#[cfg(feature = "neural-beats")]
pub mod weights;

#[cfg(any(test, feature = "synth"))]
pub mod synth;
#[cfg(any(test, feature = "synth"))]
pub mod eval;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod plan_fuzz;

use crate::types::{BarProfile, Beat, TrackAnalysis};
#[cfg(any(test, feature = "synth"))]
use analysis::Analyzer;
use analysis::Features;

/// Bump when stored rows should be measured again. 23: the beat model reads a minute of each end.
pub const ANALYSIS_VERSION: i32 = 23;
/// Seconds of music at each end the intro and outro grids are measured over: dozens of beats, short enough that
/// a live band drifts a fraction of a beat.
pub(crate) const GRID_WINDOW_S: f64 = 40.0;
/// A silence inside the music at least this long is a gap (before a hidden track), not a rest.
const LONG_GAP_MS: i64 = 6_000;
/// Below these, cues are placed from the energy envelope instead of the grid.
const CUE_MIN_CONFIDENCE: f32 = 0.4;
const CUE_MIN_STABILITY: f32 = 0.5;

/// Mean of a per-frame `curve` over `[from_s, to_s)`; the whole track's when the window is under a second.
fn window_mean(curve: &[f32], fps: f64, t0: f64, from_s: f64, to_s: f64) -> f32 {
    if curve.is_empty() || !fps.is_finite() || fps <= 0.0 {
        return 0.0;
    }
    let idx = |t: f64| ((t - t0) * fps).round().max(0.0) as usize;
    let (mut a, mut b) = (idx(from_s).min(curve.len()), idx(to_s).min(curve.len()));
    if b.saturating_sub(a) < fps as usize {
        (a, b) = (0, curve.len());
    }
    if b <= a {
        return 0.0;
    }
    curve[a..b].iter().sum::<f32>() / (b - a) as f32
}

/// A bar is still sung when its voice level is this share of the song's median bar. Set on real songs against a
/// separated vocal: loud bands keep the voice curve high between lines, so a stricter test ends the singing early,
/// which costs the last chorus; this one is rather late, over an instrumental outro.
const SUNG_SHARE: f32 = 0.7;
/// The voice curve level a sung song's loudest bar reaches: drums and chords stay under it (2 to 3.7 on the
/// synthetic songs), and every sung song of 89 real ones is over it (4.8 and up).
const SUNG_FLOOR: f32 = 4.0;

/// Where the singing of `music` starts and ends, on bars of `bar_s` counted from its start and back from its end:
/// the first of two bars in a row, and the end of the last bar, whose level on the voice curve ([`vocal`]) is
/// [`SUNG_SHARE`] of the song's median bar. None when no bar reaches [`SUNG_FLOOR`].
fn vocal_span(f: &Features, bar_s: f64, music: (f64, f64)) -> Option<(f64, f64)> {
    let level = |from: f64| voice_level(f, from, from + bar_s);
    let n = ((music.1 - music.0) / bar_s).floor() as usize;
    let back: Vec<f32> = (1..=n).map(|i| level(music.1 - i as f64 * bar_s)).collect();
    let sung = sung_level(&back)?;
    let last = back.iter().position(|&b| b >= sung)?;
    let end = music.1 - last as f64 * bar_s;
    let sung_at = |i: usize| level(music.0 + i as f64 * bar_s) >= sung;
    let first = (0..n.saturating_sub(1)).find(|&i| sung_at(i) && sung_at(i + 1)).map_or(end - bar_s, |i| music.0 + i as f64 * bar_s);
    Some((first.min(end - bar_s).max(music.0), end))
}

/// The mean of the voice curve over `[from_s, to_s)`.
fn voice_level(f: &Features, from_s: f64, to_s: f64) -> f32 {
    let e = vocal::CURVE_EVERY as f64;
    window_mean(&f.voice, f.fps / e, f.t0 + (e - 1.0) / 2.0 / f.fps, from_s, to_s)
}

/// The voice level a bar is sung at, from every bar's level: [`SUNG_SHARE`] of the median bar. None when no bar
/// reaches [`SUNG_FLOOR`].
fn sung_level(bars: &[f32]) -> Option<f32> {
    let mut sorted = bars.to_vec();
    sorted.sort_by(f32::total_cmp);
    if *sorted.last()? < SUNG_FLOOR {
        return None;
    }
    Some(SUNG_SHARE * *sorted.get(sorted.len() / 2)?)
}

/// Bars profiled at each end of a song.
const END_BARS: usize = 32;
/// Bars each side a bar's change is read over.
const CHANGE_SPAN: usize = 4;
/// A level or lows change this many dB counts as much as a full change of beat or singing.
const CHANGE_DB: f32 = 12.0;
/// Lows quieter than this are as good as gone.
const CHANGE_LOWS_DB: f32 = 24.0;
/// The summed change at which a bar reads 1, about the band giving way to pads: below it every change is a peak on
/// its own bar, not a plateau.
const CHANGE_FULL: f32 = 4.5;
/// Without a grid an end is profiled in blocks this long, seconds.
const BLOCK_S: f64 = 2.0;
/// A beat pulses when its onset is this share of the song's strong onsets (its 90th percentile).
const PULSE_HIT: f32 = 0.5;

/// The bars at one end of the music: the first [`END_BARS`] from its start, or the last back from its end, on
/// `grid` (`meter` beats to the bar), else in [`BLOCK_S`] blocks. A bar's pulse is the share of its beats (on
/// `beat_s`) that hit at [`PULSE_HIT`] of `strong`; its voice the share of its voice curve at `sung` or over.
#[allow(clippy::too_many_arguments)]
fn end_bars(f: &Features, grid: &Grid, meter: i64, beat_s: f64, music: (f64, f64), intro: bool, sung: Option<f32>, strong: f32) -> Vec<BarProfile> {
    let gridded = grid.bpm > 0.0 && grid.bpm.is_finite();
    let bar_s = if gridded { 60.0 / grid.bpm * meter as f64 } else { BLOCK_S };
    // The first bar line at or after the music's start, or the last whole bar's before its end.
    let first = if gridded {
        let beat = 60.0 / grid.bpm;
        let off = grid.offset_ms / 1000.0;
        let edge = if intro { music.0 } else { music.1 - bar_s + 0.25 * beat };
        let mut n = ((edge - off) / beat).ceil() as i64;
        if intro {
            n += (grid.downbeat_phase as i64 - n).rem_euclid(meter);
        } else {
            n -= (n - grid.downbeat_phase as i64).rem_euclid(meter);
        }
        off + n as f64 * beat
    } else if intro {
        music.0
    } else {
        music.1 - BLOCK_S
    };
    // A few bars further in than kept, for the change at the innermost ones.
    let starts = (0..END_BARS + CHANGE_SPAN).map(|i| if intro { first + i as f64 * bar_s } else { first - i as f64 * bar_s });
    let song = (window_mean(&f.power, f.fps, f.t0, music.0, music.1) as f64).max(1e-12);
    let song_low = (window_mean(&f.low_power, f.fps, f.t0, music.0, music.1) as f64).max(1e-12);
    let db = |v: f32, of: f64| (10.0 * ((v as f64).max(1e-12) / of).log10()) as f32;
    let frame = |t: f64| ((t - f.t0) * f.fps).round();
    let mut bars: Vec<ProfiledBar> = starts
        .take_while(|&s| s >= music.0 - 0.5 * bar_s && s + 0.5 * bar_s <= music.1)
        .map(|s| {
            let e = s + bar_s;
            // The strongest onset within two frames of each beat.
            let pulse = if beat_s > 0.0 && strong > 0.0 {
                let k0 = ((s - grid.offset_ms / 1000.0) / beat_s).ceil() as i64;
                let beats: Vec<f64> = (k0..).map(|k| grid.offset_ms / 1000.0 + k as f64 * beat_s).take_while(|&b| b < e).collect();
                let peak = |b: f64| {
                    let j = frame(b) as i64;
                    (j - 2..=j + 2).filter_map(|i| usize::try_from(i).ok().and_then(|i| f.onset.get(i))).fold(0f32, |m, v| m.max(*v))
                };
                beats.iter().filter(|&&b| peak(b) >= PULSE_HIT * strong).count() as f32 / beats.len().max(1) as f32
            } else {
                0.0
            };
            let voice_frames = sung.map_or(0.0, |level| {
                let e_every = vocal::CURVE_EVERY as f64;
                let (fps, t0) = (f.fps / e_every, f.t0 + (e_every - 1.0) / 2.0 / f.fps);
                let (a, b) = (((s - t0) * fps).round().max(0.0) as usize, ((e - t0) * fps).round().max(0.0) as usize);
                let part = &f.voice[a.min(f.voice.len())..b.min(f.voice.len())];
                if part.is_empty() { 0.0 } else { part.iter().filter(|v| **v >= level).count() as f32 / part.len() as f32 }
            });
            let bar = BarProfile {
                start_ms: (s * 1000.0).round() as i64,
                level_db: db(window_mean(&f.power, f.fps, f.t0, s, e), song),
                low_db: db(window_mean(&f.low_power, f.fps, f.t0, s, e), song_low),
                pulse,
                voice: voice_frames,
                change: 0.0,
            };
            (bar, chroma_of(f, s, e))
        })
        .collect();
    if !intro {
        bars.reverse();
    }
    // How much the music changes as each bar starts: the bars after it against the bars before.
    let change: Vec<f32> = (0..bars.len())
        .map(|i| {
            let (before, after) = (&bars[i.saturating_sub(CHANGE_SPAN)..i], &bars[i..(i + CHANGE_SPAN).min(bars.len())]);
            if before.len() < 2 || after.len() < 2 {
                return 0.0;
            }
            let mean = |side: &[ProfiledBar], f: &dyn Fn(&ProfiledBar) -> f32| side.iter().map(f).sum::<f32>() / side.len() as f32;
            let chords: f32 = (0..12).map(|k| (mean(before, &|b| b.1[k]) - mean(after, &|b| b.1[k])).abs()).sum();
            let level = (mean(before, &|b| b.0.level_db) - mean(after, &|b| b.0.level_db)).abs() / CHANGE_DB;
            let lows = (mean(before, &|b| b.0.low_db.max(-CHANGE_LOWS_DB)) - mean(after, &|b| b.0.low_db.max(-CHANGE_LOWS_DB))).abs() / CHANGE_DB;
            let beat = (mean(before, &|b| b.0.pulse) - mean(after, &|b| b.0.pulse)).abs();
            let voice = (mean(before, &|b| b.0.voice) - mean(after, &|b| b.0.voice)).abs();
            ((chords + level + lows + beat + voice) / CHANGE_FULL).min(1.0)
        })
        .collect();
    let keep = bars.len().min(END_BARS);
    let from = if intro { 0 } else { bars.len() - keep };
    bars[from..from + keep].iter().zip(&change[from..from + keep]).map(|((b, _), c)| BarProfile { change: *c, ..*b }).collect()
}

/// The music at each end whose beats are kept, seconds.
const END_BEATS_S: f64 = 60.0;
/// A beat moves onto an attack at most this far from where a tracker put it, seconds, weighed toward it over the
/// spread: a network's beats are right about which beat, not exactly when.
const REFINE_REACH_S: f64 = 0.04;
const REFINE_SPREAD_S: f64 = 0.02;
/// An attack a beat is sure of stands this many deviations (MAD) over the onsets of the second around it.
const REFINE_SURE_MADS: f32 = 3.0;

/// `beats` (seconds) each moved onto the attack it stands for in the onset curve (`onset` at `fps`, frame 0 at
/// `t0`): the strongest peak within [`REFINE_REACH_S`], weighed toward the beat, placed between frames by a parabola;
/// sure when it stands out of the onsets around it and hits at least [`PULSE_HIT`] as hard as the beats' usual
/// attack: a pad's slow swell peaks well after its note starts. The others move by the sure ones' median shift.
pub(crate) fn refine(beats: &[f64], onset: &[f32], fps: f64, t0: f64) -> Vec<(f64, bool)> {
    let n = onset.len();
    if n < 3 || fps.is_nan() || fps <= 0.0 {
        return beats.iter().map(|&b| (b, false)).collect();
    }
    let reach = (REFINE_REACH_S * fps).ceil() as usize;
    let around = fps.round() as usize;
    let found: Vec<Option<(f64, f32)>> = beats
        .iter()
        .map(|&b| {
            let c = (b - t0) * fps;
            if !(c >= 0.0 && c < n as f64) {
                return None;
            }
            let j0 = c.round() as usize;
            let (lo, hi) = (j0.saturating_sub(reach).max(1), (j0 + reach).min(n - 2));
            let mut best: Option<(f64, f64, usize)> = None;
            for j in lo..=hi {
                let (y0, y1, y2) = (onset[j - 1] as f64, onset[j] as f64, onset[j + 1] as f64);
                if !(y1 > y0 && y1 >= y2) {
                    continue;
                }
                let curve = y0 - 2.0 * y1 + y2;
                let pos = j as f64 + if curve < 0.0 { (0.5 * (y0 - y2) / curve).clamp(-0.5, 0.5) } else { 0.0 };
                let dt = (pos - c) / fps;
                let w = y1 * (-0.5 * (dt / REFINE_SPREAD_S).powi(2)).exp();
                if best.is_none_or(|b| w > b.1) {
                    best = Some((pos, w, j));
                }
            }
            let (pos, _, j) = best?;
            let mut local: Vec<f32> = onset[j.saturating_sub(around)..(j + around).min(n)].to_vec();
            local.sort_by(f32::total_cmp);
            let median = local[local.len() / 2];
            let mut dev: Vec<f32> = local.iter().map(|v| (v - median).abs()).collect();
            dev.sort_by(f32::total_cmp);
            let mad = dev[dev.len() / 2].max(1e-6);
            (onset[j] > median + REFINE_SURE_MADS * mad).then_some((t0 + pos / fps, onset[j]))
        })
        .collect();
    let mut hits: Vec<f32> = found.iter().flatten().map(|f| f.1).collect();
    hits.sort_by(f32::total_cmp);
    let usual = hits.get(hits.len() / 2).copied().unwrap_or(0.0);
    let found: Vec<Option<f64>> = found.into_iter().map(|f| f.filter(|f| f.1 >= PULSE_HIT * usual).map(|f| f.0)).collect();
    let mut shifts: Vec<f64> = beats.iter().zip(&found).filter_map(|(b, r)| r.map(|r| r - b)).collect();
    shifts.sort_by(f64::total_cmp);
    let shift = shifts.get(shifts.len() / 2).copied().unwrap_or(0.0);
    beats.iter().zip(found).map(|(&b, r)| r.map_or((b + shift, false), |r| (r, true))).collect()
}

/// Refined beats as stored: cleaned ([`curve::cleaned`]), every `meter`-th one starting a bar, counted along the beats
/// themselves from the phase their kick and chords give ([`structure::bar_phase`]), so a drifting tempo does not move
/// the bars.
fn end_beats(refined: &[(f64, bool)], f: &Features, meter: i64) -> Vec<Beat> {
    let heard: Vec<Beat> = refined.iter().map(|&(t, sure)| Beat { at_ms: (t * 1000.0).round() as i64, downbeat: false, sure }).collect();
    let mut beats = curve::cleaned(&heard);
    let times: Vec<f64> = beats.iter().map(|b| b.at_ms as f64 / 1000.0).collect();
    let phase = structure::bar_phase(f, &times, meter as usize);
    for (i, b) in beats.iter_mut().enumerate() {
        b.downbeat = i % meter as usize == phase;
    }
    beats
}

/// A bar and its pitch-class profile, while an end's changes are read.
type ProfiledBar = (BarProfile, [f32; 12]);

/// The mean pitch-class profile of `[from_s, to_s)`, its classes summing to 1.
fn chroma_of(f: &Features, from_s: f64, to_s: f64) -> [f32; 12] {
    let mut sum = [0f32; 12];
    if f.chroma_step > 0.0 {
        let at = |t: f64| (((t - f.chroma_t0) / f.chroma_step).round().max(0.0) as usize).min(f.chroma.len());
        for frame in &f.chroma[at(from_s)..at(to_s)] {
            for (s, v) in sum.iter_mut().zip(frame) {
                *s += v;
            }
        }
    }
    let total: f32 = sum.iter().sum();
    if total > 0.0 {
        sum.iter_mut().for_each(|v| *v /= total);
    }
    sum
}

/// One stretch of music's beat grid; all zeros when it could not be measured.
#[derive(Default, Clone, Copy, Debug)]
pub struct Grid {
    pub bpm: f64,
    pub confidence: f32,
    pub offset_ms: f64,
    pub stability: f32,
    pub downbeat_phase: i32,
    /// Where it holds, ms, when only over a run of the stretch ([`tempo::clean_run`]).
    pub span_ms: Option<(i64, i64)>,
}

/// The beat grid of `[from_s, to_s)` alone, its offset in track time. When one grid does not hold over all of it,
/// the grid of its longest steady run (the later one for an ending, `late`).
pub(crate) fn window_grid(f: &Features, from_s: f64, to_s: f64, meter: i64, late: bool) -> Grid {
    if f.fps.is_nan() || f.fps <= 0.0 || to_s - from_s < GRID_WINDOW_S / 2.0 {
        return Grid::default();
    }
    let a = (((from_s - f.t0) * f.fps).round().max(0.0) as usize).min(f.onset.len());
    let b = (((to_s - f.t0) * f.fps).round().max(0.0) as usize).min(f.onset.len());
    if b <= a {
        return Grid::default();
    }
    let t = tempo::estimate(&f.onset[a..b], f.fps, f.t0 + a as f64 / f.fps);
    if !(t.bpm > 0.0 && t.bpm.is_finite()) {
        return Grid::default();
    }
    let (t, span) = match steady_run(&t, late).filter(|(r, span)| tempo::on_beat(&f.onset, f.fps, f.t0, r.period_s, r.offset_s, *span) >= tempo::RUN_MIN_ON_BEAT) {
        Some((run, span)) => (run, Some(span)),
        None => (t, None),
    };
    let window = span.unwrap_or((from_s, to_s));
    let db = structure::downbeat(&t, f, window, Some(meter));
    let ms = |s: f64| (s * 1000.0).round() as i64;
    Grid { bpm: t.bpm, confidence: t.confidence, offset_ms: t.offset_s * 1000.0, stability: t.stability, downbeat_phase: db.phase, span_ms: span.map(|(a, b)| (ms(a), ms(b))) }
}

/// `t` over its longest steady run, and the run's first and last beat, seconds: when `t` does not hold as one grid
/// and the run does better.
fn steady_run(t: &tempo::Tempo, late: bool) -> Option<(tempo::Tempo, (f64, f64))> {
    if t.stability >= plan::MIN_STABILITY {
        return None;
    }
    let (a, b) = tempo::clean_run(&t.beats, t.period_s, late)?;
    let beats = &t.beats[a..b];
    let (first, period, rms) = tempo::fit_grid(beats, t.period_s);
    let stability = tempo::stability(beats, period, rms);
    (period > 0.0 && stability > t.stability).then(|| {
        let run = tempo::Tempo { bpm: 60.0 / period, offset_s: first.rem_euclid(period), stability, beats: beats.to_vec(), period_s: period, ..t.clone() };
        (run, (beats[0], beats[beats.len() - 1]))
    })
}

/// Wall-clock ms since the epoch.
fn now_ms() -> i64 {
    nori_time::unix_ms()
}

/// A `TrackAnalysis` plus the working data behind it, for tests and diagnostics.
pub struct Analysis {
    pub track: TrackAnalysis,
    pub tempo: tempo::Tempo,
}

/// Runs the whole-track steps on the features of one track.
pub fn finish(song_id: &str, f: &Features) -> Analysis {
    let duration_ms = (f.duration_s * 1000.0).round() as i64;
    let (s0, s1) = loudness::silence_trim(&f.blocks_raw);
    let (s0, s1) = (s0.min(duration_ms), s1.min(duration_ms));
    let lufs = loudness::integrated(&f.blocks_k);
    let (mr0, mr1) = loudness::mixramp(&f.blocks_k, lufs).map_or((s0, s1), |(a, b)| (a.clamp(s0, s1.max(s0)), b.clamp(s0, s1.max(s0))));
    let silent = s1 <= s0;
    let music = if silent { (0.0, f.duration_s) } else { (s0 as f64 / 1000.0, s1 as f64 / 1000.0) };

    let t = if silent { tempo::Tempo::default() } else { tempo::estimate(&f.onset, f.fps, f.t0) };
    let db = structure::downbeat(&t, f, music, None);
    let meter = if db.beats_per_bar == 3 { 3 } else { 4 };
    let grid_ok = t.bpm > 0.0 && t.confidence >= CUE_MIN_CONFIDENCE && t.stability >= CUE_MIN_STABILITY;
    // A short hidden track after a long silence: the song proper ends at the silence, and its outro, end grid and
    // end voices are measured there.
    let gap = if silent { None } else { loudness::last_gap(&f.blocks_raw, LONG_GAP_MS) };
    let leave = gap.filter(|(_, g1)| s1 - g1 <= plan::MAX_SKIP_MS);
    let song = leave.map_or(music, |(g0, _)| (music.0, g0 as f64 / 1000.0));
    let (intro, outro) = if silent { (0.0, 0.0) } else { structure::cues(&t, &db, f, song, grid_ok) };
    let (key, key_confidence) = if silent { (0, 0.0) } else { structure::key_of(structure::tuned_profile(f)) };
    // Brightness of the outro and intro windows, for the pair gates in `plan`.
    let (outro_centroid, intro_centroid) =
        if silent { (0.0, 0.0) } else { (window_mean(&f.centroid, f.fps, f.t0, outro, song.1), window_mean(&f.centroid, f.fps, f.t0, music.0, intro)) };

    let bar_s = if t.bpm > 0.0 { 60.0 / t.bpm * meter as f64 } else { 0.0 };
    let found = if silent || !grid_ok { None } else { structure::drop_point(&t, &db, f, music) };
    let drop = found.map(|d| d.at);
    // The last drop, after the first one.
    let last_drop = if silent || !grid_ok { None } else { structure::last_drop(&t, &db, f, music, drop) };

    // Where to leave: a closing breakdown, or the gap before a hidden track.
    let exit = if silent { None } else { structure::breakdown(&t, &db, f, music, song.1, grid_ok) }.or(leave.map(|(g0, _)| g0 as f64 / 1000.0));
    let vocal = if silent { None } else { vocal_span(f, if bar_s >= 1.0 { bar_s } else { 2.0 }, (music.0, song.1)) };

    // How far each end holds its level, in bars from the first bar line (two-second blocks without a grid).
    let (steady_intro, steady_end, steady_exit) = if silent {
        (0, 0, 0)
    } else {
        let ms = |t: f64| (t * 1000.0).round() as i64;
        let (first, bar) = structure::bar_grid(&t, &db, music).unwrap_or((music.0, 2.0));
        loudness::steady_ends(&f.blocks_raw, (s0, s1), ms(first), ms(bar), exit.map(ms))
    };

    // The end's grid is measured where a mix leaves: before a closing breakdown, whose beat has dropped away.
    let outro_to = exit.filter(|e| e - music.0 >= GRID_WINDOW_S).unwrap_or(song.1);
    let (outro_grid, intro_grid) = if silent {
        (Grid::default(), Grid::default())
    } else {
        (
            window_grid(f, (outro_to - GRID_WINDOW_S).max(music.0), outro_to, meter, true),
            window_grid(f, music.0, (music.0 + GRID_WINDOW_S).min(music.1), meter, false),
        )
    };

    // Each end in bars on its own grid (the whole song's when it has none); the ending's runs to the music's end, its
    // closing breakdown included.
    let (intro_bars, outro_bars, intro_beats, outro_beats) = if silent {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    } else {
        let bar_s = if bar_s >= 1.0 { bar_s } else { BLOCK_S };
        let n = ((music.1 - music.0) / bar_s).floor() as usize;
        // A sung bar's frames reach the floor real singing reaches, whatever the song's median.
        let sung = sung_level(&(0..n).map(|i| voice_level(f, music.0 + i as f64 * bar_s, music.0 + (i + 1) as f64 * bar_s)).collect::<Vec<_>>()).map(|l| l.max(SUNG_FLOOR));
        let strong = {
            let (a, b) = (((music.0 - f.t0) * f.fps).max(0.0) as usize, ((music.1 - f.t0) * f.fps).max(0.0) as usize);
            let mut on = f.onset[a.min(f.onset.len())..b.min(f.onset.len())].to_vec();
            on.sort_by(f32::total_cmp);
            on.get(on.len() * 9 / 10).copied().unwrap_or(0.0)
        };
        let whole = Grid { bpm: t.bpm, offset_ms: t.offset_s * 1000.0, downbeat_phase: db.phase, ..Grid::default() };
        let pick = |g: &Grid| if g.bpm > 0.0 && g.bpm.is_finite() { *g } else { whole };
        let beat = |g: &Grid| if g.bpm > 0.0 && g.bpm.is_finite() { 60.0 / g.bpm } else { 0.0 };
        let (gi, go) = (pick(&intro_grid), pick(&outro_grid));
        let beats_of = |from: f64, to: f64| {
            let these: Vec<f64> = t.beats.iter().copied().filter(|b| (from..to).contains(b)).collect();
            end_beats(&refine(&these, &f.onset, f.fps, f.t0), f, meter)
        };
        (
            end_bars(f, &gi, meter, beat(&gi), music, true, sung, strong),
            end_bars(f, &go, meter, beat(&go), song, false, sung, strong),
            beats_of(music.0, music.0 + END_BEATS_S),
            beats_of(song.1 - END_BEATS_S, song.1),
        )
    };

    let track = TrackAnalysis {
        song_id: song_id.to_string(),
        analysis_version: ANALYSIS_VERSION,
        duration_ms,
        bpm: if t.bpm.is_finite() { t.bpm } else { 0.0 },
        bpm_confidence: t.confidence,
        beat_offset_ms: t.offset_s * 1000.0,
        stability: t.stability,
        downbeat_phase: db.phase,
        downbeat_confidence: db.confidence,
        beats_per_bar: if silent || t.bpm <= 0.0 { 0 } else { meter as i32 },
        lufs: lufs as f32,
        key,
        key_confidence,
        silence_start_ms: s0,
        silence_end_ms: s1,
        mixramp_start_ms: mr0,
        mixramp_end_ms: mr1,
        intro_end_ms: (intro * 1000.0).round() as i64,
        outro_start_ms: (outro * 1000.0).round() as i64,
        outro_centroid,
        intro_centroid,
        intro_bars,
        outro_bars,
        intro_beats,
        outro_beats,
        analysed_ms: now_ms(),
        outro_bpm: outro_grid.bpm,
        outro_bpm_confidence: outro_grid.confidence,
        outro_beat_offset_ms: outro_grid.offset_ms,
        outro_stability: outro_grid.stability,
        outro_downbeat_phase: outro_grid.downbeat_phase,
        intro_bpm: intro_grid.bpm,
        intro_bpm_confidence: intro_grid.confidence,
        intro_beat_offset_ms: intro_grid.offset_ms,
        intro_stability: intro_grid.stability,
        intro_downbeat_phase: intro_grid.downbeat_phase,
        drop_ms: drop.map_or(0, |d| (d * 1000.0).round() as i64),
        drop_runup_tonal_db: found.map_or(0.0, |d| d.runup_tonal_db),
        exit_ms: exit.map_or(0, |e| (e * 1000.0).round() as i64),
        gap_ms: gap.map_or(0, |g| g.0),
        gap_end_ms: gap.map_or(0, |g| g.1),
        intro_beats_per_bar: 0,
        outro_beats_per_bar: 0,
        intro_grid_source: beats::GRID_CLASSICAL,
        outro_grid_source: beats::GRID_CLASSICAL,
        last_drop_ms: last_drop.map_or(0, |d| (d * 1000.0).round() as i64),
        vocal_start_ms: vocal.map_or(0, |v| (v.0 * 1000.0).round() as i64),
        vocal_end_ms: vocal.map_or(0, |v| (v.1 * 1000.0).round() as i64),
        steady_intro_ms: steady_intro,
        steady_end_ms: steady_end,
        steady_exit_ms: steady_exit,
        intro_grid_from_ms: intro_grid.span_ms.map_or(0, |s| s.0),
        intro_grid_to_ms: intro_grid.span_ms.map_or(0, |s| s.1),
        outro_grid_from_ms: outro_grid.span_ms.map_or(0, |s| s.0),
        outro_grid_to_ms: outro_grid.span_ms.map_or(0, |s| s.1),
    };
    Analysis { track, tempo: t }
}

/// Analyses one whole track of mono samples at `sample_rate`.
#[cfg(any(test, feature = "synth"))]
pub fn analyse(song_id: &str, pcm: &[f32], sample_rate: u32) -> Analysis {
    let mut a = Analyzer::new(sample_rate, (pcm.len() as u64 * 1000) / sample_rate.max(1) as u64);
    a.feed(pcm);
    let f = a.take_features();
    finish(song_id, &f)
}

/// Analyses interleaved little-endian PCM bytes, 16-bit or float.
#[cfg(any(test, feature = "synth"))]
pub fn analyse_bytes(song_id: &str, pcm: &[u8], sample_rate: i32, channels: i32, encoding: crate::pcm::Encoding) -> Analysis {
    let ch = channels.clamp(1, 8) as usize;
    let rate = sample_rate.max(1) as u32;
    let width = encoding.width();
    let frames = pcm.len() / width / ch;
    let mut a = Analyzer::new(rate, frames as u64 * 1000 / rate as u64);
    // Converted in slices so the whole track never exists as f32 too.
    let chunk = 1024 * ch * width;
    for part in pcm.chunks(chunk) {
        let part = &part[..part.len() / (ch * width) * (ch * width)];
        if encoding == crate::pcm::Encoding::Float {
            let mut s = [0f32; 1024 * 8];
            for (d, b) in s.iter_mut().zip(part.as_chunks::<4>().0) {
                *d = f32::from_le_bytes(*b);
            }
            a.feed_interleaved(&s[..part.len() / 4], ch, |v| v);
        } else {
            let mut s = [0i16; 1024 * 8];
            for (d, b) in s.iter_mut().zip(part.as_chunks::<2>().0) {
                *d = i16::from_le_bytes(*b);
            }
            a.feed_interleaved(&s[..part.len() / 2], ch, |v| v as f32 / 32768.0);
        }
    }
    finish(song_id, &a.take_features())
}

