//! Plugins as scrobble targets beside the profile's server: each enabled plugin with a `scrobbler`
//! export is told, on its own thread, what plays and what counted. Nothing waits for it.

use nori_model::Song;

use crate::convert::song_out;
use crate::Plugins;

impl Plugins {
    /// `song` started playing.
    pub fn scrobblers_now_playing(&self, song: &Song) {
        for id in self.extending("scrobbler") {
            if let Some(w) = self.worker(&id) {
                let s = song_out(song);
                w.post(move |st, p| p.nori_plugin_scrobbler().call_now_playing(st, &s));
            }
        }
    }

    /// `song`, started at `at_ms` (wall clock), counted as a play.
    pub fn scrobblers_scrobble(&self, song: &Song, at_ms: i64) {
        for id in self.extending("scrobbler") {
            if let Some(w) = self.worker(&id) {
                let s = song_out(song);
                w.post(move |st, p| p.nori_plugin_scrobbler().call_scrobble(st, &s, at_ms));
            }
        }
    }
}
