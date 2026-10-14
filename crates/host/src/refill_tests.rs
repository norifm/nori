use super::*;
use nori_core::remote::RemotePlayer;
use nori_engine::testing::{sine, wav, Stepper, Virtual};
use super::test_card::Card;
use std::time::Duration;

#[test]
fn late_autofill_respects_playback_and_new_queues() {
    for action in ["ended", "paused", "replaced", "stale_resume"] {
        let dir = nori_testdir::TempDir::new("host-refill");
        let prefs = nori_core::settings_store::Settings::new();
        let mut p = prefs.open(&dir.join("app.db").to_string_lossy()).unwrap();
        p.remote_control = false;
        p.auto_fill = true;
        p.auto_mix = false;
        p.lyrics_prefetch = false;
        prefs.put(p);
        let queue = Arc::new(nori_core::queue::Session::new(prefs));
        let card = Card::new();
        let clock = Virtual::default();
        let time = Stepper::new(clock.clone(), card.pull.clone());
        let s = Session::open_on(Open {
            queue,
            data: dir.path(),
            http: Http::new(),
            profile: SavedServer { id: "test".into(), url: "http://127.0.0.1:9".into(), user: "ann".into(), password: "pw".into(), ..Default::default() },
            output: Box::new(card.clone()),
            volume: Level::new(1.0, None),
            memory_mb: 16,
            covers: None,
            offline: false,
            mpris: None,
            device: nori_core::remote::RemoteMe { name: "iPod".into(), kind: nori_remote::wire::DeviceKind::Phone },
            discovery: None,
            out: Arc::new(|_| {}),
        }, Some(clock)).unwrap();
        let songs: Vec<_> = ["a", "b", "c", "d"].into_iter().enumerate().map(|(i, id)| Song {
            id: id.into(), title: id.into(), duration: 1, suffix: "wav".into(), album_id: Some(if i < 2 { "album" } else { id }.into()), ..Default::default()
        }).collect();
        s.core.download_queue(songs.clone()).unwrap();
        s.core.download_settle(songs.iter().map(|s| s.id.clone()).collect(), vec![true; songs.len()]).unwrap();
        for (i, song) in songs.iter().enumerate() {
            let path = s.store.download_path(&song.id);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, wav(44_100, &sine(44_100, 220.0 * (i + 1) as f64, 1.0, 16_000.0))).unwrap();
        }
        s.core.session.register(songs.clone());
        s.play(songs[..2].to_vec(), 0, false, Some(PageOrigin::new(nori_core::OriginKind::Album, "album")));
        time.clock.settle();
        assert!(s.core.session.song_arrived().fill, "the album begins its autofill request");
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
            crate::remote::HostPlayer(s.handle()).apply(nori_remote::wire::Op::Replace {
                songs: vec![songs[3].clone()], index: 0, position_ms: 0, play: true, order: None, shuffle: false, repeat: 0, transfer: None, listen: None,
            });
            assert!(time.until(Duration::from_secs(3), || card.heard.lock().len() >= before + 88_200), "a remote play starts without local playback");
            assert!((card.last_second_hz() - 880.0).abs() <= 1.0);
        }
        if action == "stale_resume" {
            crate::remote::HostPlayer(s.handle()).apply(nori_remote::wire::Op::Replace {
                songs: vec![songs[3].clone(), songs[1].clone()], index: 0, position_ms: 0, play: true, order: None, shuffle: false, repeat: 0, transfer: None, listen: None,
            });
            assert!(time.until(Duration::from_secs(2), || s.engine.status().state == State::Playing));
            assert!(time.until(Duration::from_secs(5), || s.engine.status().state == State::Ended));
            let len = s.core.session.playlist(|p| p.len());
            s.core.session.take(len as u32, vec!["c".into()], vec![Hand::No]);
            s.engine.queue_changed();
            s.engine.resume_after_refill(previous_end);
        }
        let before = card.heard.lock().len();
        s.handle().filled(nori_core::autofill::Refill { songs: vec![songs[2].clone()], picked: None, album: None });
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
