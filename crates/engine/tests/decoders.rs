//! The decoders in the shapes music comes in, encoded by ffmpeg and decoded as the engine decodes them
//! (symphonia's reader, `nori_player::decode::Decoder`). FLAC and ALAC are lossless, so each shape decodes
//! to what was encoded. The digests print a digest of every shape's output, equal before and after a
//! change when the decoders' output is the same bit for bit: `digest` FLAC's and MP3's, `digest_alac`,
//! `digest_vorbis` and `digest_aac` their own. Tests pass without ffmpeg.
//!
//!   cargo test --release -p nori-engine --test engine decoders::digest -- --ignored --nocapture
//!
//! Opus has shapes of its own, through every mode (CELT, SILK, hybrid) and frame length: each decodes
//! with no packet refused, and `opus_digest` prints their digest:
//!
//!   cargo test --release -p nori-engine --test engine decoders::opus_digest -- --ignored --nocapture

use nori_player::decode::{Codec, Decoder};
use symphonia::core::codecs::audio::well_known::{CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_OPUS, CODEC_ID_VORBIS};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::common::ffmpeg;

/// A FLAC shape: name, rate, channels, bits per sample, ffmpeg's encoder options.
const FLAC: &[(&str, u32, usize, u32, &[&str])] = &[
    ("level 0", 44_100, 2, 16, &["-compression_level", "0"]),
    ("level 1", 44_100, 2, 16, &["-compression_level", "1"]),
    ("level 2", 44_100, 2, 16, &["-compression_level", "2"]),
    ("level 3", 44_100, 2, 16, &["-compression_level", "3"]),
    ("level 4", 44_100, 2, 16, &["-compression_level", "4"]),
    ("level 5", 44_100, 2, 16, &["-compression_level", "5"]),
    ("level 6", 44_100, 2, 16, &["-compression_level", "6"]),
    ("level 7", 44_100, 2, 16, &["-compression_level", "7"]),
    ("level 8", 44_100, 2, 16, &["-compression_level", "8"]),
    ("level 12", 44_100, 2, 16, &["-compression_level", "12"]),
    ("orders 1-32", 44_100, 2, 16, &["-lpc_type", "levinson", "-min_prediction_order", "1", "-max_prediction_order", "32", "-prediction_order_method", "4"]),
    ("fixed", 44_100, 2, 16, &["-lpc_type", "fixed", "-ch_mode", "indep"]),
    ("mid side", 48_000, 2, 16, &["-compression_level", "8", "-ch_mode", "mid_side", "-exact_rice_parameters", "1"]),
    ("left side", 48_000, 2, 16, &["-compression_level", "5", "-ch_mode", "left_side"]),
    ("right side", 48_000, 2, 16, &["-compression_level", "5", "-ch_mode", "right_side"]),
    ("blocks 1152", 44_100, 2, 16, &["-compression_level", "5", "-frame_size", "1152"]),
    ("blocks 16384", 44_100, 2, 16, &["-compression_level", "8", "-frame_size", "16384", "-max_partition_order", "8"]),
    ("mono", 44_100, 1, 16, &["-compression_level", "5"]),
    ("6 channels", 48_000, 6, 16, &["-compression_level", "8"]),
    ("24-bit", 96_000, 2, 24, &["-compression_level", "5", "-sample_fmt", "s32"]),
    ("24-bit level 12", 96_000, 2, 24, &["-compression_level", "12", "-sample_fmt", "s32"]),
    ("24-bit 6 channels", 48_000, 6, 24, &["-compression_level", "8", "-sample_fmt", "s32"]),
];

/// An MP3 shape: name, rate, channels, ffmpeg's encoder options.
const MP3: &[(&str, u32, usize, &[&str])] = &[
    ("320k joint", 44_100, 2, &["-b:a", "320k"]),
    ("128k stereo", 44_100, 2, &["-b:a", "128k", "-joint_stereo", "0"]),
    ("vbr 0", 44_100, 2, &["-q:a", "0"]),
    ("vbr 6", 48_000, 2, &["-q:a", "6"]),
    ("32 kHz", 32_000, 2, &["-b:a", "96k"]),
    ("mono", 44_100, 1, &["-b:a", "96k"]),
    ("mpeg-2 22 kHz", 22_050, 2, &["-b:a", "64k"]),
    ("mpeg-2 16 kHz mono", 16_000, 1, &["-b:a", "32k"]),
    ("mpeg-2.5 11 kHz", 11_025, 2, &["-b:a", "32k"]),
    ("mpeg-2.5 8 kHz", 8_000, 1, &["-b:a", "8k"]),
];

