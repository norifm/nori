//! Level measurements: BS.1770 loudness, silence trims, hidden-track gaps and MixRamp points, all read from
//! 100 ms blocks measured while the audio streams.

/// Block length for every level measurement.
pub(crate) const BLOCK_MS: i64 = 100;
/// Silence trim threshold, dBFS RMS.
pub(crate) const SILENCE_DB: f64 = -55.0;
/// MixRamp threshold relative to integrated loudness, dB.
pub(crate) const MIXRAMP_DB: f64 = -17.0;
/// A loop's slice jumps by as much as its level drifts each time round: more than this is not steady, dB.
pub(crate) const STEADY_DB: f64 = 3.0;

/// BS.1770's K-weighting: the head's shelf and the high-pass, as one fourth-order filter (libebur128's
/// coefficients, MIT).
struct KWeighting {
    b: [f64; 5],
    a: [f64; 5],
    state: [f64; 5],
}

impl KWeighting {
    #[allow(non_snake_case)]
    fn new(rate: f64) -> Self {
        let (f0, G, Q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let K = (std::f64::consts::PI * f0 / rate).tan();
        let Vh = 10f64.powf(G / 20.0);
        let Vb = Vh.powf(0.4996667741545416);
        let a0 = 1.0 + K / Q + K * K;
        let pb = [(Vh + Vb * K / Q + K * K) / a0, 2.0 * (K * K - Vh) / a0, (Vh - Vb * K / Q + K * K) / a0];
        let pa = [1.0, 2.0 * (K * K - 1.0) / a0, (1.0 - K / Q + K * K) / a0];
        let (f0, Q) = (38.13547087602444, 0.5003270373238773);
        let K = (std::f64::consts::PI * f0 / rate).tan();
        let rb = [1.0, -2.0, 1.0];
        let ra = [1.0, 2.0 * (K * K - 1.0) / (1.0 + K / Q + K * K), (1.0 - K / Q + K * K) / (1.0 + K / Q + K * K)];
        let product = |p: [f64; 3], r: [f64; 3]| [p[0] * r[0], p[0] * r[1] + p[1] * r[0], p[0] * r[2] + p[1] * r[1] + p[2] * r[0], p[1] * r[2] + p[2] * r[1], p[2] * r[2]];
        KWeighting { b: product(pb, rb), a: product(pa, ra), state: [0.0; 5] }
    }

    /// The sum of the squares of `x` filtered.
    fn sum_of_squares(&mut self, x: &[f32]) -> f64 {
        let (a, b, s) = (self.a, self.b, &mut self.state);
        let mut sum = 0.0;
        for &v in x {
            // The newest state last: only its product waits on the sample before.
            s[0] = v as f64 - a[4] * s[4] - a[3] * s[3] - a[2] * s[2] - a[1] * s[1];
            let y = b[0] * s[0] + b[1] * s[1] + b[2] * s[2] + b[3] * s[3] + b[4] * s[4];
            (s[4], s[3], s[2], s[1]) = (s[3], s[2], s[1], s[0]);
            sum += y * y;
        }
        // A decaying silence would otherwise run on in subnormals, slow on some CPUs.
        for v in s.iter_mut().filter(|v| v.abs() < f64::EPSILON) {
            *v = 0.0;
        }
        sum
    }
}

/// Streaming 100 ms block meter: each block's mean square, K-weighted and plain. [`integrated`] gates the
/// blocks.
pub struct Meter {
    k: KWeighting,
    /// Samples per block (100 ms).
    block: usize,
    n: usize,
    acc_k: f64,
    acc_raw: f64,
    /// Mean square per block, K-weighted.
    pub blocks_k: Vec<f32>,
    /// Mean square per block, unweighted.
    pub blocks_raw: Vec<f32>,
}

impl Meter {
    pub fn new(rate: f64, expected_blocks: usize) -> Self {
        let rate = (rate.round() as u32).clamp(16, 2_822_400);
        Meter {
            k: KWeighting::new(rate as f64),
            block: (rate as usize + 5) / 10,
            n: 0,
            acc_k: 0.0,
            acc_raw: 0.0,
            blocks_k: Vec::with_capacity(expected_blocks),
            blocks_raw: Vec::with_capacity(expected_blocks),
        }
    }

