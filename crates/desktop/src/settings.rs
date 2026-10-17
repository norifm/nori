//! Settings pages: the core's (`settings_pages`, desktop client), worded and drawn. Rows report back the
//! setting name and the picked value.

use nori_core::numbers::{self, POINT};
use nori_core::settings::{EqLevel, StoredPrefs};
use nori_core::settings_model::{self, BeatModel, SettingKind, SettingsState};
use nori_core::settings_pages::{self, PageFacts, RowItem, SectionId, SettingsAction, SettingsClient, SettingsLink, SettingsNote, SettingsPage, SettingsRow};
use slint::{Color, ModelRc, SharedString, VecModel};

use crate::{words, SettingRow};

// Row kinds, as app.slint numbers them.
const HEADING: i32 = 0;
const TOGGLE: i32 = 1;
const CHOICE: i32 = 2;
const ACTION: i32 = 3;
const INFO: i32 = 4;
const SLIDER: i32 = 5;
const RANKED: i32 = 6;
const TEXT: i32 = 7;
const SERVER: i32 = 8;
const NOTE: i32 = 9;
const PALETTE: i32 = 10;
const LINK: i32 = 11;
const BUTTON: i32 = 12;

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
    centred: bool,
    swatches: Vec<u32>,
}

fn options(name: &str) -> Vec<String> {
    settings_model::specs().into_iter().find(|s| s.name == name).map(|s| s.options).unwrap_or_default()
}

fn minus(v: &str) -> String {
    v.replace('-', "−")
}

/// The accent to draw: the core's default (Android purple) becomes red here; others as picked.
pub fn accent_shown(argb: u32) -> u32 {
    let default = StoredPrefs::default().accent as u32;
    if argb == default { 0xFFFA2D48 } else { argb }
}

/// The settings page a tab shows.
fn tab_page(tab: i32) -> SettingsPage {
    match tab {
        1 => SettingsPage::Playing,
        2 => SettingsPage::Sound,
        3 => SettingsPage::Lyrics,
        4 => SettingsPage::Library,
        5 => SettingsPage::Data,
        6 => SettingsPage::Servers,
        _ => SettingsPage::Look,
    }
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
        Theme | Cover => "Look",
        Home => "Home",
        Screen => "Screen",
        Motion => "Motion",
        Display => "Display",
        Sources => "Sources",
        AskedOrder => "In the order they're asked",
        Keys => "Keys",
        Lists => "Lists",
        Playlists => "Playlists",
        Search => "Search",
        History => "Listening history",
        Online => "Online",
        Devices => "Other devices",
        Streaming => "Streaming quality",
        Downloads => "Downloads",
        Storage => "Storage",
        Updates => "Updates",
    }
}

