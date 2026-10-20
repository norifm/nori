//! Tasks on one thread of their own. It runs the tasks woken, fires the timers due, and otherwise parks
//! until the next timer or a wake: an idle executor costs no wakeups.
//!
//! A simulated executor ([`Native::simulated`]) keeps a clock of its own for tests: it stands still until
//! the test moves it to the next timer ([`Native::advance`]), once everything has settled
//! ([`Native::settle`]), so what happens never depends on how threads were scheduled.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;

use nori_time::Duration;

use crate::{Sleep, Task, Tasks};

/// An executor on a thread of its own, which ends once this is dropped and its tasks have ended.
pub struct Native {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    thread: OnceLock<Thread>,
    /// Told when the executor goes idle or outside work ends ([`Native::settle`]).
    settled: Condvar,
}

#[derive(Default)]
struct State {
    ready: VecDeque<Arc<Cell>>,
    /// By when each is due, µs on the executor's clock; the number tells apart timers due at once.
    timers: BTreeMap<(i64, u64), Waker>,
    next_timer: u64,
    /// Tasks spawned and not ended.
    live: usize,
    closed: bool,
    /// A simulated executor's clock, µs.
    simulated: Option<i64>,
    /// A task is being polled.
    polling: bool,
    /// Tasks polled so far: what ran while a test looked.
    polled: u64,
    /// Work outside the executor in progress ([`Native::busy`]).
    busy: u32,
}

impl State {
    /// The executor's clock, µs.
    fn now(&self) -> i64 {
        self.simulated.unwrap_or_else(|| nori_time::monotonic().as_micros() as i64)
    }

    /// Nothing runs or is due, inside or outside the executor.
    fn settled(&self) -> bool {
        let now = self.now();
        self.ready.is_empty() && !self.polling && self.busy == 0 && self.timers.keys().next().is_none_or(|(at, _)| *at > now)
    }
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

/// Real time a simulated executor may take to settle before a test fails rather than hangs.
const STUCK: std::time::Duration = std::time::Duration::from_secs(60);

/// Simulated time a test waits for something before it fails: longer than any of the core's waits.
const WAIT_LIMIT_US: i64 = 30 * 60 * 1_000_000;

impl Native {
    /// Starts the executor's thread, named `name`.
    pub fn new(name: &str) -> std::io::Result<Native> {
        Native::started(name, None)
    }

    /// An executor whose clock starts at `now_us` and moves only by [`Native::advance`].
    pub fn simulated(name: &str, now_us: i64) -> std::io::Result<Native> {
        Native::started(name, Some(now_us))
    }

    fn started(name: &str, simulated: Option<i64>) -> std::io::Result<Native> {
        let state = State { simulated, ..State::default() };
        let shared = Arc::new(Shared { state: Mutex::new(state), thread: OnceLock::new(), settled: Condvar::new() });
        let running = shared.clone();
        let thread = std::thread::Builder::new().name(name.into()).spawn(move || run(&running))?;
        let _ = shared.thread.set(thread.thread().clone());
        Ok(Native { shared })
    }

    /// Work on another thread that the executor's tasks will hear of: a simulated clock does not move
    /// until the guard is dropped.
    pub fn busy(&self) -> Busy {
        self.shared.state().busy += 1;
        Busy(Arc::downgrade(&self.shared))
    }

