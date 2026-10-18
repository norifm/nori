//! Packet-by-packet decoding to interleaved PCM (symphonia for MP3, FLAC, AAC-LC, Vorbis, ALAC;
//! opus-rs for Opus); no allocation per packet after the first.
//!
//! HE-AAC's SBR/PS is not decoded here (symphonia only decodes the core): a platform may lend its
//! decoder ([`PlatformAac`]), used by [`Decoder::whole_aac`] for streams [`he_aac`] identifies.

use symphonia::core::audio::{Channels, Position};
use symphonia::core::codecs::audio::{well_known, AudioCodecParameters, AudioDecoder as Inner, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::packet::PacketRef;
use symphonia::core::units::{Duration, Timestamp};

/// Supported codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Mp3,
    Flac,
    /// AAC-LC (HE-AAC goes to the platform's decoder).
    Aac,
    Vorbis,
    Alac,
    /// Mono and stereo Opus (mapping family 0).
    Opus,
}

impl Codec {
    /// Stable id for the platform boundary.
    pub fn id(self) -> i32 {
        self as i32 + 1
    }

    /// Most frames one packet decodes to.
    pub fn max_frames(self) -> usize {
        match self {
            Codec::Mp3 => 1152,
            Codec::Aac => 2048,
            Codec::Vorbis => 8192,
            Codec::Flac => 65535,
            Codec::Alac => 4096,
            Codec::Opus => OPUS_MAX_FRAMES,
        }
    }

    fn well_known(self) -> symphonia::core::codecs::audio::AudioCodecId {
        match self {
            Codec::Mp3 => well_known::CODEC_ID_MP3,
            Codec::Flac => well_known::CODEC_ID_FLAC,
            Codec::Aac => well_known::CODEC_ID_AAC,
            Codec::Vorbis => well_known::CODEC_ID_VORBIS,
            Codec::Alac => well_known::CODEC_ID_ALAC,
            Codec::Opus => well_known::CODEC_ID_OPUS,
        }
    }
}

/// Why a packet gave nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// This packet could not be decoded; the next one may be. Skip it.
    BadPacket,
    /// The stream cannot be decoded any further.
    Broken,
}

/// Whether the output is 16-bit integers or 32-bit floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    I16,
    F32,
}

impl Sample {
    pub fn bytes(self) -> usize {
        match self {
            Sample::I16 => 2,
            Sample::F32 => 4,
        }
    }
}

/// A float sample as 16-bit, rounded (symphonia's own conversion truncates).
pub fn rounded_i16(v: f32) -> i16 {
    (v * 32768.0).round_ties_even().clamp(-32768.0, 32767.0) as i16
}

/// The MP3 decoder's filterbank delay. media3 trims it with the encoder delay when a LAME header says
/// so; otherwise, and after every seek, it is dropped here (as Android's decoder does).
pub const MP3_DECODER_DELAY: usize = 529;

const OPUS_MAX_FRAMES: usize = 5760;
/// Opus always decodes at 48 kHz.
const OPUS_RATE: u32 = 48_000;
/// Default Opus seek pre-roll (80 ms, RFC 7845).
const OPUS_SEEK_PREROLL: usize = 3840;

/// The decoding backend.
enum Engine {
    Symphonia(Box<dyn Inner>),
    #[cfg(feature = "rust-codecs")]
    Opus { dec: opus_rs::OpusDecoder, channels: usize, pre_skip: usize, pre_roll: usize, gain: f32 },
    Platform(Box<dyn PlatformDecoder>),
}

/// A platform decoder for HE-AAC, driven packet by packet on the caller's thread.
pub trait PlatformDecoder: Send {
    /// Decodes one access unit, appending interleaved float to `out`; returns (channels, rate). Output
    /// may lag input by a packet or more.
    fn decode(&mut self, unit: &[u8], out: &mut Vec<f32>) -> Result<(usize, u32), Fault>;
    /// The end of the stream: appends what the decoder still holds.
    fn drain(&mut self, out: &mut Vec<f32>) -> Result<(usize, u32), Fault>;
    /// Discontinuity (seek): drop held state.
    fn reset(&mut self);
}

