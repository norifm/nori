//! The transition planner the audio path asks: the play window and output state are handed in when they change;
//! the engine asks for plans, whether a song wants measuring, and hands over finished measurements.
//! The engine re-asks every couple of seconds while nothing is planned, so a repeated "no" is a cached lookup.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use nori_model::alog;
use nori_player::automix::analysis::Analyzer;
use nori_player::engine::Plan;
use nori_player::transitions::{engine_plan, pick, whole_song, Skip, TransitionPrefs, WindowSong};
pub use nori_player::types::TransitionKind;
use nori_player::types::TransitionPlan;
use parking_lot::Mutex;

use super::store::{get, missing};

/// How many recent plans are kept for screens.
const NOTES_KEPT: usize = 4;

/// Reads the user's transition settings; None before they are open.
pub type ReadSettings = Box<dyn Fn() -> Option<TransitionPrefs> + Send + Sync>;

/// The planner's memory between questions.
struct State {
    /// The settings last read.
    prefs: Option<TransitionPrefs>,
    transitions_off: bool,
    window: Vec<WindowSong>,
    shuffling: bool,
    /// Bumped whenever anything a plan depends on changes: the window, the settings, a stored analysis.
    generation: u64,
    /// The last "no transition" answer: song, generation, and why (`None`: the planner chose gapless).
    none: Option<(String, u64, Option<Skip>)>,
    /// The last few plans made, newest last, each told and as made (None: gapless): a jam's guests still
    /// learn the plan out of the song heard once the host's decoder has planned the next one.
    made: Vec<(TransitionNote, Option<Plan>)>,
    /// The transition another device was in the middle of when it handed its queue here, out of that
    /// song (its id): played on as it was planned there rather than planned again.
    handed: Option<(String, Plan)>,
}

impl State {
    const fn new() -> Self {
        State { prefs: None, transitions_off: false, window: Vec::new(), shuffling: false, generation: 0, none: None, made: Vec::new(), handed: None }
    }

    fn take_prefs(&mut self, prefs: TransitionPrefs) {
        if self.prefs != Some(prefs) {
            self.prefs = Some(prefs);
            self.generation += 1;
        }
    }

    fn duration_of(&self, song_id: &str) -> i64 {
        self.window.iter().find(|s| s.id == song_id).map_or(0, |s| s.duration_ms)
    }

    fn keep(&mut self, n: TransitionNote, plan: Option<Plan>) {
        self.made.retain(|(k, _)| k.outgoing_id != n.outgoing_id);
        if self.made.len() >= NOTES_KEPT {
            self.made.remove(0);
        }
        self.made.push((n, plan));
    }
}

/// One app's planner: the window and output state handed in, the plans made, over the profile's
/// analyses and the settings `read_settings` gives.
pub struct Planner {
    state: Mutex<State>,
    db: Arc<nori_db::Profile>,
    read_settings: ReadSettings,
    /// The thread measurements are finished and stored on, never the audio thread. Started on first use.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    worker: OnceLock<Mutex<Sender<Finished>>>,
    /// Told after each plan made.
    planned: Mutex<Option<Planned>>,
    /// The profile's songs are measured (`ProfileRules::measures`), whether AutoMix is on or not.
    measures: AtomicBool,
    /// Told when an analysis is stored or forgotten, or the window moves: what [`Planner::may_measure`] reads.
    pub changed: nori_tasks::Notify,
}

/// What is told of each plan made (a jam host's guests play its transitions).
pub type Planned = Box<dyn Fn() + Send + Sync>;

impl Planner {
    pub fn new(db: Arc<nori_db::Profile>, read_settings: ReadSettings) -> Arc<Planner> {
        Arc::new(Planner { state: Mutex::new(State::new()), db, read_settings, worker: OnceLock::new(), planned: Mutex::new(None), measures: AtomicBool::new(true), changed: Default::default() })
    }

    /// Whether the output forbids touching samples (`AudioPolicy::untouched`); set whenever the
    /// policy changes.
    pub fn transition_setup(&self, transitions_off: bool) {
        let mut p = self.state.lock();
        p.transitions_off = transitions_off;
        p.generation += 1;
    }

    /// Whether the profile's songs are measured as they play and ahead (`ProfileRules::measures`).
    pub fn set_measures(&self, measures: bool) {
        self.measures.store(measures, Ordering::Relaxed);
        self.changed.notify();
    }

