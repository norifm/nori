//! An open session as its plugins reach it: their controls go through the session's own (so the remote
//! and jam rules apply, as for a press), and the engine's events become the player's moments the plugin
//! host tells its hooks of.

use nori_engine::{Event, State};
use nori_plugins::{Control, Moment, Repeat};

use crate::session::Handle;

impl Handle {
    /// Does what a plugin asked, as the client's own control would.
    pub fn plugin_control(&self, c: Control) {
        match c {
            Control::Play => self.resume(),
            Control::Pause => self.halt(),
            Control::Next => self.next(),
            Control::Previous => self.previous(),
            Control::Seek { ms } => self.seek(ms as i64),
            Control::PlayAt { index } => self.jump(index as usize),
            Control::SetShuffle { on } => self.shuffle(on),
            // media3's numbering.
            Control::SetRepeat { repeat } => self.repeat(match repeat {
                Repeat::Off => 0,
                Repeat::One => 1,
                Repeat::All => 2,
            }),
            Control::Add { songs, next } => self.enqueue(songs, next),
            Control::Remove { index } => self.remove(index as usize),
            Control::Move { index, to } => self.move_song(index as usize, to as usize),
            Control::Clear => self.clear_upcoming(),
        }
    }

    /// A media key or the system's media controls pressed `press`, told to the plugins that hear them.
    pub(crate) fn heard_press(&self, press: nori_plugins::Press) {
        if let Some(p) = &self.plugins {
            p.heard_press(press);
        }
    }

    /// The player's screen shows or not, for plugins' `while-visible` timers.
    pub fn player_shown(&self, shown: bool) {
        if let Some(p) = &self.plugins {
            p.player_shown(shown);
        }
    }

    /// Tells the plugins what is playing after engine event `e`, while one follows the player.
    pub(crate) fn plugins_heard(&self, e: &Event) {
        let Some(p) = &self.plugins else { return };
        if let Event::Output { name } = e {
            self.output.lock().clone_from(name);
        }
        if !p.follows() || !matches!(e, Event::State(_) | Event::Song { .. } | Event::Looped { .. } | Event::Position { .. } | Event::Output { .. } | Event::Stopped { .. }) {
            return;
        }
        let (playing, index, song, position) = self.engine.status_with(|s| (s.state == State::Playing, s.index, s.id.clone(), s.position_now()));
        let output = self.output.lock().clone();
        p.played(Moment { playing, index: index.map(|i| i as u32), song, position_ms: position.max(0) as u64, output });
    }
}
