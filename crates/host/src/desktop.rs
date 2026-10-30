//! What the terminal and desktop clients share around their session: the app's queue session over its
//! settings, their own settings kept as app values, logging in, the sound card (cpal) and the session
//! opened over them.

use std::path::Path;
use std::sync::Arc;

use nori_core::remote::wire::DeviceKind;
use nori_core::settings::SavedServer;
use nori_core::transport::{block_on, NetError};
use nori_http::Http;
use nori_output_cpal::CpalOutput;

use crate::session::{Open, Out, Session};
use crate::Level;

/// A calendar date, and whether the zone it is in lies south of the equator (the seasons turn half a year
/// later there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Today {
    pub month: u32,
    pub day: u32,
    pub southern: bool,
}

impl Today {
    /// The local date now, in the local time zone.
    pub fn now() -> Today {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (month, day) = nori_look::theme::utc_date(now + nori_core::library::local_offset_s(now));
        let link = std::fs::read_link("/etc/localtime").map(|p| p.to_string_lossy().into_owned()).ok();
        Today { month, day, southern: zone(std::env::var("TZ").ok(), link).is_some_and(|z| nori_look::theme::southern(&z)) }
    }
}

/// The IANA time zone: `tz` (the `TZ` variable), else where `link` (`/etc/localtime`) points.
fn zone(tz: Option<String>, link: Option<String>) -> Option<String> {
    if let Some(tz) = tz.filter(|z| !z.is_empty()) {
        return Some(tz.trim_start_matches(':').to_string());
    }
    link?.split_once("zoneinfo/").map(|(_, z)| z.to_string())
}

/// The app's queue session over its settings: the client's object graph root, which every view reads.
/// Global: the root of the client's own object graph (as Kotlin's `Nori`), made once a process.
pub fn app() -> &'static Arc<nori_core::queue::Session> {
    static APP: std::sync::OnceLock<Arc<nori_core::queue::Session>> = std::sync::OnceLock::new();
    APP.get_or_init(|| Arc::new(nori_core::queue::Session::new(nori_core::settings_store::Settings::new())))
}

/// The client's own settings, kept as app values in the database under keys of its own.
pub mod own {
    pub fn text(key: &str) -> Option<String> {
        super::app().settings.app_value(key).filter(|v| !v.is_empty())
    }

    pub fn flag(key: &str, default: bool) -> bool {
        super::app().settings.app_value(key).map_or(default, |v| v == "true")
    }

