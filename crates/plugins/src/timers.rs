//! A plugin's timers (`timers`), on the plugin host's executor: each sleeps on the executor's clock and
//! wakes the plugin's `timer` hook. Whether one may fire is the [`Gate`]'s: a `while-visible` timer only
//! while the player is on screen; an `always` one as often as it asks while the plugin has an active
//! session (a cast, a connected device), otherwise only with `schedule`, at most once per
//! [`SCHEDULE_PERIOD`] and never while music plays with the screen off. A timer that may not fire
//! sleeps on no clock: it waits for the gate to open, so it costs no wake.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::task::Poll;

use nori_tasks::Notify;
use nori_time::Duration;

use crate::manifest::Permission;
use crate::runtime::nori::plugin::timers;
use crate::runtime::points::hooks::Event;
use crate::runtime::Host;
use crate::Plugins;

/// The most often a plugin's timers fire without an active session.
const SCHEDULE_PERIOD: Duration = Duration::from_secs(15 * 60);

/// What decides whether a timer may fire now, told whenever it changes.
pub(crate) struct Gate {
    screen_on: AtomicBool,
    player_shown: AtomicBool,
    playing: AtomicBool,
    /// Plugins with an active session.
    active: Mutex<BTreeSet<String>>,
    pub(crate) changed: Notify,
}

impl Default for Gate {
    fn default() -> Gate {
        Gate { screen_on: AtomicBool::new(true), player_shown: AtomicBool::new(false), playing: AtomicBool::new(false), active: Mutex::default(), changed: Notify::default() }
    }
}

impl Gate {
    fn set(&self, flag: &AtomicBool, on: bool) {
        if flag.swap(on, Ordering::Relaxed) != on {
            self.changed.notify();
        }
    }

    pub(crate) fn screen(&self, on: bool) {
        self.set(&self.screen_on, on);
    }

    pub(crate) fn player_shown(&self, shown: bool) {
        self.set(&self.player_shown, shown);
    }

    pub(crate) fn playing(&self, playing: bool) {
        self.set(&self.playing, playing);
    }

    pub(crate) fn active(&self, plugin: &str, on: bool) {
        let mut a = self.active.lock().expect("not poisoned");
        let changed = if on { a.insert(plugin.to_string()) } else { a.remove(plugin) };
        drop(a);
        if changed {
            self.changed.notify();
        }
    }

    fn is_active(&self, plugin: &str) -> bool {
        self.active.lock().expect("not poisoned").contains(plugin)
    }

    /// Whether `plugin`'s timer of kind `when` may fire now; `schedule`: its manifest grants it.
    fn allows(&self, plugin: &str, when: timers::When, schedule: bool) -> bool {
        match when {
            timers::When::WhileVisible => self.screen_on.load(Ordering::Relaxed) && self.player_shown.load(Ordering::Relaxed),
            timers::When::Always => self.is_active(plugin) || (schedule && !(self.playing.load(Ordering::Relaxed) && !self.screen_on.load(Ordering::Relaxed))),
        }
    }

    /// Whether the screen state matters to a timer now: the client watches it only then.
    fn watches_screen(when: timers::When) -> bool {
        when == timers::When::Always
    }
}

/// The timers running, by plugin and id: each one's generation, so a timer started again or stopped
/// lets its old task go.
#[derive(Default)]
pub(crate) struct Timers {
    running: Mutex<HashMap<(String, u32), (u64, timers::When)>>,
    next: AtomicU64,
    /// When each plugin's timer last fired without an active session, on the executor's clock, µs.
    scheduled: Mutex<HashMap<String, i64>>,
}

impl Timers {
    fn current(&self, key: &(String, u32), generation: u64) -> bool {
        self.running.lock().expect("not poisoned").get(key).is_some_and(|(g, _)| *g == generation)
    }

    /// Stops every timer of `plugin` (turned off or removed).
    pub(crate) fn stop_all(&self, plugin: &str) {
        self.running.lock().expect("not poisoned").retain(|(p, _), _| p != plugin);
    }

    /// Whether any timer needs the screen's state.
    pub(crate) fn watch_screen(&self) -> bool {
        self.running.lock().expect("not poisoned").values().any(|(_, w)| Gate::watches_screen(*w))
    }
}

