//! The settings page (one scrolling page of groups, sections and rows) and the equalizer's rows. The
//! pages, their rows and values are the core's (`settings_pages`, terminal client); this words and
//! draws them. Rows send back a setting name and value (`setting_set`) or a level edit (`edit_level`).

use crate::text;
use nori_core::settings::{EqLevel, EqMode, SoundBand, StoredPrefs, EQ_RANGES};
use nori_core::settings_model::{self, BeatModel, LyricsSource, SettingKind, SettingsState};
use nori_core::settings_pages::{self, PageFacts, RowItem, SectionId, SettingsAction, SettingsClient, SettingsLink, SettingsNote, SettingsPage, SettingsPart};
use nori_core::numbers::{self, POINT};
use nori_core::MusicFolder;

use crate::app::{Cmd, Overlay, Sel, SoundToolCmd, Target, View};

/// A group of the page: the terminal's own, one of the core's pages, or About.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gid {
    Own,
    Core(SettingsPage),
    About,
}

/// What opening a row does.
pub enum Opened {
    Cmds(Vec<Cmd>),
    Overlay(Overlay),
    View(View),
    /// A terminal-only switch (never `Switch::Setting`).
    Own(Switch),
    Login,
}

/// What a toggle row switches: a core setting by name, or a terminal-only switch.
#[derive(Debug, Clone, PartialEq)]
pub enum Switch {
    Setting(String),
    /// A setting and the value switching the row stores.
    Stores(String, String),
    Mouse,
    Images,
    CardCovers,
}

/// A settings group: a headed part of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: Gid,
    pub title: &'static str,
}

/// The groups in page order.
pub const GROUPS: [Group; 9] = [
    Group { id: Gid::Own, title: "Interface" },
    Group { id: Gid::Core(SettingsPage::Playing), title: "Playback" },
    Group { id: Gid::Core(SettingsPage::Sound), title: "Sound" },
    Group { id: Gid::Core(SettingsPage::Look), title: "Look" },
    Group { id: Gid::Core(SettingsPage::Library), title: "Library" },
    Group { id: Gid::Core(SettingsPage::Lyrics), title: "Lyrics" },
    Group { id: Gid::Core(SettingsPage::Servers), title: "Server" },
    Group { id: Gid::Core(SettingsPage::Data), title: "Storage" },
    Group { id: Gid::About, title: "About" },
];

/// What a link or button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Equalizer,
    Downloads,
    /// The login, for another server.
    AddServer,
    /// The invite link of someone's jam, to join it.
    JoinJam,
    Chore(Chore),
}

pub use nori_host::session::Chore;

/// One row of a page.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Toggle { switch: Switch, title: String, detail: String, on: bool, enabled: bool },
    /// `options` are (label, value); `shown` is the chosen label, or the raw value if none matches.
    Choice { target: Target, title: String, options: Vec<(String, String)>, shown: String, enabled: bool },
    Note { text: String },
    /// Opens another screen, with a status at the end.
    Link { title: String, status: String, action: Act },
    /// Text with a button at the end.
    Action { title: String, detail: String, button: String, enabled: bool, action: Act },
    Info { title: String, detail: String },
    /// With `level`, changed through `edit_level` instead of by `name`.
    Slider { name: String, label: String, value: f32, min: f32, max: f32, centred: bool, level: Option<EqLevel> },
    /// Colour swatches, ARGB.
    Palette { name: String, colours: Vec<i64>, chosen: i64 },
    Server { id: String, label: String, detail: String, active: bool },
    Button { title: String, action: Act },
    /// A lyrics service in the ranked list: enter switches it, ← → move it.
    /// `keyed`: it needs a key, asked for when it is switched on.
    Ranked { name: String, id: String, title: String, detail: String, on: bool, keyed: bool },
    /// Free text (a service's API key).
    Text { name: String, title: String, detail: String, value: String, secret: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub title: String,
    pub sections: Vec<Section>,
}

/// Local storage use in bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Storage {
    pub stream: i64,
    pub covers: i64,
    pub lyrics: i64,
    pub downloads: i64,
    pub download_songs: u32,
    pub database: i64,
}

/// Non-setting facts shown on the page, loaded when Settings opens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// Songs AutoMix has measured.
    pub analysed: u32,
    /// Offline index counts: songs, albums, artists.
    pub indexed: (u32, u32, u32),
    pub storage: Storage,
    /// The active server's music folders.
    pub folders: Vec<MusicFolder>,
    /// Output device names.
    pub devices: Vec<String>,
    /// What the open profile offers: a jam guest's settings are this device's own.
    pub rules: nori_core::browse::ProfileRules,
}

