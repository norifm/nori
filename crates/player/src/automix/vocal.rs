//! The vocal activity curve synced lyrics are checked against (nori-lyrics sync.rs), measured from the analysis's
//! own FFT. A voice is one pitched sound in 300 Hz-3 kHz that never holds still. Each frame finds the band's clear
//! spectral peaks and keeps those that belong to its strongest harmonic series (F0 80-1000 Hz, found among peaks
//! that last over three frames): a lead voice is one series, while noise (drums, a cymbal swelling into a line)
//! has peaks that come and go and fit no series. Over those it sums how far the log level moved (syllables) and
//! how far each lasting peak's frequency moved (vibrato, glides; a held note does not move). On real songs it
//! is weak on level (AUC about 0.75) but good on timing; the band, peak test and series were tuned there.
//! A stereo song is measured on the middle of its image only, gating out panned guitars
//! ([`centre`]). One byte per [`CURVE_EVERY`] frames, about a kilobyte a minute.

use rustfft::num_complex::Complex32;

/// The voice band, Hz.
pub(crate) const LO_HZ: f64 = 300.0;
pub(crate) const HI_HZ: f64 = 3000.0;
/// Log compression of the band's amplitudes, as for the onset curves.
const GAMMA: f32 = 1000.0;
/// A peak is this many times the power of the bins three away from it.
const PEAK_OVER: f32 = 5.0;
/// A peak lasts when the frame before had one within this many bins of it, and the one before that twice as far.
const LASTS_BINS: f32 = 1.5;
/// A lasting peak's frequency moving this far in two frames (23 ms) counts in full: about the steepest slope of
/// a sung vibrato.
const MOVE_FULL_CENTS: f32 = 30.0;
/// How much a peak's frequency movement counts beside its level movement, per unit of its compressed amplitude.
const MOVE_WEIGHT: f32 = 0.5;
/// The fundamental of the harmonic series looked for, Hz.
const F0_LO_HZ: f64 = 80.0;
const F0_HI_HZ: f64 = 1000.0;
/// Candidates for the series' fundamental: the strongest lasting peaks, each as harmonic 1 to [`HARMONICS`].
const CANDIDATES: usize = 8;
const HARMONICS: usize = 8;
/// A peak is harmonic `r` of a fundamental when within this many bins plus [`HARMONIC_SHARE`] of `r` × F0...
const HARMONIC_BINS: f32 = 0.3;
const HARMONIC_SHARE: f32 = 0.015;
/// ...and counts toward the series this much less for each harmonic up, so a series an octave low loses.
const HARMONIC_DECAY: f32 = 0.92;
/// Averaging time of each bin's mid and side power, seconds: long enough that double-tracked panned guitars read
/// steadily off centre, short enough to follow a voice coming in.
const CENTRE_SMOOTH_S: f64 = 0.1;
/// A bin whose channels are less alike than this is not in the middle ([`centre`]); tuned on `metal_band`.
const CENTRE_FLOOR: f32 = 0.6;
/// Analysis frames per curve frame: about 17 a second, 58 ms each.
pub(crate) const CURVE_EVERY: usize = 5;
/// The stored form's version: a curve in another is measured again.
pub(crate) const CURVE_VERSION: u8 = 3;
/// Bytes of the stored form before the levels: the version, the frame rate and the time of frame 0.
const HEADER: usize = 9;
/// A curve frame's movement `m` is stored as `ln(1 + m) * LEVEL_SCALE`, clamped to a byte.
const LEVEL_SCALE: f32 = 40.0;

/// Builds the curve from each analysis frame's spectrum.
pub struct Tracker {
    lo: usize,
    hi: usize,
    amp_norm: f32,
    /// The band's power, with three bins of margin each side for the peak test.
    pow: Vec<f32>,
    /// The band's power the frame before.
    prev: Vec<f32>,
    /// The mid's and the side's power per bin of `pow`, averaged over about [`CENTRE_SMOOTH_S`].
    mid_pow: Vec<f32>,
    side_pow: Vec<f32>,
    /// The average's weight on what it was the frame before.
    keep: f32,
    acc: f32,
    n: usize,
    curve: Vec<f32>,
    /// Peak positions in fractional bins, ascending, of the frame before and the one before that.
    before: [Vec<f32>; 2],
    /// This frame's peaks.
    peaks: Vec<Peak>,
    /// The series' fundamental range, bins.
    f0_bins: (f32, f32),
}

