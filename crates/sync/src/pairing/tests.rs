use std::time::Duration;

use super::*;
use crate::hub::{Hub, HubLink};
use crate::profile::tests::{navidrome, phone, youtube};
use crate::room::Relay;
use crate::session::SyncEvent;
use crate::testing::{pair, settle, Device, SERVER};

const NOW: i64 = 1_700_000_000_000;

/// The new device's code: its link and words.
fn code(link: &HubLink, joining: &mut Joining) -> (String, Vec<String>) {
    link.heard().into_iter().find_map(|h| match joining.heard(link, h, Duration::ZERO) {
        Some(JoinStep::Code { link, words }) => Some((link, words)),
        _ => None,
    }).expect("a code")
}

fn steps(link: &HubLink, admitting: &mut Admitting, own: Option<&Session>, servers: &[SavedServer]) -> Vec<AdmitStep> {
    link.heard().into_iter().filter_map(|h| admitting.heard(link, h, own, servers)).collect()
}

fn join_steps(link: &HubLink, joining: &mut Joining, now: Duration) -> Vec<JoinStep> {
    link.heard().into_iter().filter_map(|h| joining.heard(link, h, now)).collect()
}

#[test]
fn the_first_pairing_makes_the_group_and_the_new_device_gets_its_key() {
    let hub = Hub::new(NOW);
    let mut pixel = Device::new(&hub, "Pixel", phone(), vec![navidrome("a", "https://music.example"), youtube("y")]);
    let mut mac = Device::new(&hub, "MacBook", phone(), vec![]);
    let confirm = pair(&mut pixel, &mut mac, NOW);
    assert_eq!(confirm, Confirmation { name: "MacBook".into(), profiles: 2, leaves: None, words: None, asks_keep_copy: true });
    let (p, m) = (pixel.session(), mac.session.as_ref().unwrap());
    assert_eq!(p.key(), m.key(), "one key");
    assert_eq!(m.server(), SERVER, "the code carried the server");
    assert!(m.keeps_copy(), "the answer the first pairing carried");
    settle(&mut [&mut pixel, &mut mac], NOW);
    assert_eq!(mac.names(), ["Home", "YT"]);
    assert_eq!(pixel.take_events(), [SyncEvent::NewDevice { name: "MacBook".into() }]);
}

#[test]
fn a_code_by_link_or_nearby_shows_the_same_words_on_both_screens() {
    for came in [Came::Link, Came::Nearby] {
        let hub = Hub::new(NOW);
        let (mac, pixel) = (hub.link(), hub.link());
        let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
        let (link, words) = code(&mac, &mut joining);
        assert_eq!(words.len(), 3);
        let mut admitting = Admitting::read(&link, came, None).unwrap();
        admitting.check(&pixel);
        let [AdmitStep::Confirm(c)] = &steps(&pixel, &mut admitting, None, &[youtube("y"), navidrome("a", "https://m")])[..] else { panic!("a confirmation") };
        assert_eq!(c.words.as_ref(), Some(&words), "{came:?}");
        assert_eq!(c.profiles, 1, "a browser receives the server's profile, not the plugin's");
    }
}

#[test]
fn a_code_dies_after_five_minutes() {
    let hub = Hub::new(NOW);
    let (mac, pixel) = (hub.link(), hub.link());
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let (link, _) = code(&mac, &mut joining);
    hub.advance(CODE_MS);
    assert!(matches!(&join_steps(&mac, &mut joining, Duration::from_millis(CODE_MS as u64))[..], [JoinStep::Refused(PairingRefused::Expired)]));
    let mut admitting = Admitting::read(&link, Came::Link, None).unwrap();
    admitting.check(&pixel);
    assert_eq!(steps(&pixel, &mut admitting, None, &[]), [AdmitStep::Refused(PairingRefused::Expired)], "by the server's clock");
}

#[test]
fn a_key_arriving_after_five_minutes_by_this_device_s_clock_is_refused() {
    let hub = Hub::new(NOW);
    let (mac, pixel) = (hub.link(), hub.link());
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let (link, _) = code(&mac, &mut joining);
    let mut admitting = Admitting::read(&link, Came::Camera, None).unwrap();
    admitting.check(&pixel);
    steps(&pixel, &mut admitting, None, &[]);
    admitting.admit_first(&pixel, "Pixel".into(), false, Settings::new(), NOW);
    steps(&pixel, &mut admitting, None, &[]);
    let late = Duration::from_millis(CODE_MS as u64 + 1);
    assert!(matches!(&join_steps(&mac, &mut joining, late)[..], [JoinStep::Refused(PairingRefused::Expired)]));
}

