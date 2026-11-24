//! A profile's SOCKS5 proxy: every connection of the profile's agent goes through it, the proxy looking
//! up the names, signed in to when it asks, except to a local address; a profile without one connects
//! directly. A proxy's failure says which it was.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use nori_core::proxy::{ProxyLogin, SavedProxy};
use nori_core::settings::SavedServer;
use nori_core::transport::{block_on, FailureKind, Transport, TransportError};
use nori_engine::{ByteSource, Cancel};
use nori_http::Http;

const WAIT: Duration = Duration::from_secs(10);

/// The host the in-test proxy cannot reach.
const UNREACHABLE: &str = "unreachable.example";

/// A user and a password.
type Login = Option<(&'static str, &'static str)>;

/// What the proxy heard on one connection.
#[derive(Debug, PartialEq)]
enum Heard {
    /// Asked for this host and port, after this login, if any; then the request line sent through it.
    Carried { to: String, login: Option<(String, String)>, request: String },
    /// A login it does not take, or none where it asks for one.
    Refused,
    /// Asked for a host it cannot reach.
    NotReached,
}

/// A SOCKS5 proxy on 127.0.0.1 that is also every server it connects to, answering each request with
/// its request line; `login` is what it asks for. Its port, and what it hears.
fn proxy(login: Login) -> (u16, Receiver<Heard>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (heard_to, heard) = channel();
    std::thread::spawn(move || {
        for c in listener.incoming() {
            carry(c.unwrap(), login, &heard_to);
        }
    });
    (port, heard)
}

fn carry(mut c: TcpStream, wanted: Option<(&str, &str)>, heard: &Sender<Heard>) {
    let hello = bytes(&mut c, 2);
    let methods = bytes(&mut c, hello[1] as usize);
    let login = match wanted {
        Some(wanted) if methods.contains(&2) => {
            c.write_all(&[5, 2]).unwrap();
            let _version = bytes(&mut c, 1);
            let given = (counted(&mut c), counted(&mut c));
            let right = (given.0.as_str(), given.1.as_str()) == wanted;
            c.write_all(&[1, if right { 0 } else { 1 }]).unwrap();
            if !right {
                return heard.send(Heard::Refused).unwrap();
            }
            Some(given)
        }
        Some(_) => {
            c.write_all(&[5, 0xff]).unwrap();
            return heard.send(Heard::Refused).unwrap();
        }
        None => {
            c.write_all(&[5, 0]).unwrap();
            None
        }
    };
    let ask = bytes(&mut c, 4);
    assert_eq!(ask[..3], [5, 1, 0], "a CONNECT");
    let host = match ask[3] {
        1 => Ipv4Addr::from(<[u8; 4]>::try_from(bytes(&mut c, 4)).unwrap()).to_string(),
        3 => counted(&mut c),
        4 => format!("[{}]", Ipv6Addr::from(<[u8; 16]>::try_from(bytes(&mut c, 16)).unwrap())),
        other => panic!("address type {other}"),
    };
    let port = u16::from_be_bytes([bytes(&mut c, 1)[0], bytes(&mut c, 1)[0]]);
    if host == UNREACHABLE {
        c.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        return heard.send(Heard::NotReached).unwrap();
    }
    c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
    let request = answer(&mut c);
    heard.send(Heard::Carried { to: format!("{host}:{port}"), login, request }).unwrap();
}

/// Answers the request `c` sends with its request line, which it returns.
fn answer(c: &mut TcpStream) -> String {
    let request = request_line(c);
    write!(c, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{request}", request.len()).unwrap();
    request
}

/// A server on 127.0.0.1 answering each request with its request line; its port.
fn server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in listener.incoming() {
            answer(&mut c.unwrap());
        }
    });
    port
}

fn bytes(c: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut b = vec![0; n];
    c.read_exact(&mut b).unwrap();
    b
}

/// A string after its length byte.
fn counted(c: &mut TcpStream) -> String {
    let n = bytes(c, 1)[0] as usize;
    String::from_utf8(bytes(c, n)).unwrap()
}

/// The first line of the request `c` sends, once its head is in.
fn request_line(c: &mut TcpStream) -> String {
    let mut got = Vec::new();
    let mut b = [0];
    while !got.ends_with(b"\r\n\r\n") && c.read(&mut b).unwrap() == 1 {
        got.push(b[0]);
    }
    String::from_utf8(got).unwrap().lines().next().unwrap_or_default().to_string()
}

fn profile(port: u16, login: Option<(&str, &str)>) -> SavedServer {
    let login = login.map(|(user, password)| ProxyLogin { user: user.into(), password: password.into() });
    SavedServer { proxy: Some(SavedProxy { host: "127.0.0.1".into(), port, login }), ..SavedServer::default() }
}

fn get(http: &Http, url: &str) -> Result<String, TransportError> {
    block_on(http.get(url.into(), 0)).map(|r| String::from_utf8(r.body).unwrap())
}

fn failure(r: Result<String, TransportError>) -> FailureKind {
    let TransportError::Failed { kind, .. } = r.expect_err("the request failed");
    kind
}

