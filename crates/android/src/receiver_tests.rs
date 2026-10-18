use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use nori_core::client::Client;
use nori_net::transport::{Exchange, Network, Transport, TransportError, TransportResponse};

struct NetworkUnused;

#[async_trait::async_trait]
impl Transport for NetworkUnused {
    async fn get(&self, _: String, _: u32) -> Result<TransportResponse, TransportError> {
        panic!("prefetch without a queue must not use the network")
    }
    async fn send(&self, _: Exchange) -> Result<TransportResponse, TransportError> {
        panic!("prefetch without a queue must not use the network")
    }
    fn address_changed(&self) {}
    fn network(&self) -> Network { Network::Unmetered }
}

#[test]
fn async_call_keeps_its_receiver() {
    let dir = nori_testdir::TempDir::new("async-receiver");
    let core = nori_core::Core::new(dir.path().join("nori.db").to_string_lossy().into(), "profile".into(), Default::default()).unwrap();
    let client = Client::new(core, Arc::new(NetworkUnused), Default::default());
    let weak = Arc::downgrade(&client);
    let handle = Arc::into_raw(client);
    // The successful entry creates a future without accessing JNIEnv; the receiver handle is valid.
    let future = unsafe {
        let handle = super::Java_uniffi_Scaffolding_methodNoriCoreClientLyricsPrefetch(std::ptr::null_mut(), std::ptr::null_mut(), handle as i64);
        Arc::from_raw(handle as *const super::UniffiRustFuture<()>)
    };
    // Closing the Kotlin object releases its handle while the async operation remains outstanding.
    unsafe { Arc::decrement_strong_count(handle) };
    assert!(weak.upgrade().is_some(), "the pending future must own its receiver");
    assert_eq!(future.future.lock().unwrap().as_mut().poll(&mut Context::from_waker(Waker::noop())), Poll::Ready(()));
    drop(future);
    assert!(weak.upgrade().is_none(), "the receiver must be released with the completed future");
}

#[test]
fn a_freed_call_lets_go_of_what_it_waited_on() {
    let dir = nori_testdir::TempDir::new("freed-call");
    let core = nori_core::Core::new(dir.path().join("nori.db").to_string_lossy().into(), "profile".into(), Default::default()).unwrap();
    let weak = Arc::downgrade(&core);
    let handle = Arc::into_raw(core);
    // Waits on the core's download marks, which keep the future's waker: a Kotlin caller that let go
    // of the call must not leave it waiting there.
    let future = unsafe { super::Java_uniffi_Scaffolding_methodNoriCoreCoreDownloadMarksMoved(std::ptr::null_mut(), std::ptr::null_mut(), handle as i64) };
    unsafe { Arc::decrement_strong_count(handle) };
    let waiting = unsafe {
        let ptr = future as *const super::UniffiRustFuture<()>;
        Arc::increment_strong_count(ptr);
        Arc::from_raw(ptr)
    };
    let waker = Waker::from(waiting.clone());
    assert_eq!(waiting.future.lock().unwrap().as_mut().poll(&mut Context::from_waker(&waker)), Poll::Pending);
    drop((waker, waiting));
    unsafe {
        super::Java_uniffi_Scaffolding_rustFutureCancel0(std::ptr::null_mut(), std::ptr::null_mut(), future);
        super::Java_uniffi_Scaffolding_rustFutureFree0(std::ptr::null_mut(), std::ptr::null_mut(), future);
    }
    assert!(weak.upgrade().is_none(), "the freed call must drop its future, and the core with it");
}
