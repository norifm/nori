//! Whether the output holds its deep buffer or only a fraction of a second, while sound pages are in
//! sight: a change made on one is then heard at once.

use std::sync::Arc;

use nori_settings::settings_pages::SettingsPage;
use parking_lot::Mutex;

/// A page where a sound change should be heard at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SoundPage {
    Equalizer,
    /// The headphone presets (AutoEQ).
    AutoEq,
    /// Settings › Sound.
    Sound,
    /// A plugin's page, or the devices sheet with plugins' rows on it: tuned from its first look, as
    /// turning a plugin on is already a change.
    Plugin,
}

impl SoundPage {
    /// Settings page `page`, if it is a sound page.
    pub fn of_settings(page: SettingsPage) -> Option<SoundPage> {
        (page == SettingsPage::Sound).then_some(SoundPage::Sound)
    }
}

/// What the output holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Buffer {
    Deep,
    /// A fraction of a second (`Engine::set_shallow`).
    Shallow,
}

/// The sound pages in sight and what they did. Pages are counted: one coming in sight can start before
/// the one it covers stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tuning {
    #[default]
    Deep,
    /// `pages` in sight, nothing changed on them yet.
    Untouched { pages: u32 },
    /// `pages` in sight, and a change was made on one since the first came.
    Shallow { pages: u32 },
}

impl Tuning {
    /// Sound page `page` came in sight. Returns the buffer to switch to, if it changes.
    pub fn shown(&mut self, page: SoundPage) -> Option<Buffer> {
        let pages = self.pages() + 1;
        let was = self.buffer();
        *self = match self {
            Tuning::Shallow { .. } => Tuning::Shallow { pages },
            _ if page == SoundPage::Plugin => Tuning::Shallow { pages },
            _ => Tuning::Untouched { pages },
        };
        self.switch(was)
    }

    /// A sound setting changed; only while a sound page is in sight does it count.
    pub fn edited(&mut self) -> Option<Buffer> {
        let was = self.buffer();
        if let Tuning::Untouched { pages } = *self {
            *self = Tuning::Shallow { pages };
        }
        self.switch(was)
    }

    /// A sound page left sight; with none left, the deep buffer comes back.
    pub fn left(&mut self) -> Option<Buffer> {
        let was = self.buffer();
        *self = match *self {
            Tuning::Deep | Tuning::Untouched { pages: 0..=1 } | Tuning::Shallow { pages: 0..=1 } => Tuning::Deep,
            Tuning::Untouched { pages } => Tuning::Untouched { pages: pages - 1 },
            Tuning::Shallow { pages } => Tuning::Shallow { pages: pages - 1 },
        };
        self.switch(was)
    }

    /// Whether a change on a page in sight would switch the buffer: the client then watches for one.
    pub fn waits_for_edit(&self) -> bool {
        matches!(self, Tuning::Untouched { .. })
    }

    pub fn buffer(&self) -> Buffer {
        match self {
            Tuning::Shallow { .. } => Buffer::Shallow,
            Tuning::Deep | Tuning::Untouched { .. } => Buffer::Deep,
        }
    }

    fn pages(&self) -> u32 {
        match *self {
            Tuning::Deep => 0,
            Tuning::Untouched { pages } | Tuning::Shallow { pages } => pages,
        }
    }

    fn switch(&self, was: Buffer) -> Option<Buffer> {
        let now = self.buffer();
        (now != was).then_some(now)
    }
}

/// What a step of [`SoundTuning`] asks of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TuningStep {
    /// The buffer to switch the output to, if it changes.
    pub set: Option<Buffer>,
    /// Whether to watch for a sound change ([`SoundTuning::edited`]) on the pages in sight.
    pub watch: bool,
}

/// A [`Tuning`] shared by a client's screens, called in order from its main thread.
#[derive(Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct SoundTuning(Mutex<Tuning>);

#[cfg_attr(feature = "ffi", uniffi::export)]
impl SoundTuning {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn shown(&self, page: SoundPage) -> TuningStep {
        self.step(|t| t.shown(page))
    }

    pub fn edited(&self) -> TuningStep {
        self.step(Tuning::edited)
    }

    pub fn left(&self) -> TuningStep {
        self.step(Tuning::left)
    }
}

impl SoundTuning {
    fn step(&self, f: impl FnOnce(&mut Tuning) -> Option<Buffer>) -> TuningStep {
        let mut t = self.0.lock();
        let set = f(&mut t);
        TuningStep { set, watch: t.waits_for_edit() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Buffer::{Deep, Shallow};

    #[test]
    fn a_change_on_a_page_in_sight_goes_shallow_until_the_last_page_leaves() {
        let mut t = Tuning::default();
        assert_eq!(t.edited(), None, "no page in sight");
        assert_eq!(t.shown(SoundPage::Equalizer), None, "opening alone changes nothing");
        assert!(t.waits_for_edit());
        assert_eq!(t.edited(), Some(Shallow));
        assert_eq!(t.edited(), None, "once");
        assert!(!t.waits_for_edit());
        // The next page comes in sight before the one it covers stops.
        assert_eq!(t.shown(SoundPage::AutoEq), None);
        assert_eq!(t.left(), None, "a page still in sight");
        assert_eq!(t.left(), Some(Deep));
        assert_eq!(t, Tuning::Deep);
    }

    #[test]
    fn a_page_left_untouched_asks_nothing_back() {
        let mut t = Tuning::default();
        t.shown(SoundPage::Sound);
        assert_eq!(t.left(), None);
        assert_eq!(t, Tuning::Deep);
    }

    #[test]
    fn a_plugin_page_is_tuned_from_its_first_look() {
        let mut t = Tuning::default();
        assert_eq!(t.shown(SoundPage::Plugin), Some(Shallow));
        assert_eq!(t.shown(SoundPage::Equalizer), None, "the next page keeps it");
        t.left();
        assert_eq!(t.left(), Some(Deep));
    }

    #[test]
    fn leaving_more_pages_than_were_shown_stays_deep() {
        let mut t = Tuning::default();
        assert_eq!(t.left(), None);
        assert_eq!(t.shown(SoundPage::Equalizer), None);
        assert_eq!(t.edited(), Some(Shallow), "the count did not go below none");
    }

    #[test]
    fn only_settings_sound_is_a_sound_page() {
        let pages: Vec<_> = SettingsPage::ALL.into_iter().filter_map(SoundPage::of_settings).collect();
        assert_eq!(pages, [SoundPage::Sound]);
    }

    #[test]
    fn the_shared_tuning_says_when_to_watch() {
        let s = SoundTuning::default();
        assert_eq!(s.shown(SoundPage::Equalizer), TuningStep { set: None, watch: true });
        assert_eq!(s.edited(), TuningStep { set: Some(Shallow), watch: false });
        assert_eq!(s.left(), TuningStep { set: Some(Deep), watch: false });
    }
}
