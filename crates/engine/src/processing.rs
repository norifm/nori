//! Post-download work, so playing a downloaded song does none: its lyrics looked up, and its analysis
//! read back from disk when it was not measured as it arrived (an MP4, a resumed download, an outdated
//! analysis). The beat model reads only songs about to play (`core::listen_to`).
//!
//! What a song needs is the core's (`nori_core::transfers::needs_analysis`). Songs queue in one line worked by one
//! lowest-priority thread, one decode per song, living only while a song is ready. A song still being
//! measured as it arrives waits until that ends ([`Analyses::kick`]). Lyrics have a line and a thread of
//! their own, one lookup at a time, as the core times them in a lane of their own. Progress phases are
//! the core's (`transfers`).

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

use nori_core::transfers::{self, Saved, Work};
use nori_core::Core;
use parking_lot::{Condvar, Mutex};

use crate::core::{decode, Analyses, Decoded, Shelf};

/// Saved songs waiting to be read back, and whether a thread works through them.
#[derive(Debug, Default)]
struct Line {
    ids: VecDeque<String>,
    running: bool,
}

impl Line {
    /// Appends new `ids`; returns whether a thread should start.
    fn add(&mut self, ids: impl IntoIterator<Item = String>) -> bool {
        for id in ids {
            if !self.ids.contains(&id) {
                self.ids.push_back(id);
            }
        }
        self.start()
    }

    /// Whether a thread should start (none runs and something waits).
    fn start(&mut self) -> bool {
        if self.running || self.ids.is_empty() {
            return false;
        }
        self.running = true;
        true
    }

    /// The first song not `busy` (still measured as it arrives). None ends the thread; [`Analyses::kick`]
    /// restarts it.
    fn next(&mut self, busy: impl Fn(&str) -> bool) -> Option<String> {
        match self.ids.iter().position(|id| !busy(id)) {
            Some(i) => self.ids.remove(i),
            None => {
                self.running = false;
                None
            }
        }
    }

    fn idle(&self) -> bool {
        self.ids.is_empty() && !self.running
    }
}

/// The read-back: where downloads are read from (set once), the songs waiting for it and for their
/// lyrics, and the threads' ends.
#[derive(Default)]
pub(crate) struct ReadBack {
    shelf: OnceLock<Box<dyn Shelf>>,
    line: Mutex<Line>,
    lyrics: Mutex<Line>,
    /// Signalled when a thread ends, for [`Analyses::wait`].
    ended: Condvar,
    lyrics_ended: Condvar,
}

/// `id`'s state once saved; `measuring`: it is being measured as it arrives.
fn saved_of(core: &Core, id: &str, measuring: bool) -> Saved {
    let analysable = nori_core::queue::analysable(id);
    Saved { analysable, measuring, analysed: analysable && core.analysis_missing(vec![id.to_string()]).is_ok_and(|m| m.is_empty()) }
}

impl Analyses {
    /// Sets where downloads are read back from; the first call wins.
    pub fn install(&self, shelf: Box<dyn Shelf>) {
        let _ = self.read_back.shelf.set(shelf);
    }

    /// `ids` just finished downloading: looks up their lyrics, marks what else each needs and queues those
    /// needing a read-back.
    pub fn saved(self: &Arc<Self>, ids: Vec<String>) {
        if let Some(client) = self.client() {
            let waiting: Vec<String> = ids.iter().filter(|id| client.core().transfers().with(|t| t.waits(id, Work::Lyrics))).cloned().collect();
            if self.read_back.lyrics.lock().add(waiting) {
                self.spawn_lyrics();
            }
        }
        self.plan(ids, true);
    }

    /// "Analyse downloaded songs" (`Core::download_unanalysed`): as [`Analyses::saved`] without lyrics.
    /// Returns how many were queued.
    pub fn analyse(self: &Arc<Self>, ids: Vec<String>) -> u32 {
        self.plan(ids, false)
    }

    fn plan(self: &Arc<Self>, ids: Vec<String>, download: bool) -> u32 {
        // Without a shelf nothing is marked, so nothing waits forever.
        let (Some(_), Some(client)) = (self.read_back.shelf.get(), self.client()) else { return 0 };
        let core = client.core();
        let mut line = Vec::new();
        for id in ids {
            let measuring = self.arrivals.lock().has(&id);
            let analysis = transfers::needs_analysis(saved_of(core, &id, measuring));
            if core.transfers().with(|t| t.plan(&id, analysis, download.then_some(analysis && !measuring))) {
                line.push(id);
            }
        }
        let n = line.len() as u32;
        if n > 0 {
            nori_core::alog::info(&format!("{n} saved songs to read back"));
            if self.read_back.line.lock().add(line) {
                self.spawn();
            }
        }
        n
    }

    /// A song's measuring as it arrived ended: resume the line.
    pub(crate) fn kick(self: &Arc<Self>) {
        if self.read_back.shelf.get().is_some() && self.read_back.line.lock().start() {
            self.spawn();
        }
    }

    /// Blocks until both lines are empty (scripts and tests).
    pub fn wait(&self) {
        let mut line = self.read_back.line.lock();
        while !line.idle() {
            self.read_back.ended.wait(&mut line);
        }
        drop(line);
        let mut lyrics = self.read_back.lyrics.lock();
        while !lyrics.idle() {
            self.read_back.lyrics_ended.wait(&mut lyrics);
        }
    }

