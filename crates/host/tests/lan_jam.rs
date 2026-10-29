//! A native LAN jam uses the same room rules and temporary profiles as a relayed jam.

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::providers::StreamParts;
use nori_core::remote::{self, JamJoin, Remote, RemoteMe, RemotePlayer, RemoteShown};
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, NetError};
use nori_core::{Core, Song};
use nori_remote::wire::{DeviceKind, Op};

struct Player(mpsc::Sender<Op>);
impl RemotePlayer for Player {
    fn apply(&self, op: Op) { let _ = self.0.send(op); }
}
struct Shown(mpsc::Sender<()>, Mutex<bool>);
impl RemoteShown for Shown {
    fn changed(&self) { let _ = self.0.send(()); }
    fn marks_changed(&self) {}
    fn jam_ended(&self, _: Option<String>) { *self.1.lock().unwrap() = true; let _ = self.0.send(()); }
}
fn wait(changed: &mpsc::Receiver<()>, ready: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !ready() { changed.recv_timeout(until.saturating_duration_since(Instant::now())).expect("LAN state changed before deadline"); }
}
/// A provider without a Subsonic server, whose jams are hosted on the LAN: the test plugin's.
fn music_plugin() -> ServerKind {
    ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }
}
struct Quiet;
impl nori_plugins::Client for Quiet {
    fn layer_changed(&self, _: u32) {}
    fn devices_changed(&self) {}
    fn status(&self, _: String, _: String, _: String, _: Vec<String>) {}
    fn shown(&self, _: String, _: String, _: bool, _: bool) {}
    fn notice(&self, _: String, _: bool, _: String, _: Vec<String>) {}
    fn browse(&self, _: String, _: bool) {}
    fn multicast(&self, _: bool) {}
    fn cast(&self, _: Option<nori_plugins::Cast>) {}
}
/// The kinds a device opens: nori's own and the test plugin's, installed under `dir`.
fn kinds(dir: &std::path::Path, settings: Arc<Settings>) -> Arc<nori_core::providers::Kinds> {
    let plugins = nori_plugins::open(dir.join("plugins"), settings, Arc::new(Quiet));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    nori_core::providers::Kinds::with_plugins(plugins)
}
/// A device's client of `profile`, with the test plugin.
fn client(path: String, profile: SavedServer, settings: Arc<Settings>) -> Arc<Client> {
    let dir = std::path::Path::new(&path).parent().unwrap().join(format!("device-{}", profile.id));
    client_with(kinds(&dir, settings.clone()), path, profile, settings)
}
fn client_with(kinds: Arc<nori_core::providers::Kinds>, path: String, profile: SavedServer, settings: Arc<Settings>) -> Arc<Client> {
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings.clone()))).unwrap();
    core.use_kinds(kinds);
    core.configure_profile(profile.clone()).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    client.set_profile(nori_core::net_profile(&profile));
    client
}

