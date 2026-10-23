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
    pub name: &'static str,
    /// What adding one does, under the login page's choice.
    pub note: &'static str,
    /// The login page's button.
    pub add: &'static str,
    /// Under a profile that names no user.
    pub no_user: &'static str,
}

pub fn provider(kind: ServerKind) -> ProviderWords {
    match kind {
        ServerKind::Subsonic => ProviderWords { name: "Your server", note: "Sign in to your Navidrome or Subsonic server.", add: "Sign In", no_user: "API key" },
        ServerKind::YouTubeMusic => ProviderWords {
            name: "YouTube Music",
            note: "Search and play YouTube Music without an account. Best effort: YouTube can stop it working at any time. Sign in for your library and likes. Songs, albums and playlists can be downloaded for offline playback. Devices on your network can be paired for remote control and jams. Signed in, songs you hear go into your YouTube history.",
            add: "Add YouTube Music",
            no_user: "best effort",
        },
    }
}

pub const SHOW_ALL: &str = "Show all";
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
pub fn lyrics_origin(origin: nori_core::lyrics_sources::LyricsOrigin) -> &'static str {
    use nori_core::lyrics_sources::LyricsOrigin;
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
        AlbumSort::ByName => "A–Z",
        AlbumSort::ByArtist => "Artist",
        AlbumSort::ArtistRows => "Artist rows",
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
pub fn lyrics_from(origin: nori_core::lyrics_sources::LyricsOrigin, synced: bool, word_timed: bool) -> String {
    let timed = if !synced { "not timed" } else if word_timed { "timed word by word" } else { "timed line by line" };
    format!("From {} · {timed}", lyrics_origin(origin))
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
pub fn device_row(d: Option<&nori_core::remote::RemoteDevice>, active: bool) -> crate::DeviceRow {
    use nori_core::remote::wire::Refusal;
    let Some(d) = d else {
        return crate::DeviceRow { name: THIS_COMPUTER.into(), active, ..Default::default() };
    };
    let state = d.state.as_ref();
    let now = state.and_then(|s| s.entries.iter().find(|e| Some(e.index) == s.index));
    let line = now.map_or_else(|| "Not playing".to_string(), |e| format!("{} · {}", e.title, e.artist));
    let note = match d.refused {
        Some(Refusal::Stale) => "The queue changed there. Try again.",
        Some(Refusal::NotAllowed) => "That device said no.",
        Some(Refusal::Unknown) => "That is no longer there.",
        Some(Refusal::TooMany) => "Too many songs waiting.",
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
    }
}

pub const JAM_UNSUPPORTED: &str = "Your server doesn't support jams yet. They need octo-fiesta with nori support in front of it.";
pub const JAM_FAILED: &str = "Couldn't start the jam. Jams need octo-fiesta in front of your server.";
/// Under the invite, when the server's address is a home network's.
pub fn jam_home_only(address: &str) -> String {
    format!("This invite only works on your home network ({address}). Set a public address in the server settings for guests elsewhere.")
}

/// The undo pill after a song is taken out of the queue.
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
        R::NoSession => "These aren’t a signed-in session’s cookies. Copy them from music.youtube.com while signed in.".into(),
        R::Refused => "YouTube didn’t accept these cookies. Sign in again in a private window and copy them anew.".into(),
        R::Unreachable { reason } => format!("YouTube could not be reached ({reason})."),
    }
}

pub const YOUTUBE_MUSIC_PAGE: &str = "https://music.youtube.com";

pub const SIGN_IN_WINDOW_FAILED: &str = "The sign-in window could not open here. Paste cookies from a browser instead.";

/// A song's title as shown: "Untitled" for one that has none (some uploads have a blank one).
pub fn title(t: &str) -> &str {
    if t.trim().is_empty() { "Untitled" } else { t }
}

/// The profile in use as the settings menu heads it: who (the account, else the profile's name, else its
/// service), then where in grey (the server's address, or the service, or that no one is signed in).
pub fn profile_lines(p: &nori_core::settings::SavedServer) -> (String, String) {
    let service = provider(p.kind).name;
    let place = nori_core::settings::label(&p.name, &p.url);
    match (p.user.is_empty(), p.kind) {
        (false, ServerKind::YouTubeMusic) => (p.user.clone(), service.into()),
        (true, ServerKind::YouTubeMusic) => (if place.is_empty() { service.into() } else { place }, "Not signed in".into()),
        (false, _) => (p.user.clone(), place),
        (true, _) => (if place.is_empty() { service.into() } else { place }, String::new()),
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
        let lines = |s: SavedServer| profile_lines(&s);
        assert_eq!(lines(p(ServerKind::Subsonic, "", "https://music.example.com/x", "ann")), ("ann".into(), "music.example.com".into()));
        assert_eq!(lines(p(ServerKind::YouTubeMusic, "", "", "Ann Lee")), ("Ann Lee".into(), "YouTube Music".into()));
        assert_eq!(lines(p(ServerKind::YouTubeMusic, "", "", "")), ("YouTube Music".into(), "Not signed in".into()));
        assert_eq!(lines(p(ServerKind::Subsonic, "Home", "http://h", "")), ("Home".into(), "".into()));
    }

    #[test]
    fn formats_counts() {
        assert_eq!((songs(1), songs_caption(2, 200)), ("1 song".into(), "2 songs · 3:20".into()));
    }
}
