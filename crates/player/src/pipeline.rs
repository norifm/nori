//! The platform-free player: walks the queue one song at a time, feeds the transition engine in
//! bursts, and outputs through [`Sink`] (media3's AudioSink with nori's processors) into a [`Track`].
//! `sim` and `nori-engine` both run this with their own [`Songs`], [`Track`], [`App`] and clock.

use crate::burst::{Burst, Fed, BUFFER_US};
use crate::engine::{Heard, Host, StreamFormat, StreamId, TransitionEngine};
use crate::heard::{HeardTracker, PlayerNow, Seen, StreamAt};
use crate::pcm::Format;
use crate::playlist::Playlist;
use crate::queue::{measure_ahead, ErrorRun, OnError, PlaybackError};
use crate::stage::{no_facts, Preparing, SongFacts, SongStages, StageSpec, Tap};
use std::sync::Arc;
use crate::transitions::WindowSong;
use nori_time::Instant;

pub use crate::sink::{blended, ChainSettings, Remake, Sink, Sound, Track, BLEND_US};

/// Start of the renderer's timeline, as in media3.
pub(crate) const BASE_OFFSET_US: i64 = 1_000_000_000_000;
/// The next song is read once the current one's end is this close.
pub(crate) const READ_AHEAD_US: i64 = 10_000_000;
/// Most buffers offered per turn.
const BUFFERS_PER_TURN: usize = 256;

/// One song opened for reading, as decoded interleaved buffers (16-bit, or float for high quality).
pub trait Reading {
    fn format(&self) -> Format;
    /// Length, µs, as far as known (exact once read to the end).
    fn duration_us(&self) -> i64;
    /// Open and the next buffer's bytes have arrived. Asked again next turn if not; nothing else is
    /// called before the first `true`.
    fn ready(&mut self) -> bool {
        true
    }
    /// Why the song cannot play on (failed to open, or its bytes stopped for good). Asked once ready
    /// and at the end.
    fn error(&self) -> Option<(PlaybackError, String)> {
        None
    }
    /// Decodes the next buffer; false at the end.
    fn fill(&mut self) -> bool;
    /// The buffer [`Reading::fill`] made.
    fn buffer(&self) -> &[u8];
    /// Song time of the buffer's start, µs.
    fn at_us(&self) -> i64;
    /// Bits per sample stored in the file (0 unknown).
    fn bits(&self) -> u32 {
        0
    }
    /// Decodes and drops everything before `ms`, before the first buffer is handed out. False if it
    /// cannot (the song is then reopened there).
    fn skip_to_ms(&mut self, _ms: i64) -> bool {
        false
    }
    /// Its bytes are no longer coming in (all here, or as many as it holds at once): the network is
    /// free for the next song's.
    fn fetched(&mut self) -> bool {
        true
    }
}

/// Opens the songs a queue names.
pub trait Songs {
    type Reading: Reading;
    /// Opens `id` from `from_ms`. An error is a song that will not play.
    fn open(&mut self, id: &str, from_ms: i64) -> Result<Self::Reading, String>;
    /// Length as tagged, album and track number, for the planner and seek bar.
    fn about(&self, id: &str) -> WindowSong;
    /// `id` plays next; a platform may prefetch it.
    fn upcoming(&mut self, _id: &str) {}
}

/// The app around the player: planner, analysis store, log.
pub trait App: Host {
    /// The clock for the next engine calls ([`Host::now_ms`] returns it).
    fn clock(&mut self, now_ms: i64);
    /// AutoMix is on (upcoming songs are measured).
    fn auto_mix(&self) -> bool;
    /// The planner's window: the ids of the previous song, then the current and following in play
    /// order. `songs` describes them.
    fn window<S: Songs>(&mut self, songs: &S, ids: &[String], shuffling: bool);
    /// Measures the unmeasured songs among `ids`.
    fn measure_ahead<S: Songs>(&mut self, _songs: &mut S, _ids: &[String]) {}
    /// Output moved to a device: its key and its bound sound, if any. `None` when the app does not
    /// track devices.
    fn output_changed(&mut self, _kind: crate::outputs::OutputKind, _name: &str) -> Option<(String, Option<Sound>)> {
        None
    }
    /// Songs were measured in the background since last asked: replan.
    fn measured(&mut self) -> bool {
        false
    }
    /// The output forbids touching samples, so the planner disables transitions.
    fn transitions_off(&mut self, _off: bool) {}
    /// Other devices play in step with this one (a hosted jam's guests listening along): its place is
    /// said wherever the music written moves it a few ms.
    fn listened_along(&self) -> bool {
        false
    }
    /// What to do about a failed song when the app tracks the failure run itself; `None` uses the
    /// player's [`ErrorRun`].
    fn on_error(&mut self, _kind: PlaybackError, _has_next: bool) -> Option<OnError> {
        None
    }
    /// Audio is coming out: resets the failure run.
    fn playing(&mut self) {}
    /// What a change of the audio focus does where nori would do `nori`; a plugin may answer instead.
    fn audio_focus(&mut self, _focus: crate::focus::Focus, nori: crate::focus::FocusAct) -> crate::focus::FocusAct {
        nori
    }
    /// ReplayGain for index `index` of `list`, the queue as the player holds it (capped by
    /// [`Player::gain_max`]), applied per song by [`TransitionEngine::set_gain`].
    fn gain(&mut self, _list: &Playlist, _index: usize) -> f32 {
        1.0
    }
    /// Song `id`'s measured integrated loudness, LUFS, for the stages; asked only while there are any.
    fn loudness(&mut self, _id: &str) -> Option<f32> {
        None
    }
    /// The sound changed (`what`: the chain or the gain) from output frame `at.output`, made from input
    /// frame `at.input` (frames since the last flush).
    fn spliced(&mut self, what: &str, at: Splice) {
        self.log(&format!("the {what} changes from output frame {} (input frame {})", at.output, at.input));
    }
}

/// Where a sound change starts: frames since the last flush, of the chain's input and of the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Splice {
    pub input: u64,
    pub output: u64,
}

/// Where the playlist is kept: the player's own, or the core's.
pub trait Queue {
    fn read<R>(&self, f: impl FnOnce(&Playlist) -> R) -> R;
    /// The player moved to `index` by itself.
    fn moved_to(&mut self, index: usize);
    /// Arriving on index `index` of `list` would skip it (explicit song, skip setting on).
    fn skips(&self, _list: &Playlist, _index: usize) -> bool {
        false
    }
}

/// The queue as the player last took it ([`Player::queue_changed`]): another thread edits `live`, and
/// the indexes the player holds keep naming songs of the list it reads until it is told.
pub struct Known<Q> {
    pub live: Q,
    list: Playlist,
}

impl<Q: Queue> Known<Q> {
    fn new(live: Q) -> Self {
        let list = live.read(Playlist::clone);
        Known { live, list }
    }

    fn take_edits(&mut self) {
        self.list = self.live.read(Playlist::clone);
    }

    pub fn read<R>(&self, f: impl FnOnce(&Playlist) -> R) -> R {
        f(&self.list)
    }

    pub fn moved_to(&mut self, index: usize) {
        self.list.moved_to(index);
        self.live.moved_to(index);
    }

    /// The client already set `mode` on the live queue; a press told late must not overwrite a newer one.
    fn set_repeat(&mut self, mode: u8) {
        self.list.set_repeat(mode);
    }

    pub fn skips(&self, index: usize) -> bool {
        self.live.skips(&self.list, index)
    }
}