/// A guest without the host's plugin plays the host's provider through the host: it searches there,
/// asks for a song, and streams the songs the jam plays from the URLs the host's plugin gives.
#[test]
fn guests_request_over_lan_and_leave_without_an_account() {
    let host_dir = nori_testdir::TempDir::new("lan-jam-host");
    let guest_dir = nori_testdir::TempDir::new("lan-jam-guest");
    let host_settings = Settings::new();
    let host_path = nori_host::db_path(host_dir.path());
    host_settings.open(&host_path).unwrap();
    let profile = SavedServer { id: "host".into(), kind: music_plugin(), user: "private-account".into(), password: "private-cookie".into(), ..Default::default() };
    let host_client = client(host_path, profile, host_settings.clone());
    let host_session = host_client.session().clone();
    let (ops, received) = mpsc::channel();
    let (changes, host_changed) = mpsc::channel();
    let host = Remote::new(host_client.clone(), RemoteMe { name: "Host".into(), kind: DeviceKind::Desktop }, Arc::new(Player(ops)), Arc::new(Shown(changes, Mutex::new(false))), None);
    let pairing = host.pairing_link().unwrap();
    let link = block_on(host.clone().jam_open()).unwrap().unwrap();
    assert!(!link.contains("private-account") && !link.contains("private-cookie"));
    assert!(!pairing.contains("private-account") && !pairing.contains("private-cookie"));
    host.clone().serve(true);
    host.clone().serve(false);
    host.clone().serve(true);
    let (server, invite) = nori_remote::parse_invite(&link).unwrap();
    let port = server.rsplit(':').next().unwrap();
    let link = nori_remote::invite_link(&format!("http://127.0.0.1:{port}"), &invite);
    let guest_settings = Settings::new();
    let guest_path = nori_host::db_path(guest_dir.path());
    guest_settings.open(&guest_path).unwrap();
    let joined = block_on(remote::jam_join(nori_http::Http::new(), guest_settings.clone(), None, link.clone(), "Guest".into())).unwrap();
    let JamJoin::Joined { pass } = joined else { panic!("guest joined: {joined:?}") };
    let guest_profile = remote::jam_profile(pass, "guest".into(), "Jam".into());
    assert_eq!(guest_profile.kind, music_plugin(), "the songs are the host's provider's");
    assert!(guest_profile.user.is_empty() && guest_profile.password.is_empty());
    let guest_client = client_with(nori_core::providers::Kinds::builtin(), guest_path, guest_profile, guest_settings.clone());
    assert_eq!(guest_client.core().kind(), music_plugin());
    let (ops, _) = mpsc::channel();
    let (changes, guest_changed) = mpsc::channel();
    let guest_shown = Arc::new(Shown(changes, Mutex::new(false)));
    let guest = Remote::new(guest_client.clone(), RemoteMe { name: "Guest".into(), kind: DeviceKind::Guest }, Arc::new(Player(ops)), guest_shown.clone(), None);
    guest.clone().watch(true);
    wait(&guest_changed, || guest.jam_view().is_some_and(|v| v.members.len() == 2));
    guest.clone().jam_act(Op::Pause);
    wait(&guest_changed, || guest.jam_view().is_some_and(|v| v.refused.is_some()));
    assert!(received.try_recv().is_err(), "guest cannot pause the host");
    let found = block_on(guest_client.read_now(Read::Search { query: "blue".into(), songs: 10, albums: 0, artists: 0 })).unwrap();
    let Page::Found { v } = found else { panic!("a search page: {found:?}") };
    let song: Song = v.songs.into_iter().next().expect("the host's plugin found it");
    assert_eq!((song.id.as_str(), song.title.as_str()), ("test-blue", "blue"));
    assert!(!song.starred, "the host's favorites are its own");
    let target = guest_client.resolve_now(&song.id);
    assert_eq!(target.key, host_client.resolve_now(&song.id).key, "its bytes are kept as the host's source keeps them");
    let resolver = guest_client.stream_resolver().expect("the guest resolves through the host");
    assert!(matches!(resolver.resolve(&target.url), Some(Err(NetError::Http { status: 403 }))), "a song the jam does not play has no source for guests");
    guest.clone().jam_act(Op::Request { song: song.clone() });
    wait(&host_changed, || host.jam_view().is_some_and(|v| v.pending.len() == 1));
    let request = host.jam_view().unwrap().pending[0].request;
    host.clone().jam_act(Op::Decide { request, accept: true });
    let Op::Add { songs, .. } = received.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!("the host adds the song") };
    assert_eq!(songs, std::slice::from_ref(&song));
    host_session.register(songs);
    host_session.set(vec![song.id.clone()], Some(0), false, None);
    let url = resolver.resolve(&target.url).expect("a song of the host's").unwrap();
    assert_eq!(url, "https://allowed.example/song/test-blue", "the URL the host's plugin gives");
    assert_eq!(resolver.parts(&target.url), Some(StreamParts { len: 1000, most: 100 }));
    assert!(host.clone().reset_pairing());
    assert_ne!(host.pairing_link(), Some(pairing));
    assert!(host.jam_view().is_some(), "forgetting paired devices preserves a separate jam");
    host.clone().jam_close();
    wait(&guest_changed, || *guest_shown.1.lock().unwrap());
    assert!(matches!(block_on(remote::jam_join(nori_http::Http::new(), guest_settings, None, link, "Later".into())).unwrap(), JamJoin::Ended));
    guest.stop();
    host.stop();
}

#[derive(Default)]
struct Discovery(Mutex<Option<remote::Announcement>>);
impl remote::Discovery for Discovery {
    fn announce(&self, door: Option<remote::Announcement>) { *self.0.lock().unwrap() = door; }
    fn browse(&self, _: bool) {}
}

