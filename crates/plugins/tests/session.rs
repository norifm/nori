//! The session a plugin reaches, its hooks, timers, cache, assets, wakes and commands, over the test
//! plugin (fixture/src/hooked.rs) on a simulated clock.

use std::sync::{Arc, Mutex};

use nori_model::Song;
use nori_net::transport::{Exchange, Network, Transport, TransportError, TransportResponse};
use nori_plugins::testing::{fixture, FIXTURE as ID};
use nori_plugins::wit::types as t;
use nori_plugins::{AssetState, Command, Control, Found, Hook, ItemKind, Library, Moment, Plugins, Profile, Repeat, Side, Value, WakeEvent};
use nori_tasks::Native;
use nori_time::Duration;

/// What the plugin did through the client.
#[derive(Default)]
struct Client {
    notices: Mutex<Vec<(String, String)>>,
    controls: Mutex<Vec<Control>>,
    wakes: Mutex<Vec<Vec<WakeEvent>>>,
    commands: Mutex<Vec<Vec<Command>>>,
    assets: Mutex<Vec<AssetState>>,
    sign_ins: Mutex<Vec<String>>,
    screen_watched: Mutex<Vec<bool>>,
    allows_provider_songs: bool,
}

impl Side for Client {
    fn stored(&self, key: &str) -> Option<Value> {
        (key == "autoFillRemote").then_some(Value::Switch(self.allows_provider_songs))
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn laid(&self, _: &str) {}
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, words: &str, args: &[String]) {
        self.notices.lock().unwrap().push((words.to_string(), args.join(" ")));
    }
    fn control(&self, c: Control) {
        self.controls.lock().unwrap().push(c);
    }
    fn sign_in(&self, _: &str, url: &str) -> bool {
        self.sign_ins.lock().unwrap().push(url.to_string());
        true
    }
    fn wake(&self, events: &[WakeEvent]) {
        self.wakes.lock().unwrap().push(events.to_vec());
    }
    fn commands(&self, commands: &[Command]) {
        self.commands.lock().unwrap().push(commands.to_vec());
    }
    fn watch_screen(&self, on: bool) {
        self.screen_watched.lock().unwrap().push(on);
    }
    fn asset(&self, _: &str, _: &str, state: AssetState) {
        self.assets.lock().unwrap().push(state);
    }
}

impl Client {
    /// The notices said for `what`, taken.
    fn said(&self, what: &str) -> Vec<String> {
        let mut n = self.notices.lock().unwrap();
        let (taken, kept) = n.drain(..).partition(|(w, _)| w == what);
        *n = kept;
        taken.into_iter().map(|(_, l)| l).collect()
    }
}

fn song(id: &str) -> Song {
    Song { id: id.into(), title: id.to_uppercase(), is_external: id.starts_with("ext-"), ..Default::default() }
}

/// A profile whose queue is songs a to e, playing c.
#[derive(Default)]
struct Shelf {
    rev: Mutex<u64>,
}

