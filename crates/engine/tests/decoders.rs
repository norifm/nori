//! FLAC and MP3 in the shapes music comes in, encoded by ffmpeg and decoded as the engine decodes them
//! (symphonia's reader, `nori_player::decode::Decoder`). FLAC is lossless, so each shape decodes to what
//! was encoded; `digest` prints a digest of every shape's output, equal before and after a change when the
//! decoders' output is the same bit for bit. Tests pass without ffmpeg.
//!
//!   cargo test --release -p nori-engine --test engine decoders::digest -- --ignored --nocapture
//!
//! Opus has shapes of its own, through every mode (CELT, SILK, hybrid) and frame length: each decodes
//! with no packet refused, and `opus_digest` prints their digest:
//!
//!   cargo test --release -p nori-engine --test engine decoders::opus_digest -- --ignored --nocapture

use nori_player::decode::{Codec, Decoder};
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
    let codec = match extension {
        "flac" => Codec::Flac,
        "opus" => Codec::Opus,
        _ => Codec::Mp3,
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

#[test]
fn flac_decodes_to_what_was_encoded() {
    if !ffmpeg() {
        return;
    }
    for &(name, rate, channels, bits, args) in FLAC {
        let pcm = signal(rate, channels, bits);
        let file = encoded(&pcm, rate, channels, bits, &[args, &["-c:a", "flac", "-f", "flac"]].concat()).unwrap();
        let (out, got) = decoded(file, "flac");
        assert_eq!(got, channels, "{name}");
        let scale = 1.0 / (1u32 << (bits - 1)) as f32;
        let wrong = out.iter().zip(&pcm).position(|(&o, &s)| o != s as f32 * scale);
        assert_eq!((out.len(), wrong), (pcm.len(), None), "{name}");
    }
}

#[test]
#[ignore]
fn digest() {
    if !ffmpeg() {
        return;
    }
    let mut all = 0xcbf2_9ce4_8422_2325u64;
    let flac = FLAC.iter().map(|&(name, rate, channels, bits, args)| (name, "flac", rate, channels, bits, [args, &["-c:a", "flac", "-f", "flac"]].concat()));
    let mp3 = MP3.iter().map(|&(name, rate, channels, args)| (name, "mp3", rate, channels, 16, [args, &["-c:a", "libmp3lame", "-f", "mp3"]].concat()));
    for (name, extension, rate, channels, bits, args) in flac.chain(mp3) {
        let file = encoded(&signal(rate, channels, bits), rate, channels, bits, &args).unwrap();
        let (out, _) = decoded(file, extension);
        let hash = out.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3));
        all = (all ^ hash).wrapping_mul(0x100_0000_01b3);
        println!("digest: {extension:4} {name:18} {hash:016x}");
    }
    println!("digest: all {all:016x}");
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
