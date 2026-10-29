//! The terminal client's English: formatted times, sizes, decibels, counts, captions and messages.

use nori_core::beat_model::BeatFailure;
use nori_core::lyrics_sources::LyricsOrigin;
use nori_core::numbers::{self, POINT};
use nori_core::profiles::CurveNotice;
use nori_core::providers::SignInResult;
use nori_core::settings::{BandMark, EqBypass, ServerKind};
use nori_core::transport::{FailureKind, NetError};
use nori_core::remote::wire::{DeviceKind, Refusal, Role};
use nori_core::remote::{KindWords, Listening, RemoteDevice};
use nori_core::{AlbumDetail, Playlist, PlaylistDetail, PresetKind, Song};

/// What the terminal says of a kind of profile (nori-core's providers): the only words a provider
/// needs here.
pub struct ProviderWords {
    /// What a profile of the kind is called without a name of its own.
    pub name: String,
    /// Under a profile that names no user.
    pub no_user: &'static str,
}

/// The words of `kind`; `plugin_name` is a plugin kind's name in its own words.
pub fn provider(kind: &ServerKind, plugin_name: Option<String>) -> ProviderWords {
    match kind {
        ServerKind::Subsonic => ProviderWords { name: "Your server".into(), no_user: "API key" },
        ServerKind::Plugin { id } => ProviderWords { name: plugin_name.unwrap_or_else(|| id.clone()), no_user: "no account" },
    }
}

/// The words of `kind`, a plugin's named in its own words.
pub fn provider_of(kinds: &nori_core::providers::Kinds, kind: &ServerKind) -> ProviderWords {
    provider(kind, kinds.plugin_name(kind.clone(), "en".into()))
}

/// How a sign-in to a profile of the kind called `name` went.
pub fn sign_in(name: &str, r: &SignInResult) -> String {
    match r {
        SignInResult::SignedIn { name: user } => format!("Signed in to {name} as {user}"),
        SignInResult::NoSession => "That isn't a signed-in session".into(),
        SignInResult::Refused => format!("{name} didn't accept this sign-in"),
        SignInResult::Unreachable { reason } => format!("Couldn't reach {name} ({reason}); nothing changed"),
    }
}

pub const INSTALL_PLUGIN: &str = "Install a plugin";
pub const LOGIN_MISSING: &str = "Fill in what's asked first";
pub const LOGIN_KEYS: &str = "tab next field · enter connect · ctrl p plugins · esc back";
pub const LOGIN_KIND_KEYS: &str = "← → kind · tab next field · enter connect · ctrl p plugins · esc back";
pub const PLUGINS: &str = "Plugins";
pub const LOGIN_KIND: &str = "Kind";
pub const FROM_A_PLUGIN: &str = "From a plugin";
pub const PLUGIN_PATH: &str = "A .noriplugin file or a plugin folder";
pub const NO_PLUGINS: &str = "No plugins installed.";
pub const PLUGIN_VERSION: &str = "Version";
pub const SECRET_SET: &str = "Set";
pub const PLUGIN_INSTALLED: &str = "Plugin installed";
pub const GET_PLUGINS: &str = "Get plugins";
pub const LOOK_FOR_PLUGINS: &str = "Look for plugins";
pub const LOOK_AGAIN: &str = "Look again";
pub const LOOKING: &str = "Looking…";
pub const NOTHING_OFFERED: &str = "No plugins offered.";
pub const INSTALL: &str = "Install";
pub const UPDATE: &str = "Update";
pub const INSTALLED: &str = "Installed";
pub const REPOS: &str = "Repos";
pub const KEEP_VERSION: &str = "Keep this version";
pub const VERSIONS: &str = "Versions";
pub const SOURCE: &str = "Source";

/// Plugins updated on their own: each one's name and new version.
pub fn plugins_updated(updated: &[(String, String)]) -> String {
    let each: Vec<String> = updated.iter().map(|(name, to)| format!("{name} to {to}")).collect();
    format!("Updated {}", each.join(", "))
}
pub const ADD_REPO: &str = "Add repo";
pub const REPO_ADDRESS: &str = "Repo address (its index.json)";
pub const TRUST_REPO: &str = "This repo isn't nori's. Its plugins can do what their pages say; add it only if you trust it.";

/// A question's keys: enter does `yes`.
pub fn confirm_keys(yes: &str) -> String {
    format!("enter {} · esc cancel", yes.to_lowercase())
}

pub const ADD: &str = "Add";
pub const REMOVE: &str = "Remove";

/// Asks before removing plugin `name`.
pub fn remove_plugin(name: &str) -> String {
    format!("Remove {name}?")
}
pub const REMOVE_PLUGIN_NOTE: &str = "Its settings go with it.";

/// Asks before installing `name` at `version`.
pub fn install_version(name: &str, version: &str) -> String {
    format!("Install {name} {version}?")
}

/// A line plugin `name` adds to a song's info, with where its link goes.
pub fn info_line(name: &str, l: &nori_plugins::InfoLine) -> String {
    match &l.link {
        Some(url) => format!("{name} · {} · {url}", l.words),
        None => format!("{name} · {}", l.words),
    }
}

