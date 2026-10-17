//! Ported from Signalsmith Linear, Copyright (c) 2025 Signalsmith Audio, MIT (see LICENSE).
//!
//! Signalsmith Linear's `DynamicSTFT` with the modified (half-bin) real FFT, as Signalsmith Stretch configures it:
//! a Kaiser window forced to perfect reconstruction, centered windows, and a self-normalizing output that divides
//! by the summed window products.

use rustfft::num_complex::Complex32;

use crate::fft::ModifiedRealFft;

const ALMOST_ZERO: f32 = 1e-30;

/// The input history, a ring per channel. Kept apart from [`Stft`] so a snapshot of it can be analyzed.
#[derive(Clone)]
pub(crate) struct Input {
    pos: usize,
    buffer: Vec<f32>,
}

impl Input {
    pub fn copy_from(&mut self, other: &Input) {
        self.pos = other.pos;
        self.buffer.copy_from_slice(&other.buffer);
    }
}

/// The overlap-added output, a ring per channel, and the window products it is divided by.
#[derive(Clone)]
pub(crate) struct Output {
    pos: usize,
    buffer: Vec<f32>,
    window_products: Vec<f32>,
}

impl Output {
    pub fn copy_from(&mut self, other: &Output) {
        self.pos = other.pos;
        self.buffer.copy_from_slice(&other.buffer);
        self.window_products.copy_from_slice(&other.window_products);
    }

    fn block(&self) -> usize {
        self.window_products.len()
    }

    /// The normalized output `offset` samples ahead of the read position.
    pub fn read(&self, channel: usize, offset: usize) -> f32 {
        let block = self.block();
        let i = (self.pos + offset) % block;
        self.buffer[channel * block + i] / self.window_products[i]
    }

    /// Moves the read position on, clearing what it passes.
    pub fn advance(&mut self, samples: usize) {
        let block = self.block();
        let channels = self.buffer.len() / block;
        for k in 0..samples {
            let i = (self.pos + k) % block;
            for c in 0..channels {
                self.buffer[c * block + i] = 0.0;
            }
            self.window_products[i] = ALMOST_ZERO;
        }
        self.pos = (self.pos + samples) % block;
    }
}

pub(crate) struct Stft {
    channels: usize,
    block: usize,
    fft_size: usize,
    interval: usize,
    input_len: usize,
    /// Centre of both windows (the analysis and synthesis offsets are equal here).
    offset: usize,
    window: Vec<f32>,
    pub input: Input,
    pub output: Output,
    spectrum: Vec<Complex32>,
    time: Vec<f32>,
    fft: ModifiedRealFft,
}

impl Stft {
    pub fn new(channels: usize, block: usize, interval: usize, extra_input_history: usize) -> Self {
        let fft_size = fast_size_above(block.div_ceil(2).div_ceil(2)) * 4;
        let input_len = block + extra_input_history;
        let mut window: Vec<f32> = kaiser(block as f64 / interval as f64, block);
        force_perfect_reconstruction(&mut window, interval);
        let mut stft = Stft {
            channels,
            block,
            fft_size,
            interval,
            input_len,
            offset: block / 2,
            window,
            input: Input { pos: 0, buffer: vec![0.0; input_len * channels] },
            output: Output { pos: 0, buffer: vec![0.0; block * channels], window_products: vec![0.0; block] },
            spectrum: vec![Complex32::default(); fft_size / 2 * channels],
            time: vec![0.0; fft_size],
            fft: ModifiedRealFft::new(fft_size),
        };
        stft.reset(0.1);
        stft
    }

    pub fn block_samples(&self) -> usize {
        self.block
    }
    pub fn fft_samples(&self) -> usize {
        self.fft_size
    }
    pub fn interval(&self) -> usize {
        self.interval
    }
    pub fn bands(&self) -> usize {
        self.fft_size / 2
    }
    pub fn analysis_latency(&self) -> usize {
        self.block - self.offset
    }
    pub fn synthesis_latency(&self) -> usize {
        self.offset
    }
    pub fn bin_to_freq(&self, b: f32) -> f32 {
        (b + 0.5) / self.fft_size as f32
    }
    pub fn freq_to_bin(&self, f: f32) -> f32 {
        f * self.fft_size as f32 - 0.5
    }

