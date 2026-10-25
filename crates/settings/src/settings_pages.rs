//! What each page of a client's settings screen lists: its sections and rows, which are shown and
//! live, and how deep each sits under the one above. Every client draws these and words them; which
//! settings a client offers and when a row applies are decided here once.

use crate::settings::{GainMode, StoredPrefs};
use crate::settings_model::{value_of, BeatModel, SettingSpec, SettingsState};

/// Which client the settings are drawn by: each offers what its platform has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SettingsClient {
    Phone,
    Desktop,
    Terminal,
    /// A browser: the settings its session and engine apply there.
    Web,
}

/// The parts of the settings a profile opens (`nori_library::browse::ProfileRules::settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[repr(u8)]
pub enum SettingsPart {
    /// The saved profiles, to switch between.
    Profiles,
    /// Adding, editing and removing profiles, and the open server's own options (music folder, second
    /// address's bitrate, syncing its library).
    ServerOptions,
    /// Theme, motion and the screen.
    Look,
    /// Transitions, controls, the queue.
    Playback,
    Sound,
    Lyrics,
    /// The account's lists, playlists, search, history, scrobbling and remote control.
    Library,
    /// Streaming quality.
    Streaming,
    Downloads,
    /// The caches on this device.
    Storage,
    About,
}

/// A page of the settings, in the order the root lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SettingsPage {
    Servers,
    Playing,
    Sound,
    Look,
    Lyrics,
    /// Every lyrics service in rank order, and their keys (reached from Lyrics on a phone).
    LyricsSources,
    Library,
    Data,
    About,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 9] = {
        use SettingsPage::*;
        [Servers, Playing, Sound, Look, Lyrics, LyricsSources, Library, Data, About]
    };
}

/// A section of a page; the client words its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SectionId {
    Accounts,
    MusicFolders,
    ThisServer,
    Between,
    Queue,
    Equalizer,
    Effects,
    Volume,
    Output,
    Theme,
    Home,
    Screen,
    Motion,
    Display,
    Sources,
    AskedOrder,
    Lists,
    Search,
    History,
    Online,
    Devices,
    Streaming,
    Downloads,
    Storage,
    Updates,
}

/// Another screen a row opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SettingsLink {
    Equalizer,
    /// The phone's own sound effects panel.
    SystemEffects,
    HomeRows,
    LibrarySections,
    LyricsSources,
    Downloads,
}

/// What a row's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SettingsAction {
    MeasureAgain,
    SyncLibrary,
    DownloadLibrary,
    AnalyseDownloads,
    ClearStream,
    ClearCovers,
    ClearLyrics,
    AddServer,
    JoinJam,
    CheckUpdate,
}

/// A line of explanation among the rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SettingsNote {
    /// Bit-perfect output takes transitions and effects out of the path.
    HeldByDac,
    /// What the USB DAC offers and plays (the phone's words).
    Dac,
    /// How the lyrics services are asked.
    LyricsSources,
}

/// What a row is.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RowItem {
    /// A setting, changed by its spec's name; `chips`: few values, all on view.
    Setting { spec: SettingSpec, value: String, chips: bool },
    Link { to: SettingsLink },
    Action { action: SettingsAction },
    Note { note: SettingsNote },
    /// What is stored on this device, by kind.
    Stored,
    /// `StoredPrefs::servers[index]`.
    Server { index: u32 },
    /// One of the active server's music folders (`PageFacts::folders[index]`): whether the library shows
    /// it, and the `musicFolders` value a tap sets.
    MusicFolder { index: u32, on: bool, tapped: String },
    /// `SettingsState::lyrics_sources[index]`, switched by `lyricsService:<id>` and moved by `lyricsMove`.
    LyricsSource { index: u32 },
    /// The compressor's gain reduction, read from the player while the page is on screen.
    CompressionMeter,
    /// Changes made here the server has not taken: queued, and dropped since the profile opened.
    Unsent { waiting: u32, dropped: u32 },
}

