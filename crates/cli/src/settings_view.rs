//! The settings page (one scrolling page of groups, sections and rows) and the equalizer's rows. The
//! pages, their rows and values are the core's (`settings_pages`, terminal client); this words and
//! draws them. Rows send back a setting name and value (`setting_set`) or a level edit (`edit_level`).

use crate::text;
use nori_core::settings::{season_follow_value, EqLevel, EqMode, SoundBand, StoredPrefs, ThemeSeason, EQ_RANGES};
use nori_core::settings_model::{self, BeatModel, EqStatus, KeyAction, LyricsSource, SettingKind, SettingsState};
use nori_core::settings_pages::{self, PageFacts, RowItem, SectionId, SettingsAction, SettingsClient, SettingsLink, SettingsNote, SettingsPage, SettingsPart};
use nori_core::numbers::{self, POINT};
use nori_core::providers::Kinds;
use nori_core::MusicFolder;
use std::collections::{HashMap, HashSet};

use nori_plugins::{AssetLine, AssetState, ControlledDevice, Offer, OfferState, Offers, PluginCard, PluginRow, ReadmeBlock, Repo, RowKind, Value};

use crate::app::{Cmd, Overlay, PluginCmd, Sel, SoundToolCmd, Target, View};
use crate::art::Day;

/// A group of the page: the terminal's own, one of the core's pages, the plugins, or About.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gid {
    Own,
    Core(SettingsPage),
    Plugins,
    About,
}

/// What opening a row does.
pub enum Opened {
    Cmds(Vec<Cmd>),
    Overlay(Overlay),
    View(View),
    /// A terminal-only switch (never `Switch::Setting`).
    Own(Switch),
    /// One of the plugin pages.
    Plugins(PluginPage),
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
    /// The terminal's own background instead of the season's page.
    Transparent,
    /// A plugin's switch row.
    Plugin { plugin: String, row: String },
    /// A plugin itself, on its page.
    Turn(String),
    /// Keeps a plugin at its version: no automatic updates.
    Hold(String),
    /// Every plugin off (the `pluginsOff` setting), the plugins read again after.
    AllOff,
}

/// A settings group: a headed part of the page, and what it holds in a few words.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: Gid,
    pub title: &'static str,
    pub summary: &'static str,
}

/// The groups in page order: the core's pages in its order, the terminal's own after the look.
pub const GROUPS: [Group; 10] = [
    Group { id: Gid::Core(SettingsPage::Servers), title: "Servers", summary: "Accounts and music folder" },
    Group { id: Gid::Core(SettingsPage::Playing), title: "Playback", summary: "Crossfade, AutoMix, queue" },
    Group { id: Gid::Core(SettingsPage::Sound), title: "Sound", summary: "Equalizer, volume, output" },
    Group { id: Gid::Core(SettingsPage::Look), title: "Appearance", summary: "Theme, colors, size" },
    Group { id: Gid::Own, title: "Terminal", summary: "Mouse, covers, output device" },
    Group { id: Gid::Core(SettingsPage::Lyrics), title: "Lyrics", summary: "Display and sources" },
    Group { id: Gid::Core(SettingsPage::Library), title: "Library", summary: "Lists, search, history" },
    Group { id: Gid::Core(SettingsPage::Data), title: "Downloads and storage", summary: "Quality, downloads, space" },
    Group { id: Gid::Plugins, title: "Plugins", summary: "Installed plugins and their settings" },
    Group { id: Gid::About, title: "About", summary: "Version, licenses" },
];

/// What a link or button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Equalizer,
    Downloads,
    /// The login, for another server.
    AddServer,
    /// The invite link of someone's jam, to join it.
    JoinJam,
    Chore(Chore),
    /// Asks for a plugin's file or folder to install.
    InstallPlugin,
    /// A plugin's action row.
    PluginAction { plugin: String, row: String },
    /// Asks the repos what they offer.
    LookForPlugins,
    /// One of the plugin pages.
    Page(PluginPage),
    /// Installs version `version` (newest first) of the offer at place `offer` in those last read.
    GetPlugin { offer: u32, version: u32 },
    /// Asks for a repo's address to add.
    AddRepo,
    /// Lets a plugin fetch its assets on mobile data.
    AllowMobileAssets(String),
    /// Asks where to save a backup, or which one to restore.
    Backup { restore: bool },
}

pub use nori_host::session::Chore;

/// One row of a page.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Toggle { switch: Switch, title: String, detail: String, on: bool, enabled: bool },
    /// `options` are (label, value); `shown` is the chosen label, or the raw value if none matches.
    /// `chips`: few options, all on view.
    Choice { target: Target, title: String, options: Vec<(String, String)>, shown: String, enabled: bool, chips: bool },
    /// One of the four seasons: its name, its kanji, its theme's accent in the mode the interface is in,
    /// whether it is the one worn, and whether picking it changes the setting.
    Season { value: String, name: &'static str, kanji: &'static str, accent: u32, worn: bool, picked: bool, enabled: bool },
    Note { text: String },
    /// Opens another screen, with a status at the end.
    Link { title: String, status: String, action: Act },
    /// Text with a button at the end.
    Action { title: String, detail: String, button: String, enabled: bool, action: Act },
    Info { title: String, detail: String },
    /// ← → move it by `step`.
    Slider { target: Target, label: String, value: f32, min: f32, max: f32, step: f32, centred: bool },
    /// A line of text: enter asks for a new one. A secret's is never shown.
    Text { title: String, shown: String, secret: bool, target: Target },
    /// An installed plugin: enter opens its page, ← → switch it, K J move it, d removes it.
    Plugin { id: String, title: String, detail: String, on: bool, enabled: bool },
    /// A plugin a repo offers, with where it stands: enter opens its page.
    Offer { id: String, title: String, detail: String, status: String },
    /// A line of a plugin's words, wrapped to the page. `quiet`: code, in the quiet ink.
    Prose { text: String, quiet: bool },
    /// A plugin repo; d removes one the user added.
    Repo { url: String, title: String, detail: String, removable: bool },
    Server { id: String, label: String, detail: String, active: bool },
    Button { title: String, action: Act },
    /// A lyrics service in the ranked list: enter switches it, ← → move it, `key` adds or changes its key.
    /// `asks_key`: switching it on asks for its key.
    Ranked { name: String, id: String, title: String, detail: String, on: bool, asks_key: bool, key: KeyAction, enabled: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub title: String,
    pub summary: String,
    pub sections: Vec<Section>,
}

/// The profile whose rows are kept under `profile` (`server_db`), as the server list names it.
fn profile_name(p: &StoredPrefs, kinds: &Kinds, profile: &str) -> String {
    p.servers.iter().find(|sv| nori_core::settings::server_db_id(&sv.id) == profile).map_or_else(
        || "a removed profile".into(),
        |sv| Some(nori_core::settings::label(&sv.name, &sv.url)).filter(|l| !l.is_empty()).unwrap_or_else(|| text::provider_of(kinds, &sv.kind).name),
    )
}

/// Local storage use in bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Storage {
    pub stream: i64,
    pub covers: i64,
    pub lyrics: i64,
    /// Every profile's downloads: they share this computer's download folder.
    pub downloads: i64,
    pub all_download_songs: u32,
    /// Each profile with downloads (the id its rows are kept under) and what they hold.
    pub download_profiles: Vec<(String, i64)>,
    /// This profile's finished downloads, which its actions work on.
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
    /// Changes waiting for the server, and dropped unsent.
    pub unsent: (u32, u32),
    /// What the open profile offers: a jam guest's settings are this device's own.
    pub rules: nori_core::browse::ProfileRules,
}

/// A page line: group heading, section title or row.
pub enum Line<'a> {
    /// A group's title and what it holds.
    Group(&'a str, &'a str),
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
    /// Only the plugins' group, opened from the first screen before any profile.
    pub alone: bool,
}

/// What the page shows beyond the settings: the terminal's own state, the kinds of profile and the
/// plugins.
#[derive(Default, Clone)]
pub struct Own {
    pub mouse: bool,
    pub images: bool,
    /// The page is painted in the season's colors.
    pub paint: bool,
    pub protocol: String,
    pub data: String,
    /// Output device for the next start; empty for the system default.
    pub device: String,
    /// The interface is dark: the seasons show their dark colors.
    pub dark: bool,
    /// What names a plugin's kind of profile.
    pub kinds: std::sync::Arc<Kinds>,
    /// The installed plugins, as last read and as their plugins said them (`Watched::as_said`).
    pub plugins: Vec<PluginCard>,
    /// Rows that show but take no input now, by plugin and row.
    pub off: HashSet<(String, String)>,
    /// Each plugin's assets, those that have some.
    pub assets: HashMap<String, Vec<AssetLine>>,
    /// The connected devices plugins control.
    pub controlled: Vec<ControlledDevice>,
    /// The plugin repos: nori's own first.
    pub repos: Vec<Repo>,
    pub shelf: Shelf,
    /// The plugin pages open over Settings › Plugins, each with the selection to come back to.
    pub stack: Vec<(PluginPage, Sel)>,
    /// The browser's search, and the repo it is narrowed to.
    pub query: String,
    pub repo_shown: Option<String>,
    /// What the README was last asked for ([`SettingsView::readme_wanted`]), and what came.
    pub readme_asked: Option<(String, Option<String>, Option<String>)>,
    pub readme: Vec<ReadmeBlock>,
    /// Columns a plugin's words wrap to; none before the first draw.
    pub wrap: usize,
}

impl Own {
    /// What the repos offer, as last read.
    pub fn offers(&self) -> &[Offer] {
        match &self.shelf {
            Shelf::Read(o) => &o.offers,
            _ => &[],
        }
    }
}

/// A page under Settings › Plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginPage {
    Browser,
    Repos,
    /// A plugin's page, installed or only offered, by its id.
    Plugin(String),
}