/// An ALAC shape: name, rate, channels, bits per sample, frames, ffmpeg's encoder options. ffmpeg's
/// encoder makes packets of 4096 frames: the song's length sets the last one's.
type AlacShape = (&'static str, u32, usize, u32, usize, &'static [&'static str]);

const ALAC: &[AlacShape] = &[
    ("16-bit", 44_100, 2, 16, 8 * 44_100, &[]),
    ("16-bit mono", 44_100, 1, 16, 8 * 44_100, &[]),
    ("16-bit 96 kHz", 96_000, 2, 16, 8 * 96_000, &[]),
    ("24-bit", 96_000, 2, 24, 8 * 96_000, &["-sample_fmt", "s32p"]),
    ("24-bit mono", 96_000, 1, 24, 8 * 96_000, &["-sample_fmt", "s32p"]),
    ("24-bit 44.1 kHz", 44_100, 2, 24, 8 * 44_100, &["-sample_fmt", "s32p"]),
    ("uncompressed", 44_100, 2, 16, 8 * 44_100, &["-compression_level", "0"]),
    ("fixed order 6", 44_100, 2, 16, 8 * 44_100, &["-compression_level", "1"]),
    ("orders 1-30", 44_100, 2, 16, 8 * 44_100, &["-min_prediction_order", "1", "-max_prediction_order", "30"]),
    ("24-bit orders 1-30", 96_000, 2, 24, 8 * 96_000, &["-sample_fmt", "s32p", "-min_prediction_order", "1", "-max_prediction_order", "30"]),
    ("last packet 1 frame", 44_100, 2, 16, 86 * 4096 + 1, &[]),
    ("last packet 3 frames", 44_100, 1, 24, 86 * 4096 + 3, &["-sample_fmt", "s32p"]),
    ("last packet 4095", 48_000, 2, 16, 92 * 4096 + 4095, &["-max_prediction_order", "30"]),
    ("6 channels", 48_000, 6, 16, 8 * 48_000, &[]),
    ("24-bit 6 channels", 48_000, 6, 24, 8 * 48_000, &["-sample_fmt", "s32p"]),
];

/// A Vorbis shape: name, rate, ffmpeg's encoder options. ffmpeg's own encoder makes stereo only.
const VORBIS: &[(&str, u32, &[&str])] = &[
    ("default", 44_100, &[]),
    ("quality 0", 44_100, &["-q:a", "0"]),
    ("quality 2", 44_100, &["-q:a", "2"]),
    ("quality 5", 44_100, &["-q:a", "5"]),
    ("quality 10", 44_100, &["-q:a", "10"]),
    ("48 kHz", 48_000, &[]),
    ("32 kHz", 32_000, &["-q:a", "3"]),
    ("22 kHz", 22_050, &["-q:a", "3"]),
    ("96 kHz", 96_000, &[]),
];

/// An AAC shape: name, rate, channels, ffmpeg's encoder options.
const AAC: &[(&str, u32, usize, &[&str])] = &[
    ("256k", 44_100, 2, &["-b:a", "256k"]),
    ("128k", 44_100, 2, &["-b:a", "128k"]),
    ("64k", 44_100, 2, &["-b:a", "64k"]),
    ("320k 48 kHz", 48_000, 2, &["-b:a", "320k"]),
    ("96k 32 kHz", 32_000, 2, &["-b:a", "96k"]),
    ("mono 128k", 44_100, 1, &["-b:a", "128k"]),
    ("mono 48k", 44_100, 1, &["-b:a", "48k"]),
    ("fast coder", 44_100, 2, &["-b:a", "192k", "-aac_coder", "fast"]),
    ("mid side", 48_000, 2, &["-b:a", "160k", "-aac_ms", "1", "-aac_is", "0", "-aac_pns", "0"]),
];

/// ffmpeg's output options per codec.
const FLAC_OUT: &[&str] = &["-c:a", "flac", "-f", "flac"];
const MP3_OUT: &[&str] = &["-c:a", "libmp3lame", "-f", "mp3"];
const ALAC_OUT: &[&str] = &["-c:a", "alac", "-f", "ipod"];
const VORBIS_OUT: &[&str] = &["-c:a", "vorbis", "-strict", "-2", "-f", "ogg"];
const AAC_OUT: &[&str] = &["-c:a", "aac", "-f", "ipod"];

/// Eight seconds of `bits`-bit samples, interleaved: music, silence, a quiet fade in steps of 4 (wasted
/// bits), loud clipped music, full-scale noise, and a sine. Channels differ.
fn signal(rate: u32, channels: usize, bits: u32) -> Vec<i32> {
    let mut seed = 7u64;
    let mut noise = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / (1u64 << 30) as f64 - 1.0
    };
    let full = (1i64 << (bits - 1)) as f64;
    let n = 8 * rate as usize;
    let mut out = Vec::with_capacity(n * channels);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let beat = t % 0.5;
        let kick = 0.5 * (-beat * 30.0).exp() * (std::f64::consts::TAU * 60.0 * beat).sin();
        for ch in 0..channels {
            let tone = |hz: f64| (std::f64::consts::TAU * hz * (1.0 + ch as f64 * 0.01) * t + ch as f64).sin();
            let music = 0.3 * tone(110.0) + 0.1 * tone(554.0) + 0.05 * tone(1661.0) + kick + 0.02 * noise();
            let v = match t as u32 {
                0 | 1 => music * 0.5,
                2 => 0.0,
                3 => (music * 0.01 * (4.0 - t) * full / 4.0).round() * 4.0 / full,
                4 | 5 => (music * 1.6).clamp(-1.0, 1.0),
                6 => noise(),
                _ => 0.7 * tone(440.0),
            };
            out.push((v * full).round().clamp(-full, full - 1.0) as i32);
        }
    }
    out
}

