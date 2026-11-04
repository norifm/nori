//! Sound stages from outside the player (plugins'): in the master slot after the equalizer, before a
//! ceiling that keeps whatever they make from clipping, ahead of silence skipping and speed; in the
//! song slot on each song as it is read, before transitions. Each runs in place on a block of float
//! samples held channel after channel, and keeps all that changes what it makes next where
//! [`SoundStage::save`] reaches it, so the chain can go back to any frame it still holds. Besides its
//! rows a stage reads the volume, the route and the song's facts ([`Inputs`]); a change of the volume,
//! the route or its rows is spliced in like a sound change, and a song's facts start at its first frame.

use std::sync::Arc;

use crate::outputs::OutputKind;

/// Where in the chain a stage runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// On each song as it is read, before transitions: one song at a time.
    Song,
    /// After the equalizer, before the ceiling.
    Master,
}

/// What every stage reads that is the same for every song: the volume and where the music plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Around {
    /// The output's volume, dB (0 at full).
    pub volume_db: f32,
    pub route: OutputKind,
}

impl Default for Around {
    fn default() -> Self {
        Around { volume_db: 0.0, route: OutputKind::Other }
    }
}

/// A volume this low is silence, dB.
pub const VOLUME_FLOOR_DB: f32 = -96.0;

impl Around {
    /// A volume `level` (0 to 1) in dB.
    pub fn volume_of(level: f32) -> f32 {
        if level.is_finite() && level > 0.0 {
            (20.0 * level.min(1.0).log10()).max(VOLUME_FLOOR_DB)
        } else {
            VOLUME_FLOOR_DB
        }
    }
}

/// What a stage reads of the song it plays (in the master slot, the song coming out).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SongFacts {
    /// The song's id, for an output that is told where each song starts (`Track::marks_songs`).
    pub id: Option<String>,
    /// The gain nori levels the song by, dB.
    pub gain_db: Option<f32>,
    /// Its measured integrated loudness, LUFS.
    pub loudness_lufs: Option<f32>,
    /// Each plugin's prepared data for the song, by its id.
    pub prepared: Vec<(String, Arc<[u8]>)>,
    /// Where in the song its reading began, ms: 0 at its start, where a seek went after one.
    pub from_ms: i64,
}

impl SongFacts {
    /// What `plugin` prepared for the song.
    pub fn prepared_by(&self, plugin: &str) -> Option<&Arc<[u8]>> {
        self.prepared.iter().find(|(p, _)| p == plugin).map(|(_, d)| d)
    }
}

/// The facts of a song nothing is known of, shared.
pub fn no_facts() -> Arc<SongFacts> {
    static NONE: std::sync::OnceLock<Arc<SongFacts>> = std::sync::OnceLock::new();
    NONE.get_or_init(Arc::default).clone()
}

/// What plugins make of songs as the player reads them (their song preparation and AutoMix analysis),
/// and what they made.
pub trait Preparing: Send + Sync {
    /// Song `id` of `format` is read from frame `from`: where its samples go, while a plugin wants them.
    fn tap(&self, id: &str, format: crate::pcm::Format, from: u64) -> Option<Box<dyn Tap>>;
    /// Each plugin's prepared data for song `id`, as far as it is in.
    fn prepared(&self, id: &str) -> Vec<(String, Arc<[u8]>)>;
    /// `wake` is called whenever a song's data comes in, from then on.
    fn on_prepared(&self, wake: Arc<dyn Fn() + Send + Sync>);
}

/// The [`Preparing`] the settings hold: the same one is equal.
#[derive(Clone)]
pub struct Prepares(pub Arc<dyn Preparing>);

impl PartialEq for Prepares {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl std::fmt::Debug for Prepares {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Prepares")
    }
}

/// Where a song's decoded samples go as the player reads them. It never waits: what it can't take
/// it drops. Dropped before [`Tap::end`], the song was left.
pub trait Tap: Send {
    /// The next interleaved samples, in order.
    fn samples(&mut self, interleaved: &[f32]);
    /// The song was read to its end.
    fn end(&mut self);
}

/// Everything a stage reads besides samples. Two are the same when they hold the same song's facts by
/// identity (prepared data may be large).
#[derive(Debug)]
pub struct Inputs {
    /// Its rows' values in manifest order.
    pub rows: Vec<f64>,
    pub around: Around,
    pub song: Arc<SongFacts>,
}

impl Clone for Inputs {
    fn clone(&self) -> Self {
        Inputs { rows: self.rows.clone(), around: self.around, song: self.song.clone() }
    }

