//! Local files: music on this device read as a library. A platform says where its files are and lists
//! them ([`Files`]); a scan compares what it lists with what the last scan kept ([`scan`]), reads the
//! tags, length and fingerprint of the files new or changed ([`tags`]), and builds the songs, albums and
//! artists ([`group`]). Pictures come from the files or their folders ([`tags::picture`]), playlists from
//! `.m3u` files ([`playlist`]). The library is kept in the core's index; this crate keeps nothing.

pub mod group;
pub mod playlist;
pub mod scan;
pub mod tags;
#[cfg(feature = "testing")]
pub mod testing;

use std::io;
use std::path::Path;

pub use symphonia::core::io::MediaSource;

/// One file where a local profile reads music, as its platform lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FoundFile {
    /// Its absolute path.
    pub path: String,
    pub size: u64,
    /// When it last changed, in Unix milliseconds.
    pub modified_ms: i64,
}

/// A playlist the platform keeps (Android's media library reads the `.m3u` files an app may not): its
/// songs as their files' paths, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DevicePlaylist {
    /// Its file's path, which names it apart from others.
    pub path: String,
    pub name: String,
    pub songs: Vec<String>,
}

/// Where a device's music files are for a local profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum MusicPlace {
    /// Folders the user picks.
    Folders,
    /// The device's own music, nothing to pick: Android's media library, the iPod's shared folder.
    Device,
}

/// What a file a scan lists is, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A song, if nori decodes what is in it ([`tags::read`]).
    Music,
    /// An `.m3u` playlist.
    Playlist,
    /// A picture, the cover of the music beside it when it is named so ([`group::folder_cover`]).
    Picture,
}

const MUSIC: [&str; 13] = ["mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "mka", "mkv", "webm", "m4b"];
const PLAYLISTS: [&str; 2] = ["m3u", "m3u8"];
const PICTURES: [&str; 5] = ["jpg", "jpeg", "png", "webp", "gif"];

/// The lower-case extension of `path`; empty without one.
pub fn extension(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

/// What `path` is by its extension; None for a file a scan leaves out.
pub fn kind_of(path: &str) -> Option<FileKind> {
    let ext = extension(path);
    let is = |list: &[&str]| list.contains(&ext.as_str());
    if is(&MUSIC) {
        Some(FileKind::Music)
    } else if is(&PLAYLISTS) {
        Some(FileKind::Playlist)
    } else if is(&PICTURES) {
        Some(FileKind::Picture)
    } else {
        None
    }
}

/// A device's music files, as a local profile reads them: the platform's half of a local profile.
pub trait Files: Send + Sync {
    /// Where they are.
    fn place(&self) -> MusicPlace;

    /// Every file of a [`FileKind`] under `folders` ([`MusicPlace::Folders`]) or in the device's music,
    /// with its size and time; None while one of them can't be read at all (a folder gone, a permission
    /// not given), which tells a scan nothing of what is there.
    fn list(&self, folders: &[String]) -> Option<Vec<FoundFile>>;

    /// The file at `path`, to read.
    fn open(&self, path: &str) -> io::Result<Box<dyn MediaSource>> {
        Ok(Box::new(std::fs::File::open(path)?))
    }

    /// The address a client loads cover `cover` at: a picture file, or a music file holding one.
    fn cover_address(&self, cover: &str) -> String {
        cover_address(cover)
    }

    /// The folders a profile reads when it names none ([`MusicPlace::Folders`]).
    fn default_folders(&self) -> Vec<String> {
        Vec::new()
    }

    /// A picture file the platform found for the album of the music file at `music`, where the files
    /// listed hold none (Android's media library reads a folder's picture an app may not); None without one.
    fn album_art(&self, _music: &str) -> Option<String> {
        None
    }

    /// The playlists the platform keeps, besides the `.m3u` files listed.
    fn playlists(&self) -> Vec<DevicePlaylist> {
        Vec::new()
    }
}

/// This computer's Music folder: the XDG one on Linux and the BSDs (`XDG_MUSIC_DIR`, else
/// `user-dirs.dirs`), else Music in the home folder (macOS, Windows).
pub fn music_folder() -> Option<String> {
    let env = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
    let home = env(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    if cfg!(any(windows, target_os = "macos", target_os = "ios")) {
        return Some(Path::new(&home).join("Music").to_string_lossy().into_owned());
    }
    if let Some(dir) = env("XDG_MUSIC_DIR") {
        return Some(dir);
    }
    let config = env("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config"));
    let dirs = std::fs::read_to_string(format!("{config}/user-dirs.dirs")).unwrap_or_default();
    Some(xdg_music_dir(&dirs, &home).unwrap_or_else(|| format!("{home}/Music")))
}

/// The Music folder `user-dirs.dirs` names (`XDG_MUSIC_DIR="$HOME/Music"`), `$HOME` as `home`; None
/// where it names none, or the home folder itself (the spec's way to turn it off).
fn xdg_music_dir(user_dirs: &str, home: &str) -> Option<String> {
    let line = user_dirs.lines().map(str::trim).find_map(|l| l.strip_prefix("XDG_MUSIC_DIR="))?;
    let dir = line.trim().trim_matches('"').replace("$HOME", home);
    (Path::new(&dir) != Path::new(home)).then_some(dir)
}

/// Files on this device's disk: the folders a profile names (this computer's Music folder until it names
/// others), or one folder that is the device's music (the iPod's shared Documents).
pub struct Disk {
    /// The device's music folder; None where the user picks folders.
    device: Option<String>,
}

impl Disk {
    /// The folders each profile names.
    pub fn folders() -> Disk {
        Disk { device: None }
    }

    /// The device's music in `folder`, whatever a profile names.
    pub fn device(folder: String) -> Disk {
        Disk { device: Some(folder) }
    }
}

impl Files for Disk {
    fn place(&self) -> MusicPlace {
        if self.device.is_some() { MusicPlace::Device } else { MusicPlace::Folders }
    }

    fn default_folders(&self) -> Vec<String> {
        music_folder().into_iter().collect()
    }

    fn list(&self, folders: &[String]) -> Option<Vec<FoundFile>> {
        let roots = match &self.device {
            Some(folder) => std::slice::from_ref(folder),
            None => folders,
        };
        let mut found = Vec::new();
        for root in roots {
            walk(std::fs::read_dir(root).ok()?, &mut found);
        }
        Some(found)
    }
}

/// Adds the files of a [`FileKind`] among `entries` to `found`, its subfolders' too; hidden entries and
/// folders reached through a link are left out, so a loop of links ends.
fn walk(entries: std::fs::ReadDir, found: &mut Vec<FoundFile>) {
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            if let Ok(below) = std::fs::read_dir(&path) {
                walk(below, found);
            }
            continue;
        }
        let Some(text) = path.to_str() else { continue };
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if !meta.is_file() || kind_of(text).is_none() {
            continue;
        }
        let modified_ms = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
        found.push(FoundFile { path: text.to_string(), size: meta.len(), modified_ms });
    }
}

