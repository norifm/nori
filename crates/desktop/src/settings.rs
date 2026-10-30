//! Settings pages: the core's (`settings_pages`, desktop client), worded and drawn. Rows report back the
//! setting name and the picked value.

use std::collections::{HashMap, HashSet};
use nori_core::numbers::{self, POINT};
use nori_core::settings::{season_follow_value, season_of, EqLevel, Level, StoredPrefs, ThemeSeason};
use nori_core::settings_model::{self, BeatModel, EqStatus, KeyAction, SettingKind, SettingsState};
use nori_core::settings_pages::{self, PageFacts, RowItem, SectionId, SettingsAction, SettingsClient, SettingsLink, SettingsNote, SettingsPage, SettingsRow};
use slint::{ModelRc, SharedString, VecModel};

use nori_look::theme;

use nori_core::providers::Kinds;
use nori_plugins::{AssetLine, AssetState, ControlledDevice, Offer, OfferState, Offers, PluginCard, PluginRow, ReadmeBlock, Repo, RowKind, Value};

use crate::look::Today;
use crate::{words, PluginTool, PluginTop, SettingRow};

// Row kinds, as app.slint numbers them.
const HEADING: i32 = 0;
const TOGGLE: i32 = 1;
const CHOICE: i32 = 2;
const ACTION: i32 = 3;
const INFO: i32 = 4;
const SLIDER: i32 = 5;
const RANKED: i32 = 6;
const SERVER: i32 = 8;
/// A server row's Edit: the account sheet of the profile in use.
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

/// The season tiles and the "follow the seasons" switch under them.
pub struct Seasons {
    /// The tile worn (0 spring to 3 winter): the calendar's while following.
    pub worn: i32,
    pub following: bool,
    /// The switch's row name: the setting and the value flipping it sets ([`toggled`]).
    pub switch: String,
    /// "Autumn now · Winter from November 7" while following.
    pub detail: String,
}

const SEASON_NAMES: [&str; 4] = ["Spring", "Summer", "Autumn", "Winter"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

pub fn seasons(p: &StoredPrefs, today: Today) -> Seasons {
    let now = theme::season_now(today.month, today.day, today.southern);
    let following = p.season == ThemeSeason::Follow;
    let worn = season_of(p.season).unwrap_or(now.season) as i32;
    let detail = if following {
        format!("{} now · {} from {} {}", SEASON_NAMES[now.season as usize], SEASON_NAMES[now.next as usize], MONTHS[now.next_month as usize - 1], now.next_day)
    } else {
        "Changes with the calendar".into()
    };
    let switch = format!("season={}", season_follow_value(!following, today.month, today.day, today.southern));
    Seasons { worn, following, switch, detail }
}

/// Which appearance the settings stand for ([`appearance_theme`]'s pick).
pub fn appearance_pick(p: &StoredPrefs) -> i32 {
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
    pub analysed: u32,
    pub indexed: (u32, u32, u32),
    pub stream_bytes: u64,
    pub cover_bytes: u64,
    pub lyrics_bytes: u64,
    /// Every profile's downloads: they share the computer's download folder.
    pub download_bytes: u64,
    pub all_download_songs: u32,
    /// Each profile with downloads (the id its rows are kept under) and what they hold.
    pub download_profiles: Vec<(String, u64)>,
    /// This profile's finished downloads, which its actions work on.
    pub download_songs: u32,
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
const ADD_SERVER: &str = "add-server";
const BACKUP: &str = "backup";
const RESTORE: &str = "restore";
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
    AddServer,
    /// A server row's Edit: the account sheet of the profile in use where its provider has one to sign
    /// in to, else the profile in the sign-in form.
    Edit(String),
    /// Switch to the server with this id.
    Server(String),
    Chore(Chore),
    /// Asks where to save a backup, or which one to restore.
    Backup { restore: bool },
    /// Asks for the password of the backup at `path`.
    BackupPassword { restore: bool, path: String },
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
        if let Some((what, path)) = name.split_once(':').filter(|(what, _)| matches!(*what, BACKUP_PASSWORD | RESTORE_PASSWORD)) {
            return Some(Act::BackupPassword { restore: what == RESTORE_PASSWORD, path: path.to_string() });
        }
        match name {
            EQUALIZER => Some(Act::Equalizer),
            ADD_SERVER => Some(Act::AddServer),
            BACKUP => Some(Act::Backup { restore: false }),
            RESTORE => Some(Act::Backup { restore: true }),
            _ => CHORES.into_iter().find(|c| chore_name(*c) == name).map(Act::Chore),
        }
    }
}

pub use nori_host::session::Chore;

const CHORES: [Chore; 6] = [Chore::SyncLibrary, Chore::DownloadLibrary, Chore::MeasureAgain, Chore::ClearStream, Chore::ClearCovers, Chore::ClearLyrics];