    /// Keeps `rows`' memory.
    fn clone_from(&mut self, o: &Self) {
        self.rows.clone_from(&o.rows);
        self.around = o.around;
        self.song.clone_from(&o.song);
    }
}

impl PartialEq for Inputs {
    fn eq(&self, o: &Self) -> bool {
        self.rows == o.rows && self.around == o.around && Arc::ptr_eq(&self.song, &o.song)
    }
}

/// A stage of samples.
pub trait SoundStage: Send {
    /// Its inputs from now on.
    fn inputs(&mut self, inputs: &Inputs);
    /// Samples channel after channel, the same number per channel, changed in place.
    fn process(&mut self, planar: &mut [f32]);
    /// As before its first sample.
    fn reset(&mut self);
    /// Everything of its own that changes what it makes next into `into` (its memory kept); the
    /// chain keeps its inputs beside it.
    fn save(&mut self, into: &mut Vec<u8>);
    fn restore(&mut self, from: &[u8]);
}

/// Makes a plugin's stage for a format.
pub trait StageMaker: Send + Sync {
    fn make(&self, rate: u32, channels: usize, inputs: &Inputs) -> Option<Box<dyn SoundStage>>;
}

/// A stage the settings ask for: its maker, its slot and its rows' values.
#[derive(Clone)]
pub struct StageSpec {
    pub maker: Arc<dyn StageMaker>,
    pub slot: Slot,
    pub rows: Vec<f64>,
}

impl StageSpec {
    fn same_maker(&self, maker: &Arc<dyn StageMaker>) -> bool {
        Arc::ptr_eq(&self.maker, maker)
    }

    fn inputs(&self, around: Around, song: &Arc<SongFacts>) -> Inputs {
        Inputs { rows: self.rows.clone(), around, song: song.clone() }
    }
}

impl PartialEq for StageSpec {
    fn eq(&self, o: &Self) -> bool {
        self.same_maker(&o.maker) && self.slot == o.slot && self.rows == o.rows
    }
}

impl std::fmt::Debug for StageSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("StageSpec").field("slot", &self.slot).field("rows", &self.rows).finish_non_exhaustive()
    }
}

/// A stage in the chain and the inputs it was last given. One turned off stays, passing samples
/// through, until the next flush.
struct Live {
    maker: Arc<dyn StageMaker>,
    stage: Box<dyn SoundStage>,
    inputs: Inputs,
    on: bool,
}

impl Live {
    /// Gives the stage `inputs` unless they are what it has.
    fn take(&mut self, inputs: &Inputs) {
        if self.inputs != *inputs {
            self.inputs.clone_from(inputs);
            self.stage.inputs(&self.inputs);
        }
    }
}

/// What [`Stages::save`] kept of each stage: whether it was on, its inputs and its state.
#[derive(Default)]
pub(crate) struct Saved {
    stages: Vec<(bool, Option<Inputs>, Vec<u8>)>,
}

/// Above this the ceiling bends samples down, so a stage's boost never clips.
const CEILING_KNEE: f32 = 0.9;

/// The chain's stages, in the settings' order, and the block they run on.
#[derive(Default)]
pub(crate) struct Stages {
    live: Vec<Live>,
    planar: Vec<f32>,
}

impl Stages {
    pub(crate) fn any_on(&self) -> bool {
        self.live.iter().any(|l| l.on)
    }

    /// Back to none (a new format): `specs` are made for it, around `around`, for `song`.
    pub(crate) fn build(&mut self, specs: &[StageSpec], around: Around, song: &Arc<SongFacts>, rate: u32, channels: usize) {
        self.live.clear();
        for s in specs {
            let inputs = s.inputs(around, song);
            if let Some(stage) = s.maker.make(rate, channels, &inputs) {
                self.live.push(Live { maker: s.maker.clone(), stage, inputs, on: true });
            }
        }
    }

    /// Goes on with `specs` around `around`: a stage already in takes its new inputs, a new one is
    /// made for the song played now, one no longer asked for is turned off.
    pub(crate) fn apply(&mut self, specs: &[StageSpec], around: Around, song: &Arc<SongFacts>, rate: u32, channels: usize) {
        for l in &mut self.live {
            l.on = false;
        }
        for s in specs {
            match self.live.iter_mut().find(|l| s.same_maker(&l.maker)) {
                Some(l) => {
                    let inputs = s.inputs(around, &l.inputs.song);
                    l.take(&inputs);
                    l.on = true;
                }
                None => {
                    let inputs = s.inputs(around, song);
                    if let Some(stage) = s.maker.make(rate, channels, &inputs) {
                        self.live.push(Live { maker: s.maker.clone(), stage, inputs, on: true });
                    }
                }
            }
        }
    }

