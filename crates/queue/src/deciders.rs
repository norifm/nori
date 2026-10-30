//! What the queue decides that plugins may decide instead ([`Deciders`]): the shuffle order, a song's
//! leveling gain, what the media controls show, what a media button and a focus change do, the
//! palette worn. nori's own
//! answer is worked out first; a plugin's is checked here before it is used. While none decides,
//! nothing is built for them.

use std::sync::Arc;

use nori_model::deciders::{ButtonAct, Deciders, Focus, FocusAct, Press, Shown};
use nori_model::Song;
use nori_look::theme::{readable, Palette};
use nori_player::gain::BOOST_MAX_DB;

use crate::Session;

/// The quietest leveling gain taken from a plugin, dB.
const QUIETEST_DB: f32 = -60.0;
/// Songs whose leveling and media-control lines a plugin decided, kept so a song is asked about once.
const REMEMBERED: usize = 16;

/// What deciders said lately, by what they were asked.
#[derive(Default)]
pub(crate) struct Decided {
    /// Song, nori's gain in dB, the gain decided (None: nori's).
    leveled: Vec<(String, f32, Option<f32>)>,
    /// Song, nori's lines, the lines decided (None: nori's).
    shown: Vec<(String, Shown, Option<Shown>)>,
    /// nori's palette, the cover's color, night, the palette decided (None: nori's).
    looks: Vec<(Palette, Option<u32>, bool, Option<Palette>)>,
}

fn roles(p: &Palette) -> nori_model::deciders::Roles {
    [p.page, p.surface, p.ink, p.accent, p.on]
}

fn palette([page, surface, ink, accent, on]: nori_model::deciders::Roles) -> Palette {
    Palette { page, surface, ink, accent, on }
}

fn remember<T>(v: &mut Vec<T>, item: T) {
    if v.len() >= REMEMBERED {
        v.remove(0);
    }
    v.push(item);
}

/// What a press does on its own.
pub fn button_act(press: Press) -> ButtonAct {
    match press {
        Press::PlayPause => ButtonAct::PlayPause,
        Press::Next | Press::Double => ButtonAct::Next,
        Press::Previous | Press::Triple => ButtonAct::Previous,
        Press::Long => ButtonAct::Nothing,
    }
}

/// What a focus change does on its own: a loss pauses what wants to play (for good, or until the focus
/// is back), a duck plays on under the other sound, and a gain resumes what a loss `held`.
pub fn focus_act(focus: Focus, wants_play: bool, held: bool) -> FocusAct {
    match focus {
        Focus::Lost | Focus::LostTransient if wants_play => FocusAct::Pause,
        Focus::Lost | Focus::LostTransient => FocusAct::Nothing,
        Focus::Duck => FocusAct::Duck,
        Focus::Gained if held => FocusAct::Resume,
        Focus::Gained => FocusAct::Nothing,
    }
}

/// Whether `order` is a permutation of `0..n` that starts where `nori`'s does (the song playing).
fn fits_shuffle(order: &[u32], nori: &[u32]) -> bool {
    let mut seen = vec![false; nori.len()];
    order.len() == nori.len() && order.first() == nori.first() && order.iter().all(|&i| (i as usize) < seen.len() && !std::mem::replace(&mut seen[i as usize], true))
}

impl Session {
    /// Where plugins decide from now on; None for nori alone.
    pub fn use_deciders(&self, deciders: Option<Arc<dyn Deciders>>) {
        *self.deciders.lock() = deciders;
    }

    /// The deciders while any would be asked.
    pub fn deciders(&self) -> Option<Arc<dyn Deciders>> {
        self.deciders.lock().clone().filter(|d| d.deciding())
    }

    /// Shuffling has started: a plugin may order it instead, from the same first song.
    pub(crate) fn decide_shuffle(&self) {
        let Some(d) = self.deciders() else { return };
        let Some((ids, nori, rev)) = self.playlist(|p| p.shuffle_order().map(|o| (p.ids().to_vec(), o.iter().map(|&i| i as u32).collect::<Vec<_>>(), p.rev()))) else { return };
        let songs: Vec<Song> = ids.iter().map(|id| self.song(id).unwrap_or_else(|| Song::only_id(id.clone()))).collect();
        let Some(order) = d.shuffle(&songs, &nori, &|o| fits_shuffle(o, &nori)) else { return };
        let mut q = self.queue.lock();
        // Changed meanwhile: nori's order stands.
        if q.list.rev() == rev {
            q.list.reorder(order.into_iter().map(|i| i as usize).collect());
        }
    }

