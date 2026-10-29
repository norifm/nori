//! nori's events for the `hooks` export. An event is built only when an enabled plugin's manifest names
//! it: [`Plugins::hears`] is one load of a bit set made when plugins are turned on or off, so a client
//! checks it before it builds anything. The player's state comes in as [`Moment`]s, from which the
//! playback, song, queue and route events are told apart here, once for every client. Lyric lines
//! are woken here too, a wake per line, only while a plugin hears them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nori_time::Duration;

use crate::convert::song_out;
use crate::manifest::{Hook, Permission};
use crate::runtime::points::hooks::Event;
use crate::session::Moment;
use crate::timers::before;
use crate::{Plugins, Value};

/// A media button, as the client's media session heard it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Press {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    Stop,
}

impl Press {
    fn name(self) -> &'static str {
        match self {
            Press::PlayPause => "play-pause",
            Press::Play => "play",
            Press::Pause => "pause",
            Press::Next => "next",
            Press::Previous => "previous",
            Press::Stop => "stop",
        }
    }
}

/// What the hooks keep between events.
#[derive(Default)]
pub(crate) struct Live {
    /// The enabled plugins' hooks, one bit per [`Hook`].
    heard: AtomicU32,
    /// Some enabled plugin reads what is playing or hears an event about it.
    follows: AtomicBool,
    /// The last moment, and when it was said on the executor's clock (µs).
    moment: Mutex<(Moment, i64)>,
    queue_rev: Mutex<Option<u64>>,
    /// The screen's state was asked of the client.
    watching_screen: AtomicBool,
    /// The settings' changes were asked of the client.
    watching_settings: AtomicBool,
    lyrics: Mutex<Lyrics>,
    /// Bumped by every moment: a lyric line's wake for an older one lets go.
    lyric_turn: AtomicU64,
}

/// A song's synced lyrics: each line's start ms and words.
type Lines = Arc<Vec<(u32, String)>>;

/// The synced lyrics of the songs heard, by id; None while being fetched or for a song without.
#[derive(Default)]
struct Lyrics(HashMap<String, Option<Lines>>);

/// How far a position said may be from where the last moment ran on to and still be on course.
const ON_COURSE_MS: u64 = 1000;

fn bit(h: Hook) -> u32 {
    1 << Hook::ALL.iter().position(|x| *x == h).expect("every hook is in ALL")
}

impl Plugins {
    /// Reads again what the enabled plugins hear and want, after one was turned on or off, installed or
    /// removed: the hooks' bits, the timers, the wake receivers and commands the client keeps on, and
    /// the Bluetooth watch.
    pub(crate) fn refresh(&self) {
        let on: Vec<_> = self.installed().into_iter().filter(|i| i.enabled).collect();
        let heard = on.iter().flat_map(|i| Hook::ALL.into_iter().filter(|h| i.manifest.hears(*h))).fold(0, |b, h| b | bit(h));
        self.live.heard.store(heard, Ordering::Relaxed);
        self.live.follows.store(heard != 0 || on.iter().any(|i| i.manifest.may(Permission::PlaybackRead)), Ordering::Relaxed);
        for i in self.installed().iter().filter(|i| !i.enabled) {
            self.timers.stop_all(&i.manifest.id);
            self.gate.idle(&i.manifest.id);
        }
        self.gate.changed.notify();
        self.watch_screen();
        let settings = self.hears(Hook::SettingChanged);
        if self.live.watching_settings.swap(settings, Ordering::Relaxed) != settings {
            self.side.watch_settings(settings);
        }
        self.wake_again();
        self.rewatch();
    }

    /// Tells the client whether a timer needs the screen's state, when that changed.
    pub(crate) fn watch_screen(&self) {
        let on = self.timers.watch_screen();
        if self.live.watching_screen.swap(on, Ordering::Relaxed) != on {
            self.side.watch_screen(on);
        }
    }

    /// Hands `event` to plugin `id`'s hooks, starting its instance if it has none.
    pub(crate) fn tell_one(&self, id: &str, event: Event) {
        if !self.installed().iter().any(|i| i.enabled && i.manifest.id == id && i.manifest.extends("hooks")) {
            return;
        }
        if let Some(w) = self.worker(id) {
            w.post(move |s, p| p.nori_plugin_hooks().call_on(s, &event));
        }
    }

    /// Hands the event `make` builds to every enabled plugin that hears `hook`; built only then.
    fn tell(&self, hook: Hook, make: impl FnOnce() -> Event) {
        self.tell_but(hook, &[], make);
    }

