//! A plugin's lyrics service over the test plugin (fixture/src/lyrics.rs).

mod common;

use nori_lyrics::services::Lookup;
use nori_model::Song;
use nori_plugins::testing::FIXTURE;

#[test]
fn a_plugins_lyrics_service_answers_for_the_race() {
    let dir = nori_testdir::TempDir::new("plugins-lyrics");
    let (plugins, _) = common::plugged(&dir, "");
    let services = plugins.lyrics_services();
    assert_eq!(services.iter().map(|s| s.plugin()).collect::<Vec<_>>(), [FIXTURE]);
    let ask = |title: &str| nori_tasks::block_on(services[0].lookup(&Song { id: "s".into(), title: title.into(), ..Default::default() }));
    let Lookup::Found(timed, _) = ask("fixture lyrics") else { panic!("found") };
    assert!(timed.synced);
    assert_eq!(timed.lines.iter().map(|l| (l.start_ms, l.end_ms, l.text.as_str())).collect::<Vec<_>>(), [(1_000, 5_000, "first fixture line"), (5_000, 10_000, "second fixture line")]);
    let Lookup::Found(untimed, _) = ask("fixture untimed") else { panic!("found") };
    assert!(!untimed.synced && untimed.lines.iter().all(|l| l.start_ms == -1));
    assert!(matches!(ask("another"), Lookup::Missing));
    plugins.turn(FIXTURE.into(), false);
    assert!(plugins.lyrics_services().is_empty(), "off, none");
}