/// What the plugin repos offer, as far as they were asked.
#[derive(Default, Clone)]
pub enum Shelf {
    #[default]
    Unasked,
    Asking,
    Read(Offers),
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
            let today = Day::now();
            let alone = self.alone;
            self.pages = Some(match self.own.stack.last() {
                Some((PluginPage::Browser, _)) => vec![Page { title: text::GET_PLUGINS.into(), summary: String::new(), sections: browser(&self.own) }],
                Some((PluginPage::Repos, _)) => vec![Page { title: text::REPOS.into(), summary: String::new(), sections: repos_page(&self.own) }],
                Some((PluginPage::Plugin(id), _)) => vec![plugin_page(&self.own, id)],
                None => GROUPS.iter().filter(|g| !alone || g.id == Gid::Plugins).map(|g| page(g.id, prefs, &state, &self.facts, &self.own, today)).collect(),
            });
        }
        self.pages.as_deref().expect("made above")
    }

    /// All lines as drawn.
    pub fn lines(pages: &[Page]) -> Vec<Line<'_>> {
        let mut out = Vec::new();
        for p in pages {
            out.push(Line::Group(&p.title, &p.summary));
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
        Self::lines(pages).iter().take(self.row.at + 1).filter(|l| matches!(l, Line::Group(..))).count().saturating_sub(1)
    }

    /// Selects a group's first row.
    pub fn jump(&mut self, group: usize) {
        let Some(pages) = self.pages.as_deref() else { return };
        let lines = Self::lines(pages);
        if let Some(at) = lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Group(..))).nth(group).map(|(i, _)| i) {
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
        matches!(self.selected(), Some(Row::Toggle { .. } | Row::Button { .. } | Row::Link { .. } | Row::Action { .. } | Row::Server { .. } | Row::Ranked { .. } | Row::Season { .. } | Row::Text { .. } | Row::Plugin { .. } | Row::Offer { .. }))
    }

    /// Whether a plugin page is open over the settings.
    pub fn deep(&self) -> bool {
        !self.own.stack.is_empty()
    }

    /// Opens `page` over what shows, on its first row that does something.
    pub fn enter(&mut self, page: PluginPage, prefs: &StoredPrefs) {
        self.own.stack.push((page, self.row));
        self.invalidate();
        let lines = Self::lines(self.pages(prefs));
        let first = lines.iter().position(|l| matches!(l, Line::Row(r) if open_row((*r).clone()).is_some()));
        self.row = Sel { at: first.unwrap_or(0), top: 0 };
    }

    /// Back to what the open plugin page covers, the selection where it was; false with none open.
    pub fn leave(&mut self) -> bool {
        let Some((_, row)) = self.own.stack.pop() else { return false };
        self.row = row;
        self.invalidate();
        true
    }

    /// Closes the pages of a plugin just removed.
    pub fn plugin_gone(&mut self, id: &str) {
        while matches!(self.own.stack.last(), Some((PluginPage::Plugin(top), _)) if top == id) {
            self.leave();
        }
    }

    /// Whether the browser shows.
    pub fn browsing(&self) -> bool {
        matches!(self.own.stack.last(), Some((PluginPage::Browser, _)))
    }

    /// The browser's search field, as typed so far.
    pub fn search_field(&self) -> Overlay {
        Overlay::Input { title: text::SEARCH_PLUGINS.into(), text: self.own.query.clone(), secret: false, target: Target::PluginSearch }
    }

    /// What the open plugin page's README has to be asked for: again when the plugin's version or the
    /// offer's address changed. None when asked already.
    pub fn readme_wanted(&mut self) -> Option<Cmd> {
        let Some((PluginPage::Plugin(id), _)) = self.own.stack.last() else { return None };
        let card = self.own.plugins.iter().find(|c| c.id == *id);
        let offer = nori_plugins::offer_of(self.own.offers(), id, card).map(|(_, o)| o.clone());
        let asked = (id.clone(), card.map(|c| c.version.clone()), offer.as_ref().and_then(|o| o.readme.clone()));
        if self.own.readme_asked.as_ref() == Some(&asked) {
            return None;
        }
        let id = id.clone();
        if self.own.readme_asked.as_ref().is_none_or(|a| a.0 != id) {
            self.own.readme.clear();
            self.invalidate();
        }
        self.own.readme_asked = Some(asked);
        Some(Cmd::Plugin(PluginCmd::Readme(id, offer.map(Box::new))))
    }

    /// Plugin `id`'s README came; the page may have moved on.
    pub fn readme_came(&mut self, id: &str, blocks: Vec<ReadmeBlock>) {
        if self.own.readme_asked.as_ref().is_some_and(|a| a.0 == id) {
            self.own.readme = blocks;
            self.invalidate();
        }
    }

    /// The keys the selection takes, for the foot of the screen.
    pub fn hints(&self) -> &'static [(&'static str, &'static str)] {
        use crate::text::hints as h;
        match (self.own.stack.last(), self.selected()) {
            (_, Some(Row::Plugin { .. })) => h::PLUGIN_ROW,
            (Some((PluginPage::Browser, _)), _) => h::PLUGIN_BROWSER,
            (Some((PluginPage::Plugin(_), _)), _) => h::PLUGIN_PAGE,
            (Some(_), _) => h::PLUGIN_REPOS,
            (None, _) => h::SETTINGS,
        }
    }

    /// Enter on the selection.
    pub fn open(&self) -> Option<Opened> {
        open_row(self.selected()?.clone())
    }

    /// ← or → on the selection: previous or next option, off or on, a slider step.
    pub fn step(&self, up: bool) -> Vec<Cmd> {
        self.selected().cloned().map_or_else(Vec::new, |row| step_row(row, up))
    }

    /// The selected lyrics service's key, to add or change.
    pub fn key(&self) -> Option<Overlay> {
        match self.selected()? {
            Row::Ranked { id, title, enabled: true, key: KeyAction::Add | KeyAction::Change, .. } => match key_input(id, title) {
                Opened::Overlay(o) => Some(o),
                _ => None,
            },
            _ => None,
        }
    }

    /// The plugin selected in a list, or whose page shows, if installed.
    fn plugin_here(&self) -> Option<&PluginCard> {
        let id = match (self.selected(), self.own.stack.last()) {
            (Some(Row::Plugin { id, .. }), _) | (_, Some((PluginPage::Plugin(id), _))) => id,
            _ => return None,
        };
        self.own.plugins.iter().find(|c| c.id == *id)
    }

    /// K or J on a plugin: moves it up or down the order.
    pub fn plugin_move(&self, up: bool) -> Option<Cmd> {
        let c = self.plugin_here()?;
        let index = self.own.plugins.iter().position(|o| o.id == c.id)? as u32;
        let to = if up { index.checked_sub(1)? } else { Some(index + 1).filter(|to| (*to as usize) < self.own.plugins.len())? };
        Some(Cmd::Plugin(PluginCmd::Place(c.id.clone(), to)))
    }

    /// Asks before removing plugin `c`.
    fn remove_sheet(c: &PluginCard) -> Overlay {
        Overlay::Confirm { title: text::remove_plugin(&c.name), lines: vec![text::REMOVE_PLUGIN_NOTE.into()], yes: text::REMOVE.into(), cmd: Box::new(Cmd::Plugin(PluginCmd::Uninstall(c.id.clone()))) }
    }

    /// d on a plugin, or a repo the user added: asks before removing it.
    pub fn plugin_key(&self) -> Option<Overlay> {
        if let Some(Row::Repo { url, title, removable: true, .. }) = self.selected() {
            return Some(Overlay::Confirm { title: text::remove_plugin(title), lines: vec![url.clone()], yes: text::REMOVE.into(), cmd: Box::new(Cmd::Plugin(PluginCmd::RemoveRepo(url.clone()))) });
        }
        self.plugin_here().map(Self::remove_sheet)
    }

    /// m on a plugin: moving and removing it.
    pub fn plugin_menu(&self) -> Option<Overlay> {
        let c = self.plugin_here()?;
        let can = |up| self.plugin_move(up).is_some();
        let lines = [(can(true), text::MOVE_UP, "up"), (can(false), text::MOVE_DOWN, "down"), (true, text::REMOVE, "remove")];
        let options = lines.into_iter().filter(|l| l.0).map(|(_, label, value)| (label.to_string(), value.to_string())).collect();
        Some(Overlay::Picker { title: c.name.clone(), options, sel: Sel::default(), target: Target::PluginMenu })
    }

    /// A line of the plugin's menu picked.
    pub fn plugin_menu_picked(&self, value: &str) -> (Option<Overlay>, Option<Cmd>) {
        match value {
            "remove" => (self.plugin_here().map(Self::remove_sheet), None),
            _ => (None, self.plugin_move(value == "up")),
        }
    }

    /// Moves the selection off a heading onto a row, preferring direction `down`.
    pub fn skip_titles(&mut self, down: bool) {
        let Some(pages) = &self.pages else { return };
        let lines = Self::lines(pages);
        let title = |i: usize| matches!(lines.get(i), Some(Line::Title(_) | Line::Group(..)));
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
        unsent_waiting: f.unsent.0,
        unsent_dropped: f.unsent.1,
        ..PageFacts::default()
    }
}

