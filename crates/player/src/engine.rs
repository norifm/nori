//! Transitions between tracks inside one output stream: crossfades and AutoMix (beat-matched, with
//! bass swap, filter sweep and tempo stretch). Sits between the decoder and the output ([`Downstream`]).
//!
//! Audio passes through until the outgoing track reaches the plan's start, then is held (up to the
//! transition's length; the rest of the ending is dropped). When the next track begins, its opening is
//! mixed into the hold.
//!
//! The output format is latched by the first PCM stream and other streams are resampled to it. With
//! [`TransitionEngine::follow_rate`], a stream nothing is mixed into reopens the output at its own
//! format instead, unless the converter can carry on seamlessly from the previous stream. With
//! [`TransitionEngine::lock_rate`] off (bit-perfect) every stream passes through native.
//!
//! Decoded audio of unanalysed tracks is fed to the streaming analyser. During a transition the
//! reported position runs ahead of what is audible, which is tracked in [`Heard`].

use std::collections::VecDeque;
use std::sync::Arc;

use crate::automix::analysis::Analyzer;
use crate::stage::{no_facts, SongFacts};
use crate::automix::mixer::Mixer;
use crate::types::TransitionPlan;
use crate::automix::resample::{Resampler, Tables};
use crate::pcm::{ByteStretcher, Format};

/// Output left below this lets a held ending go unmixed.
const DRY_US: i64 = 1_500_000;
/// A hold younger than this is exempt from [`DRY_US`] (a seek into a transition starts with no runway).
const HOLD_GRACE_MS: i64 = 10_000;
/// Maximum gain applied to a song's buffers ([`TransitionEngine::set_gain`]): +24 dB.
const MAX_GAIN: f32 = 16.0;
/// How long a `None` plan is trusted before asking again.
const NULL_PLAN_RETRY_MS: i64 = 2_000;

/// Song time covered by `frames` of output at `pace` song frames each, µs.
fn span_us(frames: usize, pace: f64, out: Format) -> i64 {
    (frames as f64 * pace * 1_000_000.0 / out.rate as f64).round() as i64
}

/// A transition from one track into the next, from the planner. Equal when it sounds the same, so a
/// new plan that only changes log text does not rebuild the ending.
#[derive(Debug, Clone)]
pub struct Plan {
    pub incoming_id: String,
    /// What the mixer runs.
    pub mixer: TransitionPlan,
}

impl PartialEq for Plan {
    fn eq(&self, o: &Plan) -> bool {
        self.incoming_id == o.incoming_id && self.mixer.sounds_same(&o.mixer)
    }
}

impl Plan {
    pub fn out_start_us(&self) -> i64 {
        self.mixer.out_start_ms * 1000
    }
    pub fn duration_us(&self) -> i64 {
        self.mixer.duration_ms * 1000
    }
    /// Outgoing song time the transition plays, µs: its length, but under a bend.
    pub fn held_us(&self) -> i64 {
        (self.mixer.out_song_ms(self.mixer.duration_ms as f64) * 1000.0).round() as i64
    }
    fn in_skip_us(&self) -> i64 {
        self.mixer.in_start_ms * 1000
    }
    fn tempo_ratio(&self) -> f32 {
        self.mixer.tempo_ratio as f32
    }
    fn ramp_us(&self) -> i64 {
        self.mixer.tempo_ramp_ms * 1000
    }
    /// Capture this much outgoing audio and loop it for the duration; 0 captures the full duration.
    pub fn out_loop_us(&self) -> i64 {
        self.mixer.out_loop_ms.unwrap_or(0) * 1000
    }
    fn stretching(&self) -> bool {
        (self.tempo_ratio() - 1.0).abs() > 1e-4
    }
}

/// A stream the decoder announces: its song, and a serial new for each stream, so the same song twice in
/// a row (queued twice, repeat one) is two streams. The engine tells streams apart by serial; the song
/// is only for the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamId {
    pub song: String,
    pub serial: u64,
}

/// A stream's format as the decoder announces it.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamFormat {
    pub id: StreamId,
    pub format: Format,
}

/// A stream's song for logs.
fn song(id: &Option<StreamId>) -> &str {
    id.as_ref().map_or("-", |s| s.song.as_str())
}

/// The real output below. Called only from the engine's thread.
pub trait Downstream {
    /// Opens the output at `format`.
    fn configure(&mut self, format: Format);
    /// Offers `data[from..]` at `pts_us`; returns (all taken, bytes taken). A partly taken buffer is
    /// offered again as the same memory with `from` advanced, so an output can check it.
    fn handle_buffer(&mut self, data: &[u8], from: usize, pts_us: i64) -> (bool, usize);
    fn handle_discontinuity(&mut self);
    /// Whether it scales what is offered by [`Downstream::song_gain`] itself, in a sound chain it runs
    /// anyway: the samples are rounded once instead of twice.
    fn applies_gain(&self) -> bool {
        false
    }
    /// The gain still to be applied to what is offered from now on; 1 when it is in the samples.
    fn song_gain(&mut self, _gain: f32) {}
    /// What the stages read of the song offered from now on (in a mix, the incoming song).
    fn song_facts(&mut self, _song: &Arc<SongFacts>) {}
    /// Song frames per output frame from now on (not 1 while the incoming song is stretched), so the
    /// output's clock counts song time.
    fn media_pace(&mut self, _pace: f64) {}
    /// Playback position, µs; `None` before there is one.
    fn position_us(&mut self, source_ended: bool) -> Option<i64>;
}

/// What the engine asks of the platform. Called on the engine's thread; must be quick.
pub trait Host {
    /// The transition out of `outgoing_id`, or `None` for gapless.
    fn plan_for(&mut self, outgoing_id: &str) -> Option<Plan>;
    /// `Some(length in ms, 0 if unknown)` if `song_id` still needs analysing; sizes the analyser up front.
    fn wants_analysis(&mut self, song_id: &str) -> Option<u64>;
    /// The analyser finished `song_id`: `frames` at `rate`, `channels` wide.
    fn analysed(&mut self, song_id: &str, analyzer: Analyzer, channels: usize, frames: u64, rate: u32);
    /// [`Heard`] started or stopped differing from the player's position, or its mixing flag changed.
    fn heard_changed(&mut self) {}
    fn log(&mut self, _message: &str) {}
    /// Monotonic clock, ms.
    fn now_ms(&self) -> i64;
}

/// What is audible while the reported position runs ahead of it: the stream whose held ending plays and
/// the position in it (song time). The seek bar and title show this.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Heard {
    /// The held stream's serial; `None` whenever the player's position is what is audible.
    pub id: Option<u64>,
    pub us: i64,
    /// [`Host::now_ms`] when `us` was read.
    pub at_ms: i64,
    /// Position in the held song where the next song becomes the louder.
    pub until_us: i64,
    /// A mix is audible now.
    pub mixing: bool,
    /// Where playback lands in the next stream at takeover, and its tempo ratio in the mix.
    pub next_from_us: i64,
    pub next_rate: f32,
    /// The serial of the stream mixed out of (a mix into the stream after it is planned or audible),
    /// and where in it the next stream takes over.
    pub from: Option<u64>,
    pub audible_us: i64,
}

/// What a stream's ending is to be.
// One per engine, inline: boxing the plan would allocate on the audio thread.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Default)]
enum Ending {
    /// To be asked for: first, or again after the settings, the queue or an analysis changed.
    #[default]
    Ask,
    /// Gapless, as the planner said at this ms; asked again after [`NULL_PLAN_RETRY_MS`].
    Gapless(i64),
    Planned(Plan),
    /// Let play as it is (no runway to hold it): not planned again.
    LetGo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Pass,
    Hold(Hold),
    /// Mixing the incoming song into the first `len` bytes of the tail.
    Mix { len: usize, out: MixOut },
}

/// An ending being held into the tail, waiting for the next song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hold {
    /// Bytes held, of at most `limit` (what does not fit is skipped by the plan).
    len: usize,
    limit: usize,
    /// Outgoing audio swallowed, µs; reported as played.
    held_us: i64,
    /// [`Host::now_ms`] when the hold began.
    at_ms: i64,
    /// How far into the transition the hold began (non-zero after a seek into it).
    late_us: i64,
}

/// How a mix reads the held ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixOut {
    /// Once through, `read` bytes in.
    Once { read: usize },
    /// An outro loop: at output frame `frame` of `frames`, the last `loop_frames` of the hold repeating.
    Looped { frame: usize, frames: usize, loop_frames: usize },
    /// Under a bend: `read` bytes of the ending in (its last `loop_bytes` repeating, 0 for no loop), at output frame
    /// `frame` of `frames`, through the outgoing stretcher from [`OutGlide::from`].
    Bent { read: usize, frame: usize, frames: usize, loop_bytes: usize },
}

/// Where byte `v` of an ending read through is in the hold (the first `len` bytes, its last `lp` repeating; 0
/// for no loop), and where that run of it ends.
fn ending_run(v: usize, len: usize, lp: usize) -> (usize, usize) {
    let prefix = len - lp.min(len);
    if lp == 0 || v < prefix {
        (v.min(len), if lp == 0 { len } else { prefix })
    } else {
        let lp = lp.min(len);
        (prefix + (v - prefix) % lp, prefix + lp)
    }
}

/// Fed to the outgoing stretcher past the end of what was held.
static SILENCE: [u8; 4096] = [0; 4096];

/// Where the outgoing song's glide starts, output frames into the mix, and its schedule
/// ([`ByteStretcher::glide`]).
#[derive(Clone, Copy)]
struct OutGlide {
    from: usize,
    hold: u64,
    ramp: u64,
    to: f64,
    started: bool,
}

/// The incoming song's opening looped: its first `frames` of output are kept as they play, then played again
/// for `left` frames; what came after them waits in the stash. All of it carries song time at `pace`.
#[derive(Clone, Copy)]
struct Repeat {
    frames: usize,
    captured: usize,
    left: usize,
    pace: f64,
}

#[derive(Default)]
enum Landing {
    #[default]
    None,
    Track,
    Mix(i64),
}
/// A queued piece of output.
struct Chunk {
    data: Vec<u8>,
    pos: usize,
    pts_us: i64,
    resync: bool,
    /// First chunk of a mix: measure the output clock's jump when it is offered ([`TransitionEngine::drain`]).
    measure: bool,
    /// Stream offset of the single song it holds (`None` for a mix), for [`TransitionEngine::rescale`].
    stream_us: Option<i64>,
    /// Song frames per frame ([`Downstream::media_pace`]).
    pace: f64,
    /// The song's gain, still to be applied by the output ([`Downstream::song_gain`]); 1 when the
    /// samples are at it.
    gain: f32,
    /// What the stages read of its song ([`Downstream::song_facts`]).
    song: Arc<SongFacts>,
}

/// Converts an incoming format to the output's.
struct Converter {
    resampler: Resampler,
    from: Format,
    /// The stream it is armed for.
    id: Option<StreamId>,
}

/// The incoming song's stretcher in its own format (the incoming one while converting).
struct Stretch {
    s: ByteStretcher,
    format: Format,
}

/// The engine's own clock for stretched or looped audio, in the incoming song's time.
#[derive(Clone, Copy)]
enum Clock {
    /// Stream time of the first incoming sample (after the skip), nothing stamped yet.
    From(i64),
    /// Where the next output is stamped.
    At(i64),
}

/// The analyser on the stream that arrives.
// Inline: boxing would allocate on the audio thread.
#[allow(clippy::large_enum_variant)]
enum Analysis {
    /// The host is asked at the stream's first buffer.
    Ask,
    /// Not wanted, or a seek broke the stream's continuity.
    Off,
    /// Analysing, `channels` wide at `rate`.
    On { analyzer: Analyzer, channels: usize, rate: u32 },
}

/// Until the output's clock reaches `until_us`, audible = clock - `us`.
#[derive(Clone, Copy)]
struct Shift {
    us: i64,
    until_us: i64,
}

/// A format announced by decode-ahead, armed once its buffers flow.
struct Staged {
    id: Option<StreamId>,
    format: Format,
}

/// The transition state a flush or seek starts afresh.
#[derive(Default)]
struct Flight {
    /// A discontinuity without a mix and nothing flowed since: the next new stream is the one about to
    /// flow (media3 announces a stream with its first buffer, after the discontinuity).
    awaiting_stream: bool,
    phase: Phase,
    /// The ending of stream `plan_for`.
    ending: Ending,
    plan_for: Option<StreamId>,
    /// The serial of the last stream whose ending went out, and its plan (`None`: gapless).
    made: Option<(u64, Option<Plan>)>,
    /// Output timestamp the hold began at.
    held_from_us: Option<i64>,
    /// The held stream.
    held_id: Option<StreamId>,
    /// Time from the mix becoming audible to the incoming song becoming the louder (`mixer::crossover_ms`
    /// less how late the hold began), and the outgoing song time played by then.
    takeover_us: i64,
    takeover_out_us: i64,
    /// The output's clock at the last [`TransitionEngine::position_us`].
    clock_us: Option<i64>,
    /// The output's clock jumps to the incoming song's time when the first mixed chunk is offered.
    shift: Option<Shift>,
    /// Output timestamp the queued mix runs to.
    mixed_end_us: Option<i64>,
    /// Output timestamp the mix is audible from.
    mix_from_us: Option<i64>,
    skip_left: usize,
    /// The incoming deck's silence before its first sample (`in_delay_ms`) still to mix, bytes of output, and the
    /// song time it is stamped from.
    lead_left: usize,
    lead_at_us: i64,
    resync_next: bool,
    measure_next: bool,
    /// The incoming song stretched, through the mix and its tempo ramp.
    stretch: Option<Stretch>,
    /// Where stretched or looped incoming audio is stamped.
    clock: Option<Clock>,
    /// The incoming opening looped.
    repeat: Option<Repeat>,
    /// The outgoing song's glide under a bend.
    out_glide: Option<OutGlide>,
    /// Built with the plan (keep_pitch), used when the next track begins; see `prepare`.
    pending_stretch: Option<(ByteStretcher, bool)>,
    /// The stream the mix is consuming.
    mix_source_id: Option<StreamId>,
}

pub struct TransitionEngine {
    /// The output format; `None` before the first stream.
    out: Option<Format>,
    /// Applied to the output once what is queued has drained.
    pending_format: Option<Format>,

    /// The stream of the last configure (may be decode-ahead).
    current_id: Option<StreamId>,
    /// The stream flowing now, from the last discontinuity; never from decode-ahead.
    playing_id: Option<StreamId>,
    /// Flushed and nothing has flowed since: the next configure is the stream about to play.
    fresh: bool,
    /// Flushed and no buffer has passed since: the next one is where a seek or a remake landed.
    landing: Landing,
    offset_us: i64,
    /// What a flush drops.
    flight: Flight,

    tail: Vec<u8>,
    /// The held stream's stream offset.
    held_offset_us: i64,
    /// The last position reported; never goes backwards. `i64::MIN` before any.
    reported: i64,
    /// Song frames per frame of the stretcher's last output.
    stretch_pace: f64,