/// A chore's row name in the UI.
fn chore_name(c: Chore) -> &'static str {
    match c {
        Chore::SyncLibrary => "sync-library",
        Chore::DownloadLibrary => "download-library",
        Chore::MeasureAgain => "measure-again",
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
    button: String,
    value: f32,
    min: f32,
    max: f32,
    /// A lyrics service's rank among them, and whether it is switched on only once given a key it lacks.
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
    /// A plugin row's icon and the letter drawn without one.
    icon: slint::Image,
    letter: String,
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

/// The desktop's categories: the pages the profile in use opens, in the core's order (`pages`). The
/// version, About's on Android, stands at the foot of Appearance.
pub fn tabs(parts: &[nori_core::settings_pages::SettingsPart]) -> Vec<SettingsPage> {
    settings_pages::pages(SettingsClient::Desktop, parts).into_iter().filter(|p| *p != SettingsPage::About).collect()
}

/// A category's name, as Android's settings list them.
pub fn tab_name(page: SettingsPage) -> &'static str {
    match page {
        SettingsPage::Servers => "Servers",
        SettingsPage::Playing => "Playback",
        SettingsPage::Sound => "Sound",
        SettingsPage::Look => "Appearance",
        SettingsPage::Lyrics | SettingsPage::LyricsSources => "Lyrics",
        SettingsPage::Library => "Library",
        SettingsPage::Data => "Downloads and storage",
        SettingsPage::About => "About",
    }
}

/// The settings page tab `tab` shows; the first where there is no such tab.
fn tab_page(f: &Facts, tab: i32) -> SettingsPage {
    tabs(&f.rules.settings).get(tab.max(0) as usize).copied().unwrap_or(SettingsPage::Look)
}

