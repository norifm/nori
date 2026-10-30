//! User-facing wording for numbers and errors (mirrors the terminal's text.rs).

use nori_core::numbers;
use nori_core::pages::{ReleaseGroup, ReleaseKind};
use nori_core::profiles::CurveNotice;
use nori_core::settings::ServerKind;
use nori_core::transport::{FailureKind, NetError};
use nori_core::Song;

/// What the desktop says of a kind of profile (nori-core's providers): the only words a provider needs
/// here.
pub struct ProviderWords {
    /// The kind's name, and what a profile of it is called without a name of its own.
    pub name: String,
    /// What adding one does, under the login page's choice.
    pub note: &'static str,
    /// The login page's button.
    pub add: String,
    /// Under a profile that names no user.
    pub no_user: &'static str,
}

/// The words of `kind`; `plugin_name` is a plugin kind's name in its own words.
pub fn provider(kind: &ServerKind, plugin_name: Option<String>) -> ProviderWords {
    match kind {
        ServerKind::Subsonic => ProviderWords { name: "OpenSubsonic".into(), note: "Sign in to your Navidrome or Subsonic server.", add: "Sign In".into(), no_user: "API key" },
        ServerKind::Plugin { id } => {
            let name = plugin_name.unwrap_or_else(|| id.clone());
            ProviderWords { add: format!("Add {name}"), name, note: "From a plugin.", no_user: "No account" }
        }
    }
}

/// The words of `kind`, a plugin's named in its own words.
pub fn provider_of(kinds: &nori_core::providers::Kinds, kind: &ServerKind) -> ProviderWords {
    provider(kind, kinds.plugin_name(kind.clone(), "en".into()))
}

pub const SHOW_ALL: &str = "Show all";

pub const PLUGINS: &str = "Plugins";
pub const FROM_A_FILE: &str = "From a file";
pub const INSTALL_FROM_FILE: &str = "Install from file";
pub const PLUGIN_PATH: &str = "A .noriplugin file or a plugin folder";
pub const NO_PLUGINS: &str = "No plugins installed.";
pub const UPDATES: &str = "Updates";
pub const INSTALLED_LIST: &str = "Installed";
pub const ALL: &str = "All";
pub const SEARCH_PLUGINS: &str = "Search plugins";
pub const NONE_FOUND: &str = "No plugins found";
pub const ABOUT: &str = "About";
pub const PLUGIN_MAY: &str = "What it may do";
pub const PLUGIN_ON: &str = "On";
pub const MOVE_UP: &str = "Move up";
pub const MOVE_DOWN: &str = "Move down";
pub const REMOVE: &str = "Remove";
pub const KEEP_VERSION: &str = "Keep this version";
pub const UNSIGNED: &str = "Unsigned";
pub const VERSIONS: &str = "Versions";
pub const SOURCE: &str = "Source";
pub const SECRET_SET: &str = "Set";
pub const NOT_SET: &str = "Not set";
pub const CHANGE: &str = "Change";
pub const SAVE: &str = "Save";
pub const BACK_UP: &str = "Back up";
pub const BACK_UP_DETAIL: &str = "Settings, sound profiles and plugins, server passwords included, locked with a password.";
pub const RESTORE: &str = "Restore a backup";
pub const RESTORE_DETAIL: &str = "Its settings replace these. Your servers stay.";
pub const BACKUP_SAVED: &str = "Backup saved";
pub const BACKUP_PASSWORD: &str = "The backup's password";

/// Asks for the file a backup is saved to, or restored from.
pub fn backup_path(restore: bool) -> &'static str {
    if restore { "Backup file to restore" } else { "File to save the backup to" }
}

/// Why a backup couldn't be made or restored.
pub fn backup_failed(f: &nori_host::desktop::BackupFailure) -> String {
    use nori_core::backup::BackupError as E;
    use nori_host::desktop::BackupFailure as F;
    match f {
        F::Backup(E::NoPassword) => "Set a password for the backup".into(),
        F::Backup(E::WrongPassword) => "Wrong password".into(),
        F::Backup(E::NotABackup) => "That isn't a nori backup".into(),
        F::Backup(E::Newer) => "Made by a newer nori: update to restore it".into(),
        F::Backup(E::Damaged) => "The backup is damaged".into(),
        F::Backup(E::Database) => "The settings couldn't be read or saved".into(),
        F::File(e) => format!("Couldn't use the file: {e}"),
    }
}

/// What a restore brought back, the plugins that didn't come back as they were named.
pub fn restored(r: &nori_core::backup::Restored) -> String {
    use nori_plugins::PluginReturn as P;
    let mut said = vec!["Restored".to_string()];
    said.extend(r.plugins.iter().filter_map(|p| match &p.outcome {
        P::AsKept => None,
        P::Other { version } => Some(format!("{} came back at {version}", p.id)),
        P::FromFile => Some(format!("{} was installed from a file: install it again", p.id)),
        P::Unavailable => Some(format!("{} wasn't found in its repo", p.id)),
    }));
    said.join(" · ")
}
pub const INSTALL: &str = "Install";
pub const UPDATE: &str = "Update";
pub const INSTALLED: &str = "Installed";
pub const PLUGIN_INSTALLED: &str = "Plugin installed. Switch it on in Settings › Plugins.";
pub const GET_PLUGINS: &str = "Get plugins";
pub const LOOK_AGAIN: &str = "Look again";
pub const LOOKING: &str = "Looking…";
pub const REPOS: &str = "Repos";
pub const ADD_REPO: &str = "Add repo";
pub const ADD: &str = "Add";
pub const REPO_ADDRESS: &str = "Repo address (its index.json)";
pub const TRUST_REPO: &str = "This repo isn’t nori’s. Its plugins can do what their pages say; add it only if you trust it.";

