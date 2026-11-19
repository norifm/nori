//! What plugins change of nori's values: overrides laid over the settings as they are derived, never
//! written to them, so the user's settings stay as the user set them and turning a plugin off brings
//! nori's own values back. A key is a setting's name (`crossfadeSec`), or a tunable's: a value with no
//! setting, fixed unless a plugin sets it.

use nori_model::TransitionPrefs;
use nori_player::stage::{Prepares, StageSpec};

use crate::settings::{set_by_name, StoredPrefs};

/// What the enabled plugins lay over nori's settings: their overrides (key and value as text, in the
/// plugins' order), their sound stages, and what they make of songs as they are read. Empty with
/// none enabled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub overrides: Vec<(String, String)>,
    pub stages: Vec<StageSpec>,
    pub preparing: Option<Prepares>,
}

/// The values with no setting, as the engine and the planner read them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tunables {
    pub speed: f32,
    pub pitch: f32,
    pub skip_silence: bool,
    pub previous_always_skips: bool,
    pub mix: MixTunables,
}

impl Default for Tunables {
    fn default() -> Self {
        Tunables { speed: 1.0, pitch: 1.0, skip_silence: false, previous_always_skips: false, mix: MixTunables::default() }
    }
}

/// AutoMix's limits set in place of its style's; None keeps the style's.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MixTunables {
    pub longest_s: Option<i32>,
    pub tempo_pct: Option<f32>,
    pub beat_match: Option<bool>,
    pub bass_swap: Option<bool>,
    pub filter_effects: Option<bool>,
    pub keep_pitch: Option<bool>,
}

impl MixTunables {
    /// `p` with the limits set laid over it.
    pub fn over(&self, p: TransitionPrefs) -> TransitionPrefs {
        TransitionPrefs {
            auto_mix_max_s: self.longest_s.unwrap_or(p.auto_mix_max_s),
            max_tempo_change_pct: self.tempo_pct.unwrap_or(p.max_tempo_change_pct),
            beat_match: self.beat_match.unwrap_or(p.beat_match),
            bass_swap: self.bass_swap.unwrap_or(p.bass_swap),
            filter_effects: self.filter_effects.unwrap_or(p.filter_effects),
            keep_pitch: self.keep_pitch.unwrap_or(p.keep_pitch),
            ..p
        }
    }
}

/// A tunable's key, the range a value is held to (0 or 1 for a switch), and where it goes.
type Tunable = (&'static str, f64, f64, fn(&mut Tunables, f64));

const TUNABLES: [Tunable; 10] = [
    ("playback.speed", 0.5, 2.0, |t, v| t.speed = v as f32),
    ("playback.pitch", 0.5, 2.0, |t, v| t.pitch = v as f32),
    ("silence.skip", 0.0, 1.0, |t, v| t.skip_silence = v >= 0.5),
    ("queue.previousAlwaysSkips", 0.0, 1.0, |t, v| t.previous_always_skips = v >= 0.5),
    // Up to the 40 s a song's end grid is measured over.
    ("autoMix.longestSec", 1.0, 40.0, |t, v| t.mix.longest_s = Some(v.round() as i32)),
    ("autoMix.tempoPct", 0.0, 12.0, |t, v| t.mix.tempo_pct = Some(v as f32)),
    ("autoMix.beatMatch", 0.0, 1.0, |t, v| t.mix.beat_match = Some(v >= 0.5)),
    ("autoMix.bassSwap", 0.0, 1.0, |t, v| t.mix.bass_swap = Some(v >= 0.5)),
    ("autoMix.filters", 0.0, 1.0, |t, v| t.mix.filter_effects = Some(v >= 0.5)),
    ("autoMix.keepPitch", 0.0, 1.0, |t, v| t.mix.keep_pitch = Some(v >= 0.5)),
];

/// A value as text: a number, or `true`/`false`.
fn number(text: &str) -> Option<f64> {
    match text {
        "true" => Some(1.0),
        "false" => Some(0.0),
        t => t.parse::<f64>().ok().filter(|v| v.is_finite()),
    }
}

impl Tunables {
    /// With `overrides` (key, value as text) applied in order, each held to its range; other keys
    /// are left for [`effective`].
    pub fn with(overrides: &[(String, String)]) -> Tunables {
        let mut t = Tunables::default();
        for (key, text) in overrides {
            let (Some((_, lo, hi, set)), Some(v)) = (TUNABLES.iter().find(|(k, ..)| k == key), number(text)) else { continue };
            set(&mut t, v.clamp(*lo, *hi));
        }
        t
    }
}

/// Whether a plugin may name `key` among its overrides.
pub fn overridable(key: &str) -> bool {
    TUNABLES.iter().any(|(k, ..)| *k == key) || crate::settings::row(key).is_some()
}

/// `stored` with the overrides of settings applied in order, each through its setting's own codec and
/// range; a value it doesn't take is skipped.
pub fn effective(stored: &StoredPrefs, overrides: &[(String, String)]) -> StoredPrefs {
    let mut p = stored.clone();
    for (key, text) in overrides {
        if let Some(c) = set_by_name(&p, key, text) {
            p = c.prefs;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn tunables_take_their_keys_within_range() {
        assert_eq!(Tunables::with(&[]), Tunables::default());
        let t = Tunables::with(&o(&[("playback.speed", "1.25"), ("playback.pitch", "9"), ("silence.skip", "true"), ("crossfadeSec", "4")]));
        assert_eq!(t, Tunables { speed: 1.25, pitch: 2.0, skip_silence: true, ..Tunables::default() });
        assert_eq!(Tunables::with(&o(&[("playback.speed", "1.5"), ("playback.speed", "0.75")])).speed, 0.75, "the last one wins");
        assert_eq!(Tunables::with(&o(&[("playback.speed", "NaN")])).speed, 1.0);
    }

    #[test]
    fn automix_limits_replace_the_styles_within_range() {
        let style = StoredPrefs::default().transition_prefs();
        assert_eq!(Tunables::with(&[]).mix.over(style), style);
        let mix = Tunables::with(&o(&[("autoMix.longestSec", "90"), ("autoMix.tempoPct", "3"), ("autoMix.bassSwap", "false")])).mix;
        let p = mix.over(style);
        assert_eq!((p.auto_mix_max_s, p.max_tempo_change_pct, p.bass_swap), (40, 3.0, false));
        assert_eq!((p.beat_match, p.keep_pitch, p.filter_effects), (style.beat_match, style.keep_pitch, style.filter_effects), "what isn't set stays the style's");
    }

    #[test]
    fn settings_are_overridden_without_being_written() {
        let stored = StoredPrefs::default();
        let p = effective(&stored, &o(&[("crossfadeSec", "6"), ("crossfadeSec", "99"), ("noSuchSetting", "1")]));
        assert_eq!(p.crossfade_sec, 12, "held to the setting's own range");
        assert_eq!(stored.crossfade_sec, StoredPrefs::default().crossfade_sec, "what is stored stays");
        assert!(overridable("crossfadeSec") && overridable("playback.speed"));
        assert!(!overridable("noSuchSetting") && !overridable("servers"));
    }
}