fn section_title(id: SectionId) -> &'static str {
    use SectionId::*;
    match id {
        Accounts => "Accounts",
        MusicFolders => "This server",
        ThisServer => "This server",
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
        "autoMix" => ("AutoMix", ""),
        "autoMixBetterBeats" => {
            let mb = s.beat_model_mb;
            let detail = match &s.beat_model {
                BeatModel::Ready => String::new(),
                BeatModel::Downloading => format!("Downloading (about {mb} MB)…"),
                BeatModel::Failed { .. } => "The download didn't work. It tries again at the next song.".into(),
                _ => format!("A download of about {mb} MB"),
            };
            return ("Better beat detection".into(), detail);
        }
        "crossfadeKeepAlbums" => ("Keep albums gapless", ""),
        "skipExplicit" => ("Skip explicit songs", ""),
        "autoFill" => ("Keep playing when the queue ends", ""),
        "autoFillKind" => ("Carry on with", ""),
        "autoFillBasis" => ("Chosen by", ""),
        "autoFillRemote" => ("Include remote songs", ""),
        "bridgeOffline" => ("Play downloads when offline", ""),
        "autoEqAuto" => ("AutoEQ for headphones", ""),
        "autoEqDownload" => ("Keep the AutoEQ list", ""),
        "profilePerOutput" => ("Remember sound per device", ""),
        "soundBypass" => ("No processing on this output", ""),
        "headphoneSound" => ("Headphone sound", ""),
        "compressor" => ("Compressor", ""),
        "compressorPreset" => ("Compressor type", ""),
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
        "scrobble" => ("Tell the server what you play", ""),
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
        ("autoFillKind", "SONGS") => "Songs".into(),
        ("autoFillKind", "ALBUMS") => "Albums".into(),
        ("autoFillBasis", "SIMILAR") => "Similar music".into(),
        ("autoFillBasis", "ARTIST") => "The same artist".into(),
        ("autoFillBasis", "GENRE") => "The same genre".into(),
        ("autoFillBasis", "ERA") => "The same era".into(),
        ("headphoneSound" | "replayGain", "OFF") => "Off".into(),
        ("headphoneSound", "NATURAL") => "Natural".into(),
        ("headphoneSound", "WIDE") => "Wide".into(),
        ("compressorPreset", "LIGHT") => "Light".into(),
        ("compressorPreset", "CAR") => "Car".into(),
        ("compressorPreset", "NIGHT") => "Night".into(),
        ("replayGain", "TRACK") => "Per song".into(),
        ("replayGain", "ALBUM") => "Per album".into(),
        ("replayGain", "AUTO") => "Automatic".into(),
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
fn row_of(r: SettingsRow, p: &StoredPrefs, s: &SettingsState, f: &Facts, kinds: &Kinds) -> Option<Row> {
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
                    let title = format!("{title}: {}", if v <= 0.0 { "Off".to_string() } else { format!("{} dB", crate::eq::signed(v)) });
                    Row { kind: SLIDER, name: spec.name, title, value: v.clamp(spec.min, spec.max), min: spec.min, max: spec.max, enabled: true, ..Default::default() }
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
            let n = f.analysed;
            let (songs, albums, artists) = f.indexed;
            let bytes = |b: u64| numbers::bytes(b as i64, POINT);
            match a {
                SettingsAction::MeasureAgain => action("Measured songs", format!("{} measured for tempo and beats.", words::count(n, "song", "songs")), "Measure again", enabled, Chore::MeasureAgain),
                SettingsAction::SyncLibrary => {
                    let counts = format!("{} · {} · {} on this Mac", words::count(songs, "song", "songs"), words::count(albums, "album", "albums"), words::count(artists, "artist", "artists"));
                    action("Offline search", counts, if f.syncing { "Updating…" } else { "Update" }, enabled, Chore::SyncLibrary)
                }
                SettingsAction::DownloadLibrary => action("Download the whole library", "Every song, at the download quality.".into(), "Download", enabled, Chore::DownloadLibrary),
                SettingsAction::ClearStream => action("Streamed music", format!("{}. Oldest goes first. Downloads stay.", bytes(f.stream_bytes)), "Clear", enabled, Chore::ClearStream),
                SettingsAction::ClearCovers => action("Covers", format!("{}. Fetched again when needed.", bytes(f.cover_bytes)), "Clear", enabled, Chore::ClearCovers),
                SettingsAction::ClearLyrics => action("Lyrics", format!("{} found online. Looked up again when needed.", bytes(f.lyrics_bytes)), "Clear", enabled, Chore::ClearLyrics),
                SettingsAction::AddServer => Row { kind: BUTTON, name: ADD_SERVER.into(), title: "Add server".into(), enabled: true, ..Default::default() },
                SettingsAction::MakeBackup => Row { kind: ACTION, name: BACKUP.into(), title: words::BACK_UP.into(), detail: words::BACK_UP_DETAIL.into(), button: "Save".into(), enabled, ..Default::default() },
                SettingsAction::RestoreBackup => Row { kind: ACTION, name: RESTORE.into(), title: words::RESTORE.into(), detail: words::RESTORE_DETAIL.into(), button: "Open".into(), enabled, ..Default::default() },
                SettingsAction::AnalyseDownloads | SettingsAction::JoinJam | SettingsAction::CheckUpdate => return None,
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
            let who = if sv.user.is_empty() { words.no_user } else { sv.user.as_str() };
            let mut detail = format!("{who} · {}", if active { "in use" } else { "not in use" });
            if !sv.alt_url.trim().is_empty() {
                detail.push_str(" · second address");
            }
            let title = Some(nori_core::settings::label(&sv.name, &sv.url)).filter(|l| !l.is_empty()).unwrap_or(words.name);
            // A jam guest switches between the profiles; changing them is not its to do.
            let edits = f.rules.settings.contains(&nori_core::settings_pages::SettingsPart::ServerOptions);
            Row { kind: SERVER, name: sv.id.clone(), title, detail, on: active, enabled: true, button: if edits { "Edit".into() } else { "".into() }, ..Default::default() }
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

/// The groups of settings tab `tab`: each section's title and rows. Tab 0 adds the desktop's own:
/// the output device and the version.
fn groups(p: &StoredPrefs, f: &Facts, kinds: &Kinds, tab: i32, today: Today) -> Vec<(String, Vec<Row>)> {
    let s = settings_model::state(p, settings_model::Output::default(), &crate::session::app().settings.model);
    let facts = PageFacts {
        parts: f.rules.settings.clone(),
        folders: f.folders.iter().map(|(_, id)| id.clone()).collect(),
        analysed: f.analysed,
        indexed_songs: f.indexed.0,
        syncing: f.syncing,
        stream_bytes: f.stream_bytes as i64,
        cover_bytes: f.cover_bytes as i64,
        lyrics_bytes: f.lyrics_bytes as i64,
        download_songs: f.download_songs,
        unsent_waiting: f.unsent.0,
        unsent_dropped: f.unsent.1,
        ..PageFacts::default()
    };
    let mut groups: Vec<(String, Vec<Row>)> = Vec::new();
    let page = tab_page(f, tab);
    for sec in settings_pages::page(page, SettingsClient::Desktop, p, &s, &facts) {
        let rows: Vec<Row> = sec.rows.into_iter().filter(|r| r.shown).flat_map(|r| flag_rows(&r, &s).unwrap_or_else(|| row_of(r, p, &s, f, kinds).into_iter().collect())).flat_map(|r| {
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
            groups.push(("Season".into(), season));
        }
        let title = section_title(sec.id);
        // Sections that share a title are one group here.
        match groups.last_mut() {
            Some((t, g)) if t == title => g.extend(rows),
            _ if rows.is_empty() => {}
            _ => groups.push((title.into(), rows)),
        }
    }
    if page == SettingsPage::Look {
        let mut devices = vec![("The system's own".to_string(), String::new())];
        devices.extend(f.devices.iter().map(|d| (d.clone(), d.clone())));
        let output = vec![
            Row { kind: CHOICE, name: DEVICE.into(), title: "Play through".into(), options: devices, enabled: true, ..Default::default() },
            note("A new device is used from the next start of nori."),
        ];
        groups.push(("Output device".into(), output));
        groups.push(("About".into(), vec![info("nori", format!("Version {}", env!("CARGO_PKG_VERSION")))]));
    }
    groups
}

/// Rows of settings tab `tab`: each group's heading, then its rows with first and last marked.
pub fn rows(p: &StoredPrefs, f: &Facts, kinds: &Kinds, tab: i32, today: Today) -> ModelRc<SettingRow> {
    model(groups(p, f, kinds, tab, today), p, f, today)
}

/// `groups` as the page draws them: each heading, then its rows with first and last marked.
fn model(groups: Vec<(String, Vec<Row>)>, p: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
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
            } else if r.kind == RANKED {
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
                icon: r.icon,
                letter: r.letter.into(),
                first: i == 0 || !plated[i - 1],
                last: i + 1 == n || !plated[i + 1],
            });
        }
    }
    ModelRc::new(VecModel::from(out))
}

