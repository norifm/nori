use super::*;
use nori_core::remote::RemotePlayer;
use nori_engine::testing::{sine, wav, Stepper, Virtual};
use super::test_card::Card;
use std::io::Read as _;
use std::time::Duration;

/// A server that holds every autofill request (similar songs) unanswered and turns the rest away; the
/// paths of the held ones come out of the receiver.
fn holding_server() -> (String, std::sync::mpsc::Receiver<String>) {
    use std::io::Write as _;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (asked, paths) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for mut c in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let n = c.read(&mut buf).unwrap_or(0);
            let line = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_string();
            if !line.contains("getSimilarSongs2") {
                let _ = c.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                continue;
            }
            held.push(c);
            if asked.send(line).is_err() {
                return;
            }
        }
    });
    (url, paths)
}

/// Waits until the session's tasks ran what was spawned before: the engine's events, which the
/// session hears among its tasks, off the virtual clock.
fn caught_up(s: &Session) {
    let (done, ran) = std::sync::mpsc::channel();
    s.run(async move { done.send(()).unwrap() });
    ran.recv().unwrap();
}

#[test]
fn late_autofill_respects_playback_and_new_queues() {
    for action in ["ended", "paused", "replaced", "stale_resume"] {
        let dir = nori_testdir::TempDir::new("host-refill");
        let prefs = nori_core::settings_store::Settings::new();
        let mut p = prefs.open(&dir.join("app.db").to_string_lossy()).unwrap();
        p.remote_control = false;
        p.auto_fill = true;
        p.auto_mix = false;
        p.third_party_lookups = false;
        prefs.put(p);
        let queue = Arc::new(nori_core::queue::Session::new(prefs));
        let card = Card::new();
        let clock = Virtual::default();
        let time = Stepper::new(clock.clone(), card.pull.clone());
        let (url, asked) = holding_server();
        let profile = SavedServer { id: "test".into(), url, user: "ann".into(), password: "pw".into(), ..Default::default() };
        let s = Session::open_on(test_open(queue, dir.path(), profile, Box::new(card.clone())), clock).unwrap();
        let songs: Vec<_> = ["a", "b", "c", "d"].into_iter().enumerate().map(|(i, id)| Song {
            id: id.into(), title: id.into(), duration: 1, suffix: "wav".into(), album_id: Some(if i < 2 { "album" } else { id }.into()), ..Default::default()
        }).collect();
        s.core.download_queue(songs.clone()).unwrap();
        s.core.download_settle(songs.iter().map(|s| s.id.clone()).collect(), vec![true; songs.len()]).unwrap();
        for (i, song) in songs.iter().enumerate() {
            let path = s.store.as_ref().expect("a store").download_path(&song.id);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, wav(44_100, &sine(44_100, 220.0 * (i + 1) as f64, 1.0, 16_000.0))).unwrap();
        }
        s.core.session.register(songs.clone());
        s.play(songs[..2].to_vec(), 0, false, Some(PageOrigin::new(nori_core::OriginKind::Album, "album")));
        time.clock.settle();
        // The session's own request, which this test answers itself (`filled`) once the queue has moved on.
        assert!(asked.recv_timeout(Duration::from_secs(60)).is_ok(), "the album begins its autofill request");
        let previous_end = s.core.session.playlist(|p| p.seqs()[1]);
        if action == "paused" {
            time.run(Duration::from_millis(1_200));
            s.engine.pause();
            assert!(time.until(Duration::from_secs(2), || s.engine.status().state == State::Paused));
        } else {
            assert!(time.until(Duration::from_secs(5), || s.engine.status().state == State::Ended));
        }
        if action == "replaced" {
            let before = card.heard.lock().len();
            crate::remote::HostPlayer((*s).clone()).apply(nori_remote::wire::Op::Replace {
                songs: vec![songs[3].clone()], index: 0, position_ms: 0, play: true, order: None, shuffle: false, repeat: 0, transfer: None, listen: None,
            });
            assert!(time.until(Duration::from_secs(3), || card.heard.lock().len() >= before + 88_200), "a remote play starts without local playback");
            assert!((card.last_second_hz() - 880.0).abs() <= 1.0);
        }
        if action == "stale_resume" {
            crate::remote::HostPlayer((*s).clone()).apply(nori_remote::wire::Op::Replace {
                songs: vec![songs[3].clone(), songs[1].clone()], index: 0, position_ms: 0, play: true, order: None, shuffle: false, repeat: 0, transfer: None, listen: None,
            });
            assert!(time.until(Duration::from_secs(2), || s.engine.status().state == State::Playing));
            assert!(time.until(Duration::from_secs(5), || s.engine.status().state == State::Ended));
            let len = s.core.session.playlist(|p| p.len());
            s.core.session.take(len as u32, vec!["c".into()], vec![Hand::No]);
            s.engine.queue_changed();
            s.engine.resume_after_refill(previous_end);
        }
        caught_up(&s);
        let before = card.heard.lock().len();
        s.filled(nori_core::autofill::Refill { songs: vec![songs[2].clone()], after: Some(previous_end), ..Default::default() });
        time.run(Duration::from_secs(2));
        let heard = card.heard.lock().len() - before;
        let ids = s.core.session.playlist(|p| p.ids().to_vec());
        s.close();
        match action {
            "ended" => assert!(heard >= 88_200, "late autoplay must produce the next song: heard {heard} samples"),
            "paused" => assert_eq!(heard, 0, "autoplay preserves an explicit pause"),
            "replaced" => assert_eq!(ids, ["d"], "the old album's refill leaves the new queue alone"),
            "stale_resume" => assert_eq!(heard, 0, "the same song in a new queue has a different entry"),
            _ => unreachable!(),
        }
    }
}