/// Where a plugin to install comes from: the repo at `url`, which will update it.
pub fn from_repo(url: &str) -> String {
    if url == nori_plugins::OFFICIAL {
        return "From nori's own repo, which keeps it up to date.".into();
    }
    format!("From {}, which keeps it up to date.", host(url))
}
pub const INSTALLED_OFF: &str = "It stays off until you turn it on.";

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
    let mut said = match is.len() {
        0 => String::new(),
        _ => format!("It's {}.", is.join(", ")),
    };
    if overrides {
        said.push_str(if said.is_empty() { "It changes playback values." } else { " It changes playback values." });
    }
    said
}

/// What a plugin may reach: the hosts it sends to and nori's values it changes.
pub fn plugin_reach(net: &[String], lan: bool, overrides: &[String]) -> String {
    let mut said = Vec::new();
    match (net.is_empty(), lan) {
        (false, false) => said.push(format!("Connects to {}.", net.join(", "))),
        (false, true) => said.push(format!("Connects to {} and devices on your network.", net.join(", "))),
        (true, true) => said.push("Connects to devices on your network.".into()),
        (true, false) => {}
    }
    if !overrides.is_empty() {
        said.push(format!("Changes {}.", overrides.join(", ")));
    }
    if said.is_empty() { "Connects to nothing.".into() } else { said.join(" ") }
}

/// A repo by its host; nori's own by name.
pub fn repo_name(r: &nori_plugins::Repo) -> String {
    if r.official {
        return "nori's own".into();
    }
    host(&r.url).to_string()
}

/// The host in `url`.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// A repo that couldn't be read.
pub fn repo_failed(r: &nori_plugins::Repo) -> String {
    format!("Couldn't reach {}", host(&r.url))
}
pub const NOT_SET: &str = "Not set";

/// Why a plugin wasn't installed.
pub fn plugin_refused(r: &nori_plugins::Refusal) -> String {
    use nori_plugins::Refusal;
    let why = match r {
        Refusal::Unreadable(what) => format!("can't be read ({what})"),
        Refusal::Api(v) => format!("made for plugin API {v}"),
        Refusal::Needs(what) => format!("needs: {}", what.join(", ")),
        Refusal::Id => "its id isn't a reverse domain name".into(),
        Refusal::UnknownKey(key) => format!("changes {key}, which nori doesn't have"),
    };
    format!("Not installed: {why}")
}

/// A plugin's level with `decimals` digits, its unit after it.
pub fn level(v: f64, decimals: u8, unit: Option<&str>) -> String {
    let places = decimals as usize;
    match unit {
        Some(u) => format!("{v:.places$} {u}"),
        None => format!("{v:.places$}"),
    }
}

/// A plugin's status row's value.
pub fn plugin_value(v: &nori_plugins::Value, r: &nori_plugins::PluginRow) -> String {
    use nori_plugins::Value;
    match v {
        Value::Switch(on) => on_off(*on).into(),
        Value::Choice(i) => r.options.get(*i as usize).cloned().unwrap_or_default(),
        Value::Flags(f) => format!("{f:#x}"),
        Value::Level(l) => level(*l, r.decimals, r.unit.as_deref()),
        Value::Text(t) => t.clone(),
    }
}

/// One shelf of a feed: its title and what it holds.
pub fn feed_shelf(f: &nori_core::FeedShelf) -> String {
    let mut parts = Vec::new();
    for (n, one, many) in [(f.songs.len(), "song", "songs"), (f.albums.len(), "album", "albums"), (f.playlists.len(), "playlist", "playlists"), (f.artists.len(), "artist", "artists")] {
        if n > 0 {
            parts.push(count(n as u64, one, many));
        }
    }
    format!("{}: {}", f.title, parts.join(", "))
}

/// A Home shelf's title.
pub fn home_row(row: nori_core::settings::HomeRow) -> &'static str {
    use nori_core::settings::HomeRow;
    match row {
        HomeRow::Recent => "Recently played",
        HomeRow::Newest => "Recently added",
        HomeRow::Frequent => "Most played albums",
        HomeRow::Starred => "Favorite albums",
        HomeRow::Random => "Random",
        HomeRow::Playlists => "Playlists",
        HomeRow::Pinned => "Favorite playlists",
        HomeRow::TopSongs => "Most played songs",
    }
}

/// The repeat mode `mode` as said when it is chosen.
pub fn repeat(mode: u8) -> &'static str {
    match mode {
        nori_core::rules::REPEAT_ONE => "Repeat one",
        nori_core::rules::REPEAT_ALL => "Repeat all",
        _ => "Repeat off",
    }
}

/// A playback position or length in ms: "3:07".
pub fn clock(ms: i64) -> String {
    numbers::clock(ms / 1000, false)
}

