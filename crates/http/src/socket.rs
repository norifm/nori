//! The sync server's WebSockets for the desktop, the terminal and the iPod (nori-sync's [`Sockets`]):
//! RFC 6455 over TCP, with TLS by the rustls and Mozilla roots ureq uses, through the proxy the
//! environment names (an `http://` one, by CONNECT). A profile's proxy and client certificate do not
//! apply: the sync server is no profile's server.
//!
//! Each socket has a thread that reads, blocked in the read; a send writes from the caller's thread
//! under the TLS state's lock, which the reader takes only to decrypt what came. So nothing polls, and a
//! send never waits for the read.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use nori_core::sync::{Ended, SocketId, Sockets, Syncing};
use rustls::{ClientConfig, ClientConnection, RootCertStore};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest the TLS and WebSocket handshakes take.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// The largest message read: the sync server's frames are at most 64 KiB by default.
const MAX_MESSAGE: usize = 1 << 20;
/// The largest handshake answer read.
const MAX_ANSWER: usize = 16 << 10;

/// The platform's WebSockets for nori-sync: [`WebSockets::tell`] the sync they belong to once it is made.
#[derive(Default)]
pub struct WebSockets(Arc<Shared>);

#[derive(Default)]
struct Shared {
    open: Mutex<HashMap<SocketId, Arc<Conn>>>,
    hears: OnceLock<Weak<Syncing>>,
}