#[test]
fn paired_devices_control_each_other_and_can_be_forgotten() {
    let a_dir = nori_testdir::TempDir::new("lan-pair-a");
    let b_dir = nori_testdir::TempDir::new("lan-pair-b");
    let make = |dir: &nori_testdir::TempDir, id: &str| {
        let settings = Settings::new();
        let path = nori_host::db_path(dir.path());
        settings.open(&path).unwrap();
        client(path, SavedServer { id: id.into(), kind: music_plugin(), ..Default::default() }, settings)
    };
    let (a_ops, a_commands) = mpsc::channel();
    let (a_changed, _) = mpsc::channel();
    let discovery = Arc::new(Discovery::default());
    let a = Remote::new(make(&a_dir, "a"), RemoteMe { name: "A".into(), kind: DeviceKind::Desktop }, Arc::new(Player(a_ops)), Arc::new(Shown(a_changed, Mutex::new(false))), Some(discovery.clone()));
    let (b_ops, _) = mpsc::channel();
    let (b_changes, b_changed) = mpsc::channel();
    let b = Remote::new(make(&b_dir, "b"), RemoteMe { name: "B".into(), kind: DeviceKind::Terminal }, Arc::new(Player(b_ops)), Arc::new(Shown(b_changes, Mutex::new(false))), None);
    let pairing = a.pairing_link().unwrap();
    assert_ne!(b.pairing_link(), Some(pairing.clone()));
    assert!(b.clone().pair(pairing.clone()));
    a.clone().serve(true);
    let announced = discovery.0.lock().unwrap().clone().unwrap();
    assert!(announced.txt.iter().all(|param| !param.value.contains(&nori_remote::lan::parse_pairing(&pairing).unwrap())), "discovery does not contain the pairing credential");
    b.clone().watch(true);
    b.clone().lan_found("host".into(), "127.0.0.1".into(), announced.port, announced.txt);
    wait(&b_changed, || b.devices().len() == 1);
    b.clone().send(a.id(), Op::Pause);
    assert!(matches!(a_commands.recv_timeout(Duration::from_secs(5)).unwrap(), Op::Pause));
    let secret = nori_remote::lan::parse_pairing(&pairing).unwrap();
    assert!(a.clone().reset_pairing());
    let url = format!("http://127.0.0.1:{}{}", announced.port, nori_remote::lan::signed(&secret, "GET", "/rest/noriRemote.poll?dev=old", b"", nori_time::unix_ms()));
    assert_eq!(block_on(nori_core::transport::Transport::get(&*nori_http::Http::new(), url, 1_000)).unwrap().status, 401, "the old pairing is refused");
    b.stop();
    a.stop();
}

/// One device's place on a [`Lan`].
#[derive(Default)]
struct Seat {
    remote: std::sync::Weak<Remote>,
    door: Option<remote::Announcement>,
    browsing: bool,
}

/// A network as mDNS makes it: a door announced reaches every device browsing, and a device that starts
/// browsing finds every door already there.
#[derive(Default)]
struct Lan(Mutex<Vec<Seat>>);

/// Device `1`'s discovery on the network `0`.
struct Port(Arc<Lan>, usize);

impl remote::Discovery for Port {
    fn announce(&self, door: Option<remote::Announcement>) {
        let browsing: Vec<_> = {
            let mut seats = self.0 .0.lock().unwrap();
            seats[self.1].door = door.clone();
            seats.iter().enumerate().filter(|(i, s)| *i != self.1 && s.browsing).filter_map(|(_, s)| s.remote.upgrade()).collect()
        };
        if let Some(door) = door {
            for remote in browsing {
                remote.lan_found(door.name.clone(), "127.0.0.1".into(), door.port, door.txt.clone());
            }
        }
    }

    fn browse(&self, on: bool) {
        let (me, doors): (_, Vec<_>) = {
            let mut seats = self.0 .0.lock().unwrap();
            seats[self.1].browsing = on;
            let doors = seats.iter().enumerate().filter(|(i, _)| *i != self.1).filter_map(|(_, s)| s.door.clone()).collect();
            (seats[self.1].remote.upgrade(), doors)
        };
        if let (Some(me), true) = (me, on) {
            for door in doors {
                me.clone().lan_found(door.name, "127.0.0.1".into(), door.port, door.txt);
            }
        }
    }
}

#[test]
fn scanning_a_code_pairs_both_devices() {
    // Either device may show its code to the other.
    for scanner in [1, 0] {
        let dirs = [nori_testdir::TempDir::new("lan-scan-a"), nori_testdir::TempDir::new("lan-scan-b")];
        let lan = Arc::new(Lan::default());
        let mut devices: Vec<_> = [("Desk", DeviceKind::Desktop), ("Phone", DeviceKind::Phone)]
            .into_iter()
            .enumerate()
            .map(|(i, (name, kind))| {
                let settings = Settings::new();
                let path = nori_host::db_path(dirs[i].path());
                settings.open(&path).unwrap();
                let client = client(path, SavedServer { id: name.into(), kind: music_plugin(), ..Default::default() }, settings);
                lan.0.lock().unwrap().push(Seat::default());
                let (ops, _) = mpsc::channel();
                let (changes, changed) = mpsc::channel();
                let remote = Remote::new(client, RemoteMe { name: name.into(), kind }, Arc::new(Player(ops)), Arc::new(Shown(changes, Mutex::new(false))), Some(Arc::new(Port(lan.clone(), i))));
                lan.0.lock().unwrap()[i].remote = Arc::downgrade(&remote);
                (remote, changed)
            })
            .collect();
        for (device, _) in &devices {
            device.clone().serve(true);
            device.clone().watch(true);
        }
        assert!(devices.iter().all(|(d, _)| d.devices().is_empty()), "not paired: neither lists the other");
        let (shower, scanning) = (devices.remove(1 - scanner), devices.remove(0));
        assert!(scanning.0.clone().pair(shower.0.pairing_link().unwrap()));
        for ((me, changed), other) in [(&scanning, &shower.0), (&shower, &scanning.0)] {
            wait(changed, || me.devices().iter().any(|d| d.id == other.id()));
        }
        scanning.0.stop();
        shower.0.stop();
    }
}
