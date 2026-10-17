//! Tasks on a browser's event loop: the thread the module runs on (a page or a worker), timers by
//! `setTimeout`.

use std::future::Future;
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
        Box::pin(OneThread(JsFuture::from(promise)))
    }
}

/// A future holding JS values, which are not `Send`: the module runs on one thread (no wasm threads),
/// so it is never sent anywhere.
struct OneThread<F>(F);

// SAFETY: wasm32-unknown-unknown without atomics has one thread.
unsafe impl<F> Send for OneThread<F> {}

impl<F: Future + Unpin> Future for OneThread<F> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut self.0).poll(cx).map(|_| ())
    }
}