impl Shared {
    fn hear(&self, f: impl FnOnce(&Syncing)) {
        if let Some(s) = self.hears.get().and_then(Weak::upgrade) {
            f(&s);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One socket.
#[derive(Default)]
struct Conn {
    /// The writing half, once connected.
    writer: Mutex<Option<Writer>>,
    /// Closed by sync: nothing more is told of it.
    closed: AtomicBool,
}

impl Conn {
    fn write(&self, op: Op, payload: &[u8]) {
        if let Some(w) = lock(&self.writer).as_mut() {
            if let Err(e) = w.write(&frame(op, payload, mask())) {
                nori_core::alog::info(&format!("sync socket: could not send: {e}"));
            }
        }
    }
}

impl WebSockets {
    pub fn new() -> Arc<WebSockets> {
        Arc::default()
    }

    /// What the sockets hear goes to `syncing`.
    pub fn tell(&self, syncing: &Arc<Syncing>) {
        let _ = self.0.hears.set(Arc::downgrade(syncing));
    }
}

impl Sockets for WebSockets {
    fn open(&self, id: SocketId, url: String) {
        let conn = Arc::new(Conn::default());
        lock(&self.0.open).insert(id, conn.clone());
        let shared = self.0.clone();
        let spawned = std::thread::Builder::new().name("nori-sync-socket".into()).spawn(move || {
            let how = run(id, &url, &conn, &shared);
            lock(&shared.open).remove(&id);
            if !conn.closed.load(Ordering::Relaxed) {
                shared.hear(|s| s.ended(id, how));
            }
        });
        if spawned.is_err() {
            lock(&self.0.open).remove(&id);
            self.0.hear(|s| s.ended(id, Ended::Dropped));
        }
    }

    fn send(&self, id: SocketId, frame: Vec<u8>) {
        if let Some(c) = lock(&self.0.open).get(&id).cloned() {
            c.write(Op::Binary, &frame);
        }
    }

    fn ping(&self, id: SocketId) {
        if let Some(c) = lock(&self.0.open).get(&id).cloned() {
            c.write(Op::Ping, &[]);
        }
    }

    fn close(&self, id: SocketId) {
        let Some(c) = lock(&self.0.open).remove(&id) else { return };
        c.closed.store(true, Ordering::Relaxed);
        let writer = lock(&c.writer).take();
        if let Some(mut w) = writer {
            let _ = w.write(&frame(Op::Close, &1000u16.to_be_bytes(), mask()));
            w.shut();
        }
    }
}

/// Connects socket `id` to `url` and reads it until it ends; how it ended.
fn run(id: SocketId, url: &str, conn: &Conn, shared: &Shared) -> Ended {
    let (mut reader, writer, mut frames) = match connect(url) {
        Ok(c) => c,
        Err(Failed::Status(status)) => return Ended::Status { status },
        Err(Failed::Io(e)) => {
            nori_core::alog::info(&format!("sync socket: {url}: {e}"));
            return Ended::Dropped;
        }
    };
    {
        let mut w = lock(&conn.writer);
        if conn.closed.load(Ordering::Relaxed) {
            writer.shut();
            return Ended::Dropped;
        }
        *w = Some(writer);
    }
    loop {
        loop {
            match frames.next() {
                Ok(Some(Got::Message(Op::Binary, data))) => {
                    if !conn.closed.load(Ordering::Relaxed) {
                        shared.hear(|s| s.frame(id, data));
                    }
                }
                // The text keep-alive's answer, which this client never asks for.
                Ok(Some(Got::Message(_, _) | Got::Pong)) => {}
                Ok(Some(Got::Ping(data))) => conn.write(Op::Pong, &data),
                Ok(Some(Got::Close(code))) => {
                    conn.write(Op::Close, &code.unwrap_or(1000).to_be_bytes());
                    if let Some(w) = lock(&conn.writer).take() {
                        w.shut();
                    }
                    // A close frame without a code is told as no refusal: 1005.
                    return Ended::Closed { code: code.unwrap_or(1005) };
                }
                Ok(None) => break,
                Err(e) => {
                    nori_core::alog::info(&format!("sync socket: the server broke the protocol: {e}"));
                    if let Some(w) = lock(&conn.writer).take() {
                        w.shut();
                    }
                    return Ended::Closed { code: 1002 };
                }
            }
        }
        match reader.read_into(&mut frames.buf) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                if let Some(w) = lock(&conn.writer).take() {
                    w.shut();
                }
                return Ended::Dropped;
            }
        }
    }
}

enum Failed {
    /// The server answered the handshake with this status.
    Status(u16),
    Io(io::Error),
}

impl From<io::Error> for Failed {
    fn from(e: io::Error) -> Failed {
        Failed::Io(e)
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

/// The parts of a `ws://` or `wss://` address.
#[derive(Debug, PartialEq)]
struct Address {
    tls: bool,
    host: String,
    port: u16,
    /// The path and query.
    target: String,
}

fn address(url: &str) -> Option<Address> {
    let (scheme, rest) = url.split_once("://")?;
    let tls = match scheme {
        "wss" => true,
        "ws" => false,
        _ => return None,
    };
    let (authority, target) = rest.find('/').map_or((rest, "/"), |at| rest.split_at(at));
    let (host, port) = match authority.rsplit_once(':').filter(|(h, _)| !h.ends_with(']') || h.starts_with('[')) {
        Some((h, p)) if !h.is_empty() && !authority.ends_with(']') => (h, p.parse().ok()?),
        _ => (authority, if tls { 443 } else { 80 }),
    };
    Some(Address { tls, host: host.trim_start_matches('[').trim_end_matches(']').to_string(), port, target: target.to_string() })
}

fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("ring's protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// A TCP connection to `a`, through the environment's proxy unless it excepts the host.
fn tcp(a: &Address, url: &str) -> Result<TcpStream, Failed> {
    let proxy = ureq::Proxy::try_from_env().filter(|p| url.parse::<ureq::http::Uri>().is_ok_and(|u| !p.is_no_proxy(&u)));
    let Some(proxy) = proxy else { return Ok(dial(&a.host, a.port)?) };
    if proxy.protocol() != ureq::ProxyProtocol::Http {
        return Err(Failed::Io(io::Error::other("the sync server goes through an http:// proxy only")));
    }
    let mut stream = dial(proxy.host(), proxy.port())?;
    let target = if a.host.contains(':') { format!("[{}]:{}", a.host, a.port) } else { format!("{}:{}", a.host, a.port) };
    let mut ask = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let (Some(user), Some(password)) = (proxy.username(), proxy.password()) {
        ask.push_str(&format!("Proxy-Authorization: Basic {}\r\n", STANDARD.encode(format!("{user}:{password}"))));
    }
    ask.push_str("\r\n");
    stream.write_all(ask.as_bytes())?;
    let answer = head(&mut |buf: &mut Vec<u8>| plain_read(&mut stream, buf))?;
    if answer.status != 200 || !answer.rest.is_empty() {
        return Err(Failed::Io(io::Error::other(format!("the proxy answered {}", answer.status))));
    }
    Ok(stream)
}

fn dial(host: &str, port: u16) -> io::Result<TcpStream> {
    let mut failed = io::Error::new(io::ErrorKind::NotFound, format!("{host} has no address"));
    for addr in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => {
                s.set_nodelay(true)?;
                return Ok(s);
            }
            Err(e) => failed = e,
        }
    }
    Err(failed)
}

/// Reads more of `stream` onto `buf`; false at its end.
fn plain_read(stream: &mut TcpStream, buf: &mut Vec<u8>) -> io::Result<bool> {
    let mut chunk = [0; 16 << 10];
    let n = stream.read(&mut chunk)?;
    buf.extend_from_slice(&chunk[..n]);
    Ok(n > 0)
}

/// An HTTP answer's head.
#[derive(Debug, PartialEq)]
struct Head {
    status: u16,
    /// Names lowercase.
    headers: Vec<(String, String)>,
    /// What came after the head.
    rest: Vec<u8>,
}

/// An HTTP answer's head, read with `read`.
fn head(read: &mut dyn FnMut(&mut Vec<u8>) -> io::Result<bool>) -> io::Result<Head> {
    let mut buf = Vec::new();
    let end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buf.len() > MAX_ANSWER || !read(&mut buf)? {
            return Err(invalid("no answer"));
        }
    };
    let text = std::str::from_utf8(&buf[..end]).map_err(|_| invalid("an answer that is no text"))?;
    let mut lines = text.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).ok_or_else(|| invalid("no status"))?;
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    Ok(Head { status, headers, rest: buf[end + 4..].to_vec() })
}