/// A clear spectral peak of one frame.
#[derive(Clone, Copy)]
struct Peak {
    /// Fractional bin.
    at: f32,
    /// Log-compressed amplitude.
    amp: f32,
    /// How far the log level moved around it since the frame before.
    level_move: f32,
    /// How far its frequency moved over two frames, 0 to 1 ([`MOVE_FULL_CENTS`]); None when it does not last.
    pitch_move: Option<f32>,
}

/// Whether the peak at bin `at` is a harmonic of the fundamental `f0` (bins).
fn harmonic(f0: f32, at: f32) -> bool {
    let r = (at / f0).round();
    r >= 1.0 && (at - r * f0).abs() <= HARMONIC_BINS + HARMONIC_SHARE * at
}

/// The offset from `at` to the nearest of `sorted` within `reach` bins.
fn nearest(sorted: &[f32], at: f32, reach: f32) -> Option<f32> {
    let i = sorted.partition_point(|q| *q < at);
    [i.checked_sub(1), Some(i)].into_iter().flatten().filter_map(|j| sorted.get(j)).map(|q| at - q).filter(|d| d.abs() <= reach).min_by(|a, b| a.abs().total_cmp(&b.abs()))
}

impl Tracker {
    /// For an `n`-point FFT at `sr` Hz where |X| × `amp_norm` is a sine's amplitude; `frames` sizes the curve.
    pub fn new(n: usize, hop: usize, sr: f64, amp_norm: f32, frames: usize) -> Self {
        let bin_hz = sr / n as f64;
        let lo = ((LO_HZ / bin_hz).ceil() as usize).max(4);
        let hi = ((HI_HZ.min(sr / 2.0 * 0.9) / bin_hz) as usize).min(n / 2 - 4).max(lo + 8);
        let w = hi - lo + 1;
        Tracker {
            lo,
            hi,
            amp_norm,
            pow: vec![0.0; w + 6],
            prev: vec![0.0; w],
            mid_pow: vec![0.0; w + 6],
            side_pow: vec![0.0; w + 6],
            keep: (-(hop as f64) / sr / CENTRE_SMOOTH_S).exp() as f32,
            acc: 0.0,
            n: 0,
            curve: Vec::with_capacity(frames / CURVE_EVERY + 2),
            before: [Vec::with_capacity(w), Vec::with_capacity(w)],
            peaks: Vec::with_capacity(w),
            f0_bins: ((F0_LO_HZ / bin_hz) as f32, (F0_HI_HZ / bin_hz) as f32),
        }
    }

    /// One frame's spectrum of a mono source.
    pub fn frame(&mut self, spec: &[Complex32]) {
        let (lo, hi, keep) = (self.lo, self.hi, self.keep);
        for (((p, c), mp), sp) in self.pow.iter_mut().zip(&spec[lo - 3..=hi + 3]).zip(&mut self.mid_pow).zip(&mut self.side_pow) {
            *p = c.norm_sqr();
            // Kept up so a stereo stretch after a mono one starts from what was heard.
            *mp = keep * *mp + (1.0 - keep) * *p;
            *sp *= keep;
        }
        self.step();
    }

    /// One frame of a stereo source: its mid and side spectra. Each bin's mid is weighted by how centred it has
    /// sounded lately ([`centre`]).
    pub fn frame_stereo(&mut self, mid: &[Complex32], side: &[Complex32]) {
        let (lo, hi, keep) = (self.lo, self.hi, self.keep);
        let bins = mid[lo - 3..=hi + 3].iter().zip(&side[lo - 3..=hi + 3]);
        for (((p, (m, s)), mp), sp) in self.pow.iter_mut().zip(bins).zip(&mut self.mid_pow).zip(&mut self.side_pow) {
            let m = m.norm_sqr();
            *mp = keep * *mp + (1.0 - keep) * m;
            *sp = keep * *sp + (1.0 - keep) * s.norm_sqr();
            *p = m * centre(*mp, *sp);
        }
        self.step();
    }