/// A page line: group heading, section title or row.
pub enum Line<'a> {
    Group(&'a str),
    Title(&'a str),
    Row(&'a Row),
}

#[derive(Default)]
pub struct SettingsView {
    pub row: Sel,
    /// Cached group pages; rebuilt after [`SettingsView::invalidate`].
    pages: Option<Vec<Page>>,
    pub facts: Facts,
    pub facts_asked: bool,
    /// State shown by the terminal's own rows.
    pub own: Own,
}

#[derive(Default, Clone)]
pub struct Own {
    pub mouse: bool,
    pub images: bool,
    pub card_covers: bool,
    pub protocol: String,
    pub data: String,
    /// Output device for the next start; empty for the system default.
    pub device: String,
}

impl SettingsView {
    pub fn invalidate(&mut self) {
        self.pages = None;
    }

    pub fn set_facts(&mut self, f: Facts) {
        self.facts = f;
        self.invalidate();
    }

    /// Every group's page, built if not cached.
    pub fn pages(&mut self, prefs: &StoredPrefs) -> &[Page] {
        if self.pages.is_none() {
            let state = settings_model::state(prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
            self.pages = Some(GROUPS.iter().map(|g| page(g.id, prefs, &state, &self.facts, &self.own)).collect());
        }
        self.pages.as_deref().expect("made above")
    }

    /// All lines as drawn.
    pub fn lines(pages: &[Page]) -> Vec<Line<'_>> {
        let mut out = Vec::new();
        for p in pages {
            out.push(Line::Group(&p.title));
            for s in &p.sections {
                if !s.title.is_empty() {
                    out.push(Line::Title(&s.title));
                }
                out.extend(s.rows.iter().map(Line::Row));
            }
        }
        out
    }

    /// The selection and line count, for list navigation.
    pub fn list(&mut self) -> (&mut Sel, usize) {
        let n = self.pages.as_deref().map_or(0, |p| Self::lines(p).len());
        (&mut self.row, n)
    }

    /// Index in `GROUPS` of the selected line's group.
    pub fn group_at(&self) -> usize {
        let Some(pages) = self.pages.as_deref() else { return 0 };
        Self::lines(pages).iter().take(self.row.at + 1).filter(|l| matches!(l, Line::Group(_))).count().saturating_sub(1)
    }

    /// Selects a group's first row.
    pub fn jump(&mut self, group: usize) {
        let Some(pages) = self.pages.as_deref() else { return };
        let lines = Self::lines(pages);
        if let Some(at) = lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Group(_))).nth(group).map(|(i, _)| i) {
            self.row.at = at;
            self.row.top = at;
            self.skip_titles(true);
        }
    }

    /// The selected row, if a row (not a heading) is selected.
    fn selected(&self) -> Option<&Row> {
        match Self::lines(self.pages.as_deref()?).into_iter().nth(self.row.at)? {
            Line::Row(r) => Some(r),
            _ => None,
        }
    }

    /// Whether a single click activates the row rather than only selecting it.
    pub fn clicks_open(&self) -> bool {
        matches!(self.selected(), Some(Row::Toggle { .. } | Row::Button { .. } | Row::Link { .. } | Row::Action { .. } | Row::Server { .. } | Row::Ranked { .. }))
    }

    /// Enter on the selection.
    pub fn open(&mut self) -> Option<Opened> {
        let row = self.selected()?.clone();
        Some(match row {
            Row::Toggle { switch: Switch::Setting(name), on, enabled, .. } => {
                if !enabled {
                    return None;
                }
                Opened::Cmds(vec![Cmd::Setting(name, (!on).to_string())])
            }
            Row::Toggle { switch: Switch::Stores(name, value), .. } => Opened::Cmds(vec![Cmd::Setting(name, value)]),
            Row::Toggle { switch, .. } => Opened::Own(switch),
            Row::Choice { target, title, options, shown, enabled: true } => {
                let at = options.iter().position(|o| o.0 == shown).unwrap_or(0);
                Opened::Overlay(Overlay::Picker { title, options, sel: Sel { at, top: 0 }, target })
            }
            Row::Link { action, .. } | Row::Action { action, enabled: true, .. } | Row::Button { action, .. } => match action {
                Act::Equalizer => Opened::View(View::Equalizer),
                Act::Downloads => Opened::View(View::Downloads),
                Act::AddServer => Opened::Login,
                Act::JoinJam => Opened::Overlay(Overlay::Join { text: String::new(), error: None, busy: false }),
                Act::Chore(c) => Opened::Cmds(vec![Cmd::Action(c)]),
            },
            Row::Server { id, active, .. } => {
                if active {
                    return None;
                }
                Opened::Cmds(vec![Cmd::SwitchServer(id)])
            }
            // Switched on, one that needs a key asks for it (empty clears it and switches it off).
            Row::Ranked { id, keyed: true, on: false, .. } => {
                Opened::Overlay(Overlay::Input { title: format!("{} key (empty: none)", text::LYRICS_KEY), text: String::new(), secret: true, name: format!("lyricsKey:{id}") })
            }
            Row::Ranked { name, on, .. } => Opened::Cmds(vec![Cmd::Setting(name, (!on).to_string())]),
            Row::Text { name, title, value, secret, .. } => Opened::Overlay(Overlay::Input { title, text: value, secret, name }),
            Row::Palette { name, colours, chosen } => {
                let at = colours.iter().position(|c| *c == chosen).map_or(0, |i| (i + 1) % colours.len());
                Opened::Cmds(vec![Cmd::Setting(name, colours[at].to_string())])
            }
            _ => return None,
        })
    }

    /// ← or → on the selection: previous or next option, off or on, a slider step.
    pub fn step(&mut self, up: bool) -> Vec<Cmd> {
        let Some(row) = self.selected().cloned() else { return Vec::new() };
        match row {
            Row::Choice { target, options, shown, enabled: true, .. } => {
                let at = options.iter().position(|o| o.0 == shown);
                let to = match at {
                    Some(i) if up => (i + 1).min(options.len() - 1),
                    Some(i) => i.saturating_sub(1),
                    None => 0,
                };
                if Some(to) == at || options.is_empty() {
                    return Vec::new();
                }
                target.cmd(options[to].1.clone()).into_iter().collect()
            }
            Row::Toggle { switch, on, enabled, .. } => {
                if on == up {
                    return Vec::new();
                }
                match switch {
                    Switch::Setting(name) if enabled => vec![Cmd::Setting(name, up.to_string())],
                    Switch::Stores(name, value) if enabled => vec![Cmd::Setting(name, value)],
                    Switch::Setting(_) | Switch::Stores(..) => Vec::new(),
                    Switch::Mouse => vec![Cmd::Mouse(up)],
                    Switch::Images => vec![Cmd::Images(up)],
                    Switch::CardCovers => vec![Cmd::CardCovers(up)],
                }
            }
            Row::Slider { name, value, min, max, level, .. } => {
                let step = slider_step(min, max);
                let v = (value + if up { step } else { -step }).clamp(min, max);
                match level {
                    Some(l) => vec![Cmd::Level(l, v)],
                    None => vec![Cmd::Setting(name, v.to_string())],
                }
            }
            Row::Palette { name, colours, chosen } => {
                let at = colours.iter().position(|c| *c == chosen).unwrap_or(0) as isize + if up { 1 } else { -1 };
                let at = at.rem_euclid(colours.len() as isize) as usize;
                vec![Cmd::Setting(name, colours[at].to_string())]
            }
            Row::Ranked { id, .. } => vec![Cmd::Setting("lyricsMove".into(), format!("{id}:{}", if up { 1 } else { -1 }))],
            _ => Vec::new(),
        }
    }

    /// Moves the selection off a heading onto a row, preferring direction `down`.
    pub fn skip_titles(&mut self, down: bool) {
        let Some(pages) = &self.pages else { return };
        let lines = Self::lines(pages);
        let title = |i: usize| matches!(lines.get(i), Some(Line::Title(_) | Line::Group(_)));
        let mut at = self.row.at;
        while title(at) && at + 1 < lines.len() && down {
            at += 1;
        }
        while title(at) && at > 0 && !down {
            at -= 1;
        }
        // Stuck on a heading at an end: go the other way.
        while title(at) && at + 1 < lines.len() {
            at += 1;
        }
        self.row.at = at;
    }
}

/// Slider step: 0.5 for wide (dB) ranges, else a fortieth of the range.
pub fn slider_step(min: f32, max: f32) -> f32 {
    let span = max - min;
    if span >= 8.0 {
        0.5
    } else {
        (span / 40.0).max(0.01)
    }
}

// ---- the pages ----

/// The core's page facts from the terminal's.
fn page_facts(f: &Facts) -> PageFacts {
    PageFacts {
        parts: f.rules.settings.clone(),
        folders: f.folders.iter().map(|m| m.id.clone()).collect(),
        analysed: f.analysed,
        indexed_songs: f.indexed.0,
        stream_bytes: f.storage.stream,
        cover_bytes: f.storage.covers,
        lyrics_bytes: f.storage.lyrics,
        download_songs: f.storage.download_songs,
        ..PageFacts::default()
    }
}