/// `pcm` encoded by ffmpeg with `args`; None without ffmpeg.
fn encoded(pcm: &[i32], rate: u32, channels: usize, bits: u32, args: &[&str]) -> Option<Vec<u8>> {
    let dir = nori_testdir::TempDir::new("decoders");
    let (raw, out) = (dir.join("in.raw"), dir.join("out"));
    let (format, bytes) = match bits {
        16 => ("s16le", pcm.iter().flat_map(|&s| (s as i16).to_le_bytes()).collect::<Vec<u8>>()),
        _ => ("s32le", pcm.iter().flat_map(|&s| (s << (32 - bits)).to_le_bytes()).collect()),
    };
    std::fs::write(&raw, bytes).unwrap();
    let (rate, channels) = (rate.to_string(), channels.to_string());
    let input = ["-hide_banner", "-loglevel", "error", "-f", format, "-ar", &rate, "-ac", &channels, "-i"];
    let ran = std::process::Command::new("ffmpeg").args(input).arg(&raw).args(args).arg("-y").arg(&out).status();
    ran.is_ok_and(|s| s.success()).then(|| std::fs::read(&out).unwrap())
}

/// What the engine's decoder makes of `file`: interleaved float, and the channels.
fn decoded(file: Vec<u8>, extension: &str) -> (Vec<f32>, usize) {
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(file)), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let mut reader = symphonia::default::get_probe().probe(&hint, source, FormatOptions::default(), MetadataOptions::default()).unwrap();
    let track = reader.default_track(TrackType::Audio).unwrap();
    let (id, delay_known) = (track.id, track.delay.is_some());
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).unwrap().clone();
    let codec = match params.codec {
        CODEC_ID_FLAC => Codec::Flac,
        CODEC_ID_MP3 => Codec::Mp3,
        CODEC_ID_ALAC => Codec::Alac,
        CODEC_ID_AAC => Codec::Aac,
        CODEC_ID_VORBIS => Codec::Vorbis,
        CODEC_ID_OPUS => Codec::Opus,
        other => panic!("{other} is not decoded here"),
    };
    let channels = params.channels.as_ref().map_or(2, |c| c.count());
    let mut decoder = Decoder::new(codec, params.sample_rate.unwrap(), channels, params.extra_data.as_deref(), codec == Codec::Mp3 && delay_known).unwrap();
    let mut out = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if packet.track_id == id {
            out.extend_from_slice(decoder.decode_lent(&packet.data).unwrap().samples);
        }
    }
    (out, decoder.channels())
}