    pub fn number(key: &str, default: f32) -> f32 {
        super::app().settings.app_value(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn keep(key: &'static str, value: String) {
        super::app().settings.keep_app_value(key, value);
    }
}

/// The app's plugins, installed under `data`/plugins and laid over its settings, and the kinds of
/// profile they and nori open. Opened once, at the first call.
pub fn plugins(data: &Path) -> &'static Plugged {
    static PLUGINS: std::sync::OnceLock<Plugged> = std::sync::OnceLock::new();
    PLUGINS.get_or_init(|| {
        let watched = Arc::new(Watched::default());
        let host = nori_plugins::open(data.join("plugins"), app().settings.clone(), watched.clone());
        let _ = watched.plugins.set(Arc::downgrade(&host));
        // Watched only while an enabled plugin reaches devices or hears outputs connect (BlueZ and
        // IOBluetooth are how the desktop hears them); elsewhere plugins' Bluetooth is unsupported.
        #[cfg(target_os = "linux")]
        host.use_bluetooth(Arc::new(crate::bluez::Bluez::new(Arc::downgrade(&host))), true);
        #[cfg(target_os = "macos")]
        host.use_bluetooth(Arc::new(crate::macbt::MacBluetooth::new(Arc::downgrade(&host))), true);
        Plugged { kinds: nori_core::providers::Kinds::with_plugins(host.clone()), host, watched }
    })
}

/// Installs (or updates) the plugin package or folder at `file` and turns it on.
pub fn install_plugin(data: &Path, file: &Path) -> Result<(), String> {
    let host = &plugins(data).host;
    let installed = host.install(file).map_err(|e| format!("{}: {e:?}", file.display()))?;
    host.turn(installed.id.clone(), true);
    Ok(())
}

/// Updates the plugins at most once a day, off this thread, through the open profile's connection;
/// `done` hears each plugin updated and its new version, when any was.
pub fn update_plugins(data: &Path, done: impl FnOnce(Vec<(String, String)>) + Send + 'static) {
    let host = plugins(data).host.clone();
    if !host.updates_due(nori_time::unix_ms()) {
        return;
    }
    crate::spawn("nori-plugin-update", move || {
        let updated = block_on(host.update(nori_time::unix_ms()));
        if !updated.is_empty() {
            done(updated.into_iter().map(|u| (u.id, u.to)).collect());
        }
    });
}

/// Why a backup couldn't be made or restored: the backup's own reason, or its file's (the system's
/// words).
#[derive(Debug, Clone, PartialEq)]
pub enum BackupFailure {
    Backup(nori_core::backup::BackupError),
    File(String),
}

/// Saves a backup of the app's settings and plugins at `path`, sealed with `password`, off this
/// thread; `done` hears how it went.
pub fn make_backup(data: &Path, path: std::path::PathBuf, password: String, done: impl FnOnce(Result<(), BackupFailure>) + Send + 'static) {
    let host = plugins(data).host.clone();
    crate::spawn("nori-backup", move || {
        let made = nori_core::backup::make(&app().settings, &host, &password).map_err(BackupFailure::Backup);
        done(made.and_then(|file| std::fs::write(&path, file).map_err(|e| BackupFailure::File(e.to_string()))));
    });
}

/// Restores the backup at `path` with `password`, off this thread; `done` hears what came back, for
/// the session to apply (`Session::backup_restored`).
pub fn restore_backup(data: &Path, path: std::path::PathBuf, password: String, done: impl FnOnce(Result<nori_core::backup::Restored, BackupFailure>) + Send + 'static) {
    let host = plugins(data).host.clone();
    crate::spawn("nori-restore", move || {
        let file = std::fs::read(&path).map_err(|e| BackupFailure::File(e.to_string()));
        done(file.and_then(|f| block_on(nori_core::backup::restore(&app().settings, &host, &f, &password)).map_err(BackupFailure::Backup)));
    });
}

/// Why plugins are off at this start, with the names of those nori stopped in; None after the first ask.
pub fn safe_start(data: &Path) -> Option<(nori_plugins::SafeStart, Vec<String>)> {
    let host = &plugins(data).host;
    let safe = host.take_safe_start()?;
    let names = match &safe {
        nori_plugins::SafeStart::Crashed { plugins } => {
            let cards = host.cards("en".into());
            plugins.iter().map(|id| cards.iter().find(|c| c.id == *id).map_or_else(|| id.clone(), |c| c.name.clone())).collect()
        }
        nori_plugins::SafeStart::AllOff => Vec::new(),
    };
    Some((safe, names))
}

/// Where a plugin a repo offers stands here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferState {
    New,
    /// Another version is installed.
    Update,
    Installed,
}

/// The newest version `offer` has that this nori runs, and where the plugin stands against it; None
/// without one.
pub fn offer_state(offer: &nori_plugins::Offer) -> Option<(&nori_plugins::Version, OfferState)> {
    let newest = offer.versions.first()?;
    let state = match &offer.installed {
        None => OfferState::New,
        Some(v) if *v == newest.version => OfferState::Installed,
        Some(_) => OfferState::Update,
    };
    Some((newest, state))
}

/// The plugin host, the kinds of profile it adds, and what hears from it.
pub struct Plugged {
    pub host: Arc<nori_plugins::Plugins>,
    pub kinds: Arc<nori_core::providers::Kinds>,
    pub watched: Arc<Watched>,
}

/// A plugin's notice, in its words.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub plugin: String,
    pub problem: bool,
    pub words: String,
}

/// Where a client takes the plugins' notices.
type Hears = Box<dyn Fn(Notice) + Send + Sync>;

/// Where a client opens a plugin's sign-in page: the plugin's id and the page's address.
type SignsIn = Box<dyn Fn(String, String) + Send + Sync>;