/// What the address of a local cover starts with.
const COVER: &str = "file://";

/// The address the cover loader reads cover `cover` at: a picture's path, or a music file's whose
/// picture it is.
pub fn cover_address(cover: &str) -> String {
    format!("{COVER}{cover}")
}

/// The path of cover address `address`; None for an address that is no local cover.
pub fn cover_path(address: &str) -> Option<&str> {
    address.strip_prefix(COVER)
}

/// The picture of local cover `path`: the picture file, or the one the music file holds.
pub fn cover_bytes(path: &str) -> io::Result<Vec<u8>> {
    if kind_of(path) == Some(FileKind::Picture) {
        return std::fs::read(path);
    }
    tags::picture(Box::new(std::fs::File::open(path)?), &extension(path)).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no picture in it"))
}

/// A 64-bit FNV-1a hash: stable across Rust versions, as what is kept with it must be.
#[derive(Clone, Copy)]
pub(crate) struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub(crate) fn add(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }

    /// The hash of `text`, as 16 hex digits.
    pub(crate) fn hex(text: &str) -> String {
        let mut h = Fnv::default();
        h.add(text.as_bytes());
        format!("{:016x}", h.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_music_folder_is_the_one_user_dirs_names() {
        let dirs = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_MUSIC_DIR=\"$HOME/Musik\"\n";
        assert_eq!(xdg_music_dir(dirs, "/home/ann").as_deref(), Some("/home/ann/Musik"));
        assert_eq!(xdg_music_dir("XDG_MUSIC_DIR=\"/mnt/music\"", "/home/ann").as_deref(), Some("/mnt/music"));
        assert_eq!(xdg_music_dir("XDG_MUSIC_DIR=\"$HOME/\"", "/home/ann"), None, "the home folder turns it off");
        assert_eq!(xdg_music_dir("XDG_VIDEOS_DIR=\"$HOME/Videos\"", "/home/ann"), None);
    }

    #[test]
    fn a_disk_lists_music_playlists_and_pictures_under_its_folders() {
        let dir = nori_testdir::TempDir::new("local-walk");
        let write = |rel: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        };
        for rel in ["a/01.FLAC", "a/cover.jpg", "a/notes.txt", "a/b/02.mp3", "mix.m3u8", ".hidden/03.mp3", "a/.04.mp3"] {
            write(rel);
        }
        let mut listed: Vec<String> = Disk::folders().list(&[dir.to_string_lossy().into()]).unwrap().into_iter().map(|f| f.path.strip_prefix(dir.to_str().unwrap()).unwrap().to_string()).collect();
        listed.sort();
        assert_eq!(listed, ["/a/01.FLAC", "/a/b/02.mp3", "/a/cover.jpg", "/mix.m3u8"]);
        assert_eq!(Disk::device(dir.join("a").to_string_lossy().into()).list(&["/elsewhere".into()]).unwrap().len(), 3, "the device's folder, whatever the profile names");
        let gone = dir.join("gone").to_string_lossy().into_owned();
        assert_eq!(Disk::folders().list(&[dir.to_string_lossy().into(), gone.clone()]), None, "a folder that can't be read tells nothing");
        assert_eq!(Disk::device(gone).list(&[]), None);
    }
}
