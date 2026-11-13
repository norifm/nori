//! The transition mixer: the outgoing track's tail and the incoming track's start (already time-stretched) in,
//! one stream out, mixed as a `TransitionPlan` says: gain curves and loudness trim, bass swap (4th-order
//! Linkwitz-Riley high-pass crossfaded in on both decks), low- and high-pass sweeps and echo on the outgoing deck,
//! vocal duck on the incoming deck.
//!
//! Past the end of the transition the output is the incoming stream untouched. Mixing allocates nothing.

use crate::pcm::Encoding;
use crate::types::{FadeCurve, TransitionKind, TransitionPlan};
use std::f64::consts::PI;

const MAX_CHANNELS: usize = 8;
/// The loudness trim lets go no faster than this, ms per dB (0.67 dB per 100 ms).
pub(crate) const TRIM_GLIDE_MS_PER_DB: f64 = 150.0;
/// Frames between sweep coefficient updates.
const SWEEP_STEP: u64 = 16;
/// A sweep's filter is faded in over this long so switching it on does not click.
const SWEEP_ENTRY_MS: f64 = 50.0;
/// Q of the vocal duck's band-pass: 3 dB down at about 300 Hz and 3.3 kHz around 1 kHz.
const VOX_Q: f64 = 0.35;

/// Where in a transition the incoming song becomes the louder of the two, ms from its start, in 10 ms steps
/// (from the gain curves only); the whole transition when it never does.
pub(crate) fn crossover_ms(p: &TransitionPlan) -> i64 {
    let len = p.duration_ms.max(0) as f64;
    let out_fade = span_ms(p.out_fade_start_ms, p.out_fade_end_ms, len);
    let in_fade = span_ms(p.in_fade_start_ms, p.in_fade_end_ms, len);
    let out_gain = db_to_gain((p.out_gain_db as f64).clamp(-24.0, 12.0));
    let gap = trim(len, p.in_gain_db as f64);
    let progress = |(start, end): (f64, f64), t: f64| if t <= start { 0.0 } else if t >= end { 1.0 } else { (t - start) / (end - start) };
    let mut t = 0.0;
    while t < len {
        let (trim_out, trim_in) = trims(gap, progress((0.0, len), t));
        let g_out = fade(p.fade_curve, progress(out_fade, t), true) * out_gain * db_to_gain(trim_out);
        let g_in = fade(p.fade_curve, progress(in_fade, t), false) * db_to_gain(trim_in);
        if g_in >= g_out {
            return t as i64;
        }
        t += 10.0;
    }
    len as i64
}

/// A fade window in ms clipped to `len`; the whole transition when it is not a window.
fn span_ms(a: i64, b: i64, len: f64) -> (f64, f64) {
    if a < 0 || b < a {
        (0.0, len)
    } else {
        ((a as f64).min(len), (b as f64).min(len))
    }
}

/// The loudness gap a transition of `len_ms` can carry, dB: the incoming song starts turned by it to the
/// outgoing one's level, and the two meet as the level glides to the incoming song's own over the whole
/// transition, no faster than [`TRIM_GLIDE_MS_PER_DB`]. At its end the incoming song is at its own level.
fn trim(len_ms: f64, db: f64) -> f64 {
    let most = len_ms.max(0.0) / TRIM_GLIDE_MS_PER_DB;
    db.clamp(-12.0, 12.0).clamp(-most, most)
}

/// The outgoing and incoming songs' turns, dB, `progress` (0..1) through a transition carrying `gap` dB.
fn trims(gap: f64, progress: f64) -> (f64, f64) {
    (-gap * progress, gap * (1.0 - progress))
}

fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[inline]
fn fade(curve: FadeCurve, x: f64, down: bool) -> f64 {
    let x = if down { 1.0 - x } else { x };
    match curve {
        FadeCurve::EqualPower => (x * PI / 2.0).sin(),
        FadeCurve::Linear => x,
        FadeCurve::SineSquared => (x * PI / 2.0).sin().powi(2),
    }
}

/// 0 -> 1 raised cosine.
#[inline]
fn raised_cos(x: f64) -> f64 {
    0.5 - 0.5 * (x * PI).cos()
}

/// A shelf at one frequency (RBJ, slope 1), high or low, its gain set per frame.
#[derive(Clone, Copy, Default)]
struct Shelf {
    sin: f64,
    cos: f64,
}

impl Shelf {
    fn at(rate: f64, hz: f64) -> Self {
        let (sin, cos) = (2.0 * PI * hz.clamp(10.0, rate * 0.49) / rate).sin_cos();
        Shelf { sin, cos }
    }

    /// Lows untouched, highs times `gain`; at a gain of 1 it passes its input exactly.
    fn high(&self, gain: f64) -> Coef {
        let a = gain.sqrt();
        let (c, beta) = (self.cos, 2.0 * a.sqrt() * self.sin / std::f64::consts::SQRT_2);
        let a0 = (a + 1.0) - (a - 1.0) * c + beta;
        Coef {
            b0: a * ((a + 1.0) + (a - 1.0) * c + beta) / a0,
            b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * c) / a0,
            b2: a * ((a + 1.0) + (a - 1.0) * c - beta) / a0,
            a1: 2.0 * ((a - 1.0) - (a + 1.0) * c) / a0,
            a2: ((a + 1.0) - (a - 1.0) * c - beta) / a0,
        }
    }

    /// Lows times `gain`, highs untouched; at a gain of 1 it passes its input exactly.
    fn low(&self, gain: f64) -> Coef {
        let a = gain.sqrt();
        let (c, beta) = (self.cos, 2.0 * a.sqrt() * self.sin / std::f64::consts::SQRT_2);
        let a0 = (a + 1.0) + (a - 1.0) * c + beta;
        Coef {
            b0: a * ((a + 1.0) - (a - 1.0) * c + beta) / a0,
            b1: 2.0 * a * ((a - 1.0) - (a + 1.0) * c) / a0,
            b2: a * ((a + 1.0) - (a - 1.0) * c - beta) / a0,
            a1: -2.0 * ((a - 1.0) + (a + 1.0) * c) / a0,
            a2: ((a + 1.0) + (a - 1.0) * c - beta) / a0,
        }
    }
}

