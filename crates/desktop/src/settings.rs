//! Settings pages: the core's (`settings_pages`, desktop client), worded and drawn. Rows report back the
//! setting name and the picked value.

use std::collections::{HashMap, HashSet};
use nori_core::numbers::{self, POINT};
use nori_core::settings::{season_follow_value, season_of, EqLevel, Level, StoredPrefs, ThemeSeason};
use nori_core::settings_model::{self, BeatModel, EqStatus, KeyAction, SettingKind, SettingsState};
use nori_core::settings_pages::{self, PageFacts, RowItem, SectionId, SettingsAction, SettingsClient, SettingsLink, SettingsNote, SettingsPage, SettingsRow};
use slint::{ModelRc, SharedString, VecModel};

use nori_look::theme;

use nori_core::providers::flow::WelcomeSource;
use nori_core::providers::Kinds;
use nori_core::settings::{ProfileIcon, SavedServer, ServerKind};
use nori_plugins::{AssetLine, AssetState, Category, ControlledDevice, Offer, OfferState, Offers, PluginCard, PluginRow, ReadmeBlock, Repo, RowKind, Value};

use crate::look::Today;
use crate::{words, IconChoice, IconPicker, Mark, MarkGlyph, PluginTool, PluginTop, SettingRow};

// Row kinds, as app.slint numbers them.
const HEADING: i32 = 0;
const TOGGLE: i32 = 1;
const CHOICE: i32 = 2;
const ACTION: i32 = 3;
const INFO: i32 = 4;
const SLIDER: i32 = 5;
pub(crate) const RANKED: i32 = 6;
pub(crate) const SERVER: i32 = 8;
const NOTE: i32 = 9;
const LINK: i32 = 11;
const BUTTON: i32 = 12;
/// The season tiles, and the four modes.
const SEASONS: i32 = 13;
const APPEARANCE_ROW: i32 = 14;
/// What is stored, as a bar of its parts.
const USAGE: i32 = 15;
/// A plugin in a list: its tile and lines, then a switch or a button (`control`).
const PLUGIN: i32 = 17;
/// A plugin's README, block by block.
const README_HEADING: i32 = 18;
const README_TEXT: i32 = 19;
const README_ITEM: i32 = 20;
const README_CODE: i32 = 21;
/// A device a plugin controls: the headset, its name and its batteries.
const CONTROLLED: i32 = 22;
/// One of a few, as chips sharing the row.
const SEGMENTS: i32 = 23;
/// Any of a few, as chips sharing the row, each lit while on (`lit`); a chip sends its `values` name.
const FLAGS: i32 = 24;
/// Opens a page of a plugin's: its label over its hint, a chevron at the end.
const PAGE: i32 = 25;
/// A fact: its title over its line, a word at its end (`button`), pressed where it has a name.
const FACT: i32 = 26;

/// The appearance row's name: the theme setting, offered as segments.
pub const APPEARANCE: &str = "appearance";

/// The `theme` value a pick stands for: Follow system, Day, Night or Midnight.
pub fn appearance_theme(pick: usize) -> &'static str {
    match pick {
        1 => "LIGHT",
        2 => "DARK",
        3 => "MIDNIGHT",
        _ => "SYSTEM",
    }
}

/// The heading over the season tiles.
const SEASON_GROUP: &str = "Season";

/// The season tiles and the "follow the seasons" switch under them.
struct Seasons {
    /// The tile worn (0 spring to 3 winter): the calendar's while following.
    worn: i32,
    following: bool,
    /// The switch's row name: the setting and the value flipping it sets ([`toggled`]).
    switch: String,
    /// "Winter comes in on November 7" while following.
    detail: String,
}

const SEASON_NAMES: [&str; 4] = ["Spring", "Summer", "Autumn", "Winter"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn seasons(p: &StoredPrefs, today: Today) -> Seasons {
    let now = theme::season_now(today.month, today.day, today.southern);
    let following = p.season == ThemeSeason::Follow;
    let worn = season_of(p.season).unwrap_or(now.season) as i32;
    let detail = if following {
        format!("{} comes in on {} {}", SEASON_NAMES[now.next as usize], MONTHS[now.next_month as usize - 1], now.next_day)
    } else {
        String::new()
    };
    let switch = format!("season={}", season_follow_value(!following, today.month, today.day, today.southern));
    Seasons { worn, following, switch, detail }
}

/// Which appearance the settings stand for ([`appearance_theme`]'s pick).
fn appearance_pick(p: &StoredPrefs) -> i32 {
    use nori_core::settings::ThemeMode;
    match p.theme {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
        ThemeMode::Midnight => 3,
    }
}

/// Non-setting facts the pages show (sizes, counts, devices).
#[derive(Default, Clone)]
pub struct Facts {
    pub indexed: (u32, u32, u32),
    pub stream_bytes: u64,
    pub cover_bytes: u64,
    pub lyrics_bytes: u64,
    /// Every profile's downloads: they share the computer's download folder.
    pub download_bytes: u64,
    pub all_download_songs: u32,
    /// Each profile with downloads (the id its rows are kept under) and what they hold.
    pub download_profiles: Vec<(String, u64)>,
    pub database_bytes: u64,
    pub folders: Vec<(String, String)>,
    pub devices: Vec<String>,
    pub device: String,
    pub syncing: bool,
    /// Changes waiting for the server, and dropped unsent.
    pub unsent: (u32, u32),
    /// What the open profile offers: a jam guest's settings are this computer's own.
    pub rules: nori_core::browse::ProfileRules,
}

/// Row name of the output device choice (a desktop setting, not the core's).
const DEVICE: &str = "!device";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    Setting(&'a str),
    Device,
}

impl<'a> Target<'a> {
    pub fn of(name: &'a str) -> Target<'a> {
        if name == DEVICE { Target::Device } else { Target::Setting(name) }
    }
}

const EQUALIZER: &str = "equalizer";
const COPY_REPORT: &str = "copy-report";
const LICENCES: &str = "licences";
/// A credit's row on the licenses page is `licence:<its place in the page>`.
const LICENCE: &str = "licence:";
const ADD_SERVER: &str = "add-server";
const BACKUP: &str = "backup";
const RESTORE: &str = "restore";
const CHECK_UPDATE: &str = "check-update";
const BACKUP_PASSWORD: &str = "backup-password";
const RESTORE_PASSWORD: &str = "restore-password";

/// The name of the sheet asking for the password of the backup at `path`.
pub fn backup_password(restore: bool, path: &str) -> String {
    format!("{}:{path}", if restore { RESTORE_PASSWORD } else { BACKUP_PASSWORD })
}
/// A folder's toggle is named `musicFolders=<the value it stores>`.
const MUSIC_FOLDERS: &str = "musicFolders";

/// The setting and value a toggle row stores: its own `true`/`false`, or the value its name carries.
pub fn toggled(name: &str, on: bool) -> (&str, String) {
    name.split_once('=').map_or((name, on.to_string()), |(name, value)| (name, value.to_string()))
}

/// A link or button on the settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Equalizer,
    /// About's facts copied, for a bug report.
    CopyReport,
    /// The licenses page.
    Licences,
    /// A credit's license, by its place on the licenses page.
    Licence(usize),
    AddServer,
    /// A server row's Edit: the profile's page, its form, account and scrobbling.
    Edit(String),
    /// A singular server row's Merge with…: picks another saved profile (or a new sign-in) to merge in.
    Merge(String),
    /// A merged server row's Sources: opens the dialog editing, reordering, removing or adding its sources.
    Sources(String),
    /// A server row's Rename: its name asked for.
    Rename(String),
    /// A server row's Remove: asked first ([`REMOVE_PROFILE`]).
    Remove(String),
    /// Switch to the server with this id.
    Server(String),
    Chore(Chore),
    /// Asks where to save a backup, or which one to restore.
    Backup { restore: bool },
    /// Asks for the password of the backup at `path`.
    BackupPassword { restore: bool, path: String },
    CheckUpdate,
}

impl Act {
    /// Parses a row name; server rows are `server:<id>`.
    pub fn of(name: &str) -> Option<Act> {
        if let Some(id) = name.strip_prefix("server:") {
            return Some(Act::Server(id.to_string()));
        }
        if let Some(id) = name.strip_prefix("edit:") {
            return Some(Act::Edit(id.to_string()));
        }
        if let Some(id) = name.strip_prefix("merge:") {
            return Some(Act::Merge(id.to_string()));
        }
        if let Some(id) = name.strip_prefix("sources:") {
            return Some(Act::Sources(id.to_string()));
        }
        if let Some(id) = name.strip_prefix("rename:") {
            return Some(Act::Rename(id.to_string()));
        }
        if let Some(id) = name.strip_prefix("remove:") {
            return Some(Act::Remove(id.to_string()));
        }
        if let Some(at) = name.strip_prefix(LICENCE) {
            return at.parse().ok().map(Act::Licence);
        }
        if let Some((what, path)) = name.split_once(':').filter(|(what, _)| matches!(*what, BACKUP_PASSWORD | RESTORE_PASSWORD)) {
            return Some(Act::BackupPassword { restore: what == RESTORE_PASSWORD, path: path.to_string() });
        }
        match name {
            EQUALIZER => Some(Act::Equalizer),
            COPY_REPORT => Some(Act::CopyReport),
            LICENCES => Some(Act::Licences),
            ADD_SERVER => Some(Act::AddServer),
            BACKUP => Some(Act::Backup { restore: false }),
            RESTORE => Some(Act::Backup { restore: true }),
            CHECK_UPDATE => Some(Act::CheckUpdate),
            _ => CHORES.into_iter().find(|c| chore_name(*c) == name).map(Act::Chore),
        }
    }
}

pub use nori_host::session::Chore;

const CHORES: [Chore; 5] = [Chore::SyncLibrary, Chore::DownloadLibrary, Chore::ClearStream, Chore::ClearCovers, Chore::ClearLyrics];

/// A chore's row name in the UI.
fn chore_name(c: Chore) -> &'static str {
    match c {
        Chore::SyncLibrary => "sync-library",
        Chore::DownloadLibrary => "download-library",
        Chore::ClearStream => "clear-stream",
        Chore::ClearCovers => "clear-covers",
        Chore::ClearLyrics => "clear-lyrics",
    }
}

#[derive(Default)]
struct Row {
    kind: i32,
    name: String,
    title: String,
    detail: String,
    on: bool,
    enabled: bool,
    options: Vec<(String, String)>,
    /// A button's words; a slider's value.
    button: String,
    value: f32,
    min: f32,
    max: f32,
    /// A ranked row's place (a lyrics service's, a profile's), and whether a lyrics service is switched on
    /// only once given a key it lacks.
    rank: i32,
    keyed: bool,
    /// A usage row's parts, as fractions of the whole.
    shares: Vec<f32>,
    /// A usage row's downloads by profile, worded.
    breakdown: Vec<String>,
    /// The option picked, where no setting holds it (a plugin's row).
    chosen: Option<i32>,
    /// A plugin row's third line, and what its switch or button sends.
    line: String,
    control: String,
    /// A plugin row's icon and the letter drawn without one, or a profile row's circle.
    icon: slint::Image,
    letter: String,
    /// The glyph the tile holds instead: nori's own provider's server, or a profile's face.
    glyph: MarkGlyph,
    /// A profile row's account picture.
    picture: slint::Image,
    /// A flags row's options that are on.
    lit: Vec<bool>,
}

/// The profile whose rows are kept under `profile` (`server_db`), as the server list names it.
fn profile_name(p: &StoredPrefs, kinds: &Kinds, profile: &str) -> String {
    p.servers.iter().find(|sv| nori_core::settings::server_db_id(&sv.id) == profile).map_or_else(
        || "A removed profile".into(),
        |sv| Some(nori_core::settings::label(&sv.name, &sv.url)).filter(|l| !l.is_empty()).unwrap_or_else(|| crate::words::provider_of(kinds, &sv.kind).name),
    )
}

fn options(name: &str) -> Vec<String> {
    settings_model::specs().into_iter().find(|s| s.name == name).map(|s| s.options).unwrap_or_default()
}

/// A loudness level's target: "−18 LUFS".
fn lufs(l: Level) -> String {
    format!("{} LUFS", minus(&numbers::fixed(l.lufs() as f64, 0, false, POINT)))
}

fn minus(v: &str) -> String {
    v.replace('-', "−")
}

/// A category of Settings: one of the core's pages, the plugins', or the Support page, which opens over
/// Settings as Android's row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Page(SettingsPage),
    Plugins,
    Support,
}

/// The desktop's categories: the pages the profile in use opens, in the core's order (`pages`), and
/// Plugins before About, as Android lists them; a build with a support service ends with Support.
pub fn tabs(parts: &[nori_core::settings_pages::SettingsPart]) -> Vec<Tab> {
    let mut tabs: Vec<Tab> = settings_pages::pages(SettingsClient::Desktop, parts).into_iter().map(Tab::Page).collect();
    let about = tabs.iter().position(|t| *t == Tab::Page(SettingsPage::About)).unwrap_or(tabs.len());
    tabs.insert(about, Tab::Plugins);
    if nori_core::support::support_service().is_some() {
        tabs.push(Tab::Support);
    }
    tabs
}

/// A category's name, as Android's settings list them.
pub fn tab_name(tab: Tab) -> &'static str {
    let page = match tab {
        Tab::Page(page) => page,
        Tab::Plugins => return words::PLUGINS,
        Tab::Support => return words::SUPPORT_TITLE,
    };
    match page {
        SettingsPage::Servers => "Profiles",
        SettingsPage::Playing => "Playback",
        SettingsPage::Sound => "Sound",
        SettingsPage::Look => "Appearance",
        SettingsPage::Lyrics | SettingsPage::LyricsSources => "Lyrics",
        SettingsPage::Library => "Library",
        SettingsPage::Data => "Downloads and storage",
        SettingsPage::About => "About",
    }
}

/// The settings page tab `tab` shows; Appearance where it shows none.
fn tab_page(f: &Facts, tab: i32) -> SettingsPage {
    match tabs(&f.rules.settings).get(tab.max(0) as usize) {
        Some(Tab::Page(page)) => *page,
        _ => SettingsPage::Look,
    }
}

fn section_title(id: SectionId) -> &'static str {
    use SectionId::*;
    match id {
        Accounts => "Accounts",
        MusicFolders => "This profile",
        ThisServer => "This profile",
        Between => "Between songs",
        Queue => "Queue",
        Equalizer => "Equalizer",
        Effects => "Effects",
        Volume => "Volume",
        Output => "Output",
        Theme => "Theme",
        Home => "Layout",
        Screen => "Screen",
        Motion => "Motion",
        Display => "Display",
        Sources => "Sources",
        AskedOrder => "In the order they're asked",
        Lists => "Songs in lists",
        Search => "Search",
        History => "Listening history",
        Online => "Online",
        Devices => "Other devices",
        Streaming => "Streaming",
        Downloads => "Downloads",
        Storage => "Storage",
        Backup => "Backup",
        Updates => "Updates",
    }
}

/// A setting's title and the line under it, which says only how things stand now.
fn setting_words(name: &str, s: &SettingsState) -> (String, String) {
    let (title, detail) = match name {
        "crossfadeSec" => ("Crossfade", ""),
        "autoMix" => {
            let detail = match &s.beat_model {
                BeatModel::Unavailable | BeatModel::Unused => String::new(),
                BeatModel::Downloading => format!("Downloading the beat model (about {} MB)…", s.beat_model_mb),
                BeatModel::Failed { .. } => "The beat model's download didn't work. It tries again at the next song.".into(),
                BeatModel::Absent | BeatModel::Ready => "Uses more battery while songs are measured".into(),
            };
            return ("AutoMix".into(), detail);
        }
        "crossfadeKeepAlbums" => ("Keep albums gapless", ""),
        "skipExplicit" => ("Skip explicit songs", ""),
        "autoFill" => ("When the queue ends", ""),
        "autoFillBasis" => ("Chosen by", ""),
        "autoFillRemote" => ("Include remote songs", ""),
        "updateCheck" => ("Check automatically", "At most once a day, from GitHub"),
        "updateBeta" => ("Beta updates", "Test versions of the next release. They may have bugs"),
        "bridgeOffline" => ("Play downloads when offline", ""),
        "playOnBluetooth" => ("Play when Bluetooth connects", "Headphones, a speaker or a car: the queue goes on where it stopped"),
        "autoEqAuto" => ("AutoEQ for headphones", ""),
        "autoEqDownload" => ("Keep the AutoEQ list", ""),
        "profilePerOutput" => ("Remember sound per device", ""),
        "soundBypass" => ("No processing on this output", ""),
        "headphoneSound" => ("Headphone sound", ""),
        "compressor" => ("Compressor", ""),
        "replayGain" => ("Volume leveling", ""),
        "loudnessLevel" => ("Level", ""),
        "volumeBoostDb" => ("Volume boost", ""),
        "hiRes" => ("High quality output", ""),
        "maxRate" => ("Highest sample rate", ""),
        "season" => ("Season", ""),
        "lyricsSweep" => ("Fill in words as they're sung", ""),
        "lyricsTranslation" => ("Show translations", ""),
        "lyricsOnline" => ("Find missing lyrics online", ""),
        "lyricsPreferWords" => ("Prefer word-by-word lyrics", ""),
        "tasteModel" => ("Keep listening history", ""),
        "thirdPartyLookups" => ("Look things up online", ""),
        "remoteControl" => ("Remote control and jams", ""),
        "wifi" => ("Streaming quality", ""),
        "download" => ("Quality for downloads", ""),
        "songsAheadWifi" => ("Prepare songs ahead", ""),
        "cacheMb" => ("Space for streamed music", ""),
        other => (other, ""),
    };
    (title.to_string(), detail.to_string())
}

/// A setting's value in words.
fn option_words(name: &str, v: &str) -> String {
    match (name, v) {
        ("crossfadeSec", "0") => "Off".into(),
        ("crossfadeSec", v) => format!("{v} s"),
        ("season", "FOLLOW") => "Follow the seasons".into(),
        ("season", "SPRING") => "Spring".into(),
        ("season", "SUMMER") => "Summer".into(),
        ("season", "AUTUMN") => "Autumn".into(),
        ("season", "WINTER") => "Winter".into(),
        ("autoFill", "OFF") => "Stop".into(),
        ("autoFill", "SONGS") => "Add songs".into(),
        ("autoFill", "ALBUMS") => "Add albums".into(),
        ("autoMix", "OFF") => "Off".into(),
        ("autoMix", "SMOOTH") => "Smooth".into(),
        ("autoMix", "DJ") => "DJ".into(),
        ("autoFillBasis", "SIMILAR") => "Similar music".into(),
        ("autoFillBasis", "ARTIST") => "The same artist".into(),
        ("autoFillBasis", "GENRE") => "The same genre".into(),
        ("autoFillBasis", "ERA") => "The same era".into(),
        ("headphoneSound" | "replayGain" | "compressor", "OFF") => "Off".into(),
        ("headphoneSound", "NATURAL") => "Natural".into(),
        ("headphoneSound", "WIDE") => "Wide".into(),
        ("compressor", "LIGHT") => "Light".into(),
        ("compressor", "CAR") => "Car".into(),
        ("compressor", "NIGHT") => "Night".into(),
        ("replayGain", "TRACK") => "Song".into(),
        ("replayGain", "ALBUM") => "Album".into(),
        ("replayGain", "AUTO") => "Auto".into(),
        ("loudnessLevel", "QUIET") => format!("Quiet ({})", lufs(Level::Quiet)),
        ("loudnessLevel", "NORMAL") => format!("Normal ({})", lufs(Level::Normal)),
        ("loudnessLevel", "LOUD") => format!("Loud ({})", lufs(Level::Loud)),
        ("maxRate", "AUTO") => "Each song's own".into(),
        ("maxRate", "KHZ48") => "48 kHz".into(),
        ("maxRate", "KHZ96") => "96 kHz".into(),
        ("maxRate", "KHZ192") => "192 kHz".into(),
        ("wifi" | "download", v) => {
            let (rate, format) = v.split_once(':').unwrap_or((v, ""));
            match format {
                "" => "Original".into(),
                "mp3" => format!("MP3 {rate}"),
                "opus" => format!("Opus {rate}"),
                _ => format!("{rate} {format}"),
            }
        }
        ("songsAheadWifi", "1") => "Next song".into(),
        ("songsAheadWifi", v) => format!("{v} songs"),
        ("cacheMb", v) => match v.parse::<u32>() {
            Ok(mb) if mb >= 1024 && mb % 1024 == 0 => format!("{} GB", mb / 1024),
            _ => format!("{v} MB"),
        },
        (_, v) => v.to_string(),
    }
}

