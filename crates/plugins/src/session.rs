//! The session a plugin reaches (`session`, `subsonic`, `sign-in`), each part behind its manifest's
//! permission. What is playing comes from the client's [`Moment`]s, the queue and the library from the
//! open profile's [`Library`] (the core's), and controls go to the client's player ([`Control`]), so
//! its own rules (a jam's, the skip rules) apply as for a press.

use std::sync::Arc;

use nori_model::{Album, Artist, Playlist, Song};

use crate::convert::{album_out, artist_out, playlist_out, song_out};
use crate::manifest::{ItemKind, Permission};
use crate::runtime::nori::plugin::{session as s, sign_in, subsonic};
use crate::runtime::{types as t, Host};

/// Where the output plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Route {
    #[default]
    Speaker,
    Wired,
    Bluetooth,
    Usb,
    Car,
    Hdmi,
    Cast,
    Other,
}

impl From<Route> for t::Route {
    fn from(r: Route) -> t::Route {
        match r {
            Route::Speaker => t::Route::Speaker,
            Route::Wired => t::Route::Wired,
            Route::Bluetooth => t::Route::Bluetooth,
            Route::Usb => t::Route::Usb,
            Route::Car => t::Route::Car,
            Route::Hdmi => t::Route::Hdmi,
            Route::Cast => t::Route::Cast,
            Route::Other => t::Route::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// The client's player as it is now, said whenever it changes while a plugin follows it
/// ([`crate::Plugins::follows`]).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Moment {
    pub playing: bool,
    /// The song playing's queue index and id.
    pub index: Option<u32>,
    pub song: Option<String>,
    /// Where it is now.
    pub position_ms: u64,
    /// The output, by the core's key for it ("Bluetooth: WH-1000XM5").
    pub output: String,
}

impl Moment {
    pub(crate) fn route(&self) -> Route {
        use nori_player::outputs::{parts, OutputPort};
        match parts(&self.output).0 {
            OutputPort::Speaker => Route::Speaker,
            OutputPort::Wired => Route::Wired,
            OutputPort::Usb => Route::Usb,
            OutputPort::Bluetooth => Route::Bluetooth,
            OutputPort::Other => Route::Other,
        }
    }

    /// The output's own name ("WH-1000XM5"), else its key.
    fn output_name(&self) -> String {
        nori_player::outputs::parts(&self.output).1.unwrap_or(&self.output).to_string()
    }
}

/// What a plugin asks the client's player to do.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Control {
    Play,
    Pause,
    Next,
    Previous,
    Seek { ms: u64 },
    PlayAt { index: u32 },
    SetShuffle { on: bool },
    SetRepeat { repeat: Repeat },
    /// After the song playing, or at the end.
    Add { songs: Vec<Song>, next: bool },
    Remove { index: u32 },
    Move { index: u32, to: u32 },
    /// The songs after the one playing.
    Clear,
}

/// The open profile, as a plugin sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: String,
    /// "subsonic", or a plugin's id.
    pub kind: String,
    pub name: String,
}

/// One item the library found.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Song(Box<Song>),
    Album(Album),
    Artist(Artist),
    Playlist(Playlist),
}

/// The open profile's queue and library, as the core gives them to plugins. Its calls wait for the
/// network; they are made on a plugin's own thread.
pub trait Library: Send + Sync {
    fn profile(&self) -> Option<Profile>;
    /// Queue indexes `start..start + count`.
    fn queue(&self, start: u32, count: u32) -> Vec<Song>;
    fn queue_length(&self) -> u32;
    /// Changes with every edit of the queue.
    fn queue_rev(&self) -> u64;
    fn shuffle(&self) -> bool;
    fn repeat(&self) -> Repeat;
    /// The songs `ids`, in order, from the queue's songs or the profile.
    fn songs(&self, ids: &[String]) -> Result<Vec<Song>, t::Error>;
    fn search(&self, query: &str, count: u32) -> Result<Vec<Found>, t::Error>;
    fn lookup(&self, kind: ItemKind, id: &str) -> Result<Found, t::Error>;
    fn songs_of(&self, kind: ItemKind, id: &str) -> Result<Vec<Song>, t::Error>;
    fn star(&self, kind: ItemKind, id: &str, on: bool) -> Result<(), t::Error>;
    fn add_to_playlist(&self, playlist: &str, ids: &[String]) -> Result<(), t::Error>;
    /// The profile's Subsonic API: `method`'s JSON answer.
    fn subsonic(&self, method: &str, params: &[(String, String)]) -> Result<Vec<u8>, t::Error>;
    /// The song's synced lyrics, as start ms and words; None without them.
    fn synced_lyrics(&self, song: &Song) -> Option<Vec<(u32, String)>>;
}

