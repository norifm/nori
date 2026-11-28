//! The lyrics shown for the song heard: which song they are for, what was found, whether to look it up,
//! and the last few songs' answers, so a song come back to starts from its words.

use nori_model::ItemKey;
use parking_lot::Mutex;

use crate::race::{lyrics_replaces, LyricsPick};

/// Songs whose lyrics are kept besides the one heard.
pub const KEPT_SONGS: usize = 4;

/// What a lyrics view shows.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum LyricsNow {
    /// Nothing plays.
    NoSong,
    /// `song`'s lyrics are being looked up and none came yet.
    Looking { song: ItemKey },
    /// `song`'s lyrics; no lines when the lookup found none.
    Shown { song: ItemKey, pick: LyricsPick },
    /// The lookup for `song` failed with nothing found.
    Failed { song: ItemKey },
}

impl LyricsNow {
    /// The song these are for; None when nothing plays.
    pub fn song(&self) -> Option<&ItemKey> {
        match self {
            LyricsNow::NoSong => None,
            LyricsNow::Looking { song } | LyricsNow::Shown { song, .. } | LyricsNow::Failed { song } => Some(song),
        }
    }

    /// The lyrics shown, when they are `song`'s and have lines.
    pub fn words_of(&self, song: &ItemKey) -> Option<&LyricsPick> {
        match self {
            LyricsNow::Shown { song: s, pick } if s == song && !pick.lyrics.lines.is_empty() => Some(pick),
            _ => None,
        }
    }
}

/// How a lookup ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum LyricsEnd {
    /// Every answer came.
    Done,
    /// The lookup failed.
    Failed,
    /// It was let go before it ended (the view stopped watching).
    Stopped,
}

struct Kept {
    song: ItemKey,
    pick: LyricsPick,
    /// Its lookup ended: it is not looked up again.
    done: bool,
}

struct State {
    now: LyricsNow,
    /// The song whose lookup runs.
    running: Option<ItemKey>,
    /// Answers with lines, the newest last.
    kept: Vec<Kept>,
}

impl State {
    fn kept(&self, song: &ItemKey) -> Option<&Kept> {
        self.kept.iter().find(|k| k.song == *song)
    }

    fn keep(&mut self, song: &ItemKey, pick: LyricsPick, done: bool) {
        self.kept.retain(|k| k.song != *song);
        self.kept.push(Kept { song: song.clone(), pick, done });
        if self.kept.len() > KEPT_SONGS {
            self.kept.remove(0);
        }
    }

    fn heard(&self) -> Option<&ItemKey> {
        self.now.song()
    }
}

/// The lyrics of the song heard, followed as it changes. The client says which song is heard
/// ([`LyricsHeard::hear`]), runs the lookups it is told to ([`LyricsHeard::fetch`]) and hands their
/// answers in; it draws [`LyricsHeard::now`].
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct LyricsHeard(Mutex<State>);

