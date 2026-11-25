use std::sync::atomic::AtomicUsize;

use nori_tasks::Native;
use nori_testdir::TempDir;

use super::*;
use crate::hub::{Hub, HubSockets as Net};
use crate::profile::tests::{navidrome, phone};

const S: Duration = Duration::from_secs(1);
const NOW: i64 = 1_700_000_000_000;
/// How far a device's executor clock is ahead of the device's own.
const EXECUTOR_AHEAD_US: i64 = 1_000 * 3_600_000_000;

/// What the client heard from sync.
#[derive(Default)]
struct Told {
    synced: Mutex<Vec<ProfilesSynced>>,
    events: Mutex<Vec<SyncEvent>>,
    /// `changed` calls since last read.
    changed: AtomicUsize,
}

impl SyncClient for Told {
    fn here(&self) -> Here {
        phone()
    }
    fn profiles(&self, synced: ProfilesSynced) {
        self.synced.lock().unwrap().push(synced);
    }
    fn install(&self, _: Vec<NeededPlugin>) {}
    fn event(&self, event: SyncEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn changed(&self) {
        self.changed.fetch_add(1, Ordering::Relaxed);
    }
}

/// A device: its settings, its sync, its own executor and clock, and its sockets.
struct Device {
    syncing: Arc<Syncing>,
    settings: Arc<Settings>,
    tasks: Arc<Native>,
    net: Arc<Net>,
    told: Arc<Told>,
    /// Executors sync made.
    made: Arc<AtomicUsize>,
    _dir: TempDir,
}

impl Device {
    fn new(hub: &Hub, reach: Reach, servers: Vec<SavedServer>) -> Device {
        let dir = TempDir::new("syncing");
        let settings = Settings::new();
        settings.open(&dir.join("nori.db").display().to_string()).unwrap();
        let active_server_id = servers.first().map(|s| s.id.clone()).unwrap_or_default();
        settings.put(StoredPrefs { servers, active_server_id, ..settings.current().unwrap() });
        // An executor's clock of its own, far from the device's: sync reads only the executor's.
        let tasks = Arc::new(Native::simulated("syncing-test", nori_time::device_us() + EXECUTOR_AHEAD_US).unwrap());
        let (net, told, made) = (Net::new(hub, None), Arc::new(Told::default()), Arc::new(AtomicUsize::new(0)));
        let (executor, counted) = (tasks.clone(), made.clone());
        let syncing = Syncing::new(settings.clone(), net.clone(), told.clone(), reach, Box::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            executor.clone() as Arc<dyn Tasks>
        }));
        Device { syncing, settings, tasks, net, told, made, _dir: dir }
    }

    fn names(&self) -> Vec<String> {
        self.settings.current().unwrap().servers.iter().map(|s| s.name.clone()).collect()
    }

    fn rename(&self, name: &str) {
        let mut prefs = self.settings.current().unwrap();
        prefs.servers[0].name = name.into();
        self.settings.put(prefs);
    }

    fn set_sync(&self, on: bool) {
        self.settings.put(StoredPrefs { sync: on, ..self.settings.current().unwrap() });
        self.syncing.settings_changed();
    }
}

/// Runs every device until none hears anything more.
fn settle(devices: &[&Device]) {
    for _ in 0..100 {
        devices.iter().for_each(|d| d.tasks.settle());
        if !devices.iter().fold(false, |any, d| d.net.deliver(&d.syncing) | any) {
            return devices.iter().for_each(|d| d.tasks.settle());
        }
    }
    panic!("the devices never went quiet");
}

/// Pairs `new` into `old`'s group (making it, keeping a copy, if `old` has none) by the camera.
fn pair(old: &Device, new: &Device) {
    new.syncing.pair_show("MacBook".into());
    settle(&[old, new]);
    let PairingView::Code { link, words } = new.syncing.pairing() else { panic!("a code: {:?}", new.syncing.pairing()) };
    assert_eq!(words.len(), 3);
    assert_eq!(old.syncing.pair_take(link, Came::Camera), None);
    settle(&[old, new]);
    let PairingView::Confirm { confirmation } = old.syncing.pairing() else { panic!("something to confirm: {:?}", old.syncing.pairing()) };
    assert_eq!(confirmation.name, "MacBook");
    old.syncing.pair_confirm("Pixel".into(), true);
    settle(&[old, new]);
    assert_eq!((old.syncing.pairing(), new.syncing.pairing()), (PairingView::Sent, PairingView::Paired));
}

