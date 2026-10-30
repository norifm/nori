//! The AutoMix seams for the planner (`nori_automix::mixing`): the enabled plugins exporting `automix`
//! plan transitions and refine analyses, those exporting `deciders` say whether two songs mix. Each is
//! asked in the plugins' order where nori decides (a plan asked for, an analysis stored), within a
//! short budget; nori checks what they answer.

use std::sync::{Arc, Weak};
use std::time::Duration;

use nori_automix::mixing::{beat_times, proposed_kind, Mixing, Proposal, Proposed, Refined};
use nori_model::Song;
use nori_player::types::{TrackAnalysis, TransitionPlan};

use crate::convert::song_out;
use crate::runtime::points::automix as w;
use crate::Plugins;

/// How long a plan, a refinement or a pair's mixing may take before nori's stands.
const BUDGET: Duration = Duration::from_millis(100);

/// The planner's way to the plugins; it holds them weakly, as the plugins outlive no profile.
pub struct PluginMixing(Weak<Plugins>);

impl Plugins {
    /// Where the planner asks the enabled plugins for AutoMix.
    pub fn mixing(self: &Arc<Self>) -> Arc<dyn Mixing> {
        Arc::new(PluginMixing(Arc::downgrade(self)))
    }

    /// Song `id` as the open profile knows it, or by its id alone.
    fn song(&self, id: &str) -> Song {
        self.songs.lock().expect("not poisoned").as_ref().and_then(|f| f(id)).unwrap_or_else(|| Song::only_id(id.to_string()))
    }
}

fn analysis_out(a: &TrackAnalysis) -> w::Analysis {
    let ms = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    w::Analysis {
        bpm: a.bpm as f32,
        bpm_confidence: a.bpm_confidence,
        key: a.key.clamp(0, 24) as u8,
        intro_end_ms: ms(a.intro_end_ms),
        outro_start_ms: ms(a.outro_start_ms),
        beats: beat_times(a).into_iter().map(ms).collect(),
        loudness_lufs: a.lufs,
    }
}

fn kind_out(k: Proposed) -> w::TransitionKind {
    match k {
        Proposed::Gapless => w::TransitionKind::Gapless,
        Proposed::Crossfade => w::TransitionKind::Crossfade,
        Proposed::BeatMatched => w::TransitionKind::BeatMatched,
        Proposed::EchoOut => w::TransitionKind::EchoOut,
        Proposed::Cut => w::TransitionKind::Cut,
    }
}

fn kind_in(k: w::TransitionKind) -> Proposed {
    match k {
        w::TransitionKind::Gapless => Proposed::Gapless,
        w::TransitionKind::Crossfade => Proposed::Crossfade,
        w::TransitionKind::BeatMatched => Proposed::BeatMatched,
        w::TransitionKind::EchoOut => Proposed::EchoOut,
        w::TransitionKind::Cut => Proposed::Cut,
    }
}

fn plan_out(t: &TransitionPlan) -> w::Transition {
    let ms = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    w::Transition {
        kind: kind_out(proposed_kind(t.kind)),
        out_start_ms: ms(t.out_start_ms),
        duration_ms: ms(t.duration_ms),
        in_start_ms: ms(t.in_start_ms),
        tempo_ratio: t.tempo_ratio as f32,
        bass_swap: t.bass_swap.is_some(),
        filter: t.low_pass.is_some(),
    }
}

impl Mixing for PluginMixing {
    fn mixes(&self, outgoing: &str, incoming: &str, nori: bool) -> Option<bool> {
        let plugins = self.0.upgrade()?;
        let deciding = plugins.extending("deciders");
        if deciding.is_empty() {
            return None;
        }
        let (o, i) = (song_out(&plugins.song(outgoing)), song_out(&plugins.song(incoming)));
        let mut answer = nori;
        for id in deciding {
            let Some(worker) = plugins.worker(&id) else { continue };
            let (o, i) = (o.clone(), i.clone());
            if let Some(a) = worker.decide(BUDGET, move |s, p| p.nori_plugin_deciders().call_mixes(s, &o, &i, answer)).flatten() {
                answer = a;
            }
        }
        (answer != nori).then_some(answer)
    }

    fn plan(&self, outgoing: &str, incoming: &str, out: Option<&TrackAnalysis>, inc: Option<&TrackAnalysis>, nori: &TransitionPlan) -> Option<Proposal> {
        let plugins = self.0.upgrade()?;
        let planning = plugins.extending("automix");
        if planning.is_empty() {
            return None;
        }
        let (o, i) = (song_out(&plugins.song(outgoing)), song_out(&plugins.song(incoming)));
        let (a, b, t) = (out.map(analysis_out), inc.map(analysis_out), plan_out(nori));
        // The first plugin with a plan of its own gives it.
        planning.into_iter().find_map(|id| {
            let worker = plugins.worker(&id)?;
            let (o, i, a, b, t) = (o.clone(), i.clone(), a.clone(), b.clone(), t);
            let p = worker.decide(BUDGET, move |s, p| p.nori_plugin_automix().call_plan(s, &o, &i, a.as_ref(), b.as_ref(), t)).flatten()?;
            Some(Proposal {
                kind: kind_in(p.kind),
                out_start_ms: p.out_start_ms as i64,
                duration_ms: p.duration_ms as i64,
                in_start_ms: p.in_start_ms as i64,
                tempo_ratio: p.tempo_ratio as f64,
                bass_swap: p.bass_swap,
                filter: p.filter,
            })
        })
    }

    fn refine(&self, song: &str, analysis: &TrackAnalysis) -> Option<Refined> {
        let plugins = self.0.upgrade()?;
        let refining = plugins.extending("automix");
        if refining.is_empty() {
            return None;
        }
        let s = song_out(&plugins.song(song));
        let a = analysis_out(analysis);
        refining.into_iter().find_map(|id| {
            // Its own pass over the song may still be finishing: nori's ended at the same sample.
            let data = plugins.preparation().and_then(|p| p.analysis(song, &id, BUDGET * 10));
            let worker = plugins.worker(&id)?;
            let (s, a) = (s.clone(), a.clone());
            let r = worker.decide(BUDGET, move |st, p| p.nori_plugin_automix().call_refine(st, &s, &a, data.as_deref())).flatten()?;
            Some(Refined {
                bpm: r.bpm as f64,
                bpm_confidence: r.bpm_confidence,
                key: r.key as i32,
                intro_end_ms: r.intro_end_ms as i64,
                outro_start_ms: r.outro_start_ms as i64,
                beats: r.beats.into_iter().map(i64::from).collect(),
                lufs: r.loudness_lufs,
            })
        })
    }
}
