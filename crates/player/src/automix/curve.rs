//! An end's beats as a tempo curve (docs/research/automix-novel.md, section 1, steps 2 and 3): a beat missed is filled
//! and one added is dropped by a dynamic program over the beats heard, then a Whittaker smoother draws the tempo
//! through them, as stiff as cross-validation allows. A live band's slow drift stays in the curve; its looseness from
//! hit to hit is left over as each beat's residual.

use crate::types::Beat;

/// The fewest beats a curve is drawn through.
const MIN_BEATS: usize = 8;
/// Beats each side whose intervals give a beat its local period.
const PERIOD_SPAN: usize = 8;
/// How far back a beat may follow the one before it, skipping the extra ones between.
const LOOK_BACK: usize = 5;
/// Beats at most filled into one gap.
const MAX_FILLED: usize = 2;
/// What a gap off the local period costs, per squared octave: a gap 10 % long costs about two beats heard.
const TIGHTNESS: f64 = 100.0;
/// A beat filled in costs as much as one heard on an attack.
const FILL_COST: f64 = 1.0;
/// What a beat kept is worth: one on an attack, one without.
const SURE_WORTH: f64 = 1.0;
const UNSURE_WORTH: f64 = 0.3;
/// The smoother's weight of a beat on an attack, one without, and one filled in.
const SURE_WEIGHT: f64 = 1.0;
const UNSURE_WEIGHT: f64 = 0.3;
const FILLED_WEIGHT: f64 = 0.05;
/// Stiffnesses tried, as powers of ten, in quarter steps.
const LAMBDA_POWERS: (i32, i32) = (0, 28);
/// The bisquare's reach, in robust deviations of the beats off their neighbours' middle, and never under this many ms.
const BISQUARE_SPREAD: f64 = 4.685;
const BISQUARE_MIN_MS: f64 = 15.0;
/// What makes a curve one the next song can follow: enough beats, few filled in, most on attacks, the hits within
/// this many ms of the curve (robust deviation), and the tempo changing at most this share over any four bars.
const HOLD_MIN_BEATS: usize = 16;
const HOLD_MAX_FILLED: f64 = 0.2;
const HOLD_MIN_SURE: f64 = 0.5;
const HOLD_SCATTER_MS: f64 = 15.0;
const HOLD_TEMPO_CHANGE: f64 = 0.06;
/// A song slows to its end when its last bar is this much slower than the bars before its last four.
const SLOWS_BY: f64 = 0.03;
/// Where slowing starts: the first beat this much slower than before.
const SLOWING: f64 = 0.01;

/// One beat on the curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    /// Where it was heard, ms; for a beat filled in, where the beats around it put it.
    pub heard_ms: f64,
    /// Where the curve puts it, ms.
    pub smooth_ms: f64,
    /// It sits on an attack heard.
    pub sure: bool,
    /// Filled into a gap where a beat was missed.
    pub filled: bool,
    pub downbeat: bool,
    /// Too far off the curve to steer it (a push, a fill, a misplaced peak).
    pub outlier: bool,
}

/// An end's cleaned beats and the curve through them.
#[derive(Clone, Debug)]
pub(crate) struct Curve {
    pub points: Vec<Point>,
}

impl Curve {
    /// The curve through `beats` (in order); None for fewer than [`MIN_BEATS`].
    pub fn of(beats: &[Beat]) -> Option<Curve> {
        if beats.len() < MIN_BEATS {
            return None;
        }
        let mut points = clean(beats);
        if points.len() < MIN_BEATS {
            return None;
        }
        smooth(&mut points);
        Some(Curve { points })
    }

    /// Beat interval `k`, ms, on the curve.
    fn ibi(&self, k: usize) -> f64 {
        self.points[k + 1].smooth_ms - self.points[k].smooth_ms
    }

    /// The median beat interval over the first `n` beats (`first`), else the last, ms.
    pub fn period(&self, first: bool, n: usize) -> f64 {
        let m = self.points.len() - 1;
        let n = n.clamp(1, m);
        let range = if first { 0..n } else { m - n..m };
        let mut ibis: Vec<f64> = range.map(|k| self.ibi(k)).collect();
        ibis.sort_by(f64::total_cmp);
        ibis[ibis.len() / 2]
    }

    /// The most the tempo changes over any `span` beats, as a share.
    pub fn tempo_change(&self, span: usize) -> f64 {
        let m = self.points.len() - 1;
        let mut most: f64 = 0.0;
        for i in 0..m {
            for j in i + 1..m.min(i + span + 1) {
                most = most.max((self.ibi(j) / self.ibi(i)).ln().abs());
            }
        }
        most.exp() - 1.0
    }