/// The key a server answers a handshake with `key` by.
fn accept_key(key: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    STANDARD.encode(digest.as_ref())
}

fn mask() -> [u8; 4] {
    let mut m = [0; 4];
    getrandom::fill(&mut m).expect("the system has randomness");
    m
}

/// Opens `url`: TCP, TLS, the WebSocket handshake.
fn connect(url: &str) -> Result<(Reader, Writer, Frames), Failed> {
    let a = address(url).ok_or_else(|| invalid("not a WebSocket address"))?;
    let stream = tcp(&a, url)?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let (mut reader, mut writer) = if a.tls {
        let name = rustls::pki_types::ServerName::try_from(a.host.clone()).map_err(|_| invalid("not a server name"))?;
        let mut tls = ClientConnection::new(tls_config(), name).map_err(io::Error::other)?;
        let mut s = stream.try_clone()?;
        while tls.is_handshaking() {
            tls.complete_io(&mut s)?;
        }
        let tls = Arc::new(Mutex::new(tls));
        (Reader::Tls(tls.clone(), stream.try_clone()?), Writer::Tls(tls, stream))
    } else {
        (Reader::Plain(stream.try_clone()?), Writer::Plain(stream))
    };
    let key = STANDARD.encode(mask().iter().chain(&mask()).chain(&mask()).chain(&mask()).copied().collect::<Vec<u8>>());
    let host = if a.port == if a.tls { 443 } else { 80 } { a.host.clone() } else { format!("{}:{}", a.host, a.port) };
    let ask = format!(
        "GET {} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: {}\r\n\r\n",
        a.target,
        nori_core::transport::USER_AGENT
    );
    writer.write(ask.as_bytes())?;
    let Head { status, headers, rest } = head(&mut |buf: &mut Vec<u8>| reader.read_into(buf))?;
    if status != 101 {
        return Err(Failed::Status(status));
    }
    let accepted = headers.iter().any(|(n, v)| n == "sec-websocket-accept" && *v == accept_key(&key));
    if !accepted {
        return Err(Failed::Io(invalid("a handshake answered with the wrong key")));
    }
    reader.timeout(None)?;
    Ok((reader, writer, Frames { buf: rest, ..Frames::default() }))
}

enum Reader {
    Plain(TcpStream),
    /// The TLS state, shared with the writer, and the TCP stream read.
    Tls(Arc<Mutex<ClientConnection>>, TcpStream),
}

impl Reader {
    fn timeout(&self, t: Option<Duration>) -> io::Result<()> {
        match self {
            Reader::Plain(s) | Reader::Tls(_, s) => s.set_read_timeout(t),
        }
    }

