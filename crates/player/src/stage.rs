//! Sound stages from outside the player (plugins'): after the equalizer, before a ceiling that keeps
//! whatever they make from clipping, ahead of silence skipping and speed. Each runs in place on a
//! block of float samples held channel after channel, and keeps all that changes what it makes next
//! where [`SoundStage::save`] reaches it, so the chain can go back to any frame it still holds.

use std::sync::Arc;

/// A stage of samples.
pub trait SoundStage: Send {
    /// Its inputs (its rows' values and what it reads besides samples) from now on.
    fn inputs(&mut self, inputs: &[f64]);
    /// Samples channel after channel, the same number per channel, changed in place.
    fn process(&mut self, planar: &mut [f32]);
    /// As before its first sample.
    fn reset(&mut self);
    /// Everything that changes what it makes next, its inputs included, into `into` (its memory kept).
    fn save(&mut self, into: &mut Vec<u8>);
    fn restore(&mut self, from: &[u8]);
}

/// Makes a plugin's stage for a format.
pub trait StageMaker: Send + Sync {
    fn make(&self, rate: u32, channels: usize, inputs: &[f64]) -> Option<Box<dyn SoundStage>>;
}

/// A stage the settings ask for: its maker and its inputs.
#[derive(Clone)]
pub struct StageSpec {
    pub maker: Arc<dyn StageMaker>,
    pub inputs: Vec<f64>,
}

impl StageSpec {
    fn same_maker(&self, maker: &Arc<dyn StageMaker>) -> bool {
        Arc::ptr_eq(&self.maker, maker)
    }
}

impl PartialEq for StageSpec {
    fn eq(&self, o: &Self) -> bool {
        self.same_maker(&o.maker) && self.inputs == o.inputs
    }
}

impl std::fmt::Debug for StageSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("StageSpec").field("inputs", &self.inputs).finish_non_exhaustive()
    }
}

/// A stage in the chain. One turned off stays, passing samples through, until the next flush.
struct Live {
    maker: Arc<dyn StageMaker>,
    stage: Box<dyn SoundStage>,
    on: bool,
}

/// What [`Stages::save`] kept: each stage's state, and whether it was on.
#[derive(Default)]
pub(crate) struct Saved {
    stages: Vec<(bool, Vec<u8>)>,
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

    /// Back to none (a new format): `specs` are made for it.
    pub(crate) fn build(&mut self, specs: &[StageSpec], rate: u32, channels: usize) {
        self.live.clear();
        for s in specs {
            if let Some(stage) = s.maker.make(rate, channels, &s.inputs) {
                self.live.push(Live { maker: s.maker.clone(), stage, on: true });
            }
        }
    }

    /// Goes on with `specs`: a stage already in takes its new inputs, a new one is made, one no longer
    /// asked for is turned off.
    pub(crate) fn apply(&mut self, specs: &[StageSpec], rate: u32, channels: usize) {
        for l in &mut self.live {
            l.on = false;
        }
        for s in specs {
            match self.live.iter_mut().find(|l| s.same_maker(&l.maker)) {
                Some(l) => {
                    l.stage.inputs(&s.inputs);
                    l.on = true;
                }
                None => {
                    if let Some(stage) = s.maker.make(rate, channels, &s.inputs) {
                        self.live.push(Live { maker: s.maker.clone(), stage, on: true });
                    }
                }
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
            into.stages.push((false, Vec::new()));
        }
        for (l, (on, state)) in self.live.iter_mut().zip(&mut into.stages) {
            *on = l.on;
            l.stage.save(state);
        }
    }

    /// Back to what `saved` kept; a stage made since is off and as new.
    pub(crate) fn restore(&mut self, saved: &Saved) {
        for (k, l) in self.live.iter_mut().enumerate() {
            match saved.stages.get(k) {
                Some((on, state)) => {
                    l.on = *on;
                    l.stage.restore(state);
                }
                None => {
                    l.on = false;
                    l.stage.reset();
                }
            }
        }
    }

    /// Runs `data` (interleaved frames of `channels` in `encoding`) through the stages that are on, in place.
    pub(crate) fn run(&mut self, data: &mut [u8], float: bool, channels: usize) {
        let width = if float { 4 } else { 2 };
        let channels = channels.max(1);
        let frames = data.len() / width / channels;
        // Only a longer block grows it.
        self.planar.resize(frames * channels, 0.0);
        for (k, sample) in data.chunks_exact(width).enumerate() {
            self.planar[(k % channels) * frames + k / channels] = sample_in(sample);
        }
        for l in self.live.iter_mut().filter(|l| l.on) {
            l.stage.process(&mut self.planar[..frames * channels]);
        }
        for (k, sample) in data.chunks_exact_mut(width).enumerate() {
            sample_out(sample, ceiling(self.planar[(k % channels) * frames + k / channels]));
        }
    }
}

/// One sample's bytes (16-bit or float, by their width) as a float.
fn sample_in(b: &[u8]) -> f32 {
    match *b {
        [a, b] => i16::from_le_bytes([a, b]) as f32 / 32768.0,
        [a, b, c, d] => f32::from_le_bytes([a, b, c, d]),
        _ => 0.0,
    }
}

/// `v` into one sample's bytes, rounded as the chain rounds 16-bit samples.
fn sample_out(b: &mut [u8], v: f32) {
    match b.len() {
        2 => b.copy_from_slice(&((v * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes()),
        _ => b.copy_from_slice(&v.to_le_bytes()),
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
}
