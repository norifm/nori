//! The settings record, its stored format (one value per key, plus the band list, sound profile and
//! server profile formats), defaults, ranges and edits. No database access (see settings_store.rs).

use std::collections::HashMap;

use nori_model::{EqBand, EqKind, EqPreset, NamedPreset, TransitionPrefs};
pub use nori_model::CompressorPreset;
use serde_json::{Map, Value};

use crate::codec::{clamped, on, within, Choice, Custom, Flags, Picks, Preamp, Quality, Raw, Row, FLAG, FLOAT, K, PICK, PICK_NEAREST, TEXT};
use crate::lyrics_sources::{self, LyricsService};
use crate::settings_store::{APPLY_AUDIO, APPLY_GAIN, CACHE_LIMIT, PLAYER, REPLAN, SOUND};

/// One stored value.
#[derive(Debug, Clone, PartialEq)]
pub enum PrefValue {
    Flag { v: bool },
    Number { v: i32 },
    Big { v: i64 },
    Decimal { v: f32 },
    Text { v: String },
}

/// One equalizer filter. Kind and channel are stored as ordinals, so their order must not change.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SoundBand {
    pub kind: EqKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    pub channel: BandChannel,
}

/// Which channel a band applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum BandChannel {
    Both,
    Left,
    Right,
}

/// The kind with ordinal `code`, as a platform passes it.
pub fn eq_kind(code: i32) -> Option<EqKind> {
    EqKind::nth(code)
}

/// A band from kind and channel ordinals; out-of-range ordinals become the first variant.
pub fn band_from(kind: i32, freq: f32, gain_db: f32, q: f32, channel: i32) -> SoundBand {
    SoundBand { kind: EqKind::nth(kind).unwrap_or(EqKind::Peaking), freq, gain_db, q, channel: BandChannel::nth(channel).unwrap_or(BandChannel::Both) }
}

pub use nori_model::GainMode;

// Player enums as settings, stored by ordinal in declaration order.
impl Choice for GainMode {
    const ALL: &'static [Self] = &[GainMode::Off, GainMode::Track, GainMode::Album, GainMode::Auto];
    const NAMES: &'static [&'static str] = &["OFF", "TRACK", "ALBUM", "AUTO"];
}

impl Choice for CompressorPreset {
    const ALL: &'static [Self] = &CompressorPreset::ALL;
    const NAMES: &'static [&'static str] = &["LIGHT", "CAR", "NIGHT"];
}

impl Choice for EqKind {
    const ALL: &'static [Self] = &[
        EqKind::Peaking,
        EqKind::LowShelf,
        EqKind::HighShelf,
        EqKind::LowPass,
        EqKind::HighPass,
        EqKind::BandPass,
        EqKind::Notch,
        EqKind::AllPass,
        EqKind::LowShelfSlope,
        EqKind::HighShelfSlope,
    ];
    const NAMES: &'static [&'static str] =
        &["PEAKING", "LOW_SHELF", "HIGH_SHELF", "LOW_PASS", "HIGH_PASS", "BAND_PASS", "NOTCH", "ALL_PASS", "LOW_SHELF_SLOPE", "HIGH_SHELF_SLOPE"];
}

/// Which equalizer plays: parametric (free filters) or graphic (fixed ISO bands). Each keeps its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum EqMode {
    Parametric,
    Graphic,
}

/// What the sound does for headphones: crossfeed (bs2b's default) or the virtualizer at its "wide"
/// strength. Part of a sound profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum HeadphoneSound {
    Off,
    Natural,
    Wide,
}

impl HeadphoneSound {
    /// The crossfeed level, dB; 0 is none.
    pub fn crossfeed_db(self) -> f32 {
        if self == HeadphoneSound::Natural { nori_player::dsp::CROSSFEED_NATURAL_DB as f32 } else { 0.0 }
    }

    /// The virtualizer's strength, 0 to 1.
    pub fn virtualizer(self) -> f32 {
        if self == HeadphoneSound::Wide { nori_player::spatial::WIDE as f32 } else { 0.0 }
    }
}

/// The loudness volume leveling aims for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Level {
    /// -23 LUFS, broadcast loudness.
    Quiet,
    /// -18 LUFS, ReplayGain's own.
    Normal,
    /// -14 LUFS, as streaming services play.
    Loud,
}

impl Level {
    pub fn lufs(self) -> f32 {
        match self {
            Level::Quiet => -23.0,
            Level::Normal => -18.0,
            Level::Loud => -14.0,
        }
    }

    /// The level nearest `lufs`; halfway, the louder one.
    fn nearest(lufs: f32) -> Level {
        [Level::Loud, Level::Normal, Level::Quiet].into_iter().min_by(|a, b| (a.lufs() - lufs).abs().total_cmp(&(b.lufs() - lufs).abs())).expect("levels")
    }
}

/// A place that wears covers' colors. With none, nothing takes them; with one, the other is black in
/// the dark theme, with only the cover's accents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum CoverPlace {
    /// Album, artist and playlist pages.
    Pages,
    Player,
}

/// Whether the player (or, not `player`, a page) is black ([`nori_look::sleeve::page_black`]) where
/// `colors` wear covers' colors: with AMOLED black where it does not wear them, and in the dark theme
/// where only the other does.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn cover_black(colors: Vec<CoverPlace>, player: bool, dark: bool, amoled: bool) -> bool {
    let wears = colors.contains(if player { &CoverPlace::Player } else { &CoverPlace::Pages });
    nori_look::sleeve::page_black(dark, amoled || !colors.is_empty() && !wears, wears)
}

/// How much the interface moves. nori's choice holds whatever Android's animation setting says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Motion {
    Full,
    /// Short fades instead of slides and zooms.
    Reduced,
    /// Nothing moves: the player redraws only when something changes, with no ticking seek bar or
    /// stepping lyrics.
    Eink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

/// Which season's theme the app wears ([`nori_look::theme::palette`]); `Follow` turns with the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ThemeSeason {
    Follow,
    Spring,
    Summer,
    Autumn,
    Winter,
}

/// The season a pick names, none for the calendar's.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn season_of(pick: ThemeSeason) -> Option<nori_look::theme::Season> {
    use nori_look::theme::Season;
    match pick {
        ThemeSeason::Follow => None,
        ThemeSeason::Spring => Some(Season::Spring),
        ThemeSeason::Summer => Some(Season::Summer),
        ThemeSeason::Autumn => Some(Season::Autumn),
        ThemeSeason::Winter => Some(Season::Winter),
    }
}

/// A condition that hides a system bar (status or gesture); the bar is hidden while any chosen one holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum HideBarWhen {
    Sideways,
    Upright,
    /// Burn-in protection is at work.
    Protected,
}

/// Which side a car screen puts the controls and tabs on; the cover goes on the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ControlsSide {
    Driver,
    Left,
    Right,
}

/// A condition that keeps the screen on while the app is in front; it is kept on while any chosen one
/// holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AwakeWhen {
    Sideways,
    Upright,
    Charging,
}

/// How long burn-in protection waits for a touch before it dims the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum DimIdle {
    Never,
    HalfMinute,
    Minute,
    FiveMinutes,
}

/// How far burn-in protection dims an untouched screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum DimLevel {
    Light,
    Medium,
    Strong,
}

/// What a tap on a song in a list does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum TapAction {
    PlayList,
    PlayOne,
    Queue,
    PlayNext,
}

/// What dragging a song row sideways does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SwipeAction {
    None,
    Queue,
    PlayNext,
    Favourite,
    Download,
}

/// What autofill adds when the last song starts: songs, or one whole album.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AutoFillKind {
    Songs,
    Albums,
}

/// The highest output rate; above it songs are resampled down (`nori_player::policy::capped_rate`).
/// Part of a sound profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum MaxRate {
    Auto,
    Khz48,
    Khz96,
    Khz192,
}

impl MaxRate {
    /// Hz; 0 for no cap.
    pub fn hz(self) -> u32 {
        match self {
            MaxRate::Auto => 0,
            MaxRate::Khz48 => 48_000,
            MaxRate::Khz96 => 96_000,
            MaxRate::Khz192 => 192_000,
        }
    }
}

/// What autofill picks songs by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AutoFillBasis {
    Similar,
    Artist,
    Genre,
    Era,
}

/// The home page's rows, stored by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum HomeRow {
    Pinned,
    Playlists,
    Recent,
    Newest,
    Frequent,
    TopSongs,
    Random,
    Starred,
}

impl HomeRow {
    /// Every row, in the order a new install shows them.
    pub fn every() -> &'static [HomeRow] {
        <HomeRow as Choice>::ALL
    }
}

/// One of the library's sections; the client names it. Stored by name, in the order the Library page lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[repr(u8)]
pub enum LibrarySection {
    Albums,
    Favourites,
    Artists,
    Songs,
    Playlists,
    Smart,
    History,
    Genres,
    Decades,
    Folders,
    Radio,
    Downloads,
}

impl LibrarySection {
    /// The sections the Library page lists, in the order a new install lists them: smart playlists are
    /// found under Playlists, and History and Downloads are the page's own buttons.
    pub const LISTED: [LibrarySection; 9] = {
        use LibrarySection::*;
        [Albums, Favourites, Artists, Songs, Playlists, Genres, Decades, Folders, Radio]
    };
}

/// What a saved profile plays from: the provider it opens with (nori-core's providers/).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ServerKind {
    /// A Subsonic server (Navidrome, octo-fiesta).
    #[default]
    Subsonic,
    /// YouTube Music: no server, best effort (docs/research/youtube-music.md).
    YouTubeMusic,
}

impl ServerKind {
    /// Every kind, in the order they are offered.
    pub const ALL: [ServerKind; 2] = [ServerKind::Subsonic, ServerKind::YouTubeMusic];

    /// The name a profile's kind is stored under.
    pub fn id(self) -> &'static str {
        match self {
            ServerKind::Subsonic => "subsonic",
            ServerKind::YouTubeMusic => "youtubeMusic",
        }
    }

    /// The kind stored as `id`.
    pub fn from_id(id: &str) -> Option<ServerKind> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// One saved server profile. Each has its own rows in the index database.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SavedServer {
    pub id: String,
    pub kind: ServerKind,
    pub name: String,
    pub url: String,
    pub alt_url: String,
    pub user: String,
    pub password: String,
    pub api_key: String,
    pub legacy_auth: bool,
    pub headers: HashMap<String, String>,
    pub allow_self_signed: bool,
    pub client_cert: String,
    pub client_cert_password: String,
    pub wifi_only: bool,
    /// The music folders the library shows; empty for all of them.
    pub music_folder_ids: Vec<String>,
    pub alt_max_bit_rate: i32,
}

/// One stream quality: `bit_rate` 0 and an empty `format` mean the original file.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SavedQuality {
    pub bit_rate: i32,
    pub format: String,
}

/// The stream qualities offered, the original file first.
const QUALITIES: &[&str] = &["0:", "320:mp3", "192:opus", "128:opus", "96:opus", "64:opus"];