    /// Blocks until no task is ready or running, no timer is due and no [`Native::busy`] work goes on.
    pub fn settle(&self) {
        let started = std::time::Instant::now();
        let mut s = self.shared.state();
        while !s.settled() {
            assert!(started.elapsed() < STUCK, "the executor did not settle");
            s = self.shared.settled.wait_timeout(s, std::time::Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Moves a simulated clock to the next timer; false when none waits.
    pub fn advance(&self) -> bool {
        let mut s = self.shared.state();
        let Some(&(at, _)) = s.timers.keys().next() else { return false };
        let now = s.simulated.as_mut().expect("a simulated executor");
        *now = (*now).max(at);
        drop(s);
        self.shared.unpark();
        true
    }

    /// How long each timer waiting has left, soonest first.
    pub fn waits(&self) -> Vec<Duration> {
        let s = self.shared.state();
        let now = s.now();
        s.timers.keys().map(|(at, _)| Duration::from_micros((at - now).max(0) as u64)).collect()
    }

    /// Lets `by` pass on a simulated clock, timer by timer, then settles.
    pub fn pass(&self, by: Duration) {
        let until = self.now_us() + by.as_micros() as i64;
        loop {
            self.settle();
            let mut s = self.shared.state();
            let next = s.timers.keys().next().map(|(at, _)| *at).filter(|at| *at <= until);
            let now = s.simulated.as_mut().expect("a simulated executor");
            *now = next.unwrap_or(until).max(*now);
            drop(s);
            self.shared.unpark();
            if next.is_none() {
                self.settle();
                return;
            }
        }
    }

    /// Settles and moves a simulated clock from timer to timer until `ready` gives a value.
    pub fn until<T>(&self, what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
        let from = self.now_us();
        loop {
            self.settle();
            let polled = self.shared.state().polled;
            if let Some(v) = ready() {
                return v;
            }
            // Checking may have started work (a future polled here wakes a task), which may even have
            // run already: let it run and look again.
            let s = self.shared.state();
            if !s.settled() || s.polled != polled {
                continue;
            }
            drop(s);
            assert!(self.now_us() - from < WAIT_LIMIT_US, "waited half an hour for {what}");
            assert!(self.advance(), "nothing more will happen: waited for {what}");
        }
    }

    /// `task`'s output, polled on this thread while a simulated clock moves as [`Native::until`] does.
    pub fn run<T>(&self, what: &str, task: impl Future<Output = T>) -> T {
        let mut task = std::pin::pin!(task);
        let mut cx = Context::from_waker(Waker::noop());
        self.until(what, || match task.as_mut().poll(&mut cx) {
            Poll::Ready(v) => Some(v),
            Poll::Pending => None,
        })
    }
}

/// See [`Native::busy`].
pub struct Busy(Weak<Shared>);

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(shared) = self.0.upgrade() {
            shared.state().busy -= 1;
            shared.settled.notify_all();
        }
    }
}

/// See [`Tasks::waiting`]: busy again once dropped.
struct Waiting(Weak<Shared>);

impl Drop for Waiting {
    fn drop(&mut self) {
        if let Some(shared) = self.0.upgrade() {
            shared.state().busy += 1;
        }
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
        let at = self.shared.state().now() + after.as_micros() as i64;
        Box::pin(Timer { shared: Arc::downgrade(&self.shared), at, key: None })
    }

    fn now_us(&self) -> i64 {
        self.shared.state().simulated.unwrap_or_else(nori_time::device_us)
    }

    fn waiting(&self) -> Option<Box<dyn Send>> {
        let mut s = self.shared.state();
        s.simulated?;
        s.busy -= 1;
        drop(s);
        self.shared.settled.notify_all();
        Some(Box::new(Waiting(Arc::downgrade(&self.shared))))
    }

