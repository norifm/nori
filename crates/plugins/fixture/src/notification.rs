//! The notification point as nori's tests check it: none (nori's buttons) but for the songs a test
//! marks. Each ask is said as a notice ("buttons"); its own button's action flips it and asks again.

use nori_plugin_sdk::notices::{show, Tone};
use nori_plugin_sdk::notification::{Button, NoriButton, Playing, Press};
use nori_plugin_sdk::types::Glyph;
use nori_plugin_sdk::{session, store, Notification};

use crate::Test;

/// Its own button's action.
const TOGGLE: &str = "toggle";

/// A lifecycle action this point's tests run: its own button pressed.
pub(crate) fn acted(id: &str) {
    if id == TOGGLE {
        let on = store::get(TOGGLE).is_some();
        if on {
            store::remove(TOGGLE);
        } else {
            store::set(TOGGLE, &[1]).unwrap();
        }
        session::buttons_changed();
    }
}

impl Notification for Test {
    /// "fixture buttons": the seal kept, shuffle drawn as a sparkle, repeat and radio gone, and its own
    /// toggle; "fixture bad buttons": a second seal, which nori refuses.
    fn buttons(&mut self, now: Playing, nori: Vec<Button>) -> Option<Vec<Button>> {
        show(Tone::Info, "buttons", &[]);
        let nori_only = |b: NoriButton| nori.iter().find(|n| matches!(n.press, Press::Nori(k) if k == b)).cloned();
        match now.song?.title.as_str() {
            "fixture buttons" => {
                let toggle = Button { press: Press::Action(TOGGLE.into()), glyph: Some(Glyph::Timer), label: Some(if store::get(TOGGLE).is_some() { "toggled" } else { "toggle" }.into()) };
                let shuffle = Button { glyph: Some(Glyph::Sparkle), label: Some("mix".into()), ..nori_only(NoriButton::Shuffle)? };
                Some(vec![nori_only(NoriButton::Favorite)?, shuffle, toggle])
            }
            "fixture bad buttons" => Some(vec![nori_only(NoriButton::Favorite)?, nori_only(NoriButton::Favorite)?]),
            _ => None,
        }
    }
}
