//! The AutoMix seams plugins fill ([`Mixing`]): whether two songs mix at all, the plan of a
//! transition, and an analysis refined. Each gets nori's answer and may give its own; nori checks
//! what it gives here and keeps its own answer when it doesn't hold.

use nori_player::types::{BassSwap, Sweep, TrackAnalysis, TransitionKind, TransitionPlan};

/// The kinds of transition a plugin may plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proposed {
    Gapless,
    Crossfade,
    BeatMatched,
    EchoOut,
    /// A short fade at the planned point.
    Cut,
}

/// A transition a plugin plans, as the plugin API has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proposal {
    pub kind: Proposed,
    pub out_start_ms: i64,
    pub duration_ms: i64,
    pub in_start_ms: i64,
    pub tempo_ratio: f64,
    pub bass_swap: bool,
    /// A low-pass sweep over the outgoing song.
    pub filter: bool,
}

/// A plugin's view of a song's analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct Refined {
    pub bpm: f64,
    pub bpm_confidence: f32,
    /// Camelot code, 0 unknown.
    pub key: i32,
    pub intro_end_ms: i64,
    pub outro_start_ms: i64,
    /// Beat times, ms.
    pub beats: Vec<i64>,
    pub lufs: f32,
}

/// Where plugins answer for AutoMix. None everywhere keeps nori's answers.
pub trait Mixing: Send + Sync {
    /// Whether songs `outgoing` and `incoming` mix at all; nori's answer is `nori` (its same-album rule).
    fn mixes(&self, outgoing: &str, incoming: &str, nori: bool) -> Option<bool>;
    /// The transition out of `outgoing` into `incoming`, given their analyses and nori's plan.
    fn plan(&self, outgoing: &str, incoming: &str, out: Option<&TrackAnalysis>, inc: Option<&TrackAnalysis>, nori: &TransitionPlan) -> Option<Proposal>;
    /// Song `song`'s analysis as a plugin refines it.
    fn refine(&self, song: &str, analysis: &TrackAnalysis) -> Option<Refined>;
}

/// The fade a cut takes, ms.
pub const CUT_MS: i64 = 30;
/// The shortest and longest overlap a plugin may plan, ms.
const MIN_MS: i64 = 20;
const MAX_MS: i64 = 60_000;

/// What a planned transition kind is to a plugin.
pub fn proposed_kind(kind: TransitionKind) -> Proposed {
    match kind {
        TransitionKind::Gapless => Proposed::Gapless,
        TransitionKind::EqualPowerFade | TransitionKind::MixRampFade => Proposed::Crossfade,
        TransitionKind::BeatMatched => Proposed::BeatMatched,
        TransitionKind::EchoOut => Proposed::EchoOut,
    }
}

