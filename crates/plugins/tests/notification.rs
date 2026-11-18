//! The notification point over the test plugin: its buttons stand in for nori's where they hold, it is
//! asked again only when what it is handed changes or it says its answer did, and its own button's
//! press reaches its action.

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nori_model::deciders::{NoriButton, SessionButtons};
use nori_model::Song;
use nori_plugins::testing::FIXTURE;
use nori_plugins::{ButtonPress, Glyph, NotificationButton, Plugins, Repeat};

const NORI: [NoriButton; 4] = [NoriButton::Favorite, NoriButton::Shuffle, NoriButton::Repeat, NoriButton::Radio];

fn nori() -> SessionButtons {
    SessionButtons { buttons: NORI.to_vec(), starred: false, shuffling: false }
}

fn own(buttons: &[NoriButton]) -> Vec<NotificationButton> {
    buttons.iter().map(|&b| NotificationButton { press: ButtonPress::Nori { button: b }, glyph: None, label: None }).collect()
}

fn buttons(plugins: &Plugins, song: &str, nori: SessionButtons) -> Vec<NotificationButton> {
    nori_tasks::block_on(plugins.notification_buttons(Some(song.into()), nori, Repeat::Off, "en".into()))
}

/// The songs by id, each titled as its id says what the fixture does with it.
fn titled(plugins: &Plugins) {
    plugins.set_songs(Arc::new(|id: &str| {
        let title = match id {
            "marked" => "fixture buttons",
            "bad" => "fixture bad buttons",
            _ => "unmarked",
        };
        Some(Song { id: id.into(), title: title.into(), ..Default::default() })
    }));
}

fn asked(side: &common::Noted) -> usize {
    side.notices.lock().unwrap().iter().filter(|(w, _)| w == "buttons").count()
}

#[test]
fn a_plugins_buttons_stand_in_for_noris_where_they_hold() {
    let dir = nori_testdir::TempDir::new("plugins-notification");
    let (plugins, side) = common::plugged(&dir, "");
    titled(&plugins);
    assert!(plugins.decides_buttons());

    assert_eq!(buttons(&plugins, "plain", nori()), own(&NORI), "no answer: nori's");
    let toggle = |label: &str| NotificationButton { press: ButtonPress::Plugin { plugin: FIXTURE.into(), action: "toggle".into() }, glyph: Some(Glyph::Timer), label: Some(label.into()) };
    let shuffle = NotificationButton { glyph: Some(Glyph::Sparkle), label: Some("Mix".into()), ..own(&[NoriButton::Shuffle]).remove(0) };
    let marked = vec![own(&[NoriButton::Favorite]).remove(0), shuffle.clone(), toggle("Toggle")];
    assert_eq!(buttons(&plugins, "marked", nori()), marked, "repeat and radio hidden, shuffle redrawn, its own added, in its words");
    assert_eq!(buttons(&plugins, "bad", nori()), own(&NORI), "a second seal: nori's");
    let radio = SessionButtons { buttons: vec![NoriButton::Shuffle, NoriButton::Repeat], ..nori() };
    assert_eq!(buttons(&plugins, "marked", radio.clone()), own(&radio.buttons), "a seal nori doesn't show: nori's");

    plugins.turn(FIXTURE.into(), false);
    assert!(!plugins.decides_buttons());
    let before = asked(&side);
    assert_eq!(buttons(&plugins, "marked", nori()), own(&NORI), "off: nori's");
    assert_eq!(asked(&side), before, "off: nothing is asked");
}

#[test]
fn buttons_are_asked_again_only_when_what_they_show_may_change() {
    let dir = nori_testdir::TempDir::new("plugins-notification-asked");
    let (plugins, side) = common::plugged(&dir, "");
    titled(&plugins);
    buttons(&plugins, "marked", nori());
    buttons(&plugins, "marked", nori());
    assert_eq!(asked(&side), 1, "the same song and state: the answer kept");
    buttons(&plugins, "marked", SessionButtons { shuffling: true, ..nori() });
    buttons(&plugins, "plain", nori());
    assert_eq!(asked(&side), 3, "shuffle on, another song: asked");

    plugins.notification_press(FIXTURE.into(), "toggle".into());
    let waited = Instant::now();
    while side.buttons_changed.load(Ordering::Relaxed) == 0 {
        assert!(waited.elapsed() < Duration::from_secs(10), "the press reached its action, which said its buttons changed");
        std::thread::yield_now();
    }
    let now = buttons(&plugins, "marked", nori());
    assert_eq!(asked(&side), 4, "asked again once it said so");
    assert_eq!(now[2].label.as_deref(), Some("Toggled"));
}
