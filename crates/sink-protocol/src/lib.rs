//! The frames between nori (the host) and a deep cast sink (the device): a speaker that holds tens of
//! seconds and takes nori's audio in bursts, asking for more with credits ([`Device::Want`]) instead of
//! being paced by TCP. Every frame is `u32` length (little-endian, the bytes after it), a kind byte, and
//! its fields, little-endian. Strings are a `u16` length and UTF-8; an option is a byte (0 none, 1 some)
//! and the value. A frame decodes borrowing the bytes it came in, so firmware copies no audio.
//! The whole protocol is in docs/developers/plugins.md ("Deep sinks").

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// The protocol's version, said in both directions' first frame.
pub const VERSION: u16 = 1;
/// The longest frame either side sends or takes (kind and fields), bytes.
pub const MAX_FRAME: usize = 1 << 20;

/// How the audio after a [`Host::Format`] is coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Interleaved little-endian samples of `bits` in [`Host::Audio`].
    Pcm,
    /// A FLAC file's own frames in [`Host::Encoded`].
    Flac,
    /// An MP3 file's own frames in [`Host::Encoded`].
    Mp3,
}

impl Codec {
    fn code(self) -> u8 {
        match self {
            Codec::Pcm => 0,
            Codec::Flac => 1,
            Codec::Mp3 => 2,
        }
    }

    fn of(code: u8) -> Result<Codec, Error> {
        match code {
            0 => Ok(Codec::Pcm),
            1 => Ok(Codec::Flac),
            2 => Ok(Codec::Mp3),
            _ => Err(Error::Value),
        }
    }

    /// Its bit in [`Caps::decodes`].
    pub fn bit(self) -> u8 {
        1 << self.code()
    }
}

/// The stream's format from the next audio on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u8,
    /// PCM's sample width, 16 or 24; 0 for an encoded stream (FLAC says its own).
    pub bits: u8,
    pub codec: Codec,
}

/// A device's button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    PlayPause,
    Next,
    Previous,
}

/// What a device can do, its answer to [`Host::Hello`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub version: u16,
    /// What its buffer holds, bytes: the host fits the depth and format to it.
    pub buffer_bytes: u32,
    /// The highest rate it plays: only a song above it is resampled.
    pub max_rate: u32,
    /// The PCM widths it takes: bit 0 16-bit, bit 1 24-bit.
    pub bits: u8,
    /// What it decodes itself, by [`Codec::bit`] (PCM always).
    pub decodes: u8,
    /// The size of the covers it shows, px square; none for no screen.
    pub cover: Option<u32>,
}

impl Caps {
    pub fn decodes(&self, codec: Codec) -> bool {
        codec == Codec::Pcm || self.decodes & codec.bit() != 0
    }

    /// Whether it takes `bits` (16 or 24) per PCM sample.
    pub fn takes_bits(&self, bits: u8) -> bool {
        match bits {
            16 => self.bits & 1 != 0,
            24 => self.bits & 2 != 0,
            _ => false,
        }
    }
}

