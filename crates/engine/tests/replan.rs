//! Transition settings changed while playing apply at the next boundary, and a play right after a
//! change uses it: the engine over a core's queue, settings and planner of the test's own.
use crate::common;

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use common::card::{Card, Pull};
use common::{Stepper, Virtual};
use nori_core::settings_store::{APPLY_AUDIO, REPLAN};
use nori_core::{Core, Song};
use nori_engine::core::{settings, Analyses, CoreApp, CoreLibrary, CoreQueue};
use nori_engine::{Body, ByteSource, Config, Engine, Store};

/// `secs` of a quiet tone as a WAV file, a different pitch per seed.
fn tone_wav(secs: usize, seed: u32) -> Vec<u8> {
    common::wav(44_100, &common::sine(44_100, 220.0 + seed as f64 * 30.0, secs as f64, 0.2 * 32767.0))
}

struct Net(HashMap<String, Arc<Vec<u8>>>);

impl ByteSource for Net {
    fn open(&self, url: &str, from: u64) -> Result<Body, nori_engine::OpenError> {
        let id = url.split("&id=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
        let file = self.0.get(id).cloned().ok_or("no such song")?;
        let len = file.len() as u64;
        let mut c = Cursor::new(file.as_ref().clone());
        c.set_position(from);
        Ok(Body { start: from, len: Some(len), reader: Box::new(c) })
    }
}

const SECS: usize = 40;

struct Rig {
    engine: common::Running,
    core: Arc<Core>,
    time: Stepper<Pull>,
    _dir: nori_testdir::TempDir,
}

impl Rig {
    /// `ids` queued in order, AutoMix on, all from one album if `one_album`, else each from its own;
    /// not playing.
    fn new(name: &str, ids: &[&str], one_album: bool) -> Rig {
        let dir = nori_testdir::TempDir::new(name);
        let (core, client) = common::own_core(&dir, |p| p.auto_mix = true);
        let prefs = core.session.settings.current().unwrap();
        let store = Store::open(dir.join("music"), 256 << 20).unwrap();
        let songs: Vec<Song> = ids
            .iter()
            .enumerate()
            .map(|(k, id)| Song {
                id: id.to_string(),
                title: id.to_string(),
                album_id: Some(if one_album { "al".into() } else { format!("al-{id}") }),
                track: k as u32 + 1,
                disc_number: 1,
                duration: SECS as u32,
                suffix: "wav".into(),
                ..Default::default()
            })
            .collect();
        let net = Arc::new(Net(ids.iter().enumerate().map(|(k, id)| (id.to_string(), Arc::new(tone_wav(SECS, k as u32)))).collect()));
        core.session.register(songs);
        // Played from the album's page.
        core.session.set(ids.iter().map(|s| s.to_string()).collect(), Some(0), false, Some(nori_core::PageOrigin::new(nori_core::OriginKind::Album, "al")));
        let analyses = Analyses::of(client.clone());
        let library = CoreLibrary { client, bytes: net, store: Some(store), analyses };
        let card = Card::new();
        let clock = Virtual::default();
        let engine = common::Running(Engine::start_on(library, CoreApp::new(core.session.clone()), CoreQueue(core.session.clone()), Box::new(card.clone()), None, Config { memory_mb: 128, settings: settings(&prefs, Default::default()), ..Config::default() }, clock.clone(), |_| {}));
        engine.queue_changed();
        Rig { engine, core, time: Stepper::new(clock, card.pull.clone()), _dir: dir }
    }

    fn until(&self, secs: u64, mut done: impl FnMut(&Rig) -> bool) -> bool {
        self.time.until(Duration::from_secs(secs), || done(self))
    }

    /// Sets a setting by name and relays its effects to the engine, as the clients do.
    fn set(&self, name: &str, value: &str) {
        let effect = self.set_only(name, value);
        self.relay(effect);
    }

    /// Sets a setting by name without telling the engine; returns its effects.
    fn set_only(&self, name: &str, value: &str) -> u32 {
        let effect = self.core.session.settings.edit_by_name(name, value).unwrap_or_else(|| panic!("{name} is a setting")).effect;
        assert_ne!(effect & REPLAN, 0, "{name} asks for the transition to be planned again");
        effect
    }

    /// Relays a change's effects to the engine, as PlaybackService does.
    fn relay(&self, effect: u32) {
        if effect & APPLY_AUDIO != 0 {
            self.engine.set_settings(settings(&self.core.session.settings.current().unwrap(), Default::default()));
        }
        if effect & REPLAN != 0 {
            self.engine.replan();
        }
    }

