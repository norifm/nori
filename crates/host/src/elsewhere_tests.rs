//! A session showing a device of mine that plays elsewhere: what its player and the system's media
//! controls show of it.

use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use nori_core::remote::{Announcement, Discovery, Playing, Remote, RemoteMe, RemotePlayer, RemoteShown};
use nori_core::settings::ServerKind;
use nori_core::settings_store::Settings;
use nori_engine::testing::Virtual;
use nori_remote::wire::DeviceKind;

use super::test_card::Card;
use super::*;
use crate::player::Playback;

/// The test plugin's provider: a profile without a server, whose devices find each other on the LAN.
fn plugin() -> ServerKind {
    ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }
}

/// The kinds a device opens, with the test plugin installed under `dir`.
fn kinds(dir: &Path, settings: Arc<Settings>) -> Arc<nori_core::providers::Kinds> {
    let plugins = nori_plugins::open(dir.join("plugins"), settings, Arc::new(nori_plugins::testing::Quiet));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    nori_core::providers::Kinds::with_plugins(plugins)
}

/// A device's mDNS: what it last announced.
#[derive(Default)]
struct Announced(Mutex<Option<Announcement>>);

impl Discovery for Announced {
    fn announce(&self, door: Option<Announcement>) {
        *self.0.lock().unwrap() = door;
    }
    fn browse(&self, _: bool) {}
}

struct Quiet;

impl RemotePlayer for Quiet {
    fn apply(&self, _: Op) {}
}

impl RemoteShown for Quiet {
    fn changed(&self) {}
    fn marks_changed(&self) {}
    fn jam_ended(&self, _: Option<String>) {}
}

/// The phone, whose settings `settings` keep, of the group `sync` is in: playing song `a` of its queue,
/// controllable from my devices nearby; its door.
fn phone(dir: &Path, settings: Arc<Settings>, sync: Arc<nori_core::sync::Syncing>) -> (Arc<Remote>, Announcement) {
    let path = db_path(dir);
    let profile = settings.with_prefs(|p| p.servers[0].clone()).unwrap();
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings.clone()))).unwrap();
    core.use_kinds(kinds(dir, settings));
    core.configure_profile(profile.clone()).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    client.set_profile(nori_core::net_profile(&profile));
    client.session().register(vec![Song { id: "a".into(), title: "A".into(), duration: 200, ..Default::default() }]);
    client.session().set(vec!["a".into()], Some(0), false, None);
    let door = Arc::new(Announced::default());
    let phone = Remote::new(client, RemoteMe { name: "Phone".into(), kind: DeviceKind::Phone }, Arc::new(Quiet), Arc::new(Quiet), Some(door.clone()), Some(sync));
    phone.clone().serve(true);
    phone.clone().played(Playing { playing: true, index: Some(0), ..Default::default() });
    let announced = door.0.lock().unwrap().clone().expect("the phone's door is announced");
    (phone, announced)
}

/// An idle Mac in sight finds the phone playing and its player shows it; the system's media controls
/// (Now Playing, MPRIS) show nothing playing here. They speak for this device: macOS tells a Bluetooth
/// headset joined to both that the Mac plays, and the headset's pause and play for the Mac (sharing
/// itself between two sources) came back through them as the phone's pause and play.
#[test]
fn a_device_playing_elsewhere_is_not_shown_to_the_system_as_playing_here() {
    let (phone_dir, mac_dir) = (nori_testdir::TempDir::new("host-elsewhere-phone"), nori_testdir::TempDir::new("host-elsewhere-mac"));
    let phone_settings = Settings::new();
    let mut p = phone_settings.open(&db_path(phone_dir.path())).unwrap();
    (p.sync, p.servers, p.active_server_id) = (true, vec![SavedServer { id: "phone".into(), kind: plugin(), ..Default::default() }], "phone".into());
    phone_settings.put(p);
    let settings = Settings::new();
    let mut p = settings.open(&db_path(mac_dir.path())).unwrap();
    (p.remote_control, p.auto_fill, p.third_party_lookups, p.sync) = (true, None, false, true);
    settings.put(p);
    // One group, the Mac taking the phone's profile as its own.
    let (hub, sim) = (nori_sync::hub::Hub::real_time(), Arc::new(nori_tasks::Native::new("host-elsewhere-sync").unwrap()));
    let syncs = nori_sync::hub::grouped(&hub, &sim, &[&phone_settings, &settings]);
    let (phone, door) = phone(phone_dir.path(), phone_settings, syncs[0].clone());
    let queue = Arc::new(nori_core::queue::Session::new(settings.clone()));
    let (said, heard) = mpsc::channel();
    let said = Mutex::new(said);
    let out: Out = Arc::new(move |s| {
        if matches!(s, Said::Remote) {
            let _ = said.lock().unwrap().send(());
        }
    });
    let profile = settings.with_prefs(|p| p.servers[0].clone()).unwrap();
    let open = Open { kinds: kinds(mac_dir.path(), settings), discovery: Some(Arc::new(Announced::default())), out, sync: Some(syncs[1].clone()), ..test_open(queue, mac_dir.path(), profile, Box::new(Card::new())) };
    let mac = Session::open_on(open, Virtual::default()).unwrap();
    let remote = mac.remote().expect("remote control is on");
    mac.in_sight(true);
    remote.lan_found("phone".into(), vec!["127.0.0.1".into()], door.port, door.txt);
    let until = Instant::now() + Duration::from_secs(5);
    while !matches!(mac.playback(), Playback::There(_)) {
        heard.recv_timeout(until.saturating_duration_since(Instant::now())).expect("the Mac finds the phone playing");
    }

    let shown = mac.now();
    assert_eq!((shown.state, shown.device.as_deref()), (State::Playing, Some("Phone")), "the player shows the phone playing");
    assert_eq!(nori_mpris::Controls::now(&*mac), nori_mpris::Now::default(), "the system is shown nothing playing here");
    mac.close();
    phone.stop();
}
