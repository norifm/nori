//! Client request values: the server profile and the writes.

use crate::transport::NetError;

pub type NetResult<T> = std::result::Result<T, NetError>;

/// The parts of a server profile the client uses; headers, certificates and Wi-Fi only are the platform's.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NetProfile {
    pub url: String,
    /// A second address of the same server, tried when `url` does not answer.
    pub alt_url: String,
    /// Restricts browsing and search to these music folders; empty means all.
    pub music_folder_ids: Vec<String>,
    /// Bitrate ceiling while connected through `alt_url`; 0 means none.
    pub alt_max_bit_rate: u32,
}

/// Kotlin's `isBlank`: nothing but whitespace.
pub fn blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

pub fn pairs(p: &[(&str, String)]) -> Vec<(String, String)> {
    p.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Starrable {
    Song,
    Album,
    Artist,
}

/// Every change the app asks the server for.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Write {
    Star { kind: Starrable, id: String, on: bool },
    CreatePlaylist { name: String, song_ids: Vec<String> },
    AddToPlaylist { id: String, song_ids: Vec<String> },
    RemoveFromPlaylist { id: String, index: i32 },
    RenamePlaylist { id: String, name: String },
    /// The playlist's songs in a new order (or a new set): they replace what it held.
    ReplacePlaylist { id: String, song_ids: Vec<String> },
    DeletePlaylist { id: String },
    CreateRadio { name: String, stream_url: String },
    DeleteRadio { id: String },
    /// `submission` false marks "now playing"; true counts the play.
    Scrobble { id: String, submission: bool, time_ms: Option<i64> },
    /// The queue handed to another device.
    SaveQueue { ids: Vec<String>, current: Option<String>, position_ms: i64 },
}

impl Write {
    /// Whether the server getting it twice changes nothing more.
    pub fn repeatable(&self) -> bool {
        !matches!(self, Write::CreatePlaylist { .. } | Write::AddToPlaylist { .. } | Write::RemoveFromPlaylist { .. } | Write::CreateRadio { .. })
    }
}
