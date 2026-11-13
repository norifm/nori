//! The transition mixer: the outgoing track's tail and the incoming track's start (already time-stretched) in,
//! one stream out, mixed as a `TransitionPlan` says: gain curves and loudness trim, bass swap (4th-order
//! Linkwitz-Riley high-pass crossfaded in on both decks), low- and high-pass sweeps on either deck, a mid dip, vocal
//! duck on the incoming deck, and an echo and a reverb on the outgoing deck's post-fader send.
//!
//! Past the end of the transition the output is the incoming stream untouched. Buffers are sized by
//! [`Mixer::prepare`]; mixing allocates nothing.

use crate::pcm::Encoding;
use crate::types::{FadeCurve, TransitionKind, TransitionPlan};
use std::f64::consts::PI;

const MAX_CHANNELS: usize = 8;
/// The loudness trim lets go no faster than this, ms per dB (0.67 dB per 100 ms).
pub(crate) const TRIM_GLIDE_MS_PER_DB: f64 = 150.0;
/// Frames between sweep coefficient updates.
const SWEEP_STEP: u64 = 16;
/// A sweep's filter is faded in over this long so switching it on does not click, and out as long once it ends open.
const SWEEP_ENTRY_MS: f64 = 50.0;
/// A sweep ending at or past these is open: its filter is switched off after it.
const OPEN_LOW_PASS_HZ: f64 = 18_000.0;
const OPEN_HIGH_PASS_HZ: f64 = 25.0;
/// Q of the band-pass the vocal duck and the mid dip subtract: 3 dB down at about 300 Hz and 3.3 kHz around 1 kHz.
const VOX_Q: f64 = 0.35;
/// An echo's or reverb's send window opens and closes over this long, so the repeats do not click.
const SEND_RAMP_MS: f64 = 10.0;
/// Freeverb's comb and all-pass lengths at 44.1 kHz, scaled to the rate.
const COMB_FRAMES: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_FRAMES: [usize; 4] = [556, 441, 341, 225];
const ALLPASS_GAIN: f64 = 0.5;
/// How much of the combs' highs each pass keeps back (Freeverb's default damping).
const COMB_DAMP: f64 = 0.2;

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

/// A low- or high-pass sweep on one deck, two cascaded Butterworth sections per channel.
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
    /// A sweep that ends open fades its filter out from its end, and is off after that.
    open_from: Option<u64>,
}

impl SweepFilter {
    fn new(rate: f64, span: Span, from: f64, to: f64, low: bool) -> Self {
        let opens = if low { to >= OPEN_LOW_PASS_HZ } else { to <= OPEN_HIGH_PASS_HZ };
        let (from, to) = (from.min(rate * 0.45), to.min(rate * 0.45));
        SweepFilter {
            span,
            from,
            to,
            low,
            coef: Coef::pass(rate, from, low),
            state: [[[0.0; 2]; MAX_CHANNELS]; 2],
            entry_from: span.start,
            open_from: opens.then_some(span.end),
        }
    }

    fn retune(&mut self, rate: f64, p: u64) {
        self.coef = Coef::pass(rate, self.from * (self.to / self.from).powf(self.span.progress(p)), self.low);
    }

    /// How much of the filtered signal is heard at frame `p`; retunes every [`SWEEP_STEP`] frames until the sweep
    /// ends.
    #[inline]
    fn wet(&mut self, rate: f64, entry: u64, p: u64) -> f64 {
        if p < self.span.start || self.open_from.is_some_and(|o| p >= o + entry) {
            return 0.0;
        }
        if (p - self.span.start).is_multiple_of(SWEEP_STEP) && p < self.span.end + SWEEP_STEP {
            self.retune(rate, p);
        }
        let fade_in = (p - self.entry_from.min(p)) as f64 / entry as f64;
        let fade_out = self.open_from.map_or(1.0, |o| 1.0 - p.saturating_sub(o) as f64 / entry as f64);
        fade_in.min(fade_out).min(1.0)
    }

    /// Filters one deck's frame.
    #[inline]
    fn apply(&mut self, rate: f64, entry: u64, p: u64, x: &mut [f64]) {
        let wet = self.wet(rate, entry, p);
        if wet > 0.0 {
            for (c, x) in x.iter_mut().enumerate() {
                let y = self.coef.run2(&mut self.state, c, *x);
                *x = *x * (1.0 - wet) + y * wet;
            }
        }
    }
}

/// A peaking cut on either deck: a band-pass subtracted, `depth` of it in the window's middle, none at its ends.
struct MidDip {
    span: Span,
    depth: f64,
    coef: Coef,
    /// Outgoing, incoming.
    decks: [bool; 2],
}

/// The echo on the outgoing deck's post-fader send.
struct EchoFx {
    delay: usize,
    feedback: f64,
    wet: f64,
    /// What is sent into the delay; all of it without a window.
    send: Option<Span>,
    /// The repeats are held down over the span, to `floor` at its end and after.
    duck: Option<(Span, f64)>,
    /// High- and low-pass in the feedback loop.
    band: Option<(Coef, Coef)>,
    /// Repeats still ringing fade out over the transition's last delay.
    tail: Span,
}

/// The reverb on the outgoing deck's post-fader send.
struct ReverbFx {
    send: Span,
    /// The wet level fades out over this.
    tail: Span,
    wet: f64,
    /// Each comb's feedback (to 60 dB down in the decay time) and input gain (an even share of the send's power).
    feedback: [f64; 8],
    input: [f64; 8],
    low_cut: Coef,
}

/// A delay line of one channel.
#[derive(Default)]
struct Line {
    buf: Vec<f32>,
    pos: usize,
}

