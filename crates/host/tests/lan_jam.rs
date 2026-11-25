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
/// The kinds a device opens: nori's own and the test plugin's, installed under `dir`.
fn kinds(dir: &std::path::Path, settings: Arc<Settings>) -> Arc<nori_core::providers::Kinds> {
    let plugins = nori_plugins::open(dir.join("plugins"), settings, Arc::new(nori_plugins::testing::Quiet));
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
    let host = Remote::new(host_client.clone(), RemoteMe { name: "Host".into(), kind: DeviceKind::Desktop }, Arc::new(Player(ops)), Arc::new(Shown(changes, Mutex::new(false))), None, None);
    let link = block_on(host.clone().jam_open()).unwrap().unwrap();
    assert!(!link.contains("private-account") && !link.contains("private-cookie"));
    host.clone().serve(true);
    host.clone().serve(false);
    host.clone().serve(true);
    let (servers, invite) = nori_remote::parse_invite(&link).unwrap();
    let port = servers[0].rsplit(':').next().unwrap();
    // The host on two networks: the guest joins at the address that answers, and keeps it.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let here = format!("http://127.0.0.1:{port}");
    let link = nori_remote::app_invite_link(&[format!("http://{closed}"), here.clone()], &invite);
    let guest_settings = Settings::new();
    let guest_path = nori_host::db_path(guest_dir.path());
    guest_settings.open(&guest_path).unwrap();
    let joined = block_on(remote::jam_join(nori_http::Http::new(), guest_settings.clone(), None, link.clone(), "Guest".into())).unwrap();
    let JamJoin::Joined { pass } = joined else { panic!("guest joined: {joined:?}") };
    assert_eq!(pass.url, here);
    let guest_profile = remote::jam_profile(pass, "guest".into(), "Jam".into());
    assert_eq!(guest_profile.kind, music_plugin(), "the songs are the host's provider's");
    assert!(guest_profile.user.is_empty() && guest_profile.password.is_empty());
    let guest_client = client_with(nori_core::providers::Kinds::builtin(), guest_path, guest_profile, guest_settings.clone());
    assert_eq!(guest_client.core().kind(), music_plugin());
    let (ops, _) = mpsc::channel();
    let (changes, guest_changed) = mpsc::channel();
    let guest_shown = Arc::new(Shown(changes, Mutex::new(false)));
    let guest = Remote::new(guest_client.clone(), RemoteMe { name: "Guest".into(), kind: DeviceKind::Guest }, Arc::new(Player(ops)), guest_shown.clone(), None, None);
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
    host.clone().jam_close();
    wait(&guest_changed, || *guest_shown.1.lock().unwrap());
    assert!(matches!(block_on(remote::jam_join(nori_http::Http::new(), guest_settings, None, link, "Later".into())).unwrap(), JamJoin::Ended));
    guest.stop();
    host.stop();
}

/// Two devices of one group, each with a profile of the test plugin's that is one synced profile, whose
/// remotes run on the network `lan` gives them (none: they are not found); their remotes, what each player
/// was told, and when each changed. Their sync runs on `sim`, through `hub`.
struct Group {
    dirs: [nori_testdir::TempDir; 2],
    remotes: Vec<Arc<Remote>>,
    ops: Vec<mpsc::Receiver<Op>>,
    changed: Vec<mpsc::Receiver<()>>,
    _syncs: Vec<Arc<nori_core::sync::Syncing>>,
}

impl Group {
    fn new(names: [(&str, DeviceKind); 2], lan: impl Fn(usize) -> Option<Arc<dyn remote::Discovery>>) -> Group {
        let (hub, sim) = (nori_sync::hub::Hub::real_time(), Arc::new(nori_tasks::Native::new("lan-group-sync").unwrap()));
        let dirs = [nori_testdir::TempDir::new("lan-group-a"), nori_testdir::TempDir::new("lan-group-b")];
        let settings: Vec<Arc<Settings>> = dirs.iter().enumerate().map(|(i, dir)| {
            let settings = Settings::new();
            let mut prefs = settings.open(&nori_host::db_path(dir.path())).unwrap();
            prefs.sync = true;
            if i == 0 {
                let profile = SavedServer { id: "first".into(), kind: music_plugin(), ..Default::default() };
                (prefs.servers, prefs.active_server_id) = (vec![profile], "first".into());
            }
            settings.put(prefs);
            settings
        }).collect();
        let syncs = nori_sync::hub::grouped(&hub, &sim, &[&settings[0], &settings[1]]);
        let (mut remotes, mut ops, mut changed) = (Vec::new(), Vec::new(), Vec::new());
        for (i, (name, kind)) in names.into_iter().enumerate() {
            let profile = settings[i].with_prefs(|p| p.servers[0].clone()).unwrap();
            settings[i].change(|p| nori_core::settings::StoredPrefs { active_server_id: profile.id.clone(), ..p.clone() });
            let client = client(nori_host::db_path(dirs[i].path()), profile, settings[i].clone());
            let (op, heard) = mpsc::channel();
            let (change, told) = mpsc::channel();
            remotes.push(Remote::new(client, RemoteMe { name: name.into(), kind }, Arc::new(Player(op)), Arc::new(Shown(change, Mutex::new(false))), lan(i), Some(syncs[i].clone())));
            ops.push(heard);
            changed.push(told);
        }
        Group { dirs, remotes, ops, changed, _syncs: syncs }
    }

