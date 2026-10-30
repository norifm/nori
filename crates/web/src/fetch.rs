//! The core's [`Transport`] over the worker's `fetch`.

use nori_core::transport::{Exchange, FailureKind, HttpHeader, Network, Transport, TransportError, TransportResponse};
use nori_tasks::OneThread;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response, WorkerGlobalScope};

/// The worker's `fetch`; `W` wakes the worker when a song's pushed bytes arrive (the player's
/// `ByteSource`), `()` where nothing plays.
pub struct Fetch<W = ()>(pub W);

fn failed(kind: FailureKind, e: JsValue) -> TransportError {
    TransportError::Failed { kind, detail: Some(format!("{e:?}")) }
}

/// The bytes at `url` as the worker's `fetch` answers them.
pub async fn fetch(x: Exchange) -> Result<TransportResponse, TransportError> {
    let init = RequestInit::new();
    init.set_method(x.method().name());
    let payload = x.payload();
    if let Some((body, _)) = payload {
        init.set_body(&js_sys::Uint8Array::from(body));
    }
    if x.timeout_ms > 0 {
        init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(x.timeout_ms)));
    }
    let req = Request::new_with_str_and_init(&x.url, &init).map_err(|e| failed(FailureKind::Other, e))?;
    for (k, v) in &x.headers {
        let _ = req.headers().set(k, v);
    }
    if let Some((_, Some(kind))) = payload {
        let _ = req.headers().set("Content-Type", kind);
    }
    let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
    let answer = JsFuture::from(scope.fetch_with_request(&req)).await.map_err(|e| failed(FailureKind::Connect, e))?;
    let answer: Response = answer.unchecked_into();
    let status = answer.status();
    let body = JsFuture::from(answer.array_buffer().map_err(|e| failed(FailureKind::Other, e))?).await.map_err(|e| failed(FailureKind::Other, e))?;
    // A browser keeps cookies itself and never shows them to a page, `set-cookie` among the headers.
    let mut headers = Vec::new();
    if let Ok(Some(entries)) = js_sys::try_iter(&answer.headers()) {
        for pair in entries.flatten() {
            let pair: js_sys::Array = pair.unchecked_into();
            if let (Some(name), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                headers.push(HttpHeader { name, value });
            }
        }
    }
    Ok(TransportResponse { status, body: js_sys::Uint8Array::new(&body).to_vec(), headers })
}

#[async_trait::async_trait]
impl<W: Send + Sync> Transport for Fetch<W> {
    async fn get(&self, url: String, timeout_ms: u32) -> Result<TransportResponse, TransportError> {
        OneThread(fetch(Exchange { url, timeout_ms, ..Default::default() })).await
    }

    async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
        OneThread(fetch(request)).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> Network {
        Network::Unmetered
    }
}