/// One row: what it is, whether it is on the page now, whether it acts now, and its depth under the
/// row above (0: a row of its own).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingsRow {
    pub item: RowItem,
    pub shown: bool,
    pub enabled: bool,
    pub depth: u8,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SettingsSection {
    pub id: SectionId,
    pub rows: Vec<SettingsRow>,
}

/// What a page depends on besides the settings, from the device.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PageFacts {
    /// The parts of the settings the open profile offers.
    pub parts: Vec<SettingsPart>,
    /// Drawn on a car's own screen.
    pub car: bool,
    /// The active server's music folders, by id.
    pub folders: Vec<String>,
    /// Songs AutoMix has measured.
    pub analysed: u32,
    /// Songs in the offline index.
    pub indexed_songs: u32,
    pub syncing: bool,
    pub stream_bytes: i64,
    pub cover_bytes: i64,
    pub lyrics_bytes: i64,
    pub download_songs: u32,
    /// A cache is being cleared.
    pub storage_busy: bool,
    /// Changes waiting for the server, and dropped unsent since the profile opened.
    pub unsent_waiting: u32,
    pub unsent_dropped: u32,
}

/// The pages `client` has, in the order the root lists them; those the profile opens.
pub fn pages(client: SettingsClient, parts: &[SettingsPart]) -> Vec<SettingsPage> {
    let offered = |p: SettingsPage| match client {
        SettingsClient::Phone => true,
        SettingsClient::Desktop | SettingsClient::Terminal => p != SettingsPage::LyricsSources,
        SettingsClient::Web => matches!(p, SettingsPage::Playing | SettingsPage::Sound | SettingsPage::Look | SettingsPage::Lyrics | SettingsPage::Library),
    };
    SettingsPage::ALL.into_iter().filter(|p| !page_parts(*p).iter().all(|part| !parts.contains(part)) && offered(*p)).collect()
}

fn page_parts(page: SettingsPage) -> &'static [SettingsPart] {
    use SettingsPart::*;
    match page {
        SettingsPage::Servers => &[Profiles],
        SettingsPage::Playing => &[Playback],
        SettingsPage::Sound => &[Sound],
        SettingsPage::Look => &[Look],
        SettingsPage::Lyrics | SettingsPage::LyricsSources => &[Lyrics],
        SettingsPage::Library => &[Library],
        SettingsPage::Data => &[Streaming, Downloads, Storage],
        SettingsPage::About => &[About],
    }
}

/// Builds a page's rows.
struct Page<'a> {
    p: &'a StoredPrefs,
    s: &'a SettingsState,
    f: &'a PageFacts,
    client: SettingsClient,
    specs: Vec<SettingSpec>,
}

fn row(item: RowItem) -> SettingsRow {
    SettingsRow { item, shown: true, enabled: true, depth: 0 }
}

impl SettingsRow {
    fn shown(self, shown: bool) -> Self {
        SettingsRow { shown, ..self }
    }
    fn enabled(self, enabled: bool) -> Self {
        SettingsRow { enabled, ..self }
    }
    fn under(self) -> Self {
        SettingsRow { depth: 1, ..self }
    }
}