/// Asserts that `file` decodes to `pcm`, `bits`-bit samples in `channels`.
fn assert_lossless(name: &str, file: Vec<u8>, extension: &str, pcm: &[i32], channels: usize, bits: u32) {
    let (out, got) = decoded(file, extension);
    assert_eq!(got, channels, "{name}");
    let scale = 1.0 / (1u32 << (bits - 1)) as f32;
    let wrong = out.iter().zip(pcm).position(|(&o, &s)| o != s as f32 * scale);
    assert_eq!((out.len(), wrong), (pcm.len(), None), "{name}");
}

#[test]
fn flac_decodes_to_what_was_encoded() {
    if !ffmpeg() {
        return;
    }
    for &(name, rate, channels, bits, args) in FLAC {
        let pcm = signal(rate, channels, bits);
        let file = encoded(&pcm, rate, channels, bits, &[args, FLAC_OUT].concat()).unwrap();
        assert_lossless(name, file, "flac", &pcm, channels, bits);
    }
}

#[test]
fn alac_decodes_to_what_was_encoded() {
    if !ffmpeg() {
        return;
    }
    for &(name, rate, channels, bits, frames, args) in ALAC {
        let pcm = &signal(rate, channels, bits)[..frames * channels];
        let file = encoded(pcm, rate, channels, bits, &[args, ALAC_OUT].concat()).unwrap();
        assert_lossless(name, file, "m4a", pcm, channels, bits);
    }
}

/// Prints a digest of what each file decodes to, a line per (codec, shape, the file's extension, file),
/// and returns one of them all.
fn digests<'a>(files: impl Iterator<Item = (&'a str, &'a str, &'a str, Vec<u8>)>) -> u64 {
    let mut all = 0xcbf2_9ce4_8422_2325u64;
    for (codec, name, extension, file) in files {
        let (out, _) = decoded(file, extension);
        let hash = out.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3));
        all = (all ^ hash).wrapping_mul(0x100_0000_01b3);
        println!("digest: {codec:4} {name:18} {hash:016x}");
    }
    all
}

#[test]
#[ignore]
fn digest() {
    if !ffmpeg() {
        return;
    }
    let flac = FLAC.iter().map(|&(name, rate, channels, bits, args)| ("flac", name, "flac", encoded(&signal(rate, channels, bits), rate, channels, bits, &[args, FLAC_OUT].concat()).unwrap()));
    let mp3 = MP3.iter().map(|&(name, rate, channels, args)| ("mp3", name, "mp3", encoded(&signal(rate, channels, 16), rate, channels, 16, &[args, MP3_OUT].concat()).unwrap()));
    println!("digest: all {:016x}", digests(flac.chain(mp3)));
}

#[test]
#[ignore]
fn digest_alac() {
    if !ffmpeg() {
        return;
    }
    let alac = ALAC.iter().map(|&(name, rate, channels, bits, frames, args)| {
        let pcm = &signal(rate, channels, bits)[..frames * channels];
        ("alac", name, "m4a", encoded(pcm, rate, channels, bits, &[args, ALAC_OUT].concat()).unwrap())
    });
    println!("digest: alac all {:016x}", digests(alac));
}

#[test]
#[ignore]
fn digest_vorbis() {
    if !ffmpeg() {
        return;
    }
    let vorbis = VORBIS.iter().map(|&(name, rate, args)| ("vorbis", name, "ogg", encoded(&signal(rate, 2, 16), rate, 2, 16, &[args, VORBIS_OUT].concat()).unwrap()));
    println!("digest: vorbis all {:016x}", digests(vorbis));
}

#[test]
#[ignore]
fn digest_aac() {
    if !ffmpeg() {
        return;
    }
    let aac = AAC.iter().map(|&(name, rate, channels, args)| ("aac", name, "m4a", encoded(&signal(rate, channels, 16), rate, channels, 16, &[args, AAC_OUT].concat()).unwrap()));
    println!("digest: aac all {:016x}", digests(aac));
}

