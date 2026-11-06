//! The audio focus, kept beside the play state (`nori_player::focus`): a change and a press are taken
//! in the order they came, so an answer is never about a state already left.

use nori_player::focus::{Focus, FocusState, Hold, Step};
use nori_player::pipeline::{App, Queue};

use super::{Clock, Command, Event, Library, Worker};

/// The platform's audio focus (Android's): asked for before the music sounds, given back when the
/// listener pauses. Called on the engine's thread.
pub trait FocusRequest: Send {
    /// Whether the focus was granted.
    fn request(&mut self) -> bool;
    fn abandon(&mut self);
}

/// The focus state, the platform's door to it (none: always granted), and the hold last said.
#[derive(Default)]
pub(super) struct Focused {
    state: FocusState,
    request: Option<Box<dyn FocusRequest>>,
    said: Hold,
}

impl<L: Library, A: App, Q: Queue, E: FnMut(Event), C: Clock> Worker<L, A, Q, E, C> {
    pub(super) fn ask_focus_of(&mut self, request: Box<dyn FocusRequest>) {
        self.focus.request = Some(request);
    }

    /// Play pressed: whether the music may sound. Refused, it is said again even if the hold is the
    /// same, since the client took the press as playing.
    pub(super) fn focus_play(&mut self) -> bool {
        let Focused { state, request, .. } = &mut self.focus;
        let granted = state.play(|| request.as_mut().is_none_or(|r| r.request()));
        if granted {
            self.say_hold();
        } else {
            self.focus.said = self.focus.state.hold;
            (self.events)(Event::Focus(self.focus.said));
        }
        granted
    }

    /// The listener paused, or the music went elsewhere: the focus is given back. Not while following
    /// another device, whose play is heard here again on play.
    pub(super) fn focus_paused(&mut self) {
        if self.following.is_some() {
            return;
        }
        let Focused { state, request, .. } = &mut self.focus;
        state.paused(|| {
            if let Some(r) = request.as_mut() {
                r.abandon();
            }
        });
        self.say_hold();
    }

    /// The focus changed: what the music does is the focus state's, with a plugin's word.
    pub(super) fn focus_changed(&mut self, change: Focus, now: i64) {
        let wants_play = match &self.away {
            Some(a) => a.playing,
            None => self.going(),
        };
        let app = &mut self.p.app;
        let step = self.focus.state.changed(change, wants_play, |f, nori| app.audio_focus(f, nori));
        self.p.app.log(&format!("audio focus {change:?}: {step:?}, {:?}", self.focus.state.hold));
        match step {
            Step::Pause => match self.following.as_mut() {
                Some(f) => {
                    f.hold(true);
                    self.pause(now, self.settings.fade_ms);
                }
                None => self.obey(Command::Pause(None), now),
            },
            Step::Resume => match self.following.as_mut() {
                Some(f) => f.hold(false),
                None if self.away.is_some() => self.obey(Command::Play, now),
                None => self.play(),
            },
            Step::Stay => {}
        }
        self.say_hold();
    }

    fn say_hold(&mut self) {
        let hold = self.focus.state.hold;
        if hold != self.focus.said {
            self.focus.said = hold;
            (self.events)(Event::Focus(hold));
        }
    }
}

impl Drop for Focused {
    /// The engine stopped: the focus goes with it.
    fn drop(&mut self) {
        let Focused { state, request, .. } = self;
        state.paused(|| {
            if let Some(r) = request.as_mut() {
                r.abandon();
            }
        });
    }
}