/// One group's page.
pub fn page(id: Gid, p: &StoredPrefs, s: &SettingsState, f: &Facts, own: &Own) -> Page {
    let title = GROUPS.iter().find(|g| g.id == id).map_or("", |g| g.title).to_string();
    let sections = match id {
        Gid::Own if !f.rules.settings.contains(&SettingsPart::Look) => Vec::new(),
        Gid::Own => own_page(own, f),
        Gid::About if !f.rules.settings.contains(&SettingsPart::About) => Vec::new(),
        Gid::About => about(own),
        Gid::Core(page) => {
            let facts = page_facts(f);
            settings_pages::page(page, SettingsClient::Terminal, p, s, &facts)
                .into_iter()
                .map(|sec| Section { title: section_title(sec.id).into(), rows: sec.rows.into_iter().filter(|r| r.shown).flat_map(|r| flag_rows(&r, s).unwrap_or_else(|| row_of(r, p, s, f).into_iter().collect())).collect() })
                .collect()
        }
    };
    Page { title, sections }
}

/// The terminal's own group.
fn own_page(o: &Own, f: &Facts) -> Vec<Section> {
    let toggle = |switch, title: &str, detail: &str, on, enabled| Row::Toggle { switch, title: title.into(), detail: detail.into(), on, enabled };
    let mut devices = vec![("system default".to_string(), String::new())];
    devices.extend(f.devices.iter().map(|d| (d.clone(), d.clone())));
    if !o.device.is_empty() && !f.devices.contains(&o.device) {
        devices.push((format!("{} (not found)", o.device), o.device.clone()));
    }
    let shown = devices.iter().find(|d| d.1 == o.device).map_or_else(|| o.device.clone(), |d| d.0.clone());
    let rows = vec![
        toggle(Switch::Mouse, "Mouse", "Clicks, the wheel and dragging the bars. Off, the terminal selects text (m)", o.mouse, true),
        toggle(Switch::Images, "Covers", "Album art in the player and on album pages (I)", o.images, true),
        toggle(Switch::CardCovers, "Covers on album cards", "Small pictures on Home and Albums; off keeps a slow link or terminal light", o.card_covers, o.images),
        Row::Choice { target: Target::Device, title: "Output device".into(), options: devices, shown, enabled: true },
        Row::Note { text: "The device is opened at start; --device overrides it for one run.".into() },
    ];
    vec![section("", rows)]
}

fn section(title: &str, rows: Vec<Row>) -> Section {
    Section { title: title.into(), rows }
}

fn info(title: &str, detail: String) -> Row {
    Row::Info { title: title.into(), detail }
}

fn section_title(id: SectionId) -> &'static str {
    use SectionId::*;
    match id {
        Accounts => "Accounts",
        MusicFolders => "Music folders",
        ThisServer => "This server",
        Between => "Transitions",
        Controls => "Controls",
        Queue => "Queue",
        Equalizer => "Equalizer",
        Effects => "Effects",
        Volume => "Volume",
        Output => "Output",
        Theme => "Look",
        Cover => "Cover",
        Home => "Home",
        Screen => "Screen",
        Motion => "Motion",
        Display => "Display",
        Sources => "Online",
        AskedOrder => "Sources, in order",
        Keys => "Keys",
        Lists => "Lists",
        Playlists => "Playlists",
        Search => "Index and search",
        History => "History",
        Online => "Lookups",
        Devices => "Other devices",
        Streaming => "Streaming",
        Downloads => "Downloads",
        Storage => "Cache",
        Updates => "Updates",
    }
}

/// A setting's title and the line under it.
fn setting_words(name: &str, s: &SettingsState) -> (String, String) {
    let (title, detail) = match name {
        "crossfadeSec" => ("Crossfade", "Songs fade into each other, not within an album"),
        "autoMix" => ("AutoMix", "Beat-matched transitions planned per song pair"),
        "autoMixBetterBeats" => {
            let state = match &s.beat_model {
                BeatModel::Ready => "model ready".to_string(),
                BeatModel::Downloading => "downloading".to_string(),
                BeatModel::WaitingForWifi => "waiting for Wi-Fi".to_string(),
                BeatModel::Failed { why } => format!("download failed: {}", text::beat_failure(*why)),
                _ => format!("{} MB model, fetched over Wi-Fi on first use", s.beat_model_mb),
            };
            return ("Neural beat tracking".into(), format!("Beat This! ({state})"));
        }
        "fadeMs" => ("Fade on play/pause", ""),
        "previousAlwaysSkips" => ("Previous always skips", "Never restart the current song"),
        "speed" => ("Speed", ""),
        "pitch" => ("Pitch", ""),
        "skipSilence" => ("Skip silence", if s.untouched { "Off while the output is bit-exact" } else { "Shorten quiet gaps" }),
        "skipExplicit" => ("Skip explicit", "Songs the server tags explicit"),
        "autoFill" => ("Autoplay", "Keep adding music when the queue runs out"),
        "autoFillKind" => ("Add", ""),
        "autoFillBasis" => ("Picked by", ""),
        "autoFillRemote" => ("Include remote songs", "Not in the library yet; each one played is downloaded to it"),
        "bridgeOffline" => ("Offline fallback", "Play downloads while the server is unreachable"),
        "headphoneSound" => ("Headphone sound", "Natural sounds like speakers, wide sounds bigger"),
        "compressor" => ("Compressor", "Evens out quiet and loud parts"),
        "compressorPreset" => ("For", ""),
        "replayGain" => ("Volume leveling", "Every song plays equally loud"),
        "loudnessLevel" => ("Level", "How loud leveled songs play"),
        "volumeBoostDb" => ("Volume boost", ""),
        "hiRes" => ("High quality output", "Float samples to the device; 24-bit files kept whole, effects in float"),
        "maxRate" => ("Highest sample rate", ""),
        "accent" => ("Accent color", ""),
        "coverColors" => ("Colors from the cover", "Where the artwork's colors show (with covers on)"),
        "lyricsSweep" => ("Word fill", "Color words in as they are sung (word-timed lyrics)"),
        "lyricsTranslation" => ("Translations", "When the server has them"),
        "lyricsOnline" => ("Online lookups", "When the server has no timed lyrics; sends artist, title and album"),
        "lyricsPreferWords" => ("Prefer word timing", "Keep asking past line-timed answers"),
        "betterLyricsKey" => ("BetterLyrics key", "For songs it has not cached"),
        "tasteModel" => ("Listening history", "Kept locally; feeds mixes and stats"),
        "scrobble" => ("Scrobble", "Report plays to the server (half the song or 4 minutes)"),
        "thirdPartyLookups" => ("Third-party lookups", "Lyrics services and the AutoEQ list; off, nothing leaves for anyone but your server"),
        "remoteControl" => ("Remote control and jams", "Control playback on your other devices (C), or invite guests to a jam. Uses your network directly or the octo-fiesta relay"),
        "wifi" => ("Streaming quality", ""),
        "download" => ("Download quality", ""),
        "cacheMb" => ("Stream cache limit", ""),
        other => (other, ""),
    };
    (title.to_string(), detail.into())
}

