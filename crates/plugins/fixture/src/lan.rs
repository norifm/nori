//! The LAN and casting calls, each run by an action whose setup the test serves at
//! `https://allowed.example/lan/<action>`; what each heard comes back as a notice named after it.

use std::cell::RefCell;

use nori_plugin_sdk::audio_out::{self, Encoding};
use nori_plugin_sdk::cast::{self, Kind};
use nori_plugin_sdk::hooks::{Event, ServedRequest, ServedResponse};
use nori_plugin_sdk::http::{Method, Request};
use nori_plugin_sdk::notices::{show, Tone};
use nori_plugin_sdk::sockets::{Tcp, TlsVerify, Udp};
use nori_plugin_sdk::{discovery, lan_serve, media};

thread_local! {
    /// What a cast keeps open while it runs.
    static CASTING: RefCell<Option<Tcp>> = const { RefCell::new(None) };
    /// A socket in a multicast group, until `leave`.
    static JOINED: RefCell<Option<Udp>> = const { RefCell::new(None) };
}

fn setup(action: &str) -> Vec<u8> {
    let r = nori_plugin_sdk::http::fetch(&Request { method: Method::Get, url: format!("https://allowed.example/lan/{action}"), headers: Vec::new(), body: None, timeout_ms: None });
    r.map(|r| r.body).unwrap_or_default()
}

fn said<T>(r: Result<T, nori_plugin_sdk::types::Error>, ok: impl FnOnce(T) -> String) -> String {
    match r {
        Ok(v) => ok(v),
        Err(e) => format!("{:?}", e.kind),
    }
}

/// `host:port` from the setup.
fn address(setup: &[u8]) -> (String, u16) {
    let text = String::from_utf8_lossy(setup);
    let (host, port) = text.rsplit_once(':').unwrap_or_default();
    (host.to_string(), port.trim().parse().unwrap_or_default())
}

/// Sends "ping" over TLS checked as the setup's first line says (`system`, `pinned`, `none`), the
/// certificate after it; says the answer.
fn tls() -> String {
    let setup = setup("tls");
    let (head, cert) = setup.split_at(setup.iter().position(|&b| b == b'\n').unwrap_or(setup.len()));
    let text = String::from_utf8_lossy(head);
    let (mode, at) = text.split_once(' ').unwrap_or_default();
    let (host, port) = address(at.as_bytes());
    let verify = match mode {
        "pinned" => TlsVerify::Pinned(cert[1..].to_vec()),
        "none" => TlsVerify::None,
        _ => TlsVerify::System,
    };
    said(Tcp::connect(&host, port, 2000), |tcp| {
        said(tcp.tls("nori.test", &verify), |()| {
            let _ = tcp.write(b"ping");
            said(tcp.read(64, 2000), |b| String::from_utf8_lossy(&b).into_owned())
        })
    })
}

/// Sends "hello" to the setup's address and says the reply and who sent it.
fn udp() -> String {
    let (host, port) = address(&setup("udp"));
    said(Udp::bind(0), |u| {
        said(u.send(&host, port, b"hello"), |()| {
            said(u.receive(2000), |got| match got {
                Some((from, _, bytes)) => format!("{from} {}", String::from_utf8_lossy(&bytes)),
                None => "none".into(),
            })
        })
    })
}

pub fn action(id: &str) {
    let heard = match id {
        "tls" => tls(),
        "udp" => udp(),
        "join" => said(Udp::bind(0), |u| {
            let r = u.join("239.255.255.250");
            JOINED.with(|j| *j.borrow_mut() = Some(u));
            said(r, |()| "joined".into())
        }),
        "leave" => {
            JOINED.with(|j| j.borrow_mut().take());
            "left".into()
        }
        "browse" => {
            discovery::browse("_test._tcp");
            "browsing".into()
        }
        "stop-browsing" => {
            discovery::stop("_test._tcp");
            "stopped".into()
        }
        "serve" => {
            lan_serve::open();
            lan_serve::base().unwrap_or_default()
        }
        "media" => said(media::stream_url("s1"), |u| u),
        "cast-socket" => {
            let (host, port) = address(&setup("cast-socket"));
            said(Tcp::connect(&host, port, 2000), |tcp| {
                let r = audio_out::to_socket(&tcp, Encoding::Wav, 48_000, 2);
                CASTING.with(|c| *c.borrow_mut() = Some(tcp));
                cast::start(Kind::Sink, "Test speaker");
                said(r, |()| "casting".into())
            })
        }
        "cast-serve" => {
            let r = audio_out::serve(Encoding::Flac);
            audio_out::set_latency(250);
            cast::start(Kind::Sink, "Test speaker");
            cast::report(true, 1500, Some(40));
            said(r, |u| u)
        }
        "cast-target" => {
            cast::start(Kind::Target, "TV");
            "casting".into()
        }
        "cast-end" => {
            audio_out::stop();
            cast::end();
            CASTING.with(|c| c.borrow_mut().take());
            "ended".into()
        }
        _ => return,
    };
    show(Tone::Info, id, &[heard]);
}

pub fn on(event: Event) {
    match event {
        Event::Discovered(s) => show(Tone::Info, "found", &[s.name, s.host, s.port.to_string()]),
        Event::MediaButton(b) => show(Tone::Info, "button", &[b]),
        _ => {}
    }
}

/// Says what was asked of it.
pub fn served(r: ServedRequest) -> Option<ServedResponse> {
    let body = format!("{} {} {}", r.method, r.path, String::from_utf8_lossy(&r.body)).into_bytes();
    Some(ServedResponse { status: 200, headers: vec![("Content-Type".into(), "text/plain".into())], body })
}