/// An Opus shape: name, channels, libopus's options. Music is CELT; speech rates are SILK (narrow, medium
/// or wide band) or hybrid, and at 32k libopus switches between CELT and hybrid as the music changes. A
/// packet holds 2.5 to 60 ms, in one to three frames; DTX sends silence as packets with no frame.
const OPUS: &[(&str, usize, &[&str])] = &[
    ("celt 128k", 2, &["-b:a", "128k"]),
    ("celt 64k", 2, &["-b:a", "64k"]),
    ("celt 256k", 2, &["-b:a", "256k"]),
    ("celt 510k", 2, &["-b:a", "510k"]),
    ("celt 128k mono", 1, &["-b:a", "128k"]),
    ("celt 64k mono", 1, &["-b:a", "64k"]),
    ("celt 32k", 2, &["-application", "lowdelay", "-b:a", "32k"]),
    ("celt narrowband", 2, &["-application", "lowdelay", "-b:a", "24k", "-cutoff", "4000"]),
    ("celt wideband", 1, &["-application", "lowdelay", "-b:a", "24k", "-cutoff", "8000"]),
    ("celt 2.5 ms", 2, &["-b:a", "128k", "-frame_duration", "2.5"]),
    ("celt 5 ms", 2, &["-b:a", "128k", "-frame_duration", "5"]),
    ("celt 10 ms", 2, &["-b:a", "96k", "-frame_duration", "10"]),
    ("celt 40 ms", 2, &["-b:a", "96k", "-frame_duration", "40"]),
    ("celt 60 ms", 2, &["-b:a", "96k", "-frame_duration", "60"]),
    ("celt cbr", 2, &["-b:a", "96k", "-vbr", "off"]),
    ("celt constrained", 2, &["-b:a", "96k", "-vbr", "constrained"]),
    ("silk nb", 1, &["-application", "voip", "-b:a", "8k"]),
    ("silk mb", 1, &["-application", "voip", "-b:a", "12k", "-cutoff", "6000"]),
    ("silk wb", 1, &["-application", "voip", "-b:a", "16k", "-cutoff", "8000"]),
    ("silk stereo", 2, &["-application", "voip", "-b:a", "20k", "-cutoff", "8000"]),
    ("silk 10 ms", 1, &["-application", "voip", "-b:a", "16k", "-cutoff", "8000", "-frame_duration", "10"]),
    ("silk 40 ms", 1, &["-application", "voip", "-b:a", "16k", "-cutoff", "8000", "-frame_duration", "40"]),
    ("silk 60 ms", 1, &["-application", "voip", "-b:a", "16k", "-cutoff", "8000", "-frame_duration", "60"]),
    ("silk cbr", 1, &["-application", "voip", "-b:a", "16k", "-cutoff", "8000", "-vbr", "off"]),
    ("silk fec", 1, &["-application", "voip", "-b:a", "24k", "-cutoff", "8000", "-fec", "1", "-packet_loss", "10"]),
    ("silk dtx", 1, &["-application", "voip", "-b:a", "16k", "-cutoff", "8000", "-dtx", "1"]),
    ("hybrid 16k", 1, &["-application", "voip", "-b:a", "16k"]),
    ("hybrid stereo", 2, &["-application", "voip", "-b:a", "20k"]),
    ("hybrid 10 ms", 1, &["-application", "voip", "-b:a", "16k", "-frame_duration", "10"]),
    ("hybrid 40 ms", 2, &["-application", "voip", "-b:a", "20k", "-frame_duration", "40"]),
    // Constant: opus-rs fails on most hybrid packets of three frames of different lengths.
    ("hybrid 60 ms cbr", 1, &["-application", "voip", "-b:a", "16k", "-frame_duration", "60", "-vbr", "off"]),
    ("voip 32k", 2, &["-application", "voip", "-b:a", "32k"]),
    ("voip 32k mono", 1, &["-application", "voip", "-b:a", "32k", "-cutoff", "12000"]),
];

/// The music of [`signal`] in `channels`, encoded by libopus with `args` and decoded as the engine does.
fn opus(channels: usize, args: &[&str]) -> (Vec<f32>, usize) {
    let file = encoded(&signal(48_000, channels, 16), 48_000, channels, 16, &[args, &["-c:a", "libopus", "-f", "ogg"]].concat()).unwrap();
    decoded(file, "opus")
}

#[test]
fn opus_decodes_every_packet() {
    if !ffmpeg() {
        return;
    }
    for &(name, channels, args) in OPUS {
        let (out, got) = opus(channels, args);
        assert_eq!((got, out.len() >= 8 * 48_000 * channels), (channels, true), "{name}");
    }
}

#[test]
#[ignore]
fn opus_digest() {
    if !ffmpeg() {
        return;
    }
    let mut all = 0xcbf2_9ce4_8422_2325u64;
    for &(name, channels, args) in OPUS {
        let (out, _) = opus(channels, args);
        let hash = out.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3));
        all = (all ^ hash).wrapping_mul(0x100_0000_01b3);
        println!("digest: opus {name:18} {hash:016x}");
    }
    println!("digest: opus all {all:016x}");
}