/// A setting value in words.
fn option_words(name: &str, v: &str) -> String {
    let off_or = |v: &str, words: &dyn Fn(&str) -> String| if v == "0" { "off".to_string() } else { words(v) };
    match (name, v) {
        ("crossfadeSec", v) => off_or(v, &|v| format!("{v} s")),
        ("fadeMs", v) => off_or(v, &|v| format!("{v} ms")),
        ("speed" | "pitch", v) => format!("{v}×"),
        ("autoFillKind", "SONGS") => "songs".into(),
        ("autoFillKind", "ALBUMS") => "albums".into(),
        ("autoFillBasis", "SIMILAR") => "similarity".into(),
        ("autoFillBasis", "ARTIST") => "artist".into(),
        ("autoFillBasis", "GENRE") => "genre".into(),
        ("autoFillBasis", "ERA") => "era".into(),
        ("headphoneSound", "OFF") | ("replayGain", "OFF") => "off".into(),
        ("headphoneSound", "NATURAL") => "natural".into(),
        ("headphoneSound", "WIDE") => "wide".into(),
        ("compressorPreset", "LIGHT") => "light".into(),
        ("compressorPreset", "CAR") => "the car".into(),
        ("compressorPreset", "NIGHT") => "the night".into(),
        ("replayGain", "TRACK") => "per song".into(),
        ("replayGain", "ALBUM") => "per album".into(),
        ("replayGain", "AUTO") => "automatic".into(),
        ("loudnessLevel", "QUIET") => "quiet (−23 LUFS)".into(),
        ("loudnessLevel", "NORMAL") => "normal (−18 LUFS)".into(),
        ("loudnessLevel", "LOUD") => "loud (−14 LUFS)".into(),
        ("maxRate", "AUTO") => "each song's own".into(),
        ("maxRate", "KHZ48") => "48 kHz".into(),
        ("maxRate", "KHZ96") => "96 kHz".into(),
        ("maxRate", "KHZ192") => "192 kHz".into(),
        ("coverColors", "PAGES") => "pages".into(),
        ("coverColors", "PLAYER") => "player".into(),
        ("wifi" | "download", v) => quality(v),
        ("cacheMb", v) => match v.parse::<i32>() {
            Ok(mb) if mb % 1024 == 0 => format!("{} GB", mb / 1024),
            _ => format!("{v} MB"),
        },
        (_, v) => v.to_string(),
    }
}

/// A set of independent flags as its title, then a switch for each; None for any other row.
fn flag_rows(r: &settings_pages::SettingsRow, s: &SettingsState) -> Option<Vec<Row>> {
    let RowItem::Setting { spec, value, .. } = &r.item else { return None };
    if spec.kind != SettingKind::Flags {
        return None;
    }
    let (title, detail) = setting_words(&spec.name, s);
    let switches = spec.options.iter().map(|flag| Row::Toggle {
        switch: Switch::Stores(spec.name.clone(), settings_model::flag_toggled(value.clone(), flag.clone())),
        title: format!("  {}", option_words(&spec.name, flag)),
        detail: String::new(),
        on: settings_model::flag_on(value.clone(), flag.clone()),
        enabled: r.enabled,
    });
    Some(std::iter::once(Row::Info { title, detail: detail.to_string() }).chain(switches).collect())
}

/// The terminal's row for one of the core's, or None for one it does not draw.
fn row_of(r: settings_pages::SettingsRow, p: &StoredPrefs, s: &SettingsState, f: &Facts) -> Option<Row> {
    let indent = if r.depth > 0 { "  " } else { "" };
    let enabled = r.enabled;
    Some(match r.item {
        RowItem::Setting { spec, value, .. } => {
            let (title, detail) = setting_words(&spec.name, s);
            let title = format!("{indent}{title}");
            match spec.kind {
                SettingKind::Switch => Row::Toggle { switch: Switch::Setting(spec.name), title, detail, on: value == "true", enabled },
                SettingKind::Choice => {
                    let options: Vec<(String, String)> = spec.options.iter().map(|v| (option_words(&spec.name, v), v.clone())).collect();
                    let shown = options.iter().find(|o| o.1 == value).map_or_else(|| value.clone(), |o| o.0.clone());
                    Row::Choice { target: Target::Setting(spec.name), title, options, shown, enabled }
                }
                SettingKind::Level => {
                    let v: f32 = value.parse().unwrap_or(spec.min);
                    let level = (spec.name == "volumeBoostDb").then_some(EqLevel::VolumeBoost);
                    let label = format!("{title} {}", if v > 0.0 { format!("+{v:.1} dB") } else { "off".into() });
                    Row::Slider { name: spec.name, label, value: v, min: spec.min, max: spec.max, centred: false, level }
                }
                SettingKind::Text => Row::Text { name: spec.name, title, detail, value, secret: true },
                SettingKind::Colour => Row::Palette { name: spec.name, colours: spec.options.iter().filter_map(|c| c.parse().ok()).collect(), chosen: p.accent },
                // Drawn by `flag_rows`.
                SettingKind::Flags => return None,
            }
        }
        RowItem::Link { to: SettingsLink::Equalizer } => {
            let status = if p.sound_bypass { "no processing" } else if s.untouched { "bypassed" } else if s.sound_chain_on { "on" } else { "off" };
            Row::Link { title: "Equalizer, balance, limiter".into(), status: status.into(), action: Act::Equalizer }
        }
        RowItem::Link { to: SettingsLink::Downloads } => {
            Row::Link { title: "Downloads".into(), status: format!("{} songs, {}", f.storage.download_songs, numbers::bytes(f.storage.downloads, POINT)), action: Act::Downloads }
        }
        RowItem::Link { .. } | RowItem::CompressionMeter => return None,
        RowItem::Action { action } => {
            let bytes = |b| numbers::bytes(b, POINT);
            let (indexed_songs, albums, artists) = f.indexed;
            let chore = |title: &str, detail: String, button: &str, chore| Row::Action { title: format!("{indent}{title}"), detail, button: button.into(), enabled, action: Act::Chore(chore) };
            match action {
                SettingsAction::MeasureAgain => chore("Measured songs", format!("{} songs with tempo and beats", f.analysed), "Forget", Chore::MeasureAgain),
                SettingsAction::SyncLibrary => chore("Offline index", format!("{indexed_songs} songs, {albums} albums, {artists} artists"), "Update", Chore::SyncLibrary),
                SettingsAction::DownloadLibrary => chore("Download everything", "Every indexed song".into(), "Download", Chore::DownloadLibrary),
                SettingsAction::ClearStream => chore("Stream cache", bytes(f.storage.stream), "Clear", Chore::ClearStream),
                SettingsAction::ClearCovers => chore("Cover cache", bytes(f.storage.covers), "Clear", Chore::ClearCovers),
                SettingsAction::ClearLyrics => chore("Lyrics cache", format!("{} of lyrics found online", bytes(f.storage.lyrics)), "Clear", Chore::ClearLyrics),
                SettingsAction::AddServer => Row::Button { title: "Add a server".into(), action: Act::AddServer },
                SettingsAction::JoinJam => Row::Button { title: text::JAM_JOIN.into(), action: Act::JoinJam },
                SettingsAction::AnalyseDownloads | SettingsAction::CheckUpdate => return None,
            }
        }
        RowItem::Note { note } => Row::Note {
            text: match note {
                SettingsNote::HeldByDac => "Bit-exact output is on: no mixing, skipping or effects.",
                SettingsNote::Boost => "The boost brings the limiter with it, so nothing clips.",
                SettingsNote::LyricsSources => "Enter switches a source; ← → move it. The best-scored answer wins; order breaks near ties.",
                SettingsNote::Dac => return None,
            }
            .into(),
        },
        RowItem::Stored => info("Database", numbers::bytes(f.storage.database, POINT)),
        RowItem::Server { index } => {
            let sv = p.servers.get(index as usize)?;
            let active = sv.id == p.active_server_id;
            let words = text::provider(sv.kind);
            let who = if sv.user.is_empty() { words.no_user } else { sv.user.as_str() };
            let mut detail = if sv.url.is_empty() { who.to_string() } else { format!("{who} · {}", sv.url) };
            if sv.wifi_only {
                detail.push_str(" · Wi-Fi only");
            }
            let label = Some(nori_core::settings::label(&sv.name, &sv.url)).filter(|l| !l.is_empty()).unwrap_or_else(|| words.name.into());
            Row::Server { id: sv.id.clone(), label, detail, active }
        }
        RowItem::MusicFolder { index, on, tapped } => {
            Row::Toggle { switch: Switch::Stores("musicFolders".into(), tapped), title: f.folders.get(index as usize)?.name.clone(), detail: String::new(), on, enabled }
        }
        RowItem::LyricsSource { index } => {
            let LyricsSource { id, on, needs_key, key_set } = s.lyrics_sources.get(index as usize)?;
            let (title, detail) = service(id);
            let detail = if *key_set { format!("{detail} · key set") } else { detail.to_string() };
            Row::Ranked { name: format!("lyricsService:{id}"), id: id.clone(), title: title.into(), detail, on: *on, keyed: *needs_key }
        }
    })
}