pub const REMOVE_PLUGIN_NOTE: &str = "Its settings go with it.";
pub const INSTALLED_OFF: &str = "It stays off until you turn it on.";

/// Asks before removing `name`.
pub fn remove_plugin(name: &str) -> String {
    format!("Remove {name}?")
}

/// Asks before installing `name` at `version`.
pub fn install_version(name: &str, version: &str) -> String {
    format!("Install {name} {version}?")
}

/// The install sheet's lines for `p`: what it says of itself, what it is and may reach, where it comes
/// from, and that it installs off.
pub fn preview_note(p: &nori_plugins::PluginPreview) -> Vec<String> {
    let c = &p.card;
    let from = p.repo.as_ref().map(|url| format!("From {}", repo_name(&nori_plugins::Repo { url: url.clone(), official: url == nori_plugins::OFFICIAL })));
    let unsigned = p.key.is_none().then(|| UNSIGNED.to_string());
    let bluetooth = c.bluetooth.then(|| REACHES_BLUETOOTH.to_string());
    c.about.iter().cloned().chain([plugin_is(&c.points, !c.overrides.is_empty()), plugin_reach(c)]).chain(bluetooth).chain(from).chain(unsigned).chain([INSTALLED_OFF.into()]).filter(|l| !l.is_empty()).collect()
}

pub const REACHES_BLUETOOTH: &str = "Reaches Bluetooth devices.";
pub const FILES: &str = "Files";
pub const ASSETS_ON_MOBILE: &str = "Download on mobile data";

/// Where a plugin's asset stands: "12 MB, not downloaded", "Downloading, 40%".
pub fn asset_state(a: &nori_plugins::AssetLine) -> String {
    use nori_plugins::AssetState;
    match a.state {
        AssetState::Missing => format!("{} MB, not downloaded", a.size.div_ceil(1 << 20)),
        AssetState::WaitingForWifi => "Waiting for Wi-Fi".into(),
        AssetState::Fetching { done, total } => format!("Downloading, {}%", (done * 100).checked_div(total).unwrap_or(0)),
        AssetState::Ready => "Downloaded".into(),
        AssetState::Failed => "Couldn’t download".into(),
    }
}
pub const BATTERY: &str = "Battery";
pub const NOISE_CONTROL: &str = "Noise control";

pub fn noise(n: nori_plugins::Noise) -> &'static str {
    use nori_plugins::Noise;
    match n {
        Noise::Off => "Off",
        Noise::Canceling => "Noise canceling",
        Noise::Transparency => "Transparency",
        Noise::Adaptive => "Adaptive",
    }
}

/// A device's name and its battery: "WH-1000XM5 · 80%".
pub fn with_level(name: &str, level: u8) -> String {
    format!("{name} · {level}%")
}

/// Each battery a device has: "80%", or "Left 80% · Right 70% · Case 50%"; empty when it said none.
pub fn battery(b: &nori_plugins::Battery) -> String {
    if let Some(s) = b.single {
        return format!("{s}%");
    }
    [("Left", b.left), ("Right", b.right), ("Case", b.case)].iter().filter_map(|(w, v)| v.map(|v| format!("{w} {v}%"))).collect::<Vec<_>>().join(" · ")
}

/// What a plugin is, from the extension points it fills and whether it overrides nori's values.
pub fn plugin_is(points: &[String], overrides: bool) -> String {
    let mut is: Vec<&str> = Vec::new();
    for p in points {
        let word = match p.as_str() {
            "provider" => "a music source",
            "sound-stage" => "a sound effect",
            "automix" => "a mixer",
            "lyrics" => "a lyrics source",
            "scrobbler" => "a scrobbler",
            "device-control" => "a device control",
            "hooks" | "deciders" | "slots" | "preparation" => "a playback helper",
            _ => continue,
        };
        if !is.contains(&word) {
            is.push(word);
        }
    }
    let mut said = if is.is_empty() { String::new() } else { format!("It’s {}.", is.join(", ")) };
    if overrides {
        said.push_str(if said.is_empty() { "It changes playback values." } else { " It changes playback values." });
    }
    said
}

/// What a plugin may reach and do: the hosts it sends to, nori's values it changes, the session, what
/// wakes it or nori for it, and the room it takes.
pub fn update_to(version: &str) -> String {
    format!("Update to {version}")
}

pub fn update_line(from: &str, to: &str) -> String {
    format!("{from} → {to}")
}

pub fn plugin_count(n: usize) -> String {
    count(n as u32, "plugin", "plugins")
}

pub fn plugin_reach(c: &nori_plugins::PluginCard) -> String {
    plugin_may(c).join(" ")
}