#[test]
fn two_devices_pair_through_the_server_and_their_profiles_sync_both_ways() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    assert_eq!((pixel.syncing.state(), mac.syncing.state()), (SyncState::Connected, SyncState::Connected));
    assert_eq!(mac.names(), ["Home"]);
    assert_eq!(mac.settings.current().unwrap().active_server().map(|s| s.url.as_str()), Some("https://music.example"), "the first profile to come is the one in use");
    assert_eq!(mac.told.synced.lock().unwrap()[..], [ProfilesSynced { removed: vec![], active: ActiveProfile::Switched }]);
    assert_eq!(pixel.told.events.lock().unwrap()[..], [SyncEvent::NewDevice { name: "MacBook".into() }]);
    assert_eq!(pixel.syncing.devices().iter().map(|d| (d.name.as_str(), d.online)).collect::<Vec<_>>(), [("MacBook", true)]);
    mac.rename("Music");
    settle(&[&pixel, &mac]);
    assert_eq!(pixel.names(), ["Music"], "changed on the Mac, through the settings as a form keeps them");
    assert_eq!(pixel.told.synced.lock().unwrap().last(), Some(&ProfilesSynced { removed: vec![], active: ActiveProfile::Edited }));
    pixel.settings.put(StoredPrefs { servers: vec![], active_server_id: String::new(), ..pixel.settings.current().unwrap() });
    settle(&[&pixel, &mac]);
    let id = |d: &Device| d.told.synced.lock().unwrap().last().cloned();
    assert!(mac.names().is_empty(), "removed on every paired device");
    assert!(matches!(id(&mac), Some(ProfilesSynced { removed, active: ActiveProfile::Switched }) if removed.len() == 1));
}

#[test]
fn a_device_not_paired_or_with_sync_off_opens_nothing() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Phone, vec![navidrome("a", "https://music.example")]);
    pixel.syncing.foreground(true);
    pixel.syncing.playing(true);
    pixel.syncing.woke();
    pixel.syncing.profiles_changed.store(true, Ordering::Relaxed);
    pixel.rename("Music");
    assert_eq!((pixel.made.load(Ordering::Relaxed), pixel.net.opened.load(Ordering::Relaxed), pixel.syncing.state()), (0, 0, SyncState::Idle), "not paired: no executor, no socket");
    pixel.set_sync(false);
    pixel.syncing.pair_show("Pixel".into());
    assert_eq!(pixel.syncing.pairing(), PairingView::Refused { reason: PairingRefused::SyncOff });
    assert_eq!(pixel.made.load(Ordering::Relaxed), 0);
    pixel.set_sync(true);
    pixel.syncing.pair_cancel();
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    mac.set_sync(false);
    settle(&[&pixel, &mac]);
    assert_eq!((mac.syncing.state(), mac.net.open_sockets()), (SyncState::Idle, 0), "off closes every connection");
    assert!(mac.tasks.waits().is_empty(), "and leaves no timer");
    assert_eq!(Arc::strong_count(&mac.tasks), 2, "and no task: the executor is the test's and its maker's alone");
    assert!(crate::paired(&mac.settings), "and forgets nothing");
    assert!(!pixel.syncing.devices()[0].online);
    mac.set_sync(true);
    settle(&[&pixel, &mac]);
    assert_eq!((mac.syncing.state(), pixel.syncing.devices()[0].online), (SyncState::Connected, true), "on again, it is back in its group");
}

