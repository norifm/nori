//! A FLAC song opened at a time with a ranged read or two: the frame for the time is guessed from the
//! seek table or the bitrate, found by the next frame header (sync code and CRC-8), and guessed once
//! more from there when it missed. Symphonia's own seek bisects the whole file, a request per probe on a
//! song still arriving. The song then reads as its stream info followed by the frames from that one
//! ([`Landed`]).

use std::io::{self, Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

/// What the song's metadata says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Head {
    /// The stream info block's body.
    info: [u8; 34],
    /// Where the first frame starts.
    first: u64,
    pub rate: u32,
    channels: u32,
    /// Samples per channel; 0 when not said.
    total: u64,
    /// The frames' block length when fixed (frame numbers count blocks of it).
    block: Option<u32>,
    /// The longest frame, samples.
    max_block: u32,
    /// The seek table's points: sample and byte offset from the first frame.
    points: Vec<(u64, u64)>,
}

/// Bytes scanned for a frame header after a guess.
const SCAN: usize = 64 * 1024;
/// A frame found this close before the time is read on from: decoding forward is cheaper than a request.
const CLOSE_MS: u64 = 2_000;
/// A guess lands this much before the time, against the bitrate varying.
const BACK_MS: u64 = 250;

/// The FLAC metadata at the start of `source`; None when it is not FLAC.
pub(crate) fn head(source: &mut dyn MediaSource) -> io::Result<Option<Head>> {
    source.seek(SeekFrom::Start(0))?;
    let mut magic = [0u8; 4];
    if read_up_to(source, &mut magic)? < 4 || &magic != b"fLaC" {
        return Ok(None);
    }
    let (mut at, mut info, mut points) = (4u64, None, Vec::new());
    loop {
        let mut h = [0u8; 4];
        source.read_exact(&mut h)?;
        let len = u32::from_be_bytes([0, h[1], h[2], h[3]]) as u64;
        match h[0] & 0x7f {
            0 if len == 34 => {
                let mut b = [0u8; 34];
                source.read_exact(&mut b)?;
                info = Some(b);
            }
            3 => {
                let mut b = vec![0u8; len as usize];
                source.read_exact(&mut b)?;
                points = b.as_chunks::<18>().0.iter().map(|p| (be(&p[..8]), be(&p[8..16]))).filter(|p| p.0 != u64::MAX).collect();
            }
            _ => {
                source.seek(SeekFrom::Start(at + 4 + len))?;
            }
        }
        at += 4 + len;
        if h[0] & 0x80 != 0 {
            break;
        }
    }
    let Some(info) = info else { return Ok(None) };
    let (min_block, max_block) = (be(&info[0..2]) as u32, be(&info[2..4]) as u32);
    let packed = be(&info[10..18]);
    let rate = (packed >> 44) as u32;
    let channels = ((packed >> 41) & 7) as u32 + 1;
    let total = packed & 0xf_ffff_ffff;
    let block = (min_block == max_block).then_some(max_block);
    Ok(Some(Head { info, first: at, rate, channels, total, block, max_block, points }))
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0, |n, &x| n << 8 | x as u64)
}

/// `source` (of `len` bytes) to open at `from_ms`: a FLAC song from the frame of that time on
/// ([`Landed`]) and the sample of the time, or else as it was, at its start.
pub(crate) fn landed(mut source: Box<dyn MediaSource>, len: u64, from_ms: i64) -> io::Result<(Box<dyn MediaSource>, Option<u64>)> {
    if let Some(head) = head(&mut *source)? {
        let ts = from_ms as u64 * head.rate as u64 / 1000;
        if let Some((at, _)) = land(&mut *source, &head, len, ts)? {
            return Ok((Box::new(Landed::new(source, &head, at, len)), Some(ts)));
        }
    }
    source.seek(SeekFrom::Start(0))?;
    Ok((source, None))
}

