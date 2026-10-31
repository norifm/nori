//! A plugin's moving cover, a muted loop in the player's sleeve: fetched once, its H.264 frames decoded on
//! a thread of its own while the sleeve shows and sent to the UI at their own pace. An HLS playlist in
//! fragmented MP4 (a master's lightest H.264 variant that is sharp enough, whole files or byte ranges), or
//! one such MP4 file. What isn't that keeps the poster. Nothing touches the audio.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nori_covers::Image;
use openh264::decoder::{Decoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;
use openh264::OpenH264API;

/// A variant at least this tall is sharp enough for the sleeve.
const ENOUGH_HEIGHT: u32 = 720;
/// The most a loop may weigh, bytes: past it the poster stays.
const MOST_BYTES: u64 = 64 << 20;
/// How long a frame shows when the file says no durations, ms.
const FALLBACK_FRAME_MS: u64 = 33;

#[derive(Debug, PartialEq)]
pub enum Failed {
    /// The server answered this error: the plugin hears it and can look the loop up again.
    Status(u16),
    /// Nothing here can play it.
    Unplayable(String),
}

/// A segment's place: its URL and, with a byte range, where in it.
#[derive(Debug, Clone, PartialEq)]
struct Part {
    url: String,
    range: Option<(u64, u64)>,
}

/// What a media playlist lists.
#[derive(Debug, Default, PartialEq)]
struct Media {
    init: Option<Part>,
    segments: Vec<Part>,
}

/// `to` against the playlist at `base`.
fn resolved(base: &str, to: &str) -> String {
    if to.contains("://") {
        return to.to_string();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("http", base));
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let dir = path.rsplit_once('/').map_or("", |d| d.0);
    match (to.strip_prefix('/'), dir.is_empty()) {
        (Some(whole), _) => format!("{scheme}://{host}/{whole}"),
        (None, true) => format!("{scheme}://{host}/{to}"),
        (None, false) => format!("{scheme}://{host}/{dir}/{to}"),
    }
}

/// An attribute of an HLS tag's list, its quotes off.
fn attribute<'a>(list: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = list;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, next) = match after.strip_prefix('"') {
            Some(quoted) => {
                let (value, tail) = quoted.split_once('"')?;
                (value, tail.strip_prefix(',').unwrap_or(tail))
            }
            None => after.split_once(',').unwrap_or((after, "")),
        };
        if key.trim() == name {
            return Some(value);
        }
        rest = next;
    }
    None
}

/// What a playlist says of its variants.
#[derive(Debug, PartialEq)]
enum Variant {
    /// It lists none: it is the media playlist.
    Itself,
    /// The lightest H.264 one as tall as [`ENOUGH_HEIGHT`], else the tallest.
    At(String),
    NoneToPlay,
}