#[test]
fn a_dropped_connection_comes_back_quietly_and_a_refusal_is_a_state() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    mac.net.drop_all();
    settle(&[&pixel, &mac]);
    let SyncState::Offline { .. } = mac.syncing.state() else { panic!("offline: {:?}", mac.syncing.state()) };
    assert!(!pixel.syncing.devices()[0].online);
    assert!(mac.told.events.lock().unwrap().is_empty(), "a drop is no event");
    let told = mac.told.changed.load(Ordering::Relaxed);
    mac.tasks.pass(5 * S);
    settle(&[&pixel, &mac]);
    assert_eq!((mac.syncing.state(), pixel.syncing.devices()[0].online), (SyncState::Connected, true), "back within 5 s");
    assert!(mac.told.changed.load(Ordering::Relaxed) >= told + 2, "the client is told it connects, and that it is connected");
    hub.wall(true);
    mac.net.drop_all();
    settle(&[&pixel, &mac]);
    mac.tasks.pass(5 * S);
    settle(&[&pixel, &mac]);
    let SyncState::Refused { refused: Refusal::DailyWall, retry_ms } = mac.syncing.state() else { panic!("refused: {:?}", mac.syncing.state()) };
    let day = 24 * 60 * 60 * 1000;
    assert!(retry_ms > nori_time::unix_ms() && retry_ms % day <= 30 * 60 * 1000, "after midnight UTC");
    assert_eq!(mac.told.events.lock().unwrap()[..], [SyncEvent::Refused { reason: Refusal::DailyWall }]);
    let opened = mac.net.opened.load(Ordering::Relaxed);
    mac.tasks.pass(60 * 60 * S);
    assert_eq!(mac.net.opened.load(Ordering::Relaxed), opened, "and does not try again before then");
}

/// A screen hears each change it shows: this device's connection, and another device going and coming back.
#[test]
fn the_client_hears_when_the_state_or_who_is_online_changes() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    let heard = |d: &Device| d.told.changed.swap(0, Ordering::Relaxed) > 0;
    let (_, _) = (heard(&pixel), heard(&mac));
    mac.net.drop_all();
    settle(&[&pixel, &mac]);
    assert_eq!((heard(&mac), heard(&pixel)), (true, true), "the Mac went offline, and left the Pixel's group room");
    mac.tasks.pass(5 * S);
    settle(&[&pixel, &mac]);
    assert_eq!(mac.syncing.state(), SyncState::Connected);
    assert_eq!((heard(&mac), heard(&pixel)), (true, true), "and came back");
    settle(&[&pixel, &mac]);
    assert_eq!((heard(&mac), heard(&pixel)), (false, false), "nothing changed, nothing told");
}

#[test]
fn a_computer_keeps_its_connection_up_on_a_timer_of_its_own() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    assert_eq!(mac.tasks.waits(), [crate::link::PING_AFTER_US as u64].map(Duration::from_micros));
    mac.tasks.pass(50 * S);
    assert_eq!(mac.net.pings.load(Ordering::Relaxed), 1);
    mac.tasks.pass(50 * S);
    assert_eq!(mac.net.pings.load(Ordering::Relaxed), 2);
}

#[test]
fn a_phone_playing_with_the_screen_off_or_paused_arms_no_timer_of_its_own() {
    let hub = Hub::new(NOW);
    let mac = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let phone = Device::new(&hub, Reach::Phone, vec![]);
    phone.syncing.foreground(true);
    pair(&mac, &phone);
    assert_eq!(phone.syncing.state(), SyncState::Connected);
    phone.syncing.playing(true);
    phone.syncing.foreground(false);
    settle(&[&mac, &phone]);
    assert!(phone.tasks.waits().is_empty(), "playing with the screen off: {:?}", phone.tasks.waits());
    phone.tasks.pass(10 * 60 * S);
    assert_eq!(phone.net.pings.load(Ordering::Relaxed), 0, "nothing woke the phone");
    phone.syncing.woke();
    phone.syncing.woke();
    assert_eq!(phone.net.pings.load(Ordering::Relaxed), 1, "the audio's wake took the keep-alive along, once");
    phone.tasks.pass(10 * S);
    phone.syncing.woke();
    assert_eq!(phone.net.pings.load(Ordering::Relaxed), 1, "not due again yet");
    assert!(phone.tasks.waits().is_empty());
    phone.syncing.playing(false);
    settle(&[&mac, &phone]);
    assert_eq!(phone.tasks.waits(), [Duration::from_secs(15 * 60)], "paused in the background: only the time it lets go");
    phone.tasks.pass(15 * 60 * S);
    settle(&[&mac, &phone]);
    assert_eq!((phone.syncing.state(), phone.net.open_sockets()), (SyncState::Resting, 0), "let go after 15 minutes");
    assert!(phone.tasks.waits().is_empty() && !mac.syncing.devices()[0].online);
    phone.syncing.foreground(true);
    settle(&[&mac, &phone]);
    assert_eq!((phone.syncing.state(), mac.syncing.devices()[0].online), (SyncState::Connected, true), "opened, it connects again");
    phone.syncing.foreground(false);
    phone.net.drop_all();
    settle(&[&mac, &phone]);
    assert_eq!(phone.syncing.state(), SyncState::Resting, "paused in the background, a dropped connection is let go");
    assert!(phone.tasks.waits().is_empty());
    phone.syncing.playing(true);
    settle(&[&mac, &phone]);
    assert_eq!(phone.syncing.state(), SyncState::Connected, "it plays: connected again");
    phone.syncing.foreground(false);
    settle(&[&mac, &phone]);
    assert!(phone.tasks.waits().is_empty());
    phone.set_sync(false);
    settle(&[&mac, &phone]);
    assert_eq!(Arc::strong_count(&phone.tasks), 2, "sync off with no timer due, its task ends all the same");
}

