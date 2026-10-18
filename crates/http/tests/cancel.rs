//! A song let go while its request waits on a server that sends nothing more.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

use nori_engine::Loader;
use nori_http::Http;

#[derive(Debug, PartialEq)]
enum Heard {
    /// The server stopped sending.
    Holding,
    HungUp,
}

/// Answers with a head (when `answers`) and no body bytes, then holds until the client hangs up.
fn server(answers: bool, heard: Sender<Heard>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut c, _) = listener.accept().unwrap();
        let mut got = Vec::new();
        let mut b = [0; 1];
        while !got.ends_with(b"\r\n\r\n") && c.read(&mut b).is_ok_and(|n| n > 0) {
            got.push(b[0]);
        }
        if answers {
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n");
        }
        let _ = heard.send(Heard::Holding);
        hold(c);
        let _ = heard.send(Heard::HungUp);
    });
    port
}

fn hold(mut c: TcpStream) {
    while c.read(&mut [0; 64]).is_ok_and(|n| n > 0) {}
}

#[test]
fn letting_a_song_go_ends_its_blocked_request_at_once() {
    for answers in [false, true] {
        let (heard_to, heard) = channel();
        let url = format!("http://127.0.0.1:{}/song", server(answers, heard_to));
        let loader = Loader::start(Http::new(), url, [0; 5], None, None);
        assert_eq!(heard.recv_timeout(Duration::from_secs(10)), Ok(Heard::Holding));
        // The client is blocked reading by now.
        std::thread::sleep(Duration::from_millis(20));
        drop(loader);
        assert_eq!(heard.recv_timeout(Duration::from_millis(100)), Ok(Heard::HungUp), "answers: {answers}");
    }
}