impl Page<'_> {
    fn setting(&self, name: &str) -> SettingsRow {
        self.setting_as(name, false)
    }

    fn chips(&self, name: &str) -> SettingsRow {
        self.setting_as(name, true)
    }

    fn setting_as(&self, name: &str, chips: bool) -> SettingsRow {
        let spec = self.specs.iter().find(|s| s.name == name).cloned().unwrap_or_else(|| panic!("{name} is a setting"));
        let value = value_of(self.p, name).unwrap_or_default();
        row(RowItem::Setting { spec, value, chips })
    }

    fn on(&self, name: &str) -> bool {
        value_of(self.p, name).as_deref() == Some("true")
    }

    fn phone(&self) -> bool {
        self.client == SettingsClient::Phone
    }

    /// Phone and desktop: the clients with a window of their own.
    fn windowed(&self) -> bool {
        matches!(self.client, SettingsClient::Phone | SettingsClient::Desktop)
    }

    /// A browser: no files of its own (no downloads, stream cache or measuring from disk) and no
    /// outputs to tell apart.
    fn web(&self) -> bool {
        self.client == SettingsClient::Web
    }

    fn sections(&self, page: SettingsPage) -> Vec<(SectionId, Vec<SettingsRow>)> {
        match page {
            SettingsPage::Servers => self.servers(),
            SettingsPage::Playing => self.playing(),
            SettingsPage::Sound => self.sound(),
            SettingsPage::Look => self.look(),
            SettingsPage::Lyrics => self.lyrics(),
            SettingsPage::LyricsSources => self.lyrics_sources(),
            SettingsPage::Library => self.library(),
            SettingsPage::Data => self.data(),
            SettingsPage::About => self.about(),
        }
    }

    fn servers(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let p = self.p;
        let mut accounts: Vec<SettingsRow> = (0..p.servers.len() as u32).map(|index| row(RowItem::Server { index })).collect();
        accounts.push(row(RowItem::Action { action: SettingsAction::AddServer }));
        let shown = p.active_server().map(|s| s.music_folder_ids.clone()).unwrap_or_default();
        let folders = crate::settings::folder_switches(shown, self.f.folders.clone())
            .into_iter()
            .enumerate()
            .map(|(i, s)| row(RowItem::MusicFolder { index: i as u32, on: s.on, tapped: s.tapped }).shown(self.f.folders.len() > 1))
            .collect();
        let alt = p.servers.iter().any(|s| s.id == p.active_server_id && !s.alt_url.trim().is_empty());
        let mut out = vec![(SectionId::Accounts, accounts), (SectionId::MusicFolders, folders)];
        if self.phone() {
            out.push((SectionId::ThisServer, vec![self.setting("altMaxBitRate").shown(alt)]));
        }
        out
    }

    fn playing(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let (p, s) = (self.p, self.s);
        let live = !s.untouched;
        let mut between = vec![row(RowItem::Note { note: SettingsNote::HeldByDac }).shown(s.untouched), self.setting("autoMix").enabled(live)];
        if s.beat_model != BeatModel::Unavailable {
            between.push(self.setting("autoMixBetterBeats").under().enabled(live && p.auto_mix));
        }
        if !self.web() {
            between.push(row(RowItem::Action { action: SettingsAction::MeasureAgain }).under().enabled(p.auto_mix && self.f.analysed > 0));
        }
        // AutoMix plans its own transitions, so the plain crossfade gives way to it.
        between.push(self.setting("crossfadeSec").enabled(live && !p.auto_mix));
        between.push(self.setting("crossfadeKeepAlbums").enabled(live && (p.auto_mix || p.crossfade_sec > 0)));
        let mut queue = vec![
            self.setting("skipExplicit"),
            self.setting("autoFill"),
            self.chips("autoFillKind").under().enabled(p.auto_fill),
            self.setting("autoFillBasis").under().enabled(p.auto_fill),
            self.setting("autoFillRemote").under().enabled(p.auto_fill),
        ];
        if !self.web() {
            queue.push(self.setting("bridgeOffline"));
        }
        vec![(SectionId::Between, between), (SectionId::Queue, queue)]
    }

    fn sound(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let p = self.p;
        let volume = vec![
            self.chips("replayGain"),
            self.chips("loudnessLevel").under().enabled(p.replay_gain != GainMode::Off),
            self.setting("volumeBoostDb"),
        ];
        let mut eq = vec![row(RowItem::Link { to: SettingsLink::Equalizer })];
        if self.web() {
            let effects = vec![self.setting("compressor"), self.chips("compressorPreset").under().enabled(p.compressor)];
            return vec![(SectionId::Equalizer, eq), (SectionId::Effects, effects), (SectionId::Volume, volume)];
        }
        if self.windowed() {
            eq.extend([self.setting("autoEqAuto"), self.setting("autoEqDownload"), self.setting("profilePerOutput")]);
        }
        if self.phone() {
            eq.push(row(RowItem::Link { to: SettingsLink::SystemEffects }));
        }
        let mut effects = vec![self.chips("headphoneSound"), self.setting("compressor"), self.chips("compressorPreset").under().enabled(p.compressor)];
        if self.phone() {
            effects.push(row(RowItem::CompressionMeter).under().enabled(p.compressor));
        }
        let mut output = vec![self.setting("hiRes"), self.setting("maxRate")];
        if self.phone() {
            output.extend([self.setting("bitPerfect"), row(RowItem::Note { note: SettingsNote::Dac }), self.setting("offload")]);
        }
        if self.windowed() {
            output.push(self.setting("soundBypass"));
        }
        vec![(SectionId::Equalizer, eq), (SectionId::Effects, effects), (SectionId::Volume, volume), (SectionId::Output, output)]
    }

    fn look(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let p = self.p;
        let mut theme = vec![self.chips("season")];
        if self.phone() || self.web() || self.client == SettingsClient::Terminal {
            theme.push(self.chips("theme"));
        }
        if self.web() {
            return vec![(SectionId::Theme, theme)];
        }
        let mut out = vec![(SectionId::Theme, theme)];
        if self.phone() {
            use crate::settings::DimIdle;
            let home = vec![row(RowItem::Link { to: SettingsLink::HomeRows }), row(RowItem::Link { to: SettingsLink::LibrarySections })];
            let mut screen = vec![
                self.setting("hideStatusBar"),
                self.setting("keepAwake"),
                self.setting("burnInProtection"),
                self.chips("dimWhenIdle").under().enabled(p.burn_in_protection),
                self.chips("dimLevel").under().enabled(p.burn_in_protection && p.dim_when_idle != DimIdle::Never),
            ];
            // Only on a car's own screen, the only place it changes anything.
            if self.f.car {
                screen.push(self.chips("controlsSide"));
            }
            out.extend([(SectionId::Home, home), (SectionId::Screen, screen), (SectionId::Motion, vec![self.chips("motion")])]);
        }
        out
    }

    fn lyrics(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let mut display = vec![self.setting("lyricsSweep")];
        if self.phone() {
            display.push(self.setting("lyricsSize"));
        }
        display.push(self.setting("lyricsTranslation"));
        if self.phone() {
            display.push(self.setting("lyricsKeepScreenOn"));
        }
        let online = self.on("lyricsOnline");
        let mut sources = vec![self.setting("lyricsOnline"), self.setting("lyricsPreferWords").under().enabled(online)];
        let mut out = vec![(SectionId::Display, display)];
        if self.phone() {
            sources.push(row(RowItem::Link { to: SettingsLink::LyricsSources }).enabled(online));
        }
        out.push((SectionId::Sources, sources));
        if !self.phone() && !self.web() {
            out.extend(self.lyrics_sources().into_iter().map(|(id, rows)| (id, rows.into_iter().map(|r| SettingsRow { enabled: r.enabled && online, ..r }).collect())));
        }
        out
    }

    /// Every lyrics service in the order they are asked, each switched and keyed where it stands.
    fn lyrics_sources(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let mut ranked: Vec<SettingsRow> = (0..self.s.lyrics_sources.len() as u32).map(|index| row(RowItem::LyricsSource { index })).collect();
        ranked.push(row(RowItem::Note { note: SettingsNote::LyricsSources }));
        vec![(SectionId::AskedOrder, ranked)]
    }

    fn library(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let mut out = Vec::new();
        if self.phone() {
            out.push((SectionId::Lists, vec![self.chips("tapAction"), self.setting("swipeRight"), self.setting("swipeLeft"), self.setting("playlistNotes")]));
        }
        if !self.web() {
            out.push((SectionId::Search, vec![row(RowItem::Action { action: SettingsAction::SyncLibrary }).enabled(!self.f.syncing)]));
        }
        out.push((SectionId::History, vec![self.setting("tasteModel"), self.setting("scrobble")]));
        out.push((SectionId::Online, vec![self.setting("thirdPartyLookups")]));
        let mut devices = vec![self.setting("remoteControl")];
        match self.client {
            SettingsClient::Phone => devices.push(self.setting("queueOffer")),
            SettingsClient::Terminal => devices.push(row(RowItem::Action { action: SettingsAction::JoinJam })),
            SettingsClient::Desktop | SettingsClient::Web => {}
        }
        out.push((SectionId::Devices, devices));
        out
    }

    fn data(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        let f = self.f;
        let mut streaming = vec![self.setting("wifi")];
        if self.phone() {
            streaming.push(self.setting("mobile"));
        }
        streaming.push(self.setting("songsAheadWifi"));
        if self.phone() {
            streaming.push(self.setting("songsAheadMobile").under());
        }
        streaming.push(self.setting("cacheMb"));
        let mut downloads = vec![self.setting("download"), row(RowItem::Action { action: SettingsAction::DownloadLibrary }).enabled(f.indexed_songs > 0)];
        match self.client {
            SettingsClient::Phone => downloads.push(row(RowItem::Action { action: SettingsAction::AnalyseDownloads }).enabled(f.download_songs > 0)),
            SettingsClient::Terminal => downloads.push(row(RowItem::Link { to: SettingsLink::Downloads })),
            SettingsClient::Desktop | SettingsClient::Web => {}
        }
        let clear = |action, bytes: i64| row(RowItem::Action { action }).enabled(!f.storage_busy && bytes > 0);
        // What is kept and what clears it, no settings: the cache's size is streaming's.
        let mut storage = vec![
            row(RowItem::Stored),
            row(RowItem::Unsent { waiting: f.unsent_waiting, dropped: f.unsent_dropped }).shown(f.unsent_waiting > 0 || f.unsent_dropped > 0),
            clear(SettingsAction::ClearStream, f.stream_bytes),
            clear(SettingsAction::ClearCovers, f.cover_bytes),
            clear(SettingsAction::ClearLyrics, f.lyrics_bytes),
        ];
        if self.phone() {
            storage.push(row(RowItem::Link { to: SettingsLink::Downloads }));
        }
        vec![(SectionId::Streaming, streaming), (SectionId::Downloads, downloads), (SectionId::Storage, storage)]
    }

    fn about(&self) -> Vec<(SectionId, Vec<SettingsRow>)> {
        if !self.phone() {
            return Vec::new();
        }
        vec![(SectionId::Updates, vec![row(RowItem::Action { action: SettingsAction::CheckUpdate }), self.setting("updateCheck"), self.setting("updateBeta")])]
    }
}