    /// The robust deviation of the beats heard on attacks from the curve, ms.
    pub fn scatter_ms(&self) -> f64 {
        let r: Vec<f64> = self.points.iter().filter(|p| p.sure && !p.filled && !p.outlier).map(|p| p.heard_ms - p.smooth_ms).collect();
        robust_deviation(&r)
    }

    /// Whether the next song can follow these beats one by one: see [`HOLD_TEMPO_CHANGE`] and the rest.
    pub fn holds(&self, meter: usize) -> bool {
        let n = self.points.len();
        let filled = self.points.iter().filter(|p| p.filled).count();
        let sure = self.points.iter().filter(|p| p.sure && !p.filled).count();
        n >= HOLD_MIN_BEATS
            && (filled as f64) <= HOLD_MAX_FILLED * n as f64
            && (sure as f64) >= HOLD_MIN_SURE * (n - filled) as f64
            && self.scatter_ms() <= HOLD_SCATTER_MS
            && self.tempo_change(4 * meter) <= HOLD_TEMPO_CHANGE
    }

    /// Where an ending starts to slow (a ritardando), ms on the curve: its last bar [`SLOWS_BY`] slower than the bars
    /// before its last four, from the first beat [`SLOWING`] slower than them.
    pub fn slows_from(&self, meter: usize) -> Option<f64> {
        let m = self.points.len() - 1;
        let (last, before) = (meter, 4 * meter);
        if m < before + 2 * meter {
            return None;
        }
        let mut base: Vec<f64> = (m.saturating_sub(before + 8 * meter)..m - before).map(|k| self.ibi(k)).collect();
        base.sort_by(f64::total_cmp);
        let base = base[base.len() / 2];
        let end = (m - last..m).map(|k| self.ibi(k)).sum::<f64>() / last as f64;
        if end < base * (1.0 + SLOWS_BY) {
            return None;
        }
        let from = (0..m).rev().take_while(|&k| self.ibi(k) > base * (1.0 + SLOWING)).last()?;
        Some(self.points[from].smooth_ms)
    }

    /// A constant grid for the stretch next to one end (`first`, else the last), `n` beats long: (bpm, a beat's place
    /// ms, the phase of the downbeats counted from it in `meter`).
    pub fn grid(&self, first: bool, n: usize, meter: usize) -> (f64, f64, i32) {
        let period = self.period(first, n);
        let k = if first { 0 } else { self.points.len() - 1 };
        let at = self.points[k].smooth_ms;
        // The downbeat nearest the anchor; the bars counted from it.
        let down = (0..self.points.len()).filter(|&j| self.points[j].downbeat).min_by_key(|&j| j.abs_diff(k));
        let phase = down.map_or(0, |j| (j as i64 - k as i64).rem_euclid(meter as i64) as i32);
        (60_000.0 / period, at, phase)
    }
}

/// `beats` cleaned as a curve's are: a beat missed filled in (not sure), one added dropped. Too few to clean are kept.
pub(crate) fn cleaned(beats: &[Beat]) -> Vec<Beat> {
    if beats.len() < MIN_BEATS {
        return beats.to_vec();
    }
    clean(beats).iter().map(|p| Beat { at_ms: p.heard_ms.round() as i64, downbeat: p.downbeat, sure: p.sure && !p.filled }).collect()
}

/// The longest run of `beats` with no gap past [`RUN_GAP`] of their median gap: where they were measured beat by
/// beat, not a stray beat a stretch of unbeaten music away from the rest.
pub(crate) fn steady_run(beats: &[Beat]) -> &[Beat] {
    if beats.len() < 2 {
        return beats;
    }
    let mut gaps: Vec<i64> = beats.windows(2).map(|w| w[1].at_ms - w[0].at_ms).collect();
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2] as f64;
    let (mut best, mut start) = ((0, 1), 0);
    for i in 1..beats.len() {
        if (beats[i].at_ms - beats[i - 1].at_ms) as f64 > RUN_GAP * median {
            start = i;
        }
        if i + 1 - start > best.1 - best.0 {
            best = (start, i + 1);
        }
    }
    &beats[best.0..best.1]
}

/// A gap past this many of a run's usual beats breaks it.
const RUN_GAP: f64 = 1.6;