fn info(title: &str, detail: String) -> Row {
    Row { kind: INFO, title: title.into(), detail, ..Default::default() }
}

fn note(text: &str) -> Row {
    Row { kind: NOTE, title: text.into(), ..Default::default() }
}

fn action(title: &str, detail: String, button: &str, enabled: bool, chore: Chore) -> Row {
    Row { kind: ACTION, name: chore_name(chore).into(), title: title.into(), detail, button: button.into(), enabled, ..Default::default() }
}

/// A set of independent flags as its title, then a switch for each (its name stores the set with the
/// flag switched); None for any other row.
fn flag_rows(r: &SettingsRow, s: &SettingsState) -> Option<Vec<Row>> {
    let RowItem::Setting { spec, value, .. } = &r.item else { return None };
    if spec.kind != SettingKind::Flags {
        return None;
    }
    let (title, detail) = setting_words(&spec.name, s);
    let switches = spec.options.iter().map(|flag| Row {
        kind: TOGGLE,
        name: format!("{}={}", spec.name, settings_model::flag_toggled(value.clone(), flag.clone())),
        title: option_words(&spec.name, flag),
        on: settings_model::flag_on(value.clone(), flag.clone()),
        enabled: r.enabled,
        ..Default::default()
    });
    Some(std::iter::once(info(&title, detail)).chain(switches).collect())
}

/// The desktop's row for one of the core's, or None for one it does not draw.
fn row_of(r: SettingsRow, p: &StoredPrefs, s: &SettingsState, f: &Facts, kinds: &Kinds, plugged: &Plugged) -> Option<Row> {
    let enabled = r.enabled;
    Some(match r.item {
        RowItem::Setting { spec, value, .. } => {
            let (title, detail) = setting_words(&spec.name, s);
            match spec.kind {
                SettingKind::Switch => Row { kind: TOGGLE, name: spec.name, title, detail, on: value == "true", enabled, ..Default::default() },
                SettingKind::Choice => {
                    let options = spec.options.iter().map(|v| (option_words(&spec.name, v), v.clone())).collect();
                    let kind = if spec.name == "season" { SEASONS } else { CHOICE };
                    Row { kind, name: spec.name, title, options, enabled, ..Default::default() }
                }
                SettingKind::Level => {
                    let v: f32 = value.parse().unwrap_or(spec.min);
                    let shown = if v <= 0.0 { "Off".to_string() } else { format!("{} dB", crate::eq::signed(v)) };
                    Row { kind: SLIDER, name: spec.name, title, button: shown, value: v.clamp(spec.min, spec.max), min: spec.min, max: spec.max, enabled: true, ..Default::default() }
                }
                // Drawn by `flag_rows`.
                SettingKind::Flags => return None,
            }
        }
        RowItem::Link { to: SettingsLink::Equalizer } => {
            let status = match s.eq_status {
                EqStatus::BitPerfect => "Off now",
                EqStatus::NoProcessing => "No processing",
                EqStatus::On => "On",
                EqStatus::Off => "Off",
            };
            Row { kind: LINK, name: EQUALIZER.into(), title: "Equalizer".into(), detail: status.into(), enabled: true, ..Default::default() }
        }
        RowItem::Stored => {
            let bytes = |b: u64| numbers::bytes(b as i64, POINT);
            let downloads = words::count(f.all_download_songs, "download", "downloads");
            let parts = [("Streamed music", f.stream_bytes), (downloads.as_str(), f.download_bytes), ("Covers", f.cover_bytes), ("Lyrics", f.lyrics_bytes), ("Library index", f.database_bytes)];
            let total: u64 = parts.iter().map(|p| p.1).sum();
            Row {
                kind: USAGE,
                title: "Stored on this computer".into(),
                detail: bytes(total),
                options: parts.iter().map(|(name, b)| (format!("{name} · {}", bytes(*b)), String::new())).collect(),
                shares: parts.iter().map(|p| p.1 as f32 / total.max(1) as f32).collect(),
                breakdown: if f.download_profiles.len() > 1 {
                    f.download_profiles.iter().map(|(profile, b)| format!("{} · {}", profile_name(p, kinds, profile), bytes(*b))).collect()
                } else {
                    Vec::new()
                },
                enabled: true,
                ..Default::default()
            }
        }
        RowItem::Unsent { waiting, dropped } => {
            let parts = [(waiting, "waiting for the server"), (dropped, "the server would not take")];
            info("Changes not sent", parts.iter().filter(|p| p.0 > 0).map(|(n, what)| format!("{n} {what}")).collect::<Vec<_>>().join(" · "))
        }
        RowItem::Link { .. } | RowItem::CompressionMeter => return None,
        RowItem::Action { action: a } => {
            let (songs, albums, artists) = f.indexed;
            let bytes = |b: u64| numbers::bytes(b as i64, POINT);
            match a {
                SettingsAction::SyncLibrary => {
                    let counts = format!("{} · {} · {} on this Mac", words::count(songs, "song", "songs"), words::count(albums, "album", "albums"), words::count(artists, "artist", "artists"));
                    action("Offline search", counts, if f.syncing { "Updating…" } else { "Update" }, enabled, Chore::SyncLibrary)
                }
                SettingsAction::DownloadLibrary => action("Download the whole library", "Every song, at the download quality.".into(), "Download", enabled, Chore::DownloadLibrary),
                SettingsAction::ClearStream => action("Streamed music", format!("{}. Oldest goes first. Downloads stay.", bytes(f.stream_bytes)), "Clear", enabled, Chore::ClearStream),
                SettingsAction::ClearCovers => action("Covers", format!("{}. Fetched again when needed.", bytes(f.cover_bytes)), "Clear", enabled, Chore::ClearCovers),
                SettingsAction::ClearLyrics => action("Lyrics", format!("{} found online. Looked up again when needed.", bytes(f.lyrics_bytes)), "Clear", enabled, Chore::ClearLyrics),
                SettingsAction::AddServer => Row { kind: BUTTON, name: ADD_SERVER.into(), title: "Add profile".into(), enabled: true, ..Default::default() },
                SettingsAction::MakeBackup => Row { kind: ACTION, name: BACKUP.into(), title: words::BACK_UP.into(), detail: words::BACK_UP_DETAIL.into(), button: "Save".into(), enabled, ..Default::default() },
                SettingsAction::RestoreBackup => Row { kind: ACTION, name: RESTORE.into(), title: words::RESTORE.into(), detail: words::RESTORE_DETAIL.into(), button: "Open".into(), enabled, ..Default::default() },
                SettingsAction::CheckUpdate => Row { kind: ACTION, name: CHECK_UPDATE.into(), title: "Check for updates".into(), detail: format!("Version {}", env!("CARGO_PKG_VERSION")), button: "Check".into(), enabled: true, ..Default::default() },
                SettingsAction::JoinJam => return None,
            }
        }
        RowItem::Note { note: n } => note(match n {
            SettingsNote::HeldByDac => "The audio is played untouched: no mixing, skipping or effects.",
            SettingsNote::LyricsSources => "Your server's lyrics always come first. Among these, the best match wins; the order breaks ties.",
            SettingsNote::Dac => return None,
        }),
        RowItem::Server { index } => {
            let sv = p.servers.get(index as usize)?;
            let active = sv.id == p.active_server_id;
            let words = crate::words::provider_of(kinds, &sv.kind);
            let who = if sv.user.is_empty() { words.no_user.to_string() } else { sv.user.clone() };
            let mut parts: Vec<String> = Vec::new();
            if !who.is_empty() {
                parts.push(who);
            }
            let sources = nori_core::settings::profile_sources(p.servers.clone(), sv.clone());
            if !sources.is_empty() {
                parts.push(format!("+ {}", sources.iter().map(|s| words::profile_label(s, kinds)).collect::<Vec<_>>().join(", ")));
            }
            if !sv.alt_url.trim().is_empty() {
                parts.push("second address".into());
            }
            let detail = parts.join(" · ");
            let title = crate::words::profile_label(sv, kinds);
            let mark = profile_mark(plugged, sv, &title);
            // A jam guest switches between the profiles; changing them is not its to do.
            let edits = f.rules.settings.contains(&nori_core::settings_pages::SettingsPart::ServerOptions);
            let menu = if edits {
                let sources_item = if sources.is_empty() {
                    (words::MERGE_WITH.to_string(), format!("merge:{}", sv.id))
                } else {
                    (words::SOURCES.to_string(), format!("sources:{}", sv.id))
                };
                vec![
                    ("Rename".to_string(), format!("rename:{}", sv.id)),
                    ("Edit".to_string(), format!("edit:{}", sv.id)),
                    sources_item,
                    ("Remove".to_string(), format!("remove:{}", sv.id)),
                ]
            } else {
                Vec::new()
            };
            Row {
                kind: SERVER, name: sv.id.clone(), title, detail, on: active, enabled: true, options: menu, rank: index as i32,
                icon: mark.icon, letter: mark.letter.into(), glyph: mark.glyph, picture: mark.picture, ..Default::default()
            }
        }
        RowItem::MusicFolder { index, on, tapped } => {
            let (name, _) = f.folders.get(index as usize)?;
            Row { kind: TOGGLE, name: format!("{MUSIC_FOLDERS}={tapped}"), title: name.clone(), on, enabled, ..Default::default() }
        }
        RowItem::LyricsSource { index } => {
            let src = s.lyrics_sources.get(index as usize)?;
            let (title, about) = service(&src.id)?;
            let button = match src.key {
                KeyAction::Change => "Change key",
                KeyAction::Add => "Add key",
                KeyAction::None => "",
            };
            Row { kind: RANKED, name: src.id.clone(), title: title.into(), detail: about.into(), on: src.on, enabled: r.enabled, button: button.into(), rank: index as i32, keyed: src.asks_key_on_enable, ..Default::default() }
        }
    })
}

/// A row of a profile's Edit page under its form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfilePart {
    /// Its account: the account sheet.
    Account,
    /// Its own server or provider taking its plays, switched.
    ServerScrobbles,
    /// A scrobbling account, by id, switched.
    Scrobbler(String),
    RemoveScrobbler(String),
    AddScrobbler,
    /// Opens the dialog adding a source (saved profiles or signing in to a new one).
    OpenAddSource,
    /// A saved profile taken as one more source.
    AddSource(String),
    /// A source moved one place earlier or later.
    SourceEarlier(String),
    SourceLater(String),
    /// A source given back its own profile.
    RemoveSource(String),
    /// A new source signed in to here: the first screen, its profile then this one's source.
    NewSource,
}

impl ProfilePart {
    pub fn of(name: &str) -> Option<ProfilePart> {
        if let Some(id) = name.strip_prefix("scrobbler-remove:") {
            return Some(ProfilePart::RemoveScrobbler(id.to_string()));
        }
        if let Some(id) = name.strip_prefix("scrobbler:") {
            return Some(ProfilePart::Scrobbler(id.to_string()));
        }
        for (prefix, part) in [("source-add:", ProfilePart::AddSource as fn(String) -> ProfilePart), ("source-earlier:", ProfilePart::SourceEarlier), ("source-later:", ProfilePart::SourceLater), ("source-remove:", ProfilePart::RemoveSource)] {
            if let Some(id) = name.strip_prefix(prefix) {
                return Some(part(id.to_string()));
            }
        }
        match name {
            "profile-account" => Some(ProfilePart::Account),
            "scrobble-server" => Some(ProfilePart::ServerScrobbles),
            "scrobbler-add" => Some(ProfilePart::AddScrobbler),
            "source-open" => Some(ProfilePart::OpenAddSource),
            "source-new" => Some(ProfilePart::NewSource),
            _ => None,
        }
    }

    pub fn name(&self) -> String {
        match self {
            ProfilePart::Account => "profile-account".into(),
            ProfilePart::ServerScrobbles => "scrobble-server".into(),
            ProfilePart::Scrobbler(id) => format!("scrobbler:{id}"),
            ProfilePart::RemoveScrobbler(id) => format!("scrobbler-remove:{id}"),
            ProfilePart::AddScrobbler => "scrobbler-add".into(),
            ProfilePart::OpenAddSource => "source-open".into(),
            ProfilePart::AddSource(id) => format!("source-add:{id}"),
            ProfilePart::SourceEarlier(id) => format!("source-earlier:{id}"),
            ProfilePart::SourceLater(id) => format!("source-later:{id}"),
            ProfilePart::RemoveSource(id) => format!("source-remove:{id}"),
            ProfilePart::NewSource => "source-new".into(),
        }
    }
}

/// The rows of profile `sv`'s Edit page under its form, as Android's: its account where its provider signs
/// in to one, then where its plays go: its own server first where it takes them (`itself`, the core's
/// rules), each scrobbling account with its switch and a menu removing it, then adding one; then its
/// other sources (unless it is itself a source of another profile).
pub fn profile_rows(kinds: &Kinds, plugged: &Plugged, sv: &SavedServer, itself: bool, p: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    let mut groups = Vec::new();
    if kinds.providers().into_iter().any(|i| i.kind == sv.kind && !i.sign_in.is_empty()) {
        let signed = !sv.user.is_empty();
        let account = Row {
            kind: ACTION,
            name: ProfilePart::Account.name(),
            title: if signed { sv.user.clone() } else { words::NOT_SIGNED_IN.into() },
            detail: if signed { words::SIGNED_IN.into() } else { String::new() },
            button: if signed { words::MANAGE } else { words::SIGN_IN }.into(),
            enabled: true,
            ..Default::default()
        };
        groups.push((words::ACCOUNT.to_string(), vec![account]));
    }
    let mut rows = Vec::new();
    if itself {
        let name = words::provider_of(kinds, &sv.kind).name;
        rows.push(Row { kind: TOGGLE, name: ProfilePart::ServerScrobbles.name(), title: name, detail: words::BUILT_IN.into(), on: !sv.scrobble_off, enabled: true, ..Default::default() });
    }
    rows.extend(sv.scrobblers.iter().map(|a| Row {
        kind: TOGGLE,
        name: ProfilePart::Scrobbler(a.id.clone()).name(),
        title: words::scrobbler(a.kind).into(),
        detail: words::scrobbler_detail(a),
        on: a.on,
        enabled: true,
        options: vec![(words::REMOVE.to_string(), ProfilePart::RemoveScrobbler(a.id.clone()).name())],
        ..Default::default()
    }));
    let hint = if sv.scrobblers.is_empty() && !itself { words::SCROBBLING_HINT } else { "" };
    rows.push(Row { kind: BUTTON, name: ProfilePart::AddScrobbler.name(), title: words::ADD_SCROBBLER.into(), detail: hint.into(), enabled: true, ..Default::default() });
    groups.push((words::SCROBBLING.to_string(), rows));
    // Shown only once the profile has merged sources; singular profiles and member sources show no Sources group.
    if !nori_core::settings::profile_sources(p.servers.clone(), sv.clone()).is_empty()
        && nori_core::settings::servers_shown(p.servers.clone()).iter().any(|s| s.id == sv.id)
    {
        groups.push((words::SOURCES.to_string(), source_rows(kinds, plugged, sv, p)));
    }
    model(groups, p, f, today)
}

/// The rows of a merged profile's `Sources` dialog (without a group heading): each source with Edit, Move
/// up/down, and Remove, followed by `+ Add source`.
pub fn source_list_rows(kinds: &Kinds, plugged: &Plugged, sv: &SavedServer, p: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    let rows: Vec<SettingRow> = lines(vec![(String::new(), source_rows(kinds, plugged, sv, p))], p, f, today).into_iter().skip(1).collect();
    ModelRc::new(VecModel::from(rows))
}

/// The Sources rows for merged profile `sv`, matching Android's `SourcesPart`: each other source with its
/// mark and menu editing, moving, or removing it, then a row opening the Add source dialog.
fn source_rows(kinds: &Kinds, plugged: &Plugged, sv: &SavedServer, p: &StoredPrefs) -> Vec<Row> {
    let sources = nori_core::settings::profile_sources(p.servers.clone(), sv.clone());
    let last = sources.len().saturating_sub(1);
    let mut rows: Vec<Row> = sources
        .iter()
        .enumerate()
        .map(|(at, s)| {
            let title = words::profile_label(s, kinds);
            let words = words::provider_of(kinds, &s.kind);
            let detail = if s.user.is_empty() { words.name } else { s.user.clone() };
            let mark = profile_mark(plugged, s, &title);
            let mut options = vec![("Edit".to_string(), format!("edit:{}", s.id))];
            if at > 0 {
                options.push((words::MOVE_UP.to_string(), ProfilePart::SourceEarlier(s.id.clone()).name()));
            }
            if at < last {
                options.push((words::MOVE_DOWN.to_string(), ProfilePart::SourceLater(s.id.clone()).name()));
            }
            options.push((words::REMOVE.to_string(), ProfilePart::RemoveSource(s.id.clone()).name()));
            Row { kind: INFO, name: s.id.clone(), title, detail, enabled: true, options, icon: mark.icon, letter: mark.letter.into(), glyph: mark.glyph, picture: mark.picture, ..Default::default() }
        })
        .collect();
    rows.push(Row { kind: BUTTON, name: ProfilePart::OpenAddSource.name(), title: words::ADD_SOURCE.into(), enabled: true, ..Default::default() });
    rows
}

/// The saved profiles `sv` can take as one more source, for the Add source dialog.
pub fn source_offered(p: &Plugged, prefs: &StoredPrefs, sv: &SavedServer, kinds: &Kinds) -> Vec<crate::ProfileItem> {
    nori_core::providers::sources_offered(prefs.servers.clone(), sv.clone())
        .iter()
        .map(|s| {
            let (who, where_) = words::profile_lines(s, kinds);
            let title = words::profile_label(s, kinds);
            let mark = profile_mark(p, s, &title);
            let detail = if who == title { where_ } else { who };
            crate::ProfileItem { name: s.id.as_str().into(), title: title.into(), detail: detail.into(), mark, on: false }
        })
        .collect()
}

/// The ask sheet's name for removing profile `id`, once confirmed.
pub const REMOVE_PROFILE: &str = "remove-profile:";

/// Display name and description of a lyrics service by core id.
fn service(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "BINILYRICS" => ("BiniLyrics", "Apple Music's lyrics, syllable by syllable, from a volunteer's copy. Unofficial."),
        "BETTER_LYRICS" => ("BetterLyrics", "Apple Music's lyrics, syllable by syllable. Finds more with a key. Unofficial."),
        "PAXSENIX" => ("PaxSenix", "Apple Music's lyrics, syllable by syllable, looked up another way. Unofficial."),
        "LYRICS_PLUS" => ("LyricsPlus", "Apple Music's lyrics and others', syllable by syllable, from volunteers. Unofficial."),
        "PAXSENIX_MUSIXMATCH" => ("PaxSenix: Musixmatch", "Musixmatch's lyrics, often word by word. Needs a PaxSenix key."),
        "UNISON" => ("Unison", "Open, written and timed by listeners. Many songs word by word."),
        "NETEASE" => ("NetEase Cloud Music", "The Chinese streaming service's own lyrics, often word by word. Unofficial."),
        "KUGOU" => ("KuGou", "Strong on Chinese, Japanese and Korean music, often word by word. Unofficial."),
        "LRCLIB" => ("LRCLIB", "Open and run by volunteers. Timed line by line, some songs word by word."),
        "PAXSENIX_SPOTIFY" => ("PaxSenix: Spotify", "Spotify's lyrics, timed line by line. Needs a PaxSenix key."),
        "YOUTUBE_MUSIC" => ("YouTube Music", "The words YouTube Music shows for the song. Not timed. Unofficial."),
        _ => return None,
    })
}