    /// The movement of this frame's voice, read from the peaks in `pow`, added to the curve.
    fn step(&mut self) {
        let w = self.prev.len();
        let g = GAMMA * self.amp_norm;
        let mut done = 0usize;
        self.peaks.clear();
        for j in 0..w {
            let k = j + 3;
            let p = self.pow[k];
            if p >= self.pow[k - 1] && p >= self.pow[k + 1] && p > PEAK_OVER * 0.5 * (self.pow[k - 3] + self.pow[k + 3]) {
                let mut level_move = 0f32;
                for i in j.saturating_sub(1).max(done)..(j + 2).min(w) {
                    let now = 1.0 + g * self.pow[i + 3].sqrt();
                    let before = 1.0 + g * self.prev[i].sqrt();
                    level_move += (now / before).ln().abs();
                }
                done = (j + 2).min(w);
                // A parabola through the log powers puts the peak between bins.
                let (a, b, c) = (self.pow[k - 1].max(1e-20).ln(), p.max(1e-20).ln(), self.pow[k + 1].max(1e-20).ln());
                let d = a - 2.0 * b + c;
                let at = (self.lo + j) as f32 + if d < -1e-9 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 };
                let pitch_move = nearest(&self.before[0], at, LASTS_BINS)
                    .and(nearest(&self.before[1], at, 2.0 * LASTS_BINS))
                    .map(|d| (1200.0 / std::f32::consts::LN_2 * d.abs() / at / MOVE_FULL_CENTS).min(1.0));
                self.peaks.push(Peak { at, amp: (1.0 + g * p.sqrt()).ln(), level_move, pitch_move });
            }
        }
        let f0 = self.strongest_series();
        let moved: f32 = self.peaks.iter().filter(|p| f0.is_some_and(|f0| harmonic(f0, p.at))).map(|p| p.level_move + MOVE_WEIGHT * p.amp * p.pitch_move.unwrap_or(0.0)).sum();
        self.before.swap(0, 1);
        self.before[0].clear();
        self.before[0].extend(self.peaks.iter().map(|p| p.at));
        self.prev.copy_from_slice(&self.pow[3..3 + w]);
        self.acc += moved;
        self.n += 1;
        if self.n == CURVE_EVERY {
            self.curve.push(self.acc / CURVE_EVERY as f32);
            (self.acc, self.n) = (0.0, 0);
        }
    }

    /// The fundamental (bins) of the harmonic series holding the most of this frame's lasting peaks.
    fn strongest_series(&self) -> Option<f32> {
        let lasting = || self.peaks.iter().filter(|p| p.pitch_move.is_some());
        let mut top = [None::<Peak>; CANDIDATES];
        for p in lasting() {
            if let Some(slot) = top.iter().position(|t| t.is_none_or(|t| t.amp < p.amp)) {
                top.copy_within(slot..CANDIDATES - 1, slot + 1);
                top[slot] = Some(*p);
            }
        }
        let (lo, hi) = self.f0_bins;
        let candidates = top.into_iter().flatten().flat_map(|p| (1..=HARMONICS).map(move |h| p.at / h as f32)).filter(|f0| (lo..=hi).contains(f0));
        let score = |f0: f32| lasting().filter(|p| harmonic(f0, p.at)).map(|p| p.amp * HARMONIC_DECAY.powf((p.at / f0).round() - 1.0)).sum::<f32>();
        // On a tie, the strongest peak's candidate.
        candidates.fold(None, |best: Option<(f32, f32)>, f0| {
            let s = score(f0);
            if s > best.map_or(0.0, |b| b.1) { Some((f0, s)) } else { best }
        })
        .map(|b| b.0)
    }

    /// The curve so far, one value per [`CURVE_EVERY`] analysis frames; the tracker starts again.
    pub fn take(&mut self) -> Vec<f32> {
        let out = std::mem::take(&mut self.curve);
        self.reset();
        out
    }

    pub(super) fn reset(&mut self) {
        self.prev.fill(0.0);
        self.before.iter_mut().for_each(Vec::clear);
        self.mid_pow.fill(0.0);
        self.side_pow.fill(0.0);
        self.curve.clear();
        (self.acc, self.n) = (0.0, 0);
    }
}