/// The API's requests and a song's stream both go through the proxy, which looks the server's name up:
/// this machine cannot.
#[test]
fn api_requests_and_streams_go_through_the_proxy() {
    let (port, heard) = proxy(None);
    let http = Http::for_profile(&profile(port, None), std::path::Path::new(""));
    assert_eq!(get(&http, "http://music.example/rest/ping").unwrap(), "GET /rest/ping HTTP/1.1");
    let mut song = String::new();
    http.open_part("http://music.example:4533/rest/stream", None, 0, 10, &Cancel::new()).unwrap().reader.read_to_string(&mut song).unwrap();
    assert_eq!(song, "GET /rest/stream HTTP/1.1");
    let carried = |to: &str, request: &str| Heard::Carried { to: to.into(), login: None, request: request.into() };
    assert_eq!(heard.recv_timeout(WAIT).unwrap(), carried("music.example:80", "GET /rest/ping HTTP/1.1"));
    assert_eq!(heard.recv_timeout(WAIT).unwrap(), carried("music.example:4533", "GET /rest/stream HTTP/1.1"));
}

/// The proxy is signed in to when it asks, with the profile's login; a login it refuses, or none,
/// fails the request.
#[test]
fn the_proxy_is_signed_in_to_when_it_asks() {
    let cases: [(Login, Login, bool); 4] = [
        (None, Some(("me", "p@ss:w0rd")), true),
        (Some(("me", "p@ss:w0rd")), Some(("me", "p@ss:w0rd")), true),
        (Some(("me", "p@ss:w0rd")), Some(("me", "wrong")), false),
        (Some(("me", "p@ss:w0rd")), None, false),
    ];
    for (asked, given, carried) in cases {
        let (port, heard) = proxy(asked);
        let got = get(&Http::for_profile(&profile(port, given), std::path::Path::new("")), "http://music.example/rest/ping");
        let case = format!("asked {asked:?}, given {given:?}");
        match heard.recv_timeout(WAIT).unwrap() {
            Heard::Carried { login, .. } if carried => {
                assert_eq!(got.unwrap(), "GET /rest/ping HTTP/1.1", "{case}");
                assert_eq!(login, asked.map(|(u, p)| (u.to_string(), p.to_string())), "{case}");
            }
            Heard::Refused if !carried => assert_eq!(failure(got), FailureKind::ProxyLogin, "{case}"),
            other => panic!("{case}: {other:?}"),
        }
    }
}

/// A proxy that is down fails the request, saying so: nothing goes around it to the server.
#[test]
fn a_proxy_that_is_down_is_not_gone_around() {
    let down = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert_eq!(failure(get(&Http::for_profile(&profile(down, None), std::path::Path::new("")), "http://music.example/rest/ping")), FailureKind::ProxyUnreachable);
}

/// A server the proxy cannot reach fails the request, saying so.
#[test]
fn a_server_the_proxy_cannot_reach_says_so() {
    let (port, heard) = proxy(None);
    assert_eq!(failure(get(&Http::for_profile(&profile(port, None), std::path::Path::new("")), &format!("http://{UNREACHABLE}/rest/ping"))), FailureKind::ProxyNoRoute);
    assert_eq!(heard.recv_timeout(WAIT).unwrap(), Heard::NotReached);
}

/// A local address, or `localhost`, is reached directly, since a proxy elsewhere cannot reach the
/// user's network; a name and any other address still go through the proxy.
#[test]
fn local_destinations_are_reached_directly() {
    let (port, heard) = proxy(None);
    let http = Http::for_profile(&profile(port, None), std::path::Path::new(""));
    let local = server();
    for host in ["127.0.0.1", "localhost"] {
        assert_eq!(get(&http, &format!("http://{host}:{local}/rest/ping")).unwrap(), "GET /rest/ping HTTP/1.1", "{host}");
    }
    for (url, to) in [("http://music.example/rest/ping", "music.example:80"), ("http://203.0.113.5:4533/rest/ping", "203.0.113.5:4533")] {
        assert_eq!(get(&http, url).unwrap(), "GET /rest/ping HTTP/1.1", "{url}");
        assert_eq!(heard.recv_timeout(WAIT).unwrap(), Heard::Carried { to: to.into(), login: None, request: "GET /rest/ping HTTP/1.1".into() });
    }
    assert!(heard.try_recv().is_err(), "a local destination went through the proxy");
}

/// A profile without a proxy connects to the server itself.
#[test]
fn a_profile_without_a_proxy_connects_directly() {
    let url = format!("http://127.0.0.1:{}/rest/ping", server());
    assert_eq!(get(&Http::for_profile(&SavedServer::default(), std::path::Path::new("")), &url).unwrap(), "GET /rest/ping HTTP/1.1");
}

/// A jam's guest profile goes through the proxy of the profile it was joined from, as the join did, so
/// the relay never sees this device's own address.
#[test]
fn a_jam_guest_goes_through_the_joining_profiles_proxy() {
    let (port, heard) = proxy(None);
    let pass = nori_core::remote::JamPass { url: "http://relay.example".into(), api_key: "k".into(), kind: Default::default() };
    let guest = nori_core::remote::jam_profile(pass, Some(profile(port, None)), "guest".into(), "Jam".into());
    assert_eq!(get(&Http::for_profile(&guest, std::path::Path::new("")), "http://relay.example/rest/ping").unwrap(), "GET /rest/ping HTTP/1.1");
    assert_eq!(heard.recv_timeout(WAIT).unwrap(), Heard::Carried { to: "relay.example:80".into(), login: None, request: "GET /rest/ping HTTP/1.1".into() });
}