/// A deck's lows during a bass swap: two low shelves at half the cut, together this far down when the deck has
/// none (as far as a 4th-order high-pass at the cut takes them, and no further above it), dB.
const SWAP_KILL_DB: f64 = -48.0;

#[derive(Clone, Copy, Debug, Default)]
struct Coef {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coef {
    /// RBJ band-pass with 0 dB at the centre: `x - d * band_pass(x)` is a peaking cut of `1 - d` there.
    fn band_pass(rate: f64, hz: f64, q: f64) -> Self {
        let w = 2.0 * PI * hz.clamp(10.0, rate * 0.45) / rate;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        Coef { b0: alpha / a0, b1: 0.0, b2: -alpha / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0 }
    }

    /// Butterworth RBJ low- or high-pass.
    fn pass(rate: f64, hz: f64, low: bool) -> Self {
        let w = 2.0 * PI * hz.clamp(10.0, rate * 0.49) / rate;
        let (s, c) = w.sin_cos();
        let alpha = s / std::f64::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1) = if low { ((1.0 - c) / 2.0, 1.0 - c) } else { ((1.0 + c) / 2.0, -(1.0 + c)) };
        Coef { b0: b0 / a0, b1: b1 / a0, b2: b0 / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0 }
    }

    /// `self` moved toward `to` by `t` (0..1), coefficient by coefficient.
    #[inline]
    fn lerp(&self, to: &Coef, t: f64) -> Coef {
        let l = |a: f64, b: f64| a + (b - a) * t;
        Coef { b0: l(self.b0, to.b0), b1: l(self.b1, to.b1), b2: l(self.b2, to.b2), a1: l(self.a1, to.a1), a2: l(self.a2, to.a2) }
    }

    #[inline]
    fn run(&self, s: &mut [f64; 2], x: f64) -> f64 {
        let y = self.b0 * x + s[0];
        s[0] = self.b1 * x - self.a1 * y + s[1];
        s[1] = self.b2 * x - self.a2 * y;
        y
    }

