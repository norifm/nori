//! The settings as a model for a client's settings screen: each setting's spec by name, current values
//! in the options' form, and derived state. Layout and wording are the client's.

use std::collections::HashMap;

use nori_automix::beat_model;
use nori_player::automix::beats;

use crate::lyrics_sources;
use crate::codec::K;
use crate::settings_store::Settings;
use crate::settings::{row, value_of_special, SettingChange, StoredPrefs, ROWS, SPECIAL_SPECS};

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SettingKind {
    /// "true" or "false".
    Switch,
    /// One of [`SettingSpec::options`] (for the music folder: the server's folder ids, "" for all).
    Choice,
    /// Any of [`SettingSpec::options`], independent of each other: the value names those on, comma
    /// separated in the options' order ("" for none).
    Flags,
    /// A number from [`SettingSpec::min`] to [`SettingSpec::max`].
    Level,
    Text,
    /// One of [`SettingSpec::options`], as ARGB numbers.
    Colour,
}

/// One setting: its name for [`setting_set`], kind, offered values, range and default. Values are in
/// the form [`SettingsState::values`] uses, so they compare as strings.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingSpec {
    pub name: String,
    pub kind: SettingKind,
    pub options: Vec<String>,
    pub min: f32,
    pub max: f32,
    pub default: String,
}

fn spec(name: &str, k: &K) -> SettingSpec {
    let (kind, options, (min, max)): (SettingKind, Vec<String>, (f32, f32)) = match k {
        K::Switch => (SettingKind::Switch, vec!["false".into(), "true".into()], (0.0, 0.0)),
        K::Choice(o) => (SettingKind::Choice, o.iter().map(|s| s.to_string()).collect(), (0.0, 0.0)),
        K::Flags(o) => (SettingKind::Flags, o.iter().map(|s| s.to_string()).collect(), (0.0, 0.0)),
        K::Level(lo, hi) => (SettingKind::Level, Vec::new(), (*lo, *hi)),
        K::Text => (SettingKind::Text, Vec::new(), (0.0, 0.0)),
        K::Colour => (SettingKind::Colour, nori_look::theme::ACCENTS.iter().map(|c| (*c as i64).to_string()).collect(), (0.0, 0.0)),
    };
    let default = value_of(&StoredPrefs::default(), name).unwrap_or_default();
    SettingSpec { name: name.into(), kind, options, min, max, default }
}

/// Every offered setting, in a stable order: the table's, then [`SPECIAL_SPECS`].
pub fn specs() -> Vec<SettingSpec> {
    let table = ROWS.iter().filter_map(|r| Some(spec(r.name?, r.spec.as_ref()?)));
    table.chain(SPECIAL_SPECS.iter().map(|(n, k)| spec(n, k))).collect()
}

/// A setting's value in its options' form ("1", "0.75"). Lookup switches read off while the master
/// lookups switch is off.
pub fn value_of(p: &StoredPrefs, name: &str) -> Option<String> {
    if let Some(v) = value_of_special(p, name) {
        return Some(v);
    }
    let r = row(name)?;
    Some(if r.lookups && !p.third_party_lookups { false.to_string() } else { (r.show)(p) })
}

/// One lyrics service in rank order. Changed by name with `lyricsService:<id>`, `lyricsPlace`
/// (`<id>:<place>`) and `lyricsMove` (`<id>:-1`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LyricsSource {
    pub id: String,
    pub on: bool,
    /// Asked only with the PaxSenix key.
    pub needs_key: bool,
}

/// The beat model's download state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum BeatModel {
    /// Not in this build.
    Unavailable,
    /// Not downloaded yet; fetched the next time AutoMix measures a song.
    Absent,
    WaitingForWifi,
    Downloading,
    Ready,
    Failed { why: nori_automix::beat_model::BeatFailure },
}

/// Settings screen state derived by the core's rules.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingsState {
    /// Every setting's value ([`value_of`]), by name.
    pub values: HashMap<String, String>,
    /// Bit-perfect DAC output: transitions, silence skipping and the sound chain are out of the path.
    pub untouched: bool,
    pub sound_chain_on: bool,
    /// Offload is wanted but off because the sound chain is needed.
    pub offload_paused: bool,
    /// Every lyrics service in rank order.
    pub lyrics_sources: Vec<LyricsSource>,
    pub beat_model: BeatModel,
    /// Approximate beat model download size, MB.
    pub beat_model_mb: u32,
}