    pub fn reset(&mut self, product_weight: f32) {
        self.input.pos = self.block;
        self.output.pos = 0;
        self.input.buffer.fill(0.0);
        self.output.buffer.fill(0.0);
        self.spectrum.fill(Complex32::default());
        self.output.window_products.fill(0.0);
        self.add_window_product();
        let wp = &mut self.output.window_products;
        for i in (0..self.block.saturating_sub(self.interval)).rev() {
            wp[i] += wp[i + self.interval];
        }
        for v in wp.iter_mut() {
            *v = *v * product_weight + ALMOST_ZERO;
        }
        self.output.advance(self.interval);
    }

    /// Writes `length` frames of interleaved `frames` (from frame `from`) into every channel's input ring.
    pub fn write_input(&mut self, frames: &[f32], from: usize, length: usize) {
        let (ch, len) = (self.channels, self.input_len);
        for c in 0..ch {
            let ring = &mut self.input.buffer[c * len..(c + 1) * len];
            let mut p = self.input.pos;
            for i in 0..length {
                ring[p] = frames[(from + i) * ch + c];
                p += 1;
                if p == len {
                    p = 0;
                }
            }
        }
        self.input.pos = (self.input.pos + length) % len;
    }

    pub fn spectrum(&self, channel: usize) -> &[Complex32] {
        let bands = self.bands();
        &self.spectrum[channel * bands..(channel + 1) * bands]
    }
    pub fn spectrum_mut(&mut self, channel: usize) -> &mut [Complex32] {
        let bands = self.bands();
        &mut self.spectrum[channel * bands..(channel + 1) * bands]
    }

    /// Windows one channel of `input`, `samples_in_past` before its write position, into that channel's spectrum.
    pub fn analyze(&mut self, channel: usize, input: &Input, samples_in_past: usize) {
        let (len, block, fft_size, offset) = (self.input_len, self.block, self.fft_size, self.offset);
        let start = (len * 2 + input.pos - block - samples_in_past) % len;
        let ring = &input.buffer[channel * len..(channel + 1) * len];
        let mut bi = start;
        for i in 0..block {
            // The half-bin shift turns the window's rotation to the center into a sign flip of its first half.
            let (w, ti) = if i < offset { (-self.window[i], i + fft_size - offset) } else { (self.window[i], i - offset) };
            self.time[ti] = ring[bi] * w;
            bi += 1;
            if bi == len {
                bi = 0;
            }
        }
        self.time[block - offset..fft_size - offset].fill(0.0);
        let bands = self.bands();
        self.fft.fft(&self.time, &mut self.spectrum[channel * bands..(channel + 1) * bands]);
    }

    /// Inverse transforms one channel's spectrum and overlap-adds it at the output position. The first channel
    /// also adds the window product the output is normalized by.
    pub fn synthesize(&mut self, channel: usize) {
        if channel == 0 {
            self.add_window_product();
        }
        let (block, fft_size, offset) = (self.block, self.fft_size, self.offset);
        let bands = self.bands();
        self.fft.ifft(&self.spectrum[channel * bands..(channel + 1) * bands], &mut self.time);
        let out = &mut self.output.buffer[channel * block..(channel + 1) * block];
        let mut bi = self.output.pos;
        for i in 0..block {
            let (w, ti) = if i < offset { (-self.window[i], i + fft_size - offset) } else { (self.window[i], i - offset) };
            out[bi] += self.time[ti] * w;
            bi += 1;
            if bi == block {
                bi = 0;
            }
        }
    }

    /// Lets the output taper away as if no more blocks follow, instead of being scaled up by missing ones.
    pub fn finish_output(&mut self, strength: f32) {
        let block = self.block;
        let mut max = 0f32;
        for i in 0..block {
            let wp = &mut self.output.window_products[(self.output.pos + i) % block];
            max = max.max(*wp);
            *wp += (max - *wp) * strength;
        }
    }