/// [`plugin_reach`], a line each.
pub fn plugin_may(c: &nori_plugins::PluginCard) -> Vec<String> {
    use nori_plugins::{Hook, Permission, WakeEvent};
    let mut said = Vec::new();
    match (c.net.is_empty(), c.lan) {
        (false, false) => said.push(format!("Connects to {}.", c.net.join(", "))),
        (false, true) => said.push(format!("Connects to {} and devices on your network.", c.net.join(", "))),
        (true, true) => said.push("Connects to devices on your network.".into()),
        (true, false) => {}
    }
    if !c.overrides.is_empty() {
        said.push(format!("Changes {}.", c.overrides.join(", ")));
    }
    for p in &c.permissions {
        said.push(match p {
            Permission::PlaybackRead => "Sees what plays and the queue.",
            Permission::PlaybackControl => "Controls playback and the queue.",
            Permission::LibraryRead => "Searches your library.",
            Permission::LibraryWrite => "Changes favorites and playlists.",
            Permission::Subsonic => "Uses your server's API as you.",
            Permission::Schedule => "Wakes now and then, never while music plays with the screen off.",
        }
        .into());
    }
    if c.hooks.contains(&Hook::LyricLine) {
        said.push("Wakes at each lyric line while music plays.".into());
    }
    if c.wakes.contains(&WakeEvent::BluetoothConnected) || c.wakes.contains(&WakeEvent::Boot) {
        said.push("Starts nori while it is closed (on a phone).".into());
    }
    if !c.commands.is_empty() {
        said.push(format!("Lets other apps run: {}.", c.commands.join(", ")));
    }
    if c.cache_bytes > 0 {
        said.push(format!("Keeps up to {} MB of cache.", c.cache_bytes.div_ceil(1 << 20)));
    }
    if c.asset_bytes > 0 {
        said.push(format!("Downloads {} MB of files.", c.asset_bytes.div_ceil(1 << 20)));
    }
    said
}

/// Why a plugin wasn't installed.
pub fn plugin_refused(r: &nori_plugins::Refusal) -> String {
    use nori_plugins::Refusal;
    let why = match r {
        Refusal::Unreadable(what) => format!("can’t be read ({what})"),
        Refusal::Api(v) => format!("made for plugin API {v}"),
        Refusal::Needs(what) => format!("needs: {}", what.join(", ")),
        Refusal::Id => "its id isn’t a reverse domain name".into(),
        Refusal::UnknownKey(key) => format!("changes {key}, which nori doesn’t have"),
        Refusal::Signature => "its signature doesn’t match its author’s key".into(),
        Refusal::OtherKey => "it isn’t signed by the author it was installed from (remove the plugin to install it)".into(),
    };
    format!("Not installed: {why}")
}

/// Why plugins are off at this start: the `names` of those nori stopped in, or none for a start that
/// didn't finish.
pub fn plugins_safe(safe: &nori_plugins::SafeStart, names: &[String]) -> String {
    match safe {
        nori_plugins::SafeStart::Crashed { .. } => format!("nori stopped while {} was running. It’s off now.", names.join(", ")),
        nori_plugins::SafeStart::AllOff => "nori didn’t finish starting last time. Plugins are off until it starts again.".into(),
    }
}

/// Plugins updated on their own: each one's name and new version.
pub fn plugins_updated(updated: &[(String, String)]) -> String {
    let each: Vec<String> = updated.iter().map(|(name, to)| format!("{name} to {to}")).collect();
    format!("Updated {}", each.join(", "))
}

/// A plugin's level at its step: whole numbers for whole steps.
pub fn level(v: f32, step: f32) -> String {
    let places = if step >= 1.0 { 0 } else { (-step.log10()).ceil().clamp(1.0, 3.0) as usize };
    format!("{v:.places$}")
}

/// A plugin's level with `decimals` digits, then its unit.
pub fn plugin_level(v: f64, decimals: u8, unit: Option<&str>) -> String {
    let d = decimals as usize;
    match unit {
        Some(u) => format!("{v:.d$} {u}"),
        None => format!("{v:.d$}"),
    }
}

/// A plugin's status row's value; `options` name a choice's.
pub fn plugin_value(v: &nori_plugins::Value, options: &[String]) -> String {
    use nori_plugins::Value;
    match v {
        Value::Switch(on) => (if *on { "On" } else { "Off" }).into(),
        Value::Choice(i) => options.get(*i as usize).cloned().unwrap_or_default(),
        Value::Flags(f) => format!("{f:#x}"),
        Value::Level(l) => level(*l as f32, 0.01),
        Value::Text(t) => t.clone(),
    }
}

/// A repo by its host; nori's own by name.
pub fn repo_name(r: &nori_plugins::Repo) -> String {
    if r.official {
        return "nori’s own".into();
    }
    host(&r.url).to_string()
}

pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// A repo that couldn't be read.
pub fn repo_failed(r: &nori_plugins::Repo) -> String {
    format!("Couldn’t reach {}", host(&r.url))
}

/// A group's head in the search field's preview.
pub fn preview_group(kind: nori_core::search::PreviewKind) -> &'static str {
    use nori_core::search::PreviewKind;
    match kind {
        PreviewKind::Artists => "ARTISTS",
        PreviewKind::Albums => "ALBUMS",
        PreviewKind::Playlists => "PLAYLISTS",
        PreviewKind::Songs => "SONGS",
    }
}

