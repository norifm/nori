//! Ported from Signalsmith Linear, Copyright (c) 2025 Signalsmith Audio, MIT (see LICENSE).
//!
//! Signalsmith Linear's "modified" real FFT: `N` real samples to `N/2` bins at half-integer frequencies
//! (`(k + 0.5) / N`), so there is no DC or Nyquist bin to pack. Unnormalized both ways, like the original; the
//! complex half-size transform underneath is rustfft's, in five parts when the size is five times a power of
//! two ([`Fives`]).

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub(crate) struct ModifiedRealFft {
    /// The half-size transforms, or with [`Fives`] the transforms of a fifth of it.
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    fives: Option<Fives>,
    /// The half-size transform's input in order, and the buffer the transforms run in (with [`Fives`],
    /// every fifth value of the input in a run of its own).
    line: Vec<Complex32>,
    work: Vec<Complex32>,
    scratch: Vec<Complex32>,
    /// The half-size transform's result in order, real and imaginary parts apart.
    re: Vec<f32>,
    im: Vec<f32>,
    /// `exp(-i (2π (k + 0.5) / N + π/2))`: the half-bin twiddle with the factor `-i` baked in.
    twiddles: Vec<Complex32>,
    /// `exp(-2πi n / N)` on the paired input samples: the half-bin shift.
    half_bin_twists: Vec<Complex32>,
}

/// A half size of five times a power of two (44.1 and 48 kHz's 2560): five transforms of every fifth value
/// and a radix-5 pass. rustfft's own plan for such a size spends about a fifth of its time transposing.
struct Fives {
    fifth: usize,
    /// `exp(-2πi r k / half)` for `r` in 1..=4 and `k` below a fifth, real and imaginary parts.
    re: [Vec<f32>; 4],
    im: [Vec<f32>; 4],
}

fn polar(phase: f32) -> Complex32 {
    Complex32::new(phase.cos(), phase.sin())
}

impl ModifiedRealFft {
    pub fn new(size: usize) -> Self {
        let half = size / 2;
        let fifth = half / 5;
        let fives = (half.is_multiple_of(5) && fifth.is_power_of_two() && fifth >= 16).then(|| {
            let turn = |r: usize, k: usize| -2.0 * PI * (r * k) as f64 / half as f64;
            Fives {
                fifth,
                re: std::array::from_fn(|r| (0..fifth).map(|k| turn(r + 1, k).cos() as f32).collect()),
                im: std::array::from_fn(|r| (0..fifth).map(|k| turn(r + 1, k).sin() as f32).collect()),
            }
        });
        let length = fives.as_ref().map_or(half, |f| f.fifth);
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(length);
        let inverse = planner.plan_fft_inverse(length);
        let scratch_len = forward.get_inplace_scratch_len().max(inverse.get_inplace_scratch_len());
        let twiddles = (0..half / 2 + 1).map(|i| polar(((i as f64 + 0.5) * (-2.0 * PI / size as f64) - PI / 2.0) as f32)).collect();
        let half_bin_twists = (0..half).map(|i| polar((-2.0 * PI * i as f64 / size as f64) as f32)).collect();
        ModifiedRealFft {
            forward,
            inverse,
            fives,
            line: vec![Complex32::default(); if length < half { half } else { 0 }],
            work: vec![Complex32::default(); half],
            scratch: vec![Complex32::default(); scratch_len],
            re: vec![0.0; half],
            im: vec![0.0; half],
            twiddles,
            half_bin_twists,
        }
    }

    /// Where the input to the half-size transform is written in order.
    fn input(&mut self) -> &mut [Complex32] {
        if self.fives.is_some() {
            &mut self.line
        } else {
            &mut self.work
        }
    }

    /// Runs the half-size transform over the input and leaves its result in `re` and `im`.
    fn transform(&mut self, inverse: bool) {
        if let Some(f) = &self.fives {
            for (r, run) in self.work.chunks_exact_mut(f.fifth).enumerate() {
                for (w, v) in run.iter_mut().zip(self.line[r..].iter().step_by(5)) {
                    *w = *v;
                }
            }
        }
        let fft = if inverse { &self.inverse } else { &self.forward };
        fft.process_with_scratch(&mut self.work, &mut self.scratch);
        match &self.fives {
            Some(f) => join_fives(f, &self.work, &mut self.re, &mut self.im, inverse),
            None => {
                for ((w, re), im) in self.work.iter().zip(&mut self.re).zip(&mut self.im) {
                    (*re, *im) = (w.re, w.im);
                }
            }
        }
    }

    pub fn fft(&mut self, time: &[f32], freq: &mut [Complex32]) {
        let half = self.work.len();
        let twists = std::mem::take(&mut self.half_bin_twists);
        for ((w, t), &[tr, ti]) in self.input().iter_mut().zip(&twists).zip(time.as_chunks::<2>().0) {
            *w = Complex32::new(tr * t.re - ti * t.im, ti * t.re + tr * t.im);
        }
        self.half_bin_twists = twists;
        self.transform(false);
        let (re, im) = (&self.re, &self.im);
        for i in 0..half / 2 + 1 {
            let conj_i = half - 1 - i;
            let tw = self.twiddles[i];
            let odd_r = (re[i] + re[conj_i]) * 0.5;
            let odd_i = (im[i] - im[conj_i]) * 0.5;
            let even_ir = (re[i] - re[conj_i]) * 0.5;
            let even_ii = (im[i] + im[conj_i]) * 0.5;
            let rot_r = even_ir * tw.re - even_ii * tw.im;
            let rot_i = even_ii * tw.re + even_ir * tw.im;
            freq[i] = Complex32::new(odd_r + rot_r, odd_i + rot_i);
            freq[conj_i] = Complex32::new(odd_r - rot_r, rot_i - odd_i);
        }
    }

