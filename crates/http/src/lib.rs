//! Desktop HTTP: the core's [`Transport`] and the engine's [`ByteSource`] over one ureq agent (rustls),
//! so API calls, covers and audio share one connection pool.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use nori_core::transport::{Exchange, FailureKind, Transport, TransportError, TransportResponse, USER_AGENT};
use nori_engine::{Body, ByteSource, Cancel, OpenError};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{Buffers, ConnectProxyConnector, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, RustlsConnector};
use ureq::{Agent, RequestBuilder};

/// Largest API response body read.
const MAX_ANSWER: u64 = 256 * 1024 * 1024;

pub struct Http {
    agent: Agent,
}

impl Http {
    pub fn new() -> Arc<Http> {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .build();
        let connector = ().chain(ConnectProxyConnector::default()).chain(Sockets).chain(RustlsConnector::default());
        let agent = Agent::with_parts(config, connector, DefaultResolver::default());
        Arc::new(Http { agent })
    }

    /// `url`'s total length from a `bytes=0-0` request; None unless the server answers 206.
    fn total_length(&self, url: &str) -> Option<u64> {
        let r = self.agent.get(url).header("Range", "bytes=0-0").call().ok()?;
        if r.status().as_u16() != 206 {
            return None;
        }
        r.headers().get("content-range")?.to_str().ok().and_then(content_range)?.1
    }

    /// GETs `url` from byte `from` on, up to `end` (exclusive) when given.
    fn open_now(&self, url: &str, from: u64, end: Option<u64>) -> Result<Body, OpenError> {
        let mut req = self.agent.get(url);
        match end {
            Some(end) => req = req.header("Range", format!("bytes={from}-{}", end.saturating_sub(1))),
            None if from > 0 => req = req.header("Range", format!("bytes={from}-")),
            None => {}
        }
        let r = req.call().map_err(|e| e.to_string())?;
        let status = r.status().as_u16();
        let header = |name: &str| r.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        // Range past the end (the length was an estimate): report the real length, asking for it when
        // a proxy dropped Content-Range.
        if status == 416 && from > 0 {
            let len = header("content-range").as_deref().and_then(unsatisfied_range).or_else(|| self.total_length(url)).filter(|&l| l <= from);
            return Err(OpenError::PastEnd { len });
        }
        if !(200..300).contains(&status) {
            return Err(OpenError::Status(status));
        }
        let (start, len) = match header("content-range").as_deref().and_then(content_range) {
            Some(r) if status == 206 => r,
            _ => (0, header("content-length").and_then(|l| l.parse().ok())),
        };
        let reader: Box<dyn Read + Send> = Box::new(r.into_body().into_reader());
        Ok(Body { start, len, reader })
    }
}

fn failure(e: ureq::Error) -> TransportError {
    use std::io::ErrorKind;
    let kind = match &e {
        ureq::Error::HostNotFound => FailureKind::UnknownHost,
        ureq::Error::ConnectionFailed => FailureKind::Connect,
        ureq::Error::Timeout(_) => FailureKind::Timeout,
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => FailureKind::Tls,
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => FailureKind::Other,
        ureq::Error::Io(io) => match io.kind() {
            ErrorKind::ConnectionRefused => FailureKind::Connect,
            ErrorKind::TimedOut => FailureKind::Timeout,
            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::UnexpectedEof => FailureKind::Interrupted,
            // std's lookup failure has no kind of its own: a name that cannot be resolved (no network).
            _ if io.to_string().starts_with("failed to lookup address") => FailureKind::UnknownHost,
            _ => FailureKind::Io,
        },
        _ => FailureKind::Io,
    };
    TransportError::Failed { kind, detail: Some(e.to_string()) }
}

/// Parses `bytes 100-199/1000` into (start, total length).
fn content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = v.strip_prefix("bytes ")?.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

/// Parses the total length out of a 416's `bytes */1000`.
fn unsatisfied_range(v: &str) -> Option<u64> {
    v.strip_prefix("bytes ")?.trim().strip_prefix("*/")?.trim().parse().ok()
}

/// Adds `headers` and a whole-request timeout (0 keeps the agent's) to `req`. That timeout is also the
/// wait for the answer: a held poll answers only when there is news.
fn configured<B>(mut req: RequestBuilder<B>, headers: &HashMap<String, String>, timeout_ms: u32) -> RequestBuilder<B> {
    for (name, value) in headers {
        req = req.header(name, value);
    }
    if timeout_ms > 0 {
        let timeout = Some(Duration::from_millis(timeout_ms as u64));
        req = req.config().timeout_global(timeout).timeout_recv_response(timeout).build();
    }
    req
}

