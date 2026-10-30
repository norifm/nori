//! The places nori picks something that plugins may decide instead (the `deciders` extension point):
//! nori works out its own answer, hands it over with the inputs, and checks what comes back before
//! using it. Implemented by the plugin host; the domain crates hold it as a trait object, so they never
//! depend on it.

use std::future::Future;
use std::pin::Pin;

use nori_player::outputs::OutputPort;

use crate::Song;

/// An answer that comes later (asked from async code, which must not wait on a plugin).
pub type Later<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What the notification, lock screen and media controls show for a song.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Shown {
    pub title: String,
    pub artist: String,
    pub album: String,
}

impl Shown {
    pub fn of(song: &Song) -> Shown {
        Shown { title: song.title.clone(), artist: song.artist.clone(), album: song.album.clone() }
    }
}

/// A press of a headset's, a car's or the system's media button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Press {
    PlayPause,
    Next,
    Previous,
    Double,
    Triple,
    Long,
}

/// What a press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ButtonAct {
    PlayPause,
    Next,
    Previous,
    Nothing,
}

/// A change of the system's audio focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Focus {
    /// Another player took it for good.
    Lost,
    /// A call or a voice prompt, for a while.
    LostTransient,
    /// Another sound plays over the music for a while.
    Duck,
    Gained,
}

/// What a focus change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum FocusAct {
    /// Paused: for good after [`Focus::Lost`], until the focus comes back otherwise.
    Pause,
    /// Played on, quieter (the system lowers it).
    Duck,
    KeepPlaying,
    /// What a focus loss paused plays again.
    Resume,
    Nothing,
}

/// The five roles of a season: page, surface, ink, accent, on (ARGB).
pub type Roles = [u32; 5];

/// What plugins decide in place of nori. Each call gets nori's answer and returns another, or None for
/// nori's. An answer `fits` refuses is never used (nor handed on to the next plugin).
pub trait Deciders: Send + Sync {
    /// Whether anything would be asked: false while no enabled plugin decides, so a seam builds no inputs.
    fn deciding(&self) -> bool;
    /// The play order when shuffling starts, as list indexes, given nori's.
    fn shuffle(&self, songs: &[Song], nori: &[u32], fits: &dyn Fn(&[u32]) -> bool) -> Option<Vec<u32>>;
    /// Song ids to add when autofill runs, given nori's picks.
    fn autofill(&self, tail: &[Song], nori: &[String], count: u32) -> Later<Option<Vec<String>>>;
    /// Song ids to play when the queue ended after `last`.
    fn queue_ended(&self, last: Option<&Song>) -> Later<Option<Vec<String>>>;
    /// The song's leveling gain, dB, given nori's.
    fn leveling(&self, song: &Song, nori_db: f32, fits: &dyn Fn(f32) -> bool) -> Option<f32>;
    /// The sound profile, by name ("" for none), for the output keyed `output`.
    fn sound_profile(&self, port: OutputPort, output: &str, nori: &str, fits: &dyn Fn(&str) -> bool) -> Option<String>;
    fn now_playing(&self, song: &Song, nori: &Shown, fits: &dyn Fn(&Shown) -> bool) -> Option<Shown>;
    fn media_button(&self, press: Press, nori: ButtonAct) -> Option<ButtonAct>;
    fn audio_focus(&self, focus: Focus, nori: FocusAct) -> Option<FocusAct>;
    /// The palette worn, given nori's, the cover's dominant color and whether it is night.
    fn look(&self, cover: Option<u32>, night: bool, nori: Roles, fits: &dyn Fn(&Roles) -> bool) -> Option<Roles>;
}