/// `p` as a plan nori can play, out of a song of `out_ms` into one of `in_ms`, its tempo at most
/// `max_tempo_pct` off; nori's plan `nori` lends what the plugin API doesn't say (fades, gains, the
/// echo). None when it doesn't hold: a position outside a song, a length out of bounds, a tempo
/// change too large or where nothing is beat-matched, an echo nori didn't plan.
pub fn adopt(nori: &TransitionPlan, p: Proposal, out_ms: i64, in_ms: i64, max_tempo_pct: f32) -> Option<TransitionPlan> {
    let mut t = nori.clone();
    t.reason = "planned by a plugin".into();
    if p.kind == Proposed::Gapless {
        let d = 0;
        t.kind = TransitionKind::Gapless;
        (t.out_start_ms, t.in_start_ms, t.duration_ms, t.tempo_ratio, t.tempo_ramp_beats, t.tempo_ramp_ms) = (out_ms.max(0), 0, d, 1.0, 0, 0);
        (t.out_fade_start_ms, t.out_fade_end_ms, t.in_fade_start_ms, t.in_fade_end_ms) = (0, 0, 0, 0);
        (t.bass_swap, t.low_pass, t.high_pass, t.echo, t.out_loop_ms, t.vocal_duck) = (None, None, None, None, None, None);
        return Some(t);
    }
    let d = if p.kind == Proposed::Cut { CUT_MS } else { p.duration_ms };
    let tempo = if p.tempo_ratio.is_finite() { p.tempo_ratio } else { return None };
    let tempo_off = (tempo - 1.0).abs() * 100.0;
    let holds = (MIN_MS..=MAX_MS).contains(&d)
        && p.out_start_ms >= 0
        && p.out_start_ms + d <= out_ms
        && p.in_start_ms >= 0
        && p.in_start_ms as f64 + d as f64 * tempo <= in_ms as f64
        && tempo > 0.0
        && tempo_off <= max_tempo_pct.max(0.0) as f64 + 1e-9
        && (tempo == 1.0 || p.kind == Proposed::BeatMatched)
        && (p.kind != Proposed::EchoOut || nori.echo.is_some());
    if !holds {
        return None;
    }
    t.kind = match p.kind {
        Proposed::BeatMatched => TransitionKind::BeatMatched,
        Proposed::EchoOut => TransitionKind::EchoOut,
        _ => TransitionKind::EqualPowerFade,
    };
    // nori's relative points, stretched to the plugin's length.
    let nd = nori.duration_ms;
    let scale = |v: i64| if nd > 0 { (v as f64 * d as f64 / nd as f64).round() as i64 } else { v };
    let fades = (scale(nori.out_fade_start_ms), scale(nori.out_fade_end_ms), scale(nori.in_fade_start_ms), scale(nori.in_fade_end_ms));
    (t.out_fade_start_ms, t.out_fade_end_ms, t.in_fade_start_ms, t.in_fade_end_ms) = if nd > 0 { fades } else { (0, d, 0, d) };
    (t.out_start_ms, t.in_start_ms, t.duration_ms, t.tempo_ratio) = (p.out_start_ms, p.in_start_ms, d, tempo);
    if tempo == 1.0 {
        (t.tempo_ramp_beats, t.tempo_ramp_ms) = (0, 0);
    }
    t.bass_swap = p.bass_swap.then(|| match nori.bass_swap {
        Some(b) => BassSwap { at_ms: scale(b.at_ms), len_ms: scale(b.len_ms).max(1), cut_hz: b.cut_hz },
        None => BassSwap { at_ms: d * 3 / 8, len_ms: (d / 4).max(1), cut_hz: 200.0 },
    });
    t.low_pass = p.filter.then(|| match nori.low_pass {
        Some(s) => Sweep { start_ms: scale(s.start_ms), end_ms: scale(s.end_ms), ..s },
        None => Sweep { start_ms: 0, end_ms: d, from_hz: 20_000.0, to_hz: 300.0 },
    });
    t.high_pass = t.high_pass.map(|s| Sweep { start_ms: scale(s.start_ms), end_ms: scale(s.end_ms), ..s });
    t.echo = if t.kind == TransitionKind::EchoOut { nori.echo } else { None };
    // A loop or a duck nori placed for its own plan doesn't fit another.
    (t.out_loop_ms, t.vocal_duck) = (None, None);
    Some(t)
}

/// `a` as `r` refines it; None when `r` doesn't hold (a tempo, confidence, key, loudness or cue out
/// of range).
pub fn refined(a: &TrackAnalysis, r: &Refined) -> Option<TrackAnalysis> {
    let within = |ms: i64| (0..=a.duration_ms.max(0)).contains(&ms);
    let holds = r.bpm.is_finite()
        && (r.bpm == 0.0 || (40.0..=250.0).contains(&r.bpm))
        && (0.0..=1.0).contains(&r.bpm_confidence)
        && (0..=24).contains(&r.key)
        && within(r.intro_end_ms)
        && within(r.outro_start_ms)
        && r.intro_end_ms <= r.outro_start_ms
        && r.lufs.is_finite()
        && (-70.0..=0.0).contains(&r.lufs);
    if !holds {
        return None;
    }
    let mut t = a.clone();
    (t.bpm, t.bpm_confidence, t.key, t.intro_end_ms, t.outro_start_ms, t.lufs) = (r.bpm, r.bpm_confidence, r.key, r.intro_end_ms, r.outro_start_ms, r.lufs);
    if let (Some(first), true) = (r.beats.iter().copied().filter(|b| *b >= 0).min(), r.bpm > 0.0) {
        t.beat_offset_ms = (first as f64).rem_euclid(60_000.0 / r.bpm);
    }
    Some(t)
}