/// A setting's title and the line under it.
fn setting_words(name: &str, s: &SettingsState) -> (String, String) {
    let (title, detail) = match name {
        "crossfadeSec" => ("Crossfade", "Songs fade into each other, not within an album."),
        "autoMix" => ("AutoMix", "Blends songs like a DJ, matching the beat."),
        "autoMixBetterBeats" => {
            let mb = s.beat_model_mb;
            let detail = match &s.beat_model {
                BeatModel::Ready => "On. Each song is listened to once, just before it plays.".into(),
                BeatModel::Downloading => format!("Downloading (about {mb} MB)…"),
                BeatModel::Failed { .. } => "The download didn't work. It tries again at the next song.".into(),
                BeatModel::WaitingForWifi => format!("Waits for Wi-Fi to download (about {mb} MB)."),
                _ => format!("Listens to the start and end of each song so mixes land on the beat. A one-time download of about {mb} MB over Wi-Fi, from the model's authors."),
            };
            return ("Better beat detection".into(), detail);
        }
        "fadeMs" => ("Fade on play and pause", ""),
        "previousAlwaysSkips" => ("Previous goes back a song", "Instead of restarting the current one."),
        "speed" => ("Speed", ""),
        "pitch" => ("Pitch", ""),
        "skipSilence" => ("Skip silence", if s.untouched { "Off while the audio is played untouched." } else { "Cuts quiet gaps in and between songs." }),
        "skipExplicit" => ("Skip explicit songs", "Songs your server marks explicit."),
        "autoFill" => ("Keep playing when the queue ends", "Adds more music automatically."),
        "autoFillKind" => ("Carry on with", ""),
        "autoFillBasis" => ("Chosen by", ""),
        "autoFillRemote" => ("Include remote songs", "Songs not in your library yet, from the server's streaming providers. Each one played is downloaded into the library."),
        "bridgeOffline" => ("Play downloads when offline", "If the server drops, keep playing from downloads."),
        "autoEqAuto" => ("AutoEQ for headphones", "Applies a known correction curve when headphones connect."),
        "autoEqDownload" => ("Keep the AutoEQ list", "Downloads the headphone list (850 kB, from github.com), and again once a month, so headphones find their curve."),
        "profilePerOutput" => ("Remember sound per device", "Each device keeps its own equalizer settings."),
        "soundBypass" => ("No processing on this output", "No equalizer or effects reach this output. Saved with the device's sound."),
        "headphoneSound" => ("Headphone sound", "Natural sounds like speakers, Wide sounds bigger."),
        "compressor" => ("Compressor", "Evens out quiet and loud parts."),
        "compressorPreset" => ("For", ""),
        "replayGain" => ("Volume leveling", "Every song plays equally loud."),
        "loudnessLevel" => ("Level", "How loud leveled songs play."),
        "volumeBoostDb" => ("Volume boost", ""),
        "hiRes" => ("High quality output", "Plays 24-bit files in full, and runs the equalizer and effects in floating point. Starts with the next song."),
        "maxRate" => ("Highest sample rate", ""),
        "accent" => ("Accent color", ""),
        "coverColors" => ("Colors from the cover", "Where the artwork's colors show."),
        "lyricsSweep" => ("Fill in words as they're sung", "For lyrics timed word by word."),
        "lyricsTranslation" => ("Show translations", "When your server has them."),
        "lyricsOnline" => ("Find missing lyrics online", "When your server has no timed lyrics. Sends the artist, song and album name to the lyrics sources."),
        "lyricsPreferWords" => ("Prefer word-by-word lyrics", "Keeps looking past lyrics timed line by line."),
        "paxSenixKey" => ("PaxSenix key", "Your own key, for its Spotify and Musixmatch lyrics."),
        "betterLyricsKey" => ("BetterLyrics key", "Without one it answers only for songs it has already stored."),
        "tasteModel" => ("Keep listening history", "Stored on this Mac. Powers mixes and stats."),
        "scrobble" => ("Tell the server what you play", "Sends your plays to your server once you've heard half a song or four minutes of it."),
        "thirdPartyLookups" => ("Look things up online", "Missing lyrics (sends the artist, song and album name) and the AutoEQ headphone list, each with its own switch. Off, nothing is asked."),
        "remoteControl" => ("Remote control and jams", "Control playback on your other devices, or invite guests to a jam. Uses your network directly or the octo-fiesta relay."),
        "wifi" => ("Streaming quality", ""),
        "download" => ("Quality for downloads", ""),
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
        ("compressorPreset", "CAR") => "The car".into(),
        ("compressorPreset", "NIGHT") => "The night".into(),
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
                    Row { kind: CHOICE, name: spec.name, title, options, enabled, ..Default::default() }
                }
                SettingKind::Level => {
                    let v: f32 = value.parse().unwrap_or(spec.min);
                    let title = format!("{title}: {}", if v <= 0.0 { "Off".to_string() } else { format!("{} dB", crate::eq::signed(v)) });
                    Row { kind: SLIDER, name: spec.name, title, value: v.clamp(spec.min, spec.max), min: spec.min, max: spec.max, enabled: true, ..Default::default() }
                }
                SettingKind::Text => Row { kind: TEXT, name: spec.name, title, detail, button: value, enabled: true, ..Default::default() },
                SettingKind::Colour => {
                    let swatches = spec.options.iter().filter_map(|c| c.parse::<i64>().ok()).map(|c| c as u32).collect();
                    Row { kind: PALETTE, name: spec.name, title, swatches, value: p.accent as u32 as f32, enabled: true, ..Default::default() }
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
            let stored = format!(
                "{} streamed · {} covers · {} lyrics · {} in {} · {} library",
                bytes(f.stream_bytes),
                bytes(f.cover_bytes),
                bytes(f.lyrics_bytes),
                bytes(f.download_bytes),
                words::count(f.download_songs, "download", "downloads"),
                bytes(f.database_bytes)
            );
            info("Stored on this Mac", stored)
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
            SettingsNote::Boost => "The boost turns the limiter on with it, so loud songs do not distort.",
            SettingsNote::LyricsSources => "The quick ones (LyricsPlus, BetterLyrics and LRCLIB) are asked first, together, and the others only when those find nothing good. Every answer is scored on how well it matches the song, how finely it is timed and whether the other services agree, and the best is shown; the order here settles near ties. Your server's own timed lyrics always come before all of these.",
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
            Row { kind: SERVER, name: sv.id.clone(), title, detail, on: active, enabled: true, ..Default::default() }
        }
        RowItem::MusicFolder { index, on, tapped } => {
            let (name, _) = f.folders.get(index as usize)?;
            Row { kind: TOGGLE, name: format!("{MUSIC_FOLDERS}={tapped}"), title: name.clone(), on, enabled, ..Default::default() }
        }
        RowItem::LyricsSource { index } => {
            let src = s.lyrics_sources.get(index as usize)?;
            let (title, about) = service(&src.id)?;
            Row { kind: RANKED, name: src.id.clone(), title: title.into(), detail: about.into(), on: src.on, enabled: true, ..Default::default() }
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
fn groups(p: &StoredPrefs, f: &Facts, tab: i32) -> Vec<(&'static str, Vec<Row>)> {
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
        ..PageFacts::default()
    };
    let mut groups: Vec<(&'static str, Vec<Row>)> = Vec::new();
    for sec in settings_pages::page(tab_page(tab), SettingsClient::Desktop, p, &s, &facts) {
        let rows: Vec<Row> = sec.rows.into_iter().filter(|r| r.shown).flat_map(|r| flag_rows(&r, &s).unwrap_or_else(|| row_of(r, p, &s, f).into_iter().collect())).collect();
        let title = section_title(sec.id);
        // Sections that share a title are one group here.
        match groups.last_mut() {
            Some((t, g)) if *t == title => g.extend(rows),
            _ if rows.is_empty() => {}
            _ => groups.push((title, rows)),
        }
    }
    if tab == 0 {
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
pub fn rows(p: &StoredPrefs, f: &Facts, tab: i32) -> ModelRc<SettingRow> {
    let mut out = Vec::new();
    for (title, rows) in groups(p, f, tab) {
        out.push(SettingRow { kind: HEADING, title: title.into(), ..Default::default() });
        let n = rows.len();
        for (i, r) in rows.into_iter().enumerate() {
            let v = match Target::of(&r.name) {
                Target::Device => f.device.clone(),
                Target::Setting(name) => settings_model::value_of(p, name).unwrap_or_default(),
            };
            let chosen = r.options.iter().position(|o| o.1 == v).map_or(-1, |i| i as i32);
            let labels: Vec<SharedString> = r.options.iter().map(|o| o.0.as_str().into()).collect();
            let swatches: Vec<Color> = r.swatches.iter().map(|c| Color::from_argb_encoded(accent_shown(*c))).collect();
            let picked = r.swatches.iter().position(|c| *c == p.accent as u32).map_or(-1, |i| i as i32);
            out.push(SettingRow {
                kind: r.kind,
                name: r.name.into(),
                title: r.title.into(),
                detail: r.detail.into(),
                on: r.on,
                enabled: r.enabled,
                options: ModelRc::new(VecModel::from(labels)),
                chosen: if r.kind == PALETTE { picked } else { chosen },
                button: r.button.into(),
                value: r.value,
                min: r.min,
                max: r.max,
                centred: r.centred,
                swatches: ModelRc::new(VecModel::from(swatches)),
                first: i == 0,
                last: i + 1 == n,
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
            for (_, rows) in groups(&p, &f, tab) {
                for r in rows.iter().filter(|r| matches!(r.kind, TOGGLE | CHOICE | TEXT) && r.name != DEVICE) {
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
    }
}