/// "1 song", "12 songs".
pub fn count(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A list's caption: "12 songs · 48:10".
pub fn songs_caption(count: u32, seconds: u64) -> String {
    format!("{} · {}", self::count(count as u64, "song", "songs"), numbers::clock(seconds as i64, false))
}

/// A record's format from one of its songs: "FLAC 24/96.0", "MP3 320 kbps".
pub fn quality(s: &Song) -> Option<String> {
    numbers::quality(&s.suffix, s.bit_depth, s.sampling_rate, s.bit_rate)
}

/// An album page's caption: "2019 · 12 songs · 48:10 · FLAC 16/44.1 · explicit".
pub fn album_caption(d: &AlbumDetail) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(5);
    if d.album.year > 0 {
        parts.push(d.album.year.to_string());
    }
    parts.push(count(d.songs.len() as u64, "song", "songs"));
    parts.push(numbers::clock(d.seconds as i64, false));
    if let Some(q) = d.songs.first().and_then(quality) {
        parts.push(q);
    }
    if d.album.explicit_status == nori_core::model::ExplicitStatus::Explicit {
        parts.push("explicit".into());
    }
    parts.join(" · ")
}

/// An artist page's caption: "14 releases · 162 songs", or its songs where it lists no albums.
pub fn artist_caption(d: &nori_core::ArtistDetail) -> String {
    let songs: u32 = d.albums.iter().map(|a| a.song_count).sum();
    let mut parts = Vec::with_capacity(2);
    if !d.albums.is_empty() {
        parts.push(count(d.albums.len() as u64, "release", "releases"));
    }
    if songs > 0 {
        parts.push(count(songs as u64, "song", "songs"));
    }
    if parts.is_empty() && !d.songs.is_empty() {
        parts.push(count(d.songs.len() as u64, "song", "songs"));
    }
    parts.join(" · ")
}

/// A playlist page's caption: "12 songs · 48:10".
pub fn playlist_caption(d: &PlaylistDetail) -> String {
    songs_caption(d.songs.len() as u32, d.seconds)
}

/// Beside a playlist's name: "12 songs", or its own line (a provider's, who made it) when not counted.
pub fn playlist_line(p: &Playlist) -> String {
    match &p.comment {
        Some(line) if p.song_count == 0 => line.clone(),
        _ => count(p.song_count as u64, "song", "songs"),
    }
}

/// Under an artist's name: "12 albums".
pub fn albums(n: u32) -> String {
    count(n as u64, "album", "albums")
}

/// How far the lyrics are nudged: "+0.5 s".
pub fn nudge(ms: i64) -> String {
    format!("{} s", numbers::nudge(ms, POINT))
}

/// A parametric band's kind.
pub fn band_kind(kind: nori_core::EqKind) -> &'static str {
    use nori_core::EqKind as K;
    match kind {
        K::Peaking => "Peak",
        K::LowShelf => "Low shelf",
        K::HighShelf => "High shelf",
        K::LowPass => "Low pass",
        K::HighPass => "High pass",
        K::BandPass => "Band pass",
        K::Notch => "Notch",
        K::AllPass => "All pass",
        K::LowShelfSlope => "Low shelf (slope)",
        K::HighShelfSlope => "High shelf (slope)",
    }
}

/// Which side a parametric band is heard on.
pub fn band_side(channel: nori_core::settings::BandChannel) -> &'static str {
    use nori_core::settings::BandChannel as C;
    match channel {
        C::Both => "both sides",
        C::Left => "left only",
        C::Right => "right only",
    }
}

/// The band chosen on the curve, in a line: "250  +6.5 dB", or a parametric band's kind, width and side.
pub fn band_line(p: &nori_core::settings::StoredPrefs, band: usize) -> String {
    use nori_core::EqKind as K;
    if p.eq_mode == nori_core::settings::EqMode::Graphic {
        let label = nori_core::dsp::graphic_bands(p.eq_graphic.len() as u32).get(band).map_or(0.0, |b| b.label_hz);
        return format!("{}  {} dB", numbers::iso_band(label, POINT), numbers::signed_db(p.eq_graphic.get(band).copied().unwrap_or(0.0), POINT));
    }
    let Some(b) = p.eq_bands.get(band) else { return String::new() };
    let width = if matches!(b.kind, K::LowShelfSlope | K::HighShelfSlope) { "slope" } else { "Q" };
    let gain = if b.kind.uses_gain() { format!("  {} dB", numbers::signed_db(b.gain_db, POINT)) } else { String::new() };
    format!("{}{gain}  ·  {}  ·  {width} {}  ·  {}", numbers::hz(b.freq, POINT), band_kind(b.kind), numbers::fixed(b.q as f64, 2, false, POINT), band_side(b.channel))
}

/// What the curve's keys and clicks do, under it.
pub fn eq_curve_keys(graphic: bool) -> &'static str {
    if graphic {
        "↑ ↓ level · drag a point"
    } else {
        "↑ ↓ level · [ ] frequency · { } width · t kind · c side · d remove · click the curve to add"
    }
}

/// A band's label: its frequency and its mark ("1k L", "63 low shelf").
pub fn band(freq: f32, mark: BandMark) -> String {
    let mark = match mark {
        BandMark::None => "",
        BandMark::Left => " L",
        BandMark::Right => " R",
        BandMark::LowShelf => " low shelf",
        BandMark::HighShelf => " high shelf",
        BandMark::NoGain => " filter",
    };
    format!("{}{mark}", numbers::hz(freq, POINT))
}