    pub fn ifft(&mut self, freq: &[Complex32], time: &mut [f32]) {
        let half = self.work.len();
        let twiddles = std::mem::take(&mut self.twiddles);
        let input = self.input();
        for (i, tw) in twiddles.iter().enumerate() {
            let conj_i = half - 1 - i;
            let (f, fc) = (freq[i], freq[conj_i]);
            let (odd_r, odd_i) = (f.re + fc.re, f.im - fc.im);
            let (rot_r, rot_i) = (f.re - fc.re, f.im + fc.im);
            let even_r = rot_r * tw.re + rot_i * tw.im;
            let even_i = rot_i * tw.re - rot_r * tw.im;
            input[i] = Complex32::new(odd_r + even_r, odd_i + even_i);
            input[conj_i] = Complex32::new(odd_r - even_r, even_i - odd_i);
        }
        self.twiddles = twiddles;
        self.transform(true);
        for (i, (re, im)) in self.re.iter().zip(&self.im).enumerate() {
            let t = self.half_bin_twists[i];
            time[2 * i] = re * t.re + im * t.im;
            time[2 * i + 1] = im * t.re - re * t.im;
        }
    }
}

/// The radix-5 pass: `runs` holds the transforms of every fifth value (run `r` of values `5m + r`), and value
/// `k + q * fifth` of the whole is `sum over r of exp(∓2πi r (k + q * fifth) / half) * runs[r][k]`.
fn join_fives(f: &Fives, runs: &[Complex32], out_re: &mut [f32], out_im: &mut [f32], inverse: bool) {
    let n = f.fifth;
    // The fifth roots of unity: cos and sin of 2π/5 and 4π/5, the sine turning with the direction.
    let sign = if inverse { 1.0 } else { -1.0 };
    let (c1, c2) = ((2.0 * PI / 5.0).cos() as f32, (4.0 * PI / 5.0).cos() as f32);
    let (s1, s2) = ((sign * (2.0 * PI / 5.0).sin()) as f32, (sign * (4.0 * PI / 5.0).sin()) as f32);
    let run = |r: usize| &runs[r * n..(r + 1) * n];
    let (y0, y1, y2, y3, y4) = (run(0), run(1), run(2), run(3), run(4));
    let (twr, twi) = (&f.re, &f.im);
    let (w1r, w2r, w3r, w4r) = (&twr[0][..n], &twr[1][..n], &twr[2][..n], &twr[3][..n]);
    let (w1i, w2i, w3i, w4i) = (&twi[0][..n], &twi[1][..n], &twi[2][..n], &twi[3][..n]);
    let (o0r, rest) = out_re.split_at_mut(n);
    let (o1r, rest) = rest.split_at_mut(n);
    let (o2r, rest) = rest.split_at_mut(n);
    let (o3r, o4r) = rest.split_at_mut(n);
    let (o0i, rest) = out_im.split_at_mut(n);
    let (o1i, rest) = rest.split_at_mut(n);
    let (o2i, rest) = rest.split_at_mut(n);
    let (o3i, o4i) = rest.split_at_mut(n);
    let (o4r, o4i) = (&mut o4r[..n], &mut o4i[..n]);
    // The twiddles are the forward transform's; the inverse turns the other way.
    let flip = -sign as f32;
    for k in 0..n {
        let turned = |y: Complex32, wr: f32, wi: f32| {
            let wi = flip * wi;
            (y.re * wr - y.im * wi, y.re * wi + y.im * wr)
        };
        let (a0r, a0i) = (y0[k].re, y0[k].im);
        let (a1r, a1i) = turned(y1[k], w1r[k], w1i[k]);
        let (a2r, a2i) = turned(y2[k], w2r[k], w2i[k]);
        let (a3r, a3i) = turned(y3[k], w3r[k], w3i[k]);
        let (a4r, a4i) = turned(y4[k], w4r[k], w4i[k]);
        let (b1r, b1i, b4r, b4i) = (a1r + a4r, a1i + a4i, a1r - a4r, a1i - a4i);
        let (b2r, b2i, b3r, b3i) = (a2r + a3r, a2i + a3i, a2r - a3r, a2i - a3i);
        o0r[k] = a0r + b1r + b2r;
        o0i[k] = a0i + b1i + b2i;
        let (t1r, t1i) = (a0r + b1r * c1 + b2r * c2, a0i + b1i * c1 + b2i * c2);
        let (t2r, t2i) = (a0r + b1r * c2 + b2r * c1, a0i + b1i * c2 + b2i * c1);
        let (u1r, u1i) = (b4r * s1 + b3r * s2, b4i * s1 + b3i * s2);
        let (u2r, u2i) = (b4r * s2 - b3r * s1, b4i * s2 - b3i * s1);
        (o1r[k], o1i[k]) = (t1r - u1i, t1i + u1r);
        (o4r[k], o4i[k]) = (t1r + u1i, t1i - u1r);
        (o2r[k], o2i[k]) = (t2r - u2i, t2i + u2r);
        (o3r[k], o3i[k]) = (t2r + u2i, t2i - u2r);
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
        // Planned whole (16, 40, 4096) and in fifths (5120: 44.1 and 48 kHz; 1280).
        for n in [16, 40, 4096, 1280, 5120] {
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