/// Where the frame holding sample `ts` (or one shortly before it) starts in a song of `len` bytes, and
/// its first sample. None when no frame is found where it should be.
pub(crate) fn land(source: &mut dyn MediaSource, head: &Head, len: u64, ts: u64) -> io::Result<Option<(u64, u64)>> {
    if head.total == 0 || ts >= head.total || head.rate == 0 {
        return Ok(None);
    }
    // The seek points around the time, else the song's ends.
    let lo = head.points.iter().filter(|p| p.0 <= ts).max_by_key(|p| p.0).map_or((0, head.first), |p| (p.0, head.first + p.1));
    let hi = head.points.iter().filter(|p| p.0 > ts).min_by_key(|p| p.0).map_or((head.total, len), |p| (p.0, head.first + p.1));
    let per = (hi.1.saturating_sub(lo.1)) as f64 / (hi.0 - lo.0).max(1) as f64;
    let ms = |samples: u64| samples * 1000 / head.rate as u64;
    // Each guess lands a frame and BACK_MS early at the bitrate it assumes: the next header after it is
    // then before the time.
    let back = (head.rate as u64 * BACK_MS / 1000 + head.max_block as u64) as f64;
    let guess = |from: (u64, u64), per: f64| (from.1 as f64 + (ts as f64 - from.0 as f64 - back) * per).max(lo.1 as f64) as u64;
    let Some(first) = frame_after(source, head, if ms(ts - lo.0) <= CLOSE_MS { lo.1 } else { guess(lo, per) })? else { return Ok(None) };
    if first.1 <= ts && ms(ts - first.1) <= CLOSE_MS {
        return Ok(Some(first));
    }
    // Once more from the frame found, at the bitrate before it or after it: the higher going back, the
    // lower going on, so the guess errs early, where reading on finds the time.
    let before = (first.0.saturating_sub(lo.1)) as f64 / first.1.saturating_sub(lo.0).max(1) as f64;
    let after = (hi.1.saturating_sub(first.0)) as f64 / hi.0.saturating_sub(first.1).max(1) as f64;
    let per = if first.1 > ts { before.max(after) } else { before.min(after) };
    let again = frame_after(source, head, guess((first.1, first.0), per))?;
    Ok(match again {
        Some(f) if f.1 <= ts => Some(f),
        _ if first.1 <= ts => Some(first),
        _ => None,
    })
}

/// The first frame header at or after byte `at`: its offset and first sample.
fn frame_after(source: &mut dyn MediaSource, head: &Head, at: u64) -> io::Result<Option<(u64, u64)>> {
    source.seek(SeekFrom::Start(at))?;
    let mut buf = Vec::with_capacity(SCAN);
    let mut from = 0;
    while buf.len() < SCAN {
        let had = buf.len();
        buf.resize(SCAN.min(had + 8192), 0);
        let n = source.read(&mut buf[had..])?;
        buf.truncate(had + n);
        if n == 0 {
            break;
        }
        // A header may straddle what came: looked at again once more is here.
        while from + MAX_HEADER <= buf.len() {
            if let Some(sample) = frame_header(&buf[from..], head) {
                return Ok(Some((at + from as u64, sample)));
            }
            from += 1;
        }
    }
    Ok(None)
}

/// The longest frame header.
const MAX_HEADER: usize = 16;

/// The first sample of the frame whose header starts `b`, if a valid one does.
fn frame_header(b: &[u8], head: &Head) -> Option<u64> {
    if b[0] != 0xff || b[1] & 0xfe != 0xf8 {
        return None;
    }
    let variable = b[1] & 1 == 1;
    let (block_code, rate_code, channels, size_code) = (b[2] >> 4, b[2] & 15, b[3] >> 4, (b[3] >> 1) & 7);
    if block_code == 0 || rate_code == 15 || channels > 10 || size_code == 3 || b[3] & 1 != 0 {
        return None;
    }
    let count = if channels < 8 { channels as u32 + 1 } else { 2 };
    if count != head.channels || variable == head.block.is_some() {
        return None;
    }
    // The UTF-8 coded frame or sample number.
    let ones = b[4].leading_ones() as usize;
    let len = match ones {
        0 => 1,
        2..=7 => ones,
        _ => return None,
    };
    let mut n = (b[4] & (0x7f >> ones)) as u64;
    for &c in &b[5..4 + len] {
        if c & 0xc0 != 0x80 {
            return None;
        }
        n = n << 6 | (c & 0x3f) as u64;
    }
    let mut at = 4 + len;
    at += match block_code {
        6 => 1,
        7 => 2,
        _ => 0,
    };
    at += match rate_code {
        12 => 1,
        13 | 14 => 2,
        _ => 0,
    };
    if crc8(&b[..at]) != b[at] {
        return None;
    }
    let sample = match head.block {
        Some(block) => n * block as u64,
        None => n,
    };
    (sample < head.total).then_some(sample)
}

fn crc8(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |mut c, &x| {
        c ^= x;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { c << 1 ^ 0x07 } else { c << 1 };
        }
        c
    })
}

