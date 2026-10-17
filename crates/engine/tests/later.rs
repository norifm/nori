//! Songs decoded by a platform decoder that answers later (a browser's WebCodecs): the same samples as
//! decoded on the spot, their encoder delay and padding cut the same way, from the start and after a
//! seek. Compressed songs come from ffmpeg; tests needing it pass trivially without it.

use crate::common;

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use nori_engine::demux::Demuxed;
use nori_player::decode::{Codec, Decoder, Fault, LaterDecoder, Platform, Setup};
use nori_player::pcm::Encoding;
use nori_player::pipeline::Reading;

/// Packets a [`Lagging`] decoder holds before answering.
const LAG: usize = 3;

/// One packet's samples, channels and rate.
type Decoded = Result<(Vec<f32>, usize, u32), Fault>;

/// The decoder here, answering only once it holds more than [`LAG`] packets, or at the end.
struct Lagging {
    dec: Decoder,
    /// The stream it decodes, to start again after a seek as WebCodecs does (configured anew).
    setup: (Codec, u32, usize, Option<Vec<u8>>),
    done: VecDeque<Decoded>,
    finished: bool,
}

impl LaterDecoder for Lagging {
    fn feed(&mut self, unit: &[u8], _at_us: i64) {
        let got = self.dec.decode_lent(unit).map(|l| (l.samples.to_vec(), l.channels, l.rate));
        self.done.push_back(got);
    }

    fn ready(&self) -> bool {
        !self.done.is_empty() && (self.finished || self.done.len() > LAG)
    }

    fn take(&mut self, out: &mut Vec<f32>) -> Option<Result<(usize, u32), Fault>> {
        if !self.ready() {
            return None;
        }
        Some(self.done.pop_front()?.map(|(samples, channels, rate)| {
            out.extend_from_slice(&samples);
            (channels, rate)
        }))
    }

    fn finish(&mut self) {
        self.finished = true;
    }

    fn reset(&mut self) {
        self.done.clear();
        self.finished = false;
        let (codec, rate, channels, config) = &self.setup;
        self.dec = Decoder::new(*codec, *rate, *channels, config.as_deref(), true).expect("it opened before");
    }
}

fn lagging(s: &Setup) -> Option<Box<dyn LaterDecoder>> {
    // The MP3 decoder's delay is the reader's to drop, as WebCodecs leaves it.
    let dec = Decoder::new(s.codec, s.rate, s.channels, s.config, true).ok()?;
    Some(Box::new(Lagging { dec, setup: (s.codec, s.rate, s.channels, s.config.map(<[u8]>::to_vec)), done: VecDeque::new(), finished: false }))
}

/// Everything `d` decodes from `from_ms`; reading on whenever it is not ready, as a platform answers.
fn decoded(song: &[u8], hint: &str, from_ms: i64, platform: Platform) -> Vec<u8> {
    let mut d = Demuxed::open(Box::new(Cursor::new(song.to_vec())), Some(hint), from_ms, None, Encoding::Pcm16, platform).unwrap();
    let mut out = Vec::new();
    while d.fill() {
        out.extend_from_slice(d.buffer());
    }
    assert!(d.error().is_none(), "{:?}", d.error());
    out
}

fn encode(dir: &Path, name: &str, codec: &[&str]) -> Vec<u8> {
    let out = dir.join(name);
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=6", "-ac", "2"])
        .args(codec)
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "ffmpeg made {name}");
    std::fs::read(out).unwrap()
}

#[test]
fn a_song_decoded_later_is_the_song_decoded_here() {
    if !common::ffmpeg() {
        return;
    }
    let dir = nori_testdir::TempDir::new("later");
    let songs = [
        ("mp3", encode(dir.path(), "a.mp3", &["-c:a", "libmp3lame", "-b:a", "128k"])),
        ("flac", encode(dir.path(), "a.flac", &["-c:a", "flac"])),
        ("ogg", encode(dir.path(), "a.ogg", &["-c:a", "vorbis", "-strict", "-2"])),
        ("opus", encode(dir.path(), "a.opus", &["-c:a", "libopus", "-b:a", "96k"])),
        ("m4a", encode(dir.path(), "a.m4a", &["-c:a", "aac", "-b:a", "128k"])),
    ];
    let later = Platform { aac: None, later: Some(lagging) };
    for (hint, song) in &songs {
        for from_ms in [0, 2_500] {
            let here = decoded(song, hint, from_ms, Platform::default());
            let there = decoded(song, hint, from_ms, later);
            assert!(!here.is_empty() && there == here, "{hint} from {from_ms} ms: {} bytes later, {} here", there.len(), here.len());
        }
    }
}
