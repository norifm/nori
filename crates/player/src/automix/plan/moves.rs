//! Moves for a pair that is not beat-matched, or whose beat-matched mix lets the outgoing singing go, and blends that
//! need no lock: each a plan and what it is worth, or none where the pair does not allow it; [`super::plan`] keeps the
//! best. Recipes and their sources:
//! docs/research/automix-dj.md and automix-software.md (B6 to B14).

use super::*;
use crate::types::{Band, BarProfile, Brake, Duck, Glide, HighsIn, MidDip, OutLoop, Reverb, Window};

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

/// Worth of each move before what it costs, roughly how good it sounds when it fits: a loop ramp, the one long locked
/// blend across a tempo gap (its repeat cost puts it under a stop the next time); a song that stops on a last hit
/// ringing out a bar or more ends on its stop, the next song coming in under the ring; then the other long blends (a
/// beatless blend, a ride, more the longer it runs); the marked effects (brake, reverb) for variety where they fit; a
/// stop with no ring to start under where none of those does, before the quick handovers (echo, filter-out), heard as
/// cuts.
const LOOP_RAMP_WORTH: f64 = 2.9;
const STOP_GLOW_WORTH: f64 = 2.8;
const STOP_WORTH: f64 = 2.7;
const CUT_STOP_GLOW_WORTH: f64 = 0.87;
const CUT_STOP_WORTH: f64 = 0.85;
const BEATLESS_WORTH: f64 = 2.0;
const HANDOFF_WORTH: f64 = 1.5;
const RIDE_WORTH: f64 = 1.4;
const BRAKE_WORTH: f64 = 1.3;
const REVERB_WORTH: f64 = 1.3;
const ECHO_WORTH: f64 = 0.9;
const FILTER_OUT_WORTH: f64 = 0.8;
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
        TransitionKind::EchoOut | TransitionKind::FilterOut => 0.6,
        TransitionKind::FilterRide => 1.3,
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