    /// Reads what comes next onto `buf`, waiting for it without holding the TLS state; false at the end.
    fn read_into(&mut self, buf: &mut Vec<u8>) -> io::Result<bool> {
        let (tls, stream) = match self {
            Reader::Plain(s) => return plain_read(s, buf),
            Reader::Tls(tls, s) => (tls, s),
        };
        let mut chunk = [0; 16 << 10];
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(false);
        }
        let mut c = lock(tls);
        let mut came = &chunk[..n];
        while !came.is_empty() {
            c.read_tls(&mut came)?;
            c.process_new_packets().map_err(io::Error::other)?;
        }
        let mut plain = [0; 16 << 10];
        loop {
            match c.reader().read(&mut plain) {
                Ok(0) => return Ok(false),
                Ok(k) => buf.extend_from_slice(&plain[..k]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        while c.wants_write() {
            c.write_tls(stream)?;
        }
        Ok(true)
    }
}

enum Writer {
    Plain(TcpStream),
    Tls(Arc<Mutex<ClientConnection>>, TcpStream),
}

impl Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Writer::Plain(s) => s.write_all(bytes),
            Writer::Tls(tls, s) => {
                let mut c = lock(tls);
                c.writer().write_all(bytes)?;
                while c.wants_write() {
                    c.write_tls(s)?;
                }
                Ok(())
            }
        }
    }

    fn shut(&self) {
        let _ = match self {
            Writer::Plain(s) | Writer::Tls(_, s) => s.shutdown(Shutdown::Both),
        };
    }
}

/// A frame's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Text = 1,
    Binary = 2,
    Close = 8,
    Ping = 9,
    Pong = 10,
}

