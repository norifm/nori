use nori_settings::settings::SavedServer;

use super::*;
use crate::profile::tests::{browser, navidrome, phone, youtube, PLUGIN};
use crate::profile::Plays;

/// One device: its profiles and its book, on a clock the test sets.
struct Device {
    by: &'static str,
    here: Here,
    servers: Vec<SavedServer>,
    book: Book,
    now_ms: i64,
}

impl Device {
    fn new(by: &'static str, servers: Vec<SavedServer>) -> Device {
        Device { by, here: phone(), servers, book: Book::default(), now_ms: 1_000 }
    }

    /// What this device tells the others after its user's edits.
    fn say(&mut self) -> Changes {
        self.book.notice(&self.servers, &self.here, self.by, self.now_ms)
    }

    fn hear(&mut self, theirs: Changes) -> bool {
        self.say();
        let theirs = theirs.fitted(self.here.plays);
        self.book.take(theirs, &mut self.servers, &self.here)
    }

    /// Both devices tell each other everything, as a meeting in the group room does.
    fn meet(&mut self, other: &mut Device) {
        self.say();
        other.say();
        let (mine, theirs) = (self.book.state(), other.book.state());
        other.hear(mine.clone());
        let back = other.book.answer(&mine, true);
        self.hear(theirs);
        self.hear(back);
        let more = self.book.answer(&other.book.state(), true);
        other.hear(more);
    }

    fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.servers.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    fn get(&self, name: &str) -> &SavedServer {
        self.servers.iter().find(|s| s.name == name).unwrap()
    }

    fn edit(&mut self, name: &str, f: impl FnOnce(&mut SavedServer)) {
        f(self.servers.iter_mut().find(|s| s.name == name).unwrap());
    }
}

fn named(name: &str, url: &str) -> SavedServer {
    SavedServer { name: name.into(), ..navidrome(&format!("id-{name}"), url) }
}

#[test]
fn a_merge_keeps_the_profile_in_use_else_the_first() {
    let offer = MergeOffer { a: "home".into(), b: "public".into() };
    assert_eq!(offer.kept("public"), ("public", "home"));
    assert_eq!(offer.kept("home"), ("home", "public"));
    assert_eq!(offer.kept("other"), ("home", "public"));
}

#[test]
fn profiles_added_on_two_devices_apart_are_both_kept() {
    let (mut a, mut b) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![named("Work", "https://work.example")]));
    a.meet(&mut b);
    assert_eq!((a.names(), b.names()), (vec!["Home", "Work"], vec!["Home", "Work"]));
    assert_eq!(a.get("Home").id, "id-Home", "a device's own profile keeps its id, and its local data with it");
    assert_ne!(b.get("Home").id, "id-Home", "the other device gives it its own");
    assert_eq!(b.get("Home").client_cert, "", "a certificate is a file on its device");
    assert_eq!(b.get("Home").password, "pw", "a server's sign-in is copied");
}

#[test]
fn the_newer_change_wins_for_the_whole_profile() {
    let (mut a, mut b) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![]));
    a.meet(&mut b);
    a.edit("Home", |s| s.wifi_only = true);
    a.now_ms += 10;
    b.edit("Home", |s| (s.name, s.alt_max_bit_rate) = ("House".into(), 128));
    let from_a = a.say();
    b.now_ms += 20;
    let from_b = b.say();
    a.hear(from_b);
    b.hear(from_a);
    for d in [&a, &b] {
        let s = d.get("House");
        assert_eq!((s.alt_max_bit_rate, s.wifi_only), (128, false), "{}: b's change won, whole", d.by);
    }
}

#[test]
fn the_counter_orders_changes_before_the_clock() {
    let (mut a, mut b) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![]));
    b.now_ms = -50_000;
    a.meet(&mut b);
    a.edit("Home", |s| s.name = "First".into());
    let first = a.say();
    b.hear(first);
    // b's clock is far behind, but its change follows the one it took.
    b.edit("First", |s| s.name = "Second".into());
    let second = b.say();
    a.hear(second);
    assert_eq!((a.names(), b.names()), (vec!["Second"], vec!["Second"]));
}

#[test]
fn a_removed_profile_does_not_come_back() {
    let (mut a, mut b, mut c) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![]), Device::new("c", vec![]));
    a.meet(&mut b);
    b.meet(&mut c);
    a.servers.clear();
    let removed = a.say();
    assert_eq!(removed.removed.len(), 1);
    b.hear(removed);
    assert!(b.servers.is_empty());
    // c never heard it, and says it still has the profile.
    b.meet(&mut c);
    a.meet(&mut c);
    assert!(a.servers.is_empty() && b.servers.is_empty() && c.servers.is_empty());
}

