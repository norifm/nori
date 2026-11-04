//! nori's final audio for a cast (`audio-out`), encoded here and sent on by the host: into a socket
//! the plugin opened, or to each device that fetches it at the door. The engine pumps it in real time
//! (`nori_engine::cast`), or for a deep sink ([`Deep`]) in the bursts the device asks for; no plugin
//! code runs per buffer.

use std::io::Write;
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::deep::Deep;

/// How long a device may hold a write up before it is let go.
const WRITE_WAIT: Duration = Duration::from_secs(2);

/// Frames in a FLAC frame.
const FLAC_BLOCK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// 16-bit samples, interleaved: little-endian into a socket (a snapserver's TCP source), big-endian
    /// at the door (`audio/L16`, RFC 2586).
    Pcm16,
    /// 16-bit samples after a WAV header of unknown length.
    Wav,
    /// FLAC with verbatim (uncompressed) subframes: lossless, every decoder plays it, no encoder to run.
    Flac,
}

impl Encoding {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Encoding::Pcm16 => "pcm",
            Encoding::Wav => "wav",
            Encoding::Flac => "flac",
        }
    }

    fn content_type(self, rate: u32, channels: u8) -> String {
        match self {
            Encoding::Pcm16 => format!("audio/L16;rate={rate};channels={channels}"),
            Encoding::Wav => "audio/wav".into(),
            Encoding::Flac => "audio/flac".into(),
        }
    }
}

/// Where a sink's bytes go.
enum To {
    /// A plugin's socket.
    Socket(TcpStream),
    /// The devices fetching it at the door.
    Listeners(Vec<TcpStream>),
}

struct Out {
    to: To,
    /// Rate and channels, once the engine opened it.
    format: Option<(u32, u8)>,
    /// The stream's header (WAV, FLAC's STREAMINFO), sent first to each.
    header: Vec<u8>,
    /// Bytes encoded for the next send, kept between writes.
    bytes: Vec<u8>,
    /// FLAC: samples waiting for a full frame, and frames made.
    pending: Vec<i16>,
    frames: u64,
    /// Its socket failed.
    gone: bool,
}

pub struct Sink {
    pub encoding: Encoding,
    /// The rate and channels the device takes; None: what nori plays (each song's own).
    pub rate: Option<u32>,
    pub channels: Option<u8>,
    /// A deep sink's connection: the engine speaks nori-sink-protocol over it instead of pumping.
    pub deep: Option<Arc<Deep>>,
    latency_ms: AtomicU32,
    out: Mutex<Out>,
}

impl Sink {
    fn new(encoding: Encoding, rate: Option<u32>, channels: Option<u8>, to: To) -> Sink {
        let out = Out { to, format: None, header: Vec::new(), bytes: Vec::new(), pending: Vec::new(), frames: 0, gone: false };
        Sink { encoding, rate, channels, deep: None, latency_ms: AtomicU32::new(0), out: Mutex::new(out) }
    }

    pub(crate) fn to_socket(encoding: Encoding, socket: TcpStream, rate: Option<u32>, channels: Option<u8>) -> Sink {
        let _ = socket.set_write_timeout(Some(WRITE_WAIT));
        Sink::new(encoding, rate, channels, To::Socket(socket))
    }

    /// A deep sink over `deep`, at `rate` and `channels` if the plugin said them.
    pub fn deep(deep: Arc<Deep>, rate: Option<u32>, channels: Option<u8>) -> Sink {
        let mut sink = Sink::new(Encoding::Pcm16, rate, channels, To::Listeners(Vec::new()));
        sink.deep = Some(deep);
        sink
    }

    pub(crate) fn served(encoding: Encoding) -> Sink {
        Sink::new(encoding, None, None, To::Listeners(Vec::new()))
    }

