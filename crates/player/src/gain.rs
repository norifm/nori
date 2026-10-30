//! Loudness normalisation from ReplayGain/R128 tags, the server's fallback gain, or measured loudness.
//!
//! ReplayGain 2.0 (and 1.0) tags target -18 LUFS, R128 tags -23 LUFS (Q7.8 dB, 5 dB less as
//! ReplayGain); a measured loudness needs `target - lufs`. The target shifts every gain by
//! `target - (-18)`.
//!
//! Gains up to 0 dB act as a volume, held under the peak. Gains above 0 dB (up to [`BOOST_MAX_DB`]) are
//! applied to float samples with the limiter behind them, so the song cannot be offloaded
//! ([`offload_allows`]) and never clips.

#[cfg(not(feature = "std"))]
use crate::math::Float;

pub use crate::policy::{GainMode, GainTags};

/// ReplayGain 2.0 reference, LUFS.
pub(crate) const RG2_REFERENCE_LUFS: f32 = -18.0;
/// Targets are clamped to within this of -18.
const TARGET_RANGE_DB: f32 = 12.0;
/// Most a quiet song is turned up, dB.
pub const BOOST_MAX_DB: f32 = 6.0;
/// Gain for songs with no tags and no measurement, ReplayGain dB.
const UNTAGGED_DB: f32 = -6.0;

/// Volume leveling settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainPrefs {
    pub mode: GainMode,
    pub target_lufs: f32,
}

impl GainPrefs {
    /// dB the target adds to a ReplayGain 2.0 gain.
    pub fn target_offset_db(&self) -> f32 {
        let t = if self.target_lufs.is_finite() { self.target_lufs } else { RG2_REFERENCE_LUFS };
        (t - RG2_REFERENCE_LUFS).clamp(-TARGET_RANGE_DB, TARGET_RANGE_DB)
    }

    /// Songs may be boosted (the limiter must then run).
    pub fn boosts(&self) -> bool {
        self.mode != GainMode::Off
    }
}

/// Loudness facts about one song.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SongLoudness {
    /// Tags in ReplayGain 2.0 terms (R128 converted); `None` when it has no tags.
    pub tags: Option<GainTags>,
    /// The server's OpenSubsonic `fallbackGain`, ReplayGain dB.
    pub fallback_db: Option<f32>,
    /// Measured integrated loudness (BS.1770), LUFS.
    pub measured_lufs: Option<f32>,
}

/// Linear gain for a song: tag (track or album; `in_album_order` for auto), else the server's fallback,
/// else measured loudness, else the untagged level, shifted to the target. Up to 1 it is held under the
/// peak; above 1 it is held to [`BOOST_MAX_DB`] and the limiter guards the peaks.
pub fn song_gain(p: &GainPrefs, song: &SongLoudness, in_album_order: bool) -> f32 {
    if p.mode == GainMode::Off {
        return 1.0;
    }
    let album = match p.mode {
        GainMode::Album => true,
        GainMode::Auto => in_album_order,
        _ => false,
    };
    let offset = p.target_offset_db();
    let tagged = song.tags.as_ref().and_then(|g| if album { g.album_gain.or(g.track_gain) } else { g.track_gain.or(g.album_gain) });
    let peak = song.tags.as_ref().and_then(|g| if album { g.album_peak.or(g.track_peak) } else { g.track_peak.or(g.album_peak) }).unwrap_or(0.0);
    let measured = song.measured_lufs.filter(|l| l.is_finite() && *l > -70.0);
    let db = match (tagged.or(song.fallback_db), measured) {
        (Some(g), _) => g + offset,
        (None, Some(lufs)) => p.target_lufs.clamp(RG2_REFERENCE_LUFS - TARGET_RANGE_DB, RG2_REFERENCE_LUFS + TARGET_RANGE_DB) - lufs,
        (None, None) => UNTAGGED_DB + offset,
    };
    let v = 10f32.powf(db / 20.0);
    if !v.is_finite() {
        return 1.0;
    }
    if v > 1.0 {
        return v.min(10f32.powf(BOOST_MAX_DB / 20.0));
    }
    let v = if peak > 0.0 { v.min(1.0 / peak) } else { v };
    v.clamp(0.0, 1.0)
}

/// Whether a song at `gain` may be offloaded (only a volume is available there, so no boost).
pub fn offload_allows(gain: f32) -> bool {
    gain <= 1.0
}

