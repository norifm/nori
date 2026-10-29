//! The LAN and casting interfaces over the test plugin (fixture/src/lan.rs): TLS and UDP sockets,
//! mDNS browsing, its endpoints at the door, songs' URLs for another device, and a cast's audio.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};

use nori_net::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
use nori_plugins::testing::{fixture, FIXTURE as ID};
use nori_plugins::{Card, Cast, CastButton, CastKind, Media, Plugins, Service, Side, Sink, Value};

/// Serves each action's setup at `/lan/<action>`.
#[derive(Default)]
struct Web {
    serves: Mutex<Vec<(String, Vec<u8>)>>,
}

impl Web {
    fn serve(&self, action: &str, body: impl Into<Vec<u8>>) {
        let url = format!("https://allowed.example/lan/{action}");
        let mut s = self.serves.lock().unwrap();
        s.retain(|(u, _)| *u != url);
        s.push((url, body.into()));
    }
}

#[async_trait::async_trait]
impl Transport for Web {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        let body = self.serves.lock().unwrap().iter().find(|(u, _)| *u == url).map_or(Vec::new(), |(_, b)| b.clone());
        Ok(TransportResponse { status: 200, body, headers: Vec::new() })
    }
    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(e.url, e.timeout_ms).await
    }
    fn address_changed(&self) {}
    fn network(&self) -> Network {
        Network::Unmetered
    }
}

/// What the plugin host asked of the client.
#[derive(Default)]
struct Session {
    notices: Mutex<Vec<(String, Vec<String>)>>,
    browsed: Mutex<Vec<(String, bool)>>,
    multicast: Mutex<Vec<bool>>,
    casts: Mutex<Vec<Option<Cast>>>,
}

impl Side for Session {
    fn stored(&self, _: &str) -> Option<Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn laid(&self, _: &str) {}
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, words: &str, args: &[String]) {
        self.notices.lock().unwrap().push((words.to_string(), args.to_vec()));
    }
    fn browse(&self, kind: &str, on: bool) {
        self.browsed.lock().unwrap().push((kind.to_string(), on));
    }
    fn multicast(&self, on: bool) {
        self.multicast.lock().unwrap().push(on);
    }
    fn cast(&self, cast: Option<&Cast>) {
        self.casts.lock().unwrap().push(cast.cloned());
    }
}

struct Rig {
    _dir: nori_testdir::TempDir,
    plugins: Plugins,
    session: Arc<Session>,
    web: Arc<Web>,
}

impl Rig {
    fn new(name: &str) -> Rig {
        let dir = nori_testdir::TempDir::new(name);
        let session = Arc::new(Session::default());
        let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
        let web = Arc::new(Web::default());
        plugins.set_transport(Some(web.clone()));
        plugins.install(&fixture("")).unwrap();
        plugins.turn(ID.into(), true);
        Rig { _dir: dir, plugins, session, web }
    }

    /// Runs `action` and waits for it; what it said.
    fn act(&self, action: &str) -> Vec<String> {
        self.plugins.act(ID.into(), action.into());
        self.settle();
        self.said(action).unwrap_or_else(|| panic!("{action} said nothing"))
    }

    /// Waits for the calls posted so far.
    fn settle(&self) {
        let w = self.plugins.worker(ID).unwrap();
        assert_eq!(nori_tasks::block_on(w.call(|_, _| Ok(1))), Some(1));
    }

    /// The last notice named `words`.
    fn said(&self, words: &str) -> Option<Vec<String>> {
        self.session.notices.lock().unwrap().iter().rev().find(|(w, _)| w == words).map(|(_, a)| a.clone())
    }
}

/// `http://host:port/...`'s port.
fn port_of(url: &str) -> u16 {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).and_then(|a| a.rsplit(':').next()).and_then(|p| p.parse().ok()).unwrap_or_else(|| panic!("{url} has a port"))
}

/// `method path` with `body` at the door listening on `port`; the whole answer.
fn ask(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(c, "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut got = String::new();
    c.read_to_string(&mut got).unwrap();
    got
}

const CERT: &[u8] = include_bytes!("data/tls-cert.der");
const KEY: &[u8] = include_bytes!("data/tls-key.der");

/// A TLS server with the test certificate that answers "ping" with "pong", for each connection.
fn tls_server() -> u16 {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(CERT.to_vec())], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())))
        .unwrap();
    let config = Arc::new(config);
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(sock) = conn else { continue };
            let config = config.clone();
            std::thread::spawn(move || {
                let mut tls = rustls::StreamOwned::new(rustls::ServerConnection::new(config).unwrap(), sock);
                let mut b = [0; 4];
                if tls.read_exact(&mut b).is_ok() && &b == b"ping" {
                    let _ = tls.write_all(b"pong");
                    let _ = tls.flush();
                }
            });
        }
    });
    port
}