    /// Two cascaded sections on channel `c`.
    #[inline]
    fn run2(&self, s: &mut [[[f64; 2]; MAX_CHANNELS]; 2], c: usize, x: f64) -> f64 {
        let y = self.run(&mut s[0][c], x);
        self.run(&mut s[1][c], y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Span {
    start: u64,
    end: u64,
}

impl Span {
    /// 0 before, 1 after, linear in between.
    #[inline]
    fn progress(&self, p: u64) -> f64 {
        if p <= self.start {
            0.0
        } else if p >= self.end {
            1.0
        } else {
            (p - self.start) as f64 / (self.end - self.start) as f64
        }
    }
}

/// A low- or high-pass sweep on the outgoing deck, two cascaded sections per channel.
#[derive(Clone, Copy)]
struct SweepFilter {
    span: Span,
    from: f64,
    to: f64,
    low: bool,
    coef: Coef,
    state: [[[f64; 2]; MAX_CHANNELS]; 2],
    /// Where the filter's fade-in is measured from: the sweep's start, or a seek that landed inside it.
    entry_from: u64,
}

impl SweepFilter {
    fn new(rate: f64, span: Span, from: f64, to: f64, low: bool) -> Self {
        let (from, to) = (from.min(rate * 0.45), to.min(rate * 0.45));
        SweepFilter { span, from, to, low, coef: Coef::pass(rate, from, low), state: [[[0.0; 2]; MAX_CHANNELS]; 2], entry_from: span.start }
    }

    fn retune(&mut self, rate: f64, p: u64) {
        self.coef = Coef::pass(rate, self.from * (self.to / self.from).powf(self.span.progress(p)), self.low);
    }

    /// How much of the filtered signal is heard at frame `p`; retunes every [`SWEEP_STEP`] frames.
    #[inline]
    fn wet(&mut self, rate: f64, entry: u64, p: u64) -> f64 {
        if p < self.span.start {
            return 0.0;
        }
        if (p - self.span.start).is_multiple_of(SWEEP_STEP) {
            self.retune(rate, p);
        }
        ((p - self.entry_from.min(p)) as f64 / entry as f64).min(1.0)
    }

    #[inline]
    fn run(&mut self, c: usize, x: f64, wet: f64) -> f64 {
        let y = self.coef.run(&mut self.state[0][c], x);
        let y = self.coef.run(&mut self.state[1][c], y);
        x * (1.0 - wet) + y * wet
    }
}

/// A sample type the mixer reads and writes.
pub trait Sample: Copy {
    fn to_f64(self) -> f64;
    fn from_f64(v: f64) -> Self;
}

impl Sample for f32 {
    #[inline]
    fn to_f64(self) -> f64 {
        self as f64
    }
    #[inline]
    fn from_f64(v: f64) -> Self {
        v as f32
    }
}

impl Sample for i16 {
    #[inline]
    fn to_f64(self) -> f64 {
        self as f64
    }
    #[inline]
    fn from_f64(v: f64) -> Self {
        v.round().clamp(-32768.0, 32767.0) as i16
    }
}

/// Frames the fade curves are interpolated over.
const FADE_RUN: u64 = 32;

/// The gains at the two ends of a run of frames, linear in between. A run never crosses the start or end
/// of a fade, the bass swap, the duck's release or the echo's, where a curve bends or steps.
#[derive(Clone, Copy, Debug, Default)]
struct FadeRun {
    start: u64,
    end: u64,
    from: Fades,
    to: Fades,
}

/// What changes over a transition, at one frame: the decks' gains (fade, level and gap trim), what both
/// decks together are scaled by to one song's power, and the effects' amounts.
#[derive(Clone, Copy, Debug, Default)]
struct Fades {
    out: f64,
    inc: f64,
    held: f64,
    /// The bass swap: each deck's low shelf, one of its two sections.
    low_out: Coef,
    low_in: Coef,
    /// The shelf holding the swapped highs to one song's power.
    shelf: Coef,
    /// The incoming highs' shelf.
    highs_in: Coef,
    /// How much of the incoming voice band is held down.
    duck: f64,
    /// How much of the echo's repeats is still heard.
    echo: f64,
}

impl FadeRun {
    /// How far frame `p` is through the run, 0..1.
    #[inline]
    fn t(&self, p: u64) -> f64 {
        (p - self.start) as f64 / (self.end - self.start) as f64
    }

    /// One of the gains, `t` through the run; only the ones a frame uses are worked out.
    #[inline]
    fn get(&self, t: f64, gain: impl Fn(&Fades) -> f64) -> f64 {
        let a = gain(&self.from);
        a + (gain(&self.to) - a) * t
    }
}

pub struct Mixer {
    rate: f64,
    pub ch: usize,
    pos: u64,
    len: u64,
    curve: FadeCurve,
    out_fade: Span,
    in_fade: Span,
    out_gain: f64,
    /// The loudness gap the two songs meet across ([`trim`]).
    gap_db: f64,
    swap: Option<Span>,
    /// The kicks are beat-matched, so the lows of both decks land together and add up.
    matched: bool,
    low_shelf: Shelf,
    /// Two cascaded sections per deck per channel: [deck][section][channel].
    swap_state: [[[[f64; 2]; MAX_CHANNELS]; 2]; 2],
    /// Holds the highs both decks play at the swap's cut, per deck per channel.
    hold_shelf: Shelf,
    hold_state: [[[f64; 2]; MAX_CHANNELS]; 2],
    /// The incoming highs' return, and how far down they are held before it.
    highs_in: Option<(Span, f64)>,
    highs_shelf: Shelf,
    highs_state: [[f64; 2]; MAX_CHANNELS],
    high_pass: Option<SweepFilter>,
    low_pass: Option<SweepFilter>,
    sweep_entry: u64,
    /// The duck's release span and how much of the band-pass is subtracted before it.
    duck: Option<(Span, f64)>,
    duck_coef: Coef,
    duck_state: [[f64; 2]; MAX_CHANNELS],
    /// Delay frames, feedback, wet gain.
    echo: Option<(usize, f64, f64)>,
    /// Repeats still ringing fade out over the transition's last delay.
    echo_out: Span,
    echo_buf: Vec<f64>,
    echo_pos: usize,
    /// The run of frames the fades are interpolated over; empty after a (re)start.
    fades: FadeRun,
}

impl Mixer {
    pub fn new(rate: u32, channels: usize) -> Self {
        Mixer {
            rate: rate.max(1) as f64,
            ch: channels.clamp(1, MAX_CHANNELS),
            pos: 0,
            len: 0,
            curve: FadeCurve::EqualPower,
            out_fade: Span::default(),
            in_fade: Span::default(),
            out_gain: 1.0,
            gap_db: 0.0,
            swap: None,
            matched: false,
            low_shelf: Shelf::default(),
            swap_state: [[[[0.0; 2]; MAX_CHANNELS]; 2]; 2],
            hold_shelf: Shelf::default(),
            hold_state: [[[0.0; 2]; MAX_CHANNELS]; 2],
            highs_in: None,
            highs_shelf: Shelf::default(),
            highs_state: [[0.0; 2]; MAX_CHANNELS],
            high_pass: None,
            low_pass: None,
            sweep_entry: 1,
            duck: None,
            duck_coef: Coef::default(),
            duck_state: [[0.0; 2]; MAX_CHANNELS],
            echo: None,
            echo_out: Span::default(),
            echo_buf: Vec::new(),
            echo_pos: 0,
            fades: FadeRun::default(),
        }
    }

    /// Takes a plan and restarts the clock.
    pub fn configure(&mut self, p: &TransitionPlan) {
        let rate = self.rate;
        let frames = |ms: f64| (ms.max(0.0) * rate / 1000.0).round() as u64;
        let len = frames(p.duration_ms as f64);
        let span = |a: i64, b: i64| {
            let (a, b) = span_ms(a, b, p.duration_ms.max(0) as f64);
            Span { start: frames(a), end: frames(b) }
        };
        self.pos = 0;
        self.fades = FadeRun::default();
        self.len = len;
        self.curve = p.fade_curve;
        self.out_fade = span(p.out_fade_start_ms, p.out_fade_end_ms);
        self.in_fade = span(p.in_fade_start_ms, p.in_fade_end_ms);
        self.out_gain = db_to_gain((p.out_gain_db as f64).clamp(-24.0, 12.0));
        self.gap_db = trim(p.duration_ms as f64, p.in_gain_db as f64);
        self.swap = p.bass_swap.map(|s| {
            let start = frames(s.at_ms as f64);
            Span { start: start.min(len), end: (start + frames((s.len_ms as f64).max(1.0))).min(len.max(1)) }
        });
        self.matched = p.kind == TransitionKind::BeatMatched;
        let cut = p.bass_swap.map_or(180.0, |s| s.cut_hz as f64);
        (self.low_shelf, self.hold_shelf) = (Shelf::at(rate, cut / 2.0), Shelf::at(rate, cut));
        self.highs_in = p.highs_in.filter(|h| h.db < 0.0).map(|h| {
            let start = frames(h.at_ms as f64).min(len);
            self.highs_shelf = Shelf::at(rate, h.hz as f64);
            (Span { start, end: (start + frames((h.len_ms as f64).max(1.0))).min(len.max(1)) }, db_to_gain((h.db as f64).clamp(-30.0, 0.0)))
        });
        let sweep = |s: crate::types::Sweep, low| SweepFilter::new(rate, span(s.start_ms, s.end_ms), s.from_hz as f64, s.to_hz as f64, low);
        self.high_pass = p.high_pass.map(|s| sweep(s, false));
        self.low_pass = p.low_pass.map(|s| sweep(s, true));
        self.sweep_entry = frames(SWEEP_ENTRY_MS).max(1);
        self.duck = p.vocal_duck.filter(|d| d.db < 0.0).map(|d| {
            let end = frames(d.until_ms as f64).min(len);
            let start = end.saturating_sub(frames(d.release_ms as f64));
            self.duck_coef = Coef::band_pass(rate, d.hz as f64, VOX_Q);
            (Span { start, end }, 1.0 - db_to_gain((d.db as f64).clamp(-30.0, 0.0)))
        });
        // Delay capped at a second; the buffer is reused across transitions.
        self.echo = p.echo.filter(|e| e.delay_ms >= 1).map(|e| {
            let d = frames(e.delay_ms as f64).clamp(1, rate as u64) as usize;
            (d, (e.feedback as f64).clamp(0.0, 0.9), db_to_gain((e.wet_db as f64).clamp(-24.0, 0.0)))
        });
        if let Some((d, _, _)) = self.echo {
            self.echo_out = Span { start: len.saturating_sub(d as u64), end: len };
            let need = d * self.ch;
            if self.echo_buf.len() < need {
                self.echo_buf.resize(need, 0.0);
            }
            self.echo_buf[..need].fill(0.0);
            self.echo_pos = 0;
        }
        self.swap_state = [[[[0.0; 2]; MAX_CHANNELS]; 2]; 2];
        self.hold_state = [[[0.0; 2]; MAX_CHANNELS]; 2];
        self.highs_state = [[0.0; 2]; MAX_CHANNELS];
        self.duck_state = [[0.0; 2]; MAX_CHANNELS];
    }

    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Starts the clock `frames` into the transition, as if it had played up to there. Filters start clean;
    /// a sweep already running fades in from here.
    pub fn seek(&mut self, frames: u64) {
        self.pos = frames.min(self.len);
        self.fades = FadeRun::default();
        let (rate, pos) = (self.rate, self.pos);
        for f in [&mut self.low_pass, &mut self.high_pass].into_iter().flatten() {
            if pos >= f.span.start {
                f.retune(rate, pos);
                f.entry_from = pos;
            }
        }
    }

    pub fn done(&self) -> bool {
        self.pos >= self.len
    }

    /// Mixes `inc` into `out` in place.
    pub fn process<T: Sample>(&mut self, out: &mut [T], inc: &[T]) {
        let ch = self.ch;
        let (mut o, mut i) = ([0f64; MAX_CHANNELS], [0f64; MAX_CHANNELS]);
        for (fo, fi) in out.chunks_exact_mut(ch).zip(inc.chunks_exact(ch)) {
            for c in 0..ch {
                (o[c], i[c]) = (fo[c].to_f64(), fi[c].to_f64());
            }
            self.mix_frame(&mut o, &mut i);
            for c in 0..ch {
                fo[c] = T::from_f64(o[c]);
            }
        }
    }

    /// [`Mixer::process`] over little-endian PCM bytes.
    pub fn process_bytes(&mut self, out: &mut [u8], inc: &[u8], enc: Encoding) {
        match enc {
            Encoding::Pcm16 => self.process_le::<2>(out, inc, |b| i16::from_le_bytes(b).to_f64(), |v| i16::from_f64(v).to_le_bytes()),
            Encoding::Float => self.process_le::<4>(out, inc, |b| f32::from_le_bytes(b).to_f64(), |v| f32::from_f64(v).to_le_bytes()),
        }
    }

    fn process_le<const W: usize>(&mut self, out: &mut [u8], inc: &[u8], load: impl Fn([u8; W]) -> f64, store: impl Fn(f64) -> [u8; W]) {
        let (ch, fb) = (self.ch, self.ch * W);
        let (mut o, mut i) = ([0f64; MAX_CHANNELS], [0f64; MAX_CHANNELS]);
        let sample = |f: &[u8], c: usize| -> [u8; W] { f[c * W..c * W + W].try_into().unwrap() };
        for (fo, fi) in out.chunks_exact_mut(fb).zip(inc.chunks_exact(fb)) {
            for c in 0..ch {
                (o[c], i[c]) = (load(sample(fo, c)), load(sample(fi, c)));
            }
            self.mix_frame(&mut o, &mut i);
            for c in 0..ch {
                fo[c * W..c * W + W].copy_from_slice(&store(o[c]));
            }
        }
    }

    /// One section of a deck's low shelf with `lows` of its lows heard, 0..1.
    fn swap_section(&self, lows: f64) -> Coef {
        self.low_shelf.low(lows.max(db_to_gain(SWAP_KILL_DB)).sqrt())
    }

    /// The fade gains at frame `p`, and how much what both decks play together is held down to one song's
    /// power: where both are up at once, two songs would otherwise play louder than either. Lows a bass
    /// swap gives to one deck at a time are not held down.
    fn fades_at(&self, p: u64) -> Fades {
        let (out, inc) = (fade(self.curve, self.out_fade.progress(p), true), fade(self.curve, self.in_fade.progress(p), false));
        let held = 1.0 / (out * out + inc * inc).sqrt().max(1.0);
        let (trim_out, trim_in) = trims(self.gap_db, p as f64 / self.len.max(1) as f64);
        // Beat-matched kicks land together and add up, so their lows change hands at equal gain. Other lows add in
        // power: they change hands at equal power, held to the power of one song under the fades.
        let swapped = self.swap.map_or(0.0, |s| raised_cos(s.progress(p)));
        let (lows_out, lows_in) = if self.matched {
            (1.0 - swapped, swapped)
        } else {
            let (o, i) = ((1.0 - swapped).sqrt(), swapped.sqrt());
            let k = ((out * out + inc * inc).min(1.0) / (out * out * o * o + inc * inc * i * i).max(f64::MIN_POSITIVE)).sqrt();
            (k * o, k * i)
        };
        // All of the incoming voice band is held down until the release, none after.
        let duck = match self.duck {
            Some((span, depth)) if p < span.end => depth * (1.0 - raised_cos(span.progress(p))),
            _ => 0.0,
        };
        Fades {
            out: out * self.out_gain * db_to_gain(trim_out),
            inc: inc * db_to_gain(trim_in),
            held,
            low_out: self.swap_section(lows_out),
            low_in: self.swap_section(lows_in),
            shelf: self.hold_shelf.high(held),
            highs_in: self.highs_in.map_or_else(Coef::default, |(span, floor)| self.highs_shelf.high(floor + (1.0 - floor) * raised_cos(span.progress(p)))),
            duck,
            echo: 1.0 - raised_cos(self.echo_out.progress(p)),
        }
    }

    /// The run from frame `p` (inside the transition): up to [`FADE_RUN`] frames, ending at the next fade
    /// start or end. A fade with no length steps from its start to the frame after, so that is an end too.
    fn fade_run(&self, p: u64) -> FadeRun {
        let (o, i) = (self.out_fade, self.in_fade);
        let (s, d, e) = (self.swap.unwrap_or_default(), self.duck.map_or_else(Span::default, |d| d.0), self.echo_out);
        let h = self.highs_in.map_or_else(Span::default, |h| h.0);
        let bends = [o.start, o.start + 1, o.end, i.start, i.start + 1, i.end, s.start, s.end, d.start, d.end, e.start, e.end, h.start, h.end];
        let end = bends.into_iter().filter(|&b| b > p).fold((p + FADE_RUN).min(self.len), u64::min);
        FadeRun { start: p, end, from: self.fades_at(p), to: self.fades_at(end) }
    }

    /// Mixes one frame: outgoing `o` and incoming `i` in, the mix out in `o`.
    #[inline]
    fn mix_frame(&mut self, o: &mut [f64; MAX_CHANNELS], i: &mut [f64; MAX_CHANNELS]) {
        let (ch, p) = (self.ch, self.pos);
        self.pos += 1;
        if p >= self.len {
            o[..ch].copy_from_slice(&i[..ch]);
            return;
        }
        if !(self.fades.start..self.fades.end).contains(&p) {
            self.fades = self.fade_run(p);
        }
        let (run, t) = (self.fades, self.fades.t(p));
        let (g_out, g_in, held) = (run.get(t, |f| f.out), run.get(t, |f| f.inc), run.get(t, |f| f.held));
        let (rate, entry) = (self.rate, self.sweep_entry);
        for f in [&mut self.high_pass, &mut self.low_pass].into_iter().flatten() {
            let wet = f.wet(rate, entry, p);
            if wet > 0.0 {
                for (c, x) in o[..ch].iter_mut().enumerate() {
                    *x = f.run(c, *x, wet);
                }
            }
        }
        // Run on to the end once the highs are back: the shelf then passes its input, and its state settles.
        if self.highs_in.is_some() {
            let shelf = run.from.highs_in.lerp(&run.to.highs_in, t);
            for (x, s) in i[..ch].iter_mut().zip(&mut self.highs_state) {
                *x = shelf.run(s, *x);
            }
        }
        let duck = if self.duck.is_some() { run.get(t, |f| f.duck) } else { 0.0 };
        if duck > 0.0 {
            for (x, s) in i[..ch].iter_mut().zip(&mut self.duck_state) {
                *x -= duck * self.duck_coef.run(s, *x);
            }
        }
        if let Some((d, fb, wet)) = self.echo {
            // Post-fader send: the repeats are of what went through the fader, so they decay once it is down.
            let left = run.get(t, |f| f.echo);
            for c in 0..ch {
                let idx = self.echo_pos * ch + c;
                let rep = self.echo_buf[idx];
                self.echo_buf[idx] = o[c] * g_out * held + rep * fb;
                o[c] = o[c] * g_out * held + rep * wet * left + i[c] * g_in * held;
            }
            self.echo_pos = (self.echo_pos + 1) % d;
            return;
        }
        // The lows go to one deck at a time, so only the highs are held to one song's power, by a shelf
        // that is no filter at all once nothing is held.
        let swap = self.swap.map(|_| (run.from.low_out.lerp(&run.to.low_out, t), run.from.low_in.lerp(&run.to.low_in, t), run.from.shelf.lerp(&run.to.shelf, t)));
        for c in 0..ch {
            let (x, y) = (o[c], i[c]);
            o[c] = match swap {
                Some((low_out, low_in, shelf)) => {
                    let (s, h) = (&mut self.swap_state, &mut self.hold_state);
                    let x = shelf.run(&mut h[0][c], low_out.run2(&mut s[0], c, x));
                    let y = shelf.run(&mut h[1][c], low_in.run2(&mut s[1], c, y));
                    x * g_out + y * g_in
                }
                None => (x * g_out + y * g_in) * held,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Echo, Sweep, VocalDuck};

    const RATE: f64 = 48000.0;

    fn plan() -> TransitionPlan {
        TransitionPlan {
            kind: TransitionKind::EqualPowerFade,
            out_start_ms: 0,
            in_start_ms: 0,
            duration_ms: 1000,
            tempo_ratio: 1.0,
            tempo_ramp_beats: 0,
            tempo_ramp_ms: 0,
            keep_pitch: true,
            fade_curve: FadeCurve::EqualPower,
            out_fade_start_ms: 0,
            out_fade_end_ms: 1000,
            in_fade_start_ms: 0,
            in_fade_end_ms: 1000,
            out_gain_db: 0.0,
            in_gain_db: 0.0,
            bass_swap: None,
            highs_in: None,
            low_pass: None,
            high_pass: None,
            echo: None,
            out_loop_ms: None,
            vocal_duck: None,
            in_loop: None,
            bend: None,
            in_low_pass: None,
            in_high_pass: None,
            mid_dip: None,
            reverb: None,
            in_delay_ms: 0,
            out_glide: None,
            brake: None,
            reason: String::new(),
        }
    }

    fn sine(freq: f64, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (0.5 * (2.0 * PI * freq * i as f64 / RATE).sin()) as f32).collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    /// `out` and `inc` mixed by a fresh mono mixer configured with `p`.
    fn mix(p: &TransitionPlan, out: &[f32], inc: &[f32]) -> Vec<f32> {
        let mut m = Mixer::new(RATE as u32, 1);
        m.configure(p);
        let mut y = out.to_vec();
        m.process(&mut y, inc);
        y
    }

    #[test]
    fn fade_curves() {
        let (a, b) = (sine(440.0, 72000), sine(1234.5, 72000));
        let y = mix(&plan(), &a, &b);
        let r = rms(&a);
        for (n, e) in y[..48000].chunks(480).map(rms).enumerate() {
            assert!((e / r - 1.0).abs() < 0.08, "slice {n}: {e} vs {r}");
        }
        assert_eq!(&y[48000..], &b[48000..], "after the fade it is the incoming track, bit for bit");
        assert!(y[..10].iter().zip(&a).all(|(y, a)| (y - a).abs() < 1e-3), "starts on the outgoing track");

        // Sine squared sums one signal to itself.
        let mut p = plan();
        p.fade_curve = FadeCurve::SineSquared;
        let mut m = Mixer::new(RATE as u32, 2);
        m.configure(&p);
        let a: Vec<f32> = sine(440.0, 48000).iter().flat_map(|v| [*v, *v]).collect();
        let mut y = a.clone();
        m.process(&mut y, &a);
        assert!(y.iter().zip(&a).all(|(u, v)| (u - v).abs() < 1e-5));

        // Crossover is where incoming gets louder.
        let mut p = plan();
        assert_eq!(crossover_ms(&p), 500);
        p.in_gain_db = -6.0;
        let trimmed = crossover_ms(&p);
        assert!(trimmed > 550 && trimmed < 1000, "{trimmed}");
        p.in_gain_db = 0.0;
        (p.in_fade_start_ms, p.in_fade_end_ms) = (500, 1000);
        let late = crossover_ms(&p);
        assert!(late > 600 && late < 1000, "{late}");
        p.duration_ms = 0;
        assert_eq!(crossover_ms(&p), 0);
    }

    /// The outgoing gain heard through fades of 10 ms and of none, starting off the run grid: near the
    /// exact curve, flat up to the fade's start, and a fade with no length is a step.
    #[test]
    fn fade_runs_follow_curves() {
        for (start_ms, end_ms) in [(7, 17), (10, 10)] {
            let mut p = plan();
            p.duration_ms = 30;
            (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (start_ms, end_ms, start_ms, end_ms);
            let (ones, zeros) = (vec![1f32; 1440], vec![0f32; 1440]);
            let y = mix(&p, &ones, &zeros);
            let span = Span { start: (start_ms * 48) as u64, end: (end_ms * 48) as u64 };
            let worst = y.iter().enumerate().map(|(k, v)| (*v as f64 - fade(p.fade_curve, span.progress(k as u64), true)).abs()).fold(0.0, f64::max);
            assert!(worst < 1.5e-3, "{start_ms}-{end_ms} ms: {worst} off the curve");
            assert!(y[..=span.start as usize].iter().all(|v| *v == 1.0), "{start_ms}-{end_ms} ms: full until the fade starts");
            assert!(y[span.end.max(span.start + 1) as usize..].iter().all(|v| *v == 0.0), "{start_ms}-{end_ms} ms: gone once it ends");
        }
    }

    #[test]
    fn bytes_mix_like_samples() {
        let (a, b): (Vec<i16>, Vec<i16>) = ((0..9600).map(|i| (i * 7) as i16).collect(), (0..9600).map(|i| -(i * 3) as i16).collect());
        let mut m = Mixer::new(RATE as u32, 2);
        m.configure(&plan());
        let mut y = a.clone();
        m.process(&mut y, &b);
        let mut bytes: Vec<u8> = a.iter().flat_map(|v| v.to_le_bytes()).collect();
        let inc: Vec<u8> = b.iter().flat_map(|v| v.to_le_bytes()).collect();
        m.configure(&plan());
        m.process_bytes(&mut bytes, &inc, Encoding::Pcm16);
        assert_eq!(bytes, y.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    }

    #[test]
    fn seek_resumes_curves_mid_transition() {
        let mut p = plan();
        p.fade_curve = FadeCurve::Linear;
        let (ones, zeros) = (vec![1f32; 48000], vec![0f32; 48000]);
        let whole = mix(&p, &ones, &zeros);
        let mut m = Mixer::new(RATE as u32, 1);
        m.configure(&p);
        m.seek(14400);
        assert_eq!(m.position(), 14400);
        let mut late = ones[14400..].to_vec();
        m.process(&mut late, &zeros[14400..]);
        for (n, (l, w)) in late.iter().zip(&whole[14400..]).enumerate() {
            assert!((l - w).abs() < 1e-5, "frame {n}: {l} vs {w}");
        }
        assert!(m.done());
    }

    #[test]
    fn fades_follow_windows() {
        let mut p = plan();
        p.duration_ms = 4000;
        p.fade_curve = FadeCurve::Linear;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (2000, 4000, 2000, 4000);
        let (ones, zeros) = (vec![1f32; 192000], vec![0f32; 192000]);
        let y = mix(&p, &ones, &zeros);
        assert!((y[48000] - 1.0).abs() < 1e-6 && (y[144000] - 0.5).abs() < 1e-3, "outgoing holds, then falls linearly");
        let y = mix(&p, &zeros, &ones);
        assert!(y[48000].abs() < 1e-6 && (y[144000] - 0.5).abs() < 1e-3, "incoming waits, then rises linearly");
    }

    /// Both decks up at once (the incoming one in before the outgoing one goes) hold one song's power, the
    /// two still in the windows' proportion: two songs never play louder than one.
    #[test]
    fn decks_up_together_hold_one_songs_power() {
        let mut p = plan();
        p.duration_ms = 4000;
        p.fade_curve = FadeCurve::Linear;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (2000, 4000, 0, 2000);
        let (ones, zeros) = (vec![1f32; 192000], vec![0f32; 192000]);
        let (out, inc) = (mix(&p, &ones, &zeros), mix(&p, &zeros, &ones));
        for (at, ratio) in [(48000, 0.5), (96000, 1.0), (144000, 2.0)] {
            let (o, i) = (out[at] as f64, inc[at] as f64);
            assert!((o * o + i * i - 1.0).abs() < 2e-3, "at frame {at}: {o:.3} and {i:.3}");
            assert!((i / o - ratio).abs() < 2e-3, "at frame {at}: {o:.3} and {i:.3}");
        }
    }

    /// One deck's level, dB every 100 ms and after, through a transition of `ms` across a loudness gap of
    /// `db`, the other deck gone: the outgoing one up to the end, or the incoming one up from the start.
    fn trim_levels(ms: i64, db: f32, incoming: bool) -> Vec<f64> {
        let mut p = plan();
        p.duration_ms = ms;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = if incoming { (0, 0, 0, 0) } else { (ms, ms, ms, ms) };
        p.in_gain_db = db;
        let n = (ms as f64 / 1000.0 * RATE) as usize + 4800;
        let (deck, quiet) = (vec![1f32; n], vec![0f32; n]);
        let y = if incoming { mix(&p, &quiet, &deck) } else { mix(&p, &deck, &quiet) };
        y.iter().skip(1).step_by(4800).map(|v| 20.0 * (*v as f64).log10()).collect()
    }

    /// The two songs meet across a loudness gap: the incoming one starts turned to the outgoing one's level,
    /// the outgoing one ends turned to the incoming one's, and the level glides evenly between, no faster
    /// than the limit, so the incoming song is at its own level as the transition ends and stays so.
    #[test]
    fn songs_meet_across_a_loudness_gap() {
        for (ms, db) in [(8000, 6.0f32), (8000, 9.0), (2000, 9.0), (2000, -9.0), (1000, 6.0), (300, -9.0)] {
            let gap = (ms as f64 / TRIM_GLIDE_MS_PER_DB).min(db.abs() as f64).copysign(db as f64);
            let inc = trim_levels(ms, db, true);
            let step = inc.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
            assert!(step <= 0.67 + 1e-3, "{ms} ms, {db} dB: {step:.2} dB in 100 ms: {inc:.2?}");
            assert!((inc[0] - gap).abs() < 0.01, "{ms} ms, {db} dB: the incoming song starts turned by {:.2}", inc[0]);
            assert!(inc.last().unwrap().abs() < 1e-3, "{ms} ms, {db} dB: at its own level after: {inc:.2?}");
            let mid = inc.len() / 2;
            assert!((inc[mid] - gap * (1.0 - (mid as f64 * 100.0) / ms as f64)).abs() < 0.05, "{ms} ms, {db} dB: evenly: {inc:.2?}");
            let out = trim_levels(ms, db, false);
            let k = (ms / 100 - 1) as usize;
            assert!(out[0].abs() < 0.01 && (out[k] + gap * k as f64 * 100.0 / ms as f64).abs() < 0.05, "{ms} ms, {db} dB: the outgoing song turned towards the incoming one: {out:.2?}");
        }
    }

    #[test]
    fn echo_out() {
        // The delay is fed after the fader: its repeats decay with the song.
        let mut p = plan();
        p.duration_ms = 2000;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 500, 2000, 2000);
        p.echo = Some(Echo { delay_ms: 250, feedback: 0.45, wet_db: -7.0, send: None, band: None, duck: None });
        let tone = sine(440.0, 96000);
        let y = mix(&p, &tone, &vec![0f32; 96000]);
        let ring = |ms: usize| rms(&y[ms * 48..(ms + 250) * 48]) / rms(&tone);
        let (first, third) = (ring(500), ring(1000));
        assert!(first > 0.05, "repeats ring after the dry deck is gone: {first:.3}");
        assert!(third < 0.3 * first, "and die away two delays on: {third:.3} against {first:.3}");

        // Echo fades out by end.
        let mut p = plan();
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 500, 1000, 1000);
        p.echo = Some(Echo { delay_ms: 250, feedback: 0.45, wet_db: -7.0, send: None, band: None, duck: None });
        let tone = sine(440.0, 48000);
        let y = mix(&p, &tone, &vec![0f32; 48000]);
        let ring = |a: usize, b: usize| rms(&y[a..b]) / rms(&tone);
        let (after, end) = (ring(26400, 33600), ring(47000, 48000));
        assert!(after > 0.05, "repeats ring after the dry deck is gone: {after:.3}");
        assert!(end < 0.05 * after, "and are faded out by the end, not cut off: {end:.4} against {after:.3}");

        // Echo repeats on the beat.
        let mut p = plan();
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 480, 480, 960);
        p.echo = Some(Echo { delay_ms: 240, feedback: 0.5, wet_db: 0.0, send: None, band: None, duck: None });
        let mut click = vec![0f32; 48000];
        click[0] = 1.0;
        let y = mix(&p, &click, &vec![0f32; 48000]);
        let d = (0.24 * RATE) as usize;
        let at = |n: usize| y[n * d..n * d + 24].iter().map(|v| v.abs()).fold(0.0f32, f32::max);
        for (n, want) in [(0, 1.0), (1, 1.0), (2, 0.5), (3, 0.25)] {
            assert!((at(n) - want).abs() < 0.1, "repeat {n}: {}", at(n));
        }
    }

    #[test]
    fn bass_swap_moves_lows_between_decks() {
        let mut p = plan();
        p.fade_curve = FadeCurve::Linear;
        // Both decks up the whole time, so only the filters act: the lows, on one deck at a time, at full
        // level, and the highs both play held to one song's power between them.
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (1000, 1000, 0, 0);
        p.bass_swap = Some(crate::types::BassSwap { at_ms: 500, len_ms: 100, cut_hz: 200.0 });
        let (bass, zeros) = (sine(50.0, 48000), vec![0f32; 48000]);
        let r = rms(&bass[..4800]);
        let y = mix(&p, &bass, &zeros);
        assert!((rms(&y[12000..24000]) / r - 1.0).abs() < 0.02, "outgoing bass untouched before the swap");
        assert!(rms(&y[30000..42000]) / r < 0.03, "outgoing bass gone after it");
        let y = mix(&p, &zeros, &bass);
        assert!(rms(&y[12000..24000]) / r < 0.03, "incoming bass cut before the swap");
        assert!((rms(&y[30000..42000]) / r - 1.0).abs() < 0.02, "and whole after it");
        let hi = sine(3000.0, 48000);
        let y = mix(&p, &zeros, &hi);
        assert!((rms(&y[2400..24000]) / rms(&hi) / std::f64::consts::FRAC_1_SQRT_2 - 1.0).abs() < 0.03, "highs pass throughout");
    }

    /// A fade between kicks that are not beat-matched hands the bass over and holds its level: one song's lows
    /// at a time, as loud as either song's own, where the fades cross too.
    #[test]
    fn a_fade_hands_the_bass_over_at_one_level() {
        let mut p = plan();
        p.bass_swap = Some(crate::types::BassSwap { at_ms: 400, len_ms: 200, cut_hz: 400.0 });
        let (a, b) = (sine(50.0, 48000), sine(80.0, 48000));
        let y = mix(&p, &a, &b);
        let db = |at: usize| 20.0 * (rms(&y[at..at + 4800]) / rms(&a)).log10();
        for at in (2400..43200).step_by(2400) {
            assert!(db(at).abs() < 1.0, "{:.2} s into the fade: {:+.1} dB", at as f64 / RATE, db(at));
        }
    }

    /// The lows change hands and nothing else: through a bass swap and after it, the mids above the cut keep
    /// the level they had before it.
    #[test]
    fn incoming_highs_come_in_at_their_own_time() {
        let mut p = plan();
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 0, 0, 0);
        p.highs_in = Some(crate::types::HighsIn { at_ms: 400, len_ms: 200, db: -15.0, hz: 3000.0 });
        let zeros = vec![0f32; 48000];
        let (hats, mids) = (sine(10_000.0, 48000), sine(400.0, 48000));
        let level = |x: &[f32], a: usize, b: usize| rms(&mix(&p, &zeros, x)[a..b]) / rms(&x[a..b]);
        assert!(level(&hats, 4800, 19200) < 0.2, "hats held down before: {}", level(&hats, 4800, 19200));
        assert!((level(&hats, 30000, 48000) - 1.0).abs() < 0.02, "whole after");
        assert!((level(&mids, 4800, 19200) - 1.0).abs() < 0.05, "the mids untouched: {}", level(&mids, 4800, 19200));
    }

    #[test]
    fn bass_swap_leaves_the_mids() {
        let mut p = plan();
        p.fade_curve = FadeCurve::Linear;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (1000, 1000, 0, 0);
        p.bass_swap = Some(crate::types::BassSwap { at_ms: 300, len_ms: 400, cut_hz: 200.0 });
        let zeros = vec![0f32; 48000];
        for (hz, most_db) in [(300.0, 2.0), (400.0, 1.0), (600.0, 0.5)] {
            let tone = sine(hz, 48000);
            let y = mix(&p, &tone, &zeros);
            let db = |a: usize| 20.0 * (rms(&y[a..a + 2400]) / rms(&y[4800..12000])).log10();
            let low = (14400..45600).step_by(1200).map(db).fold(f64::MAX, f64::min);
            assert!(low > -most_db, "{hz} Hz falls {low:.1} dB through the swap");
        }
    }

    #[test]
    fn low_pass_sweep_without_clicks() {
        let mut p = plan();
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (1000, 1000, 1000, 1000);
        p.low_pass = Some(Sweep { start_ms: 200, end_ms: 600, from_hz: 20000.0, to_hz: 300.0 });
        let hi = sine(4000.0, 48000);
        let y = mix(&p, &hi, &vec![0f32; 48000]);
        let r = rms(&hi);
        assert!((rms(&y[..9000]) / r - 1.0).abs() < 0.01, "untouched before the sweep");
        assert!(rms(&y[31000..47000]) / r < 0.02, "4 kHz is 24 dB+ under a 300 Hz 4th-order low-pass");
        let jump = y.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        assert!(jump < 0.5 * 2.0 * std::f32::consts::PI * 4000.0 / 48000.0 * 1.1, "no clicks: {jump}");
    }

    #[test]
    fn vocal_duck_until_release() {
        let mut p = plan();
        // The incoming deck at full level throughout, so only the duck acts.
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 0, 0, 0);
        p.vocal_duck = Some(VocalDuck { until_ms: 600, release_ms: 100, db: -12.0, hz: 1000.0 });
        let zeros = vec![0f32; 48000];
        for (hz, held) in [(1000.0, -12.0), (100.0, -0.6), (8000.0, -0.3)] {
            let tone = sine(hz, 48000);
            let y = mix(&p, &zeros, &tone);
            let r = rms(&tone);
            let db = |x: &[f32]| 20.0 * (rms(x) / r).log10();
            assert!((db(&y[4800..24000]) - held).abs() < 1.0, "{hz} Hz held: {:.1} dB", db(&y[4800..24000]));
            assert!(db(&y[30000..47000]).abs() < 0.2, "{hz} Hz released: {:.1} dB", db(&y[30000..47000]));
            let jump = y.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
            assert!(jump <= 0.5 * 2.0 * std::f32::consts::PI * hz as f32 / 48000.0 * 1.2 + 1e-3, "{hz} Hz: no clicks ({jump})");
        }
    }

    /// Mixing time of a 12 s stereo transition of each kind (`cargo test --release -p nori-player --lib
    /// mixer::tests::mixing_perf -- --ignored --nocapture`); `NORI_DUMP=dir` writes each mix, to compare
    /// two builds' output.
    #[test]
    #[ignore]
    fn mixing_perf() {
        let mut base = plan();
        (base.duration_ms, base.out_fade_end_ms, base.in_fade_end_ms) = (12_000, 12_000, 12_000);
        let trimmed = TransitionPlan { in_gain_db: -4.5, ..base.clone() };
        let kinds = [
            ("fade", base.clone()),
            ("fade, trimmed", trimmed.clone()),
            ("bass swap, trimmed", TransitionPlan { bass_swap: Some(crate::types::BassSwap { at_ms: 4_000, len_ms: 2_000, cut_hz: 180.0 }), ..trimmed.clone() }),
            ("vocal duck", TransitionPlan { vocal_duck: Some(VocalDuck { until_ms: 8_000, release_ms: 3_000, db: -9.0, hz: 1_500.0 }), ..trimmed.clone() }),
            ("echo", TransitionPlan { echo: Some(Echo { delay_ms: 500, feedback: 0.4, wet_db: -6.0, send: None, band: None, duck: None }), ..trimmed.clone() }),
            ("sweeps", TransitionPlan { low_pass: Some(Sweep { start_ms: 2_000, end_ms: 10_000, from_hz: 18_000.0, to_hz: 300.0 }), high_pass: Some(Sweep { start_ms: 0, end_ms: 12_000, from_hz: 20.0, to_hz: 400.0 }), ..trimmed }),
        ];
        let frames = 44_100 * 12;
        let stereo = |f: f64| -> Vec<f32> { sine(f, frames).iter().flat_map(|v| [*v, -*v]).collect() };
        let (a, b) = (stereo(220.0), stereo(331.0));
        let mut m = Mixer::new(44_100, 2);
        for (name, p) in kinds {
            let mut y = Vec::new();
            let t = std::time::Instant::now();
            for _ in 0..10 {
                m.configure(&p);
                y.clone_from(&a);
                m.process(&mut y, &b);
            }
            println!("perf: {name:20} {:6.2} ms per transition", t.elapsed().as_secs_f64() * 100.0);
            if let Ok(dir) = std::env::var("NORI_DUMP") {
                let bytes: Vec<u8> = y.iter().flat_map(|v| v.to_le_bytes()).collect();
                std::fs::write(format!("{dir}/{}.f32", name.replace([' ', ','], "")), bytes).unwrap();
            }
        }
    }
}