/// Reads until `buf` is full or the source ends; returns the bytes read.
fn read_up_to(source: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match source.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// The song from a frame on, read as its stream info and then the frames: symphonia opens it as a song
/// that starts there.
pub(crate) struct Landed {
    start: Vec<u8>,
    inner: Box<dyn MediaSource>,
    /// Where the frame is in `inner`, and `inner`'s length.
    frame: u64,
    len: u64,
    pos: u64,
}

impl Landed {
    pub(crate) fn new(inner: Box<dyn MediaSource>, head: &Head, frame: u64, len: u64) -> Landed {
        let mut start = b"fLaC\x80\x00\x00\x22".to_vec();
        start.extend_from_slice(&head.info);
        Landed { start, inner, frame, len, pos: 0 }
    }
}

impl Read for Landed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let head = self.start.len() as u64;
        if self.pos < head {
            let b = &self.start[self.pos as usize..];
            let n = b.len().min(buf.len());
            buf[..n].copy_from_slice(&b[..n]);
            self.pos += n as u64;
            if self.pos == head {
                self.inner.seek(SeekFrom::Start(self.frame))?;
            }
            return Ok(n);
        }
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Landed {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let len = self.byte_len().expect("known") as i128;
        let at = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
            SeekFrom::End(d) => len + d as i128,
        };
        let at = u64::try_from(at).map_err(|_| io::Error::other("seek before the start"))?;
        let head = self.start.len() as u64;
        if at >= head {
            self.inner.seek(SeekFrom::Start(self.frame + at - head))?;
        }
        self.pos = at;
        Ok(at)
    }
}

impl MediaSource for Landed {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.start.len() as u64 + self.len - self.frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::testing::flac;

    struct Counted(Cursor<Vec<u8>>, u32);

    impl Read for Counted {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Seek for Counted {
        fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
            let before = self.0.position();
            let at = self.0.seek(to)?;
            self.1 += (at != before) as u32;
            Ok(at)
        }
    }

    impl MediaSource for Counted {
        fn is_seekable(&self) -> bool {
            true
        }

        fn byte_len(&self) -> Option<u64> {
            Some(self.0.get_ref().len() as u64)
        }
    }

    /// Stereo samples: silent for `quiet` seconds, then a tone.
    fn song(rate: u32, quiet: f64, loud: f64) -> Vec<i16> {
        let n = |s: f64| (s * rate as f64) as usize;
        (0..n(quiet) + n(loud)).flat_map(|i| {
            let v = if i < n(quiet) { 0 } else { ((i as f64 * 0.05).sin() * 8000.0) as i16 + (i % 7) as i16 };
            [v, v]
        }).collect()
    }

    #[test]
    fn reads_the_head() {
        let f = flac(44_100, &song(44_100, 0.0, 2.0), None);
        let h = head(&mut Counted(Cursor::new(f), 0)).unwrap().unwrap();
        assert_eq!((h.rate, h.channels, h.total, h.block, h.first), (44_100, 2, 88_200, Some(4096), 42));
        assert!(head(&mut Counted(Cursor::new(b"RIFF....".to_vec()), 0)).unwrap().is_none());
    }

    #[test]
    fn lands_on_the_frame_of_the_time() {
        // Half silence (tiny frames), half tone: the bitrate's guess misses, the second lands.
        let rate = 8_000;
        for (points, quiet) in [(None, 30.0), (None, 0.0), (Some(10), 30.0)] {
            let f = flac(rate, &song(rate, quiet, 60.0 - quiet), points);
            let len = f.len() as u64;
            let mut s = Counted(Cursor::new(f), 0);
            let h = head(&mut s).unwrap().unwrap();
            for secs in [1u64, 31, 45, 59] {
                let ts = secs * rate as u64;
                s.1 = 0;
                let (at, sample) = land(&mut s, &h, len, ts).unwrap().unwrap_or_else(|| panic!("{secs} s lands, {points:?} {quiet}"));
                assert!(sample <= ts && ts - sample <= rate as u64 * CLOSE_MS / 1000, "{secs} s: frame of {sample}");
                assert!(s.1 <= 2, "{secs} s: {} reads at other places", s.1);
                // The frame is where it was found: its header parses there.
                let mut b = [0u8; MAX_HEADER];
                s.0.seek(SeekFrom::Start(at)).unwrap();
                s.0.read_exact(&mut b).unwrap();
                assert_eq!(frame_header(&b, &h), Some(sample));
            }
        }
    }

    #[test]
    fn a_header_with_a_wrong_crc_is_not_a_frame() {
        let f = flac(8_000, &song(8_000, 0.0, 2.0), None);
        let h = head(&mut Counted(Cursor::new(f.clone()), 0)).unwrap().unwrap();
        let at = h.first as usize;
        let mut b = f[at..at + MAX_HEADER].to_vec();
        assert_eq!(frame_header(&b, &h), Some(0));
        b[4] ^= 1;
        assert_eq!(frame_header(&b, &h), None);
    }
}
