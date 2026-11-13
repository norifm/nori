//! Transition planning: a pure function from two analyses and the settings to a `TransitionPlan`.
//! The fallback ladder, most to least informed:
//!
//! 1. both grids confident, tempos within reach: beat-matched, the incoming drop (or intro end, or a phrase line)
//!    landing on an outgoing downbeat where the bass swaps. Camelot distance sets length and filter;
//! 2. one grid: a fade on its downbeats; in the DJ style, an echo-out of the outgoing song after its last line
//!    instead, on its grid or its tempo;
//! 3. an analysis but no usable grid: MixRamp overlap with a filter sweep;
//! 4. nothing known: a fixed equal-power crossfade;
//! 5. same album in order: gapless.
//!
//! Loudness or timbre gaps only shorten; gates never upgrade a transition. The style ([`MixStyle`]) sets how far a
//! beat-matched mix goes: Smooth keeps it within a phrase or two; DJ runs up to 32 bars, loops an instrumental
//! intro to make the run-up, glides the tempo between the songs during the mix, and lands the incoming drop on the
//! outgoing song's last one (a double drop).
//!
//! Hard rule: at most `MAX_SKIP_MS` of either track's music goes unplayed (silence is free), or for a double drop
//! `DOUBLE_DROP_MAX_SKIP_MS` of the outgoing one. The outgoing song may be left early at a closing breakdown or
//! before a hidden track (`Ending`) within that cap.

use super::structure::key_distance;
use super::tempo::{fold, match_ratio};
use crate::types::{AutoMixSettings, BassSwap, Bend, Echo, FadeCurve, HighsIn, InLoop, MixStyle, Sweep, TrackAnalysis, TransitionKind, TransitionPlan, VocalDuck};

pub(crate) const MAX_SKIP_MS: i64 = 15_000;
/// The outgoing music a double drop may leave unplayed: the outro after its last drop.
pub(crate) const DOUBLE_DROP_MAX_SKIP_MS: i64 = 32_000;
pub(crate) const MIN_BPM_CONFIDENCE: f32 = 0.5;
pub(crate) const MIN_STABILITY: f32 = 0.6;
/// Varispeed moves pitch 0.34 semitone per 2 %; beyond that detuning is audible.
pub(crate) const VARISPEED_MAX_PCT: f64 = 2.0;
/// Longest fixed crossfade.
const MAX_BLIND_FADE_MS: i64 = 12_000;
/// Shortest overlap that is still a fade rather than a click guard.
const MIN_FADE_MS: i64 = 300;
/// Shortest MixRamp overlap: shorter overlaps on a fading ending are heard as no mix at all.
const MIN_MIXRAMP_MS: i64 = 5_000;
const BASS_CUT_HZ: f32 = 180.0;
const SWEEP_FROM_HZ: f32 = 18_000.0;
/// Where the outgoing low-pass ends: beat-matched and plain fades.
const SWEEP_TO_HZ_MATCHED: f32 = 400.0;
const SWEEP_TO_HZ_FADE: f32 = 500.0;
/// Camelot neighbour or relative: gentle muffling.
const SWEEP_TO_HZ_SOFT: f32 = 2_500.0;

fn low_pass(start_ms: i64, end_ms: i64, to_hz: f32) -> Sweep {
    Sweep { start_ms, end_ms, from_hz: SWEEP_FROM_HZ, to_hz }
}

fn blank(kind: TransitionKind, out_start: i64, in_start: i64, duration: i64, reason: String) -> TransitionPlan {
    TransitionPlan {
        kind,
        out_start_ms: out_start,
        in_start_ms: in_start,
        duration_ms: duration,
        tempo_ratio: 1.0,
        tempo_ramp_beats: 0,
        tempo_ramp_ms: 0,
        keep_pitch: true,
        fade_curve: FadeCurve::EqualPower,
        out_fade_start_ms: 0,
        out_fade_end_ms: duration,
        in_fade_start_ms: 0,
        in_fade_end_ms: duration,
        out_gain_db: 0.0,
        in_gain_db: 0.0,
        bass_swap: None,
        highs_in: None,
        low_pass: None,
        high_pass: None,
        echo: None,
        out_loop_ms: None,
        vocal_duck: None,
        in_loop: None,
        bend: None,
        in_low_pass: None,
        in_high_pass: None,
        mid_dip: None,
        reverb: None,
        in_delay_ms: 0,
        out_glide: None,
        brake: None,
        reason,
    }
}

/// An analysis only counts when it is sane and describes this file (same length within 3 s).
fn usable(a: Option<&TrackAnalysis>, duration_ms: i64) -> Option<&TrackAnalysis> {
    a.filter(|a| sound(a) && (a.duration_ms <= 0 || duration_ms <= 0 || (a.duration_ms - duration_ms).abs() <= 3000))
}

/// The fastest tempo a stored grid may have and still be taken for one.
const MAX_BPM: f64 = 1_000.0;
/// Positions beyond this (ms, a hundred hours) are corrupt.
const MAX_MS: f64 = 3.6e8;

/// Whether every number a plan uses is finite and in range. A corrupt row is ignored rather than planned into
/// nonsense or a panic on the engine thread.
fn sound(a: &TrackAnalysis) -> bool {
    let tempo = |b: f64| b.is_finite() && (0.0..=MAX_BPM).contains(&b);
    let place = |ms: f64| ms.is_finite() && ms.abs() <= MAX_MS;
    let whole = |ms: i64| (ms as f64).abs() <= MAX_MS;
    let share = |v: f32| v.is_finite();
    [a.bpm, a.intro_bpm, a.outro_bpm].into_iter().all(tempo)
        && [a.beat_offset_ms, a.intro_beat_offset_ms, a.outro_beat_offset_ms].into_iter().all(place)
        && [a.duration_ms, a.silence_start_ms, a.silence_end_ms, a.mixramp_start_ms, a.mixramp_end_ms, a.intro_end_ms, a.outro_start_ms, a.drop_ms, a.exit_ms, a.gap_ms, a.gap_end_ms]
            .into_iter()
            .all(whole)
        && [
            a.bpm_confidence,
            a.stability,
            a.downbeat_confidence,
            a.lufs,
            a.key_confidence,
            a.outro_centroid,
            a.intro_centroid,
            a.outro_bpm_confidence,
            a.outro_stability,
            a.intro_bpm_confidence,
            a.intro_stability,
            a.drop_runup_tonal_db,
        ]
        .into_iter()
        .all(share)
}

/// Beats in a bar: 3 for a waltz, else 4 (also for rows without a measured metre).
pub(super) fn bar_beats(a: &TrackAnalysis) -> i64 {
    if a.beats_per_bar == 3 {
        3
    } else {
        4
    }
}

/// Index of the last downbeat at or before `t_ms` on a grid of `beat_ms` beats from `offset_ms`, whose
/// downbeats are the beats `phase` mod `bpb`.
fn downbeat_before(t_ms: f64, offset_ms: f64, beat_ms: f64, bpb: i64, phase: i64) -> i64 {
    let phase = phase.rem_euclid(bpb);
    let mut n = ((t_ms - offset_ms) / beat_ms).floor() as i64;
    while n.rem_euclid(bpb) != phase {
        n -= 1;
    }
    n
}

/// Index of the first downbeat from `t_ms` (a quarter beat early still counts) on the same grid.
fn downbeat_from(t_ms: f64, offset_ms: f64, beat_ms: f64, bpb: i64, phase: i64) -> i64 {
    let phase = phase.rem_euclid(bpb);
    let mut n = (((t_ms - offset_ms) / beat_ms) - 0.25).ceil() as i64;
    while n.rem_euclid(bpb) != phase {
        n += 1;
    }
    n
}

pub(super) fn grid_ok(a: &TrackAnalysis) -> bool {
    a.bpm > 0.0 && a.bpm.is_finite() && a.bpm_confidence >= MIN_BPM_CONFIDENCE && a.stability >= MIN_STABILITY
}

/// `a` with its grid replaced by the one measured over one end: live bands drift and songs change tempo, so
/// the whole-song grid is wrong at the ends. The end's grid wins unless only the whole-song grid is usable; `None`
/// keeps the whole-song grid. `meter` is the end's own metre (Beat This!), 0 for none.
fn with_grid(a: &TrackAnalysis, bpm: f64, confidence: f32, offset_ms: f64, stability: f32, phase: i32, meter: i32) -> Option<TrackAnalysis> {
    let beats_per_bar = if meter > 0 { meter } else { a.beats_per_bar };
    let w = TrackAnalysis { bpm, bpm_confidence: confidence, beat_offset_ms: offset_ms, stability, downbeat_phase: phase, beats_per_bar, ..a.clone() };
    let measured = bpm > 0.0 && bpm.is_finite();
    (measured && (grid_ok(&w) || !grid_ok(a))).then_some(w)
}

/// The outgoing song, gridded over its last seconds. Gridded over the whole song, the end's span says nothing.
pub(super) fn at_end(a: &TrackAnalysis) -> TrackAnalysis {
    with_grid(a, a.outro_bpm, a.outro_bpm_confidence, a.outro_beat_offset_ms, a.outro_stability, a.outro_downbeat_phase, a.outro_beats_per_bar)
        .unwrap_or_else(|| TrackAnalysis { outro_grid_from_ms: 0, outro_grid_to_ms: 0, ..a.clone() })
}

/// The incoming song, gridded over its first seconds.
pub(super) fn at_start(b: &TrackAnalysis) -> TrackAnalysis {
    with_grid(b, b.intro_bpm, b.intro_bpm_confidence, b.intro_beat_offset_ms, b.intro_stability, b.intro_downbeat_phase, b.intro_beats_per_bar)
        .unwrap_or_else(|| TrackAnalysis { intro_grid_from_ms: 0, intro_grid_to_ms: 0, ..b.clone() })
}

/// Whether song time `from` to `to` lies in the stretch `(start, end)` ms an end's grid holds over (both 0: all of
/// it), give or take half a `beat`.
fn in_grid((start, end): (i64, i64), from: f64, to: f64, beat: f64) -> bool {
    end <= 0 || (from >= start as f64 - 0.5 * beat && to <= end as f64 + 0.5 * beat)
}

/// The analysis BPM, halved or doubled when that lands within 4 % of the tag BPM.
fn bpm_with_tag(analysis: f64, tag: f32) -> f64 {
    if !(analysis > 0.0 && analysis.is_finite()) {
        return analysis;
    }
    let tag = tag as f64;
    if !(tag > 0.0 && tag.is_finite()) {
        return analysis;
    }
    let mut best = analysis;
    let mut best_err = (analysis - tag).abs() / tag;
    for c in [analysis, analysis * 2.0, analysis / 2.0] {
        let err = (c - tag).abs() / tag;
        if err < best_err {
            best = c;
            best_err = err;
        }
    }
    if best_err <= 0.04 {
        best
    } else {
        analysis
    }
}

fn music_end(a: &TrackAnalysis, duration: i64) -> i64 {
    if a.silence_end_ms > a.silence_start_ms && a.silence_end_ms <= duration {
        a.silence_end_ms
    } else {
        duration
    }
}

/// The end of the outgoing song: where its music ends, a long silence inside it, and where a mix may leave.
/// Skipped silence costs nothing against `MAX_SKIP_MS`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ending {
    /// End of the last audible music, ms.
    pub music_end: f64,
    /// The long silence inside the music, when there is one.
    pub gap: Option<(f64, f64)>,
    /// Where to leave: a closing breakdown or the gap before a hidden track; else `music_end`.
    pub exit: f64,
    /// `exit` starts a closing breakdown (music, not a gap).
    pub breakdown: bool,
}

impl Ending {
    pub fn of(a: &TrackAnalysis, duration: i64) -> Ending {
        let music_end = music_end(a, duration) as f64;
        let gap = (a.gap_ms > a.silence_start_ms && a.gap_end_ms > a.gap_ms && a.gap_end_ms as f64 <= music_end)
            .then_some((a.gap_ms as f64, a.gap_end_ms as f64));
        let mut e = Ending { music_end, gap, exit: music_end, breakdown: false };
        let x = a.exit_ms as f64;
        if a.exit_ms > a.silence_start_ms && x < music_end {
            e.exit = x;
            e.breakdown = gap.is_none_or(|(g0, _)| (g0 - x).abs() > 1.0);
        }
        e
    }

    /// The ring-out after the song's stop, ms: a closing breakdown short enough to be its last hit or chord dying away.
    pub fn ring_out(&self) -> Option<f64> {
        let ring = self.music_end - self.exit;
        (self.breakdown && ring <= RING_OUT_MS).then_some(ring)
    }

    /// Music (not silence) after `heard_end`, ms.
    pub fn skipped(&self, heard_end: f64) -> f64 {
        let total = (self.music_end - heard_end).max(0.0);
        let silent = self.gap.map_or(0.0, |(g0, g1)| (g1 - g0.max(heard_end)).max(0.0));
        (total - silent).max(0.0)
    }

    /// The exit when the music after it fits the cap plus `slack` (a mix tail heard after the swap), else the
    /// end of the music.
    pub fn leave(&self, slack: f64) -> f64 {
        if self.skipped(self.exit) <= MAX_SKIP_MS as f64 + slack {
            self.exit
        } else {
            self.music_end
        }
    }

    /// Silence (the gap, or after the music) inside `[t0, t1)`, ms.
    pub fn silence(&self, t0: f64, t1: f64) -> f64 {
        let over = |a: f64, b: f64| (t1.min(b) - t0.max(a)).max(0.0);
        self.gap.map_or(0.0, |(g0, g1)| over(g0, g1)) + over(self.music_end, f64::MAX)
    }

    /// The closing breakdown inside `[t0, t1)`, ms.
    pub fn coda(&self, t0: f64, t1: f64) -> f64 {
        if !self.breakdown {
            return 0.0;
        }
        let end = self.gap.map_or(self.music_end, |(g0, _)| if g0 > self.exit { g0 } else { self.music_end });
        (t1.min(end) - t0.max(self.exit)).max(0.0)
    }
}

/// The longest closing breakdown that is a song's stop ringing out rather than a coda, ms.
const RING_OUT_MS: f64 = 6_000.0;

/// A song that stops cold ends on its stop when the pair cannot blend: the next song starts there at once, under the
/// ring-out, rather than over a fade of the bars before it.
fn on_its_stop(a: &TrackAnalysis, b: Option<&TrackAnalysis>, room: Room, s: &AutoMixSettings, why_not: &str) -> Option<TransitionPlan> {
    let Room { out_dur, in_dur, max_len } = room;
    let ending = Ending::of(a, out_dur);
    let ring = ending.ring_out()?;
    let dur = (ring.round() as i64).min(max_len);
    if dur < MIN_FADE_MS {
        return None;
    }
    let in_start = b.map_or(0, |b| b.silence_start_ms.clamp(0, in_dur / 3));
    let mut p = blank(TransitionKind::MixRampFade, ending.exit.round() as i64, in_start, dur, String::new());
    p.in_fade_end_ms = MIN_FADE_MS;
    p.in_gain_db = loudness_trim(Some(a), b, s);
    let tail = if why_not.is_empty() { String::new() } else { format!(" ({why_not})") };
    p.reason = format!("on its stop, the next song under the ring-out{tail}");
    Some(p)
}

fn loudness_trim(a: Option<&TrackAnalysis>, b: Option<&TrackAnalysis>, s: &AutoMixSettings) -> f32 {
    match (a, b) {
        (Some(a), Some(b)) if s.match_loudness && a.lufs > -60.0 && b.lufs > -60.0 => (a.lufs - b.lufs).clamp(-9.0, 9.0),
        _ => 0.0,
    }
}

/// How the two tracks sound together, from the stored overlap windows.
#[derive(Clone, Copy)]
struct Verdict {
    /// Two vocals, or confident keys far apart: do not blend.
    clash: bool,
    cause: &'static str,
    /// A large loudness or brightness gap: keep the overlap short.
    shorten: bool,
    short_cause: &'static str,
}

/// Key confidence a key is trusted at.
const KEY_MIN_CONFIDENCE: f32 = 0.4;
/// Confident keys this far apart on the Camelot wheel do not blend.
const KEY_FAR: i32 = 4;
const KEYS_FAR: &str = "keys far apart";
/// Bars at a song's end and start whose singing makes them sung, for the pair gate.
const SUNG_BARS: f64 = 8.0;
/// Loudness gap that shortens an overlap, dB.
const LOUD_GAP_DB: f32 = 6.0;
/// Brightness ratio (octaves of centroid) that counts as a timbre mismatch.
const TIMBRE_OCTAVES: f64 = 1.0;

const VOCALS_OVERLAP: &str = "vocals overlap";
/// Incoming voice-band cut while the outgoing song sings: 18 dB at 1 kHz, about 6 dB at 500 Hz and 2 kHz.
const VOCAL_DUCK_DB: f32 = -18.0;
const VOCAL_DUCK_HZ: f32 = 1_000.0;
/// The high-pass that thins the outgoing voice after the swap, over the first half of the rest of the mix.
const VOCAL_HP_FROM_HZ: f32 = 200.0;
pub(super) const VOCAL_HP_TO_HZ: f32 = 2_000.0;