    pub fn measures(&self) -> bool {
        self.measures.load(Ordering::Relaxed)
    }

    pub fn transitions_off(&self) -> bool {
        self.state.lock().transitions_off
    }

    /// An analysis was stored or forgotten: a cached "no transition" may no longer hold.
    pub fn analyses_changed(&self) {
        self.state.lock().generation += 1;
        self.changed.notify();
    }

    /// The songs in play order: the one before the current one, the current one, and those after it.
    pub fn transition_window(&self, window: Vec<WindowSong>, shuffling: bool) {
        let mut p = self.state.lock();
        if p.window == window && p.shuffling == shuffling {
            return;
        }
        p.window = window;
        p.shuffling = shuffling;
        p.generation += 1;
        drop(p);
        self.changed.notify();
    }

    /// The engine's question: how to mix out of `outgoing_id`, if at all.
    pub fn plan_for(&self, outgoing_id: &str) -> Option<Plan> {
        // Read without the planner locked: the settings may call into the planner under their own lock.
        let kept = (self.read_settings)();
        let mut p = self.state.lock();
        // A handed transition holds while its song is the one ending; asked for another, it is over.
        if let Some((id, plan)) = p.handed.take() {
            if id == outgoing_id {
                p.handed = Some((id, plan.clone()));
                return Some(plan);
            }
        }
        if let Some(kept) = kept {
            p.take_prefs(kept);
        }
        let prefs = p.prefs?;
        let generation = p.generation;
        if p.none.as_ref().is_some_and(|(id, g, _)| *g == generation && id == outgoing_id) {
            return None;
        }
        if prefs.auto_mix && p.window.iter().any(|s| s.duration_ms <= 0) {
            drop(p);
            self.measured_lengths();
            p = self.state.lock();
        }
        let chosen = match pick(&prefs, p.transitions_off, &p.window, outgoing_id, p.shuffling) {
            Ok(chosen) => chosen,
            Err(skip) => {
                if !p.none.as_ref().is_some_and(|(id, _, s)| *s == Some(skip) && id == outgoing_id) {
                    alog::info(&format!("planFor: {}", skip.describe(outgoing_id)));
                }
                p.none = Some((outgoing_id.to_string(), generation, Some(skip)));
                return None;
            }
        };
        let (o, n) = (p.window[chosen.out].clone(), p.window[chosen.next].clone());
        drop(p);
        // Database reads happen without the planner locked.
        let (a, b) = if prefs.auto_mix {
            self.db.get().map_or((None, None), |db| {
                let c = db.lock();
                (get(&c, &o.id).ok().flatten(), get(&c, &n.id).ok().flatten())
            })
        } else {
            (None, None)
        };
        let t = nori_player::automix::plan::plan(a.as_ref(), b.as_ref(), o.duration_ms, n.duration_ms, &chosen.settings);
        let plan = engine_plan(&t, &n.id);
        let note = TransitionNote::of(&o.id, &n.id, &t, plan.is_some());
        {
            let mut p = self.state.lock();
            p.none = plan.is_none().then(|| (outgoing_id.to_string(), generation, None));
            p.keep(note, plan.clone());
        }
        if let Some(told) = &*self.planned.lock() {
            told();
        }
        match &plan {
            None => alog::info(&format!("planFor: gapless ({})", t.reason)),
            Some(_) => alog::info(&format!(
                "transition {} -> {}: {} {} ms at {}, tempo x{:.3} ({})",
                o.title,
                n.title,
                screaming_snake(&format!("{:?}", t.kind)),
                t.duration_ms,
                t.out_start_ms,
                t.tempo_ratio,
                t.reason
            )),
        }
        plan
    }

    /// Window songs the queue knows no length of (a YouTube feed's) take the length their analysis
    /// measured, read without the planner locked.
    fn measured_lengths(&self) {
        let Some(db) = self.db.get() else { return };
        let unknown: Vec<String> = self.state.lock().window.iter().filter(|s| s.duration_ms <= 0).map(|s| s.id.clone()).collect();
        let measured: Vec<(String, i64)> = {
            let c = db.lock();
            unknown.into_iter().filter_map(|id| get(&c, &id).ok().flatten().map(|a| (id, a.duration_ms))).collect()
        };
        let mut p = self.state.lock();
        for s in p.window.iter_mut().filter(|s| s.duration_ms <= 0) {
            if let Some((_, ms)) = measured.iter().find(|(id, _)| *id == s.id) {
                s.duration_ms = *ms;
            }
        }
    }

