//! The `notification` point: the buttons beside previous, play and next on Android's media
//! notification, lock screen and car. The first enabled plugin exporting it whose answer holds
//! decides them; nori's stand otherwise. What was asked and answered is kept, so a plugin is asked
//! again only when what it is handed changes or it says its answer did (`session.buttons-changed`).
//! While no enabled plugin exports it, nothing is asked.

use std::sync::atomic::Ordering;

use nori_model::deciders::{NoriButton, SessionButtons};

use crate::convert::song_out;
use crate::manifest::{Glyph, GLYPHS};
use crate::runtime::nori::plugin::session::Repeat as WitRepeat;
use crate::runtime::points::notification as w;
use crate::session::Repeat;
use crate::worker::DECIDE_FUEL;
use crate::Plugins;

const POINT: &str = "notification";

/// What a press of a notification button does.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ButtonPress {
    Nori { button: NoriButton },
    /// Plugin `plugin`'s lifecycle `action(action)`.
    Plugin { plugin: String, action: String },
}

/// A button beside previous, play and next on the media notification.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NotificationButton {
    pub press: ButtonPress,
    /// A plugin's glyph in place of nori's own.
    pub glyph: Option<Glyph>,
    /// A plugin's label, in its words, in place of nori's own.
    pub label: Option<String>,
}

/// What the buttons were decided from.
#[derive(Clone, PartialEq)]
pub(crate) struct Asked {
    /// Raised by each `buttons-changed`.
    told: u64,
    plugins: Vec<String>,
    song: Option<String>,
    nori: SessionButtons,
    repeat: Repeat,
    lang: String,
}

/// The buttons last decided, and from what.
#[derive(Default)]
pub(crate) struct Decided {
    told: std::sync::atomic::AtomicU64,
    last: std::sync::Mutex<Option<(Asked, Vec<NotificationButton>)>>,
}

fn nori_out(b: NoriButton) -> w::NoriButton {
    match b {
        NoriButton::Favorite => w::NoriButton::Favorite,
        NoriButton::Shuffle => w::NoriButton::Shuffle,
        NoriButton::Repeat => w::NoriButton::Repeat,
        NoriButton::Radio => w::NoriButton::Radio,
    }
}

fn nori_in(b: w::NoriButton) -> NoriButton {
    match b {
        w::NoriButton::Favorite => NoriButton::Favorite,
        w::NoriButton::Shuffle => NoriButton::Shuffle,
        w::NoriButton::Repeat => NoriButton::Repeat,
        w::NoriButton::Radio => NoriButton::Radio,
    }
}

fn repeat_out(r: Repeat) -> WitRepeat {
    match r {
        Repeat::Off => WitRepeat::Off,
        Repeat::All => WitRepeat::All,
        Repeat::One => WitRepeat::One,
    }
}

/// Whether a plugin's `answer` holds: each of nori's buttons at most once and only those nori has now,
/// and a plugin's own with a glyph and a label.
fn holds(answer: &[w::Button], nori: &[NoriButton]) -> bool {
    let mut seen = Vec::new();
    answer.iter().all(|b| match b.press {
        w::Press::Nori(n) => {
            let n = nori_in(n);
            let fresh = nori.contains(&n) && !seen.contains(&n);
            seen.push(n);
            fresh
        }
        w::Press::Action(_) => b.glyph.is_some() && b.label.is_some(),
    })
}