pub(crate) fn error(kind: t::ErrorKind, detail: impl Into<String>) -> t::Error {
    t::Error { kind, detail: detail.into() }
}

fn found_out(f: &Found) -> t::Item {
    match f {
        Found::Song(s) => t::Item::Song(song_out(s)),
        Found::Album(a) => t::Item::Album(album_out(a)),
        Found::Artist(a) => t::Item::Artist(artist_out(a)),
        Found::Playlist(p) => t::Item::Playlist(playlist_out(p)),
    }
}

fn kind_in(k: t::ItemKind) -> ItemKind {
    match k {
        t::ItemKind::Song => ItemKind::Song,
        t::ItemKind::Album => ItemKind::Album,
        t::ItemKind::Artist => ItemKind::Artist,
        t::ItemKind::Playlist => ItemKind::Playlist,
    }
}

fn repeat_out(r: Repeat) -> s::Repeat {
    match r {
        Repeat::Off => s::Repeat::Off,
        Repeat::All => s::Repeat::All,
        Repeat::One => s::Repeat::One,
    }
}

impl Host {
    /// Whether its manifest grants `p`; a call without it is logged and refused.
    fn may(&self, p: Permission) -> bool {
        let ok = self.manifest.may(p);
        if !ok {
            nori_model::alog::info(&format!("plugin {}: refused, it lacks {p:?}", self.id));
        }
        ok
    }

    fn library(&self) -> Result<Arc<dyn Library>, t::Error> {
        self.plugins().and_then(|p| p.library()).ok_or_else(|| error(t::ErrorKind::Network, "no profile open"))
    }

    /// Hands `c` to the client's player, when the plugin may control it.
    fn control(&self, c: Control) {
        if self.may(Permission::PlaybackControl) {
            self.side.control(c);
        }
    }
}

impl s::Host for Host {
    fn now(&mut self) -> s::NowPlaying {
        let library = self.library().ok().filter(|_| self.may(Permission::PlaybackRead));
        let (moment, position_ms) = self.plugins().map(|p| p.moment()).unwrap_or_default();
        let song = moment.song.as_ref().zip(library.as_ref()).and_then(|(id, l)| l.songs(std::slice::from_ref(id)).ok()?.into_iter().next());
        s::NowPlaying {
            song: song.as_ref().map(song_out),
            index: moment.index,
            playing: moment.playing,
            position_ms,
            shuffle: library.as_ref().is_some_and(|l| l.shuffle()),
            repeat: repeat_out(library.as_ref().map_or(Repeat::Off, |l| l.repeat())),
            route: moment.route().into(),
            output: moment.output_name(),
        }
    }

    fn queue(&mut self, start: u32, count: u32) -> Vec<t::Song> {
        match self.library() {
            Ok(l) if self.may(Permission::PlaybackRead) => l.queue(start, count.min(500)).iter().map(song_out).collect(),
            _ => Vec::new(),
        }
    }

    fn queue_length(&mut self) -> u32 {
        match self.library() {
            Ok(l) if self.may(Permission::PlaybackRead) => l.queue_length(),
            _ => 0,
        }
    }

    fn current_profile(&mut self) -> Option<s::Profile> {
        let p = self.library().ok().filter(|_| self.may(Permission::PlaybackRead))?.profile()?;
        Some(s::Profile { id: p.id, kind: p.kind, name: p.name })
    }

    fn play(&mut self) {
        self.control(Control::Play);
    }

    fn pause(&mut self) {
        self.control(Control::Pause);
    }

    fn next(&mut self) {
        self.control(Control::Next);
    }

    fn previous(&mut self) {
        self.control(Control::Previous);
    }

    fn seek(&mut self, position_ms: u64) {
        self.control(Control::Seek { ms: position_ms });
    }

    fn play_at(&mut self, index: u32) {
        self.control(Control::PlayAt { index });
    }

    fn set_shuffle(&mut self, on: bool) {
        self.control(Control::SetShuffle { on });
    }