/// The local period at each beat: the median gap within [`PERIOD_SPAN`] beats.
fn periods(t: &[f64]) -> Vec<f64> {
    let gaps: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    (0..t.len())
        .map(|j| {
            let mut near: Vec<f64> = gaps[j.saturating_sub(PERIOD_SPAN)..(j + PERIOD_SPAN).min(gaps.len())].iter().copied().filter(|g| *g > 0.0).collect();
            near.sort_by(f64::total_cmp);
            near.get(near.len() / 2).copied().unwrap_or(0.0)
        })
        .collect()
}

/// The beats kept, missed ones filled in (Ellis's dynamic program, made local): each kept beat follows one of the
/// [`LOOK_BACK`] before it, with up to [`MAX_FILLED`] filled between, at a cost for each gap off the local period.
fn clean(beats: &[Beat]) -> Vec<Point> {
    let n = beats.len();
    let t: Vec<f64> = beats.iter().map(|b| b.at_ms as f64).collect();
    let period = periods(&t);
    let worth = |b: &Beat| if b.sure { SURE_WORTH } else { UNSURE_WORTH };
    // Best score of a path ending on beat j, the beat before it and how many filled between.
    let mut score = vec![f64::NEG_INFINITY; n];
    let mut from: Vec<Option<(usize, usize)>> = vec![None; n];
    for j in 0..n {
        if j < LOOK_BACK {
            score[j] = worth(&beats[j]);
        }
        let p = period[j];
        if p <= 0.0 {
            continue;
        }
        for i in j.saturating_sub(LOOK_BACK)..j {
            let gap = t[j] - t[i];
            if gap <= 0.0 || !score[i].is_finite() {
                continue;
            }
            for filled in 0..=MAX_FILLED {
                let off = (gap / ((filled + 1) as f64 * p)).log2();
                let s = score[i] + worth(&beats[j]) - TIGHTNESS * off * off - FILL_COST * filled as f64;
                if s > score[j] {
                    (score[j], from[j]) = (s, Some((i, filled)));
                }
            }
        }
    }
    let Some(mut j) = (n.saturating_sub(LOOK_BACK)..n).max_by(|&x, &y| score[x].total_cmp(&score[y])) else { return Vec::new() };
    let point = |b: &Beat| Point { heard_ms: b.at_ms as f64, smooth_ms: b.at_ms as f64, sure: b.sure, filled: false, downbeat: b.downbeat, outlier: false };
    let mut out = vec![point(&beats[j])];
    while let Some((i, filled)) = from[j] {
        for f in (1..=filled).rev() {
            let at = t[i] + (t[j] - t[i]) * f as f64 / (filled + 1) as f64;
            out.push(Point { heard_ms: at, smooth_ms: at, sure: false, filled: true, downbeat: false, outlier: false });
        }
        out.push(point(&beats[i]));
        j = i;
    }
    out.reverse();
    out
}

/// The robust standard deviation (1.4826 times the median absolute deviation) of `r`.
fn robust_deviation(r: &[f64]) -> f64 {
    if r.is_empty() {
        return 0.0;
    }
    let mut v = r.to_vec();
    v.sort_by(f64::total_cmp);
    let med = v[v.len() / 2];
    let mut dev: Vec<f64> = v.iter().map(|x| (x - med).abs()).collect();
    dev.sort_by(f64::total_cmp);
    1.4826 * dev[dev.len() / 2]
}

/// How far each beat sits off the middle of its neighbours, ms: a beat pushed or pulled on its own stands out, while a
/// tempo changing beat by beat moves it little.
fn off_middle(t: &[f64]) -> Vec<f64> {
    (0..t.len()).map(|k| if k == 0 || k + 1 == t.len() { 0.0 } else { t[k] - (t[k - 1] + t[k + 1]) / 2.0 }).collect()
}

/// The Whittaker smoother (Eilers 2003) through `points`' heard times: beats off the middle of their neighbours by far
/// more than the band's usual looseness weighed down (Tukey's bisquare) and marked outliers, then the stiffness of the
/// lowest generalized cross-validation score.
fn smooth(points: &mut [Point]) {
    let t: Vec<f64> = points.iter().map(|p| p.heard_ms).collect();
    let off = off_middle(&t);
    let reach = (BISQUARE_SPREAD * robust_deviation(&off)).max(BISQUARE_MIN_MS);
    let w: Vec<f64> = points
        .iter()
        .zip(&off)
        .map(|(p, o)| (if p.filled { FILLED_WEIGHT } else if p.sure { SURE_WEIGHT } else { UNSURE_WEIGHT }) * (1.0 - (o / reach).powi(2)).max(0.0).powi(2))
        .collect();
    let n = t.len() as f64;
    let mut best: Option<(f64, Vec<f64>)> = None;
    for q in LAMBDA_POWERS.0..=LAMBDA_POWERS.1 {
        let (y, trace) = whittaker(&t, &w, 10f64.powf(q as f64 / 4.0));
        let rss: f64 = t.iter().zip(&y).zip(&w).map(|((a, b), w)| w * (a - b).powi(2)).sum();
        let gcv = n * rss / (n - trace).max(1e-9).powi(2);
        if best.as_ref().is_none_or(|b| gcv < b.0) {
            best = Some((gcv, y));
        }
    }
    let Some((_, y)) = best else { return };
    for ((p, y), w) in points.iter_mut().zip(y).zip(w) {
        p.smooth_ms = y;
        p.outlier = w == 0.0 && !p.filled;
    }
}