    /// The mixer with the rate and channel count it was built for.
    mixer: Option<(Mixer, u32, usize)>,
    loop_buf: Vec<u8>,
    /// The outgoing song's stretcher for a bend, with the pitch setting and format it was built for (in `prepare`).
    out_stretch: Option<(ByteStretcher, bool, Format)>,
    /// The incoming loop's slice, and the incoming audio after it waiting for the repeats.
    loop_in: Vec<u8>,
    stash: Vec<u8>,

    /// The streaming analyser on the stream of `current_id`.
    analysis: Analysis,
    analysis_buf: Vec<f32>,

    converter: Option<Converter>,
    tables: Tables,
    staged: VecDeque<Staged>,
    /// Keep the latched output format. False while the output is bit-perfect: streams pass native.
    pub lock_rate: bool,
    /// Reopen the output at the format of a stream that nothing is mixed into instead of converting it.
    /// Off in a bare engine; the player turns it on.
    pub follow_rate: bool,

    /// Gain of the song whose buffers arrive now (ReplayGain); see [`TransitionEngine::set_gain`].
    gain: f32,
    /// What the stages read of the song whose buffers arrive now, and of the held one.
    song: Arc<SongFacts>,
    held_song: Arc<SongFacts>,
    dither: crate::dither::Dither,

    queue: VecDeque<Chunk>,
    pool: Vec<Vec<u8>>,

    heard: Heard,
    /// Where the plans come from.
    pub plans: Plans,
}

/// Where a transition's plan comes from: the host's planner, or a leader's word (a jam guest listening
/// along plays the transitions its host planned: (outgoing song, plan); a song it has no word for yet
/// ends gapless until it has).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Plans {
    #[default]
    Own,
    Led(Option<(String, Box<Plan>)>),
}

impl Plans {
    /// The plan out of `outgoing_id`.
    pub fn plan_for<H: Host>(&self, host: &mut H, outgoing_id: &str) -> Option<Plan> {
        match self {
            Plans::Own => host.plan_for(outgoing_id),
            Plans::Led(p) => p.as_ref().filter(|(id, _)| id == outgoing_id).map(|(_, p)| (**p).clone()),
        }
    }
}

impl Default for TransitionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TransitionEngine {
    pub fn new() -> Self {
        TransitionEngine {
            out: None,
            pending_format: None,
            current_id: None,
            playing_id: None,
            fresh: false,
            landing: Landing::None,
            offset_us: 0,
            flight: Flight::default(),
            tail: Vec::new(),
            held_offset_us: 0,
            reported: i64::MIN,
            stretch_pace: 1.0,
            mixer: None,
            loop_buf: Vec::new(),
            out_stretch: None,
            loop_in: Vec::new(),
            stash: Vec::new(),
            analysis: Analysis::Ask,
            analysis_buf: Vec::new(),
            converter: None,
            tables: Tables::default(),
            staged: VecDeque::new(),
            lock_rate: true,
            follow_rate: false,
            gain: 1.0,
            song: no_facts(),
            held_song: no_facts(),
            dither: crate::dither::Dither::new(),
            queue: VecDeque::new(),
            pool: Vec::new(),
            heard: Heard { until_us: i64::MAX, audible_us: i64::MAX, next_rate: 1.0, ..Default::default() },
            plans: Plans::Own,
        }
    }

    pub fn heard(&self) -> &Heard {
        &self.heard
    }

    /// Asks for the playing track's plan again at the next buffer (e.g. a new analysis arrived). A held
    /// ending keeps its plan, and one let go stays let go.
    pub fn replan(&mut self) {
        if self.flight.phase == Phase::Pass && !matches!(self.flight.ending, Ending::LetGo) {
            self.flight.ending = Ending::Ask;
        }
    }

    /// The plan for the ending of stream `plan_for`.
    fn plan(&self) -> Option<&Plan> {
        match &self.flight.ending {
            Ending::Planned(p) => Some(p),
            _ => None,
        }
    }

    /// How the ending of stream `serial` was made: `Some(plan)` once its hold began or its last buffer
    /// went out (inner `None`: gapless); `None` while its ending is still to come.
    pub fn made(&self, serial: u64) -> Option<Option<&Plan>> {
        if self.holding() && self.flight.plan_for.as_ref().is_some_and(|p| p.serial == serial) {
            return Some(self.plan());
        }
        self.flight.made.as_ref().filter(|(m, _)| *m == serial).map(|(_, p)| p.as_ref())
    }

    /// The playing song's ending is held, waiting for the next song.
    pub fn holding(&self) -> bool {
        matches!(self.flight.phase, Phase::Hold(_))
    }

    /// Outgoing audio swallowed into the hold, µs.
    fn held_us(&self) -> i64 {
        match self.flight.phase {
            Phase::Hold(h) => h.held_us,
            _ => 0,
        }
    }

    /// State summary for diagnostics.
    pub fn words(&self) -> String {
        let phase = match self.flight.phase {
            Phase::Pass => "passing",
            Phase::Hold(_) => "holding an ending",
            Phase::Mix { .. } => "mixing",
        };
        let mut w = format!("{phase}, flowing {}, arriving {}", song(&self.playing_id), song(&self.current_id));
        if let Some(p) = self.plan() {
            w.push_str(&format!(", plan {} -> {} from {} ms for {} ms", song(&self.flight.plan_for), p.incoming_id, p.out_start_us() / 1000, p.duration_us() / 1000));
        }
        if let Phase::Hold(h) = self.flight.phase {
            w.push_str(&format!(", held {} of {} of {} ({} ms)", h.len, h.limit, song(&self.flight.held_id), h.held_us / 1000));
        }
        if self.fresh {
            w.push_str(", fresh");
        }
        let queued: usize = self.queue.iter().map(|c| c.data.len() - c.pos).sum();
        w.push_str(&format!(", {} chunks ({queued} bytes) for the output", self.queue.len()));
        w
    }

    // ---- configuration ----