/// What a desktop client does with what plugins say: the open session takes the settings laid anew and
/// the plugins' controls; notices go to the client that hears them ([`Watched::hear`]), sign-in pages
/// to the one that opens them ([`Watched::sign_in_with`]); what they say of their rows is kept for their
/// pages ([`Watched::as_said`]), and commands go to D-Bus on Linux. The desktop has no wake: nothing starts it while it is closed.
#[derive(Default)]
pub struct Watched {
    engine: std::sync::Mutex<Option<std::sync::Weak<nori_engine::Engine>>>,
    session: std::sync::Mutex<Option<std::sync::Weak<crate::session::Parts>>>,
    plugins: std::sync::OnceLock<std::sync::Weak<nori_plugins::Plugins>>,
    heard: std::sync::Mutex<Option<Hears>>,
    /// Told when a plugin's page or the devices plugins control changed ([`Watched::on_changed`]).
    changed: std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// What plugins last said of their rows ([`Watched::as_said`]).
    said: std::sync::Mutex<Said>,
    /// mDNS for the plugins, while one browses.
    browsing: std::sync::Mutex<Option<Browsing>>,
    casts: std::sync::Mutex<Option<CastSeen>>,
    signs_in: std::sync::Mutex<Option<SignsIn>>,
    #[cfg(target_os = "linux")]
    commands: std::sync::Mutex<Option<nori_mpris::Commands>>,
}

/// What plugins said of their rows, by plugin and row: status rows' lines, and whether each row shows
/// and takes input.
#[derive(Default)]
struct Said {
    status: std::collections::HashMap<(String, String), String>,
    shown: std::collections::HashMap<(String, String), (bool, bool)>,
}

/// Where a client takes the cast's changes.
type CastSeen = Box<dyn Fn(Option<nori_plugins::Cast>) + Send + Sync>;

/// mDNS browsing for plugins, over mdns-sd's own thread: the kinds browsed.
struct Browsing {
    daemon: mdns_sd::ServiceDaemon,
    kinds: std::collections::BTreeSet<String>,
}

impl Watched {
    /// Sends the plugins' notices to `to`, from whatever thread a plugin runs on.
    pub fn hear(&self, to: impl Fn(Notice) + Send + Sync + 'static) {
        *self.heard.lock().expect("not poisoned") = Some(Box::new(to));
    }

    /// Tells `to` whenever what a plugin's page shows (its rows' lines, its assets) or the controlled
    /// devices changed, from whatever thread: the client reads them again.
    pub fn on_changed(&self, to: impl Fn() + Send + Sync + 'static) {
        *self.changed.lock().expect("not poisoned") = Some(Box::new(to));
    }

    fn tell_changed(&self) {
        if let Some(to) = self.changed.lock().expect("not poisoned").as_ref() {
            to();
        }
    }

    /// `cards` as their pages draw them now: the rows their plugins hide left out, each status row's
    /// value the line it said last. What it returns are the rows that show but take no input now, by
    /// plugin and row.
    pub fn as_said(&self, cards: &mut [nori_plugins::PluginCard]) -> std::collections::HashSet<(String, String)> {
        let said = self.said.lock().expect("not poisoned");
        let mut off = std::collections::HashSet::new();
        for c in cards {
            let key = |row: &str| (c.id.clone(), row.to_string());
            c.rows.retain(|r| said.shown.get(&key(&r.id)).is_none_or(|(shown, _)| *shown));
            for r in &mut c.rows {
                if let Some(line) = said.status.get(&key(&r.id)) {
                    r.value = Some(nori_plugins::Value::Text(line.clone()));
                }
                if said.shown.get(&key(&r.id)).is_some_and(|(_, enabled)| !enabled) {
                    off.insert(key(&r.id));
                }
            }
        }
        off
    }

    /// Tells `to` of each change of the cast running (None: it ended), for the output picker.
    pub fn hear_casts(&self, to: impl Fn(Option<nori_plugins::Cast>) + Send + Sync + 'static) {
        *self.casts.lock().expect("not poisoned") = Some(Box::new(to));
    }

    /// Opens plugins' sign-in pages with `open`, from whatever thread a plugin runs on.
    pub fn sign_in_with(&self, open: impl Fn(String, String) + Send + Sync + 'static) {
        *self.signs_in.lock().expect("not poisoned") = Some(Box::new(open));
    }

    fn session(&self) -> Option<crate::session::Handle> {
        self.session.lock().expect("not poisoned").as_ref().and_then(std::sync::Weak::upgrade).map(crate::session::Handle::of)
    }