/// Host to device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Host<'a> {
    /// The first frame; the device answers [`Device::Caps`].
    Hello { version: u16 },
    /// How much to hold, ms: ask for more ([`Device::Want`]) once under half of it.
    Depth { ms: u32 },
    /// The audio after it is in this format. Positional: what came before plays out in its own.
    Format(Format),
    /// PCM samples, whole frames.
    Audio(&'a [u8]),
    /// Whole packets of the file (FLAC or MP3 frames).
    Encoded(&'a [u8]),
    /// The song whose audio follows plays at this gain, dB (ReplayGain). Positional, sent only before
    /// encoded songs: in PCM nori has applied it.
    Gain { db: f32 },
    /// Drop what is held past stream frame `keep` (frames since Hello) and answer [`Device::Flushed`];
    /// none, or played past it already: drop everything not played.
    Flush { seq: u32, keep: Option<u64> },
    /// Stop at once, keeping what is held.
    Pause,
    Resume,
    /// Fade from the gain the fade is at to `target` (linear, 0 to 1) over `ms`.
    Ramp { target: f32, ms: u32 },
    /// The user's volume, linear 0 to 1, heard at once.
    Volume { gain: f32 },
    /// A cover, JPEG, kept for [`Host::Now`] to name by `id`.
    Cover { id: u32, jpeg: &'a [u8] },
    /// The song whose audio follows. Positional.
    Now { id: u32, title: &'a str, artist: &'a str, album: &'a str, duration_ms: u32, cover: Option<u32> },
}

/// Device to host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Device {
    Caps(Caps),
    /// Send this many frames more (it replaces credit left). Encoded audio goes in whole packets, so
    /// the last may overshoot by less than a packet.
    Want { frames: u32 },
    /// Frames played since Hello (never reset), frames held, and the delay after the buffer, µs. Sent
    /// with each [`Device::Want`] and on pause, resume, a depth change and a flush; never periodically.
    Status { played: u64, held: u32, delay_us: u32 },
    Flushed { seq: u32, played: u64 },
    Press(Button),
    /// The device's own volume knob, linear 0 to 1.
    Volume { gain: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A frame longer than [`MAX_FRAME`].
    TooLong,
    /// A kind this version does not know.
    Kind(u8),
    /// Fields cut short or longer than the frame.
    Short,
    /// A field out of range, or text not UTF-8.
    Value,
}

/// Writes a frame's fields after its length.
struct Out<'a>(&'a mut Vec<u8>);

impl Out<'_> {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.u32(v.to_bits())
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    fn str(&mut self, v: &str) -> &mut Self {
        // Cut at a character boundary to what the length holds.
        let mut end = v.len().min(u16::MAX as usize);
        while !v.is_char_boundary(end) {
            end -= 1;
        }
        self.u16(end as u16).bytes(&v.as_bytes()[..end])
    }
    fn opt_u32(&mut self, v: Option<u32>) -> &mut Self {
        match v {
            Some(v) => self.u8(1).u32(v),
            None => self.u8(0),
        }
    }
}

/// Reads a frame's fields.
struct In<'a>(&'a [u8]);

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err(Error::Short);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| Error::Short)?))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| Error::Short)?))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| Error::Short)?))
    }
    fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn str(&mut self) -> Result<&'a str, Error> {
        let n = self.u16()? as usize;
        core::str::from_utf8(self.take(n)?).map_err(|_| Error::Value)
    }
    fn opt<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<Option<T>, Error> {
        match self.u8()? {
            0 => Ok(None),
            1 => f(self).map(Some),
            _ => Err(Error::Value),
        }
    }
    fn rest(&mut self) -> &'a [u8] {
        core::mem::take(&mut self.0)
    }
    fn end<T>(&self, v: T) -> Result<T, Error> {
        if self.0.is_empty() {
            Ok(v)
        } else {
            Err(Error::Short)
        }
    }
}

/// Appends one frame: its length, then `fields` written after the kind.
fn frame(out: &mut Vec<u8>, kind: u8, fields: impl FnOnce(&mut Out)) {
    let at = out.len();
    out.extend_from_slice(&[0; 4]);
    out.push(kind);
    fields(&mut Out(out));
    let len = (out.len() - at - 4) as u32;
    out[at..at + 4].copy_from_slice(&len.to_le_bytes());
}