#[test]
fn tls_is_checked_as_the_plugin_asks() {
    let rig = Rig::new("plugins-lan-tls");
    let port = tls_server();
    let setup = |mode: &str, cert: &[u8]| [format!("{mode} 127.0.0.1:{port}\n").into_bytes(), cert.to_vec()].concat();
    for (mode, cert, heard) in [("pinned", CERT, "pong"), ("pinned", &b"another certificate"[..], "ErrorKind::Refused"), ("none", &[][..], "pong"), ("system", &[][..], "ErrorKind::Refused")] {
        rig.web.serve("tls", setup(mode, cert));
        assert_eq!(rig.act("tls"), [heard], "{mode}");
    }
}

#[test]
fn udp_reaches_the_lan_only_and_a_group_holds_the_lock() {
    let rig = Rig::new("plugins-lan-udp");
    let echo = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = echo.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut b = [0; 64];
        while let Ok((n, from)) = echo.recv_from(&mut b) {
            let _ = echo.send_to(&[b"hi ", &b[..n]].concat(), from);
        }
    });
    rig.web.serve("udp", format!("127.0.0.1:{port}"));
    assert_eq!(rig.act("udp"), ["127.0.0.1 hi hello"]);
    rig.web.serve("udp", "8.8.8.8:53");
    assert_eq!(rig.act("udp"), ["ErrorKind::Refused"], "not on the LAN, not among its hosts");

    assert_eq!(rig.act("join"), ["joined"]);
    assert_eq!(*rig.session.multicast.lock().unwrap(), [true]);
    rig.act("leave");
    assert_eq!(*rig.session.multicast.lock().unwrap(), [true, false], "let go with the last socket in a group");
}

#[test]
fn browsing_runs_only_while_asked_and_its_finds_reach_the_plugin() {
    let rig = Rig::new("plugins-lan-browse");
    let kind = "_test._tcp".to_string();
    let found = Service { name: "Kitchen".into(), host: "192.168.1.9".into(), port: 1705, txt: Vec::new() };
    rig.plugins.discovered(kind.clone(), found.clone());
    rig.settle();
    assert_eq!(rig.said("found"), None, "not browsing, nothing is told");
    rig.act("browse");
    assert_eq!(*rig.session.browsed.lock().unwrap(), [(kind.clone(), true)]);
    rig.plugins.discovered("_other._tcp".into(), found.clone());
    rig.plugins.discovered(kind.clone(), found);
    rig.settle();
    assert_eq!(rig.said("found"), Some(vec!["Kitchen".into(), "192.168.1.9".into(), "1705".into()]));
    rig.act("stop-browsing");
    rig.act("browse");
    rig.plugins.turn(ID.into(), false);
    assert_eq!(*rig.session.browsed.lock().unwrap(), [(kind.clone(), true), (kind.clone(), false), (kind.clone(), true), (kind, false)], "turned off, its browsing stops");
}

#[test]
fn the_door_serves_a_plugins_endpoints_while_it_is_on() {
    let rig = Rig::new("plugins-lan-door");
    let base = rig.act("serve").remove(0);
    assert!(base.starts_with("http://") && base.ends_with(&format!("/p/{ID}")), "{base}");
    let port = port_of(&base);
    let answer = ask(port, "NOTIFY", &format!("/p/{ID}/event?seq=1"), "<e/>");
    assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("NOTIFY /event?seq=1 <e/>"), "{answer}");
    assert!(ask(port, "GET", "/p/fm.nori.other/event", "").starts_with("HTTP/1.1 404"), "another plugin's");
    rig.plugins.turn(ID.into(), false);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "closed with the last plugin serving");
}

/// Songs stream from `server`.
struct Profile {
    server: String,
}

impl Media for Profile {
    fn stream_url(&self, id: &str) -> Option<String> {
        Some(format!("{}/rest/stream?id={id}", self.server))
    }
    fn cover_url(&self, id: &str, size: u32) -> Option<String> {
        Some(format!("{}/rest/getCoverArt?id={id}&size={size}", self.server))
    }
}

#[test]
fn a_songs_url_reaches_the_lan_through_the_door_when_its_server_is_this_device() {
    let rig = Rig::new("plugins-lan-media");
    assert_eq!(rig.act("media"), ["ErrorKind::Unsupported"], "no profile open");
    let lan: Arc<dyn Media> = Arc::new(Profile { server: "http://192.168.1.5:4533".into() });
    rig.plugins.set_media(Arc::downgrade(&lan));
    assert_eq!(rig.act("media"), ["http://192.168.1.5:4533/rest/stream?id=s1"], "as it is");

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let here: Arc<dyn Media> = Arc::new(Profile { server: format!("http://{}", server.local_addr().unwrap()) });
    rig.plugins.set_media(Arc::downgrade(&here));
    std::thread::spawn(move || {
        let (mut c, _) = server.accept().unwrap();
        let mut head = [0; 512];
        let n = c.read(&mut head).unwrap();
        let asked = String::from_utf8_lossy(&head[..n]).lines().next().unwrap_or_default().to_string();
        let _ = write!(c, "HTTP/1.1 206 Partial Content\r\n\r\n{asked}");
    });
    let url = rig.act("media").remove(0);
    assert!(url.contains("/m/"), "{url}");
    let answer = ask(port_of(&url), "GET", &url[url.find("/m/").unwrap()..], "");
    assert!(answer.starts_with("HTTP/1.1 206") && answer.ends_with("GET /rest/stream?id=s1 HTTP/1.1"), "{answer}");
}