/// A page over Settings › Plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginPage {
    Browser,
    Repos,
    /// A plugin's page, installed or only offered, by its id.
    Plugin(String),
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
    pub repos: Vec<Repo>,
    pub shelf: Shelf,
    /// The pages open over the list.
    pub stack: Vec<PluginPage>,
    /// The browser's search, and the repo it is narrowed to.
    pub query: String,
    pub repo_shown: Option<String>,
    /// What the README was last asked for ([`Plugged::readme_wanted`]), and what came.
    pub readme_asked: Option<(String, Option<String>, Option<String>)>,
    pub readme: Vec<ReadmeBlock>,
    pub icons: Icons,
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
            None => self.cards.iter().map(|c| c.id.clone()).collect(),
            Some(PluginPage::Browser) => nori_plugins::offers_found(offers.to_vec(), self.query.clone(), self.repo_shown.clone()).into_iter().map(|o| o.id).collect(),
            Some(PluginPage::Repos) => Vec::new(),
            Some(PluginPage::Plugin(id)) => vec![id.clone()],
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

    /// Closes the pages of a plugin just removed.
    pub fn gone(&mut self, id: &str) {
        self.stack.retain(|p| !matches!(p, PluginPage::Plugin(top) if top == id));
    }
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
    /// Back to what the page shown covers.
    Back,
    /// The browser's search, as typed.
    Search,
    /// The browser narrowed to a repo: its chip.
    Chip,
    /// Asks for a repo's address.
    AddRepo,
    /// The repo at this address, once trusted.
    TrustRepo(String),
    RemoveRepo(String),
    /// A controlled device's noise control, by the device's id.
    Noise(String),
    /// Lets a plugin fetch its assets on mobile data.
    AllowMobile(String),
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
            "plugin-back" => PluginAct::Back,
            "plugin-search" => PluginAct::Search,
            "plugin-chip" => PluginAct::Chip,
            "repo-add" => PluginAct::AddRepo,
            "repo-trust" => PluginAct::TrustRepo(id()),
            "repo-remove" => PluginAct::RemoveRepo(id()),
            "device-noise" => PluginAct::Noise(id()),
            "plugin-mobile" => PluginAct::AllowMobile(id()),
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
            PluginAct::Back => "plugin-back".into(),
            PluginAct::Search => "plugin-search".into(),
            PluginAct::Chip => "plugin-chip".into(),
            PluginAct::AddRepo => "repo-add".into(),
            PluginAct::TrustRepo(url) => format!("repo-trust:{url}"),
            PluginAct::RemoveRepo(url) => format!("repo-remove:{url}"),
            PluginAct::Noise(device) => format!("device-noise:{device}"),
            PluginAct::AllowMobile(id) => format!("plugin-mobile:{id}"),
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
    }
}

/// A plugin in a list: its tile, name, what it is, a quiet `line`; opening it opens its page.
fn plugin_line(p: &Plugged, id: &str, name: &str, about: Option<&str>, line: String) -> Row {
    let (icon, letter) = p.tile(id, name);
    Row { kind: PLUGIN, name: PluginAct::Page(id.into()).name(), title: name.into(), detail: about.unwrap_or_default().into(), line, icon, letter, enabled: true, ..Default::default() }
}

/// Updates waiting, and the installed plugins in their order, grey while every plugin is off.
fn plugin_list(p: &Plugged, all_off: bool) -> Vec<(String, Vec<Row>)> {
    if p.cards.is_empty() {
        let get = Row { kind: BUTTON, name: PluginAct::Browser.name(), title: words::GET_PLUGINS.into(), enabled: true, ..Default::default() };
        return vec![(String::new(), vec![note(words::NO_PLUGINS), get])];
    }
    let mut groups = Vec::new();
    let updates: Vec<Row> = nori_plugins::updates_waiting(p.offers(), &p.cards)
        .into_iter()
        .filter_map(|o| {
            let at = at_of(p.offers(), o);
            let line = words::update_line(o.installed.as_deref()?, &o.versions.first()?.version);
            Some(Row { control: PluginAct::Get(at as u32, 0).name(), button: words::UPDATE.into(), ..plugin_line(p, &o.id, &o.name, None, line) })
        })
        .collect();
    if !updates.is_empty() {
        groups.push((words::UPDATES.to_string(), updates));
    }
    let installed = p.cards.iter().map(|c| {
        let kinds = words::plugin_is(&c.points, !c.overrides.is_empty());
        let line = [c.version.clone(), kinds].into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" · ");
        Row { control: PluginAct::Turn(c.id.clone()).name(), on: c.enabled, enabled: !all_off, ..plugin_line(p, &c.id, &c.name, c.about.as_deref(), line) }
    });
    groups.push((words::INSTALLED_LIST.to_string(), installed.collect()));
    groups
}

/// `o`'s place among `offers`, which an install asks by.
fn at_of(offers: &[Offer], o: &Offer) -> usize {
    offers.iter().position(|x| x.id == o.id && x.repo.url == o.repo.url).unwrap_or(0)
}

/// What the repos offer, as searched and narrowed.
fn browser(p: &Plugged) -> Vec<(String, Vec<Row>)> {
    let mut rows = Vec::new();
    match &p.shelf {
        Shelf::Unasked | Shelf::Asking => rows.push(note(words::LOOKING)),
        Shelf::Read(Offers { offers, failed }) => {
            rows.extend(failed.iter().map(|f| note(&words::repo_failed(&f.repo))));
            let found = nori_plugins::offers_found(offers.clone(), p.query.clone(), p.repo_shown.clone());
            if found.is_empty() {
                rows.push(note(words::NONE_FOUND));
            }
            rows.extend(found.iter().filter_map(|o| {
                let line = format!("{} · {}", o.versions.first()?.version, words::repo_name(&o.repo));
                let (button, control) = match o.state {
                    OfferState::New => (words::INSTALL, PluginAct::Get(at_of(offers, o) as u32, 0).name()),
                    OfferState::Update => (words::UPDATE, PluginAct::Get(at_of(offers, o) as u32, 0).name()),
                    OfferState::Installed => (words::INSTALLED, String::new()),
                };
                Some(Row { button: button.into(), control, ..plugin_line(p, &o.id, &o.name, o.about.as_deref(), line) })
            }));
            rows.push(Row { kind: BUTTON, name: PluginAct::Look.name(), title: words::LOOK_AGAIN.into(), enabled: true, ..Default::default() });
        }
    }
    vec![(String::new(), rows)]
}

