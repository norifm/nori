//! Where the core's async work runs. A [`Tasks`] spawns futures and makes timers: natively [`Native`],
//! one thread that parks while no task is ready and no timer is due; in a browser `Local`, the page's
//! event loop and `setTimeout`. Work waits on futures (a request, a [`Tasks::sleep`], a [`Notify`]),
//! picks the first of two with [`first`], and is cancelled by dropping what it waits on.
//!
//! A task's futures must not block the thread they are polled on: a request answers from the
//! platform's own threads or event loop and wakes its task then.

use std::future::Future;
use std::pin::{pin, Pin};
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use nori_time::Duration;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod native;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use native::Native;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::{Local, OneThread};

/// A spawned piece of work.
pub type Task = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// A timer: ready once its time has passed. Dropping it cancels it.
pub type Sleep = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Runs tasks and keeps time for them.
pub trait Tasks: Send + Sync {
    /// Runs `task` until it ends.
    fn spawn(&self, task: Task);

    /// Ready once `after` has passed on this executor's clock.
    fn sleep(&self, after: Duration) -> Sleep;
}

/// Wakes every task waiting on it. A [`Notify::notified`] made before a notice is ready after it, so a
/// task makes it, checks what it waits for, then awaits it, and misses nothing in between.
#[derive(Default)]
pub struct Notify {
    /// Notices so far, and the tasks waiting for the next.
    inner: Mutex<(u64, Vec<Waker>)>,
}

impl Notify {
    pub fn notify(&self) {
        let waiting = {
            let mut n = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            n.0 += 1;
            std::mem::take(&mut n.1)
        };
        waiting.into_iter().for_each(Waker::wake);
    }

    /// Ready at the next notice after now.
    pub fn notified(&self) -> Notified<'_> {
        let seen = self.inner.lock().unwrap_or_else(|e| e.into_inner()).0;
        Notified { notify: self, seen }
    }
}

/// See [`Notify::notified`].
pub struct Notified<'a> {
    notify: &'a Notify,
    seen: u64,
}

impl Future for Notified<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut n = self.notify.inner.lock().unwrap_or_else(|e| e.into_inner());
        if n.0 != self.seen {
            return Poll::Ready(());
        }
        if !n.1.iter().any(|w| w.will_wake(cx.waker())) {
            n.1.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// Which of [`first`]'s futures was ready.
#[derive(Debug, PartialEq, Eq)]
pub enum Either<A, B> {
    Left(A),
    Right(B),
}

/// The first of `a` and `b` to be ready (`a` when both are); the other is dropped.
pub async fn first<A: Future, B: Future>(a: A, b: B) -> Either<A::Output, B::Output> {
    let (mut a, mut b) = (pin!(a), pin!(b));
    std::future::poll_fn(|cx| {
        if let Poll::Ready(v) = a.as_mut().poll(cx) {
            return Poll::Ready(Either::Left(v));
        }
        b.as_mut().poll(cx).map(Either::Right)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::task::Wake;

    struct Count(std::sync::atomic::AtomicUsize);

    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    fn a_notice_reaches_what_was_made_before_it() {
        let notify = Notify::default();
        let woken = Arc::new(Count(Default::default()));
        let waker = Waker::from(woken.clone());
        let mut cx = Context::from_waker(&waker);
        let mut before = pin!(notify.notified());
        assert!(before.as_mut().poll(&mut cx).is_pending());
        assert!(before.as_mut().poll(&mut cx).is_pending(), "polled twice, it waits once");
        notify.notify();
        assert_eq!(woken.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(before.as_mut().poll(&mut cx).is_ready());
        let mut after = pin!(notify.notified());
        assert!(after.as_mut().poll(&mut cx).is_pending(), "a notice is not kept for what comes after it");
    }
}
