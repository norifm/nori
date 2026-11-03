//! A streamed song whose bytes the client's own cache also wrote to disk (media3's on Android): once it
//! has arrived whole, it is read from those files and its memory copy goes, unless they hold other bytes.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use common::Virtual;
use nori_engine::{Body, ByteSource, Fetching, Loader, OpenError, Waits};

use crate::common;

const LEN: usize = 300_000;

/// 100 bytes/ms for a 3 s song: one burst brings it all.
const LOAD: [i64; 5] = [1_000, 4_000, 0, 0, 1 << 30];

fn song() -> Vec<u8> {
    (0..LEN).map(|i| (i * 7 % 251) as u8).collect()
}

/// Serves the song promising `over` bytes past its real end (a transcode's estimate); its cache holds
/// the song in `files`.
struct Cached {
    song: Vec<u8>,
    over: u64,
    files: Vec<PathBuf>,
    opens: AtomicU32,
    asked: AtomicU32,
}

impl ByteSource for Cached {
    fn open(&self, _: &str, from: u64) -> Result<Body, OpenError> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        let rest = self.song[from as usize..].to_vec();
        Ok(Body { start: from, len: Some(LEN as u64 + self.over), reader: Box::new(std::io::Cursor::new(rest)) })
    }

    fn files(&self, _: &str) -> Option<Vec<PathBuf>> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Some(self.files.clone())
    }
}

#[test]
fn a_whole_song_moves_to_its_cache_files_when_they_hold_its_bytes() {
    let dir = nori_testdir::TempDir::new("on-disk");
    let song = song();
    let piece = |name: &str, bytes: &[u8]| {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    };
    let (a, b) = (piece("a", &song[..200_000]), piece("b", &song[200_000..]));
    let short = piece("short", &song[200_000..LEN - 1_000]);
    let long = piece("long", &[&song[200_000..], &[0u8; 500][..]].concat());
    // (what, the cache's files, bytes promised past the real end, read from the files)
    let cases = [
        ("whole in two pieces", vec![a.clone(), b.clone()], 0, true),
        ("an estimated length past what came", vec![a.clone(), b.clone()], 10_000, true),
        ("a piece evicted", vec![a.clone(), dir.path().join("evicted")], 0, false),
        ("short", vec![a.clone(), short], 0, false),
        ("longer than what came", vec![a.clone(), long], 0, false),
    ];
    for (what, files, over, from_files) in cases {
        let source = Arc::new(Cached { song: song.clone(), over, files, opens: AtomicU32::new(0), asked: AtomicU32::new(0) });
        let fetching = Arc::new(Fetching::timed(Virtual::default()));
        let l = Loader::start_within(&fetching, source.clone(), "song".into(), LOAD, Some(3_000), None, None, None, Waits::default());
        let mut r = l.reader();
        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        assert!(all == song, "{what}: the song's bytes ({} of {LEN})", all.len());
        // Waits for the loader thread to settle.
        assert!(l.complete(), "{what}");
        assert_eq!(source.asked.load(Ordering::Relaxed), 1, "{what}: the files asked for once");
        let held = fetching.held();
        assert_eq!((held.on_disk == 1, l.held() == 0, held.bytes == 0), (from_files, from_files, from_files), "{what}: {held:?}");
        let mut again = l.reader();
        again.seek(SeekFrom::Start(199_990)).unwrap();
        let mut across = [0u8; 20];
        again.read_exact(&mut across).unwrap();
        assert_eq!(across, song[199_990..200_010], "{what}: read across the pieces");
        again.seek(SeekFrom::Start(LEN as u64 - 10)).unwrap();
        let mut end = Vec::new();
        again.read_to_end(&mut end).unwrap();
        assert_eq!(end, song[LEN - 10..], "{what}: to the end");
        assert_eq!(source.opens.load(Ordering::Relaxed), 1, "{what}: fetched once");
    }
}

/// Serves `song` whole from any byte.
struct Server {
    song: Vec<u8>,
    opens: AtomicU32,
}

impl ByteSource for Server {
    fn open(&self, _: &str, from: u64) -> Result<Body, OpenError> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        let rest = self.song[from as usize..].to_vec();
        Ok(Body { start: from, len: Some(self.song.len() as u64), reader: Box::new(std::io::Cursor::new(rest)) })
    }
}

#[test]
fn a_song_over_its_cap_is_read_from_its_cache_entry_once_it_has_all_come() {
    let dir = nori_testdir::TempDir::new("on-disk-cap");
    let store = nori_engine::store::Store::open(dir.path(), 1 << 30).unwrap();
    // 6 MB over a minute, a 2 MB cap: three bursts at least, the memory copy never whole.
    let song: Vec<u8> = (0..6_000_000).map(|i| (i * 7 % 251) as u8).collect();
    let load = [5_000, 20_000, 0, 0, 2_000_000];
    let source = Arc::new(Server { song: song.clone(), opens: AtomicU32::new(0) });
    let fetching = Arc::new(Fetching::timed(Virtual::default()));
    let entry = store.clone();
    let keep: nori_engine::source::Keep = Box::new(move || entry.writer("song:0"));
    let l = Loader::start_within(&fetching, source.clone(), "song".into(), load, Some(60_000), Some(keep), None, None, Waits::default());
    let mut all = Vec::new();
    l.reader().read_to_end(&mut all).unwrap();
    assert!(all == song, "the song's bytes ({} of {})", all.len(), song.len());
    assert!(l.complete());
    let held = fetching.held();
    assert_eq!((held.on_disk, l.held(), held.bytes), (1, 0, 0), "read from the entry, nothing in memory: {held:?}");
    let opens = source.opens.load(Ordering::Relaxed);
    let mut again = l.reader();
    let mut start = [0u8; 100];
    again.read_exact(&mut start).unwrap();
    assert_eq!(start, song[..100], "the start, read again");
    assert_eq!(source.opens.load(Ordering::Relaxed), opens, "from the entry, not fetched again");
}
