//! Steady playback must not allocate, per buffer on the engine's thread or ever on the device's. This
//! test binary counts allocations per thread; each path must make none after warm-up.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use nori_player::dsp::Band;
use nori_player::types::EqKind;
use nori_player::engine::Downstream;
use nori_player::pcm::{Encoding, Format};
use nori_player::pipeline::{ChainSettings, Sink, Sound};

use crate::output::{AudioOutput, Feed, OutputFormat, RingTrack};

struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    /// The most bytes one allocation asked for.
    static MOST: Cell<usize> = const { Cell::new(0) };
}

fn count(bytes: usize) {
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
    let _ = MOST.try_with(|c| c.set(c.get().max(bytes)));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count(n);
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations(f: impl FnOnce()) -> u64 {
    let before = ALLOCS.with(Cell::get);
    f();
    ALLOCS.with(Cell::get) - before
}

/// The most bytes one allocation asked for in `f`.
fn most_at_once(f: impl FnOnce()) -> usize {
    MOST.with(|c| c.set(0));
    f();
    MOST.with(Cell::get)
}

/// A device at `rate` whose feed the test pulls, holding `holds_us` of what it took.
struct Hand {
    feed: Arc<parking_lot::Mutex<Option<Feed>>>,
    rate: u32,
    holds_us: u64,
}

impl AudioOutput for Hand {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        Ok(OutputFormat { rate: self.rate, channels: want.channels, bits: 0 })
    }
    fn start(&mut self, feed: Feed) -> Result<(), String> {
        *self.feed.lock() = Some(feed);
        Ok(())
    }
    fn pause(&mut self) {}
    fn resume(&mut self) {}
    fn latency_us(&self) -> u64 {
        self.holds_us
    }
    fn depth_us(&self) -> i64 {
        self.holds_us as i64
    }
    fn close(&mut self) {}
}

fn tone(frames: usize, enc: Encoding) -> Vec<u8> {
    (0..frames)
        .flat_map(|i| {
            let v = (i as f64 / 44_100.0 * 440.0 * std::f64::consts::TAU).sin() * 9000.0 / 32768.0;
            let one = match enc {
                Encoding::Pcm16 => ((v * 32768.0) as i16).to_le_bytes().to_vec(),
                Encoding::Float => (v as f32).to_le_bytes().to_vec(),
            };
            [one.clone(), one].concat()
        })
        .collect()
}

/// Allocations while buffers go through the sink and ring and out of the device, after warm-up.
fn steady(device_rate: u32, sound: Sound, speed: f32, skip_silence: bool) -> u64 {
    steady_in(Encoding::Pcm16, device_rate, sound, speed, skip_silence)
}

fn steady_in(encoding: Encoding, device_rate: u32, sound: Sound, speed: f32, skip_silence: bool) -> u64 {
    let settings = ChainSettings { sound, speed, skip_silence, ..ChainSettings::default() };
    played(encoding, Hand { feed: Arc::default(), rate: device_rate, holds_us: 20_000 }, settings, 400).1
}

/// Plays `turns` buffers through the sink and ring and out of `device` to warm up, then as many again:
/// the most bytes one allocation asked for while warming up, and how many allocations came after.
fn played(encoding: Encoding, device: Hand, settings: ChainSettings, turns: usize) -> (usize, u64) {
    let fmt = Format { rate: 44_100, channels: 2, encoding };
    let feed = device.feed.clone();
    let mut sink = Sink::new(nori_player::burst::BUFFER_US, settings, RingTrack::new(Box::new(device)));
    sink.configure(fmt);
    sink.play();
    let data = tone(1152, encoding);
    let mut out = vec![0f32; 2048 * 2];
    let mut feed = feed.lock().take().expect("the device was started");
    let mut pts = 0i64;
    let mut turn = |sink: &mut Sink<RingTrack>, feed: &mut Feed| {
        sink.handle_buffer(&data, 0, pts);
        pts += fmt.us(data.len());
        feed.pull(&mut out);
        sink.position_us(false);
    };
    let most = most_at_once(|| {
        for _ in 0..turns {
            turn(&mut sink, &mut feed);
        }
    });
    let after = allocations(|| {
        for _ in 0..turns {
            turn(&mut sink, &mut feed);
        }
    });
    (most, after)
}