/// The output as the platform reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Output {
    /// A USB DAC in bit-perfect mode.
    pub dac_bit_perfect: bool,
    /// Something USB is attached.
    pub usb: bool,
}

fn beat_model_now(model: &beat_model::ModelFile) -> BeatModel {
    if !beats::AVAILABLE {
        return BeatModel::Unavailable;
    }
    match model.state() {
        beat_model::State::Absent => BeatModel::Absent,
        beat_model::State::WaitingForWifi => BeatModel::WaitingForWifi,
        beat_model::State::Downloading => BeatModel::Downloading,
        // Ready only while the file is there.
        beat_model::State::Ready if model.ready().is_some() => BeatModel::Ready,
        beat_model::State::Ready => BeatModel::Absent,
        beat_model::State::Failed(why) => BeatModel::Failed { why },
    }
}

/// [`SettingsState`] for these settings and this output.
pub fn state(p: &StoredPrefs, out: Output, model: &beat_model::ModelFile) -> SettingsState {
    let dsp = p.sound_chain_on();
    let prefs = nori_model::AudioPrefs {
        dsp,
        skip_silence: p.skip_silence,
        offload: p.offload,
        crossfade_s: p.crossfade_sec,
        auto_mix: p.auto_mix,
        speed: p.speed,
        pitch: p.pitch,
    };
    // A refused offload is only known to the playback service.
    let output = nori_model::OutputState { hi_res: p.hi_res, bit_perfect: out.dac_bit_perfect, usb: out.usb, offload_refused: false };
    let policy = nori_player::policy::audio_policy(&prefs, &output);
    let on = lyrics_sources::switched_on(p);
    let lyrics_sources = lyrics_sources::complete_order(&p.lyrics_order)
        .into_iter()
        .map(|s| LyricsSource { id: s.name().into(), on: on.contains(&s), needs_key: s.needs_key() })
        .collect();
    SettingsState {
        values: specs().into_iter().filter_map(|s| Some((s.name.clone(), value_of(p, &s.name)?))).collect(),
        untouched: out.dac_bit_perfect,
        sound_chain_on: dsp,
        offload_paused: !out.usb && p.offload && !policy.offload,
        lyrics_sources,
        beat_model: beat_model_now(model),
        beat_model_mb: beat_model::SIZE_MB,
    }
}

/// The settings that differ from their defaults, one `name = value` per line, for a problem report.
/// Text settings (keys) are left out.
pub fn changed(p: &StoredPrefs) -> String {
    specs()
        .into_iter()
        .filter(|s| s.kind != SettingKind::Text)
        .filter_map(|s| {
            let v = value_of(p, &s.name)?;
            (v != s.default).then(|| format!("{} = {v}\n", s.name))
        })
        .collect()
}


#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn setting_specs() -> Vec<SettingSpec> {
    specs()
}

/// A [`SettingKind::Flags`] value with `flag` switched: on when it was off, off when on.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn flag_toggled(value: String, flag: String) -> String {
    let mut on: Vec<&str> = value.split(',').filter(|f| !f.is_empty()).collect();
    match on.iter().position(|f| *f == flag) {
        Some(i) => {
            on.remove(i);
        }
        None => on.push(&flag),
    }
    on.join(",")
}

/// Whether `flag` is on in a [`SettingKind::Flags`] value.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn flag_on(value: String, flag: String) -> bool {
    value.split(',').any(|f| f == flag)
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Settings {
    /// [`changed`] for the live settings.
    pub fn settings_changed(&self) -> String {
        changed(&self.current().unwrap_or_default())
    }

    /// [`SettingsState`] for the live settings and the platform's output.
    pub fn settings_state(&self, dac_bit_perfect: bool, usb: bool) -> SettingsState {
        state(&self.current().unwrap_or_default(), Output { dac_bit_perfect, usb }, &self.model)
    }

    /// A change by name kept in the live settings (`Settings::edit_by_name`).
    pub fn setting_set(&self, name: String, value: String) -> Option<SettingChange> {
        self.edit_by_name(&name, &value)
    }
}