/// Every setting. Each field's `#[setting(...)]` line declares its key, codec, default, name, client
/// spec and effect bits (see nori-settings-derive).
#[derive(Debug, Clone, PartialEq, nori_settings_derive::Settings)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StoredPrefs {
    // The servers.
    #[setting("servers", SERVERS, default = Vec::new(), hidden)]
    pub servers: Vec<SavedServer>,
    #[setting("activeServerId", TEXT, default = String::new(), hidden)]
    pub active_server_id: String,
    // Between songs.
    /// Seconds; 0 is off. Equal power.
    #[setting("crossfadeSec", clamped(0, 12), default = 0, show = K::Choice(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"]), effect = APPLY_AUDIO | REPLAN, sound)]
    pub crossfade_sec: i32,
    /// Beat-matched transitions with every part of them on ([`StoredPrefs::transition_prefs`]).
    #[setting("autoMix", FLAG, default = false, show = K::Switch, effect = APPLY_AUDIO | REPLAN)]
    pub auto_mix: bool,
    /// "Better beat detection": the Beat This! model on each upcoming song's ends, for AutoMix's beat
    /// grids. Needs the `neural-beats` feature and a one-time model download over Wi-Fi; downloaded
    /// songs are analyzed with it as they arrive.
    #[setting("autoMixBetterBeats", FLAG, default = false, show = K::Switch)]
    pub auto_mix_better_beats: bool,
    /// Songs of one album that follow each other in order play gapless, with AutoMix or a crossfade.
    #[setting("crossfadeKeepAlbums", FLAG, default = true, show = K::Switch, effect = REPLAN)]
    pub crossfade_keep_albums: bool,
    #[setting("fadeMs", within(0, 5000), default = 0, show = K::Choice(&["0", "150", "300", "500", "1000"]), effect = PLAYER)]
    pub fade_ms: i32,
    // Controls.
    #[setting("previousAlwaysSkips", FLAG, default = false, show = K::Switch, effect = PLAYER)]
    pub previous_always_skips: bool,
    #[setting("speed", within(RATE.0, RATE.1), default = 1.0, show = K::Choice(&["0.75", "1", "1.25", "1.5", "2"]), effect = APPLY_AUDIO)]
    pub speed: f32,
    #[setting("pitch", within(RATE.0, RATE.1), default = 1.0, show = K::Choice(&["0.9", "0.95", "1", "1.05", "1.1"]), effect = APPLY_AUDIO)]
    pub pitch: f32,
    #[setting("skipSilence", FLAG, default = false, show = K::Switch, effect = APPLY_AUDIO)]
    pub skip_silence: bool,
    // The queue.
    #[setting("skipExplicit", FLAG, default = false, show = K::Switch)]
    pub skip_explicit: bool,
    #[setting("autoFill", FLAG, default = true, show = K::Switch)]
    pub auto_fill: bool,
    #[setting("autoFillKind", PICK, default = AutoFillKind::Songs, show = K::Choice(AutoFillKind::NAMES))]
    pub auto_fill_kind: AutoFillKind,
    #[setting("autoFillBasis", PICK, default = AutoFillBasis::Similar, show = K::Choice(AutoFillBasis::NAMES))]
    pub auto_fill_basis: AutoFillBasis,
    /// Autofill may queue provider songs (octo-fiesta downloads each one played into the library).
    #[setting("autoFillRemote", FLAG, default = false, show = K::Switch)]
    pub auto_fill_remote: bool,
    #[setting("bridgeOffline", FLAG, default = false, show = K::Switch)]
    pub bridge_offline: bool,
    // Sound.
    #[setting("eqEnabled", FLAG, default = false, name = "eq", show = K::Switch, effect = APPLY_AUDIO | SOUND, sound)]
    pub eq_enabled: bool,
    #[setting("eqBands", BANDS, default = graphic(), hidden, effect = SOUND, sound)]
    pub eq_bands: Vec<SoundBand>,
    /// Graphic on a new install; older installs with a parametric setup keep it ([`load`]).
    #[setting("eqMode", PICK, default = EqMode::Graphic, show = K::Choice(EqMode::NAMES), effect = SOUND, sound)]
    pub eq_mode: EqMode,
    /// Graphic sliders, dB, low to high: 5, 10, 15 or 31 (`nori_player::graphic`).
    #[setting("eqGraphic", GRAPHIC, default = vec![0.0; 10], hidden, effect = SOUND, sound)]
    pub eq_graphic: Vec<f32>,
    /// The headphone correction the sliders were fitted to (dB on `graphic::target_grid`), refitted on a
    /// layout change; empty once a slider is moved by hand.
    #[setting("eqGraphicTarget", TARGET, default = Vec::new(), hidden, effect = SOUND, sound)]
    pub eq_graphic_target: Vec<f32>,
    #[setting("headphoneSound", PICK, default = HeadphoneSound::Off, show = K::Choice(HeadphoneSound::NAMES), effect = SOUND, effects)]
    pub headphone: HeadphoneSound,
    /// Extra gain, dB, with the limiter behind it; 0 is off.
    #[setting("volumeBoostDb", clamped(0.0, VOLUME_BOOST_MAX), default = 0.0, show = level, effect = SOUND, effects)]
    pub volume_boost_db: f32,
    #[setting("compressor", FLAG, default = false, show = K::Switch, effect = SOUND, effects)]
    pub compressor: bool,
    #[setting("compressorPreset", PICK, default = CompressorPreset::Light, show = K::Choice(CompressorPreset::NAMES), effect = SOUND, effects)]
    pub compressor_preset: CompressorPreset,
    #[setting("mono", FLAG, default = false, show = K::Switch, effect = APPLY_AUDIO | SOUND, sound)]
    pub mono: bool,
    #[setting("limiter", FLAG, default = false, show = K::Switch, effect = APPLY_AUDIO | SOUND, sound)]
    pub limiter: bool,
    #[setting("eqPreampDb", Preamp(EQ_RANGES.preamp), default = None, show = K::Level(EQ_RANGES.preamp.min, EQ_RANGES.preamp.max), effect = SOUND, sound)]
    pub eq_preamp_db: Option<f32>,
    #[setting("balance", FLOAT, default = 0.0, hidden, effect = SOUND, sound)]
    pub balance: f32,
    #[setting("limiterThresholdDb", within(EQ_RANGES.limiter.min, EQ_RANGES.limiter.max), default = -1.0, show = level, effect = SOUND, sound)]
    pub limiter_threshold_db: f32,
    #[setting("autoEqAuto", FLAG, default = false, show = K::Switch)]
    pub auto_eq_auto: bool,
    /// Keep the AutoEQ index on the device (`nori_devices::autoeq::index_due`).
    #[setting("autoEqDownload", FLAG, default = true, show = K::Switch, lookups)]
    pub auto_eq_download: bool,
    /// "No processing on this output": the sound chain is left out. Volume leveling and transitions
    /// still apply. Part of a sound profile.
    #[setting("soundBypass", FLAG, default = false, show = K::Switch, effect = APPLY_AUDIO | SOUND, sound = bypass)]
    pub sound_bypass: bool,
    #[setting("profilePerOutput", FLAG, default = true, show = K::Switch)]
    pub profile_per_output: bool,
    /// Volume leveling. Also decides whether songs may be turned up, hence the SOUND bit.
    #[setting("replayGain", PICK_NEAREST, default = GainMode::Auto, show = K::Choice(GainMode::NAMES), effect = APPLY_GAIN | REPLAN | SOUND, sound)]
    pub replay_gain: GainMode,
    /// The loudness songs are leveled to.
    #[setting("loudnessLevel", PICK, default = Level::Normal, show = K::Choice(Level::NAMES), effect = APPLY_GAIN)]
    pub level: Level,
    #[setting("hiRes", FLAG, default = false, show = K::Switch, effect = PLAYER, sound)]
    pub hi_res: bool,
    #[setting("maxRate", PICK, default = MaxRate::Auto, show = K::Choice(MaxRate::NAMES), effect = PLAYER, sound)]
    pub max_rate: MaxRate,
    #[setting("bitPerfect", FLAG, default = false, show = K::Switch, effect = APPLY_AUDIO, sound)]
    pub bit_perfect: bool,
    #[setting("offload", FLAG, default = true, show = K::Switch, effect = APPLY_AUDIO)]
    pub offload: bool,
    // Appearance.
    #[setting("season", PICK, default = ThemeSeason::Follow, show = K::Choice(ThemeSeason::NAMES))]
    pub season: ThemeSeason,
    #[setting("theme", PICK, default = ThemeMode::System, show = K::Choice(ThemeMode::NAMES))]
    pub theme: ThemeMode,
    #[setting("hideStatusBarWhen", Flags, default = vec![HideBarWhen::Sideways], name = "hideStatusBar", show = K::Flags(HideBarWhen::NAMES))]
    pub hide_status_bar: Vec<HideBarWhen>,
    #[setting("keepAwakeWhen", Flags, default = Vec::new(), name = "keepAwake", show = K::Flags(AwakeWhen::NAMES))]
    pub keep_awake: Vec<AwakeWhen>,
    /// While nori keeps the screen on, everything drawn drifts a few dp ([`nori_look::burn_in`]).
    #[setting("burnInShift", FLAG, default = false, name = "burnInProtection", show = K::Switch)]
    pub burn_in_protection: bool,
    #[setting("dimIdle", PICK, default = DimIdle::Never, name = "dimWhenIdle", show = K::Choice(DimIdle::NAMES))]
    pub dim_when_idle: DimIdle,
    #[setting("dimLevel", PICK, default = DimLevel::Medium, show = K::Choice(DimLevel::NAMES))]
    pub dim_level: DimLevel,
    #[setting("controlsSide", PICK, default = ControlsSide::Driver, show = K::Choice(ControlsSide::NAMES))]
    pub controls_side: ControlsSide,
    #[setting("amoled", FLAG, default = false, show = K::Switch)]
    pub amoled: bool,
    /// Where covers' colors are worn ([`CoverPlace`]).
    #[setting("coverColorsOn", Flags, default = vec![CoverPlace::Pages, CoverPlace::Player], name = "coverColors", show = K::Flags(CoverPlace::NAMES))]
    pub cover_colors: Vec<CoverPlace>,
    /// Apple Music motion artwork in the player; offered with the next as one choice, `movingCovers`.
    #[setting("motionArtwork", FLAG, default = false)]
    pub motion_artwork: bool,
    /// Motion artwork only on unmetered networks (each is a few MB).
    #[setting("motionArtworkWifiOnly", FLAG, default = true)]
    pub motion_artwork_wifi_only: bool,
    #[setting("motion", PICK, default = Motion::Full, show = K::Choice(Motion::NAMES))]
    pub motion: Motion,
    // Lyrics.
    #[setting("lyricsSweep", FLAG, default = true, show = K::Switch)]
    pub lyrics_sweep: bool,
    #[setting("lyricsSize", within(0, 2), default = 1, show = K::Choice(&["0", "1", "2"]))]
    pub lyrics_size: i32,
    #[setting("lyricsTranslation", FLAG, default = true, show = K::Switch)]
    pub lyrics_translation: bool,
    #[setting("lyricsKeepScreenOn", FLAG, default = true, show = K::Switch)]
    pub lyrics_keep_screen_on: bool,
    /// Look lyrics up online when the server has no timed ones. Stored under its old key "lyricsLrclib".
    #[setting("lyricsLrclib", FLAG, default = true, name = "lyricsOnline", show = K::Switch, lookups)]
    pub lyrics_online: bool,
    /// Every lyrics service, in rank order; stored by name (`lyrics_sources`).
    #[setting("lyricsOrder", LYRICS_ORDER, default = lyrics_sources::default_order())]
    pub lyrics_order: Vec<LyricsService>,
    /// The lyrics services switched on (changed by `lyricsService:<name>`).
    #[setting("lyricsOn", LYRICS_ON, default = lyrics_sources::default_on(), hidden)]
    pub lyrics_on: Vec<LyricsService>,
    /// Keep asking lower-ranked services for word timing after a line-timed answer.
    #[setting("lyricsPreferWords", FLAG, default = true, show = K::Switch)]
    pub lyrics_prefer_words: bool,
    /// PaxSenix key, for its Spotify and Musixmatch lyrics; empty for none. Given where those are switched
    /// on (`lyricsKey:<name>`).
    #[setting("paxSenixKey", TEXT, default = String::new(), hidden)]
    pub paxsenix_key: String,
    /// BetterLyrics key, for songs it has not cached; empty for none.
    #[setting("betterLyricsKey", TEXT, default = String::new(), show = K::Text)]
    pub better_lyrics_key: String,
    // The library.
    #[setting("tapAction", PICK, default = TapAction::PlayList, show = K::Choice(TapAction::NAMES))]
    pub tap_action: TapAction,
    #[setting("swipeRight", PICK, default = SwipeAction::Queue, show = K::Choice(SwipeAction::NAMES))]
    pub swipe_right: SwipeAction,
    #[setting("swipeLeft", PICK, default = SwipeAction::Favourite, show = K::Choice(SwipeAction::NAMES))]
    pub swipe_left: SwipeAction,
    /// Show playlists' notes ([`nori_library::pages::playlist_description`]).
    #[setting("playlistNotes", FLAG, default = true, show = K::Switch)]
    pub playlist_notes: bool,
    #[setting("tasteModel", FLAG, default = true, show = K::Switch)]
    pub taste_model: bool,
    #[setting("scrobble", FLAG, default = true, show = K::Switch)]
    pub scrobble: bool,
    /// Offers the queue another device or app left on the server when nothing plays here.
    #[setting("queueOffer", FLAG, default = true, show = K::Switch)]
    pub queue_offer: bool,
    /// Master switch over third-party lookups (lyrics, AutoEQ, motion artwork), each with its own switch.
    #[setting("thirdPartyLookups", FLAG, default = true, show = K::Switch)]
    pub third_party_lookups: bool,
    /// Check GitHub for a new release at most daily (nori-core update.rs). Not under the lookups switch.
    #[setting("updateCheck", FLAG, default = true, show = K::Switch)]
    pub update_check: bool,
    /// Beta releases (GitHub prereleases) are offered too, as well as the stable ones.
    #[setting("updateBeta", FLAG, default = false, show = K::Switch)]
    pub update_beta: bool,
    /// Remote playback control and hosting jams, through the relay or the local network.
    #[setting("remoteControl", FLAG, default = true, show = K::Switch)]
    pub remote_control: bool,
    /// The home page's rows, in order; unlisted rows are hidden.
    #[setting("homeRows", Picks, default = HomeRow::ALL.to_vec(), hidden)]
    pub home_rows: Vec<HomeRow>,
    #[setting("librarySections", Picks, default = LibrarySection::LISTED.to_vec(), hidden)]
    pub library_sections: Vec<LibrarySection>,
    #[setting("pinnedPlaylists", LINES, default = Vec::new(), hidden)]
    pub pinned_playlists: Vec<String>,
    #[setting("listPrefs", LIST_PREFS, default = HashMap::new(), hidden)]
    pub list_prefs: HashMap<String, String>,
    // Downloads and storage.
    #[setting("wifi", Quality, default = SavedQuality::default(), show = K::Choice(QUALITIES))]
    pub wifi: SavedQuality,
    #[setting("mobile", Quality, default = SavedQuality::default(), show = K::Choice(QUALITIES))]
    pub mobile: SavedQuality,
    #[setting("download", Quality, default = SavedQuality::default(), show = K::Choice(QUALITIES))]
    pub download: SavedQuality,
    /// Upcoming songs prepared ahead (audio and lyrics) on Wi-Fi and on mobile data; the next song
    /// always is, so the player can go on without a gap.
    #[setting("songsAheadWifi", within(1, 10), default = 3, show = K::Choice(&["1", "2", "3", "5", "10"]))]
    pub songs_ahead_wifi: i32,
    #[setting("songsAheadMobile", within(1, 10), default = 1, show = K::Choice(&["1", "2", "3", "5"]))]
    pub songs_ahead_mobile: i32,
    #[setting("cacheMb", within(256, 16384), default = 1024, show = K::Choice(&["256", "1024", "4096", "16384"]), effect = CACHE_LIMIT)]
    pub cache_mb: i32,
}

/// Offered settings that are not a field of their own; read and changed in [`value_of_special`] and
/// [`set_special`].
pub(crate) const SPECIAL_SPECS: &[(&str, K)] = &[
    ("movingCovers", K::Choice(&["off", "wifi", "always"])),
    ("altMaxBitRate", K::Choice(&["0", "320", "192", "128", "96"])),
    ("eqLayout", K::Choice(&["5", "10", "15", "31"])),
];

const SERVERS: Custom<Vec<SavedServer>> = Custom {
    // An unreadable list, or a server without an id, loses the whole list.
    load: |t, _| {
        t.and_then(|json| serde_json::from_str::<Value>(json).ok())
            .and_then(|v| v.as_array().map(|a| a.iter().map(server_from).collect::<Option<Vec<_>>>()))
            .flatten()
            .unwrap_or_default()
    },
    save: |s| Value::Array(s.iter().map(server_json).collect()).to_string(),
    set: None,
    show: None,
};

/// "kind:freq:gain:q:channel" per band, bands joined by ';'.
const BANDS: Custom<Vec<SoundBand>> = Custom { load: |t, d| t.and_then(decode_bands).unwrap_or(d), save: |b| encode_bands(b), set: None, show: None };

/// The graphic sliders, comma-separated; anything but a valid layout loads as the default.
const GRAPHIC: Custom<Vec<f32>> = Custom { load: |t, d| t.and_then(decode_graphic).unwrap_or(d), save: |g| encode_floats(g), set: Some(decode_graphic), show: Some(|g| encode_floats(g)) };

/// A headphone correction's target, comma-separated, one value per grid point, or empty.
const TARGET: Custom<Vec<f32>> = Custom { load: |t, d| t.map_or(d, decode_target), save: |g| encode_floats(g), set: None, show: None };

fn encode_floats(g: &[f32]) -> String {
    g.iter().map(|v| kotlin_float(*v)).collect::<Vec<_>>().join(",")
}

/// A target from its text; empty when invalid.
pub(crate) fn decode_target(s: &str) -> Vec<f32> {
    let g: Option<Vec<f32>> = s.split(',').map(float).collect();
    g.filter(|g| g.len() == nori_player::graphic::TARGET_POINTS && g.iter().all(|v| v.is_finite())).unwrap_or_default()
}

/// Graphic sliders from their text: one number per band of a valid layout, held to the gain range.
pub(crate) fn decode_graphic(s: &str) -> Option<Vec<f32>> {
    let g: Vec<f32> = s.split(',').map(|v| float(v).map(|v| EQ_RANGES.gain.hold(v))).collect::<Option<_>>()?;
    nori_player::graphic::LAYOUTS.contains(&g.len()).then_some(g)
}

/// Volume boost limit, dB.
const VOLUME_BOOST_MAX: f32 = nori_player::dsp::VOLUME_BOOST_MAX_DB as f32;

/// The stored ranking, completed with any service it does not name.
const LYRICS_ORDER: Custom<Vec<LyricsService>> = Custom {
    load: |t, _| lyrics_sources::complete_order(&lyrics_sources::stored_or(t.unwrap_or_default(), lyrics_sources::default_order())),
    save: |o| lyrics_sources::to_names(o),
    set: Some(|v| Some(lyrics_sources::complete_order(&lyrics_sources::parse(v)))),
    show: Some(|o| lyrics_sources::to_names(o)),
};

const LYRICS_ON: Custom<Vec<LyricsService>> = Custom { load: |t, d| match t {
        Some(t) => lyrics_sources::stored_or(t, d),
        None => d,
    }, save: |o| lyrics_sources::to_names(o), set: None, show: None };

/// One per line, empty lines dropped.
const LINES: Custom<Vec<String>> =
    Custom { load: |t, _| t.map_or_else(Vec::new, |s| s.split('\n').filter(|p| !p.is_empty()).map(str::to_string).collect()), save: |l| l.join("\n"), set: None, show: None };