pub const ALL_RESULTS: &str = "All results";
/// Home's title.
pub const LISTEN_NOW: &str = "Listen now";
pub const MIXING: &str = "MIXING";

/// What to say about a device's AutoEQ curve, and the button's word.
pub fn curve_notice(n: &CurveNotice) -> (String, &'static str) {
    match n {
        CurveNotice::Offer { entry, .. } => (format!("{} connected. Use its AutoEQ curve?", entry.name), "Apply"),
        CurveNotice::Applied { curve, .. } => (format!("Using AutoEQ for {curve}"), "Undo"),
    }
}

pub fn count(n: impl Into<u64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The lyrics service lyrics came from, by name.
pub fn lyrics_origin(origin: &nori_core::lyrics_sources::LyricsOrigin, plugin_name: &dyn Fn(&str) -> String) -> String {
    use nori_core::lyrics_sources::LyricsOrigin;
    let said = match origin {
        LyricsOrigin::Server => "your server",
        LyricsOrigin::Binilyrics => "BiniLyrics",
        LyricsOrigin::BetterLyrics => "BetterLyrics",
        LyricsOrigin::Paxsenix | LyricsOrigin::PaxsenixMusixmatch | LyricsOrigin::PaxsenixSpotify => "PaxSenix",
        LyricsOrigin::LyricsPlus => "LyricsPlus",
        LyricsOrigin::Unison => "Unison",
        LyricsOrigin::Netease => "NetEase",
        LyricsOrigin::Kugou => "KuGou",
        LyricsOrigin::Lrclib => "LRCLIB",
        LyricsOrigin::YoutubeMusic => "YouTube Music",
        LyricsOrigin::Plugin { id } => return plugin_name(id),
    };
    said.to_string()
}

/// A Home shelf's title.
pub fn home_row(row: nori_core::settings::HomeRow) -> &'static str {
    use nori_core::settings::HomeRow;
    match row {
        HomeRow::Recent => "Recently played",
        HomeRow::Newest => "Recently added",
        HomeRow::Frequent => "Most played",
        HomeRow::Starred => "Favorites",
        HomeRow::Random => "Something random",
        HomeRow::Playlists => "Playlists",
        HomeRow::Pinned => "Pinned",
        HomeRow::TopSongs => "Top songs",
    }
}

/// One of the album grid's orders (`browse::album_sorts`).
pub fn album_sort(sort: nori_core::browse::AlbumSort) -> &'static str {
    use nori_core::browse::AlbumSort;
    match sort {
        AlbumSort::ByName => "Title",
        AlbumSort::ByArtist => "Artist",
        AlbumSort::ArtistRows => "Artist",
        AlbumSort::Newest => "Added",
        AlbumSort::Recent => "Played",
        AlbumSort::Frequent => "Most played",
        AlbumSort::Starred => "Favorites",
        AlbumSort::ByYear => "Year",
        AlbumSort::Random => "Random",
        AlbumSort::ByGenre => "Genres",
    }
}

pub fn songs(n: usize) -> String {
    count(n as u64, "song", "songs")
}

pub fn albums(n: u32) -> String {
    count(n, "album", "albums")
}

/// An artist page's list head: "14 releases · 162 songs", or the songs its page lists where it has no releases.
pub fn artist_caption(releases: usize, songs_in_releases: usize, songs_listed: usize) -> String {
    if releases == 0 {
        return if songs_listed > 0 { songs(songs_listed) } else { String::new() };
    }
    let mut parts = vec![count(releases as u64, "release", "releases")];
    if songs_in_releases > 0 {
        parts.push(songs(songs_in_releases));
    }
    parts.join(" · ")
}

/// An artist's shelf heading: "Albums", "EPs", or a type nori does not know by its tag ("Mixtapes").
pub fn release_shelf(g: &ReleaseGroup) -> String {
    match g.kind {
        ReleaseKind::Album => "Albums".into(),
        ReleaseKind::Ep => "EPs".into(),
        ReleaseKind::Single => "Singles".into(),
        ReleaseKind::Live => "Live".into(),
        ReleaseKind::Compilation => "Compilations".into(),
        ReleaseKind::Soundtrack => "Soundtracks".into(),
        ReleaseKind::Remix => "Remixes".into(),
        ReleaseKind::Other => "Others".into(),
        ReleaseKind::Tagged if g.tag.ends_with('s') => g.tag.clone(),
        ReleaseKind::Tagged => format!("{}s", g.tag),
    }
}

/// "12 songs · 48:10".
pub fn songs_caption(n: usize, seconds: u64) -> String {
    format!("{} · {}", songs(n), numbers::clock(seconds as i64, false))
}

/// "FLAC 24/96.0", "MP3 320 kbps".
pub fn quality(s: &Song) -> Option<String> {
    numbers::quality(&s.suffix, s.bit_depth, s.sampling_rate, s.bit_rate)
}