/// An AAC stream's setup for a platform decoder.
#[derive(Debug, Clone, Copy)]
pub struct AacSetup<'a> {
    /// Rate and channels as stated (the core's for implicitly signalled HE-AAC).
    pub rate: u32,
    pub channels: usize,
    /// AudioSpecificConfig (MP4); `None` for ADTS.
    pub config: Option<&'a [u8]>,
}

/// Makes a platform decoder for an HE-AAC stream, `None` if it cannot.
pub type PlatformAac = fn(&AacSetup) -> Option<Box<dyn PlatformDecoder>>;

/// A platform decoder that answers later (WebCodecs, in a browser, where nothing may block): fed packets
/// in order, it hands back each one's samples once they are decoded, in the same order, with the shape
/// it decoded them in. The platform drops a codec's own priming (Opus pre-skip) as it decodes; the MP3
/// decoder's delay is kept, as here.
pub trait LaterDecoder: Send {
    /// Takes the next packet, which starts `at_us` into the song (by its stamp).
    fn feed(&mut self, unit: &[u8], at_us: i64);
    /// The oldest packet fed is decoded (or failed): [`LaterDecoder::take`] has it.
    fn ready(&self) -> bool;
    /// The oldest packet's samples, appended to `out` as interleaved float, with (channels, rate); None
    /// while it is still decoding.
    fn take(&mut self, out: &mut Vec<f32>) -> Option<Result<(usize, u32), Fault>>;
    /// No more packets: those fed are decoded to their end.
    fn finish(&mut self);
    /// A seek: what was fed is dropped.
    fn reset(&mut self);
}

/// A stream a platform decoder is asked for: its codec, stated rate and channels, and the container's
/// codec setup (STREAMINFO, AudioSpecificConfig, Vorbis headers, OpusHead).
#[derive(Debug, Clone, Copy)]
pub struct Setup<'a> {
    pub codec: Codec,
    pub rate: u32,
    pub channels: usize,
    pub config: Option<&'a [u8]>,
}

/// Makes a platform decoder that answers later for a stream, `None` if it cannot decode it.
pub type PlatformLater = fn(&Setup) -> Option<Box<dyn LaterDecoder>>;

/// The platform's own decoders: for HE-AAC, decoding packet by packet (Android's MediaCodec); and for
/// whatever it can decode, answering later (a browser's WebCodecs), which comes first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Platform {
    pub aac: Option<PlatformAac>,
    pub later: Option<PlatformLater>,
}

/// Whether an AAC stream is HE-AAC: object type 5 (SBR) or 29 (PS), an explicit SBR extension in the
/// config, or AAC-LC at 24 kHz or less (implicit signalling, as radio AAC+ does).
pub fn he_aac(config: Option<&[u8]>, rate: u32) -> bool {
    let implicit = rate > 0 && rate <= 24_000;
    let Some(c) = config.filter(|c| c.len() >= 2) else { return implicit };
    let mut bits = Bits { b: c, at: 0 };
    let object = |bits: &mut Bits| -> Option<u32> {
        let o = bits.take(5)?;
        if o == 31 { Some(32 + bits.take(6)?) } else { Some(o) }
    };
    let Some(aot) = object(&mut bits) else { return implicit };
    match aot {
        5 | 29 => true,
        2 => {
            // Rate (index or 24 bits), channels, 3 GASpecificConfig bits, then maybe the SBR
            // extension: sync 0x2b7, object type 5, present flag.
            let explicit = (|| {
                if bits.take(4)? == 15 {
                    bits.take(24)?;
                }
                let channels = bits.take(4)?;
                let (_frame_length, core_coder, _extension) = (bits.take(1)?, bits.take(1)?, bits.take(1)?);
                if channels == 0 || core_coder == 1 {
                    return None;
                }
                (bits.take(11)? == 0x2b7 && object(&mut bits)? == 5).then(|| bits.take(1)).flatten()
            })();
            match explicit {
                Some(sbr) => sbr == 1,
                None => implicit,
            }
        }
        _ => false,
    }
}

/// MSB-first bit reader.
struct Bits<'a> {
    b: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn take(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.b.get(self.at / 8)?;
            v = v << 1 | (byte >> (7 - self.at % 8)) as u32 & 1;
            self.at += 1;
        }
        Some(v)
    }
}

