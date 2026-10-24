//! A listen is written on the background thread: a profile opened over the same queue session meanwhile
//! does not take it.

use std::sync::mpsc::channel;
use std::sync::Arc;

use nori_core::queue::Session;
use nori_core::scrobble::TrackChange;
use nori_core::{background, Core, Song};

#[test]
fn listen_kept_by_its_profile() {
    let session = Arc::new(Session::default());
    let first = Core::new(String::new(), "first".into(), session.clone()).unwrap();
    let song = Song { id: "s".into(), title: "Dogs".into(), duration: 600, ..Default::default() };
    session.register(vec![song]);
    session.scrobble_track(Some("s".into()), TrackChange::Moved, true, 0, 1_000, 0);
    let (go, wait) = channel::<()>();
    background::run(move || {
        let _ = wait.recv();
    });
    session.scrobble_track(None, TrackChange::Ended, false, 120_000, 121_000, 0);
    let _second = Core::new(String::new(), "second".into(), session).unwrap();
    go.send(()).unwrap();
    background::flush();
    assert_eq!(first.history_recent(10, None, true).unwrap().0.len(), 1);
}

#[test]
fn a_handed_listen_preserves_other_queue_entries() {
    use nori_core::Listen;

    let session = Session::default();
    let song = Song { id: "same".into(), duration: 200, ..Default::default() };
    session.register(vec![song]);
    session.scrobble_track(Some("same".into()), TrackChange::Moved, true, 0, 1_000, 0);
    session.receive_listen(Some(Listen { song_id: "same".into(), started_ms: 500, heard_ms: 60_000 }));
    let old = session.scrobble_track(Some("same".into()), TrackChange::Moved, true, 120_000, 121_000, 0);
    assert_eq!((old.submit_id.as_deref(), old.submit_at), (Some("same"), 1_000), "the receiving device's old queue entry still counts");
    session.scrobble_playing(false, 140_000);
    session.scrobble_playing(true, 200_000);
    let handed = session.scrobble_track(Some("same".into()), TrackChange::Looped, true, 240_000, 241_000, 0);
    assert_eq!((handed.submit_id.as_deref(), handed.submit_at), (Some("same"), 500), "the handed listen excludes the pause");
    let repeated = session.scrobble_track(None, TrackChange::Ended, false, 300_000, 301_000, 0);
    assert_eq!(repeated.submit_id, None, "a repeated queue entry does not inherit handed time");
}

#[test]
fn a_listen_left_after_a_profile_switch_stays_with_its_profile() {
    let session = Arc::new(Session::default());
    let first = Core::new(String::new(), "first".into(), session.clone()).unwrap();
    let song = Song { id: "yt-a".into(), title: "Last Resort".into(), duration: 200, ..Default::default() };
    let other = Song { id: "b".into(), title: "Session".into(), duration: 200, ..Default::default() };
    session.register(vec![song, other]);
    session.scrobble_track(Some("yt-a".into()), TrackChange::Moved, true, 0, 1_000, 0);
    let second = Core::new(String::new(), "second".into(), session.clone()).unwrap();
    let left = session.scrobble_track(Some("b".into()), TrackChange::Moved, true, 150_000, 151_000, 0);
    background::flush();
    assert_eq!(left.submit_id, None, "not scrobbled to the second profile's server");
    assert_eq!(second.history_recent(10, None, true).unwrap().0.len(), 0);
    assert_eq!(first.history_recent(10, None, true).unwrap().0.len(), 1);
}