/// `a`'s beat times, ms, from its grid.
pub fn beat_times(a: &TrackAnalysis) -> Vec<i64> {
    if a.bpm <= 0.0 || !a.bpm.is_finite() {
        return Vec::new();
    }
    let period = 60_000.0 / a.bpm;
    (0..).map(|n| a.beat_offset_ms + n as f64 * period).take_while(|ms| *ms < a.duration_ms as f64).map(|ms| ms.round() as i64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nori() -> TransitionPlan {
        let mut t = nori_player::automix::plan::plan(None, None, 200_000, 200_000, &Default::default());
        t.kind = TransitionKind::EqualPowerFade;
        (t.out_start_ms, t.in_start_ms, t.duration_ms) = (190_000, 0, 8_000);
        (t.out_fade_start_ms, t.out_fade_end_ms, t.in_fade_start_ms, t.in_fade_end_ms) = (0, 8_000, 0, 4_000);
        t
    }

    fn proposal(kind: Proposed, out_start_ms: i64, duration_ms: i64, in_start_ms: i64, tempo_ratio: f64) -> Proposal {
        Proposal { kind, out_start_ms, duration_ms, in_start_ms, tempo_ratio, bass_swap: false, filter: false }
    }

    #[test]
    fn a_plugins_plan_is_played_only_when_it_holds() {
        let ok = adopt(&nori(), proposal(Proposed::Crossfade, 180_000, 4_000, 2_000, 1.0), 200_000, 200_000, 6.0).expect("it holds");
        assert_eq!((ok.out_start_ms, ok.duration_ms, ok.in_start_ms), (180_000, 4_000, 2_000));
        assert_eq!((ok.out_fade_end_ms, ok.in_fade_end_ms), (4_000, 2_000), "nori's fades, stretched to its length");
        let cases = [
            ("past the outgoing end", proposal(Proposed::Crossfade, 198_000, 4_000, 0, 1.0)),
            ("past the incoming end", proposal(Proposed::Crossfade, 100_000, 4_000, 198_000, 1.0)),
            ("too long", proposal(Proposed::Crossfade, 100_000, 61_000, 0, 1.0)),
            ("too short", proposal(Proposed::Crossfade, 100_000, 5, 0, 1.0)),
            ("a tempo change past the setting", proposal(Proposed::BeatMatched, 100_000, 4_000, 0, 1.1)),
            ("a tempo change without beats matched", proposal(Proposed::Crossfade, 100_000, 4_000, 0, 1.02)),
            ("an echo nori didn't plan", proposal(Proposed::EchoOut, 100_000, 4_000, 0, 1.0)),
            ("no tempo", proposal(Proposed::BeatMatched, 100_000, 4_000, 0, f64::NAN)),
        ];
        for (why, p) in cases {
            assert_eq!(adopt(&nori(), p, 200_000, 200_000, 6.0), None, "{why}");
        }
        let matched = adopt(&nori(), Proposal { bass_swap: true, filter: true, ..proposal(Proposed::BeatMatched, 100_000, 8_000, 0, 1.04) }, 200_000, 200_000, 6.0).unwrap();
        assert_eq!(matched.kind, TransitionKind::BeatMatched);
        assert!(matched.bass_swap.is_some() && matched.low_pass.is_some());
        let cut = adopt(&nori(), proposal(Proposed::Cut, 150_000, 9_999, 0, 1.0), 200_000, 200_000, 6.0).unwrap();
        assert_eq!(cut.duration_ms, CUT_MS);
        let gapless = adopt(&nori(), proposal(Proposed::Gapless, 0, 0, 0, 1.0), 200_000, 200_000, 6.0).unwrap();
        assert_eq!((gapless.kind, gapless.out_start_ms, gapless.duration_ms), (TransitionKind::Gapless, 200_000, 0));
    }

    #[test]
    fn a_refined_analysis_is_kept_only_when_it_holds() {
        let a = TrackAnalysis { song_id: "a".into(), duration_ms: 200_000, bpm: 120.0, lufs: -12.0, ..Default::default() };
        let r = Refined { bpm: 125.0, bpm_confidence: 0.9, key: 8, intro_end_ms: 16_000, outro_start_ms: 180_000, beats: vec![250, 730], lufs: -9.0 };
        let t = refined(&a, &r).expect("it holds");
        assert_eq!((t.bpm, t.key, t.intro_end_ms, t.lufs), (125.0, 8, 16_000, -9.0));
        assert!((t.beat_offset_ms - 250.0).abs() < 1e-9, "the grid starts at its first beat");
        for bad in [Refined { bpm: 400.0, ..r.clone() }, Refined { key: 30, ..r.clone() }, Refined { outro_start_ms: 300_000, ..r.clone() }, Refined { intro_end_ms: 190_000, ..r.clone() }, Refined { lufs: f32::NAN, ..r.clone() }] {
            assert_eq!(refined(&a, &bad), None, "{bad:?}");
        }
        assert_eq!(beat_times(&TrackAnalysis { bpm: 120.0, beat_offset_ms: 100.0, duration_ms: 1_700, ..Default::default() }), vec![100, 600, 1100, 1600]);
    }
}