/// Solves `(W + lambda D'D) y = W t`, D the second difference: the fit, and the trace of its hat matrix
/// (`sum w_k (A^-1)_kk`, from the band of the inverse, Takahashi's recursion).
fn whittaker(t: &[f64], w: &[f64], lambda: f64) -> (Vec<f64>, f64) {
    let n = t.len();
    // The pentadiagonal A by bands: a0 the diagonal, a1 and a2 the first and second off it.
    let mut a0 = vec![0.0; n];
    let mut a1 = vec![0.0; n];
    let mut a2 = vec![0.0; n];
    for k in 0..n.saturating_sub(2) {
        // Row k of D is (1, -2, 1) at k, k + 1, k + 2.
        let d = [1.0, -2.0, 1.0];
        for i in 0..3 {
            a0[k + i] += lambda * d[i] * d[i];
            if i < 2 {
                a1[k + i] += lambda * d[i] * d[i + 1];
            }
        }
        a2[k] += lambda * d[0] * d[2];
    }
    for k in 0..n {
        a0[k] += w[k];
    }
    // LDL': L unit lower with bands l1, l2.
    let (mut d, mut l1, mut l2) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for k in 0..n {
        if k >= 2 {
            l2[k - 2] = a2[k - 2] / d[k - 2];
        }
        if k >= 1 {
            let back = if k >= 2 { l2[k - 2] * l1[k - 2] * d[k - 2] } else { 0.0 };
            l1[k - 1] = (a1[k - 1] - back) / d[k - 1];
        }
        let mut dk = a0[k];
        if k >= 1 {
            dk -= l1[k - 1] * l1[k - 1] * d[k - 1];
        }
        if k >= 2 {
            dk -= l2[k - 2] * l2[k - 2] * d[k - 2];
        }
        d[k] = dk.max(1e-12);
    }
    // L z = W t, then D L' y = z.
    let mut y: Vec<f64> = t.iter().zip(w).map(|(t, w)| t * w).collect();
    for k in 0..n {
        if k >= 1 {
            y[k] -= l1[k - 1] * y[k - 1];
        }
        if k >= 2 {
            y[k] -= l2[k - 2] * y[k - 2];
        }
    }
    for k in (0..n).rev() {
        y[k] /= d[k];
        if k + 1 < n {
            y[k] -= l1[k] * y[k + 1];
        }
        if k + 2 < n {
            y[k] -= l2[k] * y[k + 2];
        }
    }
    // The inverse's band from the bottom: s0 its diagonal, s1 and s2 the entries one and two off it.
    let (mut s0, mut s1, mut s2) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for k in (0..n).rev() {
        let (l1k, l2k) = (if k + 1 < n { l1[k] } else { 0.0 }, if k + 2 < n { l2[k] } else { 0.0 });
        let (s11, s12, s22) = (
            if k + 1 < n { s0[k + 1] } else { 0.0 },
            if k + 2 < n { s1[k + 1] } else { 0.0 },
            if k + 2 < n { s0[k + 2] } else { 0.0 },
        );
        if k + 1 < n {
            s1[k] = -l1k * s11 - l2k * s12;
        }
        if k + 2 < n {
            s2[k] = -l1k * s12 - l2k * s22;
        }
        s0[k] = 1.0 / d[k] - l1k * s1[k] - l2k * s2[k];
    }
    let trace = s0.iter().zip(w).map(|(s, w)| s * w).sum();
    (y, trace)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beats(times: impl IntoIterator<Item = f64>) -> Vec<Beat> {
        times.into_iter().enumerate().map(|(i, t)| Beat { at_ms: t.round() as i64, downbeat: i % 4 == 0, sure: true }).collect()
    }

    /// The smoother solves its system: the fit against a dense solve, and the trace against the inverse's diagonal.
    #[test]
    fn whittaker_solves_its_system() {
        let n = 12;
        let t: Vec<f64> = (0..n).map(|i| 500.0 * i as f64 + [0.0, 7.0, -4.0][i % 3]).collect();
        let w: Vec<f64> = (0..n).map(|i| [1.0, 0.3, 0.05][i % 3]).collect();
        let lambda = 3.0;
        let mut a = vec![vec![0.0; n]; n];
        for k in 0..n - 2 {
            let d = [1.0, -2.0, 1.0];
            for i in 0..3 {
                for j in 0..3 {
                    a[k + i][k + j] += lambda * d[i] * d[j];
                }
            }
        }
        for k in 0..n {
            a[k][k] += w[k];
        }
        // Gauss-Jordan for the inverse.
        let mut inv: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
        let mut m = a.clone();
        for c in 0..n {
            let p = m[c][c];
            for j in 0..n {
                m[c][j] /= p;
                inv[c][j] /= p;
            }
            for r in 0..n {
                if r != c {
                    let f = m[r][c];
                    for j in 0..n {
                        m[r][j] -= f * m[c][j];
                        inv[r][j] -= f * inv[c][j];
                    }
                }
            }
        }
        let (y, trace) = whittaker(&t, &w, lambda);
        for i in 0..n {
            let want: f64 = (0..n).map(|j| inv[i][j] * w[j] * t[j]).sum();
            assert!((y[i] - want).abs() < 1e-6, "y[{i}] {} against {want}", y[i]);
        }
        let want: f64 = (0..n).map(|k| inv[k][k] * w[k]).sum();
        assert!((trace - want).abs() < 1e-9, "trace {trace} against {want}");
    }

    /// A beat missed is filled in, one added between two is dropped, and a steady band's curve holds.
    #[test]
    fn missed_and_added_beats_are_mended() {
        let mut times: Vec<f64> = (0..40).map(|i| 1_000.0 + 500.0 * i as f64).collect();
        times.remove(17);
        times.push(1_000.0 + 500.0 * 29.5 + 40.0);
        times.sort_by(f64::total_cmp);
        let c = Curve::of(&beats(times)).expect("a curve");
        assert_eq!(c.points.len(), 40, "{:?}", c.points.iter().map(|p| p.heard_ms).collect::<Vec<_>>());
        assert!(c.points[17].filled && (c.points[17].heard_ms - 9_500.0).abs() < 1.0);
        assert!(c.points.iter().all(|p| (p.heard_ms - 1_000.0).rem_euclid(500.0) < 1.0));
        assert!(c.holds(4) && (c.period(false, 16) - 500.0).abs() < 0.5);
    }

    /// A band drifting slowly around its tempo with loose hits: the curve follows the drift, the looseness is left
    /// over, and it holds; at twice the drift and a quick swing it does not.
    #[test]
    fn a_drifting_band_is_drawn_through() {
        let drift = |amp: f64, cycle: f64| {
            (0..120).map(move |k| {
                let k = k as f64;
                1_000.0 + 500.0 * k + amp * (std::f64::consts::TAU * k / cycle).sin() + 8.0 * ((k * 12.9898).sin() * 43_758.545_3).fract()
            })
        };
        let c = Curve::of(&beats(drift(60.0, 60.0))).expect("a curve");
        let off = c.points.iter().enumerate().map(|(k, p)| (p.smooth_ms - (1_000.0 + 500.0 * k as f64 + 60.0 * (std::f64::consts::TAU * k as f64 / 60.0).sin())).abs()).fold(0.0, f64::max);
        assert!(off < 10.0, "the curve is {off:.1} ms off the drift");
        assert!(c.holds(4), "scatter {:.1} ms, change {:.3}", c.scatter_ms(), c.tempo_change(16));
        let wild = Curve::of(&beats(drift(150.0, 16.0))).expect("a curve");
        assert!(!wild.holds(4), "change {:.3}", wild.tempo_change(16));
    }

    /// An ending slowing over its last bars is found where it starts slowing; a steady one is not.
    #[test]
    fn a_slowing_ending_is_found() {
        let mut t = 0.0;
        let mut times = Vec::new();
        for k in 0..64 {
            times.push(t);
            t += if k < 52 { 500.0 } else { 500.0 * (1.0 + 0.012 * (k - 51) as f64) };
        }
        let c = Curve::of(&beats(times.clone())).expect("a curve");
        let from = c.slows_from(4).expect("it slows");
        assert!((from - times[52]).abs() <= 1_000.0, "slows from {from:.0}, not {:.0}", times[52]);
        let steady = Curve::of(&beats((0..64).map(|k| 500.0 * k as f64))).expect("a curve");
        assert_eq!(steady.slows_from(4), None);
    }
}