pub fn net_error(e: &NetError) -> String {
    let said = match e {
        NetError::Transport { kind, .. } => match kind {
            FailureKind::Metered => "This server is set to Wi-Fi only",
            FailureKind::UnknownHost => "Server not found. Check the address.",
            FailureKind::Connect => "Nothing is answering at that address. Is the port right, and is the server running?",
            FailureKind::Timeout => "The server did not answer in time.",
            FailureKind::Tls => "The server's certificate was not accepted.",
            FailureKind::Cleartext => "Cleartext HTTP was refused; use https://",
            _ => "",
        },
        NetError::Http { status } => return format!("HTTP {status}"),
        NetError::Api { code: 40, .. } => "Wrong user name or password.",
        NetError::Api { code: 41, .. } => "This server does not support token authentication.",
        NetError::Api { code: 50, .. } => "This user is not allowed to do that.",
        NetError::Api { reason, .. } => return reason.clone(),
        NetError::Parse { .. } => "That address answered, but not like a Subsonic server. Check the URL (and any reverse-proxy path).",
        NetError::Db { reason } => return format!("database: {reason}"),
    };
    if said.is_empty() {
        let NetError::Transport { detail, .. } = e else { return String::new() };
        return detail.clone().unwrap_or_default();
    }
    said.to_string()
}

/// A line of a song's menu (menus.rs's `song_menu`).
pub fn song_action(a: &nori_core::menus::SongAction) -> String {
    use nori_core::menus::SongAction as A;
    match a {
        A::Favourite { on: true } => "Add to favorites".into(),
        A::Favourite { on: false } => "Remove from favorites".into(),
        A::PlayNext => "Play next".into(),
        A::AddToQueue => "Add to queue".into(),
        A::AddToPlaylist => "Add to playlist…".into(),
        A::RemoveDownload => "Remove download".into(),
        A::StopDownload => "Stop download".into(),
        A::Download => "Download".into(),
        A::GoToAlbum { .. } => "Go to album".into(),
        A::GoToArtist { name, named: true, .. } => format!("Go to {name}"),
        A::GoToArtist { named: false, .. } => "Go to artist".into(),
        A::AddToLibrary => "Add to library".into(),
        A::Lyrics => "Lyrics".into(),
        A::SleepTimer => "Sleep timer…".into(),
        A::StartRadio => "Start radio from this song".into(),
        A::InstantMix => "Instant mix".into(),
        A::ExcludeFromMixes => "Exclude from mixes".into(),
        A::Share => "Share link".into(),
        A::Details => "Details".into(),
        A::StartJam => "Start a jam".into(),
    }
}

/// One of the sleep timer's choices: "30 minutes", "End of track", "After 3 songs", or "Off" (all zeros).
pub fn sleep_choice(c: &nori_core::menus::SleepChoice) -> String {
    match c {
        c if c.end_of_track => "End of track".into(),
        c if c.songs > 0 => format!("After {} songs", c.songs),
        c if c.minutes > 0 => format!("{} minutes", c.minutes),
        _ => "Off".into(),
    }
}

/// Where lyrics came from and how finely they are timed: "From LRCLIB · timed line by line".
pub fn lyrics_from(origin: &nori_core::lyrics_sources::LyricsOrigin, synced: bool, word_timed: bool, plugin_name: &dyn Fn(&str) -> String) -> String {
    let timed = if !synced { "not timed" } else if word_timed { "timed word by word" } else { "timed line by line" };
    format!("From {} · {timed}", lyrics_origin(origin, plugin_name))
}

/// Who moved the lyrics' timing, and by how much.
pub fn lyrics_timing(t: nori_core::LyricsTiming) -> String {
    use nori_core::LyricsTiming as T;
    match t {
        T::NotChecked => "Not checked against the singing yet".into(),
        T::DoesNotFit => "Checked against the singing: does not fit".into(),
        T::Unsure => "Checked against the singing: can't tell".into(),
        T::Automatic { ms: 0 } => "Checked against the singing: on time".into(),
        T::Automatic { ms } => format!("Adjusted automatically by {ms:+} ms"),
        T::Yours { ms, .. } => format!("Adjusted by you: {ms:+} ms"),
    }
}

/// The lyrics' shift, "+0.25 s".
pub fn lyrics_shift(ms: i64) -> String {
    format!("{:+.2} s", ms as f64 / 1000.0)
}

pub const EXCLUDED_FROM_MIXES: &str = "Left out of mixes";
pub const SHARE_FAILED: &str = "The server made no link";

pub fn added_to_playlist(name: &str) -> String {
    format!("Added to {name}")
}

pub fn playlist_created(name: &str) -> String {
    format!("Made {name}")
}

pub fn removed_from_playlist(name: &str) -> String {
    format!("Removed from {name}")
}

pub fn playlist_renamed(name: &str) -> String {
    format!("Renamed to {name}")
}

pub fn playlist_deleted(name: &str) -> String {
    format!("Deleted {name}")
}

pub const REMOVE_FROM_PLAYLIST: &str = "Remove from playlist";

pub fn playlist_failed(e: &str) -> String {
    format!("The playlist could not be changed: {e}")
}

