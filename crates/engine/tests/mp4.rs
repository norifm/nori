//! AAC songs keep their exact length: in MP4 the encoder delay and padding from the edit list are cut
//! as media3 cuts them, and HE-AAC through a platform decoder that holds packets back ends whole. Needs
//! ffmpeg; passes trivially without it.

use std::path::{Path, PathBuf};
use std::io::{Cursor, Read};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nori_engine::{Body, ByteSource, Cancel, Loader, OpenError};
use parking_lot::{Condvar, Mutex};

use nori_engine::demux::Demuxed;
use nori_player::decode::{Codec, Decoder, Fault, Platform, PlatformAac, PlatformDecoder};
use nori_player::pcm::Encoding;
use nori_player::pipeline::Reading;

const RATE: usize = 44_100;
const FRAMES: usize = RATE * 3 + 317;

use crate::common::ffmpeg;

fn run(args: &[&str]) {
    let ok = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).status().is_ok_and(|s| s.success());
    assert!(ok, "ffmpeg {args:?}");
}

/// A tone of `frames` encoded to AAC in MP4, and ffmpeg's decode of it (which honours the edit list).
fn made(dir: &Path, frames: usize) -> (PathBuf, Vec<i16>) {
    let m4a = dir.join("tone.m4a");
    let raw = dir.join("tone.raw");
    let len = format!("{frames}");
    // Noise substitution generates random samples whose sequence changes after a seek.
    run(&["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100", "-af", &format!("atrim=end_sample={len}"), "-ac", "2", "-c:a", "aac", "-aac_pns", "0", "-b:a", "160k", m4a.to_str().unwrap()]);
    run(&["-i", m4a.to_str().unwrap(), "-f", "s16le", "-acodec", "pcm_s16le", raw.to_str().unwrap()]);
    let bytes = std::fs::read(&raw).unwrap();
    (m4a, bytes.as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)).collect())
}

fn decode(path: &Path, from_ms: i64, aac: Option<PlatformAac>) -> Vec<i16> {
    let file = std::fs::File::open(path).unwrap();
    let hint = path.extension().unwrap().to_str().unwrap();
    let mut d = Demuxed::open(Box::new(file), Some(hint), from_ms, None, Encoding::Pcm16, Platform { aac, later: None }).unwrap();
    assert!(d.ready());
    let mut out = Vec::new();
    while d.fill() {
        out.extend(d.buffer().as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)));
    }
    out
}

#[test]
fn mp4_aac_keeps_exact_length() {
    if !ffmpeg() {
        return;
    }
    let dir = nori_testdir::TempDir::new("mp4");
    let (m4a, reference) = made(&dir, FRAMES);
    // ffmpeg skips the priming; newer ones (8.x) keep the last frame's padding on a raw decode, so only
    // the song's own length of it is the reference.
    assert!(reference.len() >= FRAMES * 2, "ffmpeg decoded the whole song: {}", reference.len());
    let reference = &reference[..FRAMES * 2];
    let ours = decode(&m4a, 0, None);
    // ffmpeg 8 writes the edit list in the movie's millisecond timescale, so the file itself says the song
    // to within a millisecond. Priming or padding left would be a whole AAC frame (1024) or more.
    let frames = ours.len() / 2;
    assert!(frames.abs_diff(FRAMES) <= RATE / 1000 + 1, "not a frame of priming or padding left: {frames} of {FRAMES}");
    // Two decoders differ by rounding, not by a block of samples.
    let worst = ours.iter().zip(reference).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 4, "lined up with ffmpeg's own decode: {worst}");
    // A seek lands on the same samples as playing from the start.
    let from = decode(&m4a, 1_000, None);
    assert_eq!(from.len(), ours.len() - RATE * 2, "a second in, a second shorter");
    let worst = from.iter().zip(&ours[RATE * 2..]).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 4, "a seek into an MP4 lands where the song's time says: {worst}");
}

/// A platform HE-AAC decoder that hands each packet's samples out with the next packet, as some phones'
/// MediaCodec does (here the AAC core decodes).
struct Lagging {
    core: Decoder,
    held: Vec<f32>,
    shape: (usize, u32),
}

impl PlatformDecoder for Lagging {
    fn decode(&mut self, unit: &[u8], out: &mut Vec<f32>) -> Result<(usize, u32), Fault> {
        out.append(&mut self.held);
        let l = self.core.decode_lent(unit)?;
        self.shape = (l.channels, l.rate);
        self.held.extend_from_slice(l.samples);
        Ok(self.shape)
    }

    fn drain(&mut self, out: &mut Vec<f32>) -> Result<(usize, u32), Fault> {
        out.append(&mut self.held);
        Ok(self.shape)
    }

    fn reset(&mut self) {
        self.held.clear();
        self.core.reset(false);
    }
}

