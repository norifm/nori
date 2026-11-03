//! Signalsmith Stretch 1.3.1, ported from C++: time-stretching with a phase vocoder that predicts
//! each bin's phase from its neighbors in time and frequency, locked across channels.
//!
//! Ported from Signalsmith Stretch, Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd., and Signalsmith
//! Linear, Copyright (c) 2025 Signalsmith Audio, both MIT (see LICENSE in this crate). Only what nori uses is here:
//! the cheaper preset with its split computation, `process`, `flush` and `reset`; no
//! transposition, seeking, formants or custom frequency maps.
//!
//! The original spreads each block's work over the interval that reads its output, a step per output sample, so a
//! small audio callback costs about the same each time. nori fills seconds of output at once, so a block's work
//! runs when it starts, except the synthesis: what `flush` reads depends on how far it got, so it runs when the
//! original's would.
//!
//! Buffers are interleaved. Nothing allocates after construction.

mod fft;
mod stft;

use rustfft::num_complex::Complex32;

use stft::Stft;

const NOISE_FLOOR: f32 = 1e-15;
/// The time-stretch factor above which phases are randomized.
const MAX_CLEAN_STRETCH: f32 = 2.0;
/// The original spreads the main phase prediction over this many steps; it counts in when the synthesis is due.
const SPLIT_MAIN_PREDICTION: usize = 8;

/// The current block: its output frames read so far, and how far its synthesis got.
#[derive(Clone, Copy)]
struct Block {
    /// Output frames read since the block started; the next block starts at one interval.
    read: usize,
    /// The original's step count for this block, the synthesis being its last step per channel.
    steps: usize,
    /// Channels synthesized so far.
    synthesized: usize,
}

