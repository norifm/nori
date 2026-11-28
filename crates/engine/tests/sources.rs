//! A profile of two sources whose songs share an id: each queued entry streams from its own source, as
//! that source's account, and plays its own bytes. Runs over the test's own core and a second account of
//! its server.

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use common::card::{Card, Pull};
use common::{NoApi, Stepper, Virtual};
use crate::common;
use nori_core::settings::SavedServer;
use nori_core::{ItemKey, Keyed, Song};
use nori_engine::core::{settings, Analyses, CoreApp, CoreLibrary, CoreQueue};
use nori_engine::{Body, ByteSource, Config, Engine, State};
use parking_lot::Mutex;

const RATE: u32 = 44_100;
const SECS: usize = 3;

/// A 16-bit stereo song of `hz`.
fn tone(hz: f64) -> Vec<u8> {
    let samples: Vec<i16> = (0..RATE as usize * SECS).flat_map(|i| {
        let v = ((i as f64 * hz * std::f64::consts::TAU / RATE as f64).sin() * 12_000.0) as i16;
        [v, v]
    }).collect();
    common::wav(RATE, &samples)
}

/// The server: song `1` as each account (`u=`) has it, and every stream address asked for.
struct Server {
    songs: Vec<(&'static str, Arc<Vec<u8>>)>,
    asked: Mutex<Vec<String>>,
}

impl ByteSource for Server {
    fn open(&self, url: &str, from: u64) -> Result<Body, nori_engine::OpenError> {
        self.asked.lock().push(url.to_string());
        let file = self.songs.iter().find(|(user, _)| url.contains(&format!("u={user}&"))).map(|(_, f)| f.clone()).ok_or("no such account")?;
        let mut c = Cursor::new(file.as_ref().clone());
        c.set_position(from);
        Ok(Body { start: from, len: Some(file.len() as u64), reader: Box::new(c) })
    }
}

/// The frequency of `seconds` of the left channel of `heard` from `from` (s), from its zero crossings.
fn hz(heard: &[f32], from: f64, seconds: f64) -> f64 {
    let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
    let w = &left[(from * RATE as f64) as usize..((from + seconds) * RATE as f64) as usize];
    w.windows(2).filter(|p| (p[0] < 0.0) != (p[1] < 0.0)).count() as f64 / 2.0 / seconds
}

/// Both entries of song `1`, the profile's own and the second account's, are queued and played: each
/// asked of its own account, each heard as its own source has it, the second after the first. Different
/// songs under one id (two servers numbering their songs) and the same song on two accounts.
#[test]
fn one_id_in_two_sources_plays_twice_each_from_its_source() {
    // (what, the own account's song, the second account's)
    let cases = [("different songs under one id", 330.0, 880.0), ("the same song on two accounts", 440.0, 440.0)];
    for (what, own_hz, theirs_hz) in cases {
        let dir = nori_testdir::TempDir::new("sources-engine");
        let (core, client) = common::own_core(&dir, |p| (p.auto_mix, p.crossfade_sec, p.replay_gain) = (nori_core::settings::AutoMixMode::Off, 0, nori_core::settings::GainMode::Off));
        let second = SavedServer { id: "acct2".into(), url: "http://music.test".into(), user: "v".into(), password: "q".into(), ..Default::default() };
        core.open_source(dir.join("nori.db").to_string_lossy().into_owned(), second, Arc::new(NoApi::default()), nori_core::covers::CoverNet::new()).unwrap();
        let own = Song { id: "1".into(), title: "Mine".into(), duration: SECS as u32, suffix: "wav".into(), ..Default::default() };
        let theirs = Song { title: "Theirs".into(), source: Some("acct2".into()), ..own.clone() };
        core.session.register(vec![own.clone(), theirs.clone()]);
        core.session.set(vec![own.key(), theirs.key()], Some(0), false, None);

        let server = Arc::new(Server { songs: vec![("u", Arc::new(tone(own_hz))), ("v", Arc::new(tone(theirs_hz)))], asked: Mutex::default() });
        let analyses = Analyses::of(client.clone());
        let library = CoreLibrary { client, bytes: server.clone(), store: None, analyses: analyses.clone() };
        let prefs = core.session.settings.current().unwrap();
        let card = Card::new();
        let clock = Virtual::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let said = events.clone();
        let engine = common::Running(Engine::start_on(library, CoreApp::new(core.session.clone(), analyses), CoreQueue(core.session.clone()), Box::new(card.clone()), None, Config { memory_mb: 64, settings: settings(&prefs, Default::default()), ..Config::default() }, clock.clone(), move |e| said.lock().push(e)));
        let time: Stepper<Pull> = Stepper::new(clock, card.pull.clone());
        engine.queue_changed();
        engine.play_at(0, 0);
        assert!(time.until(Duration::from_secs(20), || engine.status().state == State::Ended), "{what}: {:?}", engine.status());

        let songs: Vec<ItemKey> = events.lock().iter().filter_map(|e| match e { nori_engine::Event::Song { id, .. } => Some(id.clone()), _ => None }).collect();
        assert_eq!(songs, [own.key(), theirs.key()], "{what}: each entry is said as its own");
        let asked = server.asked.lock().clone();
        assert!(asked.len() == 2 && asked[0].contains("u=u&") && asked[1].contains("u=v&"), "{what}: each asked of its own account: {asked:?}");
        let heard = card.heard.lock().clone();
        let (first, second) = (hz(&heard, 0.5, 2.0), hz(&heard, SECS as f64 + 0.5, 2.0));
        assert!((first - own_hz).abs() < 5.0 && (second - theirs_hz).abs() < 5.0, "{what}: heard {first} Hz then {second} Hz");
    }
}