    /// [`Plugins::tell`], but not to the plugins in `told`, which heard of it already.
    pub(crate) fn tell_but(&self, hook: Hook, told: &[String], make: impl FnOnce() -> Event) {
        if !self.hears(hook) {
            return;
        }
        let hearing: Vec<String> = self.installed().into_iter().filter(|i| i.enabled && i.manifest.hears(hook) && !told.contains(&i.manifest.id)).map(|i| i.manifest.id.clone()).collect();
        if hearing.is_empty() {
            return;
        }
        let event = make();
        for id in hearing {
            self.tell_one(&id, event.clone());
        }
    }

    fn lyric_lines(&self, song: &str) -> Option<Option<Lines>> {
        self.live.lyrics.lock().expect("not poisoned").0.get(song).cloned()
    }

    /// Wakes at the next lyric line of the moment's song while it plays; fetches its lyrics first.
    fn follow_lyrics(&self) {
        let turn = self.live.lyric_turn.fetch_add(1, Ordering::Relaxed) + 1;
        self.lyrics_changed.notify();
        let (m, at) = self.live.moment.lock().expect("not poisoned").clone();
        let (Some(id), true) = (m.song.clone(), m.playing) else { return };
        let lines = match self.lyric_lines(&id) {
            Some(Some(lines)) => lines,
            Some(None) => return,
            None => {
                self.live.lyrics.lock().expect("not poisoned").0.insert(id.clone(), None);
                let (me, library) = (self.me.clone(), self.library());
                let fetch = move || {
                    let song = library.as_ref().and_then(|l| l.songs(std::slice::from_ref(&id)).ok()?.into_iter().next());
                    let lines = song.and_then(|s| library?.synced_lyrics(&s)).filter(|l| !l.is_empty()).map(Arc::new);
                    let Some(p) = me.upgrade() else { return };
                    let had = lines.is_some();
                    p.live.lyrics.lock().expect("not poisoned").0.insert(id, lines);
                    if had && p.live.lyric_turn.load(Ordering::Relaxed) == turn {
                        p.follow_lyrics();
                    }
                };
                let _ = std::thread::Builder::new().name("nori-plugin-lyrics".into()).spawn(fetch);
                return;
            }
        };
        let (tasks, me, changed) = (self.tasks(), self.me.clone(), self.lyrics_changed.clone());
        let task = async move {
            let gone = || me.upgrade().is_none_or(|p| p.live.lyric_turn.load(Ordering::Relaxed) != turn);
            let here = |now: i64| m.position_ms as i64 + (now - at) / 1000;
            let mut next = lines.partition_point(|(start, _)| (*start as i64) < here(tasks.now_us()));
            while let Some((start, words)) = lines.get(next) {
                let wait = Duration::from_millis((*start as i64 - here(tasks.now_us())).max(0) as u64);
                if !before(tasks.sleep(wait), changed.until(gone)).await || gone() {
                    return;
                }
                let Some(p) = me.upgrade() else { return };
                let words = words.clone();
                p.tell(Hook::LyricLine, || Event::LyricLine((next as u32, words)));
                next += 1;
            }
        };
        self.tasks().spawn(Box::pin(task));
    }

    /// nori's queue moved, played or paused during a cast whose device plays the songs (a press here, a
    /// plugin's `session`): its plugin hears it as the media buttons `jump:<list index>`,
    /// `seek:<ms>`, `play` and `pause`; not what only repeats what the device said itself.
    fn tell_cast(&self, c: &crate::Cast, now: &Moment, was: &Moment, ran: u64) {
        let mut heard = Vec::new();
        if now.index != was.index {
            heard.extend(now.index.map(|i| format!("jump:{i}")));
        } else if now.position_ms.abs_diff(ran) >= ON_COURSE_MS && now.position_ms.abs_diff(c.position_ms) >= ON_COURSE_MS {
            heard.push(format!("seek:{}", now.position_ms));
        }
        if now.playing != was.playing && now.playing != c.playing {
            heard.push((if now.playing { "play" } else { "pause" }).to_string());
        }
        for h in heard {
            self.tell_one(&c.plugin, Event::MediaButton(h));
        }
    }