    fn out(&self) -> std::sync::MutexGuard<'_, Out> {
        self.out.lock().expect("not poisoned")
    }

    /// The device's delay, as the plugin said it.
    pub fn latency_us(&self) -> u64 {
        self.latency_ms.load(Ordering::Relaxed) as u64 * 1000
    }

    pub(crate) fn set_latency_ms(&self, ms: u32) {
        self.latency_ms.store(ms, Ordering::Relaxed);
    }

    /// The music starts at `rate` and `channels`: the stream's header goes out (to the devices that
    /// fetch it later, first).
    pub fn open(&self, rate: u32, channels: u8) {
        let mut o = self.out();
        o.format = Some((rate, channels));
        o.header = header(self.encoding, rate, channels);
        o.pending.clear();
        o.frames = 0;
        let header = std::mem::take(&mut o.header);
        match &mut o.to {
            To::Socket(s) => {
                if s.write_all(&header).is_err() {
                    o.gone = true;
                }
            }
            To::Listeners(l) => l.retain_mut(|c| c.write_all(&header).is_ok()),
        }
        o.header = header;
    }

    /// Sends `samples` (interleaved). False once its socket failed: the device is gone.
    pub fn write(&self, samples: &[i16]) -> bool {
        let mut o = self.out();
        let Some((_, channels)) = o.format else { return !o.gone };
        let Out { to, bytes, pending, frames, gone, .. } = &mut *o;
        bytes.clear();
        match (self.encoding, &*to) {
            (Encoding::Pcm16, To::Listeners(_)) => bytes.extend(samples.iter().flat_map(|s| s.to_be_bytes())),
            (Encoding::Pcm16 | Encoding::Wav, _) => bytes.extend(samples.iter().flat_map(|s| s.to_le_bytes())),
            (Encoding::Flac, _) => {
                pending.extend_from_slice(samples);
                let block = FLAC_BLOCK * channels as usize;
                let mut at = 0;
                while pending.len() - at >= block {
                    flac_frame(bytes, &pending[at..at + block], channels, *frames);
                    *frames += 1;
                    at += block;
                }
                pending.drain(..at);
            }
        }
        match to {
            To::Socket(s) => {
                if !bytes.is_empty() && s.write_all(bytes).is_err() {
                    *gone = true;
                }
            }
            To::Listeners(l) => l.retain_mut(|c| bytes.is_empty() || c.write_all(bytes).is_ok()),
        }
        !*gone
    }

    /// A device fetching it at the door: it hears the stream from now on.
    pub(crate) fn listen(&self, mut conn: TcpStream) {
        let mut o = self.out();
        let (rate, channels) = o.format.unwrap_or((self.rate.unwrap_or(44_100), self.channels.unwrap_or(2)));
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", self.encoding.content_type(rate, channels));
        let _ = conn.set_write_timeout(Some(WRITE_WAIT));
        if conn.write_all(head.as_bytes()).and_then(|_| conn.write_all(&o.header)).is_err() {
            return;
        }
        if let To::Listeners(l) = &mut o.to {
            l.push(conn);
        }
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        if let Ok(o) = self.out.get_mut() {
            if let To::Listeners(l) = &o.to {
                for c in l {
                    let _ = c.shutdown(Shutdown::Both);
                }
            }
        }
    }
}

fn header(encoding: Encoding, rate: u32, channels: u8) -> Vec<u8> {
    match encoding {
        Encoding::Pcm16 => Vec::new(),
        Encoding::Wav => {
            let block = channels as u16 * 2;
            let mut h = Vec::with_capacity(44);
            h.extend_from_slice(b"RIFF");
            h.extend_from_slice(&u32::MAX.to_le_bytes());
            h.extend_from_slice(b"WAVEfmt ");
            h.extend_from_slice(&16u32.to_le_bytes());
            h.extend_from_slice(&1u16.to_le_bytes());
            h.extend_from_slice(&(channels as u16).to_le_bytes());
            h.extend_from_slice(&rate.to_le_bytes());
            h.extend_from_slice(&(rate * block as u32).to_le_bytes());
            h.extend_from_slice(&block.to_le_bytes());
            h.extend_from_slice(&16u16.to_le_bytes());
            h.extend_from_slice(b"data");
            h.extend_from_slice(&u32::MAX.to_le_bytes());
            h
        }
        Encoding::Flac => {
            let mut h = b"fLaC".to_vec();
            // The last metadata block, STREAMINFO, 34 bytes.
            h.extend_from_slice(&[0x80, 0, 0, 34]);
            h.extend_from_slice(&(FLAC_BLOCK as u16).to_be_bytes());
            h.extend_from_slice(&(FLAC_BLOCK as u16).to_be_bytes());
            // Frame sizes unknown.
            h.extend_from_slice(&[0; 6]);
            // Rate (20 bits), channels - 1 (3), bits - 1 (5), total samples (36, unknown).
            let packed: u64 = (rate as u64) << 44 | ((channels as u64 - 1) & 7) << 41 | 15 << 36;
            h.extend_from_slice(&packed.to_be_bytes());
            h.extend_from_slice(&[0; 16]);
            h
        }
    }
}