impl Queue for Playlist {
    fn read<R>(&self, f: impl FnOnce(&Playlist) -> R) -> R {
        f(self)
    }

    fn moved_to(&mut self, index: usize) {
        Playlist::moved_to(self, index);
    }
}

/// A stream handed to the output: song index, start in renderer time, length, applied gain, and the
/// serial that tells it from another stream of the same song ([`StreamId`]).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Period {
    index: usize,
    offset_us: i64,
    duration_us: i64,
    gain: f32,
    serial: u64,
}

/// How far a [`Reader`] has read its song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bytes {
    Flowing,
    /// The last turn found the next buffer's bytes still missing.
    Missing,
    /// Read to its end.
    Ended,
}

/// The song being read and its start in renderer time.
struct Reader<R> {
    index: usize,
    offset_us: i64,
    r: R,
    pos: usize,
    bytes: Bytes,
    /// The song slot's stages, each buffer read through them once.
    stages: Option<SongStages>,
    /// Where plugins hear the song as it is read, and the buffer it is converted in.
    tap: Option<Box<dyn Tap>>,
    heard: Vec<f32>,
}

/// A song opened from `from_ms`, not ready yet.
struct Opening<R> {
    index: usize,
    from_ms: i64,
    offset_us: i64,
    r: R,
}

impl<R: Reading> Reader<R> {
    fn new(index: usize, offset_us: i64, r: R, stages: Option<SongStages>, tap: Option<Box<dyn Tap>>) -> Reader<R> {
        Reader { index, offset_us, r, pos: 0, bytes: Bytes::Flowing, stages, tap, heard: Vec::new() }
    }

    /// Decodes the next buffer: heard by the tap as it is, then run through the song slot's stages.
    fn fill(&mut self) -> bool {
        self.pos = 0;
        let got = self.r.fill();
        self.bytes = if got { Bytes::Flowing } else { Bytes::Ended };
        match (got, self.tap.as_mut()) {
            (true, Some(t)) => {
                // Only a longer buffer grows it.
                self.heard.clear();
                crate::pcm::to_f32(self.r.buffer(), self.r.format().encoding, &mut self.heard);
                t.samples(&self.heard);
            }
            (false, Some(t)) => {
                if self.r.error().is_none() {
                    t.end();
                }
                self.tap = None;
            }
            (_, None) => {}
        }
        if let (true, Some(s)) = (got, self.stages.as_mut()) {
            s.run(self.r.buffer());
        }
        got
    }

    /// The buffer read, as the song slot made it.
    fn buffer(&self) -> &[u8] {
        match &self.stages {
            Some(s) => s.last(),
            None => self.r.buffer(),
        }
    }

    fn left(&self) -> bool {
        self.pos < self.buffer().len()
    }

    fn ended(&self) -> bool {
        self.bytes == Bytes::Ended
    }
}

/// A song that will not play, found ahead of playback: queue index, kind, reason.
type Failure = (usize, PlaybackError, String);

/// What follows the song being read.
enum After<R> {
    Unopened,
    /// The next song, opened once this one was read to its end; `waited`: the last turn would have
    /// moved on to it but its first bytes were missing.
    Opened { index: usize, r: R, waited: bool },
    /// The song read or the next will not play: raised once everything before it has played.
    Failed(Failure),
}

/// A song being read, what follows it, and the audible song opened again to make its ending anew
/// ([`Player::replan_ending`]).
struct Read<R> {
    reader: Reader<R>,
    after: After<R>,
    remaking: Option<Opening<R>>,
}

/// Whether the player makes music and plays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Paused,
    /// Paused, the music made ready for a start at a set moment.
    Priming,
    Playing,
}

/// Why playback stopped at a failed song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Halt {
    /// The failure rules stopped it.
    Stop,
    /// It failed for lack of network and the offline bridge takes over.
    Bridge,
    /// It failed for lack of network with no bridge: it waits for the network.
    Wait,
}

/// Where reading is.
enum Head<R> {
    Idle,
    /// A song to read after a jump or seek, still opening.
    Opening(Opening<R>),
    Reading(Read<R>),
    /// Stopped at failed song `index`, `ms` into it; nothing is read until a jump or seek.
    Stopped { index: usize, ms: i64, halt: Halt },
}

impl<R> Head<R> {
    fn read(&self) -> Option<&Read<R>> {
        match self {
            Head::Reading(r) => Some(r),
            _ => None,
        }
    }

    fn read_mut(&mut self) -> Option<&mut Read<R>> {
        match self {
            Head::Reading(r) => Some(r),
            _ => None,
        }
    }

    fn reader(&self) -> Option<&Reader<R>> {
        self.read().map(|r| &r.reader)
    }

    fn reader_mut(&mut self) -> Option<&mut Reader<R>> {
        self.read_mut().map(|r| &mut r.reader)
    }

    /// The song being read or opening.
    fn index(&self) -> Option<usize> {
        match self {
            Head::Reading(r) => Some(r.reader.index),
            Head::Opening(o) => Some(o.index),
            _ => None,
        }
    }

    fn failed(&self) -> bool {
        self.read().is_some_and(|r| matches!(r.after, After::Failed(_)))
    }
}

/// The player: a queue, one song read at a time, and the transition engine in front of the output.
/// Every engine call goes through [`Fed`] with the player's clock.
pub struct Player<S: Songs, T: Track, A: App, Q: Queue> {
    pub now_ms: i64,
    pub engine: TransitionEngine,
    pub burst: Burst,
    pub sink: Sink<T>,
    pub app: A,
    pub queue: Known<Q>,
    pub tracks: S,
    pub tracker: HeardTracker,
    /// Measure upcoming songs whenever the queue moves (with AutoMix on).
    pub measure_on_move: bool,
    /// ReplayGain off (bit-perfect output). Call [`Player::gain_changed`] after changing it.
    pub gain_off: bool,
    /// Maximum gain: 1 unless float output with the limiter allows boosting (`nori_player::gain`).
    pub gain_max: f32,
    head: Head<S::Reading>,
    /// The next song last prefetched ([`Songs::upcoming`]), and whether it waits for the song read to
    /// have its bytes first ([`Player::fetch_when_free`]).
    upcoming: Option<String>,
    fetch_after: bool,
    /// Where the song being read started; cleared once the clock passes it (audio is audible).
    heard_from: Option<i64>,
    periods: Vec<Period>,
    mode: Mode,
    /// Renderer position: from a seek or start, then from the engine once the output has a clock.
    position_us: Option<i64>,
    /// When `position_us` was read or set: a turn reads it before it renders, which takes a while.
    placed_at: Instant,
    current: Option<usize>,
    /// Song changes with their time; the platform drains them.
    pub changes: Vec<(i64, usize)>,
    /// Failed songs (id, reason); the platform drains them.
    pub failures: Vec<(String, String)>,
    pub errors: ErrorRun,
    /// Stop at the end of this song ([`Player::pause_at_end`]).
    stop_after: Option<usize>,
    /// The last turn hit [`BUFFERS_PER_TURN`] with the output still taking.
    hungry: bool,
    /// Start of the stream the clock is in; a new stream of the same song is a repeat-one loop.
    heard_period: Option<i64>,
    /// The serial of the stream the clock is in, and the last serial given.
    on_serial: Option<u64>,
    serials: u64,
    /// Repeat-one loops heard.
    pub loops: u32,
    /// Queue entries as last seen, to map indexes across edits.
    seqs: Vec<u64>,
    /// Plugins' song-slot stages, made for each song read.
    song_stages: Vec<StageSpec>,
    /// What plugins make of the songs read, and made.
    preparing: Option<Arc<dyn Preparing>>,
    /// What the stages read of each stream's song, by its serial, while there are stages.
    facts: Vec<(u64, Arc<SongFacts>)>,
}