    /// The moment's position now, as it runs on while playing.
    pub(crate) fn moment(&self) -> (Moment, u64) {
        let (m, at) = self.live.moment.lock().expect("not poisoned").clone();
        let ran = if m.playing { ((self.tasks().now_us() - at).max(0) / 1000) as u64 } else { 0 };
        let position = m.position_ms + ran;
        (m, position)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// Whether an enabled plugin hears `hook`: a client checks before it builds the event.
    pub fn hears(&self, hook: Hook) -> bool {
        self.live.heard.load(Ordering::Relaxed) & bit(hook) != 0
    }

    /// Whether an enabled plugin follows the player, or a cast's device plays the songs: only then does
    /// a client say its [`Moment`]s.
    pub fn follows(&self) -> bool {
        self.live.follows.load(Ordering::Relaxed) || self.lan.cast().is_some_and(|c| c.kind == crate::CastKind::Target)
    }

    /// The player as it is now: playing or paused, the song, the queue, the route; said when any of
    /// them changes or a seek lands, while [`Plugins::follows`].
    pub fn played(&self, moment: Moment) {
        let now = self.tasks().now_us();
        let (was, ran) = self.moment();
        // Only on as it was going: no change (a client may say each position it hears).
        let on_course = Moment { position_ms: moment.position_ms, ..was.clone() } == moment && moment.position_ms.abs_diff(ran) < ON_COURSE_MS;
        let queue_same = || {
            let kept = *self.live.queue_rev.lock().expect("not poisoned");
            !self.hears(Hook::QueueChanged) || (kept.is_some() && kept == self.library().map(|l| l.queue_rev()))
        };
        if let Some(c) = self.lan.cast().filter(|c| c.kind == crate::CastKind::Target) {
            self.tell_cast(&c, &moment, &was, ran);
        }
        if on_course && queue_same() {
            return;
        }
        *self.live.moment.lock().expect("not poisoned") = (moment.clone(), now);
        self.gate.playing(moment.playing);
        if moment.playing != was.playing {
            self.tell(if moment.playing { Hook::PlaybackStarted } else { Hook::PlaybackPaused }, || if moment.playing { Event::PlaybackStarted } else { Event::PlaybackPaused });
        }
        if (moment.song.as_ref(), moment.index) != (was.song.as_ref(), was.index) {
            let library = self.library();
            self.tell(Hook::SongChanged, || {
                let song = moment.song.as_ref().zip(library).and_then(|(id, l)| l.songs(std::slice::from_ref(id)).ok()?.into_iter().next());
                Event::SongChanged(song.as_ref().map(song_out))
            });
        }
        if self.hears(Hook::QueueChanged) {
            if let Some(rev) = self.library().map(|l| l.queue_rev()) {
                let before = self.live.queue_rev.lock().expect("not poisoned").replace(rev);
                if before.is_some_and(|b| b != rev) {
                    self.tell(Hook::QueueChanged, || Event::QueueChanged);
                }
            }
        }
        if moment.route() != was.route() {
            self.tell(Hook::RouteChanged, || Event::RouteChanged(moment.route().into()));
        }
        if self.hears(Hook::LyricLine) {
            self.follow_lyrics();
        }
    }

    /// The screen went on or off (asked of a client only while a timer needs it).
    pub fn screen(&self, on: bool) {
        self.gate.screen(on);
    }

    /// The player's screen shows or not: `while-visible` timers fire only while it does.
    pub fn player_shown(&self, shown: bool) {
        self.gate.player_shown(shown);
    }

    pub fn favorite_changed(&self, id: String, on: bool) {
        self.tell(Hook::FavoriteChanged, || Event::FavoriteChanged((id, on)));
    }

    /// A profile opened: `id` is nori's id of it.
    pub fn profile_opened(&self, id: String) {
        *self.live.queue_rev.lock().expect("not poisoned") = None;
        self.tell(Hook::ProfileOpened, || Event::ProfileOpened(id));
    }

    /// An output device (`name`) connected or went.
    pub fn device(&self, name: String, connected: bool) {
        self.tell(Hook::DeviceConnected, || if connected { Event::DeviceConnected(name) } else { Event::DeviceDisconnected(name) });
    }

    /// A media key pressed, told to the plugins that hear them.
    pub fn heard_press(&self, press: Press) {
        self.tell(Hook::MediaButton, || Event::MediaButton(press.name().into()));
    }
}

impl Plugins {
    /// nori's setting `key` changed to `value`, stored or in effect.
    pub(crate) fn setting_changed(&self, key: &str, value: Value) {
        self.tell(Hook::SettingChanged, || Event::SettingChanged((key.to_string(), value.into())));
    }
}