fn response(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<TransportResponse, TransportError> {
    let r = r.map_err(failure)?;
    let status = r.status().as_u16();
    let set_cookies = r.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).map(str::to_string).collect();
    let body = r.into_body().with_config().limit(MAX_ANSWER).read_to_vec().map_err(failure)?;
    Ok(TransportResponse { status, body, set_cookies })
}

/// Each request runs on a thread of its own ([`Call`]), so dropping its future ends it.
#[async_trait::async_trait]
impl Transport for Http {
    async fn get(&self, url: String, timeout_ms: u32) -> Result<TransportResponse, TransportError> {
        let agent = self.agent.clone();
        Call::run(move || response(configured(agent.get(&url), &HashMap::new(), timeout_ms).call())).await
    }

    async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
        let agent = self.agent.clone();
        Call::run(move || {
            let (headers, timeout) = (&request.headers, request.timeout_ms);
            let post = |kind, body: &String| configured(agent.post(&request.url).header("Content-Type", kind), headers, timeout).send(body.as_bytes());
            response(match (&request.json, &request.form) {
                (Some(json), _) => post("application/json", json),
                (None, Some(form)) => post("application/x-www-form-urlencoded", form),
                (None, None) => configured(agent.get(&request.url), headers, timeout).call(),
            })
        })
        .await
    }

    fn address_changed(&self) {}

    fn network(&self) -> nori_core::transport::Network {
        nori_core::transport::Network::Unmetered
    }
}

type Answer = Result<TransportResponse, TransportError>;

/// A core request on a thread of its own. Its future waits without blocking the caller; dropped before
/// the answer, it shuts the request's socket, so a held poll ends at once rather than when it is answered.
#[derive(Default)]
struct Call(Mutex<CallState>);

#[derive(Default)]
struct CallState {
    answer: Option<Answer>,
    waker: Option<Waker>,
    /// The socket the request uses now.
    socket: Option<TcpStream>,
    given_up: bool,
}

impl Call {
    fn run(request: impl FnOnce() -> Answer + Send + 'static) -> Waiting {
        let call = Arc::new(Call::default());
        let on = call.clone();
        let spawned = std::thread::Builder::new().name("nori-http".into()).spawn(move || {
            CALL.with(|c| *c.borrow_mut() = Some(on.clone()));
            let answer = request();
            let mut s = on.state();
            s.socket = None;
            s.answer = Some(answer);
            if let Some(w) = s.waker.take() {
                w.wake();
            }
        });
        if let Err(e) = spawned {
            call.state().answer = Some(Err(TransportError::Failed { kind: FailureKind::Other, detail: Some(e.to_string()) }));
        }
        Waiting(call)
    }

    fn state(&self) -> MutexGuard<'_, CallState> {
        lock(&self.0)
    }

    /// The request goes on over `stream`; Err once it was given up.
    fn uses(&self, stream: &TcpStream) -> io::Result<()> {
        let mut s = self.state();
        if s.given_up {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "given up"));
        }
        s.socket = Some(stream.try_clone()?);
        Ok(())
    }
}

/// A [`Call`]'s answer to come.
struct Waiting(Arc<Call>);

