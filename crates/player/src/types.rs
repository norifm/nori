//! Shared value types: equalizer bands, AutoMix track analysis, settings and transition plans
//! (exposed to Kotlin through uniffi `remote` records).

use crate::prelude::*;

/// Ordinals are the wire format read by `dsp.rs`: only append.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EqKind {
    Peaking,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    BandPass,
    Notch,
    AllPass,
    /// Shelves whose `q` is the RBJ slope S (1: steepest without ripple).
    LowShelfSlope,
    HighShelfSlope,
}

impl EqKind {
    /// Whether a band's gain matters for this kind (shelves and peaks; pass filters have none).
    pub fn uses_gain(self) -> bool {
        matches!(self, EqKind::Peaking | EqKind::LowShelf | EqKind::HighShelf | EqKind::LowShelfSlope | EqKind::HighShelfSlope)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EqBand {
    pub kind: EqKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// A built-in equalizer curve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PresetKind {
    #[default]
    Flat,
    BassBoost,
    BassCut,
    TrebleBoost,
    TrebleCut,
    VocalBoost,
    Loudness,
    SmallSpeakers,
}

/// A built-in curve from `dsp::eq_presets`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NamedPreset {
    pub kind: PresetKind,
    pub preamp_db: f32,
    pub bands: Vec<EqBand>,
}

/// AutoMix analysis of one track (a `track_analysis` row). Times are ms from the file's start. Beat `n`
/// is at `beat_offset_ms + n * 60000 / bpm`; beats with `n % beats_per_bar == downbeat_phase` start a bar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackAnalysis {
    pub song_id: String,
    /// Rows older than `automix::ANALYSIS_VERSION` are redone.
    pub analysis_version: i32,
    /// Analysed length; a different length means a different file under the same id.
    pub duration_ms: i64,
    /// 0 when no tempo was found.
    pub bpm: f64,
    /// 0..1; below ~0.5 the grid is not used for beat matching.
    pub bpm_confidence: f32,
    /// The first beat of the grid, 0 <= offset < one beat.
    pub beat_offset_ms: f64,
    /// 0..1: how well one constant grid fits (the lower of beat spread, 14 ms to pass, and tempo drift
    /// between halves, 1.2 %).
    pub stability: f32,
    /// 0..beats_per_bar-1: which grid beats start a bar.
    pub downbeat_phase: i32,
    pub downbeat_confidence: f32,
    /// 4, or 3 for a waltz; 0 (old rows) means 4.
    pub beats_per_bar: i32,
    /// Integrated loudness of the mono downmix, BS.1770 K-weighting and gating. -70 for silence.
    pub lufs: f32,
    /// Camelot code: 1..12 = 1A..12A (minor), 13..24 = 1B..12B (major), 0 = unknown.
    pub key: i32,
    pub key_confidence: f32,
    /// First rise above and last fall below -55 dBFS.
    pub silence_start_ms: i64,
    pub silence_end_ms: i64,
    /// MixRamp points: first rise above and last fall below 17 dB under the track's loudness.
    pub mixramp_start_ms: i64,
    pub mixramp_end_ms: i64,
    /// Phrase-aligned cues (multiples of 8 bars from the first downbeat, preferring energy jumps).
    /// `intro_end_ms == silence_start_ms`: the track starts at full energy.
    pub intro_end_ms: i64,
    pub outro_start_ms: i64,
    /// Spectral centroid (Hz) over the outro and intro; 0 unknown.
    pub outro_centroid: f32,
    pub intro_centroid: f32,
    /// Beat grids of only the last and first `automix::GRID_WINDOW_S` seconds, where mixes happen (live
    /// tempo drifts over a whole song but not over half a minute). 0 unknown.
    pub outro_bpm: f64,
    pub outro_bpm_confidence: f32,
    pub outro_beat_offset_ms: f64,
    pub outro_stability: f32,
    pub outro_downbeat_phase: i32,
    pub intro_bpm: f64,
    pub intro_bpm_confidence: f32,
    pub intro_beat_offset_ms: f64,
    pub intro_stability: f32,
    pub intro_downbeat_phase: i32,
    /// The drop: first four-bar line where level, lows and chords reach the song's body. 0 when the song starts
    /// full or has no grid.
    pub drop_ms: i64,
    /// Where the ending stops being worth playing (a closing breakdown in the last 24 s, or the silence
    /// before a short hidden track). 0: play to the end.
    pub exit_ms: i64,
    /// The last silence of 6 s or more inside the music; 0 when none.
    pub gap_ms: i64,
    pub gap_end_ms: i64,
    /// Tonal energy of the drop's run-up (most chordal four bars) relative to the body, dB; a drum
    /// intro reads far below 0.
    pub drop_runup_tonal_db: f32,
    /// Per-end metre when measured separately; 0 means `beats_per_bar`.
    pub intro_beats_per_bar: i32,
    pub outro_beats_per_bar: i32,
    /// Grid source per end: `automix::beats::GRID_CLASSICAL`, `GRID_CHECKED` (classical, kept after the
    /// model was unsure) or `GRID_NEURAL`.
    pub intro_grid_source: i32,
    pub outro_grid_source: i32,
    /// The last drop: the last four-bar line in the closing minute where the song comes back to its body
    /// from bars that lacked it (a last chorus after a breakdown). 0 when there is none, or the song has no grid.
    pub last_drop_ms: i64,
    /// Where the song's singing starts (the first of two sung bars) and where its last sung bar ends, before any
    /// hidden track: 0 when nothing is sung.
    pub vocal_start_ms: i64,
    pub vocal_end_ms: i64,
    /// How far the opening's bars keep within 3 dB of each other, from the first bar line: a loop of the opening
    /// ends by it. 0 unknown.
    pub steady_intro_ms: i64,
    /// Where the bars that keep together before the last bar line of the music start, and those before the exit's
    /// (0 without an exit): a loop of the ending starts at or after it. 0 unknown.
    pub steady_end_ms: i64,
    pub steady_exit_ms: i64,
    /// Analysis time, ms since the epoch.
    pub analysed_ms: i64,
}

