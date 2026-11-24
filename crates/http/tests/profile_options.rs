//! A profile's advanced options over the desktop's transport: its headers and TLS exceptions go to its
//! server at either address and nowhere else, its client certificate is what a server asking for one
//! sees, and the core's second address, API key and legacy sign-in reach the server through it.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use nori_core::settings::SavedServer;
use nori_core::transport::{block_on, Transport};
use nori_engine::ByteSource;
use nori_http::Http;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const WAIT: Duration = Duration::from_secs(10);
const SERVER_CERT: &[u8] = include_bytes!("data/proxy-cert.der");
const SERVER_KEY: &[u8] = include_bytes!("data/proxy-key.der");
const CLIENT_CA: &[u8] = include_bytes!("../../cert/tests/data/client-ca.der");
const OK: &str = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;

/// What a server heard of one request: its request line and headers (names in lower case).
#[derive(Debug)]
struct Heard {
    line: String,
    headers: HashMap<String, String>,
}

/// Reads a request's head off `s` and answers it with `answer(request line)`, a whole response.
fn answer(s: &mut (impl Read + Write), answer: &dyn Fn(&str) -> String) -> Heard {
    let mut reader = BufReader::new(s);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let Some((name, value)) = h.trim_end().split_once(':') else { break };
        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
    }
    let response = answer(line.trim_end());
    let s = reader.into_inner();
    s.write_all(response.as_bytes()).unwrap();
    s.flush().unwrap();
    Heard { line: line.trim_end().to_string(), headers }
}

