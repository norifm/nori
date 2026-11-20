//! Ported from Signalsmith Linear, Copyright (c) 2025 Signalsmith Audio, MIT (see LICENSE).
//!
//! Signalsmith Linear's `DynamicSTFT` with the modified (half-bin) real FFT, as Signalsmith Stretch configures it:
//! a Kaiser window forced to perfect reconstruction, centered windows, and a self-normalizing output that divides
//! by the summed window products.

use rustfft::num_complex::Complex32;

use crate::fft::ModifiedRealFft;

const ALMOST_ZERO: f32 = 1e-30;

/// The input history, a ring per channel.
pub(crate) struct Input {
    pos: usize,
    buffer: Vec<f32>,
}

/// The overlap-added output, a ring per channel, and the window products it is divided by.
pub(crate) struct Output {
    pos: usize,
    buffer: Vec<f32>,
    window_products: Vec<f32>,
}

impl Output {
    fn block(&self) -> usize {
        self.window_products.len()
    }

    /// The normalized output `offset` samples ahead of the read position.
    pub fn read(&self, channel: usize, offset: usize) -> f32 {
        let block = self.block();
        let i = (self.pos + offset) % block;
        self.buffer[channel * block + i] / self.window_products[i]
    }

    /// Writes the normalized frames from the read position into `frames` (interleaved), clears them and moves on.
    pub fn take(&mut self, frames: &mut [f32]) {
        let block = self.block();
        let channels = self.buffer.len() / block;
        let n = frames.len() / channels;
        let first = (block - self.pos).min(n);
        for (from, to, at) in [(self.pos, self.pos + first, 0), (0, n - first, first)] {
            for c in 0..channels {
                let ring = &mut self.buffer[c * block + from..c * block + to];
                for (k, (sample, product)) in ring.iter_mut().zip(&self.window_products[from..to]).enumerate() {
                    frames[(at + k) * channels + c] = *sample / product;
                    *sample = 0.0;
                }
            }
            self.window_products[from..to].fill(ALMOST_ZERO);
        }
        self.pos = (self.pos + n) % block;
    }

