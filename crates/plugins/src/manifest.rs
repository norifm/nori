//! A plugin's manifest, as the SDK's `plugin!` keeps it in the wasm's `nori:plugin` custom section:
//! what it is, extends, may reach, overrides and shows. Read before any of the plugin runs.

use std::collections::BTreeMap;

use nori_plugin_index::section::{custom_section, Section};
pub use nori_plugin_index::section::{API, POINTS};

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Reverse DNS ("fm.nori.orbit").
    pub id: String,
    pub version: String,
    /// The `nori:plugin` major version it was built for.
    pub api: u32,
    /// Its extension points: only these of its exports are called.
    pub points: Vec<&'static str>,
    pub stage: Option<Slot>,
    /// Hosts it may send HTTP to; a leading `*.` takes in their subdomains.
    pub net: Vec<String>,
    /// Hosts whose cookies the sign-in page hands back.
    pub signin: Vec<String>,
    /// nori's values it overrides, by key ("playback.speed").
    pub overrides: Vec<String>,
    pub rows: Vec<RowSpec>,
    pub menu: Vec<MenuSpec>,
    /// The places in nori's screens it fills.
    pub places: Vec<Place>,
    /// The Bluetooth devices it reaches and is offered to control.
    pub devices: Vec<DeviceFilter>,
    pub permissions: Vec<Permission>,
    /// The events its `hooks` export hears.
    pub hooks: Vec<Hook>,
    /// What may start nori while it is closed, for it.
    pub wake: Vec<WakeEvent>,
    /// Commands other apps may run, each its `lifecycle.action` id.
    pub commands: Vec<String>,
    /// Its cache's quota, bytes.
    pub cache_bytes: u64,
    pub assets: Vec<AssetSpec>,
}

/// What a plugin may do beyond its own data and rows; the install sheet lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Permission {
    PlaybackRead,
    PlaybackControl,
    LibraryRead,
    LibraryWrite,
    Subsonic,
    Schedule,
}

/// An event of nori's a plugin hears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Hook {
    PlaybackStarted,
    PlaybackPaused,
    SongChanged,
    QueueChanged,
    FavoriteChanged,
    ProfileOpened,
    RouteChanged,
    DeviceConnected,
    SettingChanged,
    MediaButton,
    LyricLine,
    OutputPicker,
}

impl Hook {
    pub(crate) const ALL: [Hook; 12] = [Hook::PlaybackStarted, Hook::PlaybackPaused, Hook::SongChanged, Hook::QueueChanged, Hook::FavoriteChanged, Hook::ProfileOpened, Hook::RouteChanged, Hook::DeviceConnected, Hook::SettingChanged, Hook::MediaButton, Hook::LyricLine, Hook::OutputPicker];
}

/// What starts nori while it is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum WakeEvent {
    BluetoothConnected,
    Boot,
}

impl WakeEvent {
    pub(crate) const ALL: [WakeEvent; 2] = [WakeEvent::BluetoothConnected, WakeEvent::Boot];

    /// As `wake.want` and the `woke` hook name it.
    pub fn name(self) -> &'static str {
        match self {
            WakeEvent::BluetoothConnected => "bluetooth-connected",
            WakeEvent::Boot => "boot",
        }
    }
}

/// A large file its manifest names.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetSpec {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// Bluetooth devices a plugin reaches: one matches when all that is given matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFilter {
    /// `*` for any run of characters, ignoring case.
    pub name: Option<String>,
    pub service: Option<String>,
    pub vendor: Option<u16>,
}

impl DeviceFilter {
    pub fn matches(&self, d: &crate::BtDevice) -> bool {
        self.name.as_deref().is_none_or(|p| glob(&p.to_lowercase(), &d.name.to_lowercase()))
            && self.service.as_deref().is_none_or(|u| d.services.iter().any(|s| s.eq_ignore_ascii_case(u)))
            && self.vendor.is_none_or(|v| d.vendor == Some(v))
    }
}