/// Two voices over a plain fade: duck the incoming voice band through the first half.
fn separate_in_fade(p: &mut TransitionPlan, s: &AutoMixSettings, sung_both: bool) {
    if !sung_both || !s.filter_effects || p.duration_ms < 2 * MIN_FADE_MS {
        return;
    }
    p.vocal_duck = Some(VocalDuck { until_ms: p.duration_ms / 2, release_ms: (p.duration_ms / 4).min(500), db: VOCAL_DUCK_DB, hz: VOCAL_DUCK_HZ });
    p.reason += ", voices kept apart";
}

/// A plain fade out of a song still singing once it would fall: the song holds until its last line ends, then falls
/// over what is left, a bar at least, as the incoming song, risen under it, takes over; that one's voice band is cut
/// until then so the line stays clear, and the outgoing song is darkened only once it has sung.
fn hold_through_line(p: &mut TransitionPlan, a: Option<&TrackAnalysis>, s: &AutoMixSettings) {
    let Some(a) = a.filter(|a| a.vocal_end_ms > 0) else {
        return;
    };
    let hold = (a.vocal_end_ms - p.out_start_ms).min(p.duration_ms - bar_ms(a).round() as i64);
    if hold <= p.out_fade_start_ms {
        return;
    }
    p.out_fade_start_ms = hold;
    if let Some(l) = p.low_pass.as_mut() {
        l.start_ms = l.start_ms.max(hold);
    }
    if s.filter_effects && p.vocal_duck.is_none_or(|d| d.until_ms < hold) {
        p.vocal_duck = Some(VocalDuck { until_ms: hold, release_ms: (p.duration_ms - hold).min(500), db: VOCAL_DUCK_DB, hz: VOCAL_DUCK_HZ });
    }
    p.reason += ", held through its last line";
}

/// A plain fade's voices and bass.
fn shape_fade(mut p: TransitionPlan, a: Option<&TrackAnalysis>, s: &AutoMixSettings, sung_both: bool) -> TransitionPlan {
    separate_in_fade(&mut p, s, sung_both);
    hold_through_line(&mut p, a, s);
    hand_over_bass_in_fade(&mut p, s);
    p
}

/// A plain fade in the DJ style: the bass changes hands once, quickly, where the fades cross, so two kicks that are
/// not beat-matched never thud together.
fn hand_over_bass_in_fade(p: &mut TransitionPlan, s: &AutoMixSettings) {
    if !s.bass_swap || !profile(s.style).fade_swap || p.duration_ms < FADE_SWAP_MIN_MS {
        return;
    }
    let (from, to) = (p.in_fade_start_ms.max(p.out_fade_start_ms), p.in_fade_end_ms.min(p.out_fade_end_ms));
    let len = (p.duration_ms / 8).clamp(FADE_SWAP_MS.0, FADE_SWAP_MS.1);
    let at = ((from + to.max(from)) / 2 - len / 2).clamp(0, p.duration_ms - len);
    p.bass_swap = Some(BassSwap { at_ms: at, len_ms: len, cut_hz: BASS_CUT_HZ });
    p.reason += ", the bass handed over";
}

/// A plain fade this long or longer hands the bass over, in this many ms at least and at most.
const FADE_SWAP_MIN_MS: i64 = 2_000;
const FADE_SWAP_MS: (i64, i64) = (250, 1_000);

/// A bar of `a`, ms; two seconds without a tempo.
fn bar_ms(a: &TrackAnalysis) -> f64 {
    if a.bpm > 0.0 && a.bpm.is_finite() { 60_000.0 / a.bpm * bar_beats(a) as f64 } else { 2_000.0 }
}

/// The outgoing song still sings after its time `t`, ms.
fn sings_after(a: &TrackAnalysis, t: f64) -> bool {
    a.vocal_end_ms > 0 && a.vocal_end_ms as f64 > t
}

/// The incoming song sings before its time `t`, ms.
fn sings_before(b: &TrackAnalysis, t: f64) -> bool {
    b.vocal_end_ms > 0 && (b.vocal_start_ms as f64) < t
}

/// Both ends sung: the outgoing song's last bars before it leaves, and the incoming song's first.
fn sung_both_ends(a: &TrackAnalysis, b: &TrackAnalysis, out_dur: i64) -> bool {
    sings_after(a, Ending::of(a, out_dur).leave(0.0) - SUNG_BARS * bar_ms(a)) && sings_before(b, b.silence_start_ms as f64 + SUNG_BARS * bar_ms(b))
}

fn pair_gate(a: &TrackAnalysis, b: &TrackAnalysis, out_dur: i64) -> Verdict {
    let mut v = Verdict { clash: false, cause: "", shorten: false, short_cause: "" };
    if camelot_dist(a, b).is_some_and(|d| d >= KEY_FAR) {
        v.clash = true;
        v.cause = KEYS_FAR;
    } else if sung_both_ends(a, b, out_dur) {
        v.clash = true;
        v.cause = VOCALS_OVERLAP;
    }
    if a.lufs > -60.0 && b.lufs > -60.0 && (a.lufs - b.lufs).abs() > LOUD_GAP_DB {
        v.shorten = true;
        v.short_cause = "loudness gap";
    } else if a.outro_centroid > 0.0
        && b.intro_centroid > 0.0
        && (a.outro_centroid as f64 / b.intro_centroid as f64).log2().abs() > TIMBRE_OCTAVES
    {
        v.shorten = true;
        v.short_cause = "timbre mismatch";
    }
    v
}

/// Camelot distance when both keys are trusted.
fn camelot_dist(a: &TrackAnalysis, b: &TrackAnalysis) -> Option<i32> {
    (a.key_confidence >= KEY_MIN_CONFIDENCE && b.key_confidence >= KEY_MIN_CONFIDENCE).then(|| key_distance(a.key, b.key)).flatten()
}

/// Low-pass for MixRamp and one-grid fades: soft when keys are neighbours, full otherwise.
fn apply_fade_filter(p: &mut TransitionPlan, a: Option<&TrackAnalysis>, b: Option<&TrackAnalysis>, s: &AutoMixSettings, dur: i64) {
    if !s.filter_effects || dur < 2000 {
        return;
    }
    let soft = a.zip(b).is_some_and(|(a, b)| matches!(camelot_dist(a, b), Some(0 | 1)));
    p.low_pass = Some(if soft { low_pass(dur / 2, dur, SWEEP_TO_HZ_SOFT) } else { low_pass(0, dur, SWEEP_TO_HZ_FADE) });
}

pub fn plan(out: Option<&TrackAnalysis>, inc: Option<&TrackAnalysis>, out_duration_ms: i64, in_duration_ms: i64, s: &AutoMixSettings) -> TransitionPlan {
    let out_dur = if out_duration_ms > 0 { out_duration_ms } else { out.map_or(0, |a| a.duration_ms) };
    let in_dur = if in_duration_ms > 0 { in_duration_ms } else { inc.map_or(0, |a| a.duration_ms) };
    if s.same_album_in_order {
        return blank(TransitionKind::Gapless, out_dur.max(0), 0, 0, "same album in order: gapless".into());
    }
    let max_len = if s.max_transition_s.is_finite() { (s.max_transition_s.clamp(0.0, 60.0) * 1000.0) as i64 } else { 0 };
    if out_dur <= 0 || in_dur <= 0 || max_len < MIN_FADE_MS {
        return blank(TransitionKind::Gapless, out_dur.max(0), 0, 0, "no room for a transition: gapless".into());
    }
    // Never more than a third of either track.
    let max_len = max_len.min(out_dur / 3).min(in_dur / 3);
    if max_len < MIN_FADE_MS {
        return blank(TransitionKind::Gapless, out_dur, 0, 0, "tracks too short: gapless".into());
    }
    let room = Room { out_dur, in_dur, max_len };
    let (a, b) = (usable(out, out_dur).map(at_end), usable(inc, in_dur).map(at_start));
    let (a, b) = (a.as_ref(), b.as_ref());
    let mut why_not = String::new();
    let verdict = a.zip(b).map(|(a, b)| pair_gate(a, b, out_dur));
    let short_cause = verdict.map(|v| if v.clash { v.cause } else { v.short_cause }).unwrap_or("");
    let short = !short_cause.is_empty();
    if let (Some(a), Some(b)) = (a, b) {
        if !s.beat_match {
            why_not = "beat matching off".into();
        } else if !grid_ok(a) || !grid_ok(b) {
            why_not = format!(
                "no reliable beat grid (out {:.0} BPM conf {:.2} stab {:.2}, in {:.0} BPM conf {:.2} stab {:.2}; needs conf ≥ {:.1}, stab ≥ {:.1})",
                a.bpm, a.bpm_confidence, a.stability, b.bpm, b.bpm_confidence, b.stability,
                MIN_BPM_CONFIDENCE, MIN_STABILITY
            );
        } else {
            match beat_matched(a, b, room, s, short_cause) {
                Ok(p) => return p,
                Err(e) => why_not = e,
            }
        }
    }
    if let Some(p) = a.and_then(|a| on_its_stop(a, b, room, s, &why_not)) {
        return p;
    }
    if let Some(p) = a.filter(|_| profile(s.style).echo_out).and_then(|a| echo_out(a, b, room, s, &why_not)) {
        return p;
    }
    let sung_both = verdict.is_some_and(|v| v.cause == VOCALS_OVERLAP);
    if s.beat_match && (a.is_some_and(grid_ok) || b.is_some_and(grid_ok)) {
        if let Some(p) = one_grid(a, b, room, s, short, &why_not) {
            return shape_fade(p, a, s, sung_both);
        }
    }
    if a.is_some() || b.is_some() {
        return shape_fade(mixramp(a, b, room, s, &why_not), a, s, sung_both);
    }
    let dur = max_len.min(MAX_BLIND_FADE_MS);
    blank(TransitionKind::EqualPowerFade, out_dur - dur, 0, dur, "not analysed: equal-power fade".into())
}

/// What a transition must fit in, ms: both songs' lengths and the longest overlap.
#[derive(Clone, Copy)]
struct Room {
    out_dur: i64,
    in_dur: i64,
    max_len: i64,
}

/// Two grids locked together.
struct Lock {
    ratio: f64,
    pct: f64,
    bpm_a: f64,
    /// The incoming tempo folded to the outgoing octave.
    b_bpm: f64,
    bpb: i64,
    /// Outgoing beat and bar, ms (the incoming ones too once stretched).
    beat: f64,
    bar: f64,
    dist: Option<i32>,
    mild_clash: bool,
    /// Keys at least [`KEY_FAR`] apart.
    far: bool,
    max_bars: i64,
    /// A tempo gap past the limit: the fewest bars it glides over, across the whole mix; 0 for none.
    glide_bars: f64,
    /// Filters are on, so two voices can be kept apart.
    separate: bool,
    profile: &'static Profile,
}

/// How far a beat-matched mix goes, by [`MixStyle`].
struct Profile {
    /// Run-ups tried, bars, longest first.
    runups: &'static [i64],
    /// Run-up bars that add to a window's worth.
    runup_worth_bars: i64,
    /// A phrase, bars: a run-up of whole phrases is worth more.
    phrase: i64,
    /// The longest mix, bars: keys the same or neighbours, two apart or a pair gate, clashing.
    most_bars: [i64; 3],
    /// An instrumental incoming opening may loop to make a run-up longer than it.
    in_loop: bool,
    /// The tempo glides between the songs over the mix's last bars, not after it.
    bend: bool,
    /// The incoming drop may land on the outgoing song's last one.
    double_drop: bool,
    /// Bars the bass takes to change hands, but for a sixteenth into a drop when `quick_drop_swap`.
    swap_bars: f64,
    quick_drop_swap: bool,
    /// The incoming highs come in before the bass ([`HighsIn`]).
    highs_in: bool,
    /// A plain fade hands the bass over ([`hand_over_bass_in_fade`]).
    fade_swap: bool,
    /// The outgoing song thins out (a rising high-pass) over the bars before the incoming drop.
    riser: bool,
    /// The outgoing song fades over a bar at least after the swap, unless its music ends sooner.
    bar_tail: bool,
    /// A pair that cannot be beat-matched echoes out ([`echo_out`]) rather than fading.
    echo_out: bool,
    /// Per bar of looped outgoing slice heard again.
    loop_cost: f64,
    /// Per second of either song's singing under the run-up: a mix waits for the outgoing voice to end, or loops
    /// the bars after it, and swaps before the incoming voice starts.
    sung_under_cost: f64,
}

const SMOOTH: Profile = Profile {
    runups: &[16, 12, 8, 6, 4, 2, 0],
    runup_worth_bars: 8,
    phrase: 4,
    most_bars: [16, 8, 8],
    in_loop: false,
    bend: false,
    double_drop: false,
    swap_bars: 1.0,
    quick_drop_swap: false,
    highs_in: false,
    fade_swap: false,
    riser: false,
    bar_tail: false,
    echo_out: false,
    loop_cost: 0.1,
    sung_under_cost: 0.0,
};

const DJ: Profile = Profile {
    runups: &[32, 24, 16, 12, 8, 6, 4, 2, 0],
    runup_worth_bars: 16,
    phrase: 8,
    most_bars: [32, 16, 8],
    in_loop: true,
    bend: true,
    double_drop: true,
    swap_bars: 0.25,
    quick_drop_swap: true,
    highs_in: true,
    fade_swap: true,
    riser: true,
    bar_tail: true,
    echo_out: true,
    loop_cost: 0.04,
    sung_under_cost: 0.1,
};

fn profile(style: MixStyle) -> &'static Profile {
    match style {
        MixStyle::Smooth => &SMOOTH,
        MixStyle::Dj => &DJ,
    }
}

/// Keys far apart: the longest mix, bars (a run-up without chords aside).
const FAR_BARS: i64 = 4;

/// Where in the incoming song the swap lands, best first.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Landing {
    /// `drop_ms`.
    Drop,
    /// The end of the intro, when it is not the drop.
    IntroEnd,
    /// A four-bar line from the first downbeat.
    Phrase,
    /// The first downbeat: no run-up.
    Start,
}

impl Landing {
    fn worth(self) -> f64 {
        match self {
            Landing::Drop => 4.0,
            Landing::IntroEnd => 3.0,
            Landing::Phrase => 1.0,
            Landing::Start => 0.0,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Landing::Drop => "in on the drop",
            Landing::IntroEnd => "in at the end of the intro",
            Landing::Phrase => "on a phrase line",
            Landing::Start => "straight in",
        }
    }
}

/// A candidate beat-matched window, ms: the outgoing song from `start` for `dur`, the incoming from `in_start`,
/// the bass swapping `swap` into it.
#[derive(Clone, Copy, Debug)]
struct Window {
    start: f64,
    dur: f64,
    swap: f64,
    in_start: f64,
    runup_bars: i64,
    landing: Landing,
    on_phrase: bool,
    /// The swap is on the bar where the outgoing singing ends.
    after_line: bool,
    /// Length of the looped outgoing slice, ms; 0 for no loop.
    out_loop: f64,
    /// The incoming opening looped: bars, and the times it plays again.
    in_loop: Option<(i64, i64)>,
    /// The incoming drop lands on the outgoing song's last drop.
    double: bool,
    /// The incoming song sings in the run-up, and before the mix ends.
    runup_sung: bool,
    after_sung: bool,
    /// Both songs singing at once, ms.
    both_sung: f64,
    score: f64,
}

/// Score weights. Per second skipped of instrumental and of sung music, and per second of incoming intro heard
/// alone after the mix.
const SKIP_COST: f64 = 0.06;
const SKIP_SUNG_COST: f64 = 0.15;
const DIP_COST: f64 = 0.1;
/// Per bar of run-up (up to eight), and per bar of mix.
const RUNUP_WORTH: f64 = 0.1;
const LENGTH_WORTH: f64 = 0.02;
/// Swap on a four-bar line of the outgoing sections.
const PHRASE_WORTH: f64 = 0.5;
/// Per second of both songs singing at once.
const BOTH_SUNG_COST: f64 = 1.0;
/// Per second of outgoing singing faded under the incoming song after the swap.
const SUNG_FADE_COST: f64 = 0.1;
/// Skipping the incoming drop.
const DROP_SKIP_COST: f64 = 2.0;
/// Per bar of looped incoming opening heard again (the outgoing slice's is the profile's).
const IN_LOOP_COST: f64 = 0.05;
/// Most bars of incoming opening played again.
const MAX_IN_LOOPED_BARS: i64 = 8;
/// The incoming song coming in already singing, past an instrumental opening.
const ENTER_SUNG_COST: f64 = 2.0;
/// Both drops together.
const DOUBLE_DROP_WORTH: f64 = 3.0;
/// Per second of outgoing silence under the run-up or before the mix.
const DEAD_AIR_COST: f64 = 0.2;
/// Swap on the outgoing exit.
const EXIT_WORTH: f64 = 0.5;
/// Run-up of whole four-bar phrases.
const PHRASE_START_WORTH: f64 = 0.3;
/// A run-up this far below the song's chord energy, dB, has no chords to clash.
const CHORDLESS_DB: f32 = 3.0;