/// Parses media3's Opus setup (OpusHead, then pre-skip and seek pre-roll in ns, 8 bytes each, native
/// order): (channels, pre-skip samples, pre-roll samples, linear gain); `None` for surround.
fn opus_setup(extra: Option<&[u8]>) -> Option<(usize, usize, usize, f32)> {
    let e = extra?;
    if e.len() < 19 || &e[..8] != b"OpusHead" || e[18] != 0 {
        return None;
    }
    let channels = e[9] as usize;
    if !(1..=2).contains(&channels) {
        return None;
    }
    let pre_skip = u16::from_le_bytes([e[10], e[11]]) as usize;
    let gain_q8 = i16::from_le_bytes([e[16], e[17]]);
    let gain = 10f32.powf(gain_q8 as f32 / (20.0 * 256.0));
    let pre_roll = e
        .get(19 + 8..19 + 16)
        .map(|b| i64::from_ne_bytes(b.try_into().unwrap()))
        .filter(|&ns| ns > 0)
        .map_or(OPUS_SEEK_PREROLL, |ns| (ns as u128 * OPUS_RATE as u128 / 1_000_000_000) as usize);
    Some((channels, pre_skip, pre_roll, gain))
}

/// Frames a reader drops after a seek in a stream a [`LaterDecoder`] decodes, which starts afresh then (an
/// Opus stream's pre-skip dropped, and the MP3 decoder's delay kept) where a decoder here drops a seek's
/// pre-roll and its delay.
pub fn later_seek_skip(codec: Codec, extra: Option<&[u8]>) -> usize {
    match codec {
        Codec::Mp3 => MP3_DECODER_DELAY,
        Codec::Opus => opus_setup(extra).map_or(0, |(_, pre_skip, pre_roll, _)| pre_roll.saturating_sub(pre_skip)),
        _ => 0,
    }
}

/// One packet's samples borrowed from [`Decoder::decode_lent`].
pub struct Lent<'a> {
    pub samples: &'a [f32],
    pub channels: usize,
    pub rate: u32,
}

pub struct Decoder {
    inner: Engine,
    codec: Codec,
    channels: usize,
    rate: u32,
    /// The last packet's interleaved samples.
    scratch: Vec<f32>,
    /// Frames of the last packet not yet handed out, starting at frame `from` of `scratch`.
    held: usize,
    from: usize,
    /// Frames still to drop from the output's start.
    skip: usize,
    /// The next packet is the first after a reset.
    fresh: bool,
    /// Rate and channels come from a decoded packet, not the opening parameters.
    decoded: bool,
}

impl Decoder {
    /// `extra` is the extractor's codec setup (STREAMINFO, AudioSpecificConfig, Vorbis headers, ALAC
    /// cookie, OpusHead). `delay_known`: an MP3 LAME header lets the platform trim the decoder delay.
    pub fn new(codec: Codec, rate: u32, channels: usize, extra: Option<&[u8]>, delay_known: bool) -> Result<Decoder, String> {
        #[cfg(not(feature = "rust-codecs"))]
        if codec == Codec::Opus {
            return Err("Opus is decoded by the platform here".into());
        }
        #[cfg(feature = "rust-codecs")]
        if codec == Codec::Opus {
            let (channels, pre_skip, pre_roll, gain) = opus_setup(extra).ok_or("not a mono or stereo Opus stream")?;
            let dec = opus_rs::OpusDecoder::new(OPUS_RATE as i32, channels).map_err(str::to_string)?;
            return Ok(Decoder {
                inner: Engine::Opus { dec, channels, pre_skip, pre_roll, gain },
                codec,
                channels,
                rate: OPUS_RATE,
                scratch: vec![0.0; OPUS_MAX_FRAMES * channels],
                held: 0,
                from: 0,
                skip: pre_skip,
                fresh: false,
                decoded: false,
            });
        }
        let inner = Decoder::symphonia(codec, rate, channels, extra)?;
        let channels = channels.max(1);
        Ok(Decoder {
            inner: Engine::Symphonia(inner),
            codec,
            channels,
            rate,
            scratch: vec![0.0; codec.max_frames() * channels],
            held: 0,
            from: 0,
            skip: if codec == Codec::Mp3 && !delay_known { MP3_DECODER_DELAY } else { 0 },
            fresh: false,
            decoded: false,
        })
    }

