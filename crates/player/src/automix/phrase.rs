//! Where an end's phrases start (docs/research/automix-novel.md, section 3): a dynamic program over its bars that
//! places phrase starts where the music changes, preferring the 8- and 16-bar phrases songs are built of and letting
//! rock's shorter ones through where the change is strong.

use crate::types::BarProfile;

/// What a phrase of each length costs (log prior): 8 bars nothing, then 16, 4, 12, 6 and 2; any other much more.
fn length_cost(bars: usize) -> f64 {
    match bars {
        8 => 0.0,
        16 => 0.3,
        4 => 0.5,
        12 => 1.5,
        6 => 2.5,
        2 => 3.0,
        _ => 5.0,
    }
}

/// The longest phrase considered, bars.
const LONGEST: usize = 16;
/// How much a bar's evidence counts against the prior.
const EVIDENCE_WEIGHT: f64 = 1.0;
/// Each part of a bar's evidence: its change (chords, level, lows, beat, voice together), its level and its lows
/// stepping, and singing starting or stopping on it.
const CHANGE_WEIGHT: f64 = 1.0;
const LEVEL_WEIGHT: f64 = 0.7;
const LOWS_WEIGHT: f64 = 0.7;
const VOICE_WEIGHT: f64 = 1.0;
/// Bars each side a step in level or lows is read over.
const STEP_SPAN: usize = 2;
/// A bar sung at least this share counts as sung.
const SUNG: f32 = 0.5;
/// The least spread each part's scores are taken over: below it, music that barely changes would read its small
/// wobbles as changes as strong as a real one.
const CHANGE_SPREAD: f64 = 0.1;
const LEVEL_SPREAD_DB: f64 = 1.0;
const LOWS_SPREAD_DB: f64 = 2.0;
/// A phrase start is a new section where the music changes this much: the bar's change, a step in level or lows, or
/// singing starting or stopping.
const SECTION_CHANGE: f64 = 0.3;
const SECTION_LEVEL_DB: f64 = 3.0;
const SECTION_LOWS_DB: f64 = 6.0;

/// Standard scores: each value's distance from the mean in standard deviations, at least `floor`.
fn z(v: &[f64], floor: f64) -> Vec<f64> {
    let n = v.len().max(1) as f64;
    let mean = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt().max(floor);
    v.iter().map(|x| (x - mean) / sd).collect()
}

/// How strongly each bar starts something new, and whether it starts a new section.
fn evidence(bars: &[BarProfile]) -> (Vec<f64>, Vec<bool>) {
    let n = bars.len();
    let step = |f: &dyn Fn(&BarProfile) -> f32| -> Vec<f64> {
        (0..n)
            .map(|b| {
                let (before, after) = (&bars[b.saturating_sub(STEP_SPAN)..b], &bars[b..(b + STEP_SPAN).min(n)]);
                if before.is_empty() {
                    return 0.0;
                }
                let mean = |side: &[BarProfile]| side.iter().map(|x| f(x) as f64).sum::<f64>() / side.len() as f64;
                (mean(after) - mean(before)).abs()
            })
            .collect()
    };
    let changes: Vec<f64> = bars.iter().map(|b| b.change as f64).collect();
    let (level_steps, lows_steps) = (step(&|b| b.level_db), step(&|b| b.low_db));
    let (change, level, lows) = (z(&changes, CHANGE_SPREAD), z(&level_steps, LEVEL_SPREAD_DB), z(&lows_steps, LOWS_SPREAD_DB));
    let voice = |b: usize| b > 0 && (bars[b].voice >= SUNG) != (bars[b - 1].voice >= SUNG);
    let e = (0..n).map(|b| CHANGE_WEIGHT * change[b] + LEVEL_WEIGHT * level[b] + LOWS_WEIGHT * lows[b] + if voice(b) { VOICE_WEIGHT } else { 0.0 }).collect();
    let new = (0..n).map(|b| changes[b] >= SECTION_CHANGE || level_steps[b] >= SECTION_LEVEL_DB || lows_steps[b] >= SECTION_LOWS_DB || voice(b)).collect();
    (e, new)
}

/// The phrase starts of `bars` ([`starts`]) where the music changes enough to start a new section.
pub(crate) fn sections(bars: &[BarProfile], from_start: bool) -> Vec<usize> {
    let (e, new) = evidence(bars);
    placed(&e, from_start).into_iter().filter(|&b| new[b]).collect()
}