    /// What is run from here on is the song `song` tells of.
    pub(crate) fn song(&mut self, song: &Arc<SongFacts>) {
        for l in &mut self.live {
            if !Arc::ptr_eq(&l.inputs.song, song) {
                l.inputs.song.clone_from(song);
                l.stage.inputs(&l.inputs);
            }
        }
    }

    /// Whether the stages in the chain are those `specs` ask for, none turned off.
    pub(crate) fn are(&self, specs: &[StageSpec]) -> bool {
        self.live.len() == specs.len() && self.live.iter().zip(specs).all(|(l, s)| l.on && s.same_maker(&l.maker))
    }

    pub(crate) fn reset(&mut self) {
        for l in &mut self.live {
            l.stage.reset();
        }
    }

    pub(crate) fn save(&mut self, into: &mut Saved) {
        into.stages.truncate(self.live.len());
        while into.stages.len() < self.live.len() {
            into.stages.push((false, None, Vec::new()));
        }
        for (l, (on, inputs, state)) in self.live.iter_mut().zip(&mut into.stages) {
            *on = l.on;
            match inputs {
                Some(i) => i.clone_from(&l.inputs),
                None => *inputs = Some(l.inputs.clone()),
            }
            l.stage.save(state);
        }
    }

    /// Back to what `saved` kept, its inputs first; a stage made since is off and as new.
    pub(crate) fn restore(&mut self, saved: &Saved) {
        for (k, l) in self.live.iter_mut().enumerate() {
            match saved.stages.get(k) {
                Some((on, inputs, state)) => {
                    l.on = *on;
                    if let Some(i) = inputs {
                        l.take(i);
                    }
                    l.stage.restore(state);
                }
                None => {
                    l.on = false;
                    l.stage.reset();
                }
            }
        }
    }