    /// symphonia's decoder for `codec` at `rate` and `channels`.
    fn symphonia(codec: Codec, rate: u32, channels: usize, extra: Option<&[u8]>) -> Result<Box<dyn Inner>, String> {
        let mut params = AudioCodecParameters::new();
        params.for_codec(codec.well_known()).with_sample_rate(rate);
        if let Some(p) = Position::from_count(channels as u32) {
            params.with_channels(Channels::Positioned(p));
        }
        if let Some(e) = extra.filter(|e| !e.is_empty()) {
            // media3 prefixes STREAMINFO with "fLaC" and the block header.
            let e = if codec == Codec::Flac && e.starts_with(b"fLaC") && e.len() >= 8 { &e[8..] } else { e };
            params.with_extra_data(e.into());
        }
        // media3 trims encoder delay and padding itself.
        let opts = AudioDecoderOptions::default().gapless(false);
        symphonia::default::get_codecs().make_audio_decoder(&params, &opts).map_err(|e| e.to_string())
    }

    /// An AAC decoder: `platform`'s for HE-AAC when there is one, else symphonia (core only for HE-AAC).
    pub fn whole_aac(platform: Option<PlatformAac>, rate: u32, channels: usize, config: Option<&[u8]>) -> Result<Decoder, String> {
        let platform = platform.filter(|_| he_aac(config, rate)).and_then(|make| make(&AacSetup { rate, channels, config }));
        match platform {
            Some(dec) => Ok(Decoder {
                inner: Engine::Platform(dec),
                codec: Codec::Aac,
                channels: channels.max(1),
                rate,
                // Two HE-AAC packets at twice the core rate, stereo.
                scratch: Vec::with_capacity(2 * 2048 * 2.max(channels)),
                held: 0,
                from: 0,
                skip: 0,
                fresh: false,
                decoded: false,
            }),
            None => Decoder::new(Codec::Aac, rate, channels, config, false),
        }
    }

    /// The platform's decoder is in use.
    #[cfg(any(test, feature = "synth"))]
    pub fn on_platform(&self) -> bool {
        matches!(self.inner, Engine::Platform(_))
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Output channels and rate (certain after the first packet).
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Decodes `packet` and borrows its samples without copying.
    pub fn decode_lent(&mut self, packet: &[u8]) -> Result<Lent<'_>, Fault> {
        self.decode(packet)?;
        let n = std::mem::take(&mut self.held) * self.channels;
        let from = self.from * self.channels;
        Ok(Lent { samples: &self.scratch[from..from + n], channels: self.channels, rate: self.rate })
    }

