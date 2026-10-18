//! A live video's audio. YouTube serves it as numbered pieces of about five seconds, each a whole
//! fragmented MP4 (`&sq=<n>`; asked before it is made, a piece is answered once it is). Their AAC frames
//! are written out as ADTS, which plays as one endless stream, as an AAC internet radio station does.

/// The URL of piece `n` of the live stream at `url`; the newest piece when None.
pub fn piece_url(url: &str, n: Option<u64>) -> String {
    match n {
        Some(n) => format!("{url}&sq={n}"),
        None => url.to_string(),
    }
}

/// Appends piece `piece`'s AAC frames to `out` as ADTS; returns the piece's number. None when it is not a
/// live piece of AAC audio.
pub fn adts(piece: &[u8], out: &mut Vec<u8>) -> Option<u64> {
    let mut config = None;
    let mut number = None;
    let mut frames: Vec<(usize, usize)> = Vec::new();
    for (kind, start, body) in boxes(piece, 0) {
        match &kind {
            b"moov" => config = audio_config(&piece[body.clone()]),
            b"emsg" => number = number.or_else(|| sequence(&piece[body])),
            b"moof" => frames.extend(samples(piece, start, body)),
            _ => {}
        }
    }
    let config = config?;
    for (at, len) in frames {
        let frame = piece.get(at..at + len)?;
        out.extend_from_slice(&config.header(frame.len()));
        out.extend_from_slice(frame);
    }
    number
}

/// The boxes in `data` from `from`: type, start, and body range.
fn boxes(data: &[u8], from: usize) -> impl Iterator<Item = ([u8; 4], usize, std::ops::Range<usize>)> + '_ {
    let mut at = from;
    std::iter::from_fn(move || {
        let head = data.get(at..at + 8)?;
        let kind: [u8; 4] = head[4..8].try_into().ok()?;
        let (size, skip) = match u32::from_be_bytes(head[..4].try_into().ok()?) {
            0 => (data.len() - at, 8),
            1 => (usize::try_from(u64::from_be_bytes(data.get(at + 8..at + 16)?.try_into().ok()?)).ok()?, 16),
            n => (n as usize, 8),
        };
        if size < skip || at + size > data.len() {
            return None;
        }
        let item = (kind, at, at + skip..at + size);
        at += size;
        Some(item)
    })
}

/// The body of the first box `kind` in `data`.
fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data, 0).find(|(k, ..)| k == kind).map(|(_, _, body)| &data[body])
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// What an ADTS header carries of an AAC track's AudioSpecificConfig.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Config {
    object: u8,
    rate_index: u8,
    channels: u8,
}

impl Config {
    /// The 7-byte header of a frame of `len` bytes.
    fn header(&self, len: usize) -> [u8; 7] {
        let n = len + 7;
        [
            0xFF,
            0xF1,
            ((self.object - 1) << 6) | (self.rate_index << 2) | (self.channels >> 2),
            ((self.channels & 3) << 6) | ((n >> 11) & 3) as u8,
            (n >> 3) as u8,
            (((n & 7) as u8) << 5) | 0x1F,
            0xFC,
        ]
    }
}

/// The AAC track's config in `moov`'s body: trak/mdia/minf/stbl/stsd/mp4a/esds.
fn audio_config(moov: &[u8]) -> Option<Config> {
    let stbl = [b"trak", b"mdia", b"minf", b"stbl"].iter().try_fold(moov, |d, k| child(d, k))?;
    // stsd: version and flags, entry count, then the entries; mp4a: 28 bytes before its boxes.
    let mp4a = child(child(stbl, b"stsd")?.get(8..)?, b"mp4a")?;
    let esds = child(mp4a.get(28..)?, b"esds")?;
    let asc = descriptor(esds.get(4..)?, 0x05)?;
    let (object, rate_index) = (asc.first()? >> 3, ((asc.first()? & 7) << 1) | (asc.get(1)? >> 7));
    let channels = (asc.get(1)? >> 3) & 15;
    // ADTS has two bits for the profile and no escaped rate.
    ((1..=4).contains(&object) && rate_index < 13).then_some(Config { object, rate_index, channels })
}