impl Default for LyricsHeard {
    fn default() -> Self {
        LyricsHeard(Mutex::new(State { now: LyricsNow::NoSong, running: None, kept: Vec::new() }))
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl LyricsHeard {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::default()
    }

    /// `song` is heard now. A song changed to starts from its kept lyrics, else looking; the same song
    /// keeps what it shows. Returns what is shown.
    pub fn hear(&self, song: Option<ItemKey>) -> LyricsNow {
        let mut s = self.0.lock();
        if s.heard() != song.as_ref() {
            s.now = match song {
                None => LyricsNow::NoSong,
                Some(song) => match s.kept(&song) {
                    Some(k) => LyricsNow::Shown { pick: k.pick.clone(), song },
                    None => LyricsNow::Looking { song },
                },
            };
        }
        s.now.clone()
    }

    /// The song heard, if it is to be looked up now: none runs and none ended for it. Its lookup then
    /// counts as running until [`LyricsHeard::ended`].
    pub fn fetch(&self) -> Option<ItemKey> {
        let mut s = self.0.lock();
        let song = s.heard()?.clone();
        if s.running.as_ref() == Some(&song) || s.kept(&song).is_some_and(|k| k.done) {
            return None;
        }
        s.running = Some(song.clone());
        Some(song)
    }

    /// The song heard is looked up again, even if its lookup ended; None when nothing plays.
    pub fn refresh(&self) -> Option<ItemKey> {
        let mut s = self.0.lock();
        let song = s.heard()?.clone();
        if let Some(k) = s.kept.iter_mut().find(|k| k.song == song) {
            k.done = false;
        }
        s.running = None;
        drop(s);
        self.fetch()
    }

    /// An answer of `song`'s lookup: true when it is shown now. Only the song heard's are, and not the
    /// same lyrics again (`lyrics_replaces`), nor none after some.
    pub fn answer(&self, song: ItemKey, pick: LyricsPick) -> bool {
        let mut s = self.0.lock();
        if s.heard() != Some(&song) {
            return false;
        }
        let shown = match &s.now {
            LyricsNow::Shown { pick, .. } => Some(pick),
            _ => None,
        };
        if shown.is_some_and(|p| !p.lyrics.lines.is_empty() && pick.lyrics.lines.is_empty()) || !lyrics_replaces(shown, &pick) {
            return false;
        }
        if !pick.lyrics.lines.is_empty() {
            s.keep(&song, pick.clone(), false);
        }
        s.now = LyricsNow::Shown { song, pick };
        true
    }

    /// `song`'s lookup ended as `end`: what is shown now, when that changed (a failure with nothing
    /// found, or no answer at all).
    pub fn ended(&self, song: ItemKey, end: LyricsEnd) -> Option<LyricsNow> {
        let mut s = self.0.lock();
        if s.running.as_ref() == Some(&song) {
            s.running = None;
        }
        if s.heard() != Some(&song) || end == LyricsEnd::Stopped {
            return None;
        }
        if let LyricsNow::Shown { pick, .. } = &s.now {
            if end == LyricsEnd::Done && !pick.lyrics.lines.is_empty() {
                let pick = pick.clone();
                s.keep(&song, pick, true);
            }
            return None;
        }
        s.now = LyricsNow::Failed { song };
        Some(s.now.clone())
    }

    /// `song`'s lyrics retimed by the listener (`Core::lyrics_nudge`): true when they are shown now.
    pub fn retimed(&self, song: ItemKey, pick: LyricsPick) -> bool {
        let mut s = self.0.lock();
        let LyricsNow::Shown { song: shown, .. } = &s.now else { return false };
        if *shown != song {
            return false;
        }
        if let Some(k) = s.kept.iter_mut().find(|k| k.song == song) {
            k.pick = pick.clone();
        }
        s.now = LyricsNow::Shown { song, pick };
        true
    }

    pub fn now(&self) -> LyricsNow {
        self.0.lock().now.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(id: &str) -> ItemKey {
        ItemKey::own(id)
    }
    use nori_settings::lyrics_sources::LyricsOrigin;
    use nori_model::{LyricLine, Lyrics};

    fn pick(words: &str, origin: LyricsOrigin) -> LyricsPick {
        let lines = words.split('/').filter(|l| !l.is_empty()).map(|l| LyricLine { text: l.into(), ..Default::default() }).collect();
        LyricsPick { lyrics: Lyrics { lines, ..Default::default() }, origin }
    }

    fn words(w: &str) -> LyricsPick {
        pick(w, LyricsOrigin::Server)
    }

    fn none() -> LyricsPick {
        words("")
    }

    fn shown(h: &LyricsHeard) -> Option<String> {
        match h.now() {
            LyricsNow::Shown { pick, .. } => Some(pick.lyrics.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("/")),
            _ => None,
        }
    }

    #[test]
    fn a_new_song_looks_and_shows_only_its_own_answers() {
        let h = LyricsHeard::new();
        assert_eq!(h.hear(Some(k("a"))), LyricsNow::Looking { song: k("a") });
        assert_eq!(h.fetch().map(|k| k.id).as_deref(), Some("a"));
        assert_eq!(h.fetch(), None, "its lookup runs");
        h.hear(Some(k("b")));
        assert!(!h.answer(k("a"), words("old")), "the song before's answer");
        assert_eq!(h.now(), LyricsNow::Looking { song: k("b") });
        assert!(h.answer(k("b"), words("new")));
        assert_eq!(shown(&h).as_deref(), Some("new"));
        assert_eq!(h.hear(None), LyricsNow::NoSong);
        assert_eq!(h.fetch(), None, "nothing to look up");
    }

    #[test]
    fn only_a_better_answer_replaces_the_words() {
        let h = LyricsHeard::new();
        h.hear(Some(k("a")));
        h.fetch();
        assert!(h.answer(k("a"), words("line")));
        assert!(!h.answer(k("a"), words("line")), "the same words again");
        assert!(!h.answer(k("a"), none()), "none after some");
        assert!(h.answer(k("a"), pick("line", LyricsOrigin::Lrclib)), "another source's");
        assert_eq!(h.ended(k("a"), LyricsEnd::Done), None);
        assert_eq!(shown(&h).as_deref(), Some("line"));
    }

    #[test]
    fn a_song_come_back_to_starts_from_its_words() {
        let h = LyricsHeard::new();
        h.hear(Some(k("a")));
        h.fetch();
        h.answer(k("a"), words("a words"));
        h.ended(k("a"), LyricsEnd::Done);
        h.hear(Some(k("b")));
        h.fetch();
        h.answer(k("b"), words("b words"));
        h.ended(k("b"), LyricsEnd::Stopped);
        h.hear(Some(k("a")));
        assert_eq!(shown(&h).as_deref(), Some("a words"));
        assert_eq!(h.fetch(), None, "its lookup had ended");
        h.hear(Some(k("b")));
        assert_eq!(shown(&h).as_deref(), Some("b words"));
        assert_eq!(h.fetch().map(|k| k.id).as_deref(), Some("b"), "its lookup was let go before the end");
    }

    #[test]
    fn only_the_last_songs_are_kept() {
        let h = LyricsHeard::new();
        for song in ["s0", "s1", "s2", "s3", "s4"] {
            h.hear(Some(k(song)));
            h.fetch();
            h.answer(k(song), words(song));
            h.ended(k(song), LyricsEnd::Done);
        }
        h.hear(Some(k("s1")));
        assert_eq!(shown(&h).as_deref(), Some("s1"));
        h.hear(Some(k("s0")));
        assert_eq!(h.now(), LyricsNow::Looking { song: k("s0") }, "the oldest is forgotten");
    }

    #[test]
    fn none_found_is_shown_and_looked_up_again_later() {
        let h = LyricsHeard::new();
        h.hear(Some(k("a")));
        h.fetch();
        assert!(h.answer(k("a"), none()));
        assert_eq!(h.ended(k("a"), LyricsEnd::Done), None);
        h.hear(Some(k("b")));
        h.hear(Some(k("a")));
        assert_eq!(h.now(), LyricsNow::Looking { song: k("a") }, "no words are kept");
        assert_eq!(h.fetch().map(|k| k.id).as_deref(), Some("a"));
    }

    #[test]
    fn a_failure_shows_only_where_nothing_was_found() {
        let h = LyricsHeard::new();
        h.hear(Some(k("a")));
        h.fetch();
        assert_eq!(h.ended(k("a"), LyricsEnd::Failed), Some(LyricsNow::Failed { song: k("a") }));
        assert_eq!(h.fetch().map(|k| k.id).as_deref(), Some("a"), "a failure is asked again");
        h.answer(k("a"), words("found"));
        assert_eq!(h.ended(k("a"), LyricsEnd::Failed), None, "the words stay");
        assert_eq!(shown(&h).as_deref(), Some("found"));
        h.hear(Some(k("b")));
        assert_eq!(h.ended(k("a"), LyricsEnd::Failed), None, "another song's failure");
        assert_eq!(h.ended(k("b"), LyricsEnd::Stopped), None);
        assert_eq!(h.now(), LyricsNow::Looking { song: k("b") });
    }

    #[test]
    fn refresh_looks_up_an_ended_song_again() {
        let h = LyricsHeard::new();
        assert_eq!(h.refresh(), None);
        h.hear(Some(k("a")));
        h.fetch();
        h.answer(k("a"), words("old"));
        h.ended(k("a"), LyricsEnd::Done);
        assert_eq!(h.fetch(), None);
        assert_eq!(h.refresh().map(|k| k.id).as_deref(), Some("a"));
        assert_eq!(shown(&h).as_deref(), Some("old"), "the words stay until better ones come");
    }

    #[test]
    fn retimed_words_replace_the_shown_and_kept() {
        let h = LyricsHeard::new();
        h.hear(Some(k("a")));
        h.fetch();
        h.answer(k("a"), words("line"));
        h.ended(k("a"), LyricsEnd::Done);
        let mut later = words("line");
        later.lyrics.timing = nori_model::LyricsTiming::Yours { ms: 200, automatic: None };
        assert!(!h.retimed(k("b"), later.clone()), "not the song shown");
        assert!(h.retimed(k("a"), later.clone()));
        h.hear(Some(k("b")));
        h.hear(Some(k("a")));
        assert_eq!(h.now(), LyricsNow::Shown { song: k("a"), pick: later });
    }

    #[test]
    fn words_of_names_only_the_songs_lines() {
        let n = LyricsNow::Shown { song: k("a"), pick: words("x") };
        assert!(n.words_of(&k("a")).is_some());
        assert!(n.words_of(&k("b")).is_none());
        assert!(n.words_of(&ItemKey { source: Some("nd2".into()), id: "a".into() }).is_none(), "another source's a");
        assert!(LyricsNow::Shown { song: k("a"), pick: none() }.words_of(&k("a")).is_none());
        assert_eq!(LyricsNow::Failed { song: k("a") }.song(), Some(&k("a")));
    }
}
