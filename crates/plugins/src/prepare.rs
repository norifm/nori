//! Song preparation and AutoMix analysis: the enabled plugins exporting `preparation` or `automix`
//! hear the songs as the player reads them (the one decode), on a thread of their own. The player's
//! tap copies samples into a feed made when the song starts and never waits: what the feed can't take
//! is dropped, and the plugin is told where the samples it gets start. The thread wakes when a feed is
//! half full and at the song's end, and sleeps while no song is read. What a plugin makes is kept in
//! memory for the last few songs: prepared data goes to its sound stage, analysis data to its `refine`.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

use nori_model::Song;
use nori_player::pcm::Format;
use nori_player::stage::{Preparing, Tap};

use crate::convert::song_out;
use crate::Worker;

/// Seconds of samples a feed holds.
const FEED_S: usize = 8;
/// Songs whose data is kept.
const KEPT: usize = 4;

/// A plugin that hears songs: its instance for this work, and which of the two points it exports.
pub(crate) struct Hearer {
    pub(crate) id: String,
    pub(crate) worker: Worker,
    pub(crate) prepares: bool,
    pub(crate) analyses: bool,
}

/// What plugins made of songs: song id, plugin id, data.
type Made = VecDeque<(String, String, Arc<[u8]>)>;

type Wake = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

/// Finds a song's record by id (the open profile's).
pub type Songs = Arc<dyn Fn(&str) -> Option<Song> + Send + Sync>;

/// The plugins that hear songs, and what they made. Made at first need, while one is enabled.
pub struct Preparation {
    to: Mutex<mpsc::Sender<Job>>,
    prepared: Arc<Mutex<Made>>,
    analysed: Arc<Mutex<Made>>,
    /// The song being heard, told when it is done.
    hearing: Arc<(Mutex<Option<String>>, Condvar)>,
    wake: Wake,
    songs: Songs,
}

/// A song to hear: its record, format, first frame and feed.
struct Job {
    song: Song,
    format: Format,
    from: u64,
    feed: Arc<Feed>,
}

/// Samples on their way from the player's tap to the thread.
struct Feed {
    ring: Mutex<Ring>,
    ready: Condvar,
    /// No plugin wants the song: the tap copies nothing.
    unwanted: AtomicBool,
}

struct Ring {
    samples: Vec<f32>,
    len: usize,
    /// The frame of the first sample held.
    at: u64,
    ended: bool,
    left: bool,
}

impl Feed {
    fn new(format: Format, from: u64) -> Feed {
        let ring = Ring { samples: vec![0.0; format.rate as usize * format.channels.max(1) * FEED_S], len: 0, at: from, ended: false, left: false };
        Feed { ring: Mutex::new(ring), ready: Condvar::new(), unwanted: AtomicBool::new(false) }
    }
}

/// The player's end of a feed, and the frame of the next samples it is given.
struct FeedTap {
    feed: Arc<Feed>,
    next: u64,
    channels: usize,
}

impl Tap for FeedTap {
    fn samples(&mut self, interleaved: &[f32]) {
        let at = self.next;
        self.next += (interleaved.len() / self.channels) as u64;
        if self.feed.unwanted.load(Ordering::Relaxed) {
            return;
        }
        // The thread may hold it for a moment: those samples are dropped, never waited for.
        let Ok(mut r) = self.feed.ring.try_lock() else { return };
        if r.len == 0 {
            r.at = at;
        }
        // After a gap what is held goes first; the samples start anew once it is taken.
        if r.at + (r.len / self.channels) as u64 == at && r.len + interleaved.len() <= r.samples.len() {
            let from = r.len;
            r.samples[from..from + interleaved.len()].copy_from_slice(interleaved);
            r.len += interleaved.len();
        }
        if r.len * 2 >= r.samples.len() {
            self.feed.ready.notify_one();
        }
    }

    fn end(&mut self) {
        if let Ok(mut r) = self.feed.ring.lock() {
            r.ended = true;
        }
        self.feed.ready.notify_one();
    }
}

impl Drop for FeedTap {
    fn drop(&mut self) {
        if let Ok(mut r) = self.feed.ring.lock() {
            r.left = !r.ended;
        }
        self.feed.ready.notify_one();
    }
}

impl Preparation {
    /// Over `hearers`, finding songs with `songs`.
    pub(crate) fn start(hearers: Vec<Hearer>, songs: Songs) -> Arc<Preparation> {
        let (to, jobs) = mpsc::channel::<Job>();
        let (prepared, analysed, wake): (Arc<Mutex<Made>>, Arc<Mutex<Made>>, Wake) = Default::default();
        let hearing: Arc<(Mutex<Option<String>>, Condvar)> = Arc::default();
        let (p, a, w, h) = (prepared.clone(), analysed.clone(), wake.clone(), hearing.clone());
        let _ = std::thread::Builder::new().name("nori-prepare".into()).spawn(move || {
            for job in jobs {
                *h.0.lock().expect("not poisoned") = Some(job.song.id.clone());
                hear(&hearers, job, &p, &a, &w);
                *h.0.lock().expect("not poisoned") = None;
                h.1.notify_all();
            }
        });
        Arc::new(Preparation { to: Mutex::new(to), prepared, analysed, hearing, wake, songs })
    }

