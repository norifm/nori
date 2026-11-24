//! Songs' bytes over the worker's `fetch`, streamed: each loader's fetching runs as a task on the
//! worker's event loop, asks for the range it wants and pushes each chunk of the response body as it
//! comes ([`Filling`]).

use std::future::poll_fn;

use js_sys::Uint8Array;
use nori_engine::{answer_span, Filling, Next, OpenError};
use nori_tasks::{Local, Tasks};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, ReadableStreamDefaultReader, ReadableStreamReadResult, Request, RequestInit, Response, WorkerGlobalScope};

/// Runs a loader's fetching as a task of the worker's; `wake` is called whenever bytes arrived.
pub fn push(fill: Filling, wake: impl Fn() + 'static) {
    wasm_bindgen_futures::spawn_local(run(fill, wake));
}

/// A response body being read, and how to call its request off.
struct Open {
    reader: ReadableStreamDefaultReader,
    abort: AbortController,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// Runs one loader's fetching until its song is let go.
async fn run(mut fill: Filling, wake: impl Fn()) {
    let mut body: Option<Open> = None;
    loop {
        let next = poll_fn(|cx| {
            let next = fill.poll_next(cx);
            if !fill.open() {
                body = None;
            }
            next
        })
        .await;
        let rest = match next {
            Next::Open(from) => match ask(fill.url(), from).await {
                Ok((open, start, len)) => {
                    body = Some(open);
                    fill.opened(from, Ok((start, len)))
                }
                Err(e) => fill.opened(from, Err(e)),
            },
            Next::Read(from) => {
                let reader = &body.as_ref().expect("a body is open").reader;
                match read(reader).await {
                    Ok(Some(bytes)) => fill.took(from, Ok(&bytes)),
                    Ok(None) => fill.took(from, Ok(&[])),
                    Err(e) => fill.took(from, Err(e)),
                }
            }
            Next::Wait | Next::Stop => return,
        };
        if !fill.open() {
            body = None;
        }
        wake();
        if let Some(d) = rest {
            Local.sleep(d).await;
        }
    }
}

fn failed(e: JsValue) -> OpenError {
    OpenError::Failed(format!("{e:?}"))
}

/// Asks for `url` from byte `from`: the body, where it starts and the resource's length.
async fn ask(url: &str, from: u64) -> Result<(Open, u64, Option<u64>), OpenError> {
    let abort = AbortController::new().map_err(failed)?;
    let init = RequestInit::new();
    init.set_signal(Some(&abort.signal()));
    let req = Request::new_with_str_and_init(url, &init).map_err(failed)?;
    if from > 0 {
        let _ = req.headers().set("Range", &format!("bytes={from}-"));
    }
    let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
    let answer: Response = JsFuture::from(scope.fetch_with_request(&req)).await.map_err(failed)?.unchecked_into();
    let header = |name: &str| answer.headers().get(name).ok().flatten();
    let (start, len) = answer_span(from, answer.status(), header("Content-Range").as_deref(), header("Content-Length").and_then(|l| l.parse().ok()))?;
    let stream = answer.body().ok_or_else(|| OpenError::Failed("an answer without a body".into()))?;
    let reader: ReadableStreamDefaultReader = stream.get_reader().unchecked_into();
    Ok((Open { reader, abort }, start, len))
}

/// The body's next chunk; None at its end.
async fn read(reader: &ReadableStreamDefaultReader) -> Result<Option<Vec<u8>>, String> {
    let got: ReadableStreamReadResult = JsFuture::from(reader.read()).await.map_err(|e| format!("{e:?}"))?.unchecked_into();
    if got.get_done().unwrap_or(false) {
        return Ok(None);
    }
    Ok(Some(Uint8Array::new(&got.get_value()).to_vec()))
}