/// One FLAC frame of `samples` (interleaved, [`FLAC_BLOCK`] frames), numbered `number`, onto `out`.
fn flac_frame(out: &mut Vec<u8>, samples: &[i16], channels: u8, number: u64) {
    let start = out.len();
    // Sync, fixed block size; 4096 frames, the rate from STREAMINFO; independent channels, 16 bits.
    out.extend_from_slice(&[0xff, 0xf8, 0xc0, (channels - 1) << 4 | 0b1000]);
    utf8_number(out, number);
    let crc = crc8(&out[start..]);
    out.push(crc);
    for c in 0..channels as usize {
        // A verbatim subframe.
        out.push(0x02);
        for frame in samples.chunks_exact(channels as usize) {
            out.extend_from_slice(&frame[c].to_be_bytes());
        }
    }
    let crc = crc16(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A frame number as FLAC codes it, UTF-8's way (up to 36 bits).
fn utf8_number(out: &mut Vec<u8>, n: u64) {
    if n < 0x80 {
        return out.push(n as u8);
    }
    let len = match 64 - n.leading_zeros() {
        ..=11 => 2,
        12..=16 => 3,
        17..=21 => 4,
        22..=26 => 5,
        27..=31 => 6,
        _ => 7,
    };
    let lead = (0xff00u16 >> len) as u8;
    out.push(lead | (n >> (6 * (len - 1))) as u8 & !(lead | 0x80 >> len));
    for i in (0..len - 1).rev() {
        out.push(0x80 | (n >> (6 * i)) as u8 & 0x3f);
    }
}

/// FLAC's CRC-8 (polynomial 0x07) and CRC-16 (0x8005) of each byte value, MSB first.
const CRC8: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u8;
        let mut k = 0;
        while k < 8 {
            crc = if crc & 0x80 != 0 { crc << 1 ^ 0x07 } else { crc << 1 };
            k += 1;
        }
        t[i] = crc;
        i += 1;
    }
    t
};
/// CRC-16 tables for eight bytes at a time: `CRC16[k][b]` is byte `b` followed by `k` zero bytes.
const CRC16: [[u16; 256]; 8] = {
    let mut t = [[0u16; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut k = 0;
        while k < 8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x8005 } else { crc << 1 };
            k += 1;
        }
        t[0][i] = crc;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let prev = t[k - 1][i];
            t[k][i] = prev << 8 ^ t[0][(prev >> 8) as usize];
            i += 1;
        }
        k += 1;
    }
    t
};

fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |crc, &b| CRC8[(crc ^ b) as usize])
}

