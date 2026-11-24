use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;

use nori_tasks::Native;
use nori_testdir::TempDir;

use super::*;
use crate::hub::{Hub, HubLink};
use crate::keys::RoomName;
use crate::profile::tests::{navidrome, phone};
use crate::room::{Heard, Member, Relay, RoomKind, To};
use crate::wire::{read_up, Code, Down, Up, Welcome};

const S: Duration = Duration::from_secs(1);
const NOW: i64 = 1_700_000_000_000;

/// One socket of the stand-in server: a hub link in one room.
struct Socket {
    link: HubLink,
    room: RoomName,
    /// The time requests waiting for their answer, as sent.
    times: VecDeque<i64>,
}

/// A sync server speaking the protocol, over the in-memory hub: what a device's sockets reach.
struct Net {
    hub: Hub,
    sockets: Mutex<HashMap<SocketId, Socket>>,
    /// Sockets that ended on the server's side or the network's, to be told.
    ended: Mutex<Vec<(SocketId, Ended)>>,
    opened: AtomicUsize,
    pings: AtomicUsize,
}

impl Net {
    fn new(hub: &Hub) -> Arc<Net> {
        Arc::new(Net { hub: hub.clone(), sockets: Mutex::default(), ended: Mutex::default(), opened: AtomicUsize::new(0), pings: AtomicUsize::new(0) })
    }

    fn open_sockets(&self) -> usize {
        self.sockets.lock().unwrap().len()
    }

    /// The network drops every socket.
    fn drop_all(&self) {
        let gone: Vec<(SocketId, Socket)> = self.sockets.lock().unwrap().drain().collect();
        for (id, s) in gone {
            s.link.drop_connection();
            self.ended.lock().unwrap().push((id, Ended::Dropped));
        }
    }

    /// Hands `syncing` what its sockets heard; true when anything.
    fn deliver(&self, syncing: &Syncing) -> bool {
        let mut out: Vec<(SocketId, Result<Vec<u8>, Ended>)> = self.ended.lock().unwrap().drain(..).map(|(id, e)| (id, Err(e))).collect();
        let mut sockets = self.sockets.lock().unwrap();
        let mut closed = Vec::new();
        for (&id, s) in sockets.iter_mut() {
            for h in s.link.heard() {
                let said = match h {
                    Heard::Joined { me, members, .. } => {
                        let w = Welcome { you: me.0 as u16, max_frame: 65_536, messages_per_minute: 240, bytes_per_minute: 4 << 20, max_members: 20, closes_at: None, members: members.iter().map(|m| m.0 as u16).collect() };
                        Ok(Down::Welcome(w))
                    }
                    Heard::Arrived { member, .. } => Ok(Down::Joined(member.0 as u16)),
                    Heard::Left { member, .. } => Ok(Down::Left(member.0 as u16)),
                    Heard::Message { from, payload, .. } => Ok(Down::Message { from: from.0 as u16, alone: false, payload }),
                    Heard::Time { ms, .. } => Ok(Down::Time { t1: s.times.pop_front().expect("asked"), t2: ms * 1000, t3: ms * 1000 }),
                    Heard::Copy { blob, .. } => Ok(Down::Kept(blob)),
                    Heard::Closed { .. } => Err(Ended::Closed { code: Code::RoomClosed.number() }),
                    Heard::Refused { asked: crate::room::Asked::Join, reason, .. } => Err(Ended::Closed { code: refused(reason).number() }),
                    Heard::Refused { reason, .. } => Ok(Down::Refused { asked: crate::wire::Asked::KeptPut, code: refused(reason), retry_after_ms: 0 }),
                };
                if said.is_err() {
                    closed.push(id);
                }
                out.push((id, said.map(|d| d.encode())));
            }
        }
        for id in closed {
            sockets.remove(&id);
        }
        drop(sockets);
        let any = !out.is_empty();
        for (id, said) in out {
            match said {
                Ok(frame) => syncing.frame(id, frame),
                Err(how) => syncing.ended(id, how),
            }
        }
        any
    }
}