/// One group's page, the season's rows as on `today`.
pub fn page(id: Gid, p: &StoredPrefs, s: &SettingsState, f: &Facts, own: &Own, today: Day) -> Page {
    let title = GROUPS.iter().find(|g| g.id == id).map_or("", |g| g.title).to_string();
    let sections = match id {
        Gid::Own if !f.rules.settings.contains(&SettingsPart::Look) => Vec::new(),
        Gid::Own => own_page(own, f),
        Gid::Plugins if !f.rules.settings.contains(&SettingsPart::Look) => Vec::new(),
        Gid::Plugins => plugins_page(own, p.plugins_off),
        Gid::About if !f.rules.settings.contains(&SettingsPart::About) => Vec::new(),
        Gid::About => about(own),
        Gid::Core(page) => {
            let facts = page_facts(f);
            let mut sections = Vec::new();
            for sec in settings_pages::page(page, SettingsClient::Terminal, p, s, &facts) {
                let mut rows = Vec::new();
                for r in sec.rows.into_iter().filter(|r| r.shown) {
                    // The seasons stand above their section, under a title of their own.
                    if let Some(seasons) = season_rows(&r, p, own.dark, today) {
                        sections.push(section(SEASON, seasons));
                        continue;
                    }
                    rows.extend(flag_rows(&r, p, s).unwrap_or_else(|| row_of(r, p, s, f, &own.kinds).into_iter().collect()));
                }
                sections.push(section(section_title(sec.id), rows));
            }
            sections
        }
    };
    let summary = GROUPS.iter().find(|g| g.id == id).map_or("", |g| g.summary).to_string();
    Page { title, summary, sections }
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
        toggle(Switch::Mouse, "Mouse", "Off, the terminal selects text", o.mouse, true),
        toggle(Switch::Images, "Covers", "In the full player", o.images, true),
        toggle(Switch::Transparent, "Transparent background", "", !o.paint, true),
        Row::Choice { target: Target::Device, title: "Output device".into(), options: devices, shown, enabled: true, chips: false },
        Row::Note { text: "The device is opened at start; --device overrides it for one run.".into() },
    ];
    vec![section("", rows)]
}

/// Settings › Plugins: the way to the browser, updates waiting, the installed plugins (each opening its
/// page) and the switch over them all.
fn plugins_page(own: &Own, all_off: bool) -> Vec<Section> {
    let cards = &own.plugins;
    let mut sections = vec![section("", vec![Row::Link { title: text::GET_PLUGINS.into(), status: String::new(), action: Act::Page(PluginPage::Browser) }])];
    if cards.is_empty() {
        sections.push(section("", vec![Row::Note { text: text::NO_PLUGINS.into() }]));
        return sections;
    }
    sections[0].rows.push(Row::Toggle { switch: Switch::AllOff, title: text::ALL_PLUGINS.into(), detail: String::new(), on: !all_off, enabled: true });
    let updates: Vec<Row> = nori_plugins::updates_waiting(own.offers(), cards)
        .into_iter()
        .filter_map(|o| Some(Row::Offer { id: o.id.clone(), title: o.name.clone(), detail: text::update_line(o.installed.as_deref()?, &o.versions.first()?.version), status: text::UPDATE.into() }))
        .collect();
    if !updates.is_empty() {
        sections.push(section(text::UPDATES, updates));
    }
    let installed = cards.iter().map(|c| {
        let mut detail: Vec<String> = c.about.iter().cloned().collect();
        detail.push(c.version.clone());
        Row::Plugin { id: c.id.clone(), title: c.name.clone(), detail: detail.join(" · "), on: c.enabled, enabled: !all_off }
    });
    sections.push(section(text::INSTALLED_LIST, installed.collect()));
    sections
}

/// What the repos offer: searched, narrowed to one repo when there are several, each opening its page.
fn browser(own: &Own) -> Vec<Section> {
    let mut tools = vec![Row::Text { title: text::SEARCH_PLUGINS.into(), shown: own.query.clone(), secret: false, target: Target::PluginSearch }];
    if own.repos.len() > 1 {
        let mut options = vec![(text::ALL.to_string(), String::new())];
        options.extend(own.repos.iter().map(|r| (text::repo_name(r), r.url.clone())));
        let shown = options.iter().find(|o| own.repo_shown.as_ref().is_some_and(|u| *u == o.1)).unwrap_or(&options[0]).0.clone();
        tools.push(Row::Choice { target: Target::PluginRepo, title: text::REPOS.into(), chips: options.len() <= 4, options, shown, enabled: true });
    }
    tools.push(Row::Link { title: text::REPOS.into(), status: own.repos.len().to_string(), action: Act::Page(PluginPage::Repos) });
    tools.push(Row::Button { title: text::INSTALL_FROM_FILE.into(), action: Act::InstallPlugin });
    let mut rows = Vec::new();
    match &own.shelf {
        Shelf::Unasked | Shelf::Asking => rows.push(Row::Note { text: text::LOOKING.into() }),
        Shelf::Read(Offers { offers, failed }) => {
            rows.extend(failed.iter().map(|f| Row::Note { text: text::repo_failed(&f.repo) }));
            let found = nori_plugins::offers_found(offers.clone(), own.query.clone(), own.repo_shown.clone());
            if found.is_empty() {
                rows.push(Row::Note { text: text::NONE_FOUND.into() });
            }
            rows.extend(found.iter().filter_map(|o| {
                let mut detail: Vec<String> = o.about.iter().cloned().collect();
                detail.push(o.versions.first()?.version.clone());
                detail.push(text::repo_name(&o.repo));
                let status = match o.state {
                    OfferState::New => text::INSTALL,
                    OfferState::Update => text::UPDATE,
                    OfferState::Installed => text::INSTALLED,
                };
                Some(Row::Offer { id: o.id.clone(), title: o.name.clone(), detail: detail.join(" · "), status: status.into() })
            }));
            rows.push(Row::Button { title: text::LOOK_AGAIN.into(), action: Act::LookForPlugins });
        }
    }
    vec![section("", tools), section(text::OFFERED, rows)]
}

/// The repos plugins come from and what each offers; adding one comes first, as it stands at Android's top.
fn repos_page(own: &Own) -> Vec<Section> {
    let mut rows = vec![Row::Button { title: text::ADD_REPO.into(), action: Act::AddRepo }];
    rows.extend(own.repos.iter().map(|r| {
        let line = match &own.shelf {
            Shelf::Read(o) if o.failed.iter().any(|f| f.repo.url == r.url) => text::repo_failed(r),
            Shelf::Read(o) => text::plugin_count(o.offers.iter().filter(|x| x.repo.url == r.url).count()),
            _ => text::LOOKING.into(),
        };
        let detail = if r.official { line } else { format!("{} · {line}", r.url) };
        Row::Repo { url: r.url.clone(), title: text::repo_name(r), detail, removable: !r.official }
    }));
    vec![section("", rows)]
}

/// Lines of `said` wrapped to the page, as rows.
fn prose(said: &str, wrap: usize) -> Vec<Row> {
    text::wrapped(said, wrap).into_iter().map(|text| Row::Prose { text, quiet: false }).collect()
}

/// A plugin's README as sections: each heading opens one.
fn readme_sections(blocks: &[ReadmeBlock], wrap: usize) -> Vec<Section> {
    let mut sections = vec![section(text::ABOUT, Vec::new())];
    for b in blocks {
        let rows = &mut sections.last_mut().expect("one to begin").rows;
        match b {
            ReadmeBlock::Heading { text, .. } => sections.push(section(text, Vec::new())),
            ReadmeBlock::Text { text } => rows.extend(prose(text, wrap)),
            ReadmeBlock::Item { text } => {
                let lines = text::wrapped(text, wrap.saturating_sub(2));
                rows.extend(lines.into_iter().enumerate().map(|(i, l)| Row::Prose { text: format!("{}{l}", if i == 0 { "• " } else { "  " }), quiet: false }));
            }
            ReadmeBlock::Code { text } => rows.extend(text.lines().map(|l| Row::Prose { text: l.to_string(), quiet: true })),
        }
    }
    sections.retain(|s| !s.rows.is_empty());
    sections
}

/// A plugin's page, installed or only offered: what it is, the way to install or update it, its
/// switches and rows, its README, what it may do, its versions and source. `m` moves and removes it.
fn plugin_page(own: &Own, id: &str) -> Page {
    let card = own.plugins.iter().find(|c| c.id == id);
    let found = nori_plugins::offer_of(own.offers(), id, card);
    let offer = found.map(|(_, o)| o);
    let newest = offer.and_then(|o| o.versions.first());
    let title = card.map(|c| c.name.clone()).or_else(|| offer.map(|o| o.name.clone())).unwrap_or_else(|| id.to_string());
    let version = card.map(|c| c.version.clone()).or_else(|| newest.map(|v| v.version.clone()));
    let from = match (card, offer) {
        (Some(c), _) if c.repo.is_none() => Some(text::FROM_A_FILE.to_string()),
        (_, Some(o)) => Some(text::repo_name(&o.repo)),
        _ => None,
    };
    let summary = version.into_iter().chain(from).collect::<Vec<_>>().join(" · ");

    let mut head = Vec::new();
    if let Some(about) = card.and_then(|c| c.about.as_ref()).or(offer.and_then(|o| o.about.as_ref())) {
        head.extend(prose(about, own.wrap));
    }
    if let Some(c) = card {
        let is = text::plugin_is(&c.points, !c.overrides.is_empty());
        if !is.is_empty() {
            head.extend(prose(&is, own.wrap));
        }
    }
    if let (Some((at, o)), Some(v)) = (found, newest) {
        let label = match (card, o.state) {
            (None, _) => Some(text::INSTALL.to_string()),
            (_, OfferState::Update) => Some(text::update_to(&v.version)),
            _ => None,
        };
        head.extend(label.map(|title| Row::Button { title, action: Act::GetPlugin { offer: at as u32, version: 0 } }));
    }
    let mut sections = vec![section("", head)];
    if let Some(c) = card {
        let mut rows = vec![Row::Toggle { switch: Switch::Turn(c.id.clone()), title: text::PLUGIN_ON.into(), detail: String::new(), on: c.enabled, enabled: true }];
        if c.repo.is_some() {
            rows.push(Row::Toggle { switch: Switch::Hold(c.id.clone()), title: text::KEEP_VERSION.into(), detail: String::new(), on: c.held, enabled: true });
        }
        sections.push(section("", rows));
        sections.extend(own.controlled.iter().filter(|d| d.card.id == c.id).map(|d| Section { title: d.name.clone(), rows: device_controls(d) }).filter(|s| !s.rows.is_empty()));
        let (mut title, mut rows) = (String::new(), Vec::new());
        for r in &c.rows {
            match r.kind {
                RowKind::Section => sections.push(Section { title: std::mem::replace(&mut title, r.label.clone()), rows: std::mem::take(&mut rows) }),
                _ => rows.extend(plugin_rows(&c.id, r, own.off.contains(&(c.id.clone(), r.id.clone())))),
            }
        }
        sections.push(Section { title, rows });
        if let Some(assets) = own.assets.get(&c.id) {
            let mut rows: Vec<Row> = assets.iter().map(|a| info(&a.name, text::asset_state(a))).collect();
            if assets.iter().any(|a| a.state == AssetState::WaitingForWifi) {
                rows.push(Row::Button { title: text::ASSETS_ON_MOBILE.into(), action: Act::AllowMobileAssets(c.id.clone()) });
            }
            sections.push(section(text::FILES, rows));
        }
    }
    sections.extend(readme_sections(&own.readme, own.wrap));
    if let Some(c) = card {
        let may = text::plugin_may(c);
        if !may.is_empty() {
            sections.push(section(text::PLUGIN_MAY, may.iter().flat_map(|l| prose(l, own.wrap)).collect()));
        }
    }
    if let Some((at, o)) = found {
        sections.push(versions(card, at, o));
    }
    sections.retain(|s| !s.rows.is_empty());
    Page { title, summary, sections }
}