    fn stop(self) {
        self.remotes.iter().for_each(|r| r.clone().stop());
        drop(self.dirs);
    }
}

#[derive(Default)]
struct Discovery(Mutex<Option<remote::Announcement>>);
impl remote::Discovery for Discovery {
    fn announce(&self, door: Option<remote::Announcement>) { *self.0.lock().unwrap() = door; }
    fn browse(&self, _: bool) {}
}

#[test]
fn a_speaker_reached_through_a_device_is_cast_to_there() {
    let discovery = Arc::new(Discovery::default());
    let announced_by = discovery.clone();
    let group = Group::new([("A", DeviceKind::Desktop), ("B", DeviceKind::Terminal)], move |i| (i == 0).then(|| announced_by.clone() as Arc<dyn remote::Discovery>));
    let (a, b) = (&group.remotes[0], &group.remotes[1]);
    a.clone().serve(true);
    let announced = discovery.0.lock().unwrap().clone().unwrap();
    b.clone().watch(true);
    b.clone().lan_found("host".into(), vec!["127.0.0.1".into()], announced.port, announced.txt);
    wait(&group.changed[1], || b.devices().iter().any(|d| d.nearby));
    assert!(b.outputs().is_empty());
    let speaker = nori_remote::wire::CastOutput { plugin: "p".into(), id: "kitchen".into(), name: "Kitchen".into(), look: Default::default() };
    a.clone().outputs_changed(vec![speaker.clone()]);
    wait(&group.changed[1], || !b.outputs().is_empty());
    let reached = b.outputs().remove(0);
    assert_eq!((reached.via.as_str(), &reached.output), ("A", &speaker));
    b.clone().cast_on(reached);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match group.ops[0].recv_timeout(until.saturating_duration_since(Instant::now())).expect("the cast asked for") {
            Op::CastTo { output } => break assert_eq!(output, speaker),
            _ => continue,
        }
    }
    group.stop();
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
                remote.lan_found(door.name.clone(), vec!["127.0.0.1".into()], door.port, door.txt.clone());
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
                me.clone().lan_found(door.name, vec!["127.0.0.1".into()], door.port, door.txt);
            }
        }
    }
}

/// My devices on one network find each other's doors, and only by their group's key: a device of no
/// group is not listed.
#[test]
fn my_devices_on_one_network_find_each_others_doors() {
    let lan = Arc::new(Lan::default());
    let seats = lan.clone();
    let group = Group::new([("Desk", DeviceKind::Desktop), ("Phone", DeviceKind::Phone)], move |i| {
        seats.0.lock().unwrap().push(Seat::default());
        Some(Arc::new(Port(seats.clone(), i)) as Arc<dyn remote::Discovery>)
    });
    for (i, r) in group.remotes.iter().enumerate() {
        lan.0.lock().unwrap()[i].remote = Arc::downgrade(r);
    }
    let stranger_dir = nori_testdir::TempDir::new("lan-stranger");
    let settings = Settings::new();
    let path = nori_host::db_path(stranger_dir.path());
    settings.open(&path).unwrap();
    lan.0.lock().unwrap().push(Seat::default());
    let (ops, _) = mpsc::channel();
    let (changes, _) = mpsc::channel();
    let stranger = Remote::new(client(path, SavedServer { id: "s".into(), kind: music_plugin(), ..Default::default() }, settings), RemoteMe { name: "Stranger".into(), kind: DeviceKind::Phone }, Arc::new(Player(ops)), Arc::new(Shown(changes, Mutex::new(false))), Some(Arc::new(Port(lan.clone(), 2))), None);
    lan.0.lock().unwrap()[2].remote = Arc::downgrade(&stranger);
    stranger.clone().serve(true);
    for r in &group.remotes {
        r.clone().serve(true);
        r.clone().watch(true);
    }
    for (me, other) in [(0, 1), (1, 0)] {
        let other = group.remotes[other].id();
        wait(&group.changed[me], || group.remotes[me].devices().iter().any(|d| d.id == other && d.nearby));
    }
    assert!(group.remotes.iter().all(|r| r.devices().iter().all(|d| d.id != stranger.id())), "a device of no group is no device of mine");
    stranger.stop();
    group.stop();
}