    /// The end of the stream: borrows what the decoder still held back (only a platform decoder does).
    pub fn drain_lent(&mut self) -> Result<Lent<'_>, Fault> {
        self.held = 0;
        if let Engine::Platform(dec) = &mut self.inner {
            self.scratch.clear();
            let (channels, rate) = dec.drain(&mut self.scratch)?;
            (self.channels, self.rate) = (channels.max(1), rate);
            self.held = self.scratch.len() / self.channels;
        }
        let n = std::mem::take(&mut self.held) * self.channels;
        Ok(Lent { samples: &self.scratch[..n], channels: self.channels, rate: self.rate })
    }

    fn decode(&mut self, packet: &[u8]) -> Result<(), Fault> {
        self.held = 0;
        let mut fresh = std::mem::take(&mut self.fresh);
        let frames = match &mut self.inner {
            #[cfg(feature = "rust-codecs")]
            Engine::Opus { dec, channels, gain, .. } => {
                let n = dec.decode(packet, OPUS_MAX_FRAMES, &mut self.scratch).map_err(|_| Fault::BadPacket)?;
                if *gain != 1.0 {
                    self.scratch[..n * *channels].iter_mut().for_each(|s| *s *= *gain);
                }
                n
            }
            Engine::Platform(dec) => {
                self.scratch.clear();
                let (channels, rate) = dec.decode(packet, &mut self.scratch)?;
                (self.channels, self.rate) = (channels.max(1), rate);
                self.decoded = true;
                self.scratch.len() / self.channels
            }
            Engine::Symphonia(inner) => {
                // symphonia rejects MP3 frames of another shape (a radio's next song): switch to a new
                // decoder, treating its first frame as after a seek.
                if self.codec == Codec::Mp3 && self.decoded {
                    if let Some((rate, channels)) = mp3_shape(packet).filter(|&s| s != (self.rate, self.channels)) {
                        *inner = Decoder::symphonia(Codec::Mp3, rate, channels, None).map_err(|_| Fault::Broken)?;
                        (self.rate, self.channels) = (rate, channels);
                        fresh = true;
                    }
                }
                let p = PacketRef::new(0, Timestamp::new(0), Duration::new(0), packet);
                match inner.decode_ref(&p) {
                    Ok(buf) => {
                        let spec = buf.spec();
                        self.channels = spec.channels().count().max(1);
                        self.rate = spec.rate();
                        let n = buf.frames();
                        if self.scratch.len() < n * self.channels {
                            self.scratch.resize(n * self.channels, 0.0);
                        }
                        buf.copy_to_slice_interleaved::<f32, _>(&mut self.scratch[..n * self.channels]);
                        self.decoded = true;
                        n
                    }
                    Err(Error::DecodeError(_)) | Err(Error::IoError(_)) => return Err(Fault::BadPacket),
                    Err(Error::ResetRequired) => {
                        inner.reset();
                        return Err(Fault::BadPacket);
                    }
                    Err(_) => return Err(Fault::Broken),
                }
            }
        };
        // The first MP3 frame after a seek that uses the bit reservoir decodes as garbage: silence it,
        // as Android's decoder does.
        if fresh && self.codec == Codec::Mp3 && mp3_main_data_begin(packet) != 0 {
            self.scratch[..frames * self.channels].fill(0.0);
        }
        let drop = self.skip.min(frames);
        self.skip -= drop;
        self.from = drop;
        self.held = frames - drop;
        Ok(())
    }

    /// Discontinuity (a seek; `at_start` at the very beginning). MP3 drops its decoder delay again;
    /// Opus skips its pre-skip at the start, its pre-roll elsewhere.
    #[cfg_attr(not(feature = "rust-codecs"), allow(unused_variables, reason = "only Opus tells the start apart"))]
    pub fn reset(&mut self, at_start: bool) {
        self.held = 0;
        self.fresh = true;
        match &mut self.inner {
            Engine::Symphonia(inner) => {
                inner.reset();
                if self.codec == Codec::Mp3 {
                    self.skip = MP3_DECODER_DELAY;
                }
            }
            Engine::Platform(dec) => dec.reset(),
            #[cfg(feature = "rust-codecs")]
            Engine::Opus { dec, channels, pre_skip, pre_roll, .. } => {
                // opus-rs has no reset: replace it.
                if let Ok(d) = opus_rs::OpusDecoder::new(OPUS_RATE as i32, *channels) {
                    *dec = d;
                }
                self.skip = if at_start { *pre_skip } else { *pre_roll };
            }
        }
    }
}

/// (rate, channels) from an MPEG audio frame header; `None` if not one.
pub(crate) fn mp3_shape(frame: &[u8]) -> Option<(u32, usize)> {
    let h = u32::from_be_bytes(frame.get(..4)?.try_into().ok()?);
    let (version, layer, bitrate, rate) = ((h >> 19) & 3, (h >> 17) & 3, (h >> 12) & 0xf, (h >> 10) & 3);
    if h >> 21 != 0x7ff || version == 1 || layer == 0 || bitrate == 0xf || rate == 3 {
        return None;
    }
    let base = [44_100, 48_000, 32_000][rate as usize];
    let rate = match version {
        3 => base,
        2 => base / 2,
        _ => base / 4,
    };
    Some((rate, if (h >> 6) & 3 == 3 { 1 } else { 2 }))
}

