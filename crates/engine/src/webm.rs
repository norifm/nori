//! The `DiscardPadding` of a Matroska/WebM file's blocks: the frames an encoder padded the last packet
//! with (Opus pads to whole 20 ms packets). symphonia reads the element but drops it, so the file is
//! walked here, as `mp4.rs` does for MP4's gapless boxes. One audio track is assumed.

use std::io::{self, BufReader, Read, Seek, SeekFrom};

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const CODEC_DELAY: u32 = 0x56AA;
const CLUSTER: u32 = 0x1F43_B675;
const TIMESTAMP: u32 = 0xE7;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const DISCARD_PADDING: u32 = 0x75A2;

/// A block group larger than this is not audio's, and is skipped.
const LARGEST_GROUP: u64 = 1 << 20;

/// What a file says to discard, before it is matched to symphonia's packets ([`Discards::frames`]).
#[derive(Debug, Default, PartialEq)]
pub struct Discards {
    /// The track's `CodecDelay`, in nanoseconds: symphonia takes it off every packet's timestamp.
    codec_delay_ns: u64,
    /// Each block that says so: its timestamp in segment ticks (cluster's plus block's), and the
    /// nanoseconds to drop from its end.
    blocks: Vec<(i64, u64)>,
}

impl Discards {
    /// Each block's padding as (packet timestamp in the track's ticks, frames at `rate`), where a
    /// track tick is `numer / denom` seconds, the time base symphonia stamps packets in.
    pub fn frames(&self, numer: u32, denom: u32, rate: u32) -> Vec<(i64, u32)> {
        // symphonia's conversion of nanoseconds into ticks (`MatroskaTicks::into_track_ticks`).
        let per_tick = (1_000_000_000 * u64::from(numer) / u64::from(denom.max(1))).max(1);
        let delay = (self.codec_delay_ns / per_tick) as i64;
        self.blocks.iter().map(|&(ts, ns)| (ts - delay, (ns as u128 * u128::from(rate) / 1_000_000_000) as u32)).collect()
    }
}

/// The frames to drop from the end of the packet stamped `pts` ([`Discards::frames`]).
pub fn discarded(frames: &[(i64, u32)], pts: i64) -> usize {
    frames.iter().find(|&&(at, _)| at == pts).map_or(0, |&(_, n)| n as usize)
}

/// The paddings of `r`; None if it is not a Matroska file. Leaves `r`'s position anywhere.
pub fn discards(r: &mut (impl Read + Seek)) -> io::Result<Option<Discards>> {
    let len = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut r = BufReader::new(r);
    let mut pos = 0;
    let mut out = Discards::default();
    let mut cluster = 0i64;
    while pos < len {
        let Some((id, size, head)) = header(&mut r)? else { break };
        if pos == 0 && id != EBML {
            return Ok(None);
        }
        let body = pos + head;
        let next = match (id, size) {
            // Containers walked into; their size may be unknown.
            (SEGMENT | TRACKS | TRACK_ENTRY | CLUSTER, _) => body,
            (_, None) => break,
            (CODEC_DELAY | TIMESTAMP | BLOCK_GROUP, Some(size)) if size <= LARGEST_GROUP => {
                let mut data = vec![0; size as usize];
                r.read_exact(&mut data)?;
                match id {
                    CODEC_DELAY => out.codec_delay_ns = unsigned(&data),
                    TIMESTAMP => cluster = unsigned(&data) as i64,
                    _ => out.blocks.extend(group(&data, cluster)),
                }
                pos = body + size;
                continue;
            }
            (_, Some(size)) => body + size,
        };
        r.seek_relative((next - (pos + head)) as i64)?;
        pos = next;
    }
    Ok(Some(out))
}