fn variant(text: &str, base: &str) -> Variant {
    let mut lines = text.lines();
    let mut found: Vec<(u32, u64, String)> = Vec::new();
    let mut any = false;
    while let Some(line) = lines.next() {
        let Some(list) = line.strip_prefix("#EXT-X-STREAM-INF:") else { continue };
        any = true;
        let uri = lines.next().unwrap_or_default().trim();
        if !attribute(list, "CODECS").is_some_and(|c| c.split(',').any(|c| c.trim().starts_with("avc1"))) {
            continue;
        }
        let height = attribute(list, "RESOLUTION").and_then(|r| r.split_once('x')).and_then(|r| r.1.parse().ok()).unwrap_or(0);
        let bandwidth = attribute(list, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(u64::MAX);
        found.push((height, bandwidth, resolved(base, uri)));
    }
    if !any {
        return Variant::Itself;
    }
    let enough = found.iter().filter(|v| v.0 >= ENOUGH_HEIGHT).min_by_key(|v| v.1);
    enough.or(found.iter().max_by_key(|v| v.0)).map_or(Variant::NoneToPlay, |v| Variant::At(v.2.clone()))
}

/// `length[@offset]`, the offset after `last` when it says none.
fn byte_range(text: &str, last: u64) -> Option<(u64, u64)> {
    let (length, at) = text.split_once('@').map_or((text, None), |(l, a)| (l, Some(a)));
    Some((at.map_or(Some(last), |a| a.parse().ok())?, length.parse().ok()?))
}

fn media(text: &str, base: &str) -> Result<Media, Failed> {
    let mut out = Media::default();
    let mut range = None;
    let mut last = 0;
    for line in text.lines().map(str::trim) {
        if let Some(key) = line.strip_prefix("#EXT-X-KEY:") {
            if attribute(key, "METHOD") != Some("NONE") {
                return Err(Failed::Unplayable("encrypted".into()));
            }
        } else if let Some(map) = line.strip_prefix("#EXT-X-MAP:") {
            let url = resolved(base, attribute(map, "URI").unwrap_or_default());
            let at = attribute(map, "BYTERANGE").and_then(|r| byte_range(r, 0));
            out.init = Some(Part { url, range: at });
        } else if let Some(r) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = byte_range(r, last);
        } else if !line.is_empty() && !line.starts_with('#') {
            last = range.map_or(0, |(at, length)| at + length);
            out.segments.push(Part { url: resolved(base, line), range: range.take() });
        }
    }
    Ok(out)
}

/// The boxes of `b`: type, contents, and where each starts.
fn boxes(b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8], usize)> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let head = b.get(at..at + 8)?;
        let size = u32::from_be_bytes(head[..4].try_into().ok()?) as usize;
        let kind = <[u8; 4]>::try_from(&head[4..8]).ok()?;
        let (skip, size) = match size {
            0 => (8, b.len() - at),
            1 => (16, u64::from_be_bytes(b.get(at + 8..at + 16)?.try_into().ok()?) as usize),
            n => (8, n),
        };
        let start = at;
        let body = b.get(at + skip..at + size)?;
        at += size.max(8);
        Some((kind, body, start))
    })
}

fn child<'a>(b: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(b).find(|x| &x.0 == kind).map(|x| x.1)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// The video track's decoder setup.
#[derive(Debug, PartialEq)]
struct Track {
    /// SPS and PPS with their start codes.
    setup: Vec<u8>,
    /// Bytes of a NAL unit's length in a sample.
    length_size: usize,
    timescale: u32,
    default_duration: u32,
    default_size: u32,
}

/// `count` length-prefixed NAL units of an `avcC` from `at`, into `setup` with start codes.
fn units(avcc: &[u8], mut at: usize, count: usize, setup: &mut Vec<u8>) -> Option<usize> {
    for _ in 0..count {
        let n = be16(avcc, at)? as usize;
        setup.extend_from_slice(&[0, 0, 0, 1]);
        setup.extend_from_slice(avcc.get(at + 2..at + 2 + n)?);
        at += 2 + n;
    }
    Some(at)
}

/// The H.264 video track of an MP4 header.
fn track(file: &[u8]) -> Option<Track> {
    let moov = child(file, b"moov")?;
    let trex = child(moov, b"mvex").and_then(|m| child(m, b"trex"));
    let default_duration = trex.and_then(|t| be32(t, 12)).unwrap_or(0);
    let default_size = trex.and_then(|t| be32(t, 16)).unwrap_or(0);
    boxes(moov).filter(|b| &b.0 == b"trak").find_map(|t| {
        let mdia = child(t.1, b"mdia")?;
        let mdhd = child(mdia, b"mdhd")?;
        let timescale = if mdhd.first() == Some(&1) { be32(mdhd, 20)? } else { be32(mdhd, 12)? };
        let stsd = child(child(child(mdia, b"minf")?, b"stbl")?, b"stsd")?;
        // The sample entry after the full box's head and its count; its own fields take 78 bytes.
        let avc1 = child(stsd.get(8..)?, b"avc1")?;
        let avcc = child(avc1.get(78..)?, b"avcC")?;
        let length_size = (*avcc.get(4)? as usize & 3) + 1;
        let mut setup = Vec::new();
        let at = units(avcc, 6, *avcc.get(5)? as usize & 31, &mut setup)?;
        units(avcc, at + 1, *avcc.get(at)? as usize, &mut setup)?;
        Some(Track { setup, length_size, timescale, default_duration, default_size })
    })
}