impl<S: Songs, T: Track, A: App, Q: Queue> Player<S, T, A, Q> {
    /// A player over `queue`, idle, writing into `track` through the deep buffer.
    pub fn build(tracks: S, queue: Q, app: A, track: T) -> Self {
        let mut p = Player {
            now_ms: 1_000,
            engine: TransitionEngine::new(),
            burst: Burst::default(),
            sink: Sink::new(BUFFER_US, ChainSettings::default(), track),
            app,
            queue: Known::new(queue),
            tracks,
            tracker: HeardTracker::new(),
            measure_on_move: true,
            gain_off: false,
            gain_max: 1.0,
            head: Head::Idle,
            upcoming: None,
            fetch_after: false,
            heard_from: None,
            periods: Vec::new(),
            mode: Mode::Paused,
            position_us: None,
            placed_at: Instant::now(),
            current: None,
            changes: Vec::new(),
            failures: Vec::new(),
            errors: ErrorRun::new(),
            stop_after: None,
            hungry: false,
            heard_period: None,
            on_serial: None,
            serials: 0,
            loops: 0,
            seqs: Vec::new(),
            song_stages: Vec::new(),
            preparing: None,
            facts: Vec::new(),
        };
        p.engine.follow_rate = true;
        p.seqs = p.queue.read(|q| q.seqs().to_vec());
        p.sync_queue();
        p
    }

    /// The id at list index `i`.
    pub fn id_at(&self, i: usize) -> String {
        self.queue.read(|q| q.ids()[i].clone())
    }

    fn next_of(&self, i: usize) -> Option<usize> {
        if self.stop_after == Some(i) {
            return None;
        }
        self.queue.read(|q| q.next_of(i, q.repeat())).map(|n| self.playable(n))
    }

    /// Stops at the end of the current song (sleep timer), as at the end of the queue; [`Player::ended`]
    /// reports it. Cleared by `false` or the next jump.
    pub fn pause_at_end(&mut self, on: bool) {
        let Some(c) = self.current.filter(|_| on) else {
            self.stop_after = None;
            return;
        };
        // Already reading the next song: re-read the rest of this one so nothing follows it.
        if self.head.reader().is_some_and(|r| r.index != c) {
            let at = self.position_ms();
            self.jump(c, at);
        }
        self.stop_after = Some(c);
    }

    /// The song [`Player::pause_at_end`] stops after.
    pub fn stopping_after(&self) -> Option<usize> {
        self.stop_after
    }

    /// `i`, or the first song after it that is not skipped on arrival.
    fn playable(&self, i: usize) -> usize {
        let mut at = i;
        for _ in 0..self.queue.read(|q| q.len()) {
            if !self.queue.skips(at) {
                return at;
            }
            match self.queue.read(|q| q.next_of(at, q.repeat())) {
                Some(n) if n != i => at = n,
                _ => return at,
            }
        }
        at
    }

