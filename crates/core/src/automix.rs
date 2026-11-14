//! Analysis store calls on the core's database. AutoMix itself is nori-automix's.

use nori_automix::beats::{EndGrid, MixEnd};
use nori_automix::analysis::Analyzer;
use nori_automix::store::{get, get_voice, measure, missing, neural_missing, put, put_finished, AnalysisStream};

use crate::settings::AutoMixMode;
use crate::{Core, Result, TrackAnalysis};

pub use nori_automix::*;

#[cfg(test)]
mod tests;

/// Why downloaded songs are offered an analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AnalysisReason {
    /// AutoMix was turned on.
    TurnedOn,
    /// An update changed the analysis ([`ANALYSIS_VERSION`]).
    Changed,
}

/// Downloaded songs to offer an analysis (the classical one and the beat model's), why, and about how many
/// minutes it takes at this device's learned pace ([`felt_minutes`]). Answered yes, the client reads them
/// back (`Core::download_unanalysed`); no, they are measured as they come up to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AnalysisOffer {
    pub songs: u32,
    pub reason: AnalysisReason,
    pub minutes: u32,
}

/// A wait of `secs` in the minutes it is felt in: whole minutes, fives past an hour; 0 is under a minute.
pub fn felt_minutes(secs: i64) -> u32 {
    let min = (secs.max(0) + 30) / 60;
    (if min >= 60 { (min + 2) / 5 * 5 } else { min }) as u32
}

/// The analysis version downloads were last offered for, so each version is offered once.
const OFFERED_KEY: &str = "analysis_offered";

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    pub fn analysis_get(&self, song_id: String) -> Result<Option<TrackAnalysis>> {
        Ok(get(&self.db.lock(), &song_id)?)
    }

    /// AutoMix went from `before` to `after`: the downloads to offer an analysis when it was turned on and
    /// some lack it. Switching between its styles asks nothing: they share one analysis.
    pub fn analysis_offer_on_switch(&self, before: AutoMixMode, after: AutoMixMode) -> Option<AnalysisOffer> {
        if before.on() || !after.on() {
            return None;
        }
        self.analysis_offer(AnalysisReason::TurnedOn)
    }

    /// At startup with AutoMix on, once per analysis version: the downloads to offer an analysis when an
    /// update left some without the current one.
    pub fn analysis_offer_at_start(&self) -> Option<AnalysisOffer> {
        if !self.session.settings.prefs(|p| p.auto_mix.on()) {
            return None;
        }
        {
            let c = self.db.lock();
            let offered = crate::db::kv_get(&c, OFFERED_KEY).ok().flatten().and_then(|v| v.parse::<i32>().ok());
            if offered.is_some_and(|v| v >= ANALYSIS_VERSION) {
                return None;
            }
            crate::db::kv_put(&c, OFFERED_KEY, &ANALYSIS_VERSION.to_string()).ok()?;
        }
        self.analysis_offer(AnalysisReason::Changed)
    }
}

impl Core {
    fn analysis_offer(&self, reason: AnalysisReason) -> Option<AnalysisOffer> {
        let (_, analyses, beats) = self.download_lacking().ok()?;
        let (analyses, beats_only) = (analyses.len() as u32, beats.len() as u32);
        let secs = self.transfers().with(|t| t.read_back_s(analyses, beats_only, self.download_beats()));
        (analyses + beats_only > 0).then_some(AnalysisOffer { songs: analyses + beats_only, reason, minutes: felt_minutes(secs) })
    }
}

impl Core {
    /// The analysable ids of `song_ids` lacking a current analysis.
    pub fn analysis_missing(&self, song_ids: Vec<String>) -> Result<Vec<String>> {
        let song_ids: Vec<String> = song_ids.into_iter().filter(|id| crate::queue::analysable(id)).collect();
        Ok(missing(&self.db.lock(), &song_ids)?)
    }

