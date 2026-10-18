//! Signalsmith Stretch 1.3.1, ported from C++: time-stretching with a phase vocoder that predicts
//! each bin's phase from its neighbors in time and frequency, locked across channels.
//!
//! Ported from Signalsmith Stretch, Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd., and Signalsmith
//! Linear, Copyright (c) 2025 Signalsmith Audio, both MIT (see LICENSE in this crate). Only what nori uses is here:
//! the cheaper preset with its split computation, `process`, `flush` and `reset`; no
//! transposition, seeking, formants or custom frequency maps.
//!
//! Buffers are interleaved. Nothing allocates after construction.

mod fft;
mod stft;

use rustfft::num_complex::Complex32;

use stft::{Input, Output, Stft};

const NOISE_FLOOR: f32 = 1e-15;
/// The time-stretch factor above which phases are randomized.
const MAX_CLEAN_STRETCH: f32 = 2.0;
/// The main phase prediction is spread over this many steps.
const SPLIT_MAIN_PREDICTION: usize = 8;

#[derive(Clone, Copy, Default)]
struct Band {
    input: Complex32,
    prev_input: Complex32,
    output: Complex32,
    input_energy: f32,
}

#[derive(Clone, Copy, Default)]
struct Prediction {
    energy: f32,
    input: Complex32,
}

impl Prediction {
    fn make_output(&self, mut phase: Complex32) -> Complex32 {
        let mut phase_norm = phase.norm_sqr();
        if phase_norm <= NOISE_FLOOR {
            // The prediction is too weak: fall back to the input.
            phase = self.input;
            phase_norm = self.input.norm_sqr() + NOISE_FLOOR;
        }
        phase * (self.energy / phase_norm).sqrt()
    }
}

#[derive(Clone, Copy)]
struct MapPoint {
    input_bin: f32,
    freq_grad: f32,
}

/// Where the current block's steps are; a block's work is spread over the interval that reads its output.
#[derive(Clone, Copy)]
struct BlockProcess {
    samples_since_last: usize,
    steps: usize,
    step: usize,
    new_spectrum: bool,
    reanalyze_prev: bool,
    time_factor: f32,
}

impl Default for BlockProcess {
    fn default() -> Self {
        BlockProcess {
            samples_since_last: usize::MAX,
            steps: 0,
            step: 0,
            new_spectrum: false,
            reanalyze_prev: false,
            time_factor: 0.0,
        }
    }
}

/// `minstd_rand0`, the C++ library's default engine, drawing uniformly in a range. Seeded with 1 where the original
/// takes `std::random_device`, so the output is reproducible.
struct MinStd(u32);

impl MinStd {
    fn uniform(&mut self, low: f32, high: f32) -> f32 {
        self.0 = ((self.0 as u64 * 16807) % 2_147_483_647) as u32;
        (high - low) * ((self.0 - 1) as f32 / 2_147_483_646.0) + low
    }
}

/// `a * conj(b)`.
fn mul_conj(a: Complex32, b: Complex32) -> Complex32 {
    Complex32::new(b.re * a.re + b.im * a.im, b.re * a.im - b.im * a.re)
}

trait Lerp: Copy + Default + std::ops::Add<Output = Self> + std::ops::Sub<Output = Self> + std::ops::Mul<f32, Output = Self> {}
impl Lerp for f32 {}
impl Lerp for Complex32 {}

fn band_value<T: Lerp>(bins: &[Band], index: i32, get: fn(&Band) -> T) -> T {
    if index < 0 || index as usize >= bins.len() {
        T::default()
    } else {
        get(&bins[index as usize])
    }
}

fn fractional<T: Lerp>(bins: &[Band], low: i32, frac: f32, get: fn(&Band) -> T) -> T {
    let (lo, hi) = (band_value(bins, low, get), band_value(bins, low + 1, get));
    lo + (hi - lo) * frac
}

fn fractional_at<T: Lerp>(bins: &[Band], index: f32, get: fn(&Band) -> T) -> T {
    let low = index.floor() as i32;
    fractional(bins, low, index - low as f32, get)
}

pub struct Stretch {
    channels: usize,
    bands: usize,
    stft: Stft,
    /// The input as it was when the current block started: what that block analyzes.
    stashed_input: Input,
    /// The output being read while the next block is synthesized into the STFT's own, one interval ahead.
    stashed_output: Output,
    channel_bands: Vec<Band>,
    predictions: Vec<Prediction>,
    output_map: Vec<MapPoint>,
    block: BlockProcess,
    process_spectrum_steps: usize,
    silence_counter: usize,
    silence_first: bool,
    prev_input_offset: i32,
    random: MinStd,
}

