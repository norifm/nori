//! Tasks on a browser's event loop: the thread the module runs on (a page or a worker), timers by
//! `setTimeout`.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use nori_time::Duration;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::{Sleep, Task, Tasks};

/// The event loop of the thread the module runs on.
pub struct Local;

impl Tasks for Local {
    fn spawn(&self, task: Task) {
        wasm_bindgen_futures::spawn_local(task);
    }

    fn sleep(&self, after: Duration) -> Sleep {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let global = js_sys::global();
            let set = js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout")).map(|f| f.unchecked_into::<js_sys::Function>());
            if let Ok(set) = set {
                let _ = set.call2(&global, &resolve, &JsValue::from_f64(after.as_millis() as f64));
            }
        });
        let timer = OneThread(JsFuture::from(promise));
        Box::pin(async move {
            let _ = timer.await;
        })
    }
}

/// A value holding JS values, which are not `Send`: the module runs on one thread (no wasm threads),
/// so it is never sent or shared anywhere.
#[derive(Clone)]
pub struct OneThread<T>(pub T);

// SAFETY: wasm32-unknown-unknown without atomics has one thread.
unsafe impl<T> Send for OneThread<T> {}
unsafe impl<T> Sync for OneThread<T> {}

impl<T> Deref for OneThread<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for OneThread<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<F: Future> Future for OneThread<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: the inner future is never moved out of the pinned wrapper.
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
    }
}