/// The part of the settings a section belongs to.
fn section_part(page: SettingsPage, id: SectionId) -> SettingsPart {
    use SettingsPart::*;
    match (page, id) {
        (SettingsPage::Servers, SectionId::Accounts) => Profiles,
        (SettingsPage::Servers, _) => ServerOptions,
        (SettingsPage::Data, SectionId::Downloads) => Downloads,
        (SettingsPage::Data, SectionId::Storage) => Storage,
        (SettingsPage::Data, _) => Streaming,
        _ => page_parts(page)[0],
    }
}

/// Page `page` as `client` draws it for these settings, state and facts: its sections in order, the
/// ones the profile opens ([`PageFacts::parts`]). Rows a setting hides are there with `shown` off, so
/// a search finds them; rows this client or device does not have are left out.
pub fn page(page: SettingsPage, client: SettingsClient, p: &StoredPrefs, s: &SettingsState, f: &PageFacts) -> Vec<SettingsSection> {
    let b = Page { p, s, f, client, specs: crate::settings_model::specs() };
    let server_options = f.parts.contains(&SettingsPart::ServerOptions);
    b.sections(page)
        .into_iter()
        .filter(|(id, _)| f.parts.contains(&section_part(page, *id)))
        .map(|(id, rows)| {
            let rows = rows.into_iter().filter(|r| server_options || !matches!(r.item, RowItem::Action { action: SettingsAction::AddServer })).collect();
            SettingsSection { id, rows }
        })
        .filter(|s| !s.rows.is_empty())
        .collect()
}