impl Stretch {
    /// Signalsmith's `presetCheaper`: 100 ms blocks every 40 ms, the work split across each interval.
    pub fn preset_cheaper(channels: usize, sample_rate: f32) -> Self {
        let block = (sample_rate as f64 * 0.1) as usize;
        let interval = (sample_rate as f64 * 0.04) as usize;
        let stft = Stft::new(channels, block, interval, interval + 1);
        let bands = stft.bands();
        Stretch {
            channels,
            bands,
            stashed_input: stft.input.clone(),
            stashed_output: stft.output.clone(),
            stft,
            channel_bands: vec![Band::default(); bands * channels],
            predictions: vec![Prediction::default(); bands * channels],
            output_map: (0..bands).map(|b| MapPoint { input_bin: b as f32, freq_grad: 1.0 }).collect(),
            block: BlockProcess::default(),
            process_spectrum_steps: 0,
            silence_counter: 0,
            silence_first: true,
            prev_input_offset: -1,
            random: MinStd(1),
        }
    }

    /// Frames between the input passed to `process` and the center of the effect.
    pub fn input_latency(&self) -> usize {
        self.stft.analysis_latency()
    }

    /// Frames between the center of the effect and the output `process` returns.
    pub fn output_latency(&self) -> usize {
        self.stft.synthesis_latency() + self.stft.interval()
    }

    /// Back to the configured state: silent history, nothing in flight.
    pub fn reset(&mut self) {
        self.stft.reset(0.1);
        self.stashed_input.copy_from(&self.stft.input);
        self.stashed_output.copy_from(&self.stft.output);
        self.prev_input_offset = -1;
        self.channel_bands.fill(Band::default());
        self.silence_counter = 0;
        self.block = BlockProcess::default();
    }

    /// Takes in `input` and writes `output`; the ratio of their lengths is the time-stretch.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let ch = self.channels;
        let (in_frames, out_frames) = (input.len() / ch, output.len() / ch);
        let interval = self.stft.interval();
        let mut copied = 0;

        let mut total_energy = 0f32;
        for c in 0..ch {
            for i in 0..in_frames {
                let s = input[i * ch + c];
                total_energy += s * s;
            }
        }
        if total_energy < NOISE_FLOOR {
            if self.silence_counter >= 2 * self.stft.block_samples() {
                if self.silence_first {
                    self.silence_first = false;
                    self.block = BlockProcess::default();
                    self.channel_bands.fill(Band::default());
                }
                if in_frames > 0 {
                    // Silence in, silence out: the input, repeated to fill the output.
                    for o in 0..out_frames {
                        let i = o % in_frames;
                        output[o * ch..(o + 1) * ch].copy_from_slice(&input[i * ch..(i + 1) * ch]);
                    }
                } else {
                    output[..out_frames * ch].fill(0.0);
                }
                self.copy_input(input, &mut copied, in_frames);
                return;
            }
            self.silence_counter += in_frames;
        } else {
            self.silence_counter = 0;
            self.silence_first = true;
        }