    /// The transition out of `outgoing_id` into `incoming_id` that another device was in the middle of
    /// when it handed its queue here: the engine mixes on by it, from wherever in it the song starts.
    pub fn handed(&self, outgoing_id: &str, incoming_id: &str, t: &TransitionPlan) {
        let plan = engine_plan(t, incoming_id);
        let mut p = self.state.lock();
        p.keep(TransitionNote::of(outgoing_id, incoming_id, t, plan.is_some()), plan.clone());
        p.handed = plan.map(|plan| (outgoing_id.to_string(), plan));
    }

    /// The plan last made out of `outgoing_id`: None while none was, Some(None) when gapless.
    pub fn made(&self, outgoing_id: &str) -> Option<Option<Plan>> {
        self.state.lock().made.iter().rev().find(|(n, _)| n.outgoing_id == outgoing_id).map(|(_, p)| p.clone())
    }

    /// Tells `told` after each plan made, from the thread that asked for it; None stops.
    pub fn on_plan(&self, told: Option<Planned>) {
        *self.planned.lock() = told;
    }

    /// The last plan out of `outgoing_id`.
    pub fn transition_note(&self, outgoing_id: &str) -> Option<TransitionNote> {
        self.state.lock().made.iter().rev().find(|(n, _)| n.outgoing_id == outgoing_id).map(|(n, _)| n.clone())
    }

    /// The last plan into `incoming_id`.
    pub fn transition_into(&self, incoming_id: &str) -> Option<TransitionNote> {
        self.state.lock().made.iter().rev().find(|(n, _)| n.incoming_id == incoming_id).map(|(n, _)| n.clone())
    }

    /// Whether `song_id` should be measured as it plays, and its length in ms (0 unknown) to size the measurement.
    pub fn wants_analysis(&self, song_id: &str) -> Option<u64> {
        let duration = self.state.lock().duration_of(song_id).max(0) as u64;
        if !self.measures() || !nori_model::analysable(song_id) {
            return None;
        }
        let db = self.db.get()?;
        let wanted = missing(&db.lock(), &[song_id.to_string()]).is_ok_and(|m| !m.is_empty());
        wanted.then_some(duration)
    }

    /// Whether `song_id` may yet be measured as it plays: it is in the window and wants measuring.
    pub fn may_measure(&self, song_id: &str) -> bool {
        let in_window = self.state.lock().window.iter().any(|s| s.id == song_id);
        in_window && self.wants_analysis(song_id).is_some()
    }

    /// The engine heard `song_id` from its first sample to its last. Finished on the planner's thread,
    /// which ends with the planner.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn analysed(self: &Arc<Self>, song_id: &str, analyzer: Analyzer, frames: u64, rate: u32) {
        let worker = self.worker.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<Finished>();
            let me: std::sync::Weak<Planner> = Arc::downgrade(self);
            std::thread::Builder::new()
                .name("nori-analysis".into())
                .spawn(move || rx.into_iter().for_each(|f| if let Some(p) = me.upgrade() { p.finish(f) }))
                .expect("the analysis thread starts");
            Mutex::new(tx)
        });
        let _ = worker.lock().send(Finished { song_id: song_id.to_string(), analyzer, frames, rate });
    }

    /// The web has no threads: there the measurement is finished at once, between the engine's turns.
    #[cfg(target_arch = "wasm32")]
    pub fn analysed(self: &Arc<Self>, song_id: &str, analyzer: Analyzer, frames: u64, rate: u32) {
        self.finish(Finished { song_id: song_id.to_string(), analyzer, frames, rate });
    }

    fn finish(&self, mut f: Finished) {
        let expected_ms = self.state.lock().duration_of(&f.song_id);
        let heard_ms = (f.frames * 1000 / f.rate.max(1) as u64) as i64;
        // Only a song heard whole, and at least 30 s long, is stored.
        if !whole_song(heard_ms, expected_ms) || f.analyzer.samples() < (f.analyzer.rate() * 30.0) as u64 {
            alog::info(&format!("analysed {}: not stored: heard {heard_ms} ms of {expected_ms} ms", f.song_id));
            return;
        }
        let features = f.analyzer.take_features();
        let Some(db) = self.db.get() else {
            alog::info(&format!("analysed {}: not stored: no database", f.song_id));
            return;
        };
        let (track, curve) = super::store::measure(&f.song_id, &features);
        let stored = super::store::put_finished(&db.lock(), track, &curve);
        let a = match stored {
            Ok(a) => a,
            Err(e) => {
                alog::info(&format!("analysed {}: not stored: {e}", f.song_id));
                return;
            }
        };
        self.analyses_changed();
        alog::info(&format!(
            "analysed {}: {:.2} bpm (conf {:.2}, stab {:.2}), key {}, heard {} ms of {} ms, {} frames at {} Hz",
            f.song_id,
            a.bpm,
            a.bpm_confidence,
            a.stability,
            nori_player::automix::structure::camelot_name(a.key),
            a.duration_ms,
            expected_ms,
            f.frames,
            f.rate,
        ));
    }
}

