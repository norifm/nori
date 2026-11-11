//! The lyrics shown for the song heard (`nori_core::lyrics_follow`), looked up while a lyrics view is
//! open. The client says which song is heard and whether a view is open, and draws each
//! [`Said::Lyrics`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::future::{AbortHandle, Abortable};
use nori_core::lyrics_follow::{LyricsEnd, LyricsHeard, LyricsNow};
use nori_core::race::{LyricsPick, LyricsShown};

use crate::session::{Handle, Out, Said};

/// The session's lyrics: what is shown, and whether a view is open.
#[derive(Default)]
pub(crate) struct Lyrics {
    heard: Arc<LyricsHeard>,
    viewed: AtomicBool,
    /// The lookup running, for its song: it follows changes from other devices until let go.
    lookup: parking_lot::Mutex<Option<(String, AbortHandle)>>,
}

/// A lookup's answers, handed to what is shown.
struct Answers {
    song: String,
    heard: Arc<LyricsHeard>,
    out: Out,
}

impl LyricsShown for Answers {
    fn show(&self, pick: LyricsPick) {
        if self.heard.answer(self.song.clone(), pick) {
            (self.out)(Said::Lyrics(self.heard.now()));
        }
    }
}

impl Handle {
    /// `song` is the one the player shows: what its lyrics view shows now. Later changes come as
    /// [`Said::Lyrics`].
    pub fn lyrics_heard(&self, song: Option<String>) -> LyricsNow {
        if self.lyrics.lookup.lock().as_ref().is_some_and(|(of, _)| song.as_ref() != Some(of)) {
            self.lyrics_stop();
        }
        let now = self.lyrics.heard.hear(song);
        self.lyrics_fetch();
        now
    }

    /// A lyrics view opened or closed: the song heard's lyrics are looked up only while one is open,
    /// and the core warms the songs ahead's meanwhile.
    pub fn lyrics_view(&self, open: bool) {
        self.lyrics.viewed.store(open, Ordering::Relaxed);
        self.client.lyrics_view(open);
        if !open {
            self.lyrics_stop();
        }
        if open {
            let client = self.client.clone();
            self.run(async move { client.lyrics_prefetch().await });
            self.lyrics_fetch();
        }
    }

    /// Looks the song heard's lyrics up again, even if they were found.
    pub fn lyrics_refresh(&self) {
        if let Some(song) = self.lyrics.heard.refresh() {
            self.lyrics_look_up(song);
        }
    }

    /// The lyrics shown a step sooner (`dir` > 0) or later (< 0), or back to the check's timing (0),
    /// kept by the core for those words; they come as [`Said::Lyrics`].
    pub fn lyrics_nudge(&self, dir: i32) {
        let LyricsNow::Shown { song, pick } = self.lyrics.heard.now() else { return };
        let pick = self.core.lyrics_nudge(song.clone(), pick, dir);
        if self.lyrics.heard.retimed(song, pick) {
            (self.out)(Said::Lyrics(self.lyrics.heard.now()));
        }
    }

    /// What the lyrics view shows now.
    pub fn lyrics_now(&self) -> LyricsNow {
        self.lyrics.heard.now()
    }

    fn lyrics_fetch(&self) {
        if !self.lyrics.viewed.load(Ordering::Relaxed) {
            return;
        }
        if let Some(song) = self.lyrics.heard.fetch() {
            self.lyrics_look_up(song);
        }
    }

    fn lyrics_stop(&self) {
        if let Some((_, stop)) = self.lyrics.lookup.lock().take() {
            stop.abort();
        }
    }

    fn lyrics_look_up(&self, song: String) {
        let (client, heard, out) = (self.client.clone(), self.lyrics.heard.clone(), self.out.clone());
        let (stop, stopped) = AbortHandle::new_pair();
        if let Some((_, before)) = self.lyrics.lookup.lock().replace((song.clone(), stop)) {
            before.abort();
        }
        self.run(async move {
            let answers = Arc::new(Answers { song: song.clone(), heard: heard.clone(), out: out.clone() });
            let end = match Abortable::new(client.lyrics_for(song.clone(), answers), stopped).await {
                Ok(Ok(_)) => LyricsEnd::Done,
                Ok(Err(_)) => LyricsEnd::Failed,
                Err(_) => LyricsEnd::Stopped,
            };
            if let Some(now) = heard.ended(song, end) {
                out(Said::Lyrics(now));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use nori_core::settings::SavedServer;

    use super::*;
    use crate::session::{test_open, Open, Session};

    #[test]
    fn a_song_heard_is_looked_up_once_a_lyrics_view_opens() {
        let dir = nori_testdir::TempDir::new("host-lyrics");
        let queue = Arc::new(nori_core::queue::Session::new(nori_core::settings_store::Settings::new()));
        let profile = SavedServer { id: "own".into(), url: "http://own".into(), user: "ann".into(), ..Default::default() };
        let (tx, rx) = mpsc::channel();
        let out: Out = Arc::new(move |s| {
            if let Said::Lyrics(now) = s {
                let _ = tx.send(now);
            }
        });
        let card = Box::new(nori_engine::wav::WavOutput::new(dir.join("out.wav"), 1.0));
        let s = Session::open(Open { offline: true, out, ..test_open(queue, dir.path(), profile, card) }).unwrap();
        assert_eq!(s.lyrics_heard(Some("a".into())), LyricsNow::Looking { song: "a".into() });
        s.lyrics_view(true);
        let now = rx.recv_timeout(Duration::from_secs(10)).expect("the lookup ends");
        assert_eq!(now.song(), Some("a"));
        assert!(!matches!(now, LyricsNow::Looking { .. }), "{now:?}");
        s.close();
    }
}