/// The warning before adding the repo at `url`, which isn't nori's.
pub fn trust_repo(url: &str) -> Overlay {
    Overlay::Confirm { title: text::ADD_REPO.into(), lines: vec![text::TRUST_REPO.into(), url.into()], yes: text::ADD.into(), cmd: Box::new(Cmd::Plugin(PluginCmd::AddRepo(url.into()))) }
}

/// Offer `o`, at place `at` in those read: its versions to install one of, and where its code is.
fn versions(card: Option<&PluginCard>, at: usize, o: &Offer) -> Section {
    let mut rows: Vec<Row> = o
        .versions
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let here = card.is_some_and(|c| c.version == v.version);
            Row::Action {
                title: v.version.clone(),
                detail: v.released.clone().unwrap_or_default(),
                button: (if here { text::INSTALLED } else { text::INSTALL }).into(),
                enabled: !here,
                action: Act::GetPlugin { offer: at as u32, version: i as u32 },
            }
        })
        .collect();
    rows.extend(o.source.iter().map(|url| info(text::SOURCE, url.clone())));
    section(text::VERSIONS, rows)
}

/// The switch over every plugin set to `off`, then the plugins told and read again.
fn all_off(off: bool) -> Vec<Cmd> {
    vec![Cmd::Setting("pluginsOff".into(), off.to_string()), Cmd::Plugin(PluginCmd::Switched)]
}

/// Enter on `row`.
pub fn open_row(row: Row) -> Option<Opened> {
    Some(match row {
        Row::Toggle { switch: Switch::Setting(name), on, enabled, .. } => {
            if !enabled {
                return None;
            }
            Opened::Cmds(vec![Cmd::Setting(name, (!on).to_string())])
        }
        Row::Toggle { switch: Switch::Stores(name, value), .. } => Opened::Cmds(vec![Cmd::Setting(name, value)]),
        Row::Toggle { switch: Switch::Plugin { plugin, row }, on, .. } => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::Set(plugin, row, Value::Switch(!on)))]),
        Row::Toggle { switch: Switch::Turn(plugin), on, .. } => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::Turn(plugin, !on))]),
        Row::Toggle { switch: Switch::Hold(plugin), on, .. } => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::Hold(plugin, !on))]),
        Row::Toggle { switch: Switch::AllOff, on, .. } => Opened::Cmds(all_off(on)),
        Row::Toggle { switch, .. } => Opened::Own(switch),
        Row::Choice { target, title, options, shown, enabled: true, .. } => {
            let at = options.iter().position(|o| o.0 == shown).unwrap_or(0);
            Opened::Overlay(Overlay::Picker { title, options, sel: Sel { at, ..Sel::default() }, target })
        }
        Row::Link { action, .. } | Row::Action { action, enabled: true, .. } | Row::Button { action, .. } => match action {
            Act::Equalizer => Opened::View(View::Equalizer),
            Act::Downloads => Opened::View(View::Downloads),
            Act::AddServer => Opened::Login,
            Act::JoinJam => Opened::Overlay(Overlay::Join { text: String::new(), error: None, busy: false }),
            Act::Chore(c) => Opened::Cmds(vec![Cmd::Action(c)]),
            Act::InstallPlugin => Opened::Overlay(Overlay::Input { title: text::PLUGIN_PATH.into(), text: String::new(), secret: false, target: Target::Install }),
            Act::PluginAction { plugin, row } => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::Act(plugin, row))]),
            Act::LookForPlugins => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::Look)]),
            Act::Page(page) => Opened::Plugins(page),
            Act::GetPlugin { offer, version } => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::Get(offer, version))]),
            Act::AddRepo => Opened::Overlay(Overlay::Input { title: text::REPO_ADDRESS.into(), text: String::new(), secret: false, target: Target::AddRepo }),
            Act::AllowMobileAssets(id) => Opened::Cmds(vec![Cmd::Plugin(PluginCmd::AllowMobileAssets(id))]),
            Act::Backup { restore } => Opened::Overlay(Overlay::Input { title: text::backup_path(restore).into(), text: String::new(), secret: false, target: Target::BackupFile { restore } }),
        },
        Row::Text { title, shown, target: Target::PluginSearch, .. } => Opened::Overlay(Overlay::Input { title, text: shown, secret: false, target: Target::PluginSearch }),
        Row::Text { title, secret, target, .. } => Opened::Overlay(Overlay::Input { title, text: String::new(), secret, target }),
        Row::Plugin { id, .. } | Row::Offer { id, .. } => Opened::Plugins(PluginPage::Plugin(id)),
        // The profile in use: its account, where its provider signs in to one.
        Row::Server { active: true, .. } => Opened::Cmds(vec![Cmd::OfferSignIn]),
        Row::Server { id, .. } => Opened::Cmds(vec![Cmd::SwitchServer(id)]),
        Row::Ranked { enabled: false, .. } => return None,
        Row::Ranked { id, title, asks_key: true, on: false, .. } => key_input(&id, &title),
        Row::Ranked { name, on, .. } => Opened::Cmds(vec![Cmd::Setting(name, (!on).to_string())]),
        // Picking the season worn while following stops following it.
        Row::Season { value, picked: false, enabled: true, .. } => Opened::Cmds(vec![Cmd::Setting("season".into(), value)]),
        _ => return None,
    })
}

/// ← or → on `row`: previous or next option, off or on, a slider step.
pub fn step_row(row: Row, up: bool) -> Vec<Cmd> {
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
                Switch::Plugin { plugin, row } => vec![Cmd::Plugin(PluginCmd::Set(plugin, row, Value::Switch(up)))],
                Switch::Turn(plugin) => vec![Cmd::Plugin(PluginCmd::Turn(plugin, up))],
                Switch::Hold(plugin) => vec![Cmd::Plugin(PluginCmd::Hold(plugin, up))],
                Switch::AllOff => all_off(!up),
                Switch::Mouse => vec![Cmd::Mouse(up)],
                Switch::Images => vec![Cmd::Images(up)],
                Switch::Transparent => vec![Cmd::Paint(!up)],
            }
        }
        Row::Slider { target, value, min, max, step, .. } => {
            let v = (value + if up { step } else { -step }).clamp(min, max);
            target.cmd(v.to_string()).into_iter().collect()
        }
        Row::Plugin { id, on, .. } if on != up => vec![Cmd::Plugin(PluginCmd::Turn(id, up))],
        Row::Ranked { id, enabled: true, .. } => vec![Cmd::Setting("lyricsMove".into(), format!("{id}:{}", if up { 1 } else { -1 }))],
        _ => Vec::new(),
    }
}

/// One row of plugin `plugin`'s page, then its hint; `off`, it shows but takes no input.
fn plugin_rows(plugin: &str, r: &PluginRow, off: bool) -> Vec<Row> {
    let hint = r.hint.clone().map(|text| Row::Note { text });
    let row = if off { Row::Info { title: r.label.clone(), detail: shown_value(r) } } else { plugin_row(plugin, r) };
    std::iter::once(row).chain(hint).collect()
}

/// What row `r` says of its value, as a line.
fn shown_value(r: &PluginRow) -> String {
    r.value.as_ref().map(|v| text::plugin_value(v, r)).unwrap_or_default()
}

/// A controlled device's batteries and noise control.
fn device_controls(d: &ControlledDevice) -> Vec<Row> {
    let mut rows = Vec::new();
    let battery = text::battery(&d.battery);
    if !battery.is_empty() {
        rows.push(info(text::BATTERY, battery));
    }
    if !d.noise_modes.is_empty() {
        let options: Vec<(String, String)> = d.noise_modes.iter().enumerate().map(|(i, m)| (text::noise(*m).to_string(), i.to_string())).collect();
        let shown = d.noise.map(|n| text::noise(n).to_string()).unwrap_or_default();
        let target = Target::Noise { device: d.id.clone(), modes: d.noise_modes.clone() };
        rows.push(Row::Choice { target, title: text::NOISE_CONTROL.into(), chips: options.len() <= 4, options, shown, enabled: true });
    }
    rows
}