/// The incoming bar at `at` hits: loud, with its beat. A quick handover (an echo, a filter-out, a brake) works only
/// onto one; onto anything softer it is heard as a cut.
fn bursts_in(b: &TrackAnalysis, at: f64) -> bool {
    bar_at(&b.intro_bars, at).is_some_and(|x| x.level_db >= LOUD_DB && x.pulse >= KICK_PULSE)
}

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
    // A loop ramp's entry is locked to the loop's downbeat, and the loop plays on under a quiet head.
    if m.kind == TransitionKind::LoopRamp {
        return;
    }
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
/// bar and held down after it, and before its singing starts. Never over a sung beat, which would stutter; only
/// onto an opening that hits ([`bursts_in`]).
pub(super) fn echo_out(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    let (beat, on_grid) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let fall = if on_grid { beat } else { bar };
    let dur = beat + ECHO_TAIL_BARS * bar;
    if dur > p.room.max_len as f64 {
        return None;
    }
    // Off a last word only with nothing sung after it: repeats under a voice still singing stutter.
    let x = leave(a, p.room.out_dur, beat, fall, beat, |x| !sings_after(a, x) && x - beat + dur <= p.room.out_dur as f64)?;
    let (_, at) = entry(Some(b), p.room.in_dur);
    if !bursts_in(b, at) || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    let mut m = handover(TransitionKind::EchoOut, x.at - beat, beat, at, dur);
    (m.out_fade_start_ms, m.out_fade_end_ms) = (beat.round() as i64, (beat + fall).round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (m.out_fade_start_ms, m.out_fade_end_ms);
    let delay = echo_delay(beat);
    // Held down once the incoming song is in, or as its singing starts if that is sooner.
    let sung = (b.vocal_end_ms > 0).then_some(beat + b.vocal_start_ms as f64 - at);
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
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
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

/// Filter out (B11): the incoming music bursts in two bars before a downbeat after the outgoing song's last line (or
/// its last word, where it sings to its end), and the outgoing song plays those bars out under it, thinning through a
/// high-pass rising over them as it falls: its ending heard, filtered away, not cut. The dry move for an ending whose
/// last beat is sung, where an echo would stutter the word; only onto an opening that hits ([`bursts_in`]).
pub(super) fn filter_out(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let rise = FILTER_OUT_BARS * bar;
    let x = leave(a, p.room.out_dur, beat, beat, 0.0, |x| x - rise >= a.silence_start_ms as f64 && x + beat <= p.room.out_dur as f64 && alive(a, x - rise, x))?;
    // Up to a bar after the exit, within the song, for a loudness match to let go over.
    let dur = rise + bar.min(p.room.out_dur as f64 - x.at);
    let (_, at) = entry(Some(b), p.room.in_dur);
    if !bursts_in(b, at) || dur > p.room.max_len as f64 || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    // The incoming song bursting in on its downbeat, the outgoing one thinning and falling under it to the exit.
    let mut m = handover(TransitionKind::FilterOut, x.at - rise, 0.0, at, dur);
    (m.out_fade_start_ms, m.out_fade_end_ms) = (0, rise.round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (0, (0.5 * beat).round() as i64);
    m.high_pass = Some(Sweep { start_ms: 0, end_ms: rise.round() as i64, from_hz: FILTER_OUT_FROM_HZ, to_hz: FILTER_OUT_TO_HZ });
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    let place = if x.word { ", off its last word" } else if x.on_line { ", on the outro phrase" } else { "" };
    m.reason = format!("filter-out over {FILTER_OUT_BARS} bars{place}{}", tail(p.why_not));
    Some(Move { plan: m, worth: FILTER_OUT_WORTH - if on_grid || x.word { 0.0 } else { OFF_GRID_COST } })
}

const FILTER_OUT_BARS: f64 = 2.0;
const FILTER_OUT_FROM_HZ: f32 = 30.0;
const FILTER_OUT_TO_HZ: f32 = 1_200.0;

/// The filter ride (B6), or the bass hand-off where both songs carry a kick at the join (B7): a DJ's long blend for
/// two songs that cannot be locked. Over up to [`RIDE_BARS`] of the outgoing song's last music, under its last lines
/// too, the incoming one comes up thin and low, its high-pass opening over the second half (its groove against the
/// outgoing one only in the last bars, its kick only at the end, so one kick plays at a time), done as the outgoing
/// song leaves after its last line or off its last word, the incoming drop or end of intro landing there where its
/// opening allows. The lows change hands over a bar there, the faders crossing from a bar before it so the two never
/// sound louder than one, and the outgoing song plays out its outro, darkening through a low-pass. Two voices
/// together for a bar at most; the longer the ride, the more it is worth.
pub(super) fn filter_ride(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if !s.filter_effects {
        return None;
    }
    let (beat, on_grid) = beat_of(a)?;
    let bar = bar_beats(a) as f64 * beat;
    let (music, _) = entry(Some(b), p.room.in_dur);
    let ending = Ending::of(a, p.room.out_dur);
    let x = leave(a, p.room.out_dur, beat, 0.0, 0.0, |x| x - RIDE_MIN_MS >= a.silence_start_ms as f64 && x + beat <= p.room.out_dur as f64 && alive(a, x - RIDE_MIN_MS, x))?;
    // The lead under the outgoing song, whole bars, within the room; two voices together for a bar at most: the
    // incoming song, from `from`, may start singing under it once the outgoing one has sung its last line.
    let room = |bars: f64| (bars * bar).min(RIDE_MAX_MS).min(p.room.max_len as f64 - bar).min(x.at - a.silence_start_ms as f64);
    let voices = |from: f64| match (a.vocal_end_ms > 0, b.vocal_end_ms > 0) {
        (true, true) => b.vocal_start_ms as f64 - from + bar + (x.at - a.vocal_end_ms as f64),
        _ => f64::MAX,
    };
    let whole = |ms: f64| (ms / bar).floor() * bar;
    // The ride starts where a new section of the outgoing song does, as it moves into it, when one starts from
    // [`RIDE_PHRASE_BARS`] short of the `most` a ride takes to `upto` before the exit and `fits`: the longest such
    // lead; else `most` itself.
    let phrases: Vec<f64> = phrase::sections(&a.outro_bars, false).into_iter().map(|i| x.at - a.outro_bars[i].start_ms as f64).collect();
    let phrased = |most: f64, upto: f64, fits: &dyn Fn(f64) -> bool| {
        let near = |l: f64| l >= RIDE_MIN_MS.max(most - RIDE_PHRASE_BARS * bar - 0.5 * beat) && l <= upto.max(most) + 0.5 * beat;
        phrases.iter().copied().filter(|&l| near(l) && fits(l)).fold(None, |m: Option<f64>, l| Some(m.map_or(l, |m| m.max(l)))).unwrap_or(most)
    };
    // Where the incoming song arrives (its drop, else the end of its intro) lands on the exit, the song started that
    // far before it, when its opening holds the lead with no more than the skip cap of it left out.
    let arrival = [(b.drop_ms, "its drop"), (b.intro_end_ms, "the end of its intro")].into_iter().find(|&(t, _)| t as f64 > music);
    let landed = arrival.and_then(|(t, what)| {
        let t = t as f64;
        let fits = |lead: f64| t - lead - music <= MAX_SKIP_MS as f64 && voices(t - lead) >= lead;
        let lead = phrased(whole(room(RIDE_BARS).min(t - music)), room(RIDE_SECTION_BARS).min(t - music), &fits);
        (lead >= RIDE_MIN_MS && fits(lead)).then_some((t - lead, lead, what))
    });
    let (from, lead, lands) = match landed {
        Some((from, lead, what)) => (from, lead, Some(what)),
        None => (music, phrased(whole(room(RIDE_BARS).min(voices(music))), room(RIDE_SECTION_BARS).min(voices(music)), &|_| true), None),
    };
    if lead < RIDE_MIN_MS || (p.room.in_dur as f64) - from < 2.0 * lead {
        return None;
    }
    // The outro after the exit, played out: to the end of the music, a beat to two bars.
    let outro = (ending.music_end - x.at).clamp(beat, RIDE_TAIL_BARS * bar).min(p.room.out_dur as f64 - x.at);
    let start = x.at - lead;
    // Both kicks where the bass changes hands: the incoming one's bars at the exit, the outgoing one's before it.
    let kicks = bar_at(&a.outro_bars, x.at - beat).is_some_and(kick)
        && mean_over(&b.intro_bars, from + lead - bar, from + lead + bar, |x| if kick(x) { 1.0 } else { 0.0 }).is_some_and(|k| k >= 0.5);
    let dur = lead + outro;
    let mut m = blank(TransitionKind::FilterRide, start.round() as i64, from.round() as i64, dur.round() as i64, String::new());
    let ms = |t: f64| t.round() as i64;
    // Over the outgoing voice the incoming one stays thinner, longer.
    let over_voice = sings_after(a, start);
    let from = if kicks { HANDOFF_IN_HZ } else if over_voice { RIDE_IN_SUNG_HZ } else { RIDE_IN_HZ };
    // Thin and low at first, so its groove is heard against the outgoing one only in the last bars: the fader on a
    // sine-squared curve, the high-pass opening as its beat arrives (its first bar with a kick, in the lead's middle
    // half), else over the second half.
    let start_in = m.in_start_ms as f64;
    let beat_in = b.intro_bars.iter().filter(|x| kick(x)).map(|x| x.start_ms as f64 - start_in).find(|&d| d >= -0.5 * bar);
    let opens = beat_in.filter(|&d| d >= 0.25 * lead && d <= 0.75 * lead).unwrap_or((0.5 * lead / bar).floor() * bar);
    m.in_high_pass = Some(Sweep { start_ms: ms(opens), end_ms: ms(lead), from_hz: from, to_hz: OPEN_HP_HZ });
    (m.in_fade_start_ms, m.in_fade_end_ms) = (0, ms(lead));
    m.fade_curve = FadeCurve::SineSquared;
    // The lows change hands over a bar on the exit.
    let len = bar.min(2.0 * (dur - lead));
    let swap = (lead - 0.5 * len).max(0.0);
    m.bass_swap = Some(BassSwap { at_ms: ms(swap), len_ms: ms(len), cut_hz: RIDE_SWAP_HZ });
    m.low_pass = Some(Sweep { start_ms: ms(lead), end_ms: ms(dur), from_hz: SWEEP_FROM_HZ, to_hz: RIDE_OUT_TO_HZ });
    (m.out_fade_start_ms, m.out_fade_end_ms) = (ms(lead - bar), ms(dur));
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    let what = if kicks { "bass hand-off" } else { "filter ride" };
    let place = if x.word { ", off its last word" } else if x.on_line { ", on the outro phrase" } else { "" };
    let landing = lands.map_or(String::new(), |what| format!(", {what} on the exit"));
    m.reason = format!("{what} over {:.0} bars{place}{landing}{}", lead / bar, tail(p.why_not));
    let worth = if kicks { HANDOFF_WORTH } else { RIDE_WORTH } + RIDE_LENGTH_WORTH * lead / 1000.0 + if lands.is_some() { RIDE_LANDING_WORTH } else { 0.0 };
    Some(Move { plan: m, worth: worth - if on_grid { 0.0 } else { OFF_GRID_COST } })
}

/// A ride's lead under the outgoing song: up to eight bars (twelve to start on a section), within 8 to 20 s.
const RIDE_BARS: f64 = 8.0;
const RIDE_MAX_MS: f64 = 20_000.0;
const RIDE_MIN_MS: f64 = 8_000.0;
/// Bars a ride may be shorter than its room to start on a new outgoing section, and the most it may take to.
const RIDE_PHRASE_BARS: f64 = 2.0;
const RIDE_SECTION_BARS: f64 = 12.0;
/// Bars of outro the outgoing song plays out after it leaves, at most.
const RIDE_TAIL_BARS: f64 = 2.0;
/// Per second of lead: the longer two songs ride together, the more it sounds like one set.
const RIDE_LENGTH_WORTH: f64 = 0.05;
/// The incoming song arriving (its drop, the end of its intro) as the outgoing one leaves.
const RIDE_LANDING_WORTH: f64 = 0.3;
/// Where the outgoing low-pass closes, two octaves over the swap's cut: lower, what is left between the two is a lone
/// band, heard as a tone.
const RIDE_OUT_TO_HZ: f32 = 800.0;
const RIDE_IN_HZ: f32 = 1_200.0;
/// The incoming high-pass under the outgoing voice: the incoming song kept thinner, longer.
const RIDE_IN_SUNG_HZ: f32 = 2_400.0;
const HANDOFF_IN_HZ: f32 = 900.0;
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
const MELT_OUT_HZ: f32 = 800.0;

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
/// four beats, and the incoming song comes in under its last one, its highs opening over its first bar, so it rises
/// out of the stop rather than slamming in. Only into an opening that hits.
pub(super) fn brake(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    let (beat, on_grid) = beat_of(a)?;
    if !on_grid {
        return None;
    }
    let bar = bar_beats(a) as f64 * beat;
    let stop = BRAKE_BEATS * beat;
    let land = beat + stop - beat;
    // A bar after it lands at least, up to BRAKE_IN_BARS within the song, for the loudness match to let go over.
    let shortest = land + bar;
    if shortest > p.room.max_len as f64 {
        return None;
    }
    let fits = |x: &Exit| x.at - beat + shortest <= p.room.out_dur as f64 && bar_at(&a.outro_bars, x.at - beat).is_some_and(|b| b.level_db >= LOUD_DB);
    let x = exits(a, p.room.out_dur, beat, stop, beat).into_iter().find(fits)?;
    let dur = (land + BRAKE_IN_BARS * bar).min(p.room.out_dur as f64 - (x.at - beat)).min(p.room.max_len as f64);
    let (_, at) = entry(Some(b), p.room.in_dur);
    // The bar it bursts in on hits, not one after it.
    if !bursts_in(b, at) || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    let mut m = handover(TransitionKind::Brake, x.at - beat, land, at, dur);
    m.brake = Some(Brake { start_ms: beat.round() as i64, len_ms: stop.round() as i64 });
    (m.out_fade_start_ms, m.out_fade_end_ms) = (beat.round() as i64, (beat + stop).round() as i64);
    (m.in_fade_start_ms, m.in_fade_end_ms) = (land.round() as i64, (land + 0.5 * beat).round() as i64);
    m.in_low_pass = Some(Sweep { start_ms: land.round() as i64, end_ms: (land + bar).round() as i64, from_hz: BRAKE_IN_HZ, to_hz: SWEEP_FROM_HZ });
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!("brake over {BRAKE_BEATS:.0} beats into a hit{}", tail(p.why_not));
    Some(Move { plan: m, worth: BRAKE_WORTH })
}

const BRAKE_BEATS: f64 = 4.0;
/// The incoming low-pass after a brake opens from here, Hz, and the mix runs on up to this many bars after it lands.
const BRAKE_IN_HZ: f32 = 800.0;
const BRAKE_IN_BARS: f64 = 2.0;

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
        // A free-time opening under a voice sounds off its beat: the outgoing song done singing early in the blend.
        let clear = !sings_after(a, start + BEATLESS_SUNG_SHARE * dur);
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

/// The share of a blend over a beatless opening the outgoing song may still sing over, while the opening is faint.
const BEATLESS_SUNG_SHARE: f64 = 0.25;

/// The shortest beatless overlap worth it.
const BEATLESS_MIN_MS: f64 = 6_000.0;

/// A song that stops cold ([`on_its_stop`]) as a move: the radio's and the rock DJ's answer, end to end. It may also
/// glow: from a beat before it, its last hit rings into the afterglow's reverb under the next song.
pub(super) fn stop(p: &Pair) -> Vec<Move> {
    let Some(dry) = on_its_stop(p.a, p.b, p.room, p.s, p.why_not) else { return Vec::new() };
    let lead = beat_of(p.a).map_or(STOP_LEAD_MS, |(beat, _)| beat);
    let rings = dry.duration_ms as f64 >= super::bar_ms(p.a);
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
        Move { plan: m, worth: if rings { STOP_GLOW_WORTH } else { CUT_STOP_GLOW_WORTH } }
    });
    let worth = if rings { STOP_WORTH } else { CUT_STOP_WORTH };
    glow.into_iter().chain([Move { plan: dry, worth }]).collect()
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
/// [`RAMP_BLEND_BARS`], the loop darkening as it comes (a low-pass rolling to [`RAMP_ROLL_HZ`], each play unlike the
/// last), the lows changing hands on a downbeat at the middle and the loop draining through a rising high-pass; after it, the incoming song eases back to its own tempo at [`EASE_PCT_PER_BEAT`].
pub(super) fn loop_ramp(p: &Pair) -> Option<Move> {
    let (a, b, s) = (p.a, p.b?, p.s);
    if p.by_beats || !grid_ok(a) || !grid_ok(b) || !s.keep_pitch || !s.beat_match || bar_beats(a) != bar_beats(b) {
        return None;
    }
    // Two tempos, the incoming one folded to the outgoing one's octave, far enough apart that a stretch cannot meet
    // them and near enough to glide; clashing keys only where a key shift brings them together.
    let b_bpm = fold(a.bpm, b.bpm);
    let gap = (b_bpm.max(a.bpm) / b_bpm.min(a.bpm) - 1.0) * 100.0;
    let shift = key_shift(a, b);
    if !(RAMP_MIN_GAP_PCT..=super::SPLIT_MAX_PCT).contains(&gap) || camelot_dist(a, b).is_some_and(|d| d >= KEY_CLASH) && shift.is_none() {
        return None;
    }
    let meet = (a.bpm * b_bpm).sqrt();
    let (to_a, to_b) = (meet / a.bpm, meet / b_bpm);
    let (beat, bpb) = (60_000.0 / a.bpm, bar_beats(a));
    let bar = bpb as f64 * beat;
    // A clean loop of 8 bars after the last line, before the song leaves or turns quiet, or of 4, where its plays allow
    // (a shorter loop repeats more).
    let ending = Ending::of(a, p.room.out_dur);
    let quiet = (a.mixramp_end_ms > a.silence_start_ms).then_some(a.mixramp_end_ms as f64);
    let end = quiet.map_or(ending.leave(0.0), |q| q.min(ending.leave(0.0)));
    let last = a.beat_offset_ms + downbeat_before(end, a.beat_offset_ms, beat, bpb, a.downbeat_phase as i64) as f64 * beat;
    // Where the outgoing grid holds and its beats were measured: a constant grid laid over bars it does not hold for
    // drifts off them.
    let out_kept = steady_run(&cleaned(&a.outro_beats)).to_vec();
    let gridded = |from: f64, to: f64| {
        in_grid((a.outro_grid_from_ms, a.outro_grid_to_ms), from, to, beat) && measured_or_beatless(&a.outro_bars, &a.outro_beats, &out_kept, from, to, bar, |_, _| false)
    };
    let clean = |from: f64, bars: usize| {
        !sings_after(a, from)
            && loopable(a, from, bars, bar, |x| x.pulse >= KICK_PULSE && x.level_db >= LOOP_LEVEL_DB)
            && seamless(&a.outro_bars, from, bars, bar)
            && gridded(from, from + bars as f64 * bar)
    };
    // The glide, in whole bars at the outgoing tempo; then the first downbeat of the loop after it, and the blend. The
    // song glides over as many of its own bars before the loop as the glide reads (in its grid, and unsung unless the
    // glide stretches a voice no more than [`SUNG_STRETCH_PCT`]); then the loop is heard at most [`MAX_LOOP_PLAYS`]
    // times up to the bass swap, halfway through the blend; after the swap it darkens, drains and fades.
    let glide_bars = ((to_a - 1.0).abs() * 100.0 / RAMP_PCT_PER_BAR).ceil().max(2.0);
    let glide_ms = glide_bars * bar;
    let glided = glide_bars * (1.0 + to_a) / 2.0;
    let played = glided + 1.0 + 0.5 * RAMP_BLEND_BARS;
    let sung_ok = (to_a - 1.0).abs() * 100.0 <= SUNG_STRETCH_PCT;
    let own = |from: f64| {
        (0..=glided.floor() as usize).rev().find(|&k| {
            let start = from - k as f64 * bar;
            start >= a.silence_start_ms as f64 && gridded(start, from) && (sung_ok || !sings_after(a, start))
        })
    };
    // Of the clean slices that skip no more than allowed, the one that goes round most seamlessly; an eight-bar one
    // (heard fewer times) first, as before where seams are not measured, and one nearer the end (less of the song
    // skipped) breaking near ties.
    let rank = |from: f64, l: usize| {
        let back = (last - from) / bar - l as f64;
        seam(&a.outro_bars, from, l, bar).unwrap_or(SEAM_TYPICAL) as f64 + if l == 8 { RAMP_LONG_WORTH } else { 0.0 } - RAMP_BACK_COST * back
    };
    let (from, looped, own) = [8usize, 4]
        .into_iter()
        .flat_map(|l| (l..=EXIT_BARS_BACK as usize + l).map(move |back| (last - back as f64 * bar, l)))
        .filter(|&(from, l)| from >= a.silence_start_ms as f64 && clean(from, l) && ending.skipped(from + l as f64 * bar) <= MAX_SKIP_MS as f64)
        .filter_map(|(from, l)| Some((from, l, own(from)?)))
        .filter(|&(_, l, k)| (played - k as f64) / l as f64 <= MAX_LOOP_PLAYS)
        .max_by(|x, y| rank(x.0, x.1).total_cmp(&rank(y.0, y.1)))?;
    let (loop_ms, own_ms) = (looped as f64 * bar, own as f64 * bar);
    let start = from - own_ms;
    // The incoming song locks on its first downbeat, its pickup (up to a bar) played before it.
    let music = music_start(Some(b), p.room.in_dur) as f64;
    let (beat_b, bar_b) = (60_000.0 / b.bpm, bar_ms(b));
    let at = b.beat_offset_ms + downbeat_from(music, b.beat_offset_ms, beat_b, bar_beats(b), b.downbeat_phase as i64) as f64 * beat_b;
    let pickup = (at - music).min(bar_b);
    if at - music > MAX_SKIP_MS as f64 {
        return None;
    }
    let mut m = blank(TransitionKind::LoopRamp, start.round() as i64, (at - pickup).round() as i64, 0, String::new());
    m.out_loop = Some(OutLoop { at_ms: own_ms.round() as i64, ms: loop_ms.round() as i64, rounds: None });
    m.out_glide = Some(Glide { window: Window { start_ms: 0, end_ms: glide_ms.round() as i64 }, to: to_a });
    let downbeat = (m.out_song_ms(glide_ms) / bar).ceil() * bar;
    let delay = glide_ms + (downbeat - m.out_song_ms(glide_ms)) / to_a;
    let bar_wall = bar / to_a;
    let blend = RAMP_BLEND_BARS * bar_wall;
    let dur = delay + blend;
    if dur > p.room.max_len as f64 || (p.room.in_dur as f64) - at < 2.0 * dur {
        return None;
    }
    // The incoming blend inside its grid, as the outgoing loop is.
    let in_kept = steady_run(&cleaned(&b.intro_beats)).to_vec();
    let in_end = at + blend * to_b;
    if !in_grid((b.intro_grid_from_ms, b.intro_grid_to_ms), at, in_end, beat_b)
        || !measured_or_beatless(&b.intro_bars, &b.intro_beats, &in_kept, at, in_end, bar_b, |_, g1| g1 <= b.silence_start_ms as f64)
    {
        return None;
    }
    m.duration_ms = dur.round() as i64;
    m.in_delay_ms = (delay - pickup / to_b).round() as i64;
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
        m.low_pass = Some(Sweep { start_ms: m.in_delay_ms, end_ms: half.round() as i64, from_hz: SWEEP_FROM_HZ, to_hz: RAMP_ROLL_HZ });
        let drain = RAMP_DRAIN_BARS * bar_wall;
        m.high_pass = Some(Sweep { start_ms: (dur - drain).round() as i64, end_ms: m.duration_ms, from_hz: RAMP_DRAIN_FROM_HZ, to_hz: RAMP_DRAIN_TO_HZ });
    }
    m.in_gain_db = loudness_trim(Some(a), Some(b), s);
    m.reason = format!(
        "loop ramp: {looped}-bar loop{}{} gliding {:.1} -> {:.1} BPM over {glide_bars:.0} bars, in locked for {RAMP_BLEND_BARS:.0}, back to {:.1} over {beats_back:.0} beats{}",
        if own > 0 { format!(" after {own} bars of its own") } else { String::new() },
        seam(&a.outro_bars, from, looped, bar).map_or(String::new(), |s| format!(", seam {s:.2}")),
        a.bpm,
        meet,
        b.bpm,
        tail(p.why_not)
    );
    if let Some(st) = shift {
        in_key(&mut m, st, b.bpm);
    }
    Some(Move { plan: m, worth: LOOP_RAMP_WORTH })
}

/// A loop ramp's slice ranks by its seam, this much more for eight bars, this much less a bar further from the end.
const RAMP_LONG_WORTH: f64 = 0.1;
const RAMP_BACK_COST: f64 = 0.01;

/// The least tempo gap a loop ramp crosses, percent; under it a stretch meets them. The most is the most two tempos are
/// moved together ([`super::SPLIT_MAX_PCT`]): further, the outgoing song races or drags.
const RAMP_MIN_GAP_PCT: f64 = 8.0;
/// The outgoing glide's rate (DJs ride 2 to 3 % a bar; BitChord 2.5 BPM a bar), and the incoming ease back after
/// (BitChord: below about 1 % a beat it reads as the groove, not the song changing speed).
pub(super) const RAMP_PCT_PER_BAR: f64 = 2.0;
pub(super) const EASE_PCT_PER_BEAT: f64 = 0.75;
const RAMP_BLEND_BARS: f64 = 8.0;
/// Most times a loop ramp's slice plays: more often, anything in it heard each time (a backing voice the voice
/// measure misses) turns robotic.
pub(super) const MAX_LOOP_PLAYS: f64 = 3.0;
/// Where the loop's low-pass closes to by the swap: each play darker than the last, so no two sound the same.
const RAMP_ROLL_HZ: f32 = 2_000.0;
const RAMP_DRAIN_BARS: f64 = 2.0;
const RAMP_DRAIN_FROM_HZ: f32 = 30.0;
const RAMP_DRAIN_TO_HZ: f32 = 1_500.0;
const RAMP_HIGHS_DB: f32 = -15.0;
const RAMP_HIGHS_HZ: f32 = 3_000.0;
/// A loopable bar is sung less than this share, next to none: any voice in a loop is heard again each time round. A
/// loop ramp's bar also has its beat and is no quieter than this against the song.
const LOOP_VOICE: f32 = 0.1;
const LOOP_LEVEL_DB: f32 = -8.0;
/// Bars before a loop unsung too: a line's held last note or a backing "ooh" trails it by up to a phrase, and the
/// voice curve, which follows a voice moving, misses a held note.
const LOOP_TAIL_BARS: usize = 4;

/// Whether `n` bars of `bars` from `from` loop without a voice: none of them sung, nor the [`LOOP_TAIL_BARS`] before
/// them, where they are profiled.
pub(super) fn unsung(bars: &[BarProfile], from: f64, n: usize, bar: f64) -> bool {
    (-(LOOP_TAIL_BARS as i64)..n as i64).all(|k| bar_at(bars, from + (k as f64 + 0.5) * bar).is_none_or(|x| x.voice < LOOP_VOICE))
}

/// Whether `bars` of the outgoing song from `from` loop without a voice ([`unsung`]), each one profiled and `fits`.
pub(super) fn loopable(a: &TrackAnalysis, from: f64, bars: usize, bar: f64, fits: impl Fn(&BarProfile) -> bool) -> bool {
    unsung(&a.outro_bars, from, bars, bar) && (0..bars).all(|i| bar_at(&a.outro_bars, from + (i as f64 + 0.5) * bar).is_some_and(&fits))
}

/// How seamlessly `n` bars of `bars` from `from` go round, 0..1, the best of three measured ways: the bar the song
/// plays after them against their first (LoopMaker's criterion), the bar before them against their last
/// (PyMusicLooper's other way round), and, for a texture that does not change from bar to bar (a drum intro before
/// its drop), how alike each of them is to the next. None when none is measured (an end analysed before bars kept
/// their repeats).
pub(super) fn seam(bars: &[BarProfile], from: f64, n: usize, bar: f64) -> Option<f32> {
    // Repeats count the stored bars: read at another tempo octave they would be the wrong ones.
    let spacing = bars.windows(2).find(|w| w[1].start_ms as f64 > from).map(|w| (w[1].start_ms - w[0].start_ms) as f64);
    if spacing.is_none_or(|s| (s / bar - 1.0).abs() > SEAM_BAR_SLACK) {
        return None;
    }
    let at = |k: f64| bar_at(bars, from + (k + 0.5) * bar);
    let repeat = |x: &BarProfile| if n >= 8 { x.repeat8 } else { x.repeat4 };
    let after = at(0.0).and_then(repeat);
    let before = at(-1.0).and_then(repeat);
    // One texture: each bar like the next, and in eight bars the first four like the four after them, so a slow build
    // (a riser, a filter opening) that drifts a little each bar is not taken for one.
    let alike = |k: usize, r: &dyn Fn(&BarProfile) -> Option<f32>| at(k as f64).and_then(r);
    let mut pairs = (0..n.saturating_sub(1)).map(|k| alike(k, &|x| x.repeat1)).collect::<Vec<_>>();
    if n >= 8 {
        pairs.extend((0..4).map(|k| alike(k, &|x| x.repeat4)));
    }
    let steady = pairs.into_iter().collect::<Option<Vec<f32>>>().and_then(|r| r.into_iter().reduce(f32::min));
    [after, before, steady].into_iter().flatten().reduce(f32::max)
}

/// The stored bars may differ from the planner's by this share before their repeats are not read.
const SEAM_BAR_SLACK: f64 = 0.1;

/// Whether `n` bars from `from` go round seamlessly enough to loop: [`seam`] at [`SEAM_MIN`] (unmeasured, as before
/// bars were measured for it, taken to), at a [`steady`] level.
pub(super) fn seamless(bars: &[BarProfile], from: f64, n: usize, bar: f64) -> bool {
    seam(bars, from, n, bar).is_none_or(|s| s >= SEAM_MIN) && steady(bars, from, n, bar)
}

/// A loop's seam at least this alike ([`seam`]); a typical one (with no seam measured, taken to be it); and what each
/// point of it off the typical is worth to a window, so a more seamless loop gains on the alternatives and a less
/// seamless one loses. Set on the lab's songs and listening: their loopable slices' seams have a median of 0.81;
/// under 0.8 a loop is heard to jump at the join (0.76), and lower the music moves on there (0.2, 0.6 to 0.67).
pub(super) const SEAM_MIN: f32 = 0.8;
pub(super) const SEAM_TYPICAL: f32 = 0.81;
pub(super) const SEAM_WORTH: f64 = 2.0;

/// Whether `n` bars from `from` keep within [`loudness::STEADY_DB`] of one another, and the last within
/// [`LOOP_WRAP_DB`] of the first: a slice of a fade-out or a build jumps back each time round. Taken to where they
/// are not all profiled.
fn steady(bars: &[BarProfile], from: f64, n: usize, bar: f64) -> bool {
    let Some(levels) = (0..n).map(|i| bar_at(bars, from + (i as f64 + 0.5) * bar).map(|x| x.level_db)).collect::<Option<Vec<f32>>>() else { return true };
    let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
    f64::from(hi - lo) <= crate::automix::loudness::STEADY_DB && levels.last().zip(levels.first()).is_none_or(|(l, f)| (l - f).abs() <= LOOP_WRAP_DB)
}

/// The most a loop's last bar may differ in level from its first, dB.
const LOOP_WRAP_DB: f32 = 1.5;