#[test]
fn removed_on_one_device_and_changed_later_on_another_the_later_wins() {
    for change_later in [true, false] {
        let (mut a, mut b) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![]));
        a.meet(&mut b);
        let (removal, change);
        if change_later {
            a.servers.clear();
            removal = a.say();
            b.hear(removal.clone());
            b.servers.push(SavedServer { name: "Back".into(), ..named("Home", "https://home.example") });
            change = b.say();
        } else {
            b.edit("Home", |s| s.name = "Changed".into());
            change = b.say();
            a.hear(change.clone());
            a.servers.clear();
            removal = a.say();
        }
        a.hear(change);
        b.hear(removal);
        let expected: Vec<&str> = if change_later { vec!["Back"] } else { vec![] };
        assert_eq!((a.names(), b.names()), (expected.clone(), expected), "changed later: {change_later}");
    }
}

#[test]
fn the_first_sync_is_a_union_where_one_profile_on_both_is_one() {
    let mut a = Device::new("a", vec![named("Home", "https://music.example/"), named("Old", "https://old.example")]);
    let mut b = Device::new("b", vec![SavedServer { id: "b-own".into(), ..named("Music", "HTTPS://music.example") }]);
    b.now_ms = 2_000;
    a.meet(&mut b);
    assert_eq!((a.names(), b.names()), (vec!["Music", "Old"], vec!["Music", "Old"]), "the later named one wins");
    assert_eq!((a.get("Music").id.as_str(), b.get("Music").id.as_str()), ("id-Home", "b-own"), "each keeps its own profile and its data");
    let (ra, rb) = (a.book.record("id-Home").unwrap(), b.book.record("b-own").unwrap());
    assert_eq!(ra.shared.id, rb.shared.id, "one sync identity");
    // A third device that already had it under the other identity follows.
    let mut c = Device::new("c", vec![named("Music", "https://music.example")]);
    c.meet(&mut b);
    assert_eq!(c.servers.len(), 2);
}

#[test]
fn a_profile_nori_cannot_tell_is_offered_for_merging() {
    let mut a = Device::new("a", vec![named("Home", "http://192.168.1.5:4533")]);
    let mut b = Device::new("b", vec![named("Public", "https://music.example")]);
    a.meet(&mut b);
    assert_eq!(a.names(), ["Home", "Public"]);
    let offers = a.book.offers(&a.servers);
    assert_eq!(offers.len(), 1);
    assert_eq!(b.book.offers(&b.servers).len(), 1);
    // Unrelated profiles are not offered.
    let mut c = Device::new("c", vec![SavedServer { user: "other".into(), ..named("Elsewhere", "http://10.0.0.2") }]);
    c.meet(&mut a);
    assert_eq!(c.book.offers(&c.servers).len(), 1, "only the pair");
    // Merged on b, keeping the public address: each device keeps its own profile.
    let (keep, drop) = (b.get("Public").id.clone(), b.get("Home").id.clone());
    let merged = b.book.merge(&keep, &drop, &mut b.servers, "b", 5_000);
    assert_eq!(b.names(), ["Public"]);
    a.hear(merged);
    assert_eq!(a.names(), ["Elsewhere", "Public"]);
    assert_eq!(a.get("Public").id, "id-Home", "a keeps the profile its user added, now at the public address");
    assert_eq!(a.get("Public").url, "https://music.example");
    assert!(a.book.offers(&a.servers).is_empty());
    // Kept apart, an offer goes away.
    let (x, y) = (c.get("Home").id.clone(), c.get("Public").id.clone());
    c.book.keep_apart(&x, &y);
    assert!(c.book.offers(&c.servers).is_empty());
}

#[test]
fn a_newer_nori_s_parts_of_a_profile_travel_unchanged() {
    let (mut a, mut b) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![]));
    a.meet(&mut b);
    let mut newer = a.book.state();
    newer.profiles[0].profile.insert("lyricsAccount".into(), "x".into());
    newer.profiles[0].newer.insert("mood".into(), "calm".into());
    newer.profiles[0].stamp.n += 1;
    b.hear(newer);
    b.edit("Home", |s| s.name = "Renamed".into());
    let sent = b.say();
    assert_eq!((sent.profiles[0].profile["lyricsAccount"].as_str(), sent.profiles[0].newer["mood"].as_str()), (Some("x"), Some("calm")));
}

#[test]
fn a_sign_in_that_is_not_copied_stays_on_its_device() {
    let (mut a, mut b) = (Device::new("a", vec![youtube("yt")]), Device::new("b", vec![]));
    a.meet(&mut b);
    assert_eq!(b.get("YT").api_key, "", "synced without the sign-in");
    b.edit("YT", |s| s.api_key = "b's own".into());
    assert!(b.say().is_empty(), "a sign-in here is no change for the others");
    a.edit("YT", |s| s.name = "Tube".into());
    let rename = a.say();
    b.hear(rename);
    assert_eq!((b.get("Tube").api_key.as_str(), a.get("Tube").api_key.as_str()), ("b's own", "cookie"));
}