/// How AutoMix mixes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MixStyle {
    /// Short blends on the beat that keep each song nearly whole.
    #[default]
    Smooth,
    /// Phrase-long mixes: longer run-ups, looped intros and outros, the tempo gliding between the songs,
    /// and drops landing together.
    Dj,
}

impl MixStyle {
    /// The longest transition, seconds.
    pub fn longest_s(self) -> i32 {
        match self {
            MixStyle::Smooth => 12,
            // Sixteen bars at 96 BPM, the length the end grids are measured over.
            MixStyle::Dj => 40,
        }
    }

    /// The most a song's tempo is changed to match the other's, percent.
    pub fn tempo_pct(self) -> f32 {
        match self {
            MixStyle::Smooth => 6.0,
            MixStyle::Dj => 10.0,
        }
    }
}

/// AutoMix settings as the planner sees them.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoMixSettings {
    /// Longest transition, seconds.
    pub max_transition_s: f32,
    pub beat_match: bool,
    /// Largest tempo change applied to the incoming track, percent.
    pub max_tempo_change_pct: f32,
    pub bass_swap: bool,
    pub filter_effects: bool,
    /// Beat-synced echo-out for clashing pairs (two vocals, far keys); a plain fade when off.
    pub echo_out: bool,
    /// Time-stretch (true) or varispeed (moves pitch, so held to 2 %).
    pub keep_pitch: bool,
    /// The tracks are consecutive on one album played in order: no transition.
    pub same_album_in_order: bool,
    /// Trim the incoming track to the outgoing one's loudness (off when ReplayGain levels them).
    pub match_loudness: bool,
    /// Tag BPM of the outgoing track (0 unknown), to settle half/double tempo errors.
    pub out_tag_bpm: f32,
    /// Server/tag BPM for the incoming track (0 unknown).
    pub in_tag_bpm: f32,
    pub style: MixStyle,
}