/// The share of a bin's power counted, from its time-averaged mid and side power. `(mid - side) / (mid + side)` is
/// the channels' similarity (Avendano-Jot): 1 centred, 0.8 panned 6 dB, 0 one-sided or unrelated. Below
/// [`CENTRE_FLOOR`] it counts nothing (a gate: the curve reads level movement, which turning a guitar down keeps).
#[inline]
fn centre(mid: f32, side: f32) -> f32 {
    let sum = mid + side;
    if sum <= 0.0 {
        return 0.0;
    }
    let w = ((mid - side) / sum - CENTRE_FLOOR) / (1.0 - CENTRE_FLOOR);
    w.max(0.0) * w.max(0.0)
}

/// A song's vocal activity, one byte per frame; frame `k` is at `t0 + k / fps` seconds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VocalCurve {
    pub fps: f32,
    pub t0: f32,
    pub level: Vec<u8>,
}

impl VocalCurve {
    /// From the raw curve of an analysis at `fps` frames a second, frame 0 at `t0`.
    pub fn from_raw(raw: &[f32], fps: f64, t0: f64) -> Self {
        let e = CURVE_EVERY as f64;
        VocalCurve {
            fps: (fps / e) as f32,
            t0: (t0 + (e - 1.0) / 2.0 / fps) as f32,
            level: raw.iter().map(|m| ((1.0 + m.max(0.0)).ln() * LEVEL_SCALE).round().clamp(0.0, 255.0) as u8).collect(),
        }
    }

    /// Seconds covered.
    pub fn seconds(&self) -> f64 {
        if self.fps > 0.0 {
            self.level.len() as f64 / self.fps as f64
        } else {
            0.0
        }
    }