/// The groups of settings page `page`: each section's title and rows, with the desktop's own: the output
/// device on Sound, About's facts.
fn groups(model: &nori_core::beat_model::ModelFile, p: &StoredPrefs, f: &Facts, kinds: &Kinds, plugged: &Plugged, page: SettingsPage, today: Today) -> Vec<(String, Vec<Row>)> {
    let s = settings_model::state(p, settings_model::Output::default(), model);
    let facts = PageFacts {
        parts: f.rules.settings.clone(),
        folders: f.folders.iter().map(|(_, id)| id.clone()).collect(),
        indexed_songs: f.indexed.0,
        syncing: f.syncing,
        stream_bytes: f.stream_bytes as i64,
        cover_bytes: f.cover_bytes as i64,
        lyrics_bytes: f.lyrics_bytes as i64,
        unsent_waiting: f.unsent.0,
        unsent_dropped: f.unsent.1,
        ..PageFacts::default()
    };
    let mut groups: Vec<(String, Vec<Row>)> = Vec::new();
    for sec in settings_pages::page(page, SettingsClient::Desktop, p, &s, &facts) {
        let rows: Vec<Row> = sec.rows.into_iter().filter(|r| r.shown).flat_map(|r| flag_rows(&r, &s).unwrap_or_else(|| row_of(r, p, &s, f, kinds, plugged).into_iter().collect())).flat_map(|r| {
            if r.kind != SEASONS {
                return vec![r];
            }
            // The four seasons' tiles and following the calendar as a switch under them, then the appearance:
            // the theme has no row on the desktop's page of the core, and it stands for it.
            let s = seasons(p, today);
            let tiles = Row { options: SEASON_NAMES.map(|n| (n.to_string(), n.to_string())).to_vec(), ..r };
            let follow = Row { kind: TOGGLE, name: s.switch, title: "Follow the calendar".into(), detail: s.detail, on: s.following, enabled: true, ..Default::default() };
            let appearance = Row { kind: APPEARANCE_ROW, name: APPEARANCE.into(), title: "Appearance".into(), options: ["System", "Day", "Night", "Midnight"].map(|o| (o.to_string(), o.to_string())).to_vec(), enabled: true, ..Default::default() };
            vec![tiles, follow, appearance]
        }).collect();
        // The season stands under a heading of its own, as on Android, above the rest of the theme.
        let (season, rows): (Vec<Row>, Vec<Row>) = match rows.iter().position(|r| r.kind == SEASONS) {
            Some(at) => {
                let mut rows = rows;
                let rest = rows.split_off(at + 2);
                (rows, rest)
            }
            None => (Vec::new(), rows),
        };
        if !season.is_empty() {
            groups.push((SEASON_GROUP.into(), season));
        }
        let title = section_title(sec.id);
        // Sections that share a title are one group here.
        match groups.last_mut() {
            Some((t, g)) if t == title => g.extend(rows),
            _ if rows.is_empty() => {}
            _ => groups.push((title.into(), rows)),
        }
    }
    // The device played through ends Sound's Output group, its note under it.
    if page == SettingsPage::Sound {
        let mut devices = vec![("The system's own".to_string(), String::new())];
        devices.extend(f.devices.iter().map(|d| (d.clone(), d.clone())));
        let device = [
            Row { kind: CHOICE, name: DEVICE.into(), title: "Play through".into(), options: devices, enabled: true, ..Default::default() },
            note("A new device is used from the next start of nori."),
        ];
        let output = section_title(SectionId::Output);
        match groups.iter_mut().find(|(t, _)| t == output) {
            Some((_, rows)) => rows.extend(device),
            None => groups.push((output.into(), device.into())),
        }
    }
    if page == SettingsPage::About {
        return about(groups);
    }
    groups
}

/// About, as Android's: the version (pressed, it copies the report), the core's updates, what nori is built
/// with, and its license with the licenses page.
fn about(updates: Vec<(String, Vec<Row>)>) -> Vec<(String, Vec<Row>)> {
    let f = words::about_facts(env!("CARGO_PKG_VERSION"), cfg!(debug_assertions), std::env::consts::OS, std::env::consts::ARCH);
    let fact = |title: &str, detail: &str, end: &str, name: &str| Row { kind: FACT, name: name.into(), title: title.into(), detail: detail.into(), button: end.into(), enabled: true, ..Default::default() };
    let nori = vec![fact(&f.title, &f.build, words::COPY_IT, COPY_REPORT)];
    let hood = f.parts.iter().map(|(what, line)| fact(what, line, "", "")).collect();
    let open = vec![
        fact(words::FREE_SOFTWARE, words::LICENCE_LINE, "AGPL-3.0", ""),
        Row { kind: PAGE, name: LICENCES.into(), title: words::LICENCES.into(), detail: words::LICENCES_DETAIL.into(), enabled: true, ..Default::default() },
    ];
    let mut groups = vec![("nori".to_string(), nori)];
    groups.extend(updates);
    groups.extend([(words::UNDER_THE_HOOD.to_string(), hood), (words::OPEN_SOURCE.to_string(), open)]);
    groups
}

/// The desktop's About report, as its version row copies it.
pub fn about_report() -> String {
    words::about_facts(env!("CARGO_PKG_VERSION"), cfg!(debug_assertions), std::env::consts::OS, std::env::consts::ARCH).report
}

/// A group of the licenses page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditGroup {
    Core,
    Desktop,
    Data,
}

/// Everything the desktop is built from that is not nori's, grouped (`nori_core::credits`).
fn credits() -> Vec<(CreditGroup, Vec<nori_core::credits::Credit>)> {
    use nori_core::credits::{client_credits, core_credits, data_credits, CreditClient::Desktop};
    vec![(CreditGroup::Core, core_credits(Desktop)), (CreditGroup::Desktop, client_credits(Desktop)), (CreditGroup::Data, data_credits(Desktop))]
}

/// The licenses page: each group's heading, then its credits, the license at each one's end.
pub fn licence_rows(p: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    let mut at = 0;
    let groups = credits()
        .into_iter()
        .map(|(group, list)| {
            let rows = list
                .into_iter()
                .map(|c| {
                    at += 1;
                    Row { kind: FACT, name: format!("{LICENCE}{}", at - 1), title: c.name, detail: c.what, button: c.licence, enabled: true, ..Default::default() }
                })
                .collect();
            (words::credit_group(group).to_string(), rows)
        })
        .collect();
    model(groups, p, f, today)
}

/// The credit at `at` on the licenses page: its name, whose it is, its terms, and their full text.
pub fn licence(at: usize) -> Option<(String, String, String, &'static str)> {
    let c = credits().into_iter().flat_map(|(_, list)| list).nth(at)?;
    let text = c.file.as_deref().map_or(Some(words::NO_LICENCE_TEXT), licence_text)?;
    Some((c.name, c.copyright, c.licence, text.trim()))
}

/// The bundled text of license `file` (`Credit::file`), the same texts the Android app ships.
fn licence_text(file: &str) -> Option<&'static str> {
    macro_rules! texts {
        ($($file:literal),*) => {
            match file {
                $($file => Some(include_str!(concat!("../../../android/app/src/main/assets/licences/", $file, ".txt"))),)*
                _ => None,
            }
        };
    }
    texts!("Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "GPL-3.0", "MIT", "MPL-2.0", "OFL-1.1")
}

/// Rows of settings tab `tab`: each group's heading, then its rows with first and last marked.
pub fn rows(model: &nori_core::beat_model::ModelFile, p: &StoredPrefs, f: &Facts, kinds: &Kinds, plugged: &Plugged, tab: i32, today: Today) -> ModelRc<SettingRow> {
    self::model(groups(model, p, f, kinds, plugged, tab_page(f, tab), today), p, f, today)
}

/// The first run's last page: Settings' own season group and its appearance row, as Settings draws them.
pub fn setup_look_rows(model: &nori_core::beat_model::ModelFile, p: &StoredPrefs, f: &Facts, kinds: &Kinds, plugged: &Plugged, today: Today) -> ModelRc<SettingRow> {
    let look = groups(model, p, f, kinds, plugged, SettingsPage::Look, today)
        .into_iter()
        .filter_map(|(title, rows)| {
            let rows: Vec<Row> = if title == SEASON_GROUP { rows } else { rows.into_iter().filter(|r| r.kind == APPEARANCE_ROW).collect() };
            (!rows.is_empty()).then_some((title, rows))
        })
        .collect();
    self::model(look, p, f, today)
}

/// `groups` as the page draws them: each heading, then its rows with first and last marked.
fn model(groups: Vec<(String, Vec<Row>)>, p: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    ModelRc::new(VecModel::from(lines(groups, p, f, today)))
}

/// `groups` as the page's rows: each group's heading, then its rows on their plate.
fn lines(groups: Vec<(String, Vec<Row>)>, p: &StoredPrefs, f: &Facts, today: Today) -> Vec<SettingRow> {
    let mut out = Vec::new();
    for (title, rows) in groups {
        out.push(SettingRow { kind: HEADING, title: title.to_uppercase().into(), ..Default::default() });
        // Rows on a plate share it; the plate's corners round where a row without one (a note, the tiles,
        // the appearance) or the group's end meets it.
        let plated: Vec<bool> = rows.iter().map(|r| !matches!(r.kind, NOTE | SEASONS | README_HEADING | README_TEXT | README_ITEM | README_CODE)).collect();
        let n = rows.len();
        for (i, r) in rows.into_iter().enumerate() {
            let v = match Target::of(&r.name) {
                Target::Device => f.device.clone(),
                Target::Setting(name) => settings_model::value_of(p, name).unwrap_or_default(),
            };
            let chosen = if let Some(c) = r.chosen {
                c
            } else if r.kind == APPEARANCE_ROW {
                appearance_pick(p)
            } else if r.kind == SEASONS {
                seasons(p, today).worn
            } else if r.kind == RANKED || r.kind == SERVER {
                r.rank
            } else { r.options.iter().position(|o| o.1 == v).map_or(-1, |i| i as i32) };
            let labels: Vec<SharedString> = r.options.iter().map(|o| o.0.as_str().into()).collect();
            out.push(SettingRow {
                kind: r.kind,
                name: r.name.into(),
                title: r.title.into(),
                detail: r.detail.into(),
                on: r.on,
                enabled: r.enabled,
                options: ModelRc::new(VecModel::from(labels)),
                chosen,
                button: r.button.into(),
                value: r.value,
                min: r.min,
                max: r.max,
                keyed: r.keyed,
                shares: ModelRc::new(VecModel::from(r.shares)),
                breakdown: ModelRc::new(VecModel::from(r.breakdown.into_iter().map(SharedString::from).collect::<Vec<_>>())),
                line: r.line.into(),
                control: r.control.into(),
                values: ModelRc::new(VecModel::from(r.options.iter().map(|o| SharedString::from(o.1.as_str())).collect::<Vec<_>>())),
                icon: r.icon,
                glyph: r.glyph,
                letter: r.letter.into(),
                picture: r.picture,
                lit: ModelRc::new(VecModel::from(r.lit)),
                first: i == 0 || !plated[i - 1],
                last: i + 1 == n || !plated[i + 1],
            });
        }
    }
    out
}

/// A page over Settings › Plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginPage {
    Browser,
    Repos,
    /// A plugin's page, installed or only offered, by its id.
    Plugin(String),
    /// One of an installed plugin's own pages: the plugin, and the page row it opens from.
    Sub { plugin: String, page: String },
}

/// What a plugin's tile shows and what its icon was asked by.
#[derive(Default)]
pub struct Icons {
    /// Each plugin's icon as last heard; None for one that has none.
    pub read: HashMap<String, Option<slint::Image>>,
    /// Asked already: the plugin, and the address its offer gave.
    pub asked: HashSet<(String, Option<String>)>,
}

/// The plugins as Settings › Plugins shows them: installed, offered by the repos, and the repos.
#[derive(Default)]
pub struct Plugged {
    /// As their plugins said them last (`Watched::as_said`).
    pub cards: Vec<PluginCard>,
    /// Rows that show but take no input now, by plugin and row.
    pub off: HashSet<(String, String)>,
    /// Each plugin's assets, those that have some.
    pub assets: HashMap<String, Vec<AssetLine>>,
    /// The connected devices plugins control.
    pub controlled: Vec<ControlledDevice>,
    /// The plugins whose rows the devices panel shows too.
    pub panel: Vec<PluginCard>,
    /// What the dialog over the window shows.
    pub dialog: Option<PageDialog>,
    pub repos: Vec<Repo>,
    /// A repo typed is being looked for.
    pub adding_repo: bool,
    pub shelf: Shelf,
    /// The pages open over the list.
    pub stack: Vec<PluginPage>,
    /// The browser's search, and the repo it is narrowed to.
    pub query: String,
    pub repo_shown: Option<String>,
    /// The category the browser shows; none for its first.
    pub category_shown: Option<Category>,
    /// What the README was last asked for ([`Plugged::readme_wanted`]), and what came.
    pub readme_asked: Option<(String, Option<String>, Option<String>)>,
    pub readme: Vec<ReadmeBlock>,
    pub icons: Icons,
    /// The first screen shows: its offers' tiles are wanted too.
    pub choosing: bool,
    /// The switch over every plugin (the `pluginsOff` setting) is on.
    pub all_off: bool,
    /// The plugin whose package is on its way for its install sheet, or whose install is: no other
    /// version is got meanwhile.
    pub getting: Option<String>,
    /// The account pictures decoded (the profiles', their channels'), by their address.
    pub pictures: HashMap<String, slint::Image>,
}

impl Plugged {
    /// What the repos offer, as last read.
    pub fn offers(&self) -> &[Offer] {
        match &self.shelf {
            Shelf::Read(o) => &o.offers,
            _ => &[],
        }
    }

