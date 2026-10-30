//! The deciders point over the test plugin: each decider hands it nori's answer, takes its own only
//! where nori's check lets it, and keeps nori's past the budget.

mod common;

use std::time::{Duration, Instant};

use nori_model::deciders::{ButtonAct, Deciders, Focus, FocusAct, Press, Shown};
use nori_model::Song;
use nori_player::outputs::OutputPort;

fn song(id: &str, title: &str) -> Song {
    Song { id: id.into(), title: title.into(), artist: "A".into(), ..Default::default() }
}

fn anything<T: ?Sized>(_: &T) -> bool {
    true
}

#[test]
fn deciders_answer_for_what_they_mark_and_leave_the_rest_to_nori() {
    let dir = nori_testdir::TempDir::new("plugins-deciders");
    let (plugins, side) = common::plugged(&dir, "");
    assert!(plugins.deciding());

    let marked = [song("a", "x"), song("b", "fixture shuffle"), song("c", "y")];
    assert_eq!(plugins.shuffle(&marked, &[1, 0, 2], &|o| anything(o)), Some(vec![1, 2, 0]));
    assert_eq!(plugins.shuffle(&marked, &[1, 0, 2], &|o| o[0] == 0), None, "refused by nori's check");
    assert_eq!(plugins.shuffle(&[song("a", "x"), song("b", "y")], &[0, 1], &|o| anything(o)), None);

    assert_eq!(plugins.leveling(&song("l", "fixture level"), -2.0, &|_| true), Some(-5.0));
    assert_eq!(plugins.leveling(&song("l", "fixture loud"), -2.0, &|db| db < 6.0), None);
    assert_eq!(plugins.sound_profile(OutputPort::Other, "fixture output", "", &|n| n == "Fixture").as_deref(), Some("Fixture"));
    assert_eq!(plugins.sound_profile(OutputPort::Other, "fixture output", "", &|n| n.is_empty()), None);

    let nori = Shown { title: "t".into(), artist: "A".into(), album: "B".into() };
    assert_eq!(plugins.now_playing(&song("n", "fixture shown"), &nori, &|_| true), Some(Shown { title: "Shown by the fixture".into(), ..nori.clone() }));
    assert_eq!(plugins.now_playing(&song("n", "other"), &nori, &|_| true), None);

    assert_eq!(plugins.media_button(Press::Triple, ButtonAct::Previous), Some(ButtonAct::PlayPause));
    assert_eq!(plugins.media_button(Press::Next, ButtonAct::Next), None);
    assert_eq!(plugins.media_button(Press::Long, ButtonAct::Nothing), Some(ButtonAct::Nothing), "its own action instead");
    let waited = Instant::now();
    while !side.notices.lock().unwrap().iter().any(|(w, _)| w == "pressed") {
        assert!(waited.elapsed() < Duration::from_secs(10), "its action ran");
        std::thread::yield_now();
    }
    assert_eq!(plugins.audio_focus(Focus::Duck, FocusAct::Duck), Some(FocusAct::Pause));
    assert_eq!(plugins.audio_focus(Focus::Lost, FocusAct::Pause), None);

    let nori_look = [0xFFF3_F2F6, 0xFFFD_FDFE, 0xFF26_262A, 0xFFB1_4C66, 0xFF2C_7F8F];
    assert_eq!(plugins.look(Some(0x12_3456), true, nori_look, &|_| true), Some([0xFF0B_0D11, 0xFF16_181C, 0xFFEB_EDEF, 0xFF7F_B0E0, 0xFF9A_9FA6]));
    assert_eq!(plugins.look(Some(0x12_3456), false, nori_look, &|_| true), None);

    let fill = [song("q1", "x"), song("q2", "fixture fill")];
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(nori_tasks::block_on(plugins.autofill(&fill, &ids(&["n1"]), 15)), Some(ids(&["fx-1", "q2", "fx-2"])));
    assert_eq!(nori_tasks::block_on(plugins.autofill(&fill[..1], &ids(&["n1"]), 15)), None);
    assert_eq!(nori_tasks::block_on(plugins.queue_ended(Some(&song("e", "fixture end")))), Some(ids(&["fx-end"])));
    assert_eq!(nori_tasks::block_on(plugins.queue_ended(None)), None);

    plugins.turn(nori_plugins::testing::FIXTURE.into(), false);
    assert!(!plugins.deciding(), "off: nothing is asked");
}

#[test]
fn a_decider_past_its_budget_keeps_noris_answer() {
    let dir = nori_testdir::TempDir::new("plugins-deciders-slow");
    let (plugins, _) = common::plugged(&dir, "");
    let asked = Instant::now();
    assert_eq!(plugins.leveling(&song("s", "fixture slow"), -2.0, &|_| true), None);
    assert!(asked.elapsed() < Duration::from_secs(2), "waited {:?}", asked.elapsed());
}