/// A JSON object of strings; empty when unreadable.
const LIST_PREFS: Custom<HashMap<String, String>> = Custom {
    load: |t, _| t.and_then(|j| serde_json::from_str::<Value>(j).ok()).and_then(|v| string_map(&v)).unwrap_or_default(),
    save: |m| serde_json::to_string(m).unwrap_or_else(|_| "{}".to_string()),
    set: None,
    show: None,
};

/// The part of the settings a sound profile remembers.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SoundSettings {
    pub eq_enabled: bool,
    pub eq_bands: Vec<SoundBand>,
    pub eq_mode: EqMode,
    pub eq_graphic: Vec<f32>,
    pub eq_graphic_target: Vec<f32>,
    pub eq_preamp_db: Option<f32>,
    pub balance: f32,
    pub mono: bool,
    /// No processing on this output.
    pub bypass: bool,
    pub limiter: bool,
    pub limiter_threshold_db: f32,
    pub effects: SoundEffects,
    pub replay_gain: GainMode,
    pub crossfade_sec: i32,
    pub hi_res: bool,
    pub max_rate: MaxRate,
    pub bit_perfect: bool,
}

/// The effects besides the equalizer.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SoundEffects {
    pub headphone: HeadphoneSound,
    pub volume_boost_db: f32,
    pub compressor: bool,
    pub compressor_preset: CompressorPreset,
}

impl Default for SoundEffects {
    fn default() -> Self {
        StoredPrefs::default().effects()
    }
}

impl SoundEffects {
    /// Whether any effect alters the samples.
    pub fn on(&self) -> bool {
        self.player().on() || self.headphone.crossfeed_db() > 0.0
    }

    /// The chain's effects (crossfeed is the equalizer's, [`HeadphoneSound::crossfeed_db`]).
    pub fn player(&self) -> nori_player::dsp::Effects {
        nori_player::dsp::Effects {
            compressor: self.compressor.then(|| self.compressor_preset.settings()),
            virtualizer: self.headphone.virtualizer() as f64,
            boost_db: self.volume_boost_db as f64,
        }
    }
}

impl StoredPrefs {
    /// Whether the sound chain is needed (anything that alters samples is on and the output is not
    /// bypassed). Audio offload is off while it is.
    pub fn sound_chain_on(&self) -> bool {
        !self.sound_bypass && nori_player::policy::sound_on(self.eq_enabled, self.headphone.crossfeed_db(), self.balance, self.mono, self.limiter, self.effects().on())
    }

    /// How songs are leveled (`nori_player::gain`).
    pub fn gain_prefs(&self) -> nori_player::gain::GainPrefs {
        nori_player::gain::GainPrefs { mode: self.replay_gain, target_lufs: self.level.lufs() }
    }

    /// What the transition planner reads from the settings (`nori_automix::planner::Planner`): AutoMix
    /// with every part of it on, and mixes up to [`AUTO_MIX_MAX_S`].
    pub fn transition_prefs(&self) -> TransitionPrefs {
        TransitionPrefs {
            auto_mix: self.auto_mix,
            crossfade_s: self.crossfade_sec,
            auto_mix_max_s: AUTO_MIX_MAX_S,
            beat_match: true,
            max_tempo_change_pct: AUTO_MIX_MAX_TEMPO_PCT,
            bass_swap: true,
            filter_effects: true,
            echo_out: true,
            keep_pitch: true,
            keep_albums: self.crossfade_keep_albums,
            // AutoMix skips its loudness matching under volume leveling.
            replay_gain: self.replay_gain != GainMode::Off,
        }
    }

    /// The profile in use; None before a login.
    pub fn active_server(&self) -> Option<&SavedServer> {
        self.servers.iter().find(|s| s.id == self.active_server_id)
    }
}

/// The longest AutoMix transition, seconds.
const AUTO_MIX_MAX_S: i32 = 12;
/// The most AutoMix changes a song's tempo to match the beat, percent.
const AUTO_MIX_MAX_TEMPO_PCT: f32 = 6.0;

/// A failed sound edit. Messages are for the log.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[cfg_attr(feature = "ffi", uniffi(flat_error))]
pub enum SoundError {
    #[error("no filters in the preset")]
    NoFilters,
    #[error("database: {0}")]
    Db(String),
}

impl From<rusqlite::Error> for SoundError {
    fn from(e: rusqlite::Error) -> Self {
        SoundError::Db(e.to_string())
    }
}

impl From<nori_model::CoreError> for SoundError {
    fn from(e: nori_model::CoreError) -> Self {
        SoundError::Db(e.to_string())
    }
}

/// Speed and pitch range.
const RATE: (f32, f32) = (0.25, 4.0);

/// The ten default bands: flat peaking filters an octave apart.
pub fn graphic() -> Vec<SoundBand> {
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0]
        .into_iter()
        .map(|freq| SoundBand { kind: EqKind::Peaking, freq, gain_db: 0.0, q: 1.41, channel: BandChannel::Both })
        .collect()
}

/// Reads a band list ("kind:freq:gain:q[:channel]" joined by ';'). Unreadable bands are dropped; None
/// when none is left.
pub(crate) fn decode_bands(s: &str) -> Option<Vec<SoundBand>> {
    let bands: Vec<SoundBand> = s
        .split(';')
        .filter_map(|b| {
            let p: Vec<&str> = b.split(':').collect();
            if p.len() < 4 {
                return None;
            }
            let kind = EqKind::nth(p[0].parse().ok()?)?;
            let channel = match p.get(4) {
                Some(c) => BandChannel::nth(c.parse().ok()?)?,
                None => BandChannel::Both,
            };
            Some(SoundBand { kind, freq: float(p[1])?, gain_db: float(p[2])?, q: float(p[3])?, channel })
        })
        .collect();
    (!bands.is_empty()).then_some(bands)
}

pub fn encode_bands(bands: &[SoundBand]) -> String {
    let each: Vec<String> =
        bands.iter().map(|b| format!("{}:{}:{}:{}:{}", b.kind as i32, kotlin_float(b.freq), kotlin_float(b.gain_db), kotlin_float(b.q), b.channel as i32)).collect();
    each.join(";")
}