/// "Pre-amp -3.5 dB (automatic)".
pub fn preamp(db: f32, automatic: bool) -> String {
    format!("{} dB{}", numbers::signed_db(db, POINT), if automatic { " (automatic)" } else { "" })
}

/// "center", "L 30%", "R 5%".
pub fn balance(balance: f32) -> String {
    if balance == 0.0 {
        return "center".into();
    }
    format!("{} {}%", if balance < 0.0 { "L" } else { "R" }, numbers::fixed((balance.abs() * 100.0) as f64, 0, false, POINT))
}

/// "-1.0 dB".
pub fn ceiling(db: f32) -> String {
    format!("{} dB", numbers::fixed(db as f64, 1, false, POINT))
}

/// Where lyrics came from, for the credit line: "your server", "LRCLIB".
pub fn lyrics_origin(origin: LyricsOrigin) -> &'static str {
    match origin {
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
    }
}

/// The credit line for lyrics from a lyrics service: "LRCLIB", "LRCLIB · not timed".
pub fn lyrics_credit(origin: LyricsOrigin, synced: bool) -> String {
    let source = lyrics_origin(origin);
    if synced { source.to_string() } else { format!("{source} · not timed") }
}

/// A built-in equalizer curve's name.
pub fn preset(kind: PresetKind) -> &'static str {
    match kind {
        PresetKind::Flat => "Flat",
        PresetKind::BassBoost => "Bass boost",
        PresetKind::BassCut => "Bass cut",
        PresetKind::TrebleBoost => "Treble boost",
        PresetKind::TrebleCut => "Treble cut",
        PresetKind::VocalBoost => "Vocal boost",
        PresetKind::Loudness => "Loudness",
        PresetKind::SmallSpeakers => "Small speakers",
    }
}

/// Why nothing on the equalizer screen reaches the sound.
pub fn eq_bypass(why: EqBypass) -> &'static str {
    match why {
        EqBypass::BitPerfect => "Bit-perfect USB output is active, so nothing here touches the audio.",
        EqBypass::Output => "No processing on this output: nothing here changes the sound. Turn it off in Settings, under Sound.",
    }
}

/// Why the beat model's download did not work.
pub fn beat_failure(why: BeatFailure) -> &'static str {
    match why {
        BeatFailure::Network => "the network failed",
        BeatFailure::WrongFile => "the file that arrived was not the right one",
        BeatFailure::Storage => "it could not be saved",
    }
}