    /// Mono samples in [-1, 1]; non-finite samples count as silence.
    pub fn feed(&mut self, mut x: &[f32]) {
        while !x.is_empty() {
            let (part, rest) = x.split_at((self.block - self.n).min(x.len()));
            // Non-finite samples make the sum non-finite; only then are they cleaned one by one.
            let raw = sum_of_squares(part);
            if raw.is_finite() {
                self.add(part, raw);
            } else {
                let mut clean = [0f32; 256];
                for c in part.chunks(clean.len()) {
                    for (d, v) in clean.iter_mut().zip(c) {
                        *d = if v.is_finite() { *v } else { 0.0 };
                    }
                    let clean = &clean[..c.len()];
                    self.add(clean, sum_of_squares(clean));
                }
            }
            if self.n == self.block {
                self.flush_block();
            }
            x = rest;
        }
    }

    fn add(&mut self, x: &[f32], raw: f64) {
        self.acc_k += self.k.sum_of_squares(x);
        self.acc_raw += raw;
        self.n += x.len();
    }

    fn flush_block(&mut self) {
        if self.n == 0 {
            return;
        }
        self.blocks_k.push((self.acc_k / self.n as f64) as f32);
        self.blocks_raw.push((self.acc_raw / self.n as f64) as f32);
        (self.acc_k, self.acc_raw, self.n) = (0.0, 0.0, 0);
    }

    /// Closes a partial last block when at least a quarter full (a shorter one would read as a fade).
    pub fn finish(&mut self) {
        if self.n * 4 >= self.block {
            self.flush_block();
        }
        (self.acc_k, self.acc_raw, self.n) = (0.0, 0.0, 0);
    }

    /// Ready for the next song, keeping allocations.
    pub fn reset(&mut self) {
        self.k.state = [0.0; 5];
        (self.acc_k, self.acc_raw, self.n) = (0.0, 0.0, 0);
        self.blocks_k.clear();
        self.blocks_raw.clear();
    }
}

/// Sum of the squares, in eight lanes so it vectorises.
fn sum_of_squares(x: &[f32]) -> f64 {
    let mut lanes = [0f64; 8];
    let (chunks, rest) = x.as_chunks::<8>();
    for c in chunks {
        for (l, v) in lanes.iter_mut().zip(c) {
            *l += *v as f64 * *v as f64;
        }
    }
    rest.iter().map(|v| *v as f64 * *v as f64).sum::<f64>() + lanes.iter().sum::<f64>()
}

fn lufs_of(mean_square: f64) -> f64 {
    if mean_square <= 1e-12 {
        -120.0
    } else {
        -0.691 + 10.0 * mean_square.log10()
    }
}

pub fn db(mean_square: f64) -> f64 {
    10.0 * mean_square.max(1e-12).log10()
}

/// Momentary (400 ms) mean squares, one per 100 ms step: window `j` covers blocks `j..j+4`.
fn momentary(blocks_k: &[f32]) -> impl Iterator<Item = f64> + '_ {
    blocks_k.windows(4.min(blocks_k.len()).max(1)).map(|w| w.iter().map(|v| *v as f64).sum::<f64>() / w.len() as f64)
}

/// Integrated loudness, BS.1770-4 gating (absolute -70 LUFS, relative -10 LU) over 400 ms windows with 75 % overlap.
pub(crate) fn integrated(blocks_k: &[f32]) -> f64 {
    let gated: Vec<f64> = momentary(blocks_k).filter(|ms| lufs_of(*ms) > -70.0).collect();
    if gated.is_empty() {
        return -70.0;
    }
    let rel = lufs_of(gated.iter().sum::<f64>() / gated.len() as f64) - 10.0;
    let (sum, n) = gated.iter().filter(|ms| lufs_of(**ms) > rel).fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 {
        -70.0
    } else {
        lufs_of(sum / n as f64).max(-70.0)
    }
}