/// A frame from this client: always masked, never in pieces.
fn frame(op: Op, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0x80 | op as u8];
    match payload.len() {
        n if n < 126 => out.push(0x80 | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// What the server sent, read as whole messages.
#[derive(Debug, PartialEq)]
enum Got {
    Message(Op, Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    /// The close frame's code, if it has one.
    Close(Option<u16>),
}

/// Frames as they come, read into messages.
#[derive(Default)]
struct Frames {
    buf: Vec<u8>,
    /// A message in pieces so far, and its kind.
    pieces: Option<(Op, Vec<u8>)>,
}

impl Frames {
    /// The next whole message or control frame; None until more comes.
    fn next(&mut self) -> Result<Option<Got>, &'static str> {
        loop {
            let b = &self.buf;
            if b.len() < 2 {
                return Ok(None);
            }
            if b[1] & 0x80 != 0 {
                return Err("a masked frame from the server");
            }
            let (len, at) = match b[1] & 0x7f {
                126 if b.len() >= 4 => (u16::from_be_bytes([b[2], b[3]]) as usize, 4),
                127 if b.len() >= 10 => (u64::from_be_bytes(b[2..10].try_into().expect("8 bytes")).min(usize::MAX as u64) as usize, 10),
                126 | 127 => return Ok(None),
                n => (n as usize, 2),
            };
            if len > MAX_MESSAGE {
                return Err("a frame too large");
            }
            if b.len() < at + len {
                return Ok(None);
            }
            let (fin, code) = (b[0] & 0x80 != 0, b[0] & 0x0f);
            let payload: Vec<u8> = self.buf.drain(..at + len).skip(at).collect();
            let op = match code {
                0 => {
                    let Some((op, mut data)) = self.pieces.take() else { return Err("a piece of no message") };
                    data.extend_from_slice(&payload);
                    if data.len() > MAX_MESSAGE {
                        return Err("a message too large");
                    }
                    if fin {
                        return Ok(Some(Got::Message(op, data)));
                    }
                    self.pieces = Some((op, data));
                    continue;
                }
                1 => Op::Text,
                2 => Op::Binary,
                8 => return Ok(Some(Got::Close(payload.get(..2).map(|c| u16::from_be_bytes([c[0], c[1]]))))),
                9 => return Ok(Some(Got::Ping(payload))),
                10 => return Ok(Some(Got::Pong)),
                _ => return Err("an unknown frame"),
            };
            if fin {
                return Ok(Some(Got::Message(op, payload)));
            }
            self.pieces = Some((op, payload));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handshake_is_accepted_by_the_key_rfc_6455_gives() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frames_go_out_masked_as_rfc_6455_shows() {
        // RFC 6455 5.7: a masked "Hello".
        assert_eq!(frame(Op::Text, b"Hello", [0x37, 0xfa, 0x21, 0x3d]), [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]);
        assert_eq!(frame(Op::Ping, b"", [1, 2, 3, 4]), [0x89, 0x80, 1, 2, 3, 4]);
        let long = frame(Op::Binary, &[0; 300], [0; 4]);
        assert_eq!(long[..4], [0x82, 0xfe, 0x01, 0x2c]);
        assert_eq!(long.len(), 4 + 4 + 300);
        assert_eq!(frame(Op::Binary, &vec![0; 70_000], [0; 4])[..10], [0x82, 0xff, 0, 0, 0, 0, 0, 1, 0x11, 0x70]);
    }

    #[test]
    fn frames_come_in_as_whole_messages() {
        let mut f = Frames::default();
        // RFC 6455 5.7: "Hello" in two pieces, a ping between them, then a 256-byte binary message.
        f.buf.extend_from_slice(&[0x01, 0x03, 0x48, 0x65, 0x6c, 0x89, 0x02, b'h', b'i', 0x80, 0x02, 0x6c, 0x6f]);
        assert_eq!(f.next(), Ok(Some(Got::Ping(b"hi".to_vec()))));
        assert_eq!(f.next(), Ok(Some(Got::Message(Op::Text, b"Hello".to_vec()))));
        assert_eq!(f.next(), Ok(None));
        f.buf.extend_from_slice(&[0x82, 0x7e, 0x01]);
        assert_eq!(f.next(), Ok(None), "a length cut short");
        f.buf.push(0x00);
        f.buf.extend_from_slice(&[7; 255]);
        assert_eq!(f.next(), Ok(None), "a payload cut short");
        f.buf.push(7);
        assert_eq!(f.next(), Ok(Some(Got::Message(Op::Binary, vec![7; 256]))));
        f.buf.extend_from_slice(&[0x88, 0x0b, 0x0f, 0xa6, b'd', b'a', b'i', b'l', b'y', b'_', b'l', b'i', b'm']);
        assert_eq!(f.next(), Ok(Some(Got::Close(Some(4006)))));
        f.buf.extend_from_slice(&[0x88, 0x00, 0x8a, 0x00]);
        assert_eq!((f.next(), f.next()), (Ok(Some(Got::Close(None))), Ok(Some(Got::Pong))));
        assert!(Frames { buf: vec![0x82, 0x81, 0, 0, 0, 0, 0], ..Frames::default() }.next().is_err(), "the server masks nothing");
        assert!(Frames { buf: vec![0x80, 0x00], ..Frames::default() }.next().is_err(), "a piece of no message");
    }

    #[test]
    fn an_address_reads_into_its_parts() {
        let a = |url: &str| address(url).map(|a| (a.tls, a.host, a.port, a.target));
        assert_eq!(a("wss://sync.norifm.com/room/x?v=1&kind=group"), Some((true, "sync.norifm.com".into(), 443, "/room/x?v=1&kind=group".into())));
        assert_eq!(a("ws://127.0.0.1:8080/room/x"), Some((false, "127.0.0.1".into(), 8080, "/room/x".into())));
        assert_eq!(a("ws://[::1]:9000/p"), Some((false, "::1".into(), 9000, "/p".into())));
        assert_eq!(a("wss://[::1]/p"), Some((true, "::1".into(), 443, "/p".into())));
        assert_eq!(a("https://example.com/"), None);
    }

    #[test]
    fn an_answer_s_head_is_read_however_it_comes() {
        let mut pieces = vec![b"HTTP/1.1 101 Switching".to_vec(), b" Protocols\r\nSec-WebSocket-Accept: abc\r\n\r\n\x82\x01x".to_vec()].into_iter();
        let answer = head(&mut |buf: &mut Vec<u8>| Ok(pieces.next().map(|p| buf.extend(p)).is_some())).unwrap();
        assert_eq!(answer, Head { status: 101, headers: vec![("sec-websocket-accept".to_string(), "abc".to_string())], rest: b"\x82\x01x".to_vec() });
        let mut none = vec![b"HTTP/1.1 503".to_vec()].into_iter();
        assert!(head(&mut |buf: &mut Vec<u8>| Ok(none.next().map(|p| buf.extend(p)).is_some())).is_err(), "it ended before its head did");
    }
}