#[test]
fn buffer_paths_allocate_nothing() {
    assert_eq!(steady(44_100, Sound::default(), 1.0, false), 0, "straight through");
    let eq = Sound { bands: vec![Band { kind: EqKind::Peaking, freq: 1000.0, gain_db: 4.0, q: 1.0, channel: 0 }], limiter: true, ..Sound::default() };
    assert_eq!(steady(44_100, eq, 1.25, true), 0, "equalizer, limiter, silence skipping and speed");
    assert_eq!(steady(48_000, Sound::default(), 1.0, false), 0, "resampled for a device at another rate");

    // Float buffer path allocates nothing.
    assert_eq!(steady_in(Encoding::Float, 44_100, Sound::default(), 1.0, false), 0, "straight through");
    let eq = Sound { bands: vec![Band { kind: EqKind::Peaking, freq: 1000.0, gain_db: 4.0, q: 1.0, channel: 0 }], limiter: true, ..Sound::default() };
    assert_eq!(steady_in(Encoding::Float, 44_100, eq, 1.25, true), 0, "equalizer, limiter and speed in float");
    assert_eq!(steady_in(Encoding::Float, 48_000, Sound::default(), 1.0, false), 0, "resampled from float");
}

#[test]
fn input_kept_for_a_deep_device_has_its_room_from_the_start() {
    // Android's AudioTrack holds seconds beyond the ring; a sound change makes those again too.
    let deep = Hand { feed: Arc::default(), rate: 44_100, holds_us: 11_500_000 };
    let (most, _) = played(Encoding::Pcm16, deep, ChainSettings::default(), 1200);
    assert!(most < 1 << 20, "{most} bytes asked for at once while playing");
}

/// Allocations per buffer read from `source` after the first 16, in `encoding`.
fn per_read(source: Box<dyn symphonia::core::io::MediaSource>, hint: &str, encoding: Encoding) -> f64 {
    use nori_player::pipeline::Reading;
    let mut d = crate::demux::Demuxed::open(source, Some(hint), 0, None, encoding, Default::default()).expect("opens");
    for _ in 0..16 {
        assert!(d.fill(), "longer than the warm-up");
    }
    let mut reads = 0;
    let made = allocations(|| {
        while d.fill() {
            reads += 1;
        }
    });
    made as f64 / reads as f64
}

/// A stereo 16-bit WAV of `frames` frames of a tone.
fn wav(frames: usize) -> Vec<u8> {
    let data = tone(frames, Encoding::Pcm16);
    let mut w = b"RIFF".to_vec();
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | 2 << 16, 44_100, 44_100 * 4, 4 | 16 << 16] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

/// A stream of unknown length, as a station's: read live.
struct Live(std::io::Cursor<Vec<u8>>);

impl std::io::Read for Live {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for Live {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(to)
    }
}

impl symphonia::core::io::MediaSource for Live {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Reading a song allocates what symphonia's container readers do, one packet each, and nothing of its
/// own: `FormatReader::next_packet` hands out an owned packet and takes no buffer to read into. A WAV
/// file and a station are read into buffers of their own.
#[test]
fn reading_allocates_only_container_packets() {
    let mp3 = || include_bytes!("../../player/testdata/tone440.mp3").to_vec();
    let file = |bytes: Vec<u8>| -> Box<dyn symphonia::core::io::MediaSource> { Box::new(std::io::Cursor::new(bytes)) };
    for (source, hint, encoding, want) in [
        (file(wav(441_000)), "wav", Encoding::Pcm16, 0.0),
        (file(wav(441_000)), "wav", Encoding::Float, 0.0),
        (file(mp3()), "mp3", Encoding::Pcm16, 1.0),
        (file(mp3()), "mp3", Encoding::Float, 1.0),
        (Box::new(Live(std::io::Cursor::new(mp3()))), "mp3", Encoding::Pcm16, 0.0),
    ] {
        assert_eq!(per_read(source, hint, encoding), want, "{hint} into {encoding:?}");
    }
}