    /// Plugin `plugin`'s AutoMix analysis data for song `id`, if it made any; while the song is still
    /// being heard, waited for at most `budget`.
    pub(crate) fn analysis(&self, id: &str, plugin: &str, budget: std::time::Duration) -> Option<Vec<u8>> {
        let (hearing, done) = &*self.hearing;
        let g = hearing.lock().expect("not poisoned");
        drop(done.wait_timeout_while(g, budget, |h| h.as_deref() == Some(id)).expect("not poisoned"));
        self.analysed.lock().expect("not poisoned").iter().find(|(s, p, _)| s == id && p == plugin).map(|(_, _, d)| d.to_vec())
    }
}

/// Keeps `data`, the newest of [`KEPT`] songs'.
fn keep(made: &Mutex<Made>, song: &str, plugin: &str, data: Vec<u8>) {
    let mut m = made.lock().expect("not poisoned");
    m.retain(|(s, p, _)| !(s == song && p == plugin));
    m.push_back((song.to_string(), plugin.to_string(), data.into()));
    while m.iter().map(|(s, _, _)| s).collect::<BTreeSet<_>>().len() > KEPT {
        let oldest = m.front().map(|(s, _, _)| s.clone());
        m.retain(|(s, _, _)| Some(s) != oldest.as_ref());
    }
}

/// Runs one song through the plugins that want it.
fn hear(hearers: &[Hearer], job: Job, prepared: &Mutex<Made>, analysed: &Mutex<Made>, wake: &Wake) {
    let Job { song, format, from, feed } = job;
    let (rate, channels) = (format.rate, format.channels.max(1) as u8);
    let mut wanting: Vec<(&Hearer, bool, bool)> = Vec::new();
    for h in hearers {
        let s = song_out(&song);
        let prepares = h.prepares && nori_tasks::block_on(h.worker.call(move |st, p| p.nori_plugin_preparation().call_begin(st, &s, rate, channels))).unwrap_or(false);
        let s = song_out(&song);
        // An analysis is of a whole song.
        let analyses = from == 0 && h.analyses && nori_tasks::block_on(h.worker.call(move |st, p| p.nori_plugin_automix().call_begin_analysis(st, &s, rate))).unwrap_or(false);
        if prepares || analyses {
            wanting.push((h, prepares, analyses));
        }
    }
    if wanting.is_empty() {
        feed.unwanted.store(true, Ordering::Relaxed);
        return;
    }
    let ch = channels as usize;
    let mut block = Vec::new();
    loop {
        let (at, ended, left) = {
            let mut r = feed.ring.lock().expect("not poisoned");
            while r.len * 2 < r.samples.len() && !r.ended && !r.left {
                r = feed.ready.wait(r).expect("not poisoned");
            }
            block.clear();
            let len = r.len;
            block.extend_from_slice(&r.samples[..len]);
            let at = r.at;
            r.at += (len / ch) as u64;
            r.len = 0;
            (at, r.ended, r.left)
        };
        if !block.is_empty() {
            for (h, prepares, analyses) in &wanting {
                if *prepares {
                    let b = block.clone();
                    nori_tasks::block_on(h.worker.call(move |st, p| p.nori_plugin_preparation().call_feed(st, &b, at)));
                }
                if *analyses {
                    let b = block.clone();
                    nori_tasks::block_on(h.worker.call(move |st, p| p.nori_plugin_automix().call_feed_analysis(st, &b)));
                }
            }
        }
        if left {
            return;
        }
        if ended {
            break;
        }
    }
    let mut came = false;
    for (h, prepares, analyses) in wanting {
        if prepares {
            if let Some(data) = nori_tasks::block_on(h.worker.call(|st, p| p.nori_plugin_preparation().call_finish(st))).flatten() {
                keep(prepared, &song.id, &h.id, data);
                came = true;
            }
        }
        if analyses {
            if let Some(data) = nori_tasks::block_on(h.worker.call(|st, p| p.nori_plugin_automix().call_finish_analysis(st))).flatten() {
                keep(analysed, &song.id, &h.id, data);
            }
        }
    }
    let told = wake.lock().expect("not poisoned").clone();
    if let (true, Some(w)) = (came, told) {
        w();
    }
}

impl Preparing for Preparation {
    fn tap(&self, id: &str, format: Format, from: u64) -> Option<Box<dyn Tap>> {
        let song = (self.songs)(id)?;
        let feed = Arc::new(Feed::new(format, from));
        self.to.lock().expect("not poisoned").send(Job { song, format, from, feed: feed.clone() }).ok()?;
        Some(Box::new(FeedTap { feed, next: from, channels: format.channels.max(1) }))
    }

    fn prepared(&self, id: &str) -> Vec<(String, Arc<[u8]>)> {
        self.prepared.lock().expect("not poisoned").iter().filter(|(s, _, _)| s == id).map(|(_, p, d)| (p.clone(), d.clone())).collect()
    }

    fn on_prepared(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock().expect("not poisoned") = Some(wake);
    }
}
