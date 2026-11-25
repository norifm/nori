use super::*;
use crate::hub::Hub;
use crate::profile::tests::{browser, navidrome, phone, youtube, PLUGIN};
use crate::testing::{pair, settle, Device};

const NOW: i64 = 1_700_000_000_000;

/// A group of two: the phone with its profiles, and the Mac paired into it.
fn two(hub: &Hub) -> (Device, Device) {
    let mut pixel = Device::new(hub, "Pixel", phone(), vec![navidrome("a", "https://music.example")]);
    let mut mac = Device::new(hub, "MacBook", phone(), vec![]);
    pair(&mut pixel, &mut mac, NOW);
    settle(&mut [&mut pixel, &mut mac], NOW);
    assert!(pixel.take_events().is_empty(), "the device that let it in told its user when it sent the key");
    (pixel, mac)
}

#[test]
fn a_change_on_one_device_reaches_the_others_while_they_run() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    let mut ipad = Device::new(&hub, "iPad", phone(), vec![]);
    pair(&mut mac, &mut ipad, NOW);
    settle(&mut [&mut pixel, &mut mac, &mut ipad], NOW);
    assert_eq!(ipad.names(), ["Home"]);
    pixel.edit(NOW + 1, |s| s[0].name = "Music".into());
    settle(&mut [&mut pixel, &mut mac, &mut ipad], NOW + 1);
    assert_eq!((mac.names(), ipad.names()), (vec!["Music"], vec!["Music"]));
    ipad.edit(NOW + 2, |s| s.clear());
    settle(&mut [&mut pixel, &mut mac, &mut ipad], NOW + 2);
    assert!(pixel.servers.is_empty() && mac.servers.is_empty(), "removed on every paired device");
}

#[test]
fn every_device_says_who_it_is_and_the_list_keeps_when_each_was_seen() {
    let hub = Hub::new(NOW);
    let (mut pixel, mac) = two(&hub);
    let seen = pixel.session().devices();
    assert_eq!(seen.iter().map(|s| (s.name.as_str(), s.online)).collect::<Vec<_>>(), [("MacBook", true)]);
    mac.link.drop_connection();
    settle(&mut [&mut pixel], NOW + 60_000);
    let seen = pixel.session().devices();
    assert_eq!((seen[0].last_ms, seen[0].online), (NOW + 60_000, false));
}

#[test]
fn a_device_that_reaches_the_room_after_the_new_one_said_hello_still_hears_it_is_new() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    pixel.link.drop_connection();
    let mut ipad = Device::new(&hub, "iPad", phone(), vec![]);
    pair(&mut mac, &mut ipad, NOW + 1);
    settle(&mut [&mut mac, &mut ipad], NOW + 1);
    assert!(mac.take_events().is_empty(), "the Mac let it in: its user heard when the key went");
    assert!(mac.session().devices().iter().any(|d| d.name == "iPad"));
    pixel.act(|s, link| s.connect(link));
    settle(&mut [&mut pixel, &mut mac, &mut ipad], NOW + 2);
    assert_eq!(pixel.take_events(), [SyncEvent::NewDevice { name: "iPad".into() }]);
    // Told once: the iPad's hellos after a drop are not news to anyone.
    for d in [&pixel, &mac] {
        d.link.drop_connection();
    }
    pixel.act(|s, link| s.connect(link));
    mac.act(|s, link| s.connect(link));
    settle(&mut [&mut pixel, &mut mac, &mut ipad], NOW + 3);
    assert_eq!((pixel.take_events(), mac.take_events(), ipad.take_events()), (vec![], vec![], vec![]));
}

