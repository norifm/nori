//! The lyrics point as nori's tests check it: the song titled "fixture lyrics" has two timed lines,
//! "fixture untimed" two untimed ones; any other has none.

use nori_plugin_sdk::lyrics::{Found, Line};
use nori_plugin_sdk::types::Song;
use nori_plugin_sdk::Lyrics;

use crate::Test;

impl Lyrics for Test {
    fn lookup(&mut self, song: Song) -> Option<Found> {
        let synced = match song.title.as_str() {
            "fixture lyrics" => true,
            "fixture untimed" => false,
            _ => return None,
        };
        let line = |start_ms, words: &str| Line { start_ms, words: words.into() };
        Some(Found { synced, lines: vec![line(1_000, "first fixture line"), line(5_000, "second fixture line")] })
    }
}