#[test]
fn a_sign_in_on_another_device_is_accepted_only_for_the_same_account() {
    let (mut a, mut b) = (Device::new("a", vec![youtube("yt")]), Device::new("b", vec![]));
    a.meet(&mut b);
    let (check, said) = a.book.signed_in("yt", Some("UC1".into()), Some("Ann".into()), "a", 2_000);
    assert_eq!(check, SignInCheck::Accepted);
    b.hear(said);
    let local = b.get("YT").id.clone();
    assert_eq!(b.book.signed_in(&local, Some("UC2".into()), Some("Bob".into()), "b", 3_000).0, SignInCheck::OtherAccount { name: Some("Ann".into()) }, "named as the account's own device names it");
    assert_eq!(b.book.signed_in(&local, Some("UC1".into()), Some("Ann".into()), "b", 3_000), (SignInCheck::Accepted, Changes::default()));
    let (_, renamed) = b.book.signed_in(&local, Some("UC1".into()), Some("Ann B.".into()), "b", 4_000);
    a.hear(renamed);
    assert_eq!(a.book.signed_in("yt", Some("UC3".into()), None, "a", 5_000).0, SignInCheck::OtherAccount { name: Some("Ann B.".into()) }, "the account's new name travels");
}

#[test]
fn a_sign_in_naming_no_account_is_kept_unchecked_and_names_none() {
    let (mut a, mut b) = (Device::new("a", vec![youtube("yt")]), Device::new("b", vec![]));
    a.meet(&mut b);
    assert_eq!(a.book.signed_in("yt", None, None, "a", 2_000), (SignInCheck::Unchecked, Changes::default()));
    let local = b.get("YT").id.clone();
    let (check, said) = b.book.signed_in(&local, Some("UC2".into()), None, "b", 3_000);
    assert_eq!(check, SignInCheck::Accepted, "no account was named before");
    a.hear(said);
    assert_eq!(a.book.signed_in("yt", Some("UC1".into()), None, "a", 4_000).0, SignInCheck::OtherAccount { name: None }, "the account named first is the profile's");
}

#[test]
fn a_plugin_a_profile_needs_is_installed_where_plugins_run() {
    let mut a = Device::new("a", vec![youtube("yt")]);
    let mut b = Device { here: Here { plugins: vec![], ..phone() }, ..Device::new("b", vec![]) };
    a.say();
    b.hear(a.book.state());
    assert_eq!(b.book.needed(&b.here).iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), [PLUGIN]);
    assert!(b.book.needed(&phone()).is_empty(), "installed, it is needed no more");
    let mut web = Device { here: browser(), ..Device::new("w", vec![]) };
    web.hear(a.book.state());
    assert!(web.book.needed(&web.here).is_empty(), "a browser runs no plugins");
    assert_eq!(web.names(), ["YT"], "it lists the profile, as unavailable");
}

#[test]
fn a_device_that_cannot_play_a_profile_keeps_no_sign_in_for_it_and_takes_none_away() {
    let copied = Here { copied: vec!["subsonic".into(), PLUGIN.into()], ..phone() };
    let mut a = Device { here: copied, ..Device::new("a", vec![youtube("yt")]) };
    let mut web = Device { here: browser(), ..Device::new("w", vec![]) };
    a.meet(&mut web);
    assert_eq!(web.get("YT").api_key, "");
    assert_eq!(web.here.plays, Plays::Servers);
    web.edit("YT", |s| s.name = "Renamed on the web".into());
    let rename = web.say();
    a.hear(rename);
    assert_eq!(a.get("Renamed on the web").api_key, "cookie");
}

#[test]
fn sync_off_keeps_the_profile_here_and_removes_it_elsewhere_if_asked() {
    for elsewhere in [false, true] {
        let (mut a, mut b) = (Device::new("a", vec![named("Home", "https://home.example")]), Device::new("b", vec![]));
        a.meet(&mut b);
        assert!(a.book.synced("id-Home"));
        let said = a.book.sync_off("id-Home", elsewhere, "a", 2_000);
        a.edit("Home", |s| s.sync_off = true);
        b.hear(said);
        assert_eq!(a.names(), ["Home"]);
        assert_eq!(b.servers.len(), usize::from(!elsewhere), "elsewhere: {elsewhere}");
        // What b says about it no longer reaches a.
        if !elsewhere {
            b.edit("Home", |s| s.name = "Changed".into());
            let change = b.say();
            a.hear(change);
        }
        a.meet(&mut b);
        assert_eq!(a.names(), ["Home"]);
        assert!(!a.book.synced("id-Home"));
    }
}

#[test]
fn a_jam_guest_s_profile_never_syncs() {
    let mut a = Device::new("a", vec![SavedServer { api_key: nori_remote::guest_key("k"), ..named("Jam", "https://music.example") }]);
    assert!(a.say().is_empty());
}

#[test]
fn the_sign_ins_the_group_holds_are_listed() {
    let mut a = Device::new("a", vec![named("Home", "https://home.example"), youtube("yt"), SavedServer { password: String::new(), ..named("Open", "https://open.example") }]);
    a.say();
    assert_eq!(a.book.signed_in_elsewhere(), ["id-Home"]);
}