    /// Finishes `stream` (one whole song from its start) into the store; dropped unless its length matches
    /// `expected_ms` (0: unknown).
    pub fn analysis_finish_whole(&self, song_id: &str, stream: AnalysisStream, expected_ms: i64) -> Result<Option<TrackAnalysis>> {
        let a = stream.into_analyzer();
        let heard_ms = (a.samples() as f64 * 1000.0 / a.rate()) as i64;
        if !nori_player::transitions::whole_song(heard_ms, expected_ms) {
            return Ok(None);
        }
        self.analysis_finish(song_id, a)
    }

    /// Finishes an analyser fed one whole song and stores the result; None under 30 s.
    pub fn analysis_finish(&self, song_id: &str, mut a: Analyzer) -> Result<Option<TrackAnalysis>> {
        if a.samples() < (a.rate() * 30.0) as u64 {
            return Ok(None);
        }
        let (track, curve) = measure(song_id, &a.take_features());
        let track = self.session.planner.refined(track);
        let stored = put_finished(&self.db.lock(), track, &curve)?;
        self.session.planner.analyses_changed();
        Ok(Some(stored))
    }

    /// The song's vocal activity curve (checked against synced lyrics).
    pub(crate) fn analysis_voice(&self, song_id: &str) -> Result<Option<nori_player::automix::vocal::VocalCurve>> {
        Ok(get_voice(&self.db.lock(), song_id)?)
    }

    /// Keeps where `lyrics` of song `song_id`, as shown, are sung, for AutoMix to mix around; only when their timing
    /// was heard to fit the voice. Server lyrics are shown unchecked, so they are checked here.
    pub(crate) fn analysis_sung(&self, song_id: &str, lyrics: &crate::Lyrics) {
        let timing = match lyrics.timing {
            nori_model::LyricsTiming::NotChecked if lyrics.synced => match self.analysis_voice(song_id).ok().flatten() {
                Some(voice) => nori_lyrics::sync::timing_against(lyrics, &voice),
                None => return,
            },
            timing => timing,
        };
        let Some(span) = nori_lyrics::sync::sung_span(lyrics, timing) else { return };
        match nori_automix::store::put_sung(&self.db.lock(), song_id, span) {
            Ok(true) => self.session.planner.analyses_changed(),
            Ok(false) => {}
            Err(e) => crate::alog::info(&format!("lyrics: where {song_id} is sung not kept: {e}")),
        }
    }

    /// The song's vocal curve once it is measured; None once it will not be while it plays (a profile
    /// that measures nothing, an analysis without a curve, or out of the play window).
    pub(crate) async fn voice_measured(&self, song_id: &str) -> Option<nori_player::automix::vocal::VocalCurve> {
        let planner = &self.session.planner;
        loop {
            let changed = planner.changed.notified();
            if let Some(voice) = self.analysis_voice(song_id).ok().flatten() {
                return Some(voice);
            }
            if !planner.may_measure(song_id) {
                return None;
            }
            changed.await;
        }
    }

    /// The ids of `song_ids` with a current analysis that has an end the beat model has not checked.
    pub fn analysis_neural_missing(&self, song_ids: Vec<String>) -> Result<Vec<String>> {
        let song_ids: Vec<String> = song_ids.into_iter().filter(|id| crate::queue::analysable(id)).collect();
        Ok(neural_missing(&self.db.lock(), &song_ids)?)
    }

    /// Stores the beat model's `grid` for `end` (adopted if confident) and marks the end checked. Returns
    /// whether it was adopted; false without an analysis.
    pub fn analysis_neural_store(&self, song_id: &str, end: MixEnd, grid: Option<EndGrid>) -> Result<bool> {
        let c = self.db.lock();
        let Some(mut row) = get(&c, song_id)? else { return Ok(false) };
        let adopted = nori_automix::beats::merge(&mut row, end, grid);
        put(&c, &row)?;
        if adopted {
            self.session.planner.analyses_changed();
        }
        Ok(adopted)
    }
}
