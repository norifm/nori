//! The deciders point as nori's tests check it: each answers none (nori's) but for the inputs a test
//! marks, so other tests using the fixture keep nori's answers.

use nori_plugin_sdk::deciders::{ButtonAct, Focus, FocusAct, Palette, Press, Shown};
use nori_plugin_sdk::notices::{show, Tone};
use nori_plugin_sdk::types::{Route, Song};
use nori_plugin_sdk::Deciders;

use crate::Test;

/// The cover color whose look is Winter's night, and the one whose look can't be read.
const NIGHT_COVER: u32 = 0x12_3456;
const FAINT_COVER: u32 = 0x65_4321;

/// A lifecycle action this point's tests run: a media button's own act.
pub(crate) fn acted(id: &str) {
    if id == "media-button" {
        show(Tone::Info, "pressed", &[]);
    }
}

impl Deciders for Test {
    /// "fixture shuffle" anywhere: nori's order with all but its first reversed.
    fn shuffle(&mut self, songs: Vec<Song>, nori: Vec<u32>) -> Option<Vec<u32>> {
        songs.iter().any(|s| s.title == "fixture shuffle").then(|| nori[..1].iter().chain(nori[1..].iter().rev()).copied().collect())
    }

    /// After a "fixture fill" song: fx-1, an already queued song, and fx-2.
    fn autofill(&mut self, queue_tail: Vec<Song>, _nori: Vec<String>, _count: u32) -> Option<Vec<String>> {
        let last = queue_tail.last()?;
        (last.title == "fixture fill").then(|| vec!["fx-1".into(), last.id.clone(), "fx-2".into()])
    }

    fn queue_ended(&mut self, last: Option<Song>) -> Option<Vec<String>> {
        (last?.title == "fixture end").then(|| vec!["fx-end".into()])
    }

    /// "fixture level": 3 dB under nori's; "fixture loud": 40 dB; "fixture slow": never answers.
    fn leveling(&mut self, song: Song, nori_db: f32) -> Option<f32> {
        match song.title.as_str() {
            "fixture level" => Some(nori_db - 3.0),
            "fixture loud" => Some(40.0),
            "fixture slow" => loop {
                std::hint::black_box(());
            },
            _ => None,
        }
    }

    fn sound_profile(&mut self, route: Route, output: String, _nori: String) -> Option<String> {
        (output == "fixture output" && route == Route::Other).then(|| "Fixture".into())
    }

    fn now_playing(&mut self, song: Song, nori: Shown) -> Option<Shown> {
        (song.title == "fixture shown").then(|| Shown { title: "Shown by the fixture".into(), ..nori })
    }

    /// A long press runs its own action; a triple press plays or pauses.
    fn media_button(&mut self, press: Press, _nori: ButtonAct) -> Option<ButtonAct> {
        match press {
            Press::Long => Some(ButtonAct::Plugin),
            Press::Triple => Some(ButtonAct::PlayPause),
            _ => None,
        }
    }

    fn audio_focus(&mut self, focus: Focus, _nori: FocusAct) -> Option<FocusAct> {
        matches!(focus, Focus::Duck).then_some(FocusAct::Pause)
    }

    fn look(&mut self, cover_dominant: Option<u32>, night: bool, _nori: Palette) -> Option<Palette> {
        match cover_dominant? {
            NIGHT_COVER if night => Some(Palette { page: 0x0B_0D11, surface: 0x16_181C, ink: 0xEB_EDEF, accent: 0x7F_B0E0, on: 0x9A_9FA6 }),
            FAINT_COVER => Some(Palette { page: 0x80_8080, surface: 0x80_8080, ink: 0x88_8888, accent: 0x80_8080, on: 0x80_8080 }),
            _ => None,
        }
    }
}