    /// Plugin `id`'s tile: its icon, or the letter of `name` once it is known to have none; neither
    /// before its icon was heard of.
    fn tile(&self, id: &str, name: &str) -> (slint::Image, String) {
        match self.icons.read.get(id) {
            Some(Some(icon)) => (icon.clone(), String::new()),
            Some(None) => (slint::Image::default(), name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
            None => (slint::Image::default(), String::new()),
        }
    }

    /// The plugins whose tiles show now and were not asked for yet, each with its offer, marked asked.
    pub fn icons_wanted(&mut self) -> Vec<(String, Option<Offer>)> {
        let offers = self.offers();
        let offer = |id: &str| nori_plugins::offer_of(offers, id, self.cards.iter().find(|c| c.id == id)).map(|(_, o)| o.clone());
        let shown: Vec<String> = match self.stack.last() {
            None => {
                let chosen_from = if self.choosing { nori_plugins::provider_offers(offers.to_vec(), self.cards.clone(), self.all_off) } else { Vec::new() };
                self.cards.iter().map(|c| c.id.clone()).chain(chosen_from.into_iter().map(|o| o.id)).collect()
            }
            Some(PluginPage::Browser) => nori_plugins::offers_found(offers.to_vec(), self.query.clone(), self.repo_shown.clone()).into_iter().map(|o| o.id).collect(),
            Some(PluginPage::Repos) => Vec::new(),
            Some(PluginPage::Plugin(id)) => vec![id.clone()],
            Some(PluginPage::Sub { .. }) => Vec::new(),
        };
        let wanted: Vec<(String, Option<Offer>)> = shown.into_iter().map(|id| (id.clone(), offer(&id))).collect();
        let fresh: Vec<_> = wanted.into_iter().filter(|(id, o)| !self.icons.asked.contains(&(id.clone(), o.as_ref().and_then(|o| o.icon.clone())))).collect();
        self.icons.asked.extend(fresh.iter().map(|(id, o)| (id.clone(), o.as_ref().and_then(|o| o.icon.clone()))));
        fresh
    }

    /// What the open plugin page's README has to be asked for: again when the plugin's version or the
    /// offer's address changed. None when asked already.
    pub fn readme_wanted(&mut self) -> Option<(String, Option<Offer>)> {
        let Some(PluginPage::Plugin(id)) = self.stack.last() else { return None };
        let id = id.clone();
        let card = self.cards.iter().find(|c| c.id == id);
        let offer = nori_plugins::offer_of(self.offers(), &id, card).map(|(_, o)| o.clone());
        let asked = (id.clone(), card.map(|c| c.version.clone()), offer.as_ref().and_then(|o| o.readme.clone()));
        if self.readme_asked.as_ref() == Some(&asked) {
            return None;
        }
        if self.readme_asked.as_ref().is_none_or(|a| a.0 != id) {
            self.readme.clear();
        }
        self.readme_asked = Some(asked);
        Some((id, offer))
    }

    /// Plugin `id`'s README came; the page may have moved on.
    pub fn readme_came(&mut self, id: &str, blocks: Vec<ReadmeBlock>) {
        if self.readme_asked.as_ref().is_some_and(|a| a.0 == id) {
            self.readme = blocks;
        }
    }

    /// What a plugin's kind says of itself; nori's own kinds are worded by the client.
    pub fn about_of(&self, kind: &ServerKind) -> Option<&str> {
        let ServerKind::Plugin { id } = kind else { return None };
        self.cards.iter().find(|c| c.id == *id)?.about.as_deref()
    }

    /// Closes the pages of a plugin just removed.
    pub fn gone(&mut self, id: &str) {
        self.stack.retain(|p| !matches!(p, PluginPage::Plugin(top) | PluginPage::Sub { plugin: top, .. } if top == id));
    }

    /// Closes the plugin pages whose page row went, with those over them, in Settings › Plugins and in
    /// the dialog; the dialog closes once what it shows went.
    pub fn pages_settled(&mut self) {
        let mut path = Vec::new();
        let standing = self
            .stack
            .iter()
            .take_while(|p| match p {
                PluginPage::Sub { plugin, page } => {
                    path.push(page.clone());
                    self.cards.iter().find(|c| c.id == *plugin).is_some_and(|c| nori_plugins::pages_standing(&c.rows, &path) == path.len())
                }
                _ => {
                    path.clear();
                    true
                }
            })
            .count();
        self.stack.truncate(standing);
        let standing = self.dialog.as_ref().and_then(|d| Some(nori_plugins::pages_standing(&self.dialog_card()?.rows, &d.pages)));
        match (standing, &mut self.dialog) {
            (Some(n), Some(d)) => d.pages.truncate(n),
            _ => self.dialog = None,
        }
        if self.dialog.as_ref().is_some_and(|d| matches!(d.root, DialogRoot::Plugin(_)) && d.pages.is_empty()) {
            self.dialog = None;
        }
    }

    /// The plugin whose rows the dialog shows.
    fn dialog_card(&self) -> Option<&PluginCard> {
        match &self.dialog.as_ref()?.root {
            DialogRoot::Device(id) => self.controlled.iter().find(|d| d.id == *id).map(|d| &d.card),
            DialogRoot::Plugin(id) => self.panel.iter().find(|c| c.id == *id),
        }
    }

    /// Opens the dialog, opens a page in it, goes back a page or closes it.
    pub fn dialog_act(&mut self, act: PluginAct) {
        match act {
            PluginAct::DeviceOpen(device) => self.dialog = Some(PageDialog { root: DialogRoot::Device(device), pages: Vec::new() }),
            PluginAct::DeviceClose => self.dialog = None,
            PluginAct::DialogPage(plugin, page) => match &mut self.dialog {
                Some(d) => d.pages.push(page),
                None => self.dialog = Some(PageDialog { root: DialogRoot::Plugin(plugin), pages: vec![page] }),
            },
            PluginAct::DialogBack => {
                if let Some(d) = &mut self.dialog {
                    d.pages.pop();
                }
            }
            _ => return,
        }
        self.pages_settled();
    }
}

/// What the dialog over the window shows: what it opened on, and the plugin's pages opened in it since, the
/// first on the plugin's own page and each after it on the one before.
#[derive(Debug, Clone, PartialEq)]
pub struct PageDialog {
    pub root: DialogRoot,
    pub pages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DialogRoot {
    /// A controlled device's settings, by its id.
    Device(String),
    /// A page of a plugin's the devices panel opened, by the plugin's id: the dialog's first page.
    Plugin(String),
}

/// What the plugin repos offer, as far as they were asked.
#[derive(Default)]
pub enum Shelf {
    #[default]
    Unasked,
    Asking,
    Read(Offers),
}

/// What a row of Settings › Plugins does, by its name.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginAct {
    /// Switches the plugin on or off.
    Turn(String),
    /// Keeps the plugin at its version, or lets it update.
    Hold(String),
    /// A row of the plugin's page, by its id.
    Row(String, String),
    /// Moves the plugin up (-1) or down (+1) the order.
    Move(String, i32),
    Remove(String),
    /// The switch over every plugin (the `pluginsOff` setting).
    AllOff,
    /// Asks for a file or folder to install.
    Install,
    /// Installs the plugin its sheet shows.
    InstallPreview,
    Look,
    /// Installs version `.1` (newest first) of offer `.0`.
    Get(u32, u32),
    /// The page at this address, in the browser.
    Open(String),
    /// The browser, or the repos, over what shows.
    Browser,
    Repos,
    /// A plugin's page over what shows.
    Page(String),
    /// One of a plugin's own pages over what shows: the plugin and the page row.
    SubPage(String, String),
    /// Back to what the page shown covers.
    Back,
    /// The browser's search, as typed.
    Search,
    /// The browser narrowed to a repo: its chip.
    Chip,
    /// The category the browser shows: its chip.
    Category,
    /// Asks for a repo's address, under the warning that it isn't nori's.
    AddRepo,
    RemoveRepo(String),
    /// Asks for a new name for the repo at this address.
    RenameRepo(String),
    /// A controlled device's noise control, by the device's id.
    Noise(String),
    /// A plugin's flags row: its id, the row and the bits switching this option stores.
    Flags(String, String, u64),
    /// A controlled device's settings, in a dialog over the window.
    DeviceOpen(String),
    /// Closes that dialog.
    DeviceClose,
    /// One of a plugin's pages, in the dialog over the window (opening it from the devices panel): the
    /// plugin and the page row.
    DialogPage(String, String),
    /// Back to the dialog's page before.
    DialogBack,
    /// Lets a plugin fetch its assets on mobile data.
    AllowMobile(String),
    /// The first screen's choice: this kind's form.
    Choose(ServerKind),
    /// Turns an installed plugin on: the first screen's choice of one that is off, or a restart of one
    /// safe mode stopped.
    TurnOn(String),
    /// The first screen's choice of a plugin not installed: its page.
    OfferPage(String),
    /// Every plugin and the repos, over the first screen.
    Sheet,
    /// Back from the form to the first screen.
    ChooseAgain,
    /// A provider's page: adds it and opens its sign-in, or goes on as a guest.
    AddSigningIn,
    AddAsGuest,
}

impl PluginAct {
    /// Parses a row name of Settings › Plugins; None for any other.
    pub fn of(name: &str) -> Option<PluginAct> {
        let (what, rest) = name.split_once(':').unwrap_or((name, ""));
        let id = || rest.to_string();
        Some(match what {
            "plugin-on" => PluginAct::Turn(id()),
            "plugin-hold" => PluginAct::Hold(id()),
            "plugin-row" => {
                let (plugin, row) = rest.split_once(':')?;
                PluginAct::Row(plugin.into(), row.into())
            }
            "plugin-up" => PluginAct::Move(id(), -1),
            "plugin-down" => PluginAct::Move(id(), 1),
            "plugin-remove" => PluginAct::Remove(id()),
            "plugins-off" => PluginAct::AllOff,
            "plugin-install" => PluginAct::Install,
            "plugin-install-preview" => PluginAct::InstallPreview,
            "plugin-look" => PluginAct::Look,
            "plugin-get" => {
                let (offer, version) = rest.split_once(':')?;
                PluginAct::Get(offer.parse().ok()?, version.parse().ok()?)
            }
            "plugin-open" => PluginAct::Open(id()),
            "plugin-browser" => PluginAct::Browser,
            "plugin-repos" => PluginAct::Repos,
            "plugin-page" => PluginAct::Page(id()),
            "plugin-subpage" => {
                let (plugin, page) = rest.split_once(':')?;
                PluginAct::SubPage(plugin.into(), page.into())
            }
            "plugin-back" => PluginAct::Back,
            "plugin-search" => PluginAct::Search,
            "plugin-chip" => PluginAct::Chip,
            "plugin-category" => PluginAct::Category,
            "repo-add" => PluginAct::AddRepo,
            "repo-remove" => PluginAct::RemoveRepo(id()),
            "repo-rename" => PluginAct::RenameRepo(id()),
            "device-noise" => PluginAct::Noise(id()),
            "plugin-flags" => {
                let (row, bits) = rest.rsplit_once(':')?;
                let (plugin, row) = row.split_once(':')?;
                PluginAct::Flags(plugin.into(), row.into(), bits.parse().ok()?)
            }
            "device-open" => PluginAct::DeviceOpen(id()),
            "device-close" => PluginAct::DeviceClose,
            "device-page" => {
                let (plugin, page) = rest.split_once(':')?;
                PluginAct::DialogPage(plugin.into(), page.into())
            }
            "device-back" => PluginAct::DialogBack,
            "plugin-mobile" => PluginAct::AllowMobile(id()),
            "provider-own" => PluginAct::Choose(ServerKind::Subsonic),
            "provider-local" => PluginAct::Choose(ServerKind::Local),
            "provider" => PluginAct::Choose(ServerKind::Plugin { id: id() }),
            "provider-on" => PluginAct::TurnOn(id()),
            "provider-offer" => PluginAct::OfferPage(id()),
            "plugins-sheet" => PluginAct::Sheet,
            "provider-again" => PluginAct::ChooseAgain,
            "provider-sign-in" => PluginAct::AddSigningIn,
            "provider-guest" => PluginAct::AddAsGuest,
            _ => return None,
        })
    }

    /// The row name that parses as `self`.
    pub fn name(&self) -> String {
        match self {
            PluginAct::Turn(id) => format!("plugin-on:{id}"),
            PluginAct::Hold(id) => format!("plugin-hold:{id}"),
            PluginAct::Row(id, row) => format!("plugin-row:{id}:{row}"),
            PluginAct::Move(id, by) => format!("{}:{id}", if *by < 0 { "plugin-up" } else { "plugin-down" }),
            PluginAct::Remove(id) => format!("plugin-remove:{id}"),
            PluginAct::AllOff => "plugins-off".into(),
            PluginAct::Install => "plugin-install".into(),
            PluginAct::InstallPreview => "plugin-install-preview".into(),
            PluginAct::Look => "plugin-look".into(),
            PluginAct::Get(offer, version) => format!("plugin-get:{offer}:{version}"),
            PluginAct::Open(url) => format!("plugin-open:{url}"),
            PluginAct::Browser => "plugin-browser".into(),
            PluginAct::Repos => "plugin-repos".into(),
            PluginAct::Page(id) => format!("plugin-page:{id}"),
            PluginAct::SubPage(plugin, page) => format!("plugin-subpage:{plugin}:{page}"),
            PluginAct::Back => "plugin-back".into(),
            PluginAct::Search => "plugin-search".into(),
            PluginAct::Chip => "plugin-chip".into(),
            PluginAct::Category => "plugin-category".into(),
            PluginAct::AddRepo => "repo-add".into(),
            PluginAct::RemoveRepo(url) => format!("repo-remove:{url}"),
            PluginAct::RenameRepo(url) => format!("repo-rename:{url}"),
            PluginAct::Noise(device) => format!("device-noise:{device}"),
            PluginAct::Flags(plugin, row, bits) => format!("plugin-flags:{plugin}:{row}:{bits}"),
            PluginAct::DeviceOpen(device) => format!("device-open:{device}"),
            PluginAct::DeviceClose => "device-close".into(),
            PluginAct::DialogPage(plugin, page) => format!("device-page:{plugin}:{page}"),
            PluginAct::DialogBack => "device-back".into(),
            PluginAct::AllowMobile(id) => format!("plugin-mobile:{id}"),
            PluginAct::Choose(ServerKind::Subsonic) => "provider-own".into(),
            PluginAct::Choose(ServerKind::Local) => "provider-local".into(),
            PluginAct::Choose(ServerKind::Plugin { id }) => format!("provider:{id}"),
            PluginAct::TurnOn(id) => format!("provider-on:{id}"),
            PluginAct::OfferPage(id) => format!("provider-offer:{id}"),
            PluginAct::Sheet => "plugins-sheet".into(),
            PluginAct::ChooseAgain => "provider-again".into(),
            PluginAct::AddSigningIn => "provider-sign-in".into(),
            PluginAct::AddAsGuest => "provider-guest".into(),
        }
    }
}

/// Settings › Plugins and the pages it opens, as they draw: the page's rows.
pub fn plugin_rows(p: &Plugged, prefs: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    model(plugin_groups(p, prefs.plugins_off), prefs, f, today)
}

fn plugin_groups(p: &Plugged, all_off: bool) -> Vec<(String, Vec<Row>)> {
    match p.stack.last() {
        None => plugin_list(p, all_off),
        Some(PluginPage::Browser) => browser(p),
        Some(PluginPage::Repos) => repos_page(p),
        Some(PluginPage::Plugin(id)) => plugin_page(p, id),
        Some(PluginPage::Sub { plugin, page }) => sub_page(p, plugin, page),
    }
}

/// Page `page` of installed plugin `plugin`, under its head: the rows on it.
fn sub_page(p: &Plugged, plugin: &str, page: &str) -> Vec<(String, Vec<Row>)> {
    let Some(c) = p.cards.iter().find(|c| c.id == plugin) else { return Vec::new() };
    let mut groups = card_groups(String::new(), c, Some(page), Vec::new(), &p.off, |_| true, Opens::Settings);
    groups.retain(|(_, rows)| !rows.is_empty());
    groups
}

/// The first screen as it draws: what a new profile can play from.
pub fn provider_rows(p: &Plugged, kinds: &Kinds, prefs: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    model(vec![(String::new(), provider_choices(p, kinds))], prefs, f, today)
}

/// The install has no profile yet: the first screen is the welcome.
pub fn first_run(prefs: &StoredPrefs) -> bool {
    prefs.servers.is_empty()
}

/// Every source the first run's welcome offers, on an equal footing: nori's own and the plugins' kinds, and
/// the provider plugins a repo offers that aren't on, in the core's order.
pub fn welcome_sources(p: &Plugged, kinds: &Kinds) -> Vec<WelcomeSource> {
    let active = kinds.providers().into_iter().map(|i| i.kind).collect();
    let offered = nori_plugins::provider_offers(p.offers().to_vec(), p.cards.clone(), p.all_off).into_iter().map(|o| o.id).collect();
    nori_core::providers::flow::welcome_sources(active, offered)
}

/// How a welcome source's row ends, as app.slint's `SourceRow::mark` numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMark {
    /// Picked or not.
    Pick(bool),
    /// Its plugin installing.
    Installing,
    /// Its sign-in, not done yet.
    Connect,
    /// Its profile in.
    Added,
}

impl SourceMark {
    fn number(self) -> i32 {
        match self {
            SourceMark::Pick(false) => 0,
            SourceMark::Pick(true) => 1,
            SourceMark::Installing => 2,
            SourceMark::Connect => 3,
            SourceMark::Added => 4,
        }
    }
}

/// A welcome source as a row: its tile, its name and what it is, ending in `mark`.
pub fn source_row(p: &Plugged, kinds: &Kinds, source: &WelcomeSource, mark: SourceMark) -> crate::SourceRow {
    let mark = mark.number();
    match source {
        WelcomeSource::Kind { kind: kind @ ServerKind::Subsonic } => {
            let title = words::provider_of(kinds, kind).name.into();
            crate::SourceRow { title, detail: words::PROVIDER_SUBSONIC_ABOUT.into(), glyph: MarkGlyph::Server, mark, ..Default::default() }
        }
        WelcomeSource::Kind { kind: kind @ ServerKind::Local } => {
            let title = words::provider_of(kinds, kind).name.into();
            crate::SourceRow { title, detail: words::PROVIDER_LOCAL_ABOUT.into(), glyph: MarkGlyph::Files, mark, ..Default::default() }
        }
        WelcomeSource::Kind { kind: kind @ ServerKind::Plugin { id } } => plugin_source(p, id, &words::provider_of(kinds, kind).name, mark),
        WelcomeSource::Offered { plugin } => plugin_source(p, plugin, &nori_plugins::plugin_display_name(plugin.clone(), p.cards.clone(), p.offers().to_vec()), mark),
    }
}

fn plugin_source(p: &Plugged, id: &str, name: &str, mark: i32) -> crate::SourceRow {
    let (icon, letter) = p.tile(id, name);
    crate::SourceRow { title: name.into(), icon, letter: letter.into(), mark, ..Default::default() }
}

/// The first screen's head: what it asks, and the plus to every plugin and the repos.
pub fn provider_top() -> PluginTop {
    PluginTop { title: words::PROVIDER_CHOOSE.into(), tools: tools(&[(TOOL_PLUS, words::GET_PLUGINS, PluginAct::Sheet)]), ..Default::default() }
}

/// nori's own and the plugins' providers in ink, then, quieter, those a repo offers that aren't on.
fn provider_choices(p: &Plugged, kinds: &Kinds) -> Vec<Row> {
    let active = kinds.providers().into_iter().map(|i| {
        let name = crate::words::provider_of(kinds, &i.kind).name;
        let act = PluginAct::Choose(i.kind.clone()).name();
        match &i.kind {
            ServerKind::Subsonic => Row { kind: PLUGIN, name: act, title: name, detail: words::PROVIDER_SUBSONIC_ABOUT.into(), glyph: MarkGlyph::Server, enabled: true, ..Default::default() },
            ServerKind::Local => Row { kind: PLUGIN, name: act, title: name, detail: words::PROVIDER_LOCAL_ABOUT.into(), glyph: MarkGlyph::Files, enabled: true, ..Default::default() },
            ServerKind::Plugin { id } => Row { name: act, ..plugin_line(p, id, &name, None, String::new()) },
        }
    });
    let offered = nori_plugins::provider_offers(p.offers().to_vec(), p.cards.clone(), p.all_off).into_iter().map(|o| {
        // With every plugin off none could run: it leads to the switch.
        let act = if p.all_off { PluginAct::Sheet } else if p.cards.iter().any(|c| c.id == o.id) { PluginAct::TurnOn(o.id.clone()) } else { PluginAct::OfferPage(o.id.clone()) };
        Row { name: act.name(), enabled: false, ..plugin_line(p, &o.id, &o.name, None, String::new()) }
    });
    active.chain(offered).collect()
}

/// A plugin in a list: its tile, name, what it is, a quiet `line`; opening it opens its page.
/// What a row that gets plugin `id` with `act` shows on its button, and what the button sends: nothing
/// while a package is on its way ([`Plugged::getting`]), the one coming saying so.
fn get_button(p: &Plugged, id: &str, button: &str, act: PluginAct) -> (String, String) {
    match &p.getting {
        Some(getting) if getting == id => (words::INSTALLING.into(), String::new()),
        Some(_) => (button.into(), String::new()),
        None => (button.into(), act.name()),
    }
}

fn plugin_line(p: &Plugged, id: &str, name: &str, about: Option<&str>, line: String) -> Row {
    let (icon, letter) = p.tile(id, name);
    Row { kind: PLUGIN, name: PluginAct::Page(id.into()).name(), title: name.into(), detail: about.unwrap_or_default().into(), line, icon, letter, enabled: true, ..Default::default() }
}

/// A profile's mark, as its icon says: the letter of who signs in (of `label` without a user), its plugin's
/// tile (the server glyph for nori's own provider), or a face.
pub fn profile_mark(p: &Plugged, sv: &SavedServer, label: &str) -> Mark {
    let glyph = match sv.icon {
        ProfileIcon::Letter => {
            let picture = sv.picture.as_ref().and_then(|a| p.pictures.get(a)).cloned().unwrap_or_default();
            return Mark { letter: nori_core::settings::profile_letter(sv.clone(), label.into()).into(), picture, ..Default::default() };
        }
        ProfileIcon::Provider => match &sv.kind {
            ServerKind::Plugin { id } => {
                let (icon, letter) = p.tile(id, label);
                return Mark { icon, letter: letter.into(), ..Default::default() };
            }
            ServerKind::Subsonic => MarkGlyph::Server,
            ServerKind::Local => MarkGlyph::Files,
        },
        ProfileIcon::Sing => MarkGlyph::Sing,
        ProfileIcon::Surprise => MarkGlyph::Surprise,
        ProfileIcon::Talking => MarkGlyph::Talking,
        ProfileIcon::LookUp => MarkGlyph::LookUp,
        ProfileIcon::Satisfied => MarkGlyph::Satisfied,
        ProfileIcon::Think => MarkGlyph::Think,
    };
    Mark { glyph, ..Default::default() }
}

/// The profiles menu's lines: every profile in its saved order (a source inside its profile), the one in
/// use marked where it stands.
pub fn profile_items(p: &Plugged, prefs: &StoredPrefs, kinds: &Kinds) -> Vec<crate::ProfileItem> {
    nori_core::settings::servers_shown(prefs.servers.clone())
        .iter()
        .map(|sv| {
            let (who, where_) = words::profile_lines(sv, kinds);
            let title = words::profile_label(sv, kinds);
            let mark = profile_mark(p, sv, &title);
            let base = if who == title { where_ } else { who };
            let sources = nori_core::settings::profile_sources(prefs.servers.clone(), sv.clone());
            let detail = match (base.is_empty(), sources.is_empty()) {
                (_, true) => base,
                (true, false) => format!("+ {}", sources.iter().map(|s| words::profile_label(s, kinds)).collect::<Vec<_>>().join(", ")),
                (false, false) => format!("{base} · + {}", sources.iter().map(|s| words::profile_label(s, kinds)).collect::<Vec<_>>().join(", ")),
            };
            crate::ProfileItem { name: sv.id.as_str().into(), title: title.into(), detail: detail.into(), mark, on: sv.id == prefs.active_server_id }
        })
        .collect()
}

/// The icons `sv`'s circle can show, each as it would show it, named (`provider` names its provider's mark).
pub fn icon_picker(p: &Plugged, sv: &SavedServer, label: &str, provider: &str) -> IconPicker {
    let choices: Vec<IconChoice> = ProfileIcon::ALL.iter().map(|&icon| IconChoice { name: words::profile_icon_name(icon, provider, sv.picture.is_some()).into(), mark: profile_mark(p, &SavedServer { icon, ..sv.clone() }, label) }).collect();
    let chosen = ProfileIcon::ALL.iter().position(|&i| i == sv.icon).expect("ALL holds every icon");
    IconPicker { choices: ModelRc::new(VecModel::from(choices)), chosen: chosen as i32 }
}

/// Updates waiting, and the installed plugins in their order, grey while every plugin is off.
fn plugin_list(p: &Plugged, all_off: bool) -> Vec<(String, Vec<Row>)> {
    // None installed: the head says so (`plugin_top`).
    if p.cards.is_empty() {
        return Vec::new();
    }
    let mut groups = Vec::new();
    let updates: Vec<Row> = nori_plugins::updates_waiting(p.offers(), &p.cards)
        .into_iter()
        .filter_map(|o| {
            let at = at_of(p.offers(), o);
            let line = words::update_line(o.installed.as_deref()?, &o.versions.first()?.version);
            let (button, control) = get_button(p, &o.id, words::UPDATE, PluginAct::Get(at as u32, 0));
            Some(Row { control, button, ..plugin_line(p, &o.id, &o.name, None, line) })
        })
        .collect();
    if !updates.is_empty() {
        groups.push((words::UPDATES.to_string(), updates));
    }
    let installed = p.cards.iter().map(|c| {
        let kinds = words::plugin_is(&c.points, !c.overrides.is_empty());
        if let Some(stopped) = c.stopped {
            return Row { control: PluginAct::TurnOn(c.id.clone()).name(), button: words::RESTART.into(), enabled: !all_off, ..plugin_line(p, &c.id, &c.name, None, words::plugin_stopped(stopped).into()) };
        }
        let line = [kinds, c.version.clone()].into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" · ");
        Row { control: PluginAct::Turn(c.id.clone()).name(), on: c.enabled, enabled: !all_off, ..plugin_line(p, &c.id, &c.name, None, line) }
    });
    groups.push((words::INSTALLED_LIST.to_string(), installed.collect()));
    groups
}