#[test]
fn codes_that_are_wrong_or_from_a_newer_nori_are_refused() {
    let hub = Hub::new(NOW);
    let mac = hub.link();
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let (link, _) = code(&mac, &mut joining);
    assert!(Admitting::read(&format!("  {link}\n"), Came::Link, None).is_ok(), "pasted with spaces");
    let cut = |k: &str| link.split('&').filter(|kv| !kv.starts_with(&format!("{k}="))).collect::<Vec<_>>().join("&");
    for (text, why) in [
        (String::new(), "empty"),
        ("https://example.com".into(), "a web page"),
        ("nori://jam?s=x&k=y".into(), "a jam invite"),
        (link.replace("v=1", "v=x"), "no version"),
        (cut("r"), "no meeting"),
        (cut("l"), "no lock"),
        (link.replace("&l=", "&l=AA"), "a lock cut wrong"),
        (cut("e"), "no expiry"),
        (link.replace("&p=s", "&p=q"), "an unknown kind of device"),
        (format!("{link}&g=1"), "a group without its tag"),
        (link.replace(&format!("s={}", b64(SERVER.as_bytes())), &format!("s={}", b64(b"ftp://x"))), "a server that is no web address"),
    ] {
        assert_eq!(Admitting::read(&text, Came::Link, None).err(), Some(PairingRefused::NotACode), "{why}");
    }
    assert_eq!(Admitting::read(&link.replace("v=1", "v=2"), Came::Link, None).err(), Some(PairingRefused::Newer));
}

#[test]
fn a_tampered_or_misdirected_key_is_refused_and_the_device_waits_on() {
    let hub = Hub::new(NOW);
    let (mac, pixel) = (hub.link(), hub.link());
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let (link, _) = code(&mac, &mut joining);
    let meeting = Code::parse(&link).unwrap().meeting;
    // Someone in the meeting room sends a key sealed to another lock, then garbage.
    pixel.join(&meeting, RoomKind::Meeting);
    let other = Lock::new();
    pixel.send(&meeting, To::Everyone, other.key().seal(b"{}", &meeting).unwrap());
    pixel.send(&meeting, To::Everyone, b"nonsense".to_vec());
    let refused: Vec<_> = join_steps(&mac, &mut joining, Duration::ZERO).into_iter().map(|s| matches!(s, JoinStep::Refused(PairingRefused::Tampered))).collect();
    assert_eq!(refused, [true, true]);
    pixel.leave(&meeting);
    pixel.heard();
    mac.heard();
    // The real key still pairs it.
    let mut admitting = Admitting::read(&link, Came::Camera, None).unwrap();
    admitting.check(&pixel);
    steps(&pixel, &mut admitting, None, &[]);
    admitting.admit_first(&pixel, "Pixel".into(), false, Settings::new(), NOW);
    assert_eq!(steps(&pixel, &mut admitting, None, &[]), [AdmitStep::Sent]);
    assert!(matches!(&join_steps(&mac, &mut joining, Duration::ZERO)[..], [JoinStep::Paired(_)]));
}

#[test]
fn a_replayed_key_is_refused() {
    let hub = Hub::new(NOW);
    let (mac, pixel, spy) = (hub.link(), hub.link(), hub.link());
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let (link, _) = code(&mac, &mut joining);
    let mut admitting = Admitting::read(&link, Came::Camera, None).unwrap();
    admitting.check(&pixel);
    steps(&pixel, &mut admitting, None, &[]);
    admitting.admit_first(&pixel, "Pixel".into(), false, Settings::new(), NOW);
    steps(&pixel, &mut admitting, None, &[]);
    let Some(Heard::Message { payload, .. }) = mac.heard().into_iter().find(|h| matches!(h, Heard::Message { .. })) else { panic!("the key") };
    let welcome = Heard::Message { room: Code::parse(&link).unwrap().meeting, from: crate::room::Member(9), payload: payload.clone() };
    assert!(matches!(joining.heard(&mac, welcome.clone(), Duration::ZERO), Some(JoinStep::Paired(_))));
    assert!(matches!(joining.heard(&mac, welcome, Duration::ZERO), Some(JoinStep::Refused(PairingRefused::Spent))));
    // Sent into another device's meeting, it opens for no one.
    let mut other = Joining::start(&spy, SERVER.into(), "Spy".into(), Plays::Servers, None, Duration::ZERO);
    code(&spy, &mut other);
    let elsewhere = Heard::Message { room: other.meeting.clone(), from: crate::room::Member(9), payload };
    assert!(matches!(other.heard(&spy, elsewhere, Duration::ZERO), Some(JoinStep::Refused(PairingRefused::Tampered))));
}

