//! `.m3u` playlists beside the music: each line a song's path, absolute or from the playlist's folder.

use std::path::{Component, Path, PathBuf};

/// A playlist file read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M3u {
    /// Its `#PLAYLIST:` name, else its file's.
    pub name: String,
    /// The paths it lists, in order, as absolute paths; web addresses are left out.
    pub paths: Vec<String>,
}

/// The id of the playlist in the file at `path`.
pub fn id(path: &str) -> String {
    format!("local-pl-{}", crate::Fnv::hex(path))
}

/// The playlist at `path` holding `text`.
pub fn parse(path: &str, text: &str) -> M3u {
    let dir = Path::new(path).parent().unwrap_or(Path::new("/"));
    let mut name = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut paths = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(n) = line.strip_prefix("#PLAYLIST:") {
            name = n.trim().to_string();
        } else if !line.starts_with('#') && !line.contains("://") {
            paths.push(normal(&dir.join(line.replace('\\', "/"))));
        }
    }
    M3u { name, paths }
}

/// `path` with its `.` and `..` resolved.
fn normal(path: &Path) -> String {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_lists_its_songs_paths() {
        let text = "\u{feff}#EXTM3U\n#PLAYLIST:Road trip\n#EXTINF:123,Ann - One\nFirst/01.flac\n\n../Other/02.mp3\nsub\\03.mp3\n/abs/04.ogg\nhttps://radio.example/live\n";
        let m = parse("/m/lists/trip.m3u8", text);
        assert_eq!(m.name, "Road trip");
        assert_eq!(m.paths, ["/m/lists/First/01.flac", "/m/Other/02.mp3", "/m/lists/sub/03.mp3", "/abs/04.ogg"]);
        assert_eq!(parse("/m/Chill.m3u", "a.mp3").name, "Chill");
    }
}