/// `o`'s place among `offers`, which an install asks by.
fn at_of(offers: &[Offer], o: &Offer) -> usize {
    offers.iter().position(|x| x.id == o.id && x.repo.url == o.repo.url).unwrap_or(0)
}

/// The categories `offers` show a page each for (none for one alone), and the one shown: `chosen` when it
/// is among them, else the first.
pub fn categories(offers: &[Offer], chosen: Option<Category>) -> (Vec<Category>, Option<Category>) {
    let all = nori_plugins::categories_shown(offers.iter().map(|o| o.category).collect());
    if all.len() < 2 {
        return (Vec::new(), None);
    }
    let shown = chosen.filter(|c| all.contains(c)).unwrap_or(all[0]);
    (all, Some(shown))
}

/// What the repos offer, as searched and narrowed to a repo.
pub fn found(p: &Plugged) -> Vec<Offer> {
    nori_plugins::offers_found(p.offers().to_vec(), p.query.clone(), p.repo_shown.clone())
}

/// What the repos offer, as searched and narrowed.
fn browser(p: &Plugged) -> Vec<(String, Vec<Row>)> {
    let mut rows = Vec::new();
    match &p.shelf {
        Shelf::Unasked | Shelf::Asking => rows.push(note(words::LOOKING)),
        Shelf::Read(Offers { offers, failed }) => {
            rows.extend(failed.iter().map(|f| note(&words::repo_failed(f))));
            let found = found(p);
            if found.is_empty() {
                rows.push(note(words::NONE_FOUND));
            }
            let shown = categories(&found, p.category_shown).1;
            rows.extend(found.iter().filter(|o| shown.is_none_or(|s| o.category == s)).filter_map(|o| {
                let line = format!("{} · {}", o.versions.first()?.version, words::repo_name(&o.repo));
                let get = |button| get_button(p, &o.id, button, PluginAct::Get(at_of(offers, o) as u32, 0));
                let (button, control) = match o.state {
                    OfferState::New => get(words::INSTALL),
                    OfferState::Update => get(words::UPDATE),
                    OfferState::Installed => (words::INSTALLED.into(), String::new()),
                };
                Some(Row { button, control, ..plugin_line(p, &o.id, &o.name, o.about.as_deref(), line) })
            }));
            rows.push(Row { kind: BUTTON, name: PluginAct::Look.name(), title: words::LOOK_AGAIN.into(), enabled: true, ..Default::default() });
        }
    }
    vec![(String::new(), rows)]
}

/// The repos and what each offers; those added can be renamed (the row) and removed (its button).
fn repos_page(p: &Plugged) -> Vec<(String, Vec<Row>)> {
    let rows = p.repos.iter().map(|r| {
        let line = match &p.shelf {
            Shelf::Read(o) => match o.failed.iter().find(|f| f.repo.url == r.url) {
                Some(f) => words::repo_problem(f.problem).into(),
                None => words::plugin_count(o.offers.iter().filter(|x| x.repo.url == r.url).count()),
            },
            _ => words::LOOKING.into(),
        };
        match r.official {
            true => info(&words::repo_name(r), line),
            false => Row {
                kind: ACTION,
                name: PluginAct::RemoveRepo(r.url.clone()).name(),
                control: PluginAct::RenameRepo(r.url.clone()).name(),
                title: words::repo_name(r),
                detail: format!("{} · {line}", r.url),
                button: words::REMOVE.into(),
                enabled: true,
                ..Default::default()
            },
        }
    });
    vec![(String::new(), rows.collect())]
}

/// A plugin's page under its head ([`plugin_top`]): its switches and rows, its README, what it may do, its
/// versions and source.
fn plugin_page(p: &Plugged, id: &str) -> Vec<(String, Vec<Row>)> {
    let card = p.cards.iter().find(|c| c.id == id);
    let found = nori_plugins::offer_of(p.offers(), id, card);
    let mut groups = Vec::new();
    if let Some(c) = card {
        let mut head = vec![match c.stopped {
            Some(stopped) => Row { kind: ACTION, name: PluginAct::TurnOn(c.id.clone()).name(), title: words::plugin_stopped(stopped).into(), button: words::RESTART.into(), enabled: true, ..Default::default() },
            None => Row { kind: TOGGLE, name: PluginAct::Turn(c.id.clone()).name(), title: words::PLUGIN_ON.into(), on: c.enabled, enabled: true, ..Default::default() },
        }];
        if c.repo.is_some() {
            head.push(Row { kind: TOGGLE, name: PluginAct::Hold(c.id.clone()).name(), title: words::KEEP_VERSION.into(), on: c.held, enabled: true, ..Default::default() });
        }
        // A device's settings are on the device's own page.
        let controls_devices = c.points.iter().any(|p| p == "device-control");
        groups.extend(card_groups(String::new(), c, None, head, &p.off, |r| !controls_devices || !nori_plugins::row_on_device_page(r.kind), Opens::Settings));
        if let Some(assets) = p.assets.get(&c.id) {
            let mut rows: Vec<Row> = assets.iter().map(|a| info(&a.name, words::asset_state(a))).collect();
            if assets.iter().any(|a| a.state == AssetState::WaitingForWifi) {
                rows.push(Row { kind: BUTTON, name: PluginAct::AllowMobile(c.id.clone()).name(), title: words::ASSETS_ON_MOBILE.into(), enabled: true, ..Default::default() });
            }
            groups.push((words::FILES.to_string(), rows));
        }
    }
    groups.push((words::ABOUT.to_string(), p.readme.iter().map(readme_row).collect()));
    if let Some(c) = card {
        let may: Vec<Row> = words::plugin_may(c).iter().map(|l| info(l, String::new())).collect();
        if !may.is_empty() {
            groups.push((words::PLUGIN_MAY.to_string(), may));
        }
    }
    if let Some((at, o)) = found {
        groups.push((words::VERSIONS.to_string(), versions(card, at, o, p.getting.is_none())));
    }
    groups.retain(|(_, rows)| !rows.is_empty());
    groups
}

fn readme_row(b: &ReadmeBlock) -> Row {
    let (kind, text) = match b {
        ReadmeBlock::Heading { text, .. } => (README_HEADING, text),
        ReadmeBlock::Text { text } => (README_TEXT, text),
        ReadmeBlock::Item { text } => (README_ITEM, text),
        ReadmeBlock::Code { text } => (README_CODE, text),
    };
    Row { kind, title: text.clone(), ..Default::default() }
}

/// Offer `o`, at place `at` among those read: its versions to install one of (when `free`: nothing
/// is on its way), and where its code is.
fn versions(card: Option<&PluginCard>, at: usize, o: &Offer, free: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = o
        .versions
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let here = card.is_some_and(|c| c.version == v.version);
            Row {
                kind: ACTION,
                name: PluginAct::Get(at as u32, i as u32).name(),
                title: v.version.clone(),
                detail: v.released.clone().unwrap_or_default(),
                button: (if here { words::INSTALLED } else { words::INSTALL }).into(),
                enabled: !here && free,
                ..Default::default()
            }
        })
        .collect();
    rows.extend(o.source.iter().map(|url| Row { kind: LINK, name: PluginAct::Open(url.clone()).name(), title: words::SOURCE.into(), detail: words::host(url).into(), enabled: true, ..Default::default() }));
    rows
}

fn strings(words: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(words.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

/// The tools at the top right of a plugin page, by their glyph (`PluginTool::glyph`).
const TOOL_PLUS: i32 = 0;
const TOOL_FOLDER: i32 = 1;
const TOOL_SERVER: i32 = 2;

fn tools(list: &[(i32, &str, PluginAct)]) -> ModelRc<PluginTool> {
    ModelRc::new(VecModel::from(list.iter().map(|(glyph, label, act)| PluginTool { glyph: *glyph, label: (*label).into(), name: act.name().into() }).collect::<Vec<_>>()))
}

/// The chips of the browser's categories, and the one lit.
fn category_chips(p: &Plugged) -> (ModelRc<SharedString>, i32) {
    let (all, shown) = categories(&found(p), p.category_shown);
    let lit = all.iter().position(|c| Some(*c) == shown).unwrap_or(0) as i32;
    (strings(all.into_iter().map(|c| words::category(c).to_string()).collect()), lit)
}

/// The head of what shows: its title and tools, and on a plugin's page what it is and the way to get it.
pub fn plugin_top(p: &Plugged, all_off: bool) -> PluginTop {
    let back = !p.stack.is_empty();
    match p.stack.last() {
        // Under its tab, which names it: no title. With none installed, what to do instead of the list.
        None if p.cards.is_empty() => PluginTop { back, about: words::NO_PLUGINS.into(), action: words::GET_PLUGINS.into(), action_name: PluginAct::Browser.name().into(), ..Default::default() },
        None => PluginTop { back, tools: tools(&[(TOOL_PLUS, words::GET_PLUGINS, PluginAct::Browser)]), every: true, all_on: !all_off, ..Default::default() },
        Some(PluginPage::Repos) => PluginTop { title: words::REPOS.into(), back, tools: tools(&[(TOOL_PLUS, words::ADD_REPO, PluginAct::AddRepo)]), ..Default::default() },
        Some(PluginPage::Browser) => {
            let shown = p.repos.iter().position(|r| p.repo_shown.as_ref() == Some(&r.url)).map_or(0, |i| i as i32 + 1);
            let chips = if p.repos.len() > 1 { std::iter::once(words::ALL.to_string()).chain(p.repos.iter().map(words::repo_name)).collect() } else { Vec::new() };
            let (categories, category) = category_chips(p);
            PluginTop {
                title: words::GET_PLUGINS.into(),
                back,
                tools: tools(&[(TOOL_FOLDER, words::INSTALL_FROM_FILE, PluginAct::Install), (TOOL_SERVER, words::REPOS, PluginAct::Repos)]),
                search: words::SEARCH_PLUGINS.into(),
                query: p.query.as_str().into(),
                chips: strings(chips),
                chip: shown,
                categories,
                category,
                ..Default::default()
            }
        }
        Some(PluginPage::Sub { plugin, page }) => {
            let row = p.cards.iter().find(|c| c.id == *plugin).and_then(|c| c.rows.iter().find(|r| r.id == *page));
            let (title, about) = row.map(|r| (r.label.clone(), r.hint.clone().unwrap_or_default())).unwrap_or_default();
            PluginTop { title: title.into(), back, about: about.into(), ..Default::default() }
        }
        Some(PluginPage::Plugin(id)) => {
            let at = p.cards.iter().position(|c| c.id == *id);
            let card = at.map(|at| &p.cards[at]);
            let found = nori_plugins::offer_of(p.offers(), id, card);
            let offer = found.map(|(_, o)| o);
            let newest = offer.and_then(|o| o.versions.first());
            let name = card.map(|c| c.name.as_str()).or(offer.map(|o| o.name.as_str())).unwrap_or(id);
            let (icon, letter) = p.tile(id, name);
            let from = match (card, offer) {
                (Some(c), _) if c.repo.is_none() => Some(words::FROM_A_FILE.to_string()),
                (_, Some(o)) => Some(words::repo_name(&o.repo)),
                _ => None,
            };
            let kinds = card.map(|c| words::plugin_is(&c.points, !c.overrides.is_empty())).filter(|k| !k.is_empty());
            let line = card.map(|c| c.version.clone()).or(newest.map(|v| v.version.clone())).into_iter().chain(from).chain(kinds).collect::<Vec<_>>().join(" · ");
            let about = card.and_then(|c| c.about.as_ref()).or(offer.and_then(|o| o.about.as_ref())).cloned().unwrap_or_default();
            let get = found.zip(newest).and_then(|((offer_at, o), v)| match (card, o.state) {
                (None, _) => Some(get_button(p, id, words::INSTALL, PluginAct::Get(offer_at as u32, 0))),
                (_, OfferState::Update) => Some(get_button(p, id, &words::update_to(&v.version), PluginAct::Get(offer_at as u32, 0))),
                _ => None,
            });
            let mut menu: Vec<(String, PluginAct)> = Vec::new();
            if let Some(at) = at {
                if at > 0 {
                    menu.push((words::MOVE_UP.into(), PluginAct::Move(id.clone(), -1)));
                }
                if at + 1 < p.cards.len() {
                    menu.push((words::MOVE_DOWN.into(), PluginAct::Move(id.clone(), 1)));
                }
                menu.push((words::REMOVE.into(), PluginAct::Remove(id.clone())));
            }
            let (labels, names): (Vec<String>, Vec<String>) = menu.into_iter().map(|(l, a)| (l, a.name())).unzip();
            PluginTop {
                title: name.into(),
                back,
                tile: true,
                icon,
                letter: letter.into(),
                line: line.into(),
                about: about.into(),
                action: get.as_ref().map(|g| g.0.as_str()).unwrap_or_default().into(),
                action_name: get.map(|g| g.1).unwrap_or_default().into(),
                menu: strings(labels),
                menu_names: strings(names),
                ..Default::default()
            }
        }
    }
}

/// Where a plugin's page row opens its page.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Opens {
    /// Over Settings › Plugins.
    Settings,
    /// In the dialog over the window.
    Dialog,
}

/// One row of plugin `plugin`'s page, drawn as the settings' own rows are; its hint is the quiet line
/// under it. Off, it shows but takes no input. A page row opens its page where `opens` says.
fn plugin_row(plugin: &str, r: &PluginRow, off: bool, opens: Opens) -> Row {
    let name = PluginAct::Row(plugin.into(), r.id.clone()).name();
    let hint = r.hint.clone().unwrap_or_default();
    let base = Row { name, title: r.label.clone(), detail: hint.clone(), enabled: !off, ..Default::default() };
    // A value said on the quiet line, the hint after it.
    let said = |value: String| [value, hint.clone()].into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" · ");
    match r.kind {
        RowKind::Switch => Row { kind: TOGGLE, on: r.value == Some(Value::Switch(true)), ..base },
        RowKind::Choice | RowKind::Segments => {
            let chosen = match r.value {
                Some(Value::Choice(i)) => i as i32,
                _ => 0,
            };
            let kind = if r.kind == RowKind::Segments { SEGMENTS } else { CHOICE };
            Row { kind, options: r.options.iter().map(|o| (o.clone(), o.clone())).collect(), chosen: Some(chosen), ..base }
        }
        RowKind::Level => {
            let (min, max) = (r.min.unwrap_or(0.0), r.max.unwrap_or(1.0));
            let value = match r.value {
                Some(Value::Level(v)) => v,
                _ => min,
            };
            Row { kind: SLIDER, button: words::plugin_level(value, r.decimals, r.unit.as_deref()), value: value as f32, min: min as f32, max: max as f32, ..base }
        }
        RowKind::Text | RowKind::Secret => {
            let typed = match &r.value {
                Some(Value::Text(t)) if !t.is_empty() => Some(t.clone()),
                _ => None,
            };
            let detail = match typed {
                Some(_) if r.kind == RowKind::Secret => words::SECRET_SET.into(),
                Some(t) => t,
                None => words::NOT_SET.into(),
            };
            Row { kind: ACTION, detail: said(detail), button: words::CHANGE.into(), ..base }
        }
        RowKind::Action => Row { kind: BUTTON, ..base },
        RowKind::Flags => Row {
            kind: FLAGS,
            options: r.options.iter().enumerate().map(|(at, o)| (o.clone(), PluginAct::Flags(plugin.into(), r.id.clone(), r.flag_switched(at)).name())).collect(),
            lit: (0..r.options.len()).map(|at| r.flag_on(at)).collect(),
            ..base
        },
        RowKind::Page => {
            let open = match opens {
                Opens::Settings => PluginAct::SubPage(plugin.into(), r.id.clone()),
                Opens::Dialog => PluginAct::DialogPage(plugin.into(), r.id.clone()),
            };
            Row { kind: PAGE, name: open.name(), ..base }
        }
        RowKind::Status => Row { kind: INFO, detail: said(r.value.as_ref().map(|v| words::plugin_value(v, &r.options)).unwrap_or_default()), ..base },
        RowKind::Meter => {
            let shown = match r.value {
                Some(Value::Level(v)) => words::plugin_level(v, r.decimals, r.unit.as_deref()),
                _ => String::new(),
            };
            Row { kind: INFO, detail: said(shown), ..base }
        }
        RowKind::Link => match &r.url {
            Some(url) => Row { kind: LINK, name: PluginAct::Open(url.clone()).name(), ..base },
            None => Row { kind: INFO, ..base },
        },
        RowKind::Note | RowKind::Section => note(&r.label),
    }
}

/// Card `c`'s `shown` rows on `page` (none for its own) as groups: under `title` (after `head`), each
/// section under its own; those in `off` take no input, and page rows open where `opens` says.
fn card_groups(title: String, c: &PluginCard, page: Option<&str>, head: Vec<Row>, off: &HashSet<(String, String)>, shown: impl Fn(&PluginRow) -> bool, opens: Opens) -> Vec<(String, Vec<Row>)> {
    let mut groups = Vec::new();
    let (mut title, mut rows) = (title, head);
    for r in nori_plugins::page_rows(&c.rows, page).filter(|r| shown(r)) {
        match r.kind {
            RowKind::Section => groups.push((std::mem::replace(&mut title, r.label.clone()), std::mem::take(&mut rows))),
            _ => rows.push(plugin_row(&c.id, r, off.contains(&(c.id.clone(), r.id.clone())), opens)),
        }
    }
    groups.push((title, rows));
    groups
}

/// The dialog over the window as it draws.
pub struct DialogShown {
    pub title: String,
    /// It has a page to go back to.
    pub back: bool,
    pub rows: ModelRc<SettingRow>,
}

/// The dialog as it draws: a controlled device's batteries, noise modes and its plugin's rows for it under
/// the device's name, or the plugin page last opened in it under that page's name.
pub fn dialog_shown(p: &Plugged, prefs: &StoredPrefs, f: &Facts, today: Today) -> Option<DialogShown> {
    let d = p.dialog.as_ref()?;
    let card = p.dialog_card()?;
    let Some(page) = d.pages.last() else {
        let DialogRoot::Device(id) = &d.root else { return None };
        let device = p.controlled.iter().find(|c| c.id == *id)?;
        return Some(DialogShown { title: device.name.clone(), back: false, rows: dialog_lines(device_page(device, &p.off), prefs, f, today) });
    };
    let row = card.rows.iter().find(|r| r.id == *page)?;
    let hint = row.hint.iter().map(|h| note(h)).collect();
    let mut groups = card_groups(String::new(), card, Some(page), hint, &p.off, |_| true, Opens::Dialog);
    groups.retain(|(_, rows)| !rows.is_empty());
    let back = d.pages.len() > 1 || matches!(d.root, DialogRoot::Device(_));
    Some(DialogShown { title: row.label.clone(), back, rows: dialog_lines(groups, prefs, f, today) })
}

/// The devices plugins control, which head the devices panel as Android's card does: each one's name over
/// its batteries, its noise modes and a way on to its settings.
pub fn controlled_rows(controlled: &[ControlledDevice], prefs: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    let rows = lines(controlled.iter().map(|d| (String::new(), device_card(d))).collect(), prefs, f, today);
    ModelRc::new(VecModel::from(rows.into_iter().filter(|r| r.kind != HEADING).collect::<Vec<_>>()))
}