    /// Moves the read position on, clearing what it passes.
    pub fn advance(&mut self, samples: usize) {
        let block = self.block();
        let channels = self.buffer.len() / block;
        let first = (block - self.pos).min(samples);
        for (from, to) in [(self.pos, self.pos + first), (0, samples - first)] {
            for c in 0..channels {
                self.buffer[c * block + from..c * block + to].fill(0.0);
            }
            self.window_products[from..to].fill(ALMOST_ZERO);
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
    /// The window with its first `offset` samples negated: the half-bin shift turns the rotation of the window to
    /// its center into a sign flip of its first half.
    signed_window: Vec<f32>,
    pub input: Input,
    pub output: Output,
    time: Vec<f32>,
    fft: ModifiedRealFft,
}

impl Stft {
    pub fn new(channels: usize, block: usize, interval: usize, extra_input_history: usize) -> Self {
        let fft_size = fast_size_above(block.div_ceil(2).div_ceil(2)) * 4;
        let input_len = block + extra_input_history;
        let mut window: Vec<f32> = kaiser(block as f64 / interval as f64, block);
        force_perfect_reconstruction(&mut window, interval);
        let offset = block / 2;
        let signed_window = window.iter().enumerate().map(|(i, w)| if i < offset { -w } else { *w }).collect();
        let mut stft = Stft {
            channels,
            block,
            fft_size,
            interval,
            input_len,
            offset,
            window,
            signed_window,
            input: Input { pos: 0, buffer: vec![0.0; input_len * channels] },
            output: Output { pos: 0, buffer: vec![0.0; block * channels], window_products: vec![0.0; block] },
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
        let (ch, len, pos) = (self.channels, self.input_len, self.input.pos);
        let first = (len - pos).min(length);
        for c in 0..ch {
            let (head, tail) = self.input.buffer[c * len..(c + 1) * len].split_at_mut(pos);
            for (to, at) in [(&mut tail[..first], from), (&mut head[..length - first], from + first)] {
                for (k, v) in to.iter_mut().enumerate() {
                    *v = frames[(at + k) * ch + c];
                }
            }
        }
        self.input.pos = (pos + length) % len;
    }

    /// Windows one channel of the input, `samples_in_past` before its write position, and transforms it into
    /// `spectrum`.
    pub fn analyze(&mut self, channel: usize, samples_in_past: usize, spectrum: &mut [Complex32]) {
        let (len, block, fft_size, offset) = (self.input_len, self.block, self.fft_size, self.offset);
        let start = (len * 2 + self.input.pos - block - samples_in_past) % len;
        let ring = &self.input.buffer[channel * len..(channel + 1) * len];
        let window = &self.signed_window;
        // The window's first half lands at the end of the transform's input, the rest at its start.
        windowed(&mut self.time[fft_size - offset..], ring, start, &window[..offset]);
        windowed(&mut self.time[..block - offset], ring, (start + offset) % len, &window[offset..]);
        self.time[block - offset..fft_size - offset].fill(0.0);
        self.fft.fft(&self.time, spectrum);
    }

    /// Inverse transforms one channel's `spectrum` and overlap-adds it at the output position. The first channel
    /// also adds the window product the output is normalized by.
    pub fn synthesize(&mut self, channel: usize, spectrum: &[Complex32]) {
        if channel == 0 {
            self.add_window_product();
        }
        let (block, fft_size, offset) = (self.block, self.fft_size, self.offset);
        self.fft.ifft(spectrum, &mut self.time);
        let out = &mut self.output.buffer[channel * block..(channel + 1) * block];
        let pos = self.output.pos;
        let window = &self.signed_window;
        overlap_add(out, pos, &self.time[fft_size - offset..], &window[..offset]);
        overlap_add(out, (pos + offset) % block, &self.time[..block - offset], &window[offset..]);
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
        let (block, scale, pos) = (self.block, self.fft_size as f32, self.output.pos);
        let (head, tail) = self.output.window_products.split_at_mut(pos);
        let first = block - pos;
        for (to, window) in [(tail, &self.window[..first]), (head, &self.window[first..])] {
            for (p, w) in to.iter_mut().zip(window) {
                *p += w * w * scale;
            }
        }
    }
}

/// `to[i] = ring[start + i] * window[i]`, the ring read on from `start` around its end.
fn windowed(to: &mut [f32], ring: &[f32], start: usize, window: &[f32]) {
    let first = (ring.len() - start).min(to.len());
    let (to_a, to_b) = to.split_at_mut(first);
    let (w_a, w_b) = window.split_at(first);
    for ((t, r), w) in to_a.iter_mut().zip(&ring[start..start + first]).zip(w_a) {
        *t = r * w;
    }
    for ((t, r), w) in to_b.iter_mut().zip(ring).zip(w_b) {
        *t = r * w;
    }
}

/// `ring[start + i] += from[i] * window[i]`, around the ring's end.
fn overlap_add(ring: &mut [f32], start: usize, from: &[f32], window: &[f32]) {
    let first = (ring.len() - start).min(from.len());
    let (ring_a, ring_b) = ring.split_at_mut(start);
    for ((o, f), w) in ring_b[..first].iter_mut().zip(&from[..first]).zip(&window[..first]) {
        *o += f * w;
    }
    for ((o, f), w) in ring_a.iter_mut().zip(&from[first..]).zip(&window[first..]) {
        *o += f * w;
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
        for chunk in x.chunks(interval) {
            stft.write_input(chunk, 0, chunk.len());
            let mut spectrum = vec![Complex32::default(); stft.bands()];
            stft.analyze(0, 0, &mut spectrum);
            stft.synthesize(0, &spectrum);
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