impl Plugins {
    /// Plugin `id` says its buttons would answer otherwise now: they are asked again.
    pub(crate) fn buttons_changed(&self, id: &str) {
        if self.extending(POINT).iter().any(|p| p == id) {
            self.buttons.told.fetch_add(1, Ordering::Relaxed);
            self.side.buttons_changed();
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// Whether an enabled plugin decides the notification's buttons; while none does, nori's stand and
    /// nothing is asked.
    pub fn decides_buttons(&self) -> bool {
        !self.extending(POINT).is_empty()
    }

    /// The notification's buttons while queued song `song` plays, given nori's and the repeat mode,
    /// a plugin's labels in `lang`.
    pub async fn notification_buttons(&self, song: Option<String>, nori: SessionButtons, repeat: Repeat, lang: String) -> Vec<NotificationButton> {
        let plugins = self.extending(POINT);
        let own = || nori.buttons.iter().map(|&b| NotificationButton { press: ButtonPress::Nori { button: b }, glyph: None, label: None }).collect();
        if plugins.is_empty() {
            return own();
        }
        let asked = Asked { told: self.buttons.told.load(Ordering::Relaxed), plugins: plugins.clone(), song: song.clone(), nori: nori.clone(), repeat, lang: lang.clone() };
        if let Some((_, kept)) = self.buttons.last.lock().expect("not poisoned").as_ref().filter(|(a, _)| *a == asked) {
            return kept.clone();
        }
        let found = self.songs.lock().expect("not poisoned").clone();
        let now = w::Playing { song: song.as_deref().and_then(|id| found.as_ref()?(id)).as_ref().map(song_out), favorite: nori.starred, shuffle: nori.shuffling, repeat: repeat_out(repeat) };
        let handed: Vec<w::Button> = nori.buttons.iter().map(|&b| w::Button { press: w::Press::Nori(nori_out(b)), glyph: None, label: None }).collect();
        let mut decided = own();
        for id in plugins {
            let Some(worker) = self.worker(&id) else { continue };
            let (now, handed) = (now.clone(), handed.clone());
            let answer = worker
                .call(move |s, p| {
                    s.set_fuel(DECIDE_FUEL)?;
                    p.nori_plugin_notification().call_buttons(s, &now, &handed)
                })
                .await
                .flatten();
            if let Some(answer) = answer.filter(|a| holds(a, &nori.buttons)) {
                let words = crate::view::words(&self.folder(&id), &lang);
                decided = answer
                    .into_iter()
                    .map(|b| NotificationButton {
                        press: match b.press {
                            w::Press::Nori(n) => ButtonPress::Nori { button: nori_in(n) },
                            w::Press::Action(action) => ButtonPress::Plugin { plugin: id.clone(), action },
                        },
                        glyph: b.glyph.map(|g| GLYPHS[g as usize]),
                        label: b.label.map(|key| words.get(&key).cloned().unwrap_or(key)),
                    })
                    .collect();
                break;
            }
        }
        *self.buttons.last.lock().expect("not poisoned") = Some((asked, decided.clone()));
        decided
    }

    /// A plugin's own notification button was pressed: its lifecycle `action` runs.
    pub fn notification_press(&self, plugin: String, action: String) {
        if !self.running().iter().any(|i| i.manifest.id == plugin && i.manifest.extends(POINT) && i.manifest.extends("lifecycle")) {
            return;
        }
        if let Some(worker) = self.worker(&plugin) {
            worker.post(move |s, p| p.nori_plugin_lifecycle().call_action(s, &action));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_holds_with_noris_buttons_once_each_and_its_own_drawn_and_worded() {
        let nori = |b| w::Button { press: w::Press::Nori(b), glyph: None, label: None };
        let own = |glyph: Option<crate::runtime::types::Glyph>, label: Option<&str>| w::Button { press: w::Press::Action("a".into()), glyph, label: label.map(Into::into) };
        let shown = [NoriButton::Favorite, NoriButton::Shuffle, NoriButton::Repeat];
        let timer = Some(crate::runtime::types::Glyph::Timer);
        for (answer, held) in [
            (vec![], true),
            (vec![nori(w::NoriButton::Shuffle), own(timer, Some("a")), nori(w::NoriButton::Favorite)], true),
            (vec![nori(w::NoriButton::Shuffle), nori(w::NoriButton::Shuffle)], false),
            (vec![nori(w::NoriButton::Radio)], false),
            (vec![own(None, Some("a"))], false),
            (vec![own(timer, None)], false),
        ] {
            assert_eq!(holds(&answer, &shown), held, "{answer:?}");
        }
    }
}