/// First and last audible moment, ms; `(0, 0)` for silence.
pub(crate) fn silence_trim(blocks_raw: &[f32]) -> (i64, i64) {
    let loud = |v: &f32| db(*v as f64) > SILENCE_DB;
    match (blocks_raw.iter().position(loud), blocks_raw.iter().rposition(loud)) {
        (Some(a), Some(b)) => (a as i64 * BLOCK_MS, (b as i64 + 1) * BLOCK_MS),
        _ => (0, 0),
    }
}

/// The last silence of at least `min_ms` inside the music, (start, end) ms: the gap before a hidden track.
pub(crate) fn last_gap(blocks_raw: &[f32], min_ms: i64) -> Option<(i64, i64)> {
    let loud = |v: &f32| db(*v as f64) > SILENCE_DB;
    let (first, last) = (blocks_raw.iter().position(loud)?, blocks_raw.iter().rposition(loud)?);
    let min_blocks = (min_ms / BLOCK_MS).max(1) as usize;
    let mut j = last;
    while j > first {
        if loud(&blocks_raw[j]) {
            j -= 1;
            continue;
        }
        let end = j + 1;
        while j > first && !loud(&blocks_raw[j]) {
            j -= 1;
        }
        if end - (j + 1) >= min_blocks {
            return Some(((j + 1) as i64 * BLOCK_MS, end as i64 * BLOCK_MS));
        }
    }
    None
}

/// The most bars a steady run is counted over: more than any loop.
const STEADY_BARS: i64 = 16;

/// Where the music is steady enough to loop, ms, in bars of `bar` from the bar line `first`: the end of the run of
/// opening bars, and the starts of the runs before the last bar line of the music and of `exit`, every bar of a run
/// within `STEADY_DB` of the others (a loop of them jumps by no more each time round).
pub(crate) fn steady_ends(blocks_raw: &[f32], music: (i64, i64), first: i64, bar: i64, exit: Option<i64>) -> (i64, i64, i64) {
    let level = |from: i64| {
        let (a, b) = ((from / BLOCK_MS).max(0) as usize, (((from + bar) / BLOCK_MS) as usize).min(blocks_raw.len()));
        db(blocks_raw[a..b.max(a)].iter().map(|v| *v as f64).sum::<f64>() / (b - a).max(1) as f64)
    };
    // How many of the bars starting at `starts` keep together.
    let run = |starts: &mut dyn Iterator<Item = i64>| {
        let (mut lo, mut hi, mut n) = (f64::MAX, f64::MIN, 0);
        for t in starts.take(STEADY_BARS as usize).filter(|&t| t >= music.0 && t + bar <= music.1) {
            let l = level(t);
            (lo, hi) = (lo.min(l), hi.max(l));
            if hi - lo > STEADY_DB {
                break;
            }
            n += 1;
        }
        n
    };
    let line_before = |t: i64| first + (t - first).div_euclid(bar) * bar;
    let back_from = |end: i64| end - run(&mut (1..).map(|m| end - m * bar)) * bar;
    let intro = first + run(&mut (0..).map(|m| first + m * bar)) * bar;
    (intro, back_from(line_before(music.1)), exit.map_or(0, |e| back_from(line_before(e))))
}

