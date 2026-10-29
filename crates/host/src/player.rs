//! What the player shows: this device's playback, the account's active device while it is another one,
//! or the jam a guest is in; the same for every client and for the system's media controls.

use std::collections::HashMap;

use nori_core::playlist::PlaylistView;
use nori_core::stage::QueueRows;
use nori_core::Song;
use nori_engine::State;
use nori_time::Instant;

use crate::remote::Elsewhere;
use crate::session::Handle;

/// Whose playback the player shows.
#[derive(Clone)]
pub enum Playback {
    /// This device's engine.
    Here,
    /// The account's active device while it is another one; the controls reach it.
    There(Elsewhere),
    /// The jam this guest is in; None until the host's state is in.
    Jam(Option<Elsewhere>),
}

impl Playback {
    /// The device or jam host shown in place of this device's own playback.
    pub fn mirrored(&self) -> Option<&Elsewhere> {
        match self {
            Playback::There(e) | Playback::Jam(Some(e)) => Some(e),
            Playback::Here | Playback::Jam(None) => None,
        }
    }
}

/// What the player shows playing, read at one moment.
#[derive(Debug, Clone)]
pub struct Now {
    pub state: State,
    /// The play button shows playing: a jam guest's as its controls say (paused here while the jam plays
    /// on), else while the music plays.
    pub playing: bool,
    pub song: Option<Song>,
    /// The song's list index.
    pub index: Option<usize>,
    /// Where the song was at `at`; it runs on at `pace` while playing and not buffering.
    pub position_ms: i64,
    pub at: Instant,
    pub pace: f32,
    pub mixing: bool,
    pub buffering: bool,
    pub shuffle: bool,
    pub repeat: u8,
    /// Songs in the queue.
    pub len: u32,
    /// The volume of the device playing elsewhere, 0 to 1, when it can be set there.
    pub volume: Option<f32>,
    /// The name of the device playing elsewhere.
    pub device: Option<String>,
    /// A jam guest's: the jam's playback.
    pub jam: bool,
}

impl Now {
    /// Where the song is now.
    pub fn position_now(&self) -> i64 {
        let runs = self.state == State::Playing && !self.buffering;
        let pace = if runs { self.pace as f64 } else { 0.0 };
        nori_remote::run_on(self.position_ms, self.at.elapsed().as_secs_f64() * 1000.0, pace, self.song.as_ref().map_or(0, |s| s.duration))
    }

    /// The device or jam host's playback, as this player shows it.
    fn mirrored(e: &Elsewhere, jam: bool, playing: bool) -> Now {
        let m = &e.mirror;
        let row = e.row();
        Now {
            state: match (m.playing, row) {
                (true, _) => State::Playing,
                (false, Some(_)) => State::Paused,
                (false, None) => State::Idle,
            },
            playing,
            song: row.map(|r| r.song.clone()),
            index: row.and(m.at).map(|a| a as usize),
            position_ms: e.position_ms(),
            at: Instant::now(),
            pace: m.rate as f32,
            mixing: m.playing && m.mixing,
            buffering: m.playing && m.buffering,
            shuffle: m.shuffle,
            repeat: m.repeat,
            len: m.len,
            volume: e.volume(),
            device: (!jam).then(|| m.name.clone()),
            jam,
        }
    }

    /// A jam guest's player before the host's playback is in.
    fn jam_waiting() -> Now {
        Now {
            state: State::Idle,
            playing: false,
            song: None,
            index: None,
            position_ms: 0,
            at: Instant::now(),
            pace: 1.0,
            mixing: false,
            buffering: false,
            shuffle: false,
            repeat: 0,
            len: 0,
            volume: None,
            device: None,
            jam: true,
        }
    }
}

/// The queue the player shows.
#[derive(Debug, Clone)]
pub struct QueueShown {
    /// Each song at its list index, in play order (`order`).
    pub view: PlaylistView,
    /// Its rows in play order around the song heard: what has played, that song, what plays next
    /// (`Core::queue_rows`).
    pub rows: QueueRows,
    /// Who asked for each song that came in through the jam, by song id.
    pub by: HashMap<String, String>,
    /// Another device's queue (or the jam's), read only here but for a tap.
    pub mirrored: bool,
    pub jam: bool,
}

