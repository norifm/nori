//! The port against the C++ original (the `signalsmith-stretch` crate) on the same input, driven the way AutoMix
//! drives it: 256-frame input blocks, each output block `256 / ratio` frames with the fraction carried.
//!
//! The two round differently: the port's FFT is rustfft, not Signalsmith's own, and the C++ compiler fuses
//! multiply-adds. The phase vocoder feeds each block's phases into the next and picks loudest channels by
//! comparisons, so a rounding can grow: the original against itself on input one ulp away departs by 1e-3 of the
//! peak within about 10 000 frames. Excerpts are checked sample for sample from a reset, long runs against the
//! original's own spread under that nudge. (With the same exact FFT on both sides and no fused multiply-adds the
//! outputs were bit-identical.)

use nori_stretch::Stretch;
use rustfft::num_complex::Complex32;

const RATE: u32 = 44_100;
const BLOCK: usize = 256;

trait Engine {
    fn process(&mut self, input: &[f32], output: &mut [f32]);
    fn flush(&mut self, output: &mut [f32]);
    fn reset(&mut self);
}

impl Engine for Stretch {
    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        Stretch::process(self, input, output)
    }
    fn flush(&mut self, output: &mut [f32]) {
        Stretch::flush(self, output)
    }
    fn reset(&mut self) {
        Stretch::reset(self)
    }
}

impl Engine for signalsmith_stretch::Stretch {
    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        signalsmith_stretch::Stretch::process(self, input, output)
    }
    fn flush(&mut self, output: &mut [f32]) {
        signalsmith_stretch::Stretch::flush(self, output)
    }
    fn reset(&mut self) {
        signalsmith_stretch::Stretch::reset(self)
    }
}

fn engines(ch: usize, rate: u32) -> (Stretch, signalsmith_stretch::Stretch) {
    (Stretch::preset_cheaper(ch, rate as f32), signalsmith_stretch::Stretch::preset_cheaper(ch as u32, rate))
}

/// Runs `x` through at `ratio` input frames per output frame, then flushes.
fn run(s: &mut impl Engine, x: &[f32], ch: usize, ratio: f64) -> Vec<f32> {
    let mut out = Vec::new();
    let mut buf = vec![0f32; (BLOCK * 4 + 8) * ch];
    let mut frac = 0.0;
    for block in x.chunks(BLOCK * ch) {
        let want = (block.len() / ch) as f64 / ratio + frac;
        let m = want.floor() as usize;
        frac = want - m as f64;
        s.process(block, &mut buf[..m * ch]);
        out.extend_from_slice(&buf[..m * ch]);
    }
    let mut tail = vec![0f32; 4000 * ch];
    s.flush(&mut tail);
    out.extend_from_slice(&tail);
    out
}

/// A small deterministic noise source in [-1, 1).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

const TAU: f64 = 2.0 * std::f64::consts::PI;

/// `secs` of `f(t, channel)`, interleaved.
fn signal(secs: f64, ch: usize, f: impl Fn(f64, usize) -> f64) -> Vec<f32> {
    let n = (secs * RATE as f64) as usize;
    (0..n).flat_map(|i| (0..ch).map(move |c| (i, c))).map(|(i, c)| f(i as f64 / RATE as f64, c) as f32).collect()
}

fn sines(ch: usize) -> Vec<f32> {
    signal(4.0, ch, |t, c| 0.3 * (TAU * 440.0 * t).sin() + 0.2 * (TAU * (1000.0 + 250.0 * c as f64) * t).sin())
}

fn chirp(ch: usize) -> Vec<f32> {
    // 100 Hz to 8 kHz, exponentially, over 4 s.
    let k = (8000.0f64 / 100.0).ln() / 4.0;
    signal(4.0, ch, move |t, c| 0.4 * (TAU * 100.0 * ((k * t).exp() - 1.0) / k + c as f64 * 0.3).sin())
}

/// Bursts of noise with gaps long enough for the stretcher's silence path.
fn noise_bursts(ch: usize) -> Vec<f32> {
    let mut r = Lcg(7);
    let n = 4 * RATE as usize;
    (0..n * ch).map(|i| if (i / ch) % 22050 < 6000 { 0.4 * r.next() } else { 0.0 }).collect()
}