#[test]
fn a_device_that_cannot_play_a_profile_gets_no_sign_in_for_it() {
    let hub = Hub::new(NOW);
    let copies = crate::profile::Here { copied: vec!["subsonic".into(), PLUGIN.into()], ..phone() };
    let mut pixel = Device::new(&hub, "Pixel", copies.clone(), vec![navidrome("a", "https://m.example"), youtube("y")]);
    let (mut web, mut mac) = (Device::new(&hub, "Browser", browser(), vec![]), Device::new(&hub, "MacBook", copies, vec![]));
    pair(&mut pixel, &mut web, NOW);
    pair(&mut pixel, &mut mac, NOW);
    settle(&mut [&mut pixel, &mut web, &mut mac], NOW);
    let key = |d: &Device, name: &str| d.servers.iter().find(|s| s.name == name).map(|s| (s.password.clone(), s.api_key.clone())).unwrap();
    assert_eq!(key(&web, "YT").1, "", "the browser lists it without its sign-in");
    assert_eq!(key(&web, "Home").0, "pw", "and plays the server's");
    assert_eq!(key(&mac, "YT").1, "cookie");
    pixel.edit(NOW + 1, |s| s[1].api_key = "new cookie".into());
    settle(&mut [&mut pixel, &mut web, &mut mac], NOW + 1);
    assert_eq!((key(&web, "YT").1.as_str(), key(&mac, "YT").1.as_str()), ("", "new cookie"));
    // Nor does the kept copy hand it over.
    let mut late = Device::new(&hub, "Late browser", browser(), vec![]);
    pair(&mut mac, &mut late, NOW + 2);
    mac.link.drop_connection();
    pixel.link.drop_connection();
    web.link.drop_connection();
    settle(&mut [&mut late], NOW + 2);
    assert_eq!(key(&late, "YT").1, "");
}

#[test]
fn the_kept_copy_lets_a_device_catch_up_alone() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    mac.link.drop_connection();
    pixel.edit(NOW + 1, |s| s.push(youtube("y")));
    settle(&mut [&mut pixel], NOW + 1);
    pixel.link.drop_connection();
    mac.act(|s, l| s.connect(l));
    settle(&mut [&mut mac], NOW + 2);
    assert_eq!(mac.names(), ["Home", "YT"]);
}

#[test]
fn an_older_copy_never_overwrites_newer_state() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    let room = pixel.session().room();
    let old_copy = hub.copy(&room).unwrap();
    // Offline, the Mac renames the profile; the copy on the server is older.
    mac.link.drop_connection();
    pixel.link.drop_connection();
    mac.edit(NOW + 5, |s| s[0].name = "Renamed offline".into());
    mac.act(|s, l| s.connect(l));
    settle(&mut [&mut mac], NOW + 6);
    assert_eq!(mac.names(), ["Renamed offline"]);
    assert_ne!(hub.copy(&room), Some(old_copy), "the newer state went back into the copy");
    pixel.act(|s, l| s.connect(l));
    settle(&mut [&mut pixel], NOW + 7);
    assert_eq!(pixel.names(), ["Renamed offline"]);
}

#[test]
fn without_a_kept_copy_a_device_catches_up_only_with_another_running() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    let room = pixel.session().room();
    pixel.act(|s, l| s.keep_copy(l, false, NOW + 1));
    settle(&mut [&mut pixel, &mut mac], NOW + 1);
    assert_eq!(hub.copy(&room), None, "turning it off deletes it");
    assert!(!mac.session().keeps_copy(), "the choice belongs to the group");
    mac.link.drop_connection();
    pixel.edit(NOW + 2, |s| s[0].name = "Music".into());
    pixel.link.drop_connection();
    mac.act(|s, l| s.connect(l));
    settle(&mut [&mut mac], NOW + 3);
    assert_eq!(mac.names(), ["Home"]);
    pixel.act(|s, l| s.connect(l));
    settle(&mut [&mut pixel, &mut mac], NOW + 4);
    assert_eq!(mac.names(), ["Music"]);
    // Turned on again on the Mac, the copy is back.
    mac.act(|s, l| s.keep_copy(l, true, NOW + 5));
    settle(&mut [&mut pixel, &mut mac], NOW + 5);
    assert!(hub.copy(&room).is_some() && pixel.session().keeps_copy());
}

#[test]
fn reset_pairing_starts_a_group_of_one_and_says_what_the_others_keep() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    let notice = pixel.session().reset_notice();
    assert_eq!(notice, ResetNotice { others: 1, signed_in: vec!["a".into()] });
    let before = pixel.session().room();
    pixel.act(|s, l| s.reset(l));
    settle(&mut [&mut pixel, &mut mac], NOW + 1);
    assert_ne!(pixel.session().room(), before, "a new key");
    assert_eq!(mac.take_events().last(), Some(&SyncEvent::DeviceLeft { name: "Pixel".into() }));
    assert_eq!(mac.names(), ["Home"], "the other device keeps its data");
    mac.edit(NOW + 2, |s| s[0].name = "Changed".into());
    settle(&mut [&mut pixel, &mut mac], NOW + 2);
    assert_eq!(pixel.names(), ["Home"], "and syncs with this one no more");
}