    /// Whether a mix is heard before playback is 10 s into `index`.
    fn mixes_into(&self, index: usize) -> bool {
        let mut mixed = false;
        let came = self.until(SECS as u64 * 2, |r| {
            let s = r.engine.status();
            mixed |= s.mixing;
            s.index == Some(index) && s.position_ms > 10_000
        });
        assert!(came, "song {index} is heard: {:?}", self.engine.status());
        mixed
    }
}

#[test]
fn keep_albums_off() {
    let rig = Rig::new("replan-keep", &["a1", "a2", "a3"], true);
    rig.engine.play_at(0, 0);
    assert!(!rig.mixes_into(1), "kept gapless while the setting says so");
    // Early in the second song, before its ending is made.
    rig.set("crossfadeKeepAlbums", "false");
    assert!(rig.mixes_into(2), "mixed into the third song once the album is no longer kept gapless");
    let note = rig.core.session.planner.transition_note("a2").expect("a2's ending was planned");
    assert_ne!(note.kind, nori_player::types::TransitionKind::Gapless, "{note:?}");

    // Keep albums off remakes gapless ending.
    let rig = Rig::new("replan-late", &["d1", "d2"], true);
    rig.engine.play_at(0, 0);
    assert!(rig.until(20, |r| r.engine.status().index == Some(0) && r.engine.status().position_ms > 1_000));
    rig.engine.seek(SECS as i64 * 1000 - 15_000);
    // The ending is already made gapless.
    // Looked at: the status is otherwise as old as the last burst.
    assert!(rig.until(20, |r| {
        r.engine.look();
        r.engine.status().position_ms > SECS as i64 * 1000 - 13_000
    }), "{:?}", rig.engine.status());
    rig.set("crossfadeKeepAlbums", "false");
    assert!(rig.mixes_into(1), "the ending is made again as a mix");
}

#[test]
fn each_setting_change_is_planned_with() {
    let rig = Rig::new("replan-each", &["b1", "b2", "b3", "b4", "b5"], false);
    rig.set("autoMix", "false");
    rig.set("crossfadeSec", "6");
    rig.engine.play_at(0, 0);
    assert!(rig.until(20, |r| r.engine.status().index == Some(0) && r.engine.status().position_ms > 2_000));
    // Each early in a song: the plan out of it uses the new value.
    type Case = (&'static str, &'static str, fn(&nori_core::automix::planner::TransitionNote) -> bool);
    let cases: [Case; 3] = [
        ("crossfadeSec", "3", |n| n.duration_ms > 0 && n.duration_ms <= 3_000),
        ("crossfadeSec", "5", |n| n.duration_ms > 3_000 && n.duration_ms <= 5_000),
        ("autoMix", "true", |n| n.kind != nori_player::types::TransitionKind::Gapless),
    ];
    for (k, (name, value, ok)) in cases.into_iter().enumerate() {
        rig.set(name, value);
        let id = format!("b{}", k + 1);
        let _ = rig.mixes_into(k + 1);
        let note = rig.core.session.planner.transition_note(&id).unwrap_or_else(|| panic!("{id}'s ending was planned"));
        assert!(ok(&note), "{name}={value}: {note:?}");
    }
}

#[test]
fn play_after_change_uses_it() {
    let rig = Rig::new("replan-play", &["c1", "c2"], false);
    rig.set("autoMix", "false");
    rig.set("crossfadeSec", "4");
    rig.engine.play_at(0, 0);
    assert!(rig.mixes_into(1), "the play right after the change mixes");
}

/// smoke.sh's AutoMix check: AutoMix switched on right before songs are played; the engine hears of
/// the setting only after the play.
#[test]
fn automix_on_before_play_mixes() {
    let rig = Rig::new("replan-smoke", &["e1", "e2", "e3"], false);
    let mut prefs = rig.core.session.settings.current().unwrap();
    prefs.auto_mix = false;
    rig.core.session.settings.put(prefs.clone());
    rig.engine.set_settings(settings(&prefs, Default::default()));
    rig.engine.play_at(2, 0);
    assert!(rig.until(20, |r| r.engine.status().index == Some(2) && r.engine.status().position_ms > 2_000));
    let effect = rig.set_only("autoMix", "true");
    rig.engine.play_at(0, 0);
    rig.relay(effect);
    assert!(rig.until(20, |r| r.engine.status().index == Some(0) && r.engine.status().position_ms > 1_000));
    rig.engine.seek(SECS as i64 * 1000 - 15_000);
    assert!(rig.mixes_into(1), "the first song mixes into the second");
}

/// "Measure again" once the ending of the song playing is made: the clients relay what the core says it
/// asks of the player, and the ending is planned again.
#[test]
fn measure_again_plans_the_ending_again() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let rig = Rig::new("replan-measure", &["f1", "f2"], false);
    rig.engine.play_at(0, 0);
    assert!(rig.until(20, |r| r.engine.status().index == Some(0) && r.engine.status().position_ms > 1_000));
    rig.engine.seek(SECS as i64 * 1000 - 20_000);
    assert!(rig.until(20, |r| r.core.session.planner.made("f1").is_some()), "f1's ending is made");
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    rig.core.session.planner.on_plan(Some(Box::new(move || _ = counted.fetch_add(1, Ordering::Relaxed))));
    let m = rig.core.measure_again().unwrap();
    rig.relay(m.effect);
    rig.until(5, |r| r.engine.status().index != Some(0) || asked.load(Ordering::Relaxed) > 0);
    assert!(asked.load(Ordering::Relaxed) > 0, "f1's ending planned again before it plays: {:?}", rig.engine.status());
}

/// A queue handed over in the middle of a mix: the song mixed out of starts where its ending was, and the
/// rest of the transition plays by the plan it came with, although nothing here would mix.
#[test]
fn a_handed_mix_plays_on() {
    let rig = Rig::new("replan-handed", &["g1", "g2"], false);
    rig.set("autoMix", "false");
    let t = nori_player::automix::plan::plan(None, None, SECS as i64 * 1000, SECS as i64 * 1000, &Default::default());
    assert!(t.duration_ms >= 2_000, "a crossfade to hand over: {t:?}");
    rig.core.session.planner.handed("g1", "g2", &t);
    rig.engine.play_at(0, t.out_start_ms + t.duration_ms / 2);
    let mut from_the_ending = false;
    assert!(rig.until(10, |r| {
        let s = r.engine.status();
        from_the_ending |= s.mixing && s.mix_from == Some(0);
        s.index == Some(1)
    }), "the second song comes: {:?}", rig.engine.status());
    assert!(from_the_ending, "the first song's ending mixed into it");
}