/// A 200 response with `body`.
fn ok(body: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

/// A plain HTTP server on 127.0.0.1 answering every request with `body(request line)`; its address and
/// what it hears.
fn server(body: fn(&str) -> String) -> (String, Receiver<Heard>) {
    responding(move |line| ok(&body(line)))
}

/// A plain HTTP server on 127.0.0.1 sending every request to `to`'s same path.
fn redirecting(to: String) -> (String, Receiver<Heard>) {
    responding(move |line| {
        let path = line.split(' ').nth(1).unwrap_or("/");
        format!("HTTP/1.1 302 Found\r\nLocation: {to}{path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    })
}

/// A plain HTTP server on 127.0.0.1 answering every request with `respond(request line)`, a whole
/// response; its address and what it hears.
fn responding(respond: impl Fn(&str) -> String + Send + 'static) -> (String, Receiver<Heard>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, heard) = channel();
    std::thread::spawn(move || {
        for c in listener.incoming() {
            let _ = tx.send(answer(&mut c.unwrap(), &respond));
        }
    });
    (address, heard)
}

/// An HTTPS server on 127.0.0.1 with a self-signed certificate, asking for a client certificate from
/// the test CA when `asks` is set; its address and, for each handshake, the certificate the client sent
/// (None: none, or the handshake failed).
fn tls_server(asks: bool) -> (String, Receiver<Option<Vec<u8>>>) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions().unwrap();
    let builder = if asks {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(CLIENT_CA.to_vec())).unwrap();
        builder.with_client_cert_verifier(rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider).build().unwrap())
    } else {
        builder.with_no_client_auth()
    };
    let config = Arc::new(builder.with_single_cert(vec![CertificateDer::from(SERVER_CERT.to_vec())], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(SERVER_KEY.to_vec()))).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("https://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, seen) = channel();
    std::thread::spawn(move || {
        for c in listener.incoming() {
            let mut tls = rustls::StreamOwned::new(rustls::ServerConnection::new(config.clone()).unwrap(), c.unwrap());
            if tls.conn.complete_io(&mut tls.sock).is_err() {
                let _ = tx.send(None);
                continue;
            }
            let sent = tls.conn.peer_certificates().and_then(|c| c.first()).map(|c| c.to_vec());
            answer(&mut tls, &|_| ok("hello"));
            let _ = tx.send(sent);
        }
    });
    (address, seen)
}

fn get(http: &Http, url: &str) -> Result<String, String> {
    block_on(http.get(url.into(), 0)).map(|r| String::from_utf8(r.body).unwrap()).map_err(|e| format!("{e:?}"))
}

fn opened(http: &Http, url: &str) -> Result<String, String> {
    let mut got = String::new();
    http.open(url, 0).map_err(|e| format!("{e:?}"))?.reader.read_to_string(&mut got).unwrap();
    Ok(got)
}

fn profile(url: &str, alt: &str) -> SavedServer {
    SavedServer { url: url.into(), alt_url: alt.into(), ..Default::default() }
}

/// The headers reach the server at both its addresses, by the core's requests and the engine's, and
/// never another host.
#[test]
fn headers_go_to_the_server_only() {
    let (first, at_first) = server(|_| "a".into());
    let (second, at_second) = server(|_| "b".into());
    let (other, at_other) = server(|_| "c".into());
    let headers = HashMap::from([("X-Token".to_string(), "t0ken".to_string())]);
    let http = Http::for_profile(&SavedServer { headers, ..profile(&first, &second) }, Path::new(""));
    get(&http, &format!("{first}/rest/ping")).unwrap();
    opened(&http, &format!("{second}/rest/stream")).unwrap();
    get(&http, &format!("{other}/api/get")).unwrap();
    opened(&http, &format!("{other}/song.mp3")).unwrap();
    for heard in [&at_first, &at_second] {
        assert_eq!(heard.recv_timeout(WAIT).unwrap().headers.get("x-token").map(String::as_str), Some("t0ken"));
    }
    for _ in 0..2 {
        assert_eq!(at_other.recv_timeout(WAIT).unwrap().headers.get("x-token"), None, "a third party gets none");
    }
}

/// Each hop of a redirect is decided again: one leaving the server carries none of its headers, by the
/// core's requests and the engine's, and one coming to it carries them.
#[test]
fn a_redirect_takes_the_headers_only_to_the_server() {
    let (other, at_other) = server(|_| "c".into());
    let (first, at_first) = redirecting(other.clone());
    let (second, at_second) = server(|_| "b".into());
    let (third_party, at_third_party) = redirecting(second.clone());
    let headers = HashMap::from([("X-Token".to_string(), "t0ken".to_string())]);
    let http = Http::for_profile(&SavedServer { headers, ..profile(&first, &second) }, Path::new(""));
    assert_eq!(get(&http, &format!("{first}/rest/ping")).as_deref(), Ok("c"));
    assert_eq!(opened(&http, &format!("{first}/rest/stream")).as_deref(), Ok("c"));
    assert_eq!(get(&http, &format!("{third_party}/rest/ping")).as_deref(), Ok("b"));
    for _ in 0..2 {
        assert_eq!(at_first.recv_timeout(WAIT).unwrap().headers.get("x-token").map(String::as_str), Some("t0ken"));
        assert_eq!(at_other.recv_timeout(WAIT).unwrap().headers.get("x-token"), None, "redirected away, none");
    }
    assert_eq!(at_third_party.recv_timeout(WAIT).unwrap().headers.get("x-token"), None);
    assert_eq!(at_second.recv_timeout(WAIT).unwrap().headers.get("x-token").map(String::as_str), Some("t0ken"), "redirected to the server, carried");
}

/// A self-signed certificate is refused unless the profile accepts one, and then only its server's.
#[test]
fn a_self_signed_server_is_reached_only_when_accepted() {
    let (server, _) = tls_server(false);
    let (other, _) = tls_server(false);
    let accepting = Http::for_profile(&SavedServer { allow_self_signed: true, ..profile(&server, "") }, Path::new(""));
    assert_eq!(get(&accepting, &format!("{server}/rest/ping")).as_deref(), Ok("hello"));
    assert_eq!(opened(&accepting, &format!("{server}/rest/stream")).as_deref(), Ok("hello"));
    assert!(get(&accepting, &format!("{other}/api")).is_err(), "another host's is still checked");
    let checking = Http::for_profile(&profile(&server, ""), Path::new(""));
    assert!(get(&checking, &format!("{server}/rest/ping")).is_err());
}

/// The client certificate imported is the one a server asking for it sees; without it the server
/// refuses the connection.
#[test]
fn a_client_certificate_is_sent_to_its_server() {
    let data = nori_testdir::TempDir::new("client-cert");
    let (server, seen) = tls_server(true);
    let base = SavedServer { id: "p1".into(), allow_self_signed: true, ..profile(&server, "") };
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cert/tests/data/client-aes.p12");
    let with = nori_core::cert::import_client_cert_file(data.path(), base.clone(), &from, "secret".into()).unwrap();
    assert_eq!(get(&Http::for_profile(&with, data.path()), &format!("{server}/rest/ping")).as_deref(), Ok("hello"));
    assert_eq!(seen.recv_timeout(WAIT).unwrap().as_deref(), Some(include_bytes!("../../cert/tests/data/client-cert.der").as_slice()));
    assert!(get(&Http::for_profile(&base, data.path()), &format!("{server}/rest/ping")).is_err());
    assert_eq!(seen.recv_timeout(WAIT).unwrap(), None);
}

/// The core's sign-in check over this transport: a first address that does not answer gives way to the
/// second; an API key signs in in place of a user and password; a server without token auth is signed
/// in to the legacy way.
#[test]
fn the_core_signs_in_through_it() {
    let dead = format!("http://127.0.0.1:{}", TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port());
    let (live, heard) = server(|_| OK.into());
    let http: Arc<dyn Transport> = Http::for_profile(&profile(&dead, &live), Path::new(""));
    let config = |api_key: Option<&str>| nori_core::ServerConfig { url: dead.clone(), user: "ann".into(), password: "pw".into(), api_key: api_key.map(str::to_string), legacy_auth: false };
    assert!(!block_on(nori_core::providers::login_check(http.clone(), config(None), live.clone())).unwrap());
    assert!(heard.recv_timeout(WAIT).unwrap().line.contains("u=ann&t="), "by the second address, with a token");
    block_on(nori_core::providers::login_check(http, config(Some("k3y")), live.clone())).unwrap();
    let line = heard.recv_timeout(WAIT).unwrap().line;
    assert!(line.contains("apiKey=k3y") && !line.contains("u=ann"), "{line}");

    let (old, heard) = server(|line| if line.contains("&t=") { r#"{"subsonic-response":{"status":"failed","error":{"code":41,"message":"token auth not supported"}}}"#.into() } else { OK.into() });
    let http: Arc<dyn Transport> = Http::for_profile(&profile(&old, ""), Path::new(""));
    let legacy = nori_core::ServerConfig { url: old.clone(), ..config(None) };
    assert!(block_on(nori_core::providers::login_check(http, legacy, String::new())).unwrap(), "kept as legacy");
    heard.recv_timeout(WAIT).unwrap();
    assert!(heard.recv_timeout(WAIT).unwrap().line.contains("u=ann&p=enc:7077"), "the password, hex-encoded");
}