    /// `nori` (a linear gain) for song `id`, or a plugin's gain within nori's range.
    pub(crate) fn decide_leveling(&self, id: &str, nori: f32) -> f32 {
        let Some(d) = self.deciders() else { return nori };
        let nori_db = 20.0 * nori.max(1e-6).log10();
        let gain = |db: Option<f32>| db.map_or(nori, |db| 10f32.powf(db / 20.0));
        if let Some(&(_, _, db)) = self.decided.lock().leveled.iter().find(|(s, n, _)| s == id && *n == nori_db) {
            return gain(db);
        }
        let Some(song) = self.song(id) else { return nori };
        let db = d.leveling(&song, nori_db, &|db| db.is_finite() && (QUIETEST_DB..=BOOST_MAX_DB).contains(&db));
        remember(&mut self.decided.lock().leveled, (id.to_string(), nori_db, db));
        gain(db)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Session {
    /// What a media button's `press` does.
    pub fn media_button(&self, press: Press) -> ButtonAct {
        let nori = button_act(press);
        self.deciders().and_then(|d| d.media_button(press, nori)).unwrap_or(nori)
    }

    /// What a change of the audio focus does to a player that `wants_play`, with playing `held` by an
    /// earlier loss.
    pub fn audio_focus(&self, focus: Focus, wants_play: bool, held: bool) -> FocusAct {
        let nori = focus_act(focus, wants_play, held);
        self.deciders().and_then(|d| d.audio_focus(focus, nori)).unwrap_or(nori)
    }

    /// What the notification, lock screen and media controls show for queued song `id` in place of its
    /// own lines: a plugin's, with a title; None for its own.
    pub fn now_playing(&self, id: String) -> Option<Shown> {
        let d = self.deciders()?;
        let song = self.song(&id)?;
        let nori = Shown::of(&song);
        if let Some((_, _, decided)) = self.decided.lock().shown.iter().find(|(i, n, _)| *i == id && *n == nori) {
            return decided.clone();
        }
        let decided = d.now_playing(&song, &nori, &|s| !s.title.trim().is_empty()).filter(|s| *s != nori);
        remember(&mut self.decided.lock().shown, (id, nori, decided.clone()));
        decided
    }

    /// Whether a plugin decides anything now: a client asks before taking over what its platform does on
    /// its own (a media button's press).
    pub fn deciding(&self) -> bool {
        self.deciders().is_some()
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Session {
    /// The palette worn where nori would wear `nori` (a season's, at `night` or not, over a cover of
    /// that dominant color): a plugin's season of its own when it reads ([`readable`]).
    pub fn look(&self, nori: Palette, cover: Option<u32>, night: bool) -> Palette {
        let Some(d) = self.deciders() else { return nori };
        if let Some((.., decided)) = self.decided.lock().looks.iter().find(|(n, c, k, _)| *n == nori && *c == cover && *k == night) {
            return decided.unwrap_or(nori);
        }
        let decided = d.look(cover, night, roles(&nori), &|r| readable(&palette(*r))).map(palette);
        remember(&mut self.decided.lock().looks, (nori, cover, night, decided));
        decided.unwrap_or(nori)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_model::deciders::{Later, Roles};
    use nori_player::outputs::OutputPort;
    use parking_lot::Mutex;

    /// Answers as it is told, counting what it is asked.
    #[derive(Default)]
    pub struct Fake {
        pub on: bool,
        pub order: Option<Vec<u32>>,
        pub db: Option<f32>,
        pub shown: Option<Shown>,
        pub act: Option<ButtonAct>,
        pub focus: Option<FocusAct>,
        pub look: Option<Roles>,
        pub asked: Mutex<u32>,
    }

    impl Fake {
        fn ask<T: Clone>(&self, v: &Option<T>) -> Option<T> {
            *self.asked.lock() += 1;
            v.clone()
        }
    }

    impl Deciders for Fake {
        fn deciding(&self) -> bool {
            self.on
        }
        fn shuffle(&self, _: &[Song], _: &[u32], fits: &dyn Fn(&[u32]) -> bool) -> Option<Vec<u32>> {
            self.ask(&self.order).filter(|o| fits(o))
        }
        fn autofill(&self, _: &[Song], _: &[String], _: u32) -> Later<Option<Vec<String>>> {
            Box::pin(async { None })
        }
        fn queue_ended(&self, _: Option<&Song>) -> Later<Option<Vec<String>>> {
            Box::pin(async { None })
        }
        fn leveling(&self, _: &Song, _: f32, fits: &dyn Fn(f32) -> bool) -> Option<f32> {
            self.ask(&self.db).filter(|d| fits(*d))
        }
        fn sound_profile(&self, _: OutputPort, _: &str, _: &str, _: &dyn Fn(&str) -> bool) -> Option<String> {
            None
        }
        fn now_playing(&self, _: &Song, _: &Shown, fits: &dyn Fn(&Shown) -> bool) -> Option<Shown> {
            self.ask(&self.shown).filter(|s| fits(s))
        }
        fn media_button(&self, _: Press, _: ButtonAct) -> Option<ButtonAct> {
            self.ask(&self.act)
        }
        fn audio_focus(&self, _: Focus, _: FocusAct) -> Option<FocusAct> {
            self.ask(&self.focus)
        }
        fn look(&self, _: Option<u32>, _: bool, _: Roles, fits: &dyn Fn(&Roles) -> bool) -> Option<Roles> {
            self.ask(&self.look).filter(|r| fits(r))
        }
    }

    fn deciding(s: &Session, fake: Fake) -> Arc<Fake> {
        let f = Arc::new(fake);
        s.use_deciders(Some(f.clone()));
        f
    }

    fn queued(n: usize, at: u32) -> Session {
        let s = Session::default();
        let ids: Vec<String> = (0..n).map(|i| format!("q{i}")).collect();
        s.register(ids.iter().map(|id| Song { id: id.clone(), title: id.clone(), ..Default::default() }).collect());
        s.set(ids, Some(at), false, None);
        s
    }

    #[test]
    fn nori_alone_answers_by_its_own_rules() {
        assert_eq!([Press::PlayPause, Press::Double, Press::Triple, Press::Long].map(button_act), [ButtonAct::PlayPause, ButtonAct::Next, ButtonAct::Previous, ButtonAct::Nothing]);
        assert_eq!(focus_act(Focus::Lost, true, false), FocusAct::Pause);
        assert_eq!(focus_act(Focus::LostTransient, false, false), FocusAct::Nothing);
        assert_eq!(focus_act(Focus::Duck, true, false), FocusAct::Duck);
        assert_eq!(focus_act(Focus::Gained, true, true), FocusAct::Resume);
        assert_eq!(focus_act(Focus::Gained, true, false), FocusAct::Nothing);
    }

    #[test]
    fn a_plugin_that_does_not_decide_is_not_asked() {
        let s = queued(4, 1);
        let f = deciding(&s, Fake { on: false, act: Some(ButtonAct::Nothing), order: Some(vec![1, 0, 2, 3]), ..Default::default() });
        assert_eq!(s.media_button(Press::Next), ButtonAct::Next);
        s.shuffle(true);
        assert_eq!(*f.asked.lock(), 0);
    }

    #[test]
    fn a_shuffle_decided_is_kept_only_when_it_fits() {
        // From the song playing (index 1), each index once.
        let s = queued(4, 1);
        deciding(&s, Fake { on: true, order: Some(vec![1, 3, 2, 0]), ..Default::default() });
        s.shuffle(true);
        assert_eq!(s.playlist(|p| p.shuffle_order().unwrap().to_vec()), [1, 3, 2, 0]);
        for wrong in [vec![0, 1, 2, 3], vec![1, 3, 3, 0], vec![1, 2, 3]] {
            let s = queued(4, 1);
            deciding(&s, Fake { on: true, order: Some(wrong.clone()), ..Default::default() });
            s.shuffle(true);
            let o = s.playlist(|p| p.shuffle_order().unwrap().iter().map(|&i| i as u32).collect::<Vec<_>>());
            assert_ne!(o, wrong, "nori's order stands");
            assert_eq!(o[0], 1);
        }
    }

    #[test]
    fn leveling_is_decided_within_nori_range_and_once_per_song() {
        let s = queued(2, 0);
        let f = deciding(&s, Fake { on: true, db: Some(-3.0), ..Default::default() });
        let g = s.decide_leveling("q0", 0.5);
        assert!((20.0 * g.log10() + 3.0).abs() < 1e-4, "{g}");
        s.decide_leveling("q0", 0.5);
        assert_eq!(*f.asked.lock(), 1, "remembered");
        let s = queued(2, 0);
        deciding(&s, Fake { on: true, db: Some(BOOST_MAX_DB + 6.0), ..Default::default() });
        assert_eq!(s.decide_leveling("q0", 0.5), 0.5, "past nori's range: nori's");
    }

    #[test]
    fn controls_take_a_plugins_answer() {
        let s = queued(1, 0);
        deciding(&s, Fake { on: true, act: Some(ButtonAct::PlayPause), focus: Some(FocusAct::KeepPlaying), shown: Some(Shown { title: "T".into(), ..Default::default() }), ..Default::default() });
        assert_eq!(s.media_button(Press::Next), ButtonAct::PlayPause);
        assert_eq!(s.audio_focus(Focus::LostTransient, true, false), FocusAct::KeepPlaying);
        assert_eq!(s.now_playing("q0".into()).map(|s| s.title).as_deref(), Some("T"));
        let s = queued(1, 0);
        deciding(&s, Fake { on: true, shown: Some(Shown { title: " ".into(), ..Default::default() }), ..Default::default() });
        assert_eq!(s.now_playing("q0".into()), None, "no title: nori's");
    }

    #[test]
    fn a_season_of_a_plugins_own_is_worn_when_it_reads() {
        let nori = nori_look::theme::Season::Spring.palette(false);
        let night = nori_look::theme::Season::Winter.palette(true);
        let s = Session::default();
        let f = deciding(&s, Fake { on: true, look: Some(roles(&night)), ..Default::default() });
        assert_eq!(s.look(nori, None, true), night);
        s.look(nori, None, true);
        assert_eq!(*f.asked.lock(), 1, "remembered");
        let faint = [night.page, night.surface, night.page, night.accent, night.on];
        let s = Session::default();
        deciding(&s, Fake { on: true, look: Some(faint), ..Default::default() });
        assert_eq!(s.look(nori, None, true), nori, "ink on its own page: nori's");
    }
}