/// Whether the interface is dark for the theme setting and the system's mode.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn theme_is_dark(theme: crate::settings::ThemeMode, system_dark: bool) -> bool {
    use crate::settings::ThemeMode;
    match theme {
        ThemeMode::System => system_dark,
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    }
}

/// Whether the turn of the phone hides a bar: `wide` is sideways.
fn turned_away(hide: &[crate::settings::HideBarWhen], wide: bool) -> bool {
    use crate::settings::HideBarWhen::*;
    hide.contains(if wide { &Sideways } else { &Upright })
}

/// Whether a system bar is hidden: while any of `hide` holds. `wide` is sideways, `protecting` is
/// burn-in protection at work.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn bar_hidden(hide: Vec<crate::settings::HideBarWhen>, wide: bool, protecting: bool) -> bool {
    turned_away(&hide, wide) || protecting && hide.contains(&crate::settings::HideBarWhen::Protected)
}

/// Whether the page keeps a bar's room while the bar is away: where it comes and goes in front of the
/// page with burn-in protection rather than with a turn of the phone, so nothing on the page moves
/// when it does.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn bar_room_kept(hide: Vec<crate::settings::HideBarWhen>, wide: bool) -> bool {
    hide.contains(&crate::settings::HideBarWhen::Protected) && !turned_away(&hide, wide)
}

/// Whether the screen is kept on: while any of `keep` holds. `wide` is sideways.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn keep_awake(keep: Vec<crate::settings::AwakeWhen>, wide: bool, charging: bool) -> bool {
    use crate::settings::AwakeWhen::*;
    keep.iter().any(|k| match k {
        Sideways => wide,
        Upright => !wide,
        Charging => charging,
    })
}

/// How long burn-in protection waits for a touch before dimming; `None`: it never dims.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn dim_after_ms(idle: crate::settings::DimIdle) -> Option<i64> {
    use crate::settings::DimIdle::*;
    match idle {
        Never => None,
        HalfMinute => Some(30_000),
        Minute => Some(60_000),
        FiveMinutes => Some(300_000),
    }
}

/// The share of light burn-in protection takes off an untouched screen.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn dim_amount(level: crate::settings::DimLevel) -> f32 {
    use crate::settings::DimLevel::*;
    match level {
        Light => 0.3,
        Medium => 0.5,
        Strong => 0.7,
    }
}

/// Whether a car screen puts the controls on the left, given where the car says the driver sits.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn controls_on_left(side: crate::settings::ControlsSide, driver_on_left: bool) -> bool {
    use crate::settings::ControlsSide::*;
    match side {
        Driver => driver_on_left,
        Left => true,
        Right => false,
    }
}