/// The repos and what each offers; those added can be removed.
fn repos_page(p: &Plugged) -> Vec<(String, Vec<Row>)> {
    let rows = p.repos.iter().map(|r| {
        let line = match &p.shelf {
            Shelf::Read(o) if o.failed.iter().any(|f| f.repo.url == r.url) => words::repo_failed(r),
            Shelf::Read(o) => words::plugin_count(o.offers.iter().filter(|x| x.repo.url == r.url).count()),
            _ => words::LOOKING.into(),
        };
        match r.official {
            true => info(&words::repo_name(r), line),
            false => Row { kind: ACTION, name: PluginAct::RemoveRepo(r.url.clone()).name(), title: words::repo_name(r), detail: format!("{} · {line}", r.url), button: words::REMOVE.into(), enabled: true, ..Default::default() },
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
        let mut head = vec![Row { kind: TOGGLE, name: PluginAct::Turn(c.id.clone()).name(), title: words::PLUGIN_ON.into(), on: c.enabled, enabled: true, ..Default::default() }];
        if c.repo.is_some() {
            head.push(Row { kind: TOGGLE, name: PluginAct::Hold(c.id.clone()).name(), title: words::KEEP_VERSION.into(), on: c.held, enabled: true, ..Default::default() });
        }
        let mut page = card_groups(String::new(), c, head, &p.off);
        let devices = p.controlled.iter().filter(|d| d.card.id == c.id);
        page.splice(1..1, devices.map(|d| (d.name.clone(), device_controls(d))).filter(|(_, rows)| !rows.is_empty()));
        groups.extend(page);
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
        groups.push((words::VERSIONS.to_string(), versions(card, at, o)));
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

/// Offer `o`, at place `at` among those read: its versions to install one of, and where its code is.
fn versions(card: Option<&PluginCard>, at: usize, o: &Offer) -> Vec<Row> {
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
                enabled: !here,
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

/// The head of what shows: its title and tools, and on a plugin's page what it is and the way to get it.
pub fn plugin_top(p: &Plugged, all_off: bool) -> PluginTop {
    let back = !p.stack.is_empty();
    match p.stack.last() {
        None => PluginTop { title: words::PLUGINS.into(), back, tools: tools(&[(TOOL_PLUS, words::GET_PLUGINS, PluginAct::Browser)]), every: !p.cards.is_empty(), all_on: !all_off, ..Default::default() },
        Some(PluginPage::Repos) => PluginTop { title: words::REPOS.into(), back, tools: tools(&[(TOOL_PLUS, words::ADD_REPO, PluginAct::AddRepo)]), ..Default::default() },
        Some(PluginPage::Browser) => {
            let shown = p.repos.iter().position(|r| p.repo_shown.as_ref() == Some(&r.url)).map_or(0, |i| i as i32 + 1);
            let chips = if p.repos.len() > 1 { std::iter::once(words::ALL.to_string()).chain(p.repos.iter().map(words::repo_name)).collect() } else { Vec::new() };
            PluginTop {
                title: words::GET_PLUGINS.into(),
                back,
                tools: tools(&[(TOOL_FOLDER, words::INSTALL_FROM_FILE, PluginAct::Install), (TOOL_SERVER, words::REPOS, PluginAct::Repos)]),
                search: words::SEARCH_PLUGINS.into(),
                query: p.query.as_str().into(),
                chips: strings(chips),
                chip: shown,
                ..Default::default()
            }
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
                (None, _) => Some((words::INSTALL.to_string(), offer_at)),
                (_, OfferState::Update) => Some((words::update_to(&v.version), offer_at)),
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
                action_name: get.map(|g| PluginAct::Get(g.1 as u32, 0).name()).unwrap_or_default().into(),
                menu: strings(labels),
                menu_names: strings(names),
                ..Default::default()
            }
        }
    }
}

/// One row of plugin `plugin`'s page, drawn as the settings' own rows are; its hint is the quiet line
/// under it. Off, it shows but takes no input.
fn plugin_row(plugin: &str, r: &PluginRow, off: bool) -> Row {
    let name = PluginAct::Row(plugin.into(), r.id.clone()).name();
    let hint = r.hint.clone().unwrap_or_default();
    let base = Row { name, title: r.label.clone(), detail: hint.clone(), enabled: !off, ..Default::default() };
    // A value said on the quiet line, the hint after it.
    let said = |value: String| [value, hint.clone()].into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" · ");
    match r.kind {
        RowKind::Switch => Row { kind: TOGGLE, on: r.value == Some(Value::Switch(true)), ..base },
        RowKind::Choice => {
            let chosen = match r.value {
                Some(Value::Choice(i)) => i as i32,
                _ => 0,
            };
            Row { kind: CHOICE, options: r.options.iter().map(|o| (o.clone(), o.clone())).collect(), chosen: Some(chosen), ..base }
        }
        RowKind::Level => {
            let (min, max) = (r.min.unwrap_or(0.0), r.max.unwrap_or(1.0));
            let value = match r.value {
                Some(Value::Level(v)) => v,
                _ => min,
            };
            Row { kind: SLIDER, title: format!("{}: {}", r.label, words::plugin_level(value, r.decimals, r.unit.as_deref())), value: value as f32, min: min as f32, max: max as f32, ..base }
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
        RowKind::Status => Row { kind: INFO, detail: said(r.value.as_ref().map(|v| words::plugin_value(v, &r.options)).unwrap_or_default()), ..base },
        RowKind::Link => match &r.url {
            Some(url) => Row { kind: LINK, name: PluginAct::Open(url.clone()).name(), ..base },
            None => Row { kind: INFO, ..base },
        },
        RowKind::Note | RowKind::Section => note(&r.label),
    }
}

/// Card `c`'s page as groups: its rows under `title` (after `head`), each section under its own; those
/// in `off` take no input.
fn card_groups(title: String, c: &PluginCard, head: Vec<Row>, off: &HashSet<(String, String)>) -> Vec<(String, Vec<Row>)> {
    let mut groups = Vec::new();
    let (mut title, mut rows) = (title, head);
    for r in &c.rows {
        match r.kind {
            RowKind::Section => groups.push((std::mem::replace(&mut title, r.label.clone()), std::mem::take(&mut rows))),
            _ => rows.push(plugin_row(&c.id, r, off.contains(&(c.id.clone(), r.id.clone())))),
        }
    }
    groups.push((title, rows));
    groups
}

/// The devices panel's plugin rows: each controlled device's page (its batteries and noise control,
/// then its plugin's rows), then the rows of the plugins `cards` show there, as on their pages.
pub fn device_rows(cards: &[PluginCard], controlled: &[ControlledDevice], off: &HashSet<(String, String)>, prefs: &StoredPrefs, f: &Facts, today: Today) -> ModelRc<SettingRow> {
    model(device_groups(cards, controlled, off), prefs, f, today)
}

/// A controlled device's batteries and noise control.
fn device_controls(d: &ControlledDevice) -> Vec<Row> {
    let mut rows = Vec::new();
    let battery = words::battery(&d.battery);
    if !battery.is_empty() {
        rows.push(info(words::BATTERY, battery));
    }
    if !d.noise_modes.is_empty() {
        let options = d.noise_modes.iter().map(|m| (words::noise(*m).to_string(), String::new())).collect();
        let chosen = d.noise.and_then(|n| d.noise_modes.iter().position(|m| *m == n)).map_or(-1, |i| i as i32);
        rows.push(Row { kind: CHOICE, name: PluginAct::Noise(d.id.clone()).name(), title: words::NOISE_CONTROL.into(), options, chosen: Some(chosen), enabled: true, ..Default::default() });
    }
    rows
}

fn device_groups(cards: &[PluginCard], controlled: &[ControlledDevice], off: &HashSet<(String, String)>) -> Vec<(String, Vec<Row>)> {
    let mut groups = Vec::new();
    for d in controlled {
        groups.push((d.name.clone(), device_controls(d)));
        if !cards.iter().any(|c| c.id == d.card.id) {
            groups.extend(card_groups(d.card.name.clone(), &d.card, Vec::new(), off));
        }
    }
    groups.extend(cards.iter().flat_map(|c| card_groups(c.name.clone(), c, Vec::new(), off)));
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

    /// A key that is optional has its button; one needed is asked for when switching on; one kept changes.
    #[test]
    fn lyrics_keys_are_asked_as_on_android() {
        let facts = Facts { rules: nori_core::browse::profile_rules(false), ..Default::default() };
        let tab = tabs(&facts.rules.settings).iter().position(|p| *p == SettingsPage::Lyrics).unwrap() as i32;
        let today = Today { month: 10, day: 19, southern: false };
        let source = |p: &StoredPrefs, id: &str| {
            let rows = rows(p, &facts, &Kinds::builtin(), tab, today);
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
            download_songs: 3,
            ..Default::default()
        };
        let p = StoredPrefs { servers: vec![SavedServer { id: "a".into(), name: "Home".into(), ..Default::default() }], active_server_id: "a".into(), ..Default::default() };
        let today = Today { month: 10, day: 19, southern: false };
        let usage = |f: &Facts| (0..8).flat_map(|tab| groups(&p, f, &Kinds::builtin(), tab, today)).flat_map(|(_, rows)| rows).find(|r| r.kind == USAGE).unwrap();
        let both = usage(&facts(vec![("a".into(), 2 << 20), ("gone".into(), 1 << 20)]));
        assert_eq!(both.options[1].0, "5 downloads · 3.0 MB");
        assert_eq!(both.breakdown, ["Home · 2.0 MB", "A removed profile · 1.0 MB"]);
        assert!(usage(&facts(vec![("a".into(), 3 << 20)])).breakdown.is_empty(), "one profile needs no split");
    }

    #[test]
    fn tabs_are_the_pages_the_profile_opens() {
        let names = |guest| tabs(&nori_core::browse::profile_rules(guest).settings).into_iter().map(tab_name).collect::<Vec<_>>();
        assert_eq!(names(false), ["Servers", "Playback", "Sound", "Appearance", "Lyrics", "Library", "Downloads and storage"]);
        assert!(!names(true).contains(&"Library"), "a jam guest has no library settings to show");
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
        let look = tabs(&facts.rules.settings).iter().position(|p| *p == SettingsPage::Look).map_or(0, |i| i as i32);
        let rows = rows(&StoredPrefs::default(), &facts, &Kinds::builtin(), look, OCTOBER);
        let kinds: Vec<_> = rows.iter().map(|r| r.kind).collect();
        let at = kinds.iter().position(|k| *k == SEASONS).expect("a season row");
        assert_eq!(kinds[at - 1..at + 4], [HEADING, SEASONS, TOGGLE, HEADING, APPEARANCE_ROW], "the season under its heading, then the theme's");
        let tiles = rows.row_data(at).unwrap();
        assert_eq!((tiles.options.row_count(), tiles.chosen), (4, 2), "the four seasons, autumn worn while following");
        let follow = rows.row_data(at + 1).unwrap();
        assert_eq!((follow.on, follow.detail.as_str()), (true, "Autumn now · Winter from November 7"));
        // Turned off, it keeps the calendar's season.
        assert_eq!(toggled(&follow.name, false), ("season", "AUTUMN".to_string()));
        let fixed = StoredPrefs { season: ThemeSeason::Spring, ..StoredPrefs::default() };
        let s = seasons(&fixed, OCTOBER);
        assert_eq!((s.worn, s.following, toggled(&s.switch, true).1.as_str()), (0, false, "FOLLOW"));
    }

    #[test]
    fn a_toggle_stores_its_state_or_the_value_its_name_carries() {
        assert_eq!(toggled("mono", true), ("mono", "true".to_string()));
        assert_eq!(toggled("musicFolders=1,3", false), ("musicFolders", "1,3".to_string()));
        assert_eq!(toggled("musicFolders=", true), ("musicFolders", String::new()));
    }

    #[test]
    fn row_names_resolve() {
        let p = StoredPrefs { auto_mix: true, auto_fill: true, compressor: true, scrobble: true, ..StoredPrefs::default() };
        let f = Facts::default();
        let specs: Vec<String> = settings_model::specs().into_iter().map(|s| s.name).collect();
        for tab in 0..7 {
            for (_, rows) in groups(&p, &f, &Kinds::builtin(), tab, OCTOBER) {
                for r in rows.iter().filter(|r| matches!(r.kind, TOGGLE | CHOICE) && r.name != DEVICE) {
                    let name = toggled(&r.name, r.on).0;
                    assert!(specs.iter().any(|s| s == name), "{name} is not a core setting");
                    assert!(!setting_words(name, &settings_model::state(&p, settings_model::Output::default(), &crate::session::app().settings.model)).0.eq(name), "{name} has no words");
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
        PluginCard { id: id.into(), version: "1.0.0".into(), name: id.into(), about: Some("Does it".into()), enabled: true, repo: Some(nori_plugins::OFFICIAL.into()), held: false, points: Vec::new(), overrides: Vec::new(), net: Vec::new(), lan: false, bluetooth: false, permissions: Vec::new(), hooks: Vec::new(), wakes: Vec::new(), commands: Vec::new(), cache_bytes: 0, asset_bytes: 0, rows }
    }

    fn official() -> Repo {
        Repo { url: nori_plugins::OFFICIAL.into(), official: true }
    }

    fn offer(id: &str, repo: &Repo, state: OfferState) -> Offer {
        let version = |v: &str| Version { repo: repo.url.clone(), version: v.into(), released: None, url: String::new(), sha256: String::new(), key: None, signature: None };
        let installed = (state != OfferState::New).then(|| "1.0.0".to_string());
        Offer { repo: repo.clone(), id: id.into(), name: id.into(), about: None, source: Some("https://code.example/x".into()), readme: None, icon: Some(format!("https://i.example/{id}.png")), versions: vec![version("2.0.0"), version("1.0.0")], installed, state }
    }

    fn row(id: &str, kind: RowKind, value: Option<Value>) -> PluginRow {
        PluginRow { id: id.into(), kind, label: id.into(), options: vec!["a".into(), "b".into()], min: Some(0.0), max: Some(2.0), step: Some(0.5), value, hint: None, unit: None, decimals: 1, url: None }
    }

    fn read(offers: Vec<Offer>) -> Shelf {
        Shelf::Read(Offers { offers, failed: Vec::new() })
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

    /// The browser searches through the core and shows by name, while a row's Install still names the
    /// offer by its place among those read.
    #[test]
    fn the_browser_shows_what_is_found_and_installs_the_offer_shown() {
        let friend = Repo { url: "https://friend.example/index.json".into(), official: false };
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

    /// Repos say what each offers; only those added can go.
    #[test]
    fn the_repos_say_what_they_offer() {
        let friend = Repo { url: "https://friend.example/index.json".into(), official: false };
        let official = official();
        let gone = Repo { url: "https://gone.example/index.json".into(), official: false };
        let p = Plugged {
            repos: vec![official.clone(), friend.clone(), gone.clone()],
            stack: vec![PluginPage::Repos],
            shelf: Shelf::Read(Offers { offers: vec![offer("a", &official, OfferState::New), offer("b", &official, OfferState::New), offer("c", &friend, OfferState::New)], failed: vec![nori_plugins::RepoFailure { repo: gone, reason: "HTTP 404".into() }] }),
            ..Default::default()
        };
        let groups = plugin_groups(&p, false);
        let rows = &groups[0].1;
        assert_eq!((rows[0].kind, rows[0].detail.as_str()), (INFO, "2 plugins"));
        assert_eq!((rows[1].button.as_str(), PluginAct::of(&rows[1].name)), ("Remove", Some(PluginAct::RemoveRepo(friend.url.clone()))));
        assert!(rows[1].detail.ends_with("1 plugin"), "{}", rows[1].detail);
        assert!(rows[2].detail.contains("Couldn’t reach gone.example"), "{}", rows[2].detail);
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
        let buds = ControlledDevice { id: "AA".into(), name: "Buds".into(), battery: nori_plugins::Battery { single: Some(80), ..Default::default() }, level: Some(80), noise: None, noise_modes: Vec::new(), card: plugin_card("a.one", Vec::new()) };
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
        assert_eq!(titles, ["", "Buds", "more", "Files", "About", "Versions"], "nothing to say of what it may do, no group");
        let one: Vec<(i32, &str)> = groups[0].1.iter().map(|r| (r.kind, r.name.as_str())).collect();
        assert_eq!(one, [(TOGGLE, "plugin-on:a.one"), (TOGGLE, "plugin-hold:a.one"), (TOGGLE, "plugin-row:a.one:on"), (CHOICE, "plugin-row:a.one:pick"), (SLIDER, "plugin-row:a.one:gain"), (ACTION, "plugin-row:a.one:key"), (BUTTON, "plugin-row:a.one:go")]);
        assert!(!groups[0].1[3].enabled, "a row its plugin turned off takes no input");
        assert_eq!(groups[0].1[3].chosen, Some(1));
        assert_eq!(groups[0].1[5].detail, "Set", "a secret stays hidden");
        assert_eq!((groups[1].1[0].kind, groups[1].1[0].detail.as_str()), (INFO, "80%"), "the device it controls, with its battery");
        assert_eq!((groups[2].1[0].kind, groups[2].1[0].detail.as_str()), (INFO, "Fine"));
        let files: Vec<(i32, &str, &str)> = groups[3].1.iter().map(|r| (r.kind, r.title.as_str(), r.detail.as_str())).collect();
        assert_eq!(files, [(INFO, "model", "Waiting for Wi-Fi"), (BUTTON, "Download on mobile data", "")]);
        assert_eq!(PluginAct::of(&groups[3].1[1].name), Some(PluginAct::AllowMobile("a.one".into())));
        assert_eq!(groups[4].1.iter().map(|r| r.kind).collect::<Vec<_>>(), [README_HEADING, README_TEXT, README_ITEM, README_CODE]);
        let versions: Vec<(&str, &str, bool)> = groups[5].1.iter().map(|r| (r.title.as_str(), r.button.as_str(), r.enabled)).collect();
        assert_eq!(versions, [("2.0.0", "Install", true), ("1.0.0", "Installed", false), ("Source", "", true)]);
        assert_eq!(groups[5].1[2].detail, "code.example");
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
        };
        let level = plugin_row("p", &row(RowKind::Level, Some(Value::Level(1.5)), Some("Before the limiter"), Some("dB"), None), false);
        assert_eq!((level.title.as_str(), level.detail.as_str()), ("Gain: 1.50 dB", "Before the limiter"));
        let status = plugin_row("p", &row(RowKind::Status, Some(Value::Text("Fine".into())), Some("Checked hourly"), None, None), false);
        assert_eq!(status.detail, "Fine · Checked hourly");
        let link = plugin_row("p", &row(RowKind::Link, None, None, None, Some("https://x.example/help")), false);
        assert_eq!((link.kind, PluginAct::of(&link.name)), (LINK, Some(PluginAct::Open("https://x.example/help".into()))));
    }

    /// A controlled device's page leads the devices panel: its batteries, its noise control (named so
    /// it parses back to the device), then its plugin's rows, shown once.
    #[test]
    fn a_controlled_device_leads_the_devices_panel() {
        use nori_plugins::{Battery, Noise};
        let card = |id: &str| PluginCard { id: id.into(), version: "1.0.0".into(), name: id.into(), about: None, enabled: true, repo: None, held: false, points: Vec::new(), overrides: Vec::new(), net: Vec::new(), lan: false, bluetooth: true, permissions: Vec::new(), hooks: Vec::new(), wakes: Vec::new(), commands: Vec::new(), cache_bytes: 0, asset_bytes: 0, rows: vec![PluginRow { id: "wear".into(), kind: RowKind::Switch, label: "Wear detection".into(), options: Vec::new(), min: None, max: None, step: None, value: Some(Value::Switch(true)), hint: None, unit: None, decimals: 0, url: None }] };
        let d = ControlledDevice { id: "AA".into(), name: "WH-1000XM5".into(), battery: Battery { single: Some(80), ..Default::default() }, level: Some(80), noise: Some(Noise::Canceling), noise_modes: vec![Noise::Off, Noise::Canceling], card: card("dev.sony") };
        let groups = device_groups(&[card("dev.sony")], std::slice::from_ref(&d), &HashSet::new());
        assert_eq!(groups.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["WH-1000XM5", "dev.sony"], "its plugin's rows once");
        let rows = &groups[0].1;
        assert_eq!((rows[0].kind, rows[0].detail.as_str()), (INFO, "80%"));
        assert_eq!((rows[1].kind, rows[1].chosen, PluginAct::of(&rows[1].name)), (CHOICE, Some(1), Some(PluginAct::Noise("AA".into()))));
        assert_eq!(device_groups(&[], std::slice::from_ref(&d), &HashSet::new()).len(), 2, "its plugin's rows show though the plugin names no place");
    }
}
