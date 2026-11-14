//! Moves for a pair that is not beat-matched, and blends that need no lock: each a plan and what it is worth, or
//! none where the pair does not allow it; [`super::plan`] keeps the best. Recipes and their sources:
//! docs/research/automix-dj.md and automix-software.md (B6 to B14).

use super::*;
use crate::types::{Band, BarProfile, Brake, Duck, Glide, HighsIn, MidDip, Reverb, Window};

/// A plan and its worth; the higher wins.
pub(super) struct Move {
    pub plan: TransitionPlan,
    pub worth: f64,
}

/// What each move reads: the outgoing song at its end, the incoming one at its start, the room, the settings and
/// why the pair is not beat-matched (for the log).
pub(super) struct Pair<'a> {
    pub a: &'a TrackAnalysis,
    pub b: Option<&'a TrackAnalysis>,
    pub room: Room,
    pub s: &'a AutoMixSettings,
    pub why_not: &'a str,
    /// An end is gridded by its beats ([`super::Gridded::Beats`]): nothing that holds one tempo on both may lock to it.
    pub by_beats: bool,
}

/// Worth of each move before what it costs, roughly how good it sounds when it fits: a blend over a beatless end
/// above an effect handover, the marked effects (brake, reverb) below the plain ones so they come up only where they
/// fit best or for variety.
const LOOP_RAMP_WORTH: f64 = 2.4;
const BEATLESS_WORTH: f64 = 2.0;
const STOP_GLOW_WORTH: f64 = 1.7;
const STOP_WORTH: f64 = 1.6;
const HANDOFF_WORTH: f64 = 1.5;
const RIDE_WORTH: f64 = 1.4;
const BRAKE_WORTH: f64 = 1.3;
const REVERB_WORTH: f64 = 1.3;
const ECHO_WORTH: f64 = 1.2;
const FILTER_OUT_WORTH: f64 = 1.0;
const MELT_WORTH: f64 = 1.0;
/// Each of Smooth's crossings (afterglow, rise), and more for the one that suits the pair ([`Suits`]).
const CROSSING_WORTH: f64 = 0.85;
const SUITS_WORTH: f64 = 0.15;
pub(super) const FADE_WORTH: f64 = 0.4;
/// Per second of beatless overlap, up to [`BEATLESS_MAX_MS`].
const BEATLESS_LENGTH_WORTH: f64 = 0.03;
/// An effect handover off a trusted grid, on the song's tempo alone.
const OFF_GRID_COST: f64 = 0.3;

/// What the same kind twice in a row costs: a marked move is heard as a tic, a plain one as a machine. Smooth's reverb
/// is a soft glow, heard again sooner than a fade.
pub(super) fn repeat_cost(kind: TransitionKind, style: MixStyle) -> f64 {
    match kind {
        TransitionKind::ReverbWash if style == MixStyle::Smooth => 0.4,
        TransitionKind::Brake | TransitionKind::ReverbWash => 2.0,
        TransitionKind::EchoOut | TransitionKind::FilterOut | TransitionKind::FilterRide => 0.6,
        TransitionKind::LoopRamp => 0.8,
        TransitionKind::BeatlessBlend => 0.2,
        _ => 0.0,
    }
}

fn tail(why_not: &str) -> String {
    if why_not.is_empty() { String::new() } else { format!(" ({why_not})") }
}

/// The bar of `bars` that `t` (ms) is in.
fn bar_at(bars: &[BarProfile], t: f64) -> Option<&BarProfile> {
    let i = bars.iter().rposition(|b| b.start_ms as f64 <= t)?;
    let len = match (bars.get(i + 1), i.checked_sub(1).map(|j| &bars[j])) {
        (Some(n), _) => (n.start_ms - bars[i].start_ms) as f64,
        (None, Some(p)) => (bars[i].start_ms - p.start_ms) as f64,
        (None, None) => return None,
    };
    (t < bars[i].start_ms as f64 + len).then_some(&bars[i])
}

/// The mean of `f` over the bars of `bars` that start in `[from, to)`; None when none do.
pub(super) fn mean_over(bars: &[BarProfile], from: f64, to: f64, f: fn(&BarProfile) -> f32) -> Option<f32> {
    let v: Vec<f32> = bars.iter().filter(|b| (from..to).contains(&(b.start_ms as f64))).map(f).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
}

/// A bar with no beat in it, audible.
const BEATLESS_PULSE: f32 = 0.3;
pub(super) const AUDIBLE_DB: f32 = -30.0;
/// A bar with a kick in it: its lows and its beat.
const KICK_LOW_DB: f32 = -6.0;
const KICK_PULSE: f32 = 0.5;
/// An effect needs something to work on: the bars it works on no quieter than this on average against the song.
const EFFECT_SOURCE_DB: f32 = -12.0;

/// The outgoing bars from `from` to `to` hold enough music to work an effect on ([`EFFECT_SOURCE_DB`]): over a
/// dying fade-out it leaves a hole.
fn alive(a: &TrackAnalysis, from: f64, to: f64) -> bool {
    mean_over(&a.outro_bars, from - bar_ms(a) + 1.0, to, |b| b.level_db).is_none_or(|l| l >= EFFECT_SOURCE_DB)
}
/// Loud enough at the exit for a brake, and a "bursting in" entry.
const LOUD_DB: f32 = -6.0;
/// An energy drop: the incoming opening's first four bars this much under the outgoing exit.
const DROP_DB: f32 = 8.0;

pub(super) fn beatless(b: &BarProfile) -> bool {
    b.pulse <= BEATLESS_PULSE && b.level_db >= AUDIBLE_DB
}

pub(super) fn kick(b: &BarProfile) -> bool {
    b.low_db >= KICK_LOW_DB && b.pulse >= KICK_PULSE
}

/// Where the outgoing song's closing run of beatless bars starts, ms, when it is at least [`BEATLESS_MIN_BARS`]
/// long and reaches its end.
fn beatless_tail(a: &TrackAnalysis) -> Option<f64> {
    let bars = &a.outro_bars;
    let audible: Vec<&BarProfile> = bars.iter().rev().skip_while(|b| b.level_db < AUDIBLE_DB).collect();
    let n = audible.iter().take_while(|b| beatless(b)).count();
    (n >= BEATLESS_MIN_BARS).then(|| audible[n - 1].start_ms as f64)
}

