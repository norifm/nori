//! The songs, albums and artists of the music files a scan kept. An album is its songs with one album
//! name and album artist; songs tagged without an album artist are one album by the folder they are in
//! (a "CD 2" folder counts as its parent), and when their artists differ it is a compilation. An artist
//! is everyone credited on a song or as an album's artist. Covers are a folder's picture, else one a
//! file holds.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use nori_model::{Album, Artist, ArtistRef, Song};

use crate::tags::Tags;
use crate::Fnv;

/// A music file as a scan kept it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The row that is its song ([`song_id`]).
    pub row: i64,
    pub path: String,
    pub size: u64,
    /// When the library first had it, as the index keeps dates.
    pub added: String,
    pub tags: Tags,
}

/// The library a scan made.
#[derive(Debug, Default)]
pub struct Library {
    pub songs: Vec<Song>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

/// The song kept as row `row`.
pub fn song_id(row: i64) -> String {
    format!("local-{row}")
}

fn album_id(key: &str) -> String {
    format!("local-al-{}", Fnv::hex(key))
}

fn artist_id(name: &str) -> String {
    format!("local-ar-{}", Fnv::hex(&name.to_lowercase()))
}

/// Names a cover picture goes by beside the music, the most likely first.
const COVER_NAMES: [&str; 5] = ["cover", "folder", "front", "album", "albumart"];

/// The cover among `pictures` for the music in `dir`: one named as covers are, else the folder's only
/// picture.
pub fn folder_cover<'a>(dir: &Path, pictures: &'a [String]) -> Option<&'a String> {
    let here: Vec<&String> = pictures.iter().filter(|p| Path::new(p).parent() == Some(dir)).collect();
    let named = |name: &str| here.iter().copied().find(|p| Path::new(p).file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case(name)));
    COVER_NAMES.iter().find_map(|n| named(n)).or(match here.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// Whether a folder's name says it holds one disc of an album ("CD 2", "Disc1", "disk-3").
fn is_disc_folder(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    let rest = ["disc", "disk", "cd"].iter().find_map(|p| name.strip_prefix(p)).unwrap_or("");
    let digits = rest.trim_start_matches([' ', '-', '_']);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// The folder an album's file is in: the album's own, above a disc's.
fn album_dir(path: &str) -> &Path {
    let dir = Path::new(path).parent().unwrap_or(Path::new(""));
    match dir.file_name().and_then(|n| n.to_str()) {
        Some(n) if is_disc_folder(n) => dir.parent().unwrap_or(dir),
        _ => dir,
    }
}

/// The album name of `e`: its tag, else its folder's name.
fn album_name(e: &Entry) -> String {
    e.tags.album.clone().unwrap_or_else(|| album_dir(&e.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// What tells `e`'s album from others: its album artist and name, else its folder and name.
fn album_key(e: &Entry) -> String {
    match &e.tags.album_artist {
        Some(by) => format!("artist\u{1f}{}\u{1f}{}", by.to_lowercase(), album_name(e).to_lowercase()),
        None => format!("folder\u{1f}{}\u{1f}{}", album_dir(&e.path).display(), album_name(e).to_lowercase()),
    }
}

/// The songs, albums and artists of `entries`, covers found among `pictures` (the scan's picture files),
/// else in the files, else by `found` for an album's first file ([`crate::Files::album_art`]).
pub fn group(entries: &[Entry], pictures: &[String], found: &dyn Fn(&str) -> Option<String>) -> Library {
    let mut albums: BTreeMap<String, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        albums.entry(album_key(e)).or_default().push(e);
    }
    let mut lib = Library::default();
    let mut artists: BTreeMap<String, Artist> = BTreeMap::new();
    for (key, mut songs) in albums {
        songs.sort_by_key(|e| (e.tags.disc.unwrap_or(1), e.tags.track.unwrap_or(u32::MAX), e.path.clone()));
        let first = songs[0];
        let id = album_id(&key);
        let name = album_name(first);
        let folder_cover = songs.iter().find_map(|e| folder_cover(album_dir(&e.path), pictures)).cloned();
        let cover = folder_cover.clone().or_else(|| songs.iter().find(|e| e.tags.picture).map(|e| e.path.clone())).or_else(|| found(&first.path));
        let mut credited: Vec<&str> = Vec::new();
        for e in &songs {
            for a in &e.tags.artists {
                if !credited.contains(&a.as_str()) {
                    credited.push(a);
                }
            }
        }
        let (by, by_id, compilation) = match (&first.tags.album_artist, credited.as_slice()) {
            (Some(by), _) => (by.clone(), Some(credit(&mut artists, by).id), songs.iter().any(|e| e.tags.compilation)),
            (None, []) => (String::new(), None, songs.iter().any(|e| e.tags.compilation)),
            (None, [one]) => (one.to_string(), Some(credit(&mut artists, one).id), songs.iter().any(|e| e.tags.compilation)),
            (None, many) => (many.join(", "), None, true),
        };
        let mut genres: HashMap<&str, usize> = HashMap::new();
        for e in &songs {
            if let Some(g) = &e.tags.genre {
                *genres.entry(g).or_default() += 1;
            }
        }
        let genre = genres.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(g, _)| g.to_string());
        let year = songs.iter().filter_map(|e| e.tags.year).min().unwrap_or(0);
        let mut seconds = 0;
        for e in &songs {
            let refs: Vec<ArtistRef> = e.tags.artists.iter().map(|a| credit(&mut artists, a)).collect();
            // A song's own picture shows unless its folder has one.
            let own = (folder_cover.is_none() && e.tags.picture).then(|| e.path.clone());
            let song = song(e, &id, &name, refs, own.or_else(|| cover.clone()));
            seconds += song.duration;
            lib.songs.push(song);
        }
        if let Some(a) = by_id.as_ref().and_then(|id| artists.get_mut(id)) {
            a.album_count = Some(a.album_count.unwrap_or(0) + 1);
            a.cover_art = a.cover_art.take().or_else(|| cover.clone());
        }
        lib.albums.push(
            Album {
                id,
                name,
                artist: by,
                artist_id: by_id,
                cover_art: cover,
                song_count: songs.len() as u32,
                duration: seconds,
                year,
                genre,
                is_compilation: compilation,
                created: songs.iter().map(|e| e.added.clone()).min(),
                ..Default::default()
            }
            .dressed(),
        );
    }
    for a in artists.values_mut().filter(|a| a.cover_art.is_none()) {
        a.cover_art = lib.songs.iter().find(|s| s.artists.iter().any(|r| r.id == a.id)).and_then(|s| s.cover_art.clone());
    }
    lib.artists = artists.into_values().collect();
    lib
}

/// The artist called `name`, among `artists` from now on.
fn credit(artists: &mut BTreeMap<String, Artist>, name: &str) -> ArtistRef {
    let id = artist_id(name);
    artists.entry(id.clone()).or_insert_with(|| Artist { id: id.clone(), name: name.to_string(), album_count: Some(0), ..Default::default() });
    ArtistRef { id, name: name.to_string() }
}

/// `e`'s song on album `album_id` named `album`, crediting `artists`, with `cover`.
fn song(e: &Entry, album_id: &str, album: &str, artists: Vec<ArtistRef>, cover: Option<String>) -> Song {
    let t = &e.tags;
    let title = t.title.clone().unwrap_or_else(|| Path::new(&e.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let suffix = crate::extension(&e.path);
    let duration = ((t.duration_ms + 500) / 1000) as u32;
    let bit_rate = (e.size * 8).checked_div(t.duration_ms).unwrap_or(0) as u32;
    Song {
        id: song_id(e.row),
        title,
        album: album.to_string(),
        artist: t.artists.join(", "),
        album_id: Some(album_id.to_string()),
        artist_id: artists.first().map(|a| a.id.clone()),
        cover_art: cover,
        duration,
        track: t.track.unwrap_or(0),
        disc_number: t.disc.unwrap_or(0),
        year: t.year.unwrap_or(0),
        genre: t.genre.clone(),
        content_type: content_type(&suffix).to_string(),
        suffix,
        bit_rate,
        size: e.size,
        sampling_rate: t.sample_rate,
        bit_depth: t.bit_depth.unwrap_or(0),
        replay_gain: t.gain.clone(),
        artists,
        created: Some(e.added.clone()),
        path: Some(e.path.clone()),
        channel_count: t.channels,
        music_brainz_id: t.recording.clone(),
        bpm: t.bpm.unwrap_or(0),
        comment: t.comment.clone(),
        ..Default::default()
    }
    .dressed()
}

/// The media type of a music file with `suffix`.
fn content_type(suffix: &str) -> &'static str {
    match suffix {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "mp4" | "m4b" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        _ => "audio/x-matroska",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(row: i64, path: &str, title: &str, artists: &[&str], album: Option<&str>, by: Option<&str>, track: u32) -> Entry {
        let tags = Tags {
            title: Some(title.into()),
            artists: artists.iter().map(|a| a.to_string()).collect(),
            album: album.map(String::from),
            album_artist: by.map(String::from),
            track: Some(track),
            duration_ms: 60_000,
            ..Tags::default()
        };
        Entry { row, path: path.into(), size: 480_000, added: format!("2026-01-0{row}T00:00:00Z"), tags }
    }

    fn albums(lib: &Library) -> Vec<(String, String, u32, bool)> {
        lib.albums.iter().map(|a| (a.name.clone(), a.artist.clone(), a.song_count, a.is_compilation)).collect()
    }

    #[test]
    fn albums_go_by_album_artist_else_by_folder() {
        let lib = group(
            &[
                entry(1, "/m/Ann/First/01.flac", "One", &["Ann"], Some("First"), Some("Ann"), 1),
                entry(2, "/m/Ann/First/CD 2/01.flac", "Two", &["Ann feat. Bo"], Some("First"), Some("Ann"), 1),
                entry(3, "/m/Hits/01.mp3", "Up", &["Cy"], Some("Hits"), None, 1),
                entry(4, "/m/Hits/02.mp3", "Down", &["Di"], Some("Hits"), None, 2),
                entry(5, "/m/Other Hits/01.mp3", "Left", &["Cy"], Some("Hits"), None, 1),
                entry(6, "/m/Loose/song.mp3", "Loose", &[], None, None, 1),
            ],
            &[],
            &|_| None,
        );
        let mut got = albums(&lib);
        got.sort();
        assert_eq!(
            got,
            [
                ("First".into(), "Ann".into(), 2, false),
                ("Hits".into(), "Cy".into(), 1, false),
                ("Hits".into(), "Cy, Di".into(), 2, true),
                ("Loose".into(), "".into(), 1, false),
            ]
        );
        let names: Vec<&str> = lib.artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names.len(), 4);
        for name in ["Ann", "Ann feat. Bo", "Cy", "Di"] {
            assert!(names.contains(&name), "{name} has a page");
        }
        let ann = lib.artists.iter().find(|a| a.name == "Ann").unwrap();
        assert_eq!(ann.album_count, Some(1));
        let comp = lib.albums.iter().find(|a| a.is_compilation && a.name == "Hits").unwrap();
        assert_eq!(comp.artist_id, None, "a compilation is no one artist's");
    }

    #[test]
    fn songs_are_their_rows_and_carry_their_album() {
        let lib = group(&[entry(7, "/m/a/x.flac", "X", &["Ann", "Bo"], Some("A"), None, 4)], &[], &|_| None);
        let s = &lib.songs[0];
        assert_eq!((s.id.as_str(), s.artist.as_str(), s.album.as_str(), s.track, s.suffix.as_str(), s.duration, s.bit_rate), ("local-7", "Ann, Bo", "A", 4, "flac", 60, 64));
        assert_eq!(s.album_id.as_deref(), Some(lib.albums[0].id.as_str()));
        assert_eq!(s.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Ann", "Bo"]);
        assert_eq!(s.artist_id.as_deref(), Some(s.artists[0].id.as_str()));
        assert_eq!(group(&[entry(7, "/m/a/x.flac", "Y", &["ann"], Some("A"), None, 4)], &[], &|_| None).songs[0].artist_id, s.artist_id, "an artist is its name, case aside");
    }

    #[test]
    fn covers_are_the_folders_picture_else_one_a_file_holds() {
        let mut held = entry(1, "/m/a/01.flac", "One", &["Ann"], Some("A"), None, 1);
        held.tags.picture = true;
        let plain = entry(2, "/m/a/02.flac", "Two", &["Ann"], Some("A"), None, 2);
        let lib = group(&[held.clone(), plain.clone()], &["/m/a/Folder.JPG".into(), "/m/a/back.jpg".into(), "/m/b/cover.jpg".into()], &|_| None);
        assert_eq!(lib.albums[0].cover_art.as_deref(), Some("/m/a/Folder.JPG"));
        assert!(lib.songs.iter().all(|s| s.cover_art.as_deref() == Some("/m/a/Folder.JPG")), "a folder's picture is every song's");
        let lib = group(&[plain.clone(), held], &[], &|_| None);
        assert_eq!(lib.albums[0].cover_art.as_deref(), Some("/m/a/01.flac"), "the picture a song holds");
        assert_eq!(lib.songs.iter().map(|s| s.cover_art.as_deref()).collect::<Vec<_>>(), [Some("/m/a/01.flac"), Some("/m/a/01.flac")]);
        assert_eq!(lib.artists[0].cover_art.as_deref(), Some("/m/a/01.flac"));
        assert_eq!(group(std::slice::from_ref(&plain), &[], &|_| None).albums[0].cover_art, None);
        let found = group(&[plain], &[], &|music| Some(format!("{music}.art")));
        assert_eq!((found.albums[0].cover_art.as_deref(), found.songs[0].cover_art.as_deref()), (Some("/m/a/02.flac.art"), Some("/m/a/02.flac.art")), "the platform's, where there is none");
        assert_eq!(folder_cover(Path::new("/m/c"), &["/m/c/scan.png".into()]).map(String::as_str), Some("/m/c/scan.png"), "a folder's only picture");
        assert_eq!(folder_cover(Path::new("/m/c"), &["/m/c/a.png".into(), "/m/c/b.png".into()]), None);
    }

    #[test]
    fn disc_folders_are_their_albums() {
        for (name, disc) in [("CD 2", true), ("disc1", true), ("Disk-3", true), ("cd", false), ("CDs", false), ("Discovery", false)] {
            assert_eq!(is_disc_folder(name), disc, "{name}");
        }
    }
}
