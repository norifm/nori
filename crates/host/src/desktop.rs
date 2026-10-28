//! What the terminal and desktop clients share around their session: the app's queue session over its
//! settings, their own settings kept as app values, logging in, the sound card (cpal) and the session
//! opened over them.

use std::path::Path;
use std::sync::Arc;

use nori_core::remote::wire::DeviceKind;
use nori_core::settings::SavedServer;
use nori_core::transport::{block_on, NetError};
use nori_engine::AudioOutput;
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
        Plugged { kinds: nori_core::providers::Kinds::with_plugins(host.clone()), host, watched }
    })
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

/// What a desktop client does with what plugins say: the open session takes the settings laid anew;
/// notices go to the client that hears them ([`Watched::hear`]); status lines go to the log.
#[derive(Default)]
pub struct Watched {
    engine: std::sync::Mutex<Option<std::sync::Weak<nori_engine::Engine>>>,
    plugins: std::sync::OnceLock<std::sync::Weak<nori_plugins::Plugins>>,
    heard: std::sync::Mutex<Option<Hears>>,
}

impl Watched {
    /// Sends the plugins' notices to `to`, from whatever thread a plugin runs on.
    pub fn hear(&self, to: impl Fn(Notice) + Send + Sync + 'static) {
        *self.heard.lock().expect("not poisoned") = Some(Box::new(to));
    }

    /// Plugin `id`'s word `key` with `args` in its `{0}`, `{1}`…; the key itself where it has no such word.
    fn said(&self, id: &str, key: String, args: &[String]) -> String {
        let word = self.plugins.get().and_then(std::sync::Weak::upgrade).and_then(|p| p.word(id.into(), key.clone(), "en".into()));
        args.iter().enumerate().fold(word.unwrap_or(key), |line, (i, a)| line.replace(&format!("{{{i}}}"), a))
    }
}

impl nori_plugins::Client for Watched {
    fn layer_changed(&self) {
        let engine = self.engine.lock().expect("not poisoned").as_ref().and_then(std::sync::Weak::upgrade);
        if let (Some(engine), Some(prefs)) = (engine, app().settings.current()) {
            engine.set_settings(nori_engine::core::settings(&prefs, app().settings.layer()));
        }
    }
    fn status(&self, id: String, row: String, words: String, args: Vec<String>) {
        nori_core::alog::info(&format!("plugin {id}: {row} is {words} {args:?}"));
    }
    fn shown(&self, _: String, _: String, _: bool, _: bool) {}
    fn notice(&self, id: String, problem: bool, words: String, args: Vec<String>) {
        let words = self.said(&id, words, &args);
        nori_core::alog::info(&format!("plugin {id} says{}: {words}", if problem { " (a problem)" } else { "" }));
        if let Some(to) = self.heard.lock().expect("not poisoned").as_ref() {
            to(Notice { plugin: id, problem, words });
        }
    }
}

/// Logs `draft` in and keeps it as the active profile, blocking ([`nori_core::providers::log_in`]).
pub fn log_in(data: &Path, http: Arc<Http>, draft: SavedServer) -> Result<SavedServer, NetError> {
    block_on(nori_core::providers::log_in(&plugins(data).kinds, http, &app().settings, draft))
}

/// The cpal device named `device` (else the system's default) at `volume` (0 to 1).
fn sound(device: Option<&str>, volume: f32) -> (Box<dyn AudioOutput>, Arc<Level>) {
    let card = match device {
        Some(name) => CpalOutput::with_device(name),
        None => CpalOutput::new(),
    };
    let level = card.volume();
    (Box::new(card), Level::new(volume, Some(Box::new(move |v| level.set(v)))))
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
    let (output, volume) = sound(o.device.as_deref(), o.volume);
    let plugged = plugins(o.data);
    let session = Session::open(Open {
        queue: app().clone(),
        data: o.data,
        http: o.http,
        profile: o.profile,
        output,
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
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

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