/// Where the incoming song's opening run of beatless bars ends, ms, when it is at least [`BEATLESS_MIN_BARS`] long.
fn beatless_head(b: &TrackAnalysis) -> Option<f64> {
    let bars: Vec<&BarProfile> = b.intro_bars.iter().skip_while(|x| x.level_db < AUDIBLE_DB).collect();
    let n = bars.iter().take_while(|x| beatless(x)).count();
    let end = match bars.get(n) {
        Some(x) => x.start_ms as f64,
        None => bars.last().map(|x| x.start_ms as f64 + bar_ms(b))?,
    };
    (n >= BEATLESS_MIN_BARS).then_some(end)
}

const BEATLESS_MIN_BARS: usize = 4;
/// The longest beatless overlap.
const BEATLESS_MAX_MS: f64 = 30_000.0;

/// A downbeat the outgoing song leaves on.
#[derive(Clone, Copy, Debug)]
struct Exit {
    at: f64,
    on_line: bool,
    /// Off the last word ([`last_word`]), not a downbeat.
    word: bool,
}

/// Downbeats of `a` to leave on, best first: the latest before its music ends or turns quiet (an effect on a dying
/// fade-out leaves a hole), else a four-bar line of its outro within four bars of that, with nothing sung from
/// `unsung` before it, enough left in it to work on, and the skip cap kept when its dry sound falls over `fall`.
fn exits(a: &TrackAnalysis, out_dur: i64, beat: f64, fall: f64, unsung: f64) -> Vec<Exit> {
    let bpb = bar_beats(a);
    let bar = bpb as f64 * beat;
    let ending = Ending::of(a, out_dur);
    let end = if ending.breakdown { ending.music_end } else { ending.leave(0.0) };
    let quiet = (a.mixramp_end_ms > a.silence_start_ms).then_some(a.mixramp_end_ms as f64 + beat);
    let limit = quiet.map_or(end, |q| q.min(end));
    let last = a.beat_offset_ms + downbeat_before(limit, a.beat_offset_ms, beat, bpb, a.downbeat_phase as i64) as f64 * beat;
    let on_line = |x: f64| {
        a.outro_start_ms > 0 && {
            let lines = (x - a.outro_start_ms as f64) / (4.0 * bar);
            lines >= -0.01 && (lines - lines.round()).abs() < 0.01
        }
    };
    let fits = |x: f64| x - beat >= a.silence_start_ms as f64 && !sings_after(a, x - unsung) && ending.skipped(x + fall) <= MAX_SKIP_MS as f64 && alive(a, x - beat, x);
    let all: Vec<f64> = (0..=EXIT_BARS_BACK).map(|n| last - n as f64 * bar).filter(|&x| fits(x)).collect();
    let lined = all.iter().take(4).copied().find(|&x| on_line(x));
    lined.into_iter().chain(all.iter().copied().filter(|&x| Some(x) != lined)).map(|x| Exit { at: x, on_line: on_line(x), word: false }).collect()
}

/// Where a handover leaves: the best of [`exits`] that `fits`, else, where the singing runs to the song's stop or
/// end, off its last word ([`last_word`]).
fn leave(a: &TrackAnalysis, out_dur: i64, beat: f64, fall: f64, unsung: f64, fits: impl Fn(f64) -> bool) -> Option<Exit> {
    exits(a, out_dur, beat, fall, unsung)
        .into_iter()
        .find(|x| fits(x.at))
        .or_else(|| last_word(a, out_dur, beat).filter(|&at| fits(at)).map(|at| Exit { at, on_line: false, word: true }))
}

/// Where a transition takes the incoming song from, ms: where its music is first heard, within its first third.
pub(super) fn music_start(b: Option<&TrackAnalysis>, in_dur: i64) -> i64 {
    b.map_or(0, |b| heard_from(b).clamp(0, in_dur / 3))
}

/// Where the incoming song's music is first heard: its start, past opening bars all under [`AUDIBLE_DB`] against
/// the song, skipped like silence (as far as [`MAX_SKIP_MS`]): a mix over them would hand over to near silence.
fn heard_from(b: &TrackAnalysis) -> i64 {
    let start = b.silence_start_ms;
    let bars: Vec<&BarProfile> = b.intro_bars.iter().filter(|x| x.start_ms as f64 + bar_ms(b) > start as f64).collect();
    match bars.iter().position(|x| x.level_db >= AUDIBLE_DB) {
        Some(i) if i > 0 && bars[i].start_ms - start <= MAX_SKIP_MS => bars[i].start_ms,
        _ => start,
    }
}

/// Bars back from the end a handover may leave on.
const EXIT_BARS_BACK: i64 = 8;

/// Where the incoming song's music starts, and where it comes in: its first downbeat when that is at most a beat
/// after the start of its music (a pickup), else that start.
fn entry(b: Option<&TrackAnalysis>, in_dur: i64) -> (f64, f64) {
    let music = music_start(b, in_dur) as f64;
    let first = b.filter(|b| grid_ok(b)).and_then(|b| {
        let beat_b = 60_000.0 / b.bpm;
        let d = b.beat_offset_ms + downbeat_from(music, b.beat_offset_ms, beat_b, bar_beats(b), b.downbeat_phase as i64) as f64 * beat_b;
        (d - music <= beat_b).then_some(d)
    });
    (music, first.unwrap_or(music))
}

/// The incoming song's level over its first four bars from `at`, against the outgoing song's at its exit `x`, dB.
fn entry_gap(a: &TrackAnalysis, b: Option<&TrackAnalysis>, x: f64, beat: f64, at: f64) -> Option<f32> {
    let b = b?;
    let out = bar_at(&a.outro_bars, x - beat)?.level_db;
    let inc = mean_over(&b.intro_bars, at - 0.5 * bar_ms(b), at + 4.0 * bar_ms(b), |x| x.level_db)?;
    Some(inc - out)
}

/// The outgoing beat, ms, and whether it is on a grid to trust; None outside a musical tempo.
fn beat_of(a: &TrackAnalysis) -> Option<(f64, bool)> {
    (MUSICAL_BPM.0..=MUSICAL_BPM.1).contains(&a.bpm).then(|| (60_000.0 / a.bpm, grid_ok(a)))
}

const MUSICAL_BPM: (f64, f64) = (30.0, 300.0);

/// A delay of a beat, doubled or halved into a quarter to a whole second.
fn echo_delay(beat: f64) -> f64 {
    let mut d = beat;
    while d < 250.0 {
        d *= 2.0;
    }
    while d > 1000.0 {
        d /= 2.0;
    }
    d
}

/// The plan's in-transition times start at the outgoing song's `start`; the incoming song starts `delay` in, at its
/// `in_start`.
fn handover(kind: TransitionKind, start: f64, delay: f64, in_start: f64, dur: f64) -> TransitionPlan {
    let mut p = blank(kind, start.round() as i64, in_start.round() as i64, dur.round() as i64, String::new());
    p.in_delay_ms = delay.round() as i64;
    p
}