/// Whether `text` is `pattern`, where `*` stands for any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else { return false };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else { return rest.is_empty() };
    for p in middle {
        match rest.find(p) {
            Some(at) => rest = &rest[at + p.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// An action it adds to nori's menus of `kinds`; `id` is its label's key in its strings.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuSpec {
    pub id: String,
    pub glyph: Glyph,
    pub kinds: Vec<ItemKind>,
}

/// A glyph of nori's own set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Glyph {
    Sparkle,
    Info,
    Share,
    Link,
    Search,
    Heart,
    Download,
    Add,
    Play,
    Tune,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ItemKind {
    Song,
    Album,
    Artist,
    Playlist,
}

/// A place in nori's screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Place {
    Artwork,
    SongInfo,
    PlayerStatus,
    /// Its rows, on the devices sheet ("Playing on") too.
    Devices,
}

pub use nori_player::stage::Slot;

/// One row of its page, drawn by each client with its own components.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSpec {
    /// Its key in the plugin's strings, and its value's in its store.
    pub id: String,
    pub kind: RowKind,
    /// A choice's options: keys in its strings.
    pub options: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// A switch's 0 or 1, a choice's index, a level's value.
    pub default: Option<f64>,
    /// A key in its strings: a quiet line under the label.
    pub hint: Option<String>,
    /// A key in its strings: a level's unit.
    pub unit: Option<String>,
    /// Digits a level's value is shown with.
    pub decimals: u8,
    /// A link row's address.
    pub url: Option<String>,
}

