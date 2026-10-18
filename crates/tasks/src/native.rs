//! Tasks on one thread of their own. It runs the tasks woken, fires the timers due, and otherwise parks
//! until the next timer or a wake: an idle executor costs no wakeups.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;

use nori_time::{Duration, Instant};

use crate::{Sleep, Task, Tasks};

/// An executor on a thread of its own, which ends once this is dropped and its tasks have ended.
pub struct Native {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    thread: OnceLock<Thread>,
}

#[derive(Default)]
struct State {
    ready: VecDeque<Arc<Cell>>,
    /// By when each is due; the number tells apart timers due at once.
    timers: BTreeMap<(Instant, u64), Waker>,
    next_timer: u64,
    /// Tasks spawned and not ended.
    live: usize,
    closed: bool,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn unpark(&self) {
        if let Some(t) = self.thread.get() {
            t.unpark();
        }
    }
}

/// A spawned task; ready to run while queued.
struct Cell {
    future: Mutex<Option<Task>>,
    queued: AtomicBool,
    shared: Weak<Shared>,
}

impl Wake for Cell {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if self.queued.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(shared) = self.shared.upgrade() {
            shared.state().ready.push_back(self.clone());
            shared.unpark();
        }
    }
}

impl Native {
    /// Starts the executor's thread, named `name`.
    pub fn new(name: &str) -> std::io::Result<Native> {
        let shared = Arc::new(Shared { state: Mutex::default(), thread: OnceLock::new() });
        let running = shared.clone();
        let thread = std::thread::Builder::new().name(name.into()).spawn(move || run(&running))?;
        let _ = shared.thread.set(thread.thread().clone());
        Ok(Native { shared })
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.shared.state().closed = true;
        self.shared.unpark();
    }
}

impl Tasks for Native {
    fn spawn(&self, task: Task) {
        let cell = Arc::new(Cell { future: Mutex::new(Some(task)), queued: AtomicBool::new(false), shared: Arc::downgrade(&self.shared) });
        self.shared.state().live += 1;
        cell.wake_by_ref();
    }

    fn sleep(&self, after: Duration) -> Sleep {
        Box::pin(Timer { shared: Arc::downgrade(&self.shared), at: Instant::now() + after, key: None })
    }
}

fn run(shared: &Shared) {
    loop {
        let cell = {
            let mut s = shared.state();
            let now = Instant::now();
            let due: Vec<Waker> = std::iter::from_fn(|| s.timers.first_entry().filter(|e| e.key().0 <= now).map(|e| e.remove())).collect();
            if !due.is_empty() {
                drop(s);
                due.into_iter().for_each(Waker::wake);
                continue;
            }
            match s.ready.pop_front() {
                Some(cell) => cell,
                None if s.closed && s.live == 0 => return,
                None => {
                    let next = s.timers.keys().next().map(|(at, _)| at.saturating_duration_since(now));
                    drop(s);
                    match next {
                        Some(wait) => std::thread::park_timeout(wait),
                        None => std::thread::park(),
                    }
                    continue;
                }
            }
        };
        cell.queued.store(false, Ordering::Release);
        let waker = Waker::from(cell.clone());
        let mut future = cell.future.lock().unwrap_or_else(|e| e.into_inner());
        if future.as_mut().is_some_and(|f| f.as_mut().poll(&mut Context::from_waker(&waker)).is_ready()) {
            *future = None;
            shared.state().live -= 1;
        }
    }
}

/// [`Native`]'s [`Tasks::sleep`]. Ready at once once the executor is gone, so nothing waits on it forever.
struct Timer {
    shared: Weak<Shared>,
    at: Instant,
    key: Option<(Instant, u64)>,
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Some(shared) = self.shared.upgrade() else { return Poll::Ready(()) };
        if Instant::now() >= self.at {
            if let Some(key) = self.key.take() {
                shared.state().timers.remove(&key);
            }
            return Poll::Ready(());
        }
        let mut s = shared.state();
        let key = match self.key {
            Some(key) => key,
            None => {
                s.next_timer += 1;
                (self.at, s.next_timer)
            }
        };
        self.key = Some(key);
        s.timers.insert(key, cx.waker().clone());
        let earliest = s.timers.keys().next() == Some(&key);
        drop(s);
        // The thread may be parked until a later timer.
        if earliest {
            shared.unpark();
        }
        Poll::Pending
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let (Some(key), Some(shared)) = (self.key, self.shared.upgrade()) {
            shared.state().timers.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Notify;
    use std::sync::mpsc::channel;

    #[test]
    fn tasks_run_and_timers_fire_in_order() {
        let tasks = Arc::new(Native::new("test").unwrap());
        let (said, heard) = channel();
        for (ms, word) in [(60, "late"), (20, "early")] {
            let (tasks2, said) = (tasks.clone(), said.clone());
            tasks.spawn(Box::pin(async move {
                tasks2.sleep(Duration::from_millis(ms)).await;
                said.send(word).unwrap();
            }));
        }
        let got: Vec<_> = (0..2).map(|_| heard.recv_timeout(std::time::Duration::from_secs(5)).unwrap()).collect();
        assert_eq!(got, ["early", "late"]);
    }

    #[test]
    fn a_notice_wakes_a_waiting_task() {
        let tasks = Arc::new(Native::new("test").unwrap());
        let notify = Arc::new(Notify::default());
        let (said, heard) = channel();
        let n = notify.clone();
        let (ready, started) = channel();
        tasks.spawn(Box::pin(async move {
            let notified = n.notified();
            ready.send(()).unwrap();
            notified.await;
            said.send(()).unwrap();
        }));
        started.recv().unwrap();
        notify.notify();
        assert!(heard.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn a_task_running_outlives_its_executor() {
        let tasks = Native::new("test-outlives").unwrap();
        let notify = Arc::new(Notify::default());
        let (said, heard) = channel();
        let (n, (ready, started)) = (notify.clone(), channel());
        tasks.spawn(Box::pin(async move {
            let notified = n.notified();
            ready.send(()).unwrap();
            notified.await;
            said.send(()).unwrap();
        }));
        started.recv().unwrap();
        drop(tasks);
        notify.notify();
        assert!(heard.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "the task ended with its executor");
    }

    #[test]
    fn the_thread_ends_with_its_executor() {
        let tasks = Native::new("test-ends").unwrap();
        let thread = tasks.shared.thread.get().unwrap().clone();
        let shared = Arc::downgrade(&tasks.shared);
        drop(tasks);
        let deadline = Instant::now() + Duration::from_secs(5);
        while shared.strong_count() > 0 {
            assert!(Instant::now() < deadline, "{:?} still runs", thread.name());
            std::thread::yield_now();
        }
    }
}
