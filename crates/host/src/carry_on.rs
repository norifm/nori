//! Going on after a session was cut off: a web player worker that crashed opens again over the saved
//! queue, which may be older than what was heard.

use super::Session;

impl Session {
    /// The session before this one stopped `position_ms` into `song`, playing or not: when the queue
    /// opened here stands on that song, it goes on from there as it was.
    pub fn carry_on(&self, song: &str, position_ms: i64, playing: bool) {
        let Some(at) = self.core.session.playlist(|p| p.current().filter(|&i| p.ids()[i] == song)) else { return };
        if playing {
            self.engine.play_at(at, position_ms);
        } else {
            self.engine.go_to(at, position_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use nori_engine::testing::{sine, wav, Stepper, Virtual};
    use nori_engine::State;

    use super::super::test_card::Card;
    use super::super::*;

    #[test]
    fn a_session_cut_off_goes_on_where_it_was() {
        for (song, playing) in [("b", true), ("b", false), ("c", true)] {
            let dir = nori_testdir::TempDir::new("host-carry-on");
            let prefs = nori_core::settings_store::Settings::new();
            let mut p = prefs.open(&dir.join("app.db").to_string_lossy()).unwrap();
            (p.remote_control, p.auto_fill, p.auto_mix, p.third_party_lookups) = (false, None, false, false);
            prefs.put(p);
            let queue = Arc::new(nori_core::queue::Session::new(prefs));
            let card = Card::new();
            let clock = Virtual::default();
            let time = Stepper::new(clock.clone(), card.pull.clone());
            let profile = SavedServer { id: "test".into(), user: "ann".into(), ..Default::default() };
            let s = Session::open_on(Open { offline: true, ..test_open(queue, dir.path(), profile, Box::new(card.clone())) }, clock).unwrap();
            let songs: Vec<Song> = ["a", "b", "c"].map(|id| Song { id: id.into(), duration: 4, suffix: "wav".into(), ..Default::default() }).into();
            s.core.download_queue(songs.clone()).unwrap();
            s.core.download_settle(songs.iter().map(|s| s.id.clone()).collect(), vec![true; songs.len()]).unwrap();
            for song in &songs {
                let path = s.store.as_ref().expect("a store").download_path(&song.id);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, wav(44_100, &sine(44_100, 440.0, 4.0, 16_000.0))).unwrap();
            }
            s.core.session.register(songs.clone());
            s.core.session.set(songs.iter().map(|s| s.id.clone()).collect(), Some(1), false, None);
            s.engine.queue_changed();
            s.engine.go_to(1, 0);
            time.clock.settle();
            let before = s.engine.status().state;

            s.carry_on(song, 2_500, playing);
            let (want, from) = match (song, playing) {
                ("b", true) => (State::Playing, 2_500),
                ("b", false) => (before, 2_500),
                _ => (before, 0),
            };
            let at = |st: &nori_engine::Status| st.index == Some(1) && st.state == want && st.position_ms >= from && st.position_ms < from + 1_000;
            assert!(time.until(Duration::from_secs(2), || at(&s.engine.status())), "{song} {playing}: {:?} at {}", s.engine.status().state, s.engine.status().position_ms);
            s.close();
        }
    }
}
