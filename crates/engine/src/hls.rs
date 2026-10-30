//! A provider's stream served as an HLS media playlist (SoundCloud's): its segments read one after
//! another as one stream, from any byte. Segments in the clear, whole files: packed audio (MP3 or AAC
//! frames), Ogg pages, or MP4 fragments after their header (`#EXT-X-MAP`), which reads as a fragmented MP4
//! file. A playlist with a key, of byte ranges or of other playlists is refused. Each segment's leading ID3
//! tag (HLS's timestamp) is left out, so the stream is what a file of them would hold.
//!
//! Segments are fetched outside any cache ([`ByteSource::open_plain`]): each is a URL of its own, while a
//! platform's cache keeps one stream per song.

use std::io::{self, Read};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::source::{ByteSource, Cancel, OpenError};

/// How an HLS playlist starts.
pub(crate) const MARK: &[u8] = b"#EXTM3U";
/// The largest playlist read: a few thousand segments.
const MOST: u64 = 1 << 20;

/// A media playlist's segments, and the size of each once read.
pub(crate) struct Playlist {
    segments: Vec<String>,
    sizes: Mutex<Vec<Option<u64>>>,
}

impl Playlist {
    /// The playlist in `text`, served at `url` (relative segment addresses are taken from there).
    pub(crate) fn parse(text: &str, url: &str) -> Result<Playlist, String> {
        let mut segments = Vec::new();
        let mut ends = false;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(tag) = line.strip_prefix('#') {
                if tag.starts_with("EXT-X-STREAM-INF") {
                    return Err("a playlist of playlists".into());
                }
                if let Some(map) = tag.strip_prefix("EXT-X-MAP:") {
                    let uri = map.strip_prefix("URI=\"").and_then(|u| u.split_once('"')).filter(|(_, rest)| rest.is_empty());
                    match uri {
                        Some((uri, _)) if segments.is_empty() => segments.push(address(uri, url)),
                        _ => return Err("an MP4 header that changes or is a byte range".into()),
                    }
                }
                if tag.starts_with("EXT-X-BYTERANGE") {
                    return Err("byte ranges".into());
                }
                if tag.starts_with("EXT-X-KEY") && !tag.contains("METHOD=NONE") {
                    return Err("encrypted".into());
                }
                ends |= tag == "EXT-X-ENDLIST" || tag == "EXT-X-PLAYLIST-TYPE:VOD";
            } else {
                segments.push(address(line, url));
            }
        }
        if !ends {
            return Err("a live playlist".into());
        }
        if segments.is_empty() {
            return Err("no segments".into());
        }
        let sizes = Mutex::new(vec![None; segments.len()]);
        Ok(Playlist { segments, sizes })
    }

    /// The stream's length, once every segment has been read.
    pub(crate) fn len(&self) -> Option<u64> {
        self.sizes.lock().iter().copied().sum()
    }

    /// The segment to start at for byte `from`, and the byte it starts at: the last one whose start is
    /// known at or before `from`.
    fn start_for(&self, from: u64) -> (usize, u64) {
        let sizes = self.sizes.lock();
        let mut start = 0;
        for (k, size) in sizes.iter().enumerate() {
            match size {
                Some(s) if start + s <= from => start += s,
                _ => return (k, start),
            }
        }
        (sizes.len(), start)
    }
}

/// `line`, a segment's address in the playlist at `url`, as a URL. Some servers (SoundCloud's) escape
/// `&` as HTML does.
fn address(line: &str, url: &str) -> String {
    let line = line.replace("&amp;", "&");
    if line.contains("://") {
        return line;
    }
    let path_end = url.find('?').unwrap_or(url.len());
    let origin_end = url.find("://").map_or(0, |s| url[s + 3..path_end].find('/').map_or(path_end, |p| s + 3 + p));
    match line.strip_prefix('/') {
        Some(_) => format!("{}{line}", &url[..origin_end]),
        None => format!("{}/{line}", &url[..url[..path_end].rfind('/').filter(|&i| i >= origin_end).unwrap_or(origin_end)]),
    }
}

/// Reads the rest of a playlist whose first bytes `head` were read from `reader`.
pub(crate) fn read_playlist(head: &[u8], reader: impl Read, url: &str) -> Result<Playlist, String> {
    let mut text = head.to_vec();
    reader.take(MOST).read_to_end(&mut text).map_err(|e| e.to_string())?;
    Playlist::parse(&String::from_utf8_lossy(&text), url)
}

/// What a segment refused on the way (an expired URL) calls, so the next open resolves the stream anew.
pub(crate) type Refused = Box<dyn Fn() + Send>;

/// `playlist`'s stream from byte `from`, a segment at a time.
pub(crate) struct Segments {
    bytes: Arc<dyn ByteSource>,
    playlist: Arc<Playlist>,
    cancel: Cancel,
    refused: Refused,
    /// The segment being read, its body (its ID3 tag passed), and the bytes read of it.
    segment: usize,
    body: Option<Box<dyn Read + Send>>,
    read: u64,
    /// Bytes still to pass over before `from`.
    skip: u64,
}