    fn spawn_lyrics(self: &Arc<Self>) {
        let me = self.clone();
        if std::thread::Builder::new().name("nori-lyrics".into()).spawn(move || me.look_up_lyrics()).is_err() {
            self.read_back.lyrics.lock().running = false;
        }
    }

    /// Looks up the lyrics of the songs in line, one at a time, those that still wait for them.
    fn look_up_lyrics(&self) {
        loop {
            let next = self.read_back.lyrics.lock().next(|_| false);
            let Some(id) = next else { break };
            let Some(client) = self.client() else { continue };
            if client.core().transfers().with(|t| t.waits(&id, Work::Lyrics)) {
                nori_core::transport::block_on(client.lyrics_for_download(id));
            }
        }
        self.read_back.lyrics_ended.notify_all();
    }

    fn spawn(self: &Arc<Self>) {
        let me = self.clone();
        if std::thread::Builder::new().name("nori-process".into()).spawn(move || me.run()).is_err() {
            self.read_back.line.lock().running = false;
        }
    }

    fn run(&self) {
        crate::arriving::lower_priority();
        loop {
            let next = self.read_back.line.lock().next(|id| self.arrivals.lock().has(id));
            let Some(id) = next else { break };
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.process(&id)));
            if let Err(p) = done {
                nori_core::alog::info(&format!("reading {id} back: {}", crate::panic_words(&*p)));
                if let Some(c) = self.client() {
                    finish(c.core(), &id);
                }
            }
        }
        self.read_back.ended.notify_all();
    }

    /// Reads `id` back for its pending analysis.
    fn process(&self, id: &str) {
        let Some(client) = self.client() else { return };
        let core = client.core();
        let tracker = core.transfers();
        if !tracker.with(|t| t.waits(id, Work::Analysis)) {
            return;
        }
        // It may have been measured elsewhere since it was saved.
        if core.analysis_missing(vec![id.to_string()]).unwrap_or_default().is_empty() {
            return finish(core, id);
        }
        let shelf = self.read_back.shelf.get().expect("a song is lined up only once a shelf is installed");
        let Some((pieces, hint)) = shelf.whole(id).and_then(|w| Some((crate::pieces::Pieces::open(&w.files).ok()?, w.hint))) else {
            nori_core::alog::info(&format!("reading {id} back: not whole on the disk"));
            return finish(core, id);
        };
        tracker.with(|t| t.working(id, Work::Analysis));
        let song = core.download_song(id);
        let expected_ms = song.as_ref().map_or(0, |s| s.duration as i64 * 1000);
        let hint = hint.or_else(|| song.map(|s| s.suffix).filter(|s| !s.is_empty()));
        let cpu = crate::arriving::thread_cpu_ms();
        // Abandoned once it no longer waits (cancelled or timed out).
        let decoded = decode(id, "reading back", pieces, hint.as_deref(), expected_ms, true, false, || tracker.with(|t| t.waits(id, Work::Analysis)));
        let Some(Decoded { stream, .. }) = decoded else { return };
        let a = stream.and_then(|s| core.analysis_finish_whole(id, s, expected_ms).ok().flatten());
        nori_core::alog::info(&match &a {
            Some(t) => format!("analysed {id} from the disk: {:.2} bpm (conf {:.2}, stab {:.2})", t.bpm, t.bpm_confidence, t.stability),
            None => format!("analysed {id} from the disk: not stored"),
        });
        finish(core, id);
        if let (Some(a), Some(b)) = (cpu, crate::arriving::thread_cpu_ms()) {
            nori_core::alog::info(&format!("reading {id} back took {} ms of CPU", b.saturating_sub(a)));
        }
        if a.is_some() {
            // Replan what was planned without it.
            let measurers = self.arrivals.lock().measurers();
            for m in measurers {
                m.stored_elsewhere();
            }
        }
    }
}

/// Marks `id`'s analysis after its download done.
fn finish(core: &Core, id: &str) {
    core.transfers().with(|t| t.work_done(id, Work::Analysis));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn line_drains_skipping_busy() {
        let mut line = Line::default();
        assert!(!line.start(), "nothing to do: no thread");
        assert!(line.add(ids(&["a", "b", "c"])), "the first songs start a thread");
        assert!(!line.add(ids(&["d", "b"])), "one is running: the songs join its line, b keeps its place");
        assert_eq!(line.ids, ids(&["a", "b", "c", "d"]));
        let arriving = |id: &str| id == "b";
        let mut order = Vec::new();
        while let Some(id) = line.next(arriving) {
            order.push(id);
            assert!(line.running, "one thread, one song at a time");
        }
        assert_eq!(order, ["a", "c", "d"], "b is still measured as it comes: left for later");
        assert!(!line.running, "nothing ready: the thread ends");
        assert!(!line.idle(), "b still waits");
        // Its measuring ended: the thread starts again for it.
        assert!(line.start());
        assert!(!line.start(), "not twice");
        assert_eq!(line.next(|_| false), Some("b".to_string()));
        assert_eq!(line.next(|_| false), None);
        assert!(line.idle(), "nothing lingers");
    }
}