/// Converts the analysis' mid-signal loudness (`TrackAnalysis::lufs`, of `(L + R) / 2`) to BS.1770
/// stereo loudness: +3 dB for stereo (`L² + R² = 2 (M² + S²)`, ignoring the unmeasured side).
pub fn stereo_loudness_of_mid(mid_lufs: f32, channels: u32) -> f32 {
    if channels == 1 {
        mid_lufs
    } else {
        mid_lufs + 10.0 * 2f32.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(v: f32) -> f32 {
        20.0 * v.log10()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn tagged(track: f32, album: f32) -> SongLoudness {
        SongLoudness { tags: Some(GainTags { track_gain: Some(track), album_gain: Some(album), ..Default::default() }), ..Default::default() }
    }

    fn prefs(mode: GainMode) -> GainPrefs {
        GainPrefs { mode, target_lufs: -18.0 }
    }

    #[test]
    fn gain_from_tags() {
        // Mode picks tag.
        let s = tagged(-6.0, -3.0);
        assert!(close(db(song_gain(&prefs(GainMode::Track), &s, true)), -6.0));
        assert!(close(db(song_gain(&prefs(GainMode::Album), &s, false)), -3.0));
        assert!(close(db(song_gain(&prefs(GainMode::Auto), &s, true)), -3.0), "inside an album in order: the album's");
        assert!(close(db(song_gain(&prefs(GainMode::Auto), &s, false)), -6.0), "elsewhere: the song's own");
        assert_eq!(song_gain(&prefs(GainMode::Off), &s, false), 1.0);
        // One tag missing: the other.
        let only_track = SongLoudness { tags: Some(GainTags { track_gain: Some(-4.0), ..Default::default() }), ..Default::default() };
        assert!(close(db(song_gain(&prefs(GainMode::Album), &only_track, false)), -4.0));

        // Target shifts gain.
        let s = tagged(-8.0, -8.0);
        for (target, want) in [(-18.0, -8.0), (-14.0, -4.0), (-23.0, -13.0)] {
            let p = GainPrefs { target_lufs: target, ..prefs(GainMode::Track) };
            assert!(close(db(song_gain(&p, &s, false)), want), "{target} LUFS: {want} dB");
        }
        // A quiet song (ReplayGain +2 dB) at -14 LUFS wants +6 dB: turned up to it.
        let p = GainPrefs { target_lufs: -14.0, ..prefs(GainMode::Track) };
        assert!(close(db(song_gain(&p, &tagged(2.0, 2.0), false)), 6.0));
        assert!(close(db(song_gain(&p, &tagged(5.0, 5.0), false)), BOOST_MAX_DB), "held to the most a song is turned up");
        // A stored target out of all reason is held within 12 dB of -18.
        let p = GainPrefs { target_lufs: 40.0, ..prefs(GainMode::Track) };
        assert!(close(db(song_gain(&p, &tagged(-20.0, -20.0), false)), -8.0));
        let p = GainPrefs { target_lufs: f32::NAN, ..prefs(GainMode::Track) };
        assert!(close(db(song_gain(&p, &s, false)), -8.0));

        // Untagged fallback order.
        let p = prefs(GainMode::Track);
        let nothing = SongLoudness::default();
        assert!(close(db(song_gain(&p, &nothing, false)), -6.0), "the untagged level");
        let measured = SongLoudness { measured_lufs: Some(-9.0), ..Default::default() };
        assert!(close(db(song_gain(&p, &measured, false)), -9.0), "-9 LUFS to -18: -9 dB");
        let quiet = SongLoudness { measured_lufs: Some(-21.0), ..Default::default() };
        assert!(close(db(song_gain(&p, &quiet, false)), 3.0), "-21 LUFS: +3 dB");
        let p14 = GainPrefs { target_lufs: -14.0, ..p };
        assert!(close(db(song_gain(&p14, &measured, false)), -5.0), "the target is the target");
        let silent = SongLoudness { measured_lufs: Some(-70.0), ..Default::default() };
        assert!(close(db(song_gain(&p, &silent, false)), -6.0), "a silence measured is no loudness");
        let fallback = SongLoudness { fallback_db: Some(-4.0), measured_lufs: Some(-9.0), ..Default::default() };
        assert!(close(db(song_gain(&p, &fallback, false)), -4.0), "the server's fallback before a measure");
        // The untagged level moves with the target, as a guess at a tagged song's gain.
        assert!(close(db(song_gain(&GainPrefs { target_lufs: -23.0, ..p }, &nothing, false)), -11.0));
    }

    #[test]
    fn peaks_hold_a_cut() {
        let peaky = SongLoudness { tags: Some(GainTags { track_gain: Some(-1.0), track_peak: Some(1.25), ..Default::default() }), ..Default::default() };
        assert!(close(song_gain(&prefs(GainMode::Track), &peaky, false), 0.8), "turned down: the peak guard still holds the volume under full scale");
        // Turned up, the peak is the limiter's.
        let quiet = SongLoudness { tags: Some(GainTags { track_gain: Some(4.0), track_peak: Some(0.9), ..Default::default() }), ..Default::default() };
        assert!(close(db(song_gain(&prefs(GainMode::Track), &quiet, false)), 4.0));
    }

    #[test]
    fn only_boost_blocks_offload() {
        assert!(offload_allows(1.0) && offload_allows(0.5) && offload_allows(0.0));
        assert!(!offload_allows(1.0001) && !offload_allows(2.0));
        assert!(!offload_allows(song_gain(&prefs(GainMode::Track), &tagged(3.0, 3.0), false)), "turned up: on the CPU");
        assert!(offload_allows(song_gain(&prefs(GainMode::Track), &tagged(-7.0, -7.0), false)), "turned down: a volume");
    }

    #[test]
    fn stereo_loudness_is_mid_plus_3_db() {
        assert!(close(stereo_loudness_of_mid(-17.0, 2), -13.99));
        assert!(close(stereo_loudness_of_mid(-17.0, 0), -13.99), "unknown: stereo");
        assert_eq!(stereo_loudness_of_mid(-17.0, 1), -17.0);
    }
}