    fn set_repeat(&mut self, repeat: s::Repeat) {
        let repeat = match repeat {
            s::Repeat::Off => Repeat::Off,
            s::Repeat::All => Repeat::All,
            s::Repeat::One => Repeat::One,
        };
        self.control(Control::SetRepeat { repeat });
    }

    fn add(&mut self, ids: Vec<String>, next: bool) -> Result<(), t::Error> {
        if !self.may(Permission::PlaybackControl) {
            return Err(error(t::ErrorKind::Refused, "playback.control"));
        }
        let songs = self.library()?.songs(&ids)?;
        if songs.iter().any(|s| s.is_external) && !matches!(self.side.stored("autoFillRemote"), Some(crate::Value::Switch(true))) {
            return Err(error(t::ErrorKind::Refused, "provider songs need the autofill switch that allows them"));
        }
        self.side.control(Control::Add { songs, next });
        Ok(())
    }

    fn remove(&mut self, index: u32) {
        self.control(Control::Remove { index });
    }

    fn move_(&mut self, index: u32, to: u32) {
        self.control(Control::Move { index, to });
    }

    fn clear(&mut self) {
        self.control(Control::Clear);
    }

    fn search(&mut self, query: String, count: u32) -> Result<Vec<t::Item>, t::Error> {
        if !self.may(Permission::LibraryRead) {
            return Err(error(t::ErrorKind::Refused, "library.read"));
        }
        Ok(self.library()?.search(&query, count.min(200))?.iter().map(found_out).collect())
    }

    fn lookup(&mut self, kind: t::ItemKind, id: String) -> Result<t::Item, t::Error> {
        if !self.may(Permission::LibraryRead) {
            return Err(error(t::ErrorKind::Refused, "library.read"));
        }
        Ok(found_out(&self.library()?.lookup(kind_in(kind), &id)?))
    }

    fn songs_of(&mut self, kind: t::ItemKind, id: String) -> Result<Vec<t::Song>, t::Error> {
        if !self.may(Permission::LibraryRead) {
            return Err(error(t::ErrorKind::Refused, "library.read"));
        }
        Ok(self.library()?.songs_of(kind_in(kind), &id)?.iter().map(song_out).collect())
    }

    fn star(&mut self, kind: t::ItemKind, id: String, on: bool) -> Result<(), t::Error> {
        if !self.may(Permission::LibraryWrite) {
            return Err(error(t::ErrorKind::Refused, "library.write"));
        }
        self.library()?.star(kind_in(kind), &id, on)
    }

    fn add_to_playlist(&mut self, playlist: String, ids: Vec<String>) -> Result<(), t::Error> {
        if !self.may(Permission::LibraryWrite) {
            return Err(error(t::ErrorKind::Refused, "library.write"));
        }
        self.library()?.add_to_playlist(&playlist, &ids)
    }
}

impl subsonic::Host for Host {
    fn call(&mut self, method: String, params: Vec<(String, String)>) -> Result<Vec<u8>, t::Error> {
        if !self.may(Permission::Subsonic) {
            return Err(error(t::ErrorKind::Refused, "account.subsonic"));
        }
        if method.is_empty() || !method.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(error(t::ErrorKind::Refused, "a method is its name alone"));
        }
        self.library()?.subsonic(&method, &params)
    }
}

impl sign_in::Host for Host {
    /// Only a provider's, for its profile while that is open: the page's cookies go to its
    /// `sign-in-done`, as its own sign-in page's do.
    fn open(&mut self, start_url: String) -> Result<(), t::Error> {
        if !self.manifest.extends("provider") {
            return Err(error(t::ErrorKind::Unsupported, "a sign-in page hands its cookies to a provider"));
        }
        if !self.library()?.profile().is_some_and(|p| p.kind == self.id) {
            return Err(error(t::ErrorKind::Refused, "its profile is not the one open"));
        }
        let host = start_url.strip_prefix("https://").and_then(|r| r.split(['/', '?', '#', ':']).next()).unwrap_or_default();
        if !self.manifest.signs_in(host) {
            return Err(error(t::ErrorKind::Refused, format!("{host} is not among its signin hosts")));
        }
        match self.side.sign_in(&self.id, &start_url) {
            true => Ok(()),
            false => Err(error(t::ErrorKind::Unsupported, "this client has no sign-in page")),
        }
    }
}
