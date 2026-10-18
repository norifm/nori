//! Play counting. Listening time is summed from play/pause edges and judged when the song is left (no
//! timer). The song left is recorded in the local history in the background; the platform is told what
//! to send the server.

use nori_db as db;
use nori_db::background;
use nori_model::{Listen, Song};

use crate::{queue, Session};

#[derive(Default)]
pub(crate) struct Scrobbler {
    /// The current song, kept from its start: the queue may be replaced before it is left.
    song: Option<Song>,
    incoming: Option<Listen>,
    started_at: i64,
    heard_ms: i64,
    /// Monotonic ms playback last started; None while paused.
    playing_since: Option<i64>,
}

impl Scrobbler {
    fn edge(&mut self, playing: bool, now: i64) {
        match (playing, self.playing_since) {
            (true, None) => self.playing_since = Some(now),
            (false, Some(since)) => {
                self.heard_ms += now - since;
                self.playing_since = None;
            }
            _ => {}
        }
    }

    /// Moves to `next`; returns the song left, how long it was heard and when it started.
    fn switch(&mut self, next: Option<Song>, playing: bool, now_ms: i64, wall_ms: i64) -> (Option<Song>, i64, i64) {
        self.edge(false, now_ms);
        let done = std::mem::replace(&mut self.song, next);
        let heard = std::mem::take(&mut self.heard_ms);
        let at = std::mem::replace(&mut self.started_at, wall_ms);
        if let Some(listen) = self.incoming.take().filter(|l| self.song.as_ref().is_some_and(|s| s.id == l.song_id)) {
            self.started_at = listen.started_ms;
            self.heard_ms = listen.heard_ms.max(0);
        }
        self.edge(playing, now_ms);
        (done, heard, at)
    }
}

/// What to tell the server when a song is left.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ScrobbleSend {
    /// The song left, if heard long enough to count, and when it started.
    pub submit_id: Option<String>,
    pub submit_at: i64,
    /// For the server's "now playing".
    pub now_playing_id: Option<String>,
}

/// Listening needed for a play to count, Last.fm's rule: half of `duration_s` or 4 minutes, whichever
/// comes first, and at least 10 s.
pub(crate) fn needed_ms(duration_s: i64) -> i64 {
    (duration_s * 500).clamp(10_000, 240_000)
}

/// Why the player's song changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum TrackChange {
    /// Onto another song, or none.
    Moved,
    /// The same song again under repeat one: a separate play.
    Looped,
    Ended,
}

/// The id to follow after a change: none once ended; a radio stream only when it loops.
fn followed(id: Option<String>, why: TrackChange) -> Option<String> {
    match why {
        TrackChange::Ended => None,
        TrackChange::Looped => id,
        TrackChange::Moved => id.filter(|i| !queue::is_radio(i)),
    }
}

impl Session {
    /// Relinquishes this listen before the old player is paused.
    pub fn hand_listen(&self, now_ms: i64) -> Option<Listen> {
        let mut s = self.scrobbler.lock();
        s.edge(false, now_ms);
        let song = s.song.take()?;
        Some(Listen { song_id: song.id, started_ms: s.started_at, heard_ms: std::mem::take(&mut s.heard_ms) })
    }

    /// The next song edge adopts the handed listen after accounting for the song it replaces.
    pub fn receive_listen(&self, listen: Option<Listen>) {
        self.scrobbler.lock().incoming = listen;
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Session {
    /// Playback started or stopped at `now_ms` (monotonic).
    pub fn scrobble_playing(&self, playing: bool, now_ms: i64) {
        self.scrobbler.lock().edge(playing, now_ms);
    }

    /// The player's song changed to `id`. Records the song left in the history when the taste model is
    /// on, and returns what to scrobble when scrobbling is on.
    pub fn scrobble_track(&self, id: Option<String>, why: TrackChange, playing: bool, now_ms: i64, wall_ms: i64, tz_offset_ms: i32) -> ScrobbleSend {
        let (taste_model, scrobble) = self.settings.prefs(|p| (p.taste_model, p.scrobble));
        let next = followed(id, why);
        let song = next.as_deref().and_then(|id| self.song(id));
        let (done, heard, at) = self.scrobbler.lock().switch(song, playing, now_ms, wall_ms);
        // The profile playing now, not the one open when the write runs.
        if let (Some(song), true, Some(db)) = (done.clone(), taste_model, self.db.get()) {
            background::run(move || {
                let _ = nori_library::history::record(&mut db.lock(), &song, at, heard, tz_offset_ms, db::now_ms());
            });
        }
        if !scrobble {
            return ScrobbleSend { submit_id: None, submit_at: 0, now_playing_id: None };
        }
        let submit_id = done.filter(|s| heard >= needed_ms(s.duration as i64)).map(|s| s.id);
        ScrobbleSend { submit_id, submit_at: at, now_playing_id: next }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrobble_rules() {
        assert_eq!(needed_ms(200), 100_000, "half");
        assert_eq!(needed_ms(600), 240_000, "four minutes");
        assert_eq!(needed_ms(10), 10_000);

        // Followed ids.
        let id = |s: &str| Some(s.to_string());
        assert_eq!(followed(id("s1"), TrackChange::Moved), id("s1"));
        assert_eq!(followed(id("radio:4"), TrackChange::Moved), None);
        assert_eq!(followed(id("radio:4"), TrackChange::Looped), id("radio:4"));
        assert_eq!(followed(id("s1"), TrackChange::Ended), None);

        // Heard time sums play edges only.
        let mut s = Scrobbler::default();
        let song = Song::only_id("a".into());
        s.switch(Some(song.clone()), true, 1_000, 50);
        s.edge(true, 2_000);
        s.edge(false, 5_000);
        s.edge(false, 9_000);
        s.edge(true, 10_000);
        let (done, heard, at) = s.switch(None, false, 12_000, 60);
        assert_eq!((done, heard, at), (Some(song), 6_000, 50));
    }

}