#[test]
fn what_sync_brings_keeps_the_profile_in_use_unless_it_went() {
    let prefs = |active: &str| StoredPrefs { servers: vec![navidrome("a", "https://a.example"), navidrome("b", "https://b.example")], active_server_id: active.into(), ..StoredPrefs::default() };
    let (after, told) = synced(&prefs("b"), vec![navidrome("a", "https://a.example")]);
    assert_eq!((after.active_server_id.as_str(), told), ("a", ProfilesSynced { removed: vec!["b".into()], active: ActiveProfile::Switched }));
    let (after, told) = synced(&prefs("a"), vec![navidrome("a", "https://a2.example"), navidrome("b", "https://b.example")]);
    assert_eq!((after.active_server_id.as_str(), told.active), ("a", ActiveProfile::Edited));
    let (_, told) = synced(&prefs("a"), vec![navidrome("a", "https://a.example")]);
    assert_eq!(told, ProfilesSynced { removed: vec!["b".into()], active: ActiveProfile::Same });
    let (after, told) = synced(&prefs("a"), vec![]);
    assert_eq!((after.active_server_id.as_str(), told.active), ("", ActiveProfile::Switched), "none left");
}

/// What remote control heard in the group room.
#[derive(Default)]
struct RoomLog(Mutex<Vec<String>>);

impl RemoteRoom for RoomLog {
    fn in_room(&self, now: InRoom) {
        self.0.lock().unwrap().push(match now { InRoom::Entered => "in", InRoom::Renamed => "renamed", InRoom::Left => "out" }.into());
    }
    fn keyed(&self) {}
    fn changed(&self) {
        self.0.lock().unwrap().push("changed".into());
    }
    fn heard(&self, _: String, message: serde_json::Value) {
        self.0.lock().unwrap().push(format!("heard {message}"));
    }
    fn left(&self, _: String) {
        self.0.lock().unwrap().push("left".into());
    }
}