impl Default for AutoMixSettings {
    fn default() -> Self {
        AutoMixSettings {
            max_transition_s: 16.0,
            beat_match: true,
            max_tempo_change_pct: 6.0,
            bass_swap: true,
            filter_effects: true,
            echo_out: true,
            keep_pitch: true,
            same_album_in_order: false,
            match_loudness: false,
            out_tag_bpm: 0.0,
            in_tag_bpm: 0.0,
            style: MixStyle::Smooth,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransitionKind {
    /// No overlap: the next track follows sample for sample.
    Gapless,
    /// Fixed cos/sin crossfade; nothing is known about either track.
    EqualPowerFade,
    /// Overlap chosen from loudness ramps and trimmed silence, optionally with a filter sweep.
    MixRampFade,
    /// Tempo-locked, bar-aligned mix, optionally with a bass swap.
    BeatMatched,
    /// The outgoing track exits into a beat-synced echo while the incoming track fades in over its tail.
    EchoOut,
    /// The outgoing track stops on a downbeat and the incoming one comes in at full there, after a riser; an
    /// instrumental ending's last beats echo over it.
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FadeCurve {
    /// cos/sin: constant power, for material that does not add coherently.
    EqualPower,
    Linear,
    /// sin²/cos²: constant amplitude, for beat-matched material that adds coherently.
    SineSquared,
}

/// A filter sweep on the outgoing deck over `[start_ms, end_ms]` of the transition, exponential in frequency.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sweep {
    pub start_ms: i64,
    pub end_ms: i64,
    pub from_hz: f32,
    pub to_hz: f32,
}

/// The incoming lows are cut below `cut_hz` until `at_ms`; over `len_ms` they come in and the outgoing lows go.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BassSwap {
    pub at_ms: i64,
    pub len_ms: i64,
    pub cut_hz: f32,
}

/// Beat-synced echo on the outgoing deck: `delay_ms` is one outgoing beat, `feedback` (0..1) what each repeat
/// keeps, `wet_db` the repeats' level.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Echo {
    pub delay_ms: i64,
    pub feedback: f32,
    pub wet_db: f32,
}

/// The incoming voice band (centred on `hz`) is held `db` down until `until_ms`, released over the
/// `release_ms` before it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VocalDuck {
    pub until_ms: i64,
    pub release_ms: i64,
    pub db: f32,
    pub hz: f32,
}

/// The incoming track's opening looped: its first `ms` (wall-clock, from `in_start_ms`) play `repeats` more times
/// before it plays on.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InLoop {
    pub ms: i64,
    pub repeats: i32,
}

/// Relative. The tempo both decks play at glides from the outgoing song's to the incoming one's between
/// `start_ms` and `end_ms`, linearly in time: the incoming track's speed from `tempo_ratio` to 1, the
/// outgoing one's from 1 to `1 / tempo_ratio`, so their beats stay together.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bend {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// A transition between two tracks. "Relative" fields count from the transition's start.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransitionPlan {
    pub kind: TransitionKind,
    /// Position in the outgoing track where the transition starts. It stops at `out_start_ms + duration_ms`.
    pub out_start_ms: i64,
    /// Position in the incoming track that plays at the start of the transition.
    pub in_start_ms: i64,
    /// Length of the overlap, wall-clock.
    pub duration_ms: i64,
    /// Playback speed of the incoming track during the overlap (1 = native).
    pub tempo_ratio: f64,
    /// After the overlap the incoming track ramps back to native speed over this many of its beats,
    pub tempo_ramp_beats: i32,
    /// which takes this long, wall-clock. 0 when there is no tempo change.
    pub tempo_ramp_ms: i64,
    /// Time-stretch (true) or varispeed.
    pub keep_pitch: bool,
    pub fade_curve: FadeCurve,
    /// Relative. The outgoing gain goes 1 -> 0 between these.
    pub out_fade_start_ms: i64,
    pub out_fade_end_ms: i64,
    /// Relative. The incoming gain goes 0 -> 1 between these.
    pub in_fade_start_ms: i64,
    pub in_fade_end_ms: i64,
    /// Constant trim on the outgoing deck during the overlap.
    pub out_gain_db: f32,
    /// Trim on the incoming deck; the mixer glides it back to 0 dB by the end of the overlap.
    pub in_gain_db: f32,
    /// Relative.
    pub bass_swap: Option<BassSwap>,
    /// Relative. Low-pass sweep on the outgoing track.
    pub low_pass: Option<Sweep>,
    /// Relative. High-pass sweep on the outgoing track (DJ "filter open").
    pub high_pass: Option<Sweep>,
    pub echo: Option<Echo>,
    /// Outro loop: the engine holds this many ms of the outgoing track and reads it round for `duration_ms`.
    pub out_loop_ms: Option<i64>,
    /// Relative.
    pub vocal_duck: Option<VocalDuck>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub in_loop: Option<InLoop>,
    /// With a bend there is no ramp after the overlap (`tempo_ramp_ms` is 0).
    #[cfg_attr(feature = "serde", serde(default))]
    pub bend: Option<Bend>,
    /// Why this plan, for logs.
    pub reason: String,
}