/// The best beat-matched window: every incoming landing (drop, intro end, phrase lines, first downbeat) against
/// every downbeat of the outgoing song's last sixteen bars (and those before its exit, and its last drop for a
/// double drop), with the profile's run-ups down to none and tails of a beat to four bars. Windows over the skip
/// cap or that do not fit are dropped; the rest are scored.
fn drop_aligned(a: &TrackAnalysis, b: &TrackAnalysis, room: Room, bpm_b: f64, k: &Lock) -> Option<Window> {
    let Room { out_dur, in_dur, max_len } = room;
    let (beat, bar, ratio) = (k.beat, k.bar, k.ratio);
    let ending = Ending::of(a, out_dur);
    let downbeat_at = |t: f64| {
        let n = downbeat_before(t + 0.5 * beat, a.beat_offset_ms, beat, k.bpb, a.downbeat_phase as i64);
        a.beat_offset_ms + n as f64 * beat
    };
    let exit = ending.leave(4.0 * bar);
    let prof = k.profile;
    // (where the swap lands, bars before the end, a double drop)
    let mut anchors: Vec<(f64, usize, bool)> = Vec::new();
    for end in [ending.music_end, exit] {
        let last = downbeat_at(end);
        for back in 0..=16usize {
            let at = last - back as f64 * bar;
            if anchors.iter().all(|(t, ..)| (t - at).abs() > 0.5 * beat) {
                anchors.push((at, back, false));
            }
        }
    }
    // Both drops at once: keys that blend, and not two voices over them.
    let both_sung = sings_after(a, a.last_drop_ms as f64) && sings_before(b, b.drop_ms as f64 + SUNG_BARS * bar_ms(b));
    if prof.double_drop && a.last_drop_ms > 0 && b.drop_ms > 0 && matches!(k.dist, Some(0 | 1)) && !both_sung {
        let at = downbeat_at(a.last_drop_ms as f64);
        let back = ((downbeat_at(ending.music_end) - at) / bar).round().max(0.0) as usize;
        anchors.retain(|(t, ..)| (t - at).abs() > 0.5 * beat);
        anchors.push((at, back, true));
    }
    let on_line = |t: f64, from: f64| {
        let x = (t - from) / (4.0 * bar);
        (x - x.round()).abs() * 4.0 < 0.05
    };
    // The incoming song on its native grid.
    let beat_n = 60_000.0 / bpm_b;
    let bar_in = k.bar * ratio;
    let nb = downbeat_from(b.silence_start_ms as f64, b.beat_offset_ms, beat_n, k.bpb, b.downbeat_phase as i64);
    let first = b.beat_offset_ms + nb as f64 * beat_n;
    // An analysis cue snapped to the nearest incoming downbeat.
    let snap = |t: f64| -> f64 {
        let m = ((t - first) / (k.bpb as f64 * beat_n)).round();
        let d = first + m * k.bpb as f64 * beat_n;
        if (d - t).abs() <= 0.5 * beat_n { d } else { t }
    };
    let drop = (b.drop_ms > 0).then(|| snap(b.drop_ms as f64));
    let intro_end = (b.intro_end_ms as f64 > first + 0.5 * bar_in).then(|| snap(b.intro_end_ms as f64));
    let arrival = drop.or(intro_end);
    let mut landings: Vec<(f64, Landing)> = Vec::new();
    if let Some(d) = drop {
        landings.push((d, Landing::Drop));
    }
    if let Some(d) = intro_end.filter(|i| drop.is_none_or(|d| (d - i).abs() > 0.5 * bar_in)) {
        landings.push((d, Landing::IntroEnd));
    }
    for m in 1..=prof.most_bars[0] / 4 {
        let d = first + (4 * m) as f64 * bar_in;
        if landings.iter().all(|(t, ..)| (t - d).abs() > 0.5 * bar_in) {
            landings.push((d, Landing::Phrase));
        }
    }
    landings.push((first, Landing::Start));
    // Where the outgoing singing ends and the incoming starts; a skip of singing costs the sung rate.
    let vocal_end = (a.vocal_end_ms > 0).then_some(a.vocal_end_ms as f64);
    let vocal_start = (b.vocal_end_ms > 0).then_some(b.vocal_start_ms as f64);
    // An instrumental opening to come in on.
    let opening = vocal_start.is_some_and(|v| v > first + 0.5 * beat_n);
    let unsung_from = |t: f64| vocal_end.is_none_or(|v| t >= v - 0.5 * beat);

    let mut best: Option<Window> = None;
    for &(d, landing) in &landings {
        let runup_sung = sings_before(b, d - 0.5 * beat_n);
        for &(at, back, double) in &anchors {
            if double && landing != Landing::Drop {
                continue;
            }
            let on_phrase = a.outro_start_ms > 0 && at >= a.outro_start_ms as f64 - 0.5 * beat && on_line(at, a.outro_start_ms as f64);
            let after_line = vocal_end.is_some_and(|v| (downbeat_at(v) - at).abs() <= 0.5 * beat);
            for &runup in prof.runups {
                // Only the song's first bar comes in without a run-up: anywhere else that skips its opening.
                if (landing == Landing::Start) != (runup == 0) {
                    continue;
                }
                let mut in_start = d - runup as f64 * bar_in;
                let mut in_loop = None;
                if in_start < (first - 0.5 * beat_n).max(0.0) {
                    // Too short an opening: loop its first bars to make up the rest, unless it is sung.
                    if !prof.in_loop || runup_sung {
                        continue;
                    }
                    let lead = ((d - first) / bar_in).round() as i64;
                    let steady = |l: i64| b.steady_intro_ms == 0 || first + l as f64 * bar_in <= b.steady_intro_ms as f64 + 0.5 * beat_n;
                    let fits = |l: i64| l <= lead && runup > lead && (runup - lead) % l == 0 && runup - lead <= MAX_IN_LOOPED_BARS && steady(l);
                    match [8i64, 4].into_iter().find(|&l| fits(l)) {
                        Some(l) => (in_start, in_loop) = (first, Some((l, (runup - lead) / l))),
                        None => continue,
                    }
                }
                // Where the incoming song's `x` is heard, from the start of the mix.
                let (loop_song, loop_wall) = in_loop.map_or((0.0, 0.0), |(l, n)| (l as f64 * bar_in, (l * n) as f64 * bar));
                let heard_at = |x: f64| (x - in_start) / ratio + if x >= in_start + loop_song { loop_wall } else { 0.0 };
                let skip_in = (in_start - b.silence_start_ms as f64).max(0.0);
                if skip_in > MAX_SKIP_MS as f64 {
                    continue;
                }
                // The outgoing song under the run-up: its own bars, or the four or eight before its end or exit
                // looped when they are not sung (a looped sung line sounds like a broken record): when its own bars are too
                // few, or would lay the run-up over its singing.
                let plain = at - runup as f64 * bar;
                let at_exit = exit < ending.music_end && (at - exit).abs() <= 0.5 * beat;
                let at_end = back == 0 || at_exit;
                // A loop's bars keep their level, or it jumps each time round: only bars of the steady run before it.
                let steady_from = if at_exit { a.steady_exit_ms } else { a.steady_end_ms } as f64 - 0.5 * beat;
                let fits = plain >= a.silence_start_ms as f64;
                let looped = [8i64, 4]
                    .into_iter()
                    .map(|lb| at - lb as f64 * bar)
                    .find(|&from| at_end && from > plain && from >= a.silence_start_ms as f64 && from >= steady_from && unsung_from(from))
                    .map(|from| (from, at - from));
                let over_singing = vocal_end.is_some_and(|v| plain < v) && prof.sung_under_cost > 0.0;
                let outs = [fits.then_some((plain, 0.0)), looped.filter(|_| !fits || over_singing)];
                for (start, out_loop) in outs.into_iter().flatten() {
                // A run-up without chords cannot clash: the pair's length cap then applies only after the swap.
                let neutral = landing == Landing::Drop && b.drop_runup_tonal_db <= -CHORDLESS_DB && !runup_sung;
                    for tail in [beat, bar, 2.0 * bar, 4.0 * bar] {
                        // A beat only where the song's music ends within the bar anyway, never under a loop.
                        if prof.bar_tail && tail < bar && (out_loop > 0.0 || at + bar <= ending.music_end) {
                            continue;
                        }
                        let dur = runup as f64 * bar + tail;
                        let bars = dur / bar;
                        let held = k.held_bars(dur);
                        let capped = if neutral { tail / bar } else { held };
                        if bars < 2.0 - 1e-6 || capped > k.max_bars as f64 + 0.3 || held > prof.most_bars[0] as f64 + 0.3 || dur > max_len as f64 {
                            continue;
                        }
                        // A glide starts once the incoming loop has played.
                        if bars - (in_loop.map_or(0, |(l, n)| l * (n + 1)) as f64) < k.glide_bars - 1e-6 {
                            continue;
                        }
                        let heard_end = if out_loop > 0.0 { at } else { at + tail };
                        let skip_out = ending.skipped(heard_end);
                        let cap = if double { DOUBLE_DROP_MAX_SKIP_MS } else { MAX_SKIP_MS };
                        if heard_end > out_dur as f64 || skip_out > cap as f64 {
                            continue;
                        }
                        if (in_dur as f64 - in_start) < 2.0 * dur * ratio {
                            continue;
                        }
                        // Where an end's grid holds over a run of it only, the mix plays inside the run.
                        let in_end = in_start + (dur - loop_wall) * ratio;
                        if !in_grid((a.outro_grid_from_ms, a.outro_grid_to_ms), start, heard_end, beat) || !in_grid((b.intro_grid_from_ms, b.intro_grid_to_ms), in_start, in_end, beat_n) {
                            continue;
                        }
                        let sung_skip_in = vocal_start.map_or(0.0, |v| (in_start - v.max(b.silence_start_ms as f64)).max(0.0));
                        let mut score = landing.worth() - (SKIP_COST * (skip_out + skip_in) + (SKIP_SUNG_COST - SKIP_COST) * sung_skip_in) / 1000.0;
                        if let Some(arr) = arrival {
                            let lands = heard_at(arr);
                            if lands > dur {
                                score -= DIP_COST * (lands - dur) / 1000.0;
                            } else if lands < -0.5 * beat {
                                score -= DROP_SKIP_COST;
                            }
                        }
                        score += RUNUP_WORTH * runup.min(prof.runup_worth_bars) as f64 + LENGTH_WORTH * bars;
                        if runup > 0 && runup % prof.phrase == 0 {
                            score += PHRASE_START_WORTH;
                        }
                        if let Some((l, n)) = in_loop {
                            score -= IN_LOOP_COST * (l * n) as f64;
                        }
                        if double {
                            score += DOUBLE_DROP_WORTH;
                        }
                        if on_phrase || after_line {
                            score += PHRASE_WORTH;
                        }
                        // Both songs singing at once, heard from the start of the mix: the outgoing song until its
                        // singing ends (none under a loop of unsung bars), the incoming from where it starts. The
                        // voice filters halve the cost.
                        let out_sung = vocal_end.filter(|_| out_loop == 0.0).map_or(0.0, |v| (v - start).clamp(0.0, dur));
                        let in_sung = vocal_start.map_or(dur, |v| heard_at(v).clamp(0.0, dur));
                        let both_sung = (out_sung - in_sung).max(0.0);
                        score -= if k.separate { 0.5 } else { 1.0 } * BOTH_SUNG_COST * both_sung / 1000.0;
                        // The incoming song comes in on its instrumental opening, when it has one: not already
                        // singing, and its voice, like the outgoing one, not under the run-up.
                        if opening {
                            if vocal_start.is_some_and(|v| in_start > v - 0.5 * beat_n) {
                                score -= ENTER_SUNG_COST;
                            }
                            score -= prof.sung_under_cost * (runup as f64 * bar - in_sung).max(0.0) / 1000.0;
                        }
                        if out_loop > 0.0 {
                            score -= prof.loop_cost * (dur - out_loop) / bar;
                        }
                        // The outgoing singing: under the run-up, faded after the swap, and skipped (over the plain rate).
                        if let Some(v) = vocal_end.filter(|_| out_loop == 0.0) {
                            let swap_at = start + runup as f64 * bar;
                            score -= prof.sung_under_cost * (v.min(swap_at) - start).max(0.0) / 1000.0;
                            score -= SUNG_FADE_COST * (v.min(heard_end) - swap_at).max(0.0) / 1000.0;
                            score -= (SKIP_SUNG_COST - SKIP_COST) * (v - heard_end).max(0.0) / 1000.0;
                        }
                        // Outgoing silence and closing breakdown under the run-up or before the mix.
                        let dead = ending.silence(start, at) + ending.silence(0.0, start).min(ending.gap.map_or(0.0, |(g0, g1)| g1 - g0));
                        score -= DEAD_AIR_COST * dead / 1000.0 + DIP_COST * (ending.coda(start, at) + ending.coda(0.0, start)) / 1000.0;
                        if exit < ending.music_end && (at - exit).abs() <= 0.5 * beat {
                            score += EXIT_WORTH;
                        }
                        if best.is_none_or(|w| score > w.score + 1e-9) {
                            best = Some(Window {
                                start,
                                dur,
                                swap: runup as f64 * bar,
                                in_start,
                                runup_bars: runup,
                                landing,
                                on_phrase,
                                after_line,
                                out_loop,
                                in_loop,
                                double,
                                runup_sung,
                                after_sung: vocal_start.is_some_and(|v| heard_at(v) < dur),
                                both_sung,
                                score,
                            });
                        }
                    }
                }
            }
        }
    }
    best
}

fn beat_matched(a: &TrackAnalysis, b: &TrackAnalysis, room: Room, s: &AutoMixSettings, short_cause: &str) -> Result<TransitionPlan, String> {
    let out_dur = room.out_dur;
    let bpm_a = bpm_with_tag(a.bpm, s.out_tag_bpm);
    let bpm_b = bpm_with_tag(b.bpm, s.in_tag_bpm);
    let mut ratio = match_ratio(bpm_a, bpm_b);
    let pct = (ratio - 1.0).abs() * 100.0;
    let mut max_pct = (s.max_tempo_change_pct as f64).clamp(0.0, 12.0);
    if !s.keep_pitch {
        max_pct = max_pct.min(VARISPEED_MAX_PCT);
    }
    let prof = profile(s.style);
    // Past the limit, a DJ mix glides: one tempo for both decks, from the outgoing song's to the incoming one's
    // across the whole mix, so each is about half the gap from its own as they cross.
    let glide = pct > max_pct + 1e-9 && prof.bend && s.keep_pitch && pct <= (2.0 * max_pct).min(GLIDE_MAX_PCT) + 1e-9;
    if pct > max_pct + 1e-9 && !glide {
        return Err(format!("tempo gap {pct:.1} % over the {max_pct:.1} % limit"));
    }
    if (ratio - 1.0).abs() < 0.0005 {
        ratio = 1.0;
    }
    let bpb = bar_beats(a);
    if bar_beats(b) != bpb {
        return Err(format!("metres differ ({}/4 against {}/4)", bpb, bar_beats(b)));
    }
    let beat_a = 60_000.0 / bpm_a;
    let bar = bpb as f64 * beat_a;
    let b_bpm = fold(bpm_a, bpm_b);
    // By Camelot distance: the profile's longest for 0-1, shorter for 2 or a pair gate, shorter again for 3, a
    // quick swap for keys far apart.
    let dist = camelot_dist(a, b);
    let mild_clash = dist.is_some_and(|d| d > 2);
    let far = dist.is_some_and(|d| d >= KEY_FAR);
    let max_bars = if far {
        FAR_BARS
    } else if mild_clash || short_cause == VOCALS_OVERLAP {
        prof.most_bars[2]
    } else if !short_cause.is_empty() || dist == Some(2) {
        prof.most_bars[1]
    } else {
        prof.most_bars[0]
    };
    let glide_bars = if glide { (pct / GLIDE_PCT_PER_BAR).ceil() } else { 0.0 };
    let k = Lock { ratio, pct, bpm_a, b_bpm, bpb, beat: beat_a, bar, dist, mild_clash, far, max_bars, glide_bars, separate: s.filter_effects, profile: prof };
    let w = drop_aligned(a, b, room, bpm_b, &k).ok_or("no bar-aligned window fits")?;
    // A glide is long, so only a pair that blends glides: keys near, never two voices at once.
    if glide && (mild_clash || w.both_sung > 0.0) {
        return Err(format!("tempo gap {pct:.1} % over the {max_pct:.1} % limit, and no clean glide"));
    }
    let ending = Ending::of(a, out_dur);
    let left = ending.exit < ending.music_end && (w.start + w.swap - ending.exit).abs() <= 0.5 * k.beat;
    let leaving = match (left, ending.breakdown) {
        (true, true) => "leaving on the closing breakdown",
        (true, false) => "leaving before the hidden track",
        _ => "",
    };
    let neutral = k.held_bars(w.dur) > k.max_bars as f64 + 0.3;
    let bits = [
        w.landing.words(),
        if w.double { "a double drop" } else { "" },
        if w.on_phrase { "on the outro phrase" } else if w.after_line { "after its last line" } else { "" },
        leaving,
        if w.out_loop > 0.0 { "intro/outro remix" } else { "" },
        if neutral { "run-up without chords" } else { "" },
    ];
    Ok(finish_beat_matched(a, b, s, &k, &w, short_cause, &bits))
}