impl Host<'_> {
    /// Appends the frame to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match *self {
            Host::Hello { version } => frame(out, 0x01, |o| {
                o.u16(version);
            }),
            Host::Depth { ms } => frame(out, 0x02, |o| {
                o.u32(ms);
            }),
            Host::Format(f) => frame(out, 0x03, |o| {
                o.u32(f.rate).u8(f.channels).u8(f.bits).u8(f.codec.code());
            }),
            Host::Audio(b) => frame(out, 0x04, |o| {
                o.bytes(b);
            }),
            Host::Encoded(b) => frame(out, 0x05, |o| {
                o.bytes(b);
            }),
            Host::Gain { db } => frame(out, 0x06, |o| {
                o.f32(db);
            }),
            Host::Flush { seq, keep } => frame(out, 0x07, |o| {
                o.u32(seq);
                match keep {
                    Some(k) => o.u8(1).u64(k),
                    None => o.u8(0),
                };
            }),
            Host::Pause => frame(out, 0x08, |_| {}),
            Host::Resume => frame(out, 0x09, |_| {}),
            Host::Ramp { target, ms } => frame(out, 0x0a, |o| {
                o.f32(target).u32(ms);
            }),
            Host::Volume { gain } => frame(out, 0x0b, |o| {
                o.f32(gain);
            }),
            Host::Cover { id, jpeg } => frame(out, 0x0c, |o| {
                o.u32(id).bytes(jpeg);
            }),
            Host::Now { id, title, artist, album, duration_ms, cover } => frame(out, 0x0d, |o| {
                o.u32(id).str(title).str(artist).str(album).u32(duration_ms).opt_u32(cover);
            }),
        }
    }

    fn decode<'a>(kind: u8, b: &'a [u8]) -> Result<Host<'a>, Error> {
        let mut i = In(b);
        let f = match kind {
            0x01 => Host::Hello { version: i.u16()? },
            0x02 => Host::Depth { ms: i.u32()? },
            0x03 => Host::Format(Format { rate: i.u32()?, channels: i.u8()?, bits: i.u8()?, codec: Codec::of(i.u8()?)? }),
            0x04 => Host::Audio(i.rest()),
            0x05 => Host::Encoded(i.rest()),
            0x06 => Host::Gain { db: i.f32()? },
            0x07 => Host::Flush { seq: i.u32()?, keep: i.opt(In::u64)? },
            0x08 => Host::Pause,
            0x09 => Host::Resume,
            0x0a => Host::Ramp { target: i.f32()?, ms: i.u32()? },
            0x0b => Host::Volume { gain: i.f32()? },
            0x0c => Host::Cover { id: i.u32()?, jpeg: i.rest() },
            0x0d => Host::Now { id: i.u32()?, title: i.str()?, artist: i.str()?, album: i.str()?, duration_ms: i.u32()?, cover: i.opt(In::u32)? },
            k => return Err(Error::Kind(k)),
        };
        i.end(f)
    }
}

impl Device {
    /// Appends the frame to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match *self {
            Device::Caps(c) => frame(out, 0x81, |o| {
                o.u16(c.version).u32(c.buffer_bytes).u32(c.max_rate).u8(c.bits).u8(c.decodes).opt_u32(c.cover);
            }),
            Device::Want { frames } => frame(out, 0x82, |o| {
                o.u32(frames);
            }),
            Device::Status { played, held, delay_us } => frame(out, 0x83, |o| {
                o.u64(played).u32(held).u32(delay_us);
            }),
            Device::Flushed { seq, played } => frame(out, 0x84, |o| {
                o.u32(seq).u64(played);
            }),
            Device::Press(b) => frame(out, 0x85, |o| {
                o.u8(match b {
                    Button::PlayPause => 0,
                    Button::Next => 1,
                    Button::Previous => 2,
                });
            }),
            Device::Volume { gain } => frame(out, 0x86, |o| {
                o.f32(gain);
            }),
        }
    }

    fn decode(kind: u8, b: &[u8]) -> Result<Device, Error> {
        let mut i = In(b);
        let f = match kind {
            0x81 => Device::Caps(Caps { version: i.u16()?, buffer_bytes: i.u32()?, max_rate: i.u32()?, bits: i.u8()?, decodes: i.u8()?, cover: i.opt(In::u32)? }),
            0x82 => Device::Want { frames: i.u32()? },
            0x83 => Device::Status { played: i.u64()?, held: i.u32()?, delay_us: i.u32()? },
            0x84 => Device::Flushed { seq: i.u32()?, played: i.u64()? },
            0x85 => Device::Press(match i.u8()? {
                0 => Button::PlayPause,
                1 => Button::Next,
                2 => Button::Previous,
                _ => return Err(Error::Value),
            }),
            0x86 => Device::Volume { gain: i.f32()? },
            k => return Err(Error::Kind(k)),
        };
        i.end(f)
    }
}

/// Gathers bytes as they arrive and splits them into frames.
#[derive(Debug, Default)]
pub struct Frames {
    buf: Vec<u8>,
    /// Bytes at the front already handed out as frames.
    used: usize,
}

impl Frames {
    pub fn new() -> Frames {
        Frames::default()
    }