/// A coded frame: where in the file, and for how long it shows.
#[derive(Debug, PartialEq, Clone, Copy)]
struct Sample {
    at: usize,
    size: usize,
    ms: u64,
}

/// The samples of the fragments of `file`.
fn samples(file: &[u8], t: &Track) -> Vec<Sample> {
    let mut out = Vec::new();
    for (kind, moof, start) in boxes(file) {
        if &kind != b"moof" {
            continue;
        }
        let Some(traf) = child(moof, b"traf") else { continue };
        let (Some(tfhd), Some(trun)) = (child(traf, b"tfhd"), child(traf, b"trun")) else { continue };
        let flags = be32(tfhd, 0).unwrap_or(0) & 0xFF_FFFF;
        let mut at = 8;
        at += if flags & 1 != 0 { 8 } else { 0 } + if flags & 2 != 0 { 4 } else { 0 };
        let mut duration = t.default_duration;
        if flags & 8 != 0 {
            duration = be32(tfhd, at).unwrap_or(duration);
            at += 4;
        }
        let size = if flags & 0x10 != 0 { be32(tfhd, at).unwrap_or(t.default_size) } else { t.default_size };
        let run = be32(trun, 0).unwrap_or(0) & 0xFF_FFFF;
        let count = be32(trun, 4).unwrap_or(0);
        let mut read = 8;
        let mut data = start;
        if run & 1 != 0 {
            data = (start as i64 + be32(trun, read).unwrap_or(0) as i32 as i64) as usize;
            read += 4;
        }
        read += if run & 4 != 0 { 4 } else { 0 };
        for _ in 0..count {
            let (mut d, mut s) = (duration, size);
            if run & 0x100 != 0 {
                d = be32(trun, read).unwrap_or(d);
                read += 4;
            }
            if run & 0x200 != 0 {
                s = be32(trun, read).unwrap_or(s);
                read += 4;
            }
            read += if run & 0x400 != 0 { 4 } else { 0 } + if run & 0x800 != 0 { 4 } else { 0 };
            let ms = if t.timescale > 0 && d > 0 { d as u64 * 1000 / t.timescale as u64 } else { FALLBACK_FRAME_MS };
            out.push(Sample { at: data, size: s as usize, ms });
            data += s as usize;
        }
    }
    out
}

/// A sample's NAL units with start codes, into `out`.
fn annex_b(sample: &[u8], length_size: usize, out: &mut Vec<u8>) {
    out.clear();
    let mut at = 0;
    while let Some(head) = sample.get(at..at + length_size) {
        let n = head.iter().fold(0usize, |n, b| n << 8 | *b as usize);
        let Some(unit) = sample.get(at + length_size..at + length_size + n) else { return };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(unit);
        at += length_size + n;
    }
}

/// A fetched loop, ready to decode again each time the sleeve shows.
pub struct Video {
    file: Vec<u8>,
    track: Track,
    samples: Vec<Sample>,
}

fn fetch(part: &Part) -> Result<Vec<u8>, Failed> {
    let mut ask = ureq::get(&part.url);
    if let Some((at, length)) = part.range {
        ask = ask.header("Range", format!("bytes={at}-{}", at + length - 1));
    }
    let mut got = ask.call().map_err(|e| match e {
        ureq::Error::StatusCode(status) => Failed::Status(status),
        e => Failed::Unplayable(e.to_string()),
    })?;
    got.body_mut().with_config().limit(MOST_BYTES).read_to_vec().map_err(|e| Failed::Unplayable(e.to_string()))
}