/// Bass, a chord that changes every half second, hats and a kick: something like a song.
fn music(ch: usize) -> Vec<f32> {
    let mut r = Lcg(11);
    let hats: Vec<f32> = (0..4 * RATE as usize).map(|_| r.next()).collect();
    signal(4.0, ch, move |t, c| {
        let root = [110.0, 87.31, 130.81, 98.0][(t * 2.0) as usize % 4];
        let bass = 0.25 * (TAU * root * t).sin();
        let chord: f64 = [1.0, 1.26, 1.5].iter().map(|m| 0.08 * (TAU * root * 4.0 * m * t + c as f64).sin()).sum();
        let beat = t % 0.5;
        let kick = 0.5 * (-beat * 30.0).exp() * (TAU * (50.0 + 80.0 * (-beat * 40.0).exp()) * beat).sin();
        let hat = 0.1 * (-((t + 0.25) % 0.5) * 200.0).exp() * hats[(t * RATE as f64) as usize] as f64;
        bass + chord + kick + hat
    })
}

type Input = (&'static str, fn(usize) -> Vec<f32>);

const INPUTS: [Input; 4] = [("sines", sines), ("chirp", chirp), ("noise bursts", noise_bursts), ("music", music)];

/// Input frames per output frame: AutoMix's ratios up to its maximum, and close to 1. Below about 0.55 the original
/// randomizes phases from an unseeded generator; see `half_speed_*`.
const RATIOS: [f64; 7] = [1.25, 0.8, 1.03, 0.9985, 0.6, 1.9, 2.0];

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, v| m.max(v.abs()))
}

/// The largest difference.
fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()))
}

fn level_db(a: &[f32], b: &[f32]) -> f64 {
    let power = |x: &[f32]| x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
    10.0 * (power(a) / power(b)).log10()
}

/// The level difference in dB per 2048-frame window and band (24 log-spaced from 60 Hz to 16 kHz), averaged over
/// those where `b` is within 60 dB of its loudest band.
fn spectral_distance(a: &[f32], b: &[f32], ch: usize) -> f64 {
    let n = 2048;
    let fft = rustfft::FftPlanner::new().plan_fft_forward(n);
    let edges: Vec<usize> = (0..=24).map(|k| (60.0 * (16000.0f64 / 60.0).powf(k as f64 / 24.0) / RATE as f64 * n as f64) as usize).collect();
    let (mut sum, mut count) = (0.0, 0usize);
    for c in 0..ch {
        for start in (0..a.len() / ch - n).step_by(n) {
            let bands = |x: &[f32]| {
                let mut buf: Vec<Complex32> =
                    (0..n).map(|i| Complex32::new(x[(start + i) * ch + c] * (0.5 - 0.5 * (TAU * i as f64 / n as f64).cos()) as f32, 0.0)).collect();
                fft.process(&mut buf);
                edges.windows(2).map(|e| buf[e[0]..e[1].max(e[0] + 1)].iter().map(|v| v.norm_sqr() as f64).sum::<f64>()).collect::<Vec<_>>()
            };
            let (ea, eb) = (bands(a), bands(b));
            let floor = eb.iter().cloned().fold(1e-12, f64::max) * 1e-6;
            for (x, y) in ea.iter().zip(&eb).filter(|(_, y)| **y > floor) {
                sum += (10.0 * ((x + 1e-30) / y).log10()).abs();
                count += 1;
            }
        }
    }
    sum / count.max(1) as f64
}

/// The difference's RMS over `b`'s.
fn relative_rms(a: &[f32], b: &[f32]) -> f64 {
    let error: f64 = a.iter().zip(b).map(|(x, y)| ((x - y) as f64).powi(2)).sum();
    let signal: f64 = b.iter().map(|y| (*y as f64).powi(2)).sum();
    (error / signal.max(1e-30)).sqrt()
}

#[test]
fn excerpts_match_sample_for_sample() {
    // Eight 8192-frame excerpts of every input at every setting, each after a reset, to the end of the flush. Most
    // stay at float rounding; the rest are where the original itself moves as much under a one-ulp nudge (noise
    // fading to silence, where bins near the noise floor flip).
    let mut errors = Vec::new();
    for ch in [1, 2] {
        for (name, input) in INPUTS {
            let x = input(ch);
            for ratio in RATIOS {
                let (mut rust, mut cpp) = engines(ch, RATE);
                for k in 0..8 {
                    let excerpt = &x[k * 19_000 * ch..(k * 19_000 + 8192) * ch];
                    rust.reset();
                    Engine::reset(&mut cpp);
                    let (a, b) = (run(&mut rust, excerpt, ch, ratio), run(&mut cpp, excerpt, ch, ratio));
                    let e = relative_rms(&a, &b);
                    assert!(e < 0.05, "{ch} ch {name} at {ratio}, excerpt {k}: {e:.1e} of the signal off");
                    errors.push(e);
                }
            }
        }
    }
    errors.sort_by(f64::total_cmp);
    let quantile = |q: f64| errors[((errors.len() - 1) as f64 * q) as usize];
    let (median, p90) = (quantile(0.5), quantile(0.9));
    eprintln!("difference over signal, RMS: median {median:.1e}, 90 % {p90:.1e}, largest {:.1e}", quantile(1.0));
    assert!(median < 1e-5 && p90 < 2e-4, "median {median:.1e}, 90 % {p90:.1e}");
}

