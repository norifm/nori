//! The scrobbler point as nori's tests check it: what it is told of a song whose id starts "fixture"
//! is a notice ("now" with the id, "play" with the id and start); any other song is let be.

use nori_plugin_sdk::notices::{show, Tone};
use nori_plugin_sdk::types::Song;
use nori_plugin_sdk::Scrobbler;

use crate::Test;

impl Scrobbler for Test {
    fn now_playing(&mut self, song: Song) {
        if song.id.starts_with("fixture") {
            show(Tone::Info, "now", &[song.id]);
        }
    }

    fn scrobble(&mut self, song: Song, at_ms: i64) {
        if song.id.starts_with("fixture") {
            show(Tone::Info, "play", &[song.id, at_ms.to_string()]);
        }
    }
}