/// Everything the server said about one file, in the order the details sheet shows it, blanks left out
/// (as Android's `Say.trackInfo`).
pub fn details(s: &nori_core::Song) -> Vec<(&'static str, String)> {
    use nori_core::numbers::{fixed, megabytes};
    let join = |parts: Vec<String>, by: &str| parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(by);
    let some = |on: bool, v: String| if on { v } else { String::new() };
    let artists = join(s.artists.iter().map(|a| a.name.clone()).collect(), ", ");
    let gain = s.replay_gain.as_ref().map_or_else(String::new, |g| {
        join(vec![
            g.track_gain.map_or_else(String::new, |v| format!("track {} dB", fixed(v as f64, 2, true, "."))),
            g.album_gain.map_or_else(String::new, |v| format!("album {} dB", fixed(v as f64, 2, true, "."))),
            g.track_peak.map_or_else(String::new, |v| format!("peak {}", fixed(v as f64, 3, false, "."))),
        ], " · ")
    });
    let khz = fixed(s.sampling_rate as f64 / 1000.0, 1, false, ".");
    let khz = khz.strip_suffix(".0").unwrap_or(&khz).to_string();
    let rows = [
        ("Title", s.title.clone()),
        ("Artist", if artists.is_empty() { s.artist.clone() } else { artists }),
        ("Album", s.album.clone()),
        ("Track", join(vec![some(s.disc_number > 0, format!("disc {}", s.disc_number)), some(s.track > 0, format!("track {}", s.track))], ", ")),
        ("Year", some(s.year > 0, s.year.to_string())),
        ("Genre", s.genre.clone().unwrap_or_default()),
        ("Duration", nori_core::numbers::clock(s.duration as i64, false)),
        ("Format", join(vec![s.suffix.to_uppercase(), s.content_type.clone()], " · ")),
        ("Quality", join(vec![
            some(s.bit_rate > 0, format!("{} kbps", s.bit_rate)),
            some(s.sampling_rate > 0, format!("{khz} kHz")),
            some(s.bit_depth > 0, format!("{} bit", s.bit_depth)),
            some(s.channel_count > 0, format!("{} ch", s.channel_count)),
        ], " · ")),
        ("Size", some(s.size > 0, megabytes(s.size as i64, "."))),
        ("ReplayGain", gain),
        ("BPM", some(s.bpm > 0, s.bpm.to_string())),
        ("Plays (server)", some(s.play_count > 0, s.play_count.to_string())),
        ("Last played", s.played.as_deref().map(|p| p.chars().take(16).collect::<String>().replace('T', " ")).unwrap_or_default()),
        ("Added", s.created.as_deref().map(|c| c.chars().take(10).collect()).unwrap_or_default()),
        ("Path", s.path.clone().unwrap_or_default()),
        ("MusicBrainz", s.music_brainz_id.clone().unwrap_or_default()),
        ("Comment", s.comment.clone().unwrap_or_default()),
        ("Id", s.id.clone()),
    ];
    rows.into_iter().filter(|(_, v)| !v.trim().is_empty()).collect()
}

/// A mix's name, as its tile and page say it.
pub fn mix_name(n: nori_core::mixes::board::MixName) -> &'static str {
    use nori_core::mixes::board::MixName as M;
    match n {
        M::Favourites => "Favorites",
        M::QuickPicks => "Quick picks",
        M::Discover => "Discover",
        M::DiscoverWeekly => "Discover Weekly",
        M::ListenAgain => "Listen again",
        M::Top => "Your top songs",
    }
}

/// Under a mix's name: what it is made from.
pub fn mix_caption(favourites: bool) -> &'static str {
    if favourites { "Your favorites" } else { "Made for you" }
}


/// The devices panel's first row, which brings the music back.
pub const THIS_COMPUTER: &str = "This computer";

/// The kinds' words, for telling devices of one name apart.
pub fn kind_words() -> nori_core::remote::KindWords {
    nori_core::remote::KindWords { phone: "phone".into(), desktop: "computer".into(), terminal: "terminal".into(), guest: "guest".into() }
}

/// The player's strip while another device plays.
pub fn playing_on(device: &str) -> String {
    format!("Playing on {device}")
}

/// A nori device's glyph (app.slint's `Glyphs.device`): a computer, a phone, a terminal, a guest.
pub fn device_glyph(kind: nori_core::remote::wire::DeviceKind) -> i32 {
    use nori_core::remote::wire::DeviceKind;
    match kind {
        DeviceKind::Desktop => 0,
        DeviceKind::Phone => 1,
        DeviceKind::Terminal => 2,
        DeviceKind::Guest => 3,
    }
}

/// What the output button calls this computer's output, its port and the name the device gave.
pub fn output_name(port: nori_core::profiles::OutputPort, name: Option<String>) -> String {
    use nori_core::profiles::OutputPort;
    match port {
        OutputPort::Speaker => "Speakers".into(),
        OutputPort::Wired => "Wired headphones".into(),
        OutputPort::Usb => name.unwrap_or_else(|| "DAC".into()),
        OutputPort::Bluetooth => name.unwrap_or_else(|| "device".into()),
        OutputPort::Other => name.unwrap_or_else(|| "Other output".into()),
    }
}

/// A place the music can play, as the devices panel lists it: this computer (`d` None) or a device of the
/// account with what it plays and its last refusal; ticked while the music plays there.
/// The id of the devices panel's row for a plugin's cast.
pub const CAST_ROW: &str = "nori-plugin-cast";

/// A plugin's cast as the devices panel lists it: playing there, its volume when it says.
pub fn cast_row(c: &nori_plugins::Cast) -> crate::DeviceRow {
    let what = match c.kind {
        nori_plugins::CastKind::Sink => "Casting",
        nori_plugins::CastKind::Target => "Playing there",
    };
    let line = match c.volume {
        Some(v) => format!("{what} · {v} %"),
        None => what.to_string(),
    };
    crate::DeviceRow { id: CAST_ROW.into(), name: c.device.clone().into(), line: line.into(), active: true, ..Default::default() }
}