/// A lyrics service's name and description by its core id.
fn service(id: &str) -> (&'static str, &'static str) {
    match id {
        "PAXSENIX" => ("PaxSenix", "Apple Music, syllable-timed · unofficial"),
        "BINILYRICS" => ("BiniLyrics", "Apple Music, syllable-timed · unofficial"),
        "UNISON" => ("Unison", "open, community-timed"),
        "BETTER_LYRICS" => ("BetterLyrics", "Apple Music, syllable-timed · key finds more"),
        "KUGOU" => ("KuGou", "word-timed, CJK · unofficial"),
        "NETEASE" => ("NetEase", "word-timed, Chinese · unofficial"),
        "LYRICS_PLUS" => ("LyricsPlus", "YouLy+, syllable-timed · unofficial"),
        "PAXSENIX_MUSIXMATCH" => ("PaxSenix Musixmatch", "word-timed · needs a PaxSenix key"),
        "LRCLIB" => ("LRCLIB", "open, line-timed"),
        "PAXSENIX_SPOTIFY" => ("PaxSenix Spotify", "line-timed · needs a PaxSenix key"),
        "YOUTUBE_MUSIC" => ("YouTube Music", "untimed · unofficial"),
        _ => ("?", ""),
    }
}

/// A stream quality value ("320:mp3") in words.
fn quality(v: &str) -> String {
    match v.split_once(':') {
        Some((_, "")) | None => "original".into(),
        Some((rate, format)) => format!("{format} {rate}k"),
    }
}

/// Version and credits.
fn about(own: &Own) -> Vec<Section> {
    let version = Section {
        title: "nori".into(),
        rows: vec![
            info("Version", env!("CARGO_PKG_VERSION").into()),
            info("Terminal client", "ratatui over crossterm; covers through ratatui-image".into()),
            info("Pictures drawn with", own.protocol.clone()),
            info("Data kept in", own.data.clone()),
        ],
    };
    let credits = nori_core::credits::core_credits().into_iter().map(|c| info(&c.name, format!("{} · {} · {}", c.what, c.copyright, c.licence))).collect();
    let tui = vec![
        info("ratatui, crossterm", "The screen and the keys · The Ratatui developers, Timon Post · MIT".into()),
        info("ratatui-image", "Covers in the terminal: kitty, sixel, iTerm2, half blocks · Benjamin Große · MIT".into()),
        info("image, icy_sixel", "Pictures handed to the terminal · The image-rs developers, Mike Krüger · MIT or Apache-2.0".into()),
    ];
    vec![version, section("The core", credits), section("The terminal client", tui)]
}

/// The value shown at a row's end (toggles are drawn by the ui).
pub fn row_value(row: &Row) -> String {
    match row {
        Row::Choice { shown, .. } => format!("‹ {shown} ›"),
        Row::Link { status, .. } => format!("{status} ›"),
        Row::Action { button, .. } => format!("[ {button} ]"),
        Row::Button { title, .. } => format!("[ {title} ]"),
        Row::Server { active, .. } => (if *active { "● in use" } else { "" }).into(),
        Row::Text { value, secret, .. } => {
            if value.is_empty() {
                "none".into()
            } else if *secret {
                "••••••".into()
            } else {
                value.clone()
            }
        }
        _ => String::new(),
    }
}

/// A row's title and detail.
pub fn row_words(row: &Row) -> (String, String) {
    match row {
        Row::Toggle { title, detail, .. } | Row::Ranked { title, detail, .. } | Row::Text { title, detail, .. } | Row::Info { title, detail, .. } => (title.clone(), detail.clone()),
        Row::Choice { title, .. } | Row::Link { title, .. } => (title.clone(), String::new()),
        Row::Note { text } => (String::new(), text.clone()),
        Row::Action { title, detail, .. } => (title.clone(), detail.clone()),
        Row::Slider { label, .. } => (label.clone(), String::new()),
        Row::Palette { .. } => ("Accent color".into(), String::new()),
        Row::Server { label, detail, .. } => (label.clone(), detail.clone()),
        Row::Button { .. } => (String::new(), String::new()),
    }
}

/// Whether the row is live; inactive settings are drawn dimmed.
pub fn row_enabled(row: &Row) -> bool {
    match row {
        Row::Toggle { enabled, .. } | Row::Choice { enabled, .. } | Row::Action { enabled, .. } => *enabled,
        _ => true,
    }
}

/// A slider's position (0 to 1) and whether it is centred.
pub fn slider_share(row: &Row) -> Option<(f32, bool)> {
    match row {
        Row::Slider { value, min, max, centred, .. } => Some((((value - min) / (max - min).max(1e-6)).clamp(0.0, 1.0), *centred)),
        _ => None,
    }
}

// ---- the equalizer ----

/// Equalizer screen controls: toolbar, bands, then the rest of the chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EqRow {
    Enabled,
    /// Graphic or parametric.
    Mode,
    /// How many graphic bands.
    Layout,
    Presets,
    AutoPreamp,
    Preamp,
    Band(usize),
    /// A graphic slider.
    Slider(usize),
    AddBand,
    Reset,
    Balance,
    Mono,
    Limiter,
    Ceiling,
}