#[test]
fn lagging_he_aac_ends_whole() {
    if !ffmpeg() {
        return;
    }
    // AAC at 22.05 kHz is taken for HE-AAC with implicit signalling: the platform decodes it.
    let lagging: PlatformAac = |s| Some(Box::new(Lagging { core: Decoder::new(Codec::Aac, s.rate, s.channels, s.config, false).ok()?, held: Vec::new(), shape: (s.channels, s.rate) }));
    let dir = nori_testdir::TempDir::new("heaac");
    let (aac, raw) = (dir.join("tone.aac"), dir.join("tone.raw"));
    run(&["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050:duration=3", "-ac", "2", "-c:a", "aac", "-b:a", "64k", "-f", "adts", aac.to_str().unwrap()]);
    run(&["-i", aac.to_str().unwrap(), "-f", "s16le", "-acodec", "pcm_s16le", raw.to_str().unwrap()]);
    let reference: Vec<i16> = std::fs::read(&raw).unwrap().as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)).collect();
    let ours = decode(&aac, 0, Some(lagging));
    assert_eq!(ours.len(), reference.len(), "every packet heard, the last ones too");
    let worst = ours.iter().zip(&reference).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 4, "the same samples: {worst}");
}

/// Serves `song` with ranges. The first `holds` answers from before its last 128 KB send `each` bytes and
/// then hold until called off, as a slow link would while the song opens.
struct Holding {
    song: Vec<u8>,
    each: u64,
    holds: Mutex<u32>,
    asked: Mutex<Vec<u64>>,
}

/// A body that stops at `stop` until `off` is set.
struct Held {
    song: Cursor<Vec<u8>>,
    stop: u64,
    off: Arc<(Mutex<bool>, Condvar)>,
}

impl Read for Held {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.song.position() >= self.stop {
            let (off, cv) = &*self.off;
            let mut off = off.lock();
            while !*off {
                cv.wait(&mut off);
            }
            return Err(std::io::Error::other("called off"));
        }
        let n = buf.len().min((self.stop - self.song.position()) as usize);
        self.song.read(&mut buf[..n])
    }
}

impl ByteSource for Holding {
    fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
        unreachable!("asked as a request that may be called off")
    }

    fn open_cancellable(&self, _: &str, _: Option<&str>, from: u64, cancel: &Cancel) -> Result<Body, OpenError> {
        self.asked.lock().push(from);
        let off = Arc::new((Mutex::new(false), Condvar::new()));
        let o = off.clone();
        cancel.on_cancel(move || {
            *o.0.lock() = true;
            o.1.notify_all();
        });
        let mut song = Cursor::new(self.song.clone());
        song.set_position(from);
        let len = self.song.len() as u64;
        let mut holds = self.holds.lock();
        let stop = if *holds > 0 && from + (128 << 10) < len { from + self.each } else { len };
        *holds = holds.saturating_sub((stop < len) as u32);
        Ok(Body { start: from, len: Some(self.song.len() as u64), reader: Box::new(Held { song, stop, off }) })
    }
}

/// Waits until `d` is ready (it wakes this thread), at most until `until`.
fn wait_ready(d: &mut Demuxed, until: Instant, what: impl Fn() -> String) {
    while !d.ready() {
        assert!(Instant::now() < until, "{}", what());
        std::thread::park_timeout(until.saturating_duration_since(Instant::now()));
    }
}

#[test]
fn mp4_plays_before_it_is_whole() {
    if !ffmpeg() {
        return;
    }
    let dir = nori_testdir::TempDir::new("mp4-ranges");
    // 20 s: its index is further on than the first answer brings soon.
    let frames = RATE * 20 + 317;
    let (m4a, reference) = made(&dir, frames);
    let reference = &reference[..frames * 2];
    // Its index (moov) is after the music, as ffmpeg writes it; the answers while it opens never get there.
    let source = Arc::new(Holding { song: std::fs::read(&m4a).unwrap(), each: 16 << 10, holds: Mutex::new(1), asked: Mutex::default() });
    let loader = Loader::start(source.clone(), "song".into(), [1_000, 4_000, 0, 0, 1 << 30], Some(20_000), None);
    let mut d = Demuxed::load(loader, std::thread::current(), Some("m4a"), 0, Some(20_000), false, Encoding::Pcm16, Platform::default());
    let until = Instant::now() + Duration::from_secs(20);
    wait_ready(&mut d, until, || format!("opened with its index asked for where it is: {:?}", source.asked.lock()));
    let mut ours = Vec::new();
    while d.fill() {
        ours.extend(d.buffer().as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)));
        wait_ready(&mut d, until, || format!("reads on: {:?}", source.asked.lock()));
    }
    assert!((ours.len() / 2).abs_diff(frames) <= RATE / 1000 + 1, "its edit list read: {} of {frames} frames", ours.len() / 2);
    let worst = ours.iter().zip(reference).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 4, "the song as ffmpeg decodes it: {worst}");
    assert!(source.asked.lock().len() <= 4, "the start, the index (and what symphonia reads before it), the music after the start: {:?}", source.asked.lock());
}