/// A transition as planned, for screens: its kind, where in the outgoing song it starts, its length (0 for
/// gapless), the incoming speed, and the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionNote {
    pub outgoing_id: String,
    pub incoming_id: String,
    pub kind: TransitionKind,
    pub start_ms: i64,
    pub duration_ms: i64,
    pub tempo_ratio: f32,
    pub reason: String,
}

impl TransitionNote {
    /// What is told of transition `t` out of `outgoing_id` into `incoming_id`; gapless without an engine plan.
    fn of(outgoing_id: &str, incoming_id: &str, t: &TransitionPlan, mixed: bool) -> TransitionNote {
        TransitionNote {
            outgoing_id: outgoing_id.to_string(),
            incoming_id: incoming_id.to_string(),
            kind: if mixed { t.kind } else { TransitionKind::Gapless },
            start_ms: t.out_start_ms,
            duration_ms: if mixed { t.duration_ms } else { 0 },
            tempo_ratio: t.tempo_ratio as f32,
            reason: t.reason.clone(),
        }
    }
}

/// `BeatMatched` -> `BEAT_MATCHED`, as the logs name kinds.
fn screaming_snake(camel: &str) -> String {
    let mut s = String::with_capacity(camel.len() + 4);
    for (i, c) in camel.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            s.push('_');
        }
        s.push(c.to_ascii_uppercase());
    }
    s
}

struct Finished {
    song_id: String,
    analyzer: Analyzer,
    frames: u64,
    rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(out: &str, inc: &str) -> TransitionNote {
        TransitionNote { outgoing_id: out.into(), incoming_id: inc.into(), kind: TransitionKind::BeatMatched, start_ms: 1, duration_ms: 8000, tempo_ratio: 1.0, reason: String::new() }
    }

    #[test]
    fn heard_measure_keeps_model_grid() {
        use nori_player::automix::beats::GRID_NEURAL;
        use nori_player::automix::synth::Synth;
        let db = std::sync::Arc::new(Mutex::new(nori_db::open("", "t").unwrap()));
        let profile = Arc::new(nori_db::Profile::default());
        profile.set(&db);
        let planner = Planner::new(profile, Box::new(|| None));
        let s = Synth::new(120.0);
        let modelled = nori_model::TrackAnalysis { song_id: "s".into(), analysis_version: crate::ANALYSIS_VERSION, duration_ms: (s.secs * 1000.0) as i64, intro_bpm: 90.0, intro_grid_source: GRID_NEURAL, ..Default::default() };
        super::super::store::put(&db.lock(), &modelled).unwrap();
        let mut analyzer = Analyzer::new(s.rate, 60_000);
        let pcm = s.render();
        analyzer.feed(&pcm);
        planner.finish(Finished { song_id: "s".into(), analyzer, frames: pcm.len() as u64, rate: s.rate });
        let kept = get(&db.lock(), "s").unwrap().unwrap();
        assert_eq!((kept.intro_grid_source, kept.intro_bpm), (GRID_NEURAL, 90.0));
        assert!(super::super::store::get_voice(&db.lock(), "s").unwrap().is_some());
    }