    /// Calls into the engine with the output fed in bursts, on the player's clock.
    fn call<R>(&mut self, f: impl FnOnce(&mut TransitionEngine, &mut Fed<'_, Sink<T>>, &mut A) -> R) -> R {
        self.app.clock(self.now_ms);
        let mut fed = Fed::new(&mut self.sink, &mut self.burst, self.now_ms);
        f(&mut self.engine, &mut fed, &mut self.app)
    }

    fn configure(&mut self, i: usize, serial: u64, format: Format) {
        let s = StreamFormat { id: StreamId { song: self.id_at(i), serial }, format };
        self.call(|e, d, a| e.configure(d, a, s));
    }

    fn new_serial(&mut self) -> u64 {
        self.serials += 1;
        self.serials
    }

    /// The serial of song `i`'s latest stream, or a new one.
    fn serial_for(&mut self, i: usize) -> u64 {
        self.serial_at(i).unwrap_or_else(|| self.new_serial())
    }

    /// The serial of song `i`'s latest stream.
    fn serial_at(&self, i: usize) -> Option<u64> {
        self.periods.iter().rev().find(|p| p.index == i).map(|p| p.serial)
    }

    /// Starts reading song `i` (opened as `r`) from `from_ms` at `offset_us`, after a flush: now if
    /// ready, else once it is. False when it failed at once.
    fn begin(&mut self, i: usize, from_ms: i64, offset_us: i64, mut r: S::Reading) -> bool {
        self.head = Head::Idle;
        // A seek within the same song keeps its stream's identity.
        let serial = self.serial_for(i);
        self.periods = vec![Period { index: i, offset_us, duration_us: 0, gain: 1.0, serial }];
        self.on_serial = Some(serial);
        self.place(offset_us + from_ms * 1000);
        self.heard_period = None;
        if r.ready() {
            return self.start_reading(i, from_ms, offset_us, r);
        }
        self.head = Head::Opening(Opening { index: i, from_ms, offset_us, r });
        true
    }

    /// Starts reading the opening song once it is ready.
    fn opened(&mut self) {
        let Head::Opening(o) = &mut self.head else { return };
        if !o.r.ready() {
            return;
        }
        let Head::Opening(o) = std::mem::replace(&mut self.head, Head::Idle) else { unreachable!("matched above") };
        self.start_reading(o.index, o.from_ms, o.offset_us, o.r);
    }

    /// The plan out of the audible song is asked for again (settings, an analysis or the queue changed).
    /// If its ending was made otherwise and can still change, it is made again from where the old and
    /// new endings part. A mix already audible plays out as it began.
    pub fn replan_ending(&mut self) {
        if self.playing() {
            self.follow_clock();
        }
        self.engine.replan();
        if self.mixing() {
            return;
        }
        let Some((cur, _)) = self.ear() else { return };
        let Some(period) = self.periods.iter().rev().find(|p| p.index == cur).copied() else { return };
        let id = self.id_at(cur);
        // Read on gaplessly into a song that no longer follows.
        let astray = self.read_astray(cur);
        self.app.clock(self.now_ms);
        let plan = self.engine.plans.plan_for(&mut self.app, &id);
        let made = match self.ending_made(cur, plan.as_ref().map(|p| p.out_start_us())) {
            Some(made) => made,
            None if astray => None,
            None => return,
        };
        if made == plan && !astray {
            return;
        }
        let starts = [&made, &plan].map(|p| p.as_ref().map(|p| p.out_start_us()));
        let part_us = starts.into_iter().flatten().min().unwrap_or(period.duration_us).min(period.duration_us);
        let said = |p: &Option<crate::engine::Plan>| p.as_ref().map_or("gapless".to_string(), |p| format!("a mix from {} ms", p.out_start_us() / 1000));
        let why = if astray { "another song follows it now".to_string() } else { format!("{} now, {} as it was made", said(&plan), said(&made)) };
        self.app.log(&format!("the ending of {id} is made again from {} ms: {why}", part_us / 1000));
        self.remake_from(cur, part_us / 1000);
    }

    /// Opens song `i` again at `ms`, to read on from there once it is open ([`Player::remade`]).
    fn remake_from(&mut self, i: usize, ms: i64) {
        let Some(offset_us) = self.periods.iter().rev().find(|p| p.index == i).map(|p| p.offset_us) else { return };
        let id = self.id_at(i);
        let Some(read) = self.head.read_mut() else { return };
        match self.tracks.open(&id, ms) {
            Ok(r) => read.remaking = Some(Opening { index: i, from_ms: ms, offset_us, r }),
            Err(why) => self.app.log(&format!("{id} would not open again ({why}): its ending plays as it was made")),
        }
        self.remade();
    }

    /// The song opened again is open: the output from there on is dropped, if the output can still
    /// replace it, and the song read on from there for the transition engine to make what follows.
    fn remade(&mut self) {
        let Some(read) = self.head.read_mut() else { return };
        if !read.remaking.as_mut().is_some_and(|o| o.r.ready()) {
            return;
        }
        let o = read.remaking.take().expect("checked");
        let Some(period) = self.periods.iter().rev().find(|p| p.offset_us == o.offset_us).copied() else { return };
        let format = o.r.format();
        let frame = o.from_ms * format.rate as i64 / 1000;
        let pts = o.offset_us + frame * 1_000_000 / format.rate as i64;
        if o.r.error().is_some() || self.sink.format != Some(format) || !self.sink.cut_at(pts) {
            self.app.log(&format!("{} is not made again from {} ms: it plays as it was made", self.id_at(o.index), o.from_ms));
            return;
        }
        self.call(|e, _, a| e.flush(a));
        self.burst.restart();
        self.periods.retain(|p| p.offset_us <= o.offset_us);
        self.sink.track.source_bits(o.r.bits());
        let song = self.facts_for(period.serial);
        let stages = SongStages::new(&self.song_stages, self.sink.settings().around, &song, format);
        // Read again: its tap heard it the first time.
        self.head = Head::Reading(Read { reader: Reader::new(o.index, o.offset_us, o.r, stages, None), after: After::Unopened, remaking: None });
        self.configure(o.index, period.serial, format);
        self.engine.set_output_stream_offset_us(o.offset_us);
        self.engine.set_gain(period.gain);
        self.engine.set_song(song);
    }

    /// Starts reading ready song `i`; false when it reports an error.
    fn start_reading(&mut self, i: usize, from_ms: i64, offset_us: i64, r: S::Reading) -> bool {
        if let Some((kind, why)) = r.error() {
            self.fail(i, kind, why);
            return false;
        }
        let (format, duration_us) = (r.format(), r.duration_us());
        self.sink.track.source_bits(r.bits());
        let gain = self.song_gain(i);
        let serial = self.serial_for(i);
        self.facts.clear();
        let song = self.tell(i, serial, gain, from_ms);
        let reader = self.reader(i, offset_us, r, &song, from_ms);
        self.head = Head::Reading(Read { reader, after: After::Unopened, remaking: None });
        self.periods = vec![Period { index: i, offset_us, duration_us, gain, serial }];
        self.place(offset_us + from_ms * 1000);
        self.heard_from = self.position_us;
        self.configure(i, serial, format);
        self.engine.set_output_stream_offset_us(offset_us);
        self.engine.set_gain(gain);
        self.engine.set_song(song);
        true
    }

    /// A reader of song `i` (opened as `r` from `from_ms`) through the song slot's stages, heard by
    /// the plugins' tap.
    fn reader(&mut self, i: usize, offset_us: i64, r: S::Reading, song: &Arc<SongFacts>, from_ms: i64) -> Reader<S::Reading> {
        let format = r.format();
        let stages = SongStages::new(&self.song_stages, self.sink.settings().around, song, format);
        let id = self.id_at(i);
        let tap = self.preparing.as_ref().and_then(|p| p.tap(&id, format, (from_ms.max(0) * format.rate as i64 / 1000) as u64));
        Reader::new(i, offset_us, r, stages, tap)
    }

    /// Whether any stage reads songs' facts.
    fn told(&self) -> bool {
        !self.song_stages.is_empty() || !self.sink.settings().stages.is_empty() || self.sink.track.marks_songs()
    }

    /// What the stages read of song `i`, played at `gain` as stream `serial` from `from_ms` in it; kept
    /// for it.
    fn tell(&mut self, i: usize, serial: u64, gain: f32, from_ms: i64) -> Arc<SongFacts> {
        if !self.told() {
            return no_facts();
        }
        let id = self.id_at(i);
        let facts = Arc::new(SongFacts {
            id: Some(id.clone()),
            gain_db: Some(20.0 * gain.max(1e-6).log10()),
            loudness_lufs: self.app.loudness(&id),
            prepared: self.preparing.as_ref().map(|p| p.prepared(&id)).unwrap_or_default(),
            from_ms,
        });
        let periods = &self.periods;
        self.facts.retain(|(s, _)| *s != serial && periods.iter().any(|p| p.serial == *s));
        self.facts.push((serial, facts.clone()));
        facts
    }

    /// What was told of stream `serial`.
    fn facts_for(&self, serial: u64) -> Arc<SongFacts> {
        self.facts.iter().find(|(s, _)| *s == serial).map_or_else(no_facts, |(_, f)| f.clone())
    }

    /// Plugins' song-slot stages from the next buffer read on; a song read later is read through them.
    pub fn set_song_stages(&mut self, specs: Vec<StageSpec>) {
        if self.song_stages == specs {
            return;
        }
        self.song_stages = specs;
        let around = self.sink.settings().around;
        if let Some(s) = self.head.reader_mut().and_then(|r| r.stages.as_mut()) {
            s.apply(&self.song_stages, around);
        }
    }

    /// What plugins make of songs as they are read, from the next song read.
    pub fn set_preparing(&mut self, preparing: Option<Arc<dyn Preparing>>) {
        self.preparing = preparing;
    }

    /// Plugins made more of the songs played (their prepared data came): what the stages read of
    /// them changes from the first frame the output can still replace.
    pub fn facts_changed(&mut self) {
        let periods: Vec<(usize, u64, f32)> = self.periods.iter().map(|p| (p.index, p.serial, p.gain)).collect();
        for (i, serial, gain) in periods {
            let Some(was) = self.facts.iter().find(|(s, _)| *s == serial).map(|(_, f)| f.clone()) else { continue };
            let now = self.tell(i, serial, gain, was.from_ms);
            if *now == *was {
                self.facts.retain(|(s, _)| *s != serial);
                self.facts.push((serial, was));
                continue;
            }
            self.engine.retell(&was, &now);
            if let Some((input, output)) = self.sink.retell(&was, &now) {
                self.app.spliced("song's facts", Splice { input, output });
            }
            if let Some(s) = self.head.reader_mut().filter(|r| r.index == i).and_then(|r| r.stages.as_mut()) {
                s.song(&now);
            }
        }
        self.burst.restart();
        self.sink.fill();
    }

    /// Song `i`'s ReplayGain, or 1 when gain is off.
    fn song_gain(&mut self, i: usize) -> f32 {
        if self.gain_off {
            return 1.0;
        }
        let Player { queue, app, .. } = self;
        queue.read(|q| app.gain(q, i)).min(self.gain_max)
    }

    /// ReplayGain settings changed: the output is made again at the new levels from the first frame the
    /// track can still replace, and the song being read from its next buffer.
    pub fn gain_changed(&mut self) {
        let mut ranges = Vec::new();
        for k in 0..self.periods.len() {
            let p = self.periods[k];
            let gain = self.song_gain(p.index);
            if gain != p.gain && p.gain > 0.0 {
                let to = self.periods.get(k + 1).map_or(i64::MAX, |n| n.offset_us);
                ranges.push((p.offset_us..to, gain / p.gain));
                self.engine.rescale(p.offset_us, gain / p.gain);
            }
            self.periods[k].gain = gain;
        }
        if !ranges.is_empty() {
            if let Some((input, output)) = self.sink.rescale(&ranges) {
                self.app.spliced("gain", Splice { input, output });
            }
            self.burst.restart();
            self.sink.fill();
        }
        if let Some(i) = self.head.reader().map(|r| r.index) {
            let gain = match self.periods.iter().rev().find(|p| p.index == i) {
                Some(p) => p.gain,
                None => self.song_gain(i),
            };
            self.engine.set_gain(gain);
        }
    }

    /// A timeline offset past everything handed out so far.
    fn fresh_offset(&self) -> i64 {
        self.periods.iter().map(|p| p.offset_us + p.duration_us).max().unwrap_or(BASE_OFFSET_US - 1_000_000) + 1_000_000
    }

    /// Plays queue index `i` from its start.
    #[cfg(any(test, feature = "synth"))]
    pub fn play_from(&mut self, i: usize) {
        self.jump(i, 0);
        self.resume();
    }

    /// Moves to list index `i` (or the first playable after it) at `from_ms`; play state unchanged.
    pub fn jump(&mut self, i: usize, from_ms: i64) {
        self.jump_opened(i, from_ms, None);
    }

    /// Opens the outgoing loop at its source start and resumes its transition phase.
    pub fn resume_mix(&mut self, i: usize, from_ms: i64, phase_us: i64) {
        self.jump(i, from_ms);
        self.engine.land_in_mix(phase_us);
    }
    /// [`Player::jump`] reusing a reading opened ahead (id, reading, opened-at ms) if it is that song
    /// and can reach `from_ms` ([`taken_from`]); otherwise the song is reopened.
    pub fn jump_from(&mut self, i: usize, from_ms: i64, opened: (String, S::Reading, i64)) {
        self.jump_opened(i, from_ms, Some(opened));
    }

    fn jump_opened(&mut self, i: usize, from_ms: i64, opened: Option<(String, S::Reading, i64)>) {
        self.stop_after = None;
        let i = self.playable(i);
        let id = self.id_at(i);
        let r = match opened.filter(|o| o.0 == id).and_then(|(_, r, at)| taken_from(r, at, from_ms)) {
            Some((r, from)) => Ok((r, from)),
            None => self.tracks.open(&id, from_ms).map(|r| (r, from_ms)),
        };
        let (r, from_ms) = match r {
            Ok(r) => r,
            Err(why) => return self.fail(i, PlaybackError::Other, why),
        };
        let offset = self.fresh_offset();
        self.call(|e, _, a| e.flush(a));
        self.burst.restart();
        self.sink.flush();
        self.queue.moved_to(i);
        if self.begin(i, from_ms, offset, r) {
            self.set_current(i);
        }
    }

    pub fn resume(&mut self) {
        self.burst.restart();
        self.sink.play();
        self.mode = Mode::Playing;
        self.follow_clock();
    }

    /// Updates the clock for commands between render turns.
    /// Sets the time, and while playing reads the place from the output's clock: the last turn may have
    /// been a burst ago, and what is done now (a pause, a report) goes from where the music is.
    pub fn clock(&mut self, now_ms: i64) {
        self.now_ms = now_ms;
        if self.playing() {
            self.follow_clock();
        }
    }

    /// Pauses; music made ready while paused ([`Player::prime`]) goes on being made.
    pub fn pause(&mut self) {
        // The last turn may have been a burst ago.
        if self.playing() {
            self.follow_clock();
        }
        self.burst.restart();
        self.sink.pause();
        self.stop_playing();
    }

    /// Pauses, and makes the music ready while paused for a start at a set moment ([`Player::resume`]).
    pub fn prime(&mut self) {
        self.pause();
        self.mode = Mode::Priming;
    }

    /// No music is made ready any more; playing goes on.
    pub fn unprime(&mut self) {
        if self.mode == Mode::Priming {
            self.mode = Mode::Paused;
        }
    }

    fn stop_playing(&mut self) {
        if self.mode == Mode::Playing {
            self.mode = Mode::Paused;
        }
    }

    pub fn playing(&self) -> bool {
        self.mode == Mode::Playing
    }

    /// Music is made while paused, ready for a start at a set moment ([`Player::prime`]).
    pub fn priming(&self) -> bool {
        self.mode == Mode::Priming
    }

    /// Frees everything a long pause does not need and returns (queue index, ms) to [`Player::jump`]
    /// back to.
    pub fn release(&mut self) -> Option<(usize, i64)> {
        let ended = self.sink.source_ended();
        if let Some(now) = self.call(|e, d, a| e.position_us(d, a, ended)) {
            self.place(now);
        }
        let at = self.current.map(|i| (i, self.position_ms()));
        self.call(|e, _, a| e.reset(a));
        self.burst.restart();
        self.sink.reset();
        if !matches!(self.head, Head::Stopped { .. }) {
            self.head = Head::Idle;
        }
        at
    }

    /// Next, as the button does it.
    pub fn next_song(&mut self) -> bool {
        match self.queue.read(Playlist::next) {
            Some(n) => {
                self.jump(n, 0);
                true
            }
            None => false,
        }
    }

    /// Previous, as the button does it.
    pub fn previous(&mut self) -> bool {
        match self.queue.read(Playlist::previous) {
            Some(n) => {
                self.jump(n, 0);
                true
            }
            None => false,
        }
    }

    /// Seeks in the current song: flushes and rereads on the same timeline (announcing the song again).
    pub fn seek(&mut self, ms: i64) {
        // The song the seek bar shows: in a mix, the outgoing one until the takeover.
        let Some(i) = self.bar().index.or(self.current) else { return };
        let offset = self.periods.iter().find(|p| p.index == i).map_or_else(|| self.fresh_offset(), |p| p.offset_us);
        let r = match self.tracks.open(&self.id_at(i), ms) {
            Ok(r) => r,
            Err(why) => return self.fail(i, PlaybackError::Other, why),
        };
        self.call(|e, _, a| e.flush(a));
        self.burst.restart();
        self.sink.flush();
        if self.begin(i, ms, offset, r) {
            self.set_current(i);
        }
    }

    /// New chain settings, heard from the first frame the output can still replace.
    pub fn set_chain(&mut self, settings: ChainSettings) {
        if *self.sink.settings() == settings {
            return;
        }
        let ear = self.sink.ear();
        if let Some((input, output)) = self.sink.change(settings) {
            self.app.spliced("chain", Splice { input, output });
            // In place; a device that drops what it holds says when its change is heard itself.
            if let Some((heard, rate)) = ear.filter(|&(heard, _)| output > heard) {
                self.app.log(&format!("the change is heard after {} ms", (output - heard) * 1000 / rate.max(1) as u64));
            }
        }
        self.burst.restart();
        self.sink.fill();
    }

    /// The output made again from the first frame it can still replace, for a change in the track.
    pub fn remake(&mut self) {
        if let Some((input, output)) = self.sink.remake() {
            self.app.spliced("track", Splice { input, output });
        }
        self.burst.restart();
        self.sink.fill();
    }

    /// Speed and pitch as set.
    pub fn speed(&self) -> (f32, f32) {
        let s = self.sink.settings();
        (s.speed, s.pitch)
    }

    /// Has the app measure upcoming songs, then replans.
    pub fn measure_ahead(&mut self) {
        let n = measure_ahead(self.app.auto_mix());
        let ids: Vec<String> = self.queue.read(|q| q.upcoming().take(n).map(|i| q.ids()[i].clone()).collect());
        if ids.is_empty() {
            return;
        }
        self.app.measure_ahead(&mut self.tracks, &ids);
        self.engine.replan();
    }

    /// Treats song `i` as failed (skipped or stopped at, per the failure rules).
    pub fn give_up(&mut self, i: usize, why: String) {
        self.fail(i, PlaybackError::Other, why);
    }

    fn fail(&mut self, i: usize, kind: PlaybackError, why: String) {
        let id = self.id_at(i);
        let next = self.next_of(i);
        self.failures.push((id.clone(), why));
        let decided = match self.app.on_error(kind, next.is_some()) {
            Some(d) => d,
            None => self.errors.failed(kind, false, false, next.is_some()),
        };
        match (decided, next) {
            (OnError::Skip, Some(n)) => {
                self.app.log(&format!("{id} will not play: skipped"));
                self.jump(n, 0);
            }
            _ => {
                self.app.log(&format!("{id} will not play: stopped"));
                let halt = match decided {
                    OnError::Bridge => Halt::Bridge,
                    OnError::Wait => Halt::Wait,
                    _ => Halt::Stop,
                };
                let ms = if self.current == Some(i) { self.position_ms() } else { 0 };
                self.head = Head::Stopped { index: i, ms, halt };
                self.call(|e, _, a| e.reset(a));
                self.sink.flush();
                self.sink.pause();
                self.stop_playing();
            }
        }
    }

    /// The queue was edited. Indexes the player holds follow their entries; a song opened ahead that is
    /// no longer next is dropped.
    pub fn queue_changed(&mut self) {
        self.queue.take_edits();
        let seqs = self.queue.read(|q| q.seqs().to_vec());
        let old = std::mem::replace(&mut self.seqs, seqs);
        let edited = old != self.seqs;
        // Emptied (another server's profile taken up): nothing is left to play or to point at.
        if self.seqs.is_empty() && !old.is_empty() {
            self.pause();
            self.release();
            self.current = None;
            self.periods.clear();
            self.stop_after = None;
            self.upcoming = None;
            self.sync_queue();
            self.engine.replan();
            return;
        }
        // The song whose next is fetched ahead: once the current one has left the queue (a new queue, or
        // deleted), the queue's own current, where playback goes on.
        let mut ahead_of = self.current;
        if !old.is_empty() && edited {
            let queue_current = self.queue.read(|q| q.current());
            let new = &self.seqs;
            let moved = |i: usize| old.get(i).and_then(|s| new.iter().position(|n| n == s));
            let at = |i: usize| moved(i).unwrap_or_else(|| i.min(new.len().saturating_sub(1)));
            ahead_of = self.current.map(|c| moved(c).or(queue_current).unwrap_or_else(|| at(c)));
            self.current = self.current.map(at);
            match &mut self.head {
                Head::Reading(read) => {
                    read.reader.index = at(read.reader.index);
                    if let Some(o) = read.remaking.as_mut() {
                        o.index = at(o.index);
                    }
                }
                Head::Opening(o) => o.index = at(o.index),
                Head::Stopped { index, .. } => *index = at(*index),
                Head::Idle => {}
            }
            for p in self.periods.iter_mut() {
                p.index = at(p.index);
            }
            self.stop_after = self.stop_after.map(at);
            let reading = self.head.reader().map(|r| r.index);
            let after = reading.and_then(|r| self.next_of(r));
            if let Some(read) = self.head.read_mut() {
                // What was found or opened ahead only stands while its song is still the one read or the
                // next (by entry, not by position).
                read.after = match std::mem::replace(&mut read.after, After::Unopened) {
                    After::Failed((i, kind, why)) => match moved(i).filter(|k| Some(*k) == reading || Some(*k) == after) {
                        Some(k) => After::Failed((k, kind, why)),
                        None => After::Unopened,
                    },
                    After::Opened { index, r, waited } if moved(index).is_some_and(|i| Some(i) == after) => After::Opened { index: after.expect("checked"), r, waited },
                    _ => After::Unopened,
                };
            }
        }
        self.sync_queue();
        // The next song may have changed.
        self.engine.replan();
        let Some(cur) = ahead_of else { return };
        // Prefetch and measure the new next song now, so its mix can be planned in time.
        let next = self.next_of(cur).map(|n| self.id_at(n));
        let other_next = next != self.upcoming;
        if other_next || edited {
            self.upcoming = next;
            if let Some(id) = &self.upcoming {
                self.tracks.upcoming(id);
            }
        }
        if (edited || other_next) && self.app.auto_mix() && self.measure_on_move {
            self.measure_ahead();
        }
    }

    /// The reader moved past `cur` into a song that no longer follows it (queue edited after the
    /// ending was made).
    fn read_astray(&self, cur: usize) -> bool {
        let Some(r) = self.head.index() else { return false };
        r != cur && Some(r) != self.next_of(cur)
    }

    /// The repeat mode (`playlist::REPEAT_*`) the client set on the queue; replans.
    pub fn set_repeat(&mut self, mode: u8) {
        self.queue.set_repeat(mode);
        self.sync_queue();
        self.engine.replan();
    }

    /// Sends the planner its window (previous song, then eight in play order, as the core's
    /// `Session::window`).
    fn sync_queue(&mut self) {
        let current = self.current;
        let (ids, shuffling) = self.queue.read(|q| {
            let repeat = q.repeat();
            let mut window = Vec::new();
            if let Some(c) = current.or(q.current()) {
                window.extend(q.previous_of(c, repeat));
                let mut at = Some(c);
                for _ in 0..8 {
                    let Some(i) = at else { break };
                    window.push(i);
                    at = q.next_of(i, repeat);
                }
            }
            let ids: Vec<String> = window.into_iter().map(|i| q.ids()[i].clone()).collect();
            (ids, q.shuffling())
        });
        self.app.window(&self.tracks, &ids, shuffling);
    }

    fn set_current(&mut self, i: usize) {
        if self.current == Some(i) {
            return;
        }
        self.current = Some(i);
        self.queue.moved_to(i);
        self.changes.push((self.now_ms, i));
        self.sync_queue();
        self.upcoming = self.next_of(i).map(|n| self.id_at(n));
        self.fetch_after = true;
        self.fetch_when_free();
    }

    /// Prefetches the song after the current one once the song read has its bytes ([`Reading::fetched`]):
    /// on a slow link they would share it.
    fn fetch_when_free(&mut self) {
        if !self.fetch_after || matches!(self.head, Head::Opening(_)) || self.head.reader_mut().is_some_and(|r| !r.r.fetched()) {
            return;
        }
        self.fetch_after = false;
        self.fetch_ahead();
    }

    /// Prefetches (and measures) the song after the current one.
    fn fetch_ahead(&mut self) {
        if let Some(id) = &self.upcoming {
            self.tracks.upcoming(id);
        }
        if self.app.auto_mix() && self.measure_on_move {
            self.measure_ahead();
        }
    }

    /// The song whose stream the output's clock has reached.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// The failed song playback stopped at, until the next jump.
    pub fn stopped_at(&self) -> Option<usize> {
        self.stopped_place().map(|(i, _)| i)
    }

    /// [`Player::stopped_at`], and the place in it heard last, ms.
    pub fn stopped_place(&self) -> Option<(usize, i64)> {
        match self.head {
            Head::Stopped { index, ms, .. } => Some((index, ms)),
            _ => None,
        }
    }

    /// Why playback stopped at [`Player::stopped_at`].
    pub fn stopped_for(&self) -> Option<Halt> {
        match self.head {
            Head::Stopped { halt, .. } => Some(halt),
            _ => None,
        }
    }

    pub fn current_id(&self) -> Option<String> {
        self.current.map(|i| self.id_at(i))
    }

    /// Position in the current song, ms.
    pub fn position_ms(&self) -> i64 {
        let Some(pos) = self.position_us else { return 0 };
        let offset = self.current.and_then(|i| self.periods.iter().rev().find(|p| p.index == i)).map_or(0, |p| p.offset_us);
        (pos - offset) / 1000
    }

    /// When the position was read from the output's clock (or set by a start or seek).
    pub fn placed_at(&self) -> Instant {
        self.placed_at
    }

    fn place(&mut self, us: i64) {
        self.position_us = Some(us);
        self.placed_at = Instant::now();
    }

    /// Position in the current song, µs.
    pub fn position_us(&self) -> i64 {
        let Some(pos) = self.position_us else { return 0 };
        let offset = self.current.and_then(|i| self.periods.iter().rev().find(|p| p.index == i)).map_or(0, |p| p.offset_us);
        pos - offset
    }

    /// Song time until the clock reaches the next stream already handed out.
    pub fn until_next_song_us(&self) -> Option<i64> {
        let pos = self.position_us?;
        self.periods.iter().map(|p| p.offset_us).filter(|&o| o > pos).min().map(|o| o - pos)
    }

    /// Time until what is heard changes within a mix or held ending, µs of song time: the takeover as
    /// the seek bar reckons it ([`crate::heard`]), or the transition engine's next change (with
    /// `song_only`, only a change of song).
    pub fn until_heard_changes_us(&self, song_only: bool) -> Option<i64> {
        let h = self.engine.heard();
        let takeover = match h.id {
            Some(_) => Some(h.until_us - h.us - (self.now_ms - h.at_ms) * 1000),
            None => h.from.map(|_| h.audible_us - self.position_ms() * 1000),
        };
        [self.engine.until_heard_changes_us(song_only), takeover.filter(|&u| u > 0)].into_iter().flatten().min()
    }

    /// The seek bar's song and position.
    pub fn bar(&mut self) -> Seen {
        let pos = self.position_ms();
        let now = PlayerNow { now_ms: self.now_ms, playing: self.playing(), on: self.on_serial, position_ms: pos };
        let periods = &self.periods;
        let after = self.head.reader().and_then(|r| self.next_of(r.index));
        let placed = |p: &Period| (p.index, if p.duration_us > 0 { p.duration_us / 1000 } else { i64::MAX });
        let find = |s: StreamAt| match s {
            StreamAt::Serial(s) => periods.iter().find(|p| p.serial == s).map(placed),
            // A stream not handed out yet is the song after the one being read.
            StreamAt::After(s) => periods.iter().filter(|p| p.serial > s).min_by_key(|p| p.serial).map(placed).or(after.map(|n| (n, i64::MAX))),
        };
        self.tracker.at(self.engine.heard(), now, &find)
    }

    /// What is audible now, per the engine.
    pub fn heard(&self) -> &Heard {
        self.engine.heard()
    }

    /// The queue entry whose ending is audible, including after the incoming song takes over.
    pub fn mix_from(&self) -> Option<usize> {
        if !self.mixing() {
            return None;
        }
        let serial = self.heard().from?;
        self.periods
            .iter()
            .find(|p| p.serial == serial)
            .map(|p| p.index)
    }

    /// Position of the outgoing side of the audible transition.
    pub fn mix_position_ms(&self) -> Option<i64> {
        self.engine.mix_position_us().map(|us| us / 1000)
    }

    /// A mix is audible now.
    pub fn mixing(&self) -> bool {
        self.engine.heard().mixing
    }

    /// The audible song and position, ms, as of the last turn. During a held ending that is the
    /// previous song, although `current` has moved on.
    fn ear(&mut self) -> Option<(usize, i64)> {
        let current = self.current?;
        if self.engine.heard().id.is_some() {
            if let Some(i) = self.bar().index {
                let heard = self.engine.heard();
                let since = if self.playing() { (self.now_ms - heard.at_ms).max(0) } else { 0 };
                let ms = (heard.us + since * 1000) / 1000;
                return Some((i, ms));
            }
        }
        Some((current, self.position_ms()))
    }

    /// How song `cur`'s ending was already made: `Some(plan)` once a hold began or its last buffer went
    /// out (`Some(None)`: gapless), also `Some(None)` while still reading it past `start_us`. `None`
    /// while a plan starting at `start_us` can still be taken up.
    fn ending_made(&self, cur: usize, start_us: Option<i64>) -> Option<Option<crate::engine::Plan>> {
        let reading = self.head.reader().filter(|r| r.index == cur);
        if let Some(made) = self.serial_at(cur).and_then(|s| self.engine.made(s)) {
            if self.engine.holding() || reading.is_none() {
                return Some(made.cloned());
            }
        }
        let r = reading?;
        (start_us? < r.r.at_us()).then_some(None)
    }

    /// The last song has been read to its end and handed out.
    pub fn source_ended(&self) -> bool {
        self.sink.source_ended()
    }

    /// Everything has played.
    pub fn ended(&self) -> bool {
        self.sink.source_ended() && self.sink.drained()
    }

    /// The last turn hit its buffer budget with the output still taking: turn again at once.
    pub fn hungry(&self) -> bool {
        self.hungry
    }

    /// State summary for diagnostics.
    pub fn words(&self) -> String {
        let id = |i: usize| self.queue.read(|q| q.ids().get(i).cloned()).unwrap_or_else(|| "?".into());
        let mode = match self.mode {
            Mode::Paused => "paused",
            Mode::Priming => "priming",
            Mode::Playing => "playing",
        };
        let mut w = format!("{mode} on {}", self.current.map_or("nothing".into(), |i| format!("{i} ({})", id(i))));
        if self.position_us.is_some() {
            w.push_str(&format!(" at {} ms", self.position_ms()));
        }
        match &self.head {
            Head::Reading(read) => {
                let r = &read.reader;
                w.push_str(&format!("; reading {} ({}) at {} ms", r.index, id(r.index), r.r.at_us() / 1000));
                match r.bytes {
                    Bytes::Ended => w.push_str(", read to its end"),
                    Bytes::Missing => w.push_str(", waiting for its bytes"),
                    Bytes::Flowing => {}
                }
                if r.left() {
                    w.push_str(", a buffer in hand");
                }
                match &read.after {
                    After::Opened { index, waited, .. } => {
                        w.push_str(&format!("; next {index} ({}) opened", id(*index)));
                        if *waited {
                            w.push_str(", waiting for its bytes");
                        }
                    }
                    After::Failed((n, _, why)) => w.push_str(&format!("; {n} ({}) failed: {why}", id(*n))),
                    After::Unopened => {}
                }
            }
            Head::Opening(o) => w.push_str(&format!("; opening {} ({}) from {} ms", o.index, id(o.index), o.from_ms)),
            Head::Stopped { index, .. } => w.push_str(&format!("; stopped at {index}")),
            Head::Idle => w.push_str("; reading nothing"),
        }
        if let Some(n) = self.stop_after {
            w.push_str(&format!("; stopping after {n}"));
        }
        if self.sink.source_ended() {
            w.push_str("; the queue read to its end");
        }
        if self.hungry {
            w.push_str("; hungry");
        }
        w.push_str(&format!("; transition engine {}", self.engine.words()));
        w
    }

    /// The song being read or opening.
    pub fn reading_index(&self) -> Option<usize> {
        self.head.index()
    }

    /// Playing from a start, jump or seek whose first sample the output's clock has not passed yet.
    pub fn unheard(&self) -> bool {
        self.heard_from.is_some()
    }

    /// The song being read is waiting for bytes or still opening.
    pub fn starved(&self) -> bool {
        match &self.head {
            Head::Opening(_) => true,
            Head::Reading(read) => !read.reader.left() && read.reader.bytes == Bytes::Missing,
            _ => false,
        }
    }

    /// Reading is blocked on bytes: [`Player::starved`], a song opened again for its ending, or the next song's first bytes.
    pub fn waiting_for_bytes(&self) -> bool {
        self.starved()
            || self.head.read().is_some_and(|read| {
                read.remaking.is_some() || (matches!(read.after, After::Opened { waited: true, .. }) && read.reader.ended() && !read.reader.left())
            })
    }

    /// One render turn at `now_ms`: reads the clock and offers audio until the output refuses.
    pub fn turn(&mut self, now_ms: i64) {
        self.now_ms = now_ms;
        self.opened();
        self.remade();
        if self.mode == Mode::Paused {
            return;
        }
        if self.playing() {
            self.follow_clock();
        }
        self.fetch_when_free();
        self.render();
        // A pending failure is raised once everything before it has played.
        if self.head.failed() && self.ended() {
            let read = self.head.read_mut().expect("checked");
            let After::Failed((n, kind, why)) = std::mem::replace(&mut read.after, After::Unopened) else { unreachable!("checked") };
            self.fail(n, kind, why);
        }
    }

    /// Updates the position and current song from the output's clock.
    fn follow_clock(&mut self) {
        let ended = self.sink.source_ended();
        if let Some(at) = self.call(|e, d, a| e.position_us(d, a, ended)) {
            self.place(at);
            if let Some(p) = self.periods.iter().rev().find(|p| at >= p.offset_us).copied() {
                // A new stream of the same song: repeat one looped.
                if self.heard_period.is_some_and(|o| o != p.offset_us) && self.current == Some(p.index) {
                    self.loops += 1;
                }
                if self.heard_period != Some(p.offset_us) {
                    self.on_serial = Some(p.serial);
                }
                self.heard_period = Some(p.offset_us);
                self.set_current(p.index);
            }
            if self.heard_from.is_some_and(|from| at > from) {
                self.heard_from = None;
                self.errors.played();
                self.app.playing();
            }
        }
    }

    fn render(&mut self) {
        self.hungry = false;
        // What a splice left to run again goes first.
        if !self.sink.fill() {
            return;
        }
        for k in 0..BUFFERS_PER_TURN {
            if !self.ensure_buffer() {
                break;
            }
            self.hungry = k + 1 == BUFFERS_PER_TURN;
            let Player { engine, sink, burst, app, head, now_ms, .. } = self;
            let r = head.reader_mut().expect("a buffer is ready");
            app.clock(*now_ms);
            let mut fed = Fed::new(sink, burst, *now_ms);
            let pts = r.offset_us + r.r.at_us();
            let (taken, used) = engine.handle_buffer(&mut fed, app, &r.buffer()[r.pos..], pts);
            r.pos += used;
            if !taken {
                self.hungry = false;
                return;
            }
        }
        if self.head.reader().is_some_and(|r| r.ended()) && (self.at_queue_end() || self.head.failed()) && !self.sink.source_ended() {
            if self.call(|e, d, a| e.play_to_end_of_stream(d, a)) {
                self.sink.end_of_stream();
            }
        } else if self.sink.source_ended() {
            self.call(|e, d, _| e.queue_empty(d));
            self.sink.fill();
        }
    }

    fn at_queue_end(&self) -> bool {
        self.head.reader().is_some_and(|r| self.next_of(r.index).is_none())
    }

    /// Ensures a buffer is in hand, moving on to the next song once this one is read and its end is
    /// within [`READ_AHEAD_US`].
    fn ensure_buffer(&mut self) -> bool {
        loop {
            let Some(read) = self.head.read_mut() else { return false };
            let r = &mut read.reader;
            if r.left() {
                return true;
            }
            if !r.ended() {
                if !r.r.ready() {
                    r.bytes = Bytes::Missing;
                    return false;
                }
                if r.fill() {
                    continue;
                }
                // Bytes stopped for good: fail once what was read has played.
                if let Some((kind, why)) = r.r.error() {
                    read.after = After::Failed((r.index, kind, why));
                }
            }
            let (i, end) = (read.reader.index, read.reader.offset_us + read.reader.r.duration_us());
            if matches!(read.after, After::Failed(_)) {
                return false;
            }
            let Some(n) = self.next_of(i) else { return false };
            let Player { head, tracks, queue, position_us, .. } = self;
            let read = head.read_mut().expect("reading");
            // Open the next song now, so failures show early and fetching gets the most time.
            if !matches!(read.after, After::Opened { index, .. } if index == n) {
                read.after = match tracks.open(&queue.read(|q| q.ids()[n].clone()), 0) {
                    Ok(r) => After::Opened { index: n, r, waited: false },
                    Err(why) => After::Failed((n, PlaybackError::Other, why)),
                };
            }
            let After::Opened { r: next, waited, .. } = &mut read.after else { return false };
            if let Some((kind, why)) = next.ready().then(|| next.error()).flatten() {
                read.after = After::Failed((n, kind, why));
                return false;
            }
            if position_us.is_none_or(|pos| pos < end - READ_AHEAD_US) {
                return false;
            }
            if !next.ready() {
                *waited = true;
                return false;
            }
            let After::Opened { r: next, .. } = std::mem::replace(&mut read.after, After::Unopened) else { unreachable!("matched above") };
            let (format, duration_us) = (next.format(), next.duration_us());
            self.sink.track.source_bits(next.bits());
            let gain = self.song_gain(n);
            let serial = self.new_serial();
            let song = self.tell(n, serial, gain, 0);
            let reader = self.reader(n, end, next, &song, 0);
            self.configure(n, serial, format);
            self.call(|e, d, a| e.handle_discontinuity(d, a));
            self.engine.set_output_stream_offset_us(end);
            self.engine.set_gain(gain);
            self.engine.set_song(song);
            self.head.read_mut().expect("reading").reader = reader;
            self.periods.push(Period { index: n, offset_us: end, duration_us, gain, serial });
        }
    }
}

/// How far before a pre-opened reading's start the audible position may be for it to be reused (that
/// much audio is skipped).
const OPENED_EARLY_MS: i64 = 40;

/// Reuses reading `r` opened at `at` ms for playback from `from_ms`: read from `at` if `from_ms` is at
/// most [`OPENED_EARLY_MS`] before it, or skipped forward to `from_ms`. `None` if unusable.
fn taken_from<R: Reading>(mut r: R, at: i64, from_ms: i64) -> Option<(R, i64)> {
    if from_ms < at - OPENED_EARLY_MS {
        return None;
    }
    if from_ms <= at {
        return Some((r, at));
    }
    r.skip_to_ms(from_ms).then_some((r, from_ms))
}