/// How long the bass takes to change hands: a sixteenth as the drop hits, which brings its own; elsewhere eased
/// over the profile's bars.
fn swap_len(k: &Lock, landing: Landing) -> f64 {
    if k.profile.quick_drop_swap && landing == Landing::Drop {
        (k.beat / 4.0).clamp(40.0, 150.0)
    } else {
        k.profile.swap_bars * k.bar
    }
}

fn finish_beat_matched(a: &TrackAnalysis, b: &TrackAnalysis, s: &AutoMixSettings, k: &Lock, w: &Window, short_cause: &str, notes: &[&str]) -> TransitionPlan {
    let dur_ms = w.dur.round() as i64;
    let swap = (w.swap.round() as i64).clamp(0, dur_ms);
    let beat_ms = k.beat.round() as i64;
    let mut p = blank(TransitionKind::BeatMatched, w.start.round() as i64, w.in_start.round().max(0.0) as i64, dur_ms, String::new());
    p.tempo_ratio = k.ratio;
    p.keep_pitch = s.keep_pitch;
    if k.ratio != 1.0 {
        p.tempo_ramp_beats = if k.pct <= 2.0 { 16 } else { 32 };
        p.tempo_ramp_ms = (p.tempo_ramp_beats as f64 * (60_000.0 / k.b_bpm) / ((1.0 + k.ratio) / 2.0)).round() as i64;
    }
    p.fade_curve = FadeCurve::SineSquared;
    // Incoming rises over the run-up (over its first bar without one); outgoing falls over the tail after the swap.
    (p.in_fade_start_ms, p.in_fade_end_ms) = (0, if swap > 0 { swap } else { (k.bar.round() as i64).min(dur_ms / 2).max(1) });
    (p.out_fade_start_ms, p.out_fade_end_ms) = (swap.min(dur_ms - 1).max(0), dur_ms);
    if s.bass_swap {
        let len = swap_len(k, w.landing).round() as i64;
        p.bass_swap = Some(BassSwap { at_ms: (swap - len).max(0), len_ms: len, cut_hz: BASS_CUT_HZ });
        if k.profile.highs_in && w.runup_bars >= HIGHS_IN_MIN_BARS {
            let back = (w.swap - (w.runup_bars as f64 / 4.0 + 1.0) * k.bar).max(0.0);
            p.highs_in = Some(HighsIn { at_ms: back.round() as i64, len_ms: k.bar.round() as i64, db: HIGHS_IN_DB, hz: HIGHS_IN_HZ });
        }
    }
    // Same key: no filter. Neighbours: soft low-pass. Distance 2: high-pass "filter open". Else: low-pass after
    // the swap.
    if s.filter_effects {
        match k.dist {
            Some(0) => {}
            Some(1) => {
                p.low_pass = Some(low_pass(((swap + dur_ms) / 2).max(swap), dur_ms, SWEEP_TO_HZ_SOFT));
            }
            Some(2) => {
                p.high_pass = Some(Sweep { start_ms: 0, end_ms: swap.max(beat_ms * 4).min(dur_ms), from_hz: 40.0, to_hz: 1_200.0 });
            }
            _ => {
                p.low_pass = Some(low_pass(swap, dur_ms, SWEEP_TO_HZ_MATCHED));
            }
        }
    }
    // Two singers: duck the incoming voice band until the swap, then thin the outgoing voice with a high-pass.
    let mut apart = false;
    if k.separate {
        if swap > 0 && w.runup_sung && sings_after(a, w.start) {
            p.vocal_duck = Some(VocalDuck { until_ms: swap, release_ms: beat_ms.min(swap), db: VOCAL_DUCK_DB, hz: VOCAL_DUCK_HZ });
            apart = true;
        }
        if w.after_sung && sings_after(a, w.start + w.swap) && dur_ms - swap >= beat_ms {
            p.high_pass = Some(Sweep { start_ms: swap, end_ms: swap + (dur_ms - swap) / 2, from_hz: VOCAL_HP_FROM_HZ, to_hz: VOCAL_HP_TO_HZ });
            apart = true;
        }
    }
    // The build into the drop: the outgoing song thins out over the bars before it.
    let riser = s.filter_effects && k.profile.riser && p.high_pass.is_none() && matches!(w.landing, Landing::Drop | Landing::IntroEnd) && w.runup_bars >= 2 * RISER_BARS;
    if riser {
        let from = swap - (RISER_BARS as f64 * k.bar).round() as i64;
        p.high_pass = Some(Sweep { start_ms: from.max(0), end_ms: swap, from_hz: RISER_FROM_HZ, to_hz: RISER_TO_HZ });
    }
    p.out_loop_ms = (w.out_loop > 0.0).then(|| w.out_loop.round() as i64);
    p.in_loop = w.in_loop.map(|(l, n)| InLoop { ms: (l as f64 * k.bar).round() as i64, repeats: n as i32 });
    p.in_gain_db = loudness_trim(Some(a), Some(b), s);
    let loop_end = w.in_loop.map_or(0.0, |(l, n)| (l * (n + 1)) as f64 * k.bar);
    if k.profile.bend {
        bend(&mut p, k, loop_end);
    }
    let mut bits: Vec<String> = notes.iter().filter(|n| !n.is_empty()).map(|n| n.to_string()).collect();
    if apart {
        bits.push("voices kept apart".to_string());
    }
    if riser {
        bits.push("a riser into it".to_string());
    }
    if p.highs_in.is_some() {
        bits.push("highs before the bass".to_string());
    }
    if w.runup_bars > 0 && p.out_loop_ms.is_none() {
        bits.push(format!("{}-bar run-up", w.runup_bars));
    }
    if let Some((l, n)) = w.in_loop {
        bits.push(format!("its first {l} bars looped {n}x"));
    }
    if let Some(b) = p.bend {
        let over = if k.glide_bars > 0.0 { "gliding across the gap" } else { "tempo bending" };
        bits.push(format!("{over} over the last {:.0} bars", k.profile_bars(b)));
    }
    if k.far {
        bits.push(format!("{KEYS_FAR}: quick"));
    } else if k.mild_clash {
        bits.push("keys clash: short".to_string());
    } else if k.dist == Some(2) {
        bits.push("keys stretch: filter-open".to_string());
    } else if !short_cause.is_empty() {
        bits.push(format!("short ({short_cause})"));
    } else if k.dist == Some(0) {
        bits.push("same key".to_string());
    } else if k.dist == Some(1) {
        bits.push("harmonic".to_string());
    }
    let bars = w.dur / k.bar;
    let bars = if (bars - bars.round()).abs() < 0.01 { format!("{:.0}", bars) } else { format!("{bars:.2}") };
    p.reason = format!(
        "beat-matched {bars} bars, {:.1} -> {:.1} BPM ({:+.1} %), {}",
        k.b_bpm,
        k.bpm_a,
        (k.ratio - 1.0) * 100.0,
        bits.join(", ")
    );
    p
}

/// The incoming highs over a run-up of this many bars or more: held this far down above this shelf, dB and Hz, back
/// over the bar ending a quarter of the run-up before the swap.
const HIGHS_IN_MIN_BARS: i64 = 4;
const HIGHS_IN_DB: f32 = -15.0;
const HIGHS_IN_HZ: f32 = 3_000.0;

/// Bars of rising high-pass on the outgoing song before the incoming drop, and its sweep, Hz.
const RISER_BARS: i64 = 2;
const RISER_FROM_HZ: f32 = 40.0;
const RISER_TO_HZ: f32 = 900.0;

/// Bars of tempo glide at the end of a DJ mix, at most and at least.
const BEND_BARS: f64 = 8.0;
/// A tempo gap past the limit glides at most this much a bar, percent, and is never past this gap.
const GLIDE_PCT_PER_BAR: f64 = 2.0;
const GLIDE_MAX_PCT: f64 = 20.0;
const BEND_MIN_BARS: f64 = 2.0;
/// Tempo gaps under this are left to the ramp after the mix, percent.
const BEND_MIN_PCT: f64 = 0.5;

impl Lock {
    /// A mix's bars as its length caps count them: with [`Profile::bar_tail`], the fade's first bar as a beat.
    fn held_bars(&self, dur: f64) -> f64 {
        let slack = if self.profile.bar_tail { self.bar - self.beat } else { 0.0 };
        (dur - slack) / self.bar
    }

    /// A bend's length in outgoing bars, as it reads in the outgoing song.
    fn profile_bars(&self, b: Bend) -> f64 {
        let r = self.ratio;
        (b.end_ms - b.start_ms) as f64 * (r + 1.0) / (2.0 * r) / self.bar
    }
}

/// Moves the tempo change into the mix: over its last bars, or all of them after the incoming loop (from `from` ms)
/// for a glide, both decks glide to the incoming tempo, so the incoming song is at its own once the mix is over. The plan's times, laid out
/// in the outgoing song's time, become wall-clock: from the bend's start the outgoing song plays at a changing
/// speed, so each later time is where that song time is reached.
fn bend(p: &mut TransitionPlan, k: &Lock, from: f64) {
    let dur = p.duration_ms as f64;
    let span = if k.glide_bars > 0.0 { dur - from } else { (BEND_BARS * k.bar).min(dur - from) };
    if k.ratio == 1.0 || k.pct < BEND_MIN_PCT || span < BEND_MIN_BARS * k.bar - 1.0 {
        return;
    }
    let (r, start) = (k.ratio, dur - span);
    // The outgoing speed goes 1 -> 1/r linearly over `len` wall ms, playing `span` of its song.
    let len = span * 2.0 * r / (r + 1.0);
    let c = 1.0 / r - 1.0;
    let wall = |song: f64| -> f64 {
        if song <= start {
            return song;
        }
        let x = (song - start).min(span);
        start + if c.abs() < 1e-12 { x } else { ((1.0 + 2.0 * c * x / len).sqrt() - 1.0) * len / c }
    };
    let at = |ms: i64| wall(ms as f64).round() as i64;
    for ms in [&mut p.out_fade_start_ms, &mut p.out_fade_end_ms, &mut p.in_fade_start_ms, &mut p.in_fade_end_ms] {
        *ms = at(*ms);
    }
    if let Some(sw) = p.bass_swap.as_mut() {
        let end = at(sw.at_ms + sw.len_ms);
        sw.at_ms = at(sw.at_ms);
        sw.len_ms = end - sw.at_ms;
    }
    if let Some(h) = p.highs_in.as_mut() {
        let end = at(h.at_ms + h.len_ms);
        h.at_ms = at(h.at_ms);
        h.len_ms = end - h.at_ms;
    }
    for sw in [p.low_pass.as_mut(), p.high_pass.as_mut()].into_iter().flatten() {
        (sw.start_ms, sw.end_ms) = (at(sw.start_ms), at(sw.end_ms));
    }
    if let Some(d) = p.vocal_duck.as_mut() {
        let until = at(d.until_ms);
        d.release_ms = until - at(d.until_ms - d.release_ms);
        d.until_ms = until;
    }
    p.duration_ms = (start + len).round() as i64;
    p.bend = Some(Bend { start_ms: start.round() as i64, end_ms: p.duration_ms });
    (p.tempo_ramp_beats, p.tempo_ramp_ms) = (0, 0);
}

/// Bars the outgoing echo rings on after the exit.
const ECHO_TAIL_BARS: f64 = 2.0;
/// Bars back from the end an echo-out may leave on.
const ECHO_BARS_BACK: i64 = 8;
const ECHO_FEEDBACK: f32 = 0.55;
const ECHO_WET_DB: f32 = -4.0;
/// The high-pass the echoes wash out through, Hz.
const ECHO_HP_FROM_HZ: f32 = 60.0;
const ECHO_HP_TO_HZ: f32 = 1_500.0;

/// An echo-out, for a pair that cannot be beat-matched: on a downbeat after the outgoing song's last line, the
/// song falls (over a beat on a trusted grid, a bar on its tempo alone) as its last beat rings on in repeats that
/// a rising high-pass thins out, and the incoming music starts there, rising over a bar. Each song keeps its own
/// tempo. Never over a sung beat, which would stutter.
fn echo_out(a: &TrackAnalysis, b: Option<&TrackAnalysis>, room: Room, s: &AutoMixSettings, why_not: &str) -> Option<TransitionPlan> {
    let Room { out_dur, in_dur, max_len } = room;
    let beat = 60_000.0 / a.bpm;
    if !(a.bpm > 0.0 && beat.is_finite()) {
        return None;
    }
    let (bpb, on_grid) = (bar_beats(a), grid_ok(a));
    let bar = bpb as f64 * beat;
    let fall = if on_grid { beat } else { bar };
    let dur = beat + ECHO_TAIL_BARS * bar;
    if dur > max_len as f64 {
        return None;
    }
    // The exit: the latest downbeat before the music ends (a closing breakdown is music to echo out of; a hidden
    // track is not) or turns quiet, or a four-bar line of the outro within four bars of it, whose beat before is
    // unsung.
    let ending = Ending::of(a, out_dur);
    let end = if ending.breakdown { ending.music_end } else { ending.leave(0.0) };
    let quiet = (a.mixramp_end_ms > a.silence_start_ms).then(|| a.mixramp_end_ms as f64 + beat);
    let limit = quiet.map_or(end, |q| q.min(end));
    let last = a.beat_offset_ms + downbeat_before(limit, a.beat_offset_ms, beat, bpb, a.downbeat_phase as i64) as f64 * beat;
    let fits = |x: f64| x - beat >= a.silence_start_ms as f64 && !sings_after(a, x - beat) && ending.skipped(x + fall) <= MAX_SKIP_MS as f64;
    let on_line = |x: f64| a.outro_start_ms > 0 && {
        let lines = (x - a.outro_start_ms as f64) / (4.0 * bar);
        lines >= -0.01 && (lines - lines.round()).abs() < 0.01
    };
    let exits: Vec<f64> = (0..=ECHO_BARS_BACK).map(|n| last - n as f64 * bar).filter(|&x| fits(x)).collect();
    let exit = exits.iter().take(4).copied().find(|&x| on_line(x)).or(exits.first().copied())?;
    // The incoming music starts on the exit (its first downbeat, after a pickup of a beat at most), or as close
    // before it as its own start allows: none of it is skipped.
    let music = b.map_or(0, |b| b.silence_start_ms.clamp(0, in_dur / 3)) as f64;
    let first = |b: &TrackAnalysis| {
        let beat_b = 60_000.0 / b.bpm;
        let d = b.beat_offset_ms + downbeat_from(music, b.beat_offset_ms, beat_b, bar_beats(b), b.downbeat_phase as i64) as f64 * beat_b;
        (d - music <= beat_b).then_some(d)
    };
    let entry = b.filter(|b| grid_ok(b)).and_then(first).unwrap_or(music);
    let in_start = (entry - beat).max(0.0).min(music).round();
    if in_dur as f64 - in_start < 2.0 * dur {
        return None;
    }
    let mut p = blank(TransitionKind::EchoOut, (exit - beat).round() as i64, in_start as i64, dur.round() as i64, String::new());
    (p.out_fade_start_ms, p.out_fade_end_ms) = (beat.round() as i64, (beat + fall).round() as i64);
    (p.in_fade_start_ms, p.in_fade_end_ms) = ((music - in_start).round() as i64, (beat + bar).round() as i64);
    // A beat delay, doubled or halved into a quarter to a whole second.
    let mut delay = beat;
    while delay < 250.0 {
        delay *= 2.0;
    }
    while delay > 1000.0 {
        delay /= 2.0;
    }
    p.echo = Some(Echo { delay_ms: delay.round() as i64, feedback: ECHO_FEEDBACK, wet_db: ECHO_WET_DB, send: None, band: None, duck: None });
    if s.filter_effects {
        p.high_pass = Some(Sweep { start_ms: 0, end_ms: p.duration_ms, from_hz: ECHO_HP_FROM_HZ, to_hz: ECHO_HP_TO_HZ });
    }
    p.in_gain_db = loudness_trim(Some(a), b, s);
    let tail = if why_not.is_empty() { String::new() } else { format!(" ({why_not})") };
    let at = if on_line(exit) { "on the outro phrase" } else if a.vocal_end_ms > 0 { "after its last line" } else { "on its last bars" };
    let by = if on_grid { "on its downbeat" } else { "by its tempo" };
    p.reason = format!("echo-out {by}, {at}{tail}");
    Some(p)
}