/// The devices panel's plugin rows: those of the plugins `cards` show there, but a controlled device's,
/// whose settings are in its dialog.
pub fn device_rows(cards: &[PluginCard], controlled: &[ControlledDevice], off: &HashSet<(String, String)>, prefs: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    model(panel_groups(cards, controlled, off), prefs, f, today)
}

/// `groups` as the dialog's rows.
fn dialog_lines(groups: Vec<(String, Vec<Row>)>, prefs: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    let mut rows = lines(groups, prefs, f, today);
    // The dialog's title stands over the first plate.
    if rows.first().is_some_and(|r| r.kind == HEADING && r.title.is_empty()) {
        rows.remove(0);
    }
    ModelRc::new(VecModel::from(rows))
}

/// A controlled device's batteries and noise modes; `named`, its name leads them, the batteries under it.
fn device_controls(d: &ControlledDevice, named: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let battery = words::battery(&d.battery);
    if named {
        rows.push(Row { kind: CONTROLLED, title: d.name.clone(), detail: battery, ..Default::default() });
    } else if !battery.is_empty() {
        rows.push(info(words::BATTERY, battery));
    }
    if !d.noise_modes.is_empty() {
        let options = d.noise_modes.iter().map(|m| (words::noise(*m).to_string(), String::new())).collect();
        let chosen = d.noise.and_then(|n| d.noise_modes.iter().position(|m| *m == n)).map_or(-1, |i| i as i32);
        rows.push(Row { kind: SEGMENTS, name: PluginAct::Noise(d.id.clone()).name(), options, chosen: Some(chosen), enabled: true, ..Default::default() });
    }
    rows
}

/// A controlled device's card in the devices panel: what it is and its noise modes, then a way on to the
/// settings its plugin has for it.
fn device_card(d: &ControlledDevice) -> Vec<Row> {
    let mut rows = device_controls(d, true);
    if d.card.rows.iter().any(|r| nori_plugins::row_on_device_page(r.kind) && r.kind != RowKind::Section) {
        rows.push(Row { kind: LINK, name: PluginAct::DeviceOpen(d.id.clone()).name(), title: words::DEVICE_SETTINGS.into(), enabled: true, ..Default::default() });
    }
    rows
}

fn panel_groups(cards: &[PluginCard], controlled: &[ControlledDevice], off: &HashSet<(String, String)>) -> Vec<(String, Vec<Row>)> {
    let apart = cards.iter().filter(|c| !controlled.iter().any(|d| d.card.id == c.id));
    let mut groups: Vec<_> = apart.flat_map(|c| card_groups(c.name.clone(), c, None, Vec::new(), off, |_| true, Opens::Dialog)).collect();
    groups.retain(|(_, rows)| !rows.is_empty());
    groups
}

fn device_page(d: &ControlledDevice, off: &HashSet<(String, String)>) -> Vec<(String, Vec<Row>)> {
    let mut groups = vec![(String::new(), device_controls(d, false))];
    groups.extend(card_groups(String::new(), &d.card, None, Vec::new(), off, |r| nori_plugins::row_on_device_page(r.kind), Opens::Dialog));
    groups.retain(|(_, rows)| !rows.is_empty());
    groups
}

/// The value of option `index` of `target`.
pub fn option_value(target: Target, index: usize, f: &Facts) -> Option<String> {
    match target {
        Target::Device => {
            if index == 0 {
                Some(String::new())
            } else {
                f.devices.get(index - 1).cloned()
            }
        }
        Target::Setting(name) => options(name).into_iter().nth(index),
    }
}