fn float(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// A float formatted as Kotlin's `toString` ("1000.0", "1.41", "1.0E-5"), the stored format.
pub(crate) fn kotlin_float(v: f32) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let a = v.abs();
    if a == 0.0 || (1e-3..1e7).contains(&a) {
        let s = v.to_string();
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        let s = format!("{v:e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        if m.contains('.') { format!("{m}E{e}") } else { format!("{m}.0E{e}") }
    }
}

fn opt_bool(o: &Map<String, Value>, k: &str) -> bool {
    o.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn opt_f64(o: &Map<String, Value>, k: &str, fallback: f64) -> f64 {
    o.get(k).and_then(Value::as_f64).unwrap_or(fallback)
}

fn opt_i32(o: &Map<String, Value>, k: &str) -> i32 {
    o.get(k).and_then(Value::as_i64).map_or(0, |i| i as i32)
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn opt_string(o: &Map<String, Value>, k: &str) -> String {
    o.get(k).map(text).unwrap_or_default()
}

fn string_map(v: &Value) -> Option<HashMap<String, String>> {
    Some(v.as_object()?.iter().map(|(k, v)| (k.clone(), text(v))).collect())
}

/// Reads a sound profile's JSON; None when it is not one. Missing fields take their defaults; a profile
/// saved before 0.6.3 is read in the new form ([`migrate_sound`]).
pub fn sound_from(json: &str) -> Option<SoundSettings> {
    let v: Value = serde_json::from_str(json).ok()?;
    let o = v.as_object()?;
    // A present pre-amp must be a number.
    let eq_preamp_db = match o.get("eqPreampDb") {
        Some(v) => Some(v.as_f64()? as f32),
        None => None,
    };
    let d = SoundEffects::default();
    let mut s = SoundSettings {
        eq_enabled: opt_bool(o, "eqEnabled"),
        eq_bands: decode_bands(&opt_string(o, "eqBands")).unwrap_or_else(graphic),
        eq_mode: EqMode::nth(opt_i32(o, "eqMode")).unwrap_or(EqMode::Parametric),
        eq_graphic: decode_graphic(&opt_string(o, "eqGraphic")).unwrap_or_else(|| vec![0.0; 10]),
        eq_graphic_target: decode_target(&opt_string(o, "eqGraphicTarget")),
        eq_preamp_db,
        balance: opt_f64(o, "balance", 0.0) as f32,
        bypass: opt_bool(o, "bypass"),
        mono: opt_bool(o, "mono"),
        limiter: opt_bool(o, "limiter"),
        limiter_threshold_db: opt_f64(o, "limiterThresholdDb", -1.0) as f32,
        effects: SoundEffects {
            headphone: HeadphoneSound::nth(opt_i32(o, "headphoneSound")).unwrap_or(d.headphone),
            volume_boost_db: opt_f64(o, "volumeBoostDb", d.volume_boost_db as f64) as f32,
            compressor: opt_bool(o, "compressor"),
            compressor_preset: CompressorPreset::nth(opt_i32(o, "compressorPreset")).unwrap_or(d.compressor_preset),
        }
        .held(),
        replay_gain: GainMode::ALL[opt_i32(o, "replayGain").clamp(0, GainMode::ALL.len() as i32 - 1) as usize],
        crossfade_sec: opt_i32(o, "crossfadeSec").clamp(0, 12),
        hi_res: opt_bool(o, "hiRes"),
        max_rate: MaxRate::nth(opt_i32(o, "maxRate")).unwrap_or(MaxRate::Auto),
        bit_perfect: opt_bool(o, "bitPerfect"),
    };
    if LEGACY_KEYS.iter().any(|k| o.contains_key(*k)) {
        migrate_sound(o, &mut s);
    }
    Some(s)
}

pub fn sound_json(s: &SoundSettings) -> String {
    let mut o = Map::new();
    o.insert("eqEnabled".into(), s.eq_enabled.into());
    o.insert("eqBands".into(), encode_bands(&s.eq_bands).into());
    if let Some(p) = s.eq_preamp_db {
        o.insert("eqPreampDb".into(), (p as f64).into());
    }
    o.insert("balance".into(), (s.balance as f64).into());
    if s.bypass {
        o.insert("bypass".into(), true.into());
    }
    o.insert("mono".into(), s.mono.into());
    o.insert("limiter".into(), s.limiter.into());
    o.insert("limiterThresholdDb".into(), (s.limiter_threshold_db as f64).into());
    o.insert("eqMode".into(), s.eq_mode.ordinal().into());
    o.insert("eqGraphic".into(), s.eq_graphic.iter().map(|v| kotlin_float(*v)).collect::<Vec<_>>().join(",").into());
    if !s.eq_graphic_target.is_empty() {
        o.insert("eqGraphicTarget".into(), encode_floats(&s.eq_graphic_target).into());
    }
    let e = &s.effects;
    o.insert("headphoneSound".into(), e.headphone.ordinal().into());
    o.insert("volumeBoostDb".into(), (e.volume_boost_db as f64).into());
    o.insert("compressor".into(), e.compressor.into());
    o.insert("compressorPreset".into(), e.compressor_preset.ordinal().into());
    o.insert("replayGain".into(), s.replay_gain.ordinal().into());
    o.insert("crossfadeSec".into(), s.crossfade_sec.into());
    o.insert("hiRes".into(), s.hi_res.into());
    o.insert("maxRate".into(), s.max_rate.ordinal().into());
    o.insert("bitPerfect".into(), s.bit_perfect.into());
    Value::Object(o).to_string()
}

/// Keys of the settings 0.6.3 removed or merged. Settings or a sound profile holding any is from before
/// it: read once in the new form ([`migrate`]), then stored without them.
pub(crate) const LEGACY_KEYS: &[&str] = &[
    "preampDb", "gainBoostDb", "untaggedGainDb", "gainMeasured", "loudnessTarget", "loudness", "loudnessRefPhon", "bassBoostDb", "virtualizer",
    "crossfeedDb", "crossfeedHz", "compThresholdDb", "compRatio", "compAttackMs", "compReleaseMs", "compMakeupDb", "compKneeDb", "expander",
    "expThresholdDb", "expRatio", "expAttackMs", "expReleaseMs", "crossfadeCurve", "crossfadeInSec", "crossfadeOutSec",
    "autoMixMaxS", "autoMixBeatMatch", "autoMixMaxTempoPct", "autoMixKeepPitch", "autoMixBassSwap", "autoMixFilters", "autoMixEchoOut",
    "autoMixBeatsMobileData", "downloadBeats", "coverColors", "playerColours", "albumColours", "artistColours", "softSleeve", "favouriteNotice",
    "uiScale", "reduceMotion", "einkScreen", "animateAnyway", "lyricsPrefetch", "skipOnError", "playlistDescriptions", "hideImportNotes",
    "liveSearchDelayMs", "scrobblePercent", "parallelDownloads", "precacheWifi", "precacheMobile", "coversAhead",
];

/// Whether stored settings are from before 0.6.3 ([`LEGACY_KEYS`]).
pub(crate) fn legacy(raw: &HashMap<String, PrefValue>) -> bool {
    LEGACY_KEYS.iter().any(|k| raw.contains_key(*k))
}

/// Whether a sound profile's JSON is from before 0.6.3.
pub(crate) fn legacy_profile(json: &str) -> bool {
    serde_json::from_str::<Value>(json).ok().and_then(|v| v.as_object().map(|o| LEGACY_KEYS.iter().any(|k| o.contains_key(*k)))).unwrap_or(false)
}

/// Old values, from stored settings or a sound profile's JSON; a value of another kind reads as missing.
trait Old {
    fn flag(&self, key: &str) -> Option<bool>;
    fn number(&self, key: &str) -> Option<f32>;
}

impl Old for HashMap<String, PrefValue> {
    fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            PrefValue::Flag { v } => Some(*v),
            _ => None,
        }
    }

    fn number(&self, key: &str) -> Option<f32> {
        match self.get(key)? {
            PrefValue::Number { v } => Some(*v as f32),
            PrefValue::Decimal { v } => Some(*v),
            _ => None,
        }
    }
}

impl Old for Map<String, Value> {
    fn flag(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    fn number(&self, key: &str) -> Option<f32> {
        self.get(key)?.as_f64().map(|v| v as f32)
    }
}

/// Settings stored before 0.6.3 in their new form ([`migrate_sound`] for the sound part): the loudness
/// target as the nearest level, the crossfade as the longer of its fades, the cover color switches as
/// the choice that colors the same places, reduced motion or e-ink as the motion, and playlist
/// descriptions as playlist notes.
fn migrate(raw: &HashMap<String, PrefValue>, p: &mut StoredPrefs) {
    if let Some(lufs) = raw.number("loudnessTarget") {
        p.level = Level::nearest(lufs);
    }
    if p.crossfade_sec > 0 {
        let side = |k: &str| raw.number(k).map_or(0, |s| s as i32).clamp(0, 12);
        let part = |s: i32| if s > 0 { s.min(p.crossfade_sec) } else { p.crossfade_sec };
        p.crossfade_sec = part(side("crossfadeInSec")).max(part(side("crossfadeOutSec")));
    }
    let on = |k: &str, d: bool| raw.flag(k).unwrap_or(d);
    // The page switches only counted with AMOLED black; without it every page kept its colors.
    let (player, pages) = (on("playerColours", true), on("albumColours", false) || on("artistColours", false));
    p.cover_colors = match (on("coverColors", true), p.amoled, player, pages) {
        (false, ..) => Vec::new(),
        (true, false, ..) | (true, true, true, true) => vec![CoverPlace::Pages, CoverPlace::Player],
        (true, true, true, false) => vec![CoverPlace::Player],
        (true, true, false, true) => vec![CoverPlace::Pages],
        (true, true, false, false) => Vec::new(),
    };
    p.motion = if on("einkScreen", false) {
        Motion::Eink
    } else if on("reduceMotion", false) {
        Motion::Reduced
    } else {
        Motion::Full
    };
    p.playlist_notes = on("playlistDescriptions", true);
    let mut sound = p.sound();
    migrate_sound(raw, &mut sound);
    *p = p.clone().with_sound(sound);
}

/// The sound part of settings or a profile stored before 0.6.3 in its new form: any leveling as
/// automatic, crossfeed as the natural headphone sound and the virtualizer as the wide one, the
/// compressor's controls as the nearest preset, and bass boost as the equalizer's bass shelf at the
/// same strength.
fn migrate_sound(old: &impl Old, s: &mut SoundSettings) {
    if s.replay_gain != GainMode::Off {
        s.replay_gain = GainMode::Auto;
    }
    s.effects.headphone = if old.number("virtualizer").is_some_and(|v| v > 0.0) {
        HeadphoneSound::Wide
    } else if old.number("crossfeedDb").is_some_and(|v| v > 0.0) {
        HeadphoneSound::Natural
    } else {
        HeadphoneSound::Off
    };
    // Unset controls had the defaults of the time.
    let knob = |k: &str, d: f64| old.number(k).map_or(d, |v| v as f64);
    let knobs = nori_player::compressor::CompressorSettings {
        threshold_db: knob("compThresholdDb", -20.0),
        ratio: knob("compRatio", 3.0),
        attack_ms: knob("compAttackMs", 10.0),
        release_ms: knob("compReleaseMs", 180.0),
        makeup_db: knob("compMakeupDb", 4.5),
        knee_db: knob("compKneeDb", 6.0),
    };
    s.effects.compressor_preset = CompressorPreset::nearest(knobs);
    if let Some(db) = old.number("bassBoostDb").filter(|db| *db >= 0.25) {
        *s = with_bass_boost(s.clone(), db.min(12.0));
    }
}

/// `s` with the bass boost preset's shelf (`nori_player::dsp::bass_shelf`) at `db` added to the
/// equalizer in use and the equalizer on; one that was off starts from flat.
fn with_bass_boost(s: SoundSettings, db: f32) -> SoundSettings {
    let shelf = nori_player::dsp::bass_shelf(db);
    // A pre-amp of the user's own makes room for the boost.
    let eq_preamp_db = if s.eq_enabled { s.eq_preamp_db.map(|p| EQ_RANGES.preamp.hold(p - db)) } else { None };
    match s.eq_mode {
        EqMode::Graphic => {
            let count = layout_of(&s);
            let r = EQ_RANGES.gain;
            let boost = nori_player::graphic::sliders_for(&[nori_player::dsp::Band::from(&shelf)], count, r.max as f64);
            let base = if s.eq_enabled { s.eq_graphic.clone() } else { vec![0.0; count] };
            let eq_graphic = base.iter().zip(boost).map(|(g, b)| r.hold(*g + b as f32)).collect();
            SoundSettings { eq_enabled: true, eq_graphic, eq_graphic_target: Vec::new(), eq_preamp_db, ..s }
        }
        EqMode::Parametric => {
            let mut eq_bands = if s.eq_enabled { s.eq_bands.clone() } else { Vec::new() };
            eq_bands.push(band_of(&shelf));
            SoundSettings { eq_enabled: true, eq_bands, eq_preamp_db, ..s }
        }
    }
}

/// One server from the stored list; None without an id.
fn server_from(v: &Value) -> Option<SavedServer> {
    let o = v.as_object()?;
    Some(SavedServer {
        id: text(o.get("id")?),
        // Absent in profiles saved before there were kinds.
        kind: o.get("kind").and_then(Value::as_str).and_then(ServerKind::from_id).unwrap_or_default(),
        name: opt_string(o, "name"),
        url: opt_string(o, "url"),
        alt_url: opt_string(o, "altUrl"),
        user: opt_string(o, "user"),
        password: opt_string(o, "password"),
        api_key: opt_string(o, "apiKey"),
        legacy_auth: opt_bool(o, "legacyAuth"),
        headers: o.get("headers").and_then(string_map).unwrap_or_default(),
        allow_self_signed: opt_bool(o, "allowSelfSigned"),
        client_cert: opt_string(o, "clientCert"),
        client_cert_password: opt_string(o, "clientCertPassword"),
        wifi_only: opt_bool(o, "wifiOnly"),
        music_folder_ids: folder_ids(o),
        alt_max_bit_rate: opt_i32(o, "altMaxBitRate"),
    })
}

/// `musicFolderIds`, or the one `musicFolderId` stored before a server could show several.
fn folder_ids(o: &Map<String, Value>) -> Vec<String> {
    match o.get("musicFolderIds").and_then(Value::as_array) {
        Some(ids) => ids.iter().map(text).collect(),
        None => Some(opt_string(o, "musicFolderId")).filter(|id| !id.is_empty()).into_iter().collect(),
    }
}

fn server_json(s: &SavedServer) -> Value {
    serde_json::json!({
        "id": s.id, "kind": s.kind.id(), "name": s.name, "url": s.url, "altUrl": s.alt_url, "user": s.user, "password": s.password,
        "apiKey": s.api_key, "legacyAuth": s.legacy_auth, "headers": s.headers, "allowSelfSigned": s.allow_self_signed,
        "clientCert": s.client_cert, "clientCertPassword": s.client_cert_password, "wifiOnly": s.wifi_only,
        "musicFolderIds": s.music_folder_ids, "altMaxBitRate": s.alt_max_bit_rate,
    })
}

/// The settings from what is stored: defaults for missing or mistyped values, ranges enforced.
pub fn load(raw: &HashMap<String, PrefValue>) -> StoredPrefs {
    let r = Raw(raw);
    let mut p = StoredPrefs::default();
    for row in ROWS {
        (row.load)(&mut p, &r);
    }
    if let Some(PrefValue::Flag { v: jam }) = raw.get("jam") {
        let remote = matches!(raw.get("remoteControl"), Some(PrefValue::Flag { v: true }));
        p.remote_control = remote || *jam;
    }
    if !raw.contains_key(EQ_MODE_KEY) && parametric_set_up(&p) {
        p.eq_mode = EqMode::Parametric;
    }
    if legacy(raw) {
        migrate(raw, &mut p);
    }
    migrate_picks(raw, &mut p);
    // Old builds saved the automatic screen defaults without recording a listener's choice.
    if !raw.contains_key("dimIdle") {
        if let Some(PrefValue::Number { v }) = raw.get("dimWhenIdle") {
            p.dim_when_idle = match v {
                1 => DimIdle::HalfMinute,
                3 => DimIdle::FiveMinutes,
                _ => DimIdle::Never,
            };
        }
    }
    p
}

pub(crate) fn old_screen_defaults(raw: &HashMap<String, PrefValue>) -> bool {
    raw.contains_key("burnInProtection") || raw.contains_key("dimWhenIdle")
}

/// Keys of the settings that were one pick of combined conditions before each became a set of them.
pub(crate) const PICK_KEYS: &[&str] = &["keepAwake", "hideStatusBar", "hideGestureBar", "coverColorsWhere"];

/// Whether stored settings hold any of [`PICK_KEYS`]: read once as sets ([`migrate_picks`]), then stored
/// without them.
pub(crate) fn picked(raw: &HashMap<String, PrefValue>) -> bool {
    PICK_KEYS.iter().any(|k| raw.contains_key(*k))
}

/// The picks of [`PICK_KEYS`] as the sets that hold where they held: "always" as both orientations, and
/// "sideways and charging" as each of them (either now keeps the screen on).
fn migrate_picks(raw: &HashMap<String, PrefValue>, p: &mut StoredPrefs) {
    let old = |k: &str| match raw.get(k) {
        Some(PrefValue::Number { v }) => Some(*v),
        _ => None,
    };
    use AwakeWhen as A;
    use HideBarWhen as H;
    if let Some(v) = old("keepAwake") {
        p.keep_awake = match v {
            1 => vec![A::Sideways],
            2 => vec![A::Charging],
            3 => vec![A::Sideways, A::Charging],
            4 => vec![A::Sideways, A::Upright],
            _ => Vec::new(),
        };
    }
    let bar = |v: i32, d: Vec<H>| match v {
        0 => Vec::new(),
        1 => vec![H::Sideways],
        2 => vec![H::Upright],
        3 => vec![H::Sideways, H::Upright],
        4 => vec![H::Protected],
        _ => d,
    };
    if let Some(v) = old("hideStatusBar") {
        p.hide_status_bar = bar(v, p.hide_status_bar.clone());
    }
    if let Some(v) = old("coverColorsWhere") {
        p.cover_colors = match v {
            1 => vec![CoverPlace::Pages],
            2 => vec![CoverPlace::Player],
            3 => Vec::new(),
            _ => vec![CoverPlace::Pages, CoverPlace::Player],
        };
    }
}

/// The equalizer mode's key; absent in settings stored before the graphic equalizer existed.
pub(crate) const EQ_MODE_KEY: &str = "eqMode";

/// Whether settings stored before the graphic equalizer have the parametric one set up (on, a pre-amp,
/// or bands other than the flat defaults); such installs keep it.
pub(crate) fn parametric_set_up(p: &StoredPrefs) -> bool {
    p.eq_enabled || p.eq_preamp_db.is_some() || p.eq_bands != graphic()
}

/// Whether a saved sound profile's JSON has a parametric equalizer set up.
pub(crate) fn profile_parametric(json: &str) -> bool {
    sound_from(json).is_some_and(|s| s.eq_enabled && (s.eq_bands != graphic() || s.eq_preamp_db.is_some()))
}

/// Everything to write for these settings.
pub fn save(p: &StoredPrefs) -> HashMap<String, PrefValue> {
    let mut put = HashMap::with_capacity(96);
    for row in ROWS {
        (row.save)(p, &mut put);
    }
    put
}

/// The result of a change by name. `server`: the active server's profile changed (the platform
/// reconnects). `effect`: `settings_store`'s effect bits, set once the change is kept.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingChange {
    pub prefs: StoredPrefs,
    pub server: bool,
    pub effect: u32,
}

pub(crate) fn row(name: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.name == Some(name))
}

/// Changes one setting by name (settings rows and the test bridge's `tools/app.sh set`). A switch reads
/// "true"/"1" as on; an enum its name or ordinal; an unreadable number keeps the value. None for an
/// unknown name or a refused value, so script typos fail loudly.
pub fn set_by_name(p: &StoredPrefs, name: &str, value: &str) -> Option<SettingChange> {
    let mut n = p.clone();
    let mut server = false;
    // The stored key, kept as an alias.
    let name = if name == "lyricsLrclib" { "lyricsOnline" } else { name };
    match set_special(p, &mut n, &mut server, name, value) {
        Some(done) => done?,
        None => {
            let row = row(name)?;
            (row.set)(&mut n, value)?;
            // Switching on an online lookup switches the master lookups switch on too.
            if row.lookups && on(value) {
                n.third_party_lookups = true;
            }
        }
    }
    Some(SettingChange { prefs: n, server, effect: 0 })
}

/// Changes by name that are not a single table row: None for a table name, else whether the value was
/// taken.
fn set_special(p: &StoredPrefs, n: &mut StoredPrefs, server: &mut bool, name: &str, value: &str) -> Option<Option<()>> {
    let service = |v: &str| {
        let (service, at) = v.split_once(':')?;
        Some((LyricsService::named(service)?, at.trim().to_string()))
    };
    Some(match name {
        // Test bridge: "raw" or "<format>:<kbps>" (e.g. "opus:128").
        "wifiQuality" => match value.split_once(':') {
            Some((format, kbps)) => kbps.parse().ok().map(|bit_rate| n.wifi = SavedQuality { bit_rate, format: format.to_string() }),
            None if value == "raw" => Some(n.wifi = SavedQuality::default()),
            None => None,
        },
        // Also switches online lyrics, both ways.
        "thirdPartyLookups" => Some((n.third_party_lookups, n.lyrics_online) = (on(value), on(value))),
        // Test bridge: back to the defaults.
        "lyricsSources" if value.trim().eq_ignore_ascii_case("default") => {
            n.lyrics_order = lyrics_sources::default_order();
            Some(n.lyrics_on = lyrics_sources::default_on())
        }
        // Test bridge: only these services, in this order (`lrclib,unison`).
        "lyricsSources" => {
            let on = lyrics_sources::parse(value);
            let rest = p.lyrics_order.iter().filter(|s| !on.contains(s)).copied();
            n.lyrics_order = on.iter().copied().chain(rest).collect();
            Some(n.lyrics_on = on)
        }
        // One service moved to a rank (drag and drop): `NETEASE:3`.
        "lyricsPlace" => service(value).and_then(|(s, to)| Some(n.lyrics_order = lyrics_sources::placed(p, s, to.parse().ok()?))),
        // One service moved by places: `NETEASE:-1`.
        "lyricsMove" => service(value).and_then(|(s, by)| Some(n.lyrics_order = lyrics_sources::moved(p, s, by.parse().ok()?))),
        // `lyricsService:NETEASE`: one service on or off. One that needs a key is not switched on without
        // it: the key is given with `lyricsKey:<name>`.
        _ if name.starts_with("lyricsService:") => LyricsService::named(&name["lyricsService:".len()..]).filter(|s| !on(value) || lyrics_sources::can_be_on(p, *s)).map(|s| {
            n.lyrics_on.retain(|x| *x != s);
            if on(value) {
                n.lyrics_on.push(s);
            }
        }),
        // `lyricsKey:PAXSENIX_SPOTIFY`: the key a service needs, kept and the service switched on; an
        // empty key switches off every service that needs it.
        _ if name.starts_with("lyricsKey:") => LyricsService::named(&name["lyricsKey:".len()..]).filter(|s| s.needs_key()).map(|s| {
            n.paxsenix_key = value.trim().to_string();
            if n.paxsenix_key.is_empty() {
                n.lyrics_on.retain(|x| !x.needs_key());
            } else if !n.lyrics_on.contains(&s) {
                n.lyrics_on.push(s);
            }
        }),
        // Graphic band count: the curve redrawn on the new layout, or refitted to the correction.
        "eqLayout" => value.trim().parse::<usize>().ok().filter(|c| nori_player::graphic::LAYOUTS.contains(c)).map(|c| match fit_target(&p.eq_graphic_target, c) {
            Some((sliders, preamp)) => (n.eq_graphic, n.eq_preamp_db) = (sliders, Some(preamp)),
            None => n.eq_graphic = relayout_graphic(&p.eq_graphic, c),
        }),
        // `motionArtwork` and `motionArtworkWifiOnly` as one choice; on, it needs the online lookups.
        "movingCovers" => matches!(value, "off" | "wifi" | "always").then(|| {
            n.motion_artwork = value != "off";
            n.motion_artwork_wifi_only = value == "wifi";
            n.third_party_lookups |= n.motion_artwork;
        }),
        // The active server profile's own settings.
        // The active server profile's own settings; `musicFolders` is a value of [`folder_switches`].
        "musicFolders" | "altMaxBitRate" => n.servers.iter_mut().find(|s| s.id == p.active_server_id).map(|s| {
            if name == "musicFolders" {
                s.music_folder_ids = value.split(',').filter(|id| !id.is_empty()).map(str::to_string).collect();
            } else {
                s.alt_max_bit_rate = value.trim().parse::<i32>().map_or(s.alt_max_bit_rate, |v| v.max(0));
            }
            *server = true;
        }),
        _ => return None,
    })
}

