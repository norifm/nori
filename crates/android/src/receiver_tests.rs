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