    fn block_on(&self, waited: Pin<&mut (dyn Future<Output = ()> + Send + '_)>) {
        if self.shared.state().simulated.is_some() {
            self.run("a blocking wait", waited)
        } else {
            crate::block_on(waited)
        }
    }
}

fn run(shared: &Shared) {
    loop {
        let cell = {
            let mut s = shared.state();
            let now = s.now();
            let due: Vec<Waker> = std::iter::from_fn(|| s.timers.first_entry().filter(|e| e.key().0 <= now).map(|e| e.remove())).collect();
            if !due.is_empty() {
                // Working until their tasks are queued, so nothing reads the executor as settled between.
                s.polling = true;
                drop(s);
                due.into_iter().for_each(Waker::wake);
                shared.state().polling = false;
                continue;
            }
            match s.ready.pop_front() {
                Some(cell) => {
                    s.polling = true;
                    s.polled += 1;
                    cell
                }
                None if s.closed && s.live == 0 => return,
                None => {
                    // A simulated clock moves only when the test moves it, and wakes the thread then.
                    let next = s.timers.keys().next().filter(|_| s.simulated.is_none()).map(|(at, _)| Duration::from_micros((at - now).max(0) as u64));
                    drop(s);
                    shared.settled.notify_all();
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
        let ended = future.as_mut().is_some_and(|f| f.as_mut().poll(&mut Context::from_waker(&waker)).is_ready());
        if ended {
            *future = None;
        }
        drop(future);
        let mut s = shared.state();
        s.polling = false;
        if ended {
            s.live -= 1;
        }
    }
}

/// [`Native`]'s [`Tasks::sleep`]. Ready at once once the executor is gone, so nothing waits on it forever.
struct Timer {
    shared: Weak<Shared>,
    at: i64,
    key: Option<(i64, u64)>,
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Some(shared) = self.shared.upgrade() else { return Poll::Ready(()) };
        let mut s = shared.state();
        if s.now() >= self.at {
            if let Some(key) = self.key.take() {
                s.timers.remove(&key);
            }
            return Poll::Ready(());
        }
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
    fn a_simulated_clock_moves_only_from_timer_to_timer() {
        let tasks = Arc::new(Native::simulated("test-simulated", 1_000).unwrap());
        let heard = Arc::new(Mutex::new(Vec::new()));
        for (s, word) in [(3_600, "late"), (20, "early")] {
            let (tasks2, heard) = (tasks.clone(), heard.clone());
            tasks.spawn(Box::pin(async move {
                tasks2.sleep(Duration::from_secs(s)).await;
                heard.lock().unwrap().push((word, tasks2.now_us()));
            }));
        }
        tasks.until("both", || (heard.lock().unwrap().len() == 2).then_some(()));
        assert_eq!(*heard.lock().unwrap(), [("early", 20_001_000), ("late", 3_600_001_000)]);
    }

    #[test]
    fn settling_waits_for_work_outside() {
        let tasks = Arc::new(Native::simulated("test-busy", 0).unwrap());
        let (go, went) = channel::<()>();
        let busy = tasks.busy();
        let done = Arc::new(Mutex::new(false));
        let done2 = done.clone();
        // The task hands work to a thread, which reports back only after the executor went idle.
        tasks.spawn(Box::pin(async move { go.send(()).unwrap() }));
        std::thread::spawn(move || {
            went.recv().unwrap();
            for _ in 0..1_000 {
                std::thread::yield_now();
            }
            *done2.lock().unwrap() = true;
            drop(busy);
        });
        tasks.settle();
        assert!(*done.lock().unwrap(), "settled with work outside going on");
    }

    #[test]
    fn waiting_moves_no_time_for_work_the_check_started() {
        let tasks = Arc::new(Native::simulated("test-check", 0).unwrap());
        let notify = Arc::new(Notify::default());
        let done = Arc::new(AtomicBool::new(false));
        let (n, d) = (notify.clone(), done.clone());
        tasks.spawn(Box::pin(async move {
            n.notified().await;
            d.store(true, Ordering::Release);
        }));
        let _later = tasks.sleep(Duration::from_secs(60));
        let mut checked = false;
        // The check wakes the task, which has run by the time the check answers.
        tasks.until("the task's work", || {
            if !std::mem::replace(&mut checked, true) {
                notify.notify();
                tasks.settle();
                return None;
            }
            done.load(Ordering::Acquire).then_some(())
        });
        assert_eq!(tasks.now_us(), 0, "time moved for work already done");
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
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while shared.strong_count() > 0 {
            assert!(std::time::Instant::now() < deadline, "{:?} still runs", thread.name());
            std::thread::yield_now();
        }
    }
}