impl Line {
    #[inline]
    fn read(&self) -> f64 {
        self.buf[self.pos] as f64
    }

    #[inline]
    fn write(&mut self, v: f64) {
        self.buf[self.pos] = v as f32;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
}

/// A small Schroeder reverb (Chowning's JCRev layout with Freeverb's lengths and damping): four all-passes in series
/// diffuse a mono send, eight damped combs in parallel ring it, and two sign patterns over the combs make two
/// decorrelated outputs.
#[derive(Default)]
struct Room {
    allpasses: [Line; 4],
    combs: [Line; 8],
    damp: [f64; 8],
    low_state: [f64; 2],
}

impl Room {
    /// Sizes the lines for `rate`, once.
    fn size(&mut self, rate: f64) {
        let frames = |n: usize| ((n as f64 * rate / 44_100.0).round() as usize).max(1);
        for (l, n) in self.allpasses.iter_mut().zip(ALLPASS_FRAMES).chain(self.combs.iter_mut().zip(COMB_FRAMES)) {
            if l.buf.is_empty() {
                l.buf = vec![0.0; frames(n)];
            }
        }
    }

    fn clear(&mut self) {
        for l in self.allpasses.iter_mut().chain(&mut self.combs) {
            l.buf.fill(0.0);
            l.pos = 0;
        }
        (self.damp, self.low_state) = ([0.0; 8], [0.0; 2]);
    }

