//! What plugins change of nori's values: overrides laid over the settings as they are derived, never
//! written to them, so the user's settings stay as the user set them and turning a plugin off brings
//! nori's own values back. A key is a setting's name (`crossfadeSec`), or a tunable's: a value with no
//! setting, fixed unless a plugin sets it.

use nori_player::stage::StageSpec;

use crate::settings::{set_by_name, StoredPrefs};

/// What the enabled plugins lay over nori's settings: their overrides (key and value as text, in the
/// plugins' order) and their sound stages. Empty with none enabled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub overrides: Vec<(String, String)>,
    pub stages: Vec<StageSpec>,
}

/// The values with no setting, as the engine reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tunables {
    pub speed: f32,
    pub pitch: f32,
    pub skip_silence: bool,
    pub previous_always_skips: bool,
}

impl Default for Tunables {
    fn default() -> Self {
        Tunables { speed: 1.0, pitch: 1.0, skip_silence: false, previous_always_skips: false }
    }
}

/// A tunable's key, the range a value is held to (0 or 1 for a switch), and where it goes.
type Tunable = (&'static str, f64, f64, fn(&mut Tunables, f64));

const TUNABLES: [Tunable; 4] = [
    ("playback.speed", 0.5, 2.0, |t, v| t.speed = v as f32),
    ("playback.pitch", 0.5, 2.0, |t, v| t.pitch = v as f32),
    ("silence.skip", 0.0, 1.0, |t, v| t.skip_silence = v >= 0.5),
    ("queue.previousAlwaysSkips", 0.0, 1.0, |t, v| t.previous_always_skips = v >= 0.5),
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
        assert_eq!(t, Tunables { speed: 1.25, pitch: 2.0, skip_silence: true, previous_always_skips: false });
        assert_eq!(Tunables::with(&o(&[("playback.speed", "1.5"), ("playback.speed", "0.75")])).speed, 0.75, "the last one wins");
        assert_eq!(Tunables::with(&o(&[("playback.speed", "NaN")])).speed, 1.0);
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