impl QueueShown {
    /// The list index of the song heard.
    pub fn heard(&self) -> Option<u32> {
        usize::try_from(self.rows.now).ok().and_then(|n| self.rows.order.get(n).copied())
    }

    /// The songs after the one heard, in play order, with their list indexes.
    pub fn upcoming(&self) -> impl Iterator<Item = (u32, &Song)> {
        let from = usize::try_from(self.rows.now).map_or(0, |n| n + 1);
        self.rows.order.get(from..).unwrap_or_default().iter().filter_map(|&i| Some((i, self.view.songs.get(i as usize)?)))
    }

    /// A mirrored queue: its rows are its play order already.
    fn mirrored(e: &Elsewhere, jam: bool, by: HashMap<String, String>) -> QueueShown {
        let view = e.view();
        let now = e.mirror.at.map_or(-1, |a| a as i32);
        let rows = QueueRows { order: view.order.clone(), reorderable: false, kept: view.order.clone(), now };
        QueueShown { view, rows, by, mirrored: true, jam }
    }
}

impl Handle {
    /// Whose playback the player shows.
    pub fn playback(&self) -> Playback {
        if self.guest {
            return Playback::Jam(self.jam_playing());
        }
        self.elsewhere().map_or(Playback::Here, Playback::There)
    }

    /// What the player shows playing.
    pub fn now(&self) -> Now {
        match self.playback() {
            Playback::There(e) => Now::mirrored(&e, false, e.mirror.playing),
            Playback::Jam(Some(e)) => {
                let controls = self.remote().and_then(|r| r.jam_controls());
                Now::mirrored(&e, true, controls.map_or(e.mirror.playing, |c| c.playing))
            }
            Playback::Jam(None) => Now::jam_waiting(),
            Playback::Here => self.now_here(),
        }
    }

    /// This device's playback.
    fn now_here(&self) -> Now {
        let (repeat, shuffle, len) = self.core.session.playlist(|p| (p.repeat(), p.lit(), p.len() as u32));
        let (now, id) = self.engine.status_with(|st| {
            let now = Now {
                state: st.state,
                playing: st.state == State::Playing,
                song: None,
                index: st.index,
                position_ms: st.position_ms,
                at: st.at,
                pace: st.pace,
                mixing: st.mixing,
                buffering: st.buffering,
                shuffle,
                repeat,
                len,
                volume: None,
                device: None,
                jam: false,
            };
            (now, st.id.clone())
        });
        Now { song: id.and_then(|id| self.core.session.song(&id)), ..now }
    }

    /// The queue the player shows. `before`, the view read last, lends its songs when they have not
    /// changed since.
    pub fn queue_shown(&self, before: Option<&PlaylistView>) -> QueueShown {
        let by = self.remote().map(|r| r.jam_added()).unwrap_or_default();
        match self.playback() {
            Playback::There(e) => QueueShown::mirrored(&e, false, by),
            Playback::Jam(Some(e)) => QueueShown::mirrored(&e, true, by),
            Playback::Jam(None) => {
                let view = PlaylistView { songs: Vec::new(), len: 0, list_rev: 0, order: Vec::new(), queued: Vec::new(), index: -1, shuffle: false, repeat: 0, bridging: false, rev: 0 };
                QueueShown { view, rows: nori_core::stage::mirrored_queue_rows(0, false, -1), by, mirrored: true, jam: true }
            }
            Playback::Here => {
                let mut view = self.core.session.view(before.map_or(u64::MAX, |v| v.list_rev));
                if view.songs.is_empty() && view.len > 0 {
                    view.songs = before.map(|b| b.songs.clone()).unwrap_or_default();
                }
                let rows = self.core.queue_rows(view.len, view.shuffle, self.heard_index(view.index));
                QueueShown { view, rows, by, mirrored: false, jam: false }
            }
        }
    }

