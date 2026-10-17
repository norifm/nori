//! Ported from Signalsmith Linear, Copyright (c) 2025 Signalsmith Audio, MIT (see LICENSE).
//!
//! Signalsmith Linear's "modified" real FFT: `N` real samples to `N/2` bins at half-integer frequencies
//! (`(k + 0.5) / N`), so there is no DC or Nyquist bin to pack. Unnormalized both ways, like the original; the
//! complex half-size transform underneath is rustfft's.

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub(crate) struct ModifiedRealFft {
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    /// The half-size complex buffer the transforms run in.
    work: Vec<Complex32>,
    scratch: Vec<Complex32>,
    /// `exp(-i (2π (k + 0.5) / N + π/2))`: the half-bin twiddle with the factor `-i` baked in.
    twiddles: Vec<Complex32>,
    /// `exp(-2πi n / N)` on the paired input samples: the half-bin shift.
    half_bin_twists: Vec<Complex32>,
}

fn polar(phase: f32) -> Complex32 {
    Complex32::new(phase.cos(), phase.sin())
}

impl ModifiedRealFft {
    pub fn new(size: usize) -> Self {
        let half = size / 2;
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(half);
        let inverse = planner.plan_fft_inverse(half);
        let scratch_len = forward.get_inplace_scratch_len().max(inverse.get_inplace_scratch_len());
        let twiddles = (0..half / 2 + 1).map(|i| polar(((i as f64 + 0.5) * (-2.0 * PI / size as f64) - PI / 2.0) as f32)).collect();
        let half_bin_twists = (0..half).map(|i| polar((-2.0 * PI * i as f64 / size as f64) as f32)).collect();
        ModifiedRealFft {
            forward,
            inverse,
            work: vec![Complex32::default(); half],
            scratch: vec![Complex32::default(); scratch_len],
            twiddles,
            half_bin_twists,
        }
    }

    pub fn fft(&mut self, time: &[f32], freq: &mut [Complex32]) {
        let half = self.work.len();
        for (i, w) in self.work.iter_mut().enumerate() {
            let (tr, ti) = (time[2 * i], time[2 * i + 1]);
            let t = self.half_bin_twists[i];
            *w = Complex32::new(tr * t.re - ti * t.im, ti * t.re + tr * t.im);
        }
        self.forward.process_with_scratch(&mut self.work, &mut self.scratch);
        let w = &self.work;
        for i in 0..half / 2 + 1 {
            let conj_i = half - 1 - i;
            let tw = self.twiddles[i];
            let odd_r = (w[i].re + w[conj_i].re) * 0.5;
            let odd_i = (w[i].im - w[conj_i].im) * 0.5;
            let even_ir = (w[i].re - w[conj_i].re) * 0.5;
            let even_ii = (w[i].im + w[conj_i].im) * 0.5;
            let rot_r = even_ir * tw.re - even_ii * tw.im;
            let rot_i = even_ii * tw.re + even_ir * tw.im;
            freq[i] = Complex32::new(odd_r + rot_r, odd_i + rot_i);
            freq[conj_i] = Complex32::new(odd_r - rot_r, rot_i - odd_i);
        }
    }

    pub fn ifft(&mut self, freq: &[Complex32], time: &mut [f32]) {
        let half = self.work.len();
        for i in 0..half / 2 + 1 {
            let conj_i = half - 1 - i;
            let tw = self.twiddles[i];
            let (f, fc) = (freq[i], freq[conj_i]);
            let (odd_r, odd_i) = (f.re + fc.re, f.im - fc.im);
            let (rot_r, rot_i) = (f.re - fc.re, f.im + fc.im);
            let even_r = rot_r * tw.re + rot_i * tw.im;
            let even_i = rot_i * tw.re - rot_r * tw.im;
            self.work[i] = Complex32::new(odd_r + even_r, odd_i + even_i);
            self.work[conj_i] = Complex32::new(odd_r - even_r, even_i - odd_i);
        }
        self.inverse.process_with_scratch(&mut self.work, &mut self.scratch);
        for (i, w) in self.work.iter().enumerate() {
            let t = self.half_bin_twists[i];
            time[2 * i] = w.re * t.re + w.im * t.im;
            time[2 * i + 1] = w.im * t.re - w.re * t.im;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The direct sum the transform stands for: `X[k] = Σ x[n] exp(-2πi (k + 0.5) n / N)`.
    fn slow_dft(x: &[f32]) -> Vec<Complex32> {
        let n = x.len();
        (0..n / 2)
            .map(|k| {
                let (mut re, mut im) = (0f64, 0f64);
                for (j, v) in x.iter().enumerate() {
                    let p = -2.0 * PI * (k as f64 + 0.5) * j as f64 / n as f64;
                    re += *v as f64 * p.cos();
                    im += *v as f64 * p.sin();
                }
                Complex32::new(re as f32, im as f32)
            })
            .collect()
    }

    #[test]
    fn forward_is_the_half_bin_dft_and_inverse_undoes_it() {
        for n in [16, 40, 5120] {
            let x: Vec<f32> = (0..n).map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0).collect();
            let mut fft = ModifiedRealFft::new(n);
            let mut freq = vec![Complex32::default(); n / 2];
            fft.fft(&x, &mut freq);
            let want = slow_dft(&x);
            let scale = (n as f32).sqrt();
            for (g, w) in freq.iter().zip(&want) {
                assert!((g - w).norm() < 1e-4 * scale, "n {n}: {g} vs {w}");
            }
            let mut back = vec![0f32; n];
            fft.ifft(&freq, &mut back);
            for (b, v) in back.iter().zip(&x) {
                assert!((b / n as f32 - v).abs() < 1e-5, "n {n}: {b} vs {v}");
            }
        }
    }
}