fn load(url: &str) -> Result<Video, Failed> {
    let plain = |url: &str| Part { url: url.to_string(), range: None };
    let first = fetch(&plain(url))?;
    let file = if first.starts_with(b"#EXTM3U") {
        let text = String::from_utf8_lossy(&first).into_owned();
        let (text, base) = match variant(&text, url) {
            Variant::Itself => (text, url.to_string()),
            Variant::At(at) => (String::from_utf8_lossy(&fetch(&plain(&at))?).into_owned(), at),
            Variant::NoneToPlay => return Err(Failed::Unplayable("no H.264 variant".into())),
        };
        let list = media(&text, &base)?;
        let mut file = Vec::new();
        for part in list.init.iter().chain(&list.segments) {
            file.extend(fetch(part)?);
            if file.len() as u64 > MOST_BYTES {
                return Err(Failed::Unplayable("too long".into()));
            }
        }
        file
    } else {
        first
    };
    let track = track(&file).ok_or_else(|| Failed::Unplayable("no H.264 track".into()))?;
    let samples = samples(&file, &track);
    if samples.is_empty() {
        return Err(Failed::Unplayable("not fragmented".into()));
    }
    Ok(Video { file, track, samples })
}

/// A decoder that keeps frames to reorder them: flushing after each sample breaks on B-frames.
fn decoder() -> Result<Decoder, openh264::Error> {
    Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().flush_after_decode(Flush::NoFlush))
}

/// Decodes `video` over and over until `stop`, handing each frame to `show` when it is due. A frame the
/// UI hasn't taken in time (`busy`) is left out.
fn run(video: &Video, stop: &AtomicBool, busy: &AtomicBool, show: &(dyn Fn(Image) + Send + Sync)) -> Result<(), Failed> {
    let failed = |e: openh264::Error| Failed::Unplayable(e.to_string());
    let mut unit = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let mut decoder = decoder().map_err(failed)?;
        decoder.decode(&video.track.setup).map_err(failed)?;
        let mut due = Instant::now();
        for s in &video.samples {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let Some(sample) = video.file.get(s.at..s.at + s.size) else { continue };
            annex_b(sample, video.track.length_size, &mut unit);
            let frame = decoder.decode(&unit).map_err(failed)?;
            due += Duration::from_millis(s.ms);
            let Some(frame) = frame else { continue };
            if Instant::now() > due || busy.load(Ordering::Relaxed) {
                continue;
            }
            let (width, height) = frame.dimensions();
            let mut pixels = vec![0; width * height * 4].into_boxed_slice();
            frame.write_rgba8(&mut pixels);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
            busy.store(true, Ordering::Relaxed);
            show(Image { width: width as u32, height: height as u32, pixels });
        }
    }
    Ok(())
}

/// A loop playing; dropped, it stops.
pub struct Playing {
    stop: Arc<AtomicBool>,
    /// A frame is on its way to the UI.
    pub busy: Arc<AtomicBool>,
}