/// A block group's block timestamp and padding, when it has both and the padding is positive.
fn group(mut data: &[u8], cluster: i64) -> Option<(i64, u64)> {
    let (mut at, mut padding) = (None, 0i64);
    while let Some((id, Some(size), _)) = header(&mut data).ok().flatten() {
        let Some(body) = data.get(..size as usize) else { break };
        match id {
            BLOCK => {
                let (_, track_len) = vint(body, false)?;
                let rel = body.get(track_len..track_len + 2)?;
                at = Some(cluster + i64::from(i16::from_be_bytes([rel[0], rel[1]])));
            }
            DISCARD_PADDING => padding = signed(body),
            _ => {}
        }
        data = &data[size as usize..];
    }
    Some((at?, u64::try_from(padding).ok().filter(|&p| p > 0)?))
}

/// An element's id, size (None: unknown) and header length; None at the end.
fn header(r: &mut impl Read) -> io::Result<Option<(u32, Option<u64>, u64)>> {
    let mut first = [0u8; 1];
    if r.read(&mut first)? == 0 {
        return Ok(None);
    }
    let id_len = first[0].leading_zeros() as usize + 1;
    if id_len > 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an EBML element id"));
    }
    let mut id = [0u8; 4];
    id[0] = first[0];
    r.read_exact(&mut id[1..id_len])?;
    let id = id[..id_len].iter().fold(0u32, |v, &b| v << 8 | u32::from(b));
    let mut size = [0u8; 8];
    r.read_exact(&mut size[..1])?;
    let size_len = size[0].leading_zeros() as usize + 1;
    if size_len > 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an EBML element size"));
    }
    r.read_exact(&mut size[1..size_len])?;
    let (value, _) = vint(&size[..size_len], true).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an EBML element size"))?;
    let unknown = value == (1u64 << (7 * size_len)) - 1;
    Ok(Some((id, (!unknown).then_some(value), (id_len + size_len) as u64)))
}

/// A variable-length integer at the start of `bytes`, its marker bit dropped (`sized`: a whole size
/// is given, else it is read from the first byte), and its length.
fn vint(bytes: &[u8], sized: bool) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || (sized && bytes.len() != len) {
        return None;
    }
    let rest = bytes.get(1..len)?;
    let mask = if len == 8 { 0 } else { 0xFFu8 >> len };
    Some((rest.iter().fold(u64::from(first & mask), |v, &b| v << 8 | u64::from(b)), len))
}

fn unsigned(bytes: &[u8]) -> u64 {
    bytes.iter().take(8).fold(0, |v, &b| v << 8 | u64::from(b))
}

fn signed(bytes: &[u8]) -> i64 {
    let Some(&first) = bytes.first() else { return 0 };
    let start = if first & 0x80 != 0 { -1i64 } else { 0 };
    bytes.iter().take(8).fold(start, |v, &b| v << 8 | i64::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_block_of_a_webm_opus_says_its_padding() {
        let mut file = io::Cursor::new(&include_bytes!("../testdata/tone440.webm")[..]);
        let d = discards(&mut file).unwrap().unwrap();
        assert!(d.codec_delay_ns > 0, "Opus's pre-skip: {d:?}");
        assert_eq!(d.blocks.len(), 1, "only the last block is padded: {d:?}");
        let frames = d.frames(1, 1000, 48_000);
        assert!(frames[0].1 > 0 && frames[0].1 < 960, "less than a 20 ms packet: {frames:?}");
    }

    #[test]
    fn other_files_have_none() {
        let mut file = io::Cursor::new(&include_bytes!("../../player/testdata/tone440.opus")[..]);
        assert_eq!(discards(&mut file).unwrap(), None);
    }

    #[test]
    fn integers_read_as_ebml_writes_them() {
        assert_eq!(vint(&[0x81], true), Some((1, 1)));
        assert_eq!(vint(&[0x40, 0x02], true), Some((2, 2)));
        assert_eq!(signed(&[0xFF, 0x38]), -200);
        assert_eq!(signed(&[0x01, 0x2C]), 300);
        assert_eq!(unsigned(&[0x0F, 0x42, 0x40]), 1_000_000);
    }
}
