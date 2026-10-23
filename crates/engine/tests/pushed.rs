//! Songs whose bytes the platform pushes as its requests answer (the web's fetch, where nothing may
//! block): opened on the caller's turns as the bytes come, decoded as from a file. Compressed songs come
//! from ffmpeg; tests needing it pass trivially without it.

use crate::common;

use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use nori_engine::demux::Demuxed;
use nori_engine::{Body, ByteSource, Fetching, Filling, Loader, Next, OpenError, Waits};
use nori_player::pcm::Encoding;
use nori_player::pipeline::Reading;
use parking_lot::Mutex;

/// Hands every loader's fetching to the test.
#[derive(Default)]
struct Pushes(Mutex<Vec<Filling>>);

impl ByteSource for Pushes {
    fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
        unreachable!("a pushed song is never read on a thread")
    }

    fn push(&self, fill: Filling) -> Option<Filling> {
        self.0.lock().push(fill);
        None
    }
}

/// The network as the test plays it: `song` answered from any byte asked for, `chunk` bytes per step.
struct Network<'a> {
    song: &'a [u8],
    chunk: usize,
    asked: Vec<u64>,
}

impl Network<'_> {
    /// One step of `fill`; false when it waits or stopped.
    fn step(&mut self, fill: &mut Filling) -> bool {
        let len = self.song.len() as u64;
        match fill.poll_next(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Next::Open(from)) => {
                self.asked.push(from);
                let answer = if from >= len { Err(OpenError::PastEnd { len: Some(len) }) } else { Ok((from, Some(len))) };
                fill.opened(from, answer);
                true
            }
            Poll::Ready(Next::Read(from)) => {
                let end = (from as usize + self.chunk).min(self.song.len());
                fill.took(from, Ok(&self.song[from as usize..end]));
                true
            }
            Poll::Ready(Next::Wait | Next::Stop) | Poll::Pending => false,
        }
    }
}

/// Everything `d` decodes, as interleaved bytes, `more` bringing bytes while it is not ready.
fn decoded(d: &mut Demuxed, mut more: impl FnMut() -> bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut waits = 0;
    loop {
        if !d.ready() {
            waits += 1;
            assert!(waits < 10_000, "it opens within a few requests, not trying over and over");
            assert!(more(), "bytes come until the song is read: {:?}", d.error());
            continue;
        }
        if !d.fill() {
            break;
        }
        out.extend_from_slice(d.buffer());
    }
    assert!(d.error().is_none(), "{:?}", d.error());
    out
}

fn encode(dir: &Path, name: &str, secs: u32, codec: &[&str]) -> Vec<u8> {
    let out = dir.join(name);
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("sine=frequency=440:sample_rate=48000:duration={secs}"), "-ac", "2"])
        .args(codec)
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "ffmpeg made {name}");
    std::fs::read(out).unwrap()
}

/// A loader of `song` (as `hint`) at `from_ms`, its bytes pushed `chunk` at a time; what it decodes
/// and the requests made, and what the same song decodes to from a file.
fn pushed_and_file(song: &[u8], hint: &str, secs: i64, from_ms: i64, load: [i64; 5], chunk: usize) -> (Vec<u8>, Vec<u8>, Vec<u64>) {
    let pushes = Arc::new(Pushes::default());
    let loader = Loader::start_within(&Arc::new(Fetching::default()), pushes.clone(), "song".into(), load, Some(secs * 1000), None, None, None, Waits::default());
    let mut fill = pushes.0.lock().pop().expect("the fetching was pushed");
    let mut net = Network { song, chunk, asked: Vec::new() };
    let mut d = Demuxed::load(loader.clone(), std::thread::current(), Some(hint), from_ms, Some(secs * 1000), false, Encoding::Pcm16, Default::default());
    let heard = decoded(&mut d, || net.step(&mut fill));
    let mut file = Demuxed::open(Box::new(Cursor::new(song.to_vec())), Some(hint), from_ms, Some(secs * 1000), Encoding::Pcm16, Default::default()).unwrap();
    let want = decoded(&mut file, || false);
    (heard, want, net.asked)
}