/// The in-place level a slider row edits.
pub fn level_of(name: &str) -> Option<EqLevel> {
    Some(match name {
        "volumeBoostDb" => EqLevel::VolumeBoost,
        "eqPreamp" => EqLevel::Preamp,
        "balance" => EqLevel::Balance,
        "limiter" => EqLevel::Limiter,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    /// A key that is optional has its button; one needed is asked for when switching on; one kept changes.
    #[test]
    fn lyrics_keys_are_asked_as_on_android() {
        let facts = Facts { rules: nori_core::browse::profile_rules(false), ..Default::default() };
        let tab = tabs(&facts.rules.settings).iter().position(|t| *t == Tab::Page(SettingsPage::Lyrics)).unwrap() as i32;
        let today = Today { month: 10, day: 19, southern: false };
        let source = |p: &StoredPrefs, id: &str| {
            let rows = rows(&Default::default(), p, &facts, &Kinds::builtin(), &Plugged::default(), tab, today);
            let r = slint::Model::iter(&rows).find(|r| r.kind == RANKED && r.name == id).unwrap();
            (r.button.to_string(), r.keyed)
        };
        let p = StoredPrefs::default();
        assert_eq!(source(&p, "BETTER_LYRICS"), ("Add key".into(), false));
        assert_eq!(source(&p, "PAXSENIX_SPOTIFY"), ("".into(), true), "asked for when switched on");
        assert_eq!(source(&p, "LRCLIB"), ("".into(), false));
        let keyed = StoredPrefs { paxsenix_key: "k".into(), ..p };
        assert_eq!(source(&keyed, "PAXSENIX_SPOTIFY"), ("Change key".into(), false));
    }

    #[test]
    fn storage_lists_each_profiles_downloads() {
        use nori_core::settings::SavedServer;
        let facts = |profiles: Vec<(String, u64)>| Facts {
            rules: nori_core::browse::profile_rules(false),
            download_bytes: 3 << 20,
            all_download_songs: 5,
            download_profiles: profiles,
            ..Default::default()
        };
        let p = StoredPrefs { servers: vec![SavedServer { id: "a".into(), name: "Home".into(), ..Default::default() }], active_server_id: "a".into(), ..Default::default() };
        let today = Today { month: 10, day: 19, southern: false };
        let usage = |f: &Facts| (0..8).flat_map(|tab| groups(&Default::default(), &p, f, &Kinds::builtin(), &Plugged::default(), tab_page(f, tab), today)).flat_map(|(_, rows)| rows).find(|r| r.kind == USAGE).unwrap();
        let both = usage(&facts(vec![("a".into(), 2 << 20), ("gone".into(), 1 << 20)]));
        assert_eq!(both.options[1].0, "5 downloads · 3.0 MB");
        assert_eq!(both.breakdown, ["Home · 2.0 MB", "A removed profile · 1.0 MB"]);
        assert!(usage(&facts(vec![("a".into(), 3 << 20)])).breakdown.is_empty(), "one profile needs no split");
    }

    /// A profile's Edit page under its form: its own server first where it takes plays, each account with its
    /// switch and Remove, then adding one; the hint only where nothing takes them.
    #[test]
    fn a_profiles_edit_rows_say_where_its_plays_go() {
        use nori_core::settings::{SavedScrobbler, ScrobblerKind};
        let kinds = Kinds::builtin();
        let account = SavedScrobbler { id: "a".into(), kind: ScrobblerKind::ListenBrainz, on: true, user: "me".into(), ..Default::default() };
        let sv = SavedServer { id: "p".into(), scrobblers: vec![account], scrobble_off: true, ..Default::default() };
        let shown = |sv: &SavedServer, itself: bool| -> Vec<(i32, String, bool, Vec<String>, String)> {
            profile_rows(&kinds, &Plugged::default(), sv, itself, &StoredPrefs::default(), &Facts::default(), OCTOBER)
                .iter()
                .map(|r| (r.kind, r.name.to_string(), r.on, r.values.iter().map(|v| v.to_string()).collect(), r.detail.to_string()))
                .collect()
        };
        let rows = shown(&sv, true);
        let names: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(names, ["", "scrobble-server", "scrobbler:a", "scrobbler-add"], "a singular profile shows no Sources group on Edit");
        assert!(!rows[1].2, "the server's switch is off where its plays are off");
        assert_eq!(rows[2].3, ["scrobbler-remove:a"]);
        assert_eq!(rows[3].4, "", "accounts take the plays");
        let alone = shown(&SavedServer { scrobblers: Vec::new(), ..sv }, false);
        assert_eq!(alone.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["", "scrobbler-add"]);
        assert_eq!(alone[1].4, crate::words::SCROBBLING_HINT, "nothing takes the plays yet");
        for part in [ProfilePart::Account, ProfilePart::ServerScrobbles, ProfilePart::Scrobbler("a".into()), ProfilePart::RemoveScrobbler("a".into()), ProfilePart::AddScrobbler, ProfilePart::OpenAddSource, ProfilePart::NewSource] {
            assert_eq!(ProfilePart::of(&part.name()), Some(part.clone()));
        }
    }

    /// Merged profiles show a Sources section on Edit (with Edit, Move up/down, Remove, Add source) and
    /// `Sources` in their list menu; singular profiles show `Merge with…` in their list menu and NO Sources
    /// group on Edit; member sources show no Sources group either.
    #[test]
    fn a_profiles_edit_rows_list_its_sources() {
        let kinds = Kinds::builtin();
        let plugged = Plugged::default();
        let saved = |id: &str, sources: &[&str]| SavedServer { id: id.into(), name: id.into(), sources: sources.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let home = saved("home", &["a", "b"]);
        let a = saved("a", &[]);
        let c = saved("c", &[]);
        let prefs = StoredPrefs { servers: vec![home.clone(), a.clone(), saved("b", &[]), c.clone()], ..Default::default() };
        let rows: Vec<(String, String, Vec<String>)> = source_rows(&kinds, &plugged, &home, &prefs).iter().map(|r| (r.name.clone(), r.title.clone(), r.options.iter().map(|o| o.1.clone()).collect())).collect();
        let row = |name: &str, title: &str, options: &[&str]| (name.to_string(), title.to_string(), options.iter().map(|o| o.to_string()).collect::<Vec<_>>());
        assert_eq!(rows, [row("a", "a", &["edit:a", "source-later:a", "source-remove:a"]), row("b", "b", &["edit:b", "source-earlier:b", "source-remove:b"]), row("source-open", words::ADD_SOURCE, &[])]);
        let offered: Vec<String> = source_offered(&plugged, &prefs, &home, &kinds).into_iter().map(|o| o.name.to_string()).collect();
        assert_eq!(offered, ["c"]);
        let home_rows: Vec<String> = profile_rows(&kinds, &plugged, &home, true, &prefs, &Facts::default(), OCTOBER).iter().map(|r| r.name.to_string()).collect();
        assert!(home_rows.iter().any(|n| n == "source-open"), "merged profile shows Sources on Edit: {home_rows:?}");
        let singular_rows: Vec<String> = profile_rows(&kinds, &plugged, &c, true, &prefs, &Facts::default(), OCTOBER).iter().map(|r| r.name.to_string()).collect();
        assert!(!singular_rows.iter().any(|n| n == "source-open"), "singular profile shows no Sources group on Edit: {singular_rows:?}");
        let child_rows: Vec<String> = profile_rows(&kinds, &plugged, &a, true, &prefs, &Facts::default(), OCTOBER).iter().map(|r| r.name.to_string()).collect();
        assert!(!child_rows.iter().any(|n| n == "source-open"), "a profile that is itself a source shows no Sources group: {child_rows:?}");
        // List menus: merged profile offers Sources, singular profile offers Merge with….
        let facts = Facts { rules: nori_core::browse::profile_rules(false), ..Default::default() };
        let server_menus: Vec<(String, Vec<String>)> = groups(&Default::default(), &prefs, &facts, &kinds, &plugged, SettingsPage::Servers, OCTOBER)
            .into_iter()
            .flat_map(|(_, rs)| rs)
            .filter(|r| r.kind == SERVER)
            .map(|r| (r.name, r.options.into_iter().map(|(_, v)| v).collect()))
            .collect();
        assert_eq!(
            server_menus,
            [
                ("home".into(), vec!["rename:home".into(), "edit:home".into(), "sources:home".into(), "remove:home".into()]),
                ("c".into(), vec!["rename:c".into(), "edit:c".into(), "merge:c".into(), "remove:c".into()]),
            ]
        );
        for part in [ProfilePart::OpenAddSource, ProfilePart::AddSource("c".into()), ProfilePart::SourceEarlier("b".into()), ProfilePart::SourceLater("a".into()), ProfilePart::RemoveSource("a".into())] {
            assert_eq!(ProfilePart::of(&part.name()), Some(part.clone()));
        }
        assert_eq!(Act::of("merge:c"), Some(Act::Merge("c".into())));
        assert_eq!(Act::of("sources:home"), Some(Act::Sources("home".into())));
    }

    /// The device played through is Sound's, last in its Output group, and nowhere on Appearance.
    #[test]
    fn the_output_device_is_in_sounds_output_group() {
        let facts = Facts { rules: nori_core::browse::profile_rules(false), ..Default::default() };
        let page = |page| groups(&Default::default(), &StoredPrefs::default(), &facts, &Kinds::builtin(), &Plugged::default(), page, OCTOBER);
        let sound = page(SettingsPage::Sound);
        let (_, output) = sound.iter().find(|(t, _)| t == "Output").expect("Sound's Output group");
        let names: Vec<&str> = output.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names[names.len() - 2..], [DEVICE, ""], "the device, then its note, end the group");
        assert!(page(SettingsPage::Look).iter().flat_map(|(_, rows)| rows).all(|r| r.name != DEVICE), "Appearance holds no device");
    }

    /// With no plugin installed the tab says so in its head, with the way to get some; nothing under it.
    #[test]
    fn the_plugins_tab_without_plugins_offers_to_get_some() {
        let top = plugin_top(&Plugged::default(), false);
        assert_eq!((top.title.as_str(), top.about.as_str(), top.action.as_str()), ("", words::NO_PLUGINS, words::GET_PLUGINS));
        assert_eq!(PluginAct::of(&top.action_name), Some(PluginAct::Browser));
        assert!(plugin_list(&Plugged::default(), false).is_empty());
    }

    #[test]
    fn tabs_are_the_pages_the_profile_opens() {
        let names = |guest| tabs(&nori_core::browse::profile_rules(guest).settings).into_iter().map(tab_name).collect::<Vec<_>>();
        let mut expected = vec!["Profiles", "Playback", "Sound", "Appearance", "Lyrics", "Library", "Downloads and storage", "Plugins", "About"];
        // A build given the support service's address (the nightly, a release) ends with its tab.
        if nori_core::support::support_service().is_some() {
            expected.push(words::SUPPORT_TITLE);
        }
        assert_eq!(names(false), expected);
        assert!(!names(true).contains(&"Library"), "a jam guest has no library settings to show");
    }

    /// About is Android's: the version that copies the report, the core's updates, what nori is built with,
    /// and its license with the licenses page.
    #[test]
    fn about_holds_the_version_updates_and_licenses() {
        let facts = Facts { rules: nori_core::browse::profile_rules(false), ..Default::default() };
        let about = groups(&Default::default(), &StoredPrefs::default(), &facts, &Kinds::builtin(), &Plugged::default(), SettingsPage::About, OCTOBER);
        let titles: Vec<&str> = about.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles, ["nori", "Updates", words::UNDER_THE_HOOD, words::OPEN_SOURCE]);
        let version = &about[0].1[0];
        assert_eq!((version.kind, version.title.as_str(), version.button.as_str()), (FACT, concat!("nori ", env!("CARGO_PKG_VERSION")), words::COPY_IT));
        assert!(version.detail.starts_with("Desktop "), "the client and platform under it: {}", version.detail);
        assert_eq!(Act::of(&version.name), Some(Act::CopyReport));
        assert_eq!(about[2].1.len(), 4, "playback, library, AutoMix and the interface");
        let open = &about[3].1;
        assert_eq!((open[0].title.as_str(), open[0].button.as_str()), (words::FREE_SOFTWARE, "AGPL-3.0"));
        assert_eq!((open[1].kind, Act::of(&open[1].name)), (PAGE, Some(Act::Licences)));
    }

    /// The licenses page lists the core's, the desktop's and the data's credits, and each opens its full text.
    #[test]
    fn every_credit_opens_its_licence() {
        use slint::Model;
        let rows = licence_rows(&StoredPrefs::default(), &Facts::default(), OCTOBER);
        let headings: Vec<String> = rows.iter().filter(|r| r.kind == HEADING).map(|r| r.title.to_string()).collect();
        assert_eq!(headings, ["RUST CORE", "DESKTOP", "FONTS AND DATA"]);
        let credits: Vec<SettingRow> = rows.iter().filter(|r| r.kind == FACT).collect();
        for name in ["Slint", "cpal", "Symphonia", "rusqlite", "Inter"] {
            assert!(credits.iter().any(|r| r.title == name), "{name} is credited");
        }
        for r in &credits {
            let Some(Act::Licence(at)) = Act::of(&r.name) else { panic!("{} opens nothing", r.title) };
            let (name, owner, terms, text) = licence(at).unwrap_or_else(|| panic!("{} has no license text", r.title));
            assert_eq!((name.as_str(), terms.as_str()), (r.title.as_str(), r.button.as_str()));
            assert!(!owner.is_empty() && !text.is_empty(), "{name}");
        }
        let slint = credits.iter().find(|r| r.title == "Slint").and_then(|r| match Act::of(&r.name) { Some(Act::Licence(at)) => licence(at), _ => None });
        assert!(slint.is_some_and(|(_, _, _, text)| text.starts_with("GNU GENERAL PUBLIC LICENSE")), "Slint's own terms in full");
    }

    #[test]
    fn an_appearance_sets_the_theme_and_reads_back() {
        for pick in 0..4 {
            let p = nori_core::settings::set_by_name(&StoredPrefs::default(), "theme", appearance_theme(pick)).expect("a setting").prefs;
            assert_eq!(appearance_pick(&p), pick as i32, "pick {pick}");
        }
    }

    const OCTOBER: Today = Today { month: 10, day: 19, southern: false };

    #[test]
    fn the_look_page_has_season_tiles_then_the_theme() {
        use slint::Model;
        let facts = Facts::default();
        let look = tabs(&facts.rules.settings).iter().position(|t| *t == Tab::Page(SettingsPage::Look)).map_or(0, |i| i as i32);
        let rows = rows(&Default::default(), &StoredPrefs::default(), &facts, &Kinds::builtin(), &Plugged::default(), look, OCTOBER);
        let kinds: Vec<_> = rows.iter().map(|r| r.kind).collect();
        let at = kinds.iter().position(|k| *k == SEASONS).expect("a season row");
        assert_eq!(kinds[at - 1..at + 4], [HEADING, SEASONS, TOGGLE, HEADING, APPEARANCE_ROW], "the season under its heading, then the theme's");
        let tiles = rows.row_data(at).unwrap();
        assert_eq!((tiles.options.row_count(), tiles.chosen), (4, 2), "the four seasons, autumn worn while following");
        let follow = rows.row_data(at + 1).unwrap();
        assert_eq!((follow.on, follow.detail.as_str()), (true, "Winter comes in on November 7"));
        // Turned off, it keeps the calendar's season.
        assert_eq!(toggled(&follow.name, false), ("season", "AUTUMN".to_string()));
        let fixed = StoredPrefs { season: ThemeSeason::Spring, ..StoredPrefs::default() };
        let s = seasons(&fixed, OCTOBER);
        assert_eq!((s.worn, s.following, s.detail.as_str(), toggled(&s.switch, true).1.as_str()), (0, false, "", "FOLLOW"));
    }

    #[test]
    fn a_toggle_stores_its_state_or_the_value_its_name_carries() {
        assert_eq!(toggled("mono", true), ("mono", "true".to_string()));
        assert_eq!(toggled("musicFolders=1,3", false), ("musicFolders", "1,3".to_string()));
        assert_eq!(toggled("musicFolders=", true), ("musicFolders", String::new()));
    }

    #[test]
    fn music_folder_toggles_flip_and_keep_the_last_one() {
        use slint::Model;
        let dir = nori_testdir::TempDir::new("desktop-music-folders");
        let store = nori_core::settings_store::Settings::new();
        store.open(dir.join("nori.db").to_str().unwrap()).unwrap();
        let profile = SavedServer { id: "s1".into(), url: "https://music.example".into(), user: "me".into(), ..Default::default() };
        store.put(StoredPrefs { servers: vec![profile], active_server_id: "s1".into(), ..store.current().unwrap() });
        let facts = Facts { folders: vec![("Music".into(), "1".into()), ("Other".into(), "2".into())], ..Facts::default() };
        let tab = tabs(&facts.rules.settings).iter().position(|t| *t == Tab::Page(SettingsPage::Servers)).unwrap() as i32;
        let toggles = || {
            let p = store.current().unwrap();
            rows(&store.model, &p, &facts, &Kinds::builtin(), &Plugged::default(), tab, OCTOBER)
                .iter()
                .filter(|r| r.kind == TOGGLE)
                .map(|r| (r.title.to_string(), r.name.to_string(), r.on, r.enabled))
                .collect::<Vec<_>>()
        };
        let before = toggles();
        assert_eq!(before, [("Music".into(), "musicFolders=2".into(), true, true), ("Other".into(), "musicFolders=1".into(), true, true)]);
        let (name, value) = toggled(&before[0].1, false);
        assert!(store.edit_by_name(name, &value).is_some());
        let after = toggles();
        assert_eq!(after, [("Music".into(), "musicFolders=".into(), false, true), ("Other".into(), "musicFolders=2".into(), true, true)]);
        let (name, value) = toggled(&after[0].1, true);
        assert!(store.edit_by_name(name, &value).is_some());
        assert_eq!(toggles(), before);
    }

    #[test]
    fn row_names_resolve() {
        let p = StoredPrefs { auto_mix: nori_core::settings::AutoMixMode::Dj, auto_fill: Some(nori_core::settings::AutoFillKind::Songs), compressor: Some(nori_core::settings::CompressorPreset::Light), ..StoredPrefs::default() };
        let f = Facts::default();
        let specs: Vec<String> = settings_model::specs().into_iter().map(|s| s.name).collect();
        for tab in 0..7 {
            for (_, rows) in groups(&Default::default(), &p, &f, &Kinds::builtin(), &Plugged::default(), tab_page(&f, tab), OCTOBER) {
                for r in rows.iter().filter(|r| matches!(r.kind, TOGGLE | CHOICE) && r.name != DEVICE) {
                    let name = toggled(&r.name, r.on).0;
                    assert!(specs.iter().any(|s| s == name), "{name} is not a core setting");
                    assert!(!setting_words(name, &settings_model::state(&p, settings_model::Output::default(), &Default::default())).0.eq(name), "{name} has no words");
                }
                for r in rows.iter().filter(|r| r.kind == SLIDER) {
                    assert!(level_of(&r.name).is_some(), "{}", r.name);
                }
            }
        }
        for c in CHORES {
            assert_eq!(Act::of(chore_name(c)), Some(Act::Chore(c)));
        }
        assert_eq!(Act::of(EQUALIZER), Some(Act::Equalizer));
        assert_eq!(Act::of(ADD_SERVER), Some(Act::AddServer));
        assert_eq!(Act::of("server:7"), Some(Act::Server("7".into())));
        assert_eq!(Act::of("edit:7"), Some(Act::Edit("7".into())));
        assert_eq!(Act::of(&backup_password(true, "C:/b:k.noribackup")), Some(Act::BackupPassword { restore: true, path: "C:/b:k.noribackup".into() }), "a path with colons");
        assert_eq!(Act::of(&backup_password(false, "/a")), Some(Act::BackupPassword { restore: false, path: "/a".into() }));
    }

    use nori_plugins::{Offer, Version};

    fn plugin_card(id: &str, rows: Vec<PluginRow>) -> PluginCard {
        PluginCard { id: id.into(), version: "1.0.0".into(), name: id.into(), about: Some("Does it".into()), enabled: true, repo: Some(nori_plugins::OFFICIAL.into()), held: false, stopped: None, points: Vec::new(), overrides: Vec::new(), net: Vec::new(), lan: false, bluetooth: false, permissions: Vec::new(), hooks: Vec::new(), wakes: Vec::new(), commands: Vec::new(), cache_bytes: 0, asset_bytes: 0, rows }
    }

    fn official() -> Repo {
        Repo { url: nori_plugins::OFFICIAL.into(), official: true, name: None }
    }

    fn offer(id: &str, repo: &Repo, state: OfferState) -> Offer {
        let version = |v: &str| Version { repo: repo.url.clone(), version: v.into(), released: None, url: String::new(), sha256: String::new(), key: None, signature: None };
        let installed = (state != OfferState::New).then(|| "1.0.0".to_string());
        Offer { repo: repo.clone(), id: id.into(), name: id.into(), about: None, source: Some("https://code.example/x".into()), readme: None, icon: Some(format!("https://i.example/{id}.png")), points: Vec::new(), category: Category::Other, versions: vec![version("2.0.0"), version("1.0.0")], installed, state }
    }

    fn row(id: &str, kind: RowKind, value: Option<Value>) -> PluginRow {
        PluginRow { id: id.into(), kind, label: id.into(), options: vec!["a".into(), "b".into()], min: Some(0.0), max: Some(2.0), step: Some(0.5), value, hint: None, unit: None, decimals: 1, url: None, page: None, glyph: None }
    }

    fn read(offers: Vec<Offer>) -> Shelf {
        Shelf::Read(Offers { offers, failed: Vec::new() })
    }

    /// While a package is on its way its plugin says so and no other version is got: on a slow connection
    /// each click fetched the plugin again, and each fetch asked to install it once more.
    #[test]
    fn nothing_is_got_while_a_package_is_on_its_way() {
        let official = official();
        let mut p = Plugged {
            shelf: read(vec![offer("a.one", &official, OfferState::New), offer("b.two", &official, OfferState::Update)]),
            stack: vec![PluginPage::Browser],
            getting: Some("a.one".into()),
            ..Default::default()
        };
        let offered = |p: &Plugged| -> Vec<(String, String, String)> {
            plugin_groups(p, false).into_iter().flat_map(|(_, rows)| rows).filter(|r| r.kind == PLUGIN).map(|r| (r.title, r.button, r.control)).collect()
        };
        let row = |id: &str, button: &str, control: &str| (id.to_string(), button.to_string(), control.to_string());
        assert_eq!(offered(&p), [row("a.one", words::INSTALLING, ""), row("b.two", words::UPDATE, "")]);
        p.stack = vec![PluginPage::Plugin("a.one".into())];
        let top = plugin_top(&p, false);
        assert_eq!((top.action.as_str(), top.action_name.as_str()), (words::INSTALLING, ""));
        let versions: Vec<bool> = plugin_groups(&p, false).into_iter().flat_map(|(_, rows)| rows).filter(|r| r.kind == ACTION).map(|r| r.enabled).collect();
        assert_eq!(versions, [false, false]);

        p.getting = None;
        assert_eq!(PluginAct::of(&plugin_top(&p, false).action_name), Some(PluginAct::Get(0, 0)));
        p.stack = vec![PluginPage::Browser];
        assert_eq!(offered(&p), [row("a.one", words::INSTALL, "plugin-get:0:0"), row("b.two", words::UPDATE, "plugin-get:1:0")]);
    }

    /// The list is a line per plugin, whose control is apart from what opening it does, and updates
    /// waiting above: not its rows, not what the repos offer.
    #[test]
    fn the_plugins_list_is_a_line_per_plugin() {
        let rows = vec![row("on", RowKind::Switch, Some(Value::Switch(true)))];
        let held = PluginCard { held: true, ..plugin_card("a.held", Vec::new()) };
        let official = official();
        let p = Plugged {
            cards: vec![plugin_card("a.one", rows), held],
            shelf: read(vec![offer("a.one", &official, OfferState::Update), offer("a.held", &official, OfferState::Update), offer("b.new", &official, OfferState::New)]),
            ..Default::default()
        };
        let groups = plugin_groups(&p, true);
        let titles: Vec<&str> = groups.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles, [words::UPDATES, words::INSTALLED_LIST]);
        let update = &groups[0].1;
        assert_eq!(update.len(), 1, "one kept at its version waits for nothing");
        assert_eq!((update[0].line.as_str(), PluginAct::of(&update[0].control)), ("1.0.0 → 2.0.0", Some(PluginAct::Get(0, 0))));
        let one = &groups[1].1[0];
        assert_eq!((one.kind, PluginAct::of(&one.name), PluginAct::of(&one.control), one.on), (PLUGIN, Some(PluginAct::Page("a.one".into())), Some(PluginAct::Turn("a.one".into())), true));
        assert!(!one.enabled, "grey while every plugin is off");
        let top = plugin_top(&p, true);
        assert!(top.every && !top.all_on, "the switch over every plugin in the head");
        assert!(!top.back);
        assert_eq!(PluginAct::of(&slint::Model::row_data(&top.tools, 0).unwrap().name), Some(PluginAct::Browser), "the plus opens the browser");
    }

    /// A plugin safe mode stopped says why and offers a restart in place of its switch, in the list and
    /// on its page.
    #[test]
    fn a_plugin_safe_mode_stopped_offers_a_restart() {
        let crashed = PluginCard { enabled: false, stopped: Some(nori_plugins::Stopped::Crashed), ..plugin_card("a.crashed", Vec::new()) };
        let p = Plugged { cards: vec![crashed], ..Default::default() };
        let row = &plugin_groups(&p, false)[0].1[0];
        assert_eq!((row.line.as_str(), row.button.as_str(), PluginAct::of(&row.control)), (words::plugin_stopped(nori_plugins::Stopped::Crashed), words::RESTART, Some(PluginAct::TurnOn("a.crashed".into()))));
        let head = &plugin_page(&p, "a.crashed")[0].1[0];
        assert_eq!((head.kind, head.button.as_str(), PluginAct::of(&head.name)), (ACTION, words::RESTART, Some(PluginAct::TurnOn("a.crashed".into()))));
    }

    /// The browser searches through the core and shows by name, while a row's Install still names the
    /// offer by its place among those read.
    #[test]
    fn the_browser_shows_what_is_found_and_installs_the_offer_shown() {
        let friend = Repo { url: "https://friend.example/index.json".into(), official: false, name: Some("friend.example".into()) };
        let official = official();
        let mut p = Plugged {
            repos: vec![official.clone(), friend.clone()],
            stack: vec![PluginPage::Browser],
            shelf: read(vec![offer("z.last", &official, OfferState::New), offer("a.first", &friend, OfferState::Update), offer("m.mid", &official, OfferState::Installed)]),
            ..Default::default()
        };
        let lines = |p: &Plugged| plugin_groups(p, false)[0].1.iter().filter(|r| r.kind == PLUGIN).map(|r| (r.title.clone(), r.button.clone(), r.control.clone())).collect::<Vec<_>>();
        assert_eq!(
            lines(&p),
            [("a.first".into(), "Update".into(), "plugin-get:1:0".into()), ("m.mid".into(), "Installed".into(), String::new()), ("z.last".into(), "Install".into(), "plugin-get:0:0".into())]
        );
        let top = plugin_top(&p, false);
        assert_eq!(slint::Model::row_count(&top.chips), 3, "All and one per repo");
        p.repo_shown = Some(friend.url.clone());
        assert_eq!(plugin_top(&p, false).chip, 2);
        assert_eq!(lines(&p).len(), 1);
        p.repo_shown = None;
        p.query = "z.la".into();
        assert_eq!(lines(&p).len(), 1);
        p.query = "nothing".into();
        assert_eq!(plugin_groups(&p, false)[0].1[0].title, words::NONE_FOUND);
        p.repos.truncate(1);
        assert_eq!(slint::Model::row_count(&plugin_top(&p, false).chips), 0, "one repo needs no chips");
    }

    /// What the repos offer in several categories shows one category at a time, by its chip.
    #[test]
    fn the_browser_shows_one_category_at_a_time() {
        let official = official();
        let of = |id: &str, category| Offer { category, ..offer(id, &official, OfferState::New) };
        let mut p = Plugged { repos: vec![official.clone()], shelf: read(vec![of("tube", Category::Service), of("orbit", Category::Playback), of("speed", Category::Playback)]), stack: vec![PluginPage::Browser], ..Default::default() };
        let listed = |p: &Plugged| plugin_groups(p, false).into_iter().flat_map(|g| g.1).filter(|r| r.kind == PLUGIN).map(|r| r.title.to_string()).collect::<Vec<_>>();
        let top = plugin_top(&p, false);
        assert_eq!((slint::Model::row_count(&top.categories), top.category), (2, 0), "Services and Playback");
        assert_eq!(listed(&p), ["tube"]);
        p.category_shown = Some(Category::Playback);
        assert_eq!((listed(&p), plugin_top(&p, false).category), (vec!["orbit".to_string(), "speed".to_string()], 1));
    }

    /// Repos say what each offers, or why they couldn't be read; only those added can be renamed or go.
    #[test]
    fn the_repos_say_what_they_offer() {
        let friend = Repo { url: "https://friend.example/index.json".into(), official: false, name: Some("Friend".into()) };
        let official = official();
        let gone = Repo { url: "https://gone.example/index.json".into(), official: false, name: Some("gone.example".into()) };
        let p = Plugged {
            repos: vec![official.clone(), friend.clone(), gone.clone()],
            stack: vec![PluginPage::Repos],
            shelf: Shelf::Read(Offers { offers: vec![offer("a", &official, OfferState::New), offer("b", &official, OfferState::New), offer("c", &friend, OfferState::New)], failed: vec![nori_plugins::RepoFailure { repo: gone, problem: nori_plugins::RepoProblem::NotFound }] }),
            ..Default::default()
        };
        let groups = plugin_groups(&p, false);
        let rows = &groups[0].1;
        assert_eq!((rows[0].kind, rows[0].detail.as_str()), (INFO, "2 plugins"));
        assert_eq!((rows[1].title.as_str(), rows[1].button.as_str(), PluginAct::of(&rows[1].name)), ("Friend", "Remove", Some(PluginAct::RemoveRepo(friend.url.clone()))));
        assert_eq!(PluginAct::of(&rows[1].control), Some(PluginAct::RenameRepo(friend.url.clone())), "the row renames it");
        assert!(rows[1].detail.ends_with("1 plugin"), "{}", rows[1].detail);
        assert!(rows[2].detail.ends_with("Not found"), "{}", rows[2].detail);
        assert_eq!(rows[0].control, "", "nori's own keeps its name");
        assert_eq!(PluginAct::of(&slint::Model::row_data(&plugin_top(&p, false).tools, 0).unwrap().name), Some(PluginAct::AddRepo));
    }

    /// A plugin's page draws each kind of row with the settings' own, each named so it parses back to
    /// the plugin and row it sets; then its README, what it may do, its versions and source.
    #[test]
    fn a_plugins_page_has_its_rows_readme_and_versions() {
        let rows = vec![
            row("on", RowKind::Switch, Some(Value::Switch(true))),
            row("pick", RowKind::Choice, Some(Value::Choice(1))),
            row("gain", RowKind::Level, Some(Value::Level(1.5))),
            row("key", RowKind::Secret, Some(Value::Text("hunter2".into()))),
            row("go", RowKind::Action, None),
            row("more", RowKind::Section, None),
            row("state", RowKind::Status, Some(Value::Text("Fine".into()))),
        ];
        let official = official();
        let buds = ControlledDevice { id: "AA".into(), name: "Buds".into(), battery: nori_plugins::Battery { single: Some(80), ..Default::default() }, level: Some(80), noise: None, noise_modes: Vec::new(), noise_cycle: Vec::new(), card: plugin_card("a.one", Vec::new()) };
        let mut p = Plugged {
            cards: vec![plugin_card("a.one", rows), plugin_card("a.two", Vec::new())],
            off: HashSet::from([("a.one".to_string(), "pick".to_string())]),
            assets: HashMap::from([("a.one".to_string(), vec![AssetLine { name: "model".into(), size: 3 << 20, state: AssetState::WaitingForWifi }])]),
            controlled: vec![buds],
            repos: vec![official.clone()],
            shelf: read(vec![offer("a.one", &official, OfferState::Update)]),
            stack: vec![PluginPage::Plugin("a.one".into())],
            readme: vec![ReadmeBlock::Heading { level: 2, text: "Use".into() }, ReadmeBlock::Text { text: "Hello".into() }, ReadmeBlock::Item { text: "One".into() }, ReadmeBlock::Code { text: "x".into() }],
            ..Default::default()
        };
        let groups = plugin_groups(&p, false);
        let titles: Vec<&str> = groups.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles, ["", "more", "Files", "About", "Versions"], "nothing to say of what it may do, no group");
        let one: Vec<(i32, &str)> = groups[0].1.iter().map(|r| (r.kind, r.name.as_str())).collect();
        assert_eq!(one, [(TOGGLE, "plugin-on:a.one"), (TOGGLE, "plugin-hold:a.one"), (TOGGLE, "plugin-row:a.one:on"), (CHOICE, "plugin-row:a.one:pick"), (SLIDER, "plugin-row:a.one:gain"), (ACTION, "plugin-row:a.one:key"), (BUTTON, "plugin-row:a.one:go")]);
        assert!(!groups[0].1[3].enabled, "a row its plugin turned off takes no input");
        assert_eq!(groups[0].1[3].chosen, Some(1));
        assert_eq!(groups[0].1[5].detail, "Set", "a secret stays hidden");
        assert_eq!((groups[1].1[0].kind, groups[1].1[0].detail.as_str()), (INFO, "Fine"));
        let files: Vec<(i32, &str, &str)> = groups[2].1.iter().map(|r| (r.kind, r.title.as_str(), r.detail.as_str())).collect();
        assert_eq!(files, [(INFO, "model", "Waiting for Wi-Fi"), (BUTTON, "Download on mobile data", "")]);
        assert_eq!(PluginAct::of(&groups[2].1[1].name), Some(PluginAct::AllowMobile("a.one".into())));
        assert_eq!(groups[3].1.iter().map(|r| r.kind).collect::<Vec<_>>(), [README_HEADING, README_TEXT, README_ITEM, README_CODE]);
        let versions: Vec<(&str, &str, bool)> = groups[4].1.iter().map(|r| (r.title.as_str(), r.button.as_str(), r.enabled)).collect();
        assert_eq!(versions, [("2.0.0", "Install", true), ("1.0.0", "Installed", false), ("Source", "", true)]);
        assert_eq!(groups[4].1[2].detail, "code.example");
        for r in groups.iter().flat_map(|(_, rows)| rows).filter(|r| !r.name.is_empty()) {
            assert_eq!(PluginAct::of(&r.name).map(|a| a.name()), Some(r.name.clone()), "{} parses back", r.name);
        }

        let top = plugin_top(&p, false);
        assert!(top.back && top.tile);
        assert_eq!((top.title.as_str(), top.action.as_str(), PluginAct::of(&top.action_name)), ("a.one", "Update to 2.0.0", Some(PluginAct::Get(0, 0))));
        let menu: Vec<String> = slint::Model::iter(&top.menu_names).map(|n| n.to_string()).collect();
        assert_eq!(menu, ["plugin-down:a.one", "plugin-remove:a.one"], "the first can't move up");

        // Only offered, it has Install, no menu, no switches.
        p.cards.clear();
        p.controlled.clear();
        p.assets.clear();
        let top = plugin_top(&p, false);
        assert_eq!((top.action.as_str(), slint::Model::row_count(&top.menu)), ("Install", 0));
        let groups = plugin_groups(&p, false);
        assert_eq!(groups.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["About", "Versions"]);
        assert_eq!(groups[1].1[1].button, "Install", "no version is the one here");
    }

    /// A removed plugin's page closes; the browser it was opened from stays.
    #[test]
    fn removing_a_plugin_closes_its_page() {
        let mut p = Plugged { stack: vec![PluginPage::Browser, PluginPage::Plugin("a.one".into())], ..Default::default() };
        p.gone("a.one");
        assert_eq!(p.stack, [PluginPage::Browser]);
    }

    /// A README is asked once per plugin, version and address, and one that comes for a page left since
    /// is dropped.
    #[test]
    fn a_readme_is_asked_when_what_it_shows_changes() {
        let official = official();
        let with = |readme: &str| Offer { readme: Some(readme.into()), ..offer("a.one", &official, OfferState::New) };
        let mut p = Plugged { stack: vec![PluginPage::Plugin("a.one".into())], ..Default::default() };
        assert_eq!(p.readme_wanted().map(|w| w.0), Some("a.one".to_string()), "its package's, before any offer");
        assert!(p.readme_wanted().is_none());
        p.shelf = read(vec![with("https://r.example/a.md")]);
        assert_eq!(p.readme_wanted().and_then(|w| w.1).and_then(|o| o.readme).as_deref(), Some("https://r.example/a.md"), "once the offer says where");
        p.readme_came("a.one", vec![ReadmeBlock::Text { text: "Hello".into() }]);
        assert_eq!(p.readme.len(), 1);
        p.readme_came("a.other", Vec::new());
        assert_eq!(p.readme.len(), 1, "another page's");
        p.cards = vec![PluginCard { version: "3.0.0".into(), ..plugin_card("a.one", Vec::new()) }];
        assert!(p.readme_wanted().is_some(), "installed, its package's own");
        assert_eq!(p.readme.len(), 1, "kept until the next comes");
        p.stack = vec![PluginPage::Plugin("a.two".into())];
        assert!(p.readme_wanted().is_some());
        assert!(p.readme.is_empty(), "another plugin's isn't shown");
    }

    fn provider_offer(id: &str) -> Offer {
        Offer { points: vec!["provider".into()], ..offer(id, &official(), OfferState::New) }
    }

    /// With `offers` read and the plugins `cards` installed.
    fn offering(offers: Vec<Offer>, cards: Vec<PluginCard>) -> Plugged {
        Plugged { cards, shelf: Shelf::Read(Offers { offers, failed: Vec::new() }), ..Default::default() }
    }

    /// nori's own provider in ink, on its glyph; an offer quiet, opening its page or, installed, turning on.
    #[test]
    fn offers_are_quiet_and_say_what_choosing_does() {
        let installed = PluginCard { enabled: false, ..plugin_card("b.off", Vec::new()) };
        let mut plugged = offering(vec![provider_offer("a.new"), provider_offer("b.off"), provider_offer("c.on"), offer("d.sound", &official(), OfferState::New)], vec![installed, plugin_card("c.on", Vec::new())]);
        let rows = provider_choices(&plugged, &Kinds::builtin());
        plugged.all_off = true;
        let off = provider_choices(&plugged, &Kinds::builtin());
        let acts: Vec<Option<PluginAct>> = off[1..].iter().map(|r| PluginAct::of(&r.name)).collect();
        assert_eq!(acts, [Some(PluginAct::Sheet), Some(PluginAct::Sheet), Some(PluginAct::Sheet)], "with every plugin off, all three quiet, leading to the switch");
        assert!(off[1..].iter().all(|r| !r.enabled));
        let got: Vec<(&str, bool, MarkGlyph)> = rows.iter().map(|r| (r.title.as_str(), r.enabled, r.glyph)).collect();
        assert_eq!(got, [("OpenSubsonic", true, MarkGlyph::Server), ("a.new", false, MarkGlyph::None), ("b.off", false, MarkGlyph::None)], "only providers not on");
        let acts: Vec<_> = rows.iter().map(|r| PluginAct::of(&r.name)).collect();
        assert_eq!(acts, [Some(PluginAct::Choose(ServerKind::Subsonic)), Some(PluginAct::OfferPage("a.new".into())), Some(PluginAct::TurnOn("b.off".into()))]);
    }

    /// The welcome offers nori's own kind and every provider a repo offers that isn't on, each named as it is.
    #[test]
    fn the_welcome_offers_every_source() {
        let installed = PluginCard { enabled: false, ..plugin_card("b.off", Vec::new()) };
        let plugged = offering(vec![provider_offer("a.new"), provider_offer("b.off"), offer("d.sound", &official(), OfferState::New)], vec![installed]);
        let sources = welcome_sources(&plugged, &Kinds::builtin());
        let offered = |id: &str| WelcomeSource::Offered { plugin: id.into() };
        assert_eq!(sources, [WelcomeSource::Kind { kind: ServerKind::Subsonic }, offered("a.new"), offered("b.off")], "no sound plugin");
        let rows: Vec<(String, MarkGlyph, i32)> = sources.iter().zip([SourceMark::Pick(true), SourceMark::Installing, SourceMark::Pick(false)]).map(|(s, m)| source_row(&plugged, &Kinds::builtin(), s, m)).map(|r| (r.title.to_string(), r.glyph, r.mark)).collect();
        assert_eq!(rows, [("OpenSubsonic".into(), MarkGlyph::Server, 1), ("a.new".into(), MarkGlyph::None, 2), ("b.off".into(), MarkGlyph::None, 0)]);
    }

    /// An active plugin's provider is chosen as its kind, in ink.
    #[test]
    fn an_active_provider_chooses_its_kind() {
        let dir = nori_testdir::TempDir::new("desktop-providers");
        let host = nori_plugins::open(dir.path().join("plugins"), nori_core::settings_store::Settings::new(), std::sync::Arc::new(nori_host::desktop::Watched::default()));
        let id = host.install_from(nori_plugins::testing::fixture("").to_string_lossy().into_owned()).unwrap();
        host.turn(id.clone(), true);
        let rows = provider_choices(&Plugged::default(), &Kinds::with_plugins(host));
        assert_eq!((rows.len(), rows[1].enabled, rows[1].glyph), (2, true, MarkGlyph::None));
        assert_eq!(PluginAct::of(&rows[1].name), Some(PluginAct::Choose(ServerKind::Plugin { id })));
    }

    /// Repos that can't be read, or not read yet, leave the providers there are.
    #[test]
    fn providers_show_without_the_repos() {
        let failed = Plugged { shelf: Shelf::Read(Offers { offers: Vec::new(), failed: vec![nori_plugins::RepoFailure { repo: official(), problem: nori_plugins::RepoProblem::Unreachable }] }), ..Default::default() };
        for p in [failed, Plugged { shelf: Shelf::Asking, ..Default::default() }] {
            assert_eq!(provider_choices(&p, &Kinds::builtin()).iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["OpenSubsonic"]);
        }
    }

    /// The first screen's tiles are asked for, and only while it shows.
    #[test]
    fn the_first_screen_wants_its_offers_tiles() {
        let mut p = offering(vec![provider_offer("a.new")], Vec::new());
        assert!(p.icons_wanted().is_empty());
        p.choosing = true;
        assert_eq!(p.icons_wanted().iter().map(|w| w.0.as_str()).collect::<Vec<_>>(), ["a.new"]);
    }

    /// The first screen's row names parse back to what they do.
    #[test]
    fn provider_acts_round_trip() {
        for act in [PluginAct::Choose(ServerKind::Subsonic), PluginAct::Choose(ServerKind::Plugin { id: "a.b".into() }), PluginAct::TurnOn("a.b".into()), PluginAct::OfferPage("a.b".into()), PluginAct::Sheet, PluginAct::ChooseAgain, PluginAct::AddSigningIn, PluginAct::AddAsGuest] {
            assert_eq!(PluginAct::of(&act.name()), Some(act));
        }
    }

    /// A profile's mark is who signs in, else its name; a plugin's tile, or the server glyph for nori's own
    /// provider, when its icon is its provider's; else its face.
    #[test]
    fn a_profile_mark_is_its_user_its_plugin_or_its_face() {
        let mut p = Plugged::default();
        p.icons.read.insert("a.one".into(), None);
        let server = |user: &str, icon| SavedServer { user: user.into(), icon, ..Default::default() };
        let plugin = |icon| SavedServer { user: "zoe".into(), kind: ServerKind::Plugin { id: "a.one".into() }, icon, ..Default::default() };
        let profiles = [
            server("admin", ProfileIcon::Letter),
            server("", ProfileIcon::Letter),
            server("admin", ProfileIcon::Provider),
            server("admin", ProfileIcon::Sing),
            plugin(ProfileIcon::Letter),
            plugin(ProfileIcon::Provider),
            plugin(ProfileIcon::Think),
        ];
        let marks: Vec<(String, MarkGlyph)> = profiles.iter().map(|sv| profile_mark(&p, sv, "home")).map(|m| (m.letter.into(), m.glyph)).collect();
        let want = [("A", MarkGlyph::None), ("H", MarkGlyph::None), ("", MarkGlyph::Server), ("", MarkGlyph::Sing), ("Z", MarkGlyph::None), ("H", MarkGlyph::None), ("", MarkGlyph::Think)];
        assert_eq!(marks, want.map(|(l, g)| (l.to_string(), g)), "the plugin's tile has its own letter without an icon");
    }

    /// The menu keeps the saved order whichever profile is in use.
    #[test]
    fn the_profiles_menu_keeps_the_saved_order() {
        let sv = |id: &str| SavedServer { id: id.into(), name: id.to_uppercase(), url: format!("http://{id}"), ..Default::default() };
        let prefs = StoredPrefs { servers: vec![sv("a"), sv("b"), sv("c")], active_server_id: "b".into(), ..Default::default() };
        let items = profile_items(&Plugged::default(), &prefs, &Kinds::builtin());
        let lines: Vec<(String, bool)> = items.iter().map(|i| (i.title.to_string(), i.on)).collect();
        assert_eq!(lines, [("A".into(), false), ("B".into(), true), ("C".into(), false)]);
    }

    /// The picker offers every icon in its order, each as the circle would show it, the provider's named for it,
    /// and points at the one shown.
    #[test]
    fn the_icon_picker_shows_each_icon_and_the_one_shown() {
        let sv = SavedServer { user: "zoe".into(), icon: ProfileIcon::Satisfied, ..Default::default() };
        let picker = icon_picker(&Plugged::default(), &sv, "home", "OpenSubsonic");
        let choices: Vec<IconChoice> = slint::Model::iter(&picker.choices).collect();
        let names: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Letter", "OpenSubsonic", "Singing", "Surprised", "Talking", "Looking up", "Smiling", "Thinking"]);
        let glyphs: Vec<MarkGlyph> = choices.iter().map(|c| c.mark.glyph).collect();
        let faces = [MarkGlyph::Sing, MarkGlyph::Surprise, MarkGlyph::Talking, MarkGlyph::LookUp, MarkGlyph::Satisfied, MarkGlyph::Think];
        assert_eq!(glyphs, [[MarkGlyph::None, MarkGlyph::Server].as_slice(), &faces].concat());
        assert_eq!((choices[0].mark.letter.as_str(), picker.chosen), ("Z", 6));
    }

    /// A profile with a picture shows it where its letter would be, so that choice is named for the picture.
    #[test]
    fn the_icon_picker_names_a_picture_for_what_it_is() {
        let sv = SavedServer { user: "zoe".into(), picture: Some("https://example.com/zoe.jpg".into()), ..Default::default() };
        let picker = icon_picker(&Plugged::default(), &sv, "home", "YouTube Music");
        assert_eq!(slint::Model::row_data(&picker.choices, 0).map(|c| c.name.to_string()).as_deref(), Some("Picture"));
    }

    /// A tile is empty until the icon is heard of, then the icon, or the letter for a plugin with none;
    /// icons are asked for what shows, once per address.
    #[test]
    fn a_tile_waits_for_its_icon() {
        let official = official();
        let mut p = Plugged { cards: vec![plugin_card("a.one", Vec::new())], ..Default::default() };
        assert_eq!(p.tile("a.one", "one").1, "");
        p.icons.read.insert("a.one".into(), None);
        assert_eq!(p.tile("a.one", "one").1, "O");
        let image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
        p.icons.read.insert("a.one".into(), Some(image));
        let (icon, letter) = p.tile("a.one", "one");
        assert_eq!((icon.size().width, letter.as_str()), (2, ""));

        assert_eq!(p.icons_wanted().iter().map(|w| w.0.as_str()).collect::<Vec<_>>(), ["a.one"]);
        assert!(p.icons_wanted().is_empty(), "asked once");
        p.shelf = read(vec![offer("a.one", &official, OfferState::Installed), offer("b.new", &official, OfferState::New)]);
        let wanted = p.icons_wanted();
        assert_eq!(wanted.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(), ["a.one"], "the offer's address; the list doesn't show what is only offered");
        p.stack = vec![PluginPage::Browser];
        assert_eq!(p.icons_wanted().iter().map(|w| w.0.as_str()).collect::<Vec<_>>(), ["b.new"]);
    }

    /// A level says its value with its digits and unit, a hint is the quiet line, and a link opens its page.
    #[test]
    fn a_plugin_row_says_its_hint_unit_and_link() {
        let row = |kind, value, hint: Option<&str>, unit: Option<&str>, url: Option<&str>| PluginRow {
            id: "r".into(),
            kind,
            label: "Gain".into(),
            options: Vec::new(),
            min: Some(0.0),
            max: Some(10.0),
            step: None,
            value,
            hint: hint.map(Into::into),
            unit: unit.map(Into::into),
            decimals: 2,
            url: url.map(Into::into),
            page: None,
            glyph: None,
        };
        let level = plugin_row("p", &row(RowKind::Level, Some(Value::Level(1.5)), Some("Before the limiter"), Some("dB"), None), false, Opens::Settings);
        assert_eq!((level.title.as_str(), level.button.as_str(), level.detail.as_str()), ("Gain", "1.50 dB", "Before the limiter"));
        let status = plugin_row("p", &row(RowKind::Status, Some(Value::Text("Fine".into())), Some("Checked hourly"), None, None), false, Opens::Settings);
        assert_eq!(status.detail, "Fine · Checked hourly");
        let meter = plugin_row("p", &row(RowKind::Meter, Some(Value::Level(8.0)), None, Some("%"), None), false, Opens::Settings);
        assert_eq!(meter.detail, "8.00 %");
        let link = plugin_row("p", &row(RowKind::Link, None, None, None, Some("https://x.example/help")), false, Opens::Settings);
        assert_eq!((link.kind, PluginAct::of(&link.name)), (LINK, Some(PluginAct::Open("https://x.example/help".into()))));
    }

    /// A controlled device's card heads the devices panel, as Android's: its name over its batteries, its
    /// noise modes as chips (named so they parse back to the device) and a way on to its settings, which
    /// its dialog shows under its name.
    #[test]
    fn a_controlled_device_is_a_card_of_its_name_its_noise_chips_and_its_settings() {
        use nori_plugins::{Battery, Noise};
        use slint::Model;
        let card = |id: &str| PluginCard { id: id.into(), version: "1.0.0".into(), name: id.into(), about: None, enabled: true, repo: None, held: false, stopped: None, points: Vec::new(), overrides: Vec::new(), net: Vec::new(), lan: false, bluetooth: true, permissions: Vec::new(), hooks: Vec::new(), wakes: Vec::new(), commands: Vec::new(), cache_bytes: 0, asset_bytes: 0, rows: vec![PluginRow { id: "wear".into(), kind: RowKind::Switch, label: "Wear detection".into(), options: Vec::new(), min: None, max: None, step: None, value: Some(Value::Switch(true)), hint: None, unit: None, decimals: 0, url: None, page: None, glyph: None }] };
        let d = ControlledDevice { id: "AA".into(), name: "WH-1000XM5".into(), battery: Battery { single: Some(80), ..Default::default() }, level: Some(80), noise: Some(Noise::Canceling), noise_modes: vec![Noise::Off, Noise::Canceling], noise_cycle: vec![Noise::Off, Noise::Canceling], card: card("dev.sony") };
        let on = PluginRow { id: "on".into(), label: "Touch".into(), kind: RowKind::Switch, ..row_blank() };
        let d = ControlledDevice { card: PluginCard { rows: vec![on], ..d.card.clone() }, ..d };
        let controlled = std::slice::from_ref(&d);

        let rows = device_card(&d);
        assert_eq!((rows[0].kind, rows[0].title.as_str(), rows[0].detail.as_str()), (CONTROLLED, "WH-1000XM5", "80%"));
        assert_eq!((rows[1].kind, rows[1].chosen, PluginAct::of(&rows[1].name)), (SEGMENTS, Some(1), Some(PluginAct::Noise("AA".into()))));
        assert_eq!(rows[1].options.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), ["Normal", "ANC"]);
        assert_eq!((rows[2].kind, PluginAct::of(&rows[2].name)), (LINK, Some(PluginAct::DeviceOpen("AA".into()))));
        let shown = controlled_rows(controlled, &Default::default(), &Default::default(), Today { month: 10, day: 19, southern: false });
        assert_eq!(shown.iter().map(|r| (r.kind, r.first, r.last)).collect::<Vec<_>>(), [(CONTROLLED, true, false), (SEGMENTS, false, false), (LINK, false, true)], "one plate, no heading over it");
        assert!(panel_groups(&[card(&d.card.id), card("dev.other")], controlled, &HashSet::new()).iter().all(|(title, _)| title == "dev.other"), "its plugin's rows wait in its dialog");

        let mut p = Plugged { controlled: controlled.to_vec(), ..Default::default() };
        p.dialog_act(PluginAct::DeviceOpen("gone".into()));
        assert_eq!(p.dialog, None, "the device gone, no dialog");
        p.dialog_act(PluginAct::DeviceOpen("AA".into()));
        let shown = dialog_shown(&p, &Default::default(), &Default::default(), Today { month: 10, day: 19, southern: false }).unwrap();
        assert_eq!((shown.title.as_str(), shown.back), ("WH-1000XM5", false));
        let page: Vec<_> = shown.rows.iter().map(|r| (r.kind, r.detail.to_string(), r.name.to_string())).collect();
        assert_eq!(page[..2].iter().map(|r| (r.0, r.1.as_str())).collect::<Vec<_>>(), [(INFO, "80%"), (SEGMENTS, "")], "the title names it: its batteries alone, then the chips");
        assert_eq!(page[2..].iter().map(|r| (r.0, r.2.as_str())).collect::<Vec<_>>(), [(HEADING, ""), (TOGGLE, "plugin-row:dev.sony:on")]);
    }

    /// A plugin's segments are chips, as Android's; its choice is the popover's.
    #[test]
    fn a_plugins_segments_are_chips_and_its_choice_a_popover() {
        let r = |kind| PluginRow { id: "mode".into(), label: "Mode".into(), kind, options: vec!["A".into(), "B".into()], value: Some(Value::Choice(1)), ..row_blank() };
        let kinds: Vec<(i32, Option<i32>)> = [RowKind::Segments, RowKind::Choice].into_iter().map(|k| plugin_row("p", &r(k), false, Opens::Settings)).map(|row| (row.kind, row.chosen)).collect();
        assert_eq!(kinds, [(SEGMENTS, Some(1)), (CHOICE, Some(1))]);
    }

    /// A flags row is one row of chips under its label, as Android's, each lit while on and storing the bits
    /// with it switched.
    #[test]
    fn a_plugins_flags_are_chips() {
        let r = PluginRow { id: "cycle".into(), label: "Cycle".into(), kind: RowKind::Flags, options: vec!["A".into(), "B".into()], value: Some(Value::Flags(0b01)), ..row_blank() };
        let row = plugin_row("p", &r, false, Opens::Settings);
        assert_eq!((row.kind, row.title.as_str(), row.lit.as_slice()), (FLAGS, "Cycle", [true, false].as_slice()));
        let sends: Vec<_> = row.options.iter().map(|o| (o.0.as_str(), PluginAct::of(&o.1))).collect();
        assert_eq!(sends, [("A", Some(PluginAct::Flags("p".into(), "cycle".into(), 0b00))), ("B", Some(PluginAct::Flags("p".into(), "cycle".into(), 0b11)))]);
    }

    fn row_blank() -> PluginRow {
        PluginRow { id: String::new(), label: String::new(), kind: RowKind::Note, options: Vec::new(), value: None, hint: None, min: None, max: None, step: None, decimals: 0, unit: None, url: None, page: None, glyph: None }
    }
}