impl TransitionPlan {
    /// The same transition over `duration_ms`: every relative time scaled to it. An incoming loop is
    /// whole bars, so it is not kept, and a bend becomes a ramp after it.
    pub fn squeeze(&mut self, duration_ms: i64) {
        let k = duration_ms as f64 / self.duration_ms.max(1) as f64;
        let at = |ms: &mut i64| *ms = (*ms as f64 * k).round() as i64;
        self.duration_ms = duration_ms;
        for ms in [&mut self.out_fade_start_ms, &mut self.out_fade_end_ms, &mut self.in_fade_start_ms, &mut self.in_fade_end_ms] {
            at(ms);
        }
        if let Some(s) = self.bass_swap.as_mut() {
            at(&mut s.at_ms);
            at(&mut s.len_ms);
        }
        for s in [self.low_pass.as_mut(), self.high_pass.as_mut()].into_iter().flatten() {
            at(&mut s.start_ms);
            at(&mut s.end_ms);
        }
        if let Some(d) = self.vocal_duck.as_mut() {
            at(&mut d.until_ms);
            at(&mut d.release_ms);
        }
        self.unbend();
        self.in_loop = None;
    }

    /// Whether both plans mix the same audio: equal but for `kind`, `reason` and `tempo_ramp_beats`
    /// (`tempo_ramp_ms` is what runs).
    pub fn sounds_same(&self, o: &TransitionPlan) -> bool {
        let TransitionPlan { kind: _, reason: _, tempo_ramp_beats: _, out_start_ms, in_start_ms, duration_ms, tempo_ratio, tempo_ramp_ms, keep_pitch, fade_curve, out_fade_start_ms, out_fade_end_ms, in_fade_start_ms, in_fade_end_ms, out_gain_db, in_gain_db, bass_swap, low_pass, high_pass, echo, out_loop_ms, vocal_duck, in_loop, bend } = self;
        (*out_start_ms, *in_start_ms, *duration_ms, *tempo_ramp_ms, *out_fade_start_ms, *out_fade_end_ms, *in_fade_start_ms, *in_fade_end_ms, *out_loop_ms)
            == (o.out_start_ms, o.in_start_ms, o.duration_ms, o.tempo_ramp_ms, o.out_fade_start_ms, o.out_fade_end_ms, o.in_fade_start_ms, o.in_fade_end_ms, o.out_loop_ms)
            && (*tempo_ratio, *keep_pitch, *fade_curve, *out_gain_db, *in_gain_db) == (o.tempo_ratio, o.keep_pitch, o.fade_curve, o.out_gain_db, o.in_gain_db)
            && (*bass_swap, *low_pass, *high_pass, *echo, *vocal_duck) == (o.bass_swap, o.low_pass, o.high_pass, o.echo, o.vocal_duck)
            && (*in_loop, *bend) == (o.in_loop, o.bend)
    }