impl Segments {
    pub(crate) fn new(bytes: Arc<dyn ByteSource>, playlist: Arc<Playlist>, from: u64, cancel: Cancel, refused: Refused) -> Segments {
        let (segment, start) = playlist.start_for(from);
        Segments { bytes, playlist, cancel, refused, segment, body: None, read: 0, skip: from - start }
    }

    fn open(&mut self) -> io::Result<()> {
        let body = self.bytes.open_plain(&self.playlist.segments[self.segment], 0, &self.cancel).map_err(|e| {
            if matches!(e, OpenError::Status(403)) {
                (self.refused)();
            }
            io::Error::other(format!("segment {}: {e}", self.segment))
        })?;
        self.body = Some(past_id3(body.reader)?);
        self.read = 0;
        Ok(())
    }

    /// The open segment ended: its size is kept, and it must be the size it was before.
    fn ended(&mut self) -> io::Result<()> {
        let mut sizes = self.playlist.sizes.lock();
        match sizes[self.segment] {
            Some(size) if size != self.read => return Err(io::Error::other(format!("segment {} changed: {} bytes, {size} before", self.segment, self.read))),
            _ => sizes[self.segment] = Some(self.read),
        }
        self.segment += 1;
        self.body = None;
        Ok(())
    }
}

impl Read for Segments {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.segment == self.playlist.segments.len() || buf.is_empty() {
                return Ok(0);
            }
            if self.body.is_none() {
                self.open()?;
            }
            let n = self.body.as_mut().expect("opened").read(buf)?;
            if n == 0 {
                self.ended()?;
                continue;
            }
            self.read += n as u64;
            let passed = self.skip.min(n as u64) as usize;
            self.skip -= passed as u64;
            if passed < n {
                buf.copy_within(passed..n, 0);
                return Ok(n - passed);
            }
        }
    }
}

/// `reader` past an ID3v2 tag at its start, if any.
fn past_id3(mut reader: Box<dyn Read + Send>) -> io::Result<Box<dyn Read + Send>> {
    let mut head = [0u8; 10];
    let mut got = 0;
    while got < head.len() {
        match reader.read(&mut head[got..])? {
            0 => break,
            n => got += n,
        }
    }
    if got < head.len() || &head[..3] != b"ID3" {
        return Ok(Box::new(io::Cursor::new(head[..got].to_vec()).chain(reader)));
    }
    let size = head[6..10].iter().fold(0u64, |s, &b| s << 7 | (b & 0x7f) as u64);
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    io::copy(&mut (&mut reader).take(size + footer), &mut io::sink())?;
    Ok(reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_addresses() {
        let url = "https://cdn.test/a/b/playlist.m3u8?sig=1";
        let cases = [
            ("https://other.test/x.mp3?a=1&amp;b=2", "https://other.test/x.mp3?a=1&b=2"),
            ("seg0.mp3", "https://cdn.test/a/b/seg0.mp3"),
            ("/root/seg0.mp3", "https://cdn.test/root/seg0.mp3"),
        ];
        for (line, want) in cases {
            assert_eq!(address(line, url), want, "{line}");
        }
        assert_eq!(address("seg.mp3", "https://cdn.test/p.m3u8"), "https://cdn.test/seg.mp3");
    }

    #[test]
    fn what_a_playlist_may_be() {
        let vod = "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:10,\na.mp3\n#EXTINF:5,\nb.mp3\n#EXT-X-ENDLIST\n";
        assert_eq!(Playlist::parse(vod, "https://h/p.m3u8").unwrap().segments, ["https://h/a.mp3", "https://h/b.mp3"]);
        let mp4 = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\na.m4s\n#EXT-X-ENDLIST";
        assert_eq!(Playlist::parse(mp4, "https://h/p.m3u8").unwrap().segments, ["https://h/init.mp4", "https://h/a.m4s"], "the header comes first");
        let refused = [
            ("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nlow.m3u8\n#EXT-X-ENDLIST", "a playlist of playlists"),
            ("#EXTM3U\n#EXT-X-MAP:URI=\"i.mp4\"\n#EXTINF:10,\na.m4s\n#EXT-X-MAP:URI=\"j.mp4\"\n#EXTINF:10,\nb.m4s\n#EXT-X-ENDLIST", "an MP4 header that changes or is a byte range"),
            ("#EXTM3U\n#EXT-X-MAP:URI=\"i.mp4\",BYTERANGE=\"800@0\"\n#EXTINF:10,\na.m4s\n#EXT-X-ENDLIST", "an MP4 header that changes or is a byte range"),
            ("#EXTM3U\n#EXTINF:10,\n#EXT-X-BYTERANGE:500@0\na.mp3\n#EXT-X-ENDLIST", "byte ranges"),
            ("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:10,\na.mp3\n#EXT-X-ENDLIST", "encrypted"),
            ("#EXTM3U\n#EXTINF:10,\na.mp3\n", "a live playlist"),
            ("#EXTM3U\n#EXT-X-ENDLIST\n", "no segments"),
        ];
        for (text, why) in refused {
            assert_eq!(Playlist::parse(text, "https://h/p.m3u8").err().as_deref(), Some(why));
        }
        let clear = "#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:10,\na.mp3\n#EXT-X-ENDLIST";
        assert!(Playlist::parse(clear, "https://h/p.m3u8").is_ok());
    }
}