impl Default for Block {
    fn default() -> Self {
        Block { read: usize::MAX, steps: 0, synthesized: 0 }
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
#[inline]
fn mul_conj(a: Complex32, b: Complex32) -> Complex32 {
    Complex32::new(b.re * a.re + b.im * a.im, b.re * a.im - b.im * a.re)
}

/// A bin's output: `phase` scaled to the bin's `energy` (its input's), or its `input` when the phase is too weak to
/// follow.
#[inline]
fn make_output(energy: f32, input: Complex32, phase: Complex32) -> Complex32 {
    let phase_norm = phase.norm_sqr();
    let weak = phase_norm <= NOISE_FLOOR;
    let (direction, norm) = if weak { (input, energy + NOISE_FLOOR) } else { (phase, phase_norm) };
    direction * (energy / norm).sqrt()
}

/// `spectrum` at fractional bin `index`, linearly interpolated, zero outside it.
#[inline]
fn fractional_at(spectrum: &[Complex32], index: f32) -> Complex32 {
    let low = index.floor() as i32;
    let frac = index - low as f32;
    let at = |i: i32| if i < 0 || i as usize >= spectrum.len() { Complex32::default() } else { spectrum[i as usize] };
    let (lo, hi) = (at(low), at(low + 1));
    lo + (hi - lo) * frac
}

/// `to[b]` = [`fractional_at`]`(from, b - shift)`, the same bit for bit: inside `from` its floor is `b + k` for a
/// whole `k`, so the reads are slices and the loop vectorizes.
fn shifted(to: &mut [Complex32], from: &[Complex32], shift: f32) {
    let k = (-shift).floor() as isize;
    let n = from.len() as isize;
    // The bins whose two neighbors, `b + k` and the one after, are both inside `from`.
    let mut end = (n - 1 - k).clamp(0, n) as usize;
    let mut start = ((-k).clamp(0, n) as usize).min(end);
    let mut floors_hold = true;
    if start < end {
        let first = (start as isize + k) as usize;
        let (lo, hi) = (&from[first..first + end - start], &from[first + 1..first + 1 + end - start]);
        let (start32, k32) = (start as i32, k as i32);
        for (b, ((t, lo), hi)) in (start32..).zip(to[start..end].iter_mut().zip(lo).zip(hi)) {
            let frac = (b as f32 - shift) - (b + k32) as f32;
            floors_hold &= (0.0..1.0).contains(&frac);
            *t = *lo + (*hi - *lo) * frac;
        }
    }
    if !floors_hold {
        // A bin's position rounded across a whole bin: take them one by one.
        (start, end) = (0, 0);
    }
    for b in (0..start).chain(end..to.len()) {
        to[b] = fractional_at(from, b as f32 - shift);
    }
}

pub struct Stretch {
    channels: usize,
    bands: usize,
    stft: Stft,
    /// The phase vocoder's state, a run of `bands` per channel.
    input: Vec<Complex32>,
    prev_input: Vec<Complex32>,
    output: Vec<Complex32>,
    input_energy: Vec<f32>,
    prediction_energy: Vec<f32>,
    prediction_input: Vec<Complex32>,
    /// Each bin's phase turn over one interval, as the original's running product makes it.
    rotation: Vec<Complex32>,
    /// Each bin's input read `time_factor` and `long_step * time_factor` bins down, times the conjugate of its
    /// prediction: the vertical steps of the phase prediction.
    up_step: Vec<Complex32>,
    up_long_step: Vec<Complex32>,
    /// Per bin, the channel the phase prediction follows, the step up from the bin below's loudest output, and the
    /// rest of the steps.
    loudest: Vec<u32>,
    step: Vec<Complex32>,
    down: Vec<Complex32>,
    /// What each channel's output is from the loudest channel's: `lock * output + lock_weak`.
    lock: Vec<Complex32>,
    lock_weak: Vec<Complex32>,
    /// The output frames of the current block's interval, interleaved.
    ready: Vec<f32>,
    block: Block,
    silence_counter: usize,
    silence_first: bool,
    prev_input_offset: i32,
    random: MinStd,
}

impl Stretch {
    /// Signalsmith's `presetCheaper`: 100 ms blocks every 40 ms.
    pub fn preset_cheaper(channels: usize, sample_rate: f32) -> Self {
        let block = (sample_rate as f64 * 0.1) as usize;
        let interval = (sample_rate as f64 * 0.04) as usize;
        let stft = Stft::new(channels, block, interval, interval + 1);
        let bands = stft.bands();
        let tau = std::f32::consts::TAU;
        let mut rot = Complex32::from_polar(1.0, stft.bin_to_freq(0.0) * interval as f32 * tau);
        let freq_step = stft.bin_to_freq(1.0) - stft.bin_to_freq(0.0);
        let rot_step = Complex32::from_polar(1.0, freq_step * interval as f32 * tau);
        let rotation = (0..bands)
            .map(|_| {
                let r = rot;
                rot *= rot_step;
                r
            })
            .collect();
        let zeros = vec![Complex32::default(); bands * channels];
        Stretch {
            channels,
            bands,
            stft,
            input: zeros.clone(),
            prev_input: zeros.clone(),
            output: zeros.clone(),
            input_energy: vec![0.0; bands * channels],
            prediction_energy: vec![0.0; bands * channels],
            prediction_input: zeros.clone(),
            rotation,
            up_step: zeros.clone(),
            up_long_step: zeros.clone(),
            loudest: vec![0; bands],
            step: vec![Complex32::default(); bands],
            down: vec![Complex32::default(); bands],
            lock: zeros.clone(),
            lock_weak: zeros,
            ready: vec![0.0; interval * channels],
            block: Block::default(),
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
        self.prev_input_offset = -1;
        self.clear_bands();
        self.silence_counter = 0;
        self.block = Block::default();
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
                    self.block = Block::default();
                    self.clear_bands();
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

        let mut o = 0;
        while o < out_frames {
            if self.block.read >= interval {
                self.start_block(input, &mut copied, o, in_frames, out_frames);
            }
            let read = self.block.read;
            let n = (interval - read).min(out_frames - o);
            output[o * ch..(o + n) * ch].copy_from_slice(&self.ready[read * ch..(read + n) * ch]);
            self.block.read += n;
            o += n;
            self.synthesize_due();
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
        self.prev_input.fill(Complex32::default());
        self.output.fill(Complex32::default());
    }

    fn clear_bands(&mut self) {
        for v in [&mut self.input, &mut self.prev_input, &mut self.output] {
            v.fill(Complex32::default());
        }
        self.input_energy.fill(0.0);
    }

    /// Copies the input from where the last copy ended up to frame `to` (at most a block and an interval of it).
    fn copy_input(&mut self, input: &[f32], copied: &mut usize, to: usize) {
        let length = (self.stft.block_samples() + self.stft.interval()).min(to - *copied);
        self.stft.write_input(input, to - length, length);
        *copied = to;
    }

    /// Starts a block at output frame `out_index`: its output to read, then all of its work but the synthesis.
    fn start_block(&mut self, input: &[f32], copied: &mut usize, out_index: usize, in_frames: usize, out_frames: usize) {
        let (ch, bands, interval) = (self.channels, self.bands, self.stft.interval());
        // Where in the input this spectrum comes from.
        let input_offset = (out_index as f32 * in_frames as f32 / out_frames as f32).round() as i32;
        let input_interval = input_offset - self.prev_input_offset;
        self.prev_input_offset = input_offset;
        self.copy_input(input, copied, input_offset as usize);
        // The synthesis runs one interval ahead of what is read.
        self.stft.output.take(&mut self.ready);

        let new_spectrum = input_interval > 0;
        // The previous analysis must be one interval back, give or take a frame.
        let reanalyze_prev = new_spectrum && (input_interval - interval as i32).abs() > 1;
        let mut steps = 0;
        if new_spectrum {
            if reanalyze_prev {
                steps += ch + 1;
            }
            // The analysis, then the rotation and the previous input kept.
            steps += 2 * (ch + 1);
        }
        steps += 1 + ch + SPLIT_MAIN_PREDICTION + 1 + ch;
        self.block = Block { read: 0, steps, synthesized: 0 };

        if new_spectrum {
            for c in 0..ch {
                if reanalyze_prev {
                    self.stft.analyze(c, interval, &mut self.prev_input[c * bands..(c + 1) * bands]);
                }
                self.stft.analyze(c, 0, &mut self.input[c * bands..(c + 1) * bands]);
            }
        }
        let time_factor = interval as f32 / (input_interval as f32).max(1.0);
        self.process_spectrum(new_spectrum, time_factor);
    }

    /// Synthesizes the channels the original's split computation would have reached by the frames read so far.
    fn synthesize_due(&mut self) {
        let (ch, interval, b) = (self.channels, self.stft.interval(), self.block);
        if b.synthesized == ch {
            return;
        }
        let ratio = b.read as f32 / interval as f32;
        let due = b.steps.min(((b.steps as f32 + 0.999) * ratio) as usize);
        let bands = self.bands;
        while self.block.synthesized < ch && b.steps - ch + self.block.synthesized < due {
            let c = self.block.synthesized;
            // The output stays as the block left it until the next block starts, by when its synthesis is done.
            self.stft.synthesize(c, &self.output[c * bands..(c + 1) * bands]);
            self.block.synthesized += 1;
        }
    }

    fn process_spectrum(&mut self, new_spectrum: bool, time_factor: f32) {
        let (ch, bands) = (self.channels, self.bands);
        let smoothing_bins = self.stft.fft_samples() as f32 / self.stft.interval() as f32;
        let long_step = smoothing_bins.round() as usize;
        let time_factor = time_factor.max(1.0 / MAX_CLEAN_STRETCH);

        if new_spectrum {
            // Move the previous input and output on by one interval at each bin's frequency.
            for c in 0..ch {
                let run = c * bands..(c + 1) * bands;
                for ((out, prev), rot) in self.output[run.clone()].iter_mut().zip(&mut self.prev_input[run]).zip(&self.rotation) {
                    *out *= *rot;
                    *prev *= *rot;
                }
            }
        }
        for (energy, input) in self.input_energy.iter_mut().zip(&self.input) {
            *energy = input.norm_sqr();
        }
        self.predict_from_previous();
        if time_factor > MAX_CLEAN_STRETCH {
            let random_low = MAX_CLEAN_STRETCH * 2.0 - time_factor;
            for b in 0..bands {
                self.predict_bin_random(b, random_low, time_factor, long_step);
            }
        } else {
            self.predict_bins(time_factor, long_step);
        }
        if new_spectrum {
            self.prev_input.copy_from_slice(&self.input);
        }
    }

    /// The preliminary phase-vocoder prediction: each output bin moved on by its input's phase change since the
    /// previous block. Output bin `b` reads input bin `b` (no transposition or formants).
    fn predict_from_previous(&mut self) {
        let bins = self.output.iter_mut().zip(&self.input).zip(&self.prev_input).zip(&self.input_energy);
        let predictions = self.prediction_energy.iter_mut().zip(&mut self.prediction_input);
        for ((((output, input), prev_input), input_energy), (prediction_energy, prediction_input)) in bins.zip(predictions) {
            let prev_energy = *prediction_energy;
            *prediction_energy = *input_energy;
            *prediction_input = *input;
            let phase = *output * mul_conj(*input, *prev_input);
            *output = phase / (prev_energy.max(*prediction_energy) + NOISE_FLOOR);
        }
    }

    /// Re-predicts each bin of its loudest channel from its neighbors' phase differences, lower bins first, then locks
    /// the other channels to it. Each bin waits on the one below it, so everything else is made beforehand: the
    /// loudest channel, the steps down from the bins above (which read their preliminary prediction), what turns the
    /// loudest channel's output into each channel's, and so the step up from the bin below.
    fn predict_bins(&mut self, time_factor: f32, long_step: usize) {
        let (ch, bands) = (self.channels, self.bands);
        let long_factor = long_step as f32 * time_factor;
        for c in 0..ch {
            let run = c * bands..(c + 1) * bands;
            shifted(&mut self.up_step[run.clone()], &self.input[run.clone()], time_factor);
            shifted(&mut self.up_long_step[run.clone()], &self.input[run.clone()], long_factor);
            let steps = self.up_step[run.clone()].iter_mut().zip(&mut self.up_long_step[run.clone()]);
            for ((step, long), prediction) in steps.zip(&self.prediction_input[run]) {
                *step = mul_conj(*prediction, *step);
                *long = mul_conj(*prediction, *long);
            }
        }
        for b in 0..bands {
            let mut loudest = 0;
            let mut max_energy = self.prediction_energy[b];
            for c in 1..ch {
                let e = self.prediction_energy[c * bands + b];
                let louder = e > max_energy;
                loudest = if louder { c } else { loudest };
                max_energy = if louder { e } else { max_energy };
            }
            self.loudest[b] = loudest as u32;
            let at = loudest * bands + b;
            let mut down = Complex32::default();
            if b + long_step < bands {
                down = mul_conj(self.output[at + long_step], self.up_long_step[at + long_step]);
            }
            if b + 1 < bands {
                down += mul_conj(self.output[at + 1], self.up_step[at + 1]);
            }
            // The step up from the bin below reads this channel's output there: the loudest one's there, locked.
            if b > 0 {
                let up = self.up_step[at];
                self.step[b] = self.lock[at - 1] * up;
                down += self.lock_weak[at - 1] * up;
            }
            self.down[b] = down;
            // The loudest output turned and scaled by this channel's input against the loudest one's (`input /
            // prediction`), so its energy is this channel's; this channel's own input where that is too weak to follow.
            let prediction = self.prediction_input[at];
            let inverse = 1.0 / max_energy;
            for c in 0..ch {
                let i = c * bands + b;
                let input = self.prediction_input[i];
                let lock = mul_conj(input, prediction);
                if max_energy * lock.norm_sqr() > NOISE_FLOOR {
                    (self.lock[i], self.lock_weak[i]) = (lock * inverse, Complex32::default());
                } else {
                    let energy = self.prediction_energy[i];
                    (self.lock[i], self.lock_weak[i]) = (Complex32::default(), input * (energy / (energy + NOISE_FLOOR)).sqrt());
                }
            }
            self.lock[at] = Complex32::new(1.0, 0.0);
            self.lock_weak[at] = Complex32::default();
        }
        // The loudest output of the bin below is `direction * scale`; turning the direction while the scale is worked
        // out keeps the multiply off the wait.
        let (mut direction, mut scale) = (Complex32::default(), 0f32);
        let output = &mut self.output;
        for b in 0..bands {
            let at = self.loudest[b] as usize * bands + b;
            let mut phase = self.down[b];
            if b >= long_step {
                phase += output[at - long_step] * self.up_long_step[at];
            }
            if b > 0 {
                phase += (direction * self.step[b]) * scale;
            }
            let energy = self.prediction_energy[at];
            let norm = phase.norm_sqr();
            let weak = norm <= NOISE_FLOOR;
            (direction, scale) = if weak {
                (self.prediction_input[at], (energy / (energy + NOISE_FLOOR)).sqrt())
            } else {
                (phase, (energy / norm).sqrt())
            };
            let out = direction * scale;
            for c in 0..ch {
                let i = c * bands + b;
                output[i] = out * self.lock[i] + self.lock_weak[i];
            }
        }
    }

    /// [`predict_bins`](Self::predict_bins) for one bin when the stretch is too large for clean phases: each vertical
    /// step reads its input at a time factor drawn from `random_low..time_factor`.
    fn predict_bin_random(&mut self, b: usize, random_low: f32, time_factor: f32, long_step: usize) {
        let (ch, bands) = (self.channels, self.bands);
        let mut loudest = 0;
        let mut max_energy = self.prediction_energy[b];
        for c in 1..ch {
            let e = self.prediction_energy[c * bands + b];
            if e > max_energy {
                loudest = c;
                max_energy = e;
            }
        }
        let offset = loudest * bands;
        let input = &self.input[offset..offset + bands];
        let predictions = &self.prediction_input[offset..offset + bands];
        let output = &self.output[offset..offset + bands];
        let prediction = predictions[b];
        let mut phase = Complex32::default();
        if b > 0 {
            let tf = self.random.uniform(random_low, time_factor);
            phase += output[b - 1] * mul_conj(prediction, fractional_at(input, b as f32 - tf));
            if b >= long_step {
                phase += output[b - long_step] * mul_conj(prediction, fractional_at(input, b as f32 - long_step as f32 * tf));
            }
        }
        if b < bands - 1 {
            let tf = self.random.uniform(random_low, time_factor);
            phase += mul_conj(output[b + 1], mul_conj(predictions[b + 1], fractional_at(input, (b + 1) as f32 - tf)));
            if b + long_step < bands {
                let up = b + long_step;
                phase += mul_conj(output[up], mul_conj(predictions[up], fractional_at(input, up as f32 - long_step as f32 * tf)));
            }
        }
        let out = make_output(max_energy, prediction, phase);
        self.output[offset + b] = out;
        for c in (0..ch).filter(|&c| c != loudest) {
            let i = c * bands + b;
            let channel_prediction = self.prediction_input[i];
            self.output[i] = make_output(self.prediction_energy[i], channel_prediction, out * mul_conj(channel_prediction, prediction));
        }
    }
}