    /// One frame of the mono send in, the two outputs.
    #[inline]
    fn run(&mut self, fx: &ReverbFx, x: f64) -> (f64, f64) {
        let mut x = fx.low_cut.run(&mut self.low_state, x);
        for a in &mut self.allpasses {
            let w = a.read();
            let v = x + ALLPASS_GAIN * w;
            a.write(v);
            x = w - ALLPASS_GAIN * v;
        }
        let (mut l, mut r) = (0.0, 0.0);
        for (k, c) in self.combs.iter_mut().enumerate() {
            let y = c.read();
            self.damp[k] = y * (1.0 - COMB_DAMP) + self.damp[k] * COMB_DAMP;
            c.write(x * fx.input[k] + self.damp[k] * fx.feedback[k]);
            l += y;
            r += if k % 2 == 0 { y } else { -y };
        }
        (l, r)
    }
}

/// 0 outside `span`, 1 inside, ramped over `ramp` frames at both ends.
#[inline]
fn window_gain(span: Span, ramp: u64, p: u64) -> f64 {
    if p <= span.start || p >= span.end {
        return 0.0;
    }
    let r = ramp.max(1) as f64;
    ((p - span.start) as f64 / r).min((span.end - p) as f64 / r).min(1.0)
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
/// of a fade, the bass swap, the duck's release, a send's ramps or an effect's tail, where a curve bends or steps.
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
    /// How much of the mid band is dipped.
    dip: f64,
    /// How much of the outgoing deck goes into the echo, and how much of its repeats is heard.
    echo_send: f64,
    echo: f64,
    /// How much of the outgoing deck goes into the reverb, and how much of its tail is heard.
    verb_send: f64,
    verb: f64,
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
    in_high_pass: Option<SweepFilter>,
    in_low_pass: Option<SweepFilter>,
    sweep_entry: u64,
    /// The duck's release span and how much of the band-pass is subtracted before it.
    duck: Option<(Span, f64)>,
    duck_coef: Coef,
    duck_state: [[f64; 2]; MAX_CHANNELS],
    mid_dip: Option<MidDip>,
    /// [deck][channel].
    dip_state: [[[f64; 2]; MAX_CHANNELS]; 2],
    send_ramp: u64,
    echo: Option<EchoFx>,
    echo_buf: Vec<f64>,
    echo_pos: usize,
    /// The band's high- and low-pass: [filter][channel].
    echo_band_state: [[[f64; 2]; MAX_CHANNELS]; 2],
    reverb: Option<ReverbFx>,
    room: Room,
    /// The brake's frames (start, length), and the outgoing audio it has taken, read back ever slower.
    brake: Option<(u64, u64)>,
    brake_buf: Vec<f64>,
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
            in_high_pass: None,
            in_low_pass: None,
            sweep_entry: 1,
            duck: None,
            duck_coef: Coef::default(),
            duck_state: [[0.0; 2]; MAX_CHANNELS],
            mid_dip: None,
            dip_state: [[[0.0; 2]; MAX_CHANNELS]; 2],
            send_ramp: 1,
            echo: None,
            echo_buf: Vec::new(),
            echo_pos: 0,
            echo_band_state: [[[0.0; 2]; MAX_CHANNELS]; 2],
            reverb: None,
            room: Room::default(),
            brake: None,
            brake_buf: Vec::new(),
            fades: FadeRun::default(),
        }
    }

    fn frames(&self, ms: f64) -> u64 {
        (ms.max(0.0) * self.rate / 1000.0).round() as u64
    }

    /// An echo's delay in frames, capped at a second.
    fn echo_delay(&self, ms: i64) -> Option<usize> {
        (ms >= 1).then(|| self.frames(ms as f64).clamp(1, self.rate as u64) as usize)
    }

    /// Sizes the buffers `p` needs, kept for later transitions: configuring and mixing it then allocate nothing.
    pub fn prepare(&mut self, p: &TransitionPlan) {
        if let Some(d) = p.echo.and_then(|e| self.echo_delay(e.delay_ms)) {
            let need = d * self.ch;
            if self.echo_buf.len() < need {
                self.echo_buf.resize(need, 0.0);
            }
        }
        if p.reverb.is_some() {
            self.room.size(self.rate);
        }
        if let Some(b) = p.brake {
            let need = (self.frames(b.len_ms as f64) as usize + 1) * self.ch;
            if self.brake_buf.len() < need {
                self.brake_buf.resize(need, 0.0);
            }
        }
    }

    /// Takes a plan and restarts the clock.
    pub fn configure(&mut self, p: &TransitionPlan) {
        self.prepare(p);
        let rate = self.rate;
        let frames = |ms: f64| (ms.max(0.0) * rate / 1000.0).round() as u64;
        let len = frames(p.duration_ms as f64);
        let span = |a: i64, b: i64| {
            let (a, b) = span_ms(a, b, p.duration_ms.max(0) as f64);
            Span { start: frames(a), end: frames(b) }
        };
        // A change that starts at `at_ms` and takes `len_ms`, inside the transition.
        let from = |at_ms: i64, len_ms: i64| {
            let start = frames(at_ms as f64).min(len);
            Span { start, end: (start + frames((len_ms as f64).max(1.0))).min(len.max(1)) }
        };
        self.pos = 0;
        self.fades = FadeRun::default();
        self.len = len;
        self.brake = p.brake.filter(|b| b.len_ms > 0).map(|b| (frames(b.start_ms as f64), frames(b.len_ms as f64).max(1)));
        self.curve = p.fade_curve;
        self.out_fade = span(p.out_fade_start_ms, p.out_fade_end_ms);
        self.in_fade = span(p.in_fade_start_ms, p.in_fade_end_ms);
        self.out_gain = db_to_gain((p.out_gain_db as f64).clamp(-24.0, 12.0));
        self.gap_db = trim(p.duration_ms as f64, p.in_gain_db as f64);
        self.swap = p.bass_swap.map(|s| from(s.at_ms, s.len_ms));
        self.matched = p.kind == TransitionKind::BeatMatched;
        let cut = p.bass_swap.map_or(180.0, |s| s.cut_hz as f64);
        (self.low_shelf, self.hold_shelf) = (Shelf::at(rate, cut / 2.0), Shelf::at(rate, cut));
        self.highs_in = p.highs_in.filter(|h| h.db < 0.0).map(|h| {
            self.highs_shelf = Shelf::at(rate, h.hz as f64);
            (from(h.at_ms, h.len_ms), db_to_gain((h.db as f64).clamp(-30.0, 0.0)))
        });
        let sweep = |s: crate::types::Sweep, low| SweepFilter::new(rate, span(s.start_ms, s.end_ms), s.from_hz as f64, s.to_hz as f64, low);
        self.high_pass = p.high_pass.map(|s| sweep(s, false));
        self.low_pass = p.low_pass.map(|s| sweep(s, true));
        self.in_high_pass = p.in_high_pass.map(|s| sweep(s, false));
        self.in_low_pass = p.in_low_pass.map(|s| sweep(s, true));
        self.sweep_entry = frames(SWEEP_ENTRY_MS).max(1);
        self.duck = p.vocal_duck.filter(|d| d.db < 0.0).map(|d| {
            let end = frames(d.until_ms as f64).min(len);
            let start = end.saturating_sub(frames(d.release_ms as f64));
            self.duck_coef = Coef::band_pass(rate, d.hz as f64, VOX_Q);
            (Span { start, end }, 1.0 - db_to_gain((d.db as f64).clamp(-30.0, 0.0)))
        });
        self.mid_dip = p.mid_dip.filter(|d| d.db < 0.0 && (d.outgoing || d.incoming)).map(|d| MidDip {
            span: span(d.window.start_ms, d.window.end_ms),
            depth: 1.0 - db_to_gain((d.db as f64).clamp(-30.0, 0.0)),
            coef: Coef::band_pass(rate, d.hz as f64, VOX_Q),
            decks: [d.outgoing, d.incoming],
        });
        self.send_ramp = frames(SEND_RAMP_MS).max(1);
        self.echo = p.echo.and_then(|e| {
            let delay = self.echo_delay(e.delay_ms)?;
            Some(EchoFx {
                delay,
                feedback: (e.feedback as f64).clamp(0.0, 0.9),
                wet: db_to_gain((e.wet_db as f64).clamp(-24.0, 0.0)),
                send: e.send.map(|w| span(w.start_ms, w.end_ms)),
                duck: e.duck.filter(|d| d.db < 0.0).map(|d| (from(d.at_ms, d.len_ms), db_to_gain((d.db as f64).clamp(-48.0, 0.0)))),
                band: e.band.map(|b| (Coef::pass(rate, b.low_hz as f64, false), Coef::pass(rate, b.high_hz as f64, true))),
                tail: Span { start: len.saturating_sub(delay as u64), end: len },
            })
        });
        if let Some(e) = &self.echo {
            self.echo_buf[..e.delay * self.ch].fill(0.0);
            self.echo_pos = 0;
        }
        let combs: [usize; 8] = std::array::from_fn(|k| self.room.combs[k].buf.len());
        self.reverb = p.reverb.map(|r| {
            let decay = (r.decay_s as f64).clamp(0.1, 20.0);
            let feedback = combs.map(|n| 10f64.powf(-3.0 * n as f64 / (rate * decay)));
            ReverbFx {
                send: span(r.send.start_ms, r.send.end_ms),
                tail: Span { start: len.saturating_sub(frames(decay * 250.0)), end: len },
                wet: db_to_gain((r.wet_db as f64).clamp(-30.0, 0.0)),
                input: feedback.map(|g| ((1.0 - g * g) / 8.0).sqrt()),
                feedback,
                low_cut: Coef::pass(rate, r.low_cut_hz as f64, false),
            }
        });
        if self.reverb.is_some() {
            self.room.clear();
        }
        self.swap_state = [[[[0.0; 2]; MAX_CHANNELS]; 2]; 2];
        self.hold_state = [[[0.0; 2]; MAX_CHANNELS]; 2];
        self.highs_state = [[0.0; 2]; MAX_CHANNELS];
        self.duck_state = [[0.0; 2]; MAX_CHANNELS];
        self.dip_state = [[[0.0; 2]; MAX_CHANNELS]; 2];
        self.echo_band_state = [[[0.0; 2]; MAX_CHANNELS]; 2];
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
        for f in [&mut self.low_pass, &mut self.high_pass, &mut self.in_low_pass, &mut self.in_high_pass].into_iter().flatten() {
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
        let dip = match &self.mid_dip {
            Some(d) if p > d.span.start && p < d.span.end => d.depth * (PI * d.span.progress(p)).sin(),
            _ => 0.0,
        };
        let tail = |s: Span| 1.0 - raised_cos(s.progress(p));
        let (echo_send, echo) = self.echo.as_ref().map_or((0.0, 0.0), |e| {
            let ducked = e.duck.map_or(1.0, |(s, floor)| 1.0 - (1.0 - floor) * raised_cos(s.progress(p)));
            (e.send.map_or(1.0, |s| window_gain(s, self.send_ramp, p)), tail(e.tail) * ducked)
        });
        let (verb_send, verb) = self.reverb.as_ref().map_or((0.0, 0.0), |r| (window_gain(r.send, self.send_ramp, p), tail(r.tail)));
        Fades {
            out: out * self.out_gain * db_to_gain(trim_out),
            inc: inc * db_to_gain(trim_in),
            held,
            low_out: self.swap_section(lows_out),
            low_in: self.swap_section(lows_in),
            shelf: self.hold_shelf.high(held),
            highs_in: self.highs_in.map_or_else(Coef::default, |(span, floor)| self.highs_shelf.high(floor + (1.0 - floor) * raised_cos(span.progress(p)))),
            duck,
            dip,
            echo_send,
            echo,
            verb_send,
            verb,
        }
    }

    /// The run from frame `p` (inside the transition): up to [`FADE_RUN`] frames, ending at the next fade
    /// start or end. A fade with no length steps from its start to the frame after, so that is an end too.
    fn fade_run(&self, p: u64) -> FadeRun {
        let (o, i) = (self.out_fade, self.in_fade);
        let (s, d) = (self.swap.unwrap_or_default(), self.duck.map_or_else(Span::default, |d| d.0));
        let h = self.highs_in.map_or_else(Span::default, |h| h.0);
        let m = self.mid_dip.as_ref().map_or_else(Span::default, |d| d.span);
        let none = Span::default();
        let (es, et, ed) = self.echo.as_ref().map_or((None, none, none), |e| (e.send, e.tail, e.duck.map_or(none, |d| d.0)));
        let (rs, rt) = self.reverb.as_ref().map_or((None, none), |r| (Some(r.send), r.tail));
        let ramp = self.send_ramp;
        let ramps = |w: Option<Span>| w.map_or([0; 4], |w| [w.start, w.start + ramp, w.end.saturating_sub(ramp), w.end]);
        let bends = [o.start, o.start + 1, o.end, i.start, i.start + 1, i.end, s.start, s.end, d.start, d.end, h.start, h.end, m.start, m.end, et.start, et.end, ed.start, ed.end, rt.start, rt.end];
        let end = bends.into_iter().chain(ramps(es)).chain(ramps(rs)).filter(|&b| b > p).fold((p + FADE_RUN).min(self.len), u64::min);
        FadeRun { start: p, end, from: self.fades_at(p), to: self.fades_at(end) }
    }

    /// Frame `k` of a `len`-frame brake on the outgoing deck: the frame is kept, and what plays is the outgoing audio
    /// where a read slowing as `(1 - t)^1.8` has reached (its integral), between frames; nothing after it.
    #[inline]
    fn brake_frame(&mut self, k: u64, len: u64, o: &mut [f64; MAX_CHANNELS]) {
        let ch = self.ch;
        if k >= len {
            o[..ch].fill(0.0);
            return;
        }
        let kept = k as usize * ch;
        self.brake_buf[kept..kept + ch].copy_from_slice(&o[..ch]);
        let x = len as f64 / 2.8 * (1.0 - (1.0 - k as f64 / len as f64).powf(2.8));
        let (j, f) = (x.floor() as usize, x - x.floor());
        let next = (j + 1).min(k as usize);
        for (c, v) in o[..ch].iter_mut().enumerate() {
            *v = self.brake_buf[j * ch + c] * (1.0 - f) + self.brake_buf[next * ch + c] * f;
        }
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
        if let Some((start, len)) = self.brake.filter(|(start, _)| p >= *start) {
            self.brake_frame(p - start, len, o);
        }
        let t = self.fades.t(p);
        let (g_out, g_in, held) = (self.fades.get(t, |f| f.out), self.fades.get(t, |f| f.inc), self.fades.get(t, |f| f.held));
        let (rate, entry) = (self.rate, self.sweep_entry);
        for f in [&mut self.high_pass, &mut self.low_pass].into_iter().flatten() {
            f.apply(rate, entry, p, &mut o[..ch]);
        }
        for f in [&mut self.in_high_pass, &mut self.in_low_pass].into_iter().flatten() {
            f.apply(rate, entry, p, &mut i[..ch]);
        }
        // Run on to the end once the highs are back: the shelf then passes its input, and its state settles.
        if self.highs_in.is_some() {
            let shelf = self.fades.from.highs_in.lerp(&self.fades.to.highs_in, t);
            for (x, s) in i[..ch].iter_mut().zip(&mut self.highs_state) {
                *x = shelf.run(s, *x);
            }
        }
        let duck = if self.duck.is_some() { self.fades.get(t, |f| f.duck) } else { 0.0 };
        if duck > 0.0 {
            for (x, s) in i[..ch].iter_mut().zip(&mut self.duck_state) {
                *x -= duck * self.duck_coef.run(s, *x);
            }
        }
        if let Some(m) = &self.mid_dip {
            let dip = self.fades.get(t, |f| f.dip);
            if dip > 0.0 {
                for ((deck, x), state) in [&mut *o, &mut *i].into_iter().enumerate().zip(&mut self.dip_state) {
                    if m.decks[deck] {
                        for (x, s) in x[..ch].iter_mut().zip(state) {
                            *x -= dip * m.coef.run(s, *x);
                        }
                    }
                }
            }
        }
        // The lows go to one deck at a time, so only the highs are held to one song's power, by a shelf
        // that is no filter at all once nothing is held.
        let swap = self.swap.map(|_| (self.fades.from.low_out.lerp(&self.fades.to.low_out, t), self.fades.from.low_in.lerp(&self.fades.to.low_in, t), self.fades.from.shelf.lerp(&self.fades.to.shelf, t)));
        // The outgoing deck after its fader: what the echo and the reverb are sent.
        let mut sent = [0f64; MAX_CHANNELS];
        for c in 0..ch {
            let (x, y) = (o[c], i[c]);
            o[c] = match swap {
                Some((low_out, low_in, shelf)) => {
                    let (s, h) = (&mut self.swap_state, &mut self.hold_state);
                    sent[c] = shelf.run(&mut h[0][c], low_out.run2(&mut s[0], c, x)) * g_out;
                    sent[c] + shelf.run(&mut h[1][c], low_in.run2(&mut s[1], c, y)) * g_in
                }
                None => {
                    sent[c] = x * g_out * held;
                    (x * g_out + y * g_in) * held
                }
            };
        }
        // Until the send opens the delay line holds silence, so it need not run.
        if let Some(e) = self.echo.as_ref().filter(|e| p >= e.send.map_or(0, |s| s.start)) {
            let (send, level) = (self.fades.get(t, |f| f.echo_send), self.fades.get(t, |f| f.echo) * e.wet);
            let line = &mut self.echo_buf[self.echo_pos * ch..(self.echo_pos + 1) * ch];
            for (c, (fed, x)) in line.iter_mut().zip(&mut o[..ch]).enumerate() {
                let rep = *fed;
                *fed = sent[c] * send + rep * e.feedback;
                if let Some((hp, lp)) = &e.band {
                    let [high, low] = &mut self.echo_band_state;
                    *fed = lp.run(&mut low[c], hp.run(&mut high[c], *fed));
                }
                *x += rep * level;
            }
            self.echo_pos = (self.echo_pos + 1) % e.delay;
        }
        if let Some(r) = self.reverb.as_ref().filter(|r| p >= r.send.start) {
            let send = self.fades.get(t, |f| f.verb_send) / ch as f64;
            let (l, rr) = self.room.run(r, sent[..ch].iter().sum::<f64>() * send);
            let level = self.fades.get(t, |f| f.verb) * r.wet;
            for (c, x) in o[..ch].iter_mut().enumerate() {
                *x += level * if c % 2 == 0 { l } else { rr };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Band, BassSwap, Duck, Echo, MidDip, Reverb, Sweep, VocalDuck, Window};

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
            in_rates: Vec::new(),
            out_rates: Vec::new(),
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

    /// A brake slows the outgoing song to a stop: its pitch falls each quarter, the first by the curve's integral, with
    /// no jump where it starts, and nothing of it plays after.
    #[test]
    fn brake_slows_to_a_stop() {
        let mut p = plan();
        p.duration_ms = 2000;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 2000, 2000, 2000);
        p.brake = Some(crate::types::Brake { start_ms: 500, len_ms: 1000 });
        let tone = sine(440.0, 96000);
        let y = mix(&p, &tone, &vec![0f32; 96000]);
        let crossings = |a: usize, b: usize| y[a * 48..b * 48].windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as i64;
        assert!((crossings(0, 500) - 440).abs() <= 3, "at its own speed before: {}", crossings(0, 500));
        let q: Vec<i64> = (0..4).map(|k| crossings(500 + 250 * k, 750 + 250 * k)).collect();
        // A quarter of the way the read has gone (1 - 0.75^2.8) / 2.8 of the brake's length: 0.198 of a second.
        assert!((q[0] - 174).abs() <= 4 && q.windows(2).all(|w| w[1] < w[0]), "slower each quarter: {q:?}");
        let step = y[499 * 48..501 * 48].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        assert!(step < 0.1, "no jump where it starts: {step}");
        assert!(y[1500 * 48..2000 * 48].iter().all(|v| *v == 0.0), "nothing after it");
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

    /// The amplitude of `hz` in `x`.
    fn tone_level(x: &[f32], hz: f64) -> f64 {
        let (s, c) = x.iter().enumerate().fold((0.0, 0.0), |(s, c), (k, v)| {
            let (sin, cos) = (2.0 * PI * hz * k as f64 / RATE).sin_cos();
            (s + *v as f64 * sin, c + *v as f64 * cos)
        });
        2.0 * (s * s + c * c).sqrt() / x.len() as f64
    }

    /// Deterministic white noise, ±0.5.
    fn noise(frames: usize) -> Vec<f32> {
        let mut x = 0x2545_f491_u32;
        (0..frames)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f64 / u32::MAX as f64 - 0.5) as f32
            })
            .collect()
    }

    /// `x` for its first `ms`, silent after.
    fn burst(mut x: Vec<f32>, ms: usize) -> Vec<f32> {
        x[ms * 48..].fill(0.0);
        x
    }

    /// The outgoing deck at full level to the end, the incoming one silent, so only the effects act.
    fn outgoing_only(ms: i64) -> TransitionPlan {
        let mut p = plan();
        p.duration_ms = ms;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (ms, ms, ms, ms);
        p
    }

    #[test]
    fn incoming_high_pass_lets_the_lows_in_and_switches_off() {
        let mut p = plan();
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 0, 0, 0);
        p.in_high_pass = Some(Sweep { start_ms: 0, end_ms: 600, from_hz: 1_200.0, to_hz: 20.0 });
        let (zeros, bass) = (vec![0f32; 48000], sine(60.0, 48000));
        let y = mix(&p, &zeros, &bass);
        assert!(rms(&y[4800..9600]) / rms(&bass) < 0.01, "60 Hz held out early: {}", rms(&y[4800..9600]) / rms(&bass));
        // Open at 600 ms, faded out by 650 ms: the deck is then untouched.
        assert_eq!(&y[31200..], &bass[31200..]);
        let jump = y.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        assert!(jump < 0.5 * 2.0 * std::f32::consts::PI * 60.0 / 48000.0 * 1.2, "no clicks: {jump}");
    }

    /// BitChord's ride: the outgoing deck low-passed from 7 kHz to 300 Hz while the incoming one is high-passed from
    /// 1.2 kHz open, both at once. Each deck's band leaves or arrives, and no tone near a moving corner is lifted.
    #[test]
    fn filter_ride_runs_both_decks_without_a_lift() {
        let mut p = plan();
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (1000, 1000, 0, 0);
        p.low_pass = Some(Sweep { start_ms: 0, end_ms: 800, from_hz: 7_000.0, to_hz: 300.0 });
        p.in_high_pass = Some(Sweep { start_ms: 0, end_ms: 600, from_hz: 1_200.0, to_hz: 20.0 });
        let (hats, bass) = (sine(4_000.0, 48000), sine(60.0, 48000));
        let y = mix(&p, &hats, &bass);
        // Both decks up hold one song's power.
        let full = 0.5 * std::f64::consts::FRAC_1_SQRT_2;
        let (early, late) = (&y[4800..7200], &y[43200..48000]);
        assert!(tone_level(early, 4_000.0) > 0.3 * full && tone_level(early, 60.0) < 0.01 * full, "the outgoing highs, no incoming bass");
        assert!(tone_level(late, 4_000.0) < 0.01 * full && (tone_level(late, 60.0) / full - 1.0).abs() < 0.02, "the incoming bass, no outgoing highs");
        let zeros = vec![0f32; 48000];
        for (hz, incoming) in [(300.0, true), (700.0, true), (1_200.0, true), (2_500.0, true), (300.0, false), (1_000.0, false), (3_000.0, false), (6_000.0, false)] {
            let tone = sine(hz, 48000);
            let y = if incoming { mix(&p, &zeros, &tone) } else { mix(&p, &tone, &zeros) };
            let most = y.chunks(480).map(rms).fold(0.0, f64::max) / (rms(&tone) * std::f64::consts::FRAC_1_SQRT_2);
            assert!(most < 1.01, "{hz} Hz on the {} deck rises to {most:.3}", if incoming { "incoming" } else { "outgoing" });
        }
    }

    #[test]
    fn mid_dip_is_a_half_sine_on_the_named_decks() {
        let zeros = vec![0f32; 48000];
        let tone = sine(1_000.0, 48000);
        for (outgoing, incoming, on_outgoing) in [(false, true, false), (true, false, true), (true, true, false)] {
            let mut p = plan();
            // One deck at full level throughout.
            let ms = if on_outgoing { 1000 } else { 0 };
            (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (ms, ms, ms, ms);
            p.mid_dip = Some(MidDip { window: Window { start_ms: 0, end_ms: 1000 }, db: -6.0, hz: 1_000.0, outgoing, incoming });
            let y = if on_outgoing { mix(&p, &tone, &zeros) } else { mix(&p, &zeros, &tone) };
            let db = |a: usize, b: usize| 20.0 * (rms(&y[a..b]) / rms(&tone[a..b])).log10();
            let dipped = if (on_outgoing && outgoing) || (!on_outgoing && incoming) { -6.0 } else { 0.0 };
            assert!((db(22800, 25200) - dipped).abs() < 0.3, "{outgoing} {incoming}: {:.2} dB in the middle", db(22800, 25200));
            assert!(db(0, 240).abs() < 0.2 && db(47760, 48000).abs() < 0.2, "{outgoing} {incoming}: none at the ends");
        }
    }

    /// One delay, a little feedback, the repeats at full level.
    fn echo(send: Option<Window>, band: Option<Band>, duck: Option<Duck>) -> Option<Echo> {
        Some(Echo { delay_ms: 250, feedback: 0.5, wet_db: 0.0, send, band, duck })
    }

    #[test]
    fn echo_repeats_only_what_its_window_sends() {
        let mut p = outgoing_only(2000);
        p.echo = echo(Some(Window { start_ms: 0, end_ms: 300 }), None, None);
        let click = |ms: usize| {
            let mut x = vec![0f32; 96000];
            x[ms * 48] = 1.0;
            x
        };
        let y = mix(&p, &click(100), &vec![0f32; 96000]);
        assert!((y[16800] - 1.0).abs() < 1e-6 && (y[28800] - 0.5).abs() < 1e-6, "sent inside: repeated at 350 and 600 ms");
        let y = mix(&p, &click(500), &vec![0f32; 96000]);
        assert!(y[24001..].iter().all(|v| *v == 0.0), "sent outside: never repeated");
    }

    /// The repeats of a 250 ms tone, against an echo with no band.
    fn kept_by_band(hz: f64, band: Band) -> f64 {
        let p = outgoing_only(2000);
        let tone = burst(sine(hz, 96000), 250);
        let repeats = |band| rms(&mix(&TransitionPlan { echo: echo(None, band, None), ..p.clone() }, &tone, &vec![0f32; 96000])[14400..62400]);
        repeats(Some(band)) / repeats(None)
    }

    #[test]
    fn band_keeps_the_repeats_in_the_mids() {
        let band = Band { low_hz: 250.0, high_hz: 3_500.0 };
        for (hz, kept) in [(60.0, false), (1_000.0, true), (10_000.0, false)] {
            let k = kept_by_band(hz, band);
            assert!(if kept { k > 0.9 } else { k < 0.1 }, "{hz} Hz: {k:.3} of the repeats kept");
        }
    }

    #[test]
    fn duck_holds_the_repeats_down() {
        let mut p = outgoing_only(3000);
        p.echo = Some(Echo { feedback: 0.8, ..echo(None, None, None).unwrap() });
        let tone = burst(sine(1_000.0, 144000), 250);
        let zeros = vec![0f32; 144000];
        let free = mix(&p, &tone, &zeros);
        p.echo = Some(Echo { duck: Some(Duck { at_ms: 1000, len_ms: 100, db: -12.0 }), ..p.echo.unwrap() });
        let ducked = mix(&p, &tone, &zeros);
        let ratio = |a: usize, b: usize| rms(&ducked[a * 48..b * 48]) / rms(&free[a * 48..b * 48]);
        assert!((ratio(500, 950) - 1.0).abs() < 1e-6, "untouched before: {}", ratio(500, 950));
        assert!((ratio(1100, 2500) - db_to_gain(-12.0)).abs() < 0.005, "12 dB down after: {}", ratio(1100, 2500));
    }

    /// An echo-out hands the bass over like any other mix: the incoming bass comes in at the swap, and the outgoing
    /// lows taken away before the echo are not repeated, while its mids are.
    #[test]
    fn echo_and_bass_swap_work_together() {
        let mut p = plan();
        p.duration_ms = 2000;
        (p.out_fade_start_ms, p.out_fade_end_ms, p.in_fade_start_ms, p.in_fade_end_ms) = (0, 500, 0, 0);
        p.echo = echo(None, None, None);
        p.bass_swap = Some(BassSwap { at_ms: 1000, len_ms: 100, cut_hz: 200.0 });
        let zeros = vec![0f32; 96000];
        let bass = sine(50.0, 96000);
        let y = mix(&p, &zeros, &bass);
        assert!(rms(&y[9600..43200]) / rms(&bass) < 0.03, "the incoming bass waits for the swap");
        assert!((rms(&y[57600..91200]) / rms(&bass) - 1.0).abs() < 0.02, "and comes in after it");
        // The swap at the start: the outgoing lows are gone before they reach the echo, its mids are not.
        let swapped = TransitionPlan { bass_swap: Some(BassSwap { at_ms: 0, len_ms: 1, cut_hz: 200.0 }), ..p.clone() };
        let unswapped = TransitionPlan { bass_swap: None, ..p };
        let kept = |x: &[f32]| rms(&mix(&swapped, x, &zeros)[28800..43200]) / rms(&mix(&unswapped, x, &zeros)[28800..43200]);
        assert!(kept(&bass) < 0.05, "{} of the outgoing bass repeated", kept(&bass));
        assert!(kept(&sine(1_000.0, 96000)) > 0.9, "{} of the outgoing mids repeated", kept(&sine(1_000.0, 96000)));
    }

    fn reverb(send: Window, decay_s: f32) -> Option<Reverb> {
        Some(Reverb { send, decay_s, wet_db: 0.0, low_cut_hz: 100.0 })
    }

    /// `x` through a band-pass around `hz`, about an octave wide.
    fn band_of(x: &[f32], hz: f64) -> Vec<f32> {
        let (c, mut s) = (Coef::band_pass(RATE, hz, 1.4), [0.0; 2]);
        x.iter().map(|v| c.run(&mut s, *v as f64) as f32).collect()
    }

    /// An impulse's reverb falls 60 dB in the decay time, in the octave around 1 kHz (the highs are damped, so they
    /// fall sooner): the Schroeder integral of its tail, from 5 to 25 dB down, tripled.
    #[test]
    fn reverb_decays_60_db_in_its_decay_time() {
        for decay in [1.0f32, 2.5] {
            let mut p = outgoing_only(6000);
            p.reverb = reverb(Window { start_ms: 0, end_ms: 6000 }, decay);
            let mut click = vec![0f32; 288000];
            click[1000] = 1.0;
            let mut y = mix(&p, &click, &vec![0f32; 288000]);
            y[1000] = 0.0;
            // Up to the tail's fade.
            let y = band_of(&y[..288000 - (decay as f64 * 250.0 * 48.0) as usize], 1_000.0);
            let tail = &y[1000..];
            let mut left: Vec<f64> = tail.iter().rev().scan(0.0, |e, v| {
                *e += (*v as f64).powi(2);
                Some(*e)
            }).collect();
            left.reverse();
            let at = |db: f64| left.iter().position(|e| 10.0 * (e / left[0]).log10() <= db).unwrap() as f64 / RATE;
            let rt60 = 3.0 * (at(-25.0) - at(-5.0));
            assert!((rt60 / decay as f64 - 1.0).abs() < 0.15, "{decay} s: {rt60:.2} s to 60 dB down");
        }
    }

    #[test]
    fn reverb_rings_on_past_its_send_and_fades_by_the_end() {
        let mut p = outgoing_only(3000);
        p.reverb = reverb(Window { start_ms: 0, end_ms: 300 }, 2.0);
        let hit = burst(noise(144000), 300);
        let y = mix(&p, &hit, &vec![0f32; 144000]);
        let level = |a: usize, b: usize| rms(&y[a * 48..b * 48]) / rms(&hit[..14400]);
        assert!(level(1000, 1100) > 0.01, "rings on after the send closes: {}", level(1000, 1100));
        assert!(level(2995, 3000) < 1e-3 * level(1000, 1100), "faded out by the end: {}", level(2995, 3000));
        // What was played after the send closed is not in it: the same ring with the deck playing on.
        let on = mix(&p, &noise(144000), &vec![0f32; 144000]);
        let dry = noise(144000);
        let ring_on: Vec<f32> = on[48000..52800].iter().zip(&dry[48000..52800]).map(|(y, x)| y - x).collect();
        assert!((rms(&ring_on) / rms(&y[48000..52800]) - 1.0).abs() < 1e-3, "only the send is reverberated");
    }

    #[test]
    fn reverb_is_wet_db_under_a_steady_send_without_its_lows() {
        let mut p = outgoing_only(6000);
        p.reverb = Some(Reverb { send: Window { start_ms: 0, end_ms: 6000 }, decay_s: 1.0, wet_db: -6.0, low_cut_hz: 250.0 });
        let zeros = vec![0f32; 288000];
        let wet = |x: &[f32]| {
            let y = mix(&p, x, &zeros);
            let ring: Vec<f32> = y[96000..192000].iter().zip(&x[96000..192000]).map(|(y, x)| y - x).collect();
            rms(&ring) / rms(&x[96000..192000])
        };
        // Mids, which the damping hardly touches.
        let db = 20.0 * wet(&band_of(&noise(288000), 1_000.0)).log10();
        assert!((db + 6.0).abs() < 1.0, "noise reverberated at {db:.1} dB");
        let (low, mid) = (wet(&sine(50.0, 288000)), wet(&sine(1_000.0, 288000)));
        assert!(low < 0.1 * mid, "50 Hz rings {low:.3}, 1 kHz {mid:.3}");
    }

    /// One send in, two outputs that hardly correlate: the reverb is wide.
    #[test]
    fn reverb_is_stereo() {
        let mut p = outgoing_only(3000);
        p.reverb = reverb(Window { start_ms: 0, end_ms: 300 }, 2.0);
        let mono = burst(noise(144000), 300);
        let stereo: Vec<f32> = mono.iter().flat_map(|v| [*v, *v]).collect();
        let mut m = Mixer::new(RATE as u32, 2);
        m.configure(&p);
        let mut y = stereo.clone();
        m.process(&mut y, &vec![0f32; 288000]);
        let (l, r): (Vec<f64>, Vec<f64>) = y[48000 * 2..96000 * 2].chunks(2).map(|f| (f[0] as f64, f[1] as f64)).unzip();
        let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
        let corr = dot(&l, &r) / (dot(&l, &l) * dot(&r, &r)).sqrt();
        assert!(corr.abs() < 0.3 && dot(&l, &l) > 0.0, "left and right correlate {corr:.2}");
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
            ("bass swap, trimmed", TransitionPlan { bass_swap: Some(BassSwap { at_ms: 4_000, len_ms: 2_000, cut_hz: 180.0 }), ..trimmed.clone() }),
            ("vocal duck", TransitionPlan { vocal_duck: Some(VocalDuck { until_ms: 8_000, release_ms: 3_000, db: -9.0, hz: 1_500.0 }), ..trimmed.clone() }),
            ("echo", TransitionPlan { echo: Some(Echo { delay_ms: 500, feedback: 0.4, wet_db: -6.0, send: None, band: None, duck: None }), ..trimmed.clone() }),
            ("sweeps", TransitionPlan { low_pass: Some(Sweep { start_ms: 2_000, end_ms: 10_000, from_hz: 18_000.0, to_hz: 300.0 }), high_pass: Some(Sweep { start_ms: 0, end_ms: 12_000, from_hz: 20.0, to_hz: 400.0 }), ..trimmed.clone() }),
            ("filter ride", TransitionPlan { low_pass: Some(Sweep { start_ms: 0, end_ms: 12_000, from_hz: 7_000.0, to_hz: 300.0 }), in_high_pass: Some(Sweep { start_ms: 0, end_ms: 9_000, from_hz: 1_200.0, to_hz: 20.0 }), ..trimmed.clone() }),
            ("mid dip", TransitionPlan { mid_dip: Some(MidDip { window: Window { start_ms: 0, end_ms: 12_000 }, db: -6.0, hz: 1_000.0, outgoing: true, incoming: true }), ..trimmed.clone() }),
            ("echo, band, duck, swap", TransitionPlan { echo: Some(Echo { delay_ms: 500, feedback: 0.5, wet_db: -6.0, send: None, band: Some(Band { low_hz: 250.0, high_hz: 3_500.0 }), duck: Some(Duck { at_ms: 8_000, len_ms: 500, db: -12.0 }) }), bass_swap: Some(BassSwap { at_ms: 4_000, len_ms: 2_000, cut_hz: 180.0 }), ..trimmed.clone() }),
            ("reverb", TransitionPlan { reverb: Some(Reverb { send: Window { start_ms: 0, end_ms: 12_000 }, decay_s: 3.5, wet_db: -3.0, low_cut_hz: 250.0 }), ..trimmed }),
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