/// A fade aligned to the one usable grid when the two could not be locked: an exit on outgoing downbeats, or an
/// entrance on an incoming downbeat. No tempo change.
fn one_grid(a: Option<&TrackAnalysis>, b: Option<&TrackAnalysis>, room: Room, s: &AutoMixSettings, short: bool, why_not: &str) -> Option<TransitionPlan> {
    let Room { out_dur, in_dur, max_len } = room;
    let tail = if why_not.is_empty() { String::new() } else { format!(" ({why_not})") };
    let ending = a.map(|x| Ending::of(x, out_dur));
    let end_a = ending.map_or(out_dur, |e| e.leave(0.0).round() as i64);
    let skipped = |heard_end: i64| ending.map_or((out_dur - heard_end) as f64, |e| e.skipped(heard_end as f64));
    // 8 bars, else 4 (only 4 when the pair gates shorten it): the one fewer voices sing over the other in first.
    let out_bars: &[i64] = if short { &[4] } else { &[8, 4] };
    let fade = |start: i64, in_start: i64, dur: i64, reason: String| {
        let mut p = blank(TransitionKind::MixRampFade, start, in_start, dur, String::new());
        apply_fade_filter(&mut p, a, b, s, dur);
        p.in_gain_db = loudness_trim(a, b, s);
        p.reason = reason;
        p
    };
    let quietest = |fades: Vec<TransitionPlan>| fades.into_iter().min_by_key(|p| over_voices(a, b, p));
    if let Some(g) = a.filter(|x| grid_ok(x)) {
        let beat = 60_000.0 / g.bpm;
        let bpb = bar_beats(g);
        let in_start = b.map_or(0, |x| x.silence_start_ms.clamp(0, in_dur / 3));
        let downbeat = |t: f64| (g.beat_offset_ms + downbeat_before(t, g.beat_offset_ms, beat, bpb, g.downbeat_phase as i64) as f64 * beat).round() as i64;
        // At the end, or on the bar the outgoing singing ends on, over the instrumental bars after it (a closing
        // breakdown's too).
        let after_line = (g.vocal_end_ms > 0).then(|| downbeat(g.vocal_end_ms as f64 + 0.5 * beat));
        let through = ending.map_or(out_dur, |e| if e.breakdown { e.music_end } else { e.exit }.round() as i64);
        let mut fades = Vec::new();
        for bars in out_bars {
            let dur = (*bars as f64 * bpb as f64 * beat).round() as i64;
            if dur > max_len || dur < MIN_FADE_MS {
                continue;
            }
            for (start, until, at) in [(downbeat(end_a as f64 - dur as f64), end_a, ""), (after_line.unwrap_or(-1), through, ", after its last line")] {
                if start < g.silence_start_ms || start + dur > until || skipped(start + dur) > MAX_SKIP_MS as f64 {
                    continue;
                }
                fades.push(fade(start, in_start, dur, format!("downbeat-aligned fade, {bars} bars out{at}{tail}")));
            }
        }
        if let Some(p) = quietest(fades) {
            return Some(p);
        }
    }
    // Else enter on the incoming song's first downbeat.
    if let Some(g) = b.filter(|x| grid_ok(x)) {
        let beat = 60_000.0 / g.bpm;
        let bpb = bar_beats(g);
        let nb = downbeat_from(g.silence_start_ms as f64, g.beat_offset_ms, beat, bpb, g.downbeat_phase as i64);
        let in_start = (g.beat_offset_ms + nb as f64 * beat).round().max(0.0) as i64;
        if in_start - g.silence_start_ms > MAX_SKIP_MS {
            return None;
        }
        let mut fades = Vec::new();
        for bars in out_bars {
            let dur = (*bars as f64 * bpb as f64 * beat).round() as i64;
            if dur > max_len || dur < MIN_FADE_MS || dur > in_dur - in_start {
                continue;
            }
            let start = end_a - dur;
            if start < 0 || skipped(start + dur) > MAX_SKIP_MS as f64 {
                continue;
            }
            fades.push(fade(start, in_start, dur, format!("downbeat-aligned fade, {bars} bars in{tail}")));
        }
        return quietest(fades);
    }
    None
}

/// How many of a fade's songs sing over the other (the outgoing one into the fade's second half, the incoming one
/// before it ends), and for how long: a voice that cannot be kept apart is kept short.
fn over_voices(a: Option<&TrackAnalysis>, b: Option<&TrackAnalysis>, p: &TransitionPlan) -> (u8, i64) {
    let out = a.is_some_and(|a| sings_after(a, (p.out_start_ms + p.duration_ms / 2) as f64));
    let inc = b.is_some_and(|b| sings_before(b, (p.in_start_ms + p.duration_ms) as f64));
    let n = out as u8 + inc as u8;
    (n, if n > 0 { p.duration_ms } else { 0 })
}