/// A server or network failure in actionable words; unknown transport failures show their detail.
pub fn net_error(e: &NetError) -> String {
    let said = match e {
        NetError::Transport { kind, .. } => match kind {
            FailureKind::Metered => "This server is set to Wi-Fi only",
            FailureKind::UnknownHost => "Server not found. Check the address.",
            FailureKind::Connect => "Nothing is answering at that address. Is the port right, and is the server running?",
            FailureKind::Timeout => "The server did not answer in time.",
            FailureKind::Tls => {
                "The server's certificate was not accepted. If it is self-signed, turn on \"Accept self-signed certificate\"; if it needs a client certificate, import one."
            }
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

/// The server search failed; the offline results stay.
pub fn search_fallback(reason: Option<&str>) -> String {
    match reason {
        Some(r) => format!("Server search failed: {r} — showing offline results"),
        None => "Server search failed — showing offline results".into(),
    }
}

// ---- other devices and jams ----

/// The keys the bottom line shows, as (key, what it does), by what has them.
pub mod hints {
    type Hints = &'static [(&'static str, &'static str)];
    pub const HOME: Hints = &[("enter", "open"), ("m", "menu"), ("x", "play"), ("/", "search"), ("1-9", "go"), ("F", "player"), ("?", "keys")];
    pub const LIBRARY: Hints = &[("enter", "open"), ("m", "menu"), ("x", "play"), ("[ ]", "section"), ("/", "search"), ("F", "player"), ("?", "keys")];
    pub const SORTED: Hints = &[("enter", "open"), ("m", "menu"), ("S", "sort"), ("[ ]", "section"), ("/", "search"), ("F", "player"), ("?", "keys")];
    pub const OFFER: Hints = &[("enter", "resume"), ("d", "close"), ("/", "search"), ("1-9", "go"), ("?", "keys")];
    pub const HISTORY: Hints = &[("enter", "play"), ("m", "menu"), ("esc", "back"), ("?", "keys")];
    pub const PAGE: Hints = &[("enter", "play"), ("m", "menu"), ("x", "play all"), ("X", "shuffle"), ("f", "favorite"), ("esc", "back"), ("?", "keys")];
    pub const RESULTS: Hints = &[("enter", "open"), ("m", "menu"), ("/", "type"), ("esc", "back"), ("?", "keys")];
    pub const TYPING: Hints = &[("enter", "results"), ("esc", "stop typing"), ("ctrl u", "clear")];
    pub const SETTINGS: Hints = &[("enter", "switch"), ("← →", "change"), ("[ ]", "group"), ("?", "keys")];
    pub const EQUALIZER: Hints = &[("← →", "choose"), ("↑ ↓", "change"), ("enter", "switch"), ("d", "remove a band"), ("?", "keys")];
    pub const DOWNLOADS: Hints = &[("enter", "play"), ("d", "remove"), ("esc", "back"), ("?", "keys")];
    pub const PLAYING: Hints = &[("space", "play"), ("n p", "next, previous"), ("m", "menu"), ("tab", "panel"), ("esc", "back"), ("?", "keys")];
    pub const QUEUE: Hints = &[("enter", "play"), ("d", "remove"), ("K J", "move"), ("s r", "shuffle, repeat"), ("tab", "panel"), ("esc", "back")];
    pub const LYRICS: Hints = &[("enter", "play from the line"), ("[ ]", "timing"), ("tab", "panel"), ("esc", "back"), ("?", "keys")];
    pub const DEVICES: Hints = &[("enter", "play there"), ("tab", "panel"), ("esc", "back"), ("?", "keys")];
}

pub const LISTEN_NOW: &str = "Listen now";
pub const SHOW_ALL: &str = "Show all";
pub const PLAY: &str = "Play";
pub const SHUFFLE: &str = "Shuffle";
pub const UP_NEXT: &str = "Playing next";
pub const BACK: &str = "Back";
pub const VOLUME: &str = "Volume";
pub const NOTHING_PLAYING: &str = "Nothing playing";
/// Beside a provider's song: playing it makes the server fetch it.
pub const REMOTE: &str = "remote";
pub const OFFLINE: &str = "offline";
pub const UNREACHABLE: &str = "unreachable";

pub const SORT_BY: &str = "Sort by";
/// Over a list showing only the favorites.
pub const FAVORITES_ONLY: &str = "Favorites only";
pub const FOR_YOU: &str = "For you";
pub const SAVE_AS_PLAYLIST: &str = "Save as playlist";
pub const NEW_PLAYLIST: &str = "New playlist";
pub const ADD_TO_PLAYLIST: &str = "Add to playlist";
pub const MORE: &str = "More";

/// Songs added to a playlist: "Added to Road trip".
pub fn added_to(name: &str) -> String {
    format!("Added to {name}")
}

/// A line of the song menu, as the phone words it.
pub fn song_action(a: &nori_core::menus::SongAction) -> String {
    use nori_core::menus::SongAction;
    match a {
        SongAction::Favourite { on: true } => "Add to favorites".into(),
        SongAction::Favourite { on: false } => "Remove from favorites".into(),
        SongAction::PlayNext => "Play next".into(),
        SongAction::AddToQueue => "Add to queue".into(),
        SongAction::AddToPlaylist => "Add to playlist…".into(),
        SongAction::RemoveDownload => "Remove download".into(),
        SongAction::StopDownload => "Stop download".into(),
        SongAction::Download => "Download".into(),
        SongAction::GoToAlbum { .. } => "Go to album".into(),
        SongAction::GoToArtist { name, named: true, .. } => format!("Go to {name}"),
        SongAction::GoToArtist { .. } => "Go to artist".into(),
        SongAction::AddToLibrary => "Add to library".into(),
        SongAction::Lyrics => "Lyrics".into(),
        SongAction::SleepTimer => "Sleep timer…".into(),
        SongAction::StartRadio => "Start radio from this song".into(),
        SongAction::InstantMix => "Instant mix".into(),
        SongAction::ExcludeFromMixes => "Exclude from mixes".into(),
        SongAction::Share => "Share link".into(),
        SongAction::Details => "Details".into(),
        SongAction::StartJam => JAM_START.into(),
    }
}

/// A line of a menu that is not the song's own.
pub fn menu_act(a: &crate::menu::MenuAct, missing: usize) -> String {
    use crate::menu::MenuAct;
    match a {
        MenuAct::Song(s) => song_action(s),
        MenuAct::Play | MenuAct::PlayPage => PLAY.into(),
        MenuAct::PlayNext => "Play next".into(),
        MenuAct::AddToQueue => "Add to queue".into(),
        MenuAct::ShufflePage => SHUFFLE.into(),
        MenuAct::SaveMix => SAVE_AS_PLAYLIST.into(),
        MenuAct::Page(p) => page_action(*p, missing),
        MenuAct::Plugin(e) => e.label.clone(),
    }
}

/// A playlist made: "Created nori discover".
pub fn created(name: &str) -> String {
    format!("Created {name}")
}

/// One of a page's head's actions, as the phone words it.
pub fn page_action(a: nori_core::menus::PageAction, missing: usize) -> String {
    use nori_core::menus::{DownloadAct, PageAction};
    match a {
        PageAction::Download { act: DownloadAct::All } => "Download".into(),
        PageAction::Download { act: DownloadAct::Missing } => format!("Download the other {missing}"),
        PageAction::Download { act: DownloadAct::Remove } => "Remove downloads".into(),
        PageAction::DownloadEverything => "Download everything".into(),
        PageAction::AddToQueue => "Add to queue".into(),
        PageAction::Refresh => "New mix".into(),
        PageAction::Edit => "Edit".into(),
    }
}

/// An artist's shelf of releases by kind.
pub fn release_shelf(g: &nori_core::pages::ReleaseGroup) -> String {
    use nori_core::pages::ReleaseKind;
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
pub const CONTINUE: &str = "Continue";
pub const RESUME: &str = "Resume";
pub const CLOSE: &str = "Close";

/// Where the offered queue stands: "Artist · 3 of 12".
pub fn offer_place(artist: &str, at: u32, of: usize) -> String {
    format!("{artist} · {at} of {of}")
}

/// A For you mix's name, as its line and its page say it.
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
pub fn mix_line(favourites: bool) -> &'static str {
    if favourites { "Your favorites" } else { "Made for you" }
}

/// One of the albums' orders (`browse::album_sorts`).
pub fn album_sort(sort: nori_core::browse::AlbumSort) -> &'static str {
    use nori_core::browse::AlbumSort;
    match sort {
        AlbumSort::Starred => "Favorites",
        AlbumSort::ByName => "Title",
        AlbumSort::ByArtist | AlbumSort::ArtistRows => "Artist",
        AlbumSort::ByYear => "Year",
        AlbumSort::Newest => "Added",
        AlbumSort::Recent => "Played",
        AlbumSort::Frequent => "Most played",
        AlbumSort::Random => "Random",
        AlbumSort::ByGenre => "Genres",
    }
}

/// One of `browse::song_sorts`, by its name.
pub fn song_sort(name: &str) -> &'static str {
    match name {
        "ARTIST" => "Artist",
        "ALBUM" => "Album",
        "YEAR" => "Year",
        "ADDED" => "Added",
        "PLAYS" => "Most played",
        "LONGEST" => "Longest",
        _ => "Title",
    }
}

/// A place at the top of the library's sections.
pub fn place(p: crate::app::Place) -> &'static str {
    use crate::app::Place;
    match p {
        Place::Favorites => "Favorites",
        Place::Downloads => "Downloads",
        Place::History => "History",
        Place::Shuffle => "Shuffle",
    }
}

