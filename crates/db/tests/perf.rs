//! Host perf of the library index: a large library indexed, synced again unchanged and with some songs
//! changed, searched and pruned. Not a check; prints one "perf:" line per step.
//!
//!   cargo test --release -p nori-db --test perf -- --ignored --nocapture
//!   python3 tools/flame.py -- <test binary> --ignored --nocapture      (a flamegraph of it)

use std::collections::HashSet;
use std::time::Instant;

use nori_model::{Album, Artist, Song};

const SONGS: usize = 30_000;
const ALBUMS: usize = 3_000;
const ARTISTS: usize = 1_000;

fn library(changed_every: Option<usize>) -> (Vec<Artist>, Vec<Album>, Vec<Song>) {
    let words = ["blue", "night", "river", "fire", "glass", "ocean", "paper", "summer", "winter", "light", "echo", "gold"];
    let word = |k: usize| words[k % words.len()];
    let artists = (0..ARTISTS).map(|a| Artist { id: format!("ar{a}"), name: format!("{} {} {a}", word(a), word(a / 7)), ..Artist::default() }).collect();
    let albums = (0..ALBUMS)
        .map(|a| Album { id: format!("al{a}"), name: format!("{} {} {a}", word(a / 3), word(a)), artist: format!("artist {}", a % ARTISTS), artist_id: Some(format!("ar{}", a % ARTISTS)), ..Album::default() })
        .collect();
    let songs = (0..SONGS)
        .map(|s| {
            let plays = changed_every.filter(|e| s % e == 0).map_or(0, |_| 1);
            Song {
                id: format!("so{s}"),
                title: format!("{} {} {} {s}", word(s), word(s / 5), word(s / 11)),
                album: format!("album {}", s % ALBUMS),
                artist: format!("artist {}", s % ARTISTS),
                album_id: Some(format!("al{}", s % ALBUMS)),
                artist_id: Some(format!("ar{}", s % ARTISTS)),
                duration: 180 + (s % 120) as u32,
                track: (s % 12) as u32 + 1,
                year: 1970 + (s % 50) as u32,
                genre: Some(word(s / 3).to_string()),
                suffix: "flac".into(),
                play_count: plays,
                ..Song::default()
            }
        })
        .collect();
    (artists, albums, songs)
}

fn step(name: &str, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    println!("perf: {name:<18} {:8.1} ms", t.elapsed().as_secs_f64() * 1000.0);
}

#[test]
#[ignore]
fn index_perf() {
    let dir = nori_testdir::TempDir::new("db-perf");
    let path = dir.join("perf.db").to_string_lossy().into_owned();
    let mut c = nori_db::open(&path, "s").unwrap();
    let (artists, albums, songs) = library(None);
    let mut kept = Vec::new();
    step("index", || kept = nori_db::index(&mut c, &artists, &albums, &songs).unwrap());
    step("sync unchanged", || kept = nori_db::index(&mut c, &artists, &albums, &songs).unwrap());
    let (artists, albums, songs) = library(Some(20));
    step("sync 5% changed", || kept = nori_db::index(&mut c, &artists, &albums, &songs).unwrap());
    step("search x100", || {
        for q in ["blu", "night riv", "gold 12", "ocean", "pa", "summer light", "echo fire 3", "wi", "glass", "river 2"].iter().cycle().take(100) {
            nori_db::search(&c, q, 20).unwrap();
        }
    });
    let half: HashSet<i64> = kept.iter().copied().step_by(2).collect();
    step("prune half", || nori_db::prune(&mut c, &half).unwrap());
}