/// 5-20 s ahead, of a five minute song.
const SMALL_WINDOW: [i64; 5] = [5_000, 20_000, 0, 0, 1 << 30];
const WHOLE: [i64; 5] = [60_000, 600_000, 0, 0, 1 << 30];

#[test]
fn a_pushed_song_opens_as_its_bytes_come() {
    if !common::ffmpeg() {
        return;
    }
    let dir = nori_testdir::TempDir::new("pushed");
    let songs = [
        ("mp3", encode(dir.path(), "a.mp3", 20, &["-c:a", "libmp3lame", "-b:a", "128k"])),
        ("flac", encode(dir.path(), "a.flac", 20, &["-c:a", "flac"])),
        // Its reader looks for the last page at the end first.
        ("ogg", encode(dir.path(), "a.ogg", 20, &["-c:a", "libopus", "-b:a", "96k"])),
    ];
    for (hint, song) in &songs {
        let (heard, want, asked) = pushed_and_file(song, hint, 20, 0, WHOLE, 16 * 1024);
        assert!(!want.is_empty() && heard == want, "{hint}: the same samples as from a file ({} and {} bytes)", heard.len(), want.len());
        if *hint != "ogg" {
            assert_eq!(asked, [0], "{hint}: read in one request, in order");
        }
    }
}

/// Opened half way with a window much smaller than the song: the header from the start, the music from
/// half way, each try finding what the last one read.
#[test]
fn a_pushed_song_opens_half_way() {
    if !common::ffmpeg() {
        return;
    }
    let dir = nori_testdir::TempDir::new("pushed-seek");
    let songs = [
        ("flac", encode(dir.path(), "b.flac", 300, &["-c:a", "flac"])),
        ("mp3", encode(dir.path(), "b.mp3", 300, &["-c:a", "libmp3lame", "-b:a", "128k"])),
        ("ogg", encode(dir.path(), "b.ogg", 300, &["-c:a", "libopus", "-b:a", "96k"])),
    ];
    for (hint, song) in &songs {
        let (heard, want, asked) = pushed_and_file(song, hint, 300, 240_000, SMALL_WINDOW, 16 * 1024);
        assert!(!want.is_empty() && heard == want, "{hint}: the same samples as from a file ({} and {} bytes), asked {asked:?}", heard.len(), want.len());
        assert!(asked.len() < 16, "{hint}: a few requests, not one per try: {asked:?}");
    }
}

/// [`Pushes`] behind a cache that forgets a song whenever told (the phone's media3 cache), counting how
/// often.
#[derive(Default)]
struct Cached(Pushes, Mutex<u32>);

impl ByteSource for Cached {
    fn open(&self, _: &str, _: u64) -> Result<Body, OpenError> {
        unreachable!("a pushed song is never read on a thread")
    }

    fn push(&self, fill: Filling) -> Option<Filling> {
        self.0.push(fill)
    }

    fn forget(&self, _: &str) -> bool {
        *self.1.lock() += 1;
        true
    }
}

/// An Ogg cut short on the server itself (without its last page) looks like a cached copy cut short:
/// fetched anew once, then played as it is.
#[test]
fn a_pushed_song_cut_short_on_the_server_is_fetched_again_once() {
    if !common::ffmpeg() {
        return;
    }
    let dir = nori_testdir::TempDir::new("pushed-cut");
    let mut song = encode(dir.path(), "c.ogg", 20, &["-c:a", "libopus", "-b:a", "96k"]);
    let last_page = song.windows(4).rposition(|w| w == b"OggS").unwrap();
    song.truncate(last_page);
    let cached = Arc::new(Cached::default());
    let loader = Loader::start_within(&Arc::new(Fetching::default()), cached.clone(), "song".into(), WHOLE, Some(20_000), None, None, None, Waits::default());
    let mut fill = cached.0 .0.lock().pop().expect("the fetching was pushed");
    let mut net = Network { song: &song, chunk: 16 * 1024, asked: Vec::new() };
    let mut d = Demuxed::load(loader, std::thread::current(), Some("ogg"), 0, Some(20_000), false, Encoding::Pcm16, Default::default());
    assert!(!decoded(&mut d, || net.step(&mut fill)).is_empty());
    assert_eq!(*cached.1.lock(), 1, "forgotten once");
}