/// A switch's state.
pub fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

/// The repeat mode `mode` as the queue's head and the song playing say it.
pub fn repeat_word(mode: u8) -> &'static str {
    match mode {
        nori_core::rules::REPEAT_ONE => "Repeat one",
        _ => "Repeat",
    }
}

/// The server did not answer: what the page shows instead.
pub fn stored_only(e: &str) -> String {
    format!("{e} · showing what is stored · R tries again")
}

/// Where this computer plays, as the phone names an output (`outputs::output_look`): the device's own name,
/// or what it is; this computer while the system's default is unnamed.
pub fn output_name(output: &str) -> String {
    use nori_core::profiles::OutputPort;
    if output.is_empty() {
        return THIS_COMPUTER.into();
    }
    let look = nori_core::outputs::output_look(output.to_string());
    let name = look.name.filter(|n| !n.is_empty());
    match look.port {
        OutputPort::Speaker => "Phone speaker".into(),
        OutputPort::Wired => "Wired headphones".into(),
        OutputPort::Usb => name.unwrap_or_else(|| "DAC".into()),
        OutputPort::Bluetooth => name.unwrap_or_else(|| "device".into()),
        OutputPort::Other => name.unwrap_or_else(|| "Other output".into()),
    }
}

/// The account's session ended at the source called `name`.
pub fn signed_out_again(name: &str) -> String {
    format!("{name} signed you out. Sign in again to see your library and likes.")
}
pub const SIGN_IN_WHY: &str = "See your own Home, your library and your likes. Music plays without it too.";
pub const SIGN_IN_KEYS: &str = "enter sign in · p paste cookies · esc close";
pub const SIGN_IN_KEYS_ENDED: &str = "enter sign in · p paste cookies · d don't show again · esc close";
pub const SIGNED_IN_KEYS: &str = "enter sign out · esc close";
pub const SIGNING_IN: &str = "Finish signing in in the window that opened.";
pub const PASTE_COOKIES: &str = "Sign in in a private browser window, copy the Cookie header of any request the page makes from the developer tools, and paste it here. enter signs in · esc back";
pub const SIGN_IN_WINDOW_FAILED: &str = "The sign-in window couldn't open here. Paste cookies from a browser instead.";
pub const THIS_COMPUTER: &str = "This computer";
pub const NOISE_CONTROL: &str = "Noise control";
pub const REACHES_BLUETOOTH: &str = "Reaches Bluetooth devices.";

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

/// Each battery a device has: "80%", or "L 80% · R 70% · Case 50%".
pub fn battery(b: &nori_plugins::Battery) -> String {
    if let Some(s) = b.single {
        return format!("{s}%");
    }
    [("L", b.left), ("R", b.right), ("Case", b.case)].iter().filter_map(|(w, v)| v.map(|v| format!("{w} {v}%"))).collect::<Vec<_>>().join(" · ")
}
pub const REMOTE_OFF: &str = "Remote control and jams are off: switch them on in Settings, under Other devices";