impl RoomLog {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

fn attached(d: &Device, local: &str) -> Arc<RoomLog> {
    let heard = Arc::new(RoomLog::default());
    d.syncing.remote_open(local.into(), heard.clone());
    heard
}

fn local(d: &Device, url: &str) -> String {
    d.settings.current().unwrap().servers.iter().find(|s| s.url == url).unwrap().id.clone()
}

#[test]
fn remote_control_reaches_the_devices_with_the_same_synced_profile_open() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example"), navidrome("b", "https://other.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    let on_pixel = attached(&pixel, "a");
    let on_mac = attached(&mac, &local(&mac, "https://music.example"));
    settle(&[&pixel, &mac]);
    assert_eq!((on_pixel.take(), on_mac.take()), (vec!["in".to_string()], vec!["in".to_string()]), "each is in the room once attached");
    assert!(pixel.syncing.server_clock().is_some(), "the server's clock is known once in the room");
    assert!(pixel.syncing.remote_say("a", None, serde_json::json!("hi")));
    let mac_id = mac.syncing.device().unwrap();
    assert!(mac.syncing.remote_say(&local(&mac, "https://music.example"), Some(pixel.syncing.device().unwrap()), serde_json::json!("back")));
    settle(&[&pixel, &mac]);
    assert_eq!((on_pixel.take(), on_mac.take()), (vec![r#"heard "back""#.to_string()], vec![r#"heard "hi""#.to_string()]));
    let other = attached(&mac, &local(&mac, "https://other.example"));
    assert!(pixel.syncing.remote_say("a", Some(mac_id), serde_json::json!("hi again")));
    settle(&[&pixel, &mac]);
    assert_eq!(other.take(), ["in"], "another profile open there hears nothing of this one's");
    pixel.syncing.reset();
    settle(&[&pixel, &mac]);
    assert_eq!((on_pixel.take(), other.take()), (vec!["out".to_string(), "in".to_string()], vec!["left".to_string()]), "a device that resets its pairing is in a room of its own");
}

#[test]
fn remote_control_leaves_the_room_with_its_connection_and_learns_the_clock_again() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    let on_mac = attached(&mac, &local(&mac, "https://music.example"));
    settle(&[&pixel, &mac]);
    assert_eq!(on_mac.take(), ["in"]);
    mac.net.drop_all();
    settle(&[&pixel, &mac]);
    assert_eq!((on_mac.take(), mac.syncing.server_clock()), (vec!["left".to_string(), "out".to_string()], None));
    mac.tasks.pass(5 * S);
    settle(&[&pixel, &mac]);
    assert_eq!(on_mac.take(), ["in"]);
    assert!(mac.syncing.server_clock().is_some());
}

#[test]
fn a_profile_that_does_not_sync_takes_no_part_in_the_group_room() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example"), SavedServer { sync_off: true, ..navidrome("c", "https://mine.example") }]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    let on_pixel = attached(&pixel, "c");
    settle(&[&pixel, &mac]);
    assert!(on_pixel.take().is_empty());
    assert!(!pixel.syncing.remote_say("c", None, serde_json::json!("hi")));
    assert_eq!(pixel.syncing.lan_secret("c"), None);
    attached(&pixel, "a");
    settle(&[&pixel, &mac]);
    assert_eq!(pixel.syncing.lan_secret("a").map(|s| s.len()), Some(64));
    let lone = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let heard = attached(&lone, "a");
    assert!(heard.take().is_empty() && !lone.syncing.remote_say("a", None, serde_json::json!("hi")), "a device never paired");
    assert_eq!((lone.made.load(Ordering::Relaxed), lone.net.opened.load(Ordering::Relaxed)), (0, 0), "and pays nothing");
}

/// What `from` says to `to` directly and back, as through `to`'s LAN door, until neither has more.
fn exchange(from: &Device, to: &Device) {
    let mut said = vec![from.syncing.direct_hello().expect("paired")];
    for side in [to, from].into_iter().cycle().take(6) {
        said = side.syncing.direct_heard(said);
        if said.is_empty() {
            return;
        }
    }
    panic!("the exchange never ended");
}

#[test]
fn profiles_sync_directly_when_the_server_refuses_both() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    hub.wall(true);
    pixel.net.drop_all();
    mac.net.drop_all();
    settle(&[&pixel, &mac]);
    let on_pixel = attached(&pixel, "a");
    mac.rename("Music");
    settle(&[&pixel, &mac]);
    assert_eq!(pixel.names(), ["Home"], "the server passes nothing on");
    exchange(&mac, &pixel);
    assert_eq!(pixel.names(), ["Music"], "the rename came directly");
    let mut prefs = pixel.settings.current().unwrap();
    prefs.servers.push(navidrome("w", "https://work.example"));
    pixel.settings.put(prefs);
    settle(&[&pixel, &mac]);
    assert!(on_pixel.take().contains(&"changed".to_string()), "the remote is told, to say it directly");
    exchange(&pixel, &mac);
    let mut names = mac.names();
    names.sort();
    assert_eq!(names, ["Home", "Music"], "and back: the new profile");
    assert_eq!(mac.told.events.lock().unwrap().iter().filter(|e| matches!(e, SyncEvent::NewDevice { .. })).count(), 0, "meeting directly is no new device");
}

#[test]
fn a_paired_device_s_sync_tells_its_client_nothing_before_the_client_has_it() {
    let hub = Hub::new(NOW);
    let pixel = Device::new(&hub, Reach::Computer, vec![navidrome("a", "https://music.example")]);
    let mac = Device::new(&hub, Reach::Computer, vec![]);
    pair(&pixel, &mac);
    let told = Arc::new(Told::default());
    let tasks = pixel.tasks.clone();
    let again = Syncing::new(pixel.settings.clone(), Net::new(&hub, None), told.clone(), Reach::Computer, Box::new(move || tasks.clone() as Arc<dyn Tasks>));
    assert_eq!(told.changed.load(Ordering::Relaxed), 0, "nothing while it is made");
    assert_eq!(again.state(), SyncState::Connecting, "and it starts at once");
}