/// The body of descriptor `tag` in an ES descriptor's tree.
fn descriptor(mut data: &[u8], tag: u8) -> Option<&[u8]> {
    loop {
        let t = *data.first()?;
        let mut len = 0usize;
        let mut at = 1;
        loop {
            let b = *data.get(at)?;
            len = (len << 7) | (b & 0x7F) as usize;
            at += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
        let body = data.get(at..at + len)?;
        if t == tag {
            return Some(body);
        }
        data = match t {
            // ES_ID, then flags saying what follows them.
            0x03 => {
                let flags = *body.get(2)?;
                let skip = 3 + if flags & 0x80 != 0 { 2 } else { 0 } + if flags & 0x40 != 0 { 1 + *body.get(3)? as usize } else { 0 } + if flags & 0x20 != 0 { 2 } else { 0 };
                body.get(skip..)?
            }
            // Object type, stream type, buffer size, bit rates.
            0x04 => body.get(13..)?,
            _ => data.get(at + len..)?,
        };
    }
}

/// The piece's sequence number in an `emsg` body ("Sequence-Number: 25355").
fn sequence(emsg: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(emsg);
    let rest = &text[text.find("Sequence-Number: ")? + "Sequence-Number: ".len()..];
    rest[..rest.find(|c: char| !c.is_ascii_digit())?].parse().ok()
}

/// Where each sample of a `moof` (at `start`, its body `body`) lies in `data`: start and length.
fn samples(data: &[u8], start: usize, body: std::ops::Range<usize>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (kind, _, traf) in boxes(data, body.start).take_while(|(_, s, _)| *s < body.end) {
        if &kind != b"traf" {
            continue;
        }
        let traf = &data[traf];
        let (Some(tfhd), Some(trun)) = (child(traf, b"tfhd"), child(traf, b"trun")) else { continue };
        let Some(tf) = be32(tfhd, 0) else { continue };
        let mut at = 8;
        let base = if tf & 1 != 0 {
            let b = tfhd.get(at..at + 8).and_then(|b| b.try_into().ok()).map(u64::from_be_bytes);
            at += 8;
            b.map_or(start, |b| b as usize)
        } else {
            start
        };
        at += if tf & 2 != 0 { 4 } else { 0 } + if tf & 8 != 0 { 4 } else { 0 };
        let default_size = if tf & 0x10 != 0 { be32(tfhd, at) } else { None };
        let (Some(tr), Some(count)) = (be32(trun, 0), be32(trun, 4)) else { continue };
        let mut at = 8;
        let mut offset = base;
        if tr & 1 != 0 {
            let Some(o) = be32(trun, at) else { continue };
            offset = (base as i64 + o as i32 as i64) as usize;
            at += 4;
        }
        at += if tr & 4 != 0 { 4 } else { 0 };
        for _ in 0..count {
            at += if tr & 0x100 != 0 { 4 } else { 0 };
            let size = if tr & 0x200 != 0 {
                let s = be32(trun, at);
                at += 4;
                s
            } else {
                default_size
            };
            at += if tr & 0x400 != 0 { 4 } else { 0 } + if tr & 0x800 != 0 { 4 } else { 0 };
            let Some(size) = size else { break };
            out.push((offset, size as usize));
            offset += size as usize;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live piece as YouTube sends one, made by hand: AAC-LC at 44.1 kHz in stereo, number 41, two
    /// fragments holding the frames "first" and "second!".
    fn piece() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/live_piece.mp4")).unwrap()
    }

    #[test]
    fn a_piece_becomes_adts_frames() {
        let mut out = Vec::new();
        assert_eq!(adts(&piece(), &mut out), Some(41));
        // LC (profile 1), 44.1 kHz (index 4), two channels; each header's length counts itself.
        let header = |n: usize| Config { object: 2, rate_index: 4, channels: 2 }.header(n);
        assert_eq!(header(5)[..4], [0xFF, 0xF1, 0x50, 0x80]);
        assert_eq!(out, [header(5).as_slice(), b"first", &header(7), b"second!"].concat());
    }

    #[test]
    fn not_a_live_piece() {
        let mut out = Vec::new();
        let mut no_number = piece();
        let at = no_number.windows(4).position(|w| w == b"Sequ").unwrap();
        no_number[at] = b'X';
        assert_eq!(adts(&no_number, &mut out), None);
        assert_eq!(adts(b"<html>sorry</html>", &mut out), None);
    }

    #[test]
    fn pieces_are_asked_by_number() {
        assert_eq!(piece_url("https://v/videoplayback?a=1", None), "https://v/videoplayback?a=1");
        assert_eq!(piece_url("https://v/videoplayback?a=1", Some(7)), "https://v/videoplayback?a=1&sq=7");
    }
}
