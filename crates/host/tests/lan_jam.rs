//! A native LAN jam uses the same room rules and temporary profiles as a relayed jam.

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use nori_core::client::Client;
use nori_core::remote::{self, JamJoin, Remote, RemoteMe, RemotePlayer, RemoteShown};
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::block_on;
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
fn client(path: String, profile: SavedServer, settings: Arc<Settings>) -> Arc<Client> {
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    core.configure_profile(profile.clone()).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    client.set_profile(nori_core::net_profile(&profile));
    client
}

#[test]
fn guests_request_over_lan_and_leave_without_an_account() {
    let host_dir = nori_testdir::TempDir::new("lan-jam-host");
    let guest_dir = nori_testdir::TempDir::new("lan-jam-guest");
    let host_settings = Settings::new();
    let host_path = nori_host::db_path(host_dir.path());
    host_settings.open(&host_path).unwrap();
    let profile = SavedServer { id: "host".into(), kind: ServerKind::YouTubeMusic, user: "private-account".into(), password: "private-cookie".into(), ..Default::default() };
    let host_client = client(host_path, profile, host_settings.clone());
    let (ops, received) = mpsc::channel();
    let (changes, host_changed) = mpsc::channel();
    let host = Remote::new(host_client, RemoteMe { name: "Host".into(), kind: DeviceKind::Desktop }, Arc::new(Player(ops)), Arc::new(Shown(changes, Mutex::new(false))), None);
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
    let JamJoin::Joined { pass } = block_on(remote::jam_join(nori_http::Http::new(), guest_settings.clone(), None, link.clone(), "Guest".into())).unwrap() else { panic!("guest joined") };
    let guest_profile = remote::jam_profile(pass, "guest".into(), "Jam".into());
    assert_eq!(guest_profile.kind, ServerKind::YouTubeMusic);
    assert!(guest_profile.user.is_empty() && guest_profile.password.is_empty());
    let guest_client = client(guest_path, guest_profile, guest_settings.clone());
    let (ops, _) = mpsc::channel();
    let (changes, guest_changed) = mpsc::channel();
    let guest_shown = Arc::new(Shown(changes, Mutex::new(false)));
    let guest = Remote::new(guest_client, RemoteMe { name: "Guest".into(), kind: DeviceKind::Guest }, Arc::new(Player(ops)), guest_shown.clone(), None);
    guest.clone().watch(true);
    wait(&guest_changed, || guest.jam_view().is_some_and(|v| v.members.len() == 2));
    guest.clone().jam_act(Op::Pause);
    wait(&guest_changed, || guest.jam_view().is_some_and(|v| v.refused.is_some()));
    assert!(received.try_recv().is_err(), "guest cannot pause the host");
    guest.clone().jam_act(Op::Request { song: Song { id: "yt-song".into(), title: "A song".into(), ..Default::default() } });
    wait(&host_changed, || host.jam_view().is_some_and(|v| v.pending.len() == 1));
    let request = host.jam_view().unwrap().pending[0].request;
    host.clone().jam_act(Op::Decide { request, accept: true });
    assert!(matches!(received.recv_timeout(Duration::from_secs(5)).unwrap(), Op::Add { songs, .. } if songs[0].id == "yt-song"));
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
fn paired_youtube_devices_control_each_other_and_can_be_forgotten() {
    let a_dir = nori_testdir::TempDir::new("lan-pair-a");
    let b_dir = nori_testdir::TempDir::new("lan-pair-b");
    let make = |dir: &nori_testdir::TempDir, id: &str| {
        let settings = Settings::new();
        let path = nori_host::db_path(dir.path());
        settings.open(&path).unwrap();
        client(path, SavedServer { id: id.into(), kind: ServerKind::YouTubeMusic, ..Default::default() }, settings)
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

/// A network of one door: browsing finds it as it starts, as NSD and mdns-sd report what is already there.
/// Counts the times browsing started.
struct Network(Mutex<Option<std::sync::Weak<Remote>>>, remote::Announcement, std::sync::atomic::AtomicUsize);
impl remote::Discovery for Network {
    fn announce(&self, _: Option<remote::Announcement>) {}
    fn browse(&self, on: bool) {
        let Some(remote) = self.0.lock().unwrap().as_ref().and_then(std::sync::Weak::upgrade).filter(|_| on) else { return };
        self.2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let door = self.1.clone();
        remote.lan_found(door.name, "127.0.0.1".into(), door.port, door.txt);
    }
}

#[test]
fn a_device_paired_by_scanning_sees_the_one_that_showed_the_code() {
    let a_dir = nori_testdir::TempDir::new("lan-scan-a");
    let b_dir = nori_testdir::TempDir::new("lan-scan-b");
    let make = |dir: &nori_testdir::TempDir, id: &str| {
        let settings = Settings::new();
        let path = nori_host::db_path(dir.path());
        settings.open(&path).unwrap();
        client(path, SavedServer { id: id.into(), kind: ServerKind::YouTubeMusic, ..Default::default() }, settings)
    };
    let (a_ops, _) = mpsc::channel();
    let (a_changed, _) = mpsc::channel();
    let a_discovery = Arc::new(Discovery::default());
    let a = Remote::new(make(&a_dir, "a"), RemoteMe { name: "Desk".into(), kind: DeviceKind::Desktop }, Arc::new(Player(a_ops)), Arc::new(Shown(a_changed, Mutex::new(false))), Some(a_discovery.clone()));
    a.clone().serve(true);
    let door = a_discovery.0.lock().unwrap().clone().unwrap();
    let network = Arc::new(Network(Mutex::new(None), door, Default::default()));
    let (b_ops, _) = mpsc::channel();
    let (b_changes, b_changed) = mpsc::channel();
    let b = Remote::new(make(&b_dir, "b"), RemoteMe { name: "Phone".into(), kind: DeviceKind::Phone }, Arc::new(Player(b_ops)), Arc::new(Shown(b_changes, Mutex::new(false))), Some(network.clone()));
    *network.0.lock().unwrap() = Some(Arc::downgrade(&b));
    b.clone().watch(true);
    assert_eq!(network.2.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(b.devices().is_empty(), "found before pairing, under another secret: not listed");
    assert!(b.clone().pair(a.pairing_link().unwrap()));
    wait(&b_changed, || b.devices().iter().any(|d| d.id == a.id()));
    b.stop();
    a.stop();
}