fn crc16(bytes: &[u8]) -> u16 {
    let t = &CRC16;
    let (chunks, rest) = bytes.as_chunks::<8>();
    let crc = chunks.iter().fold(0u16, |crc, b| {
        let [hi, lo] = crc.to_be_bytes();
        t[7][(b[0] ^ hi) as usize] ^ t[6][(b[1] ^ lo) as usize] ^ t[5][b[2] as usize] ^ t[4][b[3] as usize] ^ t[3][b[4] as usize] ^ t[2][b[5] as usize] ^ t[1][b[6] as usize] ^ t[0][b[7] as usize]
    });
    rest.iter().fold(crc, |crc, &b| crc << 8 ^ t[0][((crc >> 8) as u8 ^ b) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// A sink into a socket, and the other end.
    fn socket(encoding: Encoding) -> (Sink, TcpStream) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let s = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (other, _) = l.accept().unwrap();
        (Sink::to_socket(encoding, s, Some(48_000), Some(2)), other)
    }

    fn heard(mut other: TcpStream) -> Vec<u8> {
        let mut got = Vec::new();
        other.read_to_end(&mut got).unwrap();
        got
    }

    #[test]
    fn pcm_and_wav_carry_the_samples_as_they_are() {
        let (sink, other) = socket(Encoding::Wav);
        sink.open(48_000, 2);
        assert!(sink.write(&[1, -2, 3, -4]));
        drop(sink);
        let got = heard(other);
        assert_eq!((&got[..4], &got[8..16], u32::from_le_bytes(got[24..28].try_into().unwrap())), (&b"RIFF"[..], &b"WAVEfmt "[..], 48_000));
        assert_eq!(&got[44..], &[1, 0, 0xfe, 0xff, 3, 0, 0xfc, 0xff]);

        let (sink, other) = socket(Encoding::Pcm16);
        sink.open(48_000, 2);
        sink.write(&[1, -2]);
        drop(sink);
        assert_eq!(heard(other), [1, 0, 0xfe, 0xff], "little-endian, no header, into a socket");
    }

    #[test]
    fn a_gone_device_is_said() {
        let (sink, other) = socket(Encoding::Pcm16);
        sink.open(48_000, 2);
        drop(other);
        let big = vec![0i16; 1 << 16];
        let gone = (0..50).any(|_| !sink.write(&big));
        assert!(gone);
    }

    /// Time to send a minute of stereo, per encoding, to a served stream nobody fetches yet. Not a check:
    /// `cargo test --release -p nori-plugins --lib sink::tests::a_minute_sent -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn a_minute_sent() {
        let samples: Vec<i16> = (0..44_100 * 2).map(|i| ((i * 7919) % 65_536) as i16).collect();
        for encoding in [Encoding::Pcm16, Encoding::Flac] {
            let sink = Sink::served(encoding);
            sink.open(44_100, 2);
            let start = std::time::Instant::now();
            for _ in 0..60 {
                for block in samples.chunks(4096) {
                    sink.write(block);
                }
            }
            println!("perf: {} a minute in {:.1} ms", encoding.extension(), start.elapsed().as_secs_f64() * 1000.0);
        }
    }

    #[test]
    fn flac_frames_are_whole_blocks_a_decoder_checks() {
        let (sink, other) = socket(Encoding::Flac);
        sink.open(44_100, 2);
        let samples: Vec<i16> = (0..FLAC_BLOCK as i32 * 2 * 2 + 10).map(|i| (i * 37 % 65_536 - 32_768) as i16).collect();
        sink.write(&samples[..1000]);
        sink.write(&samples[1000..]);
        drop(sink);
        let got = heard(other);
        assert_eq!(&got[..4], b"fLaC");
        let frame = 4 + 4 + 34;
        let size = 4 + 1 + 1 + 2 * (1 + FLAC_BLOCK * 2) + 2;
        assert_eq!(got.len(), frame + 2 * size, "two whole frames; the rest waits for its block");
        for (n, f) in got[frame..].chunks(size).enumerate() {
            assert_eq!((f[0], f[1], f[4] as usize), (0xff, 0xf8, n));
            assert_eq!(crc8(&f[..5]), f[5]);
            assert_eq!(crc16(&f[..size - 2]), u16::from_be_bytes([f[size - 2], f[size - 1]]));
            let left = i16::from_be_bytes([f[7], f[8]]);
            assert_eq!(left, samples[n * FLAC_BLOCK * 2], "channel after channel");
        }
        let mut utf8 = Vec::new();
        for (n, coded) in [(0x7f, vec![0x7f]), (0x80, vec![0xc2, 0x80]), (0x10ffff, vec![0xf4, 0x8f, 0xbf, 0xbf])] {
            utf8.clear();
            utf8_number(&mut utf8, n);
            assert_eq!(utf8, coded, "{n:x}");
        }
        // CRC-16/UMTS and CRC-8/SMBUS check values (FLAC's).
        assert_eq!((crc16(b"123456789"), crc8(b"123456789")), (0xfee8, 0xf4));
        // Eight bytes at a time, as one at a time, at every length's remainder.
        let bitwise = |bytes: &[u8]| {
            bytes.iter().fold(0u16, |mut crc, &b| {
                crc ^= (b as u16) << 8;
                for _ in 0..8 {
                    crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x8005 } else { crc << 1 };
                }
                crc
            })
        };
        let bytes: Vec<u8> = (0..300u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        for len in 0..bytes.len() {
            assert_eq!(crc16(&bytes[..len]), bitwise(&bytes[..len]), "{len} bytes");
        }
    }
}