/// Values of the [`SPECIAL_SPECS`] settings.
pub(crate) fn value_of_special(p: &StoredPrefs, name: &str) -> Option<String> {
    let server = || p.active_server();
    Some(match name {
        "movingCovers" => match (p.motion_artwork && p.third_party_lookups, p.motion_artwork_wifi_only) {
            (false, _) => "off",
            (true, true) => "wifi",
            (true, false) => "always",
        }
        .to_string(),
        "eqLayout" => p.eq_graphic.len().to_string(),
        "altMaxBitRate" => server().map_or(0, |s| s.alt_max_bit_rate).to_string(),
        _ => return None,
    })
}

/// One music folder's switch: whether the library shows it, and the `musicFolders` value a tap sets.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FolderSwitch {
    pub on: bool,
    pub tapped: String,
}

/// The switches of a server's `folders` (ids, in its order) for a profile that shows `shown`. Every
/// folder on is stored as none picked, so folders added later show too; the last one on stays on.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn folder_switches(shown: Vec<String>, folders: Vec<String>) -> Vec<FolderSwitch> {
    let on = |id: &String| shown.is_empty() || shown.contains(id);
    let value = |ids: Vec<&String>| if ids.len() == folders.len() { String::new() } else { ids.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(",") };
    let now: Vec<&String> = folders.iter().filter(|id| on(id)).collect();
    folders
        .iter()
        .map(|id| {
            let after: Vec<&String> = folders.iter().filter(|x| on(x) != (*x == id)).collect();
            FolderSwitch { on: on(id), tapped: value(if after.is_empty() { now.clone() } else { after }) }
        })
        .collect()
}

/// A server's display name: its name, or else its address's host.
pub fn label(name: &str, url: &str) -> String {
    if !name.trim().is_empty() {
        return name.to_string();
    }
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or_default().to_string()
}

fn with_bands(s: SoundSettings, eq_bands: Vec<SoundBand>) -> SoundSettings {
    SoundSettings { eq_bands, ..s }
}

fn band_of(b: &nori_model::EqBand) -> SoundBand {
    SoundBand { kind: b.kind, freq: b.freq, gain_db: b.gain_db, q: b.q, channel: BandChannel::Both }
}

/// The graphic layout in use; 10 when the sliders are not a valid layout.
fn layout_of(s: &SoundSettings) -> usize {
    if nori_player::graphic::LAYOUTS.contains(&s.eq_graphic.len()) { s.eq_graphic.len() } else { 10 }
}

/// Graphic sliders for another layout that draw the same curve; unchanged for an invalid `count`.
pub(crate) fn relayout_graphic(sliders: &[f32], count: usize) -> Vec<f32> {
    if !nori_player::graphic::LAYOUTS.contains(&count) {
        return sliders.to_vec();
    }
    let g: Vec<f64> = sliders.iter().map(|v| *v as f64).collect();
    nori_player::graphic::relayout(&g, count).into_iter().map(|v| EQ_RANGES.gain.hold(((v * 10.0).round() / 10.0) as f32)).collect()
}

/// One graphic slider moved, held to the gain range; an index past the end changes nothing. Clears the
/// correction target.
pub fn set_graphic(s: SoundSettings, index: u32, gain_db: f32) -> SoundSettings {
    let mut g = s.eq_graphic.clone();
    match g.get_mut(index as usize) {
        Some(v) => *v = EQ_RANGES.gain.hold(gain_db),
        None => return s,
    }
    SoundSettings { eq_graphic: g, eq_graphic_target: Vec::new(), ..s }
}

/// Sliders of a `count`-band layout fitted to a correction `target`, and the pre-amp that prevents
/// boosting; None without a target.
fn fit_target(target: &[f32], count: usize) -> Option<(Vec<f32>, f32)> {
    let t: Vec<f64> = target.iter().map(|v| *v as f64).collect();
    let fit = nori_player::graphic::fit_target(&t, count, EQ_RANGES.gain.max as f64)?;
    Some((fit.sliders.iter().map(|v| *v as f32).collect(), EQ_RANGES.preamp.hold(fit.preamp_db as f32)))
}

/// A headphone correction (AutoEQ text: a `GraphicEQ:` curve or filters) as a graphic fitting target;
/// None when the text has neither.
pub(crate) fn correction_target(text: &str) -> Option<Vec<f32>> {
    let target = match nori_player::eqfit::parse_graphic(text) {
        Some(points) => nori_player::graphic::target_from_points(&points),
        None => {
            let preset = parse_eq_preset(text.to_string());
            if preset.bands.is_empty() {
                return None;
            }
            let bands: Vec<nori_player::dsp::Band> = preset.bands.iter().map(nori_player::dsp::Band::from).collect();
            nori_player::graphic::target_from_bands(&bands)
        }
    };
    Some(target.iter().map(|v| *v as f32).collect())
}

/// A built-in curve applied to the equalizer in use, which is switched on. A pre-amp of 0 means
/// automatic; "Flat" (no bands) restores the defaults. On the graphic equalizer the sliders take the
/// curve's response and the pre-amp is automatic.
pub fn apply_preset(s: SoundSettings, p: &NamedPreset) -> SoundSettings {
    if s.eq_mode == EqMode::Graphic {
        let bands: Vec<nori_player::dsp::Band> = p.bands.iter().map(nori_player::dsp::Band::from).collect();
        let count = layout_of(&s);
        let r = EQ_RANGES.gain;
        let eq_graphic = nori_player::graphic::sliders_for(&bands, count, r.max as f64).into_iter().map(|v| r.hold(v as f32)).collect();
        return SoundSettings { eq_enabled: true, eq_preamp_db: None, eq_graphic, eq_graphic_target: Vec::new(), ..s };
    }
    let bands: Vec<SoundBand> = p.bands.iter().map(band_of).collect();
    SoundSettings {
        eq_enabled: true,
        eq_preamp_db: (p.preamp_db != 0.0).then_some(p.preamp_db),
        eq_bands: if bands.is_empty() { graphic() } else { bands },
        ..s
    }
}

/// Imports an AutoEQ / Equalizer APO preset and switches the equalizer on. On the graphic equalizer the
/// sliders are fitted to it; otherwise its filters replace the parametric bands. No filters is
/// [`SoundError::NoFilters`].
pub fn import(s: SoundSettings, text: &str) -> Result<SoundSettings, SoundError> {
    if s.eq_mode == EqMode::Graphic {
        let target = correction_target(text).ok_or(SoundError::NoFilters)?;
        let count = layout_of(&s);
        let (eq_graphic, preamp) = fit_target(&target, count).ok_or(SoundError::NoFilters)?;
        return Ok(SoundSettings { eq_enabled: true, eq_graphic, eq_graphic_target: target, eq_preamp_db: Some(preamp), ..s });
    }
    let preset = parse_eq_preset(text.to_string());
    if preset.bands.is_empty() {
        return Err(SoundError::NoFilters);
    }
    Ok(SoundSettings { eq_enabled: true, eq_mode: EqMode::Parametric, eq_preamp_db: Some(preset.preamp_db), eq_bands: preset.bands.iter().map(band_of).collect(), ..s })
}

/// Appends a flat 1 kHz peak.
pub fn add_band(s: SoundSettings) -> SoundSettings {
    let mut bands = s.eq_bands.clone();
    bands.push(SoundBand { kind: EqKind::Peaking, freq: 1000.0, gain_db: 0.0, q: 1.0, channel: BandChannel::Both });
    with_bands(s, bands)
}

/// Removes a band; removing the last one restores the ten defaults.
pub fn remove_band(s: SoundSettings, index: u32) -> SoundSettings {
    let bands: Vec<SoundBand> = s.eq_bands.iter().enumerate().filter(|(i, _)| *i != index as usize).map(|(_, b)| *b).collect();
    with_bands(s, if bands.is_empty() { graphic() } else { bands })
}

/// A control's range; every edit through the core is held inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Span {
    pub min: f32,
    pub max: f32,
}

impl Span {
    pub(crate) fn hold(self, v: f32) -> f32 {
        if v.is_nan() { self.min.max(0.0).min(self.max) } else { v.clamp(self.min, self.max) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct EqRanges {
    /// A band's boost or cut, dB.
    pub gain: Span,
    /// The manual equalizer pre-amp, dB.
    pub preamp: Span,
    /// -1 hard left, +1 hard right.
    pub balance: Span,
    /// Limiter ceiling, dB.
    pub limiter: Span,
    /// A band's width (or a shelf's slope).
    pub q: Span,
    /// A band's frequency, Hz.
    pub freq: Span,
}

pub const EQ_RANGES: EqRanges = EqRanges {
    gain: Span { min: -12.0, max: 12.0 },
    preamp: Span { min: -20.0, max: 6.0 },
    balance: Span { min: -1.0, max: 1.0 },
    limiter: Span { min: -12.0, max: 0.0 },
    q: Span { min: 0.2, max: 8.0 },
    freq: Span { min: 20.0, max: 20_000.0 },
};

/// Editor facts about one `EqKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BandKindInfo {
    pub uses_gain: bool,
    /// A shelf given by its slope rather than a Q.
    pub slope: bool,
}

/// The equalizer editor's static facts: band kinds in `EqKind` order, and the ranges.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct EqModel {
    pub band_kinds: Vec<BandKindInfo>,
    pub eq_ranges: EqRanges,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn eq_model_get() -> EqModel {
    EqModel {
        band_kinds: EqKind::ALL
            .iter()
            .map(|k| BandKindInfo { uses_gain: k.uses_gain(), slope: matches!(k, EqKind::LowShelfSlope | EqKind::HighShelfSlope) })
            .collect(),
        eq_ranges: EQ_RANGES,
    }
}

/// A band with gain, width and frequency held inside [`EQ_RANGES`].
fn held(b: SoundBand) -> SoundBand {
    let r = EQ_RANGES;
    SoundBand {
        kind: b.kind,
        freq: r.freq.hold(b.freq),
        gain_db: r.gain.hold(b.gain_db),
        q: r.q.hold(b.q),
        channel: b.channel,
    }
}

/// One band changed, held in range; an index past the end changes nothing.
pub fn set_band(s: SoundSettings, index: u32, band: SoundBand) -> SoundSettings {
    let mut bands = s.eq_bands.clone();
    match bands.get_mut(index as usize) {
        Some(b) => *b = held(band),
        None => return s,
    }
    with_bands(s, bands)
}

impl SoundSettings {
    pub fn effective_preamp_db(&self) -> f32 {
        mode_preamp_db(self.eq_enabled, self.eq_preamp_db, self.eq_mode, &self.eq_graphic, &self.eq_bands)
    }
}

/// The pre-amp in effect for the equalizer in use: the manual one, or else automatic; 0 when it is off.
pub fn mode_preamp_db(eq_enabled: bool, eq_preamp_db: Option<f32>, mode: EqMode, graphic: &[f32], bands: &[SoundBand]) -> f32 {
    match mode {
        EqMode::Graphic => effective_preamp_db(eq_enabled, eq_preamp_db, graphic.iter().map(|g| (EqKind::Peaking, *g))),
        EqMode::Parametric => effective_preamp_db(eq_enabled, eq_preamp_db, bands.iter().map(|b| (b.kind, b.gain_db))),
    }
}

/// The pre-amp in effect from the equalizer's parts: on or off, the manual pre-amp (None for
/// automatic) and each band's kind and gain.
fn effective_preamp_db(eq_enabled: bool, eq_preamp_db: Option<f32>, bands: impl IntoIterator<Item = (EqKind, f32)>) -> f32 {
    if !eq_enabled {
        return 0.0;
    }
    eq_preamp_db.unwrap_or_else(|| nori_player::dsp::auto_preamp_db(bands))
}

/// The automatic pre-amp on or off; switched off, the manual pre-amp starts at the current level.
pub fn set_auto_preamp(s: SoundSettings, automatic: bool) -> SoundSettings {
    let eq_preamp_db = if automatic { None } else { Some(EQ_RANGES.preamp.hold(s.effective_preamp_db())) };
    SoundSettings { eq_preamp_db, ..s }
}

/// A level slider on the sound screens (`settings_store::edit_level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum EqLevel {
    Preamp,
    Balance,
    Limiter,
    VolumeBoost,
}

impl EqLevel {
    /// Every level in declaration order; JNI passes the index.
    pub const ALL: [EqLevel; 4] = [EqLevel::Preamp, EqLevel::Balance, EqLevel::Limiter, EqLevel::VolumeBoost];

    pub fn of(self, s: &SoundSettings) -> f32 {
        match self {
            EqLevel::Preamp => s.effective_preamp_db(),
            EqLevel::Balance => s.balance,
            EqLevel::Limiter => s.limiter_threshold_db,
            EqLevel::VolumeBoost => s.effects.volume_boost_db,
        }
    }
}

/// Balance within 0.04 of the centre snaps to 0.
pub(crate) fn balance_snap(v: f32) -> f32 {
    if v.abs() < 0.04 { 0.0 } else { v }
}

/// A level set, held in range and snapped ([`balance_snap`]; a volume boost under 0.25 dB is off).
pub fn set_level(s: SoundSettings, level: EqLevel, value: f32) -> SoundSettings {
    let r = EQ_RANGES;
    match level {
        EqLevel::Preamp => SoundSettings { eq_preamp_db: Some(r.preamp.hold(value)), ..s },
        EqLevel::Balance => SoundSettings { balance: balance_snap(r.balance.hold(value)), ..s },
        EqLevel::Limiter => SoundSettings { limiter_threshold_db: r.limiter.hold(value), ..s },
        EqLevel::VolumeBoost => {
            let v = Span { min: 0.0, max: VOLUME_BOOST_MAX }.hold(value);
            SoundSettings { effects: SoundEffects { volume_boost_db: if v < 0.25 { 0.0 } else { v }, ..s.effects }, ..s }
        }
    }
}

/// Why the sound chain is out of the path, so the equalizer screen can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[repr(u8)]
pub enum EqBypass {
    /// Bit-perfect USB output.
    BitPerfect,
    /// This output is set to no processing (`soundBypass`).
    Output,
}
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn eq_bypass(bit_perfect: bool, output: bool) -> Option<EqBypass> {
    if bit_perfect {
        Some(EqBypass::BitPerfect)
    } else if output {
        Some(EqBypass::Output)
    } else {
        None
    }
}

/// The mark a band's label shows after its frequency ("1k L", "63 ↙").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum BandMark {
    None,
    /// One channel only.
    Left,
    Right,
    LowShelf,
    HighShelf,
    /// A notch or pass filter.
    NoGain,
}

/// Channel first, then shelf, then no gain.
pub fn band_mark(kind: i32, channel: i32) -> BandMark {
    let low = kind == EqKind::LowShelf as i32 || kind == EqKind::LowShelfSlope as i32;
    let high = kind == EqKind::HighShelf as i32 || kind == EqKind::HighShelfSlope as i32;
    match channel {
        1 => BandMark::Left,
        2 => BandMark::Right,
        _ if low => BandMark::LowShelf,
        _ if high => BandMark::HighShelf,
        _ if !eq_kind(kind).is_some_and(EqKind::uses_gain) => BandMark::NoGain,
        _ => BandMark::None,
    }
}

/// The saved servers and which one is in use.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ServerList {
    pub servers: Vec<SavedServer>,
    pub active_server_id: String,
}

