//! When a device holds its connection to the sync server, and how it keeps it up. A computer stays
//! connected while nori runs. A phone is connected while nori is open or playing; paused in the
//! background it lets go after [`LET_GO_US`], or at once if the connection drops, and connects again when
//! nori is opened or plays. A phone's keep-alive rides on the wake-ups the audio already causes while it
//! plays in the background ([`Presence::pings`]): it never wakes the phone by itself.

/// What kind of device nori runs on, for when it is reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Reach {
    /// Connected while nori runs.
    Computer,
    /// Connected while nori is open or playing.
    Phone,
}

/// How a connection's keep-alives go out now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pings {
    /// On a timer of the sync's own.
    Timer,
    /// When the audio wakes the device anyway ([`crate::Syncing::woke`]).
    Woken,
    /// Not at all: the connection lasts as long as it does.
    None,
}

/// How long a phone paused in the background holds its connection.
pub(crate) const LET_GO_US: i64 = 15 * 60 * 1_000_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Presence {
    reach: Reach,
    foreground: bool,
    playing: bool,
    /// Since when a phone has been in the background and paused, on the sync's clock; None before it
    /// was ever open or playing.
    quiet_since: Option<i64>,
}

impl Presence {
    pub(crate) fn new(reach: Reach) -> Presence {
        Presence { reach, foreground: false, playing: false, quiet_since: None }
    }

    fn active(&self) -> bool {
        self.foreground || self.playing
    }

    fn told(&mut self, now: i64, change: impl FnOnce(&mut Presence)) {
        let was = self.active();
        change(self);
        if was && !self.active() {
            self.quiet_since = Some(now);
        }
    }

    pub(crate) fn foreground(&mut self, on: bool, now: i64) {
        self.told(now, |p| p.foreground = on);
    }

    pub(crate) fn playing(&mut self, on: bool, now: i64) {
        self.told(now, |p| p.playing = on);
    }

    /// Whether the device holds its connection now.
    pub(crate) fn wants(&self, now: i64) -> bool {
        match self.reach {
            Reach::Computer => true,
            Reach::Phone => self.active() || self.quiet_since.is_some_and(|t| now - t < LET_GO_US),
        }
    }

    /// When a phone paused in the background lets go.
    pub(crate) fn let_go_at(&self) -> Option<i64> {
        match self.reach {
            Reach::Phone if !self.active() => self.quiet_since.map(|t| t + LET_GO_US),
            _ => None,
        }
    }

    /// Whether a dropped connection comes back by itself: not on a phone paused in the background.
    pub(crate) fn reconnects(&self) -> bool {
        self.reach == Reach::Computer || self.active()
    }

    pub(crate) fn pings(&self) -> Pings {
        match self.reach {
            Reach::Computer => Pings::Timer,
            Reach::Phone if self.foreground => Pings::Timer,
            Reach::Phone if self.playing => Pings::Woken,
            Reach::Phone => Pings::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60 * 1_000_000;

    #[test]
    fn a_computer_stays_connected_and_pings_on_its_own() {
        let p = Presence::new(Reach::Computer);
        assert!(p.wants(0) && p.reconnects() && p.let_go_at().is_none());
        assert_eq!(p.pings(), Pings::Timer);
    }

    #[test]
    fn a_phone_connects_while_open_or_playing_and_pings_only_on_the_audio_s_wakes_in_the_background() {
        let mut p = Presence::new(Reach::Phone);
        assert!(!p.wants(0), "started in the background, never opened");
        p.foreground(true, 0);
        assert_eq!((p.wants(0), p.pings()), (true, Pings::Timer));
        p.playing(true, 0);
        p.foreground(false, MIN);
        assert_eq!((p.wants(60 * MIN), p.pings(), p.let_go_at()), (true, Pings::Woken, None), "playing with the screen off");
        p.playing(false, 2 * MIN);
        assert_eq!((p.pings(), p.reconnects()), (Pings::None, false), "paused in the background");
        assert_eq!(p.let_go_at(), Some(2 * MIN + LET_GO_US));
        assert!(p.wants(16 * MIN) && !p.wants(17 * MIN), "lets go after 15 minutes");
        p.playing(true, 30 * MIN);
        assert!(p.wants(30 * MIN) && p.let_go_at().is_none(), "and connects again when it plays");
    }
}
