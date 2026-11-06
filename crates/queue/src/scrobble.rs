//! Play counting. Listening time is summed from play/pause edges and judged when the song is left (no
//! timer). The song left is recorded in the local history in the background; the platform is told what
//! to send the server.

use std::sync::{Arc, Weak};

use nori_db as db;
use nori_db::background;
use nori_model::{Listen, Song};
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::{queue, Session};

#[derive(Default)]
pub(crate) struct Scrobbler {
    current: Option<Current>,
    incoming: Option<Listen>,
    /// Monotonic ms playback last started; None while paused.
    playing_since: Option<i64>,
}

/// The song playing, kept from its start: the queue may be replaced before it is left.
struct Current {
    song: Song,
    /// The database of the profile it started in.
    profile: Weak<Mutex<Connection>>,
    /// When it started, wall clock ms.
    started_at: i64,
    heard_ms: i64,
}

impl Scrobbler {
    fn edge(&mut self, playing: bool, now: i64) {
        match (playing, self.playing_since) {
            (true, None) => self.playing_since = Some(now),
            (false, Some(since)) => {
                if let Some(c) = &mut self.current {
                    c.heard_ms += now - since;
                }
                self.playing_since = None;
            }
            _ => {}
        }
    }

    /// Moves to `next`, started in `profile` at `wall_ms` unless it is the listen handed here; the song left.
    fn switch(&mut self, next: Option<Song>, profile: Weak<Mutex<Connection>>, playing: bool, now_ms: i64, wall_ms: i64) -> Option<Current> {
        self.edge(false, now_ms);
        let incoming = self.incoming.take();
        let next = next.map(|song| {
            let handed = incoming.filter(|l| l.song_id == song.id);
            Current { started_at: handed.as_ref().map_or(wall_ms, |l| l.started_ms), heard_ms: handed.map_or(0, |l| l.heard_ms.max(0)), song, profile }
        });
        let done = std::mem::replace(&mut self.current, next);
        self.edge(playing, now_ms);
        done
    }
}

/// Where plays go besides the profile's server (plugins' scrobblers). They count by the same rule,
/// whatever the server's scrobbling switch and the profile's rules: their accounts are their own.
pub trait Scrobbles: Send + Sync {
    fn now_playing(&self, song: &Song);
    /// `song`, started at `at_ms` (wall clock), was heard long enough to count.
    fn scrobble(&self, song: &Song, at_ms: i64);
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
        let c = s.current.take()?;
        Some(Listen { song_id: c.song.id, started_ms: c.started_at, heard_ms: c.heard_ms })
    }

    /// The next song edge adopts the handed listen after accounting for the song it replaces.
    pub fn receive_listen(&self, listen: Option<Listen>) {
        self.scrobbler.lock().incoming = listen;
    }

    /// Plays go to `to` too from now on; None stops it.
    pub fn scrobble_to(&self, to: Option<Arc<dyn Scrobbles>>) {
        *self.scrobbles.lock() = to;
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
        let open = self.db.weak();
        let done = self.scrobbler.lock().switch(song, open.clone(), playing, now_ms, wall_ms);
        // The profile the song started in, not one opened since or while the write waits.
        if taste_model {
            if let Some((c, db)) = done.as_ref().and_then(|c| Some((c, c.profile.upgrade()?))) {
                let (song, at, heard) = (c.song.clone(), c.started_at, c.heard_ms);
                background::run(move || {
                    let _ = nori_library::history::record(&mut db.lock(), &song, at, heard, tz_offset_ms, db::now_ms());
                });
            }
        }
        let counted = done.filter(|c| Weak::ptr_eq(&c.profile, &open) && c.heard_ms >= needed_ms(c.song.duration as i64));
        if let Some(to) = self.scrobbles.lock().clone() {
            if let Some(c) = &counted {
                to.scrobble(&c.song, c.started_at);
            }
            if let Some(s) = next.as_deref().and_then(|id| self.song(id)) {
                to.now_playing(&s);
            }
        }
        if !scrobble {
            return ScrobbleSend { submit_id: None, submit_at: 0, now_playing_id: None };
        }
        let submit_at = counted.as_ref().map_or(0, |c| c.started_at);
        ScrobbleSend { submit_id: counted.map(|c| c.song.id), submit_at, now_playing_id: next }
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
        s.switch(Some(song.clone()), Weak::new(), true, 1_000, 50);
        s.edge(true, 2_000);
        s.edge(false, 5_000);
        s.edge(false, 9_000);
        s.edge(true, 10_000);
        let done = s.switch(None, Weak::new(), false, 12_000, 60).unwrap();
        assert_eq!((done.song, done.heard_ms, done.started_at), (song, 6_000, 50));
    }

    /// Notes what it is told: "now <id>" and "play <id> <at>".
    #[derive(Default)]
    struct Told(Mutex<Vec<String>>);

    impl Scrobbles for Told {
        fn now_playing(&self, song: &Song) {
            self.0.lock().push(format!("now {}", song.id));
        }
        fn scrobble(&self, song: &Song, at_ms: i64) {
            self.0.lock().push(format!("play {} {at_ms}", song.id));
        }
    }

    #[test]
    fn plays_go_to_a_sink_by_the_same_rule_whatever_the_server_switch() {
        let (_dir, session) = crate::playlist::tests::opened(|p| p.scrobble = false);
        let song = |id: &str| Song { id: id.into(), duration: 100, ..Default::default() };
        session.register(vec![song("a"), song("b"), song("c")]);
        let told = Arc::new(Told::default());
        session.scrobble_to(Some(told.clone()));
        session.scrobble_track(Some("a".into()), TrackChange::Moved, true, 0, 1_000, 0);
        // Heard 20 s of 100: not half.
        session.scrobble_track(Some("b".into()), TrackChange::Moved, true, 20_000, 21_000, 0);
        // Heard 60 s: counts, with its start.
        let sent = session.scrobble_track(Some("c".into()), TrackChange::Moved, true, 80_000, 81_000, 0);
        assert_eq!(sent.submit_id, None, "the server's switch is off");
        assert_eq!(*told.0.lock(), ["now a", "now b", "play b 21000", "now c"]);
        session.scrobble_to(None);
        session.scrobble_track(None, TrackChange::Ended, false, 200_000, 201_000, 0);
        assert_eq!(told.0.lock().len(), 4, "stopped");
    }

}