    /// The player announces the stream whose buffers come next.
    pub fn configure<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, stream: StreamFormat) {
        let StreamFormat { id: sid, format: f } = stream;
        let id = Some(sid.clone());
        host.log(&format!("sink: {} {} Hz x{} {:?}", song(&id), f.rate, f.channels, f.encoding));
        // After a flush, or after a boundary nothing has flowed across yet, the announced stream is the
        // one about to flow: it must be armed now (staged, it would never be armed and play unconverted at
        // the wrong rate).
        let new_stream = id != self.current_id;
        let awaited = self.flight.awaiting_stream && new_stream;
        let for_current = !new_stream || self.fresh || awaited;
        if self.fresh || awaited {
            self.fresh = false;
            self.flight.awaiting_stream = false;
            self.playing_id = id.clone();
        }
        if new_stream {
            self.on_new_stream(host, sid);
        }
        if !self.lock_rate {
            // Bit-perfect: the output follows the native format.
            self.converter = None;
            if self.out != Some(f) {
                self.abandon_transition(host);
                self.pending_format = Some(f);
            }
            return;
        }
        let Some(out) = self.out else {
            self.apply(down, f);
            host.log(&format!("sink pins {} Hz x{} for the queue", f.rate, f.channels));
            return;
        };
        if self.flight.phase != Phase::Pass || !for_current {
            // Decode-ahead: never interrupts a mix; armed once its buffers flow.
            self.staged.push_back(Staged { id, format: f });
        } else if f == out {
            // Already the output's format: never reconfigure the output for it.
            self.converter = None;
        } else if let Some(c) = self.converter.as_mut().filter(|c| c.from == f) {
            c.id = id;
        } else if self.follow_rate {
            self.follow(host, f);
        } else if !self.arm_conversion(host, id, f) {
            self.abandon_transition(host);
            self.pending_format = Some(f);
        }
    }

    /// Converts stream `id` (format `src`) to the output format. Drops a stretcher built in `prepare`,
    /// since the stretcher now works in `src`.
    fn arm_conversion<H: Host>(&mut self, host: &mut H, id: Option<StreamId>, src: Format) -> bool {
        let Some(out) = self.out else { return false };
        self.converter = Resampler::new(&mut self.tables, src.rate as i32, src.channels as i32, out.rate as i32, out.channels as i32).map(|resampler| Converter { resampler, from: src, id });
        if self.converter.is_none() {
            return false;
        }
        self.flight.pending_stretch = None;
        host.log(&format!("converting {} Hz x{} -> {} Hz x{}", src.rate, src.channels, out.rate, out.channels));
        true
    }

    fn converting(&self) -> bool {
        self.converter.is_some()
    }

    /// The format buffers arrive in: the converter's input, else the output's.
    fn native(&self, out: Format) -> Format {
        self.converter.as_ref().map_or(out, |c| c.from)
    }

    /// Reopens the output at `f` once the queue has drained, instead of converting.
    fn follow<H: Host>(&mut self, host: &mut H, f: Format) {
        self.converter = None;
        self.pending_format = Some(f);
        host.log(&format!("sink follows {} Hz x{}: nothing overlaps, so the output opens again rather than convert", f.rate, f.channels));
    }

    /// Arms the staged format of `id` now that its buffers flow; other staged formats wait. `gapless`:
    /// nothing is mixed into it, so the output may follow its rate.
    fn arm_staged_for<H: Host>(&mut self, host: &mut H, id: Option<StreamId>, gapless: bool) {
        let mut found: Option<Staged> = None;
        let mut keep = VecDeque::new();
        while let Some(s) = self.staged.pop_front() {
            if s.id == id || (id.is_none() && found.is_none()) {
                found = Some(s);
            } else {
                keep.push_back(s);
            }
        }
        self.staged = keep;
        let Some(Staged { format: f, .. }) = found else { return };
        if Some(f) == self.out {
            if self.converter.as_ref().is_some_and(|c| c.id.is_some()) {
                self.converter = None;
            }
            return;
        }
        // Re-arming would lose the filter history and click: a gapless successor carries on with it.
        if let Some(c) = self.converter.as_mut().filter(|c| c.from == f && (c.id == id || gapless)) {
            c.id = id;
            return;
        }
        if gapless && self.follow_rate {
            self.follow(host, f);
            return;
        }
        if !self.arm_conversion(host, id, f) {
            self.abandon_transition(host);
            self.pending_format = Some(f);
        }
    }

    fn apply<D: Downstream>(&mut self, down: &mut D, f: Format) {
        self.out = Some(f);
        down.configure(f);
    }

    pub fn set_output_stream_offset_us(&mut self, offset_us: i64) {
        self.offset_us = offset_us;
    }

    /// Sets the gain (ReplayGain) of the song whose buffers come next, from its first sample. Applied per
    /// song before holding or mixing, so each side of a mix keeps its own level; the analyser gets the
    /// unscaled audio. Above 1 only float samples are boosted (16-bit would clip here).
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = if gain.is_finite() { gain.clamp(0.0, MAX_GAIN) } else { 1.0 };
    }

    /// What the stages read of the song whose buffers come next, from its first sample.
    pub fn set_song(&mut self, song: Arc<SongFacts>) {
        self.song = song;
    }

    /// What is queued of the song `was` tells of is told of by `now` (its prepared data came).
    pub fn retell(&mut self, was: &Arc<SongFacts>, now: &Arc<SongFacts>) {
        for c in self.queue.iter_mut().filter(|c| Arc::ptr_eq(&c.song, was)) {
            c.song = now.clone();
        }
        for s in [&mut self.song, &mut self.held_song] {
            if Arc::ptr_eq(s, was) {
                *s = now.clone();
            }
        }
    }

    /// Scales by `ratio` everything still queued or held of the song at `stream_offset_us` (settings
    /// changed), including the untaken rest of a partly taken chunk. Mixed chunks are left alone.
    /// Allocates nothing.
    pub fn rescale(&mut self, stream_offset_us: i64, ratio: f32) {
        let Some(out) = self.out else { return };
        if !ratio.is_finite() || ratio < 0.0 || ratio == 1.0 {
            return;
        }
        for c in self.queue.iter_mut().filter(|c| c.stream_us == Some(stream_offset_us)) {
            if c.gain != 1.0 {
                c.gain *= ratio;
            } else {
                crate::pcm::scale(&mut c.data[c.pos..], out.encoding, ratio);
            }
        }
        let held = match self.flight.phase {
            Phase::Hold(h) => h.len,
            Phase::Mix { len, .. } => len,
            Phase::Pass => 0,
        };
        if held > 0 && self.held_offset_us == stream_offset_us {
            // All of it: a looped hold is read again.
            crate::pcm::scale(&mut self.tail[..held], out.encoding, ratio);
        }
    }

    // ---- the audio path ----

    /// One decoded buffer at `pts_us`. Returns (all taken, bytes taken); the rest is offered again.
    pub fn handle_buffer<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, buffer: &[u8], pts_us: i64) -> (bool, usize) {
        self.fresh = false;
        self.flight.awaiting_stream = false;
        if let Some(f) = self.pending_format.take() {
            if !self.drain(down) {
                self.pending_format = Some(f);
                return (false, 0);
            }
            self.apply(down, f);
        }
        let Some(out) = self.out else { return down.handle_buffer(buffer, 0, pts_us) };
        if !self.drain(down) {
            return (false, 0);
        }
        let native = self.native(out);
        let gain = if native.encoding == crate::pcm::Encoding::Float { self.gain } else { self.gain.min(1.0) };
        // Passed straight on, the output's sound chain applies the gain as it runs.
        let chained = gain != 1.0 && self.flight.phase == Phase::Pass && self.flight.stretch.is_none() && !self.converting() && down.applies_gain();
        let scaled = (gain != 1.0 && !chained).then(|| {
            let mut b = self.copy_of(buffer);
            crate::pcm::scale_dithered(&mut b, native.encoding, gain, native.channels, &mut self.dither);
            b
        });
        let taken = self.route(down, host, buffer, scaled.as_deref().unwrap_or(buffer), pts_us, out, native, if chained { gain } else { 1.0 });
        if let Some(b) = scaled {
            self.recycle(b);
        }
        taken
    }

    /// Routes a buffer by phase. `raw` is unscaled (for the analyser), `buffer` is at the song's gain
    /// but for `gain`, which the output applies when it is passed on.
    #[allow(clippy::too_many_arguments)]
    fn route<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, raw: &[u8], buffer: &[u8], pts_us: i64, out: Format, native: Format, gain: f32) -> (bool, usize) {
        // The incoming loop's repeats, and its silence before it starts, go out before more of the song is taken.
        if !self.replay(down, host, out) || (matches!(self.flight.phase, Phase::Mix { .. }) && !self.lead(down, host, out)) {
            return (false, 0);
        }
        self.feed_analysis(host, raw, native);
        match self.flight.phase {
            Phase::Pass => {
                if self.flight.stretch.is_some() {
                    self.stretch_out(host, buffer, pts_us);
                    self.drain(down);
                    return (true, buffer.len());
                }
                let converted = if self.converting() {
                    match self.converted(host, buffer) {
                        Some(b) => Some(b),
                        None => {
                            self.drain(down);
                            return (true, buffer.len());
                        }
                    }
                } else {
                    None
                };
                let result = self.pass_or_hold(down, host, converted.as_deref().unwrap_or(buffer), buffer.len(), pts_us, out, gain);
                if let Some(b) = converted {
                    self.recycle(b);
                }
                if let Some(r) = result {
                    return r;
                }
            }
            Phase::Hold(_) => {
                if self.converting() {
                    if let Some(b) = self.converted(host, buffer) {
                        self.hold(&b, out);
                        self.recycle(b);
                    }
                } else {
                    self.hold(buffer, out);
                }
            }
            Phase::Mix { .. } => {
                self.mix(host, buffer, pts_us, out);
                self.replay(down, host, out);
            }
        }
        self.drain(down);
        (true, buffer.len())
    }

    /// Passes `buf` (already converted) through, or at the plan's start passes the head and holds the
    /// rest. `Some` is the result to return now; `None` means it was taken whole. `gain`: still to be
    /// applied, by the output or, to what is held, here.
    #[allow(clippy::too_many_arguments)]
    fn pass_or_hold<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, buf: &[u8], whole: usize, pts_us: i64, out: Format, gain: f32) -> Option<(bool, usize)> {
        if self.playing_id.is_none() {
            self.playing_id = self.current_id.clone();
        }
        let landing = std::mem::take(&mut self.landing);
        let landed = !matches!(landing, Landing::None);
        let mix_phase = match landing {
            Landing::Mix(us) => Some(us),
            _ => None,
        };
        let mut p = self.refresh_plan(host);
        if p.is_none() && self.playing_id != self.current_id {
            // A stale playing id (rapid skips): trust the decoder's id; costs one replan.
            self.playing_id = self.current_id.clone();
            p = self.refresh_plan(host);
        }
        let track_pos = pts_us - self.offset_us;
        let fb = out.frame_bytes();
        let frames = (buf.len() / fb) as i64;
        let start_frame = p.map_or(i64::MAX, |(start_us, _)| {
            let frames = (start_us - track_pos) * out.rate as i64 ;
            (frames + if frames >= 0 { 500_000 } else { -500_000 }) / 1_000_000
        });
        // Past the start but inside the transition (a seek): hold what is left. Past its end: nothing to hold.
        let skip_transition = p.is_some_and(|(_, duration_us)| start_frame < -duration_us * out.rate as i64 / 1_000_000);
        if start_frame >= frames || skip_transition {
            return Some(self.pass(down, buf, whole, pts_us, gain));
        }
        let late = start_frame < 0;
        // A leader's plan come after the music here passed its start: not mixed from somewhere in it; the
        // leader's change of song is followed instead. Landed in it (started there), what is left mixes,
        // as there.
        if late && !landed && matches!(self.plans, Plans::Led(_)) {
            host.log(&format!("transition: the leader's plan came {} ms after its start: not mixed", -start_frame * 1000 / out.rate as i64));
            self.flight.ending = Ending::LetGo;
            return Some(self.pass(down, buf, whole, pts_us, gain));
        }
        let before = start_frame.max(0) as usize * fb;
        if before > 0 {
            let head = self.copy_of(&buf[..before]);
            self.enqueue(head, pts_us, Some(self.offset_us), 1.0, gain);
        }
        let late_us = mix_phase.unwrap_or_else(|| {
            if late { -start_frame * 1_000_000 / out.rate as i64 } else { 0 }
        });
        // Played from inside, the transition keeps both decks at one speed: the loop and the bend are for one
        // played whole.
        if let (true, Ending::Planned(p)) = (late_us > 0, &mut self.flight.ending) {
            p.mixer.unbend();
            p.mixer.in_loop = None;
        }
        let p = self.plan().cloned().expect("a plan exists past this point");
        self.begin_hold(&p, out, late_us, host.now_ms());
        if late {
            host.log(&format!("transition: late hold, {} ms in", late_us / 1000));
        }
        let held_from = pts_us + (before / fb) as i64 * 1_000_000 / out.rate as i64 + mix_phase.unwrap_or(0);
        self.flight.held_from_us = Some(held_from);
        self.heard.audible_us = held_from - self.held_offset_us + self.flight.takeover_out_us;
        let runway = down.position_us(false).map(|at| held_from - at);
        host.log(&match runway {
            None => "holding the ending, no sound still in the sink".to_string(),
            Some(r) => format!("holding the ending, {} ms of sound still in the sink", r / 1000),
        });
        // Landing in or at the transition, the sink holds only what was before the cut: the hold is meant.
        if let Some(runway) = runway.filter(|&r| r < DRY_US && !late && !landed) {
            // Decode never got ahead (the next track still fetching): don't hold.
            host.log(&format!("transition: no runway ({} ms), letting the ending play", runway / 1000));
            self.abandon_transition(host);
            return Some(self.pass(down, &buf[before..], whole, pts_us, gain));
        }
        let held = self.hold(&buf[before..], out);
        if gain != 1.0 {
            crate::pcm::scale_dithered(&mut self.tail[held], out.encoding, gain, out.channels, &mut self.dither);
        }
        None
    }

    /// Passes `buffer` on and reports `whole` bytes taken. Offered as it is when nothing waits before it;
    /// what the output does not take is copied and queued, so [`TransitionEngine::rescale`] and the next
    /// offer work on the engine's own memory.
    fn pass<D: Downstream>(&mut self, down: &mut D, buffer: &[u8], whole: usize, pts_us: i64, gain: f32) -> (bool, usize) {
        let direct = self.queue.is_empty() && !self.flight.resync_next && !self.flight.measure_next;
        let (taken, used) = if direct {
            down.media_pace(1.0);
            down.song_gain(gain);
            down.song_facts(&self.song);
            down.handle_buffer(buffer, 0, pts_us)
        } else {
            (false, 0)
        };
        if !taken {
            let c = self.copy_of(buffer);
            self.enqueue(c, pts_us, Some(self.offset_us), 1.0, gain);
            match self.queue.back_mut() {
                // The output just said it is full.
                Some(c) if direct => c.pos = used,
                _ => {
                    self.drain(down);
                }
            }
        }
        (true, whole)
    }

    /// The playing song's plan as (start, duration) µs, asked for when unknown. A gapless answer is
    /// often transient (queue edits, analyses landing), so it is asked again after [`NULL_PLAN_RETRY_MS`].
    fn refresh_plan<H: Host>(&mut self, host: &mut H) -> Option<(i64, i64)> {
        let id = self.playing_id.as_ref()?;
        let now = host.now_ms();
        let known = self.flight.plan_for.as_ref() == Some(id)
            && match self.flight.ending {
                Ending::Ask => false,
                Ending::Gapless(asked_at) => now - asked_at <= NULL_PLAN_RETRY_MS,
                Ending::Planned(_) | Ending::LetGo => true,
            };
        if !known {
            // An analysis stored meanwhile (measured ahead, or as it came) ends the one made as it plays.
            if matches!(self.analysis, Analysis::On { .. }) && self.current_id.as_ref() == Some(id) && host.wants_analysis(&id.song).is_none() {
                self.analysis = Analysis::Off;
            }
            self.flight.ending = self.plans.plan_for(host, &id.song).map_or(Ending::Gapless(now), Ending::Planned);
            if self.flight.plan_for.as_ref() != Some(id) {
                self.flight.plan_for = Some(id.clone());
            }
            if let Some(p) = self.plan().cloned() {
                self.prepare(&p);
            }
        }
        self.plan().map(|p| (p.out_start_us(), p.held_us()))
    }

    /// Allocates what the transition needs when the plan arrives, not when the mix starts (allocating
    /// between two buffers can starve the output).
    fn prepare(&mut self, p: &Plan) {
        let Some(out) = self.out else { return };
        let bytes = out.bytes(p.held_us());
        if bytes > 0 && self.tail.len() < bytes {
            self.tail.resize(bytes, 0);
        }
        self.ensure_mixer(out).prepare(&p.mixer);
        let keep = p.mixer.keep_pitch;
        if (p.mixer.bend.is_some() || p.mixer.out_glide.is_some()) && !self.out_stretch.as_ref().is_some_and(|(_, k, f)| *k == keep && *f == out) {
            self.out_stretch = Some((ByteStretcher::new(out.rate, out.channels, keep), keep, out));
        }
        if let Some(l) = p.mixer.in_loop {
            let bytes = out.bytes(l.ms * 1000);
            if self.loop_in.len() < bytes {
                self.loop_in.resize(bytes, 0);
            }
            // What one stretched buffer past the slice can hold.
            self.stash.reserve(1 << 17);
        }
        if p.stretching() {
            if self.flight.pending_stretch.as_ref().is_some_and(|(_, k)| *k != p.mixer.keep_pitch) {
                self.flight.pending_stretch = None;
            }
            if self.flight.pending_stretch.is_none() {
                self.flight.pending_stretch = Some((ByteStretcher::new(out.rate, out.channels, p.mixer.keep_pitch), p.mixer.keep_pitch));
            }
        }
        while self.pool.len() < 8 {
            self.pool.push(Vec::with_capacity(16384));
        }
    }

    /// The mixer for `out`, rebuilt only when rate or channels changed.
    fn ensure_mixer(&mut self, out: Format) -> &mut Mixer {
        if !matches!(self.mixer, Some((_, rate, channels)) if rate == out.rate && channels == out.channels) {
            self.mixer = Some((Mixer::new(out.rate, out.channels), out.rate, out.channels));
        }
        &mut self.mixer.as_mut().expect("just ensured").0
    }

    /// Holds the ending from now (`at_ms`), `late_us` into the transition.
    fn begin_hold(&mut self, p: &Plan, out: Format, late_us: i64, at_ms: i64) {
        self.flight.held_id = self.playing_id.clone().or_else(|| self.current_id.clone());
        self.held_offset_us = self.offset_us;
        self.held_song = self.song.clone();
        self.heard.next_rate = if p.stretching() { p.tempo_ratio() } else { 1.0 };
        self.take_over(p, late_us);
        let late = late_us.clamp(0, p.duration_us());
        self.heard.from = self.flight.held_id.as_ref().map(|h| h.serial);
        self.heard.audible_us = i64::MAX;
        // Outro loop: only the loop slice is captured.
        let hold_us = if p.out_loop_us() > 0 { p.out_loop_us() } else { p.held_us() };
        let bytes = out.bytes(hold_us);
        if self.tail.len() < bytes {
            self.tail.resize(bytes, 0);
        }
        // A late hold only holds what is left of the overlap.
        let late_hold = if p.out_loop_us() > 0 { 0 } else { late };
        let limit = out.bytes((hold_us - late_hold).max(0));
        self.flight.phase = Phase::Hold(Hold { len: 0, limit, held_us: 0, at_ms, late_us });
    }

    /// Where `p`, held `late_us` into it, hands over: the next song takes over where it becomes the
    /// louder, not where the fade begins.
    fn take_over(&mut self, p: &Plan, late_us: i64) {
        let late = late_us.clamp(0, p.duration_us());
        self.flight.takeover_us = (crate::automix::mixer::crossover_ms(&p.mixer) * 1000 - late).max(0);
        let (late_ms, at_ms) = (late as f64 / 1000.0, (late + self.flight.takeover_us) as f64 / 1000.0);
        let (m, us) = (&p.mixer, |ms: f64| (ms * 1000.0).round() as i64);
        self.flight.takeover_out_us = us(m.out_song_ms(at_ms) - m.out_song_ms(late_ms));
        self.heard.next_from_us = p.in_skip_us() + us(m.in_song_ms(at_ms));
    }

    /// Appends outgoing audio to the hold; what does not fit is skipped by the plan. Returns where in
    /// the tail it went.
    fn hold(&mut self, buffer: &[u8], out: Format) -> core::ops::Range<usize> {
        let Phase::Hold(h) = &mut self.flight.phase else { unreachable!("held only while holding") };
        h.held_us += out.us(buffer.len());
        let n = buffer.len().min(h.limit - h.len);
        let from = h.len;
        self.tail[from..from + n].copy_from_slice(&buffer[..n]);
        h.len += n;
        from..from + n
    }

    /// The next track begins (or a seek landed): start mixing into the hold, or let the hold go unmixed.
    pub fn handle_discontinuity<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H) {
        let p = self.plan().cloned();
        let ending = self.flight.plan_for.clone().or_else(|| self.playing_id.clone());
        if p.is_some() && self.out.is_some() && matches!(self.flight.phase, Phase::Hold(h) if h.len > 0) {
            // The incoming stream, announced before this: its format is armed so stretch and skip are
            // measured in its own domain. One that cannot be converted lets the hold go unmixed.
            self.arm_staged_for(host, self.current_id.clone(), false);
        }
        let mixed = matches!(self.flight.phase, Phase::Hold(h) if h.len > 0) && self.out.is_some();
        self.flight.made = ending.map(|id| (id.serial, if mixed { p.clone() } else { None }));
        match (self.flight.phase, p, self.out) {
            (Phase::Hold(h), Some(mut p), Some(out)) if h.len > 0 => {
                self.flight.mix_source_id = self.current_id.clone();
                self.playing_id = self.current_id.clone();
                let late = h.late_us.clamp(0, p.duration_us());
                if p.out_loop_us() <= 0 && h.len < h.limit {
                    // The song ended before the planned end: the mix runs over what was held, its fades with it.
                    let ms = (late + out.us(h.len) + 999) / 1000;
                    host.log(&format!("mixing: {} ended {} ms into its {} ms ending: the mix is made to fit", song(&self.flight.held_id), ms, p.duration_us() / 1000));
                    p.mixer.squeeze(ms);
                    self.take_over(&p, h.late_us);
                    if let Some(from) = self.flight.held_from_us {
                        self.heard.audible_us = from - self.held_offset_us + self.flight.takeover_out_us;
                    }
                }
                // A bend or glide needs the outgoing stretcher made for it; without one both decks keep one speed.
                let keep = p.mixer.keep_pitch;
                if (p.mixer.bend.is_some() || p.mixer.out_glide.is_some()) && !self.out_stretch.as_ref().is_some_and(|(_, k, f)| *k == keep && *f == out) {
                    p.mixer.unbend();
                    p.mixer.out_glide = None;
                }
                let m = self.ensure_mixer(out);
                m.configure(&p.mixer);
                // A late hold starts the mix curves that far in.
                if late > 0 {
                    m.seek(((late * out.rate as i64  + 500_000) / 1_000_000) as u64);
                }
                self.flight.skip_left = 0;
                self.enter_incoming(&p, late);
                let left = self.flight.held_from_us.zip(down.position_us(false)).map(|(from, at)| from - at);
                host.log(&format!(
                    "mixing: the next track arrived {} ms into the hold with {} of sound left",
                    host.now_ms() - h.at_ms,
                    left.map_or("no".to_string(), |l| format!("{} ms", l / 1000)),
                ));
                // The hold goes out as the mix now; the position already reported stands.
                let frames_of = |ms: i64| (ms.max(0) * out.rate as i64 / 1000) as usize;
                self.flight.out_glide = None;
                let loop_frames = ((p.out_loop_us() * out.rate as i64 / 1_000_000) as usize).max(1);
                // Under a bend the outgoing speed glides to the incoming tempo, under a glide to its own target.
                let glide = match (p.mixer.bend, p.mixer.out_glide) {
                    (Some(b), _) => Some((b.start_ms, b.end_ms, 1.0 / p.mixer.tempo_ratio)),
                    (None, Some(g)) => Some((g.window.start_ms, g.window.end_ms, g.to)),
                    _ => None,
                };
                let mix_out = if let (Some((start_ms, end_ms, to)), Some((st, ..))) = (glide, self.out_stretch.as_ref()) {
                    // The stretcher takes over early enough to cross in at ratio 1, warmed on the bars before.
                    let start = frames_of(start_ms);
                    let from = start.saturating_sub(st.entry_frames()).max(st.latency_frames().min(start));
                    let ramp = frames_of(end_ms - start_ms) as u64;
                    self.flight.out_glide = Some(OutGlide { from, hold: (start - from) as u64, ramp, to, started: false });
                    let loop_bytes = if p.out_loop_us() > 0 { loop_frames * out.frame_bytes() } else { 0 };
                    MixOut::Bent { read: 0, frame: 0, frames: frames_of(p.mixer.duration_ms), loop_bytes }
                } else if p.out_loop_us() > 0 {
                    MixOut::Looped {
                        frame: ((late * out.rate as i64 + 500_000) / 1_000_000) as usize,
                        frames: (p.duration_us() * out.rate as i64 / 1_000_000).max(0) as usize,
                        loop_frames,
                    }
                } else {
                    MixOut::Once { read: 0 }
                };
                self.flight.mixed_end_us = None;
                self.flight.mix_from_us = None;
                self.flight.resync_next = true;
                self.flight.measure_next = true;
                self.flight.phase = Phase::Mix { len: h.len, out: mix_out };
            }
            _ => {
                self.abandon_transition(host);
                self.playing_id = self.current_id.clone();
                self.arm_staged_for(host, self.current_id.clone(), true);
                self.flight.mix_source_id = None;
                self.flight.awaiting_stream = true;
                down.handle_discontinuity();
            }
        }
        self.flight.ending = Ending::Ask;
        self.flight.plan_for = None;
    }

    /// Sets up the mix's incoming side once its format is known: the stretcher and the skip, in the
    /// incoming format when converting.
    fn enter_incoming(&mut self, p: &Plan, late: i64) {
        let Some(out) = self.out else { return };
        let m = &p.mixer;
        let in_late_us = (m.in_song_ms(late as f64 / 1000.0) * 1000.0).round() as i64;
        let s_fmt = self.native(out);
        // The repeats are copies, so the stretcher makes that much less of the mix.
        let repeats_us = m.repeats_ms() * 1000;
        if p.stretching() {
            let taken = if !self.converting() { self.flight.pending_stretch.take().filter(|(_, k)| *k == m.keep_pitch).map(|(s, _)| s) } else { None };
            self.flight.pending_stretch = None;
            let mut s = taken.unwrap_or_else(|| ByteStretcher::new(s_fmt.rate, s_fmt.channels, m.keep_pitch));
            let (glide_us, ramp_us) = match m.bend {
                Some(b) => (b.start_ms * 1000, (b.end_ms - b.start_ms) * 1000),
                None => (p.duration_us(), p.ramp_us()),
            };
            let frames = |us: i64| (us * s_fmt.rate as i64 / 1_000_000).max(0) as u64;
            s.configure(p.tempo_ratio() as f64, frames(glide_us - repeats_us - late), frames(ramp_us));
            self.flight.stretch = Some(Stretch { s, format: s_fmt });
            self.stretch_pace = p.tempo_ratio() as f64;
        }
        self.flight.clock = None;
        let pace = if p.stretching() { p.tempo_ratio() as f64 } else { 1.0 };
        self.flight.repeat = m.in_loop.filter(|_| late == 0).map(|l| {
            let frames = (out.bytes(l.ms * 1000) / out.frame_bytes()).max(1);
            Repeat { frames, captured: 0, left: frames * l.repeats.max(0) as usize, pace }
        });
        let skip_frames = ((p.in_skip_us() + in_late_us).max(0) * s_fmt.rate as i64 + 500_000) / 1_000_000;
        self.flight.skip_left = skip_frames as usize * s_fmt.frame_bytes();
        let lead_frames = ((m.in_delay_ms * 1000 - late).max(0) * out.rate as i64 / 1_000_000) as usize;
        self.flight.lead_left = lead_frames * out.frame_bytes();
        self.flight.lead_at_us = p.in_skip_us() - out.us(self.flight.lead_left);
    }

    /// Mixes the incoming deck's silence before its first sample into the ending, a piece at a time as the output
    /// takes it; false while the output is full and some is left.
    fn lead<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, out: Format) -> bool {
        let fb = out.frame_bytes();
        while self.flight.lead_left > 0 {
            if !matches!(self.flight.phase, Phase::Mix { .. }) {
                self.flight.lead_left = 0;
                break;
            }
            let n = self.flight.lead_left.min(SILENCE.len() / fb * fb);
            self.mix_src(host, &SILENCE[..n], self.flight.lead_at_us, 1.0, out);
            self.flight.lead_at_us += out.us(n);
            self.flight.lead_left -= n;
            if !self.drain(down) && self.flight.lead_left > 0 {
                return false;
            }
        }
        true
    }

    /// Mixes the incoming track into the held ending.
    fn mix<H: Host>(&mut self, host: &mut H, buffer: &[u8], pts_us: i64, out: Format) {
        let mut buffer = buffer;
        let mut first_us = pts_us;
        if self.flight.skip_left > 0 {
            let n = self.flight.skip_left.min(buffer.len());
            buffer = &buffer[n..];
            self.flight.skip_left -= n;
            first_us += self.native(out).us(n);
            if buffer.is_empty() {
                return;
            }
        }
        if self.flight.clock.is_none() && (self.flight.stretch.is_some() || self.flight.repeat.is_some()) {
            // Started as far back as the repeats will carry it, so it reads the song's own time after them.
            let looped_us = self.flight.repeat.map_or(0, |r| span_us(r.left, r.pace, out));
            self.flight.clock = Some(Clock::From(first_us - looped_us));
        }
        // Stretch in the incoming format, then convert to the output's.
        let stretched = if self.flight.stretch.is_some() {
            match self.stretched(buffer) {
                Some(b) => Some(b),
                None => return,
            }
        } else {
            None
        };
        let buffer: &[u8] = stretched.as_deref().unwrap_or(buffer);
        let converted = if self.converting() {
            match self.converted(host, buffer) {
                Some(b) => Some(b),
                None => {
                    if let Some(b) = stretched {
                        self.recycle(b);
                    }
                    return;
                }
            }
        } else {
            None
        };
        let src: &[u8] = converted.as_deref().unwrap_or(buffer);
        let pace = if stretched.is_some() { self.stretch_pace } else { 1.0 };
        let fb = out.frame_bytes();
        // The loop's slice is kept as it plays; what follows it waits for the repeats.
        let mut head = src.len();
        if let Some(r) = self.flight.repeat.as_mut().filter(|r| r.captured < r.frames) {
            let k = ((r.frames - r.captured) * fb).min(src.len());
            self.loop_in[r.captured * fb..r.captured * fb + k].copy_from_slice(&src[..k]);
            r.captured += k / fb;
            if k < src.len() {
                self.stash.extend_from_slice(&src[k..]);
                head = k;
            }
        }
        self.mix_src(host, &src[..head], first_us, pace, out);
        if let Some(b) = converted {
            self.recycle(b);
        }
        if let Some(b) = stretched {
            self.recycle(b);
        }
    }

    /// Plays the incoming loop's repeats, then the audio that came after its slice; false while the output is
    /// full and some are left.
    fn replay<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, out: Format) -> bool {
        let fb = out.frame_bytes();
        while let Some(r) = self.flight.repeat.filter(|r| r.captured == r.frames) {
            if r.left > 0 {
                let pos = r.frames - 1 - (r.left - 1) % r.frames;
                let n = (r.frames - pos).min(r.left).min((16_384 / fb).max(1));
                let c = self.loop_copy(pos * fb, n * fb);
                self.mix_src(host, &c, 0, r.pace, out);
                self.recycle(c);
                if let Some(r) = self.flight.repeat.as_mut() {
                    r.left -= n;
                }
            } else {
                self.end_repeat(host, r, out);
            }
            if !self.drain(down) {
                return false;
            }
        }
        true
    }

    /// Mixes in the audio that came after the loop's slice and ends the loop.
    fn end_repeat<H: Host>(&mut self, host: &mut H, r: Repeat, out: Format) {
        self.flight.repeat = None;
        let stash = std::mem::take(&mut self.stash);
        self.mix_src(host, &stash, 0, r.pace, out);
        self.stash = stash;
        self.stash.clear();
        // Unstretched, the song's own timestamps carry on from here.
        if self.flight.stretch.is_none() {
            self.flight.clock = None;
        }
    }

    /// A pooled copy of the loop's slice from byte `from`.
    fn loop_copy(&mut self, from: usize, n: usize) -> Vec<u8> {
        let mut c = self.take_pooled(n);
        c.extend_from_slice(&self.loop_in[from..from + n]);
        c
    }

    /// Mixes `src` (incoming audio in the output format, at `pace`, the song at `first_us`) into the held ending;
    /// what is past the mix's end plays as the incoming song.
    fn mix_src<H: Host>(&mut self, host: &mut H, src: &[u8], first_us: i64, pace: f64, out: Format) {
        let fb = out.frame_bytes();
        let (len, mut mix_out) = match self.flight.phase {
            Phase::Mix { len, out } => (len, out),
            _ => (0, MixOut::Once { read: 0 }),
        };
        let frames = match mix_out {
            MixOut::Looped { frame, frames, .. } | MixOut::Bent { frame, frames, .. } => frames.saturating_sub(frame),
            MixOut::Once { read } => (len - read) / fb,
        };
        let frames = (src.len() / fb).min(frames);
        let mut used = 0;
        if frames > 0 {
            let bytes = frames * fb;
            match &mut mix_out {
                MixOut::Looped { frame, loop_frames, .. } => {
                    let hold_frames = (len / fb).max(1);
                    let mut chunk = std::mem::take(&mut self.loop_buf);
                    chunk.resize(bytes, 0);
                    self.wrap_out(hold_frames, *loop_frames, *frame, &mut chunk, frames, fb);
                    if let Some((m, ..)) = self.mixer.as_mut() {
                        m.process_bytes(&mut chunk, &src[..bytes], out.encoding);
                    }
                    let at = self.stamp(first_us, frames, pace, out);
                    let c = self.copy_of(&chunk);
                    self.loop_buf = chunk;
                    self.enqueue(c, at, None, pace, 1.0);
                    self.flight.mixed_end_us = Some(at + span_us(frames, pace, out));
                    *frame += frames;
                }
                MixOut::Bent { read, frame, loop_bytes, .. } => {
                    let mut c = self.take_pooled(bytes);
                    c.resize(bytes, 0);
                    self.bent_out(&mut c, read, frame, len, *loop_bytes, out);
                    if let Some((m, ..)) = self.mixer.as_mut() {
                        m.process_bytes(&mut c, &src[..bytes], out.encoding);
                    }
                    let at = self.stamp(first_us, frames, pace, out);
                    self.enqueue(c, at, None, pace, 1.0);
                    self.flight.mixed_end_us = Some(at + span_us(frames, pace, out));
                }
                MixOut::Once { read } => {
                    let r = *read;
                    if let Some((m, ..)) = self.mixer.as_mut() {
                        m.process_bytes(&mut self.tail[r..r + bytes], &src[..bytes], out.encoding);
                    }
                    let at = self.stamp(first_us, frames, pace, out);
                    let mut c = self.take_pooled(bytes);
                    c.extend_from_slice(&self.tail[r..r + bytes]);
                    self.enqueue(c, at, None, pace, 1.0);
                    self.flight.mixed_end_us = Some(at + span_us(frames, pace, out));
                    *read += bytes;
                }
            }
            used = bytes;
        }
        if used < src.len() {
            let rest = &src[used..];
            let at = self.stamp(first_us + out.us(used), rest.len() / fb, pace, out);
            let c = self.copy_of(rest);
            self.enqueue(c, at, Some(self.offset_us), pace, 1.0);
            self.flight.mixed_end_us = Some(at + span_us(rest.len() / fb, pace, out));
        }
        if !matches!(self.flight.phase, Phase::Mix { .. }) {
            return;
        }
        let done = match mix_out {
            MixOut::Looped { frame, frames, .. } | MixOut::Bent { frame, frames, .. } => frame >= frames,
            MixOut::Once { read } => read >= len,
        };
        if done {
            self.flight.phase = Phase::Pass;
            self.finish_conversion(host);
        } else {
            self.flight.phase = Phase::Mix { len, out: mix_out };
        }
    }

    /// Fills `dst` with the held ending under a bend from byte `read` of it (the first `len` held, its last `lp`
    /// repeating when looped), at output frame `frame`: as it was until the glide starts, then through the
    /// outgoing stretcher. Silence once it runs out.
    fn bent_out(&mut self, dst: &mut [u8], read: &mut usize, frame: &mut usize, len: usize, lp: usize, out: Format) {
        let (fb, enc) = (out.frame_bytes(), out.encoding);
        let (Some(g), Some((st, ..))) = (self.flight.out_glide, self.out_stretch.as_mut()) else { return };
        let n = dst.len() / fb;
        let (mut done, mut started) = (0, g.started);
        while done < n {
            if *frame == g.from && !started {
                // The ending just before, in at most three runs (a loop's end and start).
                let mut v = read.saturating_sub((st.latency_frames() * fb).min(*read));
                let mut runs: [&[u8]; 3] = [&[]; 3];
                for r in runs.iter_mut() {
                    let (at, end) = ending_run(v, len, lp);
                    let k = (end - at).min(*read - v);
                    *r = &self.tail[at..at + k];
                    v += k;
                }
                st.glide(g.to, g.hold, g.ramp, &runs, enc);
                started = true;
            } else if *frame < g.from {
                let (at, end) = ending_run(*read, len, lp);
                let k = (g.from - *frame).min(n - done).min((end - at) / fb);
                dst[done * fb..(done + k) * fb].copy_from_slice(&self.tail[at..at + k * fb]);
                (*read, *frame, done) = (*read + k * fb, *frame + k, done + k);
                if k == 0 {
                    break;
                }
            } else {
                // Past the held ending, silence pushes the stretcher's last frames out.
                let (at, end) = ending_run(*read, len, lp);
                let held = at < end;
                let input = if held { &self.tail[at..end] } else { &SILENCE[..] };
                let (used, made) = st.process(input, &mut dst[done * fb..], enc);
                (*read, *frame, done) = (*read + if held { used } else { 0 }, *frame + made / fb, done + made / fb);
                if made == 0 && used == 0 {
                    break;
                }
            }
        }
        dst[done * fb..].fill(0);
        *frame += n - done;
        if let Some(g) = self.flight.out_glide.as_mut() {
            g.started = started;
        }
    }

    /// Copies looped outgoing audio: frames before the last `loop_frames` of the hold play once, the
    /// last `loop_frames` repeat.
    fn wrap_out(&self, hold_frames: usize, loop_frames: usize, from_frame: usize, dst: &mut [u8], frames: usize, fb: usize) {
        let lp = loop_frames.clamp(1, hold_frames);
        let prefix = hold_frames.saturating_sub(lp);
        let mut i = 0;
        while i < frames {
            let f = from_frame + i;
            let src_frame = if f < prefix { f } else { prefix + (f - prefix) % lp };
            let run = if f < prefix { (frames - i).min(prefix - f) } else { (frames - i).min(lp - (f - prefix) % lp) };
            let (s, n) = (src_frame * fb, run * fb);
            dst[i * fb..i * fb + n].copy_from_slice(&self.tail[s..s + n]);
            i += run;
        }
    }

    /// Converts incoming-format audio to the output format. `None` when nothing is ready yet. If
    /// conversion fails, drops the converter and lets the ending play unmixed.
    fn converted<H: Host>(&mut self, host: &mut H, input: &[u8]) -> Option<Vec<u8>> {
        let (Some(src), Some(out)) = (self.converter.as_ref().map(|c| c.from), self.out) else { return None };
        let in_frames = input.len() / src.frame_bytes().max(1);
        let need = (((in_frames as u64 * out.rate as u64 / src.rate.max(1) as u64) as usize + 4) * out.frame_bytes()).max(16384);
        let mut b = self.take_pooled(need);
        b.resize(need, 0);
        let rs = &mut self.converter.as_mut().expect("checked").resampler;
        let mut r = rs.process(input, src.encoding, &mut b, out.encoding);
        if r.is_none() {
            b.resize(b.len() * 2 + 16384, 0);
            r = rs.process(input, src.encoding, &mut b, out.encoding);
        }
        let Some((_, made)) = r else {
            host.log(&format!("conversion {} Hz x{} -> {} Hz x{} failed, letting the ending play", src.rate, src.channels, out.rate, out.channels));
            self.recycle(b);
            self.converter = None;
            self.abandon_transition(host);
            return None;
        };
        b.truncate(made);
        if made == 0 {
            self.recycle(b);
            None
        } else {
            Some(b)
        }
    }

    /// The mix is done: arm the mixed-in stream's own staged format.
    fn finish_conversion<H: Host>(&mut self, host: &mut H) {
        self.flight.mix_source_id = None;
        self.arm_staged_for(host, self.current_id.clone(), false);
    }

    /// Timestamp for mixed or stretched output: the engine's own clock in the incoming song's time,
    /// advanced by the song time each frame carries, so a stretch never looks like a jump.
    fn stamp(&mut self, pts_us: i64, frames: usize, pace: f64, out: Format) -> i64 {
        let Some(Clock::From(at) | Clock::At(at)) = self.flight.clock else { return pts_us };
        self.flight.clock = Some(Clock::At(at + span_us(frames, pace, out)));
        at
    }

    fn stretched(&mut self, input: &[u8]) -> Option<Vec<u8>> {
        let st = self.flight.stretch.as_ref()?;
        let fmt = st.format;
        let need = input.len() * 3 + st.s.latency_frames() * fmt.frame_bytes() + 8192;
        let mut buf = self.take_pooled(need);
        buf.resize(need, 0);
        let s = &mut self.flight.stretch.as_mut().expect("checked").s;
        let mut produced = 0;
        let mut at = 0;
        let fb = fmt.frame_bytes();
        while at < input.len() {
            let (used, made) = s.process(&input[at..], &mut buf[produced..], fmt.encoding);
            at += used;
            produced += made;
            if used == 0 && made == 0 {
                break;
            }
        }
        // Song time per output frame, including a finished stretcher's last frames.
        let (produced, content) = if s.bypassed() { self.finish_stretch(&mut buf, produced, fmt) } else { (produced, s.take_content()) };
        if produced >= fb {
            self.stretch_pace = content / (produced / fb) as f64;
        }
        buf.truncate(produced);
        if produced == 0 {
            self.recycle(buf);
            None
        } else {
            Some(buf)
        }
    }

    /// After a mix, the incoming track stays in the stretcher until its tempo ramp ends.
    fn stretch_out<H: Host>(&mut self, host: &mut H, buffer: &[u8], pts_us: i64) {
        let Some(out) = self.out else { return };
        // Stamped on the running clock, including the stretcher's final output (stamped 0 it would pull
        // the output's clock back and a later hold would never be released).
        let at = match self.flight.clock {
            Some(Clock::At(at)) => at,
            _ => pts_us,
        };
        let Some(s) = self.stretched(buffer) else { return };
        let o = if self.converting() {
            let c = self.converted(host, &s);
            self.recycle(s);
            match c {
                Some(o) => o,
                None => return,
            }
        } else {
            s
        };
        let frames = o.len() / out.frame_bytes();
        let pace = self.stretch_pace;
        // Finished: the resync belongs to the buffer after this one, back on real timestamps.
        let resync = self.flight.stretch.is_none() && std::mem::take(&mut self.flight.resync_next);
        self.enqueue(o, at, Some(self.offset_us), pace, 1.0);
        self.flight.resync_next |= resync;
        if let Some(Clock::At(at)) = self.flight.clock.as_mut() {
            *at += span_us(frames, pace, out);
        }
    }

    /// Drains the stretcher into `buf` after `produced` bytes and lets it go; returns the bytes made and
    /// the song time they carry.
    fn finish_stretch(&mut self, buf: &mut Vec<u8>, produced: usize, fmt: Format) -> (usize, f64) {
        let Some(Stretch { mut s, .. }) = self.flight.stretch.take() else { return (produced, 0.0) };
        let room = (s.latency_frames() + 4 * crate::automix::stretch::BLOCK) * fmt.frame_bytes();
        if buf.len() < produced + room {
            buf.resize(produced + room, 0);
        }
        let more = s.drain(&mut buf[produced..], fmt.encoding);
        // Back on the track's own timestamps: the output takes the next one as a new reference.
        self.flight.resync_next = true;
        (produced + more, s.take_content())
    }

    /// Lets the held audio go unmixed (the next track never came, or the mix was cut short). A cut
    /// mix queues only the song's audio not yet mixed out. The converter is kept.
    fn abandon_transition<H: Host>(&mut self, host: &mut H) {
        self.flight.measure_next = false;
        // The repeats left are dropped, the clock stepping over them; what came after the loop still mixes in.
        if let (Some(r), Some(out)) = (self.flight.repeat, self.out) {
            if let Some(Clock::From(at) | Clock::At(at)) = self.flight.clock.as_mut() {
                *at += span_us(r.left, r.pace, out);
            }
            self.end_repeat(host, r, out);
        }
        let (from, end) = match self.flight.phase {
            Phase::Hold(h) => (0, h.len),
            Phase::Mix { len, out: MixOut::Once { read } | MixOut::Bent { read, loop_bytes: 0, .. } } => (read.min(len), len),
            // The hold is the loop, already heard: nothing of the song is left in it.
            Phase::Mix { len, out: MixOut::Looped { .. } | MixOut::Bent { .. } } => (len, len),
            Phase::Pass => (0, 0),
        };
        if from < end {
            // At its own timestamps (the output keeps its clock from them), after any queued mix.
            let at = match (self.flight.phase, self.flight.mixed_end_us) {
                (Phase::Mix { .. }, Some(end)) => end,
                _ => self.flight.held_from_us.unwrap_or(0),
            };
            let mut c = self.take_pooled(end - from);
            c.extend_from_slice(&self.tail[from..end]);
            self.enqueue(c, at, Some(self.held_offset_us), 1.0, 1.0);
            if let Some(c) = self.queue.back_mut() {
                c.song = self.held_song.clone();
            }
        }
        match self.flight.phase {
            Phase::Hold(_) => host.log("transition abandoned in Hold"),
            Phase::Mix { .. } => host.log("transition abandoned in Mix"),
            Phase::Pass => {}
        }
        self.heard.from = None;
        self.flight.phase = Phase::Pass;
        self.flight.held_from_us = None;
        self.flight.mix_source_id = None;
        // The next buffer of the song flowing does not hold again.
        self.flight.ending = Ending::LetGo;
        self.flight.plan_for = self.playing_id.clone();
    }

    // ---- analysis tap ----

    fn on_new_stream<H: Host>(&mut self, host: &mut H, id: StreamId) {
        self.finish_analysis(host);
        self.current_id = Some(id);
        self.analysis = Analysis::Ask;
    }

    fn feed_analysis<H: Host>(&mut self, host: &mut H, bytes: &[u8], f: Format) {
        let Some(id) = self.current_id.as_ref().filter(|_| !bytes.is_empty()) else { return };
        if let Analysis::Ask = self.analysis {
            self.analysis = match host.wants_analysis(&id.song) {
                Some(expected_ms) => Analysis::On { analyzer: Analyzer::new(f.rate, expected_ms), channels: f.channels, rate: f.rate },
                None => Analysis::Off,
            };
        }
        if let Analysis::On { analyzer, channels, .. } = &mut self.analysis {
            self.analysis_buf.clear();
            crate::pcm::to_f32(bytes, f.encoding, &mut self.analysis_buf);
            analyzer.feed_interleaved(&self.analysis_buf, *channels, |v| v);
        }
    }

    /// Hands the stream's analysis to the host; nothing is analysed of it after.
    fn finish_analysis<H: Host>(&mut self, host: &mut H) {
        let Analysis::On { analyzer, channels, rate } = std::mem::replace(&mut self.analysis, Analysis::Off) else { return };
        let Some(id) = self.current_id.as_ref() else { return };
        let frames = analyzer.samples();
        host.analysed(&id.song, analyzer, channels, frames, rate);
    }

    // ---- output queue ----

    /// Queues `data` at `pts_us`, at `pace` song frames a frame and with `gain` still to be applied by the
    /// output; `stream_us` is its song's stream offset (`None` for a mix).
    fn enqueue(&mut self, data: Vec<u8>, pts_us: i64, stream_us: Option<i64>, pace: f64, gain: f32) {
        if data.is_empty() {
            self.recycle(data);
            return;
        }
        self.queue.push_back(Chunk { data, pos: 0, pts_us, resync: self.flight.resync_next, measure: self.flight.measure_next, stream_us, pace, gain, song: self.song.clone() });
        self.flight.resync_next = false;
        self.flight.measure_next = false;
    }

    /// An empty buffer with capacity for `n` bytes, from the pool when one fits.
    fn take_pooled(&mut self, n: usize) -> Vec<u8> {
        let mut b = match self.pool.iter().position(|b| b.capacity() >= n) {
            Some(i) => self.pool.swap_remove(i),
            None => Vec::with_capacity(n.max(16384)),
        };
        b.clear();
        b
    }

    fn copy_of(&mut self, src: &[u8]) -> Vec<u8> {
        let mut b = self.take_pooled(src.len());
        b.extend_from_slice(src);
        b
    }

    fn recycle(&mut self, b: Vec<u8>) {
        if self.pool.len() < 32 {
            self.pool.push(b);
        }
    }

    /// Sends queued chunks. False when the output would not take everything yet.
    fn drain<D: Downstream>(&mut self, down: &mut D) -> bool {
        loop {
            let Some(c) = self.queue.front_mut() else { return true };
            if c.resync && c.pos == 0 {
                down.handle_discontinuity();
                c.resync = false;
            }
            let before = if c.measure { down.position_us(false) } else { None };
            down.media_pace(c.pace);
            down.song_gain(c.gain);
            down.song_facts(&c.song);
            let (taken, used) = down.handle_buffer(&c.data, c.pos, c.pts_us);
            c.pos += used;
            if c.measure {
                // The output jumps its clock to this chunk's time on taking it; if it refused before
                // looking, the next offer is measured again.
                let jump = before.zip(down.position_us(false)).map(|(b, a)| a - b).filter(|j| j.abs() > 50_000);
                if let Some(j) = jump {
                    self.flight.shift = Some(Shift { us: j, until_us: c.pts_us });
                }
                if jump.is_some() || taken {
                    self.flight.mix_from_us = Some(c.pts_us);
                    c.measure = false;
                }
            }
            if !taken {
                return false;
            }
            let c = self.queue.pop_front().expect("front exists");
            self.recycle(c.data);
        }
    }

    /// The position to report to the player, called every few ms. Also the one place that notices a
    /// held ending about to starve the output (the next track is late) and lets it go unmixed.
    pub fn position_us<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H, source_ended: bool) -> Option<i64> {
        let Some(at) = down.position_us(source_ended) else { return self.held_from_nothing(host) };
        self.flight.clock_us = Some(at);
        if let (Phase::Hold(h), Some(from)) = (self.flight.phase, self.flight.held_from_us) {
            if from - at < DRY_US && host.now_ms() - h.at_ms > HOLD_GRACE_MS {
                host.log(&format!("transition: nothing to mix in yet with {} ms of sound left, letting the ending play", (from - at) / 1000));
                self.abandon_transition(host);
                self.drain(down);
            }
        }
        // Held audio counts as played, so the player reads the next track (which it does only near the
        // end) in time to mix it in. Never goes backwards; once the mix starts it stands until the audible
        // position catches up.
        self.reported = self.reported.max(at + self.held_us());
        // The first mixed sample is audible: the clock is now the incoming song's.
        if self.flight.shift.is_some_and(|s| at >= s.until_us) {
            self.flight.shift = None;
        }
        let ear = at - self.flight.shift.map_or(0, |s| s.us);
        let was_heard = self.heard.id.is_some();
        // While the mix plays but before takeover, the audible song is still the outgoing one: mix
        // start plus time since, where the clock runs at the incoming song's tempo (`pace`).
        let pace = self.heard.next_rate.max(0.01) as f64;
        let incoming_from = self.flight.mix_from_us.filter(|&from| self.flight.shift.is_none() && self.flight.held_from_us.is_some() && at >= from);
        let taking_over = incoming_from.is_some_and(|from| at < from + (self.flight.takeover_us as f64 * pace) as i64);
        match &self.flight.held_id {
            Some(id) if self.reported > ear + 20_000 || taking_over => {
                let start = self.flight.held_from_us.map(|from| from - self.held_offset_us);
                // Never read the incoming song's clock as the outgoing song's position.
                self.heard.us = match (start, incoming_from) {
                    (Some(start), Some(from)) => start + ((at - from) as f64 / pace) as i64,
                    _ => ear - self.held_offset_us,
                };
                self.heard.until_us = start.map_or(i64::MAX, |start| start + self.flight.takeover_out_us);
                self.heard.at_ms = host.now_ms();
                self.heard.id = Some(id.serial);
            }
            _ => self.heard.id = None,
        }
        if self.heard.id.is_some() != was_heard {
            host.heard_changed();
        }
        // Caught up: later clock wobbles must not revive the held song.
        if self.heard.id.is_none() && self.flight.phase == Phase::Pass && self.flight.shift.is_none() {
            self.flight.held_id = None;
        }
        if self.flight.mix_from_us.is_some() && self.flight.mixed_end_us.is_some_and(|end| at >= end) {
            self.flight.mix_from_us = None;
            // The whole mix was heard; clearing these also stops nori-engine waking for a mix.
            if self.flight.phase == Phase::Pass {
                self.heard.from = None;
            }
        }
        let mixing = self.flight.mix_from_us.is_some_and(|from| at >= from);
        if mixing != self.heard.mixing {
            self.heard.mixing = mixing;
            host.heard_changed();
        }
        Some(self.reported)
    }

    /// The outgoing side's position while the transition is audible.
    pub(crate) fn mix_position_us(&self) -> Option<i64> {
        if !self.heard.mixing {
            return None;
        }
        let start = self.flight.held_from_us? - self.held_offset_us;
        Some(
            start
                + ((self.flight.clock_us? - self.flight.mix_from_us?) as f64
                    / self.heard.next_rate.max(0.01) as f64) as i64,
        )
    }

    /// Output time from the last [`TransitionEngine::position_us`] until what is heard changes: a mix's
    /// takeover and, unless `song_only`, a mix becoming audible, the end of it or of the clock's shift, a
    /// held ending caught up.
    pub fn until_heard_changes_us(&self, song_only: bool) -> Option<i64> {
        let at = self.flight.clock_us?;
        let pace = self.heard.next_rate.max(0.01) as f64;
        let takeover = self.flight.mix_from_us.map(|from| from + (self.flight.takeover_us as f64 * pace) as i64);
        let shift = self.flight.shift;
        let caught_up = self.heard.id.map(|_| self.reported - 20_000 + shift.map_or(0, |s| s.us));
        let more = [self.flight.mix_from_us, self.flight.mixed_end_us, shift.map(|s| s.until_us), caught_up].into_iter().filter(|_| !song_only);
        std::iter::once(takeover).chain(more).flatten().filter(|&t| t > at).min().map(|t| t - at)
    }

    /// The output has no position because it was given nothing: everything since a seek into a
    /// transition is being held. Held audio still counts as played (or the next song is never read and
    /// nothing plays), and the audible position is the hold's start.
    fn held_from_nothing<H: Host>(&mut self, host: &mut H) -> Option<i64> {
        let held_us = self.held_us();
        let from = self.flight.held_from_us.filter(|_| held_us > 0)?;
        self.reported = self.reported.max(from + held_us);
        if let Some(id) = self.flight.held_id.as_ref() {
            let start = from - self.held_offset_us;
            self.heard.us = start;
            self.heard.until_us = start + self.flight.takeover_out_us;
            self.heard.at_ms = host.now_ms();
            if self.heard.id != Some(id.serial) {
                self.heard.id = Some(id.serial);
                host.heard_changed();
            }
        }
        Some(self.reported)
    }

    /// The source ended: flush the hold and finish the last analysis. Returns whether everything
    /// queued went to the output.
    pub fn play_to_end_of_stream<D: Downstream, H: Host>(&mut self, down: &mut D, host: &mut H) -> bool {
        self.abandon_transition(host);
        self.finish_analysis(host);
        self.drain(down)
    }

    /// Drains, then whether the queue is empty.
    pub fn queue_empty<D: Downstream>(&mut self, down: &mut D) -> bool {
        if !self.queue.is_empty() {
            self.drain(down);
        }
        self.queue.is_empty()
    }

    fn clear<H: Host>(&mut self, host: &mut H) {
        while let Some(c) = self.queue.pop_front() {
            self.recycle(c.data);
        }
        self.flight = Flight::default();
        self.stash.clear();
        self.reported = i64::MIN;
        self.stretch_pace = 1.0;
        self.heard.from = None;
        if self.heard.mixing {
            self.heard.mixing = false;
            host.heard_changed();
        }
        if self.heard.id.take().is_some() {
            host.heard_changed();
        }
        // The output and formats stay; the converter restarts and staged formats are stale.
        if let Some(c) = self.converter.as_mut() {
            c.resampler.reset();
        }
        self.staged.clear();
        self.pending_format = None;
        // The analyser no longer hears the track continuously.
        self.analysis = Analysis::Off;
    }

    /// A seek or a jump in the queue: drops everything in flight.
    pub fn flush<H: Host>(&mut self, host: &mut H) {
        self.clear(host);
        self.fresh = true;
        self.landing = Landing::Track;
    }

    pub(crate) fn land_in_mix(&mut self, phase_us: i64) {
        self.landing = Landing::Mix(phase_us);
    }

    /// Stopped: also forgets the output format and frees buffers.
    pub fn reset<H: Host>(&mut self, host: &mut H) {
        self.clear(host);
        self.converter = None;
        self.out = None;
        self.mixer = None;
        self.pool.clear();
        self.tail = Vec::new();
        self.loop_buf = Vec::new();
        self.out_stretch = None;
        self.loop_in = Vec::new();
        self.stash = Vec::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automix::plan;
    use crate::pcm::Encoding;
    use crate::types::AutoMixSettings;

    const RATE: u32 = 44_100;
    const FMT: Format = Format { rate: RATE, channels: 2, encoding: Encoding::Pcm16 };

    /// A fake output: records what it takes; the test sets its position.
    #[derive(Default)]
    struct Down {
        taken: Vec<(Vec<u8>, i64)>,
        configured: Vec<Format>,
        discontinuities: usize,
        position: Option<i64>,
        /// Take at most this many bytes of the next offer.
        take_only: Option<usize>,
        /// Refuse this many offers outright.
        refuse: usize,
        /// Bytes left of a partly taken buffer, which must be offered next (as `pipeline::Sink` requires).
        owed: Option<usize>,
    }

    impl Downstream for Down {
        fn configure(&mut self, f: Format) {
            self.configured.push(f);
        }
        fn handle_buffer(&mut self, data: &[u8], from: usize, pts_us: i64) -> (bool, usize) {
            let key = data.len() - from;
            if let Some(owed) = self.owed {
                assert_eq!(owed, key, "offered another buffer while one was only partly taken (pipeline::Sink panics here)");
            }
            if self.refuse > 0 {
                self.refuse -= 1;
                return (false, 0);
            }
            let n = self.take_only.take().unwrap_or(usize::MAX).min(data.len() - from);
            self.taken.push((data[from..from + n].to_vec(), pts_us));
            let all = from + n == data.len();
            self.owed = if all { None } else { Some(key - n) };
            (all, n)
        }
        fn handle_discontinuity(&mut self) {
            self.discontinuities += 1;
        }
        fn position_us(&mut self, _: bool) -> Option<i64> {
            self.position
        }
    }

    impl Down {
        fn samples(&self) -> Vec<i16> {
            self.taken.iter().flat_map(|(d, _)| d.as_chunks::<2>().0.iter().map(|&c| i16::from_le_bytes(c))).collect()
        }
    }

    #[derive(Default)]
    struct Host_ {
        plans: std::collections::HashMap<String, Plan>,
        log: Vec<String>,
        now: i64,
        analysed: Vec<String>,
    }

    impl Host for Host_ {
        fn plan_for(&mut self, id: &str) -> Option<Plan> {
            self.plans.get(id).cloned()
        }
        fn wants_analysis(&mut self, _: &str) -> Option<u64> {
            Some(0)
        }
        fn analysed(&mut self, id: &str, _: Analyzer, _: usize, _: u64, _: u32) {
            self.analysed.push(id.to_string());
        }
        fn log(&mut self, m: &str) {
            self.log.push(m.to_string());
        }
        fn now_ms(&self) -> i64 {
            self.now
        }
    }

    /// Song `id`'s stream; its serial is the id's first byte, so each song here is one stream.
    fn stream(id: &str, f: Format) -> StreamFormat {
        StreamFormat { id: StreamId { song: id.into(), serial: id.as_bytes()[0] as u64 }, format: f }
    }

    /// `secs` of a constant stereo 16-bit value at 44.1 kHz.
    fn tone(v: i16, secs: f64) -> Vec<u8> {
        tone_at(v, RATE, secs)
    }

    /// `secs` of a constant stereo 16-bit value at `rate`.
    fn tone_at(v: i16, rate: u32, secs: f64) -> Vec<u8> {
        let frames = (rate as f64 * secs) as usize;
        (0..frames * 2).flat_map(|_| v.to_le_bytes()).collect()
    }

    /// Feeds 44.1 kHz `data` in 4096-byte buffers stamped from `from_us`.
    fn feed(e: &mut TransitionEngine, d: &mut Down, h: &mut Host_, data: &[u8], from_us: i64) {
        feed_in(e, d, h, data, FMT, from_us);
    }

    /// Feeds `data` (at `f`) in 4096-byte buffers stamped from `from_us`.
    fn feed_in(e: &mut TransitionEngine, d: &mut Down, h: &mut Host_, data: &[u8], f: Format, from_us: i64) {
        let mut at = 0;
        while at < data.len() {
            let n = 4096.min(data.len() - at);
            let (all, used) = e.handle_buffer(d, h, &data[at..at + n], from_us + f.us(at));
            assert!(all && used == n, "the fake output takes everything");
            at += n;
        }
    }

    /// A 2 s equal-power fade into `to`, starting `start_us` into the outgoing song.
    fn fade(to: &str, start_us: i64) -> Plan {
        let s = AutoMixSettings { max_transition_s: 2.0, ..Default::default() };
        let mut t = plan::plan(None, None, 60_000, 60_000, &s);
        assert_eq!(t.duration_ms, 2000);
        (t.out_start_ms, t.in_start_ms, t.tempo_ratio, t.keep_pitch, t.tempo_ramp_ms, t.out_loop_ms) = (start_us / 1000, 0, 1.0, true, 0, None);
        Plan { incoming_id: to.into(), mixer: t }
    }

    #[test]
    fn plans_equal_when_sounding_same() {
        // The engine rebuilds an ending when its plan is not equal to the new one.
        let a = fade("b", 1_000_000);
        let mut b = a.clone();
        (b.mixer.reason, b.mixer.tempo_ramp_beats) = ("beat matching off".into(), 8);
        assert_eq!(a, b, "only the log text differs");
        b.mixer.in_fade_end_ms -= 1;
        assert_ne!(a, b);
    }

    /// Announced before the boundary (this pipeline's order) or after it (media3's).
    #[test]
    fn other_rate_is_converted() {
        for announced_first in [true, false] {
            let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
            e.configure(&mut d, &mut h, stream("a", FMT));
            feed(&mut e, &mut d, &mut h, &tone(1000, 0.5), 0);
            if announced_first {
                e.configure(&mut d, &mut h, stream("b", F48));
                e.handle_discontinuity(&mut d, &mut h);
            } else {
                e.handle_discontinuity(&mut d, &mut h);
                e.configure(&mut d, &mut h, stream("b", F48));
            }
            let before = taken(&d);
            feed_in(&mut e, &mut d, &mut h, &tone_at(500, 48_000, 1.0), F48, 1_000_000);
            assert_eq!(d.configured.len(), 1, "the output stays as it was opened");
            let secs = secs_after(&d, before, FMT);
            assert!((secs - 1.0).abs() < 0.01, "one second of 48 kHz comes out as one second at 44.1: {secs}");
            assert!(h.log.iter().any(|l| l.starts_with("converting 48000 Hz")), "{:?}", h.log);
        }
    }

    #[test]
    fn first_format_after_flush_is_armed() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(1000, 0.3), 0);
        e.flush(&mut h);
        e.configure(&mut d, &mut h, stream("b", F48));
        assert!(h.log.iter().any(|l| l.starts_with("converting 48000 Hz")), "{:?}", h.log);
    }

    #[test]
    fn fade_mixes_next_track_into_hold() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        assert!(h.log.iter().any(|l| l.starts_with("holding the ending")), "{:?}", h.log);
        let passed = d.samples().len();
        assert_eq!(passed, FMT.bytes(1_000_000) / 2, "only the part before the transition went out");
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(-8000, 3.0), 3_000_000);
        let s = d.samples();
        let frames = s.len() / 2;
        // 1 s alone, 2 s mixed, then the rest of b: the held second of a past the fade is skipped.
        assert_eq!(frames, (RATE as f64 * (1.0 + 3.0)) as usize);
        let mid = (RATE as usize * 2) * 2;
        assert!(s[mid].abs() < 8000, "the middle of the fade is a mix, not either track: {}", s[mid]);
        assert_eq!(s[..passed].iter().copied().collect::<std::collections::HashSet<_>>().len(), 1);
        assert!(s[s.len() - 10..].iter().all(|&v| v == -8000), "b plays alone after the fade");
        assert_eq!(d.discontinuities, 1, "one resync, for the mix's timestamps");
        assert_eq!(e.heard().from, Some(b'a' as u64));
    }

    #[test]
    fn refused_first_chunk_resyncs_once() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        d.refuse = 3;
        let b = tone(-8000, 0.1);
        e.handle_buffer(&mut d, &mut h, &b[..4096], 3_000_000);
        assert!(d.refuse < 3, "the output refused the first chunk");
        while d.refuse > 0 {
            e.drain(&mut d);
        }
        assert!(e.drain(&mut d), "and takes it in the end");
        assert_eq!(d.discontinuities, 1, "one resync for the mix, however often the first chunk was offered");
    }

    /// The incoming song's frame number as its sample on both channels, so a dropped or repeated stretch shows.
    fn ramp(frames: usize) -> Vec<u8> {
        (0..frames).flat_map(|f| [(f % 20_000) as i16; 2]).flat_map(i16::to_le_bytes).collect()
    }

    #[test]
    fn outro_loop_mix_keeps_incoming_past_the_window() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        p.mixer.out_loop_ms = Some(500);
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        let passed = d.samples().len() / 2;
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        let incoming = 3 * RATE as usize;
        feed(&mut e, &mut d, &mut h, &ramp(incoming), 3_000_000);
        let s = d.samples();
        assert_eq!(s.len() / 2, passed + incoming, "the whole incoming song plays once");
        let window = 2 * RATE as usize;
        let after = &s[(passed + window) * 2..];
        assert!(after.chunks(2).enumerate().all(|(i, f)| f == [((window + i) % 20_000) as i16; 2]), "the incoming song goes on right where the window ends");
    }

    /// The next song ends inside a looped mix: what was looped is not played again after it (the outgoing song
    /// heard once more, as if repeated).
    #[test]
    fn a_cut_looped_mix_does_not_replay_the_loop() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        p.mixer.out_loop_ms = Some(500);
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        let passed = d.samples().len() / 2;
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(-8000, 0.8), 3_000_000);
        assert!(e.play_to_end_of_stream(&mut d, &mut h));
        let s = d.samples();
        assert_eq!(s.len() / 2 - passed, (0.8 * RATE as f64) as usize, "only the mix that was made plays");
        assert!(!s[s.len() - 200..].iter().all(|&v| v == 8000), "the loop is not heard again after it");
    }

    /// The incoming song's first quarter second of the mix looped twice, at its own speed and slowed 3 %: it
    /// plays three times, then the song goes on after it, on a clock that never steps back and ends at the
    /// song's own time.
    #[test]
    fn an_incoming_loop_repeats_its_opening_then_goes_on() {
        for ratio in [1.0, 0.97] {
            let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
            let mut p = fade("b", 1_000_000);
            (p.mixer.tempo_ratio, p.mixer.in_loop) = (ratio, Some(crate::types::InLoop { ms: 250, repeats: 2 }));
            h.plans.insert("a".into(), p);
            e.configure(&mut d, &mut h, stream("a", FMT));
            feed(&mut e, &mut d, &mut h, &tone(0, 3.0), 0);
            let (passed, mixed_from) = (d.samples().len() / 2, d.taken.len());
            e.configure(&mut d, &mut h, stream("b", FMT));
            e.handle_discontinuity(&mut d, &mut h);
            let b = [tone(4000, 0.25), tone(-4000, 2.75)].concat();
            feed(&mut e, &mut d, &mut h, &b, 3_000_000);
            assert!(e.play_to_end_of_stream(&mut d, &mut h));
            let s = d.samples();
            let l = RATE as usize / 4;
            // The mix plays 2 s for (2 s - 2 loops) x ratio of b; the rest at its own speed.
            let expected = 2.0 + 3.0 - (2.0 - 0.5) * ratio;
            let made = (s.len() / 2 - passed) as f64 / RATE as f64;
            assert!((made - expected).abs() < 0.005, "ratio {ratio}: all of b plays, and the loop twice more: {made} s");
            let mix: Vec<i16> = s[passed * 2..].iter().step_by(2).copied().collect();
            assert!(mix[l / 2..3 * l].iter().all(|&v| v > 100), "ratio {ratio}: the opening three times");
            assert!(mix[3 * l + 2048..].iter().all(|&v| v < -100), "ratio {ratio}: then the rest of b");
            let pts: Vec<i64> = d.taken[mixed_from..].iter().map(|(_, at)| *at).collect();
            assert!(pts.windows(2).all(|w| w[0] <= w[1]), "ratio {ratio}: the clock never steps back: {pts:?}");
            let (last, at) = d.taken.last().expect("b played");
            assert!((at + FMT.us(last.len()) - 6_000_000).abs() < 2_000, "ratio {ratio}: b ends at its own time: {at}");
        }
    }

    /// The last second of a 2 s mix bends 5 %: the outgoing song plays faster through it, so more of it is
    /// held, with no step where the stretcher takes over.
    #[test]
    fn a_bent_mix_speeds_the_ending_up_seamlessly() {
        for keep_pitch in [true, false] {
            let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
            let mut p = fade("b", 1_000_000);
            (p.mixer.tempo_ratio, p.mixer.keep_pitch, p.mixer.bend) = (0.95, keep_pitch, Some(crate::types::Bend { start_ms: 1000, end_ms: 2000 }));
            let held = p.held_us();
            assert!((held - 2_025_600).abs() < 1000, "one second at 1 to 1/0.95 holds 25.6 ms more: {held}");
            h.plans.insert("a".into(), p);
            e.configure(&mut d, &mut h, stream("a", FMT));
            let a: Vec<u8> = (0..RATE as usize + FMT.bytes(held) / 4)
                .flat_map(|i| {
                    let v = ((i as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() * 8000.0) as i16;
                    [v, v]
                })
                .flat_map(i16::to_le_bytes)
                .collect();
            feed(&mut e, &mut d, &mut h, &a, 0);
            let passed = d.samples().len() / 2;
            assert!(matches!(e.flight.phase, Phase::Hold(h) if h.len == FMT.bytes(held)), "pitch {keep_pitch}: the bend's song time is held");
            e.configure(&mut d, &mut h, stream("b", FMT));
            e.handle_discontinuity(&mut d, &mut h);
            feed(&mut e, &mut d, &mut h, &tone(0, 3.0), 3_000_000);
            let s: Vec<i16> = d.samples().iter().step_by(2).copied().collect();
            let mix = &s[passed - 100..passed + 2 * RATE as usize];
            let step = mix.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap_or(0);
            assert!(step < 700, "pitch {keep_pitch}: no jump in the ending, at most {step}");
            if !keep_pitch {
                // Varispeed: the pitch rises with the tempo, 440 Hz to 463 Hz over the bend.
                let bent = &s[passed + RATE as usize..passed + RATE as usize * 19 / 10];
                let crossings = bent.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
                assert!((crossings as i64 - 811).abs() <= 6, "the ending speeds up: {crossings} zero crossings");
            }
        }
    }

    /// The incoming song delayed half a second into a 2 s mix: silence on its deck until then, its first sample
    /// right after, all of it played, on a clock that never steps back and ends at its own time.
    #[test]
    fn a_delayed_incoming_song_starts_where_its_delay_ends() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        p.mixer.in_delay_ms = 500;
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(0, 3.0), 0);
        let (passed, mixed_from) = (d.samples().len() / 2, d.taken.len());
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(4000, 3.0), 3_000_000);
        assert!(e.play_to_end_of_stream(&mut d, &mut h));
        let mix: Vec<i16> = d.samples()[passed * 2..].iter().step_by(2).copied().collect();
        let at = RATE as usize / 2;
        assert!(mix[..at].iter().all(|&v| v == 0), "nothing of b before its delay");
        assert!(mix[at..at + 64].iter().all(|&v| v > 100), "b from its first sample on: {:?}", &mix[at..at + 4]);
        let made = mix.len() as f64 / RATE as f64;
        assert!((made - 3.5).abs() < 0.005, "the mix and all of b after its delay: {made} s");
        let pts: Vec<i64> = d.taken[mixed_from..].iter().map(|(_, at)| *at).collect();
        assert!(pts.windows(2).all(|w| w[0] <= w[1]), "the clock never steps back: {pts:?}");
        let (last, at) = d.taken.last().expect("b played");
        assert!((at + FMT.us(last.len()) - 6_000_000).abs() < 2_000, "b ends at its own time: {at}");
    }

    /// The outgoing song alone glides to 5 % faster over the mix's second second (varispeed), and holds it: more of
    /// it is held, and it speeds up with no jump.
    #[test]
    fn a_glided_ending_speeds_up_alone() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        p.mixer.keep_pitch = false;
        p.mixer.out_glide = Some(crate::types::Glide { window: crate::types::Window { start_ms: 1000, end_ms: 2000 }, to: 1.05 });
        let held = p.held_us();
        assert!((held - 2_025_000).abs() < 1000, "one second from 1 to 1.05 holds 25 ms more: {held}");
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        let a: Vec<u8> = (0..RATE as usize + FMT.bytes(held) / 4)
            .flat_map(|i| {
                let v = ((i as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() * 8000.0) as i16;
                [v, v]
            })
            .flat_map(i16::to_le_bytes)
            .collect();
        feed(&mut e, &mut d, &mut h, &a, 0);
        let passed = d.samples().len() / 2;
        assert!(matches!(e.flight.phase, Phase::Hold(h) if h.len == FMT.bytes(held)), "the glide's song time is held");
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(0, 3.0), 3_000_000);
        let s: Vec<i16> = d.samples().iter().step_by(2).copied().collect();
        let mix = &s[passed - 100..passed + 2 * RATE as usize];
        let step = mix.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap_or(0);
        assert!(step < 700, "no jump in the ending, at most {step}");
        // 440 Hz rising to 462 Hz: 0.9 s of the glide plays 0.92 s of the song.
        let glided = &s[passed + RATE as usize..passed + RATE as usize * 19 / 10];
        let crossings = glided.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        assert!((crossings as i64 - 810).abs() <= 6, "the ending speeds up: {crossings} zero crossings");
    }

    /// A bent mix over a looped ending (varispeed): the half-second slice, 220 periods of 440 Hz, repeats with no
    /// jump and speeds up over the bend; what follows it in the song (880 Hz) is never heard.
    #[test]
    fn a_bent_mix_over_a_looped_ending_speeds_the_loop_up() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        (p.mixer.out_loop_ms, p.mixer.tempo_ratio, p.mixer.keep_pitch) = (Some(500), 0.95, false);
        p.mixer.bend = Some(crate::types::Bend { start_ms: 1000, end_ms: 2000 });
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        let a: Vec<u8> = (0..3 * RATE as usize)
            .flat_map(|i| {
                let hz = if i < 3 * RATE as usize / 2 { 440.0 } else { 880.0 };
                let v = ((i as f64 * hz * std::f64::consts::TAU / RATE as f64).sin() * 8000.0) as i16;
                [v, v]
            })
            .flat_map(i16::to_le_bytes)
            .collect();
        feed(&mut e, &mut d, &mut h, &a, 0);
        let passed = d.samples().len() / 2;
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(0, 3.0), 3_000_000);
        let s: Vec<i16> = d.samples().iter().step_by(2).copied().collect();
        let mix = &s[passed - 100..passed + 2 * RATE as usize];
        let step = mix.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap_or(0);
        assert!(step < 700, "no jump in the looped ending, at most {step}");
        let crossings = |from: f64, to: f64| {
            let w = &s[passed + (from * RATE as f64) as usize..passed + (to * RATE as f64) as usize];
            w.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as i64
        };
        assert!((crossings(0.5, 1.0) - 440).abs() <= 3, "the slice again, not what follows it: {}", crossings(0.5, 1.0));
        assert!((crossings(1.0, 1.9) - 811).abs() <= 6, "the loop speeds up: {} zero crossings", crossings(1.0, 1.9));
    }

    /// The next song is in a format the output cannot be converted to (six channels): the held ending
    /// plays out once and the next song plays alone, from its start, in its own format.
    #[test]
    fn unconvertible_next_song_plays_after_the_hold() {
        const F6: Format = Format { channels: 6, ..FMT };
        for looped in [false, true] {
            let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
            let mut p = fade("b", 1_000_000);
            p.mixer.out_loop_ms = looped.then_some(500);
            h.plans.insert("a".into(), p);
            e.configure(&mut d, &mut h, stream("a", FMT));
            feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
            e.configure(&mut d, &mut h, stream("b", F6));
            e.handle_discontinuity(&mut d, &mut h);
            let b: Vec<u8> = (0..RATE as usize * 6).flat_map(|_| (-8000i16).to_le_bytes()).collect();
            feed_in(&mut e, &mut d, &mut h, &b, F6, 3_000_000);
            assert_eq!(d.configured.last(), Some(&F6), "looped {looped}: the output follows b");
            let s = d.samples();
            let a = s.len() - RATE as usize * 6;
            assert!(a >= RATE as usize * 2, "looped {looped}: a plays up to its ending");
            assert!(s[..a].iter().all(|&v| v == 8000), "looped {looped}: a plays once, as it was held");
            assert!(s[a..].iter().all(|&v| v == -8000), "looped {looped}: then all of b, alone");
        }
    }

    /// A song mixed into itself (queued twice): the second copy is a stream of its own, whose ending is
    /// still to come after the first copy's was made.
    #[test]
    fn same_song_twice_is_two_streams() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("a", 2_500_000));
        let copy = |serial| StreamFormat { id: StreamId { song: "a".into(), serial }, format: FMT };
        e.configure(&mut d, &mut h, copy(1));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        e.configure(&mut d, &mut h, copy(2));
        e.handle_discontinuity(&mut d, &mut h);
        e.set_output_stream_offset_us(3_000_000);
        feed(&mut e, &mut d, &mut h, &tone(-8000, 0.5), 3_000_000);
        assert!(e.made(1).is_some_and(|p| p.is_some()), "the first copy's ending was mixed");
        assert_eq!(e.made(2), None, "the second copy's ending is still to come");
        feed(&mut e, &mut d, &mut h, &tone(-8000, 2.5), 3_500_000);
        assert!(e.holding() && e.made(2).is_some(), "and is held at its own plan: {}", e.words());
    }

    /// Feeds `data` in 4096-byte buffers, re-offering the rest of partly taken ones. Buffer `at_buffer`
    /// is only partly taken (1024 bytes); `between` runs between re-offers and after that buffer.
    fn offer(e: &mut TransitionEngine, d: &mut Down, h: &mut Host_, data: &[u8], at_buffer: usize, mut between: impl FnMut(&mut TransitionEngine)) {
        for (i, slice) in data.chunks(4096).enumerate() {
            if i == at_buffer {
                d.take_only = Some(1024);
            }
            let mut from = 0;
            loop {
                let (all, used) = e.handle_buffer(d, h, &slice[from..], FMT.us(i * 4096 + from));
                from += used;
                if all {
                    break;
                }
                between(e);
            }
            if i == at_buffer {
                between(e);
            }
        }
    }

    #[test]
    fn rescale_reaches_partial_buffer() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.configure(&mut d, &mut h, stream("a", FMT));
        e.set_gain(0.5);
        let mut changed = false;
        offer(&mut e, &mut d, &mut h, &tone(1000, 1.0), 10, |e| {
            if !std::mem::replace(&mut changed, true) {
                e.rescale(0, 0.25 / 0.5);
                e.set_gain(0.25);
            }
        });
        let s = d.samples();
        let cut = (10 * 4096 + 1024) / 2;
        assert_eq!(s.len(), tone(1000, 1.0).len() / 2, "every sample, once");
        assert!(s[..cut].iter().all(|&v| v == 500), "what was taken before the change at the old level");
        assert!(s[cut..].iter().all(|&v| v == 250), "and everything after it at the new one, the rest of that buffer too");
    }

    #[test]
    fn rescale_reaches_held_ending() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        d.position = Some(0);
        h.plans.insert("a".into(), fade("b", 2_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        e.set_gain(0.5);
        feed(&mut e, &mut d, &mut h, &tone(1000, 3.0), 0);
        assert!(e.holding(), "the ending is held");
        e.rescale(0, 0.25 / 0.5);
        e.abandon_transition(&mut h);
        e.drain(&mut d);
        let s = d.samples();
        let held = FMT.bytes(2_000_000) / 2;
        assert!(s[..held].iter().all(|&v| v == 500) && s[held..].iter().all(|&v| v == 250) && s.len() > held, "held at 0.5, let go at 0.25");
    }

    #[test]
    fn no_plan_plays_gapless() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(100, 1.0), 0);
        assert_eq!(d.taken[1].1, FMT.us(4096), "timestamps are the decoder's");
        e.configure(&mut d, &mut h, stream("b", FMT));
        assert_eq!(h.analysed, vec!["a".to_string()], "a's analysis handed over on the next stream");
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(200, 1.0), 1_000_000);
        let s = d.samples();
        let half = RATE as usize * 2;
        assert!(s.len() == 2 * half && s[..half].iter().all(|&v| v == 100) && s[half..].iter().all(|&v| v == 200), "a then b, whole");
        assert_eq!(d.discontinuities, 1);
        assert_eq!(d.configured.len(), 1, "the first stream pins the output; the same format keeps it open");
    }

    #[test]
    fn unmixed_ending_released_in_time() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(1000, 3.0), 0);
        let before = d.samples().len();
        h.now += HOLD_GRACE_MS + 1;
        d.position = Some(900_000);
        e.position_us(&mut d, &mut h, false);
        assert!(h.log.iter().any(|l| l.contains("letting the ending play")), "{:?}", h.log);
        assert_eq!(d.samples().len() - before, FMT.bytes(2_000_000) / 2, "the held ending went out unmixed");
    }

    /// An ending let play for want of runway stays let go: the next buffers of the song do not ask
    /// for the plan again and hold it late.
    #[test]
    fn abandoned_ending_is_not_held_late() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_ { now: 60_000, ..Host_::default() });
        h.plans.insert("a".into(), fade("b", 1_000_000));
        d.position = Some(999_000);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(1000, 3.0), 0);
        assert!(h.log.iter().any(|l| l.contains("no runway")), "{:?}", h.log);
        assert!(!h.log.iter().any(|l| l.contains("late hold")), "{:?}", h.log);
        assert_eq!(d.samples().len(), (RATE as usize * 3) * 2, "everything played straight through");
    }

    /// A seek or remake landing at the plan's start holds there although the sink has little left.
    #[test]
    fn landing_at_the_start_holds() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        d.position = Some(999_000);
        e.flush(&mut h);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(1000, 2.0), 1_000_000);
        assert!(h.log.iter().any(|l| l.contains("holding the ending")), "{:?}", h.log);
        assert!(!h.log.iter().any(|l| l.contains("no runway")), "{:?}", h.log);
        assert!(d.samples().is_empty(), "the ending waits for the next song");
    }

    #[test]
    fn seek_into_mix_mixes_on() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        // From 1.5 s: half a second into the fade.
        feed(&mut e, &mut d, &mut h, &tone(1000, 1.5), 1_500_000);
        assert!(h.log.iter().any(|l| l.contains("late hold, 500 ms in")), "{:?}", h.log);
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        // Takeover is 1 s into the fade, so 1 s into b.
        assert_eq!(e.heard().next_from_us, 1_000_000, "the next song is entered where the fade hands it over");
    }

    #[test]
    fn heard_lags_while_holding() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(1000, 3.0), 0);
        d.position = Some(500_000);
        let reported = e.position_us(&mut d, &mut h, false).unwrap();
        assert!(reported > 500_000, "the held ending counts as played so the next track is read in time");
        let heard = e.heard();
        assert_eq!(heard.id, Some(b'a' as u64));
        assert_eq!(heard.us, 500_000);
        // The fade starts at 1 s and crosses over at its middle.
        assert!((heard.until_us - 2_000_000).abs() <= 23, "the ear leaves a where b becomes the louder, to the frame: {}", heard.until_us);
    }

    #[test]
    fn stretch_returns_to_track_clock() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        (p.mixer.tempo_ratio, p.mixer.tempo_ramp_ms) = (1.03, 500);
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(4000, 3.0), 0);
        e.configure(&mut d, &mut h, stream("b", FMT));
        e.handle_discontinuity(&mut d, &mut h);
        feed(&mut e, &mut d, &mut h, &tone(-4000, 6.0), 3_000_000);
        assert!(d.discontinuities >= 2, "a resync into the mix and one back onto real timestamps: {}", d.discontinuities);
        let s = d.samples();
        assert!(s[s.len() - 10..].iter().all(|&v| (v + 4000).abs() <= 1), "after the stretch the track plays as it is");
        // Regression: the stretcher's last audio was stamped 0, pulling the clock back so the next
        // hold was never released. Timestamps must only move forward from the mix on.
        let from_mix: Vec<i64> = d.taken.iter().map(|(_, p)| *p).skip_while(|&p| p < 1_000_000).collect();
        assert!(from_mix.windows(2).all(|w| w[1] >= w[0]), "{from_mix:?}");
    }

    /// Seconds of output at `f` taken after the first `from` bytes.
    fn secs_after(d: &Down, from: usize, f: Format) -> f64 {
        let got = d.taken.iter().map(|(b, _)| b.len()).sum::<usize>() - from;
        got as f64 / f.frame_bytes() as f64 / f.rate as f64
    }

    fn taken(d: &Down) -> usize {
        d.taken.iter().map(|(b, _)| b.len()).sum()
    }

    #[test]
    fn pinned_format_not_converted() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.configure(&mut d, &mut h, stream("a", F48));
        feed_in(&mut e, &mut d, &mut h, &tone_at(1000, 48_000, 0.5), F48, 0);
        e.handle_discontinuity(&mut d, &mut h);
        e.configure(&mut d, &mut h, stream("b", FMT));
        let before = taken(&d);
        feed(&mut e, &mut d, &mut h, &tone(700, 1.0), 1_000_000);
        let secs = secs_after(&d, before, F48);
        assert!((secs - 1.0).abs() < 0.01, "the 44.1 kHz song is converted: {secs}");
        e.handle_discontinuity(&mut d, &mut h);
        e.configure(&mut d, &mut h, stream("c", F48));
        let before = taken(&d);
        let c = tone_at(300, 48_000, 1.0);
        feed_in(&mut e, &mut d, &mut h, &c, F48, 2_000_000);
        // The converter's leftover of b may come first; then c untouched.
        let got = taken(&d) - before;
        assert!(got <= c.len() + 64 && got + 64 >= c.len(), "c goes down sample for sample: {got} of {} bytes", c.len());
        let s = d.samples();
        assert!(s[s.len() - 1000..].iter().all(|&v| v == 300));
        assert_eq!(d.configured.len(), 1);
    }

    #[test]
    fn mix_into_other_rate_converts_it() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        e.configure(&mut d, &mut h, stream("b", F48));
        e.handle_discontinuity(&mut d, &mut h);
        feed_in(&mut e, &mut d, &mut h, &tone_at(-8000, 48_000, 3.0), F48, 3_000_000);
        let frames = d.samples().len() / 2;
        // 1 s alone + b's 3 s, less the converter's lookahead (< 2 ms).
        let want = RATE as usize * 4;
        assert!(frames.abs_diff(want) < 160, "{frames} frames, not {want}");
        let s = d.samples();
        // Dithered after conversion: within a step.
        assert!(s[s.len() - 1000..].iter().all(|&v| (v + 8000).abs() <= 1), "b plays alone after the fade");
        assert_eq!(d.configured.len(), 1);
    }

    #[test]
    fn stretched_mix_into_other_rate() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        let mut p = fade("b", 1_000_000);
        (p.mixer.tempo_ratio, p.mixer.tempo_ramp_ms) = (1.03, 500);
        h.plans.insert("a".into(), p);
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(4000, 3.0), 0);
        e.configure(&mut d, &mut h, stream("b", F48));
        e.handle_discontinuity(&mut d, &mut h);
        feed_in(&mut e, &mut d, &mut h, &tone_at(-4000, 48_000, 6.0), F48, 3_000_000);
        let frames = d.samples().len() / 2;
        // 1 s alone, then 6 s of b with the first 2.5 s up to 3 % fast.
        let secs = frames as f64 / RATE as f64;
        assert!((6.8..7.05).contains(&secs), "{secs} s");
        let s = d.samples();
        assert!(s[s.len() - 10..].iter().all(|&v| (v + 4000).abs() <= 1), "after the stretch the track plays as it is");
    }

    #[test]
    fn gapless_follows_next_rate() {
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.follow_rate = true;
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(1000, 1.0), 0);
        e.configure(&mut d, &mut h, stream("b", F48));
        e.handle_discontinuity(&mut d, &mut h);
        let before = taken(&d);
        let b = sine(HZ, 48_000, 1.0, 0);
        feed_in(&mut e, &mut d, &mut h, &b, F48, 1_000_000);
        assert_eq!(d.configured.len(), 2, "opened again for b");
        let got: Vec<u8> = d.taken.iter().flat_map(|t| t.0.iter().copied()).skip(before).collect();
        assert!(got == b, "b sample for sample");
        assert!(!h.log.iter().any(|l| l.contains("converting")), "{:?}", h.log);
        // Mixed into, it is still converted.
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.follow_rate = true;
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(8000, 3.0), 0);
        e.handle_discontinuity(&mut d, &mut h);
        e.configure(&mut d, &mut h, stream("b", F48));
        feed_in(&mut e, &mut d, &mut h, &tone_at(-8000, 48_000, 3.0), F48, 3_000_000);
        assert_eq!(d.configured.len(), 1, "the mix at a's rate");
        assert!(h.log.iter().any(|l| l.contains("converting 48000 Hz x2 -> 44100 Hz x2")), "{:?}", h.log);
    }

    #[test]
    fn converter_carries_across_join() {
        // a (44.1) mixes into b (48, converted); c (48) follows b gaplessly. The converter must carry
        // on (no reopen, no reset click): a tone through b into c stays one tone.
        let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
        e.follow_rate = true;
        h.plans.insert("a".into(), fade("b", 1_000_000));
        e.configure(&mut d, &mut h, stream("a", FMT));
        feed(&mut e, &mut d, &mut h, &tone(0, 3.0), 0);
        e.handle_discontinuity(&mut d, &mut h);
        e.configure(&mut d, &mut h, stream("b", F48));
        let b = sine(HZ, 48_000, 4.0, 0);
        feed_in(&mut e, &mut d, &mut h, &b, F48, 3_000_000);
        e.configure(&mut d, &mut h, stream("c", F48));
        e.handle_discontinuity(&mut d, &mut h);
        let from = d.samples().len() / 2;
        feed_in(&mut e, &mut d, &mut h, &sine(HZ, 48_000, 1.0, 48_000 * 4), F48, 7_000_000);
        assert_eq!(d.configured.len(), 1, "no opening again between b and c");
        assert_eq!(h.log.iter().filter(|l| l.contains("converting 48000")).count(), 1, "one converter, b's: {:?}", h.log);
        let left: Vec<f64> = d.samples().as_chunks::<2>().0.iter().map(|c| c[0] as f64).collect();
        let own = 8000.0 * std::f64::consts::TAU * HZ / RATE as f64;
        let near = &left[from - 2_000..from + 2_000];
        let step = near.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        assert!(step <= own * 1.01 + 2.0, "a step of {step:.0} at the join, the tone's own {own:.0}");
        let hz = last_second_hz(&d, FMT);
        assert!((hz - HZ).abs() < 1.0, "c at its own pitch, converted: {hz}");
    }

    // ---- tempo after a beat-matched mix, a seek, a skip or a pause ----

    const F48: Format = Format { rate: 48_000, ..FMT };
    const HZ: f64 = 1000.0;

    /// `secs` of a stereo 16-bit sine of `hz` at `rate`, starting at frame `from`.
    fn sine(hz: f64, rate: u32, secs: f64, from: usize) -> Vec<u8> {
        let frames = (rate as f64 * secs) as usize;
        (from..from + frames)
            .flat_map(|k| {
                let v = ((k as f64 * hz * std::f64::consts::TAU / rate as f64).sin() * 8000.0).round() as i16;
                [v, v]
            })
            .flat_map(i16::to_le_bytes)
            .collect()
    }

    /// Pitch of the last second taken, from zero crossings.
    fn last_second_hz(d: &Down, f: Format) -> f64 {
        let s = d.samples();
        let left: Vec<i16> = s.as_chunks::<2>().0.iter().map(|c| c[0]).collect();
        let w = &left[left.len() - f.rate as usize..];
        w.windows(2).filter(|p| (p[0] < 0) != (p[1] < 0)).count() as f64 / 2.0
    }

    /// A 2 s beat-matched mix into `c` 1 s in, `c` 4 % fast (pitch too), ramped back over 1 s.
    fn beat_matched() -> Plan {
        let mut p = fade("c", 1_000_000);
        (p.mixer.tempo_ratio, p.mixer.keep_pitch, p.mixer.tempo_ramp_ms) = (1.04, false, 1000);
        p
    }

    /// `a` (48 kHz) pins the output, `b` (44.1) is converted, `b` mixes into `c` (48); returns with
    /// `secs` of `c` fed.
    fn into_the_mix(e: &mut TransitionEngine, d: &mut Down, h: &mut Host_, secs: f64) {
        h.plans.insert("b".into(), beat_matched());
        e.configure(d, h, stream("a", F48));
        feed_in(e, d, h, &sine(HZ, 48_000, 1.0, 0), F48, 0);
        e.configure(d, h, stream("b", FMT));
        e.handle_discontinuity(d, h);
        e.set_output_stream_offset_us(1_000_000);
        feed_in(e, d, h, &sine(HZ, RATE, 3.0, 0), FMT, 1_000_000);
        assert!(h.log.iter().any(|l| l.starts_with("holding the ending")), "{:?}", h.log);
        e.configure(d, h, stream("c", F48));
        e.handle_discontinuity(d, h);
        e.set_output_stream_offset_us(4_000_000);
        feed_in(e, d, h, &sine(HZ, 48_000, secs, 0), F48, 4_000_000);
    }

    fn assert_own_pitch(d: &Down, what: &str) {
        let hz = last_second_hz(d, F48);
        assert!((hz - HZ).abs() < 3.0, "{what}: the song plays at {hz} Hz, not {HZ} (x{:.4})", hz / HZ);
    }

    /// After a beat-matched mix the song is back at its own pitch, however it was left.
    #[test]
    fn tempo_restored() {
        type Then = fn(&mut TransitionEngine, &mut Down, &mut Host_);
        let seek: Then = |e, d, h| {
            e.flush(h);
            feed_in(e, d, h, &sine(HZ, 48_000, 3.0, 48_000 * 20), F48, 24_000_000);
        };
        let cases: [(&str, f64, Then); 5] = [
            ("after the mix and the ramp", 6.0, |_, _, _| {}),
            ("after a seek in the mix", 0.5, seek),
            // 0.5 s after the 2 s mix: inside the ramp.
            ("after a seek in the ramp", 2.5, seek),
            ("after a skip in the mix", 0.5, |e, d, h| {
                for (k, id) in ["d", "c"].into_iter().enumerate() {
                    e.flush(h);
                    e.configure(d, h, stream(id, F48));
                    feed_in(e, d, h, &sine(HZ, 48_000, 3.0, 0), F48, 30_000_000 + k as i64 * 10_000_000);
                    assert_own_pitch(d, id);
                }
            }),
            ("after a pause in the mix", 0.5, |e, d, h| {
                // Paused: only position queries, nothing flowing.
                for _ in 0..50 {
                    h.now += 100;
                    e.position_us(d, h, false);
                }
                feed_in(e, d, h, &sine(HZ, 48_000, 5.0, 24_000), F48, 4_500_000);
            }),
        ];
        for (what, secs, then) in cases {
            let (mut e, mut d, mut h) = (TransitionEngine::new(), Down::default(), Host_::default());
            into_the_mix(&mut e, &mut d, &mut h, secs);
            then(&mut e, &mut d, &mut h);
            assert_own_pitch(&d, what);
        }
    }
}