    /// Plugin `id`'s word `key` with `args` in its `{0}`, `{1}`…; the key itself where it has no such word.
    fn said(&self, id: &str, key: String, args: &[String]) -> String {
        let word = self.plugins.get().and_then(std::sync::Weak::upgrade).and_then(|p| p.word(id.into(), key.clone(), "en".into()));
        args.iter().enumerate().fold(word.unwrap_or(key), |line, (i, a)| line.replace(&format!("{{{i}}}"), a))
    }
}

impl nori_plugins::Client for Watched {
    /// The engine takes all of its settings again, whatever `effect` says.
    fn layer_changed(&self, _effect: u32) {
        let engine = self.engine.lock().expect("not poisoned").as_ref().and_then(std::sync::Weak::upgrade);
        if let (Some(engine), Some(prefs)) = (engine, app().settings.current()) {
            engine.set_settings(nori_engine::core::settings(&prefs, app().settings.layer()));
        }
    }
    fn status(&self, id: String, row: String, words: String, args: Vec<String>) {
        let line = self.said(&id, words, &args);
        self.said.lock().expect("not poisoned").status.insert((id, row), line);
        self.tell_changed();
    }
    fn shown(&self, id: String, row: String, shown: bool, enabled: bool) {
        self.said.lock().expect("not poisoned").shown.insert((id, row), (shown, enabled));
        self.tell_changed();
    }
    fn notice(&self, id: String, problem: bool, words: String, args: Vec<String>) {
        let words = self.said(&id, words, &args);
        nori_core::alog::info(&format!("plugin {id} says{}: {words}", if problem { " (a problem)" } else { "" }));
        if let Some(to) = self.heard.lock().expect("not poisoned").as_ref() {
            to(Notice { plugin: id, problem, words });
        }
    }
    fn devices_changed(&self) {
        self.tell_changed();
    }

    fn browse(&self, kind: String, on: bool) {
        let mut b = self.browsing.lock().expect("not poisoned");
        let ty = format!("{kind}.local.");
        if !on {
            if let Some(browsing) = b.as_mut() {
                let _ = browsing.daemon.stop_browse(&ty);
                browsing.kinds.remove(&kind);
                if browsing.kinds.is_empty() {
                    let _ = browsing.daemon.shutdown();
                    *b = None;
                }
            }
            return;
        }
        if b.is_none() {
            let Ok(daemon) = mdns_sd::ServiceDaemon::new() else { return };
            *b = Some(Browsing { daemon, kinds: Default::default() });
        }
        let browsing = b.as_mut().expect("made");
        let (Ok(events), Some(plugins)) = (browsing.daemon.browse(&ty), self.plugins.get().cloned()) else { return };
        browsing.kinds.insert(kind.clone());
        crate::spawn("nori-plugin-mdns", move || {
            while let Ok(e) = events.recv() {
                let Some(p) = plugins.upgrade() else { return };
                match e {
                    mdns_sd::ServiceEvent::ServiceResolved(s) => {
                        let Some(ip) = crate::remote::address_of(&s) else { continue };
                        let name = s.fullname.strip_suffix(&format!(".{ty}")).unwrap_or(&s.fullname).to_string();
                        let txt = s.txt_properties.iter().map(|t| nori_plugins::TxtEntry { key: t.key().to_string(), value: t.val_str().to_string() }).collect();
                        p.discovered(kind.clone(), nori_plugins::Service { name, host: ip.to_string(), port: s.port, txt });
                    }
                    mdns_sd::ServiceEvent::SearchStopped(_) => return,
                    _ => {}
                }
            }
        });
    }

    /// The desktop holds no lock for multicast.
    fn multicast(&self, _: bool) {}