    /// Runs `data` (interleaved frames of `channels`, float or 16-bit) through the stages that are on, in place.
    pub(crate) fn run(&mut self, data: &mut [u8], float: bool, channels: usize) {
        let channels = channels.max(1);
        let frames = data.len() / if float { 4 } else { 2 } / channels;
        // Only a longer block grows it.
        self.planar.resize(frames * channels, 0.0);
        let planar = &mut self.planar[..frames * channels];
        if float {
            to_planar(data.as_chunks::<4>().0, planar, channels, f32::from_le_bytes);
        } else {
            to_planar(data.as_chunks::<2>().0, planar, channels, |b| i16::from_le_bytes(b) as f32 / 32768.0);
        }
        for l in self.live.iter_mut().filter(|l| l.on) {
            l.stage.process(planar);
        }
        if float {
            from_planar(planar, data.as_chunks_mut::<4>().0, channels, |v| ceiling(v).to_le_bytes());
        } else {
            // Rounded as the chain rounds 16-bit samples.
            from_planar(planar, data.as_chunks_mut::<2>().0, channels, |v| ((ceiling(v) * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
        }
    }
}

/// Interleaved `samples` into `planar`, a channel after another.
fn to_planar<const W: usize>(samples: &[[u8; W]], planar: &mut [f32], channels: usize, value: impl Fn([u8; W]) -> f32) {
    let frames = planar.len() / channels;
    for (c, lane) in planar.chunks_exact_mut(frames.max(1)).enumerate() {
        for (v, frame) in lane.iter_mut().zip(samples.chunks_exact(channels)) {
            *v = value(frame[c]);
        }
    }
}

/// `planar` back into interleaved `samples`.
fn from_planar<const W: usize>(planar: &[f32], samples: &mut [[u8; W]], channels: usize, bytes: impl Fn(f32) -> [u8; W]) {
    let frames = planar.len() / channels;
    for (c, lane) in planar.chunks_exact(frames.max(1)).enumerate() {
        for (&v, frame) in lane.iter().zip(samples.chunks_exact_mut(channels)) {
            frame[c] = bytes(v);
        }
    }
}

/// The song slot's stages on one song as it is read: each buffer once, into a copy the song is then
/// read from. Made for the song's format; a seek reads it from a fresh set.
pub(crate) struct SongStages {
    stages: Stages,
    format: crate::pcm::Format,
    out: Vec<u8>,
}

impl SongStages {
    /// `specs` for a song of `format` with `song`'s facts; None without any.
    pub(crate) fn new(specs: &[StageSpec], around: Around, song: &Arc<SongFacts>, format: crate::pcm::Format) -> Option<SongStages> {
        let mut stages = Stages::default();
        stages.build(specs, around, song, format.rate, format.channels);
        stages.any_on().then(|| SongStages { stages, format, out: Vec::new() })
    }

    /// Goes on with `specs`, from the next buffer.
    pub(crate) fn apply(&mut self, specs: &[StageSpec], around: Around) {
        let song = self.stages.live.first().map_or_else(no_facts, |l| l.inputs.song.clone());
        self.stages.apply(specs, around, &song, self.format.rate, self.format.channels);
    }

    /// The song's facts changed (its prepared data came): from the next buffer.
    pub(crate) fn song(&mut self, song: &Arc<SongFacts>) {
        self.stages.song(song);
    }

    /// `buffer` through the stages.
    pub(crate) fn run(&mut self, buffer: &[u8]) -> &[u8] {
        self.out.clear();
        self.out.extend_from_slice(buffer);
        self.stages.run(&mut self.out, self.format.encoding == crate::pcm::Encoding::Float, self.format.channels);
        &self.out
    }

    pub(crate) fn last(&self) -> &[u8] {
        &self.out
    }
}

/// `x` bent smoothly down above [`CEILING_KNEE`], never past full scale.
fn ceiling(x: f32) -> f32 {
    let a = x.abs();
    if a <= CEILING_KNEE {
        return x;
    }
    let room = 1.0 - CEILING_KNEE;
    (CEILING_KNEE + room * ((a - CEILING_KNEE) / room).tanh()).copysign(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Halves the first channel and turns the second over: shows which lane is which.
    struct Lanes;

    impl SoundStage for Lanes {
        fn inputs(&mut self, _: &Inputs) {}
        fn process(&mut self, planar: &mut [f32]) {
            let (left, right) = planar.split_at_mut(planar.len() / 2);
            left.iter_mut().for_each(|v| *v *= 0.5);
            right.iter_mut().for_each(|v| *v = -*v);
        }
        fn reset(&mut self) {}
        fn save(&mut self, _: &mut Vec<u8>) {}
        fn restore(&mut self, _: &[u8]) {}
    }

    impl StageMaker for Lanes {
        fn make(&self, _: u32, _: usize, _: &Inputs) -> Option<Box<dyn SoundStage>> {
            Some(Box::new(Lanes))
        }
    }

    #[test]
    fn a_stage_sees_each_channel_as_its_own_lane() {
        let mut stages = Stages::default();
        let spec = StageSpec { maker: Arc::new(Lanes), slot: Slot::Master, rows: Vec::new() };
        stages.build(&[spec], Around::default(), &no_facts(), 44_100, 2);
        let frames = [(0.5f32, 0.25f32), (-0.5, 0.125), (0.75, -0.5)];

        let mut float: Vec<u8> = frames.iter().flat_map(|&(l, r)| [l, r]).flat_map(f32::to_le_bytes).collect();
        stages.run(&mut float, true, 2);
        let out: Vec<f32> = float.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        assert_eq!(out, frames.iter().flat_map(|&(l, r)| [l * 0.5, -r]).collect::<Vec<_>>());

        let mut pcm: Vec<u8> = frames.iter().flat_map(|&(l, r)| [l, r]).flat_map(|v| ((v * 32768.0) as i16).to_le_bytes()).collect();
        stages.run(&mut pcm, false, 2);
        let out: Vec<i16> = pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        assert_eq!(out, [8192, -8192, -8192, -4096, 12288, 16384]);
    }

    #[test]
    fn the_ceiling_keeps_quiet_samples_and_never_clips() {
        assert_eq!(ceiling(0.5), 0.5);
        assert_eq!(ceiling(-0.9), -0.9);
        for x in [0.95f32, 1.0, 2.0, 40.0] {
            let y = ceiling(x);
            assert!(y > CEILING_KNEE && y <= 1.0, "{x} -> {y}");
            assert_eq!(ceiling(-x), -y);
        }
    }

    #[test]
    fn a_volume_level_in_db() {
        assert_eq!(Around::volume_of(1.0), 0.0);
        assert!((Around::volume_of(0.5) + 6.02).abs() < 0.01);
        assert_eq!(Around::volume_of(0.0), VOLUME_FLOOR_DB);
        assert_eq!(Around::volume_of(f32::NAN), VOLUME_FLOOR_DB);
    }
}