/// One row of plugin `plugin`'s page, drawn as the settings' own rows are.
pub fn plugin_row(plugin: &str, r: &PluginRow) -> Row {
    let target = Target::Plugin { plugin: plugin.into(), row: r.id.clone(), kind: r.kind };
    let title = r.label.clone();
    match r.kind {
        RowKind::Switch => Row::Toggle { switch: Switch::Plugin { plugin: plugin.into(), row: r.id.clone() }, title, detail: String::new(), on: r.value == Some(Value::Switch(true)), enabled: true },
        RowKind::Choice => {
            let options: Vec<(String, String)> = r.options.iter().enumerate().map(|(i, o)| (o.clone(), i.to_string())).collect();
            let at = match r.value {
                Some(Value::Choice(i)) => i as usize,
                _ => 0,
            };
            let shown = options.get(at).map(|o| o.0.clone()).unwrap_or_default();
            Row::Choice { target, title, chips: options.len() <= 4, options, shown, enabled: true }
        }
        RowKind::Level => {
            let (min, max) = (r.min.unwrap_or(0.0) as f32, r.max.unwrap_or(1.0) as f32);
            let step = r.step.map_or_else(|| slider_step(min, max), |s| s as f32);
            let value = match r.value {
                Some(Value::Level(v)) => v,
                _ => min as f64,
            };
            Row::Slider { target, label: format!("{title}: {}", text::level(value, r.decimals, r.unit.as_deref())), value: value as f32, min, max, step, centred: min < 0.0 && max > 0.0 }
        }
        RowKind::Text | RowKind::Secret => {
            let typed = match &r.value {
                Some(Value::Text(t)) if !t.is_empty() => Some(t.clone()),
                _ => None,
            };
            let secret = r.kind == RowKind::Secret;
            let shown = match typed {
                Some(_) if secret => text::SECRET_SET.into(),
                Some(t) => t,
                None => text::NOT_SET.into(),
            };
            Row::Text { title, shown, secret, target }
        }
        RowKind::Action => Row::Button { title, action: Act::PluginAction { plugin: plugin.into(), row: r.id.clone() } },
        RowKind::Status => Row::Info { title, detail: shown_value(r) },
        // A link shows where it goes: the terminal opens no pages.
        RowKind::Link => Row::Info { title, detail: r.url.clone().unwrap_or_default() },
        RowKind::Note | RowKind::Section => Row::Note { text: title },
    }
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
        MusicFolders | ThisServer => "This server",
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

/// The title over the seasons.
const SEASON: &str = "Season";

/// A setting's title and the line under it, which says only how things stand now.
fn setting_words(name: &str, p: &StoredPrefs, s: &SettingsState) -> (String, String) {
    let title = match name {
        "crossfadeSec" => "Crossfade",
        "autoMix" => "AutoMix",
        "autoMixBetterBeats" => {
            let mb = s.beat_model_mb;
            let state = match &s.beat_model {
                _ if !p.auto_mix_better_beats => format!("A download of about {mb} MB"),
                BeatModel::Failed { why } => format!("The download didn't work ({}). It tries again at the next song.", text::beat_failure(*why)),
                BeatModel::Downloading => format!("Downloading (about {mb} MB)…"),
                BeatModel::Ready => String::new(),
                _ => format!("A download of about {mb} MB"),
            };
            return ("Better beat detection".into(), state);
        }
        "crossfadeKeepAlbums" => "Keep albums gapless",
        "skipExplicit" => "Skip explicit songs",
        "autoFill" => "Keep playing when the queue ends",
        "autoFillKind" => "Carry on with",
        "autoFillBasis" => "Chosen by",
        "autoFillRemote" => "Include remote songs",
        "bridgeOffline" => "Play downloads when offline",
        "headphoneSound" => "Headphone sound",
        "compressor" => "Compressor",
        "compressorPreset" => "Compressor type",
        "replayGain" => "Volume leveling",
        "loudnessLevel" => "Level",
        "volumeBoostDb" => "Volume boost",
        "hiRes" => "High quality output",
        "maxRate" => "Highest sample rate",
        "season" => SEASON,
        "theme" => "Appearance",
        "lyricsSweep" => "Fill in words as they're sung",
        "lyricsTranslation" => "Show translations",
        "lyricsOnline" => "Find missing lyrics online",
        "lyricsPreferWords" => "Prefer word-by-word lyrics",
        "playlistNotes" => "Playlist notes",
        "tasteModel" => "Keep listening history",
        "scrobble" => "Tell the server what you play",
        "queueOffer" => "Continue from other devices",
        "thirdPartyLookups" => "Look things up online",
        "remoteControl" => "Remote control and jams",
        // A computer streams on one network: its quality and its songs ahead without the phone's "on Wi-Fi".
        "wifi" => "Streaming quality",
        "songsAheadWifi" => "Prepare songs ahead",
        "download" => "Quality for downloads",
        "cacheMb" => "Space for streamed music",
        other => other,
    };
    (title.to_string(), String::new())
}

/// Asks for lyrics service `id`'s key (empty: none).
fn key_input(id: &str, title: &str) -> Opened {
    Opened::Overlay(Overlay::Input { title: format!("{title} key (empty: none)"), text: String::new(), secret: true, target: Target::Setting(format!("lyricsKey:{id}")) })
}

/// A setting value in words.
fn option_words(name: &str, v: &str) -> String {
    let off_or = |v: &str, words: &dyn Fn(&str) -> String| if v == "0" { "Off".to_string() } else { words(v) };
    match (name, v) {
        ("crossfadeSec", v) => off_or(v, &|v| format!("{v} s")),
        ("season", "FOLLOW") => "Follow the calendar".into(),
        ("season", v) => TILES.iter().find(|t| t.2 == v).map_or(v, |t| t.0).into(),
        ("theme", "SYSTEM") => "System".into(),
        ("theme", "LIGHT") => "Day".into(),
        ("theme", "DARK") => "Night".into(),
        ("theme", "MIDNIGHT") => "Midnight".into(),
        ("autoFillKind", "SONGS") => "Songs".into(),
        ("autoFillKind", "ALBUMS") => "Albums".into(),
        ("autoFillBasis", "SIMILAR") => "Similar music".into(),
        ("autoFillBasis", "ARTIST") => "The same artist".into(),
        ("autoFillBasis", "GENRE") => "The same genre".into(),
        ("autoFillBasis", "ERA") => "The same era".into(),
        ("headphoneSound", "OFF") | ("replayGain", "OFF") => "Off".into(),
        ("headphoneSound", "NATURAL") => "Natural".into(),
        ("headphoneSound", "WIDE") => "Wide".into(),
        ("compressorPreset", "LIGHT") => "Light".into(),
        ("compressorPreset", "CAR") => "Car".into(),
        ("compressorPreset", "NIGHT") => "Night".into(),
        ("replayGain", "TRACK") => "Per song".into(),
        ("replayGain", "ALBUM") => "Per album".into(),
        ("replayGain", "AUTO") => "Automatic".into(),
        ("loudnessLevel", "QUIET") => "Quiet".into(),
        ("loudnessLevel", "NORMAL") => "Normal".into(),
        ("loudnessLevel", "LOUD") => "Loud".into(),
        ("maxRate", "AUTO") => "Each song's own".into(),
        ("maxRate", "KHZ48") => "48 kHz".into(),
        ("maxRate", "KHZ96") => "96 kHz".into(),
        ("maxRate", "KHZ192") => "192 kHz".into(),
        ("wifi" | "download", v) => quality(v),
        ("songsAheadWifi", "1") => "Next song".into(),
        ("songsAheadWifi", v) => format!("{v} songs"),
        ("cacheMb", v) => match v.parse::<i32>() {
            Ok(mb) if mb >= 1024 && mb % 1024 == 0 => format!("{} GB", mb / 1024),
            _ => format!("{v} MB"),
        },
        (_, v) => v.to_string(),
    }
}

/// The four seasons as their names, kanji and the setting's values.
const TILES: [(&str, &str, &str, ThemeSeason); 4] = [
    ("Spring", "春", "SPRING", ThemeSeason::Spring),
    ("Summer", "夏", "SUMMER", ThemeSeason::Summer),
    ("Autumn", "秋", "AUTUMN", ThemeSeason::Autumn),
    ("Winter", "冬", "WINTER", ThemeSeason::Winter),
];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The season setting as the four seasons, the one worn marked, each with its theme's accent in the mode
/// the interface is in (`dark`), then the switch that follows the calendar, saying when the next comes in.
fn season_rows(r: &settings_pages::SettingsRow, p: &StoredPrefs, dark: bool, today: Day) -> Option<Vec<Row>> {
    let RowItem::Setting { spec, .. } = &r.item else { return None };
    if spec.name != "season" {
        return None;
    }
    let now = nori_look::theme::season_now(today.month, today.day, today.southern);
    let following = p.season == ThemeSeason::Follow;
    let worn = nori_core::settings::season_of(p.season).unwrap_or(now.season);
    let mut rows: Vec<Row> = TILES
        .iter()
        .map(|&(name, kanji, value, season)| {
            let accent = nori_core::settings::season_palette(season, p.theme, dark, today.month, today.day, today.southern).accent;
            let is = nori_core::settings::season_of(season) == Some(worn);
            Row::Season { value: value.into(), name, kanji, accent, worn: is, picked: p.season == season, enabled: r.enabled }
        })
        .collect();
    let next = TILES[now.next as usize].0;
    let detail = if following { format!("{next} comes in on {} {}", MONTHS[now.next_month as usize - 1], now.next_day) } else { String::new() };
    let to = season_follow_value(!following, today.month, today.day, today.southern);
    rows.push(Row::Toggle { switch: Switch::Stores(spec.name.clone(), to.into()), title: option_words("season", "FOLLOW"), detail, on: following, enabled: r.enabled });
    Some(rows)
}

/// A set of independent flags as its title, then a switch for each; None for any other row.
fn flag_rows(r: &settings_pages::SettingsRow, p: &StoredPrefs, s: &SettingsState) -> Option<Vec<Row>> {
    let RowItem::Setting { spec, value, .. } = &r.item else { return None };
    if spec.kind != SettingKind::Flags {
        return None;
    }
    let (title, detail) = setting_words(&spec.name, p, s);
    let switches = spec.options.iter().map(|flag| Row::Toggle {
        switch: Switch::Stores(spec.name.clone(), settings_model::flag_toggled(value.clone(), flag.clone())),
        title: format!("  {}", option_words(&spec.name, flag)),
        detail: String::new(),
        on: settings_model::flag_on(value.clone(), flag.clone()),
        enabled: r.enabled,
    });
    Some(std::iter::once(Row::Info { title, detail }).chain(switches).collect())
}

/// "12 songs", "1 song".
fn counted(n: u32, one: &str, many: &str) -> String {
    text::count(n as u64, one, many)
}

/// The terminal's row for one of the core's, or None for one it does not draw.
fn row_of(r: settings_pages::SettingsRow, p: &StoredPrefs, s: &SettingsState, f: &Facts, kinds: &Kinds) -> Option<Row> {
    let indent = if r.depth > 0 { "  " } else { "" };
    let enabled = r.enabled;
    Some(match r.item {
        RowItem::Setting { spec, value, chips } => {
            let (title, detail) = setting_words(&spec.name, p, s);
            let title = format!("{indent}{title}");
            match spec.kind {
                SettingKind::Switch => Row::Toggle { switch: Switch::Setting(spec.name), title, detail, on: value == "true", enabled },
                SettingKind::Choice => {
                    let options: Vec<(String, String)> = spec.options.iter().map(|v| (option_words(&spec.name, v), v.clone())).collect();
                    let shown = options.iter().find(|o| o.1 == value).map_or_else(|| value.clone(), |o| o.0.clone());
                    Row::Choice { target: Target::Setting(spec.name), title, options, shown, enabled, chips }
                }
                SettingKind::Level => {
                    let v: f32 = value.parse().unwrap_or(spec.min);
                    let target = if spec.name == "volumeBoostDb" { Target::Level(EqLevel::VolumeBoost) } else { Target::Setting(spec.name) };
                    let label = format!("{title}: {}", if v > 0.0 { format!("{} dB", numbers::signed_db(v, POINT)) } else { "Off".into() });
                    Row::Slider { target, label, value: v, min: spec.min, max: spec.max, step: slider_step(spec.min, spec.max), centred: false }
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
            Row::Link { title: "Equalizer and crossfeed".into(), status: status.into(), action: Act::Equalizer }
        }
        RowItem::Link { to: SettingsLink::Downloads } => {
            Row::Action { title: "Downloads".into(), detail: "Remove them from the Downloads page.".into(), button: "Show".into(), enabled: true, action: Act::Downloads }
        }
        RowItem::Link { .. } | RowItem::CompressionMeter => return None,
        RowItem::Action { action } => {
            let (indexed_songs, albums, artists) = f.indexed;
            let chore = |title: &str, detail: String, button: &str, chore| Row::Action { title: format!("{indent}{title}"), detail, button: button.into(), enabled, action: Act::Chore(chore) };
            match action {
                SettingsAction::MeasureAgain => chore("Measured songs", format!("{} measured for tempo and beats.", counted(f.analysed, "song", "songs")), "Measure again", Chore::MeasureAgain),
                SettingsAction::SyncLibrary => {
                    let counts = format!("{} · {} · {} on this device", counted(indexed_songs, "song", "songs"), counted(albums, "album", "albums"), counted(artists, "artist", "artists"));
                    chore("Offline search", counts, "Update", Chore::SyncLibrary)
                }
                SettingsAction::DownloadLibrary => chore("Download the whole library", "Every song, at the download quality.".into(), "Download", Chore::DownloadLibrary),
                SettingsAction::ClearStream => chore("Streamed music", "Oldest goes first. Downloads stay.".into(), "Clear", Chore::ClearStream),
                SettingsAction::ClearCovers => chore("Covers", "Fetched again when needed.".into(), "Clear", Chore::ClearCovers),
                SettingsAction::ClearLyrics => chore("Lyrics", format!("{} found online. Looked up again when needed.", numbers::bytes(f.storage.lyrics, POINT)), "Clear", Chore::ClearLyrics),
                SettingsAction::AddServer => Row::Button { title: "Add server".into(), action: Act::AddServer },
                SettingsAction::JoinJam => Row::Button { title: text::JAM_JOIN.into(), action: Act::JoinJam },
                SettingsAction::MakeBackup => Row::Action { title: text::BACK_UP.into(), detail: text::BACK_UP_DETAIL.into(), button: "Save".into(), enabled, action: Act::Backup { restore: false } },
                SettingsAction::RestoreBackup => Row::Action { title: text::RESTORE.into(), detail: text::RESTORE_DETAIL.into(), button: "Open".into(), enabled, action: Act::Backup { restore: true } },
                SettingsAction::AnalyseDownloads | SettingsAction::CheckUpdate => return None,
            }
        }
        RowItem::Note { note } => Row::Note {
            text: match note {
                SettingsNote::HeldByDac => "Off in bit-perfect mode.",
                SettingsNote::LyricsSources if p.lyrics_online => "Your server's lyrics always come first. Among these, the best match wins; the order breaks ties.",
                SettingsNote::LyricsSources => "Off while Find missing lyrics online is off.",
                SettingsNote::Dac => return None,
            }
            .into(),
        },
        RowItem::Stored => {
            let st = &f.storage;
            let total = st.stream + st.downloads + st.covers + st.lyrics + st.database;
            let parts = [
                format!("Streamed music {}", numbers::bytes(st.stream, POINT)),
                format!("{} {}", counted(st.all_download_songs, "download", "downloads"), numbers::bytes(st.downloads, POINT)),
                format!("Covers {}", numbers::bytes(st.covers, POINT)),
                format!("Lyrics {}", numbers::bytes(st.lyrics, POINT)),
                format!("Library index {}", numbers::bytes(st.database, POINT)),
            ];
            let mut detail = parts.join(" · ");
            // The downloads by profile, when more than one has any.
            if st.download_profiles.len() > 1 {
                let each: Vec<String> = st.download_profiles.iter().map(|(profile, b)| format!("{} {}", profile_name(p, kinds, profile), numbers::bytes(*b, POINT))).collect();
                detail.push_str(&format!(" (downloads: {})", each.join(", ")));
            }
            info(&format!("Stored on this device  {}", numbers::bytes(total, POINT)), detail)
        }
        RowItem::Unsent { waiting, dropped } => {
            let parts = [(waiting, "waiting for the server"), (dropped, "the server would not take")];
            info("Changes not sent", parts.iter().filter(|p| p.0 > 0).map(|(n, what)| format!("{n} {what}")).collect::<Vec<_>>().join(" · "))
        }
        RowItem::Server { index } => {
            let sv = p.servers.get(index as usize)?;
            let active = sv.id == p.active_server_id;
            let words = text::provider_of(kinds, &sv.kind);
            let who = if sv.user.is_empty() { words.no_user } else { sv.user.as_str() };
            let mut parts = vec![who, if active { "in use" } else { "not in use" }];
            if sv.wifi_only {
                parts.push("Wi-Fi only");
            }
            if !sv.alt_url.trim().is_empty() {
                parts.push("second address");
            }
            let label = Some(nori_core::settings::label(&sv.name, &sv.url)).filter(|l| !l.is_empty()).unwrap_or(words.name);
            Row::Server { id: sv.id.clone(), label, detail: parts.join(" · "), active }
        }
        RowItem::MusicFolder { index, on, tapped } => {
            Row::Toggle { switch: Switch::Stores("musicFolders".into(), tapped), title: f.folders.get(index as usize)?.name.clone(), detail: String::new(), on, enabled }
        }
        RowItem::LyricsSource { index } => {
            let LyricsSource { id, on, key_set, key, asks_key_on_enable, .. } = s.lyrics_sources.get(index as usize)?;
            let (title, about) = service(id);
            let detail = if *key_set { format!("{about} Key set.") } else { about.to_string() };
            Row::Ranked { name: format!("lyricsService:{id}"), id: id.clone(), title: title.into(), detail, on: *on, asks_key: *asks_key_on_enable, key: *key, enabled: r.enabled }
        }
    })
}

/// A lyrics service's name and what it is good at, by its core id.
fn service(id: &str) -> (&'static str, &'static str) {
    match id {
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
        _ => ("?", ""),
    }
}

/// A stream quality value ("320:mp3") in words.
fn quality(v: &str) -> String {
    match v.split_once(':') {
        Some((_, "")) | None => "Original".into(),
        Some((rate, "mp3")) => format!("MP3 {rate}"),
        Some((rate, "opus")) => format!("Opus {rate}"),
        Some((rate, format)) => format!("{rate} {format}"),
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
        Row::Offer { status, .. } => format!("{status} ›"),
        Row::Plugin { on, .. } => format!("{} ›", text::on_off(*on)),
        Row::Button { title, .. } => format!("[ {title} ]"),
        Row::Text { shown, .. } => format!("{shown} ›"),
        _ => String::new(),
    }
}

/// A row's title and detail.
pub fn row_words(row: &Row) -> (String, String) {
    match row {
        Row::Toggle { title, detail, .. } | Row::Ranked { title, detail, .. } | Row::Info { title, detail, .. } => (title.clone(), detail.clone()),
        Row::Choice { title, .. } | Row::Link { title, .. } | Row::Text { title, .. } => (title.clone(), String::new()),
        Row::Plugin { title, detail, .. } | Row::Repo { title, detail, .. } | Row::Offer { title, detail, .. } => (title.clone(), detail.clone()),
        Row::Prose { .. } => (String::new(), String::new()),
        Row::Note { text } => (String::new(), text.clone()),
        Row::Action { title, detail, .. } => (title.clone(), detail.clone()),
        Row::Slider { label, .. } => (label.clone(), String::new()),
        Row::Server { label, detail, .. } => (label.clone(), detail.clone()),
        Row::Season { name, kanji, .. } => (format!("{kanji}  {name}"), String::new()),
        Row::Button { .. } => (String::new(), String::new()),
    }
}

/// Whether the row is live; inactive settings are drawn dimmed.
pub fn row_enabled(row: &Row) -> bool {
    match row {
        Row::Toggle { enabled, .. } | Row::Choice { enabled, .. } | Row::Action { enabled, .. } | Row::Ranked { enabled, .. } | Row::Season { enabled, .. } | Row::Plugin { enabled, .. } => *enabled,
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

/// A parametric band's frequency step for [ and ], on the band scale (a sixth of an octave).
const FREQ_STEP: f32 = 1.0 / 60.0;
/// A parametric band's Q step for { and }.
const Q_STEP: f32 = 1.12;
/// The band kinds there are (`EqKind`).
const KINDS: i32 = 10;

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
        let on = |b: bool| (if b { "on" } else { "off" }).to_string();
        match *self {
            EqRow::Enabled => ("Equalizer".into(), on(p.eq_enabled)),
            EqRow::Mode => ("Kind".into(), if p.eq_mode == EqMode::Graphic { "graphic" } else { "parametric" }.into()),
            EqRow::Layout => ("Bands".into(), p.eq_graphic.len().to_string()),
            EqRow::Slider(i) => {
                let label = nori_core::dsp::graphic_bands(p.eq_graphic.len() as u32).get(i).map_or(0.0, |b| b.label_hz);
                (numbers::iso_band(label, POINT), format!("{} dB", numbers::signed_db(p.eq_graphic.get(i).copied().unwrap_or(0.0), POINT)))
            }
            EqRow::Presets => ("Presets".into(), String::new()),
            EqRow::AutoPreamp => ("Automatic pre-amp".into(), on(p.eq_preamp_db.is_none())),
            EqRow::Preamp => ("Pre-amp".into(), text::preamp(p.eq_preamp_db.unwrap_or(0.0), false)),
            EqRow::Band(i) => {
                let b = p.eq_bands.get(i).copied().unwrap_or(nori_core::settings::band_from(0, 0.0, 0.0, 1.0, 0));
                (text::band(b.freq, nori_core::settings::band_mark(b.kind as i32, b.channel as i32)), format!("{} dB", numbers::signed_db(b.gain_db, POINT)))
            }
            EqRow::AddBand => ("Add a band".into(), String::new()),
            EqRow::Reset => ("Back to flat".into(), String::new()),
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

    /// Its point dragged to `pos` across the curve and `db` (rounded to 0.5 dB): a graphic band only up
    /// or down; None if unchanged.
    pub fn place(&self, p: &StoredPrefs, pos: f32, db: f32) -> Option<Cmd> {
        let r = EQ_RANGES.gain;
        let db = ((db * 2.0).round() / 2.0).clamp(r.min, r.max);
        match *self {
            EqRow::Slider(i) => (p.eq_graphic.get(i).copied()? != db).then_some(Cmd::Graphic(i as u32, db)),
            EqRow::Band(i) => {
                let b = *p.eq_bands.get(i)?;
                let gain_db = if b.kind.uses_gain() { db } else { b.gain_db };
                self.band_to(p, SoundBand { freq: nori_core::dsp::band_moved(b.freq, pos), gain_db, ..b })
            }
            _ => None,
        }
    }

    /// A parametric band's frequency a step up or down the band scale.
    pub fn shift(&self, p: &StoredPrefs, up: bool) -> Option<Cmd> {
        let b = self.parametric(p)?;
        let at = nori_core::dsp::band_position(b.freq) + if up { FREQ_STEP } else { -FREQ_STEP };
        self.band_to(p, SoundBand { freq: nori_core::dsp::band_freq_at(at), ..b })
    }

    /// A parametric band narrower (a higher Q or steeper slope) or wider.
    pub fn width(&self, p: &StoredPrefs, narrower: bool) -> Option<Cmd> {
        let b = self.parametric(p)?;
        let q = if narrower { b.q * Q_STEP } else { b.q / Q_STEP };
        self.band_to(p, SoundBand { q: ((q * 100.0).round() / 100.0).clamp(EQ_RANGES.q.min, EQ_RANGES.q.max), ..b })
    }

    /// A parametric band's next kind, or next side (both, left, right).
    pub fn cycle(&self, p: &StoredPrefs, side: bool) -> Option<Cmd> {
        let b = self.parametric(p)?;
        let (kind, channel) = if side { (b.kind as i32, (b.channel as i32 + 1) % 3) } else { ((b.kind as i32 + 1) % KINDS, b.channel as i32) };
        self.band_to(p, nori_core::settings::band_from(kind, b.freq, b.gain_db, b.q, channel))
    }

    fn parametric(&self, p: &StoredPrefs) -> Option<SoundBand> {
        match *self {
            EqRow::Band(i) => p.eq_bands.get(i).copied(),
            _ => None,
        }
    }

    /// Band `self` set to `to`; None if that is how it is.
    fn band_to(&self, p: &StoredPrefs, to: SoundBand) -> Option<Cmd> {
        let EqRow::Band(i) = *self else { return None };
        (p.eq_bands.get(i)? != &to).then_some(Cmd::Band(i as u32, to))
    }

    /// Whether a single click activates it.
    pub fn clicks(&self) -> bool {
        matches!(self, EqRow::Enabled | EqRow::Mode | EqRow::Presets | EqRow::AutoPreamp | EqRow::AddBand | EqRow::Reset | EqRow::Mono | EqRow::Limiter)
    }

    /// Chip label.
    pub fn chip(&self, p: &StoredPrefs) -> String {
        let (title, value) = self.words(p);
        match self {
            EqRow::Enabled => (if p.eq_enabled { "Equalizer on" } else { "Equalizer off" }).into(),
            EqRow::Presets => "Presets".into(),
            EqRow::AddBand => "Add a band".into(),
            EqRow::Reset => "Flat".into(),
            EqRow::Mode => (if p.eq_mode == EqMode::Graphic { "Graphic" } else { "Parametric" }).into(),
            EqRow::Layout => format!("{} bands", p.eq_graphic.len()),
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
            EqRow::AddBand => Some(Cmd::Sound(SoundToolCmd::AddBand(1000.0, 0.0))),
            EqRow::Reset => Some(Cmd::Sound(SoundToolCmd::ResetBands)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Day = Day { month: 10, day: 19, southern: false };

    #[test]
    fn the_look_picks_a_season_and_follows_the_calendar_under_it() {
        let look = |season, theme| {
            let prefs = StoredPrefs { season, theme, ..StoredPrefs::default() };
            let state = settings_model::state(&prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
            page(Gid::Core(SettingsPage::Look), &prefs, &state, &Facts::default(), &Own { paint: true, dark: true, ..Own::default() }, DAY).sections
        };
        // The seasons first, under their own title, each its value, whether it is worn and picked.
        let seasons = |sections: &[Section]| -> Vec<(String, bool, bool)> {
            assert_eq!(sections[0].title, SEASON);
            sections[0].rows.iter().filter_map(|r| match r {
                Row::Season { value, worn, picked, .. } => Some((value.clone(), *worn, *picked)),
                _ => None,
            }).collect()
        };
        let follow = |sections: &[Section]| match sections[0].rows.last() {
            Some(Row::Toggle { switch: Switch::Stores(name, value), title, on, detail, .. }) if name == "season" && title == "Follow the calendar" => (value.clone(), *on, detail.clone()),
            other => panic!("{other:?}"),
        };
        let following = look(ThemeSeason::Follow, nori_core::settings::ThemeMode::Dark);
        let s = |v: &str, worn, picked| (v.to_string(), worn, picked);
        assert_eq!(seasons(&following), [s("SPRING", false, false), s("SUMMER", false, false), s("AUTUMN", true, false), s("WINTER", false, false)], "the calendar's worn, none picked");
        assert_eq!(follow(&following), ("AUTUMN".into(), true, "Winter comes in on November 7".into()), "off keeps the calendar's");
        let winter = look(ThemeSeason::Winter, nori_core::settings::ThemeMode::Dark);
        assert_eq!(seasons(&winter)[3], s("WINTER", true, true));
        let (value, on, detail) = follow(&winter);
        assert_eq!((value.as_str(), on, detail.as_str()), ("FOLLOW", false, ""));
        // Each in its theme's accent, dark as the interface is.
        let accent = |sections: &[Section]| match &sections[0].rows[1] {
            Row::Season { accent, .. } => *accent,
            other => panic!("{other:?}"),
        };
        let summer = nori_look::theme::palette(Some(nori_look::theme::Season::Summer), DAY.month, DAY.day, false, true, false);
        assert_eq!(accent(&winter), summer.accent);
        // Picking a season stops following; the one picked asks nothing.
        let mut v = SettingsView { pages: Some(vec![Page { title: String::new(), summary: String::new(), sections: winter }]), ..SettingsView::default() };
        v.row.at = 2;
        assert!(matches!(v.open(), Some(Opened::Cmds(c)) if c == [Cmd::Setting("season".into(), "SPRING".into())]));
        v.row.at = 5;
        assert!(v.open().is_none());
        // Then the appearance, its four modes on view.
        let theme = &following[1];
        assert!(matches!(&theme.rows[..], [Row::Choice { title, options, chips: true, .. }] if title == "Appearance" && options.iter().map(|o| o.0.as_str()).collect::<Vec<_>>() == ["System", "Day", "Night", "Midnight"]), "{theme:?}");
    }

    /// A jam guest's settings are this computer's: the profiles to switch back with, sound, look and
    /// caches, but nothing of the account's or the host's server.
    #[test]
    fn a_guest_sees_only_this_computers_settings() {
        let prefs = StoredPrefs { servers: vec![nori_core::settings::SavedServer { id: "a".into(), ..Default::default() }], active_server_id: "a".into(), ..StoredPrefs::default() };
        let state = settings_model::state(&prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
        let titles = |rules: nori_core::browse::ProfileRules| -> Vec<String> {
            let facts = Facts { folders: vec![MusicFolder { id: "1".into(), name: "A".into() }, MusicFolder { id: "2".into(), name: "B".into() }], rules, ..Facts::default() };
            let pages = GROUPS.iter().map(|g| page(g.id, &prefs, &state, &facts, &Own::default(), DAY));
            pages.flat_map(|p| p.sections.into_iter().flat_map(|s| std::iter::once(s.title.clone()).chain(s.rows.iter().map(|r| row_words(r).0)).collect::<Vec<_>>())).collect()
        };
        let account = titles(nori_core::browse::profile_rules(false));
        let guest = titles(nori_core::browse::profile_rules(true));
        for kept in ["Accounts", "Streaming quality", "Streamed music", "Appearance"] {
            assert!(account.iter().any(|t| t == kept) && guest.iter().any(|t| t == kept), "{kept}");
        }
        for gone in ["Quality for downloads", "Downloads"] {
            assert!(account.iter().any(|t| t == gone) && !guest.iter().any(|t| t == gone), "{gone}");
        }
        assert!(guest.len() < account.len());
    }

    fn every_row(prefs: &StoredPrefs) -> Vec<(&'static str, Row)> {
        let facts = Facts { folders: vec![MusicFolder { id: "1".into(), name: "A".into() }, MusicFolder { id: "2".into(), name: "B".into() }], ..Facts::default() };
        every_row_with(prefs, &facts)
    }

    #[test]
    fn storage_counts_every_profiles_downloads_and_names_each() {
        let server = |id: &str, name: &str| nori_core::settings::SavedServer { id: id.into(), name: name.into(), ..Default::default() };
        let prefs = StoredPrefs { servers: vec![server("a", "Home"), server("b", "Away")], active_server_id: "a".into(), ..StoredPrefs::default() };
        let storage = Storage {
            downloads: 3_000_000,
            all_download_songs: 5,
            download_profiles: vec![("a".into(), 2_000_000), ("b".into(), 1_000_000)],
            download_songs: 3,
            ..Storage::default()
        };
        let stored = |storage: Storage| {
            let facts = Facts { storage, ..Facts::default() };
            every_row_with(&prefs, &facts).into_iter().map(|(_, r)| row_words(&r)).find(|(t, _)| t.starts_with("Stored on this device")).unwrap().1
        };
        let both = stored(storage.clone());
        assert!(both.contains("5 downloads"), "{both}");
        assert!(both.contains("Home 1.9 MB") && both.contains("Away 977 KB"), "{both}");
        let one = stored(Storage { download_profiles: vec![("a".into(), 3_000_000)], ..storage });
        assert!(!one.contains("Home"), "one profile needs no split: {one}");
    }

    fn every_row_with(prefs: &StoredPrefs, facts: &Facts) -> Vec<(&'static str, Row)> {
        let state = settings_model::state(prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
        GROUPS
            .iter()
            .flat_map(|g| page(g.id, prefs, &state, facts, &Own::default(), DAY).sections.into_iter().flat_map(move |s| s.rows.into_iter().map(move |r| (g.title, r))))
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
            let mut v = SettingsView { pages: Some(vec![Page { title: group.to_string(), summary: String::new(), sections: vec![Section { title: String::new(), rows: vec![row.clone()] }] }]), ..Default::default() };
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
                Row::Slider { target, label, .. } => {
                    let c = v.step(true);
                    let sent = match (&c[..], target) {
                        ([Cmd::Level(l, _)], Target::Level(to)) => l == to,
                        ([Cmd::Setting(n, _)], Target::Setting(name)) => n == name,
                        _ => false,
                    };
                    assert!(sent, "{label}");
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
        assert_eq!(shown("replayGain"), "Automatic");
        assert_eq!(shown("wifi"), "Original");
        assert_eq!(shown("crossfadeSec"), "Off");
        assert_eq!((quality("320:mp3"), quality("128:opus")), ("MP3 320".to_string(), "Opus 128".to_string()));

        // Phone only settings hidden.
        let names: Vec<String> = every_row(&everything_on())
            .into_iter()
            .filter_map(|(_, r)| match r {
                Row::Toggle { switch: Switch::Setting(name), .. } | Row::Choice { target: Target::Setting(name), .. } => Some(name),
                _ => None,
            })
            .collect();
        for phone in ["swipeLeft", "swipeRight", "tapAction", "offload", "bitPerfect", "lyricsKeepScreenOn", "motion", "soundBypass", "altMaxBitRate"] {
            assert!(!names.iter().any(|n| n == phone), "{phone} offered in a terminal");
        }
    }

    #[test]
    fn music_folders_switch_from_all() {
        let server = nori_core::settings::SavedServer { id: "a".into(), ..Default::default() };
        let prefs = StoredPrefs { servers: vec![server], active_server_id: "a".into(), ..StoredPrefs::default() };
        let state = settings_model::state(&prefs, settings_model::Output::default(), &crate::backend::app().settings.model);
        let facts = Facts { folders: vec![MusicFolder { id: "1".into(), name: "A".into() }, MusicFolder { id: "2".into(), name: "B".into() }], ..Facts::default() };
        let page = page(Gid::Core(SettingsPage::Servers), &prefs, &state, &facts, &Own::default(), DAY);
        let folders = page.sections.iter().find(|s| s.rows.iter().any(|r| matches!(r, Row::Toggle { switch: Switch::Stores(n, _), .. } if n == "musicFolders"))).unwrap();
        let switches: Vec<_> = folders.rows.iter().map(|r| match r {
            Row::Toggle { switch: Switch::Stores(name, value), title, on, .. } => (name.as_str(), value.as_str(), title.as_str(), *on),
            other => panic!("{other:?}"),
        }).collect();
        assert_eq!(switches, [("musicFolders", "2", "A", true), ("musicFolders", "1", "B", true)]);
    }

    #[test]
    fn lyrics_sources_move_and_switch() {
        // Two are switched off: one without a key, one whose key is optional.
        let d = StoredPrefs::default();
        let prefs = StoredPrefs { lyrics_online: true, third_party_lookups: true, lyrics_on: d.lyrics_on.iter().filter(|s| !["UNISON", "BETTER_LYRICS"].contains(&s.name())).copied().collect(), ..d };
        let mut v = SettingsView::default();
        let pages = v.pages(&prefs).to_vec();
        let lines = SettingsView::lines(&pages);
        let ranked: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Row(Row::Ranked { .. }))).map(|(i, _)| i).collect();
        assert_eq!(ranked.len(), nori_core::lyrics_sources::LyricsService::ALL.len());
        assert!(lines.iter().all(|l| !matches!(l, Line::Row(Row::Ranked { title, .. }) if title == "?")), "every service has a name");
        let off = *ranked.iter().find(|i| matches!(lines[**i], Line::Row(Row::Ranked { on: false, asks_key: false, key: KeyAction::None, .. }))).unwrap();
        let Line::Row(Row::Ranked { id, name, .. }) = lines[off] else { unreachable!() };
        let (id, name) = (id.clone(), name.clone());
        v.row.at = off;
        assert!(matches!(&v.step(false)[..], [Cmd::Setting(n, value)] if n == "lyricsMove" && *value == format!("{id}:-1")));
        let Some(Opened::Cmds(c)) = v.open() else { panic!() };
        assert!(matches!(&c[..], [Cmd::Setting(n, value)] if *n == name && value == "true"));
        // A service that needs a key asks for it when switched on.
        let keyed = *ranked.iter().find(|i| matches!(lines[**i], Line::Row(Row::Ranked { asks_key: true, on: false, .. }))).unwrap();
        v.row.at = keyed;
        let Some(Opened::Overlay(Overlay::Input { target: Target::Setting(name), secret: true, .. })) = v.open() else { panic!("no key asked") };
        assert!(name.starts_with("lyricsKey:"), "{name}");
        // One whose key is optional switches on without a prompt; its key is added on its own key.
        let optional = *ranked.iter().find(|i| matches!(lines[**i], Line::Row(Row::Ranked { key: KeyAction::Add, on: false, .. }))).unwrap();
        v.row.at = optional;
        assert!(matches!(v.open(), Some(Opened::Cmds(c)) if matches!(&c[..], [Cmd::Setting(n, on)] if n.starts_with("lyricsService:") && on == "true")));
        assert!(matches!(v.key(), Some(Overlay::Input { target: Target::Setting(name), .. }) if name.starts_with("lyricsKey:")));
        v.row.at = off;
        assert!(v.key().is_none(), "a service without a key has none to add");
        // Third-party lookups off: the sources stay listed, grayed, and take no keys.
        let quiet = StoredPrefs { third_party_lookups: false, ..prefs };
        v.invalidate();
        let pages = v.pages(&quiet).to_vec();
        let lines = SettingsView::lines(&pages);
        assert!(lines.iter().any(|l| matches!(l, Line::Row(Row::Ranked { .. }))));
        assert!(lines.iter().all(|l| !matches!(l, Line::Row(Row::Ranked { enabled: true, .. }))));
        v.row.at = keyed;
        assert!(v.open().is_none() && v.step(true).is_empty(), "a grayed source does nothing");
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
        let prefs = StoredPrefs { eq_mode: EqMode::Parametric, eq_bands: (0..3).map(|i| SoundBand { kind: nori_core::EqKind::Peaking, freq: 100.0 * 10f32.powi(i), gain_db: 0.0, q: 1.0, channel: nori_core::settings::BandChannel::Both }).collect(), ..StoredPrefs::default() };
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