#[test]
fn long_runs_stay_within_the_originals_own_rounding_spread() {
    let (mut worst_dist, mut worst_level) = (0f64, 0f64);
    for ch in [1, 2] {
        for (name, input) in INPUTS {
            let x = input(ch);
            let nudged: Vec<f32> = x.iter().map(|v| if *v != 0.0 { v.next_up() } else { 0.0 }).collect();
            for ratio in RATIOS {
                let (mut rust, mut cpp) = engines(ch, RATE);
                let (_, mut cpp_nudged) = engines(ch, RATE);
                let (a, b, c) = (run(&mut rust, &x, ch, ratio), run(&mut cpp, &x, ch, ratio), run(&mut cpp_nudged, &nudged, ch, ratio));
                let (dist, own) = (spectral_distance(&a, &b, ch), spectral_distance(&c, &b, ch));
                let level = level_db(&a, &b);
                let case = format!("{ch} ch {name} at {ratio}");
                assert!(dist <= 2.0 * own + 0.02, "{case}: {dist:.3} dB from the original, which is {own:.3} dB from itself");
                assert!(level.abs() < 0.05, "{case}: {level:.3} dB louder than the original");
                (worst_dist, worst_level) = (worst_dist.max(dist), worst_level.max(level.abs()));
            }
        }
    }
    eprintln!("spectra at most {worst_dist:.3} dB apart on average, levels {worst_level:.4} dB");
}

#[test]
fn half_speed_matches_the_original_statistically() {
    // AutoMix's slowest ratio: some blocks stretch past 2x, where both randomize phases (the original unseeded).
    for ch in [1, 2] {
        for (name, input) in INPUTS {
            let x = input(ch);
            let (mut rust, mut cpp) = engines(ch, RATE);
            let (a, b) = (run(&mut rust, &x, ch, 0.5), run(&mut cpp, &x, ch, 0.5));
            let (dist, level) = (spectral_distance(&a, &b, ch), level_db(&a, &b));
            assert!(dist < 1.0 && level.abs() < 0.3, "{ch} ch {name}: {dist:.3} dB apart, {level:.3} dB louder");
        }
    }
}

#[test]
fn latencies_are_the_originals() {
    for rate in [8000, 22_050, 44_100, 48_000, 96_000] {
        for ch in [1, 2, 6] {
            let (rust, cpp) = engines(ch, rate);
            assert_eq!((rust.input_latency(), rust.output_latency()), (cpp.input_latency(), cpp.output_latency()), "{rate} Hz, {ch} ch");
        }
    }
}

#[test]
fn reset_forgets_what_came_before() {
    let x = music(2);
    let (mut used, _) = engines(2, RATE);
    run(&mut used, &noise_bursts(2), 2, 0.8);
    used.reset();
    let (mut fresh, _) = engines(2, RATE);
    let (a, b) = (run(&mut used, &x, 2, 1.25), run(&mut fresh, &x, 2, 1.25));
    // Only each bin's previous energy survives a reset, as in the original; it scales a phase that is normalized.
    let d = max_diff(&a, &b) / peak(&b);
    assert!(d < 1e-5, "{d:.2e} of the peak apart");
}

#[test]
fn identical_channels_come_out_as_the_mono_signal() {
    let mono = music(1);
    let stereo: Vec<f32> = mono.iter().flat_map(|v| [*v, *v]).collect();
    for ratio in [1.25, 0.8] {
        let (mut one, _) = engines(1, RATE);
        let (mut two, _) = engines(2, RATE);
        let (a, b) = (run(&mut one, &mono, 1, ratio), run(&mut two, &stereo, 2, ratio));
        let (left, right): (Vec<f32>, Vec<f32>) = b.chunks(2).map(|f| (f[0], f[1])).unzip();
        assert_eq!(left, a, "{ratio}: the left channel is not the mono signal");
        let d = max_diff(&right, &left) / peak(&left);
        // Locked to the left through a renormalization that rounds: the original is 3e-4 of the peak apart here.
        assert!(d < 1e-3, "{ratio}: right {d:.2e} of the peak from left");
    }
}
