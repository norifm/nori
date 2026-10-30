//! The `deciders` point: the enabled plugins exporting it, in their order, each handed the inputs and
//! the answer so far (nori's, or the plugin before's) and answering another or none. An answer the
//! caller's check refuses is passed over; a call past its budget keeps the answer so far. Nothing is
//! called, and no instance made, while no enabled plugin exports it.

use std::time::Duration;

use nori_model::deciders::{ButtonAct, Deciders, Focus, FocusAct, Later, Press, Roles, Shown};
use nori_model::Song;
use nori_player::outputs::OutputPort;

use crate::convert::song_out;
use crate::runtime::points::deciders as w;
use crate::runtime::types as t;
use crate::worker::DECIDE_FUEL;
use crate::{Plugins, Worker};

/// A decider on a command's path (a press, a focus change, shuffle turned on).
const SHORT: Duration = Duration::from_millis(50);
/// Any other.
const LONG: Duration = Duration::from_millis(250);

/// The lifecycle action a plugin's own button act runs.
const BUTTON_ACTION: &str = "media-button";

const POINT: &str = "deciders";

impl Plugins {
    fn deciding_workers(&self) -> Vec<(String, Worker)> {
        self.extending(POINT).into_iter().filter_map(|id| Some((id.clone(), self.worker(&id)?))).collect()
    }

    /// `nori` passed through each deciding plugin's `ask` in turn, within `budget`; an answer `fits`
    /// refuses is passed over.
    fn decided<T: Clone + Send + 'static>(&self, nori: T, budget: Duration, fits: impl Fn(&T) -> bool, ask: impl Fn(&mut crate::Store, &crate::runtime::Plugin, T) -> wasmtime::Result<Option<T>> + Clone + Send + 'static) -> Option<T> {
        let mut answer: Option<T> = None;
        for (_, w) in self.deciding_workers() {
            let so_far = answer.clone().unwrap_or_else(|| nori.clone());
            let ask = ask.clone();
            if let Some(v) = w.decide(budget, move |s, p| ask(s, p, so_far)).flatten().filter(|v| fits(v)) {
                answer = Some(v);
            }
        }
        answer
    }
}

fn port_out(p: OutputPort) -> t::Route {
    match p {
        OutputPort::Speaker => t::Route::Speaker,
        OutputPort::Wired => t::Route::Wired,
        OutputPort::Usb => t::Route::Usb,
        OutputPort::Bluetooth => t::Route::Bluetooth,
        OutputPort::Other => t::Route::Other,
    }
}

fn press_out(p: Press) -> w::Press {
    match p {
        Press::PlayPause => w::Press::PlayPause,
        Press::Next => w::Press::Next,
        Press::Previous => w::Press::Previous,
        Press::Double => w::Press::Double,
        Press::Triple => w::Press::Triple,
        Press::Long => w::Press::Long,
    }
}

fn focus_out(f: Focus) -> w::Focus {
    match f {
        Focus::Lost => w::Focus::Lost,
        Focus::LostTransient => w::Focus::LostTransient,
        Focus::Duck => w::Focus::Duck,
        Focus::Gained => w::Focus::Gained,
    }
}

fn focus_act_out(a: FocusAct) -> w::FocusAct {
    match a {
        FocusAct::Pause => w::FocusAct::Pause,
        FocusAct::Duck => w::FocusAct::Duck,
        FocusAct::KeepPlaying => w::FocusAct::KeepPlaying,
        FocusAct::Resume => w::FocusAct::Resume,
        FocusAct::Nothing => w::FocusAct::Nothing,
    }
}

fn focus_act_in(a: w::FocusAct) -> FocusAct {
    match a {
        w::FocusAct::Pause => FocusAct::Pause,
        w::FocusAct::Duck => FocusAct::Duck,
        w::FocusAct::KeepPlaying => FocusAct::KeepPlaying,
        w::FocusAct::Resume => FocusAct::Resume,
        w::FocusAct::Nothing => FocusAct::Nothing,
    }
}

fn shown_out(s: &Shown) -> w::Shown {
    w::Shown { title: s.title.clone(), artist: s.artist.clone(), album: s.album.clone() }
}

/// An ARGB color as the WIT's 0xRRGGBB, and back (opaque).
fn rgb(c: u32) -> u32 {
    c & 0x00FF_FFFF
}

fn argb(c: u32) -> u32 {
    0xFF00_0000 | (c & 0x00FF_FFFF)
}

fn palette_out(r: &Roles) -> w::Palette {
    w::Palette { page: rgb(r[0]), surface: rgb(r[1]), ink: rgb(r[2]), accent: rgb(r[3]), on: rgb(r[4]) }
}

impl Deciders for Plugins {
    fn deciding(&self) -> bool {
        !self.extending(POINT).is_empty()
    }

    fn shuffle(&self, songs: &[Song], nori: &[u32], fits: &dyn Fn(&[u32]) -> bool) -> Option<Vec<u32>> {
        let songs: Vec<t::Song> = songs.iter().map(song_out).collect();
        self.decided(nori.to_vec(), SHORT, |o: &Vec<u32>| fits(o), move |s, p, so_far| p.nori_plugin_deciders().call_shuffle(s, &songs, &so_far))
    }