    /// Stored form: [`CURVE_VERSION`], fps and t0 (f32 LE), the levels.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.level.len());
        out.push(CURVE_VERSION);
        out.extend_from_slice(&self.fps.to_le_bytes());
        out.extend_from_slice(&self.t0.to_le_bytes());
        out.extend_from_slice(&self.level);
        out
    }

    /// None for another version or a malformed blob.
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER || b[0] != CURVE_VERSION {
            return None;
        }
        let fps = f32::from_le_bytes(b[1..5].try_into().ok()?);
        let t0 = f32::from_le_bytes(b[5..9].try_into().ok()?);
        (fps.is_finite() && fps > 0.0 && t0.is_finite()).then(|| VocalCurve { fps, t0, level: b[HEADER..].to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::super::analysis::Analyzer;
    use super::super::eval::{corpus_all, Song, Style, FULL, SUNG};
    use super::*;

    fn curve_of(song: &Song) -> (VocalCurve, super::super::eval::Truth, f64) {
        let (x, truth) = song.render();
        let mut a = Analyzer::new(song.rate, 0);
        a.feed(&x);
        let secs = x.len() as f64 / song.rate as f64;
        (a.take_features().voice_curve(), truth, secs)
    }

    fn auc(pos: &[f32], neg: &[f32]) -> f64 {
        let mut all: Vec<(f32, bool)> = pos.iter().map(|v| (*v, true)).chain(neg.iter().map(|v| (*v, false))).collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        let rank_sum: f64 = all.iter().enumerate().filter(|(_, (_, p))| *p).map(|(r, _)| (r + 1) as f64).sum();
        let (np, nn) = (pos.len() as f64, neg.len() as f64);
        (rank_sum - np * (np + 1.0) / 2.0) / (np * nn)
    }

    /// AUC of sung against unsung frames of a corpus song.
    fn separation(song: &Song) -> f64 {
        let (c, truth, _) = curve_of(song);
        separation_of(&c, &|t| (t >= truth.music.0 + 1.0 && t <= truth.music.1 - 1.0).then(|| truth.sung_at(t)))
    }

    #[test]
    fn curve_rises_where_voice_sings() {
        let song = Song { sections: vec![(4, FULL), (8, SUNG), (4, FULL), (8, SUNG)], ..Song::new("sung", Style::Backbeat, 110.0, 2, false) };
        let a = separation(&song);
        assert!(a > 0.95, "sung told from unsung frames with an AUC of {a:.3}");
    }

    #[test]
    fn curve_is_small_and_round_trips() {
        let song = Song { sections: vec![(24, SUNG)], ..Song::new("sung", Style::House, 120.0, 2, false) };
        let (c, _, secs) = curve_of(&song);
        let per_min = c.level.len() as f64 / secs * 60.0;
        assert!((1000.0..1100.0).contains(&per_min), "{per_min} bytes a minute");
        assert!((c.fps - 17.2).abs() < 0.1, "{}", c.fps);
        assert_eq!(VocalCurve::decode(&c.encode()), Some(c.clone()));
        assert_eq!(VocalCurve::decode(&[2, 0, 0]), None);
        let mut other = c.encode();
        other[0] = CURVE_VERSION + 1;
        assert_eq!(VocalCurve::decode(&other), None, "another version is measured again");
    }

    /// A dry voice at `rate` Hz: syllables of 180-350 ms on a minor scale from G3, gliding between notes, with
    /// vibrato and two formants; `next(t)` is when singing resumes at or after `t` (`t` while it sings).
    fn voice(secs: f64, rate: f64, next: &dyn Fn(f64) -> f64, rnd: &mut dyn FnMut() -> f64) -> Vec<f64> {
        use std::f64::consts::TAU;
        let len = (secs * rate) as usize;
        let scale = [0.0, 2.0, 3.0, 5.0, 7.0, 8.0, 10.0, 12.0];
        let (mut t, mut f_was, mut phase) = (0.0, 220.0, 0f64);
        let mut dry = vec![0f64; len];
        while t < secs {
            if next(t) > t {
                t = next(t);
                continue;
            }
            let dur = 0.18 + 0.17 * rnd();
            let f = 196.0 * 2f64.powf(scale[(rnd() * 8.0) as usize] / 12.0);
            let (f1, f2) = ([500.0, 700.0, 350.0, 400.0][(rnd() * 4.0) as usize], [1000.0, 1200.0, 2200.0, 1800.0][(rnd() * 4.0) as usize]);
            let (a, n) = ((t * rate) as usize, (dur * rate) as usize);
            for i in 0..n.min(len.saturating_sub(a)) {
                let tau = i as f64 / rate;
                let glide = f_was + (f - f_was) * (tau / 0.04).min(1.0);
                let fv = glide * 2f64.powf(0.25 / 12.0 * (TAU * 5.5 * tau).sin() * ((tau - 0.1) / 0.1).clamp(0.0, 1.0));
                phase = (phase + fv / rate).fract();
                let mut v = 0.0;
                for h in 1..=14 {
                    let hf = fv * h as f64;
                    let formant = (-((hf - f1) / 200.0).powi(2)).exp() + 0.6 * (-((hf - f2) / 300.0).powi(2)).exp() + 0.1;
                    v += formant / h as f64 * (TAU * phase * h as f64).sin();
                }
                let env = (tau / 0.015).min(1.0) * ((dur - tau) / 0.03).min(1.0);
                dry[a + i] = 0.22 * v * env;
            }
            f_was = f;
            t += dur + 0.04;
        }
        dry
    }

    /// A stereo metal band at 44.1 kHz: two distorted, bending takes of a riff hard left and right, drums and a
    /// voice in a room in the middle every other four seconds. Returns the samples and whether the voice sings at
    /// a time (None near a change).
    fn metal_band(secs: f64) -> (Vec<f32>, impl Fn(f64) -> Option<bool>) {
        use std::f64::consts::TAU;
        let rate = 44_100.0;
        let len = (secs * rate) as usize;
        let (mut l, mut r) = (vec![0f64; len], vec![0f64; len]);
        let mut rng = crate::automix::eval::Rng(0x2545_f491_4f6c_dd1d);
        let mut rnd = move || rng.unit();
        let semis = |f: f64, s: f64| f * 2f64.powf(s / 12.0);
        // Eighths at 150 BPM, E2 power chords, every other note bent up, the last quarter held with vibrato.
        let eighth = 0.2;
        let riff = [0.0, 0.0, 3.0, 0.0, 5.0, 3.0, 7.0];
        // Each take's notes land up to 8 ms and 5 cents apart from the other's.
        let hash = |k: u64| {
            let mut z = k.wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        for (take, (out, late, cents)) in [(&mut l, 0.0, 0.0), (&mut r, 0.012, 7.0)].into_iter().enumerate() {
            let mut phase = [0f64; 3];
            for (i, o) in out.iter_mut().enumerate() {
                let note_at = (i as f64 / rate / eighth) as u64 * 2 + take as u64;
                let t = i as f64 / rate - late - 0.008 * hash(note_at);
                let cents = cents + 10.0 * hash(note_at + 1_000_000) - 5.0;
                if t < 0.0 {
                    continue;
                }
                let bar_t = t % (8.0 * eighth);
                let step = ((bar_t / eighth) as usize).min(7);
                let (note, since) = if step < 6 { (riff[step], bar_t - step as f64 * eighth) } else { (riff[6], bar_t - 6.0 * eighth) };
                let bend = if step % 2 == 1 && step < 6 { -(1.0 - since / 0.08).max(0.0) } else { 0.0 };
                let vib = if step >= 6 { 0.4 * (TAU * 6.0 * since + take as f64).sin() * (since / 0.1).min(1.0) } else { 0.0 };
                let root = semis(82.41, note + bend + vib + cents / 100.0);
                let mut v = 0.0;
                for (h, ratio) in [1.0, 1.5, 2.0].iter().enumerate() {
                    phase[h] = (phase[h] + root * ratio / rate).fract();
                    v += 2.0 * phase[h] - 1.0;
                }
                let pick = (since / 0.005).min(1.0) * (0.75 + 0.25 * (-since / 0.3).exp());
                *o += 0.3 * (3.0 * v * pick).tanh();
            }
        }
        // Kick on every beat, snare on two and four.
        for beat in 0..(secs / 0.4) as usize {
            let at = (beat as f64 * 0.4 * rate) as usize;
            for i in 0..(0.3 * rate) as usize {
                let tau = i as f64 / rate;
                let kick = (TAU * (50.0 * tau + 100.0 * 0.03 * (1.0 - (-tau / 0.03).exp()))).sin() * (-tau / 0.12).exp() * 0.5;
                let snare = if beat % 2 == 1 { (rnd() * 2.0 - 1.0) * (-tau / 0.08).exp() * 0.35 } else { 0.0 };
                if at + i < len {
                    l[at + i] += kick + snare;
                    r[at + i] += kick + snare;
                }
            }
        }
        // Syllables every other four seconds.
        let sings = |t: f64| (t / 4.0) as usize % 2 == 1;
        let dry = voice(secs, rate, &|t| if sings(t) { t } else { (t / 4.0).floor() * 4.0 + 4.0 }, &mut rnd);
        // A room: a dozen echoes 15-150 ms late, different per channel, 6 dB under the voice.
        let wet = 0.5;
        for out in [&mut l, &mut r] {
            let taps: Vec<(usize, f64)> = (0..12).map(|_| { let d = 0.015 + 0.135 * rnd(); ((d * rate) as usize, (-d / 0.08).exp() * if rnd() < 0.5 { -1.0 } else { 1.0 }) }).collect();
            let norm = wet / taps.iter().map(|t| t.1 * t.1).sum::<f64>().sqrt();
            for i in 0..len {
                let mut v = dry[i];
                for &(d, g) in &taps {
                    if i >= d {
                        v += norm * g * dry[i - d];
                    }
                }
                out[i] += v;
            }
        }
        let x = l.iter().zip(&r).flat_map(|(a, b)| [*a as f32 * 0.5, *b as f32 * 0.5]).collect();
        let truth = move |t: f64| {
            let edge = (t - (t / 4.0).round() * 4.0).abs() < 0.5;
            (!edge && t > 4.0).then(|| sings(t))
        };
        (x, truth)
    }

    /// AUC of sung against unsung frames, the curve smoothed over half a second as the sync check reads it.
    fn separation_of(c: &VocalCurve, truth: &dyn Fn(f64) -> Option<bool>) -> f64 {
        let r = (0.25 * c.fps) as usize;
        let (mut pos, mut neg) = (Vec::new(), Vec::new());
        for k in 0..c.level.len() {
            let t = c.t0 as f64 + k as f64 / c.fps as f64;
            let (a, b) = (k.saturating_sub(r), (k + r + 1).min(c.level.len()));
            let v = c.level[a..b].iter().map(|v| *v as f32).sum::<f32>() / (b - a) as f32;
            match truth(t) {
                Some(true) => pos.push(v),
                Some(false) => neg.push(v),
                None => {}
            }
        }
        auc(&pos, &neg)
    }

    #[test]
    fn stereo_middle() {
        // Distorted guitars read as voice in the downmix; panned, they drop out of the middle.
        let (x, truth) = metal_band(40.0);
        let mid: Vec<f32> = x.as_chunks::<2>().0.iter().map(|p| (p[0] + p[1]) * 0.5).collect();
        let mut a = Analyzer::new(44_100, 40_000);
        a.feed(&mid);
        let mono = separation_of(&a.take_features().voice_curve(), &truth);
        a.feed_interleaved(&x, 2, |v| v);
        let stereo = separation_of(&a.take_features().voice_curve(), &truth);
        println!("sung told from unsung: downmix AUC {mono:.3}, the middle {stereo:.3}");
        assert!(mono < 0.7, "the downmix already told them apart ({mono:.3}): the band is no test");
        assert!(stereo > 0.9 && stereo - mono > 0.25, "downmix {mono:.3}, middle {stereo:.3}");

        // Identical channels measure exactly as mono; real stereo changes only the vocal curve.
        let song = Song { sections: vec![(4, FULL), (8, SUNG), (4, FULL)], ..Song::new("sung", Style::Backbeat, 110.0, 2, false) };
        let (x, _) = song.render();
        let mut a = Analyzer::new(song.rate, 0);
        a.feed(&x);
        let mono = a.take_features();
        let twice: Vec<f32> = x.iter().flat_map(|v| [*v, *v]).collect();
        a.feed_interleaved(&twice, 2, |v| v);
        let same = a.take_features();
        assert_eq!(same.voice, mono.voice);
        assert_eq!((&same.onset, &same.power, &same.vocal, &same.chroma, &same.blocks_k), (&mono.onset, &mono.power, &mono.vocal, &mono.chroma, &mono.blocks_k));
        a.feed_interleaved(&x, 1, |v| v);
        assert_eq!(a.take_features().voice, mono.voice, "one channel");

        // The same mid with a side: right 6 dB down, left made up.
        let panned: Vec<f32> = x.iter().flat_map(|v| [*v * 4.0 / 3.0, *v * 2.0 / 3.0]).collect();
        a.feed_interleaved(&panned, 2, |v| v);
        let st = a.take_features();
        let close = |p: &[f32], q: &[f32]| p.len() == q.len() && p.iter().zip(q).all(|(p, q)| (p - q).abs() <= 1e-4 * (1.0 + p.abs().max(q.abs())));
        assert!(close(&st.onset, &mono.onset) && close(&st.power, &mono.power) && close(&st.vocal, &mono.vocal) && close(&st.low_onset, &mono.low_onset));
        assert!(close(&st.blocks_k, &mono.blocks_k), "loudness is measured on the same downmix");
        let (fa, fb) = (super::super::finish("a", &mono).track, super::super::finish("a", &st).track);
        assert!((fa.bpm - fb.bpm).abs() < 1e-6 && (fa.beat_offset_ms - fb.beat_offset_ms).abs() < 1e-3, "{fa:?} {fb:?}");
        assert_eq!((fa.key, fa.intro_end_ms, fa.outro_start_ms, fa.drop_ms), (fb.key, fb.intro_end_ms, fb.outro_start_ms, fb.drop_ms));
        assert_ne!(st.voice, mono.voice, "a sound 6 dB to one side counts less");
    }

    /// A noise that swells into every line (a reversed cymbal, a riser), as on Radiohead's Climbing Up the
    /// Walls: rising for 1.2 s before the voice comes in, dying away under it. The curve must rise with the
    /// voice, not with the swell.
    #[test]
    fn a_swell_is_not_a_voice() {
        let (rate, secs, every) = (44_100.0, 48.0, 6.0);
        let starts: Vec<f64> = (1..7).map(|k| k as f64 * every).collect();
        let line = |t: f64| starts.iter().copied().find(|s| t < s + 3.5).unwrap_or(f64::INFINITY);
        let mut rng = crate::automix::eval::Rng(0x9e37_79b9_7f4a_7c15);
        let mut x = voice(secs, rate, &|t| line(t).max(t), &mut || rng.unit());
        for &s in &starts {
            let from = ((s - 1.2) * rate) as usize;
            for (i, v) in x[from..((s + 1.0) * rate) as usize].iter_mut().enumerate() {
                let t = (from + i) as f64 / rate - s;
                let env = if t < 0.0 { (1.0 + t / 1.2).powi(2) } else { 1.0 - t };
                *v += 0.25 * env * rng.signed();
            }
        }
        let x: Vec<f32> = x.iter().map(|v| *v as f32).collect();
        let mut a = Analyzer::new(rate as u32, 0);
        a.feed(&x);
        let c = a.take_features().voice_curve();
        let mean = |from: f64, to: f64| {
            let k = |t: f64| (((t - c.t0 as f64) * c.fps as f64).max(0.0) as usize).min(c.level.len());
            let v = &c.level[k(from)..k(to)];
            v.iter().map(|v| *v as f64).sum::<f64>() / v.len() as f64
        };
        let at = |from: f64, to: f64| starts.iter().map(|s| mean(s + from, s + to)).sum::<f64>() / starts.len() as f64;
        let (quiet, swell, sung) = (at(-2.0, -1.4), at(-0.8, -0.1), at(0.3, 3.2));
        let share = (swell - quiet) / (sung - quiet);
        println!("quiet {quiet:.1}, swell {swell:.1}, sung {sung:.1}: the swell reads as {share:.2} of the voice");
        assert!(share < 0.5, "the swell reads as {share:.2} of the voice");
    }

    /// The curve tells sung from unsung over every corpus song with a voice (real songs tuned it: nori-lyrics'
    /// `real_songs_are_moved_only_when_right`).
    #[test]
    fn vocal_curve_separates_voices() {
        let songs: Vec<Song> = corpus_all().into_iter().filter(|s| s.sections.iter().any(|x| x.1.voice)).collect();
        let got: Vec<f64> = std::thread::scope(|sc| songs.iter().map(|s| sc.spawn(move || separation(s))).collect::<Vec<_>>().into_iter().map(|h| h.join().unwrap()).collect());
        let mean = got.iter().sum::<f64>() / got.len() as f64;
        assert!(mean >= 0.95 && got.iter().all(|&a| a >= 0.85), "AUC {got:?}");
    }
}