pub fn device_row(d: Option<&nori_core::remote::RemoteDevice>, active: bool) -> crate::DeviceRow {
    use nori_core::remote::wire::Refusal;
    let Some(d) = d else {
        return crate::DeviceRow { name: THIS_COMPUTER.into(), active, ..Default::default() };
    };
    let line = d.now.as_ref().map_or_else(|| "Not playing".to_string(), |e| format!("{} · {}", e.title, e.artist));
    let note = match d.refused {
        Some(Refusal::Stale) => "The queue changed there. Try again.",
        Some(Refusal::NotAllowed) => "That device said no.",
        Some(Refusal::Unknown) => "That is no longer there.",
        Some(Refusal::TooMany) => "Too many songs waiting.",
        Some(Refusal::OtherSource) => "That device plays from another source.",
        None => "",
    };
    crate::DeviceRow {
        id: d.id.clone().into(),
        name: d.name.clone().into(),
        line: line.into(),
        kind: device_glyph(d.kind),
        active,
        note: note.into(),
    }
}

/// Who listens in the jam: "2 listening".
pub fn jam_listening(n: usize) -> String {
    match n {
        0 => "No one yet".into(),
        n => format!("{n} listening"),
    }
}

/// The jam as the player's strip says it: "Jam · 2 listening".
pub fn jam_strip(n: usize) -> String {
    format!("Jam · {}", jam_listening(n).to_lowercase())
}

/// A guest's strip: "Jam · Desk · 2 listening".
pub fn jam_guest_strip(host: &str, n: usize) -> String {
    format!("Jam · {host} · {}", jam_listening(n).to_lowercase())
}

/// Under a song a guest asked for.
pub fn jam_waiting(host: &str) -> String {
    format!("Waiting for {host}")
}

/// Beside a song a guest asked for, in the lists.
pub const ASKED: &str = "Asked";
pub const JAM_GUEST: &str = "Jam";
pub const NOT_AN_INVITE: &str = "That is not a jam invite. Paste the whole link the host sent you.";
pub const JAM_LEFT: &str = "You left the jam";
/// Beside a jam guest's song while it paused its own listening: play joins the jam again.
pub const JAM_PAUSED_HERE: &str = "Paused here · Jam still playing";

pub fn jam_ended(host: Option<&str>) -> String {
    host.map_or("The jam ended".into(), |h| format!("{h} ended the jam"))
}

pub const JAM_OWN: &str = "That's your own jam";
pub const JAM_INVITE_ENDED: &str = "This jam has ended";

/// Why a jam was not joined.
pub fn jam_join_failed(e: &nori_host::JoinError) -> String {
    match e {
        nori_host::JoinError::Own => JAM_OWN.into(),
        nori_host::JoinError::Ended => JAM_INVITE_ENDED.into(),
        nori_host::JoinError::Failed(e) => format!("Couldn't join the jam ({})", net_error(e)),
    }
}

pub fn jam_asked(from: &str) -> String {
    format!("Asked by {from}")
}

/// Under a provider's song asked for: accepting it makes the server download it.
pub const JAM_DOWNLOADS: &str = "Downloaded to your server if accepted";

pub fn jam_role(role: nori_core::remote::wire::Role) -> &'static str {
    use nori_core::remote::wire::Role;
    match role {
        Role::Host => "Host",
        Role::Admin => "Admin",
        Role::Guest => "Guest",
    }
}

/// Why a guest asking to listen along does not, if it does not.
pub fn jam_along(l: nori_core::remote::Listening) -> &'static str {
    use nori_core::remote::Listening;
    match l {
        Listening::Watching | Listening::Playing => "",
        Listening::HostOff => "The host doesn't let guests listen along right now. You can still ask for songs.",
        Listening::ServerOff => "This server doesn't let guests listen along. You can still ask for songs.",
        Listening::Refused => "The host's songs don't play on this device: it's on another network. You can still ask for songs.",
    }
}

pub const JAM_UNSUPPORTED: &str = "Your server doesn't support jams yet. They need octo-fiesta with nori support in front of it.";
pub const JAM_FAILED: &str = "Couldn't start the jam. Jams need octo-fiesta in front of your server.";
/// Under the invite, when the server's address is a home network's.
pub fn jam_home_only(address: &str) -> String {
    format!("This invite only works on your home network ({address}). Set a public address in the server settings for guests elsewhere.")
}

/// The undo pill after a song is taken out of the queue.
/// Where the offered song stands in its queue, on Home's Continue card: "Artist · 3 of 12".
pub fn offer_place(artist: &str, at: usize, of: usize) -> String {
    format!("{artist} · {at} of {of}")
}

pub fn removed(title: &str) -> String {
    format!("Removed “{title}”")
}

pub const LINK_COPIED: &str = "Link copied";

pub const PAIR_FAILED: &str = "Pairing failed";