/// [`page`] for the platform.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn settings_page(page: SettingsPage, client: SettingsClient, prefs: StoredPrefs, state: SettingsState, facts: PageFacts) -> Vec<SettingsSection> {
    self::page(page, client, &prefs, &state, &facts)
}

/// [`pages`] for the platform.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn settings_pages(client: SettingsClient, parts: Vec<SettingsPart>) -> Vec<SettingsPage> {
    pages(client, &parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SavedServer;
    use crate::settings_model::{state, Output, SettingKind};

    const ALL_PARTS: [SettingsPart; 11] = {
        use SettingsPart::*;
        [Profiles, ServerOptions, Look, Playback, Sound, Lyrics, Library, Streaming, Downloads, Storage, About]
    };

    fn facts() -> PageFacts {
        PageFacts { parts: ALL_PARTS.to_vec(), folders: vec!["1".into(), "2".into()], car: true, ..PageFacts::default() }
    }

    fn model() -> nori_automix::beat_model::ModelFile {
        nori_automix::beat_model::ModelFile::default()
    }

    fn every_row(client: SettingsClient, p: &StoredPrefs, f: &PageFacts) -> Vec<SettingsRow> {
        let s = state(p, Output::default(), &model());
        pages(client, &f.parts).into_iter().flat_map(|pg| page(pg, client, p, &s, f)).flat_map(|s| s.rows).collect()
    }

    fn names(rows: &[SettingsRow], shown_only: bool) -> Vec<String> {
        rows.iter()
            .filter(|r| r.shown || !shown_only)
            .filter_map(|r| match &r.item {
                RowItem::Setting { spec, .. } => Some(spec.name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_offered_setting_is_on_a_phone_page_once() {
        let p = StoredPrefs { servers: vec![SavedServer { id: "a".into(), alt_url: "http://b".into(), ..SavedServer::default() }], active_server_id: "a".into(), ..StoredPrefs::default() };
        let on_pages = names(&every_row(SettingsClient::Phone, &p, &facts()), false);
        let mut sorted = on_pages.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), on_pages.len(), "each once");
        // The equalizer's own screen has these.
        let elsewhere = ["eq", "eqMode", "eqPreampDb", "limiterThresholdDb", "mono", "limiter", "eqLayout"];
        for s in crate::settings_model::specs() {
            if elsewhere.contains(&s.name.as_str()) || (s.name == "autoMixBetterBeats" && !nori_player::automix::beats::AVAILABLE) {
                continue;
            }
            assert!(on_pages.contains(&s.name), "{} is on no page", s.name);
        }
    }

    /// Whether `name`'s row acts for `p` on `client`'s pages.
    fn live(client: SettingsClient, p: &StoredPrefs, name: &str) -> bool {
        every_row(client, p, &facts()).iter().any(|r| r.shown && r.enabled && matches!(&r.item, RowItem::Setting { spec, .. } if spec.name == name))
    }

    #[test]
    fn rows_follow_the_settings() {
        let live = |p: &StoredPrefs, name: &str| live(SettingsClient::Phone, p, name);
        let d = StoredPrefs::default();
        assert!(live(&d, "crossfadeSec") && !live(&StoredPrefs { auto_mix: true, ..d.clone() }, "crossfadeSec"), "AutoMix plans its own");
        let mixing = [StoredPrefs { auto_mix: true, ..d.clone() }, StoredPrefs { crossfade_sec: 4, ..d.clone() }];
        assert!(!live(&d, "crossfadeKeepAlbums") && mixing.iter().all(|p| live(p, "crossfadeKeepAlbums")), "only while songs mix");
        assert!(!live(&d, "compressorPreset") && live(&StoredPrefs { compressor: true, ..d.clone() }, "compressorPreset"));
        assert!(live(&d, "loudnessLevel") && !live(&StoredPrefs { replay_gain: GainMode::Off, ..d.clone() }, "loudnessLevel"));
        assert!(!live(&StoredPrefs { auto_fill: false, ..d.clone() }, "autoFillKind"));
        assert!(!live(&d, "dimLevel") && live(&StoredPrefs { burn_in_protection: true, dim_when_idle: crate::settings::DimIdle::Minute, ..d.clone() }, "dimLevel"));
        assert!(!live(&StoredPrefs { lyrics_online: false, ..d.clone() }, "lyricsPreferWords"));
    }

    #[test]
    fn a_switch_grays_the_rows_under_it_and_moves_none() {
        let d = StoredPrefs::default();
        let switched = [
            StoredPrefs { auto_mix: true, compressor: true, auto_fill: false, burn_in_protection: true, replay_gain: GainMode::Off, theme: crate::settings::ThemeMode::Light, lyrics_online: false, ..d.clone() },
            d.clone(),
        ];
        for client in [SettingsClient::Phone, SettingsClient::Desktop, SettingsClient::Terminal, SettingsClient::Web] {
            let [a, b] = switched.each_ref().map(|p| every_row(client, p, &facts()).into_iter().map(|r| (std::mem::discriminant(&r.item), r.shown, r.depth)).collect::<Vec<_>>());
            assert_eq!(a, b, "{client:?}: the same rows in the same places");
        }
    }

    #[test]
    fn each_client_offers_its_own() {
        let d = StoredPrefs::default();
        let phone = names(&every_row(SettingsClient::Phone, &d, &facts()), false);
        for client in [SettingsClient::Desktop, SettingsClient::Terminal] {
            let rows = every_row(client, &d, &facts());
            let has = names(&rows, false);
            for phone_only in ["swipeLeft", "tapAction", "offload", "bitPerfect", "lyricsKeepScreenOn", "motion", "hideStatusBar", "mobile", "songsAheadMobile", "altMaxBitRate"] {
                assert!(phone.iter().any(|n| n == phone_only) && !has.iter().any(|n| n == phone_only), "{phone_only} offered by {client:?}");
            }
            assert!(rows.iter().any(|r| matches!(r.item, RowItem::LyricsSource { .. })), "the services are on the lyrics page");
            assert!(!pages(client, &ALL_PARTS).contains(&SettingsPage::LyricsSources));
        }
        let terminal = names(&every_row(SettingsClient::Terminal, &d, &facts()), false);
        assert!(!terminal.iter().any(|n| n == "soundBypass" || n == "autoEqAuto"));
        // The terminal paints its page in the season's colors, light, dark or black.
        assert!(["season", "theme"].iter().all(|n| terminal.iter().any(|t| t == n)));
        assert!(names(&every_row(SettingsClient::Desktop, &d, &facts()), false).iter().any(|n| n == "soundBypass"));
        // Not on a car's screen: no side for the controls.
        let parked = PageFacts { car: false, ..facts() };
        assert!(!names(&every_row(SettingsClient::Phone, &d, &parked), false).iter().any(|n| n == "controlsSide"));
    }

    #[test]
    fn a_browser_offers_what_applies_there() {
        let d = StoredPrefs::default();
        assert_eq!(pages(SettingsClient::Web, &ALL_PARTS), [SettingsPage::Playing, SettingsPage::Sound, SettingsPage::Look, SettingsPage::Lyrics, SettingsPage::Library]);
        let rows = every_row(SettingsClient::Web, &d, &facts());
        let has = names(&rows, false);
        for kept in ["autoMix", "crossfadeSec", "replayGain", "loudnessLevel", "compressor", "lyricsSweep", "remoteControl", "season", "theme"] {
            assert!(has.iter().any(|n| n == kept), "{kept} is on the web's pages");
        }
        for left in ["bridgeOffline", "hiRes", "maxRate", "soundBypass", "autoEqAuto", "headphoneSound", "cacheMb", "download", "wifi", "songsAheadWifi", "accent", "lyricsSize"] {
            assert!(!has.iter().any(|n| n == left), "{left} is not offered in a browser");
        }
        let links: Vec<_> = rows.iter().filter_map(|r| match r.item {
            RowItem::Link { to } => Some(to),
            _ => None,
        }).collect();
        assert_eq!(links, [SettingsLink::Equalizer], "the equalizer is the web's one page within Settings");
        assert!(!rows.iter().any(|r| matches!(r.item, RowItem::Action { .. } | RowItem::LyricsSource { .. })), "nothing the web has no screen or work for");
        assert!(!live(SettingsClient::Web, &StoredPrefs { auto_mix: true, ..d.clone() }, "crossfadeSec"), "AutoMix plans its own");
    }

    #[test]
    fn a_guest_sees_only_this_devices_settings() {
        let guest = PageFacts { parts: ALL_PARTS.into_iter().filter(|p| !matches!(p, SettingsPart::ServerOptions | SettingsPart::Library | SettingsPart::Downloads)).collect(), ..facts() };
        let d = StoredPrefs { servers: vec![SavedServer { id: "a".into(), ..SavedServer::default() }], active_server_id: "a".into(), ..StoredPrefs::default() };
        let s = state(&d, Output::default(), &model());
        assert!(!pages(SettingsClient::Phone, &guest.parts).contains(&SettingsPage::Library));
        let servers = page(SettingsPage::Servers, SettingsClient::Phone, &d, &s, &guest);
        assert_eq!(servers.iter().map(|s| s.id).collect::<Vec<_>>(), [SectionId::Accounts]);
        assert!(!servers[0].rows.iter().any(|r| matches!(r.item, RowItem::Action { action: SettingsAction::AddServer })));
        let data = page(SettingsPage::Data, SettingsClient::Phone, &d, &s, &guest);
        assert_eq!(data.iter().map(|s| s.id).collect::<Vec<_>>(), [SectionId::Streaming, SectionId::Storage]);
    }

    #[test]
    fn storage_holds_no_settings() {
        let d = StoredPrefs::default();
        let s = state(&d, Output::default(), &model());
        for client in [SettingsClient::Phone, SettingsClient::Desktop, SettingsClient::Terminal] {
            let data = page(SettingsPage::Data, client, &d, &s, &facts());
            let storage = data.iter().find(|s| s.id == SectionId::Storage).unwrap();
            assert!(!storage.rows.iter().any(|r| matches!(r.item, RowItem::Setting { .. })), "{client:?}");
            let streaming = data.iter().find(|s| s.id == SectionId::Streaming).unwrap();
            assert!(streaming.rows.iter().any(|r| matches!(&r.item, RowItem::Setting { spec, .. } if spec.name == "cacheMb")), "{client:?}");
        }
    }

    #[test]
    fn unsent_changes_show_when_there_are_some() {
        let d = StoredPrefs::default();
        let s = state(&d, Output::default(), &model());
        let unsent = |f: PageFacts| {
            let data = page(SettingsPage::Data, SettingsClient::Phone, &d, &s, &f);
            data.into_iter().flat_map(|s| s.rows).find(|r| matches!(r.item, RowItem::Unsent { .. })).map(|r| (r.shown, r.item))
        };
        assert_eq!(unsent(facts()).map(|r| r.0), Some(false));
        assert_eq!(unsent(PageFacts { unsent_waiting: 3, ..facts() }), Some((true, RowItem::Unsent { waiting: 3, dropped: 0 })));
        assert_eq!(unsent(PageFacts { unsent_dropped: 1, ..facts() }), Some((true, RowItem::Unsent { waiting: 0, dropped: 1 })));
    }

    #[test]
    fn rows_carry_their_spec_and_value() {
        let p = StoredPrefs { volume_boost_db: 3.0, ..StoredPrefs::default() };
        let rows = every_row(SettingsClient::Phone, &p, &facts());
        let boost = rows.iter().find_map(|r| match &r.item {
            RowItem::Setting { spec, value, .. } if spec.name == "volumeBoostDb" => Some((spec.clone(), value.clone())),
            _ => None,
        });
        let (spec, value) = boost.unwrap();
        assert_eq!((spec.kind, spec.min, spec.max, value.as_str()), (SettingKind::Level, 0.0, 12.0, "3"), "the slider's range is the setting's");
        let folders: Vec<(bool, String)> = rows.iter().filter_map(|r| match &r.item {
            RowItem::MusicFolder { on, tapped, .. } => Some((*on, tapped.clone())),
            _ => None,
        }).collect();
        assert_eq!(folders, [(true, "2".to_string()), (true, "1".to_string())]);
        let measure = rows.iter().find(|r| matches!(r.item, RowItem::Action { action: SettingsAction::MeasureAgain })).unwrap();
        assert!(!measure.enabled && measure.depth == 1, "nothing measured yet");
    }
}
