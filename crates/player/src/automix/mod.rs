//! AutoMix: DJ-style transitions.
//!
//! - `analysis`, `tempo`, `structure`, `loudness`: one streamed pass over a track's PCM gives a `TrackAnalysis`.
//! - `beats` (+ `neural`, `weights`, `checkpoint` with `neural-beats`): the optional Beat This! tracker, whose
//!   grids replace the classical intro and outro grids where it is sure.
//! - `plan`: two analyses and the settings to a `TransitionPlan`.
//! - `mixer`, `stretch`, `resample`: per-buffer rendering of a plan.
//! - `vocal`: the vocal activity curve synced lyrics are checked against.
//! - `synth`, `eval` (tests and `synth`): synthetic songs the analysis and plans are scored on.

pub mod analysis;
pub mod beats;
#[cfg(any(test, feature = "neural-beats"))]
pub mod checkpoint;
pub mod loudness;
pub mod mixer;
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

use crate::types::TrackAnalysis;
#[cfg(any(test, feature = "synth"))]
use analysis::Analyzer;
use analysis::Features;

/// Bump when stored rows should be measured again. 18: an end's grid over its steady run when one grid does not hold
/// over all of it.
pub const ANALYSIS_VERSION: i32 = 18;
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
    let e = vocal::CURVE_EVERY as f64;
    let (fps, t0) = (f.fps / e, f.t0 + (e - 1.0) / 2.0 / f.fps);
    let n = ((music.1 - music.0) / bar_s).floor() as usize;
    let level = |from: f64| window_mean(&f.voice, fps, t0, from, from + bar_s);
    let back: Vec<f32> = (1..=n).map(|i| level(music.1 - i as f64 * bar_s)).collect();
    let mut sorted = back.clone();
    sorted.sort_by(f32::total_cmp);
    if *sorted.last()? < SUNG_FLOOR {
        return None;
    }
    let sung = SUNG_SHARE * *sorted.get(n / 2)?;
    let last = back.iter().position(|&b| b >= sung)?;
    let end = music.1 - last as f64 * bar_s;
    let sung_at = |i: usize| level(music.0 + i as f64 * bar_s) >= sung;
    let first = (0..n.saturating_sub(1)).find(|&i| sung_at(i) && sung_at(i + 1)).map_or(end - bar_s, |i| music.0 + i as f64 * bar_s);
    Some((first.min(end - bar_s).max(music.0), end))
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

    let (outro_grid, intro_grid) = if silent {
        (Grid::default(), Grid::default())
    } else {
        (
            window_grid(f, (song.1 - GRID_WINDOW_S).max(music.0), song.1, meter, true),
            window_grid(f, music.0, (music.0 + GRID_WINDOW_S).min(music.1), meter, false),
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