/// The incoming song's quiet head (its opening under the MixRamp level, longer than [`QUIET_HEAD_MS`]) starts under
/// the outgoing song, as far ahead as the plan brings it in, its fade moved with it: a handover onto it would
/// otherwise leave a near-silence after the exit.
pub(super) fn pre_roll(m: &mut TransitionPlan, b: &TrackAnalysis) {
    let head = b.mixramp_start_ms - m.in_start_ms;
    let pre = head.min(m.in_delay_ms);
    if head > QUIET_HEAD_MS && pre > 0 {
        m.in_delay_ms -= pre;
        m.in_fade_start_ms -= pre;
        m.in_fade_end_ms -= pre;
    }
}

const QUIET_HEAD_MS: i64 = 1_000;

/// Echo out (automix-software.md B10): on a downbeat after the last line, the outgoing song's lows go, its last beat
/// rings on in band-limited repeats a rising high-pass thins, and it falls (over a beat on a trusted grid, a bar on
/// its tempo alone); the incoming music starts on that downbeat, rising as it falls, the repeats ringing over its first
/// bar and held down after it, and before its singing starts. Never over a sung beat, which would stutter.
pub(super) fn echo_out(p: &Pair) -> Option<Move> {
    let (a, s) = (p.a, p.s);
    let (beat, on_grid) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let fall = if on_grid { beat } else { bar };
    let dur = beat + ECHO_TAIL_BARS * bar;
    if dur > p.room.max_len as f64 {
        return None;
    }
    // Off a last word only with nothing sung after it: repeats under a voice still singing stutter.
    let x = leave(a, p.room.out_dur, beat, fall, beat, |x| !sings_after(a, x) && x - beat + dur <= p.room.out_dur as f64)?;
    let (_, at) = entry(p.b, p.room.in_dur);
    if (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    let mut m = handover(TransitionKind::EchoOut, x.at - beat, beat, at, dur);
    (m.out_fade_start_ms, m.out_fade_end_ms) = (beat.round() as i64, (beat + fall).round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (m.out_fade_start_ms, m.out_fade_end_ms);
    let delay = echo_delay(beat);
    // Held down once the incoming song is in, or as its singing starts if that is sooner.
    let sung = p.b.filter(|b| b.vocal_end_ms > 0).map(|b| beat + b.vocal_start_ms as f64 - at);
    let duck_at = sung.map_or(beat + bar, |v| v.clamp(beat, beat + bar) - 0.5 * beat);
    m.echo = Some(Echo {
        delay_ms: delay.round() as i64,
        feedback: ECHO_FEEDBACK,
        wet_db: ECHO_WET_DB,
        send: Some(Window { start_ms: 0, end_ms: (beat + fall).round() as i64 }),
        band: Some(Band { low_hz: ECHO_LOW_HZ, high_hz: ECHO_HIGH_HZ }),
        duck: Some(Duck { at_ms: duck_at.round() as i64, len_ms: (0.5 * beat).round() as i64, db: ECHO_DUCK_DB }),
    });
    if s.filter_effects {
        // The lows going from half a beat before the exit, the repeats thinning after it.
        m.high_pass = Some(Sweep { start_ms: (0.5 * beat).round() as i64, end_ms: (beat + bar).round() as i64, from_hz: ECHO_HP_FROM_HZ, to_hz: ECHO_HP_TO_HZ });
    }
    m.in_gain_db = loudness_trim(Some(a), p.b, s);
    let line = if x.word { "off its last word" } else if x.on_line { "on the outro phrase" } else if a.vocal_end_ms > 0 { "after its last line" } else { "on its last bars" };
    let by = if on_grid { "on its downbeat" } else { "by its tempo" };
    m.reason = format!("echo-out {by}, {line}{}", tail(p.why_not));
    Some(Move { plan: m, worth: ECHO_WORTH - if on_grid { 0.0 } else { OFF_GRID_COST } })
}

/// Bars the outgoing echo rings on after the exit.
const ECHO_TAIL_BARS: f64 = 2.0;
const ECHO_FEEDBACK: f32 = 0.55;
const ECHO_WET_DB: f32 = -3.0;
const ECHO_LOW_HZ: f32 = 250.0;
const ECHO_HIGH_HZ: f32 = 3_500.0;
const ECHO_DUCK_DB: f32 = -12.0;
const ECHO_HP_FROM_HZ: f32 = 80.0;
const ECHO_HP_TO_HZ: f32 = 1_500.0;

/// Filter out (B11): the outgoing song thins through a high-pass rising over two bars to a downbeat after its last
/// line, or to its last word where it sings to its end, where it falls over a beat as the incoming music bursts in on
/// it, clean. The dry move for an ending whose last beat is sung, where an echo would stutter the word.
pub(super) fn filter_out(p: &Pair) -> Option<Move> {
    let (a, s) = (p.a, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let rise = FILTER_OUT_BARS * bar;
    let x = leave(a, p.room.out_dur, beat, beat, 0.0, |x| x - rise >= a.silence_start_ms as f64 && x + beat <= p.room.out_dur as f64 && alive(a, x - rise, x))?;
    // A tail within the song after the exit, for a loudness match to let go over.
    let dur = rise + (FILTER_OUT_TAIL_BARS * bar).min(p.room.out_dur as f64 - x.at).max(beat);
    let (_, at) = entry(p.b, p.room.in_dur);
    if dur > p.room.max_len as f64 || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    // The incoming song bursting in on its downbeat, the thinned outgoing one gone over the same beat.
    let mut m = handover(TransitionKind::FilterOut, x.at - rise, rise, at, dur);
    (m.out_fade_start_ms, m.out_fade_end_ms) = (rise.round() as i64, (rise + beat).round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (rise.round() as i64, (rise + 0.5 * beat).round() as i64);
    m.high_pass = Some(Sweep { start_ms: 0, end_ms: (rise + beat).round() as i64, from_hz: FILTER_OUT_FROM_HZ, to_hz: FILTER_OUT_TO_HZ });
    m.in_gain_db = loudness_trim(Some(a), p.b, s);
    let place = if x.word { ", off its last word" } else if x.on_line { ", on the outro phrase" } else { "" };
    m.reason = format!("filter-out over {FILTER_OUT_BARS} bars{place}{}", tail(p.why_not));
    Some(Move { plan: m, worth: FILTER_OUT_WORTH - if on_grid || x.word { 0.0 } else { OFF_GRID_COST } })
}

const FILTER_OUT_BARS: f64 = 2.0;
const FILTER_OUT_TAIL_BARS: f64 = 2.0;
const FILTER_OUT_FROM_HZ: f32 = 30.0;
const FILTER_OUT_TO_HZ: f32 = 1_200.0;

/// The filter ride (B6), or the bass hand-off where both songs carry a kick at the join (B7): over the outgoing
/// song's last bars before it leaves, it darkens through a low-pass while the incoming one comes up through a
/// high-pass that opens, the lows changing hands once, on a beat, so one kick plays at a time. Two grooves overlap,
/// but in bands that do not crowd each other; short, so the tempos never drift far apart.
pub(super) fn filter_ride(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let (music, _) = entry(Some(b), p.room.in_dur);
    let in_kicks = mean_over(&b.intro_bars, music, music + 2.0 * bar_ms(b), |x| if kick(x) { 1.0 } else { 0.0 }).is_some_and(|k| k >= 0.5);
    let (x, kicks, beats, len) = exits(a, p.room.out_dur, beat, 0.0, 0.0).into_iter().find_map(|x| {
        let kicks = in_kicks && bar_at(&a.outro_bars, x.at - beat).is_some_and(kick);
        let (beats, cap) = if kicks { (HANDOFF_BEATS, HANDOFF_MAX_MS) } else { (RIDE_BEATS, RIDE_MAX_MS) };
        let len = (beats * beat).min(cap);
        alive(a, x.at - len, x.at).then_some((x, kicks, beats, len))
    })?;
    if len > p.room.max_len as f64 || (p.room.in_dur as f64) - music < 2.0 * len {
        return None;
    }
    // The outgoing singing done while the low-pass is still over 3 kHz, before its words would muffle under it.
    let start = x.at - len;
    if start < a.silence_start_ms as f64 || sings_after(a, start + RIDE_SUNG_SHARE * len) {
        return None;
    }
    let both_sing = sings_before(b, music + len);
    let mut m = blank(TransitionKind::FilterRide, start.round() as i64, music.round() as i64, len.round() as i64, String::new());
    let ms = |f: f64| (f * len).round() as i64;
    // The swap on the beat nearest its point, over half a beat.
    let swap_beat = |f: f64| ((f * len / beat).round() * beat - 0.25 * beat).min(len - 0.5 * beat).max(0.0);
    if kicks {
        m.in_high_pass = Some(Sweep { start_ms: 0, end_ms: ms(0.38), from_hz: HANDOFF_IN_HZ, to_hz: OPEN_HP_HZ });
        let swap = swap_beat(0.43);
        m.bass_swap = Some(BassSwap { at_ms: swap.round() as i64, len_ms: (0.5 * beat).round() as i64, cut_hz: RIDE_SWAP_HZ });
        m.low_pass = Some(Sweep { start_ms: (swap + 0.5 * beat).round() as i64, end_ms: ms(1.0), from_hz: SWEEP_FROM_HZ, to_hz: HANDOFF_OUT_HZ });
        (m.out_fade_start_ms, m.out_fade_end_ms) = ((swap + 0.5 * beat).round() as i64, ms(1.0));
        (m.in_fade_start_ms, m.in_fade_end_ms) = (0, ms(0.4));
    } else {
        let from = if both_sing { RIDE_IN_SUNG_HZ } else { RIDE_IN_HZ };
        m.in_high_pass = Some(Sweep { start_ms: 0, end_ms: ms(0.7), from_hz: from, to_hz: OPEN_HP_HZ });
        m.low_pass = Some(Sweep { start_ms: 0, end_ms: ms(1.0), from_hz: RIDE_OUT_FROM_HZ, to_hz: RIDE_OUT_TO_HZ });
        let swap = swap_beat(0.5);
        m.bass_swap = Some(BassSwap { at_ms: swap.round() as i64, len_ms: (0.5 * beat).round() as i64, cut_hz: RIDE_SWAP_HZ });
        (m.out_fade_start_ms, m.out_fade_end_ms) = (ms(0.35), ms(1.0));
        (m.in_fade_start_ms, m.in_fade_end_ms) = (0, ms(0.55));
    }
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    let what = if kicks { "bass hand-off" } else { "filter ride" };
    m.reason = format!("{what} over {beats:.0} beats{}", tail(p.why_not));
    let worth = if kicks { HANDOFF_WORTH } else { RIDE_WORTH };
    Some(Move { plan: m, worth: worth - if on_grid { 0.0 } else { OFF_GRID_COST } })
}

const RIDE_BEATS: f64 = 12.0;
/// The share of a ride the outgoing song may still sing over.
const RIDE_SUNG_SHARE: f64 = 0.25;
const RIDE_MAX_MS: f64 = 8_000.0;
const HANDOFF_BEATS: f64 = 8.0;
const HANDOFF_MAX_MS: f64 = 7_000.0;
const RIDE_OUT_FROM_HZ: f32 = 7_000.0;
const RIDE_OUT_TO_HZ: f32 = 300.0;
const RIDE_IN_HZ: f32 = 1_200.0;
/// The incoming high-pass when both songs sing: the incoming voice kept thin longer.
const RIDE_IN_SUNG_HZ: f32 = 2_400.0;
const HANDOFF_IN_HZ: f32 = 900.0;
const HANDOFF_OUT_HZ: f32 = 1_600.0;
/// A high-pass this low has opened.
const OPEN_HP_HZ: f32 = 20.0;
const RIDE_SWAP_HZ: f32 = 200.0;

/// Melt, Smooth's filter ride (automix-software.md B6, softened): over the outgoing song's last bars it darkens
/// slowly through a low-pass while the incoming song comes up thin through a high-pass that opens, the lows changing
/// hands over a bar in the middle, so two grooves at different tempos overlap only in bands that do not crowd each
/// other, and nothing moves quickly. Not over two voices.
pub(super) fn melt(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let len = (MELT_BEATS * beat).min(MELT_MAX_MS).min(p.room.max_len as f64);
    let x = exits(a, p.room.out_dur, beat, 0.0, 0.0).into_iter().find(|x| alive(a, x.at - len, x.at))?;
    let (music, _) = entry(Some(b), p.room.in_dur);
    if len < MELT_MIN_MS || (p.room.in_dur as f64) - music < 2.0 * len {
        return None;
    }
    // The outgoing singing done before its low-pass closes far, the incoming one not yet started.
    let start = x.at - len;
    if start < a.silence_start_ms as f64 || sings_after(a, start + MELT_SUNG_SHARE * len) || sings_before(b, music + 0.5 * len) {
        return None;
    }
    let mut m = blank(TransitionKind::FilterRide, start.round() as i64, music.round() as i64, len.round() as i64, String::new());
    let ms = |f: f64| (f * len).round() as i64;
    m.in_high_pass = Some(Sweep { start_ms: 0, end_ms: ms(0.6), from_hz: MELT_IN_HZ, to_hz: OPEN_HP_HZ });
    m.low_pass = Some(Sweep { start_ms: ms(0.25), end_ms: ms(1.0), from_hz: SWEEP_FROM_HZ, to_hz: MELT_OUT_HZ });
    let bar = bar_beats(a) as f64 * beat;
    let swap = (0.5 * len - 0.5 * bar).max(0.0);
    m.bass_swap = Some(BassSwap { at_ms: swap.round() as i64, len_ms: bar.min(len - swap).round() as i64, cut_hz: BASS_CUT_HZ });
    (m.out_fade_start_ms, m.out_fade_end_ms) = (ms(0.3), ms(1.0));
    (m.in_fade_start_ms, m.in_fade_end_ms) = (0, ms(0.6));
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!("melt over {:.0} s{}", len / 1000.0, tail(p.why_not));
    Some(Move { plan: m, worth: MELT_WORTH - if on_grid { 0.0 } else { OFF_GRID_COST } })
}

const MELT_BEATS: f64 = 16.0;
const MELT_MAX_MS: f64 = 10_000.0;
const MELT_MIN_MS: f64 = 5_000.0;
/// The share of a melt the outgoing song may still sing over.
const MELT_SUNG_SHARE: f64 = 0.3;
const MELT_IN_HZ: f32 = 600.0;
const MELT_OUT_HZ: f32 = 500.0;

/// Where a Smooth handover's two-beat crossing starts: around a downbeat after the last line, or, where the singing
/// runs to the song's stop or end, at its last word.
struct Crossing {
    at: f64,
    word: bool,
    on_line: bool,
    /// What placing it costs: a downbeat off a trusted grid may be wrong; a last word is wherever it falls.
    cost: f64,
}

fn crossing(a: &TrackAnalysis, out_dur: i64, beat: f64, on_grid: bool) -> Option<Crossing> {
    let x = leave(a, out_dur, beat, beat, 2.0 * beat, |_| true)?;
    Some(if x.word {
        Crossing { at: x.at, word: true, on_line: false, cost: 0.0 }
    } else {
        Crossing { at: x.at - beat, word: false, on_line: x.on_line, cost: if on_grid { 0.0 } else { OFF_GRID_COST } }
    })
}

impl Crossing {
    fn reason(&self) -> &'static str {
        match (self.word, self.on_line) {
            (true, _) => ", off its last word",
            (false, true) => ", on the outro phrase",
            (false, false) => "",
        }
    }
}

/// Where a crossing off the last word starts, where the singing runs to the song's stop or end: just after it, or,
/// where the song ends too soon after it for a tail, up to a bar before (its last line thrown into the reverb). The
/// word still loud (not a sung fade-out), and at most the cap of music after it.
fn last_word(a: &TrackAnalysis, out_dur: i64, beat: f64) -> Option<f64> {
    let v = a.vocal_end_ms as f64;
    let cross = v.min(out_dur as f64 - 3.0 * beat);
    let heard = bar_at(&a.outro_bars, v - beat).is_some_and(|b| b.level_db >= EFFECT_SOURCE_DB);
    let fits = v - cross <= bar_beats(a) as f64 * beat && Ending::of(a, out_dur).skipped(cross + 2.0 * beat) <= MAX_SKIP_MS as f64;
    (a.vocal_end_ms > a.silence_start_ms && heard && fits).then_some(cross)
}

/// Which of Smooth's crossings suits a pair: into an opening on a kick, the next song rises from below; into a softer
/// one, the outgoing song glows into a reverb whose tail is heard over it.
#[derive(Clone, Copy, PartialEq)]
enum Suits {
    Rise,
    Glow,
}

fn suits(b: &TrackAnalysis, at: f64) -> Suits {
    let opens_on_kick = mean_over(&b.intro_bars, at - 0.5 * bar_ms(b), at + 2.0 * bar_ms(b), |x| if kick(x) { 1.0 } else { 0.0 }).is_some_and(|k| k >= 0.5);
    if opens_on_kick { Suits::Rise } else { Suits::Glow }
}

/// What one of Smooth's crossings is worth for a pair, `fit` the one that suits it.
fn crossing_worth(fit: bool, x: &Crossing) -> f64 {
    CROSSING_WORTH + if fit { SUITS_WORTH } else { 0.0 } - x.cost
}

/// A transition centred on the crossing `x`: the outgoing song falls over its two beats as the incoming one, coming in
/// at `at`, rises over them, equal-power, with a tail after them within the outgoing song and as much before, so a
/// loudness match glides through evenly. Also what an effect takes in: the last word, or the groove's last beat, and
/// the fall. None where it does not fit.
fn centred(p: &Pair, kind: TransitionKind, x: &Crossing, beat: f64, at: f64) -> Option<(TransitionPlan, Window)> {
    let a = p.a;
    let middle = x.at + beat;
    let ring = CROSSING_TAIL_MS.max(bar_beats(a) as f64 * beat).min(p.room.out_dur as f64 - middle);
    let (start, dur) = (middle - ring, 2.0 * ring);
    if ring < 2.0 * beat || dur > p.room.max_len as f64 || (p.room.in_dur as f64) - at < 2.0 * dur || start < a.silence_start_ms as f64 {
        return None;
    }
    let ms = |t: f64| (t - start).round() as i64;
    let mut m = handover(kind, start, x.at - start, at, dur);
    (m.out_fade_start_ms, m.out_fade_end_ms) = (ms(x.at), ms(x.at + 2.0 * beat));
    (m.in_fade_start_ms, m.in_fade_end_ms) = (ms(x.at), ms(x.at + 2.0 * beat));
    m.in_gain_db = loudness_trim(Some(a), p.b, p.s);
    let sent = if x.word { 2.0 * beat } else { beat };
    Some((m, Window { start_ms: ms(x.at - sent), end_ms: ms(x.at + 2.0 * beat) }))
}

/// The tail heard after a centred crossing.
const CROSSING_TAIL_MS: f64 = 4_000.0;

/// Afterglow, Smooth's dissolve (B13, softened): a centred crossing whose outgoing song falls into a long, soft reverb
/// that washes on over the incoming one: the grooves overlap only as one falls and the other comes up. Off a last
/// word, the word rings into the reverb.
pub(super) fn afterglow(p: &Pair) -> Option<Move> {
    let (a, b) = (p.a, p.b?);
    if !p.s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let x = crossing(a, p.room.out_dur, beat, on_grid)?;
    let (_, at) = entry(Some(b), p.room.in_dur);
    let (mut m, send) = centred(p, TransitionKind::ReverbWash, &x, beat, at)?;
    m.reverb = Some(Reverb { send, decay_s: AFTERGLOW_DECAY_S, wet_db: AFTERGLOW_WET_DB, low_cut_hz: REVERB_LOW_CUT_HZ });
    m.reason = format!("afterglow into the next song{}{}", x.reason(), tail(p.why_not));
    Some(Move { plan: m, worth: crossing_worth(suits(b, at) == Suits::Glow, &x) })
}

const AFTERGLOW_DECAY_S: f32 = 4.5;
const AFTERGLOW_WET_DB: f32 = -8.0;

/// Rise, Smooth's way under a last line: the incoming song comes up from below, low-passed, while the outgoing song
/// sings on; the lows change hands over a bar at the middle, and at the crossing the outgoing song falls over two
/// beats as the incoming one opens out. Never into an opening that sings.
pub(super) fn rise(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let x = crossing(a, p.room.out_dur, beat, on_grid)?;
    let (music, at) = entry(Some(b), p.room.in_dur);
    let len = (RISE_BEATS * beat).min(RISE_MAX_MS).min(p.room.max_len as f64 - 2.0 * beat);
    let (start, dur) = (x.at - len, len + 2.0 * beat);
    if len < RISE_MIN_MS || start < a.silence_start_ms as f64 || sings_before(b, music + dur) || (p.room.in_dur as f64) - music < 2.0 * dur {
        return None;
    }
    let mut m = blank(TransitionKind::FilterRide, start.round() as i64, music.round() as i64, dur.round() as i64, String::new());
    let ms = |t: f64| t.round() as i64;
    m.in_low_pass = Some(Sweep { start_ms: 0, end_ms: ms(dur), from_hz: RISE_IN_HZ, to_hz: SWEEP_FROM_HZ });
    let swap = 0.5 * len - 0.5 * bar;
    m.bass_swap = Some(BassSwap { at_ms: ms(swap.max(0.0)), len_ms: ms(bar.min(len)), cut_hz: BASS_CUT_HZ });
    (m.in_fade_start_ms, m.in_fade_end_ms) = (0, ms(0.75 * len));
    (m.out_fade_start_ms, m.out_fade_end_ms) = (ms(len), ms(dur));
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!("rise over {:.0} s{}{}", dur / 1000.0, x.reason(), tail(p.why_not));
    Some(Move { plan: m, worth: crossing_worth(suits(b, at) == Suits::Rise, &x) })
}

/// A rise's length before its crossing: 16 beats, within 5 to 10 s.
const RISE_BEATS: f64 = 16.0;
const RISE_MAX_MS: f64 = 10_000.0;
const RISE_MIN_MS: f64 = 5_000.0;
/// Where the incoming low-pass starts opening from: its bass and kick, under the outgoing voice.
const RISE_IN_HZ: f32 = 250.0;

/// Brake (B14): a loud outgoing song stops like a turntable switched off, on a downbeat after its last line, over
/// four beats, and the incoming song bursts in under its last one. Only into an opening that hits.
pub(super) fn brake(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    let (beat, on_grid) = beat_of(a)?;
    if !on_grid {
        return None;
    }
    let bar = bar_beats(a) as f64 * beat;
    let stop = BRAKE_BEATS * beat;
    let land = beat + stop - beat;
    let dur = land + bar;
    if dur > p.room.max_len as f64 {
        return None;
    }
    let fits = |x: &Exit| x.at - beat + dur <= p.room.out_dur as f64 && bar_at(&a.outro_bars, x.at - beat).is_some_and(|b| b.level_db >= LOUD_DB);
    let x = exits(a, p.room.out_dur, beat, stop, beat).into_iter().find(fits)?;
    let (_, at) = entry(Some(b), p.room.in_dur);
    let hits = mean_over(&b.intro_bars, at - 0.5 * bar_ms(b), at + 2.0 * bar_ms(b), |x| if x.level_db >= LOUD_DB && x.pulse >= KICK_PULSE { 1.0 } else { 0.0 }).is_some_and(|h| h >= 0.5);
    if !hits || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    let mut m = handover(TransitionKind::Brake, x.at - beat, land, at, dur);
    m.brake = Some(Brake { start_ms: beat.round() as i64, len_ms: stop.round() as i64 });
    (m.out_fade_start_ms, m.out_fade_end_ms) = (beat.round() as i64, (beat + stop).round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (land.round() as i64, (land + 0.5 * beat).round() as i64);
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!("brake over {BRAKE_BEATS:.0} beats into a hit{}", tail(p.why_not));
    Some(Move { plan: m, worth: BRAKE_WORTH })
}

const BRAKE_BEATS: f64 = 4.0;

/// Reverb wash (B13): the outgoing song swells into a reverb on its last bar and falls on a downbeat after its last
/// line, the tail ringing on while a calmer incoming song (an energy drop, or a beatless opening) swells in under it.
pub(super) fn reverb_wash(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, _) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let land = bar + beat;
    let dur = (land + REVERB_IN_BARS * bar).max(bar + REVERB_DECAY_S as f64 * 1000.0);
    let x = exits(a, p.room.out_dur, beat, 2.0 * beat, beat).into_iter().find(|x| x.at - bar + dur <= p.room.out_dur as f64)?;
    let (_, at) = entry(Some(b), p.room.in_dur);
    let calmer = entry_gap(a, Some(b), x.at, beat, at).is_some_and(|g| g <= -DROP_DB) || beatless_head(b).is_some();
    if !calmer || dur > p.room.max_len as f64 || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    let mut m = handover(TransitionKind::ReverbWash, x.at - bar, land, at, dur);
    m.reverb = Some(Reverb { send: Window { start_ms: (bar - 2.0 * beat).round() as i64, end_ms: (bar + 2.0 * beat).round() as i64 }, decay_s: REVERB_DECAY_S, wet_db: REVERB_WET_DB, low_cut_hz: REVERB_LOW_CUT_HZ });
    (m.out_fade_start_ms, m.out_fade_end_ms) = (bar.round() as i64, (bar + 2.0 * beat).round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (land.round() as i64, (land + REVERB_IN_BARS * bar).round() as i64);
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!("reverb wash into a calmer song{}", tail(p.why_not));
    Some(Move { plan: m, worth: REVERB_WORTH })
}

const REVERB_DECAY_S: f32 = 3.5;
const REVERB_WET_DB: f32 = -3.0;
const REVERB_LOW_CUT_HZ: f32 = 250.0;
const REVERB_IN_BARS: f64 = 2.0;

/// A blend at any tempo where one side has no beat (B3): the incoming song comes up under the outgoing song's
/// beatless ending (a pad coda, a held chord), or its own beatless opening comes up under the outgoing song's last
/// bars, the beatful side gone before the other's beat starts. Long, and never two voices at once.
pub(super) fn beatless_blend(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    let Room { out_dur, in_dur, max_len } = p.room;
    let ending = Ending::of(a, out_dur);
    let (music, _) = entry(Some(b), in_dur);
    let max = (max_len as f64).min(BEATLESS_MAX_MS);
    let mut best: Option<(TransitionPlan, f64)> = None;
    // The outgoing song's beatless ending carries it: from its start (or later, to fit) to the end of the music.
    if let Some(from) = beatless_tail(a) {
        let end = ending.music_end;
        let start = from.max(end - max);
        let dur = end - start;
        if dur >= BEATLESS_MIN_MS && !sings_before(b, music + 0.5 * dur) && in_dur as f64 - music >= 2.0 * dur {
            let mut m = blank(TransitionKind::BeatlessBlend, start.round() as i64, music.round() as i64, dur.round() as i64, String::new());
            (m.out_fade_start_ms, m.out_fade_end_ms) = ((0.4 * dur).round() as i64, dur.round() as i64);
            (m.in_fade_start_ms, m.in_fade_end_ms) = (0, (0.6 * dur).round() as i64);
            m.reason = format!("blend over its beatless ending, {:.0} s", dur / 1000.0);
            best = Some((m, dur));
        }
    }
    // The incoming song's beatless opening carries it: the outgoing song's last bars over it, gone before its beat.
    if let Some(beat_in) = beatless_head(b) {
        let leave = ending.leave(0.0);
        let dur = (beat_in - music).min(max);
        let start = leave - dur;
        let clear = !sings_after(a, start + 0.5 * dur) || !sings_before(b, music + dur);
        if dur >= BEATLESS_MIN_MS && start >= a.silence_start_ms as f64 && clear && best.as_ref().is_none_or(|(_, d)| dur > *d) {
            let mut m = blank(TransitionKind::BeatlessBlend, start.round() as i64, music.round() as i64, dur.round() as i64, String::new());
            (m.out_fade_start_ms, m.out_fade_end_ms) = ((0.2 * dur).round() as i64, dur.round() as i64);
            (m.in_fade_start_ms, m.in_fade_end_ms) = (0, (0.7 * dur).round() as i64);
            m.reason = format!("blend over its beatless opening, {:.0} s", dur / 1000.0);
            best = Some((m, dur));
        }
    }
    let (mut m, dur) = best?;
    if ending.skipped(m.out_start_ms as f64 + dur) > MAX_SKIP_MS as f64 {
        return None;
    }
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    hand_over_bass_in_fade(&mut m, s);
    m.reason += &tail(p.why_not);
    Some(Move { worth: BEATLESS_WORTH + BEATLESS_LENGTH_WORTH * dur / 1000.0, plan: m })
}

/// The shortest beatless overlap worth it.
const BEATLESS_MIN_MS: f64 = 6_000.0;

/// A song that stops cold ([`on_its_stop`]) as a move: the radio's and the rock DJ's answer, end to end. It may also
/// glow: from a beat before it, its last hit rings into the afterglow's reverb under the next song.
pub(super) fn stop(p: &Pair) -> Vec<Move> {
    let Some(dry) = on_its_stop(p.a, p.b, p.room, p.s, p.why_not) else { return Vec::new() };
    let lead = beat_of(p.a).map_or(STOP_LEAD_MS, |(beat, _)| beat);
    let glows = p.s.filter_effects && dry.out_start_ms as f64 - lead >= p.a.silence_start_ms as f64;
    let glow = glows.then(|| {
        let ms = |t: f64| t.round() as i64;
        let mut m = dry.clone();
        m.kind = TransitionKind::ReverbWash;
        (m.out_start_ms, m.duration_ms, m.in_delay_ms) = (dry.out_start_ms - ms(lead), dry.duration_ms + ms(lead), ms(lead));
        (m.out_fade_start_ms, m.out_fade_end_ms) = (ms(lead), m.duration_ms);
        (m.in_fade_start_ms, m.in_fade_end_ms) = (ms(lead), ms(lead) + dry.in_fade_end_ms);
        m.reverb = Some(Reverb { send: Window { start_ms: 0, end_ms: ms(2.0 * lead) }, decay_s: AFTERGLOW_DECAY_S, wet_db: AFTERGLOW_WET_DB, low_cut_hz: REVERB_LOW_CUT_HZ });
        m.reason = format!("on its stop, its last hit glowing under the next song{}", tail(p.why_not));
        Move { plan: m, worth: STOP_GLOW_WORTH }
    });
    glow.into_iter().chain([Move { plan: dry, worth: STOP_WORTH }]).collect()
}

/// How far before a stop off any musical tempo its glow starts, ms.
const STOP_LEAD_MS: f64 = 500.0;

/// The mids dipped on both decks, where two songs' bodies would crowd each other (the three-band blend's mid cut).
pub(super) fn dip_mids(m: &mut TransitionPlan, s: &AutoMixSettings) {
    if s.filter_effects && m.duration_ms >= 2 * MIN_FADE_MS {
        m.mid_dip = Some(MidDip { window: Window { start_ms: 0, end_ms: m.duration_ms }, db: MID_DIP_DB, hz: MID_DIP_HZ, outgoing: true, incoming: true });
    }
}

const MID_DIP_DB: f32 = -6.0;
const MID_DIP_HZ: f32 = 1_000.0;

/// The loop tempo ramp (automix-software.md B8, BitChord's loop mix done the Ishizaki way): for tempos too far apart
/// to stretch one to the other, the outgoing song loops its last clean bars (instrumental, steady, with drums) and,
/// alone, glides at most [`RAMP_PCT_PER_BAR`] a bar to a tempo between the two, where neither is more than half the
/// gap from its own; the incoming song comes in on a downbeat of the loop at that tempo, locked, over
/// [`RAMP_BLEND_BARS`], the lows changing hands on a downbeat at the middle and the loop draining through a
/// rising high-pass; after it, the incoming song eases back to its own tempo at [`EASE_PCT_PER_BEAT`].
pub(super) fn loop_ramp(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if p.by_beats || !grid_ok(a) || !grid_ok(b) || !s.keep_pitch || !s.beat_match || bar_beats(a) != bar_beats(b) {
        return None;
    }
    // Two tempos in one octave, far enough apart that a stretch cannot meet them and near enough to glide.
    let ratio = b.bpm / a.bpm;
    let gap = (ratio - 1.0).abs() * 100.0;
    if !(RAMP_MIN_GAP_PCT..=RAMP_MAX_GAP_PCT).contains(&gap) || camelot_dist(a, b).is_some_and(|d| d > 2) {
        return None;
    }
    let meet = (a.bpm * b.bpm).sqrt();
    let (to_a, to_b) = (meet / a.bpm, meet / b.bpm);
    let (beat, bpb) = (60_000.0 / a.bpm, bar_beats(a));
    let bar = bpb as f64 * beat;
    // The latest clean loop of 8 bars after the last line, before the song leaves or turns quiet; else of 4, 2 or 1,
    // as few as its plays allow (a shorter loop repeats more).
    let ending = Ending::of(a, p.room.out_dur);
    let quiet = (a.mixramp_end_ms > a.silence_start_ms).then_some(a.mixramp_end_ms as f64);
    let end = quiet.map_or(ending.leave(0.0), |q| q.min(ending.leave(0.0)));
    let last = a.beat_offset_ms + downbeat_before(end, a.beat_offset_ms, beat, bpb, a.downbeat_phase as i64) as f64 * beat;
    let clean = |from: f64, bars: usize| {
        (0..bars).all(|i| {
            bar_at(&a.outro_bars, from + (i as f64 + 0.5) * bar).is_some_and(|x| x.voice < LOOP_VOICE && x.pulse >= KICK_PULSE && x.level_db >= LOOP_LEVEL_DB)
        }) && !sings_after(a, from)
    };
    // The glide, in whole bars at the outgoing tempo; then the first downbeat of the loop after it, and the blend: the
    // loop plays through all of it, at most [`MAX_LOOP_PLAYS`] times.
    let glide_bars = ((to_a - 1.0).abs() * 100.0 / RAMP_PCT_PER_BAR).ceil().max(2.0);
    let glide_ms = glide_bars * bar;
    let played = glide_bars + 1.0 + RAMP_BLEND_BARS;
    let (start, looped) = [8usize, 4, 2, 1]
        .into_iter()
        .filter(|&l| played / l as f64 <= MAX_LOOP_PLAYS)
        .flat_map(|l| (l..=EXIT_BARS_BACK as usize + l).map(move |back| (last - back as f64 * bar, l)))
        .find(|&(from, l)| from >= a.silence_start_ms as f64 && clean(from, l))?;
    let loop_ms = looped as f64 * bar;
    if ending.skipped(start + loop_ms) > MAX_SKIP_MS as f64 {
        return None;
    }
    let (music, at) = entry(Some(b), p.room.in_dur);
    if at - music > MAX_SKIP_MS as f64 {
        return None;
    }
    let mut m = blank(TransitionKind::LoopRamp, start.round() as i64, at.round() as i64, 0, String::new());
    m.out_loop_ms = Some(loop_ms.round() as i64);
    m.out_glide = Some(Glide { window: Window { start_ms: 0, end_ms: glide_ms.round() as i64 }, to: to_a });
    let downbeat = (m.out_song_ms(glide_ms) / bar).ceil() * bar;
    let delay = glide_ms + (downbeat - m.out_song_ms(glide_ms)) / to_a;
    let bar_wall = bar / to_a;
    let blend = RAMP_BLEND_BARS * bar_wall;
    let dur = delay + blend;
    if dur > p.room.max_len as f64 || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    m.duration_ms = dur.round() as i64;
    m.in_delay_ms = delay.round() as i64;
    (m.tempo_ratio, m.keep_pitch) = (to_b, true);
    // Back to its own tempo after the mix, no faster than the ease.
    let beats_back = ((to_b - 1.0).abs() * 100.0 / EASE_PCT_PER_BEAT).ceil();
    (m.tempo_ramp_beats, m.tempo_ramp_ms) = (beats_back as i32, (beats_back * 60_000.0 / b.bpm).round() as i64);
    m.fade_curve = FadeCurve::SineSquared;
    let half = delay + 0.5 * blend;
    (m.in_fade_start_ms, m.in_fade_end_ms) = (m.in_delay_ms, (half - bar_wall).round() as i64);
    (m.out_fade_start_ms, m.out_fade_end_ms) = (half.round() as i64, m.duration_ms);
    if s.bass_swap {
        m.bass_swap = Some(BassSwap { at_ms: (half - 0.25 * beat / to_a).round() as i64, len_ms: (0.5 * beat / to_a).round() as i64, cut_hz: BASS_CUT_HZ });
        m.highs_in = Some(HighsIn { at_ms: m.in_delay_ms, len_ms: (half - bar_wall - delay).max(0.0).round() as i64, db: RAMP_HIGHS_DB, hz: RAMP_HIGHS_HZ });
    }
    if s.filter_effects {
        let drain = RAMP_DRAIN_BARS * bar_wall;
        m.high_pass = Some(Sweep { start_ms: (dur - drain).round() as i64, end_ms: m.duration_ms, from_hz: RAMP_DRAIN_FROM_HZ, to_hz: RAMP_DRAIN_TO_HZ });
    }
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!(
        "loop ramp: {looped}-bar loop gliding {:.1} -> {:.1} BPM over {glide_bars:.0} bars, in locked for {RAMP_BLEND_BARS:.0}, back to {:.1} over {beats_back:.0} beats{}",
        a.bpm,
        meet,
        b.bpm,
        tail(p.why_not)
    );
    Some(Move { plan: m, worth: LOOP_RAMP_WORTH })
}

/// Tempo gaps a loop ramp crosses, percent; under the lower a stretch meets them.
const RAMP_MIN_GAP_PCT: f64 = 8.0;
const RAMP_MAX_GAP_PCT: f64 = 45.0;
/// The outgoing glide's rate (DJs ride 2 to 3 % a bar; BitChord 2.5 BPM a bar), and the incoming ease back after
/// (BitChord: below about 1 % a beat it reads as the groove, not the song changing speed).
pub(super) const RAMP_PCT_PER_BAR: f64 = 2.0;
pub(super) const EASE_PCT_PER_BEAT: f64 = 0.75;
const RAMP_BLEND_BARS: f64 = 8.0;
/// Most times a loop ramp's slice plays: a bar or two round more often turns into a broken record.
pub(super) const MAX_LOOP_PLAYS: f64 = 5.0;
const RAMP_DRAIN_BARS: f64 = 2.0;
const RAMP_DRAIN_FROM_HZ: f32 = 30.0;
const RAMP_DRAIN_TO_HZ: f32 = 1_500.0;
const RAMP_HIGHS_DB: f32 = -15.0;
const RAMP_HIGHS_HZ: f32 = 3_000.0;
/// A loopable bar: unsung, with its beat, and no quieter than this against the song.
const LOOP_VOICE: f32 = 0.4;
const LOOP_LEVEL_DB: f32 = -8.0;