    /// Takes `bytes` that arrived.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.used > 0 {
            self.buf.drain(..self.used);
            self.used = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame's kind and fields, if one has arrived.
    fn next_raw(&mut self) -> Option<Result<(u8, &[u8]), Error>> {
        let b = &self.buf[self.used..];
        let len = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
        if len > MAX_FRAME {
            return Some(Err(Error::TooLong));
        }
        if len == 0 {
            return Some(Err(Error::Short));
        }
        if b.len() < 4 + len {
            return None;
        }
        let at = self.used;
        self.used += 4 + len;
        Some(Ok((self.buf[at + 4], &self.buf[at + 5..at + 4 + len])))
    }

    /// The next frame from the host, read on a device.
    pub fn next_host(&mut self) -> Option<Result<Host<'_>, Error>> {
        self.next_raw().map(|r| r.and_then(|(k, b)| Host::decode(k, b)))
    }

    /// The next frame from the device, read on the host.
    pub fn next_device(&mut self) -> Option<Result<Device, Error>> {
        self.next_raw().map(|r| r.and_then(|(k, b)| Device::decode(k, b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_decodes_as_it_was_encoded() {
        let f = Format { rate: 44_100, channels: 2, bits: 24, codec: Codec::Pcm };
        let host = [
            Host::Hello { version: VERSION },
            Host::Depth { ms: 45_000 },
            Host::Format(f),
            Host::Audio(&[1, 2, 3, 4]),
            Host::Encoded(&[0xff, 0xf8]),
            Host::Gain { db: -6.5 },
            Host::Flush { seq: 7, keep: Some(1 << 40) },
            Host::Flush { seq: 8, keep: None },
            Host::Pause,
            Host::Resume,
            Host::Ramp { target: 0.25, ms: 300 },
            Host::Volume { gain: 0.5 },
            Host::Cover { id: 3, jpeg: &[0xff, 0xd8] },
            Host::Now { id: 4, title: "Blue", artist: "Ä", album: "", duration_ms: 1000, cover: Some(3) },
        ];
        let mut bytes = Vec::new();
        for h in &host {
            h.encode(&mut bytes);
        }
        let mut frames = Frames::new();
        // Bytes arrive one at a time: a frame comes out only once whole.
        let mut got = Vec::new();
        for b in &bytes {
            frames.push(core::slice::from_ref(b));
            while let Some(h) = frames.next_host() {
                got.push(format!("{:?}", h.unwrap()));
            }
        }
        assert_eq!(got, host.iter().map(|h| format!("{h:?}")).collect::<Vec<_>>());

        let caps = Caps { version: VERSION, buffer_bytes: 16 << 20, max_rate: 96_000, bits: 3, decodes: Codec::Flac.bit() | Codec::Mp3.bit(), cover: Some(240) };
        let device = [
            Device::Caps(caps),
            Device::Want { frames: 480_000 },
            Device::Status { played: 1 << 33, held: 9, delay_us: 20_000 },
            Device::Flushed { seq: 7, played: 12 },
            Device::Press(Button::Next),
            Device::Volume { gain: 0.75 },
        ];
        let mut bytes = Vec::new();
        for d in &device {
            d.encode(&mut bytes);
        }
        frames.push(&bytes);
        let got: Vec<Device> = core::iter::from_fn(|| frames.next_device()).map(Result::unwrap).collect();
        assert_eq!(got, device);
        assert!(caps.decodes(Codec::Mp3) && caps.takes_bits(24) && !caps.takes_bits(32));
    }

    #[test]
    fn a_bad_frame_is_said() {
        let mut frames = Frames::new();
        frames.push(&[2, 0, 0, 0, 0x7f, 0]);
        assert_eq!(frames.next_host().map(|r| r.err()), Some(Some(Error::Kind(0x7f))));
        frames.push(&[2, 0, 0, 0, 0x01, 1]);
        assert_eq!(frames.next_host().map(|r| r.err()), Some(Some(Error::Short)), "a field past the frame's end");
        frames.push(&[4, 0, 0, 0, 0x01, 1, 0, 9]);
        assert_eq!(frames.next_host().map(|r| r.err()), Some(Some(Error::Short)), "bytes left after the fields");
        frames.push(&((MAX_FRAME + 1) as u32).to_le_bytes());
        assert_eq!(frames.next_host().map(|r| r.err()), Some(Some(Error::TooLong)));
    }
}