    fn add_window_product(&mut self) {
        let (block, scale) = (self.block, self.fft_size as f32);
        let mut bi = self.output.pos;
        for i in 0..block {
            let w = self.window[i];
            self.output.window_products[bi] += w * w * scale;
            bi += 1;
            if bi == block {
                bi = 0;
            }
        }
    }
}

/// Signalsmith Linear's `SplitFFT::fastSizeAbove`: a size `2^k * {1..8 but 7}` at or above `size`.
fn fast_size_above(size: usize) -> usize {
    let mut pow2 = 1;
    while pow2 < 16 && pow2 < size {
        pow2 *= 2;
    }
    while pow2 * 8 < size {
        pow2 *= 2;
    }
    let mut multiple = size.div_ceil(pow2);
    if multiple == 7 {
        multiple += 1;
    }
    multiple * pow2
}

fn bessel0(x: f64) -> f64 {
    let (mut result, mut term, mut m) = (0.0, 1.0, 0.0);
    while term > 1e-4 {
        result += term;
        m += 1.0;
        term *= (x * x) / (4.0 * m * m);
    }
    result
}

/// A Kaiser window of `size` for `bandwidth`, with Signalsmith's heuristic for the best bandwidth.
fn kaiser(bandwidth: f64, size: usize) -> Vec<f32> {
    let bandwidth = bandwidth + 8.0 / ((bandwidth + 3.0) * (bandwidth + 3.0)) + 0.25 * (3.0 - bandwidth).max(0.0);
    let bandwidth = bandwidth.max(2.0);
    let beta = (bandwidth * bandwidth * 0.25 - 1.0).sqrt() * std::f64::consts::PI;
    let inv_b0 = 1.0 / bessel0(beta);
    let inv_size = 1.0 / size as f64;
    (0..size)
        .map(|i| {
            let r = (2 * i + 1) as f64 * inv_size - 1.0;
            (bessel0(beta * (1.0 - r * r).sqrt()) * inv_b0) as f32
        })
        .collect()
}

/// Scales the window so the squares of every `interval`-spaced set of samples sum to 1.
fn force_perfect_reconstruction(window: &mut [f32], interval: usize) {
    for i in 0..interval {
        let sum2: f64 = window[i..].iter().step_by(interval).map(|v| (v * v) as f64).sum();
        let factor = 1.0 / sum2.sqrt();
        for v in window[i..].iter_mut().step_by(interval) {
            *v = (*v as f64 * factor) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_sizes_match_signalsmith() {
        // presetCheaper at 44.1 and 48 kHz (4410 / 4800 frame blocks), and a small block.
        assert_eq!(Stft::new(1, 4410, 1764, 1765).fft_samples(), 5120);
        assert_eq!(Stft::new(1, 4800, 1920, 1921).fft_samples(), 5120);
        assert_eq!(Stft::new(1, 800, 320, 321).fft_samples(), 1024);
        assert_eq!(fast_size_above(1103), 1280);
        assert_eq!(fast_size_above(13), 16);
        assert_eq!(fast_size_above(897), 1024);
    }

    #[test]
    fn analysis_then_synthesis_reconstructs() {
        let (block, interval) = (400, 160);
        let mut stft = Stft::new(1, block, interval, interval + 1);
        let x: Vec<f32> = (0..8000).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        let mut y = Vec::new();
        let mut snapshot = Input { pos: 0, buffer: vec![0.0; stft.input.buffer.len()] };
        for chunk in x.chunks(interval) {
            stft.write_input(chunk, 0, chunk.len());
            snapshot.copy_from(&stft.input);
            stft.analyze(0, &snapshot, 0);
            stft.synthesize(0);
            for i in 0..chunk.len() {
                y.push(stft.output.read(0, i));
            }
            stft.output.advance(chunk.len());
        }
        let lat = stft.analysis_latency() + stft.synthesis_latency() - interval;
        let err = (2000..7000).map(|i| (y[i + lat] - x[i]).abs()).fold(0f32, f32::max);
        assert!(err < 1e-4, "reconstruction error {err}");
    }
}