/// `profile` made active: it replaces the saved one with its id, or is appended.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn servers_activated(list: ServerList, profile: SavedServer) -> ServerList {
    let id = profile.id.clone();
    let mut servers: Vec<SavedServer> = list.servers.into_iter().filter(|s| s.id != id).collect();
    servers.push(profile);
    ServerList { servers, active_server_id: id }
}

/// A saved profile changed in place; an unknown one is not added.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn servers_updated(list: ServerList, profile: SavedServer) -> ServerList {
    let servers = list.servers.into_iter().map(|s| if s.id == profile.id { profile.clone() } else { s }).collect();
    ServerList { servers, ..list }
}

/// A profile removed. Removing the active one activates the one in use before it (the list is in the
/// order they were activated: the last one left), or none.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn servers_removed(list: ServerList, id: String) -> ServerList {
    let was_active = list.active_server_id == id;
    let servers: Vec<SavedServer> = list.servers.into_iter().filter(|s| s.id != id).collect();
    let active_server_id = if was_active { servers.last().map(|s| s.id.clone()).unwrap_or_default() } else { list.active_server_id };
    ServerList { servers, active_server_id }
}

/// The server id rows in the app database are kept under: the active profile's, or "default".
pub fn server_db_id(active_server_id: &str) -> String {
    if active_server_id.is_empty() { "default".into() } else { active_server_id.to_string() }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_db(active_server_id: String) -> String {
    server_db_id(&active_server_id)
}

/// A fresh profile id: eight hex digits.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn new_server_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    // A counter so two ids made in the same instant differ.
    static N: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, Ordering::Relaxed));
    h.write_u128(nori_time::since_epoch().as_nanos());
    format!("{:08x}", h.finish() as u32)
}