impl Library for Shelf {
    fn profile(&self) -> Option<Profile> {
        Some(Profile { id: "p".into(), kind: ID.into(), name: "Home".into() })
    }
    fn queue(&self, start: u32, count: u32) -> Vec<Song> {
        ["a", "b", "c", "d", "e"].iter().skip(start as usize).take(count as usize).map(|i| song(i)).collect()
    }
    fn queue_length(&self) -> u32 {
        5
    }
    fn queue_rev(&self) -> u64 {
        *self.rev.lock().unwrap()
    }
    fn shuffle(&self) -> bool {
        false
    }
    fn repeat(&self) -> Repeat {
        Repeat::All
    }
    fn songs(&self, ids: &[String]) -> Result<Vec<Song>, t::Error> {
        Ok(ids.iter().map(|i| song(i)).collect())
    }
    fn search(&self, query: &str, _: u32) -> Result<Vec<Found>, t::Error> {
        Ok(vec![Found::Song(Box::new(song(query)))])
    }
    fn lookup(&self, _: ItemKind, id: &str) -> Result<Found, t::Error> {
        Ok(Found::Song(Box::new(song(id))))
    }
    fn songs_of(&self, _: ItemKind, id: &str) -> Result<Vec<Song>, t::Error> {
        Ok(vec![song(id)])
    }
    fn star(&self, _: ItemKind, _: &str, _: bool) -> Result<(), t::Error> {
        Ok(())
    }
    fn add_to_playlist(&self, _: &str, _: &[String]) -> Result<(), t::Error> {
        Ok(())
    }
    fn subsonic(&self, method: &str, params: &[(String, String)]) -> Result<Vec<u8>, t::Error> {
        Ok(format!("{method}?{}", params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")).into_bytes())
    }
    fn synced_lyrics(&self, s: &Song) -> Option<Vec<(u32, String)>> {
        (s.id == "c").then(|| vec![(0, "one".into()), (1000, "two".into()), (2500, "three".into())])
    }
}

/// Serves the asset, on a network metered or not; notes what it was asked.
#[derive(Default)]
struct Web {
    metered: bool,
    asked: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Transport for Web {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        self.asked.lock().unwrap().push(url.clone());
        let body = if url.ends_with("/blob") { b"asset bytes".to_vec() } else { b"ok".to_vec() };
        Ok(TransportResponse { status: 200, body, headers: Vec::new() })
    }
    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(e.url, e.timeout_ms).await
    }
    fn address_changed(&self) {}
    fn network(&self) -> Network {
        if self.metered { Network::Metered } else { Network::Unmetered }
    }
}

struct Rig {
    _dir: nori_testdir::TempDir,
    client: Arc<Client>,
    plugins: Arc<Plugins>,
    clock: Arc<Native>,
    shelf: Arc<Shelf>,
}

impl Rig {
    fn new(name: &str, client: Client, web: Web) -> Rig {
        let dir = nori_testdir::TempDir::new(name);
        let client = Arc::new(client);
        let plugins = Plugins::open(dir.path().to_path_buf(), client.clone());
        let clock = Arc::new(Native::simulated("plugin-clock", 0).unwrap());
        plugins.use_tasks(clock.clone());
        let shelf = Arc::new(Shelf::default());
        plugins.set_library(Some(shelf.clone()));
        plugins.set_transport(Some(Arc::new(web)));
        plugins.install(&fixture("")).unwrap();
        Rig { _dir: dir, client, plugins, clock, shelf }
    }

    fn on(name: &str) -> Rig {
        let r = Rig::new(name, Client::default(), Web::default());
        r.plugins.turn(ID.into(), true);
        r
    }

    /// Waits until the plugin has run everything posted to it.
    fn settle(&self) {
        let w = self.plugins.worker(ID).unwrap();
        assert_eq!(nori_tasks::block_on(w.call(|_, _| Ok(1))), Some(1));
    }

    fn act(&self, action: &str) {
        self.plugins.act(ID.into(), action.into());
        self.settle();
    }

    /// Lets `ms` pass on the clock, then lets the plugin run what that woke.
    fn pass(&self, ms: u64) {
        self.clock.pass(Duration::from_millis(ms));
        self.settle();
    }