#[test]
fn sync_off_asks_and_its_answer_reaches_the_others() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    assert!(pixel.session().synced("a"));
    pixel.act(|s, l| s.sync_off(l, "a", true, NOW + 1));
    pixel.servers[0].sync_off = true;
    settle(&mut [&mut pixel, &mut mac], NOW + 1);
    assert!(mac.servers.is_empty());
    assert_eq!(pixel.names(), ["Home"]);
}

#[test]
fn devices_with_versions_apart_say_which_needs_an_update() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    let room = mac.session().room();
    let key = mac.session().key().clone();
    let newer = |said: serde_json::Value| {
        let env = serde_json::json!({ "v": 2, "readBy": 2, "said": said });
        key.seal(Sealed::Message, &serde_json::to_vec(&env).unwrap())
    };
    let older = Card { device: "d".into(), name: "Old phone".into(), plays: Plays::Servers, version: 0, open: None };
    let hello = serde_json::json!({ "v": 0, "readBy": 0, "said": Said::Hello { card: older, answer: true, new: false } });
    let hello = key.seal(Sealed::Message, &serde_json::to_vec(&hello).unwrap());
    mac.link.send(&room, To::Everyone, newer(serde_json::json!({ "t": "future" })));
    mac.link.send(&room, To::Everyone, hello);
    settle(&mut [&mut pixel, &mut mac], NOW);
    assert_eq!(pixel.take_events(), [SyncEvent::UpdateThis, SyncEvent::UpdateOther { name: "Old phone".into() }]);
}

#[test]
fn a_message_a_newer_nori_added_is_passed_over() {
    let hub = Hub::new(NOW);
    let (mut pixel, mut mac) = two(&hub);
    let room = mac.session().room();
    let env = serde_json::json!({ "v": 2, "readBy": 1, "said": { "t": "lyricsShared" } });
    let sealed = mac.session().key().seal(Sealed::Message, &serde_json::to_vec(&env).unwrap());
    mac.link.send(&room, To::Everyone, sealed);
    settle(&mut [&mut pixel, &mut mac], NOW);
    assert!(pixel.take_events().is_empty());
}

#[test]
fn a_sign_in_here_is_checked_against_the_account_the_profile_syncs_with() {
    let hub = Hub::new(NOW);
    let mut pixel = Device::new(&hub, "Pixel", phone(), vec![youtube("y")]);
    let mut mac = Device::new(&hub, "MacBook", phone(), vec![]);
    pair(&mut pixel, &mut mac, NOW);
    settle(&mut [&mut pixel, &mut mac], NOW);
    assert_eq!(pixel.act(|s, l| s.signed_in(l, "y", Some("UC1".into()), Some("Ann".into()), NOW + 1)), SignInCheck::Accepted);
    settle(&mut [&mut pixel, &mut mac], NOW + 1);
    let local = mac.servers[0].id.clone();
    assert_eq!(mac.act(|s, l| s.signed_in(l, &local, Some("UC9".into()), None, NOW + 2)), SignInCheck::OtherAccount { name: Some("Ann".into()) });
}

#[test]
fn the_group_is_kept_beside_the_profiles_and_a_device_that_never_paired_has_none() {
    let dir = nori_testdir::TempDir::new("sync-kept");
    let store = Settings::new();
    store.open(dir.path().join("nori.db").to_str().unwrap()).unwrap();
    assert!(Session::open(store.clone()).is_none());
    assert!(!crate::paired(&store));
    let hub = Hub::new(NOW);
    let mut pixel = Device { store: store.clone(), ..Device::new(&hub, "Pixel", phone(), vec![navidrome("a", "https://m")]) };
    let mut mac = Device::new(&hub, "MacBook", phone(), vec![]);
    pair(&mut pixel, &mut mac, NOW);
    settle(&mut [&mut pixel, &mut mac], NOW);
    store.writer.flush();
    let reopened = Session::open(store.clone()).expect("kept");
    assert_eq!((reopened.key(), reopened.devices().len()), (pixel.session().key(), 1));
    assert!(reopened.synced("a"));
}
