//! Settings pages: the core's (`settings_pages`, desktop client), worded and drawn. Rows report back the
//! setting name and the picked value.

use nori_core::numbers::{self, POINT};
use nori_core::settings::{season_follow_value, season_of, EqLevel, StoredPrefs, ThemeSeason};
use nori_core::settings_model::{self, BeatModel, SettingKind, SettingsState};
use nori_core::settings_pages::{self, PageFacts, RowItem, SectionId, SettingsAction, SettingsClient, SettingsLink, SettingsNote, SettingsPage, SettingsRow};
use slint::{ModelRc, SharedString, VecModel};

use nori_look::theme;

use crate::look::Today;
use crate::{words, SettingRow};

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
/// The season tiles, and light / dark / AMOLED.
const SEASONS: i32 = 13;
const APPEARANCE_ROW: i32 = 14;
/// What is stored, as a bar of its parts.
const USAGE: i32 = 15;

/// The appearance row's name: it is not one setting but two (the theme and AMOLED).
pub const APPEARANCE: &str = "appearance";

/// The appearance a pick stands for: the system's, light, dark, or AMOLED (dark on black).
pub fn appearance_settings(pick: usize) -> [(&'static str, &'static str); 2] {
    match pick {
        1 => [("theme", "LIGHT"), ("amoled", "false")],
        2 => [("theme", "DARK"), ("amoled", "false")],
        3 => [("theme", "DARK"), ("amoled", "true")],
        _ => [("theme", "SYSTEM"), ("amoled", "false")],
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

/// Which appearance the settings stand for ([`appearance_settings`]'s pick).
pub fn appearance_pick(p: &StoredPrefs) -> i32 {
    use nori_core::settings::ThemeMode;
    match p.theme {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark if p.amoled => 3,
        ThemeMode::Dark => 2,
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
    pub download_bytes: u64,
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
        match name {
            EQUALIZER => Some(Act::Equalizer),
            ADD_SERVER => Some(Act::AddServer),
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
}

fn options(name: &str) -> Vec<String> {
    settings_model::specs().into_iter().find(|s| s.name == name).map(|s| s.options).unwrap_or_default()
}

fn minus(v: &str) -> String {
    v.replace('-', "−")
}

/// The desktop's tabs in its order: the pages the profile in use opens (the core's `pages`).
pub fn tabs(parts: &[nori_core::settings_pages::SettingsPart]) -> Vec<SettingsPage> {
    use SettingsPage::*;
    let offered = settings_pages::pages(SettingsClient::Desktop, parts);
    [Look, Playing, Sound, Lyrics, Library, Data, Servers].into_iter().filter(|p| offered.contains(p)).collect()
}

/// A tab's name.
pub fn tab_name(page: SettingsPage) -> &'static str {
    match page {
        SettingsPage::Look => "General",
        SettingsPage::Playing => "Playback",
        SettingsPage::Sound => "Sound",
        SettingsPage::Lyrics | SettingsPage::LyricsSources => "Lyrics",
        SettingsPage::Library => "Library",
        SettingsPage::Data => "Storage",
        SettingsPage::Servers => "Accounts",
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
        MusicFolders => "Music folders",
        ThisServer => "This server",
        Between => "Between songs",
        Controls => "Controls",
        Queue => "Queue",
        Equalizer => "Equalizer",
        Effects => "Effects",
        Volume => "Volume",
        Output => "Output",
        Theme => "Look",
        Home => "Home",
        Screen => "Screen",
        Motion => "Motion",
        Display => "Display",
        Sources => "Sources",
        AskedOrder => "In the order they're asked",
        Lists => "Lists",
        Search => "Search",
        History => "Listening history",
        Online => "Online",
        Devices => "Other devices",
        Streaming => "Streaming",
        Downloads => "Downloads",
        Storage => "Storage",
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
        "fadeMs" => ("Fade on play and pause", ""),
        "previousAlwaysSkips" => ("Previous goes back a song", ""),
        "speed" => ("Speed", ""),
        "pitch" => ("Pitch", ""),
        "skipSilence" => ("Skip silence", if s.untouched { "Off in bit-perfect mode." } else { "" }),
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
        "coverColors" => ("Colors from the cover", ""),
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
    let seconds = |v: &str| format!("{v} s");
    match (name, v) {
        ("crossfadeSec" | "fadeMs", "0") => "Off".into(),
        ("crossfadeSec", v) => seconds(v),
        ("fadeMs", v) => match v.parse::<u32>() {
            Ok(ms) if ms % 1000 == 0 => seconds(&(ms / 1000).to_string()),
            _ => format!("{v} ms"),
        },
        ("speed", "1") | ("pitch", "1") => "Normal".into(),
        ("speed", v) => format!("{v}×"),
        ("pitch", v) => {
            let pct = ((v.parse::<f32>().unwrap_or(1.0) - 1.0) * 100.0).round() as i32;
            if pct < 0 { format!("−{} %", -pct) } else { format!("+{pct} %") }
        }
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
        ("loudnessLevel", "QUIET") => format!("Quiet ({} LUFS)", minus("-23")),
        ("loudnessLevel", "NORMAL") => format!("Normal ({} LUFS)", minus("-18")),
        ("loudnessLevel", "LOUD") => format!("Loud ({} LUFS)", minus("-14")),
        ("maxRate", "AUTO") => "Each song's own".into(),
        ("maxRate", "KHZ48") => "48 kHz".into(),
        ("maxRate", "KHZ96") => "96 kHz".into(),
        ("maxRate", "KHZ192") => "192 kHz".into(),
        ("coverColors", "PAGES") => "Pages".into(),
        ("coverColors", "PLAYER") => "Player".into(),
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
fn row_of(r: SettingsRow, p: &StoredPrefs, s: &SettingsState, f: &Facts) -> Option<Row> {
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
            let status = if s.untouched { "Off now" } else if s.sound_chain_on { "On" } else { "Off" };
            Row { kind: LINK, name: EQUALIZER.into(), title: "Equalizer".into(), detail: status.into(), enabled: true, ..Default::default() }
        }
        RowItem::Stored => {
            let bytes = |b: u64| numbers::bytes(b as i64, POINT);
            let downloads = words::count(f.download_songs, "download", "downloads");
            let parts = [("Streamed music", f.stream_bytes), (downloads.as_str(), f.download_bytes), ("Covers", f.cover_bytes), ("Lyrics", f.lyrics_bytes), ("Library index", f.database_bytes)];
            let total: u64 = parts.iter().map(|p| p.1).sum();
            Row {
                kind: USAGE,
                title: "Stored on this computer".into(),
                detail: bytes(total),
                options: parts.iter().map(|(name, b)| (format!("{name} · {}", bytes(*b)), String::new())).collect(),
                shares: parts.iter().map(|p| p.1 as f32 / total.max(1) as f32).collect(),
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
            let words = crate::words::provider(sv.kind);
            let who = if sv.user.is_empty() { words.no_user } else { sv.user.as_str() };
            let mut detail = format!("{who} · {}", if active { "in use" } else { "not in use" });
            if !sv.alt_url.trim().is_empty() {
                detail.push_str(" · second address");
            }
            let title = Some(nori_core::settings::label(&sv.name, &sv.url)).filter(|l| !l.is_empty()).unwrap_or_else(|| words.name.into());
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
            // As on Android: a key that is optional has a button beside the switch, one kept can be changed
            // there, and one needed is asked for when the service is switched on (`keyed`).
            let button = match (src.key_set, src.takes_key && !src.needs_key) {
                (true, _) => "Change key",
                (false, true) => "Add key",
                (false, false) => "",
            };
            Row { kind: RANKED, name: src.id.clone(), title: title.into(), detail: about.into(), on: src.on, enabled: r.enabled, button: button.into(), rank: index as i32, keyed: src.needs_key && !src.key_set, ..Default::default() }
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
fn groups(p: &StoredPrefs, f: &Facts, tab: i32, today: Today) -> Vec<(&'static str, Vec<Row>)> {
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
    let mut groups: Vec<(&'static str, Vec<Row>)> = Vec::new();
    let page = tab_page(f, tab);
    for sec in settings_pages::page(page, SettingsClient::Desktop, p, &s, &facts) {
        let rows: Vec<Row> = sec.rows.into_iter().filter(|r| r.shown).flat_map(|r| flag_rows(&r, &s).unwrap_or_else(|| row_of(r, p, &s, f).into_iter().collect())).flat_map(|r| {
            if r.kind != SEASONS {
                return vec![r];
            }
            // The four seasons' tiles, following the calendar as a switch under them, then the appearance: the
            // theme and AMOLED have no rows on the desktop's page of the core, and it stands for both.
            let s = seasons(p, today);
            let tiles = Row { options: SEASON_NAMES.map(|n| (n.to_string(), n.to_string())).to_vec(), ..r };
            let follow = Row { kind: TOGGLE, name: s.switch, title: "Follow the seasons".into(), detail: s.detail, on: s.following, enabled: true, ..Default::default() };
            let appearance = Row { kind: APPEARANCE_ROW, name: APPEARANCE.into(), options: ["System", "Light", "Dark", "AMOLED"].map(|o| (o.to_string(), o.to_string())).to_vec(), enabled: true, ..Default::default() };
            vec![tiles, follow, appearance]
        }).collect();
        let title = section_title(sec.id);
        // Sections that share a title are one group here.
        match groups.last_mut() {
            Some((t, g)) if *t == title => g.extend(rows),
            _ if rows.is_empty() => {}
            _ => groups.push((title, rows)),
        }
    }
    if page == SettingsPage::Look {
        let mut devices = vec![("The system's own".to_string(), String::new())];
        devices.extend(f.devices.iter().map(|d| (d.clone(), d.clone())));
        let output = vec![
            Row { kind: CHOICE, name: DEVICE.into(), title: "Play through".into(), options: devices, enabled: true, ..Default::default() },
            note("A new device is used from the next start of nori."),
        ];
        groups.push(("Output device", output));
        groups.push(("About", vec![info("nori", format!("Version {}", env!("CARGO_PKG_VERSION")))]));
    }
    groups
}

/// Rows of settings tab `tab`: each group's heading, then its rows with first and last marked.
pub fn rows(p: &StoredPrefs, f: &Facts, tab: i32, today: Today) -> ModelRc<SettingRow> {
    let mut out = Vec::new();
    for (title, rows) in groups(p, f, tab, today) {
        out.push(SettingRow { kind: HEADING, title: title.into(), ..Default::default() });
        // Rows on a plate share it; the plate's corners round where a row without one (a note, the tiles,
        // the appearance) or the group's end meets it.
        let plated: Vec<bool> = rows.iter().map(|r| !matches!(r.kind, NOTE | SEASONS | APPEARANCE_ROW)).collect();
        let n = rows.len();
        for (i, r) in rows.into_iter().enumerate() {
            let v = match Target::of(&r.name) {
                Target::Device => f.device.clone(),
                Target::Setting(name) => settings_model::value_of(p, name).unwrap_or_default(),
            };
            let chosen = if r.kind == APPEARANCE_ROW {
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
                first: i == 0 || !plated[i - 1],
                last: i + 1 == n || !plated[i + 1],
            });
        }
    }
    ModelRc::new(VecModel::from(out))
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
            let rows = rows(p, &facts, tab, today);
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
    fn tabs_are_the_pages_the_profile_opens() {
        use nori_core::settings::{SavedServer, ServerKind};
        let rules = |kind| nori_core::providers::rules_of(SavedServer { kind, ..Default::default() }, false).settings;
        let names = |kind| tabs(&rules(kind)).into_iter().map(tab_name).collect::<Vec<_>>();
        assert_eq!(names(ServerKind::Subsonic), ["General", "Playback", "Sound", "Lyrics", "Library", "Storage", "Accounts"]);
        assert!(!names(ServerKind::YouTubeMusic).contains(&"Library"), "no library settings to show");
    }

    #[test]
    fn an_appearance_sets_the_theme_and_amoled_and_reads_back() {
        for pick in 0..4 {
            let mut p = StoredPrefs::default();
            for (name, value) in appearance_settings(pick) {
                p = nori_core::settings::set_by_name(&p, name, value).expect("a setting").prefs;
            }
            assert_eq!(appearance_pick(&p), pick as i32, "pick {pick}");
        }
    }

    const OCTOBER: Today = Today { month: 10, day: 19, southern: false };

    #[test]
    fn the_look_page_has_season_tiles_then_the_appearance() {
        use slint::Model;
        let rows = rows(&StoredPrefs::default(), &Facts::default(), 0, OCTOBER);
        let kinds: Vec<_> = rows.iter().map(|r| r.kind).collect();
        let at = kinds.iter().position(|k| *k == SEASONS).expect("a season row");
        assert_eq!(kinds[at + 1..at + 3], [TOGGLE, APPEARANCE_ROW]);
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
            for (_, rows) in groups(&p, &f, tab, OCTOBER) {
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
    }
}