fn mixramp(a: Option<&TrackAnalysis>, b: Option<&TrackAnalysis>, room: Room, s: &AutoMixSettings, why_not: &str) -> TransitionPlan {
    let Room { out_dur, in_dur, max_len } = room;
    let end_a = a.map_or(out_dur, |a| Ending::of(a, out_dur).leave(0.0).round() as i64);
    let in_start = b.map_or(0, |b| b.silence_start_ms.clamp(0, in_dur / 3));
    // The outgoing quiet tail and the incoming quiet head (an exit before the end has no quiet tail).
    let tail = a.map(|a| if a.mixramp_end_ms > 0 && a.mixramp_end_ms <= end_a { end_a - a.mixramp_end_ms } else { 0 });
    let head = b.map(|b| if b.mixramp_start_ms > 0 { (b.mixramp_start_ms - in_start).max(0) } else { 0 });
    let mut dur = tail.unwrap_or(0) + head.unwrap_or(0);
    if tail.is_none() || head.is_none() {
        dur = dur.max(max_len.min(4000));
    }
    let dur = dur.max(MIN_MIXRAMP_MS).clamp(MIN_FADE_MS.min(max_len), max_len);
    let mut p = blank(TransitionKind::MixRampFade, end_a - dur, in_start, dur, String::new());
    // A quiet start rides its own ramp; a loud one comes in at once only over the outgoing song's quiet tail,
    // and over a loud ending fades in as that one fades out.
    p.in_fade_end_ms = match (head, tail) {
        (Some(h), _) if h > 0 => h.clamp(MIN_FADE_MS.min(dur), dur),
        (Some(_), Some(t)) if t > 0 => MIN_FADE_MS.min(dur),
        _ => dur,
    };
    apply_fade_filter(&mut p, a, b, s, dur);
    p.in_gain_db = loudness_trim(a, b, s);
    p.reason = format!("mixramp fade {:.1} s{}", dur as f64 / 1000.0, if why_not.is_empty() { String::new() } else { format!(" ({why_not})") });
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automix::structure::camelot;
    use crate::types::MixStyle;

    /// A 4-minute track, 4/4, grid from 120 ms, an 8-bar intro, a 16-bar outro and silence at both ends.
    fn track(bpm: f64) -> TrackAnalysis {
        let beat = 60_000.0 / bpm;
        TrackAnalysis {
            song_id: "t".into(),
            analysis_version: 1,
            duration_ms: 240_000,
            bpm,
            bpm_confidence: 0.9,
            beat_offset_ms: 120.0,
            stability: 0.9,
            downbeat_phase: 0,
            downbeat_confidence: 0.8,
            beats_per_bar: 4,
            lufs: -9.0,
            key: camelot(0, false),
            key_confidence: 0.8,
            silence_start_ms: 100,
            silence_end_ms: 238_500,
            mixramp_start_ms: 400,
            mixramp_end_ms: 236_000,
            intro_end_ms: (120.0 + 32.0 * beat) as i64,
            outro_start_ms: (120.0 + beat * 4.0 * (((238_500.0 - 120.0) / (4.0 * beat)).floor() - 16.0)) as i64,
            outro_centroid: 1200.0,
            intro_centroid: 1200.0,
            analysed_ms: 0,
            outro_bpm: bpm,
            outro_bpm_confidence: 0.9,
            outro_beat_offset_ms: 120.0,
            outro_stability: 0.9,
            outro_downbeat_phase: 0,
            intro_bpm: bpm,
            intro_bpm_confidence: 0.9,
            intro_beat_offset_ms: 120.0,
            intro_stability: 0.9,
            intro_downbeat_phase: 0,
            drop_ms: 0,
            drop_runup_tonal_db: 0.0,
            exit_ms: 0,
            gap_ms: 0,
            gap_end_ms: 0,
            intro_beats_per_bar: 0,
            outro_beats_per_bar: 0,
            intro_grid_source: 0,
            outro_grid_source: 0,
            last_drop_ms: 0,
            vocal_start_ms: 0,
            vocal_end_ms: 0,
            steady_intro_ms: 0,
            steady_end_ms: 0,
            steady_exit_ms: 0,
            intro_grid_from_ms: 0,
            intro_grid_to_ms: 0,
            outro_grid_from_ms: 0,
            outro_grid_to_ms: 0,
            intro_bars: Vec::new(),
            outro_bars: Vec::new(),
        }
    }

    /// At most `MAX_SKIP_MS` of either song's music goes unplayed (of the outgoing one, `DOUBLE_DROP_MAX_SKIP_MS`
    /// for a double drop).
    fn check_skip(p: &TransitionPlan, a: &TrackAnalysis, b: &TrackAnalysis, out_dur: i64) {
        assert!(p.in_start_ms - b.silence_start_ms <= MAX_SKIP_MS, "{p:?}");
        let heard_end = p.out_start_ms as f64 + p.out_loop_ms.map_or(p.out_song_ms(p.duration_ms as f64), |l| l as f64);
        let cap = if p.reason.contains("double drop") { DOUBLE_DROP_MAX_SKIP_MS } else { MAX_SKIP_MS };
        assert!(Ending::of(a, out_dur).skipped(heard_end) <= cap as f64, "{p:?}");
        assert!(p.out_start_ms >= 0 && p.duration_ms >= 0 && heard_end <= out_dur as f64 + 1.0, "{p:?}");
    }

    fn dj() -> AutoMixSettings {
        let style = MixStyle::Dj;
        AutoMixSettings { max_transition_s: style.longest_s() as f32, max_tempo_change_pct: style.tempo_pct(), style, ..Default::default() }
    }

    /// The incoming song's `t` reaches the outgoing song's bass swap, and that is on an outgoing downbeat.
    fn lands_on_swap(p: &TransitionPlan, a: &TrackAnalysis, t: i64) {
        let swap = swap_at(p) as f64;
        let heard = p.in_start_ms as f64 + p.in_song_ms(swap);
        assert!((heard - t as f64).abs() < 15.0, "{t} of the incoming song is at the swap {swap}, not {heard}: {}", p.reason);
        assert!(on_downbeat(a, p.out_start_ms as f64 + p.out_song_ms(swap)), "{}", p.reason);
    }

    #[test]
    fn dj_mixes_run_longer_and_loop_a_short_intro() {
        let beat = 60_000.0 / 128.0;
        // The drop eight bars in, the opening drums only.
        let b = TrackAnalysis { drop_ms: (120.0 + 32.0 * beat) as i64, drop_runup_tonal_db: -6.0, ..track(128.0) };
        let a = track(128.0);
        let smooth = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 12.0, ..Default::default() });
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.duration_ms > smooth.duration_ms + 4 * 4 * beat as i64, "DJ {} against Smooth {}", p.reason, smooth.reason);
        let l = p.in_loop.expect(&p.reason);
        assert!(l.repeats >= 1 && (l.ms as f64 / (4.0 * beat)).round() as i64 % 4 == 0, "{l:?}: {}", p.reason);
        assert_eq!(p.in_start_ms, 120, "the loop is the opening from its first downbeat");
        lands_on_swap(&p, &a, b.drop_ms);
        check_skip(&p, &a, &b, 240_000);
        // A sung opening is not looped.
        let sung = TrackAnalysis { vocal_start_ms: 120, vocal_end_ms: 200_000, ..b.clone() };
        let p = plan(Some(&a), Some(&sung), 240_000, 240_000, &dj());
        assert_eq!(p.in_loop, None, "{}", p.reason);
        // Smooth never loops it.
        assert_eq!(smooth.in_loop, None, "{}", smooth.reason);
        // An opening that builds after two bars would drop back each time round: not looped.
        let building = TrackAnalysis { steady_intro_ms: (120.0 + 8.0 * beat) as i64, ..b };
        let p = plan(Some(&a), Some(&building), 240_000, 240_000, &dj());
        assert_eq!(p.in_loop, None, "{}", p.reason);
    }

    /// Into a drop, DJ thins the outgoing song out with a rising high-pass over the two bars before the swap;
    /// Smooth leaves it whole.
    #[test]
    fn dj_builds_a_riser_into_the_drop() {
        let bar = 4.0 * 60_000.0 / 128.0;
        let b = TrackAnalysis { drop_ms: (120.0 + 16.0 * bar) as i64, ..track(128.0) };
        let a = track(128.0);
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        let swap = swap_at(&p);
        let h = p.high_pass.expect(&p.reason);
        assert_eq!((h.end_ms, h.from_hz, h.to_hz), (swap, RISER_FROM_HZ, RISER_TO_HZ), "{}", p.reason);
        let riser = (p.out_song_ms(swap as f64) - p.out_song_ms(h.start_ms as f64)) / bar;
        assert!((riser - RISER_BARS as f64).abs() < 0.01, "{riser} bars of riser: {}", p.reason);
        lands_on_swap(&p, &a, b.drop_ms);
        let smooth = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(smooth.high_pass, None, "{}", smooth.reason);
        let off = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { filter_effects: false, ..dj() });
        assert_eq!(off.high_pass, None, "filters off: {}", off.reason);
    }

    /// A track whose singing ends `bars_left` bars before its music does, with nothing after the music.
    fn sung_until(bars_left: f64) -> (TrackAnalysis, i64) {
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = 120.0 + bar * ((238_500.0 - 120.0) / bar).floor();
        let vocal_end = (end - bars_left * bar) as i64;
        (TrackAnalysis { vocal_start_ms: 10_000, vocal_end_ms: vocal_end, ..track(128.0) }, vocal_end)
    }

    #[test]
    fn mixes_wait_for_the_last_line() {
        // Sung to the last bar: the bass swaps once the line ends, not on the outro line before it.
        let (a, vocal_end) = sung_until(1.0);
        let b = track(128.0);
        let swap_in_a = |a: &TrackAnalysis, s: &AutoMixSettings| {
            let p = plan(Some(a), Some(&b), 238_500, 240_000, s);
            assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
            p.out_start_ms as f64 + p.out_song_ms(swap_at(&p) as f64)
        };
        let half_beat = 30_000.0 / 128.0;
        for s in [AutoMixSettings { max_transition_s: 40.0, ..Default::default() }, dj()] {
            assert!(swap_in_a(&a, &s) >= vocal_end as f64 - half_beat, "{:?}", s.style);
            let unsung = TrackAnalysis { vocal_end_ms: 0, ..a.clone() };
            assert!(swap_in_a(&unsung, &s) < vocal_end as f64 - half_beat, "{:?}: unsung, it swaps earlier", s.style);
        }
    }

    #[test]
    fn dj_loops_the_bars_after_the_singing() {
        // Five unsung bars at the end and a 16-bar intro: DJ loops the last four under its run-up rather than
        // laying it over the singing; Smooth plays the song's own bars.
        let (a, vocal_end) = sung_until(5.0);
        let bar = 4.0 * 60_000.0 / 128.0;
        let b = TrackAnalysis { drop_ms: (120.0 + 16.0 * bar) as i64, intro_end_ms: (120.0 + 16.0 * bar) as i64, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 238_500, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        let out_loop = p.out_loop_ms.expect(&p.reason);
        assert!(p.out_start_ms >= vocal_end && (out_loop as f64 - 4.0 * bar).abs() < 1.0, "{}", p.reason);
        assert!(p.duration_ms > 2 * out_loop, "{}", p.reason);
        let smooth = plan(Some(&a), Some(&b), 238_500, 240_000, &AutoMixSettings::default());
        assert_eq!(smooth.out_loop_ms, None, "{}", smooth.reason);
        // Bars that fade out jump back up each time round: an ending steady over only its last two bars is not looped.
        let fading = TrackAnalysis { steady_end_ms: vocal_end + (3.0 * bar) as i64, ..a };
        let p = plan(Some(&fading), Some(&b), 238_500, 240_000, &dj());
        assert_eq!(p.out_loop_ms, None, "{}", p.reason);
    }

    #[test]
    fn dj_tempo_bends_during_the_mix() {
        let (a, b) = (track(128.0), track(122.0));
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        let bend = p.bend.expect(&p.reason);
        assert_eq!((bend.end_ms, p.tempo_ramp_ms), (p.duration_ms, 0), "the incoming song is at its own tempo as the mix ends");
        let loop_end = p.in_loop.map_or(0, |l| l.ms * (l.repeats as i64 + 1));
        assert!(bend.start_ms >= loop_end && bend.start_ms <= swap_at(&p) && (p.in_ratio_at(p.duration_ms as f64) - 1.0).abs() < 1e-9, "{p:?}");
        // The outgoing song plays whole bars of its own under the bend, and the beats stay together.
        let bars = p.out_song_ms(p.duration_ms as f64) / (4.0 * 60_000.0 / 128.0);
        assert!(((bars * 4.0) - (bars * 4.0).round()).abs() < 0.01, "{bars} bars");
        lands_on_swap(&p, &a, p.in_start_ms + (p.in_song_ms(swap_at(&p) as f64)).round() as i64);
        for t in [bend.start_ms, (bend.start_ms + bend.end_ms) / 2, bend.end_ms] {
            let t = t as f64;
            assert!((p.in_ratio_at(t) - p.out_ratio_at(t) * p.tempo_ratio).abs() < 1e-9, "both decks at one tempo at {t}");
        }
        check_skip(&p, &a, &b, 240_000);
        // Smooth ramps after the mix.
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert!(p.bend.is_none() && p.tempo_ramp_ms > 0, "{}", p.reason);
    }

    /// A tempo gap past the limit glides in DJ mixes: one tempo for both decks across the whole mix, two percent a
    /// bar at most, each song near its own while it is the louder; past twice the limit it does not mix.
    #[test]
    fn dj_glides_across_a_tempo_gap_past_the_limit() {
        let a = track(128.0);
        for (bpm, glides) in [(110.0, true), (104.0, false)] {
            let b = track(bpm);
            let pct = (128.0 / bpm - 1.0) * 100.0;
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
            if !glides {
                assert_ne!(p.kind, TransitionKind::BeatMatched, "{pct:.0} %: {}", p.reason);
                continue;
            }
            assert_eq!(p.kind, TransitionKind::BeatMatched, "{pct:.0} %: {}", p.reason);
            let bend = p.bend.expect(&p.reason);
            let loop_end = p.in_loop.map_or(0, |l| l.ms * (l.repeats as i64 + 1));
            assert_eq!((bend.start_ms, bend.end_ms), (loop_end, p.duration_ms), "across the whole mix: {p:?}");
            let bars = p.out_song_ms(p.duration_ms as f64) / (4.0 * 60_000.0 / 128.0);
            assert!(bars >= pct / 2.0 - 0.01, "{pct:.0} % over {bars} bars");
            let swap = swap_at(&p) as f64;
            let off = |r: f64| (r - 1.0).abs() * 100.0;
            assert!(off(p.in_ratio_at(swap)) < pct && off(p.out_ratio_at(swap)) < pct, "both nearer their own at the swap: {p:?}");
            lands_on_swap(&p, &a, p.in_start_ms + (p.in_song_ms(swap)).round() as i64);
            check_skip(&p, &a, &b, 240_000);
            assert_eq!(plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default()).kind, TransitionKind::MixRampFade, "Smooth never glides");
        }
    }

    #[test]
    fn dj_lands_the_drop_on_the_last_one() {
        let bar = 4.0 * 60_000.0 / 128.0;
        // The last chorus 20 bars before the last downbeat, the incoming drop eight bars in.
        let a = TrackAnalysis { last_drop_ms: (120.0 + 107.0 * bar) as i64, ..track(128.0) };
        let b = TrackAnalysis { drop_ms: (120.0 + 8.0 * bar) as i64, ..track(128.0) };
        let b = TrackAnalysis { vocal_start_ms: b.drop_ms, vocal_end_ms: 200_000, ..b };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert!(p.reason.contains("double drop"), "{}", p.reason);
        lands_on_swap(&p, &a, b.drop_ms);
        assert!((p.out_start_ms as f64 + p.out_song_ms(swap_at(&p) as f64) - a.last_drop_ms as f64).abs() < 2.0, "{p:?}");
        check_skip(&p, &a, &b, 240_000);
        // Not in Smooth, not over two voices, not across keys that do not blend.
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert!(!p.reason.contains("double drop"), "{}", p.reason);
        let sung = TrackAnalysis { vocal_start_ms: 10_000, vocal_end_ms: a.last_drop_ms + (8.0 * bar) as i64, ..a.clone() };
        let p = plan(Some(&sung), Some(&b), 240_000, 240_000, &dj());
        assert!(!p.reason.contains("double drop"), "{}", p.reason);
        let far = TrackAnalysis { key: camelot(2, false), ..b.clone() };
        let p = plan(Some(&a), Some(&far), 240_000, 240_000, &dj());
        assert!(!p.reason.contains("double drop"), "{}", p.reason);
    }

    /// `check_skip` without analyses.
    fn check_skip_blind(p: &TransitionPlan, out_dur: i64) {
        assert!(p.in_start_ms <= MAX_SKIP_MS && out_dur - (p.out_start_ms + p.duration_ms) <= MAX_SKIP_MS, "{p:?}");
    }

    #[test]
    fn unanalysed_is_equal_power_fade() {
        let s = AutoMixSettings { max_transition_s: 8.0, ..Default::default() };
        let p = plan(None, None, 200_000, 180_000, &s);
        assert_eq!(p.kind, TransitionKind::EqualPowerFade);
        assert_eq!(p.fade_curve, FadeCurve::EqualPower);
        assert_eq!((p.out_start_ms, p.in_start_ms, p.duration_ms), (192_000, 0, 8000));
        assert_eq!((p.bass_swap, p.low_pass, p.tempo_ratio), (None, None, 1.0));
        check_skip_blind(&p, 200_000);
        // Capped, and at most a third of a short track.
        let p = plan(None, None, 200_000, 180_000, &AutoMixSettings { max_transition_s: 30.0, ..Default::default() });
        assert_eq!(p.duration_ms, 12_000);
        let p = plan(None, None, 200_000, 9_000, &AutoMixSettings { max_transition_s: 30.0, ..Default::default() });
        assert_eq!(p.duration_ms, 3_000);
        let p = plan(None, None, 200_000, 600, &s);
        assert_eq!(p.kind, TransitionKind::Gapless);

        // Stale analysis ignored.
        let a = TrackAnalysis { duration_ms: 300_000, ..track(128.0) };
        let p = plan(Some(&a), None, 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::EqualPowerFade);

        // Same album in order is gapless.
        let s = AutoMixSettings { same_album_in_order: true, ..Default::default() };
        let p = plan(Some(&track(128.0)), Some(&track(128.0)), 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::Gapless);
        assert_eq!((p.out_start_ms, p.in_start_ms, p.duration_ms), (240_000, 0, 0));
    }

    /// Where the bass has changed hands, relative to the start of the mix.
    fn swap_at(p: &TransitionPlan) -> i64 {
        p.bass_swap.map_or(-1, |s| s.at_ms + s.len_ms)
    }

    /// Where `t` of the incoming track (its own time) is heard, relative to the start of the mix.
    fn lands(p: &TransitionPlan, t: i64) -> f64 {
        (t - p.in_start_ms) as f64 / p.tempo_ratio
    }

    /// Whether `t` of the outgoing track is on one of its downbeats.
    fn on_downbeat(a: &TrackAnalysis, t: f64) -> bool {
        let beats = (t - a.beat_offset_ms) / (60_000.0 / a.bpm);
        (beats - beats.round()).abs() < 0.01 && (beats.round() as i64).rem_euclid(4) == a.downbeat_phase as i64
    }

    /// An end whose grid holds over a run of it only (a live band's steady bars) is mixed inside the run.
    #[test]
    fn mixes_play_where_the_end_grids_hold() {
        let (beat, bar) = (60_000.0 / 128.0, 4.0 * 60_000.0 / 128.0);
        let (a, b) = (track(128.0), TrackAnalysis { drop_ms: (120.0 + 32.0 * 60_000.0 / 128.0) as i64, ..track(128.0) });
        let heard = |p: &TransitionPlan| (p.out_start_ms as f64, p.out_start_ms as f64 + p.out_loop_ms.map_or(p.out_song_ms(p.duration_ms as f64), |l| l as f64));
        let played = |p: &TransitionPlan| (p.in_start_ms as f64, p.in_start_ms as f64 + p.in_song_ms(p.duration_ms as f64));
        let free = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(free.kind, TransitionKind::BeatMatched, "{}", free.reason);
        // The outgoing grid holds up to 2 bars before where the free mix ends, the incoming one from 2 bars in.
        let to = (heard(&free).1 - 2.0 * bar) as i64;
        let a_held = TrackAnalysis { outro_grid_from_ms: to - (24.0 * bar) as i64, outro_grid_to_ms: to, ..a.clone() };
        let from = (120.0 + 2.0 * bar) as i64;
        let b_held = TrackAnalysis { intro_grid_from_ms: from, intro_grid_to_ms: from + (24.0 * bar) as i64, ..b.clone() };
        let p = plan(Some(&a_held), Some(&b_held), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        let ((o0, o1), (i0, i1)) = (heard(&p), played(&p));
        assert!(o0 >= a_held.outro_grid_from_ms as f64 - beat / 2.0 && o1 <= to as f64 + beat / 2.0, "outgoing {o0}..{o1} past {to}: {p:?}");
        assert!(i0 >= from as f64 - beat / 2.0 && i1 <= b_held.intro_grid_to_ms as f64 + beat / 2.0, "incoming {i0}..{i1}: {p:?}");
        check_skip(&p, &a_held, &b_held, 240_000);
        // Gridded over the whole song instead, the span says nothing.
        let weak = TrackAnalysis { outro_bpm_confidence: 0.1, ..a_held.clone() };
        assert_eq!(at_end(&weak).outro_grid_to_ms, 0);
    }

    #[test]
    fn confident_grids_beat_match() {
        let s = AutoMixSettings::default();
        let (a, b) = (track(128.0), track(124.0));
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        assert!((p.tempo_ratio - 128.0 / 124.0).abs() < 1e-9);
        assert!(p.keep_pitch);
        assert_eq!(p.tempo_ramp_beats, 32, "a 3 % change ramps back over 8 bars");
        let (beat, bar) = (60_000.0 / 128.0, 4.0 * 60_000.0 / 128.0);
        // The whole 8-bar intro under the outgoing song, the swap at its end, one beat of tail (16 s at 128 BPM).
        assert_eq!(p.in_start_ms, 120);
        assert!((p.duration_ms as f64 - 8.0 * bar - beat).abs() <= 1.0, "{}: {}", p.duration_ms, p.reason);
        assert!((lands(&p, b.intro_end_ms) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        assert!(on_downbeat(&a, p.out_start_ms as f64) && on_downbeat(&a, (p.out_start_ms + swap_at(&p)) as f64), "{}", p.out_start_ms);
        assert_eq!(p.bass_swap.unwrap().len_ms, bar.round() as i64, "Smooth hands the bass over across a bar");
        assert_eq!((p.in_fade_start_ms, p.in_fade_end_ms), (0, swap_at(&p)));
        assert_eq!((p.out_fade_start_ms, p.out_fade_end_ms), (swap_at(&p), p.duration_ms));
        assert_eq!(p.fade_curve, FadeCurve::SineSquared);
        assert_eq!(p.low_pass, None, "same key: no low-pass");
        assert_eq!(p.in_gain_db, 0.0, "loudness matching is off by default");
        assert!(p.reason.contains("same key") && p.reason.contains("end of the intro"), "{}", p.reason);

        // With room for more, a longer tail after the swap.
        let long = AutoMixSettings { max_transition_s: 40.0, ..Default::default() };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &long);
        assert!(p.duration_ms as f64 > 9.0 * bar && p.duration_ms <= 40_000, "{}", p.reason);
        assert!((lands(&p, b.intro_end_ms) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
    }

    #[test]
    fn enters_on_the_drop() {
        // Intro ends 4 bars in, drop 8 bars in: the swap is on the drop.
        let beat: f64 = 60_000.0 / 128.0;
        let b = TrackAnalysis { intro_end_ms: (120.0 + 16.0 * beat) as i64, drop_ms: (120.0 + 32.0 * beat) as i64, ..track(128.0) };
        let a = track(128.0);
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!((lands(&p, b.drop_ms) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        assert!(p.reason.contains("on the drop"), "{}", p.reason);
        assert_eq!(p.in_start_ms, 120, "the whole run-up fits: nothing skipped");
        check_skip(&p, &a, &b, 240_000);

        // Drop 12 bars in: 4 bars (7.5 s) skipped to reach it. 20 bars in would skip 22 s: never.
        let b = TrackAnalysis { drop_ms: (120.0 + 48.0 * beat) as i64, intro_end_ms: (120.0 + 48.0 * beat) as i64, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert!((lands(&p, b.drop_ms) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        assert_eq!(p.in_start_ms, (120.0 + 16.0 * beat).round() as i64, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        let b = TrackAnalysis { drop_ms: (120.0 + 80.0 * beat) as i64, intro_end_ms: (120.0 + 80.0 * beat) as i64, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        assert!(lands(&p, b.drop_ms) > swap_at(&p) as f64, "the drop is out of reach: {}", p.reason);
        let long = AutoMixSettings { max_transition_s: 45.0, ..Default::default() };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &long);
        assert!((lands(&p, b.drop_ms) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
    }

    #[test]
    fn chordless_runup_escapes_key_cap() {
        // Camelot 2 caps chords over chords at 8 bars; a 16-bar drum run-up still fits whole, a pad one does not.
        let beat: f64 = 60_000.0 / 128.0;
        let a = track(128.0);
        let drop = (120.0 + 64.0 * beat) as i64;
        let drums = TrackAnalysis { key: camelot(2, false), drop_ms: drop, intro_end_ms: drop, drop_runup_tonal_db: -12.0, ..track(128.0) };
        let long = AutoMixSettings { max_transition_s: 40.0, ..Default::default() };
        let p = plan(Some(&a), Some(&drums), 240_000, 240_000, &long);
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!((lands(&p, drop) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        assert_eq!(p.in_start_ms, 120, "the whole intro under the outgoing song: {}", p.reason);
        assert!(p.reason.contains("without chords"), "{}", p.reason);
        check_skip(&p, &a, &drums, 240_000);
        let pads = TrackAnalysis { drop_runup_tonal_db: -1.0, ..drums };
        let p = plan(Some(&a), Some(&pads), 240_000, 240_000, &long);
        assert!(p.duration_ms as f64 <= 8.25 * 4.0 * beat + 1.0, "{}", p.reason);
        check_skip(&p, &a, &pads, 240_000);
    }

    #[test]
    fn swaps_on_phrase_lines() {
        // A 16-bar intro into a 16 s mix: a run-up of 4 or 8 bars, never 6.
        let beat: f64 = 60_000.0 / 128.0;
        let a = track(128.0);
        let b = TrackAnalysis { drop_ms: (120.0 + 64.0 * beat) as i64, intro_end_ms: (120.0 + 64.0 * beat) as i64, drop_runup_tonal_db: -12.0, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        let runup = swap_at(&p) as f64 / (4.0 * beat);
        assert!((runup - runup.round()).abs() < 0.01 && runup.round() as i64 % 4 == 0, "{runup}: {}", p.reason);
        check_skip(&p, &a, &b, 240_000);

        // Full start swaps on phrase line.
        // No intro or drop: in on the first downbeat, swap on a four-bar line.
        let b = TrackAnalysis { intro_end_ms: 100, ..track(128.0) };
        let a = track(128.0);
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert_eq!(p.in_start_ms, 120);
        let bar = 4.0 * 60_000.0 / 128.0;
        let swap_bars = swap_at(&p) as f64 / bar;
        assert!((swap_bars - swap_bars.round()).abs() < 0.01 && swap_bars.round() as i64 % 4 == 0, "{swap_bars}: {}", p.reason);
        assert!(on_downbeat(&a, (p.out_start_ms + swap_at(&p)) as f64));
    }

    #[test]
    fn near_keys_filter() {
        let a = track(128.0);
        let b = TrackAnalysis { key: camelot(7, false), ..track(124.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.reason.contains("harmonic"), "{}", p.reason);
        let bar = 4.0 * 60_000.0 / 128.0;
        assert!(p.duration_ms as f64 > 9.0 * bar, "{}", p.duration_ms);
        assert_eq!(p.low_pass.map(|f| f.to_hz), Some(SWEEP_TO_HZ_SOFT), "soft low-pass");

        // Key distance two opens high pass.
        let a = track(128.0);
        let b = TrackAnalysis { key: camelot(2, false), ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.reason.contains("filter-open"), "{}", p.reason);
        assert!(p.duration_ms as f64 <= 8.25 * 4.0 * 60_000.0 / 128.0 + 1.0, "{}", p.duration_ms);
        assert!(p.high_pass.is_some_and(|f| f.to_hz > 500.0), "filter-open high-pass");
        assert_eq!(p.low_pass, None);
    }

    #[test]
    fn short_outro_loops_unless_sung() {
        // Five bars of outgoing music, an 8-bar incoming intro: the last four outgoing bars loop under the run-up.
        let beat = 60_000.0 / 128.0;
        let music = (5.0 * 4.0 * beat) as i64;
        let a = TrackAnalysis {
            silence_start_ms: 238_500 - music,
            silence_end_ms: 238_500,
            outro_start_ms: 238_500 - music,
            mixramp_end_ms: 238_500 - music / 2,
            ..track(128.0)
        };
        let b = track(128.0);
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        let out_loop = p.out_loop_ms.expect(&p.reason);
        assert!(p.reason.contains("remix"), "{}", p.reason);
        assert!(p.duration_ms > out_loop);
        assert_eq!(p.in_start_ms, 120, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        let a = TrackAnalysis { vocal_start_ms: 10_000, vocal_end_ms: 238_000, ..a };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert_eq!(p.out_loop_ms, None, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
    }

    #[test]
    fn tempo_matched() {
        let a = TrackAnalysis { bpm: 64.0, ..track(128.0) };
        let b = track(128.0);
        let s = AutoMixSettings { out_tag_bpm: 128.0, max_transition_s: 40.0, ..Default::default() };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!((p.tempo_ratio - 1.0).abs() < 0.01, "folded to tag: ratio {}", p.tempo_ratio);

        // Half and double tempo match.
        let p = plan(Some(&track(174.0)), Some(&track(88.0)), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!((p.tempo_ratio - 174.0 / 176.0).abs() < 1e-9);
        assert_eq!(p.tempo_ramp_beats, 16);

        // Drifting band matched on end grids.
        let a = TrackAnalysis { stability: 0.0, ..track(128.0) };
        let b = TrackAnalysis { stability: 0.0, ..track(126.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);

        // Mixes at outro tempo.
        let a = TrackAnalysis { bpm: 100.0, ..track(128.0) };
        let p = plan(Some(&a), Some(&track(128.0)), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert_eq!(p.tempo_ratio, 1.0, "the ends already agree: {}", p.reason);
    }

    #[test]
    fn tempo_limits_fall_back_to_mixramp() {
        let (a, b) = (track(128.0), track(118.0)); // 8.5 %
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::MixRampFade);
        assert!(p.reason.contains("tempo gap"), "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);

        let b = track(125.0); // 2.4 %: fine for time-stretch, too much for varispeed
        let s = AutoMixSettings { keep_pitch: false, ..Default::default() };
        assert_eq!(plan(Some(&a), Some(&b), 240_000, 240_000, &s).kind, TransitionKind::MixRampFade);
        let b = track(126.0); // 1.6 %
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::BeatMatched);
        assert!(!p.keep_pitch);

        let off = AutoMixSettings { beat_match: false, ..Default::default() };
        let p = plan(Some(&a), Some(&a), 240_000, 240_000, &off);
        assert_eq!(p.kind, TransitionKind::MixRampFade);
        assert!(p.reason.contains("beat matching off"));
    }

    #[test]
    fn weak_grids_fade() {
        let a = track(128.0);
        for b in [
            TrackAnalysis { bpm_confidence: 0.2, intro_bpm_confidence: 0.2, ..track(128.0) },
            TrackAnalysis { stability: 0.3, intro_stability: 0.3, ..track(128.0) },
            TrackAnalysis { bpm: 0.0, intro_bpm: 0.0, ..track(128.0) },
        ] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
            assert_eq!(p.kind, TransitionKind::MixRampFade, "{b:?}");
            assert!(p.reason.contains("no reliable beat grid"));
        }

        // One grid aligns fade.
        // Outgoing grid only: 8 bars from an outgoing downbeat.
        let (a, b) = (track(128.0), TrackAnalysis { bpm_confidence: 0.2, intro_bpm_confidence: 0.2, ..track(128.0) });
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::MixRampFade);
        assert!(p.reason.contains("downbeat-aligned") && p.reason.contains("bars out"), "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        let beat = 60_000.0 / 128.0;
        assert!((p.duration_ms as f64 - 8.0 * 4.0 * beat).abs() <= 1.0, "{}", p.duration_ms);
        let beats = (p.out_start_ms as f64 - 120.0) / beat;
        assert!((beats - beats.round()).abs() < 0.01 && (beats.round() as i64) % 4 == 0, "{beats}");

        // Incoming grid only: in on its first downbeat.
        let (a, b) = (TrackAnalysis { stability: 0.3, outro_stability: 0.3, ..track(128.0) }, track(128.0));
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert!(p.reason.contains("bars in"), "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        assert_eq!(p.in_start_ms, 120);
    }

    /// A DJ pair that cannot beat-match echoes out: the outgoing song falls on a downbeat after its last line, its
    /// last beat ringing on in repeats a rising high-pass thins, and the incoming music starts on that downbeat,
    /// rising over a bar, nothing of it skipped. Smooth fades.
    #[test]
    fn dj_echoes_out_where_it_cannot_beat_match() {
        let beat: f64 = 60_000.0 / 128.0;
        let a = TrackAnalysis { vocal_start_ms: 20_000, vocal_end_ms: 225_000, ..track(128.0) };
        let lead = TrackAnalysis { silence_start_ms: 2_000, mixramp_start_ms: 2_300, intro_beat_offset_ms: 2_000.0, beat_offset_ms: 2_000.0, ..track(170.0) };
        let drifting = TrackAnalysis { stability: 0.3, outro_stability: 0.3, ..a.clone() };
        for (a, b, grid) in [(a.clone(), track(170.0), true), (a.clone(), lead, true), (drifting, track(128.0), false)] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
            assert_eq!(p.kind, TransitionKind::EchoOut, "{}", p.reason);
            check_skip(&p, &a, &b, 240_000);
            let echo = p.echo.expect(&p.reason);
            assert_eq!(echo.delay_ms, beat.round() as i64, "{echo:?}");
            let exit = p.out_start_ms as f64 + p.out_fade_start_ms as f64;
            assert!(on_downbeat(&a, exit) && exit - beat >= a.vocal_end_ms as f64, "{p:?}");
            let fall = (p.out_fade_end_ms - p.out_fade_start_ms) as f64;
            assert!(if grid { (fall - beat).abs() < 1.0 } else { (fall - 4.0 * beat).abs() < 1.0 }, "{p:?}");
            assert!((p.in_fade_end_ms - p.in_fade_start_ms) as f64 >= 4.0 * beat - 1.0, "the next song rises over a bar: {p:?}");
            assert!(p.in_start_ms <= b.silence_start_ms && p.in_start_ms as f64 + p.in_fade_start_ms as f64 >= b.silence_start_ms as f64, "{p:?}");
            if b.silence_start_ms as f64 >= beat {
                assert_eq!(p.in_start_ms as f64 + p.in_song_ms(p.out_fade_start_ms as f64), b.silence_start_ms as f64, "{p:?}");
            }
            let hp = p.high_pass.expect(&p.reason);
            assert!(hp.to_hz > hp.from_hz && hp.end_ms == p.duration_ms, "{p:?}");
            let smooth = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
            assert!(smooth.kind != TransitionKind::EchoOut && smooth.echo.is_none(), "{}", smooth.reason);
        }
    }

    /// A DJ fade between kicks that are not beat-matched (out of a song sung to its end, which cannot echo out)
    /// hands the bass over once, where the fades cross; Smooth's fades keep both.
    #[test]
    fn dj_fades_hand_the_bass_over_where_they_cross() {
        let a = TrackAnalysis { vocal_start_ms: 10_000, vocal_end_ms: 238_000, ..track(128.0) };
        let b = TrackAnalysis { bpm_confidence: 0.2, intro_bpm_confidence: 0.2, ..track(97.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::MixRampFade, "{}", p.reason);
        let swap = p.bass_swap.expect(&p.reason);
        let cross = (p.in_fade_start_ms.max(p.out_fade_start_ms) + p.in_fade_end_ms.min(p.out_fade_end_ms)) / 2;
        assert!(swap.at_ms < cross && cross < swap.at_ms + swap.len_ms && swap.len_ms <= 1_000, "{p:?}");
        let smooth = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert!(smooth.kind == TransitionKind::MixRampFade && smooth.bass_swap.is_none(), "{}", smooth.reason);
    }

    #[test]
    fn mixramp_overlaps_the_quiet_ends() {
        let s = AutoMixSettings { max_transition_s: 12.0, match_loudness: true, ..Default::default() };
        let a = TrackAnalysis { bpm: 0.0, outro_bpm: 0.0, silence_end_ms: 230_000, mixramp_end_ms: 226_000, lufs: -8.0, ..track(128.0) };
        let b = TrackAnalysis { bpm: 0.0, intro_bpm: 0.0, silence_start_ms: 1_000, mixramp_start_ms: 3_000, lufs: -14.0, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::MixRampFade);
        assert_eq!((p.out_start_ms, p.in_start_ms, p.duration_ms), (224_000, 1_000, 6_000));
        assert_eq!(p.in_fade_end_ms, 2_000, "the incoming fade follows its own ramp");
        assert_eq!(p.low_pass.map(|f| (f.start_ms, f.to_hz)), Some((3_000, SWEEP_TO_HZ_SOFT)), "same key: soft low-pass in the second half");
        assert_eq!(p.in_gain_db, 6.0, "incoming 6 dB quieter gets 6 dB");
        check_skip(&p, &a, &b, 240_000);

        // An abrupt end into a loud start still shares the last five seconds, the two crossing over all of it:
        // a loud start comes in at once only over a quiet tail.
        let a = TrackAnalysis { bpm: 0.0, outro_bpm: 0.0, silence_end_ms: 240_000, mixramp_end_ms: 240_000, ..track(128.0) };
        let b = TrackAnalysis { bpm: 0.0, intro_bpm: 0.0, silence_start_ms: 0, mixramp_start_ms: 0, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
        assert_eq!((p.duration_ms, p.in_fade_end_ms, p.out_start_ms), (MIN_MIXRAMP_MS, MIN_MIXRAMP_MS, 240_000 - MIN_MIXRAMP_MS));
        let quiet_tail = TrackAnalysis { mixramp_end_ms: 234_000, ..a.clone() };
        let p = plan(Some(&quiet_tail), Some(&b), 240_000, 240_000, &s);
        assert_eq!(p.in_fade_end_ms, MIN_FADE_MS, "over a quiet tail a loud start comes in at once");

        let p = plan(Some(&a), None, 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::MixRampFade);
        assert_eq!(p.duration_ms, MIN_MIXRAMP_MS);
        assert_eq!(p.in_start_ms, 0);
    }

    #[test]
    fn silence_not_mixed_over() {
        // 40 s of trailing silence and a 25 s silent lead-in are skipped whole.
        let a = TrackAnalysis { silence_end_ms: 200_000, mixramp_end_ms: 198_000, outro_start_ms: 170_000, ..track(128.0) };
        let b = TrackAnalysis {
            silence_start_ms: 25_000,
            mixramp_start_ms: 26_000,
            beat_offset_ms: 25_020.0 % (60_000.0 / 128.0),
            intro_beat_offset_ms: 25_020.0 % (60_000.0 / 128.0),
            intro_end_ms: 25_020 + 15_000,
            ..track(128.0)
        };
        for s in [AutoMixSettings::default(), AutoMixSettings { beat_match: false, ..Default::default() }] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
            check_skip(&p, &a, &b, 240_000);
            let swap = if p.bass_swap.is_some() { swap_at(&p) } else { p.duration_ms / 2 };
            assert!(p.out_start_ms + swap <= 200_000 + 500, "{}: {p:?}", p.reason);
            assert!(p.in_start_ms >= 25_000 - 500, "{}: {p:?}", p.reason);
        }
    }

    #[test]
    fn leaves_before_its_end() {
        // A six-bar coda: the swap lands where it begins.
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let exit = (120.0 + bar * (((238_500.0 - 120.0) / bar).floor() - 6.0)) as i64;
        let a = TrackAnalysis { exit_ms: exit, ..track(128.0) };
        let b = track(128.0);
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(((p.out_start_ms + swap_at(&p)) - exit).abs() <= 2, "{}: swap at {}", p.reason, p.out_start_ms + swap_at(&p));
        assert!(p.reason.contains("closing breakdown"), "{}", p.reason);
        assert!((lands(&p, b.intro_end_ms) - swap_at(&p) as f64).abs() <= 2.0, "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        // A coda too long to skip is played.
        let exit = (120.0 + bar * (((238_500.0 - 120.0) / bar).floor() - 14.0)) as i64;
        let a = TrackAnalysis { exit_ms: exit, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        check_skip(&p, &a, &b, 240_000);
        assert!(p.out_start_ms + swap_at(&p) > exit, "{}", p.reason);

        // Leaves before short hidden track.
        // Song ends at 180 s, 40 s of silence, a 10 s hidden track: mixed at 180 s.
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = (120.0 + bar * 96.0) as i64; // a downbeat, 180 s
        let a = TrackAnalysis {
            exit_ms: end,
            gap_ms: end,
            gap_end_ms: end + 40_000,
            silence_end_ms: end + 50_000,
            mixramp_end_ms: end + 49_000,
            outro_start_ms: end - (16.0 * bar) as i64,
            ..track(128.0)
        };
        let b = track(128.0);
        for s in [AutoMixSettings::default(), AutoMixSettings { beat_match: false, ..Default::default() }] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
            check_skip(&p, &a, &b, 240_000);
            assert!(p.out_start_ms + p.duration_ms <= end + 2_000, "{}: {p:?}", p.reason);
        }
        // 30 s of hidden music is over the cap: played.
        let a = TrackAnalysis { exit_ms: 0, silence_end_ms: end + 70_000, mixramp_end_ms: end + 69_000, ..a };
        let p = plan(Some(&a), Some(&b), 260_000, 240_000, &AutoMixSettings::default());
        check_skip(&p, &a, &b, 260_000);
        assert!(p.out_start_ms > end + 40_000, "{}: {p:?}", p.reason);
    }

    #[test]
    fn far_keys_mix_quick() {
        let a = track(128.0);
        let b = TrackAnalysis { key: camelot(6, false), ..track(128.0) };
        let bar = 4.0 * 60_000.0 / 128.0;
        let with = |style| AutoMixSettings { max_transition_s: 40.0, style, ..Default::default() };
        // Both styles: a quick beat-matched mix, never a cut.
        for style in [MixStyle::Smooth, MixStyle::Dj] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &with(style));
            assert_eq!(p.kind, TransitionKind::BeatMatched, "{style:?}: {}", p.reason);
            assert!(p.duration_ms as f64 <= 5.25 * bar + 1.0 && p.reason.contains("keys far apart"), "{style:?}: {}", p.reason);
            check_skip(&p, &a, &b, 240_000);
        }
        // No mix fits (tempos too far apart): a fade, or a DJ echo-out.
        let b = TrackAnalysis { bpm: 150.0, intro_bpm: 150.0, ..b };
        for (style, kind) in [(MixStyle::Smooth, TransitionKind::MixRampFade), (MixStyle::Dj, TransitionKind::EchoOut)] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &with(style));
            assert_eq!(p.kind, kind, "{style:?}: {}", p.reason);
        }
    }

    #[test]
    fn dj_fades_the_outgoing_song_over_a_bar_at_least() {
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        // Leaving on a closing breakdown four bars from the end.
        let end = 120.0 + bar * ((238_500.0 - 120.0) / bar).floor();
        let a = TrackAnalysis { exit_ms: (end - 4.0 * bar) as i64, ..track(128.0) };
        // A drop 16 bars in, the run-up from the first bar: the whole length cap of a short mix (a timbre gap).
        let drop = (120.0 + 16.0 * bar) as i64;
        let dark = TrackAnalysis { outro_centroid: 500.0, ..a.clone() };
        let b = TrackAnalysis { drop_ms: drop, intro_end_ms: drop, ..track(128.0) };
        // And its unsung last bars looped under the run-up: a loop has no end of its own.
        let (looped, _) = sung_until(5.0);
        for a in [&a, &dark, &looped] {
            let p = plan(Some(a), Some(&b), 240_000, 240_000, &dj());
            assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
            assert!((p.out_fade_end_ms - p.out_fade_start_ms) as f64 >= bar - 1.0, "the outgoing song stops within a bar: {p:?}");
            check_skip(&p, a, &b, 240_000);
        }
    }

    #[test]
    fn dj_brings_the_incoming_instruments_in_before_its_voice() {
        let bar = 4.0 * 60_000.0 / 128.0;
        let a = track(128.0);
        // Sixteen instrumental bars, then the voice; the drop eight bars after that.
        let voice = (120.0 + 16.0 * bar) as i64;
        let b = TrackAnalysis { vocal_start_ms: voice, vocal_end_ms: 200_000, drop_ms: (120.0 + 24.0 * bar) as i64, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.in_start_ms < voice, "it comes in singing: {}", p.reason);
        assert!(lands(&p, voice) >= swap_at(&p) as f64 - 1.0, "its voice comes in under the run-up: {}", p.reason);
        // Sung from its first bar, it comes in all the same.
        let cold = TrackAnalysis { vocal_start_ms: 120, ..b.clone() };
        assert_eq!(plan(Some(&a), Some(&cold), 240_000, 240_000, &dj()).kind, TransitionKind::BeatMatched);
    }

    #[test]
    fn dj_mixes_far_keys_on_the_incoming_instruments() {
        let a = track(128.0);
        // Keys far apart, the voice from the end of the intro: a quick mix, in before the voice.
        let b = TrackAnalysis { key: camelot(6, false), ..track(128.0) };
        let sung = TrackAnalysis { vocal_start_ms: b.intro_end_ms, vocal_end_ms: 200_000, ..b };
        let p = plan(Some(&a), Some(&sung), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.in_start_ms < sung.vocal_start_ms, "it comes in singing: {}", p.reason);
        assert!(lands(&p, sung.vocal_start_ms) >= swap_at(&p) as f64 - 1.0, "its voice comes in under the run-up: {}", p.reason);
        check_skip(&p, &a, &sung, 240_000);
    }

    #[test]
    fn beat_matched_mixes_bring_the_incoming_song_in_over_a_bar() {
        let bar = 4.0 * 60_000.0 / 128.0;
        // Every singing layout of the two ends: the outgoing voice ending 0 to 8 bars from the end, the incoming one
        // starting 0 to 16 bars in.
        for out_bars in 0..=8 {
            let (a, _) = sung_until(out_bars as f64);
            for in_bars in [0, 1, 2, 4, 8, 16] {
                let b = TrackAnalysis { vocal_start_ms: (120.0 + in_bars as f64 * bar) as i64, vocal_end_ms: 200_000, ..track(128.0) };
                for s in [dj(), AutoMixSettings { max_transition_s: 40.0, ..Default::default() }] {
                    let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
                    if p.kind == TransitionKind::BeatMatched {
                        let rise = (p.in_fade_end_ms - p.in_fade_start_ms) as f64;
                        assert!(rise >= bar - 1.0, "{:?}, sung to {out_bars} bars from the end, from bar {in_bars}: in over {rise} ms: {}", s.style, p.reason);
                    }
                }
            }
        }
    }

    #[test]
    fn dj_echoes_out_after_the_last_line() {
        // A closing breakdown of seven bars, the voice running a bar into it; the next song without a grid, so an
        // echo-out on the outgoing downbeats: in the breakdown after the line, not over the line.
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = 120.0 + bar * ((238_500.0 - 120.0) / bar).floor();
        let exit = (end - 7.0 * bar) as i64;
        let vocal_end = (end - 6.0 * bar) as i64;
        let a = TrackAnalysis { exit_ms: exit, vocal_start_ms: 10_000, vocal_end_ms: vocal_end, ..track(128.0) };
        let b = TrackAnalysis { bpm_confidence: 0.1, intro_bpm_confidence: 0.1, ..track(128.0) };
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        assert_eq!(p.kind, TransitionKind::EchoOut, "{}", p.reason);
        assert!(p.out_start_ms >= vocal_end, "it echoes the singing: {}", p.reason);
        check_skip(&p, &a, &b, 240_000);
    }

    /// A fade out of a song singing near its end holds it whole until the line ends (or a bar before the fade does),
    /// the incoming voice band cut until then, and falls over a bar at least. DJ echoes out after the line instead.
    #[test]
    fn fades_hold_through_the_last_line() {
        let bar = 4.0 * 60_000.0 / 128.0;
        let (a, vocal_end) = sung_until(1.5);
        let gridless = |t: TrackAnalysis| TrackAnalysis { bpm_confidence: 0.1, intro_bpm_confidence: 0.1, outro_bpm_confidence: 0.1, ..t };
        // Tempos too far apart to mix, no grid on the incoming song, or none on either.
        let slow = TrackAnalysis { bpm: 95.0, intro_bpm: 95.0, ..track(95.0) };
        let pairs = [(a.clone(), slow), (a.clone(), gridless(track(128.0))), (gridless(a), gridless(track(128.0)))];
        for (a, b) in &pairs {
            let p = plan(Some(a), Some(b), 240_000, 240_000, &dj());
            assert!(p.kind == TransitionKind::EchoOut && p.out_start_ms >= vocal_end, "{}", p.reason);
            let p = plan(Some(a), Some(b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
            assert_eq!(p.kind, TransitionKind::MixRampFade, "{}", p.reason);
            let held_to = (vocal_end - p.out_start_ms).min(p.duration_ms - bar as i64);
            assert!(p.out_fade_start_ms >= held_to - 1, "falls at {} of {held_to}: {}", p.out_fade_start_ms, p.reason);
            assert!((p.out_fade_end_ms - p.out_fade_start_ms) as f64 >= bar - 1.0, "{p:?}");
            assert!(p.vocal_duck.is_some_and(|d| d.until_ms >= p.out_fade_start_ms), "{p:?}");
            check_skip(&p, a, b, 240_000);
        }
    }

    /// A song that stops cold, its last hit ringing out, ends on its stop: a mix that cannot blend starts the next
    /// song there under the ring-out instead of fading the bars before it.
    #[test]
    fn a_song_that_stops_ends_on_its_stop() {
        let bar = 4.0 * 60_000.0 / 128.0;
        let stop = (120.0 + 125.0 * bar) as i64;
        let a = TrackAnalysis { exit_ms: stop, ..track(128.0) };
        let far_tempo = track(150.0);
        let far_keys = TrackAnalysis { key: camelot(6, false), ..far_tempo.clone() };
        for (b, style) in [(&far_tempo, MixStyle::Smooth), (&far_keys, MixStyle::Smooth), (&far_tempo, MixStyle::Dj), (&far_keys, MixStyle::Dj)] {
            for beat_match in [true, false] {
                let s = AutoMixSettings { max_transition_s: 40.0, style, beat_match, ..Default::default() };
                let p = plan(Some(&a), Some(b), 240_000, 240_000, &s);
                assert_eq!((p.kind, p.out_start_ms, p.in_start_ms), (TransitionKind::MixRampFade, stop, b.silence_start_ms), "{style:?}: {p:?}");
                assert_eq!(p.out_start_ms + p.duration_ms, a.silence_end_ms, "the ring-out is heard to its end: {p:?}");
                assert!(p.in_fade_end_ms <= MIN_FADE_MS && p.low_pass.is_none(), "the next song comes in at once: {p:?}");
                check_skip(&p, &a, b, 240_000);
            }
        }
    }

    /// The bass changes hands in a sixteenth only as a DJ mix lands on a drop; elsewhere over a beat (DJ) or a bar
    /// (Smooth).
    #[test]
    fn bass_swaps_are_quick_only_into_a_drop() {
        let beat: f64 = 60_000.0 / 128.0;
        let a = track(128.0);
        let drop = TrackAnalysis { drop_ms: (120.0 + 32.0 * beat) as i64, ..track(128.0) };
        let smooth = AutoMixSettings { max_transition_s: 40.0, ..Default::default() };
        for (b, s, len) in [(&drop, dj(), beat / 4.0), (&a, dj(), beat), (&drop, smooth.clone(), 4.0 * beat), (&a, smooth, 4.0 * beat)] {
            let p = plan(Some(&a), Some(b), 240_000, 240_000, &s);
            assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
            assert_eq!(p.bass_swap.expect(&p.reason).len_ms, len.round() as i64, "{:?}: {}", s.style, p.reason);
        }
    }

    /// In a DJ run-up the incoming highs come back a quarter of it before the bass swaps; Smooth leaves them be.
    #[test]
    fn dj_brings_the_incoming_highs_in_before_the_bass() {
        let beat: f64 = 60_000.0 / 128.0;
        let (a, b) = (track(128.0), TrackAnalysis { drop_ms: (120.0 + 32.0 * beat) as i64, ..track(128.0) });
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &dj());
        let (h, swap) = (p.highs_in.expect(&p.reason), p.bass_swap.expect(&p.reason));
        let runup = swap_at(&p) as f64 / (4.0 * beat);
        assert!(runup >= 4.0, "{}", p.reason);
        assert_eq!(h.at_ms + h.len_ms, (swap_at(&p) as f64 - runup / 4.0 * 4.0 * beat).round() as i64, "{p:?}");
        assert!(h.at_ms + h.len_ms < swap.at_ms && h.db < 0.0, "{p:?}");
        let smooth = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert!(smooth.bass_swap.is_some() && smooth.highs_in.is_none(), "{}", smooth.reason);
        assert!(plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { bass_swap: false, ..dj() }).highs_in.is_none());
    }

    /// An echo-out repeats the outgoing song's last beat: over a sung line that is a stutter. It is never planned over
    /// a sung ending, whatever else fails.
    #[test]
    fn sung_endings_never_echo() {
        let sung = TrackAnalysis { vocal_start_ms: 10_000, vocal_end_ms: 238_000, ..track(128.0) };
        let far = TrackAnalysis { key: camelot(6, false), ..track(128.0) };
        for style in [MixStyle::Smooth, MixStyle::Dj] {
            for (out, inc) in [(sung.clone(), far.clone()), (sung.clone(), TrackAnalysis { vocal_start_ms: 120, vocal_end_ms: 200_000, ..far.clone() }), (sung.clone(), TrackAnalysis { bpm: 150.0, intro_bpm: 150.0, ..far.clone() })] {
                for filter_effects in [true, false] {
                    let s = AutoMixSettings { max_transition_s: 40.0, filter_effects, style, ..Default::default() };
                    let p = plan(Some(&out), Some(&inc), 240_000, 240_000, &s);
                    assert!(p.kind != TransitionKind::EchoOut && p.echo.is_none(), "{style:?}: {}", p.reason);
                }
            }
        }
    }

    /// Sung to its end into a song that starts singing eight bars before its drop (as Nobody's Listening into High
    /// Waters): neither style lays one voice under the other for more than a bar, though a long run-up into the drop
    /// would.
    #[test]
    fn two_voices_meet_for_a_bar_at_most() {
        let bar = 4.0 * 60_000.0 / 128.0;
        let a = TrackAnalysis { vocal_start_ms: 10_000, vocal_end_ms: 238_500, ..track(128.0) };
        let drop = (120.0 + 16.0 * bar) as i64;
        let b = TrackAnalysis { drop_ms: drop, intro_end_ms: drop, vocal_start_ms: (120.0 + 8.0 * bar) as i64, vocal_end_ms: 200_000, ..track(128.0) };
        for s in [AutoMixSettings { max_transition_s: 40.0, ..Default::default() }, dj()] {
            let p = plan(Some(&a), Some(&b), 240_000, 240_000, &s);
            let out_sung = (a.vocal_end_ms - p.out_start_ms).clamp(0, p.duration_ms) as f64;
            let in_sung = (((b.vocal_start_ms - p.in_start_ms) as f64 / p.tempo_ratio).max(0.0) + p.in_loop.map_or(0.0, |l| (l.ms * l.repeats as i64) as f64)).min(p.duration_ms as f64);
            assert!(out_sung - in_sung <= bar + 1.0, "{:?}: {:.0} ms of both singing: {}", s.style, out_sung - in_sung, p.reason);
            check_skip(&p, &a, &b, 240_000);
        }
    }

    #[test]
    fn vocals_kept_apart_and_gaps_shorten() {
        // Both sing: a short beat-matched mix, incoming voice ducked until the swap, outgoing thinned after it.
        let sung = |t: TrackAnalysis| TrackAnalysis { vocal_start_ms: 120, vocal_end_ms: 238_500, ..t };
        let (a, b) = (sung(track(128.0)), sung(track(128.0)));
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.reason.contains("voices kept apart") && p.reason.contains("vocals overlap"), "{}", p.reason);
        assert!(p.duration_ms as f64 <= 8.25 * 4.0 * 60_000.0 / 128.0 + 1.0, "{}", p.reason);
        let swap = swap_at(&p);
        assert!(swap > 0 && p.duration_ms - swap >= (60_000.0f64 / 128.0) as i64, "{}", p.reason);
        let d = p.vocal_duck.expect(&p.reason);
        assert_eq!((d.until_ms, d.db), (swap, VOCAL_DUCK_DB), "{}", p.reason);
        assert!(d.release_ms > 0 && d.release_ms <= swap);
        let h = p.high_pass.expect(&p.reason);
        assert_eq!((h.start_ms, h.end_ms, h.to_hz), (swap, swap + (p.duration_ms - swap) / 2, VOCAL_HP_TO_HZ), "{}", p.reason);
        check_skip(&p, &a, &b, 240_000);
        // Filters off: a short mix, nothing to keep the voices apart.
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings { filter_effects: false, ..Default::default() });
        assert_eq!((p.kind, p.vocal_duck), (TransitionKind::BeatMatched, None), "{}", p.reason);
        assert!(p.reason.contains("vocals overlap"), "{}", p.reason);
        let p = plan(Some(&track(128.0)), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!((p.vocal_duck, p.high_pass), (None, None), "{}", p.reason);
        // No grid: a fade, the outgoing song (sung to its end) held whole, the incoming voice ducked until it falls.
        let (a, b) = (TrackAnalysis { bpm: 0.0, outro_bpm: 0.0, ..a }, TrackAnalysis { bpm: 0.0, intro_bpm: 0.0, ..b });
        let p = plan(Some(&a), Some(&b), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::MixRampFade, "{}", p.reason);
        assert!(p.out_fade_start_ms > p.duration_ms / 2, "{}", p.reason);
        assert_eq!(p.vocal_duck.map(|d| d.until_ms), Some(p.out_fade_start_ms), "{}", p.reason);
        // Only one sings: the incoming voice is ducked under the outgoing line all the same.
        let p = plan(Some(&a), Some(&TrackAnalysis { vocal_end_ms: 0, ..b }), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.vocal_duck.map(|d| d.until_ms), Some(p.out_fade_start_ms), "{}", p.reason);
        // Loudness gap: short.
        let b = TrackAnalysis { lufs: -2.0, ..track(128.0) };
        let p = plan(Some(&track(128.0)), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert_eq!(p.kind, TransitionKind::BeatMatched);
        assert!(p.duration_ms as f64 <= 8.25 * 4.0 * 60_000.0 / 128.0 + 1.0, "{}", p.reason);
        // Timbre mismatch: short.
        let b = TrackAnalysis { intro_centroid: 5000.0, ..track(128.0) };
        let p = plan(Some(&track(128.0)), Some(&b), 240_000, 240_000, &AutoMixSettings { max_transition_s: 40.0, ..Default::default() });
        assert_eq!(p.kind, TransitionKind::BeatMatched);
        assert!(p.duration_ms as f64 <= 8.25 * 4.0 * 60_000.0 / 128.0 + 1.0, "{}", p.reason);
    }

    #[test]
    fn waltz_mixes_in_threes_only() {
        let waltz = |bpm| TrackAnalysis { beats_per_bar: 3, ..track(bpm) };
        let p = plan(Some(&waltz(150.0)), Some(&waltz(150.0)), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        let bar = 3.0 * 60_000.0 / 150.0;
        let bars = p.duration_ms as f64 / bar;
        assert!((bars - bars.round()).abs() < 0.01, "{} ms is not whole bars of three", p.duration_ms);
        let n = (p.out_start_ms as f64 - 120.0) / (60_000.0 / 150.0);
        assert!((n - n.round()).abs() < 0.01 && (n.round() as i64).rem_euclid(3) == 0, "{}", p.out_start_ms);

        let p = plan(Some(&waltz(150.0)), Some(&track(150.0)), 240_000, 240_000, &AutoMixSettings::default());
        assert_ne!(p.kind, TransitionKind::BeatMatched, "{}", p.reason);
        assert!(p.reason.contains("metres differ"), "{}", p.reason);
        // A row without a metre reads as 4/4.
        let p = plan(Some(&TrackAnalysis { beats_per_bar: 0, ..track(128.0) }), Some(&track(128.0)), 240_000, 240_000, &AutoMixSettings::default());
        assert_eq!(p.kind, TransitionKind::BeatMatched);
    }

    #[test]
    fn switches_turn_effects_off() {
        let s = AutoMixSettings { bass_swap: false, filter_effects: false, ..Default::default() };
        let p = plan(Some(&track(128.0)), Some(&track(128.0)), 240_000, 240_000, &s);
        assert_eq!(p.kind, TransitionKind::BeatMatched);
        assert_eq!((p.bass_swap, p.low_pass, p.tempo_ratio, p.tempo_ramp_ms), (None, None, 1.0, 0));
    }
}