fn refused(reason: Refusal) -> Code {
    match reason {
        Refusal::DailyWall => Code::DailyLimit,
        Refusal::Limit => Code::RoomFull,
        Refusal::Version => Code::VersionUnsupported,
        Refusal::WrongSecret => Code::WrongSecret,
        Refusal::Unavailable => Code::Unavailable,
        Refusal::Invalid => Code::BadRequest,
    }
}

impl Sockets for Net {
    fn open(&self, id: SocketId, url: String) {
        self.opened.fetch_add(1, Ordering::Relaxed);
        let (path, query) = url.split_once("/room/").expect("a room's address").1.split_once('?').expect("a query");
        let kind = if query.contains("kind=meeting") { RoomKind::Meeting } else { RoomKind::Group };
        let room = RoomName::parse(path).expect("a room's name");
        let link = self.hub.link();
        link.join(&room, kind);
        self.sockets.lock().unwrap().insert(id, Socket { link, room, times: VecDeque::new() });
    }

    fn send(&self, id: SocketId, frame: Vec<u8>) {
        let mut sockets = self.sockets.lock().unwrap();
        let Some(s) = sockets.get_mut(&id) else { return };
        match read_up(&frame) {
            Up::SendAll(p) => s.link.send(&s.room, To::Everyone, p.to_vec()),
            Up::SendTo(m, p) => s.link.send(&s.room, To::Member(Member(m.into())), p.to_vec()),
            Up::Time(t1) => {
                s.times.push_back(t1);
                s.link.ask_time(&s.room);
            }
            Up::KeptGet => s.link.get_copy(&s.room),
            Up::KeptPut(secret, blob) => s.link.put_copy(&s.room, *secret, blob.to_vec()),
            Up::KeptDelete(secret) => s.link.delete_copy(&s.room, *secret),
        }
    }

    fn ping(&self, _: SocketId) {
        self.pings.fetch_add(1, Ordering::Relaxed);
    }

    fn close(&self, id: SocketId) {
        if let Some(s) = self.sockets.lock().unwrap().remove(&id) {
            s.link.leave(&s.room);
        }
    }
}

/// What the client heard from sync.
#[derive(Default)]
struct Told {
    synced: Mutex<Vec<ProfilesSynced>>,
    events: Mutex<Vec<SyncEvent>>,
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
    fn changed(&self) {}
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
        let tasks = Arc::new(Native::simulated("syncing-test", 0).unwrap());
        let (net, told, made) = (Net::new(hub), Arc::new(Told::default()), Arc::new(AtomicUsize::new(0)));
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
    let SyncState::Waiting { refused: None, .. } = mac.syncing.state() else { panic!("waiting: {:?}", mac.syncing.state()) };
    assert!(!pixel.syncing.devices()[0].online);
    assert!(mac.told.events.lock().unwrap().is_empty(), "a drop is no event");
    mac.tasks.pass(5 * S);
    settle(&[&pixel, &mac]);
    assert_eq!((mac.syncing.state(), pixel.syncing.devices()[0].online), (SyncState::Connected, true), "back within 5 s");
    hub.wall(true);
    mac.net.drop_all();
    settle(&[&pixel, &mac]);
    mac.tasks.pass(5 * S);
    settle(&[&pixel, &mac]);
    let SyncState::Waiting { refused: Some(Refusal::DailyWall), retry_ms } = mac.syncing.state() else { panic!("refused: {:?}", mac.syncing.state()) };
    let day = 24 * 60 * 60 * 1000;
    assert!(retry_ms > nori_time::unix_ms() && retry_ms % day <= 30 * 60 * 1000, "after midnight UTC");
    assert_eq!(mac.told.events.lock().unwrap()[..], [SyncEvent::Refused { reason: Refusal::DailyWall }]);
    let opened = mac.net.opened.load(Ordering::Relaxed);
    mac.tasks.pass(60 * 60 * S);
    assert_eq!(mac.net.opened.load(Ordering::Relaxed), opened, "and does not try again before then");
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
