//! A real plugin's (fixture/) sound stage inputs and song slot, its song preparation and AutoMix
//! analysis as the player's tap feeds them, and its refinements and plans.

mod common;

use std::sync::{mpsc, Arc};
use std::time::Duration;

use nori_model::Song;
use nori_player::outputs::OutputKind;
use nori_player::pcm::{Encoding, Format};
use nori_player::stage::{Around, Inputs, Slot, SongFacts};
use nori_player::types::{TrackAnalysis, TransitionKind};
use nori_plugins::testing::FIXTURE;
use nori_plugins::Value;

use common::plugged;

const FORMAT: Format = Format { rate: 44_100, channels: 2, encoding: Encoding::Pcm16 };

#[test]
fn a_stage_reads_the_volume_the_route_and_its_songs_facts() {
    let dir = nori_testdir::TempDir::new("plugins-inputs");
    let (plugins, _) = plugged(&dir, "");
    // Below 0 its gain row has it write what it reads over the block.
    plugins.set(FIXTURE.into(), "gain".into(), Value::Level(-1.0));
    let spec = plugins.stages().remove(0);
    assert_eq!((spec.slot, spec.rows.clone()), (Slot::Master, vec![-1.0]));
    let song = Arc::new(SongFacts { gain_db: Some(-3.0), loudness_lufs: Some(-14.0), prepared: vec![(FIXTURE.into(), Arc::from(&1234u64.to_le_bytes()[..])), ("other.plugin".into(), Arc::from(&[9u8; 8][..]))] });
    let inputs = Inputs { rows: spec.rows.clone(), around: Around { volume_db: -6.0, route: OutputKind::Bluetooth }, song };
    let mut stage = spec.maker.make(48_000, 2, &inputs).unwrap();
    let mut b = vec![0.0f32; 16];
    stage.process(&mut b);
    // The route's index in the WIT's enum: speaker, wired, bluetooth.
    assert_eq!(&b[..5], &[-6.0, 2.0, -3.0, -14.0, 1234.0], "volume, route, gain, loudness, its own prepared data");
    stage.inputs(&Inputs { song: nori_player::stage::no_facts(), ..inputs });
    stage.process(&mut b);
    assert!(b[2].is_nan() && b[3].is_nan() && b[4] == -1.0, "a song nothing is known of: {:?}", &b[..5]);
}

#[test]
fn a_song_slot_stage_installs_in_its_slot() {
    let dir = nori_testdir::TempDir::new("plugins-song-slot");
    let (plugins, _) = plugged(&dir, "song");
    assert_eq!(plugins.stages().iter().map(|s| s.slot).collect::<Vec<_>>(), vec![Slot::Song]);
}

/// The song `id` titled `title`, as the profile knows it.
fn songs(titles: &'static [(&'static str, &'static str)]) -> nori_plugins::Songs {
    Arc::new(move |id: &str| titles.iter().find(|(i, _)| *i == id).map(|(i, t)| Song { id: i.to_string(), title: t.to_string(), duration: 60, ..Default::default() }))
}

#[test]
fn a_plugin_prepares_and_analyses_the_songs_it_hears() {
    let dir = nori_testdir::TempDir::new("plugins-prepare");
    let (plugins, _) = plugged(&dir, "");
    plugins.set_songs(songs(&[("s1", "fixture one"), ("s2", "other"), ("s3", "fixture three")]));
    let preparing = plugins.preparing().expect("it hears songs");
    let (told, came) = mpsc::channel();
    let told = std::sync::Mutex::new(told);
    preparing.on_prepared(Arc::new(move || told.lock().unwrap().send(()).unwrap()));
    let samples = vec![0.25f32; 2 * 1000];

    let mut tap = preparing.tap("s1", FORMAT, 0).unwrap();
    for _ in 0..3 {
        tap.samples(&samples);
    }
    tap.end();
    drop(tap);
    came.recv_timeout(Duration::from_secs(20)).expect("its data came");
    let data = preparing.prepared("s1");
    assert_eq!(data.len(), 1);
    assert_eq!((data[0].0.as_str(), &data[0].1[..]), (FIXTURE, &[3000u64.to_le_bytes(), 0u64.to_le_bytes()].concat()[..]), "every frame fed, from the first");

    // Its AutoMix data for the song refines nori's analysis; a song it has none of stays nori's.
    let mixing = plugins.mixing();
    let nori = TrackAnalysis { song_id: "s1".into(), duration_ms: 60_000, bpm: 100.0, lufs: -12.0, ..Default::default() };
    let r = mixing.refine("s1", &nori).expect("refined");
    assert_eq!((r.bpm, r.key), (123.0, 7));
    assert_eq!(mixing.refine("s3", &TrackAnalysis { song_id: "s3".into(), ..nori.clone() }), None, "s3 was not analysed");

    // A song it doesn't want: nothing is made, and the tap stops copying.
    let mut tap = preparing.tap("s2", FORMAT, 0).unwrap();
    tap.samples(&samples);
    tap.end();
    drop(tap);
    assert!(came.recv_timeout(Duration::from_secs(2)).is_err());
    assert!(preparing.prepared("s2").is_empty());

    // Read from part-way, it is told where the samples start, and not analysed.
    let mut tap = preparing.tap("s3", FORMAT, 4410).unwrap();
    tap.samples(&samples);
    tap.end();
    drop(tap);
    came.recv_timeout(Duration::from_secs(20)).expect("its data came");
    assert_eq!(&preparing.prepared("s3")[0].1[..], &[1000u64.to_le_bytes(), 4410u64.to_le_bytes()].concat()[..]);
    assert_eq!(mixing.refine("s3", &TrackAnalysis { song_id: "s3".into(), ..nori }), None, "a song heard in part is not analysed");
}

#[test]
fn a_plugin_plans_the_transitions_out_of_its_songs() {
    let dir = nori_testdir::TempDir::new("plugins-plan");
    let (plugins, _) = plugged(&dir, "");
    plugins.set_songs(songs(&[("s1", "fixture one"), ("s2", "other")]));
    let mixing = plugins.mixing();
    let mut nori = nori_player::automix::plan::plan(None, None, 60_000, 60_000, &Default::default());
    (nori.kind, nori.out_start_ms, nori.duration_ms) = (TransitionKind::EqualPowerFade, 50_000, 6_000);
    let p = mixing.plan("s1", "s2", None, None, &nori).expect("its plan");
    assert_eq!((p.out_start_ms, p.duration_ms, p.in_start_ms), (47_000, 2_000, 0));
    assert_eq!(mixing.plan("s2", "s1", None, None, &nori), None, "out of another song, nori's");
}
