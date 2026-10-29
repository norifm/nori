//! A plugin as a scrobble target over the test plugin (fixture/src/scrobbler.rs).

mod common;

use std::time::Duration;

use nori_model::Song;
use nori_plugins::testing::FIXTURE;

#[test]
fn a_scrobbler_is_told_what_plays_and_what_counted() {
    let dir = nori_testdir::TempDir::new("plugins-scrobbler");
    let (plugins, side) = common::plugged(&dir, "");
    let song = |id: &str| Song { id: id.into(), title: "t".into(), ..Default::default() };
    plugins.scrobblers_now_playing(&song("fixture-1"));
    plugins.scrobblers_scrobble(&song("fixture-1"), 1_700_000_000_000);
    // Its calls run in order: once one after them is answered, they are in.
    plugins.worker(FIXTURE).unwrap().decide(Duration::from_secs(30), |_, _| Ok(())).unwrap();
    let said = |words: &str, args: &[&str]| (words.to_string(), args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
    assert_eq!(*side.notices.lock().unwrap(), [said("now", &["fixture-1"]), said("play", &["fixture-1", "1700000000000"])]);
    plugins.turn(FIXTURE.into(), false);
    plugins.scrobblers_now_playing(&song("fixture-2"));
    assert!(plugins.worker(FIXTURE).is_none(), "off: no instance is made");
    assert_eq!(side.notices.lock().unwrap().len(), 2);
}