    #[test]
    fn songs_analysed_as_they_play() {
        let db = std::sync::Arc::new(Mutex::new(nori_db::open("", "t").unwrap()));
        let profile = Arc::new(nori_db::Profile::default());
        profile.set(&db);
        let planner = Planner::new(profile, Box::new(|| Some(TransitionPrefs { auto_mix: true, ..nori_player::sim::prefs_off() })));
        assert_eq!(planner.wants_analysis("s"), Some(0), "a library song, its length unknown");
        assert_eq!(planner.wants_analysis("ext-2"), Some(0), "a provider's song, from the bytes it plays");
        assert_eq!(planner.wants_analysis("radio:1"), None, "a station's stream never ends");
    }

    fn song(id: &str, duration_ms: i64) -> WindowSong {
        WindowSong { id: id.into(), duration_ms, ..WindowSong::default() }
    }

    /// A planner over an empty database whose crossfade length the test sets (0: off).
    fn crossfading() -> (Arc<Planner>, Arc<Mutex<i32>>, Arc<Mutex<rusqlite::Connection>>) {
        let db = Arc::new(Mutex::new(nori_db::open("", "t").unwrap()));
        let profile = Arc::new(nori_db::Profile::default());
        profile.set(&db);
        let crossfade = Arc::new(Mutex::new(6));
        let read = crossfade.clone();
        let planner = Planner::new(profile, Box::new(move || Some(TransitionPrefs { crossfade_s: *read.lock(), ..nori_player::sim::prefs_off() })));
        (planner, crossfade, db)
    }

    #[test]
    fn a_cached_no_follows_what_it_depends_on() {
        let (planner, crossfade, _db) = crossfading();
        assert!(planner.plan_for("a").is_none(), "an empty window");
        planner.transition_window(vec![song("a", 200_000), song("b", 200_000)], false);
        assert!(planner.plan_for("a").is_some(), "a new window");
        let note = planner.transition_note("a").unwrap();
        assert_eq!((note.incoming_id.as_str(), note.kind, note.duration_ms), ("b", TransitionKind::EqualPowerFade, 6000));
        assert_eq!(planner.transition_into("b"), Some(note));
        assert_eq!(planner.transition_into("a"), None);

        planner.transition_setup(true);
        assert!(planner.transitions_off());
        assert!(planner.plan_for("a").is_none(), "the output forbids it");
        planner.transition_setup(false);
        assert!(!planner.transitions_off());
        assert!(planner.plan_for("a").is_some(), "the output allows it again");

        *crossfade.lock() = 0;
        assert!(planner.plan_for("a").is_none(), "crossfade off");
        *crossfade.lock() = 4;
        assert!(planner.plan_for("a").is_some(), "crossfade on again");
        assert_eq!(planner.transition_note("a").unwrap().duration_ms, 4000);
    }

    #[test]
    fn the_same_window_again_keeps_a_cached_no() {
        let db = Arc::new(Mutex::new(nori_db::open("", "t").unwrap()));
        let profile = Arc::new(nori_db::Profile::default());
        profile.set(&db);
        let planner = Planner::new(profile, Box::new(|| Some(TransitionPrefs { auto_mix: true, ..nori_player::sim::prefs_off() })));
        let made = Arc::new(Mutex::new(0));
        let count = made.clone();
        planner.on_plan(Some(Box::new(move || *count.lock() += 1)));
        let track = |id: &str, track| WindowSong { album_id: Some("x".into()), track, ..song(id, 200_000) };
        let window = || vec![track("a", 1), track("b", 2)];
        planner.transition_window(window(), false);
        assert!(planner.plan_for("a").is_none(), "same album in order: gapless");
        planner.transition_window(window(), false);
        assert!(planner.plan_for("a").is_none());
        assert_eq!(*made.lock(), 1, "planned once");
        planner.transition_window(vec![track("a", 1), track("c", 2)], false);
        assert!(planner.plan_for("a").is_none());
        assert_eq!(*made.lock(), 2, "another window is planned again");
    }