/// The controls in ← → order.
pub fn eq_rows(p: &StoredPrefs) -> Vec<EqRow> {
    let graphic = p.eq_mode == EqMode::Graphic;
    let mut rows = vec![EqRow::Enabled, EqRow::Mode];
    if graphic {
        rows.push(EqRow::Layout);
    }
    rows.extend([EqRow::Presets, EqRow::AutoPreamp]);
    if p.eq_preamp_db.is_some() {
        rows.push(EqRow::Preamp);
    }
    if !graphic {
        rows.push(EqRow::AddBand);
    }
    rows.push(EqRow::Reset);
    if graphic {
        rows.extend((0..p.eq_graphic.len()).map(EqRow::Slider));
    } else {
        rows.extend((0..p.eq_bands.len()).map(EqRow::Band));
    }
    rows.extend([EqRow::Balance, EqRow::Mono, EqRow::Limiter]);
    if p.limiter {
        rows.push(EqRow::Ceiling);
    }
    rows
}

impl EqRow {
    /// Title and value.
    pub fn words(&self, p: &StoredPrefs) -> (String, String) {
        let on = |b: bool| (if b { "● on" } else { "○ off" }).to_string();
        match *self {
            EqRow::Enabled => ("Equalizer".into(), on(p.eq_enabled)),
            EqRow::Mode => ("Kind".into(), if p.eq_mode == EqMode::Graphic { "‹ graphic ›" } else { "‹ parametric ›" }.into()),
            EqRow::Layout => ("Bands".into(), format!("‹ {} ›", p.eq_graphic.len())),
            EqRow::Slider(i) => {
                let label = nori_core::dsp::graphic_bands(p.eq_graphic.len() as u32).get(i).map_or(0.0, |b| b.label_hz);
                (numbers::iso_band(label, POINT), format!("{} dB", numbers::signed_db(p.eq_graphic.get(i).copied().unwrap_or(0.0), POINT)))
            }
            EqRow::Presets => ("Presets".into(), "choose ›".into()),
            EqRow::AutoPreamp => ("Automatic pre-amp".into(), on(p.eq_preamp_db.is_none())),
            EqRow::Preamp => ("Pre-amp".into(), text::preamp(p.eq_preamp_db.unwrap_or(0.0), false)),
            EqRow::Band(i) => {
                let b = p.eq_bands.get(i).copied().unwrap_or(nori_core::settings::band_from(0, 0.0, 0.0, 1.0, 0));
                (text::band(b.freq, nori_core::settings::band_mark(b.kind as i32, b.channel as i32)), format!("{} dB", numbers::signed_db(b.gain_db, POINT)))
            }
            EqRow::AddBand => ("Add a band".into(), "[ Add ]".into()),
            EqRow::Reset => ("Back to flat".into(), "[ Reset ]".into()),
            EqRow::Balance => ("Balance".into(), text::balance(p.balance)),
            EqRow::Mono => ("Mono".into(), on(p.mono)),
            EqRow::Limiter => ("Limiter".into(), on(p.limiter)),
            EqRow::Ceiling => ("Limiter ceiling".into(), text::ceiling(p.limiter_threshold_db)),
        }
    }

    /// For a band fader: its index and gain.
    pub fn band(&self, p: &StoredPrefs) -> Option<(usize, f32)> {
        match *self {
            EqRow::Slider(i) => Some((i, *p.eq_graphic.get(i)?)),
            EqRow::Band(i) => Some((i, p.eq_bands.get(i)?.gain_db)),
            _ => None,
        }
    }

    /// Whether it is in the toolbar above the bands.
    pub fn above_bands(&self) -> bool {
        matches!(self, EqRow::Enabled | EqRow::Mode | EqRow::Layout | EqRow::Presets | EqRow::AutoPreamp | EqRow::Preamp | EqRow::AddBand | EqRow::Reset)
    }

    /// Sets a band fader to `db`, rounded to 0.5 dB; None if unchanged.
    pub fn set_gain(&self, p: &StoredPrefs, db: f32) -> Option<Cmd> {
        let r = EQ_RANGES.gain;
        let db = ((db * 2.0).round() / 2.0).clamp(r.min, r.max);
        match *self {
            EqRow::Slider(i) => (p.eq_graphic.get(i).copied()? != db).then_some(Cmd::Graphic(i as u32, db)),
            EqRow::Band(i) => {
                let b = *p.eq_bands.get(i)?;
                (b.gain_db != db).then_some(Cmd::Band(i as u32, SoundBand { gain_db: db, ..b }))
            }
            _ => None,
        }
    }

    /// Whether a single click activates it.
    pub fn clicks(&self) -> bool {
        matches!(self, EqRow::Enabled | EqRow::Mode | EqRow::Presets | EqRow::AutoPreamp | EqRow::AddBand | EqRow::Reset | EqRow::Mono | EqRow::Limiter)
    }

    /// Chip label.
    pub fn chip(&self, p: &StoredPrefs) -> String {
        let (title, value) = self.words(p);
        match self {
            EqRow::Enabled => (if p.eq_enabled { "● Equalizer on" } else { "○ Equalizer off" }).into(),
            EqRow::Presets => "Presets ▾".into(),
            EqRow::AddBand => "+ Add a band".into(),
            EqRow::Reset => "↺ Flat".into(),
            EqRow::Mode => (if p.eq_mode == EqMode::Graphic { "Graphic ‹›" } else { "Parametric ‹›" }).into(),
            EqRow::Layout => format!("{} bands ‹›", p.eq_graphic.len()),
            _ => format!("{} {}", title.trim(), value),
        }
    }

    /// ↑ or ↓ on it; None when the value would not change, so a key held at a range end sends nothing.
    pub fn step(&self, p: &StoredPrefs, up: bool) -> Option<Cmd> {
        let d = if up { 0.5 } else { -0.5 };
        let r = EQ_RANGES;
        let level = |level: EqLevel, was: f32, to: f32| (to != was).then_some(Cmd::Level(level, to));
        match *self {
            EqRow::Enabled => (p.eq_enabled != up).then_some(Cmd::Setting("eq".into(), up.to_string())),
            EqRow::Mode => {
                let want = if up { EqMode::Graphic } else { EqMode::Parametric };
                (p.eq_mode != want).then(|| Cmd::Setting("eqMode".into(), if up { "GRAPHIC" } else { "PARAMETRIC" }.into()))
            }
            EqRow::Layout => {
                let at = nori_core::dsp::graphic::LAYOUTS.iter().position(|n| *n == p.eq_graphic.len()).unwrap_or(0);
                let to = if up { (at + 1).min(nori_core::dsp::graphic::LAYOUTS.len() - 1) } else { at.saturating_sub(1) };
                (to != at).then(|| Cmd::Setting("eqLayout".into(), nori_core::dsp::graphic::LAYOUTS[to].to_string()))
            }
            EqRow::Slider(i) => {
                let was = *p.eq_graphic.get(i)?;
                let to = (was + d).clamp(r.gain.min, r.gain.max);
                (to != was).then_some(Cmd::Graphic(i as u32, to))
            }
            EqRow::Mono => (p.mono != up).then_some(Cmd::Setting("mono".into(), up.to_string())),
            EqRow::Limiter => (p.limiter != up).then_some(Cmd::Setting("limiter".into(), up.to_string())),
            EqRow::AutoPreamp => (p.eq_preamp_db.is_none() != up).then_some(Cmd::Sound(SoundToolCmd::AutoPreamp(up))),
            EqRow::Preamp => {
                let was = p.eq_preamp_db.unwrap_or(0.0);
                level(EqLevel::Preamp, was, (was + d).clamp(r.preamp.min, r.preamp.max))
            }
            EqRow::Band(i) => {
                let b = *p.eq_bands.get(i)?;
                let gain_db = (b.gain_db + d).clamp(r.gain.min, r.gain.max);
                (gain_db != b.gain_db).then_some(Cmd::Band(i as u32, SoundBand { gain_db, ..b }))
            }
            EqRow::Balance => level(EqLevel::Balance, p.balance, (p.balance + d / 10.0).clamp(r.balance.min, r.balance.max)),
            EqRow::Ceiling => level(EqLevel::Limiter, p.limiter_threshold_db, (p.limiter_threshold_db + d).clamp(r.limiter.min, r.limiter.max)),
            EqRow::Presets | EqRow::AddBand | EqRow::Reset => None,
        }
    }