    fn cast(&self, cast: Option<nori_plugins::Cast>) {
        nori_core::alog::info(&format!("plugins: casting {cast:?}"));
        if let Some(to) = self.casts.lock().expect("not poisoned").as_ref() {
            to(cast);
        }
    }
    fn control(&self, control: nori_plugins::Control) {
        if let Some(s) = self.session() {
            s.plugin_control(control);
        }
    }
    /// The desktop holds no Wi-Fi lock.
    fn streaming(&self, _: bool) {}
    fn pressed(&self, press: nori_core::deciders::Press) {
        if let Some(s) = self.session() {
            s.pressed(press);
        }
    }
    fn sign_in(&self, id: String, url: String) -> bool {
        let opens = self.signs_in.lock().expect("not poisoned");
        if let Some(open) = opens.as_ref() {
            open(id, url);
        }
        opens.is_some()
    }
    fn wake(&self, _: Vec<nori_plugins::WakeEvent>) {}
    /// Linux: `fm.nori.Commands` on the session bus while any plugin has commands.
    fn commands(&self, commands: Vec<nori_plugins::Command>) {
        #[cfg(target_os = "linux")]
        {
            let mut held = self.commands.lock().expect("not poisoned");
            *held = None;
            let plugins = self.plugins.get().cloned();
            if !commands.is_empty() {
                let run = move |plugin: String, command: String| {
                    if let Some(p) = plugins.as_ref().and_then(std::sync::Weak::upgrade) {
                        p.command(nori_plugins::Command { plugin, command });
                    }
                };
                *held = nori_mpris::Commands::start(Box::new(run)).map_err(|e| nori_core::alog::info(&format!("plugin commands: {e}"))).ok();
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = commands;
    }
    /// A desktop's screen is on while it runs.
    fn watch_screen(&self, _: bool) {}
    /// Its line is read again with the page ([`nori_plugins::Plugins::assets`]).
    fn asset(&self, _: String, _: String, _: nori_plugins::AssetState) {
        self.tell_changed();
    }
}

/// Logs `draft` in and keeps it as the active profile, blocking ([`nori_core::providers::log_in`]).
pub fn log_in(data: &Path, http: Arc<Http>, draft: SavedServer) -> Result<SavedServer, NetError> {
    block_on(nori_core::providers::log_in(&plugins(data).kinds, http, &app().settings, draft))
}

/// The cpal device named `device` (else the system's default) at `volume` (0 to 1).
fn card(device: Option<&str>) -> CpalOutput {
    match device {
        Some(name) => CpalOutput::with_device(name),
        None => CpalOutput::new(),
    }
}

/// A desktop session's opening.
pub struct DesktopOpen<'a> {
    pub data: &'a Path,
    pub http: Arc<Http>,
    pub profile: SavedServer,
    /// The sound card by name; None for the system's default.
    pub device: Option<String>,
    /// 0 to 1.
    pub volume: f32,
    pub covers: Option<nori_covers::loader::Config>,
    pub offline: bool,
    /// The process's media controls, driven by this session while it is open.
    pub mpris: Option<Arc<nori_mpris::Mpris>>,
    /// This client, as the account's other devices list it.
    pub kind: DeviceKind,
    pub out: Out,
}

/// Opens the profile on the sound card, without touching the network ([`Session::open`]).
pub fn open(o: DesktopOpen) -> Result<Session, String> {
    let output = card(o.device.as_deref());
    let level = output.volume();
    let set = level.clone();
    let volume = Level::new(o.volume, Some(Box::new(move |v| set.set(v))));
    let device = o.device.clone();
    let plugged = plugins(o.data);
    let session = Session::open(Open {
        queue: app().clone(),
        data: o.data,
        http: o.http,
        profile: o.profile,
        output: Box::new(output),
        volume,
        memory_mb: 256,
        covers: o.covers,
        offline: o.offline,
        mpris: o.mpris,
        device: nori_core::remote::RemoteMe { name: crate::device_name(), kind: o.kind },
        discovery: None,
        tasks: crate::tasks(),
        keeps_music: true,
        decoders: Default::default(),
        out: o.out,
        kinds: plugged.kinds.clone(),
    })?;
    *plugged.watched.engine.lock().expect("not poisoned") = Some(Arc::downgrade(&session.engine));
    // A plugin's cast takes the music, and the sound card gets it back, at its volume, when it ends.
    let engine = Arc::downgrade(&session.engine);
    let (core, covers) = (session.core.clone(), session.covers.clone());
    plugged.host.set_cards(Box::new(move |to| {
        let Some(engine) = engine.upgrade() else { return };
        match to {
            nori_plugins::Card::Sink(sink) => {
                let (queue, core, covers) = (core.session.clone(), core.clone(), covers.clone());
                let cover = covers.map(|c| {
                    Box::new(move |art: &str, size: u32| {
                        let mut file = Vec::new();
                        c.kept(&core.cover_address(art.to_string(), nori_core::covers::cover_rendition(size)), &mut file).then_some(file)
                    }) as nori_engine::core::CoverFile
                });
                let shows = Arc::new(nori_engine::core::QueueShows { queue, cover });
                let (out, offload) = nori_engine::cast::card(sink, Some(shows));
                engine.set_card(out, offload, true)
            }
            nori_plugins::Card::Here => engine.set_card(Box::new(card(device.as_deref()).with_volume(level.clone())), None, false),
            nori_plugins::Card::Elsewhere { playing, position_ms } => engine.play_elsewhere(playing, position_ms.map(|ms| ms as i64)),
            nori_plugins::Card::Back => engine.come_back(),
        }
    }));
    *plugged.watched.session.lock().expect("not poisoned") = Some(session.weak());
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugins_page_shows_what_it_said_of_its_rows() {
        use nori_plugins::{Client, PluginCard, PluginRow, RowKind, Value};
        let w = Watched::default();
        w.status("p".into(), "state".into(), "Fine {0}".into(), vec!["now".into()]);
        w.shown("p".into(), "gone".into(), false, true);
        w.shown("p".into(), "dim".into(), true, false);
        let row = |id: &str| PluginRow { id: id.into(), kind: RowKind::Status, label: id.into(), options: Vec::new(), min: None, max: None, step: None, value: None, hint: None, unit: None, decimals: 0, url: None };
        let mut cards = [PluginCard { id: "p".into(), version: "1.0.0".into(), name: "P".into(), about: None, enabled: true, repo: None, held: false, points: Vec::new(), overrides: Vec::new(), net: Vec::new(), lan: false, bluetooth: false, permissions: Vec::new(), hooks: Vec::new(), wakes: Vec::new(), commands: Vec::new(), cache_bytes: 0, asset_bytes: 0, rows: vec![row("state"), row("gone"), row("dim")] }];
        let off = w.as_said(&mut cards);
        let rows: Vec<(&str, Option<&Value>)> = cards[0].rows.iter().map(|r| (r.id.as_str(), r.value.as_ref())).collect();
        assert_eq!(rows, [("state", Some(&Value::Text("Fine now".into()))), ("dim", None)], "a hidden row is left out");
        assert_eq!(off, std::collections::HashSet::from([("p".to_string(), "dim".to_string())]));
    }

    #[test]
    fn the_zone_is_tz_else_the_localtime_link() {
        let link = || Some("/usr/share/zoneinfo/Australia/Sydney".to_string());
        assert_eq!(zone(None, link()).as_deref(), Some("Australia/Sydney"));
        assert_eq!(zone(Some(":Europe/Berlin".into()), link()).as_deref(), Some("Europe/Berlin"));
        assert_eq!(zone(Some(String::new()), link()).as_deref(), Some("Australia/Sydney"), "an empty TZ is unset");
        assert_eq!(zone(None, Some("/etc/UTC".into())), None);
    }

    #[test]
    fn a_plugins_notice_reaches_the_client_in_its_words() {
        let dir = nori_testdir::TempDir::new("host-plugin-notice");
        let watched = Arc::new(Watched::default());
        let host = nori_plugins::open(dir.path().join("plugins"), nori_core::settings_store::Settings::new(), watched.clone());
        let _ = watched.plugins.set(Arc::downgrade(&host));
        let id = host.install_from(nori_plugins::testing::fixture("").to_string_lossy().into_owned()).unwrap();
        host.turn(id, true);
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = heard.clone();
        watched.hear(move |n| into.lock().unwrap().push(n));
        let entry = host.menu_entries(nori_plugins::ItemKind::Song, "en".into()).remove(0);
        let song = nori_core::Song { title: "Blue".into(), ..Default::default() };
        block_on(host.run_menu(entry, nori_plugins::Items::Songs(vec![song])));
        assert_eq!(*heard.lock().unwrap(), [Notice { plugin: nori_plugins::testing::FIXTURE.into(), problem: false, words: "Count letters".into() }]);
    }
}