/// MixRamp points, ms: centres of the first and last 400 ms windows at or above `lufs + MIXRAMP_DB`.
pub fn mixramp(blocks_k: &[f32], lufs: f64) -> Option<(i64, i64)> {
    let thresh = lufs + MIXRAMP_DB;
    let (mut first, mut last) = (None, None);
    for (j, ms) in momentary(blocks_k).enumerate() {
        if lufs_of(ms) >= thresh {
            first.get_or_insert(j);
            last = Some(j);
        }
    }
    let centre = |j: usize| j as i64 * BLOCK_MS + 2 * BLOCK_MS;
    Some((centre(first?), centre(last?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ebur128::{EbuR128, Mode};

    fn meter(x: &[f32], rate: f64) -> Meter {
        let mut m = Meter::new(rate, 0);
        x.chunks(1000).for_each(|c| m.feed(c));
        m.finish();
        m
    }

    fn sine(freq: f64, amp: f64, secs: f64, rate: f64) -> Vec<f32> {
        (0..(secs * rate) as usize).map(|i| (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / rate).sin()) as f32).collect()
    }

    #[test]
    fn integrated_loudness() {
        for rate in [44100.0, 48000.0] {
            let m = meter(&sine(997.0, 1.0, 5.0, rate), rate);
            let l = integrated(&m.blocks_k);
            assert!((l + 3.01).abs() < 0.1, "{rate}: {l}");
        }
        let m = meter(&sine(997.0, 0.1, 5.0, 44100.0), 44100.0);
        assert!((integrated(&m.blocks_k) + 23.01).abs() < 0.1);

        // Integrated matches ebur128.
        let rate = 44100.0;
        let mut steps = sine(440.0, 0.5, 10.0, rate);
        steps.extend(sine(440.0, 0.02, 10.0, rate)); // below the relative gate
        steps.extend(vec![0f32; 44100]); // below the absolute one
        steps.extend(sine(3000.0, 0.2, 7.3, rate));
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..44100 * 30)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed as f32 / u32::MAX as f32 - 0.5) * (0.05 + 0.4 * ((i as f32 / 44100.0 * 0.3).sin().abs()))
            })
            .collect();
        for (name, x, rate) in [("997 Hz", sine(997.0, 0.3, 5.0, 48000.0), 48000.0), ("steps", steps, rate), ("noise", noise, rate)] {
            let mut r = EbuR128::new(1, rate as u32, Mode::I).unwrap();
            r.add_frames_f32(&x).unwrap();
            let theirs = r.loudness_global().unwrap();
            let ours = integrated(&meter(&x, rate).blocks_k);
            assert!((ours - theirs).abs() < 0.01, "{name}: {ours} against ebur128's {theirs}");
        }
    }

    #[test]
    fn chunks_and_nan_do_not_matter() {
        let rate = 44100.0;
        let x = sine(440.0, 0.5, 3.0, rate);
        let whole = meter(&x, rate);
        let mut m = Meter::new(rate, 0);
        for c in x.chunks(777) {
            m.feed(c);
        }
        m.finish();
        assert_eq!((&m.blocks_k, &m.blocks_raw), (&whole.blocks_k, &whole.blocks_raw));

        let (mut with_nan, mut with_zero) = (x.clone(), x);
        for i in [10_000, 10_001, 50_000] {
            (with_nan[i], with_zero[i]) = (if i == 50_000 { f32::INFINITY } else { f32::NAN }, 0.0);
        }
        let (a, b) = (meter(&with_nan, rate), meter(&with_zero, rate));
        assert_eq!((&a.blocks_k, &a.blocks_raw), (&b.blocks_k, &b.blocks_raw));
    }

    /// 100 ms blocks at `db` each: a level per second, ten blocks to it.
    fn blocks(levels: &[(f64, f64)]) -> Vec<f32> {
        levels.iter().flat_map(|&(secs, db)| std::iter::repeat_n(10f64.powf(db / 10.0) as f32, (secs * 10.0).round() as usize)).collect()
    }

    #[test]
    fn steady_ends_stop_where_the_level_moves() {
        // Silence, 8 s of intro, a jump of 8 dB, the body, a 12 s fade to silence (40 dB, a step a second); 2 s bars
        // from 1 s.
        let fade: Vec<(f64, f64)> = (0..12).map(|i| (1.0, -20.0 - 40.0 * (i as f64 + 0.5) / 12.0)).collect();
        let song = [vec![(1.0, -90.0), (8.0, -28.0), (100.0, -20.0)], fade, vec![(2.0, -90.0)]].concat();
        let b = blocks(&song);
        let music = silence_trim(&b);
        let (intro, end, exit) = steady_ends(&b, music, 1_000, 2_000, Some(109_000));
        assert_eq!(intro, 9_000, "the opening's bars keep together until the jump at 9 s");
        assert_eq!(end, 117_000, "only the last bar of a fade: {music:?}");
        assert_eq!(exit, 109_000 - STEADY_BARS * 2_000, "the body before the fade, as many bars as are counted");
        // A bar that swings within itself, the same each time, is steady.
        let swing: Vec<(f64, f64)> = (0..30).flat_map(|_| [(1.2, -12.0), (0.8, -20.0)]).collect();
        let b = blocks(&[vec![(1.0, -90.0)], swing, vec![(2.0, -90.0)]].concat());
        let (s0, s1) = silence_trim(&b);
        assert_eq!(steady_ends(&b, (s0, s1), s0, 2_000, None), (s0 + STEADY_BARS * 2_000, s1 - STEADY_BARS * 2_000, 0));
    }

    #[test]
    fn finds_last_long_gap() {
        let rate = 8000.0;
        let mut x = sine(440.0, 0.5, 10.0, rate);
        x.extend(vec![0f32; (rate * 2.0) as usize]); // a 2 s rest
        x.extend(sine(440.0, 0.5, 5.0, rate));
        x.extend(vec![0f32; (rate * 8.0) as usize]); // an 8 s gap
        x.extend(sine(440.0, 0.5, 3.0, rate));
        x.extend(vec![0f32; (rate * 4.0) as usize]); // trailing silence is not inside the music
        let m = meter(&x, rate);
        assert_eq!(last_gap(&m.blocks_raw, 6000), Some((17_000, 25_000)));
        assert_eq!(last_gap(&m.blocks_raw, 1000), Some((17_000, 25_000)), "the last one, not the first");
        assert_eq!(last_gap(&m.blocks_raw, 9000), None);
        assert_eq!(last_gap(&[], 6000), None);
    }

    #[test]
    fn trims_and_ramps() {
        let rate = 44100.0;
        let mut x = vec![0f32; (rate * 2.0) as usize]; // 2 s of silence
        let fade_in: Vec<f32> = sine(440.0, 0.5, 4.0, rate).iter().enumerate().map(|(i, v)| v * (i as f32 / (rate as f32 * 4.0))).collect();
        x.extend(fade_in); // a 4 s linear fade-in
        x.extend(sine(440.0, 0.5, 20.0, rate));
        x.extend(vec![0f32; (rate * 3.0) as usize]);
        let m = meter(&x, rate);
        let (start, end) = silence_trim(&m.blocks_raw);
        // The ramp crosses -55 dB after 4 s * 10^(-46/20) = 20 ms.
        assert!((2000..=2100).contains(&start), "start {start}");
        assert_eq!(end, 26000);
        let lufs = integrated(&m.blocks_k);
        let (r_in, r_out) = mixramp(&m.blocks_k, lufs).unwrap();
        // -17 dB is 0.56 s into the ramp.
        assert!((2400..=3000).contains(&r_in), "ramp in {r_in}");
        assert!((25700..=26100).contains(&r_out), "ramp out {r_out}");

        // Silence is minus 70 and has no ramp.
        let m = meter(&vec![0.0; 44100 * 3], 44100.0);
        assert_eq!(integrated(&m.blocks_k), -70.0);
        assert_eq!(silence_trim(&m.blocks_raw), (0, 0));
        assert_eq!(mixramp(&m.blocks_k, -70.0), None);
    }

}