    /// Enter.
    pub fn open(&self, p: &StoredPrefs) -> Option<Cmd> {
        match *self {
            EqRow::Enabled => Some(Cmd::Setting("eq".into(), (!p.eq_enabled).to_string())),
            EqRow::Mode => self.step(p, p.eq_mode != EqMode::Graphic),
            EqRow::Mono => Some(Cmd::Setting("mono".into(), (!p.mono).to_string())),
            EqRow::Limiter => Some(Cmd::Setting("limiter".into(), (!p.limiter).to_string())),
            EqRow::AutoPreamp => Some(Cmd::Sound(SoundToolCmd::AutoPreamp(p.eq_preamp_db.is_some()))),
            EqRow::AddBand => Some(Cmd::Sound(SoundToolCmd::AddBand)),
            EqRow::Reset => Some(Cmd::Sound(SoundToolCmd::ResetBands)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A jam guest's settings are this computer's: the profiles to switch back with, sound, look and
    /// caches, but nothing of the account's or the host's server.
    #[test]
    fn a_guest_sees_only_this_computers_settings() {
        let prefs = StoredPrefs { servers: vec![nori_core::settings::SavedServer { id: "a".into(), ..Default::default() }], active_server_id: "a".into(), ..StoredPrefs::default() };
        let state = settings_model::state(&prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
        let titles = |rules: nori_core::browse::ProfileRules| -> Vec<String> {
            let facts = Facts { folders: vec![MusicFolder { id: "1".into(), name: "A".into() }, MusicFolder { id: "2".into(), name: "B".into() }], rules, ..Facts::default() };
            let pages = GROUPS.iter().map(|g| page(g.id, &prefs, &state, &facts, &Own::default()));
            pages.flat_map(|p| p.sections.into_iter().flat_map(|s| std::iter::once(s.title.clone()).chain(s.rows.iter().map(|r| row_words(r).0)).collect::<Vec<_>>())).collect()
        };
        let account = titles(nori_core::browse::profile_rules(false));
        let guest = titles(nori_core::browse::profile_rules(true));
        for kept in ["Accounts", "Streaming quality", "Stream cache", "Colors from the cover"] {
            assert!(account.iter().any(|t| t == kept) && guest.iter().any(|t| t == kept), "{kept}");
        }
        for gone in ["Music folders", "Download quality", "Downloads"] {
            assert!(account.iter().any(|t| t == gone) && !guest.iter().any(|t| t == gone), "{gone}");
        }
        assert!(guest.len() < account.len());
    }

    fn every_row(prefs: &StoredPrefs) -> Vec<(&'static str, Row)> {
        let facts = Facts { folders: vec![MusicFolder { id: "1".into(), name: "A".into() }, MusicFolder { id: "2".into(), name: "B".into() }], ..Facts::default() };
        every_row_with(prefs, &facts)
    }

    fn every_row_with(prefs: &StoredPrefs, facts: &Facts) -> Vec<(&'static str, Row)> {
        let state = settings_model::state(prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
        GROUPS
            .iter()
            .flat_map(|g| page(g.id, prefs, &state, facts, &Own::default()).sections.into_iter().flat_map(move |s| s.rows.into_iter().map(move |r| (g.title, r))))
            .collect()
    }

    /// Settings that open every row there is.
    fn everything_on() -> StoredPrefs {
        let server = nori_core::settings::SavedServer { id: "a".into(), url: "https://a".into(), alt_url: "https://b".into(), ..Default::default() };
        StoredPrefs {
            auto_mix: true,
            auto_fill: true,
            compressor: true,
            replay_gain: nori_core::settings::GainMode::Track,
            scrobble: true,
            lyrics_online: true,
            third_party_lookups: true,
            servers: vec![server],
            active_server_id: "a".into(),
            ..StoredPrefs::default()
        }
    }

    #[test]
    fn rows_and_labels() {
        let prefs = everything_on();
        let rows = every_row(&prefs);
        assert!(rows.len() > 40, "rows: {}", rows.len());
        for (group, row) in &rows {
            let mut v = SettingsView::default();
            v.pages = Some(vec![Page { title: group.to_string(), sections: vec![Section { title: String::new(), rows: vec![row.clone()] }] }]);
            v.row.at = 1;
            match row {
                Row::Toggle { switch: Switch::Setting(name), enabled: true, .. } => {
                    let Some(Opened::Cmds(c)) = v.open() else { panic!("{name} does not switch") };
                    assert!(matches!(&c[0], Cmd::Setting(n, _) if n == name), "{name}");
                }
                Row::Choice { target: Target::Setting(name), enabled: true, options, .. } => {
                    let Some(Opened::Overlay(Overlay::Picker { target: picked, options: shown, .. })) = v.open() else { panic!("{name} offers no choice") };
                    assert_eq!(picked, Target::Setting(name.clone()));
                    assert_eq!(shown.len(), options.len());
                    let steps = [v.step(true), v.step(false)].concat();
                    assert!(steps.iter().all(|c| matches!(c, Cmd::Setting(n, _) if n == name)), "{name}");
                    for (_, value) in options {
                        assert!(nori_core::settings::set_by_name(&prefs, name, value).is_some(), "{name} = {value}");
                    }
                }
                Row::Slider { name, level, .. } => {
                    let c = v.step(true);
                    assert!(matches!(&c[..], [Cmd::Level(l, _)] if Some(*l) == *level) || matches!(&c[..], [Cmd::Setting(n, _)] if n == name), "{name}");
                }
                _ => {}
            }
        }

        // Choice labels.
        let d = StoredPrefs::default();
        let rows = every_row(&d);
        let shown = |name: &str| {
            rows.iter()
                .find_map(|(_, r)| match r {
                    Row::Choice { target: Target::Setting(n), shown, .. } if n == name => Some(shown.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no {name}"))
        };
        assert_eq!(shown("replayGain"), "automatic");
        assert_eq!(shown("wifi"), "original");
        assert_eq!(shown("speed"), "1×");
        assert_eq!(quality("320:mp3"), "mp3 320k");

        // Phone only settings hidden.
        let names: Vec<String> = every_row(&everything_on())
            .into_iter()
            .filter_map(|(_, r)| match r {
                Row::Toggle { switch: Switch::Setting(name), .. } | Row::Choice { target: Target::Setting(name), .. } => Some(name),
                _ => None,
            })
            .collect();
        for phone in ["swipeLeft", "swipeRight", "tapAction", "offload", "bitPerfect", "movingCovers", "amoled", "dynamicColor", "lyricsKeepScreenOn", "motion", "soundBypass", "altMaxBitRate"] {
            assert!(!names.iter().any(|n| n == phone), "{phone} offered in a terminal");
        }
    }

    #[test]
    fn music_folders_switch_from_all() {
        let server = nori_core::settings::SavedServer { id: "a".into(), ..Default::default() };
        let prefs = StoredPrefs { servers: vec![server], active_server_id: "a".into(), ..StoredPrefs::default() };
        let state = settings_model::state(&prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
        let facts = Facts { folders: vec![MusicFolder { id: "1".into(), name: "A".into() }, MusicFolder { id: "2".into(), name: "B".into() }], ..Facts::default() };
        let page = page(Gid::Core(SettingsPage::Servers), &prefs, &state, &facts, &Own::default());
        let folders = page.sections.iter().find(|s| s.title == "Music folders").unwrap();
        let switches: Vec<_> = folders.rows.iter().map(|r| match r {
            Row::Toggle { switch: Switch::Stores(name, value), title, on, .. } => (name.as_str(), value.as_str(), title.as_str(), *on),
            other => panic!("{other:?}"),
        }).collect();
        assert_eq!(switches, [("musicFolders", "2", "A", true), ("musicFolders", "1", "B", true)]);
    }

    #[test]
    fn lyrics_sources_move_and_switch() {
        // All services default on; one is switched off to have an off row.
        let d = StoredPrefs::default();
        let prefs = StoredPrefs { lyrics_online: true, third_party_lookups: true, lyrics_on: d.lyrics_on.iter().filter(|s| s.name() != "UNISON").copied().collect(), ..d };
        let mut v = SettingsView::default();
        let pages = v.pages(&prefs).to_vec();
        let lines = SettingsView::lines(&pages);
        let ranked: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Row(Row::Ranked { .. }))).map(|(i, _)| i).collect();
        assert_eq!(ranked.len(), nori_core::lyrics_sources::LyricsService::ALL.len());
        assert!(lines.iter().all(|l| !matches!(l, Line::Row(Row::Ranked { title, .. }) if title == "?")), "every service has a name");
        let off = *ranked.iter().find(|i| matches!(lines[**i], Line::Row(Row::Ranked { on: false, .. }))).unwrap();
        let Line::Row(Row::Ranked { id, name, .. }) = lines[off] else { unreachable!() };
        let (id, name) = (id.clone(), name.clone());
        v.row.at = off;
        assert!(matches!(&v.step(false)[..], [Cmd::Setting(n, value)] if n == "lyricsMove" && *value == format!("{id}:-1")));
        let Some(Opened::Cmds(c)) = v.open() else { panic!() };
        assert!(matches!(&c[..], [Cmd::Setting(n, value)] if *n == name && value == "true"));
        // A service that needs a key asks for it when switched on.
        let keyed = *ranked.iter().find(|i| matches!(lines[**i], Line::Row(Row::Ranked { keyed: true, .. }))).unwrap();
        v.row.at = keyed;
        let Some(Opened::Overlay(Overlay::Input { name, secret: true, .. })) = v.open() else { panic!("no key asked") };
        assert!(name.starts_with("lyricsKey:"), "{name}");
        // Third-party lookups off: no sources listed.
        let quiet = StoredPrefs { third_party_lookups: false, ..prefs };
        v.invalidate();
        let pages = v.pages(&quiet).to_vec();
        assert!(!SettingsView::lines(&pages).iter().any(|l| matches!(l, Line::Row(Row::Ranked { .. }))));
    }

    #[test]
    fn sound_controls() {
        let p = StoredPrefs { eq_mode: EqMode::Graphic, ..StoredPrefs::default() };
        let rows = eq_rows(&p);
        assert_eq!(rows.iter().filter(|r| matches!(r, EqRow::Slider(_))).count(), 10);
        assert!(!rows.iter().any(|r| matches!(r, EqRow::Band(_) | EqRow::AddBand)), "no parametric bands on the graphic one");
        assert_eq!(EqRow::Slider(0).words(&p), ("31.5".to_string(), "+0.0 dB".to_string()));
        assert!(matches!(EqRow::Slider(3).step(&p, true), Some(Cmd::Graphic(3, v)) if v == 0.5));
        assert!(matches!(EqRow::Layout.step(&p, true), Some(Cmd::Setting(n, v)) if n == "eqLayout" && v == "15"));
        assert!(matches!(EqRow::Layout.step(&p, false), Some(Cmd::Setting(n, v)) if n == "eqLayout" && v == "5"));
        let five = StoredPrefs { eq_graphic: vec![0.0; 5], ..p.clone() };
        assert!(EqRow::Layout.step(&five, false).is_none(), "five is the fewest");
        assert_eq!(eq_rows(&five).iter().filter(|r| matches!(r, EqRow::Slider(_))).count(), 5);
        assert_eq!(EqRow::Slider(0).words(&five).0, "63");
        assert!(matches!(EqRow::Mode.open(&p), Some(Cmd::Setting(n, v)) if n == "eqMode" && v == "PARAMETRIC"));
        assert!(rows.contains(&EqRow::Layout));
        let parametric = eq_rows(&StoredPrefs { eq_mode: EqMode::Parametric, ..StoredPrefs::default() });
        assert!(parametric.contains(&EqRow::Mode) && !parametric.contains(&EqRow::Layout));

        // Parametric bands step in range.
        let prefs = StoredPrefs { eq_mode: EqMode::Parametric, eq_bands: nori_core::settings::graphic(), ..StoredPrefs::default() };
        let rows = eq_rows(&prefs);
        assert_eq!(rows.iter().filter(|r| matches!(r, EqRow::Band(_))).count(), prefs.eq_bands.len());
        let Some(Cmd::Band(0, b)) = EqRow::Band(0).step(&prefs, true) else { panic!() };
        assert_eq!(b.gain_db, prefs.eq_bands[0].gain_db + 0.5);
        let loud = StoredPrefs { eq_bands: prefs.eq_bands.iter().map(|b| SoundBand { gain_db: 12.0, ..*b }).collect(), ..prefs.clone() };
        assert_eq!(EqRow::Band(0).step(&loud, true), None, "held at the top of the core's range: nothing asked");
        let Some(Cmd::Band(_, b)) = EqRow::Band(0).step(&loud, false) else { panic!() };
        assert_eq!(b.gain_db, 11.5);
    }

}
