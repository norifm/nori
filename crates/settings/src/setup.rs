//! First-run setup: the questions asked once the first profile is in, and the settings each answer sets,
//! then pairing with another device. The welcome page before it is the profile's own adding
//! (`nori_core::providers::flow`).

use crate::codec::Choice;
use crate::settings::AutoMixMode;

/// A question of the first run, asked once the first profile connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SetupStep {
    /// What AutoMix does between songs.
    Listening,
    /// The quality on mobile data.
    Data,
    /// Appearance and season, each pick taken as it is made; no [`SetupAnswer`].
    Look,
    /// Pairing with another device, which brings its profiles; under it the sync server's address and
    /// switch. No [`SetupAnswer`].
    Pair,
}

/// An answer to a [`SetupStep`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SetupAnswer {
    /// AutoMix off: songs play whole, one after another.
    NoMixing,
    /// AutoMix, smooth.
    AutoMix,
    /// AutoMix, DJ.
    AutoMixDj,
    /// Mobile data streams the original file.
    Original,
    /// Mobile data streams 192 kbps Opus.
    High,
    /// Mobile data streams 128 kbps Opus.
    SaveData,
}

/// One setting an answer sets, by its name, as a client changes any setting.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingPick {
    pub name: String,
    pub value: String,
}

/// The questions a first run asks, in order; `mobile_data`: the device streams over a mobile network;
/// `pairs`: the client has a pairing screen and the device is in no group yet.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn setup_steps(mobile_data: bool, pairs: bool) -> Vec<SetupStep> {
    let asked = |s: &SetupStep| match s {
        SetupStep::Data => mobile_data,
        SetupStep::Pair => pairs,
        _ => true,
    };
    [SetupStep::Listening, SetupStep::Data, SetupStep::Look, SetupStep::Pair].into_iter().filter(asked).collect()
}

/// The answers `step` offers, in order.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn setup_answers(step: SetupStep) -> Vec<SetupAnswer> {
    use SetupAnswer::*;
    match step {
        SetupStep::Listening => vec![NoMixing, AutoMix, AutoMixDj],
        SetupStep::Data => vec![Original, High, SaveData],
        SetupStep::Look | SetupStep::Pair => Vec::new(),
    }
}

/// The settings `answer` sets, each to be changed by name.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn setup_settings(answer: SetupAnswer) -> Vec<SettingPick> {
    let pick = |name: &str, value: &str| vec![SettingPick { name: name.into(), value: value.into() }];
    match answer {
        SetupAnswer::NoMixing => pick("autoMix", AutoMixMode::Off.name()),
        SetupAnswer::AutoMix => pick("autoMix", AutoMixMode::Smooth.name()),
        SetupAnswer::AutoMixDj => pick("autoMix", AutoMixMode::Dj.name()),
        SetupAnswer::Original => pick("mobile", "0:"),
        SetupAnswer::High => pick("mobile", "192:opus"),
        SetupAnswer::SaveData => pick("mobile", "128:opus"),
    }
}

/// The covers the AutoMix question plays, the same on every client: albums from norifm.com whose edges
/// show on light and dark paper alike, as WebP.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn setup_covers() -> Vec<Vec<u8>> {
    [
        include_bytes!("../assets/setup/the-miseducation-of-lauryn-hill.webp").as_slice(),
        include_bytes!("../assets/setup/1992-deluxe.webp"),
        include_bytes!("../assets/setup/channel-orange.webp"),
        include_bytes!("../assets/setup/casiopea.webp"),
    ]
    .iter()
    .map(|c| c.to_vec())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{set_by_name, SavedQuality, StoredPrefs};

    fn answered(p: &StoredPrefs, answer: SetupAnswer) -> StoredPrefs {
        setup_settings(answer).iter().fold(p.clone(), |p, s| set_by_name(&p, &s.name, &s.value).unwrap_or_else(|| panic!("{} refused {}", s.name, s.value)).prefs)
    }

    #[test]
    fn answers_set_their_settings() {
        let fresh = StoredPrefs::default();
        for (answer, mix) in [(SetupAnswer::AutoMixDj, AutoMixMode::Dj), (SetupAnswer::AutoMix, AutoMixMode::Smooth), (SetupAnswer::NoMixing, AutoMixMode::Off)] {
            let p = answered(&StoredPrefs { auto_mix: AutoMixMode::Dj, ..fresh.clone() }, answer);
            assert_eq!(p.auto_mix, mix, "{answer:?}");
        }
        let saving = answered(&fresh, SetupAnswer::SaveData);
        assert_eq!(saving.mobile, SavedQuality { bit_rate: 128, format: "opus".into() });
        assert_eq!(answered(&saving, SetupAnswer::High).mobile, SavedQuality { bit_rate: 192, format: "opus".into() });
        assert_eq!(answered(&saving, SetupAnswer::Original).mobile, SavedQuality::default());
    }

    #[test]
    fn data_is_asked_only_with_mobile_data_and_pairing_last_where_it_pairs() {
        assert_eq!(setup_steps(true, true), [SetupStep::Listening, SetupStep::Data, SetupStep::Look, SetupStep::Pair]);
        assert_eq!(setup_steps(false, true), [SetupStep::Listening, SetupStep::Look, SetupStep::Pair]);
        assert_eq!(setup_steps(false, false), [SetupStep::Listening, SetupStep::Look]);
    }
}
