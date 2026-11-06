//! The sleep timer: when the music pauses by itself. A time is the platform's to wait for (an alarm
//! where the CPU sleeps); a song's end is the engine's (`Engine::pause_at_end`); the songs are counted
//! here, as each arrives.

use nori_library::menus::SleepChoice;

use crate::Session;

/// Slack allowed to the platform's alarm, so its wake can be batched with others, ms.
const ALARM_WINDOW_MS: i64 = 15_000;

/// The sleep timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Sleep {
    #[default]
    Off,
    /// Pauses at `at_ms`, on the clock the platform gave when it was set.
    At { at_ms: i64 },
    /// Pauses at the end of the song playing.
    EndOfSong,
    /// Pauses at the end of the `songs`th song, the one playing counted first (2 or more).
    AfterSongs { songs: u32 },
}

/// The platform's alarm for a [`Sleep::At`]: due at `at_ms`, any time in the `window_ms` after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SleepAlarm {
    pub at_ms: i64,
    pub window_ms: i64,
}

/// What a timer set asks of the platform: whether the engine pauses at the end of this song, and the
/// alarm to arm (any armed before goes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SleepPlan {
    pub pause_at_end: bool,
    pub alarm: Option<SleepAlarm>,
}

impl Sleep {
    fn of(choice: SleepChoice, now_ms: i64) -> Sleep {
        match choice {
            SleepChoice { minutes: m @ 1.., .. } => Sleep::At { at_ms: now_ms + m as i64 * 60_000 },
            SleepChoice { end_of_track: true, .. } | SleepChoice { songs: 1, .. } => Sleep::EndOfSong,
            SleepChoice { songs: songs @ 2.., .. } => Sleep::AfterSongs { songs },
            _ => Sleep::Off,
        }
    }

    /// A new song arrived.
    fn song_changed(self) -> Sleep {
        match self {
            Sleep::AfterSongs { songs: 2 } => Sleep::EndOfSong,
            Sleep::AfterSongs { songs } => Sleep::AfterSongs { songs: songs - 1 },
            other => other,
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Session {
    /// Sets the sleep timer to `choice` (all zeros: off), at `now_ms` on the platform's clock.
    pub fn sleep_pick(&self, choice: SleepChoice, now_ms: i64) -> SleepPlan {
        let sleep = Sleep::of(choice, now_ms);
        self.controls.lock().sleep = sleep;
        let alarm = match sleep {
            Sleep::At { at_ms } => Some(SleepAlarm { at_ms, window_ms: ALARM_WINDOW_MS }),
            _ => None,
        };
        SleepPlan { pause_at_end: sleep == Sleep::EndOfSong, alarm }
    }

    /// The sleep timer as it runs now, for the player to show.
    pub fn sleep(&self) -> Sleep {
        self.controls.lock().sleep
    }

    /// The platform's alarm went off at `now_ms`: whether the music pauses now. The timer is over then.
    pub fn sleep_due(&self, now_ms: i64) -> bool {
        let mut c = self.controls.lock();
        let due = matches!(c.sleep, Sleep::At { at_ms } if now_ms >= at_ms);
        if due {
            c.sleep = Sleep::Off;
        }
        due
    }

    /// The music stopped by itself (the engine's `Event::Stopped`): a timer for this song's end is over.
    pub fn sleep_stopped(&self) {
        let mut c = self.controls.lock();
        if c.sleep == Sleep::EndOfSong {
            c.sleep = Sleep::Off;
        }
    }
}

impl Session {
    /// A new song arrived: whether the engine now pauses at its end.
    pub(crate) fn sleep_song_changed(&self) -> bool {
        let mut c = self.controls.lock();
        let was = c.sleep;
        c.sleep = was.song_changed();
        was != c.sleep && c.sleep == Sleep::EndOfSong
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(minutes: u32, end_of_track: bool, songs: u32) -> SleepChoice {
        SleepChoice { minutes, end_of_track, songs }
    }

    #[test]
    fn each_choice_sets_its_timer() {
        let s = Session::default();
        for (c, sleep, pause_at_end) in [
            (choice(30, false, 0), Sleep::At { at_ms: 1_000 + 30 * 60_000 }, false),
            (choice(0, true, 0), Sleep::EndOfSong, true),
            (choice(0, false, 1), Sleep::EndOfSong, true),
            (choice(0, false, 3), Sleep::AfterSongs { songs: 3 }, false),
            (choice(0, false, 0), Sleep::Off, false),
        ] {
            let plan = s.sleep_pick(c, 1_000);
            assert_eq!((s.sleep(), plan.pause_at_end, plan.alarm.map(|a| a.at_ms)), (sleep, pause_at_end, match sleep { Sleep::At { at_ms } => Some(at_ms), _ => None }), "{c:?}");
        }
    }

    #[test]
    fn songs_are_counted_down_to_the_last_ones_end() {
        let s = Session::default();
        s.sleep_pick(choice(0, false, 3), 0);
        assert!(!s.sleep_song_changed());
        assert_eq!(s.sleep(), Sleep::AfterSongs { songs: 2 });
        assert!(s.sleep_song_changed(), "the last song: pause at its end");
        assert_eq!(s.sleep(), Sleep::EndOfSong);
        assert!(!s.sleep_song_changed(), "asked once");
        s.sleep_stopped();
        assert_eq!(s.sleep(), Sleep::Off, "over once the music stopped there");
    }

    #[test]
    fn an_alarm_pauses_only_when_its_time_has_come() {
        let s = Session::default();
        s.sleep_pick(choice(15, false, 0), 0);
        assert!(!s.sleep_due(15 * 60_000 - 1), "an earlier alarm's");
        s.sleep_stopped();
        assert!(s.sleep_due(15 * 60_000));
        assert_eq!(s.sleep(), Sleep::Off);
        assert!(!s.sleep_due(16 * 60_000), "once");
    }
}