/// What a plugin's cast is, beside its device's name.
pub fn cast_line(c: &nori_plugins::Cast) -> String {
    let what = match c.kind {
        nori_plugins::CastKind::Sink => "Casting",
        nori_plugins::CastKind::Target => "Playing there",
    };
    match c.volume {
        Some(v) => format!("{what} · {v} %"),
        None => what.to_string(),
    }
}

/// The player bar's line while another device plays: "Playing on Desk".
pub fn playing_on(device: &str) -> String {
    format!("Playing on {device}")
}

pub fn device_kind(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Phone => "phone",
        DeviceKind::Desktop => "computer",
        DeviceKind::Terminal => "terminal",
        DeviceKind::Guest => "guest",
    }
}

/// The kinds' words, for telling devices of one name apart.
pub fn kind_words() -> KindWords {
    let word = |k| device_kind(k).to_string();
    KindWords { phone: word(DeviceKind::Phone), desktop: word(DeviceKind::Desktop), terminal: word(DeviceKind::Terminal), guest: word(DeviceKind::Guest) }
}

/// What a device of the account plays, or its answer to the last command when it said no.
pub fn device_line(d: &RemoteDevice) -> String {
    let refused = match d.refused {
        Some(Refusal::Stale) => "The queue changed there. Try again.",
        Some(Refusal::NotAllowed) => "That device said no.",
        Some(Refusal::Unknown) => "That is no longer there.",
        Some(Refusal::TooMany) => "Too many songs waiting.",
        Some(Refusal::OtherSource) => "That device plays from another source.",
        None => "",
    };
    if !refused.is_empty() {
        return refused.into();
    }
    match &d.now {
        Some(e) if d.state.as_ref().is_some_and(|s| s.playing) => format!("{} · {}", e.title, e.artist),
        Some(e) => format!("Paused · {}", e.title),
        None => "Not playing".into(),
    }
}

pub const JAM_START: &str = "Start a jam";
pub const JAM_END: &str = "End the jam";
pub const JAM_INVITE: &str = "invite";
pub const JAM_DECIDE: &str = "⏎ add · d no";
/// Under a provider's song asked for: accepting it makes the server download it.
pub const JAM_DOWNLOADS: &str = "Downloaded to your server if accepted";
pub const JAMS_OFF: &str = "Jams are off: switch them on in Settings, under Other devices";
pub const JAM_UNSUPPORTED: &str = "Your server doesn't support jams yet. They need octo-fiesta with nori support in front of it.";
pub const JAM_FAILED: &str = "Couldn't start the jam. Jams need octo-fiesta in front of your server.";
pub const INVITE_TITLE: &str = " Invite to the jam · any key closes ";
pub const INVITE_HOW: &str = "Scan with a phone's camera, or send the link:";
pub const INVITE_ROOM: &str = "Make the window bigger to show the QR code.";

// A guest's.
pub const JAM_GUEST: &str = "Jam";
pub const JAM_JOIN: &str = "Join a jam";
pub const JOIN_TITLE: &str = " Join a jam · enter joins, esc drops ";
pub const JOIN_HOW: &str = "Paste the invite link the host sent you:";
pub const JOINING: &str = "Joining…";
pub const NOT_AN_INVITE: &str = "That is not a jam invite. Paste the whole link the host sent you.";
pub const JAM_LEAVE: &str = "Leave the jam";
pub const JAM_LEFT: &str = "You left the jam";

/// What to say about a device's AutoEQ curve, with the key that answers it.
pub fn curve_notice(n: &CurveNotice) -> String {
    match n {
        CurveNotice::Offer { entry, .. } => format!("{} connected. Use its AutoEQ curve? (E: apply)", entry.name),
        CurveNotice::Applied { curve, .. } => format!("Using AutoEQ for {curve} (E: undo)"),
    }
}

pub fn jam_ended(host: Option<&str>) -> String {
    host.map_or("The jam ended".into(), |h| format!("{h} ended the jam"))
}
pub const LISTEN_HERE: &str = "Listen here";
pub const PLAYING_HERE: &str = "Playing here";
pub const LISTEN_KEYS: &str = "⏎ listen";
pub const STOP_KEYS: &str = "⏎ stop";
pub const YOU_ASKED: &str = "You asked for";
/// Beside a song a guest asked for, until the host takes it.
pub const ASKED: &str = "Asked";

/// Under a jam guest's player while it paused its own listening: play joins the jam again.
pub const JAM_PAUSED_HERE: &str = "Paused here · Jam still playing";

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

/// The jam a guest is in, as the player bar and the queue say it: "Jam · Desk · 2 listening".
pub fn jam_guest_strip(host: &str, n: usize) -> String {
    format!("Jam · {host} · {}", jam_listening(n))
}

/// Beside a song a guest asked for: " · waiting for Desk".
pub fn jam_waiting(host: &str) -> String {
    format!(" · waiting for {host}")
}