/// Extra HTTP headers as typed, one "Name: value" per line. Lines without a colon or name are skipped;
/// a repeated name keeps its last value.
pub(crate) fn parse_headers(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            (!k.trim().is_empty()).then(|| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Headers as the form shows them, sorted by name.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_headers_text(headers: HashMap<String, String>) -> String {
    let mut h: Vec<(&String, &String)> = headers.iter().collect();
    h.sort();
    h.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n")
}

/// The schemes to offer for an address typed without one, https first.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_url_schemes(url: String) -> Vec<String> {
    if url.is_empty() || url.contains("://") { Vec::new() } else { vec!["https://".into(), "http://".into()] }
}

/// The profile as the form sends it: addresses trimmed, headers parsed.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_from_form(profile: SavedServer, headers: String) -> SavedServer {
    SavedServer { url: profile.url.trim().to_string(), alt_url: profile.alt_url.trim().to_string(), headers: parse_headers(&headers), ..profile }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_label(name: String, url: String) -> String {
    label(&name, &url)
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn prefs_sound(prefs: StoredPrefs) -> SoundSettings {
    prefs.sound()
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn prefs_with_sound(prefs: StoredPrefs, sound: SoundSettings) -> StoredPrefs {
    prefs.with_sound(sound)
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sound_from_json(json: String) -> Option<SoundSettings> {
    sound_from(&json)
}

/// The equalizer in use reset to flat with the automatic pre-amp: the ten default bands (parametric)
/// or every slider at 0 in the same layout (graphic).
pub(crate) fn eq_reset_bands(sound: SoundSettings) -> SoundSettings {
    match sound.eq_mode {
        EqMode::Graphic => SoundSettings { eq_graphic: vec![0.0; sound.eq_graphic.len().max(1)], eq_graphic_target: Vec::new(), eq_preamp_db: None, ..sound },
        EqMode::Parametric => SoundSettings { eq_bands: graphic(), eq_preamp_db: None, ..sound },
    }
}

/// Indices into `names` of the app database's files (`nori.db` and its WAL and shared memory).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn storage_index_files(names: Vec<String>) -> Vec<u32> {
    names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.strip_prefix(nori_db::DB_FILE).is_some_and(|rest| ["", "-wal", "-shm"].contains(&rest)))
        .map(|(i, _)| i as u32)
        .collect()
}

/// Reads an AutoEQ "ParametricEQ.txt" / Equalizer APO preset (`Preamp: -6.2 dB`,
/// `Filter 1: ON PK Fc 105 Hz Gain -3.5 dB Q 0.70`); other lines are ignored. A file with no filters but
/// a `GraphicEQ:` curve is fitted with parametric filters (`nori_player::eqfit`).
pub fn parse_eq_preset(text: String) -> EqPreset {
    let mut preset = EqPreset::default();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let after = |key: &str| t.iter().position(|w| w.eq_ignore_ascii_case(key)).and_then(|i| t.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
        if t.first().is_some_and(|w| w.eq_ignore_ascii_case("preamp:")) {
            preset.preamp_db = t.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        } else if t.first().is_some_and(|w| w.eq_ignore_ascii_case("filter")) {
            let Some(on) = t.iter().position(|w| w.eq_ignore_ascii_case("ON")) else { continue };
            let token = t.get(on + 1).map(|k| k.to_ascii_uppercase()).unwrap_or_default();
            let kind = match token.as_str() {
                "PK" | "PEQ" | "MODAL" => EqKind::Peaking,
                "LS" | "LSC" | "LSQ" => EqKind::LowShelf,
                "HS" | "HSC" | "HSQ" => EqKind::HighShelf,
                "LS6" => EqKind::LowShelfSlope,
                "HS6" => EqKind::HighShelfSlope,
                "LP" | "LPQ" => EqKind::LowPass,
                "HP" | "HPQ" => EqKind::HighPass,
                "BP" => EqKind::BandPass,
                "NO" | "NOTCH" => EqKind::Notch,
                "AP" => EqKind::AllPass,
                _ => continue,
            };
            let Some(freq) = after("Fc") else { continue };
            let gain_db = after("Gain").unwrap_or(0.0);
            // Shelves and peaks need a gain; pass filters have none.
            if kind.uses_gain() && after("Gain").is_none() {
                continue;
            }
            preset.bands.push(EqBand { kind, freq, gain_db, q: after("Q").unwrap_or(0.71) });
        }
    }
    if preset.bands.is_empty() {
        if let Some(points) = nori_player::eqfit::parse_graphic(&text) {
            let fit = nori_player::eqfit::fit_graphic(&points);
            return EqPreset { preamp_db: fit.preamp_db, bands: fit.bands };
        }
    }
    preset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(entries: &[(&str, PrefValue)]) -> HashMap<String, PrefValue> {
        entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    fn t(v: &str) -> PrefValue {
        PrefValue::Text { v: v.to_string() }
    }
    fn n(v: i32) -> PrefValue {
        PrefValue::Number { v }
    }

    #[test]
    fn keys_and_names_unique() {
        let saved = save(&StoredPrefs { eq_preamp_db: Some(0.0), ..StoredPrefs::default() });
        let mut keys: Vec<&str> = ROWS.iter().map(|r| r.key).collect();
        let mut names: Vec<&str> = ROWS.iter().filter_map(|r| r.name).chain(SPECIAL_SPECS.iter().map(|(n, _)| *n)).collect();
        for k in &keys {
            assert!(saved.contains_key(*k) || saved.contains_key(&format!("{k}BitRate")), "{k} is not stored under its key");
        }
        keys.sort_unstable();
        names.sort_unstable();
        let (k, n) = (keys.len(), names.len());
        keys.dedup();
        names.dedup();
        assert_eq!((keys.len(), names.len()), (k, n), "each key and name once");
    }

    #[test]
    fn legacy_remote_switches_merge() {
        for (remote, jam) in [(false, false), (false, true), (true, false), (true, true)] {
            let prefs = load(&raw(&[("remoteControl", PrefValue::Flag { v: remote }), ("jam", PrefValue::Flag { v: jam })]));
            assert_eq!(prefs.remote_control, remote || jam, "remote={remote}, jam={jam}");
            let mut saved = save(&prefs);
            saved.insert("remoteControl".into(), PrefValue::Flag { v: false });
            assert!(!load(&saved).remote_control, "the merged switch can be turned off after migration");
        }
    }

    #[test]
    fn loading() {
        assert_eq!(load(&HashMap::new()), StoredPrefs::default());

        // Load clamps out of range.
        let p = load(&raw(&[("crossfadeSec", n(40)), ("replayGain", n(9)), ("theme", n(7)), ("tapAction", n(-2)), ("swipeLeft", n(5))]));
        assert_eq!(p.crossfade_sec, 12);
        assert_eq!(p.replay_gain, GainMode::Auto, "volume leveling takes the nearest");
        assert_eq!(p.theme, ThemeMode::System);
        assert_eq!(p.tap_action, TapAction::PlayList);
        assert_eq!(p.swipe_left, SwipeAction::Favourite);
        assert_eq!(load(&raw(&[("crossfadeSec", n(-1))])).crossfade_sec, 0);
        // A value of the wrong type is as good as missing.
        assert_eq!(load(&raw(&[("cacheMb", t("big"))])).cache_mb, 1024);

        // Server list loads whole or empty.
        let p = load(&raw(&[("servers", t(r#"[{"id":"a","legacyAuth":true,"altMaxBitRate":128}]"#)), ("activeServerId", t("a"))]));
        assert_eq!((p.servers.len(), p.active_server_id.as_str()), (1, "a"));
        assert!(p.servers[0].legacy_auth);
        assert_eq!(p.servers[0].alt_max_bit_rate, 128);
        // A list that does not read, or a server without an id, loses the whole list.
        assert!(load(&raw(&[("servers", t(r#"[{"id":"a"},{"name":"x"}]"#))])).servers.is_empty());
        assert!(load(&raw(&[("servers", t("nope"))])).servers.is_empty());

        // Load home rows pins list prefs.
        let p = load(&raw(&[("homeRows", t("RANDOM,NOPE,PINNED")), ("pinnedPlaylists", t("a\n\nb")), ("listPrefs", t(r#"{"x":"1","y":2}"#))]));
        assert_eq!(p.home_rows, [HomeRow::Random, HomeRow::Pinned]);
        assert_eq!(p.pinned_playlists, ["a", "b"]);
        assert_eq!(p.list_prefs.get("y").map(String::as_str), Some("2"));
        assert!(load(&raw(&[("homeRows", t(""))])).home_rows.is_empty(), "every row hidden");
        assert!(load(&raw(&[("listPrefs", t("{"))])).list_prefs.is_empty());
    }

    #[test]
    fn sound_json_shape() {
        let s = SoundSettings {
            eq_enabled: true,
            eq_bands: vec![band_from(1, 105.0, -3.5, 0.7, 0)],
            eq_mode: EqMode::Graphic,
            eq_graphic: (0..15).map(|i| i as f32 - 7.5).collect(),
            eq_graphic_target: (0..96).map(|i| (i as f32 * 0.37).sin() * 4.0).collect(),
            eq_preamp_db: Some(-6.2),
            bypass: true,
            balance: -0.25,
            mono: true,
            limiter: true,
            limiter_threshold_db: -2.0,
            effects: SoundEffects { headphone: HeadphoneSound::Wide, volume_boost_db: 3.0, compressor: true, compressor_preset: CompressorPreset::Night },
            replay_gain: GainMode::Album,
            crossfade_sec: 4,
            hi_res: true,
            max_rate: MaxRate::Khz96,
            bit_perfect: false,
        };
        assert_eq!(sound_from(&sound_json(&s)).unwrap(), s);
        assert_eq!(sound_from(&sound_json(&SoundSettings { eq_preamp_db: None, ..s.clone() })).unwrap().eq_preamp_db, None);

        // Sound json defaults.
        let s = sound_from("{}").unwrap();
        assert_eq!(s.eq_bands, graphic());
        assert_eq!(s.limiter_threshold_db, -1.0);
        assert_eq!(s.eq_preamp_db, None);
        assert_eq!(sound_from(r#"{"replayGain":7}"#).unwrap().replay_gain, GainMode::Auto);
        assert_eq!(sound_from(r#"{"replayGain":-1}"#).unwrap().replay_gain, GainMode::Off);
        assert_eq!(s.max_rate, MaxRate::Auto, "a profile saved before the maximum rate: the device's own");
        assert_eq!(sound_from(r#"{"maxRate":9}"#).unwrap().max_rate, MaxRate::Auto);
        assert_eq!(sound_from(r#"{"eqPreampDb":"x"}"#), None, "a pre-amp that is not a number");
        assert_eq!(sound_from(r#"{"eqPreampDb":null}"#), None);
        assert_eq!(sound_from("not json"), None);
        assert_eq!(sound_from("[]"), None);

        // Band wire format.
        let b = [band_from(0, 1000.0, -2.5, 1.41, 0), band_from(9, 31.0, 0.0, 0.00001, 2)];
        assert_eq!(encode_bands(&b), "0:1000.0:-2.5:1.41:0;9:31.0:0.0:1.0E-5:2");
        assert_eq!(decode_bands(&encode_bands(&b)).unwrap(), b);
        // Four fields is a band on both channels; anything that does not read is dropped.
        assert_eq!(decode_bands("1:100:3:0.7").unwrap(), [band_from(1, 100.0, 3.0, 0.7, 0)]);
        assert_eq!(decode_bands("1:100:3:0.7;12:1:1:1;x:1:1:1;1:1:1;2:1:1:1:3").unwrap().len(), 1);
        assert_eq!(decode_bands(""), None);
        assert_eq!(decode_bands("1:1:1"), None);
        assert_eq!(kotlin_float(12_345_678.0), "1.2345678E7");
        assert_eq!(kotlin_float(-0.5), "-0.5");
    }

    #[test]
    fn settings_by_name() {
        let p = StoredPrefs::default();
        assert!(set_by_name(&p, "limiter", "TRUE").unwrap().prefs.limiter);
        assert_eq!(set_by_name(&p, "eqPreampDb", "12").unwrap().prefs.eq_preamp_db, Some(6.0), "held to the equalizer's range");
        assert_eq!(set_by_name(&StoredPrefs { eq_preamp_db: Some(2.0), ..p.clone() }, "eqPreampDb", "auto").unwrap().prefs.eq_preamp_db, None);
        assert!(set_by_name(&p, "mono", "1").unwrap().prefs.mono);
        assert!(!set_by_name(&StoredPrefs { mono: true, ..p.clone() }, "mono", "yes").unwrap().prefs.mono);
        assert_eq!(set_by_name(&p, "crossfadeSec", "99").unwrap().prefs.crossfade_sec, 12);
        assert_eq!(set_by_name(&p, "crossfadeSec", "x").unwrap().prefs.crossfade_sec, 0, "unreadable keeps the value");
        for not_a_number in ["NaN", "inf", "-infinity"] {
            assert_eq!(set_by_name(&p, "volumeBoostDb", not_a_number).unwrap().prefs.volume_boost_db, p.volume_boost_db, "{not_a_number}");
        }
        assert_eq!(set_by_name(&p, "cacheMb", "10").unwrap().prefs.cache_mb, 256);
        assert_eq!(set_by_name(&p, "speed", "9").unwrap().prefs.speed, 4.0);
        assert_eq!(set_by_name(&p, "fadeMs", "-5").unwrap().prefs.fade_ms, 0);
        assert_eq!(set_by_name(&p, "autoFillKind", "albums").unwrap().prefs.auto_fill_kind, AutoFillKind::Albums);
        assert_eq!(set_by_name(&p, "autoFillBasis", "era").unwrap().prefs.auto_fill_basis, AutoFillBasis::Era);
        assert_eq!(set_by_name(&p, "autoFillBasis", "mood"), None);
        assert_eq!(set_by_name(&p, "nope", "1"), None);

        // Set by name parses each kind.
        let p = StoredPrefs::default();
        let set = |name: &str, v: &str| set_by_name(&p, name, v).unwrap().prefs;
        assert_eq!(set("replayGain", "ALBUM").replay_gain, GainMode::Album);
        assert_eq!(set("replayGain", "3").replay_gain, GainMode::Auto);
        assert_eq!(set_by_name(&p, "replayGain", "9"), None, "an ordinal out of range is not a value");
        assert_eq!(set("theme", "dark").theme, ThemeMode::Dark);
        assert_eq!(set("tapAction", "PLAY_NEXT").tap_action, TapAction::PlayNext);
        assert_eq!(set("swipeLeft", "DOWNLOAD").swipe_left, SwipeAction::Download);
        assert_eq!(set("wifi", "320:mp3").wifi, SavedQuality { bit_rate: 320, format: "mp3".into() });
        assert_eq!(set("mobile", "0:").mobile, SavedQuality::default());
        assert_eq!(set_by_name(&p, "download", "flac"), None);
        assert_eq!(set("season", "WINTER").season, ThemeSeason::Winter);
        assert_eq!(set("speed", "0.75").speed, 0.75);
        assert_eq!(set("lyricsSize", "7").lyrics_size, 2);
        assert_eq!(set("loudnessLevel", "loud").level, Level::Loud);
        assert_eq!(set("motion", "EINK").motion, Motion::Eink);
        // A set by its names, each once, in their order; unknown ones dropped, none for "".
        assert_eq!(set("coverColors", "player,pages,PLAYER,glow").cover_colors, vec![CoverPlace::Pages, CoverPlace::Player]);
        assert_eq!(set("keepAwake", "").keep_awake, vec![]);
        assert_eq!(crate::settings_model::value_of(&set("hideStatusBar", "protected,sideways"), "hideStatusBar").as_deref(), Some("SIDEWAYS,PROTECTED"));

        // Active server settings by name.
        let a = SavedServer { id: "a".into(), ..SavedServer::default() };
        let b = SavedServer { id: "b".into(), ..SavedServer::default() };
        let p = StoredPrefs { servers: vec![a, b], active_server_id: "b".into(), ..StoredPrefs::default() };
        let c = set_by_name(&p, "musicFolders", "7,9").unwrap();
        assert!(c.server);
        assert_eq!((c.prefs.servers[0].music_folder_ids.clone(), c.prefs.servers[1].music_folder_ids.clone()), (vec![], vec!["7".to_string(), "9".to_string()]));
        assert!(set_by_name(&c.prefs, "musicFolders", "").unwrap().prefs.servers[1].music_folder_ids.is_empty(), "all");
        assert_eq!(set_by_name(&p, "altMaxBitRate", "128").unwrap().prefs.servers[1].alt_max_bit_rate, 128);
        assert_eq!(set_by_name(&StoredPrefs::default(), "musicFolders", "7"), None, "no server in use");
        assert!(!set_by_name(&p, "mono", "1").unwrap().server);

        // Lyrics services by name.
        let p = StoredPrefs::default();
        let all = p.lyrics_on.len();
        let off = set_by_name(&p, "lyricsService:kugou", "false").unwrap().prefs;
        assert!(!off.lyrics_on.contains(&LyricsService::Kugou) && off.lyrics_on.len() == all - 1);
        let on = set_by_name(&off, "lyricsService:kugou", "true").unwrap().prefs;
        assert!(on.lyrics_on.contains(&LyricsService::Kugou) && on.lyrics_on.len() == all);
        assert_eq!(on.lyrics_order, p.lyrics_order, "a switch never moves a service");
        let off = set_by_name(&on, "lyricsService:paxsenix", "false").unwrap().prefs;
        assert_eq!((off.lyrics_order.clone(), off.lyrics_on.len()), (p.lyrics_order.clone(), all - 1));
        assert!(set_by_name(&p, "lyricsService:nobody", "true").is_none(), "no such service");
        let at = |o: &[LyricsService]| o.iter().position(|x| *x == LyricsService::Lrclib).unwrap();
        let moved = set_by_name(&on, "lyricsMove", "LRCLIB:-1").unwrap().prefs;
        assert_eq!(at(&moved.lyrics_order), at(&on.lyrics_order) - 1, "one place, whoever is above it");
        let placed = set_by_name(&on, "lyricsPlace", "LRCLIB:0").unwrap().prefs;
        assert_eq!(crate::lyrics_sources::switched_on(&placed)[0].name(), "LRCLIB", "dropped first, asked first");
        assert_eq!(placed.lyrics_on, on.lyrics_on);
        assert!(set_by_name(&on, "lyricsPlace", "LRCLIB:x").is_none());
        let only = set_by_name(&p, "lyricsSources", "lrclib, kugou").unwrap().prefs;
        assert_eq!(only.lyrics_on, [LyricsService::Lrclib, LyricsService::Kugou]);
        assert_eq!(only.lyrics_order[..2], [LyricsService::Lrclib, LyricsService::Kugou]);
        let back = set_by_name(&only, "lyricsSources", "default").unwrap().prefs;
        assert_eq!((back.lyrics_on, back.lyrics_order), (p.lyrics_on.clone(), p.lyrics_order.clone()));
        let keyed = set_by_name(&placed, "lyricsKey:PAXSENIX_SPOTIFY", "  k  ").unwrap().prefs;
        let back = load(&save(&keyed));
        assert_eq!((back.lyrics_on, back.lyrics_order, back.paxsenix_key), (keyed.lyrics_on.clone(), keyed.lyrics_order.clone(), "k".to_string()));
        assert_eq!(load(&HashMap::new()).lyrics_on, crate::lyrics_sources::default_on(), "nothing stored: the default services");
        assert_eq!(load(&HashMap::new()).lyrics_order, crate::lyrics_sources::default_order());

        // Effects by name.
        let p = StoredPrefs::default();
        let c = set_by_name(&set_by_name(&p, "compressor", "true").unwrap().prefs, "compressorPreset", "car").unwrap().prefs;
        assert!(c.compressor && c.effects().player().compressor == Some(CompressorPreset::Car.settings()) && c.sound_chain_on());
        assert!(set_by_name(&p, "compressorPreset", "loud").is_none());
        assert_eq!(set_by_name(&p, "volumeBoostDb", "40").unwrap().prefs.volume_boost_db, 12.0);
        let wide = set_by_name(&p, "headphoneSound", "WIDE").unwrap().prefs;
        assert!(wide.sound_chain_on() && wide.effects().player().virtualizer > 0.0);
        let s = wide.sound();
        assert_eq!(sound_from(&sound_json(&s)).unwrap().effects, s.effects, "a profile keeps it");
        let l = set_by_name(&p, "eqLayout", "31").unwrap().prefs;
        assert_eq!(l.eq_graphic.len(), 31);
        assert_eq!(value_of_special(&l, "eqLayout").as_deref(), Some("31"));
        assert!(set_by_name(&p, "eqLayout", "12").is_none());
        // Five bands are every other one of the ten: moved there, those sliders stay as they were.
        let ten = StoredPrefs { eq_graphic: (1..=10).map(|v| v as f32).collect(), ..p.clone() };
        let five = set_by_name(&ten, "eqLayout", "5").unwrap().prefs;
        assert_eq!(five.eq_graphic, [2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(decode_graphic("1,2,3,4,5"), Some(vec![1.0, 2.0, 3.0, 4.0, 5.0]), "five sliders is a layout");
    }

    #[test]
    fn lookups_switch() {
        // Stored off stays off.
        let kept = load(&save(&StoredPrefs { third_party_lookups: false, auto_eq_download: false, ..StoredPrefs::default() }));
        assert!(!kept.third_party_lookups && !kept.auto_eq_download);
        let p = StoredPrefs { third_party_lookups: false, auto_eq_download: false, ..StoredPrefs::default() };
        let on = set_by_name(&p, "autoEqDownload", "true").unwrap().prefs;
        assert!(on.auto_eq_download && on.third_party_lookups, "the AutoEQ list switches lookups on");
        assert!(!set_by_name(&on, "autoEqDownload", "false").unwrap().prefs.auto_eq_download);

        // Lyrics online and lookups switch.
        let p = StoredPrefs::default();
        let on = set_by_name(&p, "lyricsOnline", "true").unwrap().prefs;
        assert!(on.lyrics_online && on.third_party_lookups, "lyrics online switches lookups on");
        let off = set_by_name(&on, "lyricsLrclib", "false").unwrap().prefs;
        assert!(!off.lyrics_online && off.third_party_lookups, "and off leaves the lookups alone");
        let all_off = set_by_name(&on, "thirdPartyLookups", "false").unwrap().prefs;
        assert!(!all_off.lyrics_online && !all_off.third_party_lookups);
        let all_on = set_by_name(&all_off, "thirdPartyLookups", "true").unwrap().prefs;
        assert!(all_on.lyrics_online && all_on.third_party_lookups);
    }

    #[test]
    fn band_levels() {
        let s = sound();
        let b = set_band(s.clone(), 3, band_from(42, 5.0, 30.0, 0.0, 7));
        assert_eq!(b.eq_bands[3], band_from(0, 20.0, 12.0, 0.2, 0));
        let ok = band_from(1, 120.0, -3.5, 0.7, 2);
        assert_eq!(set_band(s.clone(), 0, ok).eq_bands[0], ok);
        assert_eq!(set_band(s.clone(), 99, ok), s, "no such band");

        // Set level snaps and holds.
        let s = sound();
        assert_eq!(set_level(s.clone(), EqLevel::Balance, 0.03).balance, 0.0);
        assert_eq!(set_level(s.clone(), EqLevel::Balance, -3.0).balance, -1.0);
        assert_eq!(set_level(s.clone(), EqLevel::Limiter, 2.0).limiter_threshold_db, 0.0);
        assert_eq!(set_level(s, EqLevel::Preamp, -30.0).eq_preamp_db, Some(-20.0));

        // Band marks.
        assert_eq!(band_mark(0, 0), BandMark::None);
        assert_eq!(band_mark(1, 1), BandMark::Left);
        assert_eq!(band_mark(8, 0), BandMark::LowShelf);
        assert_eq!(band_mark(2, 0), BandMark::HighShelf);
        assert_eq!(band_mark(9, 2), BandMark::Right);
        assert_eq!(band_mark(6, 0), BandMark::NoGain);

        // Effect levels hold and snap off.
        for (i, l) in EqLevel::ALL.iter().enumerate() {
            assert_eq!(*l as usize, i, "the door's ordinal is the declaration's");
        }
        let s = sound();
        assert_eq!(set_level(s.clone(), EqLevel::VolumeBoost, 30.0).effects.volume_boost_db, 12.0);
        assert_eq!(set_level(s.clone(), EqLevel::VolumeBoost, 0.1).effects.volume_boost_db, 0.0, "the bottom is off");
        assert_eq!(set_level(s, EqLevel::VolumeBoost, f32::NAN).effects.volume_boost_db, 0.0);
    }

    #[test]
    fn bypasses() {
        assert_eq!(eq_bypass(false, false), None);
        assert_eq!(eq_bypass(true, true), Some(EqBypass::BitPerfect));
        assert_eq!(eq_bypass(false, true), Some(EqBypass::Output));

        // Sound bypass.
        let busy = StoredPrefs { eq_enabled: true, headphone: HeadphoneSound::Natural, limiter: true, compressor: true, ..StoredPrefs::default() };
        assert!(busy.sound_chain_on());
        let bypassed = set_by_name(&busy, "soundBypass", "true").unwrap().prefs;
        assert!(!bypassed.sound_chain_on(), "nothing in the chain, so offload may play it");
        assert!(bypassed.eq_enabled && bypassed.compressor, "the settings are kept for when it is off again");
        let s = bypassed.sound();
        assert!(s.bypass);
        assert!(sound_from(&sound_json(&s)).unwrap().bypass, "a profile keeps it");
        assert!(!sound_from("{}").unwrap().bypass, "an old profile has it off");
        assert!(StoredPrefs::default().with_sound(s).sound_bypass);
        assert!(!sound_json(&busy.sound()).contains("bypass"), "left out when off, as before");

        // Auto preamp off keeps level.
        let mut s = sound();
        s.eq_enabled = true;
        s.eq_bands[2].gain_db = 4.5;
        assert_eq!(s.effective_preamp_db(), -4.5);
        let manual = set_auto_preamp(s.clone(), false);
        assert_eq!(manual.eq_preamp_db, Some(-4.5));
        assert_eq!(set_auto_preamp(manual, true).eq_preamp_db, None);
        assert_eq!(SoundSettings { eq_enabled: false, ..s.clone() }.effective_preamp_db(), 0.0);
        assert_eq!(set_auto_preamp(SoundSettings { eq_enabled: false, ..s }, false).eq_preamp_db, Some(0.0));
    }

    #[test]
    fn folder_switches_start_from_all() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let switches = |shown: &[&str]| folder_switches(ids(shown), ids(&["1", "2", "3"])).into_iter().map(|f| (f.on, f.tapped)).collect::<Vec<_>>();
        let sw = |on, tapped: &str| (on, tapped.to_string());
        // All shown: each tap hides that one.
        assert_eq!(switches(&[]), [sw(true, "2,3"), sw(true, "1,3"), sw(true, "1,2")]);
        // Two shown: the third's tap is all again, the others' leave one.
        assert_eq!(switches(&["1", "2"]), [sw(true, "2"), sw(true, "1"), sw(false, "")]);
        // The last one on stays on.
        assert_eq!(switches(&["2"]), [sw(false, "1,2"), sw(true, "2"), sw(false, "2,3")]);
        // A folder the server no longer has is dropped by the next tap.
        assert_eq!(switches(&["2", "9"])[0], sw(false, "1,2"));
    }

    #[test]
    fn servers() {
        let s = |id: &str, name: &str| SavedServer { id: id.into(), name: name.into(), ..SavedServer::default() };
        let list = ServerList { servers: vec![s("a", "A"), s("b", "B")], active_server_id: "b".into() };
        let l = servers_activated(list.clone(), s("a", "A2"));
        assert_eq!(l.servers.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["B", "A2"], "replaced, and moved to the end");
        assert_eq!(l.active_server_id, "a");
        let l = servers_updated(list.clone(), s("a", "A3"));
        assert_eq!((l.servers[0].name.as_str(), l.active_server_id.as_str()), ("A3", "b"));
        assert_eq!(servers_updated(list.clone(), s("z", "Z")).servers.len(), 2, "an unknown profile is not added");
        let l = servers_removed(list.clone(), "b".into());
        assert_eq!((l.servers.len(), l.active_server_id.as_str()), (1, "a"), "the one left takes over");
        assert_eq!(servers_removed(list.clone(), "a".into()).active_server_id, "b");
        // A jam guest's profile left: the profile in use before joining takes over, not the oldest.
        let joined = servers_activated(servers_activated(list.clone(), s("c", "C")), s("g", "Jam"));
        assert_eq!(servers_removed(joined, "g".into()).active_server_id, "c");
        assert_eq!(servers_removed(ServerList { servers: vec![s("a", "")], active_server_id: "a".into() }, "a".into()).active_server_id, "");
        assert_eq!((server_db_id(""), server_db_id("x1")), ("default".into(), "x1".into()));
        let id = new_server_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(new_server_id(), new_server_id());

        // Server labels.
        assert_eq!(label("Home", "https://x"), "Home");
        assert_eq!(label(" ", "https://music.example.com:4533/navidrome"), "music.example.com:4533");
        assert_eq!(label("", "music.example.com/x"), "music.example.com");
        assert_eq!(label("", ""), "");

        // A folder stored alone, before a server could show several.
        let folders = |json: Value| server_from(&json).unwrap().music_folder_ids;
        assert_eq!(folders(serde_json::json!({"id": "a", "musicFolderId": "3"})), ["3"]);
        assert_eq!(folders(serde_json::json!({"id": "a", "musicFolderId": ""})), Vec::<String>::new());
        assert_eq!(folders(serde_json::json!({"id": "a", "musicFolderIds": ["3", "5"], "musicFolderId": "3"})), ["3", "5"]);

        // A saved profile activated again is moved, not copied.
        let saved = SavedServer { id: "a1".into(), password: "old".into(), ..SavedServer::default() };
        let other = SavedServer { id: "b2".into(), ..SavedServer::default() };
        let list = ServerList { servers: vec![saved.clone(), other], active_server_id: "b2".into() };
        let l = servers_activated(list, SavedServer { password: "new".into(), ..saved });
        assert_eq!((l.servers.len(), l.active_server_id.as_str(), l.servers[1].password.as_str()), (2, "a1", "new"), "activated, not copied");

        // Login form.
        let h = parse_headers("X-Auth: a:b\n: nope\nno colon\n  CF-Id :  x  \nX-Auth: c");
        assert_eq!(h.len(), 2);
        assert_eq!(h["X-Auth"], "c", "the last one wins");
        assert_eq!(h["CF-Id"], "x");
        assert_eq!(server_headers_text(parse_headers("b: 2\na: 1")), "a: 1\nb: 2");
        assert_eq!(server_url_schemes("music.local".into()), ["https://", "http://"]);
        assert!(server_url_schemes("".into()).is_empty() && server_url_schemes("http://x".into()).is_empty());
        let p = SavedServer { url: " https://x ".into(), alt_url: " y ".into(), ..SavedServer::default() };
        let f = server_from_form(p, "A: 1".into());
        assert_eq!((f.url.as_str(), f.alt_url.as_str(), f.headers["A"].as_str()), ("https://x", "y", "1"));
    }

    /// Every kind is stored under its own name and read back; a profile saved before there were kinds,
    /// or with a kind this build does not know, is a server's.
    #[test]
    fn kinds_are_stored_by_name() {
        for kind in ServerKind::ALL {
            let p = SavedServer { id: "p".into(), kind, ..SavedServer::default() };
            assert_eq!(server_from(&server_json(&p)), Some(p));
        }
        assert_eq!(server_from(&serde_json::json!({"id": "a", "url": "https://x"})).unwrap().kind, ServerKind::Subsonic, "saved before kinds");
        assert_eq!(server_from(&serde_json::json!({"id": "a", "kind": "later"})).unwrap().kind, ServerKind::Subsonic);
    }

    fn sound() -> SoundSettings {
        sound_from("{}").unwrap()
    }

    #[test]
    fn eq_edits() {
        let flat = NamedPreset { kind: nori_model::PresetKind::Flat, preamp_db: 0.0, bands: vec![] };
        let s = apply_preset(SoundSettings { eq_preamp_db: Some(-4.0), ..sound() }, &flat);
        assert!(s.eq_enabled);
        assert_eq!(s.eq_preamp_db, None, "a pre-amp of 0 is automatic");
        assert_eq!(s.eq_bands, graphic());
        let bass = NamedPreset { kind: nori_model::PresetKind::BassBoost, preamp_db: -6.0, bands: vec![nori_model::EqBand { kind: EqKind::LowShelf, freq: 100.0, gain_db: 6.0, q: 0.7 }] };
        let s = apply_preset(sound(), &bass);
        assert_eq!(s.eq_preamp_db, Some(-6.0));
        assert_eq!(s.eq_bands, [band_from(1, 100.0, 6.0, 0.7, 0)]);

        let added = add_band(sound());
        assert_eq!(added.eq_bands.len(), 11);
        assert_eq!(added.eq_bands[10], band_from(0, 1000.0, 0.0, 1.0, 0));
        assert_eq!(remove_band(added.clone(), 10).eq_bands, graphic());
        assert_eq!(remove_band(added, 99).eq_bands.len(), 11);
        let one = SoundSettings { eq_bands: vec![band_from(2, 5.0, 1.0, 1.0, 0)], ..sound() };
        assert_eq!(remove_band(one, 0).eq_bands, graphic(), "never an empty equalizer");

        // Graphic edits.
        let g = SoundSettings { eq_mode: EqMode::Graphic, ..sound() };
        let moved = set_graphic(g.clone(), 3, 20.0);
        assert_eq!(moved.eq_graphic[3], 12.0, "held to the range");
        assert_eq!(moved.eq_bands, g.eq_bands, "the parametric bands are left alone");
        assert_eq!(set_graphic(g.clone(), 10, 3.0), g, "past the end: nothing");
        // Automatic pre-amp: the largest slider paid back.
        let loud = set_graphic(set_graphic(g.clone(), 0, 6.0), 5, -9.0);
        assert_eq!(SoundSettings { eq_enabled: true, ..loud.clone() }.effective_preamp_db(), -6.0);
        // A preset lands on the sliders; the bands are kept for when parametric comes back.
        let bass = NamedPreset { kind: nori_model::PresetKind::BassBoost, preamp_db: -6.0, bands: vec![nori_model::EqBand { kind: EqKind::LowShelf, freq: 100.0, gain_db: 6.0, q: 0.7 }] };
        let p = apply_preset(loud.clone(), &bass);
        assert!(p.eq_enabled && p.eq_preamp_db.is_none());
        assert!(p.eq_graphic[0] > 5.0 && p.eq_graphic[9].abs() < 0.1, "{:?}", p.eq_graphic);
        assert_eq!(p.eq_bands, loud.eq_bands);
        assert_eq!(eq_reset_bands(p.clone()).eq_graphic, vec![0.0; 10]);
        assert_eq!(eq_reset_bands(p.clone()).eq_bands, p.eq_bands, "reset is the one in use");
        // Filters imported on the parametric equalizer stay filters there; the graphic one fits them (below).
        let imported = import(SoundSettings { eq_mode: EqMode::Parametric, ..p }, "Filter 1: ON PK Fc 105 Hz Gain -3.5 dB Q 0.70\n").unwrap();
        assert_eq!(imported.eq_mode, EqMode::Parametric);
        // Another layout draws the same curve.
        assert_eq!(relayout_graphic(&[0.0, 2.0, 4.0, 6.0, 4.0, 2.0, 0.0, -2.0, -4.0, -6.0], 31).len(), 31);
        assert_eq!(decode_graphic("1,2,3"), None, "not a layout");
        assert_eq!(decode_graphic(&vec!["99"; 10].join(",")), Some(vec![12.0; 10]));

        // Eq mode migration.
        assert_eq!(load(&HashMap::new()).eq_mode, EqMode::Graphic, "a new install");
        // From before this version, with only the defaults: nothing was set up, so graphic.
        assert_eq!(load(&stored_before(StoredPrefs::default())).eq_mode, EqMode::Graphic);
        // From before, with a parametric equalizer in any form: it stays.
        let d = StoredPrefs::default();
        let mut moved = graphic();
        moved[3].gain_db = 2.5;
        let bass = vec![band_from(1, 100.0, 6.0, 0.7, 0)];
        for (what, p) in [
            ("a band moved", StoredPrefs { eq_bands: moved, ..d.clone() }),
            ("a preset or an AutoEQ curve", StoredPrefs { eq_bands: bass, eq_preamp_db: Some(-6.0), ..d.clone() }),
            ("a band added", StoredPrefs { eq_bands: add_band(d.sound()).eq_bands, ..d.clone() }),
            ("the equalizer on, flat", StoredPrefs { eq_enabled: true, ..d.clone() }),
            ("a pre-amp of its own", StoredPrefs { eq_preamp_db: Some(-3.0), ..d.clone() }),
        ] {
            assert_eq!(load(&stored_before(p)).eq_mode, EqMode::Parametric, "{what}");
        }
        // Once chosen, the choice is what is read, whatever else is stored.
        let chosen = StoredPrefs { eq_mode: EqMode::Graphic, eq_enabled: true, eq_bands: vec![band_from(1, 100.0, 6.0, 0.7, 0)], ..d.clone() };
        assert_eq!(load(&save(&chosen)).eq_mode, EqMode::Graphic);
        assert_eq!(load(&save(&StoredPrefs { eq_mode: EqMode::Parametric, ..d.clone() })).eq_mode, EqMode::Parametric);
        // A sound profile with a curve in it counts; a flat or switched-off one does not.
        assert!(profile_parametric(&sound_json(&SoundSettings { eq_enabled: true, eq_bands: vec![band_from(1, 100.0, 6.0, 0.7, 0)], ..sound() })));
        assert!(!profile_parametric(&sound_json(&SoundSettings { eq_enabled: false, eq_bands: vec![band_from(1, 100.0, 6.0, 0.7, 0)], ..sound() })));
        assert!(!profile_parametric(&sound_json(&SoundSettings { eq_enabled: true, ..sound() })));
        assert!(!profile_parametric("not json"));

        // Import parametric preset.
        let s = import(sound(), "Preamp: -6.2 dB\nFilter 1: ON PK Fc 105 Hz Gain -3.5 dB Q 0.70\n").unwrap();
        assert!(s.eq_enabled);
        assert_eq!(s.eq_preamp_db, Some(-6.2));
        assert_eq!(s.eq_bands.len(), 1);
        assert!(matches!(import(sound(), "Preamp: 0 dB\n"), Err(SoundError::NoFilters)));
        let zero = import(sound(), "Filter 1: ON PK Fc 105 Hz Gain -3.5 dB Q 0.70\n").unwrap();
        assert_eq!(zero.eq_preamp_db, Some(0.0), "an imported pre-amp is kept even at 0");

        // Correction fits graphic sliders.
        let graphic = include_str!("../../player/testdata/graphiceq/sennheiser-hd-600.txt");
        let parametric = include_str!("../../player/testdata/graphiceq/sennheiser-hd-600.parametric.txt");
        let g = SoundSettings { eq_mode: EqMode::Graphic, eq_enabled: false, ..sound() };
        let s = import(g.clone(), graphic).unwrap();
        assert!(s.eq_enabled && s.eq_mode == EqMode::Graphic, "it stays on the graphic equalizer");
        assert_eq!(s.eq_graphic.len(), 10);
        assert_eq!(s.eq_bands, g.eq_bands, "the parametric bands are left as they were");
        assert_eq!(s.eq_graphic_target.len(), nori_player::graphic::TARGET_POINTS);
        let preamp = s.eq_preamp_db.unwrap();
        assert!(preamp < -3.0, "the pre-amp pays back the boost: {preamp}");
        // The filters, where there is no curve, are a target too.
        assert!(import(g.clone(), parametric).unwrap().eq_graphic.iter().any(|v| *v != 0.0));
        assert!(matches!(import(g, "Preamp: 0 dB\n"), Err(SoundError::NoFilters)));
        // Another layout is fitted to the correction again, not stretched from the ten sliders.
        let p = StoredPrefs::default().with_sound(s.clone());
        let l = set_by_name(&p, "eqLayout", "31").unwrap().prefs;
        let t: Vec<f64> = s.eq_graphic_target.iter().map(|v| *v as f64).collect();
        let fitted = nori_player::graphic::fit_target(&t, 31, 12.0).unwrap();
        assert_eq!(l.eq_graphic, fitted.sliders.iter().map(|v| *v as f32).collect::<Vec<_>>());
        assert_eq!(l.eq_graphic_target, s.eq_graphic_target);
        // Moved by hand, a preset or a reset: the correction is gone.
        assert!(set_graphic(s.clone(), 0, 1.0).eq_graphic_target.is_empty());
        assert!(eq_reset_bands(s.clone()).eq_graphic_target.is_empty());
        // And it travels in a sound profile.
        assert_eq!(sound_from(&sound_json(&s)).unwrap(), s);
        // How closely it is followed, for the screen.
        let f = crate::dsp::graphic_follow(s.eq_graphic.clone(), s.eq_graphic_target.clone()).unwrap();
        assert!(f.rms_db > 0.0 && f.rms_db < 2.0 && f.max_db >= f.rms_db);
        assert!(crate::dsp::graphic_follow(s.eq_graphic, Vec::new()).is_none());
    }

    /// Settings as an install from before the graphic equalizer stored them: everything but the choice.
    fn stored_before(p: StoredPrefs) -> HashMap<String, PrefValue> {
        let mut raw = save(&p);
        raw.remove(EQ_MODE_KEY);
        raw
    }

    #[test]
    fn headphone_sound() {
        let p = StoredPrefs::default();
        assert!(!p.sound_chain_on());
        let natural = StoredPrefs { headphone: HeadphoneSound::Natural, ..p.clone() };
        assert!(natural.sound_chain_on() && natural.headphone.crossfeed_db() > 0.0 && natural.effects().player().virtualizer == 0.0, "crossfeed alone");
        let wide = StoredPrefs { headphone: HeadphoneSound::Wide, ..p };
        assert!(wide.sound_chain_on() && wide.headphone.crossfeed_db() == 0.0 && wide.effects().player().virtualizer > 0.0, "the virtualizer alone");
    }

    #[test]
    fn cover_colors_places() {
        use CoverPlace::*;
        // (player black, page black) in the dark theme, without and with AMOLED.
        let black = |c: &[CoverPlace], amoled| (cover_black(c.to_vec(), true, true, amoled), cover_black(c.to_vec(), false, true, amoled));
        let places: [&[CoverPlace]; 4] = [&[Pages, Player], &[Pages], &[Player], &[]];
        assert_eq!(places.map(|c| black(c, false)), [(false, false), (true, false), (false, true), (false, false)]);
        assert_eq!(places.map(|c| black(c, true)), [(false, false), (true, false), (false, true), (true, true)]);
        assert!(!cover_black(vec![Pages], true, false, false), "black is a dark theme's");
    }

    #[test]
    fn nearest_level() {
        assert_eq!([-30.0, -23.0, -20.0, -18.0, -16.0, -14.0, -9.0].map(Level::nearest), [Level::Quiet, Level::Quiet, Level::Normal, Level::Normal, Level::Loud, Level::Loud, Level::Loud], "halfway is the louder one");
    }

    #[test]
    fn storage_index_files_match_db() {
        let names = ["nori.db", "nori.db-wal", "nori.db-shm", "nori.db-journal", "certs", "other.db", "nori.db.bak"].map(String::from).to_vec();
        assert_eq!(storage_index_files(names), [0, 1, 2]);
    }

    #[test]
    fn moving_covers_is_one_choice() {
        let off = StoredPrefs { third_party_lookups: false, ..StoredPrefs::default() };
        let value = |p: &StoredPrefs| crate::settings_model::value_of(p, "movingCovers").unwrap();
        assert_eq!(value(&off), "off");
        for (choice, (on, wifi_only)) in [("wifi", (true, true)), ("always", (true, false)), ("off", (false, false))] {
            let n = set_by_name(&off, "movingCovers", choice).unwrap().prefs;
            assert_eq!((n.motion_artwork, n.motion_artwork_wifi_only, value(&n)), (on, wifi_only, choice.to_string()), "{choice}");
            assert_eq!(n.third_party_lookups, on, "{choice}: on needs the lookups, off leaves them");
        }
        let looked_up_off = StoredPrefs { motion_artwork: true, ..off.clone() };
        assert_eq!(value(&looked_up_off), "off", "off while lookups are off");
        assert!(set_by_name(&off, "movingCovers", "sometimes").is_none());
    }
}