    /// Whether `shown` is behind the queue: another device's or the jam's always may be; this device's
    /// once the queue or the song heard moved on.
    pub fn queue_stale(&self, shown: Option<&QueueShown>) -> bool {
        let Some(q) = shown.filter(|q| !q.mirrored && matches!(self.playback(), Playback::Here)) else { return true };
        let (rev, repeat, index) = self.core.session.playlist(|p| (p.rev(), p.repeat(), p.current().map_or(-1, |c| c as i32)));
        q.view.rev != rev || q.view.repeat != repeat || q.view.index != index || q.heard().map_or(-1, |i| i as i32) != self.heard_index(index)
    }

    /// The song heard, which during a mix is not yet the queue's current one (`current`).
    fn heard_index(&self, current: i32) -> i32 {
        self.engine.status_with(|s| s.index).map_or(current, |i| i as i32)
    }
}

/// The media keys and the system's media controls: the session's own controls, so they reach what the
/// player shows.
#[cfg(feature = "desktop")]
impl nori_mpris::Controls for Handle {
    fn play(&self) {
        self.pressed(nori_plugins::Press::Play);
        self.resume();
    }
    fn pause(&self) {
        self.pressed(nori_plugins::Press::Pause);
        self.halt();
    }
    fn toggle(&self) {
        self.pressed(nori_plugins::Press::PlayPause);
        Handle::toggle(self);
    }
    fn next(&self) {
        self.pressed(nori_plugins::Press::Next);
        Handle::next(self);
    }
    fn previous(&self) {
        self.pressed(nori_plugins::Press::Previous);
        Handle::previous(self);
    }
    fn seek(&self, ms: i64) {
        Handle::seek(self, ms);
    }
    fn like(&self) {
        let playback = self.playback();
        let there = playback.mirrored();
        let Some(song) = there.map_or_else(|| self.now_here().song, |e| e.song().cloned()) else { return };
        let on = !self.starred(&song, there);
        self.star_song(song.id, on, there);
    }
    fn now(&self) -> nori_mpris::Now {
        let now = Handle::now(self);
        let playback = self.playback();
        let song = now.song.clone().unwrap_or_default();
        let mut shown = nori_mpris::Now {
            playing: now.state == State::Playing,
            loaded: now.state == State::Paused,
            index: now.index,
            title: song.title.clone(),
            artist: song.artist.clone(),
            album: song.album.clone(),
            length_ms: song.duration as i64 * 1000,
            position_ms: now.position_now(),
            starred: now.song.as_ref().is_some_and(|s| self.starred(s, playback.mirrored())),
            art: None,
            art_file: None,
        };
        let cover = self.media.as_ref().zip(song.cover_art.as_deref()).and_then(|(m, id)| m.cover.of(id));
        if let Some(cover) = cover {
            (shown.art, shown.art_file) = (cover.bytes, cover.path);
        }
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mirrored queue runs in its play order around the song playing there, takes no edits, and keeps
    /// each song's list index there; with nothing playing, all of it is to come.
    #[test]
    fn a_mirrored_queue_is_shown_in_its_play_order() {
        let mut e = crate::remote::tests::desk();
        let q = QueueShown::mirrored(&e, false, HashMap::new());
        let upcoming: Vec<(u32, &str)> = q.upcoming().map(|(i, s)| (i, s.id.as_str())).collect();
        assert_eq!((q.heard(), upcoming), (Some(1), vec![(2, "c")]));
        assert_eq!((q.rows.order.clone(), q.rows.kept.clone(), q.rows.reorderable), (vec![0, 1, 2], vec![0, 1, 2], false));
        e.mirror.at = None;
        let q = QueueShown::mirrored(&e, false, HashMap::new());
        assert_eq!((q.heard(), q.upcoming().count()), (None, 3));
    }

    /// The device's playback as the player shows it: its song, place and volume, paused once it pauses.
    #[test]
    fn a_mirrored_device_plays_as_shown() {
        let mut e = crate::remote::tests::desk();
        let now = Now::mirrored(&e, false, true);
        assert_eq!((now.state, now.index, now.song.as_ref().map(|s| s.id.as_str()), now.volume, now.device.as_deref()), (State::Playing, Some(1), Some("b"), Some(0.4), Some("Desk")));
        e.mirror.playing = false;
        let now = Now::mirrored(&e, true, false);
        assert_eq!((now.state, now.device.as_deref(), now.jam, now.position_now()), (State::Paused, None, true, now.position_ms));
    }
}