/// Whether the setting depends on charging (only then is the charger watched).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn keep_awake_watches_charging(keep: Vec<crate::settings::AwakeWhen>) -> bool {
    use crate::settings::AwakeWhen::*;
    keep.contains(&Charging) && !(keep.contains(&Sideways) && keep.contains(&Upright))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{set_by_name, SavedServer};

    #[test]
    fn a_flag_toggles_on_and_off_in_its_set() {
        let p = StoredPrefs::default();
        let toggled = |name: &str, flag: &str| {
            let v = flag_toggled(value_of(&p, name).unwrap(), flag.into());
            value_of(&set_by_name(&p, name, &v).unwrap().prefs, name).unwrap()
        };
        assert_eq!(toggled("coverColors", "PAGES"), "PLAYER");
        assert_eq!(toggled("keepAwake", "CHARGING"), "CHARGING");
        assert_eq!(toggled("hideStatusBar", "PROTECTED"), "SIDEWAYS,PROTECTED");
        assert!(flag_on("SIDEWAYS,PROTECTED".into(), "PROTECTED".into()) && !flag_on("SIDEWAYS".into(), "SIDE".into()));
    }

    #[test]
    fn changed_report_skips_text() {
        let p = StoredPrefs::default();
        assert_eq!(changed(&p), "");
        let switch = specs().into_iter().find(|s| s.kind == SettingKind::Switch).unwrap();
        let other = if switch.default == "true" { "false" } else { "true" };
        let p = set_by_name(&p, &switch.name, other).unwrap().prefs;
        let text = specs().into_iter().find(|s| s.kind == SettingKind::Text).unwrap();
        let p = set_by_name(&p, &text.name, "secret-key").unwrap().prefs;
        let said = changed(&p);
        assert!(said.lines().any(|l| l == format!("{} = {other}", switch.name)), "{said}");
        assert!(!said.contains("secret-key"), "{said}");
    }

    /// Every set of `all`.
    fn sets<T: Copy>(all: &[T]) -> Vec<Vec<T>> {
        (0..1u32 << all.len()).map(|m| all.iter().enumerate().filter(|(i, _)| m & (1 << i) != 0).map(|(_, t)| *t).collect()).collect()
    }

    #[test]
    fn a_bar_hides_while_any_chosen_condition_holds() {
        use crate::settings::HideBarWhen::*;
        for h in sets(&[Sideways, Upright, Protected]) {
            for (wide, protecting) in [(false, false), (false, true), (true, false), (true, true)] {
                let any = h.iter().any(|c| match c {
                    Sideways => wide,
                    Upright => !wide,
                    Protected => protecting,
                });
                assert_eq!(bar_hidden(h.clone(), wide, protecting), any, "{h:?} wide {wide} protecting {protecting}");
                // The room is kept where protection alone decides.
                let follows = bar_hidden(h.clone(), wide, true) != bar_hidden(h.clone(), wide, false);
                assert_eq!(bar_room_kept(h.clone(), wide), follows, "{h:?} wide {wide}");
            }
        }
    }

    #[test]
    fn the_screen_stays_on_while_any_chosen_condition_holds() {
        use crate::settings::AwakeWhen::*;
        for k in sets(&[Sideways, Upright, Charging]) {
            for (wide, charging) in [(false, false), (false, true), (true, false), (true, true)] {
                let any = k.iter().any(|c| match c {
                    Sideways => wide,
                    Upright => !wide,
                    Charging => charging,
                });
                assert_eq!(keep_awake(k.clone(), wide, charging), any, "{k:?} wide {wide} charging {charging}");
            }
            // The charger is watched where it decides.
            let decides = [false, true].into_iter().any(|wide| keep_awake(k.clone(), wide, true) != keep_awake(k.clone(), wide, false));
            assert_eq!(keep_awake_watches_charging(k.clone()), decides, "{k:?}");
        }
    }

    #[test]
    fn dimming_rules() {
        use crate::settings::DimIdle::*;
        use crate::settings::DimLevel::*;
        let waits = [HalfMinute, Minute, FiveMinutes].map(|i| dim_after_ms(i).unwrap());
        assert!(dim_after_ms(Never).is_none() && waits.windows(2).all(|w| w[0] < w[1]), "{waits:?}");
        let amounts = [Light, Medium, Strong].map(dim_amount);
        assert!(amounts.windows(2).all(|w| w[0] < w[1]) && amounts.iter().all(|a| (0.0..1.0).contains(a)), "{amounts:?}");
    }

    #[test]
    fn offered_values() {
        let p = StoredPrefs {
            servers: vec![SavedServer { id: "a".into(), ..SavedServer::default() }],
            active_server_id: "a".into(),
            ..StoredPrefs::default()
        };
        let all = specs();
        assert!(all.len() > 70, "{}", all.len());
        let mut names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len(), "each once");
        for s in &all {
            assert!(value_of(&p, &s.name).is_some(), "{} has no value", s.name);
            // Each value offered is taken by name, and reads back as itself.
            for o in &s.options {
                let after = set_by_name(&p, &s.name, o).unwrap_or_else(|| panic!("{} does not take {o}", s.name)).prefs;
                assert_eq!(value_of(&after, &s.name).as_deref(), Some(o.as_str()), "{} = {o}", s.name);
            }
            // Out of the box, a choice's value is one it offers.
            if s.kind == SettingKind::Choice && !s.options.is_empty() {
                assert!(s.options.contains(&s.default), "{}: {} is not offered", s.name, s.default);
            }
            if s.kind == SettingKind::Level {
                assert!(s.min < s.max, "{}", s.name);
                let mid = ((s.min + s.max) / 2.0).to_string();
                assert!(set_by_name(&p, &s.name, &mid).is_some(), "{}", s.name);
                // Asked past its ends, a level is held at them.
                for (asked, end) in [(s.min - 100.0, s.min), (s.max + 100.0, s.max)] {
                    let held = set_by_name(&p, &s.name, &asked.to_string()).unwrap().prefs;
                    assert_eq!(value_of(&held, &s.name).and_then(|v| v.parse::<f32>().ok()), Some(end), "{} = {asked}", s.name);
                }
            }
        }
        assert!(set_by_name(&p, "lyricsService:LRCLIB", "false").is_some());

        // Values match option form.
        let d = StoredPrefs::default();
        assert_eq!(value_of(&StoredPrefs { pitch: 0.9, ..d.clone() }, "pitch").as_deref(), Some("0.9"));
        assert_eq!(value_of(&d, "nope"), None);
        // In effect: off while looking things up is off.
        let off = StoredPrefs { third_party_lookups: false, ..d.clone() };
        assert_eq!(value_of(&d, "lyricsOnline").as_deref(), Some("true"));
        assert_eq!(value_of(&off, "lyricsOnline").as_deref(), Some("false"));
        assert_eq!(value_of(&off, "autoEqDownload").as_deref(), Some("false"));
    }

    #[test]
    fn state_rules() {
        let d = StoredPrefs::default();
        let s = state(&d, Output::default(), &beat_model::ModelFile::default());
        assert!(!s.untouched && !s.sound_chain_on && !s.offload_paused);
        assert_eq!(s.values["crossfadeSec"], d.crossfade_sec.to_string());
        assert!(!state(&StoredPrefs { hi_res: true, ..d.clone() }, Output::default(), &beat_model::ModelFile::default()).untouched, "high quality output keeps the chain");
        assert!(state(&d, Output { dac_bit_perfect: true, usb: true }, &beat_model::ModelFile::default()).untouched);
        assert!(state(&StoredPrefs { mono: true, ..d.clone() }, Output::default(), &beat_model::ModelFile::default()).sound_chain_on);
        // The battery saver stands down while an effect is on, but not over USB, where it is not offered.
        let eq = StoredPrefs { eq_enabled: true, offload: true, ..d.clone() };
        assert!(state(&eq, Output::default(), &beat_model::ModelFile::default()).offload_paused);
        assert!(!state(&eq, Output { dac_bit_perfect: false, usb: true }, &beat_model::ModelFile::default()).offload_paused);
        // No processing on this output: the effects are kept but out of the path, so offload comes back.
        let none = state(&StoredPrefs { sound_bypass: true, ..eq.clone() }, Output::default(), &beat_model::ModelFile::default());
        assert!(!none.sound_chain_on && !none.offload_paused);
        // Every lyrics service, in the order they are asked, each on or off where it stands.
        let order: Vec<&str> = d.lyrics_order.iter().map(|s| s.name()).collect();
        assert_eq!(s.lyrics_sources.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), order);
        let on: Vec<&str> = s.lyrics_sources.iter().filter(|l| l.on).map(|l| l.id.as_str()).collect();
        let defaults: Vec<&str> = crate::lyrics_sources::default_on().iter().map(|s| s.name()).collect();
        assert_eq!(on, defaults, "the default services on");
        let off = set_by_name(&d, "lyricsService:BINILYRICS", "false").unwrap().prefs;
        let s2 = state(&off, Output::default(), &beat_model::ModelFile::default());
        assert_eq!(s2.lyrics_sources[5], LyricsSource { id: "BINILYRICS".into(), on: false, needs_key: false }, "switched off where it stands");
        assert!(s.lyrics_sources.iter().any(|l| l.needs_key));
        assert_eq!(s.beat_model == BeatModel::Unavailable, !beats::AVAILABLE);
    }
}