#[test]
fn a_device_already_in_the_group_is_told_so() {
    let hub = Hub::new(NOW);
    let mut pixel = Device::new(&hub, "Pixel", phone(), vec![]);
    let mut mac = Device::new(&hub, "MacBook", phone(), vec![]);
    pair(&mut pixel, &mut mac, NOW);
    settle(&mut [&mut pixel, &mut mac], NOW);
    let mut joining = Joining::start(&mac.link, SERVER.into(), "MacBook".into(), Plays::Servers, mac.session.as_ref(), Duration::ZERO);
    let (link, _) = code(&mac.link, &mut joining);
    assert_eq!(Admitting::read(&link, Came::Camera, pixel.session.as_ref()).err(), Some(PairingRefused::AlreadyPaired));
    let (mut other, mut third) = (Device::new(&hub, "Other", phone(), vec![]), Device::new(&hub, "Third", phone(), vec![]));
    pair(&mut other, &mut third, NOW);
    assert!(Admitting::read(&link, Came::Camera, other.session.as_ref()).is_ok(), "another group's device may take it");
}

#[test]
fn a_device_paired_into_another_group_leaves_its_own_and_says_so() {
    let hub = Hub::new(NOW);
    let mut old_friend = Device::new(&hub, "iPad", phone(), vec![]);
    let mut mac = Device::new(&hub, "MacBook", phone(), vec![navidrome("m", "https://mac.example")]);
    pair(&mut mac, &mut old_friend, NOW);
    settle(&mut [&mut mac, &mut old_friend], NOW);
    old_friend.take_events();
    let mut pixel = Device::new(&hub, "Pixel", phone(), vec![youtube("y")]);
    let confirm = pair(&mut pixel, &mut mac, NOW);
    assert_eq!(confirm.leaves, Some(1), "MacBook stops syncing with its 1 other device");
    settle(&mut [&mut pixel, &mut mac, &mut old_friend], NOW);
    assert_eq!(old_friend.take_events(), [SyncEvent::DeviceLeft { name: "MacBook".into() }]);
    assert!(old_friend.session().devices().is_empty());
    assert_eq!((pixel.names(), mac.names()), (vec!["Home", "YT"], vec!["Home", "YT"]), "the first sync is a union");
    old_friend.servers.clear();
    old_friend.edit(NOW, |_| {});
    settle(&mut [&mut pixel, &mut mac, &mut old_friend], NOW);
    assert_eq!(mac.names(), ["Home", "YT"], "the old group reaches it no more");
}

#[test]
fn a_new_device_gone_before_the_key_is_told_apart() {
    let hub = Hub::new(NOW);
    let (mac, pixel) = (hub.link(), hub.link());
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let (link, _) = code(&mac, &mut joining);
    let mut admitting = Admitting::read(&link, Came::Camera, None).unwrap();
    admitting.check(&pixel);
    steps(&pixel, &mut admitting, None, &[]);
    mac.drop_connection();
    admitting.admit_first(&pixel, "Pixel".into(), false, Settings::new(), NOW);
    assert_eq!(steps(&pixel, &mut admitting, None, &[]), [AdmitStep::Refused(PairingRefused::Gone)]);
}

#[test]
fn a_server_refusal_is_said_and_ends_the_pairing() {
    let hub = Hub::new(NOW);
    hub.wall(true);
    let mac = hub.link();
    let mut joining = Joining::start(&mac, SERVER.into(), "MacBook".into(), Plays::Servers, None, Duration::ZERO);
    let refused = Some(PairingRefused::Server { reason: Refusal::DailyWall });
    let steps = join_steps(&mac, &mut joining, Duration::ZERO);
    assert!(!steps.is_empty() && steps.iter().all(|s| matches!(s, JoinStep::Refused(r) if Some(*r) == refused)));
}