/// An MP3 frame's `main_data_begin`: bytes back into earlier frames (0: none).
fn mp3_main_data_begin(frame: &[u8]) -> u16 {
    if frame.len() < 7 {
        return 0;
    }
    let mpeg1 = frame[1] & 0x08 != 0;
    let crc = frame[1] & 0x01 == 0;
    let side = if crc { 6 } else { 4 };
    if frame.len() < side + 2 {
        return 0;
    }
    let bits = u16::from_be_bytes([frame[side], frame[side + 1]]);
    // MPEG-1: 9 bits; MPEG-2 and 2.5: 8.
    if mpeg1 { bits >> 7 } else { bits >> 8 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{mp3_frames, ogg_opus};

    fn tone_mp3() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/tone440.mp3")).unwrap()
    }

    #[test]
    fn mp3_decodes_tone() {
        let file = tone_mp3();
        let frames = mp3_frames(&file);
        assert!(frames.len() > 30);
        let mut d = Decoder::new(Codec::Mp3, 44_100, 2, None, true).unwrap();
        let mut out = vec![0i16; Codec::Mp3.max_frames() * 2];
        let mut pcm: Vec<i16> = Vec::new();
        for f in &frames {
            let n = decode_i16(&mut d, f, &mut out).unwrap();
            pcm.extend_from_slice(&out[..n * 2]);
        }
        assert_eq!((d.channels(), d.rate()), (2, 44_100));
        let left: Vec<f64> = pcm.chunks(2).skip(4410).take(22_050).map(|c| c[0] as f64).collect();
        let rms = (left.iter().map(|v| v * v).sum::<f64>() / left.len() as f64).sqrt();
        // ffmpeg's decoder gives 8060.1 over the same stretch.
        assert!((rms - 8060.1).abs() < 5.0, "rms {rms}");
        let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((438..=442).contains(&crossings), "{crossings} crossings");
    }

    #[test]
    fn reset_then_continue() {
        let file = tone_mp3();
        let frames = mp3_frames(&file);
        let mut d = Decoder::new(Codec::Mp3, 44_100, 2, None, true).unwrap();
        let mut out = vec![0i16; 1152 * 2];
        for f in &frames[..10] {
            decode_i16(&mut d, f, &mut out).unwrap();
        }
        d.reset(false);
        let n: usize = frames[20..].iter().map(|f| decode_i16(&mut d, f, &mut out).unwrap_or(0)).sum();
        assert!(n > 1152 * (frames.len() - 22));
    }

    #[test]
    fn mp3_delay_dropped_untrimmed() {
        let file = tone_mp3();
        let frames = mp3_frames(&file);
        let mut out = vec![0i16; 1152 * 2];
        let mut known = Decoder::new(Codec::Mp3, 44_100, 2, None, true).unwrap();
        assert_eq!(decode_i16(&mut known, frames[0], &mut out), Ok(1152));
        let mut unknown = Decoder::new(Codec::Mp3, 44_100, 2, None, false).unwrap();
        assert_eq!(decode_i16(&mut unknown, frames[0], &mut out), Ok(1152 - MP3_DECODER_DELAY));
        assert_eq!(decode_i16(&mut unknown, frames[1], &mut out), Ok(1152));
        // After a seek, always.
        decode_i16(&mut known, frames[1], &mut out).unwrap();
        known.reset(false);
        assert_eq!(decode_i16(&mut known, frames[20], &mut out), Ok(1152 - MP3_DECODER_DELAY));
        assert_eq!(decode_i16(&mut known, frames[21], &mut out), Ok(1152));
    }

    /// Decodes `packet` into `out` as rounded 16-bit, returning frames written.
    fn decode_i16(d: &mut Decoder, packet: &[u8], out: &mut [i16]) -> Result<usize, Fault> {
        let l = d.decode_lent(packet)?;
        out.iter_mut().zip(l.samples).for_each(|(o, &s)| *o = rounded_i16(s));
        Ok(l.samples.len() / l.channels)
    }

    #[test]
    fn reservoir_frame_after_seek() {
        let file = tone_mp3();
        let frames = mp3_frames(&file);
        let reservoir = frames.iter().position(|f| mp3_main_data_begin(f) != 0).expect("a frame using the bit reservoir");
        let mut d = Decoder::new(Codec::Mp3, 44_100, 2, None, true).unwrap();
        let mut out = vec![1i16; 1152 * 2];
        d.reset(false);
        let n = decode_i16(&mut d, frames[reservoir], &mut out).unwrap();
        assert!(out[..n * 2].iter().all(|&s| s == 0));
    }

    #[test]
    fn mp3_header_shape() {
        assert_eq!(mp3_shape(mp3_frames(&tone_mp3())[3]), Some((44_100, 2)));
        // MPEG-2 at 22.05 kHz, mono; MPEG-2.5 at 8 kHz, joint stereo; MPEG-1 at 48 kHz.
        assert_eq!(mp3_shape(&[0xff, 0xf3, 0x00, 0xc0]), Some((22_050, 1)));
        assert_eq!(mp3_shape(&[0xff, 0xe3, 0x08, 0x40]), Some((8_000, 2)));
        assert_eq!(mp3_shape(&[0xff, 0xfb, 0x94, 0x00]), Some((48_000, 2)));
        // Not a header: no sync, the reserved version, the reserved rate, too short.
        assert_eq!(mp3_shape(&[0x00, 0xfb, 0x90, 0x00]), None);
        assert_eq!(mp3_shape(&[0xff, 0xeb, 0x90, 0x00]), None);
        assert_eq!(mp3_shape(&[0xff, 0xfb, 0x9c, 0x00]), None);
        assert_eq!(mp3_shape(&[0xff, 0xfb]), None);

        // Mp3 follows shape change.
        let file = tone_mp3();
        let frames = mp3_frames(&file);
        let mut d = Decoder::new(Codec::Mp3, 44_100, 2, None, true).unwrap();
        d.decode_lent(frames[0]).unwrap();
        // A 22.05 kHz mono MPEG-2 frame of silence: header, side info, no main data.
        let mut mpeg2 = vec![0u8; 64];
        mpeg2[..4].copy_from_slice(&[0xff, 0xf3, 0x10, 0xc0]);
        mpeg2.truncate(mp3_frame_len(&mpeg2));
        assert_eq!(d.decode_lent(&mpeg2).map(|l| l.samples.len() / l.channels), Ok(576), "a frame of the new shape decodes");
        assert_eq!((d.rate(), d.channels()), (22_050, 1));
        assert!(d.decode_lent(frames[5]).is_ok(), "and back");
        assert_eq!((d.rate(), d.channels()), (44_100, 2));
    }

    /// An MPEG-2 layer III frame's length from its header (8 kbps steps per the table's index).
    fn mp3_frame_len(h: &[u8]) -> usize {
        let kbps = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][(h[2] >> 4) as usize];
        72 * kbps * 1000 / mp3_shape(h).unwrap().0 as usize
    }

    #[test]
    fn he_aac_found() {
        // AudioSpecificConfigs: object type, rate index, channels, GASpecificConfig, extension.
        let lc_44 = [0x12, 0x10]; // AAC-LC, 44.1 kHz, stereo
        let lc_22 = [0x13, 0x90]; // AAC-LC, 22.05 kHz, stereo
        let sbr = [0x2b, 0x92, 0x08, 0x00]; // object type 5 (SBR), 22.05 kHz core, stereo, 44.1 kHz out
        let ps = [0xeb, 0x09, 0x88, 0x00]; // object type 29 (PS), 24 kHz core, mono
        let explicit = [0x13, 0x90, 0x56, 0xe5, 0x98]; // AAC-LC 22.05 kHz stereo, sync 0x2b7, SBR 5, present, 44.1 kHz
        let explicit_off = [0x11, 0x90, 0x56, 0xe5, 0x00]; // AAC-LC 48 kHz, sync 0x2b7, SBR 5, not present
        assert!(!he_aac(Some(&lc_44), 44_100), "AAC-LC at a full rate");
        assert!(he_aac(Some(&lc_22), 22_050), "AAC-LC at a core rate: SBR signalled in the stream, as radio sends it");
        assert!(he_aac(Some(&sbr), 22_050) && he_aac(Some(&sbr), 44_100));
        assert!(he_aac(Some(&ps), 24_000));
        assert!(he_aac(Some(&explicit), 22_050), "the SBR extension after an AAC-LC config");
        assert!(!he_aac(Some(&explicit_off), 48_000), "the extension saying there is no SBR");
        assert!(he_aac(None, 22_050) && !he_aac(None, 48_000), "ADTS: by its rate");
        assert!(!he_aac(None, 0), "unknown: as it says");
        assert!(!he_aac(Some(&[0x0a, 0x10]), 22_050), "AAC Main, whatever its rate");

        // He aac uses platform decoder.
        let echo: PlatformAac = |_| Some(Box::new(Echo));
        let mut d = Decoder::whole_aac(Some(echo), 22_050, 2, None).unwrap();
        assert!(d.on_platform(), "an ADTS stream at a core rate");
        let l = d.decode_lent(&[7, 1, 2]).unwrap();
        assert_eq!((l.samples.len(), l.channels, l.rate), (4096, 2, 44_100));
        assert!(l.samples.iter().all(|&s| s == 7.0));
        assert_eq!((d.rate(), d.channels()), (44_100, 2), "the platform's shape is the stream's");
        assert!(!Decoder::whole_aac(Some(echo), 44_100, 2, Some(&[0x12, 0x10])).unwrap().on_platform(), "AAC-LC is decoded here");
        assert!(!Decoder::new(Codec::Aac, 22_050, 2, None, false).unwrap().on_platform(), "the core alone, when asked for");
    }

    /// Outputs each unit's first byte as 2048 stereo frames at 44.1 kHz.
    struct Echo;

    impl PlatformDecoder for Echo {
        fn decode(&mut self, unit: &[u8], out: &mut Vec<f32>) -> Result<(usize, u32), Fault> {
            out.extend(std::iter::repeat_n(unit[0] as f32, 2048 * 2));
            Ok((2, 44_100))
        }
        fn drain(&mut self, _: &mut Vec<f32>) -> Result<(usize, u32), Fault> {
            Ok((2, 44_100))
        }
        fn reset(&mut self) {}
    }

    #[test]
    fn i16_rounds_not_truncates() {
        assert_eq!([0.5 / 32768.0, -1.6 / 32768.0, 1.0, -1.0].map(rounded_i16), [0, -2, 32767, -32768], "ties to even, rounded, clamped");
    }

    #[test]
    fn flac_setup_from_media3() {
        // "fLaC", a STREAMINFO block header, then a 34-byte STREAMINFO for 44.1 kHz stereo 16-bit.
        let mut e = b"fLaC\x80\x00\x00\x22".to_vec();
        e.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0x0A, 0xC4, 0x42, 0xF0, 0, 0, 0, 0]);
        e.extend_from_slice(&[0u8; 16]);
        assert!(Decoder::new(Codec::Flac, 44_100, 2, Some(&e), false).is_ok());
    }

    #[test]
    fn opus() {
        let file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/tone440.opus")).unwrap();
        let (setup, packets) = ogg_opus(&file);
        let mut d = Decoder::new(Codec::Opus, 48_000, 2, Some(&setup), false).unwrap();
        let mut out = vec![0i16; Codec::Opus.max_frames() * 2];
        let mut pcm: Vec<i16> = Vec::new();
        for p in &packets {
            let n = decode_i16(&mut d, p, &mut out).unwrap();
            pcm.extend_from_slice(&out[..n * 2]);
        }
        assert_eq!((d.channels(), d.rate()), (2, 48_000));
        // One second of tone plus at most the last packet's padding.
        assert!(pcm.len() / 2 >= 48_000 && pcm.len() / 2 < 48_000 + 960, "{} frames", pcm.len() / 2);
        let left: Vec<f64> = pcm.chunks(2).skip(4800).take(24_000).map(|c| c[0] as f64).collect();
        let rms = (left.iter().map(|v| v * v).sum::<f64>() / left.len() as f64).sqrt();
        // ffmpeg's libopus decode gives 8463.3 over the same stretch.
        assert!((rms - 8463.3).abs() < 30.0, "rms {rms}");
        let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((438..=442).contains(&crossings), "{crossings} crossings");
        d.reset(false);
        assert_eq!(decode_i16(&mut d, &packets[20], &mut out), Ok(0), "the first 20 ms go to the 80 ms pre-roll");

        // Surround opus rejected.
        let mut head = b"OpusHead\x01\x06\x38\x01\x80\xbb\x00\x00\x00\x00\x01".to_vec();
        head.extend_from_slice(&[4, 2, 0, 4, 1, 2, 3, 5]);
        assert!(Decoder::new(Codec::Opus, 48_000, 6, Some(&head), false).is_err());
    }

}