        for o in 0..out_frames {
            if self.block.samples_since_last >= interval {
                self.start_block(input, &mut copied, o, in_frames, out_frames);
            }
            let ratio = (self.block.samples_since_last + 1) as f32 / interval as f32;
            let steps = self.block.steps;
            let process_to = steps.min(((steps as f32 + 0.999) * ratio) as usize);
            while self.block.step < process_to {
                let step = self.block.step;
                self.block.step += 1;
                self.run_step(step);
            }
            self.block.samples_since_last += 1;
            for c in 0..ch {
                output[o * ch + c] = self.stashed_output.read(c, 0);
            }
            self.stashed_output.advance(1);
        }
        self.copy_input(input, &mut copied, in_frames);
        self.prev_input_offset -= in_frames as i32;
    }

    /// Writes out what is still inside, with no further input; `output` should hold at least
    /// [`output_latency`](Self::output_latency) frames.
    pub fn flush(&mut self, output: &mut [f32]) {
        let ch = self.channels;
        let frames = output.len() / ch;
        let block = self.stft.block_samples();
        let plain = frames.min(block);
        let folded_back = frames.min(block - plain);
        self.stft.finish_output(1.0);
        for c in 0..ch {
            for i in 0..plain {
                output[i * ch + c] = self.stft.output.read(c, i);
            }
            for i in 0..folded_back {
                output[(frames - 1 - i) * ch + c] -= self.stft.output.read(c, plain + i);
            }
        }
        self.stft.reset(0.1);
        // The phase vocoder starts afresh on the next block.
        for b in &mut self.channel_bands {
            b.prev_input = Complex32::default();
            b.output = Complex32::default();
        }
    }

    /// Copies the input from where the last copy ended up to frame `to` (at most a block and an interval of it).
    fn copy_input(&mut self, input: &[f32], copied: &mut usize, to: usize) {
        let length = (self.stft.block_samples() + self.stft.interval()).min(to - *copied);
        self.stft.write_input(input, to - length, length);
        *copied = to;
    }

    fn start_block(&mut self, input: &[f32], copied: &mut usize, out_index: usize, in_frames: usize, out_frames: usize) {
        let (ch, interval) = (self.channels, self.stft.interval());
        let b = &mut self.block;
        (b.step, b.steps, b.samples_since_last) = (0, 0, 0);
        // Where in the input this spectrum comes from.
        let input_offset = (out_index as f32 * in_frames as f32 / out_frames as f32).round() as i32;
        let input_interval = input_offset - self.prev_input_offset;
        self.prev_input_offset = input_offset;
        self.copy_input(input, copied, input_offset as usize);
        self.stashed_input.copy_from(&self.stft.input);
        self.stashed_output.copy_from(&self.stft.output);
        // The synthesis runs one interval ahead of what is read.
        self.stft.output.advance(interval);

        let b = &mut self.block;
        b.new_spectrum = input_interval > 0;
        if b.new_spectrum {
            // The previous analysis must be one interval back, give or take a frame.
            b.reanalyze_prev = (input_interval - interval as i32).abs() > 1;
            if b.reanalyze_prev {
                b.steps += ch + 1;
            }
            b.steps += ch + 1;
        }
        b.time_factor = interval as f32 / (input_interval as f32).max(1.0);
        self.process_spectrum_steps = self.spectrum_steps();
        let b = &mut self.block;
        b.steps += self.process_spectrum_steps;
        b.steps += ch + 1;
    }

    fn spectrum_steps(&self) -> usize {
        let ch = self.channels;
        let mut steps = 0;
        if self.block.new_spectrum {
            steps += ch;
        }
        steps += 1 + ch + SPLIT_MAIN_PREDICTION;
        if self.block.new_spectrum {
            steps += 1;
        }
        steps
    }

    fn run_step(&mut self, mut step: usize) {
        let (ch, bands) = (self.channels, self.bands);
        if self.block.new_spectrum {
            if self.block.reanalyze_prev {
                if step < ch {
                    self.stft.analyze(step, &self.stashed_input, self.stft.interval());
                    return;
                }
                step -= ch;
                if step < 1 {
                    for c in 0..ch {
                        let spectrum = self.stft.spectrum(c);
                        for (band, s) in self.channel_bands[c * bands..(c + 1) * bands].iter_mut().zip(spectrum) {
                            band.prev_input = *s;
                        }
                    }
                    return;
                }
                step -= 1;
            }
            if step < ch {
                self.stft.analyze(step, &self.stashed_input, 0);
                return;
            }
            step -= ch;
            if step < 1 {
                for c in 0..ch {
                    let spectrum = self.stft.spectrum(c);
                    for (band, s) in self.channel_bands[c * bands..(c + 1) * bands].iter_mut().zip(spectrum) {
                        band.input = *s;
                    }
                }
                return;
            }
            step -= 1;
        }
        if step < self.process_spectrum_steps {
            self.process_spectrum(step);
            return;
        }
        step -= self.process_spectrum_steps;
        if step < 1 {
            for c in 0..ch {
                let spectrum = self.stft.spectrum_mut(c);
                for (s, band) in spectrum.iter_mut().zip(&self.channel_bands[c * bands..(c + 1) * bands]) {
                    *s = band.output;
                }
            }
            return;
        }
        step -= 1;
        if step < ch {
            self.stft.synthesize(step);
        }
    }

    fn process_spectrum(&mut self, mut step: usize) {
        let (ch, bands) = (self.channels, self.bands);
        let interval = self.stft.interval();
        let smoothing_bins = self.stft.fft_samples() as f32 / interval as f32;
        let long_step = smoothing_bins.round() as i32;
        let time_factor = self.block.time_factor.max(1.0 / MAX_CLEAN_STRETCH);
        let random_time_factor = time_factor > MAX_CLEAN_STRETCH;
        let random_low = MAX_CLEAN_STRETCH * 2.0 * random_time_factor as i32 as f32 - time_factor;

        if self.block.new_spectrum {
            if step < ch {
                // Move the previous input and output on by one interval at each bin's frequency.
                let tau = std::f32::consts::TAU;
                let mut rot = Complex32::from_polar(1.0, self.stft.bin_to_freq(0.0) * interval as f32 * tau);
                let freq_step = self.stft.bin_to_freq(1.0) - self.stft.bin_to_freq(0.0);
                let rot_step = Complex32::from_polar(1.0, freq_step * interval as f32 * tau);
                for bin in &mut self.channel_bands[step * bands..(step + 1) * bands] {
                    bin.output *= rot;
                    bin.prev_input *= rot;
                    rot *= rot_step;
                }
                return;
            }
            step -= ch;
        }
        if step == 0 {
            for bin in &mut self.channel_bands {
                bin.input_energy = bin.input.norm_sqr();
            }
            return;
        }
        step -= 1;
        if step < ch {
            self.predict_from_previous(step);
            return;
        }
        step -= ch;
        if step < SPLIT_MAIN_PREDICTION {
            let (start, end) = (bands * step / SPLIT_MAIN_PREDICTION, bands * (step + 1) / SPLIT_MAIN_PREDICTION);
            let tf = if random_time_factor { Some((random_low, time_factor)) } else { None };
            for b in start..end {
                self.predict_bin(b, time_factor, tf, long_step);
            }
            return;
        }
        step -= SPLIT_MAIN_PREDICTION;
        if self.block.new_spectrum && step == 0 {
            for bin in &mut self.channel_bands {
                bin.prev_input = bin.input;
            }
        }
    }

    /// The preliminary phase-vocoder prediction for one channel: each output bin moved on by its input's phase
    /// change since the previous block.
    fn predict_from_previous(&mut self, c: usize) {
        let bands = self.bands;
        let bins = &mut self.channel_bands[c * bands..(c + 1) * bands];
        let predictions = &mut self.predictions[c * bands..(c + 1) * bands];
        for b in 0..bands {
            let map = self.output_map[b];
            let low = map.input_bin.floor() as i32;
            let frac = map.input_bin - low as f32;
            let prediction = &mut predictions[b];
            let prev_energy = prediction.energy;
            // Scaled by the local stretch of the frequency map.
            prediction.energy = fractional(bins, low, frac, |b| b.input_energy) * map.freq_grad.max(0.0);
            prediction.input = fractional(bins, low, frac, |b| b.input);
            let prev_input = fractional(bins, low, frac, |b| b.prev_input);
            let freq_twist = mul_conj(prediction.input, prev_input);
            let phase = bins[b].output * freq_twist;
            bins[b].output = phase / (prev_energy.max(prediction.energy) + NOISE_FLOOR);
        }
    }

    /// Re-predicts bin `b` of the loudest channel from its neighbors' phase differences, then locks the other
    /// channels to it. `random` is the range the time factor is drawn from when the stretch is too large for clean
    /// phases.
    fn predict_bin(&mut self, b: usize, time_factor: f32, random: Option<(f32, f32)>, long_step: i32) {
        let (ch, bands) = (self.channels, self.bands);
        let mut max_channel = 0;
        let mut max_energy = self.predictions[b].energy;
        for c in 1..ch {
            let e = self.predictions[c * bands + b].energy;
            if e > max_energy {
                max_channel = c;
                max_energy = e;
            }
        }
        let offset = max_channel * bands;
        let predictions = &self.predictions[offset..offset + bands];
        let bins = &self.channel_bands[offset..offset + bands];
        let prediction = predictions[b];
        let map = self.output_map[b];
        let mut phase = Complex32::default();
        let ib = b as i32;

        // Upwards vertical steps.
        if b > 0 {
            let tf = match random {
                Some((lo, hi)) => self.random.uniform(lo, hi),
                None => time_factor,
            };
            let down_input = fractional_at(bins, map.input_bin - tf, |b| b.input);
            phase += bins[b - 1].output * mul_conj(prediction.input, down_input);
            if ib >= long_step {
                let long_down_input = fractional_at(bins, map.input_bin - long_step as f32 * tf, |b| b.input);
                phase += bins[b - long_step as usize].output * mul_conj(prediction.input, long_down_input);
            }
        }
        // Downwards vertical steps.
        if b < bands - 1 {
            let up_prediction = predictions[b + 1];
            let up_map = self.output_map[b + 1];
            let tf = match random {
                Some((lo, hi)) => self.random.uniform(lo, hi),
                None => time_factor,
            };
            let down_input = fractional_at(bins, up_map.input_bin - tf, |b| b.input);
            phase += mul_conj(bins[b + 1].output, mul_conj(up_prediction.input, down_input));
            if ib < bands as i32 - long_step {
                let up = b + long_step as usize;
                let long_down_input = fractional_at(bins, self.output_map[up].input_bin - long_step as f32 * tf, |b| b.input);
                phase += mul_conj(bins[up].output, mul_conj(predictions[up].input, long_down_input));
            }
        }

        let output = prediction.make_output(phase);
        self.channel_bands[offset + b].output = output;
        // Every other channel is locked in phase to this one.
        for c in (0..ch).filter(|&c| c != max_channel) {
            let channel_prediction = self.predictions[c * bands + b];
            let channel_phase = output * mul_conj(channel_prediction.input, prediction.input);
            self.channel_bands[c * bands + b].output = channel_prediction.make_output(channel_phase);
        }
    }
}
