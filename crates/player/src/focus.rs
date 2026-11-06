//! The audio focus: what another sound (a call, another player, a voice prompt) does to the music, and
//! when the platform's focus is asked for and given back. The engine holds one [`FocusState`] beside
//! its play state, so a change and a press are taken in the order they came; a plugin may answer a
//! change in nori's place (`Deciders::audio_focus`), checked by the state, never stale.

/// A change of the system's audio focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Another player took it for good.
    Lost,
    /// A call or a voice prompt, for a while.
    LostTransient,
    /// Another sound plays over the music for a while.
    Duck,
    Gained,
}

/// What a focus change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusAct {
    /// Paused: for good after [`Focus::Lost`], until the focus comes back otherwise.
    Pause,
    /// Played on, quieter (the system lowers it).
    Duck,
    KeepPlaying,
    /// What a focus loss paused plays again.
    Resume,
    Nothing,
}

/// What a focus change does on its own: a loss pauses what wants to play (for good, or until the focus
/// is back), a duck plays on under the other sound, and a gain resumes what a loss `held`.
pub fn focus_act(focus: Focus, wants_play: bool, held: bool) -> FocusAct {
    match focus {
        Focus::Lost | Focus::LostTransient if wants_play => FocusAct::Pause,
        Focus::Lost | Focus::LostTransient => FocusAct::Nothing,
        Focus::Duck => FocusAct::Duck,
        Focus::Gained if held => FocusAct::Resume,
        Focus::Gained => FocusAct::Nothing,
    }
}

/// Where the focus holds the music.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hold {
    /// Nothing holds it: it plays or pauses as asked.
    #[default]
    Free,
    /// Another sound plays over it for a while.
    Ducked,
    /// A loss paused it until the focus comes back, still wanted.
    Held,
    /// A loss paused it for good, or the platform refused the focus to a play.
    Lost,
}

/// What the player does for a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Pause,
    Resume,
    Stay,
}

/// The focus as the player keeps it: where it holds the music, and whether the platform's focus is
/// this player's (asked for, not lost or given back).
#[derive(Debug, Default)]
pub struct FocusState {
    pub hold: Hold,
    owned: bool,
}

impl FocusState {
    /// A change of the focus while the music `wants_play`: what the player does. `decide` takes nori's
    /// act and answers the one carried out.
    pub fn changed(&mut self, focus: Focus, wants_play: bool, decide: impl FnOnce(Focus, FocusAct) -> FocusAct) -> Step {
        if focus == Focus::Lost {
            self.owned = false;
        }
        let held = self.hold == Hold::Held;
        match decide(focus, focus_act(focus, wants_play, held)) {
            FocusAct::Pause if wants_play => {
                self.hold = if focus == Focus::Lost { Hold::Lost } else { Hold::Held };
                Step::Pause
            }
            FocusAct::Resume if held => {
                self.hold = Hold::Free;
                Step::Resume
            }
            FocusAct::Duck => {
                self.hold = Hold::Ducked;
                Step::Stay
            }
            // What a loss held it for is over: nothing plays it again.
            _ if focus == Focus::Lost && held => {
                self.hold = Hold::Lost;
                Step::Stay
            }
            _ if focus == Focus::Gained && self.hold == Hold::Ducked => {
                self.hold = Hold::Free;
                Step::Stay
            }
            _ => Step::Stay,
        }
    }

    /// Play was pressed: whether the music may sound. The platform's focus is asked for again each time
    /// (`request`), since it refuses it during a call; refused, a hold stays where it was, and a focus
    /// held from before is still this player's.
    pub fn play(&mut self, request: impl FnOnce() -> bool) -> bool {
        let granted = request();
        self.owned |= granted;
        if granted {
            self.hold = Hold::Free;
        } else if self.hold != Hold::Held {
            self.hold = Hold::Lost;
        }
        granted
    }

    /// The listener paused or let the music go: no loss holds it any more, and the focus is given back
    /// (`abandon`) when it was this player's.
    pub fn paused(&mut self, abandon: impl FnOnce()) {
        self.hold = Hold::Free;
        if core::mem::take(&mut self.owned) {
            abandon();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nori(_: Focus, act: FocusAct) -> FocusAct {
        act
    }

    #[test]
    fn nori_alone_answers_by_its_own_rules() {
        assert_eq!(focus_act(Focus::Lost, true, false), FocusAct::Pause);
        assert_eq!(focus_act(Focus::LostTransient, false, false), FocusAct::Nothing);
        assert_eq!(focus_act(Focus::Duck, true, false), FocusAct::Duck);
        assert_eq!(focus_act(Focus::Gained, true, true), FocusAct::Resume);
        assert_eq!(focus_act(Focus::Gained, true, false), FocusAct::Nothing);
    }

    #[test]
    fn a_call_holds_the_music_until_it_ends() {
        let mut f = FocusState::default();
        assert!(f.play(|| true));
        assert_eq!(f.changed(Focus::LostTransient, true, nori), Step::Pause);
        assert_eq!(f.hold, Hold::Held);
        assert!(!f.play(|| false), "play during the call: refused");
        assert_eq!(f.hold, Hold::Held, "held still");
        assert_eq!(f.changed(Focus::Gained, false, nori), Step::Resume);
        assert_eq!(f.hold, Hold::Free);
    }

    #[test]
    fn a_pause_during_a_call_is_not_undone_by_its_end() {
        let mut f = FocusState::default();
        f.play(|| true);
        f.changed(Focus::LostTransient, true, nori);
        assert!(!f.play(|| false), "refused during the call");
        let mut given_back = 0;
        f.paused(|| given_back += 1);
        assert_eq!(f.changed(Focus::Gained, false, nori), Step::Stay);
        assert_eq!(given_back, 1);
        f.paused(|| given_back += 1);
        assert_eq!(given_back, 1, "given back once");
    }

    #[test]
    fn a_loss_for_good_never_resumes() {
        let mut f = FocusState::default();
        f.play(|| true);
        assert_eq!(f.changed(Focus::Lost, true, nori), Step::Pause);
        assert_eq!(f.hold, Hold::Lost);
        assert_eq!(f.changed(Focus::Gained, false, nori), Step::Stay);
        let mut given_back = false;
        f.paused(|| given_back = true);
        assert!(!given_back, "lost, not this player's to give back");
        // A held pause the platform then ends for good (iOS: an interruption that should not resume).
        f.play(|| true);
        f.changed(Focus::LostTransient, true, nori);
        assert_eq!(f.changed(Focus::Lost, false, nori), Step::Stay);
        assert_eq!((f.hold, f.changed(Focus::Gained, false, nori)), (Hold::Lost, Step::Stay));
    }

    #[test]
    fn a_plugins_answer_is_taken_where_it_fits() {
        let mut f = FocusState::default();
        f.play(|| true);
        assert_eq!(f.changed(Focus::LostTransient, true, |_, _| FocusAct::KeepPlaying), Step::Stay);
        assert_eq!(f.changed(Focus::Duck, true, |_, _| FocusAct::Pause), Step::Pause, "a duck paused, until the focus is back");
        assert_eq!(f.changed(Focus::Gained, false, nori), Step::Resume);
        assert_eq!(f.changed(Focus::Gained, true, |_, _| FocusAct::Resume), Step::Stay, "nothing held to resume");
        assert_eq!(f.changed(Focus::LostTransient, false, |_, _| FocusAct::Pause), Step::Stay, "paused already");
    }
}
