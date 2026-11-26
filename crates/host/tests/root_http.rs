//! What reaches the web outside a session (a jam joined, plugins before one opens) goes over the active
//! profile's connections, with its server's headers, or plain ones while no profile is saved.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc::channel;
use std::time::Duration;

use nori_core::settings::{SavedServer, StoredPrefs};
use nori_core::transport::{block_on, Transport};

#[test]
fn outside_a_session_the_active_profiles_connections_are_used() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, heard) = channel();
    std::thread::spawn(move || {
        for c in listener.incoming() {
            let mut c = c.unwrap();
            let token = BufReader::new(&c).lines().map_while(Result::ok).take_while(|l| !l.is_empty()).find_map(|l| l.strip_prefix("X-Token: ").or_else(|| l.strip_prefix("x-token: ")).map(str::to_string));
            c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            tx.send(token).unwrap();
        }
    });
    let dir = nori_testdir::TempDir::new("root-http");
    let root = nori_host::desktop::Root::new(dir.path());
    root.open_settings().unwrap();
    let ask = |root: &nori_host::desktop::Root| {
        block_on(root.http().get(format!("{server}/rest/ping"), 0)).unwrap();
        heard.recv_timeout(Duration::from_secs(10)).unwrap()
    };
    assert_eq!(ask(&root), None, "no profile, plain");
    let profile = SavedServer { id: "p".into(), url: server.clone(), headers: [("X-Token".to_string(), "t0ken".to_string())].into(), ..Default::default() };
    root.app.settings.put(StoredPrefs { servers: vec![profile], active_server_id: "p".into(), ..root.app.settings.current().unwrap() });
    assert_eq!(ask(&root).as_deref(), Some("t0ken"));
}