impl Future for Waiting {
    type Output = Answer;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Answer> {
        let mut s = self.0.state();
        match s.answer.take() {
            Some(a) => Poll::Ready(a),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut s = self.0.state();
        if s.answer.is_none() {
            s.given_up = true;
            if let Some(socket) = s.socket.take() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
}

thread_local! {
    /// The core request this thread runs: a [`Call`]'s thread.
    static CALL: RefCell<Option<Arc<Call>>> = const { RefCell::new(None) };

    /// The engine request this thread opens or reads. Thread-local because ureq pools connections
    /// across requests and has no per-call hook into the transport.
    static REQUEST: RefCell<Option<Arc<Request>>> = const { RefCell::new(None) };
}

/// An engine request ([`ByteSource::open_cancellable`]): cancelling it shuts the socket it uses.
struct Request {
    cancel: Cancel,
}

impl Request {
    /// Runs `f` as this thread's current request.
    fn during<R>(self: &Arc<Self>, f: impl FnOnce() -> R) -> R {
        let before = REQUEST.with(|r| r.replace(Some(self.clone())));
        let r = f();
        REQUEST.with(|r| *r.borrow_mut() = before);
        r
    }
}

/// Opens TCP connections as [`Socket`]s (ureq's own TCP connector keeps its socket to itself); a CONNECT
/// proxy's connection, made before, goes on as it is.
#[derive(Debug)]
struct Sockets;

impl<In: ureq::unversioned::transport::Transport> Connector<In> for Sockets {
    type Out = Either<In, Socket>;

    fn connect(&self, details: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        if let Some(c) = chained {
            return Ok(Some(Either::A(c)));
        }
        let timeout = details.timeout.not_zero().map(|t| *t);
        let mut failed = None;
        for addr in &details.addrs {
            let connected = match timeout {
                Some(t) => TcpStream::connect_timeout(addr, t),
                None => TcpStream::connect(addr),
            };
            match connected {
                Ok(stream) => {
                    stream.set_nodelay(details.config.no_delay())?;
                    let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
                    return Ok(Some(Either::B(Socket { stream, buffers, call: Weak::new(), request: Arc::default(), read_timeout: None, write_timeout: None })));
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => failed = Some(ureq::Error::Timeout(ureq::Timeout::Connect)),
                Err(e) => failed = Some(e.into()),
            }
        }
        Err(failed.unwrap_or(ureq::Error::ConnectionFailed))
    }
}

/// A TCP connection that gives the [`Call`] using it a handle to shut it down.
#[derive(Debug)]
struct Socket {
    stream: TcpStream,
    buffers: LazyBuffers,
    /// The call last given the handle.
    call: Weak<Call>,
    /// The engine request using it now; a request's cancel shuts it only while this is that request.
    request: Arc<Mutex<Weak<Request>>>,
    /// The socket's timeouts as last set, so they are set only when they change.
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

/// Sets `timeout` through `set` unless `now` already is it.
fn set_timeout(now: &mut Option<Duration>, timeout: NextTimeout, set: impl FnOnce(Option<Duration>) -> io::Result<()>) -> io::Result<()> {
    let wanted = timeout.not_zero().map(|t| *t);
    if *now != wanted {
        set(wanted)?;
        *now = wanted;
    }
    Ok(())
}

impl Socket {
    /// Gives this thread's call (if any) the handle, once; Err when that call was given up meanwhile.
    fn in_call(&mut self) -> io::Result<()> {
        CALL.with(|c| {
            let Some(call) = c.borrow().clone() else { return Ok(()) };
            if self.call.as_ptr() != Arc::as_ptr(&call) {
                call.uses(&self.stream)?;
                self.call = Arc::downgrade(&call);
            }
            Ok(())
        })
    }

    /// Lets this thread's engine request (if any) shut the socket when it is cancelled.
    fn in_request(&mut self) -> io::Result<()> {
        let now = REQUEST.with(|r| r.borrow().as_ref().map_or_else(Weak::new, Arc::downgrade));
        let mut using = lock(&self.request);
        if using.ptr_eq(&now) {
            return Ok(());
        }
        *using = now.clone();
        drop(using);
        let Some(request) = now.upgrade() else { return Ok(()) };
        let (stream, using) = (self.stream.try_clone()?, self.request.clone());
        request.cancel.on_cancel(move || {
            if lock(&using).ptr_eq(&now) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        Ok(())
    }

    fn in_use(&mut self) -> io::Result<()> {
        self.in_call()?;
        self.in_request()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn timed_out(e: io::Error, timeout: NextTimeout) -> ureq::Error {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ureq::Error::Timeout(timeout.reason),
        _ => e.into(),
    }
}

impl ureq::unversioned::transport::Transport for Socket {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.in_use()?;
        set_timeout(&mut self.write_timeout, timeout, |t| self.stream.set_write_timeout(t))?;
        self.stream.write_all(&self.buffers.output()[..amount]).map_err(|e| timed_out(e, timeout))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.in_use()?;
        set_timeout(&mut self.read_timeout, timeout, |t| self.stream.set_read_timeout(t))?;
        let read = self.stream.read(self.buffers.input_append_buf()).map_err(|e| timed_out(e, timeout))?;
        self.buffers.input_appended(read);
        Ok(read > 0)
    }

    /// A connection at rest has nothing to read: anything there, or its end, means it is done.
    fn is_open(&mut self) -> bool {
        if self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let at_rest = matches!(self.stream.read(&mut [0]), Err(e) if e.kind() == io::ErrorKind::WouldBlock);
        at_rest && self.stream.set_nonblocking(false).is_ok()
    }
}

impl Http {
    /// [`Http::open_now`] that `cancel` calls off, its body's reads too.
    fn open_under(&self, cancel: &Cancel, url: &str, from: u64, end: Option<u64>) -> Result<Body, OpenError> {
        let request = Arc::new(Request { cancel: cancel.clone() });
        let mut b = request.during(|| self.open_now(url, from, end))?;
        b.reader = Box::new(CancellableBody { inner: b.reader, request });
        Ok(b)
    }
}

/// A response body whose reads run under its request; once that is cancelled, its end is an error.
struct CancellableBody {
    inner: Box<dyn Read + Send>,
    request: Arc<Request>,
}

impl Read for CancellableBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (inner, request) = (&mut self.inner, &self.request);
        match request.during(|| inner.read(buf)) {
            Ok(0) | Err(_) if request.cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            read => read,
        }
    }
}

impl ByteSource for Http {
    fn open(&self, url: &str, from: u64) -> Result<Body, OpenError> {
        self.open_now(url, from, None)
    }

    fn open_cancellable(&self, url: &str, _key: Option<&str>, from: u64, cancel: &Cancel) -> Result<Body, OpenError> {
        self.open_under(cancel, url, from, None)
    }

    fn open_part(&self, url: &str, _key: Option<&str>, from: u64, end: u64, cancel: &Cancel) -> Result<Body, OpenError> {
        self.open_under(cancel, url, from, Some(end))
    }

    /// Radio stream with ICY metadata requested; also returns `icy-metaint` (audio bytes between metadata blocks).
    fn open_live(&self, url: &str) -> Result<(Body, Option<usize>), String> {
        let r = self.agent.get(url).header("Icy-MetaData", "1").call().map_err(|e| e.to_string())?;
        let status = r.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        let every = r.headers().get("icy-metaint").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let reader: Box<dyn Read + Send> = Box::new(r.into_body().into_reader());
        Ok((Body { start: 0, len: None, reader }, every))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range() {
        assert_eq!(content_range("bytes 100-199/1000"), Some((100, Some(1000))));
        assert_eq!(content_range("bytes 5-9/*"), Some((5, None)));
        assert_eq!(content_range("items 1-2/3"), None);
    }

    #[test]
    fn failures_are_told_apart() {
        use std::io::{Error, ErrorKind};
        let lookup = "failed to lookup address information: nodename nor servname provided, or not known";
        let cases = [
            (Error::other(lookup), FailureKind::UnknownHost),
            (Error::new(ErrorKind::ConnectionRefused, "refused"), FailureKind::Connect),
            (Error::new(ErrorKind::PermissionDenied, "denied"), FailureKind::Io),
        ];
        for (io, want) in cases {
            let TransportError::Failed { kind, .. } = failure(ureq::Error::Io(io));
            assert_eq!(kind, want);
        }
    }

    /// A part asks a range with its end, and its body says where it starts in the whole and how long the
    /// whole is.
    #[test]
    fn a_part_asks_a_range_with_an_end() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/audio", server.local_addr().unwrap());
        let answering = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut range = String::new();
            for line in BufReader::new(socket.try_clone().unwrap()).lines().map(Result::unwrap).take_while(|l| !l.is_empty()) {
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: ") {
                    range = v.to_string();
                }
            }
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1000-1009/5000\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789").unwrap();
            range
        });
        let mut body = Http::new().open_part(&url, None, 1000, 1010, &Cancel::new()).unwrap();
        let mut got = String::new();
        body.reader.read_to_string(&mut got).unwrap();
        assert_eq!((body.start, body.len, got.as_str()), (1000, Some(5000), "0123456789"));
        assert_eq!(answering.join().unwrap(), "bytes=1000-1009");
    }

    #[test]
    fn parses_unsatisfied_range() {
        assert_eq!(unsatisfied_range("bytes */6406842"), Some(6_406_842));
        assert_eq!(unsatisfied_range("bytes */*"), None);
        assert_eq!(unsatisfied_range("bytes 0-9/10"), None);
    }
}