    /// The engine's decoder asks for the plan out of the next song while the song heard still plays to its
    /// own mix (seconds ahead with a deep buffer): the plan out of the song heard stays known, for a jam's
    /// guests to play it.
    #[test]
    fn the_plan_out_of_the_song_heard_outlives_the_next_ones() {
        let (planner, _, _db) = crossfading();
        planner.transition_window(vec![song("a", 200_000), song("b", 200_000), song("c", 200_000)], false);
        let a = planner.plan_for("a").expect("a plan out of a");
        planner.transition_window(vec![song("a", 200_000), song("b", 200_000), song("c", 200_000)], false);
        assert!(planner.plan_for("b").is_some());
        assert_eq!(planner.made("a"), Some(Some(a)), "still known");
        assert!(planner.made("b").is_some_and(|p| p.is_some_and(|p| p.incoming_id == "c")));
        assert_eq!(planner.made("c"), None, "not planned");
    }

    #[test]
    fn measures_with_automix_off_unless_the_profile_does_not() {
        let (planner, crossfade, _db) = crossfading();
        *crossfade.lock() = 0;
        planner.transition_window(vec![song("a", 200_000), song("b", 180_000)], false);
        assert!(planner.plan_for("a").is_none(), "no transition with AutoMix and crossfade off");
        assert_eq!(planner.wants_analysis("b"), Some(180_000), "sized by the window");
        assert!(planner.may_measure("b"));
        planner.set_measures(false);
        assert_eq!(planner.wants_analysis("b"), None, "a profile that measures nothing");
        assert!(!planner.may_measure("b"));
    }

    /// Only a song heard whole, and at least 30 s of it, is stored.
    #[test]
    fn stores_only_whole_songs() {
        use nori_player::automix::synth::Synth;
        for (secs, window_ms, stored) in [(60.0, 60_000, true), (60.0, 0, true), (60.0, 200_000, false), (20.0, 20_000, false)] {
            let (planner, _, db) = crossfading();
            planner.transition_window(vec![song("s", window_ms)], false);
            let s = Synth { secs, ..Synth::new(120.0) };
            let mut analyzer = Analyzer::new(s.rate, 60_000);
            let pcm = s.render();
            analyzer.feed(&pcm);
            planner.finish(Finished { song_id: "s".into(), analyzer, frames: pcm.len() as u64, rate: s.rate });
            assert_eq!(get(&db.lock(), "s").unwrap().is_some(), stored, "{secs} s heard of {window_ms} ms");
        }
    }

    /// Songs the queue knows no length of (a YouTube feed's) mix at the length their analysis measured.
    #[test]
    fn unknown_lengths_mix_at_the_measured_ones() {
        use nori_player::automix::synth::Synth;
        let db = Arc::new(Mutex::new(nori_db::open("", "t").unwrap()));
        let profile = Arc::new(nori_db::Profile::default());
        profile.set(&db);
        let planner = Planner::new(profile, Box::new(|| Some(TransitionPrefs { auto_mix: true, ..nori_player::sim::prefs_off() })));
        planner.transition_window(vec![song("a", 0), song("b", 0)], false);
        assert!(planner.plan_for("a").is_none(), "nothing measured yet");
        for id in ["a", "b"] {
            let s = Synth { secs: 60.0, ..Synth::new(120.0) };
            let mut analyzer = Analyzer::new(s.rate, 60_000);
            let pcm = s.render();
            analyzer.feed(&pcm);
            planner.finish(Finished { song_id: id.into(), analyzer, frames: pcm.len() as u64, rate: s.rate });
        }
        let plan = planner.plan_for("a").expect("a mix from the measured lengths");
        assert_eq!(plan.incoming_id, "b");
        let note = planner.transition_note("a").unwrap();
        assert!((40_000..60_000).contains(&note.start_ms), "{note:?}");
    }

    #[test]
    fn notes() {
        let mut p = State::new();
        p.keep(n("a", "b"), None);
        p.keep(TransitionNote { kind: TransitionKind::EchoOut, ..n("a", "b") }, None);
        assert_eq!(p.made.iter().map(|(n, _)| n.kind).collect::<Vec<_>>(), [TransitionKind::EchoOut], "a replan replaces the note");
        for i in 0..8 {
            p.keep(n(&format!("x{i}"), "y"), None);
        }
        assert_eq!(p.made.len(), NOTES_KEPT);
        assert_eq!(p.made.last().unwrap().0.outgoing_id, "x7");

        // Kind names log as screaming snake.
        assert_eq!(screaming_snake("BeatMatched"), "BEAT_MATCHED");
        assert_eq!(screaming_snake("Gapless"), "GAPLESS");
    }

}