    fn heard(&self) -> Vec<String> {
        self.client.said("heard")
    }
}

#[test]
fn the_session_reads_the_player_and_library_and_controls_through_the_client() {
    let r = Rig::on("plugins-session");
    r.plugins.played(Moment { playing: true, index: Some(2), song: Some("c".into()), position_ms: 1000, ..Default::default() });
    r.clock.pass(Duration::from_millis(500));
    r.act("now");
    assert_eq!(r.client.said("now"), ["C Some(2) true 1500 5"], "the position runs on while it plays");
    r.act("queue");
    assert_eq!(r.client.said("queue"), ["B,C,D,E"]);
    r.act("add");
    assert_eq!(r.client.said("add"), ["Ok(())"]);
    r.act("next");
    assert_eq!(*r.client.controls.lock().unwrap(), [Control::Add { songs: vec![song("a"), song("b")], next: true }, Control::Next]);
    r.act("add-provider");
    assert_eq!(r.client.said("add-provider"), ["Err(\"Refused\")"], "a provider song the user didn't ask for");
    r.act("search");
    assert_eq!(r.client.said("search"), ["Ok([\"BLUE\"])"]);
    r.act("star");
    assert_eq!(r.client.said("star"), ["Err(\"Refused\")"], "without library.write");
    r.act("subsonic");
    assert_eq!(r.client.said("subsonic"), ["Ok(\"getUser?username=u\")"]);
    r.act("subsonic-path");
    assert_eq!(r.client.said("subsonic-path"), ["Err(\"Refused\")"]);
    r.act("sign-in");
    assert_eq!(r.client.said("sign-in"), ["Ok(()) Err(\"Refused\")"], "only at its signin hosts");
    assert_eq!(*r.client.sign_ins.lock().unwrap(), ["https://accounts.example/start"]);

    let allowed = Rig::new("plugins-session-provider", Client { allows_provider_songs: true, ..Default::default() }, Web::default());
    allowed.plugins.turn(ID.into(), true);
    allowed.act("add-provider");
    assert_eq!(allowed.client.said("add-provider"), ["Ok(())"], "the user's switch lets autofill add them");
}

#[test]
fn events_are_built_only_for_what_an_enabled_plugin_hears() {
    let r = Rig::new("plugins-hooks", Client::default(), Web::default());
    r.plugins.turn(ID.into(), false);
    assert!(!r.plugins.hears(Hook::SongChanged) && !r.plugins.follows(), "off, nothing is heard");
    r.plugins.turn(ID.into(), true);
    assert!(r.plugins.hears(Hook::SongChanged) && r.plugins.follows());
    assert!(!r.plugins.hears(Hook::MediaButton) && !r.plugins.hears(Hook::SettingChanged), "only what its manifest names");

    let at = |song: &str, playing: bool| Moment { playing, index: Some(0), song: Some(song.into()), ..Default::default() };
    r.plugins.played(at("a", true));
    r.settle();
    assert_eq!(r.heard(), ["started", "song A"]);
    r.plugins.played(at("a", true));
    *r.shelf.rev.lock().unwrap() += 1;
    r.plugins.played(at("a", true));
    r.plugins.played(Moment { index: Some(1), ..at("b", false) });
    r.plugins.favorite_changed("b".into(), true);
    r.plugins.device("WH-1000XM5".into(), true);
    r.plugins.heard_press(nori_plugins::Press::Next);
    r.settle();
    assert_eq!(r.heard(), ["queue", "paused", "song B", "favorite b true", "device WH-1000XM5"], "the same moment again is no event; a media button no one hears is none");

    r.plugins.turn(ID.into(), false);
    assert!(!r.plugins.hears(Hook::SongChanged) && !r.plugins.follows());
}

#[test]
fn lyric_lines_wake_as_they_are_reached_while_the_song_plays() {
    let r = Rig::on("plugins-lyrics");
    r.plugins.played(Moment { playing: true, index: Some(2), song: Some("c".into()), position_ms: 0, ..Default::default() });
    // The lyrics come on a thread of their own; the clock stands still meanwhile.
    r.until_heard("lyric 0 one");
    assert_eq!(r.heard(), ["started", "song C", "lyric 0 one"]);
    r.pass(999);
    assert!(r.heard().is_empty());
    r.pass(1);
    assert_eq!(r.heard(), ["lyric 1 two"]);
    r.plugins.played(Moment { playing: false, index: Some(2), song: Some("c".into()), position_ms: 1200, ..Default::default() });
    r.clock.settle();
    assert!(r.clock.waits().is_empty(), "paused, no wake is left for the next line");
    r.pass(5000);
    assert_eq!(r.heard(), ["paused"]);
    r.plugins.played(Moment { playing: true, index: Some(2), song: Some("c".into()), position_ms: 2000, ..Default::default() });
    r.pass(500);
    assert_eq!(r.heard(), ["started", "lyric 2 three"], "from where it plays again");
}

impl Rig {
    /// Waits until the plugin has heard `line`: what a thread of the host's own (lyrics, an asset)
    /// brings in, in real time.
    fn until_heard(&self, line: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !self.client.notices.lock().unwrap().iter().any(|(w, l)| w == "heard" && l == line) {
            assert!(std::time::Instant::now() < deadline, "never heard {line}");
            self.clock.settle();
            self.settle();
            std::thread::yield_now();
        }
    }
}

#[test]
fn timers_fire_only_while_their_gate_is_open_and_never_wait_on_the_clock_otherwise() {
    let r = Rig::on("plugins-timers");
    r.act("every-second");
    assert_eq!(*r.client.screen_watched.lock().unwrap(), [true], "an always timer needs the screen's state");
    r.pass(10_000);
    assert!(r.heard().is_empty(), "no schedule permission and no active session");
    r.clock.settle();
    assert!(r.clock.waits().is_empty(), "a timer that may not fire sleeps on no clock");

    r.act("cast-target");
    r.clock.settle();
    r.pass(3500);
    assert_eq!(r.heard(), ["timer 1", "timer 1", "timer 1"], "a cast makes its session active");
    r.act("cast-end");
    r.clock.settle();
    r.pass(3000);
    assert!(r.heard().is_empty(), "the cast ended");
    r.act("stop");
    r.pass(3000);
    assert!(r.heard().is_empty());
    assert_eq!(r.client.screen_watched.lock().unwrap().last(), Some(&false), "no timer needs it now");

    r.act("visible");
    r.pass(3000);
    assert!(r.heard().is_empty(), "the player isn't on screen");
    r.plugins.player_shown(true);
    r.pass(2000);
    assert_eq!(r.heard(), ["timer 2", "timer 2"]);
    r.plugins.screen(false);
    r.pass(2000);
    assert!(r.heard().is_empty(), "the screen went off");

    r.act("cast-target");
    r.act("once");
    r.clock.settle();
    r.pass(2000);
    assert_eq!(r.heard(), ["timer 3"], "a one-shot fires once");
    r.plugins.turn(ID.into(), false);
    r.clock.settle();
    assert!(r.clock.waits().is_empty(), "turned off, its timers go");
}

#[test]
fn its_cache_keeps_within_the_quota_its_manifest_names() {
    let r = Rig::on("plugins-cache");
    r.act("cache");
    assert_eq!(r.client.said("cache"), ["Ok(()) Err(\"Refused\") Some(\"kept\") true"]);
}

#[test]
fn assets_are_fetched_on_wifi_or_when_the_user_allows_mobile_data() {
    let r = Rig::new("plugins-assets-mobile", Client::default(), Web { metered: true, ..Default::default() });
    r.plugins.turn(ID.into(), true);
    assert_eq!(r.plugins.assets(ID.into())[0].state, AssetState::Missing);
    r.act("asset");
    assert_eq!(*r.client.assets.lock().unwrap(), [AssetState::WaitingForWifi]);
    r.plugins.allow_mobile_assets(ID.into());
    r.until_heard("asset blob");
    assert_eq!(r.heard(), ["asset blob"]);
    assert_eq!(r.client.assets.lock().unwrap()[1..], [AssetState::Fetching { done: 0, total: 11 }, AssetState::Fetching { done: 11, total: 11 }, AssetState::Ready]);
    r.act("asset-read");
    assert_eq!(r.client.said("asset-read"), ["Ready Some(\"asset bytes\")"]);
    // Told from within the plugin's own call, so it runs after the call: waited for.
    r.act("asset");
    r.until_heard("asset blob");
    assert_eq!(r.heard(), ["asset blob"], "one in already is said at once");

    // An update keeps it; removing the plugin removes it.
    r.plugins.install(&fixture("")).unwrap();
    assert_eq!(r.plugins.assets(ID.into())[0].state, AssetState::Ready);
    r.plugins.uninstall(ID.into());
    r.plugins.install(&fixture("")).unwrap();
    assert_eq!(r.plugins.assets(ID.into())[0].state, AssetState::Missing);
}

#[test]
fn wakes_and_commands_are_on_only_while_a_plugin_wants_them() {
    let r = Rig::new("plugins-wake", Client::default(), Web::default());
    r.plugins.turn(ID.into(), false);
    assert_eq!(r.client.commands.lock().unwrap().last(), Some(&vec![]), "nothing on for a plugin that is off");
    r.client.commands.lock().unwrap().clear();
    r.client.wakes.lock().unwrap().clear();
    r.plugins.turn(ID.into(), true);
    assert_eq!(*r.client.commands.lock().unwrap(), [vec![Command { plugin: ID.into(), command: "ping".into() }]]);
    assert_eq!(*r.client.wakes.lock().unwrap(), [vec![]], "declared, but not wanted yet");
    r.act("wake");
    assert_eq!(r.client.wakes.lock().unwrap().last().unwrap(), &[WakeEvent::Boot], "only what its manifest declares");
    r.plugins.woke(WakeEvent::Boot);
    r.plugins.woke(WakeEvent::BluetoothConnected);
    r.plugins.command(Command { plugin: ID.into(), command: "ping".into() });
    r.plugins.command(Command { plugin: ID.into(), command: "now".into() });
    r.settle();
    assert_eq!(r.heard(), ["woke boot"]);
    assert_eq!(r.client.said("ping"), ["pong"]);
    assert!(r.client.said("now").is_empty(), "an action it doesn't declare as a command");
    r.plugins.turn(ID.into(), false);
    assert_eq!((r.client.wakes.lock().unwrap().last().unwrap().len(), r.client.commands.lock().unwrap().last().unwrap().len()), (0, 0));
}