/// Where the music played, each time it moved: a sink, or "here" or "elsewhere".
type Played = Arc<Mutex<Vec<Result<Arc<Sink>, &'static str>>>>;

fn cards(plugins: &Plugins) -> Played {
    let played: Played = Arc::default();
    let to = played.clone();
    plugins.set_cards(Box::new(move |c| {
        to.lock().unwrap().push(match c {
            Card::Sink(s) => Ok(s),
            Card::Here => Err("here"),
            Card::Elsewhere => Err("elsewhere"),
        })
    }));
    played
}

/// The sink the music last went to.
fn sink(played: &Played) -> Arc<Sink> {
    played.lock().unwrap().last().unwrap().clone().expect("a sink")
}

fn last(played: &Played) -> Option<&'static str> {
    played.lock().unwrap().last().and_then(|p| p.as_ref().err().copied())
}

#[test]
fn a_cast_pumps_audio_into_the_plugins_socket_until_the_plugin_stops() {
    let rig = Rig::new("plugins-lan-cast");
    let sinks = cards(&rig.plugins);
    let speaker = TcpListener::bind("127.0.0.1:0").unwrap();
    rig.web.serve("cast-socket", format!("127.0.0.1:{}", speaker.local_addr().unwrap().port()));
    assert_eq!(rig.act("cast-socket"), ["casting"]);
    let (mut heard, _) = speaker.accept().unwrap();
    let sink = sink(&sinks);
    assert_eq!(sink.wants, Some((48_000, 2)));
    sink.open(48_000, 2);
    assert!(sink.write(&[1, -1]));
    let mut got = [0; 48];
    heard.read_exact(&mut got).unwrap();
    assert_eq!((&got[..4], &got[44..]), (&b"RIFF"[..], &[1, 0, 0xff, 0xff][..]));
    let cast = rig.plugins.cast().unwrap();
    assert_eq!((cast.plugin.as_str(), cast.kind, cast.device.as_str()), (ID, CastKind::Sink, "Test speaker"));

    rig.plugins.cast_button(CastButton::VolumeUp);
    rig.settle();
    assert_eq!(rig.said("button"), Some(vec!["volume-up".into()]));
    rig.plugins.turn(ID.into(), false);
    assert_eq!(last(&sinks), Some("here"), "nori plays on its own output again");
    assert_eq!(rig.session.casts.lock().unwrap().last(), Some(&None), "the cast ends with the plugin");
    assert_eq!(rig.plugins.cast(), None);
}

#[test]
fn served_audio_reaches_each_device_that_fetches_it() {
    let rig = Rig::new("plugins-lan-served");
    let sinks = cards(&rig.plugins);
    let url = rig.act("cast-serve").remove(0);
    assert!(url.ends_with("/a/flac"), "{url}");
    let sink = sink(&sinks);
    assert_eq!((sink.wants, sink.latency_us()), (None, 250_000));
    let cast = rig.plugins.cast().unwrap();
    assert_eq!((cast.playing, cast.position_ms, cast.volume), (true, 1500, Some(40)));
    sink.open(44_100, 2);

    let mut device = TcpStream::connect(("127.0.0.1", port_of(&url))).unwrap();
    write!(device, "GET {} HTTP/1.1\r\n\r\n", &url[url.find("/a/").unwrap()..]).unwrap();
    let mut got = vec![0; 512];
    let mut n = 0;
    while !String::from_utf8_lossy(&got[..n]).contains("fLaC") {
        n += device.read(&mut got[n..]).unwrap();
    }
    assert!(String::from_utf8_lossy(&got[..n]).contains("Content-Type: audio/flac"));

    rig.act("cast-end");
    assert_eq!(last(&sinks), Some("here"));
    assert_eq!(rig.plugins.cast(), None);
}

#[test]
fn a_cast_whose_device_plays_the_songs_lets_the_output_here_go() {
    let rig = Rig::new("plugins-lan-target");
    let played = cards(&rig.plugins);
    rig.act("cast-target");
    let cast = rig.plugins.cast().unwrap();
    assert_eq!((cast.kind, cast.device.as_str()), (CastKind::Target, "TV"));
    assert_eq!(last(&played), Some("elsewhere"));
    rig.act("cast-end");
    assert_eq!(rig.session.casts.lock().unwrap().last(), Some(&None));
    assert_eq!(played.lock().unwrap().len(), 1, "nothing plays here again until asked");
}