    /// Where the tempo changes: from `tempo_ratio` to 1 over (start, length) ms of the transition, the bend or
    /// the ramp after the overlap.
    fn glide(&self) -> (f64, f64) {
        match self.bend {
            Some(b) => (b.start_ms as f64, (b.end_ms - b.start_ms).max(0) as f64),
            None => (self.duration_ms as f64, self.tempo_ramp_ms.max(0) as f64),
        }
    }

    /// The incoming track's speed `t_ms` into the transition (past its end too: the bend, or the ramp after
    /// the overlap).
    pub fn in_ratio_at(&self, t_ms: f64) -> f64 {
        let (r, (from, len)) = (self.tempo_ratio, self.glide());
        if t_ms <= from {
            r
        } else if len <= 0.0 || t_ms >= from + len {
            1.0
        } else {
            r + (1.0 - r) * (t_ms - from) / len
        }
    }

    /// Incoming song time the deck plays in the first `t_ms` of the transition, the loop's repeats counted
    /// as played.
    pub fn in_played_ms(&self, t_ms: f64) -> f64 {
        let (r, (from, len)) = (self.tempo_ratio, self.glide());
        let t = t_ms.max(0.0);
        if t <= from {
            return t * r;
        }
        let ramp = (t - from).min(len);
        from * r + ramp * (r + self.in_ratio_at(from + ramp)) / 2.0 + (t - from - ramp)
    }

    /// The loop's repeats, wall-clock ms.
    pub fn repeats_ms(&self) -> i64 {
        self.in_loop.map_or(0, |l| l.ms.max(0) * l.repeats.max(0) as i64)
    }

    /// Where the incoming song is `t_ms` into the transition, ms past `in_start_ms`: what it played, less the
    /// loop's repeats (it stands at the loop's end while they play).
    pub fn in_song_ms(&self, t_ms: f64) -> f64 {
        let looped = match self.in_loop {
            Some(l) => (t_ms - l.ms as f64).clamp(0.0, self.repeats_ms() as f64),
            None => 0.0,
        };
        self.in_played_ms(t_ms) - looped * self.tempo_ratio
    }

    /// The transition time at which the incoming song's `song_ms` past `in_start_ms` is heard; a moment of the
    /// loop, the first time ([`TransitionPlan::in_song_ms`] inverted).
    pub fn wall_of_in_song(&self, song_ms: f64) -> f64 {
        let looped = self.in_loop.filter(|l| song_ms >= l.ms as f64 * self.tempo_ratio).map_or(0.0, |_| self.repeats_ms() as f64 * self.tempo_ratio);
        let played = song_ms + looped;
        let (mut lo, mut hi) = (0.0, played.max(0.0) / self.tempo_ratio.clamp(0.01, 1.0) + 1.0);
        for _ in 0..48 {
            let mid = (lo + hi) / 2.0;
            if self.in_played_ms(mid) < played {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }

    /// Outgoing song time played in the first `t_ms` of the transition: under a bend its speed is the
    /// incoming one over `tempo_ratio`.
    pub fn out_song_ms(&self, t_ms: f64) -> f64 {
        match self.bend {
            Some(_) if self.tempo_ratio > 0.0 => self.in_played_ms(t_ms) / self.tempo_ratio,
            _ => t_ms.max(0.0),
        }
    }

    /// The outgoing speed `t_ms` into the transition.
    pub fn out_ratio_at(&self, t_ms: f64) -> f64 {
        match self.bend {
            Some(_) if self.tempo_ratio > 0.0 => self.in_ratio_at(t_ms) / self.tempo_ratio,
            _ => 1.0,
        }
    }

    /// The bend becomes a tempo ramp after the overlap, as long, both decks at their own speed until then: for a
    /// transition played other than whole from its start.
    pub fn unbend(&mut self) {
        if let Some(b) = self.bend.take() {
            self.tempo_ramp_ms = (b.end_ms - b.start_ms).max(0);
        }
    }
}