/// A session of the server at `url` over four one-second songs, kept as downloads under `dir`.
fn session_of_songs(dir: &nori_testdir::TempDir, url: &str, card: &Card, clock: Virtual) -> (Session, Vec<Song>) {
    let prefs = nori_core::settings_store::Settings::new();
    let mut p = prefs.open(&db_path(dir.path())).unwrap();
    p.remote_control = false;
    p.auto_fill = false;
    p.auto_mix = false;
    prefs.put(p);
    let queue = Arc::new(nori_core::queue::Session::new(prefs));
    let profile = SavedServer { id: "test".into(), url: url.into(), user: "ann".into(), password: "pw".into(), ..Default::default() };
    let s = Session::open_on(test_open(queue, dir.path(), profile, Box::new(card.clone())), clock).unwrap();
    let songs: Vec<Song> = ["a", "b", "c", "d"].into_iter().map(|id| Song { id: id.into(), title: id.into(), duration: 1, suffix: "wav".into(), album_id: Some("album".into()), ..Default::default() }).collect();
    s.core.download_queue(songs.clone()).unwrap();
    s.core.download_settle(songs.iter().map(|s| s.id.clone()).collect(), vec![true; songs.len()]).unwrap();
    for (i, song) in songs.iter().enumerate() {
        let path = s.store.as_ref().expect("a store").download_path(&song.id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, wav(44_100, &sine(44_100, 220.0 * (i + 1) as f64, 1.0, 16_000.0))).unwrap();
    }
    s.core.session.register(songs.clone());
    (s, songs)
}

/// The media keys are the session's controls: play on a queue restored but never started starts it
/// where it was, and next at the queue's end asks for autofill songs.
#[cfg(feature = "desktop")]
#[test]
fn media_keys_start_a_restored_queue_and_fill_at_its_end() {
    let dir = nori_testdir::TempDir::new("host-keys");
    let (url, asked) = holding_server();
    let (card, clock) = (Card::new(), Virtual::default());
    let time = Stepper::new(clock.clone(), card.pull.clone());
    let (s, songs) = session_of_songs(&dir, &url, &card, clock);
    s.play(songs[..2].to_vec(), 1, false, None);
    s.engine.pause();
    assert!(time.until(Duration::from_secs(2), || s.engine.status().state == State::Paused));
    s.close();

    let (card, clock) = (Card::new(), Virtual::default());
    let time = Stepper::new(clock.clone(), card.pull.clone());
    let (s, _) = session_of_songs(&dir, &url, &card, clock);
    nori_mpris::Controls::toggle(&*s);
    assert!(time.until(Duration::from_secs(3), || card.heard.lock().len() >= 88_200), "the restored queue plays");
    assert!((card.last_second_hz() - 440.0).abs() <= 1.0, "from its own song: {} Hz", card.last_second_hz());

    // Switched on only now: the songs arriving asked for nothing, so the press is what asks.
    s.setting("autoFill", "true").expect("a setting");
    nori_mpris::Controls::next(&*s);
    assert!(asked.recv_timeout(Duration::from_secs(60)).is_ok(), "next at the end asks for more songs");
    s.close();
}


/// A server that answers every request with `body`.
fn answering_server(body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut c in listener.incoming().flatten() {
            use std::io::Write as _;
            let mut buf = [0u8; 4096];
            let _ = c.read(&mut buf);
            let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    url
}

/// A radio from the song playing goes on with it where it is; only the songs after it change.
#[test]
fn a_radio_from_the_song_playing_keeps_it_playing() {
    let dir = nori_testdir::TempDir::new("host-radio");
    let url = answering_server(r#"{"subsonic-response":{"status":"ok","version":"1.16.1","similarSongs2":{"song":[{"id":"b","title":"b","duration":1,"suffix":"wav"},{"id":"c","title":"c","duration":1,"suffix":"wav"}]}}}"#);
    let (card, clock) = (Card::new(), Virtual::default());
    let time = Stepper::new(clock.clone(), card.pull.clone());
    let (s, songs) = session_of_songs(&dir, &url, &card, clock);
    let a = Song { duration: 6, ..songs[0].clone() };
    let path = s.store.as_ref().expect("a store").download_path("a");
    std::fs::write(path, wav(44_100, &sine(44_100, 220.0, 6.0, 16_000.0))).unwrap();
    s.core.session.register(vec![a.clone()]);
    s.play(vec![a], 0, false, None);
    assert!(time.until(Duration::from_secs(5), || card.secs() >= 2.0), "a plays");
    s.play_later(Fetch::Radio("a".into()), false);
    // The answer comes on the network's own time, with the card's clock held.
    assert!(!time.clock.hang_while(|| s.core.session.playlist(|p| p.len()) != 3, Duration::from_secs(10)), "the radio is queued");
    assert!(time.until(Duration::from_secs(10), || (card.last_second_hz() - 440.0).abs() <= 1.0), "b follows a");
    let heard = card.secs();
    s.close();
    assert!(heard < 7.5, "a played once, not again from its start: {heard:.1} s before b's first second");
}