/// `a` until it is ready, unless `b` is first: then false.
pub(crate) async fn before(a: impl Future<Output = ()>, b: impl Future<Output = ()>) -> bool {
    let (mut a, mut b) = (pin!(a), pin!(b));
    std::future::poll_fn(|cx| match (a.as_mut().poll(cx), b.as_mut().poll(cx)) {
        (Poll::Ready(()), _) => Poll::Ready(true),
        (_, Poll::Ready(())) => Poll::Ready(false),
        _ => Poll::Pending,
    })
    .await
}

impl Plugins {
    /// Plugin `id`'s timer `timer`: after `ms`, and every `ms` after when `repeat`.
    fn start_timer(&self, id: &str, timer: u32, ms: u32, repeat: bool, when: timers::When, schedule: bool) {
        let key = (id.to_string(), timer);
        let generation = self.timers.next.fetch_add(1, Ordering::Relaxed);
        self.timers.running.lock().expect("not poisoned").insert(key.clone(), (generation, when));
        // A timer started again lets its old task go now, not at its old time.
        self.gate.changed.notify();
        self.watch_screen();
        let (tasks, gate, me) = (self.tasks(), self.gate.clone(), self.me.clone());
        let period = Duration::from_millis(ms.max(1) as u64);
        let task = async move {
            let mut left = period;
            loop {
                let live = || me.upgrade().is_some_and(|p| p.timers.current(&key, generation));
                let allowed = || gate.allows(&key.0, when, schedule);
                gate.changed.until(|| !live() || allowed()).await;
                if !live() {
                    return;
                }
                // Without a session, no sooner than a period after its last scheduled wake.
                let alone = when == timers::When::Always && !gate.is_active(&key.0);
                let wait = match alone {
                    true => me.upgrade().map_or(left, |p| p.timers.scheduled.lock().expect("not poisoned").get(&key.0).map_or(left, |last| left.max(Duration::from_micros((last + SCHEDULE_PERIOD.as_micros() as i64 - tasks.now_us()).max(0) as u64)))),
                    false => left,
                };
                let from = tasks.now_us();
                if !before(tasks.sleep(wait), gate.changed.until(|| !live() || !allowed())).await {
                    left = wait.saturating_sub(Duration::from_micros((tasks.now_us() - from).max(0) as u64));
                    continue;
                }
                let Some(p) = me.upgrade().filter(|p| p.timers.current(&key, generation)) else { return };
                if alone {
                    p.timers.scheduled.lock().expect("not poisoned").insert(key.0.clone(), tasks.now_us());
                }
                p.tell_one(&key.0, Event::Timer(key.1));
                if !repeat {
                    p.timers.running.lock().expect("not poisoned").remove(&key);
                    p.watch_screen();
                    return;
                }
                left = period;
            }
        };
        self.tasks().spawn(Box::pin(task));
    }

    fn stop_timer(&self, id: &str, timer: u32) {
        self.timers.running.lock().expect("not poisoned").remove(&(id.to_string(), timer));
        self.gate.changed.notify();
        self.watch_screen();
    }

    /// Plugin `id` has an active session (a cast, a connected device) or not: its `always` timers fire
    /// as often as they ask while it has.
    pub fn session_active(&self, id: &str, on: bool) {
        self.gate.active(id, on);
    }
}

impl timers::Host for Host {
    fn start(&mut self, id: u32, ms: u32, repeat: bool, when: timers::When) {
        let schedule = self.manifest.may(Permission::Schedule);
        if let Some(p) = self.plugins() {
            p.start_timer(&self.id, id, ms, repeat, when, schedule);
        }
    }

    fn stop(&mut self, id: u32) {
        if let Some(p) = self.plugins() {
            p.stop_timer(&self.id, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timer_fires_only_when_its_kind_and_the_plugins_session_allow() {
        use timers::When::{Always, WhileVisible};
        // when, active session, schedule, playing, screen on, player shown: allowed
        let table = [
            (Always, true, false, true, false, false, true),
            (Always, false, false, false, true, false, false),
            (Always, false, true, false, false, false, true),
            (Always, false, true, true, true, false, true),
            (Always, false, true, true, false, false, false),
            (WhileVisible, true, true, false, true, false, false),
            (WhileVisible, false, false, true, true, true, true),
            (WhileVisible, false, false, false, false, true, false),
        ];
        for (when, active, schedule, playing, screen, shown, allowed) in table {
            let g = Gate::default();
            g.active("p", active);
            g.playing(playing);
            g.screen(screen);
            g.player_shown(shown);
            assert_eq!(g.allows("p", when, schedule), allowed, "{when:?} active {active} schedule {schedule} playing {playing} screen {screen} shown {shown}");
        }
    }
}