impl Drop for Playing {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The video once fetched, kept for the song.
pub type Kept = Arc<Mutex<Option<Arc<Video>>>>;

/// Plays the loop at `url`: frames to `show`, a failure to `failed`.
pub fn play(url: String, kept: Kept, show: impl Fn(Image) + Send + Sync + 'static, failed: impl Fn(Failed) + Send + 'static) -> Playing {
    let (stop, busy) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let (halt, wait) = (stop.clone(), busy.clone());
    nori_host::spawn("nori-motion", move || {
        let have = kept.lock().expect("not poisoned").clone();
        let video = match have.map_or_else(|| load(&url).map(Arc::new), Ok) {
            Ok(v) => v,
            Err(e) => return failed(e),
        };
        *kept.lock().expect("not poisoned") = Some(video.clone());
        if let Err(e) = run(&video, &halt, &wait, &show) {
            failed(e);
        }
    });
    Playing { stop, busy }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn a_relative_address_goes_by_its_playlist() {
        for (to, want) in [("b.mp4", "https://h/a/b.mp4"), ("/d.mp4", "https://h/d.mp4"), ("https://x/e", "https://x/e")] {
            assert_eq!(resolved("https://h/a/p.m3u8?t=1", to), want);
        }
    }

    #[test]
    fn a_master_gives_the_lightest_sharp_h264() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900,RESOLUTION=540x540,CODECS=\"avc1.640020\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1000,RESOLUTION=720x720,CODECS=\"hvc1.2.4.L123.B0\"\nhevc.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000,RESOLUTION=1080x1080,CODECS=\"avc1.640028,mp4a.40.2\"\nhigh.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1500,RESOLUTION=720x720,CODECS=\"avc1.64001f\"\nmid.m3u8\n";
        assert_eq!(variant(master, "https://h/m.m3u8"), Variant::At("https://h/mid.m3u8".into()));
        let small = "#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=480x480,CODECS=\"avc1.1\"\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2,RESOLUTION=540x540,CODECS=\"avc1.1\"\nb.m3u8\n";
        assert_eq!(variant(small, "https://h/m.m3u8"), Variant::At("https://h/b.m3u8".into()), "the tallest there is");
        let hevc = "#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"hvc1.1\"\na.m3u8\n";
        assert_eq!(variant(hevc, "https://h/m.m3u8"), Variant::NoneToPlay, "the poster stays");
        assert_eq!(variant("#EXTM3U\n#EXTINF:1,\ns.m4s\n", "https://h/m.m3u8"), Variant::Itself);
    }

    #[test]
    fn a_media_playlist_gives_its_parts_in_order() {
        let list = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"800@0\"\n#EXTINF:6,\n#EXT-X-BYTERANGE:100@800\nv.mp4\n#EXTINF:6,\n#EXT-X-BYTERANGE:50\nv.mp4\n#EXTINF:6,\nw.m4s\n";
        let m = media(list, "https://h/a/p.m3u8").unwrap();
        assert_eq!(m.init, Some(Part { url: "https://h/a/init.mp4".into(), range: Some((0, 800)) }));
        let at: Vec<_> = m.segments.iter().map(|p| (p.url.as_str(), p.range)).collect();
        assert_eq!(at, [("https://h/a/v.mp4", Some((800, 100))), ("https://h/a/v.mp4", Some((900, 50))), ("https://h/a/w.m4s", None)]);
        assert!(matches!(media("#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n", "https://h/p"), Err(Failed::Unplayable(_))), "encrypted");
    }

    #[test]
    fn a_fragment_gives_its_samples_where_they_are() {
        let t = Track { setup: Vec::new(), length_size: 4, timescale: 1000, default_duration: 40, default_size: 0 };
        // tfhd: default-base-is-moof, no fields; trun: a data offset and each sample's size.
        let tfhd = boxed(b"tfhd", &[0, 2, 0, 0, 0, 0, 0, 1]);
        let trun = |offset: u32| [&[0, 0, 2, 1, 0, 0, 0, 2][..], &offset.to_be_bytes(), &5u32.to_be_bytes(), &7u32.to_be_bytes()].concat();
        let moof = |offset| boxed(b"moof", &boxed(b"traf", &[tfhd.clone(), boxed(b"trun", &trun(offset))].concat()));
        let offset = moof(0).len() as u32 + 8;
        let file = [boxed(b"ftyp", &[0; 8]), moof(offset), boxed(b"mdat", &[0; 12])].concat();
        let start = 16 + offset as usize;
        assert_eq!(samples(&file, &t), [Sample { at: start, size: 5, ms: 40 }, Sample { at: start + 5, size: 7, ms: 40 }]);
    }

    #[test]
    fn a_sample_becomes_units_with_start_codes() {
        let mut out = Vec::new();
        annex_b(&[0, 0, 0, 2, 9, 8, 0, 0, 0, 1, 7], 4, &mut out);
        assert_eq!(out, [0, 0, 0, 1, 9, 8, 0, 0, 0, 1, 7]);
        annex_b(&[0, 0, 0, 9, 1], 4, &mut out);
        assert_eq!(out, Vec::<u8>::new(), "a cut unit is left out");
    }
}