/// Why a guest who asked to listen along does not.
pub fn jam_along(l: Listening) -> &'static str {
    match l {
        Listening::Watching | Listening::Playing => "",
        Listening::HostOff => "The host doesn't let guests listen along right now. You can still ask for songs.",
        Listening::ServerOff => "This server doesn't let guests listen along. You can still ask for songs.",
        Listening::Refused => "The host's songs don't play on this device: it's on another network. You can still ask for songs.",
    }
}

/// Who listens in the jam: "2 listening".
pub fn jam_listening(n: usize) -> String {
    match n {
        0 => "no one yet".into(),
        n => format!("{n} listening"),
    }
}

/// The jam as the player bar and the devices say it: "Jam · 2 listening".
pub fn jam_strip(n: usize) -> String {
    format!("Jam · {}", jam_listening(n))
}

/// A listener by name, and role when it is more than a guest's.
pub fn jam_member(name: &str, role: Role) -> String {
    match role {
        Role::Admin => format!("{name} (admin)"),
        Role::Host | Role::Guest => name.into(),
    }
}

/// Who asked for a song: " · asked by Gus".
pub fn jam_asked(from: &str) -> String {
    format!(" · asked by {from}")
}

pub const JAM_SETTINGS: &str = "Jam settings";
pub const JAM_SETTINGS_KEY: &str = "s  Jam settings";
pub const JAM_SETTINGS_BACK: &str = "Esc  Back to invite";

pub fn room_settings(along: bool, remote_songs: bool) -> [String; 2] {
    let state = |on| if on { "on" } else { "off" };
    [
        format!("l  Let guests listen along: {}", state(along)),
        format!("r  Allow remote songs: {}", state(remote_songs)),
    ]
}

pub const PAIR_DEVICES: &str = "Pair devices (paste another device’s code with o)";
pub const FORGET_DEVICES: &str = "Forget paired devices";
pub const PAIRED: &str = "Devices paired";
pub const PAIR_FAILED: &str = "Pairing failed";
pub const FORGOTTEN: &str = "Paired devices forgotten";
pub const PAIR_UNAVAILABLE: &str = "Pairing unavailable";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(band(1000.0, BandMark::LowShelf), "1k low shelf");
        assert_eq!((balance(0.0), balance(-0.3), balance(0.05)), ("center".into(), "L 30%".into(), "R 5%".into()));
        assert_eq!(nudge(-250), "-0.25 s");
        assert_eq!((songs_caption(1, 200), albums(2)), ("1 song · 3:20".into(), "2 albums".into()));

        // Net errors.
        let t = |kind, detail: Option<&str>| net_error(&NetError::Transport { kind, detail: detail.map(str::to_string) });
        assert_eq!(t(FailureKind::Metered, None), "This server is set to Wi-Fi only");
        assert_eq!(t(FailureKind::UnknownHost, Some("x")), "Server not found. Check the address.");
        assert_eq!(t(FailureKind::Io, Some("raw")), "raw");
        assert_eq!(net_error(&NetError::Http { status: 522 }), "HTTP 522");
        assert_eq!(net_error(&NetError::Api { code: 40, reason: "x".into() }), "Wrong user name or password.");
        assert_eq!(net_error(&NetError::Api { code: 70, reason: "gone".into() }), "gone");
        assert!(net_error(&NetError::Parse { reason: "x".into() }).starts_with("That address answered"));

        assert_eq!((jam_strip(0), jam_strip(2)), ("Jam · no one yet".into(), "Jam · 2 listening".into()));
        let state = |playing| nori_core::remote::wire::DeviceState {
            playing,
            index: Some(4),
            entries: vec![nori_core::remote::wire::Entry { index: 4, title: "Wish".into(), artist: "Gus".into(), ..Default::default() }],
            ..Default::default()
        };
        let wish = state(true).entries.pop();
        let mut d = RemoteDevice { id: "d".into(), name: "Desk".into(), kind: DeviceKind::Desktop, state: Some(state(true)), now: wish.clone(), age_ms: 0, nearby: true, refused: None };
        assert_eq!(device_line(&d), "Wish · Gus");
        d.state = Some(state(false));
        assert_eq!(device_line(&d), "Paused · Wish", "the active device, paused");
        d.now = None;
        assert_eq!(device_line(&d), "Not playing", "a device another took over from");
        d.now = wish;
        d.refused = Some(Refusal::Stale);
        assert_eq!(device_line(&d), "The queue changed there. Try again.", "a refusal over what it plays");
        d.refused = Some(Refusal::OtherSource);
        assert_eq!(device_line(&d), "That device plays from another source.");

        // Lyrics credit says untimed.
        assert_eq!(lyrics_credit(LyricsOrigin::Lrclib, true), "LRCLIB");
        assert_eq!(lyrics_credit(LyricsOrigin::Lrclib, false), "LRCLIB · not timed");

        // The output by what it is, or its own name (`output_look`).
        let outputs = ["", "USB: K3", "USB: ", "Wired headphones", "Bluetooth: Buds", "MacBook Pro Speakers"].map(output_name);
        assert_eq!(outputs, ["This computer", "K3", "DAC", "Wired headphones", "Buds", "MacBook Pro Speakers"]);
    }

}
