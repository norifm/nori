//! The plugins' LAN door: plain HTTP on a port the system picks, open only while a plugin serves
//! something on it. Its own listener, not remote control's door: what is asked here (a speaker
//! fetching a stream, a renderer's event) comes unsigned, and remote control's door takes nothing that
//! is not signed with the account's secret. Each connection gets a thread of its own while it lasts.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Connections served at once; later ones are closed unread.
const MAX_CONNECTIONS: usize = 16;
/// Bytes of a request's line and headers.
const HEAD_MOST: u64 = 16 << 10;
/// Bytes of a request's body (a UPnP event's XML).
const BODY_MOST: usize = 1 << 20;
/// How long a connection waits for its request.
const READ_WAIT: Duration = Duration::from_secs(10);

pub(crate) struct Request {
    pub method: String,
    /// Without the query.
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub(crate) struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn status(status: u16) -> Response {
        Response { status, headers: Vec::new(), body: Vec::new() }
    }
}

/// What the door does with each request: it answers on the connection, or keeps it (a stream).
pub(crate) type Answer = Box<dyn Fn(Request, TcpStream) + Send + Sync>;

pub(crate) struct Door {
    port: u16,
    closed: Arc<AtomicBool>,
}

impl Door {
    pub fn open(answer: Answer) -> std::io::Result<Door> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let port = listener.local_addr()?.port();
        let closed = Arc::new(AtomicBool::new(false));
        let (shut, answer, live) = (closed.clone(), Arc::new(answer), Arc::new(AtomicUsize::new(0)));
        std::thread::Builder::new().name("nori-plugin-door".into()).spawn(move || {
            for conn in listener.incoming() {
                if shut.load(Ordering::Acquire) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                if live.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                    live.fetch_sub(1, Ordering::AcqRel);
                    continue;
                }
                let (answer, live) = (answer.clone(), live.clone());
                let _ = std::thread::Builder::new().name("nori-plugin-door-in".into()).spawn(move || {
                    let _ = conn.set_read_timeout(Some(READ_WAIT));
                    if let Some(request) = conn.try_clone().ok().and_then(|c| read(&mut BufReader::new(c))) {
                        answer(request, conn);
                    }
                    live.fetch_sub(1, Ordering::AcqRel);
                });
            }
        })?;
        Ok(Door { port, closed })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Door {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        // Wakes the accepting thread so it sees the door closed.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// The next request on a connection; None at its end or past its budgets.
fn read(r: &mut impl BufRead) -> Option<Request> {
    let mut head = (&mut *r).take(HEAD_MOST);
    let mut line = String::new();
    let mut next = |line: &mut String| {
        line.clear();
        head.read_line(line).ok().filter(|_| line.ends_with('\n')).map(|_| ())
    };
    next(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut headers = Vec::new();
    loop {
        next(&mut line)?;
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let length: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).map_or(Some(0), |(_, v)| v.parse().ok())?;
    if length > BODY_MOST {
        return None;
    }
    let mut body = vec![0; length];
    r.read_exact(&mut body).ok()?;
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target, None),
    };
    Some(Request { method, path, query, headers, body })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "",
    }
}

/// Writes `r` and closes the connection.
pub(crate) fn respond(mut conn: TcpStream, r: &Response) {
    let mut head = format!("HTTP/1.1 {} {}\r\n", r.status, reason(r.status));
    for (k, v) in r.headers.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("content-length") && !k.eq_ignore_ascii_case("connection")) {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", r.body.len()));
    let _ = conn.write_all(head.as_bytes()).and_then(|_| conn.write_all(&r.body));
    let _ = conn.shutdown(Shutdown::Both);
}

/// Forwards `request` to `url` (plain HTTP) and pipes its answer back as it comes: a song a device
/// fetches through the door, with its range.
pub(crate) fn forward(url: &str, request: &Request, mut conn: TcpStream) {
    let Some((host, port, path)) = http_parts(url) else { return respond(conn, &Response::status(502)) };
    let Ok(mut upstream) = TcpStream::connect((host.as_str(), port)) else { return respond(conn, &Response::status(502)) };
    let method = if request.method == "HEAD" { "HEAD" } else { "GET" };
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n");
    if let Some(range) = request.header("range") {
        head.push_str(&format!("Range: {range}\r\n"));
    }
    head.push_str("\r\n");
    if upstream.write_all(head.as_bytes()).is_err() {
        return respond(conn, &Response::status(502));
    }
    let _ = std::io::copy(&mut upstream, &mut conn);
    let _ = conn.shutdown(Shutdown::Both);
}

/// An `http://` URL's host, port and path with its query.
fn http_parts(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = rest.find('/').map_or((rest, "/"), |at| (&rest[..at], &rest[at..]));
    // A bracketed IPv6 address keeps its colons.
    let (host, port) = match authority.rsplit_once(':').filter(|(_, p)| !p.contains(']')) {
        Some((h, p)) => (h.trim_matches(['[', ']']).to_string(), p.parse().ok()?),
        None => (authority.trim_matches(['[', ']']).to_string(), 80),
    };
    Some((host, port, path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_reads_with_its_headers_and_body() {
        let raw = b"NOTIFY /p/fm.nori.x/event?sid=1 HTTP/1.1\r\nSID: uuid:1\r\nContent-Length: 5\r\n\r\nhello";
        let r = read(&mut &raw[..]).unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), r.query.as_deref(), r.header("sid"), r.body.as_slice()), ("NOTIFY", "/p/fm.nori.x/event", Some("sid=1"), Some("uuid:1"), &b"hello"[..]));
        assert!(read(&mut &b"GET / HTTP/1.1\r\nContent-Length: 99999999\r\n\r\n"[..]).is_none(), "a body past its budget");
        assert_eq!(http_parts("http://127.0.0.1:4533/rest/stream?id=1"), Some(("127.0.0.1".into(), 4533, "/rest/stream?id=1".into())));
        assert_eq!(http_parts("http://music.lan"), Some(("music.lan".into(), 80, "/".into())));
        assert_eq!(http_parts("https://music.lan"), None);
    }
}