/// Why pasted cookies did not sign the account in.
pub fn sign_in_failed(r: &nori_core::providers::SignInResult) -> String {
    use nori_core::providers::SignInResult as R;
    match r {
        R::SignedIn { name } => format!("Signed in as {name}"),
        R::NoSession => "These aren’t a signed-in session’s cookies.".into(),
        R::Refused => "These cookies weren’t accepted. Sign in again in a private window and copy them anew.".into(),
        R::Unreachable { reason } => format!("Couldn’t be reached ({reason})."),
    }
}

pub const SIGN_IN_WINDOW_FAILED: &str = "The sign-in window could not open here. Paste cookies from a browser instead.";

/// A song's title as shown: "Untitled" for one that has none (some uploads have a blank one).
pub fn title(t: &str) -> &str {
    if t.trim().is_empty() { "Untitled" } else { t }
}

/// The profile in use as the settings menu heads it: who (the account, else the profile's name, else its
/// service), then where in grey (the server's address, or the service, or that no one is signed in).
pub fn profile_lines(p: &nori_core::settings::SavedServer, kinds: &nori_core::providers::Kinds) -> (String, String) {
    let service = provider_of(kinds, &p.kind).name;
    let place = nori_core::settings::label(&p.name, &p.url);
    let signs_in = kinds.providers().iter().any(|i| i.kind == p.kind && !i.sign_in.is_empty());
    match (p.user.is_empty(), signs_in) {
        (false, true) => (p.user.clone(), service),
        (true, true) => (if place.is_empty() { service } else { place }, "Not signed in".into()),
        (false, false) => (p.user.clone(), place),
        (true, false) => (if place.is_empty() { service } else { place }, String::new()),
    }
}

/// The AutoEQ search field's placeholder, with how many headphones the list has.
pub fn autoeq_search(count: u32) -> String {
    format!("Search {count} headphones")
}

/// An AutoEQ curve's line under its name: who measured it, the form and the target, whichever are known.
pub fn autoeq_caption(e: &nori_core::AutoEqEntry) -> String {
    [e.source.as_str(), e.form.as_str(), e.target.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

pub fn autoeq_applied(name: &str) -> String {
    format!("The equalizer is now {name}’s AutoEQ curve.")
}

/// Over the curves suggested for the output playing.
pub fn autoeq_for(output: &str) -> String {
    format!("For {output}")
}

pub const AUTOEQ_NO_CURVE: &str = "AutoEQ has no curve for these headphones.";

/// A saved profile's line: the devices it is used on by itself.
pub fn profile_use(outputs: &[String]) -> String {
    if outputs.is_empty() { String::new() } else { format!("Used on {}", outputs.join(", ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_menu_names_who_and_where() {
        use nori_core::settings::SavedServer;
        let p = |kind, name: &str, url: &str, user: &str| SavedServer { kind, name: name.into(), url: url.into(), user: user.into(), ..Default::default() };
        let lines = |s: SavedServer| profile_lines(&s, &nori_core::providers::Kinds::builtin());
        assert_eq!(lines(p(ServerKind::Subsonic, "", "https://music.example.com/x", "ann")), ("ann".into(), "music.example.com".into()));
        assert_eq!(lines(p(ServerKind::Plugin { id: "dev.example.music".into() }, "", "", "")), ("dev.example.music".into(), "".into()), "a plugin's not here is named by its id");
        assert_eq!(lines(p(ServerKind::Subsonic, "Home", "http://h", "")), ("Home".into(), "".into()));
    }

    #[test]
    fn lyrics_name_their_service() {
        use nori_core::lyrics_sources::LyricsOrigin;
        let named = |id: &str| if id == "fm.nori.test" { "Test".to_string() } else { id.to_string() };
        assert_eq!(lyrics_from(&LyricsOrigin::Lrclib, true, false, &named), "From LRCLIB · timed line by line");
        assert_eq!(lyrics_from(&LyricsOrigin::Plugin { id: "fm.nori.test".into() }, false, false, &named), "From Test · not timed", "a plugin by its name");
    }

    #[test]
    fn the_install_sheet_says_what_a_plugin_is_and_may_reach() {
        let card = nori_plugins::PluginCard {
            id: "a.one".into(),
            version: "1.0.0".into(),
            name: "One".into(),
            about: Some("Counts letters".into()),
            enabled: false,
            repo: None,
            held: false,
            points: vec!["slots".into()],
            overrides: Vec::new(),
            net: vec!["a.example".into()],
            lan: true,
            bluetooth: false,
            permissions: Vec::new(),
            hooks: vec![nori_plugins::Hook::LyricLine],
            wakes: Vec::new(),
            commands: Vec::new(),
            cache_bytes: 0,
            asset_bytes: 3 << 20,
            rows: Vec::new(),
        };
        let mut p = nori_plugins::PluginPreview { card, file: String::new(), repo: Some(nori_plugins::OFFICIAL.into()), key: Some("ab".into()) };
        assert_eq!(preview_note(&p), ["Counts letters", "It’s a playback helper.", "Connects to a.example and devices on your network. Wakes at each lyric line while music plays. Downloads 3 MB of files.", "From nori’s own", "It stays off until you turn it on."]);
        p.key = None;
        assert!(preview_note(&p).contains(&"Unsigned".to_string()));
    }

    #[test]
    fn formats_counts() {
        assert_eq!((songs(1), songs_caption(2, 200)), ("1 song".into(), "2 songs · 3:20".into()));
    }
}