    fn autofill(&self, tail: &[Song], nori: &[String], count: u32) -> Later<Option<Vec<String>>> {
        let workers = self.deciding_workers();
        let tail: Vec<t::Song> = tail.iter().map(song_out).collect();
        let nori = nori.to_vec();
        Box::pin(async move {
            let mut answer: Option<Vec<String>> = None;
            for (_, worker) in workers {
                let (tail, so_far) = (tail.clone(), answer.clone().unwrap_or_else(|| nori.clone()));
                let got = worker
                    .call(move |s, p| {
                        s.set_fuel(DECIDE_FUEL)?;
                        p.nori_plugin_deciders().call_autofill(s, &tail, &so_far, count)
                    })
                    .await
                    .flatten();
                answer = got.or(answer);
            }
            answer
        })
    }

    fn queue_ended(&self, last: Option<&Song>) -> Later<Option<Vec<String>>> {
        let workers = self.deciding_workers();
        let last = last.map(song_out);
        Box::pin(async move {
            for (_, worker) in workers {
                let last = last.clone();
                let got = worker
                    .call(move |s, p| {
                        s.set_fuel(DECIDE_FUEL)?;
                        p.nori_plugin_deciders().call_queue_ended(s, last.as_ref())
                    })
                    .await
                    .flatten();
                if got.as_ref().is_some_and(|ids| !ids.is_empty()) {
                    return got;
                }
            }
            None
        })
    }

    fn leveling(&self, song: &Song, nori_db: f32, fits: &dyn Fn(f32) -> bool) -> Option<f32> {
        let song = song_out(song);
        self.decided(nori_db, LONG, |db: &f32| fits(*db), move |s, p, so_far| p.nori_plugin_deciders().call_leveling(s, &song, so_far))
    }

    fn sound_profile(&self, port: OutputPort, output: &str, nori: &str, fits: &dyn Fn(&str) -> bool) -> Option<String> {
        let (route, output) = (port_out(port), output.to_string());
        self.decided(nori.to_string(), LONG, |n: &String| fits(n), move |s, p, so_far| p.nori_plugin_deciders().call_sound_profile(s, route, &output, &so_far))
    }

    fn now_playing(&self, song: &Song, nori: &Shown, fits: &dyn Fn(&Shown) -> bool) -> Option<Shown> {
        let song = song_out(song);
        self.decided(nori.clone(), LONG, fits, move |s, p, so_far| {
            let got = p.nori_plugin_deciders().call_now_playing(s, &song, &shown_out(&so_far))?;
            Ok(got.map(|g| Shown { title: g.title, artist: g.artist, album: g.album }))
        })
    }

    fn media_button(&self, press: Press, nori: ButtonAct) -> Option<ButtonAct> {
        let mut answer = None;
        for (id, worker) in self.deciding_workers() {
            let so_far = answer.unwrap_or(nori);
            let asked = worker.decide(SHORT, move |s, p| {
                let n = match so_far {
                    ButtonAct::PlayPause => w::ButtonAct::PlayPause,
                    ButtonAct::Next => w::ButtonAct::Next,
                    ButtonAct::Previous => w::ButtonAct::Previous,
                    ButtonAct::Nothing => w::ButtonAct::Nothing,
                };
                p.nori_plugin_deciders().call_media_button(s, press_out(press), n)
            });
            answer = match asked.flatten() {
                Some(w::ButtonAct::PlayPause) => Some(ButtonAct::PlayPause),
                Some(w::ButtonAct::Next) => Some(ButtonAct::Next),
                Some(w::ButtonAct::Previous) => Some(ButtonAct::Previous),
                Some(w::ButtonAct::Nothing) => Some(ButtonAct::Nothing),
                // Its own action runs; the player does nothing.
                Some(w::ButtonAct::Plugin) => {
                    if self.installed().iter().any(|i| i.manifest.id == id && i.manifest.extends("lifecycle")) {
                        worker.post(|s, p| p.nori_plugin_lifecycle().call_action(s, BUTTON_ACTION));
                    }
                    Some(ButtonAct::Nothing)
                }
                None => answer,
            };
        }
        answer
    }

    fn audio_focus(&self, focus: Focus, nori: FocusAct) -> Option<FocusAct> {
        let answer = self.decided(focus_act_out(nori), SHORT, |_| true, move |s, p, so_far| p.nori_plugin_deciders().call_audio_focus(s, focus_out(focus), so_far));
        answer.map(focus_act_in)
    }

    fn look(&self, cover: Option<u32>, night: bool, nori: Roles, fits: &dyn Fn(&Roles) -> bool) -> Option<Roles> {
        let cover = cover.map(rgb);
        let roles = |p: &w::Palette| [p.page, p.surface, p.ink, p.accent, p.on].map(argb);
        let got = self.decided(palette_out(&nori), LONG, |p: &w::Palette| fits(&roles(p)), move |s, p, so_far| p.nori_plugin_deciders().call_look(s, cover, night, so_far))?;
        Some(roles(&got))
    }
}