impl RowSpec {
    /// Its default as a value of its kind.
    pub(crate) fn initial(&self) -> Option<crate::Value> {
        let d = self.default?;
        Some(match self.kind {
            RowKind::Switch => crate::Value::Switch(d != 0.0),
            RowKind::Choice => crate::Value::Choice(d as u32),
            _ => crate::Value::Level(d),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RowKind {
    Switch,
    Choice,
    Level,
    /// One line; masked and kept out of logs.
    Secret,
    Text,
    Action,
    Status,
    Section,
    Note,
    /// Opens its `url`.
    Link,
}

impl RowKind {
    const ALL: [RowKind; 10] = [RowKind::Switch, RowKind::Choice, RowKind::Level, RowKind::Secret, RowKind::Text, RowKind::Action, RowKind::Status, RowKind::Section, RowKind::Note, RowKind::Link];

    /// Whether its value is a number a sound stage reads.
    pub fn numeric(self) -> bool {
        matches!(self, RowKind::Switch | RowKind::Choice | RowKind::Level)
    }
}

/// Why a plugin can't be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum Refusal {
    /// Its package, wasm or manifest can't be read.
    Unreadable(String),
    /// Built for another plugin API.
    Api(u32),
    /// It needs host interfaces this nori lacks (their WIT names).
    Needs(Vec<String>),
    /// Its id isn't reverse DNS.
    Id,
    /// It would override a value nori doesn't have.
    UnknownKey(String),
    /// Its repo lists a signature that isn't its author's key's over this package.
    Signature,
    /// It isn't signed by the key the installed plugin was first installed with: it installs only
    /// once the plugin is removed.
    OtherKey,
}

/// For logs (uniffi gives no message).
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Refusal {}

impl Manifest {
    /// The manifest in `wasm`'s `nori:plugin` section.
    pub fn of_wasm(wasm: &[u8]) -> Result<Manifest, Refusal> {
        Manifest::of_section(&Manifest::section(wasm)?)
    }

    /// `wasm`'s `nori:plugin` section, as installing keeps it beside the plugin (`manifest.bin`).
    pub(crate) fn section(wasm: &[u8]) -> Result<Vec<u8>, Refusal> {
        custom_section(wasm, "nori:plugin").ok_or_else(|| Refusal::Unreadable("no nori:plugin section (built without the SDK's plugin!)".into()))
    }

    /// The manifest kept beside an installed plugin, whatever API it was built for: one left behind by a
    /// new nori stays listed, with its store, until its update replaces it.
    pub(crate) fn of_installed(section: &[u8]) -> Option<Manifest> {
        Manifest::decode(section)
    }

    /// The manifest in a `nori:plugin` section's bytes.
    pub(crate) fn of_section(section: &[u8]) -> Result<Manifest, Refusal> {
        let m = Manifest::decode(section).ok_or_else(|| Refusal::Unreadable("its manifest section can't be read".into()))?;
        if m.api != API {
            return Err(Refusal::Api(m.api));
        }
        let parts: Vec<&str> = m.id.split('.').collect();
        if parts.len() < 2 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
            return Err(Refusal::Id);
        }
        if let Some(k) = m.overrides.iter().find(|k| !nori_settings::overrides::overridable(k)) {
            return Err(Refusal::UnknownKey(k.clone()));
        }
        let hex = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if let Some(a) = m.assets.iter().find(|a| !hex(&a.sha256) || a.name.is_empty() || !a.name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) || a.name.starts_with('.')) {
            return Err(Refusal::Unreadable(format!("asset {:?} needs a plain name and a sha256", a.name)));
        }
        Ok(m)
    }

    fn decode(bytes: &[u8]) -> Option<Manifest> {
        let w = Section::read(bytes)?;
        let stage = match w.stage {
            1 => Some(Slot::Song),
            2 => Some(Slot::Master),
            _ => None,
        };
        let rows = w
            .rows
            .iter()
            .map(|r| {
                Some(RowSpec {
                    id: r.id.clone(),
                    kind: *RowKind::ALL.get(r.kind as usize)?,
                    options: r.options.clone(),
                    min: r.min,
                    max: r.max,
                    step: r.step,
                    default: r.default,
                    hint: r.hint.clone(),
                    unit: r.unit.clone(),
                    decimals: r.decimals,
                    url: r.url.clone(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        const GLYPHS: [Glyph; 10] = [Glyph::Sparkle, Glyph::Info, Glyph::Share, Glyph::Link, Glyph::Search, Glyph::Heart, Glyph::Download, Glyph::Add, Glyph::Play, Glyph::Tune];
        const KINDS: [ItemKind; 4] = [ItemKind::Song, ItemKind::Album, ItemKind::Artist, ItemKind::Playlist];
        const PLACES: [Place; 4] = [Place::Artwork, Place::SongInfo, Place::PlayerStatus, Place::Devices];
        let menu = w
            .menu
            .iter()
            .map(|a| Some(MenuSpec { id: a.id.clone(), glyph: *GLYPHS.get(a.glyph as usize)?, kinds: a.kinds.iter().map(|k| KINDS.get(*k as usize).copied()).collect::<Option<_>>()? }))
            .collect::<Option<Vec<_>>>()?;
        let places = w.places.iter().map(|p| PLACES.get(*p as usize).copied()).collect::<Option<Vec<_>>>()?;
        let devices = w.devices.iter().map(|d| DeviceFilter { name: d.name.clone(), service: d.service.clone(), vendor: u16::try_from(d.vendor).ok() }).collect();
        let points = w.points();
        let mut m = Manifest { points, stage, rows, menu, places, devices, id: w.id, version: w.version, api: w.api, net: w.net, signin: w.signin, overrides: w.overrides, permissions: Vec::new(), hooks: Vec::new(), wake: Vec::new(), commands: Vec::new(), cache_bytes: 0, assets: Vec::new() };
        let Some(later) = w.later else { return Some(m) };
        const PERMISSIONS: [Permission; 6] = [Permission::PlaybackRead, Permission::PlaybackControl, Permission::LibraryRead, Permission::LibraryWrite, Permission::Subsonic, Permission::Schedule];
        // Kinds a newer SDK adds are skipped.
        m.permissions = later.permissions.iter().filter_map(|b| PERMISSIONS.get(*b as usize).copied()).collect();
        m.hooks = later.hooks.iter().filter_map(|b| Hook::ALL.get(*b as usize).copied()).collect();
        m.wake = later.wake.iter().filter_map(|b| WakeEvent::ALL.get(*b as usize).copied()).collect();
        m.commands = later.commands;
        m.cache_bytes = later.cache_mb as u64 * 1024 * 1024;
        m.assets = later.assets.into_iter().map(|a| AssetSpec { name: a.name, url: a.url, size: a.size, sha256: a.sha256 }).collect();
        Some(m)
    }

    pub fn may(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }

    pub fn hears(&self, h: Hook) -> bool {
        self.extends("hooks") && self.hooks.contains(&h)
    }

    /// Whether it reaches Bluetooth devices.
    pub fn reaches_devices(&self) -> bool {
        !self.devices.is_empty()
    }

    /// Whether Bluetooth device `d` is among those it reaches.
    pub fn reaches_device(&self, d: &crate::BtDevice) -> bool {
        self.devices.iter().any(|f| f.matches(d))
    }

    pub fn extends(&self, point: &str) -> bool {
        self.points.contains(&point)
    }

    /// Its numeric rows' values, in order, from what is stored (or their defaults).
    pub fn row_numbers(&self, stored: &BTreeMap<String, f64>) -> Vec<f64> {
        self.rows.iter().filter(|r| r.kind.numeric()).map(|r| stored.get(&r.id).copied().or(r.default).unwrap_or(0.0)).collect()
    }

    /// Whether it may reach private addresses: `lan` among its hosts.
    pub fn lan(&self) -> bool {
        self.net.iter().any(|d| d == crate::lan::LAN)
    }

    /// Whether HTTP to `host` is allowed by name (`lan` is no host's name).
    pub fn reaches(&self, host: &str) -> bool {
        among(self.net.iter().filter(|d| *d != crate::lan::LAN), host)
    }

    /// Whether a sign-in page may start at `host`.
    pub fn signs_in(&self, host: &str) -> bool {
        among(&self.signin, host)
    }
}

/// Whether `host` is one of `hosts`, where a leading `*.` takes in subdomains.
fn among<'a>(hosts: impl IntoIterator<Item = &'a String>, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    !host.is_empty()
        && hosts.into_iter().any(|d| match d.strip_prefix("*.") {
            Some(base) => host == base || host.ends_with(&format!(".{base}")),
            None => host == *d,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_plugin_sdk::manifest as sdk;

    /// A manifest as the SDK writes it, without a wasm around it.
    fn written(m: &sdk::Manifest, points: u32) -> Manifest {
        let bytes: [u8; 1024] = sdk::encode::<1024>(m, "1.2.3", points);
        Manifest::decode(&bytes[..sdk::encoded_len(m, "1.2.3")]).unwrap()
    }

    #[test]
    fn what_the_sdk_writes_reads_back() {
        const ROWS: [sdk::Row; 3] = [sdk::Row::level("speed", 0.02, 1.0, 0.01, 0.12).hint("speed_hint").unit("turns"), sdk::Row::choice("mode", &["a", "b"], 1), sdk::Row::link("site", "https://nori.fm")];
        const MENU: [sdk::MenuAction; 1] = [sdk::MenuAction { id: "look_up", glyph: sdk::Glyph::Search, kinds: &[sdk::ItemKind::Song, sdk::ItemKind::Album] }];
        let m = sdk::Manifest { stage: Some(sdk::Slot::Master), net: &["music.youtube.com"], overrides: &["playback.speed"], rows: &ROWS, menu: &MENU, places: &[sdk::Place::Artwork, sdk::Place::Devices], ..sdk::Manifest::new("fm.nori.orbit") };
        let read = written(&m, 1 << 3);
        assert_eq!((read.id.as_str(), read.version.as_str(), read.api, read.stage), ("fm.nori.orbit", "1.2.3", API, Some(Slot::Master)));
        assert_eq!(read.points, vec!["sound-stage"]);
        assert_eq!((&read.net, &read.overrides), (&vec!["music.youtube.com".to_string()], &vec!["playback.speed".to_string()]));
        assert_eq!(read.rows[0], RowSpec { id: "speed".into(), kind: RowKind::Level, options: Vec::new(), min: Some(0.02), max: Some(1.0), step: Some(0.01), default: Some(0.12), hint: Some("speed_hint".into()), unit: Some("turns".into()), decimals: 2, url: None });
        assert_eq!((read.rows[1].kind, read.rows[1].options.len(), read.rows[1].default, read.rows[1].min), (RowKind::Choice, 2, Some(1.0), None));
        assert_eq!(read.row_numbers(&BTreeMap::from([("speed".to_string(), 0.5)])), vec![0.5, 1.0]);
        assert_eq!((read.rows[2].kind, read.rows[2].url.as_deref()), (RowKind::Link, Some("https://nori.fm")));
        assert_eq!(read.menu, vec![MenuSpec { id: "look_up".into(), glyph: Glyph::Search, kinds: vec![ItemKind::Song, ItemKind::Album] }]);
        assert_eq!(read.places, vec![Place::Artwork, Place::Devices]);
    }

    #[test]
    fn session_hooks_wakes_and_storage_read_back() {
        const ASSETS: [sdk::Asset; 1] = [sdk::Asset { name: "model.bin", url: "https://x.example/m", size: 5 << 30, sha256: "ab" }];
        let m = sdk::Manifest {
            permissions: &[sdk::Permission::PlaybackControl, sdk::Permission::Schedule],
            hooks: &[sdk::Hook::SongChanged, sdk::Hook::LyricLine],
            wake: &[sdk::Wake::Boot],
            commands: &["toggle"],
            cache_mb: 3,
            assets: &ASSETS,
            ..sdk::Manifest::new("fm.nori.hooked")
        };
        let read = written(&m, 1 << 1);
        assert_eq!((read.permissions.clone(), read.hooks.clone(), read.wake.clone(), read.commands.clone(), read.cache_bytes), (vec![Permission::PlaybackControl, Permission::Schedule], vec![Hook::SongChanged, Hook::LyricLine], vec![WakeEvent::Boot], vec!["toggle".to_string()], 3 << 20));
        assert_eq!(read.assets.clone(), vec![AssetSpec { name: "model.bin".into(), url: "https://x.example/m".into(), size: 5 << 30, sha256: "ab".into() }]);
        assert!(read.hears(Hook::LyricLine) && !read.hears(Hook::QueueChanged));
    }

    #[test]
    fn the_points_are_wits_in_its_order() {
        let mut resolve = wit_parser::Resolve::default();
        let (pkg, _) = resolve.push_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit")).unwrap();
        let world = resolve.select_world(&[pkg], Some("plugin")).unwrap();
        let names: Vec<String> = resolve.worlds[world]
            .exports
            .values()
            .filter_map(|i| match i {
                wit_parser::WorldItem::Interface { id, .. } => resolve.interfaces[*id].name.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(names, POINTS.to_vec());
    }

    #[test]
    fn devices_match_by_name_service_and_vendor() {
        const FILTERS: [sdk::DeviceFilter; 2] = [sdk::DeviceFilter::ANY.name("WH-1000XM*"), sdk::DeviceFilter::ANY.service("0000fe2c-0000-1000-8000-00805f9b34fb").vendor(0x054c)];
        let m = written(&sdk::Manifest { devices: &FILTERS, ..sdk::Manifest::new("fm.nori.sony") }, 0);
        assert_eq!(m.devices[1].vendor, Some(0x054c));
        let device = |name: &str, services: &[&str], vendor| crate::BtDevice { id: "a".into(), name: name.into(), services: services.iter().map(|s| s.to_string()).collect(), vendor, product: None };
        for (d, reached) in [
            (device("WH-1000XM5", &[], None), true),
            (device("wh-1000xm4 (living room)", &[], None), true),
            (device("WF-1000XM5", &[], None), false),
            (device("Headset", &["0000FE2C-0000-1000-8000-00805F9B34FB"], Some(0x054c)), true),
            (device("Headset", &["0000fe2c-0000-1000-8000-00805f9b34fb"], Some(0x004c)), false),
        ] {
            assert_eq!(m.reaches_device(&d), reached, "{}", d.name);
        }
        let none = written(&sdk::Manifest::new("fm.nori.none"), 0);
        assert!(!none.reaches_devices() && !none.reaches_device(&device("WH-1000XM5", &[], None)));
    }

    #[test]
    fn manifests_of_each_older_sdk_read_with_none_of_what_came_after() {
        const FILTERS: [sdk::DeviceFilter; 1] = [sdk::DeviceFilter::ANY.name("WH-*")];
        // The bytes an empty group takes: the devices' count, then the permissions', hooks' and wakes'
        // counts, the commands' count, the cache quota and the assets' count.
        let (devices, session) = (2, 2 * 3 + 2 + 4 + 2);
        let full = sdk::Manifest { places: &[sdk::Place::Devices], devices: &FILTERS, hooks: &[sdk::Hook::SongChanged], ..sdk::Manifest::new("fm.nori.new") };
        for (m, cut, has_devices, has_hooks) in [
            // Released plugins: nothing after the places.
            (sdk::Manifest { places: &[sdk::Place::Devices], ..sdk::Manifest::new("fm.nori.old") }, devices + session, false, false),
            // Built with devices, before the session's fields.
            (sdk::Manifest { places: &[sdk::Place::Devices], devices: &FILTERS, ..sdk::Manifest::new("fm.nori.devices") }, session, true, false),
            (full, 0, true, true),
        ] {
            let bytes: [u8; 1024] = sdk::encode::<1024>(&m, "1.0.0", 1 << 1);
            let read = Manifest::decode(&bytes[..sdk::encoded_len(&m, "1.0.0") - cut]).unwrap();
            assert_eq!(read.places, vec![Place::Devices], "{}", m.id);
            assert_eq!(read.reaches_devices(), has_devices, "{}", m.id);
            assert_eq!(read.hears(Hook::SongChanged), has_hooks, "{}", m.id);
            assert!(read.permissions.is_empty() && read.assets.is_empty() && read.cache_bytes == 0, "{}", m.id);
        }
    }

    #[test]
    fn globs_take_any_run_for_a_star() {
        for (p, t, ok) in [("a*c", "abbc", true), ("a*c", "ab", false), ("*", "", true), ("ab", "abc", false), ("*b*", "abc", true), ("a*a", "a", false)] {
            assert_eq!(glob(p, t), ok, "{p} {t}");
        }
    }

    #[test]
    fn http_reaches_only_its_hosts() {
        let m = written(&sdk::Manifest { net: &["music.youtube.com", "*.googlevideo.com"], ..sdk::Manifest::new("fm.nori.yt") }, 0);
        for (host, ok) in [("music.youtube.com", true), ("MUSIC.youtube.com", true), ("youtube.com", false), ("r3.googlevideo.com", true), ("googlevideo.com", true), ("evilgooglevideo.com", false)] {
            assert_eq!(m.reaches(host), ok, "{host}");
        }
    }
}