/// The bars of `bars` (in order) that start a phrase, by index. `from_start`: the first bar starts the song, so a
/// phrase; else the stretch starts mid-phrase, as an ending's does. The last phrase runs past the stretch.
pub(crate) fn starts(bars: &[BarProfile], from_start: bool) -> Vec<usize> {
    placed(&evidence(bars).0, from_start)
}

/// The phrase starts the dynamic program places on bars of evidence `e`.
fn placed(e: &[f64], from_start: bool) -> Vec<usize> {
    let n = e.len();
    if n == 0 {
        return Vec::new();
    }
    // best[b]: the best score of phrases starting at bar b, all earlier ones placed; from[b] the start before it.
    let mut best = vec![f64::NEG_INFINITY; n];
    let mut from: Vec<Option<usize>> = vec![None; n];
    for b in 0..n {
        // The stretch's first phrase: whole from the song's start, else any part of one.
        if b == 0 || (!from_start && b < LONGEST) {
            best[b] = EVIDENCE_WEIGHT * e[b];
        }
        for len in 1..=LONGEST.min(b) {
            let s = best[b - len] - length_cost(len) + EVIDENCE_WEIGHT * e[b];
            if s > best[b] {
                (best[b], from[b]) = (s, Some(b - len));
            }
        }
    }
    // The last phrase may run on past the stretch: any start within the longest phrase of its end.
    let Some(mut b) = (n.saturating_sub(LONGEST)..n).max_by(|x, y| best[*x].total_cmp(&best[*y])) else { return Vec::new() };
    let mut out = vec![b];
    while let Some(a) = from[b] {
        out.push(a);
        b = a;
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bars with (level, voice, change) each.
    fn bars(n: usize, f: impl Fn(usize) -> (f32, f32, f32)) -> Vec<BarProfile> {
        (0..n)
            .map(|i| {
                let (level_db, voice, change) = f(i);
                BarProfile { start_ms: 2_000 * i as i64, level_db, low_db: level_db, pulse: 1.0, voice, change }
            })
            .collect()
    }

    /// Sections changing every eight bars are found; with nothing changing, phrases of eight from the song's start.
    #[test]
    fn phrases_follow_the_sections() {
        let sections = bars(32, |i| ([-6.0, 0.0, -3.0, 0.0][i / 8], 0.0, 0.0));
        assert_eq!(starts(&sections, true), vec![0, 8, 16, 24]);
        let flat = bars(32, |_| (0.0, 0.0, 0.0));
        assert_eq!(starts(&flat, true), vec![0, 8, 16, 24]);
    }

    /// A rock song's six-bar verse after a two-bar pickup: strong changes win over the eight-bar habit, and a voice
    /// coming in starts a phrase.
    #[test]
    fn strong_changes_beat_the_habit() {
        let pickup = bars(24, |i| {
            let level = if i < 2 { -12.0 } else if i < 8 { 0.0 } else { -4.0 };
            (level, if (8..16).contains(&i) { 1.0 } else { 0.0 }, if i == 2 || i == 8 { 1.0 } else { 0.0 })
        });
        let found = starts(&pickup, true);
        assert!(found.starts_with(&[0, 2, 8]), "{found:?}");
        assert!(found.contains(&16), "{found:?}");
    }

    /// Of the phrases placed, only those where the music changes start sections; music that wobbles a little starts
    /// none.
    #[test]
    fn sections_need_a_change() {
        let one = bars(32, |i| (if i < 6 { -12.0 } else { 0.0 }, 0.0, if i == 6 { 1.0 } else { 0.0 }));
        assert_eq!(sections(&one, true), vec![6]);
        let wobbling = bars(32, |i| ([0.0, 0.4, -0.3, 0.2, -0.5][i % 5], 0.0, [0.02, 0.08, 0.01][i % 3]));
        assert_eq!(sections(&wobbling, true), Vec::<usize>::new());
    }

    /// An ending's stretch starts mid-phrase: its first start is where the music changes, not its first bar.
    #[test]
    fn an_ending_starts_mid_phrase() {
        let ending = bars(32, |i| (if (i + 3) % 16 < 8 { 0.0 } else { -5.0 }, 0.0, 0.0));
        let found = starts(&ending, false);
        assert_eq!(found.first(), Some(&5), "{found:?}");
    }
}
